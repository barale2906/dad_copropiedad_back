//! Error HTTP unificado: convierte errores de aplicación/dominio en respuestas JSON.

use application::AppError;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use domain::DominioError;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    App(#[from] AppError),

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

impl ApiError {
    fn status_y_codigo(&self) -> (StatusCode, &'static str) {
        match self {
            Self::App(AppError::Dominio(d)) => match d {
                DominioError::TransicionInvalida { .. }
                | DominioError::PreguntaNoAbierta(_)
                | DominioError::ReunionNoInstalada(_) => (StatusCode::CONFLICT, "conflicto_estado"),
                DominioError::OpcionInvalida(_)
                | DominioError::CoeficienteInvalido(_)
                | DominioError::ClaveIdempotenciaInvalida(_)
                | DominioError::ValorDesconocido { .. } => {
                    (StatusCode::UNPROCESSABLE_ENTITY, "validacion")
                }
            },
            Self::App(AppError::NoEncontrado(_)) => (StatusCode::NOT_FOUND, "no_encontrado"),
            Self::App(AppError::BienPrivadoYaVoto) => (StatusCode::CONFLICT, "ya_voto"),
            Self::App(AppError::NoInscrito) => (StatusCode::FORBIDDEN, "no_inscrito"),
            Self::App(AppError::Infraestructura(_)) | Self::Interno(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "interno")
            }
            Self::NoDisponible(_) => (StatusCode::SERVICE_UNAVAILABLE, "no_disponible"),
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
        (
            status,
            Json(json!({ "error": { "codigo": codigo, "mensaje": mensaje } })),
        )
            .into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
