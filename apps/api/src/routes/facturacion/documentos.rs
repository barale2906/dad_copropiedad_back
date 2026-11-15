//! Documentos electrónicos y facturas recibidas (plan 15 fases 3 y 4, módulo
//! `facturacion_electronica`): `/entidades/{entidad_id}/facturacion/…`. El XML de una factura recibida viaja en Base64 dentro del JSON.

use application::{
    AppError,
    casos_uso::facturacion::{documentos as uc, radian},
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
    tributario::{EstadoDocumentoElectronico, EstadoFacturaRecibida, TipoDocumentoElectronico},
};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::routes::contabilidad::dto_tributario::*;
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_FACTURACION,
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas de documentos electrónicos y facturas recibidas.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(emitir_factura_electronica))
        .routes(routes!(transcribir_factura_contingencia))
        .routes(routes!(emitir_nota_credito))
        .routes(routes!(emitir_documento_soporte))
        .routes(routes!(emitir_nota_ajuste_soporte))
        .routes(routes!(listar_documentos))
        .routes(routes!(ver_documento))
        .routes(routes!(reintentar_documento))
        .routes(routes!(representacion_grafica))
        .routes(routes!(paquete_adquirente))
        .routes(routes!(listar_recibidas, cargar_recibida))
        .routes(routes!(ver_recibida))
        .routes(routes!(vincular_recibida))
        .routes(routes!(rechazar_recibida))
        .routes(routes!(evento_radian))
        .routes(routes!(aceptacion_tacita))
        .routes(routes!(eventos_documento))
}

// ---------------------------------------------------------------------------------------------
// Emisión
// ---------------------------------------------------------------------------------------------

/// Emite la factura electrónica de una factura de venta emitida. Pedirla otra vez devuelve el
/// mismo documento (mismo CUFE) y lo reintenta si quedó en contingencia.
#[utoipa::path(
    post,
    path = "/facturas/{factura_id}/factura-electronica",
    tag = ETIQUETA_FACTURACION,
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
        &state.expedicion(),
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        FacturaVentaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(d.into()))
}

/// Factura de talonario que se transcribe.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TranscripcionPeticion {
    /// Número de la factura de papel, dentro de una resolución de contingencia vigente en la
    /// fecha de la factura (`tipo: contingencia` en `/tributario/resoluciones`).
    #[schema(example = 7)]
    pub numero_talonario: u64,
}

/// Transcribe como factura electrónica **tipo 03** una factura de venta que se expidió en papel
/// con el talonario de contingencia del facturador mientras no pudo facturar electrónicamente.
/// La DIAN exige transcribirla dentro de las 48 horas siguientes a superar el inconveniente.
#[utoipa::path(
    post,
    path = "/facturas/{factura_id}/factura-contingencia",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura de venta")),
    request_body = TranscripcionPeticion,
    responses(
        (status = OK, description = "Documento (validado, rechazado o en espera de la DIAN)", body = DocumentoElectronicoRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.emitir` (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Factura inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ese número ya se transcribió o la factura ya tiene otra factura electrónica (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "El número no está en una resolución de contingencia vigente o la factura no está emitida (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn transcribir_factura_contingencia(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<TranscripcionPeticion>,
) -> ApiResult<Json<DocumentoElectronicoRespuesta>> {
    let d = uc::transcribir_factura_contingencia(
        &state.unidad,
        &state.expedicion(),
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        FacturaVentaId::from_uuid(ruta.1),
        p.numero_talonario,
    )
    .await?;
    Ok(Json(d.into()))
}

/// Anula una factura de venta con factura electrónica validada mediante una nota crédito
/// electrónica (revierte su contabilización).
#[utoipa::path(
    post,
    path = "/facturas/{factura_id}/nota-credito",
    tag = ETIQUETA_FACTURACION,
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
        &state.expedicion(),
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
    path = "/compras/{factura_id}/documento-soporte",
    tag = ETIQUETA_FACTURACION,
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
        &state.expedicion(),
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        FacturaProveedorId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(d.into()))
}

/// Emite la nota de ajuste que anula el documento soporte validado de una compra (anula
/// también la compra y, con contabilidad, revierte su comprobante).
#[utoipa::path(
    post,
    path = "/compras/{factura_id}/nota-ajuste",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura de proveedor")),
    request_body = NotaCreditoPeticion,
    responses(
        (status = OK, description = "Nota de ajuste", body = DocumentoElectronicoRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.facturas.gestionar` ni `facturacion.emitir` (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Compra inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "La compra no tiene documento soporte validado o ya tiene pagos (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Motivo inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn emitir_nota_ajuste_soporte(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<NotaCreditoPeticion>,
) -> ApiResult<Json<DocumentoElectronicoRespuesta>> {
    let d = uc::emitir_nota_ajuste_soporte(
        &state.unidad,
        &state.expedicion(),
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        FacturaProveedorId::from_uuid(ruta.1),
        &p.motivo,
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
    path = "/documentos",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroDocumentosParametros),
    responses(
        (status = OK, description = "Documentos", body = Vec<DocumentoElectronicoRespuesta>),
        (status = FORBIDDEN, description = "Sin `facturacion.ver` o sin el módulo", body = ErrorRespuesta),
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
            ..FiltroDocumentos::default()
        },
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Documento electrónico con sus eventos.
#[utoipa::path(
    get,
    path = "/documentos/{documento_id}",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("documento_id" = Uuid, Path, description = "Documento")),
    responses(
        (status = OK, description = "Documento", body = DocumentoDetalleRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.ver` o sin el módulo", body = ErrorRespuesta),
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
    path = "/documentos/{documento_id}/reintento",
    tag = ETIQUETA_FACTURACION,
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
        &state.expedicion(),
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
    path = "/documentos/{documento_id}/pdf",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("documento_id" = Uuid, Path, description = "Documento")),
    responses(
        (status = OK, description = "PDF", content_type = "application/pdf", body = Vec<u8>),
        (status = FORBIDDEN, description = "Sin `facturacion.ver` o sin el módulo", body = ErrorRespuesta),
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
        (&state.actas, state.pie_facturacion.as_deref()),
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

/// ZIP del documento tal como lo recibe el adquirente: el `AttachedDocument` (el documento y la
/// respuesta de la DIAN) y la representación gráfica en PDF.
#[utoipa::path(
    get,
    path = "/documentos/{documento_id}/paquete",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("documento_id" = Uuid, Path, description = "Documento")),
    responses(
        (status = OK, description = "ZIP", content_type = "application/zip", body = Vec<u8>),
        (status = FORBIDDEN, description = "Sin `facturacion.ver` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Documento inexistente", body = ErrorRespuesta),
    )
)]
pub async fn paquete_adquirente(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    let (nombre, zip) = uc::paquete_documento(
        &state.unidad,
        &state.expedicion(),
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        DocumentoElectronicoId::from_uuid(ruta.1),
    )
    .await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{nombre}\""),
            ),
        ],
        zip,
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
    path = "/recibidas",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroRecibidasParametros),
    responses(
        (status = OK, description = "Facturas recibidas", body = Vec<FacturaRecibidaRespuesta>),
        (status = FORBIDDEN, description = "Sin `facturacion.ver` o sin el módulo", body = ErrorRespuesta),
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
    path = "/recibidas",
    tag = ETIQUETA_FACTURACION,
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
    path = "/recibidas/{recibida_id}",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("recibida_id" = Uuid, Path, description = "Factura recibida")),
    responses(
        (status = OK, description = "Factura", body = FacturaRecibidaDetalleRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.ver` o sin el módulo", body = ErrorRespuesta),
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
    path = "/recibidas/{recibida_id}/vinculo",
    tag = ETIQUETA_FACTURACION,
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
    path = "/recibidas/{recibida_id}/rechazo",
    tag = ETIQUETA_FACTURACION,
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

/// Registra un evento RADIAN (acuse, recibo del bien, aceptación expresa o reclamo) sobre una
/// factura recibida. Con el canal `propio` se firma y se envía a la DIAN; si no, lo registra el
/// proveedor. Cada evento exige sus previos y no se repite.
#[utoipa::path(
    post,
    path = "/recibidas/{recibida_id}/eventos",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("recibida_id" = Uuid, Path, description = "Factura recibida")),
    request_body = EventoRadianPeticion,
    responses(
        (status = CREATED, description = "Evento registrado (mire `estado`: la DIAN pudo rechazarlo)", body = EventoRadianRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.recibir` (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Factura inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya registrado (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Faltan los eventos previos, el concepto del reclamo o quien recibió (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn evento_radian(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<EventoRadianPeticion>,
) -> ApiResult<(StatusCode, Json<EventoRadianRespuesta>)> {
    let e = radian::registrar_evento_radian(
        &state.unidad,
        &state.expedicion(),
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        FacturaRecibidaId::from_uuid(ruta.1),
        radian::DatosEvento {
            tipo: p.tipo,
            motivo: p.motivo,
            concepto_reclamo: p.concepto_reclamo,
            quien_recibe: p.quien_recibe.map(Into::into),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(e.into())))
}

/// Registra la aceptación tácita (evento 034) de una factura electrónica propia, validada y a
/// crédito, cuando el adquirente no la aceptó ni la reclamó en los tres días hábiles siguientes
/// al recibo del bien. La DIAN verifica el plazo y los eventos del adquirente.
#[utoipa::path(
    post,
    path = "/documentos/{documento_id}/aceptacion-tacita",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("documento_id" = Uuid, Path, description = "Factura electrónica")),
    responses(
        (status = CREATED, description = "Evento registrado (mire `estado`: la DIAN pudo rechazarlo)", body = EventoRadianRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.emitir` (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Documento inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya registrada (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es una factura validada a crédito (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn aceptacion_tacita(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<(StatusCode, Json<EventoRadianRespuesta>)> {
    let e = radian::registrar_aceptacion_tacita(
        &state.unidad,
        &state.expedicion(),
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        DocumentoElectronicoId::from_uuid(ruta.1),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(e.into())))
}

/// Eventos RADIAN que la entidad registró sobre su propia factura electrónica.
#[utoipa::path(
    get,
    path = "/documentos/{documento_id}/eventos",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("documento_id" = Uuid, Path, description = "Factura electrónica")),
    responses(
        (status = OK, description = "Eventos", body = Vec<EventoRadianRespuesta>),
        (status = FORBIDDEN, description = "Sin `facturacion.ver` (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn eventos_documento(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<EventoRadianRespuesta>>> {
    let eventos = radian::eventos_de_documento(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        DocumentoElectronicoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(eventos.into_iter().map(Into::into).collect()))
}
