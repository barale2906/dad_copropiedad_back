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
    /// Imagen, audio, ubicación… se ignoran con una respuesta de ayuda.
    NoSoportado {
        tipo_original: String,
    },
}

/// Elemento seleccionable (botón o fila de lista).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Eleccion {
    pub id: String,
    pub titulo: String,
}

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
}

impl ContenidoSaliente {
    /// Límites de WhatsApp Cloud API para mensajes interactivos.
    pub const MAX_BOTONES: usize = 3;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MensajeSaliente {
    pub linea: String,
    pub telefono: String,
    pub contenido: ContenidoSaliente,
}

pub trait Mensajeria: Send + Sync {
    /// Envía el mensaje y devuelve el `wamid` asignado por el proveedor.
    fn enviar(&self, mensaje: &MensajeSaliente) -> impl Future<Output = AppResult<String>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opciones(n: usize) -> Vec<Eleccion> {
        (0..n)
            .map(|i| Eleccion {
                id: i.to_string(),
                titulo: format!("Opción {i}"),
            })
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
