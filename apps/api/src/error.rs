//! Error HTTP unificado: único lugar donde los errores de aplicación y dominio se traducen a
//! códigos HTTP (AGENTS.md §5).
//!
//! Toda respuesta de error tiene la forma [`ErrorRespuesta`]:
//! `{ "error": { "codigo": "...", "mensaje": "..." } }`. El `codigo` es estable y forma parte
//! del contrato con los clientes (web y apps móviles); el `mensaje` es legible y puede cambiar.

use application::AppError;
use axum::{
    Json,
    extract::rejection::{JsonRejection, PathRejection, QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use domain::DominioError;
use serde::Serialize;
use utoipa::ToSchema;

/// Catálogo de códigos de error de la API. **Contrato**: no renombrar ni reutilizar un código
/// con otro significado; agregar uno nuevo es un cambio compatible.
///
/// | Código | HTTP | Significado |
/// |---|---|---|
/// | `peticion_invalida` | 400 | Petición mal formada (JSON inválido, parámetro con tipo incorrecto). |
/// | `no_encontrado` | 404 | El recurso o la ruta no existen (o no pertenecen a la copropiedad). |
/// | `conflicto_estado` | 409 | La operación no es válida en el estado actual del recurso. |
/// | `ya_voto` | 409 | El bien privado ya votó en esta pregunta. |
/// | `no_inscrito` | 403 | La persona no representa al bien privado en la reunión. |
/// | `validacion` | 422 | Datos bien formados que violan una regla de negocio. |
/// | `interno` | 500 | Error inesperado; el detalle queda en logs con el `x-request-id`. |
/// | `no_disponible` | 503 | Una dependencia (p. ej. la base de datos) no está disponible. |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodigoError {
    /// 400 · La petición está mal formada (JSON inválido, parámetro con tipo incorrecto).
    PeticionInvalida,
    /// 404 · El recurso o la ruta no existen (o no pertenecen a la copropiedad).
    NoEncontrado,
    /// 409 · La operación no es válida en el estado actual del recurso.
    ConflictoEstado,
    /// 409 · El bien privado ya votó en esta pregunta.
    YaVoto,
    /// 403 · La persona no está inscrita como representante del bien privado en la reunión.
    NoInscrito,
    /// 422 · Los datos son sintácticamente correctos pero violan una regla de negocio.
    Validacion,
    /// 500 · Error inesperado; el detalle queda en los logs con el `x-request-id`.
    Interno,
    /// 503 · Una dependencia (p. ej. la base de datos) no está disponible.
    NoDisponible,
}

/// Cuerpo de toda respuesta de error.
#[derive(Debug, Serialize, ToSchema)]
#[schema(example = json!({ "error": { "codigo": "no_encontrado", "mensaje": "ruta no encontrada" } }))]
pub struct ErrorRespuesta {
    pub error: DetalleError,
}

/// Código estable y mensaje legible del error.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleError {
    pub codigo: CodigoError,
    /// Mensaje para mostrar o registrar; nunca incluye detalles internos.
    pub mensaje: String,
}

/// Errores que un handler puede devolver; se convierten en [`ErrorRespuesta`].
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    App(#[from] AppError),

    /// Petición mal formada: cuerpo, parámetros de ruta o de consulta inválidos.
    #[error("petición inválida: {0}")]
    PeticionInvalida(String),

    #[error("ruta no encontrada")]
    RutaNoEncontrada,

    #[error("servicio no disponible: {0}")]
    NoDisponible(&'static str),

    /// Errores inesperados. El detalle se registra en logs y **no** se expone al cliente.
    #[error(transparent)]
    Interno(#[from] anyhow::Error),
}

impl From<DominioError> for ApiError {
    fn from(e: DominioError) -> Self {
        Self::App(AppError::Dominio(e))
    }
}

impl From<JsonRejection> for ApiError {
    fn from(e: JsonRejection) -> Self {
        Self::PeticionInvalida(e.body_text())
    }
}

impl From<QueryRejection> for ApiError {
    fn from(e: QueryRejection) -> Self {
        Self::PeticionInvalida(e.body_text())
    }
}

impl From<PathRejection> for ApiError {
    fn from(e: PathRejection) -> Self {
        Self::PeticionInvalida(e.body_text())
    }
}

impl ApiError {
    /// Estado HTTP y código estable de cada error.
    pub fn status_y_codigo(&self) -> (StatusCode, CodigoError) {
        use CodigoError as C;
        match self {
            Self::App(AppError::Dominio(d)) => match d {
                DominioError::TransicionInvalida { .. }
                | DominioError::PreguntaNoAbierta(_)
                | DominioError::ReunionNoInstalada(_) => (StatusCode::CONFLICT, C::ConflictoEstado),
                DominioError::OpcionInvalida(_)
                | DominioError::CoeficienteInvalido(_)
                | DominioError::ClaveIdempotenciaInvalida(_)
                | DominioError::CorreoInvalido
                | DominioError::ValorDesconocido { .. } => {
                    (StatusCode::UNPROCESSABLE_ENTITY, C::Validacion)
                }
            },
            Self::App(AppError::Validacion(_)) => (StatusCode::UNPROCESSABLE_ENTITY, C::Validacion),
            Self::App(AppError::NoEncontrado(_)) | Self::RutaNoEncontrada => {
                (StatusCode::NOT_FOUND, C::NoEncontrado)
            }
            Self::App(AppError::BienPrivadoYaVoto) => (StatusCode::CONFLICT, C::YaVoto),
            Self::App(AppError::NoInscrito) => (StatusCode::FORBIDDEN, C::NoInscrito),
            Self::PeticionInvalida(_) => (StatusCode::BAD_REQUEST, C::PeticionInvalida),
            Self::App(AppError::Infraestructura(_)) | Self::Interno(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, C::Interno)
            }
            Self::NoDisponible(_) => (StatusCode::SERVICE_UNAVAILABLE, C::NoDisponible),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, codigo) = self.status_y_codigo();
        let mensaje = if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = ?self, "error interno");
            "error interno del servidor".to_owned()
        } else {
            self.to_string()
        };
        let cuerpo = ErrorRespuesta {
            error: DetalleError { codigo, mensaje },
        };
        (status, Json(cuerpo)).into_response()
    }
}

/// Resultado de los handlers.
pub type ApiResult<T> = Result<T, ApiError>;

/// Respuesta para rutas inexistentes: mismo formato que cualquier otro error.
pub async fn ruta_no_encontrada() -> ApiError {
    ApiError::RutaNoEncontrada
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codigos_se_serializan_en_snake_case() {
        let json = serde_json::to_value(CodigoError::PeticionInvalida).expect("serializa");
        assert_eq!(json, "peticion_invalida");
    }

    #[tokio::test]
    async fn error_interno_no_expone_detalle() {
        let res = ApiError::Interno(anyhow::anyhow!("contraseña=secreta")).into_response();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let cuerpo = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("cuerpo legible");
        let texto = String::from_utf8_lossy(&cuerpo);
        assert!(!texto.contains("secreta"), "{texto}");
        assert!(texto.contains("\"codigo\":\"interno\""), "{texto}");
    }

    #[test]
    fn validacion_de_aplicacion_es_422() {
        let (status, codigo) = ApiError::App(AppError::Validacion("x".into())).status_y_codigo();
        assert_eq!(
            (status, codigo),
            (StatusCode::UNPROCESSABLE_ENTITY, CodigoError::Validacion)
        );
    }
}
