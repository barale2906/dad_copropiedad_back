//! Reintentos seguros de los POST con la cabecera `Idempotency-Key` (plan 10, fase 6).
//!
//! Con la cabecera y una sesión válida, la primera petición reserva la clave para la cuenta y
//! guarda su respuesta; un reintento con la misma clave y la misma petición (método, ruta y
//! cuerpo) recibe esa respuesta con `Idempotency-Replayed: true`, sin ejecutar nada otra vez.
//! La misma clave con otra petición responde 422 `idempotencia_reusada`; mientras la primera
//! sigue en curso, 409 `conflicto`. Las respuestas 5xx no se guardan (se puede reintentar). Las
//! claves valen 24 h; el worker de gestión del gateway purga las vencidas.
//!
//! Sin la cabecera, o sin sesión válida (el handler responde 401), la petición pasa intacta.
//! Tampoco toca los votos ([`idempotencia_de_negocio`]): allí la clave es parte del voto
//! (invariante 2 de AGENTS.md §9) y la resuelve la transacción.

use application::{
    AppError,
    casos_uso::autenticacion::autenticar,
    puertos::{
        RegistroIdempotencia, Reloj, RelojSistema,
        idempotencia::{Reserva, RespuestaGuardada},
    },
};
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderName, HeaderValue, Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use domain::{ClaveIdempotencia, CuentaId};
use sha2::{Digest, Sha256};

use crate::{error::ApiError, sesion, state::AppState};

/// Cabecera de la clave.
pub const CABECERA: HeaderName = HeaderName::from_static("idempotency-key");
/// Cabecera que marca una respuesta repetida.
pub const REPETIDA: HeaderName = HeaderName::from_static("idempotency-replayed");
/// Cuerpo máximo que se lee para calcular la huella (fotos en Base64 de la portería).
const MAX_PETICION: usize = 32 * 1024 * 1024;
/// Respuesta máxima que se guarda (el CHECK de la tabla); más grande, se libera la clave.
const MAX_RESPUESTA: usize = 1024 * 1024;

/// Si la petición se cancela (p. ej. por el timeout) sin terminar, libera la clave para que
/// el cliente pueda reintentar.
struct Reservada {
    state: AppState,
    cuenta: CuentaId,
    clave: ClaveIdempotencia,
    terminada: bool,
}

impl Drop for Reservada {
    fn drop(&mut self) {
        if !self.terminada {
            let (registro, cuenta, clave) = (
                self.state.idempotencia.clone(),
                self.cuenta,
                self.clave.clone(),
            );
            tokio::spawn(async move {
                if let Err(e) = registro.liberar(cuenta, &clave).await {
                    tracing::warn!(error = %e, "no se pudo liberar una clave de idempotencia");
                }
            });
        }
    }
}

/// Rutas donde la `Idempotency-Key` la usa el caso de uso (votos de la mesa y de la papeleta):
/// el middleware las deja pasar sin reservar nada.
pub fn idempotencia_de_negocio(ruta: &str) -> bool {
    ruta.ends_with("/votos")
}

fn huella(metodo: &Method, ruta: &str, cuerpo: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(metodo.as_str());
    h.update(b"\n");
    h.update(ruta);
    h.update(b"\n");
    h.update(cuerpo);
    hex::encode(h.finalize())
}

fn repetir(r: RespuestaGuardada) -> Response {
    let mut res = Response::new(Body::from(r.cuerpo));
    *res.status_mut() =
        axum::http::StatusCode::from_u16(r.estado).unwrap_or(axum::http::StatusCode::OK);
    if let Some(t) = r.tipo.and_then(|t| HeaderValue::from_str(&t).ok()) {
        res.headers_mut().insert(header::CONTENT_TYPE, t);
    }
    res.headers_mut()
        .insert(REPETIDA, HeaderValue::from_static("true"));
    res
}

async fn cuenta_de(state: &AppState, parts: &axum::http::request::Parts) -> Option<CuentaId> {
    let token = sesion::token(parts).ok()?;
    autenticar(&state.plataforma, &RelojSistema, token)
        .await
        .ok()
        .map(|s| s.cuenta_id)
}

/// Middleware de `Idempotency-Key` para los POST.
pub async fn capa(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if req.method() != Method::POST
        || !req.headers().contains_key(CABECERA)
        || idempotencia_de_negocio(req.uri().path())
    {
        return next.run(req).await;
    }
    let clave = req
        .headers()
        .get(CABECERA)
        .and_then(|v| v.to_str().ok())
        .map(ClaveIdempotencia::new);
    let Some(Ok(clave)) = clave else {
        return ApiError::PeticionInvalida(
            "`Idempotency-Key` debe ser texto visible de 1 a 200 caracteres".into(),
        )
        .into_response();
    };
    let (parts, cuerpo) = req.into_parts();
    let Some(cuenta) = cuenta_de(&state, &parts).await else {
        return next.run(Request::from_parts(parts, cuerpo)).await;
    };
    let Ok(bytes) = to_bytes(cuerpo, MAX_PETICION).await else {
        return ApiError::PeticionInvalida("el cuerpo es demasiado grande".into()).into_response();
    };
    let ruta = parts
        .uri
        .path_and_query()
        .map_or_else(|| parts.uri.path().to_owned(), ToString::to_string);
    let h = huella(&parts.method, &ruta, &bytes);
    match state
        .idempotencia
        .reservar(cuenta, &clave, &h, RelojSistema.ahora())
        .await
    {
        Err(e) => return ApiError::from(e).into_response(),
        Ok(Reserva::OtraPeticion) => return ApiError::IdempotenciaReusada.into_response(),
        Ok(Reserva::EnCurso) => {
            return ApiError::from(AppError::Conflicto(
                "una petición con esta `Idempotency-Key` aún está en curso".into(),
            ))
            .into_response();
        }
        Ok(Reserva::Repetida(r)) => return repetir(r),
        Ok(Reserva::Nueva) => {}
    }
    let mut reservada = Reservada {
        state: state.clone(),
        cuenta,
        clave,
        terminada: false,
    };
    let respuesta = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    let (rparts, rcuerpo) = respuesta.into_parts();
    if rparts.status.is_server_error() {
        // Se libera en el Drop: el cliente puede reintentar.
        return Response::from_parts(rparts, rcuerpo);
    }
    let rbytes = match to_bytes(rcuerpo, usize::MAX).await {
        Ok(b) => b,
        Err(e) => return ApiError::from(AppError::infra(e)).into_response(),
    };
    if rbytes.len() <= MAX_RESPUESTA {
        let guardada = RespuestaGuardada {
            estado: rparts.status.as_u16(),
            tipo: rparts
                .headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned),
            cuerpo: rbytes.to_vec(),
        };
        match state
            .idempotencia
            .guardar(cuenta, &reservada.clave, &guardada)
            .await
        {
            Ok(()) => reservada.terminada = true,
            Err(e) => tracing::warn!(error = %e, "no se pudo guardar la respuesta idempotente"),
        }
    }
    Response::from_parts(rparts, Body::from(rbytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_huella_distingue_metodo_ruta_y_cuerpo() {
        let base = huella(&Method::POST, "/a", b"{}");
        assert_eq!(base.len(), 64);
        assert_eq!(base, huella(&Method::POST, "/a", b"{}"));
        assert_ne!(base, huella(&Method::POST, "/b", b"{}"));
        assert_ne!(base, huella(&Method::POST, "/a", b"{\"x\":1}"));
        assert_ne!(base, huella(&Method::PUT, "/a", b"{}"));
    }

    #[test]
    fn los_votos_resuelven_su_propia_idempotencia() {
        assert!(idempotencia_de_negocio(
            "/api/v1/copropiedades/x/asambleas/r/preguntas/p/votos"
        ));
        assert!(idempotencia_de_negocio(
            "/api/v1/copropiedades/x/papeleta/votos"
        ));
        assert!(!idempotencia_de_negocio("/api/v1/copropiedades/x/tickets"));
    }
}
