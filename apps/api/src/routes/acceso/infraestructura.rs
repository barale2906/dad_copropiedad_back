//! `/puntos-acceso`, `/puentes` y `/dispositivos-acceso`: configuración del hardware de acceso.

use application::{
    casos_uso::acceso::puentes::{self as uc, DatosDispositivo, DatosPunto},
    puertos::{
        RelojSistema,
        acceso::{DispositivoAcceso, Puente, PuntoAcceso},
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{
    DispositivoAccesoId, PuenteId, PuntoAccesoId, RecursoId, SentidoAcceso, TipoDispositivoAcceso,
    TipoPuntoAcceso,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_ACCESO,
    sesion::EnCopropiedad,
    state::AppState,
};

pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_puntos, crear_punto))
        .routes(routes!(editar_punto))
        .routes(routes!(listar_puentes, registrar_puente))
        .routes(routes!(rotar_credencial))
        .routes(routes!(retirar_puente))
        .routes(routes!(listar_dispositivos, crear_dispositivo))
        .routes(routes!(editar_dispositivo))
}

/// Punto de acceso.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PuntoDto {
    #[serde(default, skip_deserializing)]
    pub id: Uuid,
    #[schema(example = "Portería principal")]
    pub nombre: String,
    /// `peatonal`, `vehicular`, `mixto` o `interno`.
    #[schema(value_type = String, example = "peatonal")]
    pub tipo: TipoPuntoAcceso,
    /// `entrada`, `salida` o `ambos`.
    #[schema(value_type = String, example = "ambos")]
    pub sentido: SentidoAcceso,
    /// Bien común al que da acceso (reservas con acceso automático).
    pub recurso_id: Option<Uuid>,
    #[serde(default = "verdadero")]
    pub activo: bool,
}

const fn verdadero() -> bool {
    true
}

impl From<PuntoAcceso> for PuntoDto {
    fn from(p: PuntoAcceso) -> Self {
        Self {
            id: p.id.as_uuid(),
            nombre: p.nombre,
            tipo: p.tipo,
            sentido: p.sentido,
            recurso_id: p.recurso_id.map(RecursoId::as_uuid),
            activo: p.activo,
        }
    }
}

impl PuntoDto {
    fn datos(self) -> DatosPunto {
        DatosPunto {
            nombre: self.nombre,
            tipo: self.tipo,
            sentido: self.sentido,
            recurso_id: self.recurso_id.map(RecursoId::from_uuid),
            activo: self.activo,
        }
    }
}

/// Puntos de acceso de la copropiedad.
#[utoipa::path(
    get,
    path = "/puntos-acceso",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Puntos", body = Vec<PuntoDto>),
        (status = FORBIDDEN, description = "Sin `acceso.registrar` ni `acceso.configurar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_puntos(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<PuntoDto>>> {
    let v = uc::listar_puntos(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea un punto de acceso.
#[utoipa::path(
    post,
    path = "/puntos-acceso",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = PuntoDto,
    responses(
        (status = CREATED, description = "Punto creado", body = PuntoDto),
        (status = FORBIDDEN, description = "Sin `acceso.configurar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya existe uno con ese nombre (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_punto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<PuntoDto>,
) -> ApiResult<(StatusCode, Json<PuntoDto>)> {
    let x = uc::guardar_punto(&state.unidad, &RelojSistema, &en.ctx, None, p.datos()).await?;
    Ok((StatusCode::CREATED, Json(x.into())))
}

/// Edita un punto de acceso.
#[utoipa::path(
    put,
    path = "/puntos-acceso/{punto_id}",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("punto_id" = Uuid, Path, description = "Punto de acceso")),
    request_body = PuntoDto,
    responses(
        (status = OK, description = "Punto guardado", body = PuntoDto),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn editar_punto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PuntoDto>,
) -> ApiResult<Json<PuntoDto>> {
    let x = uc::guardar_punto(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        Some(PuntoAccesoId::from_uuid(ruta.1)),
        p.datos(),
    )
    .await?;
    Ok(Json(x.into()))
}

/// Puente de acceso.
#[derive(Debug, Serialize, ToSchema)]
pub struct PuenteRespuesta {
    pub id: Uuid,
    pub nombre: String,
    /// `activo` o `retirado`.
    pub estado: String,
    pub version: Option<String>,
    pub ultimo_latido_en: Option<DateTime<Utc>>,
    pub creado_en: DateTime<Utc>,
}

impl From<Puente> for PuenteRespuesta {
    fn from(p: Puente) -> Self {
        Self {
            id: p.id.as_uuid(),
            nombre: p.nombre,
            estado: p.estado.as_str().into(),
            version: p.version,
            ultimo_latido_en: p.ultimo_latido_en,
            creado_en: p.creado_en,
        }
    }
}

/// Puente nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PuentePeticion {
    #[schema(example = "Mini PC portería")]
    pub nombre: String,
}

/// Puente con su token de máquina (se muestra **una sola vez**).
#[derive(Debug, Serialize, ToSchema)]
pub struct PuenteRegistradoRespuesta {
    pub puente: PuenteRespuesta,
    /// Token del puente para `Authorization: Bearer` en `/api/v1/puentes/…`.
    pub token: String,
}

/// Puentes de la copropiedad.
#[utoipa::path(
    get,
    path = "/puentes",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Puentes", body = Vec<PuenteRespuesta>),
        (status = FORBIDDEN, description = "Sin `acceso.configurar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_puentes(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<PuenteRespuesta>>> {
    let v = uc::listar_puentes(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Registra un puente (equipo nuevo o existente en la PH) y entrega su token.
#[utoipa::path(
    post,
    path = "/puentes",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = PuentePeticion,
    responses(
        (status = CREATED, description = "Puente registrado", body = PuenteRegistradoRespuesta),
        (status = FORBIDDEN, description = "Sin `acceso.configurar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_puente(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<PuentePeticion>,
) -> ApiResult<(StatusCode, Json<PuenteRegistradoRespuesta>)> {
    let (puente, token) =
        uc::registrar_puente(&state.unidad, &RelojSistema, &en.ctx, &p.nombre).await?;
    Ok((
        StatusCode::CREATED,
        Json(PuenteRegistradoRespuesta {
            puente: puente.into(),
            token,
        }),
    ))
}

/// Token nuevo del puente.
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenPuenteRespuesta {
    pub token: String,
}

/// Rota el token de máquina: el anterior deja de servir de inmediato.
#[utoipa::path(
    post,
    path = "/puentes/{puente_id}/credencial",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("puente_id" = Uuid, Path, description = "Puente")),
    responses(
        (status = OK, description = "Token nuevo", body = TokenPuenteRespuesta),
        (status = CONFLICT, description = "El puente está retirado (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn rotar_credencial(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<TokenPuenteRespuesta>> {
    let token = uc::rotar_credencial_puente(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PuenteId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(TokenPuenteRespuesta { token }))
}

/// Retira el puente (ya no sincroniza ni reporta).
#[utoipa::path(
    post,
    path = "/puentes/{puente_id}/retiro",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("puente_id" = Uuid, Path, description = "Puente")),
    responses(
        (status = OK, description = "Puente retirado", body = PuenteRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn retirar_puente(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<PuenteRespuesta>> {
    let p = uc::retirar_puente(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PuenteId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(p.into()))
}

/// Dispositivo de acceso.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct DispositivoDto {
    #[serde(default, skip_deserializing)]
    pub id: Uuid,
    pub puente_id: Uuid,
    pub punto_acceso_id: Uuid,
    /// `torniquete`, `talanquera`, `puerta`, `lector_qr`, `lector_rfid`, `teclado_pin`,
    /// `biometrico_huella`, `biometrico_facial` o `camara_lpr`.
    #[schema(value_type = String, example = "camara_lpr")]
    pub tipo: TipoDispositivoAcceso,
    #[schema(example = "Hikvision")]
    pub fabricante: String,
    pub modelo: Option<String>,
    /// Configuración del adaptador **sin secretos** (IP, canal, puerto…).
    #[serde(default)]
    #[schema(value_type = Object)]
    pub config: serde_json::Value,
    #[serde(default = "verdadero")]
    pub activo: bool,
}

impl From<DispositivoAcceso> for DispositivoDto {
    fn from(d: DispositivoAcceso) -> Self {
        Self {
            id: d.id.as_uuid(),
            puente_id: d.puente_id.as_uuid(),
            punto_acceso_id: d.punto_acceso_id.as_uuid(),
            tipo: d.tipo,
            fabricante: d.fabricante,
            modelo: d.modelo,
            config: d.config,
            activo: d.activo,
        }
    }
}

impl DispositivoDto {
    fn datos(self) -> DatosDispositivo {
        DatosDispositivo {
            puente_id: PuenteId::from_uuid(self.puente_id),
            punto_acceso_id: PuntoAccesoId::from_uuid(self.punto_acceso_id),
            tipo: self.tipo,
            fabricante: self.fabricante,
            modelo: self.modelo,
            config: if self.config.is_null() {
                serde_json::json!({})
            } else {
                self.config
            },
            activo: self.activo,
        }
    }
}

/// Filtro de dispositivos.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DispositivosQuery {
    pub puente_id: Option<Uuid>,
}

/// Dispositivos (de un puente o todos).
#[utoipa::path(
    get,
    path = "/dispositivos-acceso",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), DispositivosQuery),
    responses(
        (status = OK, description = "Dispositivos", body = Vec<DispositivoDto>),
        (status = FORBIDDEN, description = "Sin `acceso.configurar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_dispositivos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(q): Query<DispositivosQuery>,
) -> ApiResult<Json<Vec<DispositivoDto>>> {
    let v = uc::listar_dispositivos(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        q.puente_id.map(PuenteId::from_uuid),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Registra un dispositivo en un puente.
#[utoipa::path(
    post,
    path = "/dispositivos-acceso",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = DispositivoDto,
    responses(
        (status = CREATED, description = "Dispositivo registrado", body = DispositivoDto),
        (status = NOT_FOUND, description = "El puente o el punto no existen", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "La configuración trae secretos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_dispositivo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<DispositivoDto>,
) -> ApiResult<(StatusCode, Json<DispositivoDto>)> {
    let x = uc::guardar_dispositivo(&state.unidad, &RelojSistema, &en.ctx, None, p.datos()).await?;
    Ok((StatusCode::CREATED, Json(x.into())))
}

/// Edita un dispositivo.
#[utoipa::path(
    put,
    path = "/dispositivos-acceso/{dispositivo_id}",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("dispositivo_id" = Uuid, Path, description = "Dispositivo")),
    request_body = DispositivoDto,
    responses(
        (status = OK, description = "Dispositivo guardado", body = DispositivoDto),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn editar_dispositivo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<DispositivoDto>,
) -> ApiResult<Json<DispositivoDto>> {
    let x = uc::guardar_dispositivo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        Some(DispositivoAccesoId::from_uuid(ruta.1)),
        p.datos(),
    )
    .await?;
    Ok(Json(x.into()))
}
