//! Casos de uso. Cada uno es genérico sobre los puertos que necesita.

pub mod emitir_voto;
pub mod resolver_reunion;

pub use emitir_voto::{EmitirVoto, SolicitudVoto};
pub use resolver_reunion::{Resolucion, resolver_reunion};
