//! Adaptador de WhatsApp Cloud API (Meta).
//!
//! - [`firma`]: verificación de `X-Hub-Signature-256` (HMAC-SHA256 con el *app secret*).
//! - [`webhook`]: tipos del payload de Meta y normalización a
//!   [`application::puertos::mensajeria::MensajeEntrante`].
//!
//! - [`cliente`]: envío (texto, botones, listas, plantillas) y descarga de archivos; implementa
//!   [`application::puertos::Mensajeria`]. El ritmo de 80 mensajes/s por línea lo controla el
//!   worker del gateway.
//!
//! Cambiar de BSP = otro crate que implemente los mismos puertos; el dominio no se toca.

pub mod cliente;
pub mod firma;
pub mod webhook;

pub use cliente::ClienteWhatsapp;
