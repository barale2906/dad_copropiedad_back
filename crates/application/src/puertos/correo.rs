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
    /// Archivos adjuntos (p. ej. el ZIP de la factura electrónica para el adquirente). Viajan en
    /// el outbox con el mensaje: que sean pocos y pequeños.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjuntos: Vec<AdjuntoCorreo>,
}

/// Archivo adjunto a un correo.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdjuntoCorreo {
    /// Nombre con que lo recibe el destinatario.
    pub nombre: String,
    /// Tipo MIME (`application/zip`, `application/pdf`…).
    pub tipo: String,
    /// Contenido (hexadecimal en el outbox).
    #[serde(with = "hex_serde")]
    pub contenido: Vec<u8>,
}

impl std::fmt::Debug for AdjuntoCorreo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdjuntoCorreo")
            .field("nombre", &self.nombre)
            .field("tipo", &self.tipo)
            .field("bytes", &self.contenido.len())
            .finish()
    }
}

mod hex_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let texto = String::deserialize(d)?;
        hex::decode(texto).map_err(serde::de::Error::custom)
    }
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
