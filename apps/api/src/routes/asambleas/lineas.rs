//! Líneas de WhatsApp y tarifas de Meta (`/api/v1/plataforma/whatsapp`, exige segundo factor y
//! `plataforma.whatsapp.gestionar`).
//!
//! Todas las líneas de una misma cuenta de WhatsApp Business (WABA) se usan con la misma app y
//! el mismo token (`WHATSAPP_ACCESS_TOKEN`); cada una se identifica por su `phone_number_id`.
//! Las conversaciones nuevas (plantillas de asambleas y avisos de tickets) salen por la activa
//! con más franquicia de servicio libre este mes.

use application::{
    casos_uso::asambleas::lineas as uc,
    puertos::asambleas::{EstadoLinea, LineaWhatsapp, TarifaWhatsapp},
};
use axum::{extract::State, http::StatusCode};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path},
    openapi::ETIQUETA_PLATAFORMA,
    sesion::OperadorPlataforma,
    state::AppState,
};

/// Rutas de `/plataforma/whatsapp`.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar, crear))
        .routes(routes!(actualizar))
        .routes(routes!(listar_tarifas, guardar_tarifa))
}

/// Línea de WhatsApp con su uso del mes.
#[derive(Debug, Serialize, ToSchema)]
pub struct LineaRespuesta {
    pub id: Uuid,
    pub phone_number_id: String,
    pub numero_visible: String,
    pub nombre: String,
    pub limite_mps: u32,
    /// `activa`, `degradada` (Meta bajó su calidad) o `pausada`.
    pub estado: &'static str,
    pub calidad: Option<String>,
    /// Mensajes de servicio gratuitos al mes.
    pub franquicia_servicio_mes: u32,
    /// Mensajes de servicio de este mes (consumen la franquicia).
    pub servicio_mes: u64,
    /// Plantillas de este mes (se pagan aparte).
    pub plantillas_mes: u64,
    /// Franquicia que le queda: las conversaciones nuevas salen por la que tenga más.
    pub franquicia_disponible: u64,
}

impl From<LineaWhatsapp> for LineaRespuesta {
    fn from(l: LineaWhatsapp) -> Self {
        let franquicia_disponible = l.franquicia_disponible();
        Self {
            id: l.id,
            phone_number_id: l.phone_number_id,
            numero_visible: l.numero_visible,
            nombre: l.nombre,
            limite_mps: l.limite_mps,
            estado: l.estado.as_str(),
            calidad: l.calidad,
            franquicia_servicio_mes: l.franquicia_servicio_mes,
            franquicia_disponible,
            servicio_mes: l.servicio_mes,
            plantillas_mes: l.plantillas_mes,
        }
    }
}

/// Líneas de la plataforma (cualquiera atiende cualquier copropiedad).
#[utoipa::path(
    get,
    path = "/whatsapp/lineas",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Líneas", body = Vec<LineaRespuesta>),
        (status = FORBIDDEN, description = "Sin permiso o sin segundo factor", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    op: OperadorPlataforma,
) -> ApiResult<Json<Vec<LineaRespuesta>>> {
    let v = uc::listar_lineas(&state.plataforma, &op.0.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Línea nueva (ya verificada en Meta).
#[derive(Debug, Deserialize, ToSchema)]
pub struct LineaPeticion {
    pub phone_number_id: String,
    #[schema(example = "+57 300 000 0000")]
    pub numero_visible: String,
    pub nombre: String,
}

/// Registra una línea.
#[utoipa::path(
    post,
    path = "/whatsapp/lineas",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = LineaPeticion,
    responses(
        (status = CREATED, description = "Línea registrada", body = LineaRespuesta),
        (status = CONFLICT, description = "Ya existe (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn crear(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Json(p): Json<LineaPeticion>,
) -> ApiResult<(StatusCode, Json<LineaRespuesta>)> {
    let l = uc::crear_linea(
        &state.plataforma,
        &op.0.ctx,
        &p.phone_number_id,
        &p.numero_visible,
        &p.nombre,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(l.into())))
}

/// Cambios de una línea.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ActualizarLineaPeticion {
    /// `activa`, `degradada` o `pausada`.
    #[schema(example = "activa")]
    pub estado: String,
    #[schema(example = 80)]
    pub limite_mps: u32,
    #[schema(example = 1000)]
    pub franquicia_servicio_mes: u32,
}

/// Cambia el estado, el ritmo o la franquicia de una línea.
#[utoipa::path(
    patch,
    path = "/whatsapp/lineas/{linea_id}",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("linea_id" = Uuid, Path, description = "Línea")),
    request_body = ActualizarLineaPeticion,
    responses(
        (status = NO_CONTENT, description = "Actualizada"),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn actualizar(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<ActualizarLineaPeticion>,
) -> ApiResult<StatusCode> {
    let estado = EstadoLinea::desde(&p.estado).map_err(ApiError::App)?;
    uc::actualizar_linea(
        &state.plataforma,
        &op.0.ctx,
        id,
        estado,
        p.limite_mps,
        p.franquicia_servicio_mes,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Tarifa de Meta.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct TarifaDto {
    #[schema(example = "CO")]
    pub pais: String,
    /// `utilidad`, `servicio`, `marketing` o `autenticacion`.
    #[schema(example = "utilidad")]
    pub categoria: String,
    #[schema(example = "COP")]
    pub moneda: String,
    #[schema(value_type = String, example = "2.95")]
    pub valor: Decimal,
    pub vigente_desde: NaiveDate,
}

impl From<TarifaWhatsapp> for TarifaDto {
    fn from(t: TarifaWhatsapp) -> Self {
        Self {
            pais: t.pais,
            categoria: t.categoria,
            moneda: t.moneda,
            valor: t.valor,
            vigente_desde: t.vigente_desde,
        }
    }
}

/// Tarifas registradas (la vigente es la más reciente por país, categoría y moneda).
#[utoipa::path(
    get,
    path = "/whatsapp/tarifas",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    responses((status = OK, description = "Tarifas", body = Vec<TarifaDto>))
)]
pub async fn listar_tarifas(
    State(state): State<AppState>,
    op: OperadorPlataforma,
) -> ApiResult<Json<Vec<TarifaDto>>> {
    let v = uc::listar_tarifas(&state.plataforma, &op.0.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Registra la tarifa vigente desde una fecha (Meta publica tablas nuevas cada trimestre).
#[utoipa::path(
    post,
    path = "/whatsapp/tarifas",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = TarifaDto,
    responses(
        (status = CREATED, description = "Tarifa registrada", body = TarifaDto),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn guardar_tarifa(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Json(t): Json<TarifaDto>,
) -> ApiResult<(StatusCode, Json<TarifaDto>)> {
    let t = uc::guardar_tarifa(
        &state.plataforma,
        &op.0.ctx,
        &t.pais,
        &t.categoria,
        &t.moneda,
        t.valor,
        t.vigente_desde,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(t.into())))
}
