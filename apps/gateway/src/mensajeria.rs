//! Envío por WhatsApp desde el gateway: el cliente de Cloud API (si hay token) con el ritmo
//! máximo de mensajes por segundo de cada línea.

use std::{collections::HashMap, sync::Arc, time::Duration};

use application::{
    AppError, AppResult,
    puertos::mensajeria::{MediaDescargada, MensajeSaliente, Mensajeria},
};
use infra_whatsapp::ClienteWhatsapp;
use tokio::{sync::Mutex, time::Instant};

/// Límite de Meta por número emisor (AGENTS.md §9).
pub const MENSAJES_POR_SEGUNDO: u32 = 80;

/// Espaciado mínimo entre envíos de una misma línea.
#[derive(Debug, Default)]
pub struct Ritmo {
    proximo: Mutex<HashMap<String, Instant>>,
}

impl Ritmo {
    fn intervalo() -> Duration {
        Duration::from_secs(1) / MENSAJES_POR_SEGUNDO
    }

    /// Espera el turno de la línea y reserva el siguiente.
    pub async fn esperar_turno(&self, linea: &str) {
        let turno = {
            let mut proximo = self.proximo.lock().await;
            let ahora = Instant::now();
            let turno = proximo.get(linea).copied().unwrap_or(ahora).max(ahora);
            proximo.insert(linea.to_owned(), turno + Self::intervalo());
            turno
        };
        tokio::time::sleep_until(turno).await;
    }
}

/// Mensajería del gateway: `None` = WhatsApp sin token (no envía ni descarga).
#[derive(Clone, Default)]
pub struct MensajeriaGateway {
    cliente: Option<ClienteWhatsapp>,
    ritmo: Arc<Ritmo>,
}

impl MensajeriaGateway {
    pub fn new(cliente: Option<ClienteWhatsapp>) -> Self {
        Self {
            cliente,
            ritmo: Arc::default(),
        }
    }

    /// ¿Hay token para enviar?
    pub fn activa(&self) -> bool {
        self.cliente.is_some()
    }

    fn cliente(&self) -> AppResult<&ClienteWhatsapp> {
        self.cliente.as_ref().ok_or_else(|| {
            AppError::infra(std::io::Error::other(
                "WHATSAPP_ACCESS_TOKEN no está configurado",
            ))
        })
    }
}

impl Mensajeria for MensajeriaGateway {
    async fn enviar(&self, mensaje: &MensajeSaliente) -> AppResult<String> {
        let cliente = self.cliente()?;
        self.ritmo.esperar_turno(&mensaje.linea).await;
        cliente.enviar(mensaje).await
    }

    async fn descargar_media(&self, media_id: &str) -> AppResult<MediaDescargada> {
        self.cliente()?.descargar_media(media_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn espacia_los_envios_de_una_linea_pero_no_entre_lineas() {
        let ritmo = Ritmo::default();
        let inicio = Instant::now();
        for _ in 0..3 {
            ritmo.esperar_turno("A").await;
        }
        assert_eq!(Instant::now() - inicio, Ritmo::intervalo() * 2);

        let antes = Instant::now();
        ritmo.esperar_turno("B").await;
        assert_eq!(Instant::now(), antes);
    }

    #[tokio::test]
    async fn sin_token_no_envia() {
        let m = MensajeriaGateway::new(None);
        assert!(!m.activa());
        assert!(m.descargar_media("x").await.is_err());
    }
}
