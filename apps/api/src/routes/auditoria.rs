//! `/auditoria` dentro de `/api/v1/copropiedades/{copropiedad_id}`: consulta de la auditoría
//! (`auditoria.ver`, p. ej. el revisor fiscal). Solo lectura.

use application::{
    casos_uso::auditoria::{self as uc, cursor_de},
    puertos::auditoria::{FiltroAuditoria, RegistroAuditado},
};
use axum::extract::State;
use chrono::{DateTime, Utc};
use domain::CuentaId;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Query},
    openapi::ETIQUETA_AUDITORIA,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de auditoría.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(consultar))
}

/// Evento auditado.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventoAuditadoRespuesta {
    /// Cursor opaco del evento (para `despues`).
    pub id: Uuid,
    pub ocurrido_en: DateTime<Utc>,
    /// Cuenta que actuó; `null` = el sistema.
    pub actor_id: Option<Uuid>,
    #[schema(example = "acceso.visita.decidir")]
    pub accion: String,
    #[schema(example = "visita")]
    pub entidad: String,
    pub entidad_id: Option<Uuid>,
    /// `ok`, `rechazado` o `error`.
    pub resultado: String,
    /// Detalle sin datos personales.
    #[schema(value_type = Object)]
    pub detalle: serde_json::Value,
    pub request_id: Option<String>,
}

impl From<RegistroAuditado> for EventoAuditadoRespuesta {
    fn from(r: RegistroAuditado) -> Self {
        Self {
            id: cursor_de(r.id),
            ocurrido_en: r.ocurrido_en,
            actor_id: r.actor_id.map(CuentaId::as_uuid),
            accion: r.accion,
            entidad: r.entidad,
            entidad_id: r.entidad_id,
            resultado: r.resultado,
            detalle: r.detalle,
            request_id: r.request_id,
        }
    }
}

/// Filtros de la consulta.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroAuditoriaQuery {
    #[param(example = "visita")]
    pub entidad: Option<String>,
    pub entidad_id: Option<Uuid>,
    /// Acción exacta (`acceso.visita.decidir`) o prefijo terminado en punto (`acceso.`).
    pub accion: Option<String>,
    pub actor_id: Option<Uuid>,
    pub desde: Option<DateTime<Utc>>,
    pub hasta: Option<DateTime<Utc>>,
}

/// Eventos de la auditoría, más recientes primero.
#[utoipa::path(
    get,
    path = "/auditoria",
    tag = ETIQUETA_AUDITORIA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroAuditoriaQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Eventos", body = PaginaRespuesta<EventoAuditadoRespuesta>),
        (status = FORBIDDEN, description = "Sin `auditoria.ver` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Cursor inválido o fechas invertidas (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn consultar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroAuditoriaQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<EventoAuditadoRespuesta>>> {
    let filtro = FiltroAuditoria {
        entidad: f.entidad,
        entidad_id: f.entidad_id,
        accion: f.accion,
        actor_id: f.actor_id.map(CuentaId::from_uuid),
        desde: f.desde,
        hasta: f.hasta,
    };
    let pagina = uc::consultar_auditoria(&state.unidad, &en.ctx, filtro, p.validar()?).await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}
