//! Puerto de envío de correo electrónico. Implementación: `crates/infra-correo` (SMTP; en
//! producción el servidor de correo de HestiaCP). Cambiar de proveedor no toca la aplicación.

use std::future::Future;

use domain::DireccionCorreo;
use serde::{Deserialize, Serialize};

use crate::AppResult;

/// Correo listo para enviar. El remitente lo fija el adaptador (configuración), no el mensaje.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorreoSaliente {
    pub para: DireccionCorreo,
    pub asunto: String,
    /// Versión en texto plano (obligatoria: la leen clientes sin HTML y mejora la entrega).
    pub texto: String,
    /// Versión HTML opcional; si existe, se envía como alternativa al texto.
    pub html: Option<String>,
}

/// Envío de correo.
pub trait EnvioCorreo: Send + Sync {
    /// Envía el correo. Devuelve el identificador que asigne el servidor, si lo informa.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Infraestructura`] si el servidor no acepta el mensaje (el worker lo
    /// reintenta según la política de reintentos).
    fn enviar(
        &self,
        correo: &CorreoSaliente,
    ) -> impl Future<Output = AppResult<Option<String>>> + Send;
}
