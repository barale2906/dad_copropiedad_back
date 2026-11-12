//! `/api/v1/yo/dispositivos-push`: la app Flutter registra el token de FCM del teléfono al
//! iniciar sesión y lo retira al cerrarla.

use application::{
    casos_uso::acceso::push as uc,
    puertos::{RelojSistema, acceso::DispositivoPush},
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{DispositivoPushId, PlataformaPush};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path},
    openapi::ETIQUETA_CUENTA,
    sesion::CuentaAutenticada,
    state::AppState,
};

/// Rutas dentro de `/api/v1/yo`.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar, registrar))
        .routes(routes!(retirar))
}

/// Dispositivo registrado (el token no se devuelve).
#[derive(Debug, Serialize, ToSchema)]
pub struct DispositivoPushRespuesta {
    pub id: Uuid,
    /// `android` o `ios`.
    #[schema(value_type = crate::esquemas::PlataformaPush)]
    pub plataforma: String,
    pub nombre: Option<String>,
    pub registrado_en: DateTime<Utc>,
}

impl From<DispositivoPush> for DispositivoPushRespuesta {
    fn from(d: DispositivoPush) -> Self {
        Self {
            id: d.id.as_uuid(),
            plataforma: d.plataforma.as_str().into(),
            nombre: d.nombre,
            registrado_en: d.registrado_en,
        }
    }
}

/// Registro del dispositivo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegistrarDispositivoPeticion {
    #[schema(value_type = crate::esquemas::PlataformaPush)]
    pub plataforma: PlataformaPush,
    /// Token de FCM del teléfono (`FirebaseMessaging.instance.getToken()`).
    pub token: String,
    #[schema(example = "Teléfono de Ana")]
    pub nombre: Option<String>,
}

/// Dispositivos activos de la cuenta.
#[utoipa::path(
    get,
    path = "/dispositivos-push",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Dispositivos", body = Vec<DispositivoPushRespuesta>),
        (status = UNAUTHORIZED, description = "Sin sesión (`no_autenticado`)", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    c: CuentaAutenticada,
) -> ApiResult<Json<Vec<DispositivoPushRespuesta>>> {
    let v = uc::listar_dispositivos(&state.plataforma, &c.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Registra (o reactiva) el dispositivo: desde ahí los avisos de portería llegan por la app y no
/// por WhatsApp.
#[utoipa::path(
    post,
    path = "/dispositivos-push",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    request_body = RegistrarDispositivoPeticion,
    responses(
        (status = CREATED, description = "Dispositivo registrado", body = DispositivoPushRespuesta),
        (status = UNAUTHORIZED, description = "Sin sesión (`no_autenticado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Token inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar(
    State(state): State<AppState>,
    c: CuentaAutenticada,
    Json(p): Json<RegistrarDispositivoPeticion>,
) -> ApiResult<(StatusCode, Json<DispositivoPushRespuesta>)> {
    let d = uc::registrar_dispositivo(
        &state.plataforma,
        &RelojSistema,
        &c.ctx,
        p.plataforma,
        &p.token,
        p.nombre.as_deref(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(d.into())))
}

/// Retira el dispositivo (al cerrar sesión en la app).
#[utoipa::path(
    delete,
    path = "/dispositivos-push/{dispositivo_id}",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    params(("dispositivo_id" = Uuid, Path, description = "Dispositivo")),
    responses(
        (status = NO_CONTENT, description = "Retirado"),
        (status = NOT_FOUND, description = "No es de la cuenta o ya se retiró", body = ErrorRespuesta),
    )
)]
pub async fn retirar(
    State(state): State<AppState>,
    c: CuentaAutenticada,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    uc::retirar_dispositivo(&state.plataforma, &c.ctx, DispositivoPushId::from_uuid(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
