//! Liquidación de expensas (plan 14 fase 3): cuotas extraordinarias, vista previa, liquidación,
//! anulación, cuentas de cobro, su PDF y su envío.

use application::{
    casos_uso::contabilidad_ph::liquidacion::{self as liq, NuevaCuota},
    puertos::RelojSistema,
};
use axum::{
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use domain::{
    BienPrivadoId, ConceptoCobroId, CuentaCobroId, CuotaExtraordinariaId, LiquidacionExpensasId,
    PreguntaId,
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
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de la liquidación.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_cuotas, crear_cuota))
        .routes(routes!(anular_cuota))
        .routes(routes!(vista_previa))
        .routes(routes!(listar_liquidaciones, liquidar))
        .routes(routes!(ver_liquidacion))
        .routes(routes!(anular_liquidacion))
        .routes(routes!(enviar_cuentas))
        .routes(routes!(listar_cuentas_cobro))
        .routes(routes!(cuenta_cobro_pdf))
}

/// Cuotas extraordinarias, las más recientes primero.
#[utoipa::path(
    get,
    path = "/cartera/cuotas-extraordinarias",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Cuotas", body = Vec<CuotaExtraordinariaRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn listar_cuotas(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<CuotaExtraordinariaRespuesta>>> {
    let v = liq::cuotas_extraordinarias(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Registra una cuota extraordinaria aprobada: se cobra en las liquidaciones de su calendario.
#[utoipa::path(
    post,
    path = "/cartera/cuotas-extraordinarias",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevaCuotaPeticion,
    responses(
        (status = CREATED, description = "Cuota registrada", body = CuotaExtraordinariaRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.liquidar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Concepto inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Concepto no extraordinario, valor o número de cuotas inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_cuota(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevaCuotaPeticion>,
) -> ApiResult<(StatusCode, Json<CuotaExtraordinariaRespuesta>)> {
    let c = liq::crear_cuota_extraordinaria(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevaCuota {
            concepto: ConceptoCobroId::from_uuid(p.concepto_cobro_id),
            finalidad: p.finalidad,
            valor_total: p.valor_total,
            numero_cuotas: p.numero_cuotas,
            primer_periodo: p.primer_periodo,
            destinacion_especifica: p.destinacion_especifica,
            pregunta: p.pregunta_id.map(PreguntaId::from_uuid),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Anula una cuota extraordinaria: las liquidaciones siguientes ya no la cobran.
#[utoipa::path(
    post,
    path = "/cartera/cuotas-extraordinarias/{cuota_id}/anulacion",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("cuota_id" = Uuid, Path, description = "Cuota")),
    responses(
        (status = NO_CONTENT, description = "Cuota anulada"),
        (status = FORBIDDEN, description = "Sin `cartera.liquidar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Cuota inexistente o ya anulada", body = ErrorRespuesta),
    )
)]
pub async fn anular_cuota(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    liq::anular_cuota_extraordinaria(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CuotaExtraordinariaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Calcula la liquidación del periodo sin guardar nada.
#[utoipa::path(
    post,
    path = "/cartera/liquidaciones/vista-previa",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = PeriodoPeticion,
    responses(
        (status = OK, description = "Liquidación calculada", body = VistaLiquidacionRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.liquidar` o sin los módulos", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin política, presupuesto o cuadro vigentes, o un concepto no se puede repartir (`validacion`, `regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn vista_previa(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<PeriodoPeticion>,
) -> ApiResult<Json<VistaLiquidacionRespuesta>> {
    let v = liq::vista_previa_liquidacion(&state.unidad, &RelojSistema, &en.ctx, p.periodo).await?;
    Ok(Json(v.into()))
}

/// Liquidaciones por periodo descendente.
#[utoipa::path(
    get,
    path = "/cartera/liquidaciones",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Liquidaciones", body = Vec<LiquidacionRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn listar_liquidaciones(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<LiquidacionRespuesta>>> {
    let v = liq::liquidaciones(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Liquida y contabiliza el periodo: obligaciones de cada bien, comprobante (evento
/// `ph.liquidacion`), una cuenta de cobro por bien y la aplicación de los anticipos.
#[utoipa::path(
    post,
    path = "/cartera/liquidaciones",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = PeriodoPeticion,
    responses(
        (status = CREATED, description = "Liquidación contabilizada", body = LiquidadoRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.liquidar` o sin los módulos", body = ErrorRespuesta),
        (status = CONFLICT, description = "El periodo ya está liquidado (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin política, presupuesto, cuadro o regla; bienes sin propietario; periodo contable no abierto (`validacion`, `regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn liquidar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<PeriodoPeticion>,
) -> ApiResult<(StatusCode, Json<LiquidadoRespuesta>)> {
    let l = liq::liquidar_expensas(&state.unidad, &RelojSistema, &en.ctx, p.periodo).await?;
    Ok((StatusCode::CREATED, Json(l.into())))
}

/// Liquidación con sus obligaciones y su saldo actual.
#[utoipa::path(
    get,
    path = "/cartera/liquidaciones/{liquidacion_id}",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("liquidacion_id" = Uuid, Path, description = "Liquidación")),
    responses(
        (status = OK, description = "Liquidación", body = DetalleLiquidacionRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Liquidación inexistente", body = ErrorRespuesta),
    )
)]
pub async fn ver_liquidacion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DetalleLiquidacionRespuesta>> {
    let (l, obligaciones) = liq::liquidacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        LiquidacionExpensasId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(DetalleLiquidacionRespuesta {
        liquidacion: l.into(),
        obligaciones: obligaciones.into_iter().map(Into::into).collect(),
    }))
}

/// Anula una liquidación sin pagos ni ajustes (revierte su comprobante; los cargos de reservas
/// vuelven a pendientes). Con pagos se corrige con notas crédito.
#[utoipa::path(
    post,
    path = "/cartera/liquidaciones/{liquidacion_id}/anulacion",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("liquidacion_id" = Uuid, Path, description = "Liquidación")),
    request_body = MotivoPeticion,
    responses(
        (status = OK, description = "Liquidación anulada", body = LiquidacionRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.liquidar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Liquidación inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya anulada o con pagos o ajustes (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn anular_liquidacion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<MotivoPeticion>,
) -> ApiResult<Json<LiquidacionRespuesta>> {
    let l = liq::anular_liquidacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        LiquidacionExpensasId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(Json(l.into()))
}

/// Envía por correo cada cuenta de cobro de la liquidación a su pagador.
#[utoipa::path(
    post,
    path = "/cartera/liquidaciones/{liquidacion_id}/envio",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("liquidacion_id" = Uuid, Path, description = "Liquidación")),
    responses(
        (status = OK, description = "Cuentas encoladas", body = EnvioCuentasRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.liquidar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Liquidación inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Liquidación anulada (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn enviar_cuentas(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<EnvioCuentasRespuesta>> {
    let e = liq::enviar_cuentas_cobro(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        LiquidacionExpensasId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(e.into()))
}

/// Filtro de cuentas de cobro.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroCuentasCobroParametros {
    pub liquidacion_id: Option<Uuid>,
    pub bien_privado_id: Option<Uuid>,
}

/// Cuentas de cobro de una liquidación o de un bien, por número.
#[utoipa::path(
    get,
    path = "/cartera/cuentas-cobro",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroCuentasCobroParametros),
    responses(
        (status = OK, description = "Cuentas de cobro", body = Vec<CuentaCobroRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn listar_cuentas_cobro(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroCuentasCobroParametros>,
) -> ApiResult<Json<Vec<CuentaCobroRespuesta>>> {
    let v = liq::cuentas_cobro(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        f.liquidacion_id.map(LiquidacionExpensasId::from_uuid),
        f.bien_privado_id.map(BienPrivadoId::from_uuid),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// La cuenta de cobro en PDF.
#[utoipa::path(
    get,
    path = "/cartera/cuentas-cobro/{cuenta_cobro_id}/pdf",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("cuenta_cobro_id" = Uuid, Path, description = "Cuenta de cobro")),
    responses(
        (status = OK, description = "PDF de la cuenta de cobro", content_type = "application/pdf", body = Vec<u8>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Cuenta de cobro inexistente", body = ErrorRespuesta),
    )
)]
pub async fn cuenta_cobro_pdf(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    let pdf = liq::cuenta_cobro_pdf(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        &en.ctx,
        CuentaCobroId::from_uuid(ruta.1),
    )
    .await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"cuenta-cobro-{}.pdf\"", ruta.1),
            ),
        ],
        pdf,
    )
        .into_response())
}
