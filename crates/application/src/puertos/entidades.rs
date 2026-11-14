//! Ficha genérica de una entidad (ADR 0016): lo que tiene cualquier empresa —nombre, NIT,
//! ubicación, contacto, zona horaria y estado—, guardado en `entidades`. La PH lo extiende en su
//! propia ficha ([`super::copropiedades::FichaCopropiedad`]) con los datos de la Ley 675.

use std::future::Future;

use domain::{CodigoDane, DireccionCorreo, EstadoCopropiedad, Nit, Telefono};
use rust_decimal::Decimal;

use crate::AppResult;

/// Datos generales de la entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FichaEntidad {
    pub nombre: String,
    pub razon_social: Option<String>,
    pub nit: Option<Nit>,
    pub direccion: Option<String>,
    pub municipio_dane: Option<CodigoDane>,
    pub departamento: Option<String>,
    pub barrio: Option<String>,
    pub latitud: Option<Decimal>,
    pub longitud: Option<Decimal>,
    pub telefono: Option<Telefono>,
    pub correo: Option<DireccionCorreo>,
    pub sitio_web: Option<String>,
    /// Zona IANA en que se evalúan los horarios (`America/Bogota`).
    pub zona_horaria: String,
    /// Lo cambia solo la plataforma (activar, suspender, retirar).
    pub estado: EstadoCopropiedad,
}

/// Lectura y escritura de la ficha de la entidad de la transacción.
pub trait RepositorioEntidad: Send {
    /// Ficha de la entidad de la transacción.
    fn ficha_entidad(&mut self) -> impl Future<Output = AppResult<Option<FichaEntidad>>> + Send;

    /// Reemplaza los datos generales (no el estado).
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el NIT ya lo usa otra entidad.
    fn guardar_ficha_entidad(
        &mut self,
        ficha: &FichaEntidad,
    ) -> impl Future<Output = AppResult<()>> + Send;
}
