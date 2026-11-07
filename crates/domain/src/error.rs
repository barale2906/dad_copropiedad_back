use crate::{EstadoPregunta, EstadoReunion, OpcionId};

/// Errores de reglas de negocio. El mapeo a HTTP vive en `apps/api/src/error.rs`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DominioError {
    #[error("transición de {entidad} no permitida: {desde} -> {hacia}")]
    TransicionInvalida {
        entidad: &'static str,
        desde: &'static str,
        hacia: &'static str,
    },

    #[error("la pregunta no está abierta para recibir votos (estado actual: {0})")]
    PreguntaNoAbierta(EstadoPregunta),

    #[error("la reunión no está instalada (estado actual: {0})")]
    ReunionNoInstalada(EstadoReunion),

    #[error("la opción {0} no pertenece a la pregunta")]
    OpcionInvalida(OpcionId),

    #[error("coeficiente inválido: {0}")]
    CoeficienteInvalido(String),

    #[error("clave de idempotencia inválida: {0}")]
    ClaveIdempotenciaInvalida(&'static str),

    /// No incluye el valor recibido: un correo es dato personal y no debe llegar a los logs.
    #[error("dirección de correo inválida")]
    CorreoInvalido,

    #[error("valor desconocido `{valor}` para {tipo}")]
    ValorDesconocido { tipo: &'static str, valor: String },
}
