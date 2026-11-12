//! Puerto de mensajería (WhatsApp Cloud API hoy; otro BSP mañana sin tocar el dominio).
//! Implementación: `crates/infra-whatsapp`.

use std::future::Future;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::AppResult;

/// Mensaje recibido por el webhook, ya normalizado (independiente del proveedor).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MensajeEntrante {
    /// Línea que recibió el mensaje (`phone_number_id` en Meta).
    pub linea: String,
    /// Teléfono del remitente en formato E.164 sin `+` (como lo entrega Meta).
    pub telefono: String,
    /// Id único del mensaje. Meta reintenta webhooks: se usa como clave de idempotencia.
    pub wamid: String,
    pub recibido_en: DateTime<Utc>,
    pub contenido: ContenidoEntrante,
}

/// Contenido de un mensaje que llega por WhatsApp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum ContenidoEntrante {
    Texto {
        cuerpo: String,
    },
    /// Respuesta a botones o a una lista: `id` es el que la aplicación definió al enviarlos.
    Respuesta {
        id: String,
        titulo: String,
    },
    /// Foto (p. ej. de un daño). `media_id` se descarga con [`Mensajeria::descargar_media`].
    Imagen {
        media_id: String,
        leyenda: Option<String>,
    },
    /// Audio, ubicación, documento… se ignoran con una respuesta de ayuda.
    NoSoportado {
        tipo_original: String,
    },
}

/// Elemento seleccionable (botón o fila de lista).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eleccion {
    pub id: String,
    /// Título corto (WhatsApp admite 20 caracteres en botones y 24 en filas de lista).
    pub titulo: String,
    /// Texto secundario de una fila de lista (hasta 72 caracteres).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descripcion: Option<String>,
}

impl Eleccion {
    /// Elección sin descripción.
    pub fn new(id: impl Into<String>, titulo: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            titulo: titulo.into(),
            descripcion: None,
        }
    }

    /// Agrega la descripción (filas de lista).
    pub fn con_descripcion(mut self, descripcion: impl Into<String>) -> Self {
        self.descripcion = Some(descripcion.into());
        self
    }
}

/// Contenido de un mensaje que se envía por WhatsApp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum ContenidoSaliente {
    Texto {
        cuerpo: String,
    },
    /// Hasta [`ContenidoSaliente::MAX_BOTONES`] botones de respuesta.
    Botones {
        cuerpo: String,
        botones: Vec<Eleccion>,
    },
    /// Hasta [`ContenidoSaliente::MAX_FILAS_LISTA`] filas.
    Lista {
        cuerpo: String,
        boton: String,
        filas: Vec<Eleccion>,
    },
    /// Plantilla aprobada por Meta: lo único que se puede enviar fuera de la ventana de 24 h
    /// desde el último mensaje del usuario (avisos de tickets, urgencias).
    Plantilla {
        nombre: String,
        idioma: String,
        parametros: Vec<String>,
        /// `payload` de los botones de respuesta rápida de la plantilla, en orden (p. ej. el
        /// «Confirmar» de `confirmar_asistencia`). Vacío si la plantilla no tiene botones.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        botones: Vec<String>,
    },
    /// Imagen que Meta descarga de un enlace HTTPS (p. ej. el QR de una visita, con un enlace
    /// firmado de vida corta). Solo dentro de la ventana de 24 h.
    Imagen {
        url: String,
        leyenda: Option<String>,
    },
}

impl ContenidoSaliente {
    /// Límites de WhatsApp Cloud API para mensajes interactivos.
    pub const MAX_BOTONES: usize = 3;
    /// Filas máximas de una lista de WhatsApp.
    pub const MAX_FILAS_LISTA: usize = 10;

    /// Elige botones o lista según cuántas opciones haya.
    pub fn elecciones(cuerpo: impl Into<String>, opciones: Vec<Eleccion>) -> Self {
        let cuerpo = cuerpo.into();
        if opciones.len() <= Self::MAX_BOTONES {
            Self::Botones {
                cuerpo,
                botones: opciones,
            }
        } else {
            Self::Lista {
                cuerpo,
                boton: "Ver opciones".into(),
                filas: opciones,
            }
        }
    }
}

/// Mensaje a enviar: línea de origen, teléfono de destino y contenido.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MensajeSaliente {
    pub linea: String,
    pub telefono: String,
    pub contenido: ContenidoSaliente,
}

/// Archivo multimedia descargado del proveedor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaDescargada {
    pub contenido: Vec<u8>,
    /// Tipo MIME que declara el proveedor (el tipo real se valida por contenido).
    pub mime: Option<String>,
}

/// Envío de mensajes y descarga de multimedia del proveedor de WhatsApp.
pub trait Mensajeria: Send + Sync {
    /// Envía el mensaje y devuelve el `wamid` asignado por el proveedor.
    fn enviar(&self, mensaje: &MensajeSaliente) -> impl Future<Output = AppResult<String>> + Send;

    /// Descarga un archivo recibido (`media_id` de [`ContenidoEntrante::Imagen`]).
    fn descargar_media(
        &self,
        media_id: &str,
    ) -> impl Future<Output = AppResult<MediaDescargada>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opciones(n: usize) -> Vec<Eleccion> {
        (0..n)
            .map(|i| Eleccion::new(i.to_string(), format!("Opción {i}")))
            .collect()
    }

    #[test]
    fn hasta_tres_opciones_usa_botones_si_no_lista() {
        assert!(matches!(
            ContenidoSaliente::elecciones("¿Aprueba?", opciones(3)),
            ContenidoSaliente::Botones { .. }
        ));
        assert!(matches!(
            ContenidoSaliente::elecciones("Elija", opciones(4)),
            ContenidoSaliente::Lista { .. }
        ));
    }
}
