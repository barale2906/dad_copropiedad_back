//! Búsquedas de la portería: `/acceso/bienes` y `/acceso/personas`. Devuelven lo mínimo para
//! que el vigilante y el supervisor elijan el bien o la persona sin `bienes_privados.ver` ni
//! `personas.ver`.

use application::{
    casos_uso::acceso::busquedas::{self as uc, BienPorteria, PersonaPorteria},
    puertos::RelojSistema,
};
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Query},
    openapi::ETIQUETA_ACCESO,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::EnCopropiedad,
    state::AppState,
};

pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(buscar_bienes))
        .routes(routes!(buscar_personas))
}

/// Texto que se busca.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BusquedaQuery {
    /// Fragmento del código del bien, o del nombre (sin tildes ni mayúsculas) o el documento de
    /// la persona.
    pub q: Option<String>,
}

/// Bien privado para la portería: solo id y código.
#[derive(Debug, Serialize, ToSchema)]
pub struct BienPorteriaRespuesta {
    pub id: Uuid,
    #[schema(example = "T1-101")]
    pub codigo: String,
}

impl From<BienPorteria> for BienPorteriaRespuesta {
    fn from(b: BienPorteria) -> Self {
        Self {
            id: b.id.as_uuid(),
            codigo: b.codigo,
        }
    }
}

/// Persona para la portería: nombre y documento enmascarado.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonaPorteriaRespuesta {
    pub id: Uuid,
    #[schema(example = "Marta Gómez")]
    pub nombre: String,
    /// Últimos cuatro caracteres del documento; `null` si no tiene.
    #[schema(example = "****3456")]
    pub documento: Option<String>,
}

impl From<PersonaPorteria> for PersonaPorteriaRespuesta {
    fn from(p: PersonaPorteria) -> Self {
        Self {
            id: p.id.as_uuid(),
            nombre: p.nombre,
            documento: p.documento,
        }
    }
}

/// Bienes privados activos por código, para anunciar visitas y recibir correspondencia.
#[utoipa::path(
    get,
    path = "/acceso/bienes",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), BusquedaQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Bienes (id y código)", body = PaginaRespuesta<BienPorteriaRespuesta>),
        (status = FORBIDDEN, description = "Sin `acceso.registrar` (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn buscar_bienes(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(b): Query<BusquedaQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<BienPorteriaRespuesta>>> {
    let pagina =
        uc::buscar_bienes(&state.unidad, &RelojSistema, &en.ctx, b.q, p.validar()?).await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Personas por nombre o documento, para emitir credenciales o registrar el consentimiento
/// biométrico. Exige al menos 2 caracteres.
#[utoipa::path(
    get,
    path = "/acceso/personas",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), BusquedaQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Personas (nombre y documento enmascarado)", body = PaginaRespuesta<PersonaPorteriaRespuesta>),
        (status = FORBIDDEN, description = "Sin `acceso.credenciales.gestionar` ni `acceso.biometria.enrolar` (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Menos de 2 caracteres (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn buscar_personas(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(b): Query<BusquedaQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<PersonaPorteriaRespuesta>>> {
    let pagina = uc::buscar_personas(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        b.q.as_deref().unwrap_or_default(),
        p.validar()?,
    )
    .await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}
