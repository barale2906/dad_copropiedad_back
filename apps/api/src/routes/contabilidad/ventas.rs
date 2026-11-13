//! Facturas de venta y cartera de clientes por edades (plan 13 fase 2b).

use application::{
    casos_uso::contabilidad::ventas::{self as uc, NuevaFacturaVenta, NuevaLineaVenta},
    puertos::{RelojSistema, contabilidad::Origen, tesoreria::FiltroFacturas},
};
use axum::{extract::State, http::StatusCode};
use chrono::NaiveDate;
use domain::{FacturaVentaId, PersonaId, contabilidad::tesoreria::EstadoFacturaVenta};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::codigo;
use super::dto_tesoreria::*;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_TESORERIA,
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas de facturas de venta.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_facturas, crear_factura))
        .routes(routes!(ver_factura, eliminar_factura))
        .routes(routes!(emitir_factura))
        .routes(routes!(anular_factura))
        .routes(routes!(cartera))
}

/// Filtro de facturas de venta.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroFacturasVentaParametros {
    pub persona_id: Option<Uuid>,
    #[param(value_type = Option<crate::esquemas::EstadoFacturaVenta>)]
    pub estado: Option<EstadoFacturaVenta>,
    pub desde: Option<NaiveDate>,
    pub hasta: Option<NaiveDate>,
}

/// Facturas de venta por vencimiento (a lo sumo 500, sin líneas).
#[utoipa::path(
    get,
    path = "/ventas/facturas",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroFacturasVentaParametros),
    responses(
        (status = OK, description = "Facturas", body = Vec<FacturaVentaRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_facturas(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroFacturasVentaParametros>,
) -> ApiResult<Json<Vec<FacturaVentaRespuesta>>> {
    let v = uc::facturas_venta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FiltroFacturas {
            persona_id: f.persona_id.map(PersonaId::from_uuid),
            estado: f.estado.map(|e| e.as_str().to_owned()),
            desde: f.desde,
            hasta: f.hasta,
        },
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea una factura de venta en borrador (el IVA de cada línea lo elige quien factura).
#[utoipa::path(
    post,
    path = "/ventas/facturas",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevaFacturaVentaPeticion,
    responses(
        (status = CREATED, description = "Factura en borrador", body = FacturaVentaRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.facturas_venta.gestionar` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Cliente inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "El documento origen ya tiene factura (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Línea inválida (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_factura(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevaFacturaVentaPeticion>,
) -> ApiResult<(StatusCode, Json<FacturaVentaRespuesta>)> {
    let mut lineas = Vec::with_capacity(p.lineas.len());
    for l in p.lineas {
        lineas.push(NuevaLineaVenta {
            descripcion: l.descripcion,
            cuenta: codigo(&l.cuenta)?,
            centro_costo: l.centro_costo,
            cantidad: l.cantidad,
            valor_unitario: l.valor_unitario,
            tarifa_iva: l.tarifa_iva,
        });
    }
    let f = uc::crear_factura_venta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevaFacturaVenta {
            persona: PersonaId::from_uuid(p.persona_id),
            fecha: p.fecha,
            vencimiento: p.vencimiento,
            descripcion: p.descripcion,
            origen: p.origen.map(|o| Origen {
                tipo: o.tipo,
                id: o.id,
            }),
            lineas,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(f.into())))
}

/// Factura de venta con sus líneas.
#[utoipa::path(
    get,
    path = "/ventas/facturas/{factura_id}",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura")),
    responses(
        (status = OK, description = "Factura", body = FacturaVentaRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn ver_factura(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<FacturaVentaRespuesta>> {
    let f = uc::factura_venta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FacturaVentaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(f.into()))
}

/// Borra una factura en borrador.
#[utoipa::path(
    delete,
    path = "/ventas/facturas/{factura_id}",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura")),
    responses(
        (status = NO_CONTENT, description = "Borrada"),
        (status = FORBIDDEN, description = "Sin `tesoreria.facturas_venta.gestionar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya se emitió (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn eliminar_factura(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    uc::eliminar_factura_venta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FacturaVentaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Emite la factura: la contabiliza y le asigna el número de su comprobante `FV`.
#[utoipa::path(
    post,
    path = "/ventas/facturas/{factura_id}/emision",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura")),
    responses(
        (status = OK, description = "Factura emitida", body = FacturaVentaRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.facturas_venta.gestionar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está en borrador (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin regla, periodo cerrado o descuadre (`regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn emitir_factura(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<FacturaVentaRespuesta>> {
    let f = uc::emitir_factura_venta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FacturaVentaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(f.into()))
}

/// Anula una factura emitida sin recaudos (revierte su comprobante).
#[utoipa::path(
    post,
    path = "/ventas/facturas/{factura_id}/anulacion",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura")),
    request_body = AnulacionPeticion,
    responses(
        (status = OK, description = "Factura anulada", body = FacturaVentaRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.facturas_venta.gestionar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Tiene recaudos o no se emitió (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn anular_factura(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AnulacionPeticion>,
) -> ApiResult<Json<FacturaVentaRespuesta>> {
    let f = uc::anular_factura_venta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FacturaVentaId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(Json(f.into()))
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
    path = "/ventas/cartera",
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
    let v = uc::cartera_por_edades(&state.unidad, &RelojSistema, &en.ctx, q.corte).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}
