//! Adaptadores de facturación electrónica (plan 15, ADR 0020).
//!
//! - [`ProveedorSimulado`] implementa [`application::puertos::tributario::ProveedorFacturacion`]
//!   sin salir a la red: valida lo mínimo, calcula un CUFE determinista (SHA-384 del documento) y
//!   devuelve un XML de muestra. Sirve para desarrollo, pruebas y para operar todo el flujo
//!   mientras se elige el proveedor tecnológico (decisión D3); el adaptador real se agrega al
//!   lado sin tocar los casos de uso.
//! - [`LectorUbl`] implementa [`application::puertos::tributario::LectorFacturas`]: lee una
//!   factura UBL 2.1 de la DIAN o el `AttachedDocument` que la envuelve.
//!
//! Ninguno guarda datos ni escribe logs con el contenido de los documentos.

mod lector;
mod simulado;

pub use lector::LectorUbl;
pub use simulado::ProveedorSimulado;
