//! Lectura de planillas (CSV o XLSX): la usan las importaciones de la PH y los extractos
//! bancarios del núcleo contable, por eso vive fuera de ambos.

use domain::TipoArchivo;

use crate::AppResult;

/// Lector de planillas: convierte un CSV o XLSX en filas de texto (la primera, los
/// encabezados). Solo CPU, sin IO: por eso es síncrono.
pub trait LectorPlanillas: Send + Sync {
    /// # Errors
    ///
    /// [`crate::AppError::Validacion`] si el archivo no se puede leer como planilla.
    fn leer(&self, tipo: TipoArchivo, contenido: &[u8]) -> AppResult<Vec<Vec<String>>>;
}
