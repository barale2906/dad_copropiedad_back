//! Puertos (traits) que implementan los adaptadores `infra-*`.
//!
//! Los métodos asíncronos devuelven `impl Future + Send` para poder usarse desde Tokio sin
//! `async-trait`. En la implementación basta con escribir `async fn`.

pub mod actas;
pub mod cola_salida;
pub mod correo;
pub mod mensajeria;
pub mod reloj;
pub mod repositorios;
pub mod sesiones;
pub mod unidad_trabajo;

pub use actas::GeneradorActas;
pub use cola_salida::{ColaSalida, LoteSalida, MensajePendiente};
pub use correo::EnvioCorreo;
pub use mensajeria::Mensajeria;
pub use reloj::{Reloj, RelojSistema};
pub use repositorios::{
    DirectorioReuniones, RepositorioInscripciones, RepositorioPreguntas, RepositorioVotos,
};
pub use sesiones::SesionesConversacion;
pub use unidad_trabajo::{RegistroAuditoria, RegistroOutbox, Transaccion, UnidadDeTrabajo};
