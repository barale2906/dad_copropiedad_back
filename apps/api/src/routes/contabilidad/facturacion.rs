//! Documentos electrónicos y facturas recibidas (plan 15 fases 3 y 4, módulo
//! `facturacion_electronica`). El XML de una factura recibida viaja en Base64 dentro del JSON.

use application::{
    AppError,
    casos_uso::contabilidad::facturacion as uc,
    puertos::{RelojSistema, tributario::FiltroDocumentos},
};
use axum::{
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use domain::{
    DocumentoElectronicoId, FacturaProveedorId, FacturaRecibidaId, FacturaVentaId, NombreArchivo,
    contabilidad::tributario::{
        EstadoDocumentoElectronico, EstadoFacturaRecibida, TipoDocumentoElectronico,
    },
};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto_tributario::*;
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_TRIBUTARIO,
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas de documentos electrónicos y facturas recibidas.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(emitir_factura_electronica))
        .routes(routes!(emitir_nota_credito))
        .routes(routes!(emitir_documento_soporte))
        .routes(routes!(listar_documentos))
        .routes(routes!(ver_documento))
        .routes(routes!(reintentar_documento))
        .routes(routes!(representacion_grafica))
        .routes(routes!(listar_recibidas, cargar_recibida))
        .routes(routes!(ver_recibida))
        .routes(routes!(vincular_recibida))
        .routes(routes!(rechazar_recibida))
        .routes(routes!(evento_radian))
}

// ---------------------------------------------------------------------------------------------
// Emisión
// ---------------------------------------------------------------------------------------------

/// Emite la factura electrónica de una factura de venta emitida. Pedirla otra vez devuelve el
/// mismo documento (mismo CUFE) y lo reintenta si quedó en contingencia.
#[utoipa::path(
    post,
    path = "/ventas/facturas/{factura_id}/factura-electronica",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura de venta")),
    responses(
        (status = OK, description = "Documento (validado, rechazado o en contingencia)", body = DocumentoElectronicoRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.emitir` o sin el módulo `facturacion_electronica`", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Factura inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin perfil de facturador, sin resolución vigente o factura sin emitir (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn emitir_factura_electronica(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DocumentoElectronicoRespuesta>> {
    let d = uc::emitir_factura_electronica(
        &state.unidad,
        &state.facturacion,
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        FacturaVentaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(d.into()))
}

/// Anula una factura de venta con factura electrónica validada mediante una nota crédito
/// electrónica (revierte su contabilización).
#[utoipa::path(
    post,
    path = "/ventas/facturas/{factura_id}/nota-credito",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura de venta")),
    request_body = NotaCreditoPeticion,
    responses(
        (status = OK, description = "Nota crédito", body = DocumentoElectronicoRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.emitir` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Factura inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "La factura no tiene factura electrónica validada o ya tiene pagos (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Motivo inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn emitir_nota_credito(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<NotaCreditoPeticion>,
) -> ApiResult<Json<DocumentoElectronicoRespuesta>> {
    let d = uc::emitir_nota_credito(
        &state.unidad,
        &state.facturacion,
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        FacturaVentaId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(Json(d.into()))
}

/// Emite el documento soporte de una compra a quien no está obligado a facturar.
#[utoipa::path(
    post,
    path = "/cuentas-por-pagar/facturas/{factura_id}/documento-soporte",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura de proveedor")),
    responses(
        (status = OK, description = "Documento soporte", body = DocumentoElectronicoRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.emitir` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Factura inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "El proveedor factura electrónicamente, la compra no está causada o no hay resolución (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn emitir_documento_soporte(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DocumentoElectronicoRespuesta>> {
    let d = uc::emitir_documento_soporte(
        &state.unidad,
        &state.facturacion,
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        FacturaProveedorId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(d.into()))
}

// ---------------------------------------------------------------------------------------------
// Seguimiento
// ---------------------------------------------------------------------------------------------

/// Filtro de documentos electrónicos.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroDocumentosParametros {
    #[param(value_type = Option<crate::esquemas::TipoDocumentoElectronico>)]
    pub tipo: Option<TipoDocumentoElectronico>,
    #[param(value_type = Option<crate::esquemas::EstadoDocumentoElectronico>)]
    pub estado: Option<EstadoDocumentoElectronico>,
}

/// Documentos electrónicos (los más recientes primero, a lo sumo 500).
#[utoipa::path(
    get,
    path = "/facturacion/documentos",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroDocumentosParametros),
    responses(
        (status = OK, description = "Documentos", body = Vec<DocumentoElectronicoRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_documentos(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroDocumentosParametros>,
) -> ApiResult<Json<Vec<DocumentoElectronicoRespuesta>>> {
    let v = uc::documentos(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FiltroDocumentos {
            tipo: f.tipo,
            estado: f.estado,
        },
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Documento electrónico con sus eventos.
#[utoipa::path(
    get,
    path = "/facturacion/documentos/{documento_id}",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("documento_id" = Uuid, Path, description = "Documento")),
    responses(
        (status = OK, description = "Documento", body = DocumentoDetalleRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Documento inexistente", body = ErrorRespuesta),
    )
)]
pub async fn ver_documento(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DocumentoDetalleRespuesta>> {
    let (d, eventos) = uc::documento(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        DocumentoElectronicoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(DocumentoDetalleRespuesta {
        documento: d.into(),
        eventos: eventos.into_iter().map(Into::into).collect(),
    }))
}

/// Reintenta un documento por enviar o en contingencia (o consulta uno enviado).
#[utoipa::path(
    post,
    path = "/facturacion/documentos/{documento_id}/reintento",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("documento_id" = Uuid, Path, description = "Documento")),
    responses(
        (status = OK, description = "Documento", body = DocumentoElectronicoRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.emitir` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Documento inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya está validado o rechazado (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn reintentar_documento(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DocumentoElectronicoRespuesta>> {
    let d = uc::reintentar_documento(
        &state.unidad,
        &state.facturacion,
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        DocumentoElectronicoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(d.into()))
}

/// Representación gráfica del documento en PDF (con su CUFE).
#[utoipa::path(
    get,
    path = "/facturacion/documentos/{documento_id}/pdf",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("documento_id" = Uuid, Path, description = "Documento")),
    responses(
        (status = OK, description = "PDF", content_type = "application/pdf", body = Vec<u8>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Documento inexistente", body = ErrorRespuesta),
    )
)]
pub async fn representacion_grafica(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    let pdf = uc::representacion_grafica(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        &en.ctx,
        DocumentoElectronicoId::from_uuid(ruta.1),
    )
    .await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"documento-{}.pdf\"", ruta.1),
            ),
        ],
        pdf,
    )
        .into_response())
}

// ---------------------------------------------------------------------------------------------
// Facturas recibidas
// ---------------------------------------------------------------------------------------------

/// Filtro de facturas recibidas.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroRecibidasParametros {
    #[param(value_type = Option<crate::esquemas::EstadoFacturaRecibida>)]
    pub estado: Option<EstadoFacturaRecibida>,
}

/// Facturas electrónicas recibidas (las más recientes primero).
#[utoipa::path(
    get,
    path = "/facturacion/recibidas",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroRecibidasParametros),
    responses(
        (status = OK, description = "Facturas recibidas", body = Vec<FacturaRecibidaRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_recibidas(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroRecibidasParametros>,
) -> ApiResult<Json<Vec<FacturaRecibidaRespuesta>>> {
    let v = uc::facturas_recibidas(&state.unidad, &RelojSistema, &en.ctx, f.estado).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Carga el XML de una factura electrónica recibida: lee sus datos, la asocia al proveedor con
/// ese NIT y la deja pendiente de registrar. La misma factura (CUFE o archivo) no entra dos veces.
#[utoipa::path(
    post,
    path = "/facturacion/recibidas",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = CargarFacturaPeticion,
    responses(
        (status = CREATED, description = "Factura recibida", body = FacturaRecibidaRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.recibir` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya se cargó (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es una factura electrónica legible (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn cargar_recibida(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<CargarFacturaPeticion>,
) -> ApiResult<(StatusCode, Json<FacturaRecibidaRespuesta>)> {
    let contenido = STANDARD.decode(p.archivo_base64.trim()).map_err(|_| {
        ApiError::App(AppError::Validacion(
            "`archivo_base64` no es Base64 válido".into(),
        ))
    })?;
    let nombre = NombreArchivo::new(&p.nombre_archivo).map_err(|e| ApiError::App(e.into()))?;
    let f = uc::cargar_factura_recibida(
        &state.unidad,
        state.archivos.as_ref(),
        &state.lector_facturas,
        &RelojSistema,
        &en.ctx,
        nombre,
        contenido,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(f.into())))
}

/// Factura recibida con sus eventos RADIAN.
#[utoipa::path(
    get,
    path = "/facturacion/recibidas/{recibida_id}",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("recibida_id" = Uuid, Path, description = "Factura recibida")),
    responses(
        (status = OK, description = "Factura", body = FacturaRecibidaDetalleRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Factura inexistente", body = ErrorRespuesta),
    )
)]
pub async fn ver_recibida(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<FacturaRecibidaDetalleRespuesta>> {
    let (f, eventos) = uc::factura_recibida(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FacturaRecibidaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(FacturaRecibidaDetalleRespuesta {
        factura: f.into(),
        eventos: eventos.into_iter().map(Into::into).collect(),
    }))
}

/// Vincula la factura recibida con la factura de proveedor registrada (mismo CUFE).
#[utoipa::path(
    post,
    path = "/facturacion/recibidas/{recibida_id}/vinculo",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("recibida_id" = Uuid, Path, description = "Factura recibida")),
    request_body = VincularPeticion,
    responses(
        (status = OK, description = "Factura registrada", body = FacturaRecibidaRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.recibir` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Factura inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya registrada o rechazada (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "El CUFE no coincide (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn vincular_recibida(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<VincularPeticion>,
) -> ApiResult<Json<FacturaRecibidaRespuesta>> {
    let f = uc::vincular_factura_recibida(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FacturaRecibidaId::from_uuid(ruta.1),
        FacturaProveedorId::from_uuid(p.factura_proveedor_id),
    )
    .await?;
    Ok(Json(f.into()))
}

/// Rechaza internamente una factura recibida (no la registra). Para reclamarla ante el emisor
/// se registra el evento RADIAN `reclamo`.
#[utoipa::path(
    post,
    path = "/facturacion/recibidas/{recibida_id}/rechazo",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("recibida_id" = Uuid, Path, description = "Factura recibida")),
    request_body = RechazoPeticion,
    responses(
        (status = OK, description = "Factura rechazada", body = FacturaRecibidaRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.recibir` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Factura inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya registrada o rechazada (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Motivo inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn rechazar_recibida(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<RechazoPeticion>,
) -> ApiResult<Json<FacturaRecibidaRespuesta>> {
    let f = uc::rechazar_factura_recibida(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FacturaRecibidaId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(Json(f.into()))
}

/// Registra un evento RADIAN (acuse, recibo del bien, aceptación o reclamo) por medio del
/// proveedor. Cada evento exige sus previos y no se repite.
#[utoipa::path(
    post,
    path = "/facturacion/recibidas/{recibida_id}/eventos",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("recibida_id" = Uuid, Path, description = "Factura recibida")),
    request_body = EventoRadianPeticion,
    responses(
        (status = CREATED, description = "Evento registrado", body = EventoRadianRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.recibir` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Factura inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya registrado (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Faltan los eventos previos o reclamo sin motivo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn evento_radian(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<EventoRadianPeticion>,
) -> ApiResult<(StatusCode, Json<EventoRadianRespuesta>)> {
    let e = uc::registrar_evento_radian(
        &state.unidad,
        &state.facturacion,
        &RelojSistema,
        &en.ctx,
        FacturaRecibidaId::from_uuid(ruta.1),
        p.tipo,
        p.motivo.as_deref(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(e.into())))
}
