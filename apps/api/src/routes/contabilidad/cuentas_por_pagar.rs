//! Catálogo de retenciones (lectura), facturas de proveedores, causación y certificados de
//! retención (plan 13 fase 2).

use application::{
    casos_uso::contabilidad::cuentas_por_pagar::{
        self as uc, NuevaFacturaProveedor, NuevaLineaFacturaProveedor,
    },
    puertos::{RelojSistema, contabilidad::Origen, tesoreria::FiltroFacturas},
};
use axum::{
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use domain::{
    ArchivoId, FacturaProveedorId, PersonaId, contabilidad::tesoreria::EstadoFacturaProveedor,
};
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

/// Rutas de cuentas por pagar.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_retenciones))
        .routes(routes!(listar_uvt))
        .routes(routes!(listar_facturas, registrar_factura))
        .routes(routes!(ver_factura))
        .routes(routes!(causar_factura))
        .routes(routes!(anular_factura))
        .routes(routes!(ver_certificado))
        .routes(routes!(certificado_pdf))
        .routes(routes!(enviar_certificado))
}

/// Fecha de vigencia.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct VigenciaParametros {
    /// Solo las versiones vigentes en la fecha (sin ella, todas).
    pub vigentes_en: Option<NaiveDate>,
}

/// Conceptos de retención del catálogo de la plataforma.
#[utoipa::path(
    get,
    path = "/tesoreria/retenciones",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), VigenciaParametros),
    responses(
        (status = OK, description = "Conceptos", body = Vec<ConceptoRetencionRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_retenciones(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<VigenciaParametros>,
) -> ApiResult<Json<Vec<ConceptoRetencionRespuesta>>> {
    let v = uc::conceptos_retencion(&state.unidad, &RelojSistema, &en.ctx, f.vigentes_en).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Valores de la UVT por año.
#[utoipa::path(
    get,
    path = "/tesoreria/uvt",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "UVT", body = Vec<UvtRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_uvt(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<UvtRespuesta>>> {
    let v = uc::valores_uvt(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Filtro de facturas de proveedor.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroFacturasProveedorParametros {
    pub persona_id: Option<Uuid>,
    #[param(value_type = Option<crate::esquemas::EstadoFacturaProveedor>)]
    pub estado: Option<EstadoFacturaProveedor>,
    pub desde: Option<NaiveDate>,
    pub hasta: Option<NaiveDate>,
}

/// Facturas de proveedor por vencimiento (a lo sumo 500, sin líneas).
#[utoipa::path(
    get,
    path = "/cuentas-por-pagar/facturas",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroFacturasProveedorParametros),
    responses(
        (status = OK, description = "Facturas", body = Vec<FacturaProveedorRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_facturas(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroFacturasProveedorParametros>,
) -> ApiResult<Json<Vec<FacturaProveedorRespuesta>>> {
    let v = uc::facturas_proveedor(
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

/// Registra una factura de proveedor y calcula sus retenciones con la tarifa vigente en su
/// fecha. Un número que el proveedor ya usó se rechaza salvo excepción con motivo.
#[utoipa::path(
    post,
    path = "/cuentas-por-pagar/facturas",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevaFacturaProveedorPeticion,
    responses(
        (status = CREATED, description = "Factura registrada", body = FacturaProveedorRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.facturas_proveedor.gestionar` (o `tesoreria.egresos.autorizar` para la excepción) o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Tercero inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Factura repetida del proveedor (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Dato inválido o concepto no vigente (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_factura(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevaFacturaProveedorPeticion>,
) -> ApiResult<(StatusCode, Json<FacturaProveedorRespuesta>)> {
    let mut lineas = Vec::with_capacity(p.lineas.len());
    for l in p.lineas {
        lineas.push(NuevaLineaFacturaProveedor {
            descripcion: l.descripcion,
            cuenta: codigo(&l.cuenta)?,
            centro_costo: l.centro_costo,
            base: l.base,
            iva: l.iva,
            iva_descontable: l.iva_descontable,
            retenciones: l.retenciones,
        });
    }
    let f = uc::registrar_factura_proveedor(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevaFacturaProveedor {
            persona: PersonaId::from_uuid(p.persona_id),
            prefijo: p.prefijo,
            numero: p.numero,
            fecha: p.fecha,
            vencimiento: p.vencimiento,
            descripcion: p.descripcion,
            origen: p.origen.map(|o| Origen {
                tipo: o.tipo,
                id: o.id,
            }),
            cufe: p.cufe,
            archivo_id: p.archivo_id.map(ArchivoId::from_uuid),
            lineas,
            excepcion_duplicado: p.excepcion_duplicado,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(f.into())))
}

/// Factura de proveedor con sus líneas y retenciones.
#[utoipa::path(
    get,
    path = "/cuentas-por-pagar/facturas/{factura_id}",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura")),
    responses(
        (status = OK, description = "Factura", body = FacturaProveedorRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn ver_factura(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<FacturaProveedorRespuesta>> {
    let f = uc::factura_proveedor(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FacturaProveedorId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(f.into()))
}

/// Causa la factura (gasto, IVA, retenciones y cuenta por pagar) en su fecha.
#[utoipa::path(
    post,
    path = "/cuentas-por-pagar/facturas/{factura_id}/causacion",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura")),
    responses(
        (status = OK, description = "Factura causada", body = FacturaProveedorRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.facturas_proveedor.gestionar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya se causó o se anuló (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin regla, periodo cerrado o descuadre (`regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn causar_factura(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<FacturaProveedorRespuesta>> {
    let f = uc::causar_factura_proveedor(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FacturaProveedorId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(f.into()))
}

/// Anula una factura sin pagos (si estaba causada, revierte su comprobante).
#[utoipa::path(
    post,
    path = "/cuentas-por-pagar/facturas/{factura_id}/anulacion",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura")),
    request_body = AnulacionPeticion,
    responses(
        (status = OK, description = "Factura anulada", body = FacturaProveedorRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.facturas_proveedor.gestionar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Tiene pagos o ya se anuló (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn anular_factura(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AnulacionPeticion>,
) -> ApiResult<Json<FacturaProveedorRespuesta>> {
    let f = uc::anular_factura_proveedor(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FacturaProveedorId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(Json(f.into()))
}

/// Periodo del certificado.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PeriodoCertificadoParametros {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
}

/// Certificado de las retenciones practicadas al tercero en el periodo.
#[utoipa::path(
    get,
    path = "/cuentas-por-pagar/certificados/{persona_id}",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("persona_id" = Uuid, Path, description = "Tercero"), PeriodoCertificadoParametros),
    responses(
        (status = OK, description = "Certificado", body = CertificadoRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.facturas_proveedor.gestionar` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn ver_certificado(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(q): Query<PeriodoCertificadoParametros>,
) -> ApiResult<Json<CertificadoRespuesta>> {
    let c = uc::certificado_retenciones(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PersonaId::from_uuid(ruta.1),
        q.desde,
        q.hasta,
    )
    .await?;
    Ok(Json(c.into()))
}

/// El certificado de retenciones en PDF.
#[utoipa::path(
    get,
    path = "/cuentas-por-pagar/certificados/{persona_id}/pdf",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("persona_id" = Uuid, Path, description = "Tercero"), PeriodoCertificadoParametros),
    responses(
        (status = OK, description = "PDF del certificado", content_type = "application/pdf", body = Vec<u8>),
        (status = FORBIDDEN, description = "Sin `tesoreria.facturas_proveedor.gestionar` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn certificado_pdf(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(q): Query<PeriodoCertificadoParametros>,
) -> ApiResult<Response> {
    let pdf = uc::certificado_retenciones_pdf(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        &en.ctx,
        PersonaId::from_uuid(ruta.1),
        q.desde,
        q.hasta,
    )
    .await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"certificado-retenciones-{}.pdf\"",
                    q.hasta
                ),
            ),
        ],
        pdf,
    )
        .into_response())
}

/// Envía el certificado por correo al correo de facturación del tercero (o a su correo).
#[utoipa::path(
    post,
    path = "/cuentas-por-pagar/certificados/{persona_id}/envio",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("persona_id" = Uuid, Path, description = "Tercero"), PeriodoCertificadoParametros),
    responses(
        (status = ACCEPTED, description = "Certificado en la cola de correo", body = EnvioCertificadoRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.facturas_proveedor.gestionar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "El tercero no tiene correo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn enviar_certificado(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(q): Query<PeriodoCertificadoParametros>,
) -> ApiResult<(StatusCode, Json<EnvioCertificadoRespuesta>)> {
    let para = uc::enviar_certificado_retenciones(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PersonaId::from_uuid(ruta.1),
        q.desde,
        q.hasta,
    )
    .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(EnvioCertificadoRespuesta {
            para: para.as_str().to_owned(),
        }),
    ))
}
