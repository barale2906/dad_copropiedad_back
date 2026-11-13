//! Recaudo y cartera por bien (plan 14 fase 4): recaudos, recaudo por referencia, anulación,
//! aplicación de anticipos, cobros manuales, notas crédito, cartera inicial, estado de cuenta,
//! saldos por bien y cuadre contra el mayor.

use application::{
    casos_uso::contabilidad_ph::recaudo::{
        self as rec, FilaCarteraInicial, NuevoCobro, NuevoRecaudo,
    },
    puertos::RelojSistema,
};
use axum::{extract::State, http::StatusCode};
use chrono::NaiveDate;
use domain::{
    ArchivoId, BienPrivadoId, ConceptoCobroId, CuentaTesoreriaId, ObligacionCobroId, ReciboCajaId,
};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::*;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_CARTERA,
    routes::contabilidad::dto_tesoreria::ReciboRespuesta,
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas del recaudo y de la cartera por bien.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(recaudar))
        .routes(routes!(recaudar_por_referencia))
        .routes(routes!(anular_recaudo))
        .routes(routes!(aplicar_recibo))
        .routes(routes!(registrar_cobro))
        .routes(routes!(anular_cobro))
        .routes(routes!(nota_credito))
        .routes(routes!(cartera_inicial))
        .routes(routes!(estado_cuenta))
        .routes(routes!(saldos))
        .routes(routes!(cuadre))
}

/// Registra un pago para un bien: se imputa a sus obligaciones según la política, gana el pronto
/// pago si llega a tiempo y cubre la deuda, y lo que sobra queda como anticipo.
#[utoipa::path(
    post,
    path = "/cartera/recaudos",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoRecaudoPeticion,
    responses(
        (status = CREATED, description = "Recaudo registrado", body = RecaudoRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.recibos.registrar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Bien o cuenta de tesorería inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin política vigente, sin pagador u obligaciones elegidas sin permitirlo (`validacion`, `regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn recaudar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoRecaudoPeticion>,
) -> ApiResult<(StatusCode, Json<RecaudoRespuesta>)> {
    let r = rec::recaudar(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoRecaudo {
            bien: BienPrivadoId::from_uuid(p.bien_privado_id),
            cuenta_tesoreria: CuentaTesoreriaId::from_uuid(p.cuenta_tesoreria_id),
            fecha: p.fecha,
            valor: p.valor,
            medio: p.medio,
            referencia: p.referencia,
            descripcion: p.descripcion,
            soporte_archivo_id: p.soporte_archivo_id.map(ArchivoId::from_uuid),
            elegidas: p
                .obligaciones_elegidas
                .into_iter()
                .map(ObligacionCobroId::from_uuid)
                .collect(),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Recauda los créditos pendientes del extracto cuya referencia es la de un bien (archivo
/// Asobancaria o extracto con referencia) y los concilia.
#[utoipa::path(
    post,
    path = "/cartera/recaudos/por-referencia",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = RecaudoReferenciaPeticion,
    responses(
        (status = OK, description = "Recaudos creados", body = RecaudoExtractoRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.conciliar`, `tesoreria.recibos.registrar` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn recaudar_por_referencia(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<RecaudoReferenciaPeticion>,
) -> ApiResult<Json<RecaudoExtractoRespuesta>> {
    let r = rec::recaudar_por_referencia(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CuentaTesoreriaId::from_uuid(p.cuenta_tesoreria_id),
        p.desde,
        p.hasta,
    )
    .await?;
    Ok(Json(r.into()))
}

/// Anula un recaudo: revierte su pronto pago y anula el recibo (las obligaciones recuperan su
/// saldo).
#[utoipa::path(
    post,
    path = "/cartera/recaudos/{recibo_id}/anulacion",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("recibo_id" = Uuid, Path, description = "Recibo de caja")),
    request_body = MotivoPeticion,
    responses(
        (status = OK, description = "Recibo anulado", body = ReciboRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.recibos.registrar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Recibo inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya anulado (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn anular_recaudo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<MotivoPeticion>,
) -> ApiResult<Json<ReciboRespuesta>> {
    let r = rec::anular_recaudo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReciboCajaId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(Json(r.into()))
}

/// Aplica lo sin aplicar de un recibo (anticipo o por identificar) a las obligaciones de un bien.
#[utoipa::path(
    post,
    path = "/cartera/recibos/{recibo_id}/aplicacion",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("recibo_id" = Uuid, Path, description = "Recibo de caja")),
    request_body = AplicacionBienPeticion,
    responses(
        (status = OK, description = "Lo aplicado a cada obligación", body = Vec<AplicadoRespuesta>),
        (status = FORBIDDEN, description = "Sin `tesoreria.recibos.registrar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Recibo o bien inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Recibo anulado (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Recibo de otra persona o nada que aplicar (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn aplicar_recibo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AplicacionBienPeticion>,
) -> ApiResult<Json<Vec<AplicadoRespuesta>>> {
    let v = rec::aplicar_recibo_a_bien(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReciboCajaId::from_uuid(ruta.1),
        BienPrivadoId::from_uuid(p.bien_privado_id),
        p.fecha,
    )
    .await?;
    Ok(Json(aplicados(v)))
}

/// Registra un cobro manual (multa, uso de un bien común, daño, otro) con su comprobante (evento
/// `ph.cobro`).
#[utoipa::path(
    post,
    path = "/cartera/cobros",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoCobroPeticion,
    responses(
        (status = CREATED, description = "Obligación creada", body = ObligacionRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ajustar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Bien o concepto inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Concepto que solo se liquida, valor o fechas inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_cobro(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoCobroPeticion>,
) -> ApiResult<(StatusCode, Json<ObligacionRespuesta>)> {
    let o = rec::registrar_cobro(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoCobro {
            bien: BienPrivadoId::from_uuid(p.bien_privado_id),
            concepto: ConceptoCobroId::from_uuid(p.concepto_cobro_id),
            valor: p.valor,
            fecha: p.fecha,
            vencimiento: p.vencimiento,
            descripcion: p.descripcion,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(o.into())))
}

/// Anula un cobro manual sin pagos ni ajustes (revierte su comprobante).
#[utoipa::path(
    post,
    path = "/cartera/obligaciones/{obligacion_id}/anulacion",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("obligacion_id" = Uuid, Path, description = "Obligación")),
    request_body = MotivoPeticion,
    responses(
        (status = NO_CONTENT, description = "Cobro anulado"),
        (status = FORBIDDEN, description = "Sin `cartera.ajustar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Obligación inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "No es manual, ya anulada o con pagos o ajustes (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn anular_cobro(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<MotivoPeticion>,
) -> ApiResult<StatusCode> {
    rec::anular_cobro(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ObligacionCobroId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Nota crédito sobre una obligación (corrige lo liquidado; evento `ph.nota_credito`).
#[utoipa::path(
    post,
    path = "/cartera/obligaciones/{obligacion_id}/notas-credito",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("obligacion_id" = Uuid, Path, description = "Obligación")),
    request_body = NotaCreditoPeticion,
    responses(
        (status = CREATED, description = "Nota crédito registrada", body = AjusteRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ajustar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Obligación inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Valor cero o mayor que el saldo, motivo corto (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn nota_credito(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<NotaCreditoPeticion>,
) -> ApiResult<(StatusCode, Json<AjusteRespuesta>)> {
    let a = rec::registrar_nota_credito(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ObligacionCobroId::from_uuid(ruta.1),
        p.valor,
        &p.motivo,
        p.fecha,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(a.into())))
}

/// Carga (o valida) la cartera inicial por bien, concepto y antigüedad. Con errores no carga
/// nada.
#[utoipa::path(
    post,
    path = "/cartera/inicial",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = CarteraInicialPeticion,
    responses(
        (status = OK, description = "Resultado (con errores por fila si los hay)", body = CarteraInicialRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ajustar` o sin los módulos", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin filas (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn cartera_inicial(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<CarteraInicialPeticion>,
) -> ApiResult<Json<CarteraInicialRespuesta>> {
    let filas = p
        .filas
        .into_iter()
        .map(|f| FilaCarteraInicial {
            bien: f.bien,
            concepto: f.concepto,
            periodo: f.periodo,
            valor: f.valor,
            vencimiento: f.vencimiento,
        })
        .collect();
    let r =
        rec::cargar_cartera_inicial(&state.unidad, &RelojSistema, &en.ctx, filas, p.solo_validar)
            .await?;
    Ok(Json(r.into()))
}

/// Fecha de corte.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CorteParametros {
    /// Por defecto, hoy.
    pub corte: Option<NaiveDate>,
}

/// Estado de cuenta de un bien al corte.
#[utoipa::path(
    get,
    path = "/cartera/bienes/{bien_privado_id}/estado-cuenta",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("bien_privado_id" = Uuid, Path, description = "Bien privado"), CorteParametros),
    responses(
        (status = OK, description = "Estado de cuenta", body = EstadoCuentaRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Bien inexistente", body = ErrorRespuesta),
    )
)]
pub async fn estado_cuenta(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(q): Query<CorteParametros>,
) -> ApiResult<Json<EstadoCuentaRespuesta>> {
    let e = rec::estado_cuenta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
        q.corte,
    )
    .await?;
    Ok(Json(e.into()))
}

/// Cartera por bien al corte (solo los bienes con saldo).
#[utoipa::path(
    get,
    path = "/cartera/saldos",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), CorteParametros),
    responses(
        (status = OK, description = "Saldos por bien", body = Vec<SaldoBienRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn saldos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(q): Query<CorteParametros>,
) -> ApiResult<Json<Vec<SaldoBienRespuesta>>> {
    let v = rec::cartera_por_bien(&state.unidad, &RelojSistema, &en.ctx, q.corte).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Cuadre de la cartera contra el mayor (cuentas que debita la regla `ph.liquidacion`).
#[utoipa::path(
    get,
    path = "/cartera/cuadre",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Cuadre por cuenta", body = Vec<CuadreCuentaRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin regla vigente de la liquidación (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn cuadre(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<CuadreCuentaRespuesta>>> {
    let v = rec::cuadre_cartera(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}
