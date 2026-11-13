//! Reportes contables en JSON o CSV (`formato=csv`). Cada CSV queda en la auditoría con su
//! filtro y su huella SHA-256 (cabecera `X-Contenido-Sha256`).

use application::{
    casos_uso::contabilidad::reportes as uc,
    puertos::{RelojSistema, contabilidad::AgrupacionSaldos},
};
use axum::{
    extract::State,
    http::{HeaderName, HeaderValue, header},
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use domain::PersonaId;
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::*;
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Query},
    openapi::ETIQUETA_CONTABILIDAD,
    sesion::EnEntidad,
    state::AppState,
};

const CABECERA_HUELLA: HeaderName = HeaderName::from_static("x-contenido-sha256");

/// Rutas de reportes.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(balance_prueba))
        .routes(routes!(libro_diario))
        .routes(routes!(libro_mayor))
        .routes(routes!(estados_financieros))
}

fn es_csv(formato: Option<&str>) -> ApiResult<bool> {
    match formato {
        None | Some("json") => Ok(false),
        Some("csv") => Ok(true),
        Some(_) => Err(ApiError::PeticionInvalida("formato: json o csv".into())),
    }
}

async fn responder_csv(
    state: &AppState,
    en: &EnEntidad,
    libro: &'static str,
    filtro: String,
    encabezado: &[&str],
    filas: Vec<Vec<String>>,
) -> ApiResult<Response> {
    let contenido = uc::a_csv(encabezado, &filas);
    let huella = uc::registrar_exportacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        libro,
        &filtro,
        filas.len(),
        &contenido,
    )
    .await?;
    let disposicion = format!("attachment; filename=\"{libro}.csv\"");
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/csv; charset=utf-8"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&disposicion).map_err(|e| ApiError::Interno(e.into()))?,
            ),
            (
                CABECERA_HUELLA,
                HeaderValue::from_str(&huella).map_err(|e| ApiError::Interno(e.into()))?,
            ),
        ],
        contenido,
    )
        .into_response())
}

fn texto<T: ToString>(v: Option<T>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

/// Filtro del balance de prueba.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroBalance {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    /// Una fila por tercero.
    #[serde(default)]
    pub por_tercero: bool,
    /// Una fila por centro de costo.
    #[serde(default)]
    pub por_centro_costo: bool,
    /// `json` (por defecto) o `csv` (exige `contabilidad.exportar`).
    pub formato: Option<String>,
}

/// Balance de prueba entre dos fechas (por cuenta, tercero y centro de costo).
#[utoipa::path(
    get,
    path = "/reportes/balance-prueba",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroBalance),
    responses(
        (status = OK, description = "Balance de prueba (JSON o `text/csv`)", body = Vec<FilaSaldoRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` (o `contabilidad.exportar` para CSV) o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn balance_prueba(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroBalance>,
) -> ApiResult<Response> {
    let csv = es_csv(f.formato.as_deref())?;
    let agrupacion = AgrupacionSaldos {
        por_tercero: f.por_tercero,
        por_centro_costo: f.por_centro_costo,
        sin_cierre_anual: false,
    };
    let filas: Vec<FilaSaldoRespuesta> = uc::balance_prueba(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        f.desde,
        f.hasta,
        agrupacion,
    )
    .await?
    .into_iter()
    .map(Into::into)
    .collect();
    if !csv {
        return Ok(Json(filas).into_response());
    }
    let datos = filas
        .iter()
        .map(|r| {
            vec![
                r.codigo.clone(),
                r.nombre.clone(),
                texto(r.persona_id),
                texto(r.centro_costo_id),
                r.saldo_inicial.to_string(),
                r.debitos.to_string(),
                r.creditos.to_string(),
                r.saldo_final.to_string(),
            ]
        })
        .collect();
    let filtro = format!(
        "{} a {}, tercero={}, centro={}",
        f.desde, f.hasta, f.por_tercero, f.por_centro_costo
    );
    responder_csv(
        &state,
        &en,
        "balance_prueba",
        filtro,
        &[
            "cuenta",
            "nombre",
            "tercero",
            "centro_costo",
            "saldo_inicial",
            "debitos",
            "creditos",
            "saldo_final",
        ],
        datos,
    )
    .await
}

/// Rango y formato de un libro.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroLibro {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    /// `json` (por defecto) o `csv` (exige `contabilidad.exportar`).
    pub formato: Option<String>,
}

/// Libro diario: comprobantes contabilizados con sus líneas.
#[utoipa::path(
    get,
    path = "/reportes/libro-diario",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroLibro),
    responses(
        (status = OK, description = "Libro diario (JSON o `text/csv`)", body = Vec<ComprobanteRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` (o `contabilidad.exportar` para CSV) o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn libro_diario(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroLibro>,
) -> ApiResult<Response> {
    let csv = es_csv(f.formato.as_deref())?;
    let comprobantes: Vec<ComprobanteRespuesta> =
        uc::libro_diario(&state.unidad, &RelojSistema, &en.ctx, f.desde, f.hasta)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    if !csv {
        return Ok(Json(comprobantes).into_response());
    }
    let datos = comprobantes
        .iter()
        .flat_map(|c| {
            c.lineas.iter().map(move |l| {
                vec![
                    c.fecha.to_string(),
                    c.tipo.clone(),
                    texto(c.numero),
                    c.descripcion.clone(),
                    l.cuenta_id.to_string(),
                    texto(l.persona_id),
                    texto(l.centro_costo_id),
                    l.debito.to_string(),
                    l.credito.to_string(),
                ]
            })
        })
        .collect();
    responder_csv(
        &state,
        &en,
        "libro_diario",
        format!("{} a {}", f.desde, f.hasta),
        &[
            "fecha",
            "tipo",
            "numero",
            "descripcion",
            "cuenta_id",
            "tercero",
            "centro_costo",
            "debito",
            "credito",
        ],
        datos,
    )
    .await
}

/// Cuenta (y tercero) del libro mayor.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroMayor {
    #[param(example = "110505")]
    pub cuenta: String,
    /// Con tercero: el auxiliar de ese tercero.
    pub persona_id: Option<Uuid>,
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    /// `json` (por defecto) o `csv` (exige `contabilidad.exportar`).
    pub formato: Option<String>,
}

/// Libro mayor de una cuenta (o auxiliar de un tercero).
#[utoipa::path(
    get,
    path = "/reportes/libro-mayor",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroMayor),
    responses(
        (status = OK, description = "Libro mayor (JSON o `text/csv`)", body = MayorRespuesta),
        (status = NOT_FOUND, description = "La cuenta no existe", body = ErrorRespuesta),
    )
)]
pub async fn libro_mayor(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroMayor>,
) -> ApiResult<Response> {
    let csv = es_csv(f.formato.as_deref())?;
    let cuenta = codigo(&f.cuenta)?;
    let mayor: MayorRespuesta = uc::libro_mayor(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        &cuenta,
        f.persona_id.map(PersonaId::from_uuid),
        f.desde,
        f.hasta,
    )
    .await?
    .into();
    if !csv {
        return Ok(Json(mayor).into_response());
    }
    let mut saldo = mayor.saldo_inicial;
    let datos = mayor
        .movimientos
        .iter()
        .map(|m| {
            saldo += m.debito - m.credito;
            vec![
                m.fecha.to_string(),
                m.tipo.clone(),
                m.numero.to_string(),
                m.descripcion.clone(),
                texto(m.persona_id),
                m.debito.to_string(),
                m.credito.to_string(),
                saldo.to_string(),
            ]
        })
        .collect();
    let filtro = format!(
        "cuenta {} {} a {}, tercero={}",
        f.cuenta,
        f.desde,
        f.hasta,
        texto(f.persona_id)
    );
    responder_csv(
        &state,
        &en,
        "libro_mayor",
        filtro,
        &[
            "fecha",
            "tipo",
            "numero",
            "descripcion",
            "tercero",
            "debito",
            "credito",
            "saldo",
        ],
        datos,
    )
    .await
}

/// Fecha de corte.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroEstados {
    pub corte: NaiveDate,
}

/// Estados financieros (situación financiera y resultados) a la fecha de corte, con notas.
#[utoipa::path(
    get,
    path = "/reportes/estados-financieros",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroEstados),
    responses(
        (status = OK, description = "Estados financieros", body = EstadosFinancierosRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn estados_financieros(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroEstados>,
) -> ApiResult<Json<EstadosFinancierosRespuesta>> {
    Ok(Json(
        uc::estados_financieros(&state.unidad, &RelojSistema, &en.ctx, f.corte)
            .await?
            .into(),
    ))
}
