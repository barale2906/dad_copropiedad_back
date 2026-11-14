//! Rutas de la facturación, que se vende sola (plan 18, ADR 0022):
//! `/api/v1/entidades/{entidad_id}/facturacion/…` con el extractor [`crate::sesion::EnEntidad`].
//! Sirven con el módulo de facturación electrónica o con la contabilidad: catálogo, facturas de
//! venta, compras para el documento soporte, documentos electrónicos, facturas recibidas,
//! exportaciones para el contador y contabilización de lo facturado antes de tener contabilidad.

mod documentos;
mod exportaciones;
mod habilitacion;
mod items;
mod ventas;

use application::{casos_uso::contabilidad::cuentas_por_pagar as uc_cxp, puertos::RelojSistema};
use axum::{extract::State, http::StatusCode};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::routes::contabilidad::{cuentas_por_pagar::nueva_factura_proveedor, dto_tesoreria::*};
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::Json,
    openapi::ETIQUETA_FACTURACION,
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas de facturación de una entidad (se montan en `/entidades/{entidad_id}/facturacion`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(items::rutas())
        .merge(ventas::rutas())
        .merge(documentos::rutas())
        .merge(exportaciones::rutas())
        .merge(habilitacion::rutas())
        .routes(routes!(registrar_compra))
}

/// Registra una compra a un proveedor para emitir su documento soporte. Sin contabilidad va sin
/// cuentas, centros de costo ni retenciones; con contabilidad es la misma factura de proveedor
/// de las cuentas por pagar.
#[utoipa::path(
    post,
    path = "/compras",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = uuid::Uuid, Path, description = "Entidad")),
    request_body = NuevaFacturaProveedorPeticion,
    responses(
        (status = CREATED, description = "Compra registrada", body = FacturaProveedorRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.facturas.gestionar` (o, con contabilidad, `tesoreria.facturas_proveedor.gestionar`) o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Tercero inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Compra repetida del proveedor (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Dato inválido; sin contabilidad, cuentas o retenciones (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_compra(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevaFacturaProveedorPeticion>,
) -> ApiResult<(StatusCode, Json<FacturaProveedorRespuesta>)> {
    let f = uc_cxp::registrar_factura_proveedor(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        nueva_factura_proveedor(p)?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(f.into())))
}
