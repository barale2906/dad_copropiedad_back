//! Sondas de salud para Docker, balanceadores y monitoreo.

use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::Json,
    openapi::ETIQUETA_SALUD,
    state::AppState,
};

/// El proceso está vivo.
#[derive(Debug, Serialize, ToSchema)]
pub struct EstadoVivo {
    #[schema(example = "ok")]
    pub status: &'static str,
    /// Versión del binario desplegado.
    #[schema(example = "0.1.0")]
    pub version: &'static str,
}

/// El servicio puede atender tráfico.
#[derive(Debug, Serialize, ToSchema)]
pub struct EstadoListo {
    #[schema(example = "ok")]
    pub status: &'static str,
    #[schema(example = "ok")]
    pub database: &'static str,
}

/// Liveness: el proceso está vivo. No toca dependencias externas.
#[utoipa::path(
    get,
    path = "/health",
    tag = ETIQUETA_SALUD,
    responses((status = OK, description = "Proceso vivo", body = EstadoVivo))
)]
pub async fn liveness() -> Json<EstadoVivo> {
    Json(EstadoVivo {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Readiness: el servicio puede atender tráfico (PostgreSQL responde).
#[utoipa::path(
    get,
    path = "/ready",
    tag = ETIQUETA_SALUD,
    responses(
        (status = OK, description = "Listo para atender", body = EstadoListo),
        (status = SERVICE_UNAVAILABLE, description = "Una dependencia no responde (`no_disponible`)",
         body = ErrorRespuesta),
    )
)]
pub async fn readiness(State(state): State<AppState>) -> ApiResult<Json<EstadoListo>> {
    infra_postgres::ping(&state.db).await.map_err(|e| {
        tracing::warn!(error = %e, "readiness: PostgreSQL no responde");
        ApiError::NoDisponible("base de datos")
    })?;

    Ok(Json(EstadoListo {
        status: "ok",
        database: "ok",
    }))
}
