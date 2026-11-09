//! Papeleta virtual del asistente (`/papeleta`, sin sesión): el token de su enlace personal,
//! en la cabecera `X-Papeleta`, es la credencial. Sirve para el ingreso virtual (reuniones
//! virtuales o mixtas), votar por web y retirarse.

use application::{casos_uso::asambleas::registro as uc, puertos::RelojSistema};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
};
use domain::{ClaveIdempotencia, OpcionId, PreguntaId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::{AsistenteRespuesta, PreguntaRespuesta, ReunionRespuesta, VotoRespuesta};
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::Json,
    limite::LimitePublico,
    openapi::ETIQUETA_ASAMBLEAS,
    sesion::PapeletaVirtual,
    state::AppState,
};

pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(ver))
        .routes(routes!(ingreso))
        .routes(routes!(salida))
        .routes(routes!(votar))
}

/// Pregunta abierta en la papeleta.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreguntaPapeletaRespuesta {
    pub pregunta: PreguntaRespuesta,
    pub ya_voto: bool,
}

/// Papeleta del asistente.
#[derive(Debug, Serialize, ToSchema)]
pub struct PapeletaRespuesta {
    pub reunion: ReunionRespuesta,
    pub asistente: AsistenteRespuesta,
    pub preguntas_abiertas: Vec<PreguntaPapeletaRespuesta>,
}

/// Papeleta: reunión, asistente con sus bienes y preguntas abiertas.
#[utoipa::path(
    get,
    path = "/papeleta",
    tag = ETIQUETA_ASAMBLEAS,
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("X-Papeleta" = String, Header, description = "Token del enlace personal"),
    ),
    responses(
        (status = OK, description = "Papeleta", body = PapeletaRespuesta),
        (status = UNAUTHORIZED, description = "Token inválido (`no_autenticado`)", body = ErrorRespuesta),
    )
)]
pub async fn ver(
    _limite: LimitePublico,
    State(state): State<AppState>,
    p: PapeletaVirtual,
) -> ApiResult<Json<PapeletaRespuesta>> {
    let v = uc::ver_papeleta(&state.unidad, &RelojSistema, &p.ctx, &p.token).await?;
    Ok(Json(PapeletaRespuesta {
        reunion: v.reunion.into(),
        asistente: v.asistente.into(),
        preguntas_abiertas: v
            .preguntas_abiertas
            .into_iter()
            .map(|x| PreguntaPapeletaRespuesta {
                pregunta: x.pregunta.into(),
                ya_voto: x.ya_voto,
            })
            .collect(),
    }))
}

/// Ingreso virtual (reuniones virtuales o mixtas): cuenta para el quórum desde ahora.
#[utoipa::path(
    post,
    path = "/papeleta/ingreso",
    tag = ETIQUETA_ASAMBLEAS,
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("X-Papeleta" = String, Header, description = "Token del enlace personal"),
    ),
    responses(
        (status = OK, description = "Ingreso registrado", body = AsistenteRespuesta),
        (status = CONFLICT, description = "Presencial, no admite registro o ya está presente (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn ingreso(
    _limite: LimitePublico,
    State(state): State<AppState>,
    p: PapeletaVirtual,
) -> ApiResult<Json<AsistenteRespuesta>> {
    Ok(Json(
        uc::ingresar_con_papeleta(&state.unidad, &RelojSistema, &p.ctx, &p.token)
            .await?
            .into(),
    ))
}

/// «Salir de la asamblea»: sus votos se conservan.
#[utoipa::path(
    post,
    path = "/papeleta/salida",
    tag = ETIQUETA_ASAMBLEAS,
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("X-Papeleta" = String, Header, description = "Token del enlace personal"),
    ),
    responses(
        (status = OK, description = "Salida registrada", body = AsistenteRespuesta),
        (status = CONFLICT, description = "No estaba presente (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn salida(
    _limite: LimitePublico,
    State(state): State<AppState>,
    p: PapeletaVirtual,
) -> ApiResult<Json<AsistenteRespuesta>> {
    Ok(Json(
        uc::salir_con_papeleta(&state.unidad, &RelojSistema, &p.ctx, &p.token)
            .await?
            .into(),
    ))
}

/// Voto desde la papeleta.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VotoPapeletaPeticion {
    pub pregunta_id: Uuid,
    pub opciones: Vec<Uuid>,
}

/// Vota por **todos** sus bienes habilitados (canal web). Exige `Idempotency-Key`.
#[utoipa::path(
    post,
    path = "/papeleta/votos",
    tag = ETIQUETA_ASAMBLEAS,
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("X-Papeleta" = String, Header, description = "Token del enlace personal"),
        ("Idempotency-Key" = String, Header, description = "La genera el cliente por cada voto"),
    ),
    request_body = VotoPapeletaPeticion,
    responses(
        (status = CREATED, description = "Voto registrado (o el original si se reintenta)", body = VotoRespuesta),
        (status = FORBIDDEN, description = "No está presente (`no_inscrito`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya votó (`ya_voto`) o la pregunta cerró (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn votar(
    _limite: LimitePublico,
    State(state): State<AppState>,
    p: PapeletaVirtual,
    cabeceras: HeaderMap,
    Json(v): Json<VotoPapeletaPeticion>,
) -> ApiResult<(StatusCode, Json<VotoRespuesta>)> {
    let clave = cabeceras
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::PeticionInvalida("falta la cabecera `Idempotency-Key`".into()))?;
    let r = uc::votar_con_papeleta(
        &state.unidad,
        &RelojSistema,
        &p.ctx,
        &p.token,
        PreguntaId::from_uuid(v.pregunta_id),
        v.opciones.into_iter().map(OpcionId::from_uuid).collect(),
        ClaveIdempotencia::new(clave)?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}
