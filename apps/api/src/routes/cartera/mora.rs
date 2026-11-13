//! Mora, acuerdos, edades y certificados (plan 14 fase 5): tasas de referencia (plataforma),
//! causación de intereses, acuerdos de pago, cartera por edades, paz y salvo y certificados de
//! deuda con su verificación pública.

use application::{
    casos_uso::contabilidad_ph::mora::{self as uc, NuevaTasa, NuevoAcuerdo},
    puertos::{Reloj, RelojSistema},
};
use axum::{extract::State, http::StatusCode};
use chrono::NaiveDate;
use domain::{AcuerdoPagoId, BienPrivadoId, CausacionMoraId, TenantId};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::*;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    limite::LimitePublico,
    openapi::{ETIQUETA_CARTERA, ETIQUETA_PLATAFORMA},
    sesion::{EnCopropiedad, OperadorPlataforma},
    state::AppState,
};

/// Rutas de la mora en la copropiedad.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(tasas))
        .routes(routes!(vista_previa_intereses))
        .routes(routes!(listar_causaciones, causar))
        .routes(routes!(ver_causacion))
        .routes(routes!(anular_causacion))
        .routes(routes!(listar_acuerdos, proponer_acuerdo))
        .routes(routes!(aprobar_acuerdo))
        .routes(routes!(anular_acuerdo))
        .routes(routes!(edades))
        .routes(routes!(listar_certificados, emitir_certificado))
        .routes(routes!(verificar))
}

/// Rutas de plataforma (se montan en `/api/v1/plataforma`).
pub fn rutas_plataforma() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(tasas_plataforma, crear_tasa))
}

/// Tasas de referencia (interés bancario corriente) por vigencia.
#[utoipa::path(
    get,
    path = "/tasas-referencia",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Tasas", body = Vec<TasaReferenciaRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar`", body = ErrorRespuesta),
    )
)]
pub async fn tasas_plataforma(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
) -> ApiResult<Json<Vec<TasaReferenciaRespuesta>>> {
    let v = uc::tasas_referencia_plataforma(&state.plataforma, &op.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Registra el interés bancario corriente certificado de un periodo.
#[utoipa::path(
    post,
    path = "/tasas-referencia",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevaTasaPeticion,
    responses(
        (status = CREATED, description = "Tasa registrada", body = TasaReferenciaRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar`", body = ErrorRespuesta),
        (status = CONFLICT, description = "Se solapa con otra tasa (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Tasa o fechas inválidas (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_tasa(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevaTasaPeticion>,
) -> ApiResult<(StatusCode, Json<TasaReferenciaRespuesta>)> {
    let t = uc::crear_tasa_referencia(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        NuevaTasa {
            vigente_desde: p.vigente_desde,
            vigente_hasta: p.vigente_hasta,
            tasa_efectiva_anual: p.tasa_efectiva_anual,
            resolucion: p.resolucion,
            fuente: p.fuente,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(t.into())))
}

/// Tasas de referencia por vigencia (las que usa la causación de intereses).
#[utoipa::path(
    get,
    path = "/cartera/tasas-referencia",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Tasas", body = Vec<TasaReferenciaRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn tasas(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<TasaReferenciaRespuesta>>> {
    let v = uc::tasas_referencia(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Calcula los intereses de mora hasta la fecha sin guardar nada.
#[utoipa::path(
    post,
    path = "/cartera/intereses/vista-previa",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = HastaPeticion,
    responses(
        (status = OK, description = "Intereses calculados", body = CalculoMoraRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.liquidar` o sin los módulos", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin política o sin tasa de referencia para algún día (`validacion`, `regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn vista_previa_intereses(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<HastaPeticion>,
) -> ApiResult<Json<CalculoMoraRespuesta>> {
    let c = uc::vista_previa_intereses(&state.unidad, &RelojSistema, &en.ctx, p.hasta).await?;
    Ok(Json(c.into()))
}

/// Causaciones de intereses por periodo descendente.
#[utoipa::path(
    get,
    path = "/cartera/intereses",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Causaciones", body = Vec<CausacionRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn listar_causaciones(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<CausacionRespuesta>>> {
    let v = uc::causaciones(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Causa y contabiliza los intereses de mora hasta la fecha (evento `ph.intereses`).
#[utoipa::path(
    post,
    path = "/cartera/intereses",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = HastaPeticion,
    responses(
        (status = CREATED, description = "Intereses causados", body = CausadoRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.liquidar` o sin los módulos", body = ErrorRespuesta),
        (status = CONFLICT, description = "El periodo ya tiene causación (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin política, concepto de intereses, tasa o regla; nada que causar (`validacion`, `regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn causar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<HastaPeticion>,
) -> ApiResult<(StatusCode, Json<CausadoRespuesta>)> {
    let c = uc::causar_intereses(&state.unidad, &RelojSistema, &en.ctx, p.hasta).await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Causación con el detalle de cada tramo (base, días y tasa).
#[utoipa::path(
    get,
    path = "/cartera/intereses/{causacion_id}",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("causacion_id" = Uuid, Path, description = "Causación")),
    responses(
        (status = OK, description = "Causación", body = DetalleCausacionRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Causación inexistente", body = ErrorRespuesta),
    )
)]
pub async fn ver_causacion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DetalleCausacionRespuesta>> {
    let (c, detalle) = uc::causacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CausacionMoraId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(DetalleCausacionRespuesta {
        causacion: c.into(),
        detalle: detalle.into_iter().map(Into::into).collect(),
    }))
}

/// Anula una causación sin pagos ni ajustes sobre sus intereses.
#[utoipa::path(
    post,
    path = "/cartera/intereses/{causacion_id}/anulacion",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("causacion_id" = Uuid, Path, description = "Causación")),
    request_body = MotivoPeticion,
    responses(
        (status = OK, description = "Causación anulada", body = CausacionRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.liquidar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Causación inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya anulada o con pagos o ajustes (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn anular_causacion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<MotivoPeticion>,
) -> ApiResult<Json<CausacionRespuesta>> {
    let c = uc::anular_causacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CausacionMoraId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Filtro de acuerdos.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroAcuerdosParametros {
    pub bien_privado_id: Option<Uuid>,
}

/// Acuerdos de pago (del bien, si se da) con su situación hoy.
#[utoipa::path(
    get,
    path = "/cartera/acuerdos",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroAcuerdosParametros),
    responses(
        (status = OK, description = "Acuerdos", body = Vec<AcuerdoRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn listar_acuerdos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroAcuerdosParametros>,
) -> ApiResult<Json<Vec<AcuerdoRespuesta>>> {
    let v = uc::acuerdos(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        f.bien_privado_id.map(BienPrivadoId::from_uuid),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Propone un acuerdo de pago por todo lo que debe el bien (lo aprueba otra cuenta).
#[utoipa::path(
    post,
    path = "/cartera/acuerdos",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoAcuerdoPeticion,
    responses(
        (status = CREATED, description = "Acuerdo propuesto", body = AcuerdoRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ajustar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Bien inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "El bien ya tiene un acuerdo activo (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "El bien no debe, sin propietario o cuotas inválidas (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn proponer_acuerdo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoAcuerdoPeticion>,
) -> ApiResult<(StatusCode, Json<AcuerdoRespuesta>)> {
    let a = uc::proponer_acuerdo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoAcuerdo {
            bien: BienPrivadoId::from_uuid(p.bien_privado_id),
            numero_cuotas: p.numero_cuotas,
            primer_vencimiento: p.primer_vencimiento,
            fecha: p.fecha,
            observaciones: p.observaciones,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(a.into())))
}

/// Aprueba un acuerdo propuesto por otra cuenta (suspende la mora).
#[utoipa::path(
    post,
    path = "/cartera/acuerdos/{acuerdo_id}/aprobacion",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("acuerdo_id" = Uuid, Path, description = "Acuerdo")),
    responses(
        (status = OK, description = "Acuerdo vigente", body = AcuerdoRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.liquidar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Acuerdo inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Lo propuso la misma cuenta (`conflicto`) o no está propuesto (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn aprobar_acuerdo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<AcuerdoRespuesta>> {
    let a = uc::aprobar_acuerdo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        AcuerdoPagoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(a.into()))
}

/// Anula un acuerdo propuesto o vigente (la mora se reanuda).
#[utoipa::path(
    post,
    path = "/cartera/acuerdos/{acuerdo_id}/anulacion",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("acuerdo_id" = Uuid, Path, description = "Acuerdo")),
    responses(
        (status = OK, description = "Acuerdo anulado", body = AcuerdoRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.liquidar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Acuerdo inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya cerrado (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn anular_acuerdo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<AcuerdoRespuesta>> {
    let a = uc::anular_acuerdo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        AcuerdoPagoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(a.into()))
}

/// Fecha de corte.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CorteEdadesParametros {
    /// Por defecto, hoy.
    pub corte: Option<NaiveDate>,
}

/// Cartera por edades (0–30, 31–60, 61–90, más de 90 días) por bien, con el total.
#[utoipa::path(
    get,
    path = "/cartera/edades",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), CorteEdadesParametros),
    responses(
        (status = OK, description = "Edades", body = CarteraEdadesRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn edades(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(q): Query<CorteEdadesParametros>,
) -> ApiResult<Json<CarteraEdadesRespuesta>> {
    let corte = q.corte.unwrap_or_else(|| RelojSistema.ahora().date_naive());
    let (filas, total) =
        uc::cartera_por_edades(&state.unidad, &RelojSistema, &en.ctx, Some(corte)).await?;
    Ok(Json(CarteraEdadesRespuesta {
        corte,
        bienes: filas.into_iter().map(Into::into).collect(),
        total: total.into(),
    }))
}

/// Certificados emitidos de un bien.
#[utoipa::path(
    get,
    path = "/cartera/bienes/{bien_privado_id}/certificados",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("bien_privado_id" = Uuid, Path, description = "Bien privado")),
    responses(
        (status = OK, description = "Certificados", body = Vec<CertificadoRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn listar_certificados(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<CertificadoRespuesta>>> {
    let v = uc::certificados(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Emite un paz y salvo (solo si el bien no debe) o un certificado de deuda. El código de
/// verificación y el PDF se entregan una sola vez.
#[utoipa::path(
    post,
    path = "/cartera/bienes/{bien_privado_id}/certificados",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("bien_privado_id" = Uuid, Path, description = "Bien privado")),
    request_body = NuevoCertificadoPeticion,
    responses(
        (status = CREATED, description = "Certificado emitido", body = CertificadoEmitidoRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.certificados.emitir` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Bien inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Paz y salvo de un bien con saldo (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn emitir_certificado(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<NuevoCertificadoPeticion>,
) -> ApiResult<(StatusCode, Json<CertificadoEmitidoRespuesta>)> {
    let c = uc::emitir_certificado(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
        p.tipo,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Verifica el código de un paz y salvo o certificado de deuda (sin sesión: el código es la
/// credencial; límite por IP). Solo dice si es válido, su clase y su corte.
#[utoipa::path(
    post,
    path = "/certificados/verificacion",
    tag = ETIQUETA_CARTERA,
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = VerificacionPeticion,
    responses(
        (status = OK, description = "Resultado", body = VerificacionRespuesta),
        (status = TOO_MANY_REQUESTS, description = "Demasiadas peticiones (`demasiadas_peticiones`)", body = ErrorRespuesta),
    )
)]
pub async fn verificar(
    _limite: LimitePublico,
    State(state): State<AppState>,
    Path(copropiedad): Path<Uuid>,
    Json(p): Json<VerificacionPeticion>,
) -> ApiResult<Json<VerificacionRespuesta>> {
    let v = uc::verificar_certificado(&state.unidad, TenantId::from_uuid(copropiedad), &p.codigo)
        .await?;
    Ok(Json(VerificacionRespuesta {
        valido: v.is_some(),
        tipo: v.map(|x| x.tipo),
        corte: v.map(|x| x.corte),
    }))
}
