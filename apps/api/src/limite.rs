//! Límite de peticiones por IP para las rutas expuestas sin sesión o con credenciales
//! adivinables (AGENTS.md §10: rate limiting en endpoints públicos).
//!
//! - Ventana fija de un minuto por (grupo, IP), en memoria del proceso: suficiente para una
//!   instancia de la API detrás de Nginx. Si se escala a varias instancias, Nginx
//!   (`limit_req`) o un almacén compartido toman el relevo; el contrato (429 +
//!   `Retry-After`) no cambia.
//! - Grupos: [`Grupo::Auth`] (registro, inicio de sesión, segundo factor, recuperar contraseña:
//!   frena la fuerza bruta) y [`Grupo::Publico`] (papeleta, puentes, descarga con enlace).
//! - La IP real detrás de Nginx sale de `X-Real-IP` **solo** con `CONFIAR_PROXY=true`; si no,
//!   de la conexión. Así un cliente directo no puede falsear su IP con la cabecera.
//! - Los extractores [`LimiteAuth`] y [`LimitePublico`] se agregan a los handlers.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::request::Parts,
};

use crate::{error::ApiError, state::AppState};

/// Duración de la ventana.
pub const VENTANA: Duration = Duration::from_secs(60);
/// Entradas máximas antes de limpiar las ventanas vencidas.
const MAX_ENTRADAS: usize = 50_000;

/// Grupo de rutas con su propio límite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Grupo {
    Auth,
    Publico,
}

/// Límites por minuto (`0` = sin límite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limites {
    pub auth_por_minuto: u32,
    pub publico_por_minuto: u32,
    /// Tomar la IP de `X-Real-IP` (solo detrás de un proxy propio).
    pub confiar_proxy: bool,
}

impl Limites {
    /// Sin límites (pruebas y herramientas internas).
    pub const fn ninguno() -> Self {
        Self {
            auth_por_minuto: 0,
            publico_por_minuto: 0,
            confiar_proxy: false,
        }
    }

    const fn de(&self, g: Grupo) -> u32 {
        match g {
            Grupo::Auth => self.auth_por_minuto,
            Grupo::Publico => self.publico_por_minuto,
        }
    }
}

/// Contador por (grupo, IP).
#[derive(Debug)]
pub struct Limitador {
    limites: Limites,
    ventanas: Mutex<HashMap<(Grupo, String), (Instant, u32)>>,
}

impl Limitador {
    /// Limitador con los límites por grupo.
    pub fn new(limites: Limites) -> Self {
        Self {
            limites,
            ventanas: Mutex::new(HashMap::new()),
        }
    }

    /// Cuenta una petición. `Err(segundos)` si superó el límite, con la espera hasta la ventana
    /// siguiente.
    pub fn contar(&self, grupo: Grupo, clave: &str, ahora: Instant) -> Result<(), u64> {
        let limite = self.limites.de(grupo);
        if limite == 0 {
            return Ok(());
        }
        let mut v = self
            .ventanas
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if v.len() > MAX_ENTRADAS {
            v.retain(|_, (inicio, _)| ahora.duration_since(*inicio) < VENTANA);
        }
        let e = v.entry((grupo, clave.to_owned())).or_insert((ahora, 0));
        if ahora.duration_since(e.0) >= VENTANA {
            *e = (ahora, 0);
        }
        if e.1 >= limite {
            let resta = VENTANA.saturating_sub(ahora.duration_since(e.0));
            return Err(resta.as_secs().max(1));
        }
        e.1 += 1;
        Ok(())
    }

    /// IP del cliente según la configuración del proxy.
    pub fn clave(&self, parts: &Parts) -> String {
        if self.limites.confiar_proxy
            && let Some(ip) = parts
                .headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<IpAddr>().ok())
        {
            return ip.to_string();
        }
        parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map_or_else(|| "desconocida".to_owned(), |c| c.0.ip().to_string())
    }
}

async fn exigir(parts: &Parts, state: &AppState, grupo: Grupo) -> Result<(), ApiError> {
    let l = &state.limitador;
    l.contar(grupo, &l.clave(parts), Instant::now())
        .map_err(|segundos| ApiError::DemasiadasPeticiones { segundos })
}

/// Cuenta la petición en el grupo [`Grupo::Auth`].
pub struct LimiteAuth;

impl FromRequestParts<AppState> for LimiteAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        exigir(parts, state, Grupo::Auth).await.map(|()| Self)
    }
}

/// Cuenta la petición en el grupo [`Grupo::Publico`].
pub struct LimitePublico;

impl FromRequestParts<AppState> for LimitePublico {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        exigir(parts, state, Grupo::Publico).await.map(|()| Self)
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Request;

    use super::*;

    fn limitador(auth: u32, proxy: bool) -> Limitador {
        Limitador::new(Limites {
            auth_por_minuto: auth,
            publico_por_minuto: 0,
            confiar_proxy: proxy,
        })
    }

    #[test]
    fn corta_al_superar_el_limite_y_reabre_con_la_ventana() {
        let l = limitador(2, false);
        let t = Instant::now();
        assert!(l.contar(Grupo::Auth, "1.2.3.4", t).is_ok());
        assert!(l.contar(Grupo::Auth, "1.2.3.4", t).is_ok());
        assert_eq!(
            l.contar(Grupo::Auth, "1.2.3.4", t + Duration::from_secs(10)),
            Err(50)
        );
        assert!(l.contar(Grupo::Auth, "5.6.7.8", t).is_ok(), "otra IP");
        assert!(
            l.contar(Grupo::Publico, "1.2.3.4", t).is_ok(),
            "otro grupo sin límite"
        );
        assert!(l.contar(Grupo::Auth, "1.2.3.4", t + VENTANA).is_ok());
    }

    #[test]
    fn la_cabecera_del_proxy_solo_cuenta_si_se_confia() {
        let (parts, ()) = Request::builder()
            .header("x-real-ip", "9.9.9.9")
            .body(())
            .expect("petición")
            .into_parts();
        assert_eq!(limitador(1, true).clave(&parts), "9.9.9.9");
        assert_eq!(limitador(1, false).clave(&parts), "desconocida");
    }
}
