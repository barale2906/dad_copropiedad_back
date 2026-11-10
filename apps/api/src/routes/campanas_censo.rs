//! Campañas de actualización del censo por WhatsApp (plan 10, fase 5): `/censo/campanas…`,
//! dentro de `/api/v1/copropiedades/{copropiedad_id}`.

use application::{
    casos_uso::campanas_censo as uc,
    puertos::{
        RelojSistema,
        censo::{CampanaCenso, RespuestaCensoListada, ResumenCampanaCenso},
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{BienPrivadoId, CampanaCensoId, PersonaId};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path},
    openapi::ETIQUETA_CENSO,
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de las campañas.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar, lanzar))
        .routes(routes!(respuestas))
        .routes(routes!(atender))
}

/// Campaña lanzada.
#[derive(Debug, Serialize, ToSchema)]
pub struct CampanaCensoRespuesta {
    pub id: Uuid,
    pub creada_en: DateTime<Utc>,
    /// Plantillas `actualizar_datos` encoladas.
    pub enviados: u32,
}

impl From<CampanaCenso> for CampanaCensoRespuesta {
    fn from(c: CampanaCenso) -> Self {
        Self {
            id: c.id.as_uuid(),
            creada_en: c.creada_en,
            enviados: c.enviados,
        }
    }
}

/// Campaña con sus contadores.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResumenCampanaRespuesta {
    pub campana: CampanaCensoRespuesta,
    pub confirmados: u32,
    pub con_cambios: u32,
    /// Con cambios que aún no se aplican en el censo.
    pub por_atender: u32,
}

impl From<ResumenCampanaCenso> for ResumenCampanaRespuesta {
    fn from(r: ResumenCampanaCenso) -> Self {
        Self {
            campana: r.campana.into(),
            confirmados: r.confirmados,
            con_cambios: r.con_cambios,
            por_atender: r.por_atender,
        }
    }
}

/// Respuesta de un responsable.
#[derive(Debug, Serialize, ToSchema)]
pub struct RespuestaCensoRespuesta {
    pub persona_id: Uuid,
    pub nombre: String,
    pub bien_privado_id: Uuid,
    #[schema(example = "T1-506")]
    pub codigo: String,
    /// `confirmado` o `cambios`.
    #[schema(example = "cambios")]
    pub respuesta: String,
    /// Cambios descritos por el responsable.
    pub detalle: Option<String>,
    pub respondida_en: DateTime<Utc>,
    pub atendida_en: Option<DateTime<Utc>>,
}

impl From<RespuestaCensoListada> for RespuestaCensoRespuesta {
    fn from(l: RespuestaCensoListada) -> Self {
        let r = l.respuesta;
        Self {
            persona_id: r.persona.as_uuid(),
            nombre: l.nombre,
            bien_privado_id: r.bien.as_uuid(),
            codigo: l.codigo,
            respuesta: r.respuesta.as_str().into(),
            detalle: r.detalle,
            respondida_en: r.respondida_en,
            atendida_en: r.atendida_en,
        }
    }
}

/// Campañas de actualización, de la más reciente a la más antigua (máximo 100).
#[utoipa::path(
    get,
    path = "/censo/campanas",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Campañas", body = Vec<ResumenCampanaRespuesta>),
        (status = FORBIDDEN, description = "Sin `censo.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<ResumenCampanaRespuesta>>> {
    let v = uc::listar_campanas_censo(&state.unidad, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Lanza una campaña: la plantilla `actualizar_datos` llega al responsable de cada bien con
/// residentes (o al representante de los titulares si nadie reside) que tenga teléfono.
#[utoipa::path(
    post,
    path = "/censo/campanas",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = CREATED, description = "Campaña lanzada", body = CampanaCensoRespuesta),
        (status = FORBIDDEN, description = "Sin `censo.editar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No hay una línea de WhatsApp (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn lanzar(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<(StatusCode, Json<CampanaCensoRespuesta>)> {
    let c = uc::lanzar_campana_censo(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Respuestas de una campaña; primero las que tienen cambios por atender.
#[utoipa::path(
    get,
    path = "/censo/campanas/{campana_id}/respuestas",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("campana_id" = Uuid, Path, description = "Campaña"),
    ),
    responses(
        (status = OK, description = "Respuestas", body = Vec<RespuestaCensoRespuesta>),
        (status = FORBIDDEN, description = "Sin `censo.ver` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe (`no_encontrado`)", body = ErrorRespuesta),
    )
)]
pub async fn respuestas(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<RespuestaCensoRespuesta>>> {
    let v = uc::respuestas_campana_censo(&state.unidad, &en.ctx, CampanaCensoId::from_uuid(ruta.1))
        .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Marca atendida una respuesta con cambios (ya aplicados en el censo).
#[utoipa::path(
    post,
    path = "/censo/campanas/{campana_id}/respuestas/{persona_id}/{bien_privado_id}/atender",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("campana_id" = Uuid, Path, description = "Campaña"),
        ("persona_id" = Uuid, Path, description = "Persona que respondió"),
        ("bien_privado_id" = Uuid, Path, description = "Bien privado"),
    ),
    responses(
        (status = NO_CONTENT, description = "Atendida"),
        (status = FORBIDDEN, description = "Sin `censo.editar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No hay una respuesta con cambios (`no_encontrado`)", body = ErrorRespuesta),
    )
)]
pub async fn atender(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    uc::atender_respuesta_censo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CampanaCensoId::from_uuid(ruta.1),
        PersonaId::from_uuid(ruta.2),
        BienPrivadoId::from_uuid(ruta.3),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
