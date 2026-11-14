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
//! - [`EmpaquetadorZip`] implementa [`application::puertos::archivos::Empaquetador`] con el
//!   ZIP del motor propio (paquete de XML y PDF para el contador).
//! - [`propio`]: motor propio que genera, firma y transmite los documentos a la DIAN (plan 17).
//!
//! Ninguno guarda datos ni escribe logs con el contenido de los documentos.

mod empaquetador;
mod lector;
pub mod propio;
mod simulado;

pub use empaquetador::EmpaquetadorZip;
pub use lector::LectorUbl;
pub use propio::{LectorPkcs12, MotorPropio, ProveedorDian};
pub use simulado::ProveedorSimulado;
