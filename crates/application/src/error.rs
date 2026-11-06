use domain::DominioError;

/// Error de los casos de uso. El mapeo a HTTP vive en `apps/api/src/error.rs`.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Dominio(#[from] DominioError),

    #[error("{0} no encontrada")]
    NoEncontrado(&'static str),

    #[error("el inmueble ya votó en esta pregunta")]
    InmuebleYaVoto,

    #[error("la persona no está inscrita como representante del inmueble en esta reunión")]
    NoInscrito,

    /// Fallo de un adaptador (BD, red, proveedor). El detalle va a logs, no al cliente.
    #[error("error de infraestructura: {0}")]
    Infraestructura(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl AppError {
    /// Atajo para envolver errores de adaptadores.
    pub fn infra(e: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Infraestructura(Box::new(e))
    }
}

pub type AppResult<T> = Result<T, AppError>;
