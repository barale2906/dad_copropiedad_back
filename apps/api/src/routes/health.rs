//! Sondas de salud para Docker/Kubernetes/balanceadores.

use axum::{Json, extract::State};
use serde_json::{Value, json};

use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
};

/// Liveness: el proceso está vivo. No toca dependencias externas.
pub async fn liveness() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

/// Readiness: el servicio puede atender tráfico (PostgreSQL responde).
pub async fn readiness(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    infra_postgres::ping(&state.db).await.map_err(|e| {
        tracing::warn!(error = %e, "readiness: PostgreSQL no responde");
        ApiError::NoDisponible("base de datos")
    })?;

    Ok(Json(json!({ "status": "ok", "database": "ok" })))
}
