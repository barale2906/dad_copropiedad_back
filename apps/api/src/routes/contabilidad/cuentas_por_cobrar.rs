//! Cuentas por cobrar de las facturas de venta (contabilidad): cartera de clientes por edades.
//! El recaudo va por los recibos de caja de la tesorería.

use application::{casos_uso::contabilidad::cuentas_por_cobrar, puertos::RelojSistema};
use axum::extract::State;
use chrono::NaiveDate;
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::dto_tesoreria::*;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Query},
    openapi::ETIQUETA_TESORERIA,
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas de cuentas por cobrar.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(cartera))
}

/// Fecha de corte.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CorteParametros {
    pub corte: NaiveDate,
}

/// Cartera de clientes por edades a la fecha de corte (saldo actual de cada factura en el tramo
/// de sus días vencidos).
#[utoipa::path(
    get,
    path = "/cuentas-por-cobrar/edades",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), CorteParametros),
    responses(
        (status = OK, description = "Cartera", body = Vec<FilaCarteraRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn cartera(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<CorteParametros>,
) -> ApiResult<Json<Vec<FilaCarteraRespuesta>>> {
    let v = cuentas_por_cobrar::cartera_por_edades(&state.unidad, &RelojSistema, &en.ctx, q.corte)
        .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}
