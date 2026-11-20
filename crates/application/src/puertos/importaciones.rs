//! Importaciones masivas vistas desde cualquier módulo (no solo la PH): re-exporta el puerto que
//! nació con la vertical PH para que los módulos genéricos (usuarios, nómina) no dependan de
//! `puertos::copropiedades`.

pub use super::copropiedades::{Importacion, RepositorioImportaciones, ResumenImportacion};
