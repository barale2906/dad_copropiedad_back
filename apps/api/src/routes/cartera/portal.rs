//! Portal del propietario (`/mi-cartera/…`), integraciones con asambleas y reservas, e informes
//! para el consejo y la asamblea (plan 14 fases 6 y 7).

use application::{
    casos_uso::contabilidad_ph::{informes, integraciones, portal},
    puertos::RelojSistema,
};
use axum::{
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use domain::{BienPrivadoId, CuentaCobroId, ReunionId};
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

/// Rutas del portal, las integraciones y los informes.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(mis_bienes))
        .routes(routes!(mi_estado_cuenta))
        .routes(routes!(mis_cuentas_cobro))
        .routes(routes!(mi_cuenta_cobro_pdf))
        .routes(routes!(mi_paz_y_salvo))
        .routes(routes!(morosos_asamblea))
        .routes(routes!(restricciones_mora))
        .routes(routes!(estado_fondo))
        .routes(routes!(flujo_caja))
        .routes(routes!(proyectar_presupuesto))
        .routes(routes!(informe_asamblea))
}

fn pdf(nombre: String, contenido: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/pdf".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{nombre}\""),
            ),
        ],
        contenido,
    )
        .into_response()
}

/// Bienes de la cuenta (titular vigente o pagador) con sus datos de pago.
#[utoipa::path(
    get,
    path = "/mi-cartera/bienes",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Mis bienes", body = Vec<BienCarteraRespuesta>),
        (status = FORBIDDEN, description = "Sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn mis_bienes(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<BienCarteraRespuesta>>> {
    let v = portal::mis_bienes(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Fecha de corte.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CorteParametros {
    /// Por defecto, hoy.
    pub corte: Option<NaiveDate>,
}

/// Estado de cuenta de un bien propio.
#[utoipa::path(
    get,
    path = "/mi-cartera/bienes/{bien_privado_id}/estado-cuenta",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("bien_privado_id" = Uuid, Path, description = "Bien privado"), CorteParametros),
    responses(
        (status = OK, description = "Estado de cuenta", body = EstadoCuentaRespuesta),
        (status = FORBIDDEN, description = "Sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "El bien no es de la cuenta", body = ErrorRespuesta),
    )
)]
pub async fn mi_estado_cuenta(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(q): Query<CorteParametros>,
) -> ApiResult<Json<EstadoCuentaRespuesta>> {
    let e = portal::mi_estado_cuenta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
        q.corte,
    )
    .await?;
    Ok(Json(e.into()))
}

/// Cuentas de cobro de un bien propio.
#[utoipa::path(
    get,
    path = "/mi-cartera/bienes/{bien_privado_id}/cuentas-cobro",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("bien_privado_id" = Uuid, Path, description = "Bien privado")),
    responses(
        (status = OK, description = "Cuentas de cobro", body = Vec<CuentaCobroRespuesta>),
        (status = FORBIDDEN, description = "Sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "El bien no es de la cuenta", body = ErrorRespuesta),
    )
)]
pub async fn mis_cuentas_cobro(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<CuentaCobroRespuesta>>> {
    let v = portal::mis_cuentas_cobro(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// PDF de una cuenta de cobro de un bien propio.
#[utoipa::path(
    get,
    path = "/mi-cartera/cuentas-cobro/{cuenta_cobro_id}/pdf",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("cuenta_cobro_id" = Uuid, Path, description = "Cuenta de cobro")),
    responses(
        (status = OK, description = "PDF", content_type = "application/pdf", body = Vec<u8>),
        (status = FORBIDDEN, description = "Sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No es de un bien de la cuenta", body = ErrorRespuesta),
    )
)]
pub async fn mi_cuenta_cobro_pdf(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    let contenido = portal::mi_cuenta_cobro_pdf(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        &en.ctx,
        CuentaCobroId::from_uuid(ruta.1),
    )
    .await?;
    Ok(pdf(format!("cuenta-cobro-{}.pdf", ruta.1), contenido))
}

/// El propietario o pagador descarga el paz y salvo de su bien (solo si no debe nada). El
/// código y el PDF se entregan una sola vez.
#[utoipa::path(
    post,
    path = "/mi-cartera/bienes/{bien_privado_id}/paz-y-salvo",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("bien_privado_id" = Uuid, Path, description = "Bien privado")),
    responses(
        (status = CREATED, description = "Paz y salvo", body = CertificadoEmitidoRespuesta),
        (status = FORBIDDEN, description = "Sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "El bien no es de la cuenta", body = ErrorRespuesta),
        (status = CONFLICT, description = "El bien tiene saldo (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn mi_paz_y_salvo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<(StatusCode, Json<CertificadoEmitidoRespuesta>)> {
    let c = portal::mi_paz_y_salvo(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Fija los morosos de una reunión con los bienes que tienen cartera vencida al corte.
#[utoipa::path(
    post,
    path = "/cartera/asambleas/{reunion_id}/morosos",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    request_body = CorteMorososPeticion,
    responses(
        (status = OK, description = "Morosos fijados", body = MorososRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ver`, sin `asambleas.convocar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Reunión inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Reunión instalada o sin padrón (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn morosos_asamblea(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<CorteMorososPeticion>,
) -> ApiResult<Json<MorososRespuesta>> {
    let r = integraciones::morosos_desde_cartera(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(ruta.1),
        p.corte,
    )
    .await?;
    Ok(Json(MorososRespuesta {
        marcados: r.marcados,
        no_encontrados: r.no_encontrados,
    }))
}

/// Restringe las reservas de los bienes con vencido desde el umbral y levanta las que ya no lo
/// superan (solo las que creó la cartera).
#[utoipa::path(
    post,
    path = "/cartera/reservas/restricciones-mora",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = UmbralMoraPeticion,
    responses(
        (status = OK, description = "Restricciones sincronizadas", body = SincronizacionMoraRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ver`, sin `reservas.restringir` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn restricciones_mora(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<UmbralMoraPeticion>,
) -> ApiResult<Json<SincronizacionMoraRespuesta>> {
    let r = integraciones::sincronizar_restricciones_mora(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        p.umbral,
    )
    .await?;
    Ok(Json(SincronizacionMoraRespuesta {
        restringidos: r.restringidos,
        levantadas: r.levantadas,
    }))
}

/// Año del informe.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AnioParametros {
    /// Por defecto, el año en curso.
    pub anio: Option<i32>,
}

/// Estado del fondo de imprevistos frente al presupuesto ordinario del año.
#[utoipa::path(
    get,
    path = "/cartera/informes/fondo",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), AnioParametros),
    responses(
        (status = OK, description = "Estado del fondo", body = EstadoFondoRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin política vigente (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn estado_fondo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(q): Query<AnioParametros>,
) -> ApiResult<Json<EstadoFondoRespuesta>> {
    let f = informes::estado_fondo(&state.unidad, &RelojSistema, &en.ctx, q.anio).await?;
    Ok(Json(f.into()))
}

/// Rango de meses del flujo de caja.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RangoMesesParametros {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
}

/// Flujo de caja mensual de las cuentas de tesorería (de 1 a 24 meses).
#[utoipa::path(
    get,
    path = "/cartera/informes/flujo-caja",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), RangoMesesParametros),
    responses(
        (status = OK, description = "Flujo por mes", body = Vec<MesFlujoRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Rango inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn flujo_caja(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(q): Query<RangoMesesParametros>,
) -> ApiResult<Json<Vec<MesFlujoRespuesta>>> {
    let v = informes::flujo_caja(&state.unidad, &RelojSistema, &en.ctx, q.desde, q.hasta).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Proyecto de presupuesto del año siguiente: copia el vigente con un ajuste porcentual (queda en
/// borrador).
#[utoipa::path(
    post,
    path = "/cartera/presupuestos/proyecto",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = ProyectoPresupuestoPeticion,
    responses(
        (status = CREATED, description = "Borrador del año siguiente", body = PresupuestoRespuesta),
        (status = FORBIDDEN, description = "Sin `presupuesto.gestionar` o sin los módulos", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin presupuesto vigente o ajuste fuera de rango (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn proyectar_presupuesto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<ProyectoPresupuestoPeticion>,
) -> ApiResult<(StatusCode, Json<PresupuestoRespuesta>)> {
    let v = informes::proyectar_presupuesto(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        p.anio_base,
        p.ajuste_porcentual,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(v.into())))
}

/// Año y mes de corte del informe para la asamblea.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InformeAsambleaParametros {
    pub anio: i32,
    /// Mes del corte (1 a 12).
    pub hasta_mes: u32,
}

/// Paquete en PDF para la asamblea: ejecución presupuestal, cartera por edades y fondo.
#[utoipa::path(
    get,
    path = "/cartera/informes/asamblea",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), InformeAsambleaParametros),
    responses(
        (status = OK, description = "PDF del informe", content_type = "application/pdf", body = Vec<u8>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin presupuesto vigente o mes inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn informe_asamblea(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(q): Query<InformeAsambleaParametros>,
) -> ApiResult<Response> {
    let contenido = informes::informe_asamblea_pdf(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        &en.ctx,
        q.anio,
        q.hasta_mes,
    )
    .await?;
    Ok(pdf(format!("informe-asamblea-{}.pdf", q.anio), contenido))
}
