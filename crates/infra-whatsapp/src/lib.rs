//! Adaptador de WhatsApp Cloud API (Meta).
//!
//! - [`firma`]: verificación de `X-Hub-Signature-256` (HMAC-SHA256 con el *app secret*).
//! - [`webhook`]: tipos del payload de Meta y normalización a
//!   [`application::puertos::mensajeria::MensajeEntrante`].
//!
//! Pendiente (porción vertical): cliente de envío que implemente
//! [`application::puertos::Mensajeria`] y limitador de 80 mps por línea (`governor`).
//! Cambiar de BSP = otro crate que implemente los mismos puertos; el dominio no se toca.

pub mod firma;
pub mod webhook;
