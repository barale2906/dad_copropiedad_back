//! Facturas de venta (plan 13 fase 2b y plan 18): `/entidades/{entidad_id}/facturacion/facturas`.
//! Sirven con el módulo de facturación electrónica o con la contabilidad; sin contabilidad no
//! llevan cuentas y el pago se marca a mano.

use application::{
    casos_uso::facturacion::ventas::{
        self as uc, NuevaFacturaVenta, NuevaLineaVenta, NuevoPagoManual,
    },
    puertos::{RelojSistema, contabilidad::Origen, tesoreria::FiltroFacturas},
};
use axum::{extract::State, http::StatusCode};
use chrono::NaiveDate;
use domain::{
    FacturaVentaId, ItemFacturacionId, PersonaId, contabilidad::tesoreria::EstadoFacturaVenta,
    facturacion::MedioPagoManual,
};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::routes::contabilidad::dto::codigo;
use crate::routes::contabilidad::dto_tesoreria::*;
use crate::routes::contabilidad::dto_tributario::DocumentoElectronicoRespuesta;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_FACTURACION,
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
        .routes(routes!(marcar_pagada))
        .routes(routes!(emitir_nota_debito))
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
    path = "/facturas",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroFacturasVentaParametros),
    responses(
        (status = OK, description = "Facturas", body = Vec<FacturaVentaRespuesta>),
        (status = FORBIDDEN, description = "Sin `facturacion.ver` o sin el módulo", body = ErrorRespuesta),
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

/// Líneas de la petición con su cuenta validada.
fn lineas_venta(lineas: Vec<LineaVentaPeticion>) -> ApiResult<Vec<NuevaLineaVenta>> {
    lineas
        .into_iter()
        .map(|l| {
            Ok(NuevaLineaVenta {
                item: l.item_id.map(ItemFacturacionId::from_uuid),
                descripcion: l.descripcion,
                cuenta: l.cuenta.as_deref().map(codigo).transpose()?,
                centro_costo: l.centro_costo,
                cantidad: l.cantidad,
                valor_unitario: l.valor_unitario,
                tarifa_iva: l.tarifa_iva,
                clasificacion: l.clasificacion,
            })
        })
        .collect()
}

/// Nota débito de una factura de venta.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NotaDebitoPeticion {
    #[schema(value_type = crate::esquemas::ConceptoNotaDebito)]
    pub concepto: domain::tributario::ConceptoNotaDebito,
    /// Motivo (de 5 a 300 caracteres): descripción de la nota y de su corrección ante la DIAN.
    #[schema(example = "Intereses de mora de noviembre")]
    pub motivo: String,
    pub fecha: NaiveDate,
    pub vencimiento: NaiveDate,
    pub lineas: Vec<LineaVentaPeticion>,
}

/// Emite la nota débito de una factura con factura electrónica validada: un documento por
/// cobrar más del mismo cliente (entra a la cartera, se abona con recibos de caja o se marca
/// pagada a mano y, con contabilidad, se contabiliza) que se transmite a la DIAN referido a la
/// factura electrónica original.
#[utoipa::path(
    post,
    path = "/facturas/{factura_id}/nota-debito",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura de venta que se ajusta")),
    request_body = NotaDebitoPeticion,
    responses(
        (status = OK, description = "Documento electrónico de la nota débito (su `origen` es la nota, que se ve en `/facturas`)", body = DocumentoElectronicoRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.facturas.gestionar` ni `facturacion.emitir` (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Factura inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "La factura no tiene factura electrónica validada (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Factura anulada, es otra nota débito, motivo o línea inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn emitir_nota_debito(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<NotaDebitoPeticion>,
) -> ApiResult<Json<DocumentoElectronicoRespuesta>> {
    let d = application::casos_uso::facturacion::documentos::emitir_nota_debito(
        &state.unidad,
        &state.expedicion(),
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        FacturaVentaId::from_uuid(ruta.1),
        application::casos_uso::facturacion::documentos::NuevaNotaDebito {
            concepto: p.concepto,
            motivo: p.motivo,
            fecha: p.fecha,
            vencimiento: p.vencimiento,
            lineas: lineas_venta(p.lineas)?,
        },
    )
    .await?;
    Ok(Json(d.into()))
}

/// Crea una factura de venta en borrador (el IVA de cada línea lo elige quien factura).
#[utoipa::path(
    post,
    path = "/facturas",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevaFacturaVentaPeticion,
    responses(
        (status = CREATED, description = "Factura en borrador", body = FacturaVentaRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.facturas.gestionar` o sin el módulo", body = ErrorRespuesta),
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
    let lineas = lineas_venta(p.lineas)?;
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
    path = "/facturas/{factura_id}",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura")),
    responses(
        (status = OK, description = "Factura", body = FacturaVentaRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.ver` o sin el módulo", body = ErrorRespuesta),
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
    path = "/facturas/{factura_id}",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura")),
    responses(
        (status = NO_CONTENT, description = "Borrada"),
        (status = FORBIDDEN, description = "Sin `facturacion.facturas.gestionar` o sin el módulo", body = ErrorRespuesta),
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

/// Emite la factura: le asigna su número interno del año y, si la entidad lleva contabilidad, la
/// contabiliza.
#[utoipa::path(
    post,
    path = "/facturas/{factura_id}/emision",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura")),
    responses(
        (status = OK, description = "Factura emitida", body = FacturaVentaRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.facturas.gestionar` o sin el módulo", body = ErrorRespuesta),
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

/// Anula una factura emitida sin pagos (revierte su comprobante si se contabilizó). Con factura
/// electrónica validada se anula con nota crédito.
#[utoipa::path(
    post,
    path = "/facturas/{factura_id}/anulacion",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura")),
    request_body = AnulacionPeticion,
    responses(
        (status = OK, description = "Factura anulada", body = FacturaVentaRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.facturas.gestionar` o sin el módulo", body = ErrorRespuesta),
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

/// Pago de una factura que se marca a mano.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PagoManualPeticion {
    pub fecha: NaiveDate,
    #[schema(value_type = crate::esquemas::MedioPagoManual)]
    pub medio: MedioPagoManual,
    /// Dato personal: referencia del pago (número de la transferencia…).
    pub referencia: Option<String>,
}

/// Marca pagada una factura emitida de una entidad **sin contabilidad**. Con contabilidad el pago
/// se registra con un recibo de caja de la tesorería.
#[utoipa::path(
    post,
    path = "/facturas/{factura_id}/pago",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("factura_id" = Uuid, Path, description = "Factura")),
    request_body = PagoManualPeticion,
    responses(
        (status = OK, description = "Factura pagada", body = FacturaVentaRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.facturas.gestionar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "La entidad lleva contabilidad (`conflicto`) o la factura no está emitida (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Fecha anterior a la factura (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn marcar_pagada(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PagoManualPeticion>,
) -> ApiResult<Json<FacturaVentaRespuesta>> {
    let f = uc::marcar_pagada(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FacturaVentaId::from_uuid(ruta.1),
        NuevoPagoManual {
            fecha: p.fecha,
            medio: p.medio,
            referencia: p.referencia,
        },
    )
    .await?;
    Ok(Json(f.into()))
}
