//! Protección de datos personales (Ley 1581 de 2012, ADR 0004) dentro de
//! `/api/v1/copropiedades/{copropiedad_id}`: política de tratamiento, autorizaciones por
//! finalidad, solicitudes del titular (consultas y reclamos con plazos legales) y la consulta
//! «mis datos». El titular actúa sobre sí mismo; la administración con
//! `proteccion_datos.gestionar`.

use std::collections::BTreeSet;

use application::{
    casos_uso::{
        censo::PersonaVista,
        proteccion_datos::{self as uc, AutorizacionOtorgada, NuevaPolitica},
    },
    puertos::{
        RelojSistema,
        proteccion_datos::{
            AutorizacionRegistrada, FiltroSolicitudes, PoliticaTratamiento, SolicitudTitular,
        },
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, EstadoSolicitudTitular, FinalidadTratamiento, MedioAutorizacion, PersonaId,
    SolicitudTitularId, TipoSolicitudTitular,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::censo::PersonaRespuesta;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_PROTECCION_DATOS,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de protección de datos.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(politica_vigente))
        .routes(routes!(historial_politicas, publicar_politica))
        .routes(routes!(mis_datos))
        .routes(routes!(datos_de_persona))
        .routes(routes!(autorizaciones, otorgar))
        .routes(routes!(revocar))
        .routes(routes!(listar_solicitudes, radicar))
        .routes(routes!(tramitar))
        .routes(routes!(prorrogar))
        .routes(routes!(responder))
}

fn persona(ruta: (Uuid, Uuid)) -> PersonaId {
    PersonaId::from_uuid(ruta.1)
}

fn solicitud_id(ruta: (Uuid, Uuid)) -> SolicitudTitularId {
    SolicitudTitularId::from_uuid(ruta.1)
}

/// Versión de la política de tratamiento.
#[derive(Debug, Serialize, ToSchema)]
pub struct PoliticaRespuesta {
    pub id: Uuid,
    #[schema(example = 1)]
    pub version: u32,
    pub texto: Option<String>,
    /// PDF de la política (descarga con `/archivos/{archivo_id}/enlace`).
    pub archivo_id: Option<Uuid>,
    pub vigente_desde: DateTime<Utc>,
}

impl From<PoliticaTratamiento> for PoliticaRespuesta {
    fn from(p: PoliticaTratamiento) -> Self {
        Self {
            id: p.id.as_uuid(),
            version: p.version,
            texto: p.texto,
            archivo_id: p.archivo_id.map(ArchivoId::as_uuid),
            vigente_desde: p.vigente_desde,
        }
    }
}

/// Política vigente (la lee cualquier cuenta de la PH antes de autorizar); `null` si no hay.
#[utoipa::path(
    get,
    path = "/proteccion-datos/politica",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses((status = OK, description = "Política vigente", body = Option<PoliticaRespuesta>))
)]
pub async fn politica_vigente(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Option<PoliticaRespuesta>>> {
    let p = uc::politica_vigente(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(p.map(Into::into)))
}

/// Todas las versiones publicadas, la más reciente primero.
#[utoipa::path(
    get,
    path = "/proteccion-datos/politicas",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Versiones", body = Vec<PoliticaRespuesta>),
        (status = FORBIDDEN, description = "Sin `proteccion_datos.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn historial_politicas(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<PoliticaRespuesta>>> {
    let v = uc::historial_politicas(&state.unidad, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Versión nueva de la política.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PoliticaPeticion {
    pub texto: Option<String>,
    /// PDF ya subido con `/archivos`.
    pub archivo_id: Option<Uuid>,
    /// Desde ahora si se omite (nunca en el pasado).
    pub vigente_desde: Option<DateTime<Utc>>,
}

/// Publica una versión nueva de la política (las autorizaciones anteriores conservan la suya).
#[utoipa::path(
    post,
    path = "/proteccion-datos/politicas",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = PoliticaPeticion,
    responses(
        (status = CREATED, description = "Versión publicada", body = PoliticaRespuesta),
        (status = FORBIDDEN, description = "Sin `proteccion_datos.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin texto ni archivo o vigencia pasada (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn publicar_politica(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<PoliticaPeticion>,
) -> ApiResult<(StatusCode, Json<PoliticaRespuesta>)> {
    let x = uc::publicar_politica(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevaPolitica {
            texto: p.texto,
            archivo_id: p.archivo_id.map(ArchivoId::from_uuid),
            vigente_desde: p.vigente_desde,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(x.into())))
}

/// Autorización por finalidad.
#[derive(Debug, Serialize, ToSchema)]
pub struct AutorizacionTratamientoRespuesta {
    pub id: Uuid,
    /// `administracion`, `comunicaciones`, `control_acceso` o `publicidad`.
    #[schema(example = "comunicaciones")]
    pub finalidad: String,
    /// `web`, `app`, `whatsapp` o `escrito`.
    pub medio: String,
    pub politica_version: u32,
    pub evidencia_archivo_id: Option<Uuid>,
    pub otorgada_en: DateTime<Utc>,
    pub revocada_en: Option<DateTime<Utc>>,
}

impl From<AutorizacionRegistrada> for AutorizacionTratamientoRespuesta {
    fn from(r: AutorizacionRegistrada) -> Self {
        let a = r.autorizacion;
        Self {
            id: a.id.as_uuid(),
            finalidad: a.finalidad.as_str().into(),
            medio: a.medio.as_str().into(),
            politica_version: r.politica_version,
            evidencia_archivo_id: a.evidencia_archivo_id.map(ArchivoId::as_uuid),
            otorgada_en: a.otorgada_en,
            revocada_en: r.revocada_en,
        }
    }
}

/// Solicitud del titular.
#[derive(Debug, Serialize, ToSchema)]
pub struct SolicitudTitularRespuesta {
    pub id: Uuid,
    pub persona_id: Uuid,
    /// `consulta`, `actualizacion`, `rectificacion`, `supresion` o `revocatoria`.
    #[schema(example = "consulta")]
    pub tipo: String,
    /// `radicada`, `en_tramite` o `respondida`.
    #[schema(example = "radicada")]
    pub estado: String,
    pub detalle: Option<String>,
    pub radicada_en: DateTime<Utc>,
    /// Vencimiento legal inicial (10 días hábiles las consultas, 15 los reclamos).
    pub vence_el: NaiveDate,
    /// Vencimiento con la prórroga, si se usó.
    pub prorrogada_hasta: Option<NaiveDate>,
    pub motivo_prorroga: Option<String>,
    pub respuesta: Option<String>,
    pub respuesta_archivo_id: Option<Uuid>,
    pub respondida_en: Option<DateTime<Utc>>,
}

impl From<SolicitudTitular> for SolicitudTitularRespuesta {
    fn from(s: SolicitudTitular) -> Self {
        Self {
            id: s.id.as_uuid(),
            persona_id: s.persona_id.as_uuid(),
            tipo: s.tipo.as_str().into(),
            estado: s.estado.as_str().into(),
            detalle: s.detalle,
            radicada_en: s.radicada_en,
            vence_el: s.vence_el,
            prorrogada_hasta: s.prorrogada_hasta,
            motivo_prorroga: s.motivo_prorroga,
            respuesta: s.respuesta,
            respuesta_archivo_id: s.respuesta_archivo_id.map(ArchivoId::as_uuid),
            respondida_en: s.respondida_en,
        }
    }
}

/// Lo que la PH trata del titular (art. 14).
#[derive(Debug, Serialize, ToSchema)]
pub struct DatosTitularRespuesta {
    pub persona: PersonaRespuesta,
    pub autorizaciones: Vec<AutorizacionTratamientoRespuesta>,
    pub solicitudes: Vec<SolicitudTitularRespuesta>,
}

fn datos(d: uc::DatosDelTitular) -> DatosTitularRespuesta {
    DatosTitularRespuesta {
        persona: PersonaVista {
            persona: d.persona,
            es_menor: None,
            datos_sensibles_ocultos: false,
        }
        .into(),
        autorizaciones: d.autorizaciones.into_iter().map(Into::into).collect(),
        solicitudes: d.solicitudes.into_iter().map(Into::into).collect(),
    }
}

/// «Mis datos»: la ficha de la cuenta en la PH, sus autorizaciones y solicitudes. Se audita.
#[utoipa::path(
    get,
    path = "/proteccion-datos/mis-datos",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Datos del titular", body = DatosTitularRespuesta),
        (status = NOT_FOUND, description = "La cuenta no tiene persona en la PH", body = ErrorRespuesta),
    )
)]
pub async fn mis_datos(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<DatosTitularRespuesta>> {
    Ok(Json(datos(
        uc::mis_datos(&state.unidad, &en.ctx, None).await?,
    )))
}

/// Lo que la PH trata de una persona (para responder una consulta). Se audita.
#[utoipa::path(
    get,
    path = "/personas/{persona_id}/datos-tratados",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("persona_id" = Uuid, Path, description = "Persona")),
    responses(
        (status = OK, description = "Datos del titular", body = DatosTitularRespuesta),
        (status = FORBIDDEN, description = "Ni es el titular ni tiene `proteccion_datos.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn datos_de_persona(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DatosTitularRespuesta>> {
    Ok(Json(datos(
        uc::mis_datos(&state.unidad, &en.ctx, Some(persona(ruta))).await?,
    )))
}

/// Autorizaciones (vigentes y revocadas) de la persona.
#[utoipa::path(
    get,
    path = "/personas/{persona_id}/autorizaciones-tratamiento",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("persona_id" = Uuid, Path, description = "Persona")),
    responses(
        (status = OK, description = "Autorizaciones", body = Vec<AutorizacionTratamientoRespuesta>),
        (status = FORBIDDEN, description = "Sin permiso (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn autorizaciones(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<AutorizacionTratamientoRespuesta>>> {
    let v = uc::autorizaciones_de(&state.unidad, &en.ctx, persona(ruta)).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Finalidades que se autorizan.
#[derive(Debug, Deserialize, ToSchema)]
pub struct OtorgarPeticion {
    #[schema(value_type = Vec<String>, example = json!(["administracion", "comunicaciones"]))]
    pub finalidades: BTreeSet<FinalidadTratamiento>,
    /// `web`, `app`, `whatsapp` o `escrito`.
    #[schema(value_type = String, example = "app")]
    pub medio: MedioAutorizacion,
    /// Formato firmado escaneado (autorizaciones por escrito), ya subido con `/archivos`.
    pub evidencia_archivo_id: Option<Uuid>,
}

/// Finalidades que quedaron autorizadas (las que ya lo estaban no se duplican).
#[derive(Debug, Serialize, ToSchema)]
pub struct FinalidadesRespuesta {
    pub finalidades: Vec<String>,
}

/// Registra la autorización del titular bajo la política vigente.
#[utoipa::path(
    post,
    path = "/personas/{persona_id}/autorizaciones-tratamiento",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("persona_id" = Uuid, Path, description = "Persona")),
    request_body = OtorgarPeticion,
    responses(
        (status = CREATED, description = "Autorizadas", body = FinalidadesRespuesta),
        (status = FORBIDDEN, description = "Sin permiso (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin finalidades o sin política vigente (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn otorgar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<OtorgarPeticion>,
) -> ApiResult<(StatusCode, Json<FinalidadesRespuesta>)> {
    let v = uc::registrar_autorizacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        AutorizacionOtorgada {
            persona_id: persona(ruta),
            finalidades: p.finalidades,
            medio: p.medio,
            evidencia_archivo_id: p.evidencia_archivo_id.map(ArchivoId::from_uuid),
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(FinalidadesRespuesta {
            finalidades: v.iter().map(|f| f.as_str().to_owned()).collect(),
        }),
    ))
}

/// Finalidades que se revocan.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RevocarPeticion {
    #[schema(value_type = Vec<String>, example = json!(["publicidad"]))]
    pub finalidades: Vec<FinalidadTratamiento>,
}

/// Autorizaciones revocadas.
#[derive(Debug, Serialize, ToSchema)]
pub struct RevocadasRespuesta {
    pub revocadas: u64,
}

/// Revoca la autorización para esas finalidades (art. 8, lit. e).
#[utoipa::path(
    post,
    path = "/personas/{persona_id}/autorizaciones-tratamiento/revocacion",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("persona_id" = Uuid, Path, description = "Persona")),
    request_body = RevocarPeticion,
    responses(
        (status = OK, description = "Revocadas", body = RevocadasRespuesta),
        (status = FORBIDDEN, description = "Sin permiso (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn revocar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<RevocarPeticion>,
) -> ApiResult<Json<RevocadasRespuesta>> {
    let n = uc::revocar_autorizacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        persona(ruta),
        &p.finalidades,
    )
    .await?;
    Ok(Json(RevocadasRespuesta { revocadas: n }))
}

/// Filtros de las solicitudes.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroSolicitudesQuery {
    #[param(value_type = Option<String>, example = "radicada")]
    pub estado: Option<EstadoSolicitudTitular>,
    pub persona_id: Option<Uuid>,
    /// Sin responder que vencen (con prórroga) hasta esta fecha, p. ej. hoy para las vencidas.
    pub vencen_hasta: Option<NaiveDate>,
}

/// Solicitudes, más recientes primero. El titular ve las suyas; la administración, todas.
#[utoipa::path(
    get,
    path = "/proteccion-datos/solicitudes",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroSolicitudesQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Solicitudes", body = PaginaRespuesta<SolicitudTitularRespuesta>),
        (status = FORBIDDEN, description = "Pide las de otra persona sin permiso (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_solicitudes(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroSolicitudesQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<SolicitudTitularRespuesta>>> {
    let filtro = FiltroSolicitudes {
        estado: f.estado,
        persona_id: f.persona_id.map(PersonaId::from_uuid),
        vencen_hasta: f.vencen_hasta,
    };
    let pagina = uc::listar_solicitudes(&state.unidad, &en.ctx, filtro, p.validar()?).await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Solicitud nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RadicarSolicitudPeticion {
    /// El titular (su `persona_id` sale de `/proteccion-datos/mis-datos`).
    pub persona_id: Uuid,
    #[schema(value_type = String, example = "rectificacion")]
    pub tipo: TipoSolicitudTitular,
    #[schema(example = "Mi número de celular cambió")]
    pub detalle: Option<String>,
}

/// Radica una consulta o un reclamo; el vencimiento se calcula en días hábiles de Colombia.
#[utoipa::path(
    post,
    path = "/proteccion-datos/solicitudes",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = RadicarSolicitudPeticion,
    responses(
        (status = CREATED, description = "Solicitud radicada", body = SolicitudTitularRespuesta),
        (status = FORBIDDEN, description = "Ni es el titular ni tiene `proteccion_datos.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn radicar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<RadicarSolicitudPeticion>,
) -> ApiResult<(StatusCode, Json<SolicitudTitularRespuesta>)> {
    let s = uc::radicar_solicitud(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PersonaId::from_uuid(p.persona_id),
        p.tipo,
        p.detalle,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(s.into())))
}

/// La administración empieza a atender la solicitud.
#[utoipa::path(
    post,
    path = "/proteccion-datos/solicitudes/{solicitud_id}/tramite",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("solicitud_id" = Uuid, Path, description = "Solicitud")),
    responses(
        (status = OK, description = "En trámite", body = SolicitudTitularRespuesta),
        (status = CONFLICT, description = "Ya se respondió (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn tramitar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<SolicitudTitularRespuesta>> {
    Ok(Json(
        uc::tramitar_solicitud(&state.unidad, &en.ctx, solicitud_id(ruta))
            .await?
            .into(),
    ))
}

/// Motivo de la prórroga.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ProrrogaPeticion {
    #[schema(example = "Se verifica la información con la portería")]
    pub motivo: String,
}

/// Prórroga única antes del vencimiento (5 días hábiles las consultas, 8 los reclamos).
#[utoipa::path(
    post,
    path = "/proteccion-datos/solicitudes/{solicitud_id}/prorroga",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("solicitud_id" = Uuid, Path, description = "Solicitud")),
    request_body = ProrrogaPeticion,
    responses(
        (status = OK, description = "Prorrogada", body = SolicitudTitularRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Ya se prorrogó, ya venció o ya se respondió (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn prorrogar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ProrrogaPeticion>,
) -> ApiResult<Json<SolicitudTitularRespuesta>> {
    let s = uc::prorrogar_solicitud(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        solicitud_id(ruta),
        &p.motivo,
    )
    .await?;
    Ok(Json(s.into()))
}

/// Respuesta al titular.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RespuestaSolicitudPeticion {
    pub respuesta: Option<String>,
    /// Documento de respuesta ya subido con `/archivos`.
    pub archivo_id: Option<Uuid>,
}

/// Responde la solicitud (texto, archivo o ambos).
#[utoipa::path(
    post,
    path = "/proteccion-datos/solicitudes/{solicitud_id}/respuesta",
    tag = ETIQUETA_PROTECCION_DATOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("solicitud_id" = Uuid, Path, description = "Solicitud")),
    request_body = RespuestaSolicitudPeticion,
    responses(
        (status = OK, description = "Respondida", body = SolicitudTitularRespuesta),
        (status = CONFLICT, description = "Ya se respondió (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin texto ni archivo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn responder(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<RespuestaSolicitudPeticion>,
) -> ApiResult<Json<SolicitudTitularRespuesta>> {
    let s = uc::responder_solicitud(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        solicitud_id(ruta),
        p.respuesta,
        p.archivo_id.map(ArchivoId::from_uuid),
    )
    .await?;
    Ok(Json(s.into()))
}
