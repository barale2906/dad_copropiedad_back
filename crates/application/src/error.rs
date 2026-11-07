use domain::{DominioError, Modulo, Permiso};

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

    /// Falta la sesión o no es válida (token vencido, revocado o inexistente).
    #[error("se requiere iniciar sesión")]
    NoAutenticado,

    /// Usuario/correo o contraseña incorrectos, o cuenta bloqueada. Un solo mensaje para no
    /// revelar si la cuenta existe.
    #[error("usuario o contraseña incorrectos")]
    CredencialesInvalidas,

    /// Demasiados intentos fallidos: la cuenta queda bloqueada unos minutos.
    #[error("demasiados intentos fallidos; intente de nuevo más tarde")]
    DemasiadosIntentos,

    #[error("no tiene el permiso `{0}`")]
    SinPermiso(Permiso),

    /// La operación exige una sesión con segundo factor verificado.
    #[error("esta operación exige el segundo factor de autenticación")]
    SegundoFactorRequerido,

    /// El recurso ya existe o la operación choca con el estado actual (usuario repetido,
    /// último administrador…).
    #[error("{0}")]
    Conflicto(String),

    /// La copropiedad no tiene contratado el módulo.
    #[error("el módulo `{0}` no está habilitado para esta copropiedad")]
    ModuloNoHabilitado(Modulo),

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
