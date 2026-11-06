use domain::DominioError;

/// Error de los casos de uso. El mapeo a HTTP vive en `apps/api/src/error.rs`.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Dominio(#[from] DominioError),

    /// Datos de entrada que no cumplen las reglas del caso de uso (p. ej. límite de página).
    #[error("{0}")]
    Validacion(String),

    #[error("{0} no encontrada")]
    NoEncontrado(&'static str),

    #[error("el bien privado ya votó en esta pregunta")]
    BienPrivadoYaVoto,

    #[error("la persona no está inscrita como representante del bien privado en esta reunión")]
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
