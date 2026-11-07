//! Casos de uso. Cada uno es genérico sobre los puertos que necesita.

pub mod archivos;
pub mod autenticacion;
pub mod despachar_salida;
pub mod emitir_voto;
pub mod modulos;
pub mod organizaciones;
pub mod plataforma;
pub mod proteccion_datos;
pub mod resolver_reunion;
pub mod roles;

pub use archivos::{ArchivoSubido, ResumenMigracion, enlace_descarga, migrar_lote, subir_archivo};
pub use despachar_salida::{ResumenDespacho, despachar_lote};
pub use emitir_voto::{EmitirVoto, SolicitudVoto};
pub use resolver_reunion::{Resolucion, resolver_reunion};
