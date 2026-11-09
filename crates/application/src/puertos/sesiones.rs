//! Sesiones del chat de WhatsApp (plataforma, sin tenant): paso del menú, copropiedad elegida y
//! borradores. Se guardan en PostgreSQL con vencimiento (decisión del 2026-11-08); no son
//! fuente de verdad: si se pierden, el usuario vuelve al menú.
//!
//! Cada mensaje entrante se cierra con [`SesionesConversacion::cerrar_turno`], que en **una
//! sola transacción** guarda la sesión, encola las respuestas y marca el `wamid` como
//! procesado. Si el proceso cae antes, Meta reintenta el webhook y el turno se repite: los
//! casos de uso que invoca (radicar un ticket, reservar) son idempotentes por `wamid`.

use std::future::Future;

use chrono::{DateTime, Utc};

use crate::{AppResult, puertos::mensajeria::MensajeSaliente};

/// Estado guardado de una conversación. `estado` es el paso del chat serializado (lo
/// interpreta [`crate::casos_uso::chat`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SesionGuardada {
    pub estado: serde_json::Value,
    /// Último mensaje del usuario: abre la ventana de 24 h para responder con texto libre.
    pub ultimo_entrante_en: DateTime<Utc>,
}

/// Resultado de un turno del chat que se persiste junto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CierreTurno {
    pub linea: String,
    pub telefono: String,
    pub wamid: String,
    /// `None` = borrar la sesión.
    pub sesion: Option<SesionGuardada>,
    pub expira_en: DateTime<Utc>,
    pub respuestas: Vec<MensajeSaliente>,
}

pub trait SesionesConversacion: Send + Sync {
    /// ¿Ya se procesó este `wamid`? (Meta reintenta los webhooks.)
    fn ya_procesado(&self, wamid: &str) -> impl Future<Output = AppResult<bool>> + Send;

    /// Marca el `wamid` como procesado sin tocar la sesión (mensajes de una asamblea).
    /// Devuelve `false` si ya lo estaba.
    fn marcar_procesado(&self, wamid: &str) -> impl Future<Output = AppResult<bool>> + Send;

    /// Sesión vigente a `ahora`, si existe.
    fn obtener(
        &self,
        linea: &str,
        telefono: &str,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<SesionGuardada>>> + Send;

    /// Guarda la sesión, encola las respuestas (outbox de plataforma, prioridad de
    /// confirmación) y marca el `wamid`, todo en una transacción. Devuelve `false` si el
    /// `wamid` ya estaba procesado (otro proceso ganó la carrera): entonces no hace nada.
    fn cerrar_turno(&self, cierre: &CierreTurno) -> impl Future<Output = AppResult<bool>> + Send;
}
