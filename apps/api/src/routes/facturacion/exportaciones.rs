//! Exportaciones para el contador externo y contabilización de lo pendiente:
//! `/entidades/{entidad_id}/facturacion/{libro-ventas,libro-compras,paquete,contabilizacion}`.

use application::{
    casos_uso::facturacion::{
        contabilizacion::{self as uc_contabilizar, InformeContabilizacion},
        exportaciones::{self as uc, Exportacion},
    },
    puertos::RelojSistema,
};
use axum::{
    extract::State,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::routes::contabilidad::reportes::CABECERA_HUELLA;
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Query},
    openapi::ETIQUETA_FACTURACION,
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas de exportación y contabilización.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(libro_ventas))
        .routes(routes!(libro_compras))
        .routes(routes!(paquete))
        .routes(routes!(contabilizar))
}

/// Periodo de la exportación.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct PeriodoParametros {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
}

fn responder(e: Exportacion) -> ApiResult<Response> {
    let disposicion = format!("attachment; filename=\"{}\"", e.nombre);
    let valor = |v: &str| HeaderValue::from_str(v).map_err(|x| ApiError::Interno(x.into()));
    Ok((
        [
            (header::CONTENT_TYPE, valor(e.tipo)?),
            (header::CONTENT_DISPOSITION, valor(&disposicion)?),
            (CABECERA_HUELLA, valor(&e.huella)?),
        ],
        e.contenido,
    )
        .into_response())
}

/// Libro de ventas del periodo en CSV (número interno, número y CUFE de la DIAN, cliente y
/// valores). Queda en la auditoría con su huella (cabecera `X-Contenido-Sha256`).
#[utoipa::path(
    get,
    path = "/libro-ventas",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), PeriodoParametros),
    responses(
        (status = OK, description = "CSV", content_type = "text/csv", body = String),
        (status = FORBIDDEN, description = "Sin `facturacion.exportar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Periodo inválido o demasiado grande (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn libro_ventas(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<PeriodoParametros>,
) -> ApiResult<Response> {
    responder(uc::libro_ventas(&state.unidad, &RelojSistema, &en.ctx, q.desde, q.hasta).await?)
}

/// Libro de compras del periodo en CSV (facturas electrónicas recibidas).
#[utoipa::path(
    get,
    path = "/libro-compras",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), PeriodoParametros),
    responses(
        (status = OK, description = "CSV", content_type = "text/csv", body = String),
        (status = FORBIDDEN, description = "Sin `facturacion.exportar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Periodo inválido o demasiado grande (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn libro_compras(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<PeriodoParametros>,
) -> ApiResult<Response> {
    responder(uc::libro_compras(&state.unidad, &RelojSistema, &en.ctx, q.desde, q.hasta).await?)
}

/// ZIP para conservar los documentos electrónicos del periodo: en `emitidos/` el XML firmado, el
/// `AttachedDocument` y el PDF de cada documento transmitido; en `recibidos/` el XML de cada
/// factura recibida; en `eventos/` los eventos RADIAN firmados; e `indice.csv` con el documento,
/// el CUFE, el estado, la fecha y el SHA-256 de cada archivo.
#[utoipa::path(
    get,
    path = "/paquete",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), PeriodoParametros),
    responses(
        (status = OK, description = "ZIP", content_type = "application/zip", body = Vec<u8>),
        (status = FORBIDDEN, description = "Sin `facturacion.exportar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Periodo inválido o demasiado grande (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn paquete(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<PeriodoParametros>,
) -> ApiResult<Response> {
    responder(
        uc::paquete_documentos(
            &state.unidad,
            (
                state.archivos.as_ref(),
                &state.actas,
                &state.empaquetador,
                state.pie_facturacion.as_deref(),
            ),
            &RelojSistema,
            &en.ctx,
            q.desde,
            q.hasta,
        )
        .await?,
    )
}

/// Factura que no se pudo contabilizar.
#[derive(Debug, Serialize, ToSchema)]
pub struct FacturaFallida {
    pub factura_id: Uuid,
    pub motivo: String,
}

/// Resultado de contabilizar lo pendiente.
#[derive(Debug, Serialize, ToSchema)]
pub struct InformeContabilizacionRespuesta {
    pub contabilizadas: Vec<Uuid>,
    /// Contabilizadas que se habían pagado a mano: registre su recaudo en la contabilidad.
    pub pagadas_a_mano: Vec<Uuid>,
    pub fallidas: Vec<FacturaFallida>,
}

impl From<InformeContabilizacion> for InformeContabilizacionRespuesta {
    fn from(i: InformeContabilizacion) -> Self {
        Self {
            contabilizadas: i.contabilizadas.iter().map(|f| f.as_uuid()).collect(),
            pagadas_a_mano: i.pagadas_a_mano.iter().map(|f| f.as_uuid()).collect(),
            fallidas: i
                .fallidas
                .into_iter()
                .map(|(f, motivo)| FacturaFallida {
                    factura_id: f.as_uuid(),
                    motivo,
                })
                .collect(),
        }
    }
}

/// Contabiliza las facturas emitidas antes de tener contabilidad (exige el módulo de
/// contabilidad). Cada una va en su transacción; repetir no duplica nada.
#[utoipa::path(
    post,
    path = "/contabilizacion",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = PeriodoParametros,
    responses(
        (status = OK, description = "Informe", body = InformeContabilizacionRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.comprobantes.contabilizar` o sin el módulo de contabilidad", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Periodo inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn contabilizar(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<PeriodoParametros>,
) -> ApiResult<Json<InformeContabilizacionRespuesta>> {
    let i = uc_contabilizar::contabilizar_pendientes(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        p.desde,
        p.hasta,
    )
    .await?;
    Ok(Json(i.into()))
}
