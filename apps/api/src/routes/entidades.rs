//! Rutas de una entidad cualquiera: `/api/v1/entidades/{entidad_id}/…` (ADR 0016). Sirven a
//! cualquier empresa —con o sin vertical— con el extractor [`crate::sesion::EnEntidad`]; en una
//! copropiedad, el id es el mismo de `/copropiedades/{copropiedad_id}`.

use application::{casos_uso::modulos::capacidades as uc_capacidades, puertos::RelojSistema};
use axum::extract::State;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::ficha::CapacidadesRespuesta;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::Json,
    openapi::ETIQUETA_CONTABILIDAD,
    sesion::EnEntidad,
    state::AppState,
};

/// Router de una entidad (se monta en `/api/v1/entidades/{entidad_id}`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(capacidades_entidad))
        .nest("/contabilidad", super::contabilidad::rutas())
}

/// Permisos de la cuenta en la entidad y módulos habilitados (para cualquier entidad).
#[utoipa::path(
    get,
    path = "/capacidades",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Permisos y módulos", body = CapacidadesRespuesta),
        (status = NOT_FOUND, description = "La cuenta no tiene roles en la entidad (`no_encontrado`)", body = ErrorRespuesta),
    )
)]
pub async fn capacidades_entidad(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<CapacidadesRespuesta>> {
    let c = uc_capacidades(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(c.into()))
}
