//! Parqueaderos y sorteos (plan 04): `/parqueaderos`, `/asignaciones-parqueadero` y
//! `/sorteos`, dentro de `/api/v1/copropiedades/{copropiedad_id}`.

use application::{
    casos_uso::parqueaderos::{
        self as uc, NuevoParqueadero, NuevoSorteo, ParqueaderoConAsignacion,
        SolicitudAsignacionParqueadero, VistaSorteo,
    },
    puertos::{RelojSistema, reservas::AsignacionParqueadero},
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{
    AsignacionParqueaderoId, BienPrivadoId, EstadoSorteo, NaturalezaParqueadero,
    OrigenAsignacionParqueadero, RecursoId, SorteoId, TipoParqueadero, VehiculoId,
    sorteo::CriteriosSorteo,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path},
    openapi::ETIQUETA_RESERVAS,
    routes::{
        ficha::HorarioDto,
        reservas::{PoliticaDto, RecursoRespuesta},
    },
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de parqueaderos y sorteos.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar, crear))
        .routes(routes!(asignar))
        .routes(routes!(liberar))
        .routes(routes!(listar_sorteos, crear_sorteo))
        .routes(routes!(inscribir))
        .routes(routes!(ejecutar))
}

/// Asignación de un parqueadero.
#[derive(Debug, Serialize, ToSchema)]
pub struct AsignacionRespuesta {
    pub id: Uuid,
    pub recurso_id: Uuid,
    pub bien_privado_id: Uuid,
    pub vehiculo_id: Option<Uuid>,
    /// `propiedad`, `sorteo`, `arriendo` o `manual`.
    #[schema(value_type = String, example = "sorteo")]
    pub origen: OrigenAsignacionParqueadero,
    pub desde: DateTime<Utc>,
    pub hasta: Option<DateTime<Utc>>,
    pub sorteo_id: Option<Uuid>,
}

impl From<AsignacionParqueadero> for AsignacionRespuesta {
    fn from(a: AsignacionParqueadero) -> Self {
        Self {
            id: a.id.as_uuid(),
            recurso_id: a.recurso_id.as_uuid(),
            bien_privado_id: a.bien_privado_id.as_uuid(),
            vehiculo_id: a.vehiculo_id.map(VehiculoId::as_uuid),
            origen: a.origen,
            desde: a.desde,
            hasta: a.hasta,
            sorteo_id: a.sorteo_id.map(SorteoId::as_uuid),
        }
    }
}

/// Parqueadero con su asignación vigente.
#[derive(Debug, Serialize, ToSchema)]
pub struct ParqueaderoRespuesta {
    pub recurso: RecursoRespuesta,
    pub asignacion: Option<AsignacionRespuesta>,
}

impl From<ParqueaderoConAsignacion> for ParqueaderoRespuesta {
    fn from(p: ParqueaderoConAsignacion) -> Self {
        Self {
            recurso: p.recurso.into(),
            asignacion: p.asignacion.map(Into::into),
        }
    }
}

/// Parqueaderos con su asignación vigente.
#[utoipa::path(
    get,
    path = "/parqueaderos",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Parqueaderos", body = Vec<ParqueaderoRespuesta>),
        (status = FORBIDDEN, description = "Sin `parqueaderos.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<ParqueaderoRespuesta>>> {
    let lista = uc::listar_parqueaderos(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Parqueadero nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoParqueaderoPeticion {
    #[schema(example = "S1-045")]
    pub codigo: String,
    #[schema(value_type = String, example = "carro")]
    pub tipo_vehiculo: TipoParqueadero,
    /// `privado`, `uso_exclusivo`, `comunal` o `visitantes`.
    #[schema(value_type = String, example = "comunal")]
    pub naturaleza: NaturalezaParqueadero,
    /// Solo para los privados.
    pub bien_privado_id: Option<Uuid>,
    pub ubicacion: Option<String>,
    /// Solo visitantes (por defecto: franjas inmediatas de 30 min a 4 h, hasta 7 días antes).
    pub politica: Option<PoliticaDto>,
    /// Horario de uso (visitantes).
    #[serde(default)]
    pub horario: HorarioDto,
}

/// Crea un parqueadero (privado, de uso exclusivo, comunal o de visitantes).
#[utoipa::path(
    post,
    path = "/parqueaderos",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoParqueaderoPeticion,
    responses(
        (status = CREATED, description = "Parqueadero creado", body = RecursoRespuesta),
        (status = CONFLICT, description = "Código repetido (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos incoherentes (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoParqueaderoPeticion>,
) -> ApiResult<(StatusCode, Json<RecursoRespuesta>)> {
    let r = uc::crear_parqueadero(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoParqueadero {
            codigo: p.codigo,
            tipo_vehiculo: p.tipo_vehiculo,
            naturaleza: p.naturaleza,
            bien_privado_id: p.bien_privado_id.map(BienPrivadoId::from_uuid),
            ubicacion: p.ubicacion,
            politica: p.politica.map(Into::into),
            horario: p.horario.a_dominio()?,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Asignación nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AsignacionPeticion {
    pub bien_privado_id: Uuid,
    pub vehiculo_id: Option<Uuid>,
    /// `propiedad`, `arriendo` o `manual` (las de sorteo las crea el sorteo).
    #[schema(value_type = String, example = "arriendo")]
    pub origen: OrigenAsignacionParqueadero,
    pub desde: Option<DateTime<Utc>>,
    pub hasta: Option<DateTime<Utc>>,
}

/// Asigna un parqueadero a un bien privado por un periodo.
#[utoipa::path(
    post,
    path = "/parqueaderos/{recurso_id}/asignaciones",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("recurso_id" = Uuid, Path, description = "Parqueadero"),
    ),
    request_body = AsignacionPeticion,
    responses(
        (status = CREATED, description = "Asignado", body = AsignacionRespuesta),
        (status = CONFLICT, description = "Ya asignado en ese periodo (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn asignar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AsignacionPeticion>,
) -> ApiResult<(StatusCode, Json<AsignacionRespuesta>)> {
    let a = uc::asignar_parqueadero(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        SolicitudAsignacionParqueadero {
            recurso_id: RecursoId::from_uuid(ruta.1),
            bien_privado_id: BienPrivadoId::from_uuid(p.bien_privado_id),
            vehiculo_id: p.vehiculo_id.map(VehiculoId::from_uuid),
            origen: p.origen,
            desde: p.desde,
            hasta: p.hasta,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(a.into())))
}

/// Libera un parqueadero (termina la asignación desde ahora).
#[utoipa::path(
    post,
    path = "/asignaciones-parqueadero/{asignacion_id}/liberar",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("asignacion_id" = Uuid, Path, description = "Asignación"),
    ),
    responses(
        (status = NO_CONTENT, description = "Liberado"),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn liberar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    uc::liberar_parqueadero(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        AsignacionParqueaderoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Puntos de cada prioridad del sorteo.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
pub struct CriteriosDto {
    #[schema(example = 100)]
    pub puntos_discapacidad: u32,
    #[schema(example = 10)]
    pub puntos_no_favorecido: u32,
}

impl From<CriteriosDto> for CriteriosSorteo {
    fn from(c: CriteriosDto) -> Self {
        Self {
            puntos_discapacidad: c.puntos_discapacidad,
            puntos_no_favorecido: c.puntos_no_favorecido,
        }
    }
}

impl From<CriteriosSorteo> for CriteriosDto {
    fn from(c: CriteriosSorteo) -> Self {
        Self {
            puntos_discapacidad: c.puntos_discapacidad,
            puntos_no_favorecido: c.puntos_no_favorecido,
        }
    }
}

/// Sorteo. La `semilla` aparece solo después de ejecutarlo; antes, su `compromiso` (SHA-256)
/// permite verificar que no se cambió.
#[derive(Debug, Serialize, ToSchema)]
pub struct SorteoRespuesta {
    pub id: Uuid,
    pub nombre: String,
    pub recursos: Vec<Uuid>,
    pub vigencia_desde: DateTime<Utc>,
    pub vigencia_hasta: DateTime<Utc>,
    /// `abierto` o `ejecutado`.
    #[schema(value_type = String, example = "abierto")]
    pub estado: EstadoSorteo,
    pub compromiso: String,
    pub semilla: Option<String>,
    pub criterios: CriteriosDto,
    /// Orden de los bienes privados (el primero, el primer favorecido).
    pub resultado: Option<Vec<Uuid>>,
    pub inscritos: Vec<Uuid>,
    pub ejecutado_en: Option<DateTime<Utc>>,
}

impl From<VistaSorteo> for SorteoRespuesta {
    fn from(v: VistaSorteo) -> Self {
        let s = v.sorteo;
        Self {
            id: s.id.as_uuid(),
            nombre: s.nombre,
            recursos: s.recursos.into_iter().map(RecursoId::as_uuid).collect(),
            vigencia_desde: s.vigencia_desde,
            vigencia_hasta: s.vigencia_hasta,
            estado: s.estado,
            compromiso: s.compromiso,
            semilla: v.semilla_revelada,
            criterios: s.criterios.into(),
            resultado: s
                .resultado
                .map(|r| r.into_iter().map(BienPrivadoId::as_uuid).collect()),
            inscritos: v
                .inscripciones
                .into_iter()
                .map(|i| i.bien_privado_id.as_uuid())
                .collect(),
            ejecutado_en: s.ejecutado_en,
        }
    }
}

/// Sorteos de la copropiedad (públicos para quien ve los bienes comunes).
#[utoipa::path(
    get,
    path = "/sorteos",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses((status = OK, description = "Sorteos", body = Vec<SorteoRespuesta>))
)]
pub async fn listar_sorteos(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<SorteoRespuesta>>> {
    let lista = uc::listar_sorteos(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Sorteo nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoSorteoPeticion {
    #[schema(example = "Parqueaderos comunales, primer semestre 2027")]
    pub nombre: String,
    /// Parqueaderos comunales que se adjudican.
    pub recursos: Vec<Uuid>,
    /// Periodo de las asignaciones (p. ej. 3, 6 o 12 meses).
    pub vigencia_desde: DateTime<Utc>,
    pub vigencia_hasta: DateTime<Utc>,
    pub criterios: Option<CriteriosDto>,
}

/// Crea un sorteo y publica el compromiso de su semilla.
#[utoipa::path(
    post,
    path = "/sorteos",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoSorteoPeticion,
    responses(
        (status = CREATED, description = "Sorteo creado", body = SorteoRespuesta),
        (status = FORBIDDEN, description = "Sin `sorteos.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin parqueaderos comunales o vigencia inválida (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_sorteo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoSorteoPeticion>,
) -> ApiResult<(StatusCode, Json<SorteoRespuesta>)> {
    let s = uc::crear_sorteo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoSorteo {
            nombre: p.nombre,
            recursos: p.recursos.into_iter().map(RecursoId::from_uuid).collect(),
            vigencia_desde: p.vigencia_desde,
            vigencia_hasta: p.vigencia_hasta,
            criterios: p.criterios.map(Into::into).unwrap_or_default(),
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(
            VistaSorteo {
                sorteo: s,
                semilla_revelada: None,
                inscripciones: Vec::new(),
            }
            .into(),
        ),
    ))
}

/// Inscripción en un sorteo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct InscripcionPeticion {
    pub bien_privado_id: Uuid,
    /// Inscribe a una persona con discapacidad (dato sensible: prioridad del sorteo).
    #[serde(default)]
    pub discapacidad: bool,
}

/// Inscribe un bien privado (lo hace su titular o residente, o la administración).
#[utoipa::path(
    post,
    path = "/sorteos/{sorteo_id}/inscripciones",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("sorteo_id" = Uuid, Path, description = "Sorteo"),
    ),
    request_body = InscripcionPeticion,
    responses(
        (status = NO_CONTENT, description = "Inscrito"),
        (status = CONFLICT, description = "Ya inscrito o sorteo ejecutado (`conflicto`)", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin vínculo con el bien ni `sorteos.gestionar`", body = ErrorRespuesta),
    )
)]
pub async fn inscribir(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<InscripcionPeticion>,
) -> ApiResult<StatusCode> {
    uc::inscribirse_sorteo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        SorteoId::from_uuid(ruta.1),
        BienPrivadoId::from_uuid(p.bien_privado_id),
        p.discapacidad,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Resultado del sorteo con las asignaciones creadas.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResultadoRespuesta {
    pub sorteo: SorteoRespuesta,
    pub asignaciones: Vec<AsignacionRespuesta>,
}

/// Ejecuta el sorteo: revela la semilla y asigna los parqueaderos por el periodo.
#[utoipa::path(
    post,
    path = "/sorteos/{sorteo_id}/ejecutar",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("sorteo_id" = Uuid, Path, description = "Sorteo"),
    ),
    responses(
        (status = OK, description = "Ejecutado", body = ResultadoRespuesta),
        (status = CONFLICT, description = "Ya ejecutado o parqueadero ocupado (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn ejecutar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ResultadoRespuesta>> {
    let r = uc::ejecutar_sorteo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        SorteoId::from_uuid(ruta.1),
    )
    .await?;
    let semilla = r.sorteo.semilla.as_str().to_owned();
    Ok(Json(ResultadoRespuesta {
        sorteo: VistaSorteo {
            sorteo: r.sorteo,
            semilla_revelada: Some(semilla),
            inscripciones: Vec::new(),
        }
        .into(),
        asignaciones: r.asignaciones.into_iter().map(Into::into).collect(),
    }))
}
