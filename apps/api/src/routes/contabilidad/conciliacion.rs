//! Extractos bancarios y conciliación (plan 13 fase 3). El archivo del extracto viaja en Base64
//! dentro del JSON, con el mapeo de columnas del banco.

use application::{
    AppError,
    casos_uso::contabilidad::conciliacion::{self as uc, FormatoEntrada, ImportacionExtracto},
    puertos::RelojSistema,
};
use axum::{extract::State, http::StatusCode};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::NaiveDate;
use domain::{
    ComprobanteId, ConciliacionId, CuentaTesoreriaId, MovimientoExtractoId, NombreArchivo,
    contabilidad::tesoreria::{EstadoMovimientoExtracto, FormatoExtracto},
};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto_tesoreria::*;
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_TESORERIA,
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas de extractos y conciliación.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(previsualizar_extracto))
        .routes(routes!(listar_extractos, importar_extracto))
        .routes(routes!(listar_movimientos))
        .routes(routes!(listar_libros))
        .routes(routes!(conciliar_automaticamente))
        .routes(routes!(listar_conciliaciones, conciliar_manual))
        .routes(routes!(deshacer_conciliacion))
        .routes(routes!(marcar_partida))
        .routes(routes!(recibo_no_identificado))
        .routes(routes!(informe))
}

fn validacion(texto: &str) -> ApiError {
    ApiError::App(AppError::Validacion(texto.to_owned()))
}

/// Contenido y formato del extracto de la petición.
fn archivo_y_formato(p: &mut ExtractoPeticion) -> ApiResult<(Vec<u8>, FormatoEntrada)> {
    let contenido = STANDARD
        .decode(p.archivo_base64.trim())
        .map_err(|_| validacion("`archivo_base64` no es Base64 válido"))?;
    let formato = match p.formato {
        FormatoExtracto::Planilla => FormatoEntrada::Planilla(
            p.mapeo
                .take()
                .ok_or_else(|| validacion("una planilla necesita el mapeo de columnas"))?
                .into(),
        ),
        FormatoExtracto::Asobancaria2001 => FormatoEntrada::Asobancaria2001,
    };
    Ok((contenido, formato))
}

/// Lee el extracto y muestra lo que se importaría, con los problemas por fila (no guarda nada).
#[utoipa::path(
    post,
    path = "/tesoreria/cuentas/{cuenta_id}/extractos/vista-previa",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("cuenta_id" = Uuid, Path, description = "Cuenta de tesorería")),
    request_body = ExtractoPeticion,
    responses(
        (status = OK, description = "Vista previa", body = VistaPreviaExtractoRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.conciliar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Archivo o mapeo inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn previsualizar_extracto(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(_ruta): Path<(Uuid, Uuid)>,
    Json(mut p): Json<ExtractoPeticion>,
) -> ApiResult<Json<VistaPreviaExtractoRespuesta>> {
    let (contenido, formato) = archivo_y_formato(&mut p)?;
    let leido = uc::previsualizar_extracto(
        &state.unidad,
        &state.planillas,
        &RelojSistema,
        &en.ctx,
        &contenido,
        &formato,
    )
    .await?;
    Ok(Json(leido.into()))
}

/// Extractos importados en la cuenta, más recientes primero.
#[utoipa::path(
    get,
    path = "/tesoreria/cuentas/{cuenta_id}/extractos",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("cuenta_id" = Uuid, Path, description = "Cuenta de tesorería")),
    responses(
        (status = OK, description = "Extractos", body = Vec<ExtractoRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_extractos(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<ExtractoRespuesta>>> {
    let v = uc::extractos(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CuentaTesoreriaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Importa el extracto (el archivo queda como soporte). El mismo archivo no se importa dos
/// veces; un extracto con filas inválidas se rechaza (revíselo con la vista previa).
#[utoipa::path(
    post,
    path = "/tesoreria/cuentas/{cuenta_id}/extractos",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("cuenta_id" = Uuid, Path, description = "Cuenta de tesorería")),
    request_body = ExtractoPeticion,
    responses(
        (status = CREATED, description = "Extracto importado", body = ExtractoRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.conciliar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya se importó en la cuenta (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Filas inválidas o sin movimientos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn importar_extracto(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(mut p): Json<ExtractoPeticion>,
) -> ApiResult<(StatusCode, Json<ExtractoRespuesta>)> {
    let (contenido, formato) = archivo_y_formato(&mut p)?;
    let nombre = NombreArchivo::new(&p.nombre_archivo).map_err(|e| ApiError::App(e.into()))?;
    let e = uc::importar_extracto(
        &state.unidad,
        state.archivos.as_ref(),
        &state.planillas,
        &RelojSistema,
        &en.ctx,
        ImportacionExtracto {
            cuenta_tesoreria: CuentaTesoreriaId::from_uuid(ruta.1),
            nombre,
            contenido,
            formato,
            saldo_inicial: p.saldo_inicial,
            saldo_final: p.saldo_final,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(e.into())))
}

/// Rango y estado de los movimientos.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroMovimientosParametros {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    #[param(value_type = Option<crate::esquemas::EstadoMovimientoExtracto>)]
    pub estado: Option<EstadoMovimientoExtracto>,
}

/// Movimientos del extracto de la cuenta entre dos fechas.
#[utoipa::path(
    get,
    path = "/tesoreria/cuentas/{cuenta_id}/movimientos",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("cuenta_id" = Uuid, Path, description = "Cuenta de tesorería"), FiltroMovimientosParametros),
    responses(
        (status = OK, description = "Movimientos", body = Vec<MovimientoExtractoRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_movimientos(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(f): Query<FiltroMovimientosParametros>,
) -> ApiResult<Json<Vec<MovimientoExtractoRespuesta>>> {
    let v = uc::movimientos(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CuentaTesoreriaId::from_uuid(ruta.1),
        f.desde,
        f.hasta,
        f.estado,
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Rango de las líneas de libros.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroLibrosParametros {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    /// Solo las no conciliadas.
    #[serde(default)]
    pub solo_pendientes: bool,
}

/// Líneas contabilizadas de la cuenta contable del banco entre dos fechas.
#[utoipa::path(
    get,
    path = "/tesoreria/cuentas/{cuenta_id}/libros",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("cuenta_id" = Uuid, Path, description = "Cuenta de tesorería"), FiltroLibrosParametros),
    responses(
        (status = OK, description = "Líneas", body = Vec<LineaLibrosRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_libros(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(f): Query<FiltroLibrosParametros>,
) -> ApiResult<Json<Vec<LineaLibrosRespuesta>>> {
    let v = uc::lineas_libros(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CuentaTesoreriaId::from_uuid(ruta.1),
        f.desde,
        f.hasta,
        f.solo_pendientes,
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Concilia automáticamente por valor, fecha (con tolerancia) y referencia; lo que no casa queda
/// para la conciliación manual.
#[utoipa::path(
    post,
    path = "/tesoreria/cuentas/{cuenta_id}/conciliacion-automatica",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("cuenta_id" = Uuid, Path, description = "Cuenta de tesorería")),
    request_body = ConciliacionAutomaticaPeticion,
    responses(
        (status = OK, description = "Conciliaciones hechas", body = Vec<ConciliacionRespuesta>),
        (status = FORBIDDEN, description = "Sin `tesoreria.conciliar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Tolerancia mayor a 10 días (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn conciliar_automaticamente(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ConciliacionAutomaticaPeticion>,
) -> ApiResult<Json<Vec<ConciliacionRespuesta>>> {
    let v = uc::conciliar_automaticamente_cuenta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CuentaTesoreriaId::from_uuid(ruta.1),
        p.desde,
        p.hasta,
        p.tolerancia_dias,
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Conciliaciones de la cuenta, más recientes primero.
#[utoipa::path(
    get,
    path = "/tesoreria/cuentas/{cuenta_id}/conciliaciones",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("cuenta_id" = Uuid, Path, description = "Cuenta de tesorería")),
    responses(
        (status = OK, description = "Conciliaciones", body = Vec<ConciliacionRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_conciliaciones(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<ConciliacionRespuesta>>> {
    let v = uc::conciliaciones(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CuentaTesoreriaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Concilia a mano movimientos y líneas de libros cuyas sumas coinciden.
#[utoipa::path(
    post,
    path = "/tesoreria/cuentas/{cuenta_id}/conciliaciones",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("cuenta_id" = Uuid, Path, description = "Cuenta de tesorería")),
    request_body = ConciliacionManualPeticion,
    responses(
        (status = CREATED, description = "Conciliación", body = ConciliacionRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.conciliar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Partida ajena o conciliada (`validacion`) o sumas distintas (`regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn conciliar_manual(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ConciliacionManualPeticion>,
) -> ApiResult<(StatusCode, Json<ConciliacionRespuesta>)> {
    let c = uc::conciliar_manual(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CuentaTesoreriaId::from_uuid(ruta.1),
        p.movimientos
            .into_iter()
            .map(MovimientoExtractoId::from_uuid)
            .collect(),
        p.lineas
            .into_iter()
            .map(|l| (ComprobanteId::from_uuid(l.comprobante_id), l.orden))
            .collect(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Deshace una conciliación con un registro nuevo (sus movimientos vuelven a pendientes).
#[utoipa::path(
    post,
    path = "/tesoreria/conciliaciones/{conciliacion_id}/deshacer",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("conciliacion_id" = Uuid, Path, description = "Conciliación")),
    responses(
        (status = CREATED, description = "Registro de deshacer", body = ConciliacionRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.conciliar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya se deshizo o es un deshacer (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn deshacer_conciliacion(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<(StatusCode, Json<ConciliacionRespuesta>)> {
    let c = uc::deshacer_conciliacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ConciliacionId::from_uuid(ruta.1),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Marca un movimiento pendiente como partida conciliatoria (o lo devuelve a pendiente).
#[utoipa::path(
    put,
    path = "/tesoreria/movimientos/{movimiento_id}/partida-conciliatoria",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("movimiento_id" = Uuid, Path, description = "Movimiento del extracto")),
    request_body = PartidaConciliatoriaPeticion,
    responses(
        (status = OK, description = "Movimiento", body = MovimientoExtractoRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.conciliar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Está conciliado (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn marcar_partida(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PartidaConciliatoriaPeticion>,
) -> ApiResult<Json<MovimientoExtractoRespuesta>> {
    let m = uc::marcar_partida_conciliatoria(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        MovimientoExtractoId::from_uuid(ruta.1),
        p.partida,
    )
    .await?;
    Ok(Json(m.into()))
}

/// Crea un recibo no identificado por un crédito del extracto y lo concilia con él.
#[utoipa::path(
    post,
    path = "/tesoreria/movimientos/{movimiento_id}/recibo",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("movimiento_id" = Uuid, Path, description = "Movimiento del extracto")),
    request_body = ReciboNoIdentificadoPeticion,
    responses(
        (status = CREATED, description = "Recibo no identificado", body = ReciboRespuesta),
        (status = FORBIDDEN, description = "Sin `tesoreria.conciliar` o `tesoreria.recibos.registrar`, o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es un crédito pendiente (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn recibo_no_identificado(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ReciboNoIdentificadoPeticion>,
) -> ApiResult<(StatusCode, Json<ReciboRespuesta>)> {
    let r = uc::recibo_no_identificado(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        MovimientoExtractoId::from_uuid(ruta.1),
        &p.descripcion,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Fecha de corte del informe.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CorteInformeParametros {
    pub corte: NaiveDate,
}

/// Informe de conciliación: saldo del extracto, de libros y partidas conciliatorias.
#[utoipa::path(
    get,
    path = "/tesoreria/cuentas/{cuenta_id}/informe-conciliacion",
    tag = ETIQUETA_TESORERIA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("cuenta_id" = Uuid, Path, description = "Cuenta de tesorería"), CorteInformeParametros),
    responses(
        (status = OK, description = "Informe", body = InformeConciliacionRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn informe(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(q): Query<CorteInformeParametros>,
) -> ApiResult<Json<InformeConciliacionRespuesta>> {
    let i = uc::informe_conciliacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CuentaTesoreriaId::from_uuid(ruta.1),
        q.corte,
    )
    .await?;
    Ok(Json(i.into()))
}
