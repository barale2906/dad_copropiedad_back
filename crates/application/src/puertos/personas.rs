//! Personas de la entidad, vistas desde cualquier módulo (no solo la PH): el tercero contable
//! es la persona del tenant (ADR 0017). Re-exporta el puerto que nació con la vertical PH para
//! que los módulos genéricos no dependan de `puertos::copropiedades`.

pub use super::copropiedades::{Persona, RepositorioPersonas};
