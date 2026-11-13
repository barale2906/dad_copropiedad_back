//! Cuentas de tesorería, recibos de caja y egresos (plan 13 fase 1).

use application::{
    casos_uso::contabilidad::tesoreria::{
        self as uc, CambioCuentaTesoreria, NuevaCuentaTesoreria, NuevoEgreso, NuevoGasto,
        NuevoRecibo,
    },
    puertos::{RelojSistema, tesoreria::FiltroRecibos},
};
use axum::{extract::State, http::StatusCode};
use chrono::NaiveDate;
use domain::{
    ArchivoId, CuentaTesoreriaId, EgresoId, FacturaProveedorId, FacturaVentaId, PersonaId,
    ReciboCajaId, contabilidad::tesoreria::EstadoEgreso,
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

/// Rutas de cuentas de tesorería, recibos y egresos.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_cuentas, crear_cuenta))
        .routes(routes!(actualizar_cuenta))
        .routes(routes!(listar_recibos, registrar_recibo))
        .routes(routes!(ver_recibo))
        .routes(routes!(aplicar_recibo))
        .routes(routes!(anular_recibo))
        .routes(routes!(listar_egresos, registrar_egreso))
        .routes(routes!(ver_egreso))
        .routes(routes!(autorizar_egreso))
        .routes(routes!(pagar_egreso))
        .routes(routes!(anular_egreso))
}

/// Cuentas de tesorería.
#[utoipa::path(
    get,
    path = "/tesoreria/cuentas",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Cuentas", body = Vec<CuentaTesoreriaRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_cuentas(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<CuentaTesoreriaRespuesta>>> {
    let v = uc::cuentas_tesoreria(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea una cuenta de tesorería respaldada por una cuenta contable de movimiento de naturaleza
/// débito. Del número de cuenta solo se guardan los cuatro últimos dígitos.
#[utoipa::path(
    post,
    path = "/tesoreria/cuentas",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevaCuentaTesoreriaPeticion,
    responses(
        (status = CREATED, description = "Cuenta creada", body = CuentaTesoreriaRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Nombre o cuenta contable ya usados (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos del banco o cuenta contable inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_cuenta(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevaCuentaTesoreriaPeticion>,
) -> ApiResult<(StatusCode, Json<CuentaTesoreriaRespuesta>)> {
    let c = uc::crear_cuenta_tesoreria(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevaCuentaTesoreria {
            tipo: p.tipo,
            nombre: p.nombre,
            banco: p.banco,
            numero_cuenta: p.numero_cuenta,
            tipo_cuenta: p.tipo_cuenta,
            cuenta_contable: codigo(&p.cuenta_contable)?,
            destinacion: p.destinacion,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Cambia el nombre, la destinación o si la cuenta está activa.
#[utoipa::path(
    put,
    path = "/tesoreria/cuentas/{cuenta_id}",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("cuenta_id" = Uuid, Path, description = "Cuenta de tesorería")),
    request_body = CambioCuentaTesoreriaPeticion,
    responses(
        (status = OK, description = "Cuenta", body = CuentaTesoreriaRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Nombre ya usado (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn actualizar_cuenta(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<CambioCuentaTesoreriaPeticion>,
) -> ApiResult<Json<CuentaTesoreriaRespuesta>> {
    let c = uc::actualizar_cuenta_tesoreria(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CuentaTesoreriaId::from_uuid(ruta.1),
        CambioCuentaTesoreria {
            nombre: p.nombre,
            destinacion: p.destinacion,
            activa: p.activa,
        },
    )
    .await?;
    Ok(Json(c.into()))
}

/// Filtro de recibos.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroRecibosParametros {
    pub desde: Option<NaiveDate>,
    pub hasta: Option<NaiveDate>,
    pub persona_id: Option<Uuid>,
    pub cuenta_tesoreria_id: Option<Uuid>,
    /// Solo los no identificados.
    #[serde(default)]
    pub sin_identificar: bool,
}

/// Recibos de caja por fecha descendente (a lo sumo 500).
#[utoipa::path(
    get,
    path = "/tesoreria/recibos",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroRecibosParametros),
    responses(
        (status = OK, description = "Recibos", body = Vec<ReciboRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_recibos(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroRecibosParametros>,
) -> ApiResult<Json<Vec<ReciboRespuesta>>> {
    let v = uc::recibos(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FiltroRecibos {
            desde: f.desde,
            hasta: f.hasta,
            persona_id: f.persona_id.map(PersonaId::from_uuid),
            cuenta_tesoreria_id: f.cuenta_tesoreria_id.map(CuentaTesoreriaId::from_uuid),
            sin_identificar: f.sin_identificar,
        },
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Registra y contabiliza un recibo de caja; lo que no se aplica a facturas queda sin aplicar.
#[utoipa::path(
    post,
    path = "/tesoreria/recibos",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevoReciboPeticion,
    responses(
        (status = CREATED, description = "Recibo registrado", body = ReciboRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.recibos.registrar` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Cuenta, tercero o factura inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Cuenta inactiva o factura que no admite recaudos (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Aplicaciones inválidas (`validacion`) o sin regla, periodo cerrado o descuadre (`regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_recibo(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevoReciboPeticion>,
) -> ApiResult<(StatusCode, Json<ReciboRespuesta>)> {
    let r = uc::registrar_recibo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoRecibo {
            cuenta_tesoreria: CuentaTesoreriaId::from_uuid(p.cuenta_tesoreria_id),
            persona: p.persona_id.map(PersonaId::from_uuid),
            fecha: p.fecha,
            valor: p.valor,
            medio: p.medio,
            referencia: p.referencia,
            descripcion: p.descripcion,
            soporte_archivo_id: p.soporte_archivo_id.map(ArchivoId::from_uuid),
            aplicaciones: p
                .aplicaciones
                .into_iter()
                .map(|a| (FacturaVentaId::from_uuid(a.factura_venta_id), a.valor))
                .collect(),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Recibo de caja con sus aplicaciones.
#[utoipa::path(
    get,
    path = "/tesoreria/recibos/{recibo_id}",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("recibo_id" = Uuid, Path, description = "Recibo")),
    responses(
        (status = OK, description = "Recibo", body = ReciboRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn ver_recibo(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ReciboRespuesta>> {
    let r = uc::recibo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReciboCajaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(r.into()))
}

/// Aplica lo que el recibo tiene sin aplicar a una factura de venta (si no tenía pagador, queda
/// identificado con el cliente de la factura).
#[utoipa::path(
    post,
    path = "/tesoreria/recibos/{recibo_id}/aplicaciones",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("recibo_id" = Uuid, Path, description = "Recibo")),
    request_body = AplicarReciboPeticion,
    responses(
        (status = OK, description = "Recibo", body = ReciboRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.recibos.registrar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Recibo anulado o factura que no admite recaudos (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Supera lo sin aplicar o el saldo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn aplicar_recibo(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AplicarReciboPeticion>,
) -> ApiResult<Json<ReciboRespuesta>> {
    let r = uc::aplicar_recibo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReciboCajaId::from_uuid(ruta.1),
        FacturaVentaId::from_uuid(p.factura_venta_id),
        p.valor,
        p.fecha,
    )
    .await?;
    Ok(Json(r.into()))
}

/// Anula el recibo: revierte su comprobante y devuelve el saldo a las facturas.
#[utoipa::path(
    post,
    path = "/tesoreria/recibos/{recibo_id}/anulacion",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("recibo_id" = Uuid, Path, description = "Recibo")),
    request_body = AnulacionPeticion,
    responses(
        (status = OK, description = "Recibo anulado", body = ReciboRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.recibos.registrar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya está anulado (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin motivo (`validacion`) o periodo cerrado (`regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn anular_recibo(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AnulacionPeticion>,
) -> ApiResult<Json<ReciboRespuesta>> {
    let r = uc::anular_recibo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReciboCajaId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(Json(r.into()))
}

/// Estado de los egresos.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroEgresosParametros {
    #[param(value_type = Option<crate::esquemas::EstadoEgreso>)]
    pub estado: Option<EstadoEgreso>,
}

/// Egresos por fecha descendente (a lo sumo 500).
#[utoipa::path(
    get,
    path = "/tesoreria/egresos",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroEgresosParametros),
    responses(
        (status = OK, description = "Egresos", body = Vec<EgresoRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_egresos(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroEgresosParametros>,
) -> ApiResult<Json<Vec<EgresoRespuesta>>> {
    let v = uc::egresos(&state.unidad, &RelojSistema, &en.ctx, f.estado).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Registra un egreso por autorizar (facturas de proveedor del beneficiario, gastos directos o
/// ambos).
#[utoipa::path(
    post,
    path = "/tesoreria/egresos",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevoEgresoPeticion,
    responses(
        (status = CREATED, description = "Egreso registrado", body = EgresoRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.egresos.registrar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Cuenta inactiva o factura que no admite pagos (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin facturas ni gastos, saldo superado o falta el número del cheque (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_egreso(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevoEgresoPeticion>,
) -> ApiResult<(StatusCode, Json<EgresoRespuesta>)> {
    let mut gastos = Vec::with_capacity(p.gastos.len());
    for g in p.gastos {
        gastos.push(NuevoGasto {
            cuenta: codigo(&g.cuenta)?,
            persona: g.persona_id.map(PersonaId::from_uuid),
            centro_costo: g.centro_costo,
            valor: g.valor,
            descripcion: g.descripcion,
        });
    }
    let e = uc::registrar_egreso(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoEgreso {
            cuenta_tesoreria: CuentaTesoreriaId::from_uuid(p.cuenta_tesoreria_id),
            persona: PersonaId::from_uuid(p.persona_id),
            fecha: p.fecha,
            medio: p.medio,
            referencia: p.referencia,
            descripcion: p.descripcion,
            soporte_archivo_id: p.soporte_archivo_id.map(ArchivoId::from_uuid),
            facturas: p
                .facturas
                .into_iter()
                .map(|f| {
                    (
                        FacturaProveedorId::from_uuid(f.factura_proveedor_id),
                        f.valor,
                    )
                })
                .collect(),
            gastos,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(e.into())))
}

/// Egreso.
#[utoipa::path(
    get,
    path = "/tesoreria/egresos/{egreso_id}",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("egreso_id" = Uuid, Path, description = "Egreso")),
    responses(
        (status = OK, description = "Egreso", body = EgresoRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn ver_egreso(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<EgresoRespuesta>> {
    let e = uc::egreso(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        EgresoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(e.into()))
}

/// Autoriza un egreso que registró **otra** cuenta (segregación de funciones).
#[utoipa::path(
    post,
    path = "/tesoreria/egresos/{egreso_id}/autorizacion",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("egreso_id" = Uuid, Path, description = "Egreso")),
    responses(
        (status = OK, description = "Egreso autorizado", body = EgresoRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.egresos.autorizar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Lo registró la misma cuenta (`conflicto`) o el estado no lo permite (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn autorizar_egreso(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<EgresoRespuesta>> {
    let e = uc::autorizar_egreso(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        EgresoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(e.into()))
}

/// Registra el pago de un egreso autorizado: lo contabiliza y abona las facturas que paga.
#[utoipa::path(
    post,
    path = "/tesoreria/egresos/{egreso_id}/pago",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("egreso_id" = Uuid, Path, description = "Egreso")),
    request_body = PagoEgresoPeticion,
    responses(
        (status = OK, description = "Egreso pagado", body = EgresoRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.egresos.registrar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está autorizado (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin regla, periodo cerrado o descuadre (`regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn pagar_egreso(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PagoEgresoPeticion>,
) -> ApiResult<Json<EgresoRespuesta>> {
    let e = uc::pagar_egreso(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        EgresoId::from_uuid(ruta.1),
        p.fecha,
    )
    .await?;
    Ok(Json(e.into()))
}

/// Anula el egreso; si ya se pagó, revierte su comprobante y devuelve el saldo a las facturas.
#[utoipa::path(
    post,
    path = "/tesoreria/egresos/{egreso_id}/anulacion",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("egreso_id" = Uuid, Path, description = "Egreso")),
    request_body = AnulacionPeticion,
    responses(
        (status = OK, description = "Egreso anulado", body = EgresoRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.egresos.registrar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya está anulado (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn anular_egreso(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AnulacionPeticion>,
) -> ApiResult<Json<EgresoRespuesta>> {
    let e = uc::anular_egreso(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        EgresoId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(Json(e.into()))
}
