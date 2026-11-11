//! Directorio de comercios y servicios (plan 08, fase 1; ADR 0013).
//!
//! - En una copropiedad: `/api/v1/copropiedades/{copropiedad_id}/directorio/…` (fichas que la
//!   alcanzan, destacadas, categorías, reseñas y reportes).
//! - En la plataforma: `/api/v1/plataforma/publicidad/…` (catálogo de categorías, anunciantes,
//!   fichas, verificación y moderación de reseñas).
//!
//! Las reseñas salen **sin autor**: solo `propia` le dice a quien consulta cuál es la suya.

use application::{
    AppError,
    casos_uso::directorio::{self as uc, DatosAnunciante, DatosFicha, DecisionModeracion},
    puertos::{
        RelojSistema,
        directorio::{
            Anunciante, Categoria, Contacto, Ficha, FichaDirectorio, FiltroAnunciantes,
            FiltroDirectorio, OrganizacionAnunciante, Resena, ResenaReportada,
        },
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{
    AnuncianteId, Cobertura, CodigoCategoria, CodigoDane, Coordenadas, CuentaId, DireccionCorreo,
    EstadoVerificacion, OrganizacionId, ResenaId, Telefono, TenantId, TipoCobertura, Ubicacion,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_DIRECTORIO,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    routes::organizaciones::NuevaOrganizacionPeticion,
    sesion::{EnCopropiedad, OperadorPlataforma},
    state::AppState,
};

/// Rutas dentro de una copropiedad.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(consultar))
        .routes(routes!(destacadas))
        .routes(routes!(categorias))
        .routes(routes!(ficha))
        .routes(routes!(resenas))
        .routes(routes!(resenar))
        .routes(routes!(reportar))
}

/// Rutas de la plataforma (se montan en `/api/v1/plataforma`).
pub fn rutas_plataforma() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(categorias_plataforma))
        .routes(routes!(guardar_categoria))
        .routes(routes!(listar_anunciantes, crear_anunciante))
        .routes(routes!(ver_anunciante, editar_anunciante))
        .routes(routes!(guardar_ficha))
        .routes(routes!(cambiar_estado))
        .routes(routes!(resenas_reportadas))
        .routes(routes!(moderar))
}

fn texto_opcional<T>(v: Option<&T>, f: impl Fn(&T) -> &str) -> Option<String> {
    v.map(|x| f(x).to_owned())
}

fn codigos(v: &[CodigoCategoria]) -> Vec<String> {
    v.iter().map(|c| c.as_str().to_owned()).collect()
}

// ---------------------------------------------------------------------------------------------
// DTOs comunes
// ---------------------------------------------------------------------------------------------

/// Categoría del catálogo.
#[derive(Debug, Serialize, ToSchema)]
pub struct CategoriaRespuesta {
    #[schema(example = "costura")]
    pub codigo: String,
    /// Rama; `null` si es una rama.
    #[schema(example = "oficios")]
    pub padre: Option<String>,
    #[schema(example = "Costura y modistería")]
    pub nombre: String,
    #[schema(example = json!(["modista", "arreglos de ropa"]))]
    pub sinonimos: Vec<String>,
    /// Licores, apuestas, salud con fórmula: no se pueden elegir en una ficha.
    pub restringida: bool,
    pub activa: bool,
}

impl From<Categoria> for CategoriaRespuesta {
    fn from(c: Categoria) -> Self {
        Self {
            codigo: c.codigo.as_str().to_owned(),
            padre: texto_opcional(c.padre.as_ref(), CodigoCategoria::as_str),
            nombre: c.nombre,
            sinonimos: c.sinonimos,
            restringida: c.restringida,
            activa: c.activa,
        }
    }
}

/// Contacto publicado por el anunciante.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContactoRespuesta {
    #[schema(example = "573001234567")]
    pub telefono: Option<String>,
    #[schema(example = "573001234567")]
    pub whatsapp: Option<String>,
    pub correo: Option<String>,
    #[schema(example = "https://modisteriarosa.co")]
    pub sitio_web: Option<String>,
}

impl From<Contacto> for ContactoRespuesta {
    fn from(c: Contacto) -> Self {
        Self {
            telefono: texto_opcional(c.telefono.as_ref(), Telefono::as_str),
            whatsapp: texto_opcional(c.whatsapp.as_ref(), Telefono::as_str),
            correo: texto_opcional(c.correo.as_ref(), DireccionCorreo::as_str),
            sitio_web: c.sitio_web,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Copropiedad
// ---------------------------------------------------------------------------------------------

/// Ficha del directorio vista desde la copropiedad.
#[derive(Debug, Serialize, ToSchema)]
pub struct FichaDirectorioRespuesta {
    pub anunciante_id: Uuid,
    #[schema(example = "Modistería Rosa")]
    pub nombre_comercial: String,
    pub descripcion: Option<String>,
    /// Códigos de categoría; el primero es el principal.
    #[schema(example = json!(["costura"]))]
    pub categorias: Vec<String>,
    pub contacto: ContactoRespuesta,
    #[schema(example = "Lunes a sábado de 8 a. m. a 6 p. m.")]
    pub horario: Option<String>,
    pub direccion: Option<String>,
    #[schema(example = "11001")]
    pub municipio_dane: Option<String>,
    /// Destacada (solo donde la plataforma habilitó el módulo `publicidad`).
    pub destacada: bool,
    /// Kilómetros hasta la copropiedad, si ambos están ubicados.
    #[schema(value_type = Option<String>, example = "1.2")]
    pub distancia_km: Option<Decimal>,
    /// Promedio de todas las copropiedades, con un decimal.
    #[schema(value_type = Option<String>, example = "4.5")]
    pub calificacion_promedio: Option<Decimal>,
    /// Reseñas publicadas en todas las copropiedades.
    pub resenas: u32,
}

impl From<FichaDirectorio> for FichaDirectorioRespuesta {
    fn from(f: FichaDirectorio) -> Self {
        Self {
            anunciante_id: f.anunciante_id.as_uuid(),
            nombre_comercial: f.nombre_comercial,
            descripcion: f.descripcion,
            categorias: codigos(&f.categorias),
            contacto: f.contacto.into(),
            horario: f.horario,
            direccion: f.direccion,
            municipio_dane: texto_opcional(f.municipio.as_ref(), CodigoDane::as_str),
            destacada: f.destacada,
            distancia_km: f.distancia_km,
            calificacion_promedio: f.calificacion.promedio,
            resenas: f.calificacion.total,
        }
    }
}

/// Filtros del directorio.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroDirectorioQuery {
    /// Código de una categoría o de una rama (incluye sus subcategorías).
    #[param(example = "servicios_tecnicos")]
    pub categoria: Option<String>,
    /// Texto libre: nombre, descripción, categoría o sinónimos («modista», «lavadora»).
    #[param(example = "costurera")]
    pub q: Option<String>,
}

fn categoria(v: Option<&str>) -> Result<Option<CodigoCategoria>, ApiError> {
    Ok(v.map(CodigoCategoria::new).transpose()?)
}

/// Directorio de comercios y servicios que alcanzan a la copropiedad, paginado.
#[utoipa::path(
    get,
    path = "/directorio",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroDirectorioQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Fichas", body = PaginaRespuesta<FichaDirectorioRespuesta>),
        (status = FORBIDDEN, description = "Sin `directorio.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn consultar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroDirectorioQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<FichaDirectorioRespuesta>>> {
    let filtro = FiltroDirectorio {
        categoria: categoria(f.categoria.as_deref())?,
        texto: f.q,
        ..FiltroDirectorio::default()
    };
    let pagina =
        uc::consultar_directorio(&state.unidad, &RelojSistema, &en.ctx, filtro, p.validar()?)
            .await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Categoría opcional.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CategoriaQuery {
    /// Código de una categoría o de una rama.
    pub categoria: Option<String>,
}

/// Fichas destacadas vigentes (hasta 10). Vacío si la copropiedad no tiene el módulo
/// `publicidad`.
#[utoipa::path(
    get,
    path = "/directorio/destacadas",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), CategoriaQuery),
    responses(
        (status = OK, description = "Destacadas", body = Vec<FichaDirectorioRespuesta>),
        (status = FORBIDDEN, description = "Sin `directorio.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn destacadas(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(q): Query<CategoriaQuery>,
) -> ApiResult<Json<Vec<FichaDirectorioRespuesta>>> {
    let v = uc::destacadas(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        categoria(q.categoria.as_deref())?,
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Categorías para filtrar el directorio (activas y no restringidas).
#[utoipa::path(
    get,
    path = "/directorio/categorias",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Categorías", body = Vec<CategoriaRespuesta>),
        (status = FORBIDDEN, description = "Sin `directorio.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn categorias(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<CategoriaRespuesta>>> {
    let v = uc::categorias_directorio(&state.unidad, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Ficha de un anunciante que alcanza a la copropiedad.
#[utoipa::path(
    get,
    path = "/directorio/anunciantes/{anunciante_id}",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("anunciante_id" = Uuid, Path, description = "Anunciante"),
    ),
    responses(
        (status = OK, description = "Ficha", body = FichaDirectorioRespuesta),
        (status = NOT_FOUND, description = "No existe o no alcanza a la copropiedad", body = ErrorRespuesta),
    )
)]
pub async fn ficha(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<FichaDirectorioRespuesta>> {
    let f = uc::ficha_directorio(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        AnuncianteId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(f.into()))
}

/// Reseña de un residente o propietario (sin autor).
#[derive(Debug, Serialize, ToSchema)]
pub struct ResenaRespuesta {
    pub id: Uuid,
    /// De 1 a 5.
    #[schema(example = 5)]
    pub calificacion: u8,
    #[schema(example = "Muy cumplida con los arreglos")]
    pub texto: Option<String>,
    /// `true` si es la reseña de quien consulta.
    pub propia: bool,
    pub creada_en: DateTime<Utc>,
    pub actualizada_en: DateTime<Utc>,
}

fn resena_respuesta(r: Resena, quien: Option<CuentaId>) -> ResenaRespuesta {
    ResenaRespuesta {
        id: r.id.as_uuid(),
        calificacion: r.calificacion.valor(),
        texto: r.texto,
        propia: quien == Some(r.cuenta_id),
        creada_en: r.creada_en,
        actualizada_en: r.actualizada_en,
    }
}

/// Reseñas publicadas del anunciante en esta copropiedad, de la más reciente a la más antigua.
#[utoipa::path(
    get,
    path = "/directorio/anunciantes/{anunciante_id}/resenas",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("anunciante_id" = Uuid, Path, description = "Anunciante"),
        ParametrosPagina,
    ),
    responses(
        (status = OK, description = "Reseñas", body = PaginaRespuesta<ResenaRespuesta>),
        (status = NOT_FOUND, description = "La ficha no alcanza a la copropiedad", body = ErrorRespuesta),
    )
)]
pub async fn resenas(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<ResenaRespuesta>>> {
    let pagina = uc::resenas_de_anunciante(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        AnuncianteId::from_uuid(ruta.1),
        p.validar()?,
    )
    .await?;
    let quien = en.ctx.actor_id;
    Ok(Json(pagina.mapear(|r| resena_respuesta(r, quien)).into()))
}

/// Calificación de un anunciante.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResenaPeticion {
    /// De 1 a 5.
    #[schema(example = 5)]
    pub calificacion: u8,
    /// Hasta 1000 caracteres.
    pub texto: Option<String>,
}

/// Califica (o vuelve a calificar) a un anunciante. Solo propietarios y residentes vigentes;
/// una reseña por cuenta y anunciante en la copropiedad.
#[utoipa::path(
    put,
    path = "/directorio/anunciantes/{anunciante_id}/resena",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("anunciante_id" = Uuid, Path, description = "Anunciante"),
    ),
    request_body = ResenaPeticion,
    responses(
        (status = OK, description = "Reseña guardada", body = ResenaRespuesta),
        (status = FORBIDDEN, description = "Sin `directorio.resenar` o no es propietario ni residente vigente (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La ficha no alcanza a la copropiedad", body = ErrorRespuesta),
        (status = CONFLICT, description = "La moderación ocultó su reseña anterior (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Calificación o texto inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn resenar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ResenaPeticion>,
) -> ApiResult<Json<ResenaRespuesta>> {
    let r = uc::resenar(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        AnuncianteId::from_uuid(ruta.1),
        p.calificacion,
        p.texto.as_deref(),
    )
    .await?;
    Ok(Json(resena_respuesta(r, en.ctx.actor_id)))
}

/// Motivo del reporte.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReportePeticion {
    /// Hasta 500 caracteres.
    #[schema(example = "Lenguaje ofensivo")]
    pub motivo: String,
}

/// Resultado del reporte.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReporteRespuesta {
    /// `false` si ya la había reportado.
    pub nuevo: bool,
}

/// Reporta una reseña publicada para que la modere la plataforma.
#[utoipa::path(
    post,
    path = "/directorio/resenas/{resena_id}/reportes",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("resena_id" = Uuid, Path, description = "Reseña"),
    ),
    request_body = ReportePeticion,
    responses(
        (status = OK, description = "Reporte registrado", body = ReporteRespuesta),
        (status = NOT_FOUND, description = "La reseña no existe o ya está oculta", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin motivo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn reportar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ReportePeticion>,
) -> ApiResult<Json<ReporteRespuesta>> {
    let nuevo = uc::reportar_resena(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ResenaId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(Json(ReporteRespuesta { nuevo }))
}

// ---------------------------------------------------------------------------------------------
// Plataforma: catálogo
// ---------------------------------------------------------------------------------------------

/// Todo el catálogo de categorías (incluidas las restringidas e inactivas).
#[utoipa::path(
    get,
    path = "/publicidad/categorias",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Catálogo", body = Vec<CategoriaRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.directorio.gestionar`", body = ErrorRespuesta),
    )
)]
pub async fn categorias_plataforma(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
) -> ApiResult<Json<Vec<CategoriaRespuesta>>> {
    let v = uc::categorias_plataforma(&state.plataforma, &op.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Datos de una categoría.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CategoriaPeticion {
    /// Rama (debe ser una categoría sin rama); se omite para crear una rama.
    #[schema(example = "oficios")]
    pub padre: Option<String>,
    #[schema(example = "Costura y modistería")]
    pub nombre: String,
    /// Otras formas de nombrarla (hasta 30, de hasta 60 caracteres).
    #[serde(default)]
    pub sinonimos: Vec<String>,
    #[serde(default)]
    pub restringida: bool,
    /// `true` si se omite.
    pub activa: Option<bool>,
}

/// Crea o edita una categoría del catálogo.
#[utoipa::path(
    put,
    path = "/publicidad/categorias/{codigo}",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("codigo" = String, Path, description = "Código (a-z, 0-9, _)", example = "costura")),
    request_body = CategoriaPeticion,
    responses(
        (status = OK, description = "Categoría guardada", body = CategoriaRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.directorio.gestionar`", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos o rama que no existe (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn guardar_categoria(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(codigo): Path<String>,
    Json(p): Json<CategoriaPeticion>,
) -> ApiResult<Json<CategoriaRespuesta>> {
    let c = Categoria {
        codigo: CodigoCategoria::new(&codigo)?,
        padre: categoria(p.padre.as_deref())?,
        nombre: p.nombre,
        sinonimos: p.sinonimos,
        restringida: p.restringida,
        activa: p.activa.unwrap_or(true),
    };
    Ok(Json(
        uc::guardar_categoria(&state.plataforma, &op.ctx, c)
            .await?
            .into(),
    ))
}

// ---------------------------------------------------------------------------------------------
// Plataforma: anunciantes y fichas
// ---------------------------------------------------------------------------------------------

/// Ficha completa del anunciante (vista de la plataforma).
#[derive(Debug, Serialize, ToSchema)]
pub struct FichaRespuesta {
    pub descripcion: Option<String>,
    #[schema(example = json!(["costura", "belleza_domicilio"]))]
    pub categorias: Vec<String>,
    /// `nacional`, `municipio` o `radio`.
    #[schema(example = "radio")]
    pub cobertura: String,
    #[schema(example = 3)]
    pub radio_km: Option<u16>,
    pub contacto: ContactoRespuesta,
    pub horario: Option<String>,
    pub destacada_hasta: Option<DateTime<Utc>>,
    pub publicada: bool,
    pub actualizada_en: DateTime<Utc>,
}

impl From<Ficha> for FichaRespuesta {
    fn from(f: Ficha) -> Self {
        Self {
            descripcion: f.descripcion,
            categorias: codigos(&f.categorias),
            cobertura: f.cobertura.tipo().as_str().to_owned(),
            radio_km: f.cobertura.radio_km(),
            contacto: f.contacto.into(),
            horario: f.horario,
            destacada_hasta: f.destacada_hasta,
            publicada: f.publicada,
            actualizada_en: f.actualizada_en,
        }
    }
}

/// Anunciante del directorio.
#[derive(Debug, Serialize, ToSchema)]
pub struct AnuncianteRespuesta {
    pub id: Uuid,
    pub organizacion_id: Uuid,
    #[schema(example = "Modistería Rosa")]
    pub nombre_comercial: String,
    pub direccion: Option<String>,
    #[schema(example = "11001")]
    pub municipio_dane: Option<String>,
    #[schema(value_type = Option<String>, example = "4.711000")]
    pub latitud: Option<Decimal>,
    #[schema(value_type = Option<String>, example = "-74.072100")]
    pub longitud: Option<Decimal>,
    /// `pendiente`, `verificado`, `rechazado` o `suspendido`.
    pub estado: String,
    pub motivo_estado: Option<String>,
    pub creado_en: DateTime<Utc>,
    pub actualizado_en: DateTime<Utc>,
    /// `null` si todavía no tiene ficha.
    pub ficha: Option<FichaRespuesta>,
}

fn anunciante_respuesta(a: Anunciante, ficha: Option<Ficha>) -> AnuncianteRespuesta {
    let c = a.ubicacion.coordenadas;
    AnuncianteRespuesta {
        id: a.id.as_uuid(),
        organizacion_id: a.organizacion_id.as_uuid(),
        nombre_comercial: a.nombre_comercial,
        direccion: a.direccion,
        municipio_dane: texto_opcional(a.ubicacion.municipio.as_ref(), CodigoDane::as_str),
        latitud: c.map(Coordenadas::latitud),
        longitud: c.map(Coordenadas::longitud),
        estado: a.estado.as_str().to_owned(),
        motivo_estado: a.motivo_estado,
        creado_en: a.creado_en,
        actualizado_en: a.actualizado_en,
        ficha: ficha.map(Into::into),
    }
}

/// Filtros de anunciantes.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroAnunciantesQuery {
    /// `pendiente`, `verificado`, `rechazado` o `suspendido`.
    pub estado: Option<String>,
    /// Fragmento del nombre comercial.
    pub q: Option<String>,
}

/// Anunciantes de la plataforma, paginados.
#[utoipa::path(
    get,
    path = "/publicidad/anunciantes",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(FiltroAnunciantesQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Anunciantes (sin ficha)", body = PaginaRespuesta<AnuncianteRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.directorio.gestionar` ni `plataforma.publicidad.moderar`", body = ErrorRespuesta),
    )
)]
pub async fn listar_anunciantes(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Query(f): Query<FiltroAnunciantesQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<AnuncianteRespuesta>>> {
    let filtro = FiltroAnunciantes {
        estado: f
            .estado
            .as_deref()
            .map(EstadoVerificacion::try_from)
            .transpose()?,
        texto: f.q,
    };
    let pagina = uc::listar_anunciantes(&state.plataforma, &op.ctx, &filtro, p.validar()?).await?;
    Ok(Json(
        pagina.mapear(|a| anunciante_respuesta(a, None)).into(),
    ))
}

/// Nombre, dirección y ubicación de un anunciante.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DatosAnunciantePeticion {
    #[schema(example = "Modistería Rosa")]
    pub nombre_comercial: String,
    pub direccion: Option<String>,
    /// Código DANE del municipio (5 dígitos).
    #[schema(example = "11001")]
    pub municipio_dane: Option<String>,
    #[schema(value_type = Option<String>, example = "4.711000")]
    pub latitud: Option<Decimal>,
    #[schema(value_type = Option<String>, example = "-74.072100")]
    pub longitud: Option<Decimal>,
}

impl TryFrom<DatosAnunciantePeticion> for DatosAnunciante {
    type Error = ApiError;

    fn try_from(p: DatosAnunciantePeticion) -> Result<Self, ApiError> {
        let coordenadas = match (p.latitud, p.longitud) {
            (Some(la), Some(lo)) => Some(Coordenadas::new(la, lo)?),
            (None, None) => None,
            _ => {
                return Err(ApiError::App(AppError::Validacion(
                    "envíe latitud y longitud juntas".into(),
                )));
            }
        };
        Ok(Self {
            nombre_comercial: p.nombre_comercial,
            direccion: p.direccion,
            ubicacion: Ubicacion {
                coordenadas,
                municipio: p
                    .municipio_dane
                    .as_deref()
                    .map(CodigoDane::new)
                    .transpose()?,
            },
        })
    }
}

/// Alta de un anunciante.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoAnunciantePeticion {
    /// Organización existente. Se envía esta **o** `organizacion`.
    pub organizacion_id: Option<Uuid>,
    /// Organización nueva (si el NIT ya existe, se reutiliza).
    pub organizacion: Option<NuevaOrganizacionPeticion>,
    #[serde(flatten)]
    pub datos: DatosAnunciantePeticion,
}

/// Da de alta un anunciante (queda verificado: la plataforma lo revisa al darlo de alta).
#[utoipa::path(
    post,
    path = "/publicidad/anunciantes",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    request_body = NuevoAnunciantePeticion,
    responses(
        (status = CREATED, description = "Anunciante creado", body = AnuncianteRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.directorio.gestionar`", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La organización no existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "La organización ya tiene un anunciante (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_anunciante(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevoAnunciantePeticion>,
) -> ApiResult<(StatusCode, Json<AnuncianteRespuesta>)> {
    let organizacion = match (p.organizacion_id, p.organizacion) {
        (Some(id), None) => OrganizacionAnunciante::Existente(OrganizacionId::from_uuid(id)),
        (None, Some(nueva)) => OrganizacionAnunciante::Nueva(nueva.try_into()?),
        _ => {
            return Err(ApiError::App(AppError::Validacion(
                "envíe `organizacion_id` o `organizacion`, no ambos".into(),
            )));
        }
    };
    let a = uc::dar_de_alta_anunciante(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        organizacion,
        p.datos.try_into()?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(anunciante_respuesta(a, None))))
}

/// Anunciante con su ficha.
#[utoipa::path(
    get,
    path = "/publicidad/anunciantes/{anunciante_id}",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("anunciante_id" = Uuid, Path, description = "Anunciante")),
    responses(
        (status = OK, description = "Anunciante", body = AnuncianteRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver_anunciante(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<AnuncianteRespuesta>> {
    let (a, f) = uc::anunciante(&state.plataforma, &op.ctx, AnuncianteId::from_uuid(id)).await?;
    Ok(Json(anunciante_respuesta(a, f)))
}

/// Edita nombre, dirección y ubicación del anunciante.
#[utoipa::path(
    put,
    path = "/publicidad/anunciantes/{anunciante_id}",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("anunciante_id" = Uuid, Path, description = "Anunciante")),
    request_body = DatosAnunciantePeticion,
    responses(
        (status = OK, description = "Anunciante actualizado (sin ficha)", body = AnuncianteRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos o la ubicación no sirve para la cobertura de la ficha (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn editar_anunciante(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<DatosAnunciantePeticion>,
) -> ApiResult<Json<AnuncianteRespuesta>> {
    let a = uc::editar_anunciante(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        AnuncianteId::from_uuid(id),
        p.try_into()?,
    )
    .await?;
    Ok(Json(anunciante_respuesta(a, None)))
}

/// Contenido de la ficha.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FichaPeticion {
    /// Hasta 1000 caracteres.
    pub descripcion: Option<String>,
    /// De 1 a 5 códigos; el primero es el principal. Nunca una categoría restringida.
    #[schema(example = json!(["costura"]))]
    pub categorias: Vec<String>,
    /// `nacional`, `municipio` o `radio`.
    #[schema(example = "radio")]
    pub cobertura: String,
    /// Solo con `radio` (1 a 100 km).
    #[schema(example = 3)]
    pub radio_km: Option<u16>,
    #[schema(example = "573001234567")]
    pub telefono: Option<String>,
    #[schema(example = "573001234567")]
    pub whatsapp: Option<String>,
    pub correo: Option<String>,
    /// Solo `https://`.
    pub sitio_web: Option<String>,
    pub horario: Option<String>,
    /// Destacada (suscripción `directorio_destacado`) hasta esta fecha.
    pub destacada_hasta: Option<DateTime<Utc>>,
    /// `true` si se omite; `false` la retira del directorio.
    pub publicada: Option<bool>,
}

impl TryFrom<FichaPeticion> for DatosFicha {
    type Error = ApiError;

    fn try_from(p: FichaPeticion) -> Result<Self, ApiError> {
        Ok(Self {
            descripcion: p.descripcion,
            categorias: p
                .categorias
                .iter()
                .map(|c| CodigoCategoria::new(c))
                .collect::<Result<_, _>>()?,
            cobertura: Cobertura::new(TipoCobertura::try_from(p.cobertura.as_str())?, p.radio_km)?,
            contacto: Contacto {
                telefono: p.telefono.as_deref().map(Telefono::new).transpose()?,
                whatsapp: p.whatsapp.as_deref().map(Telefono::new).transpose()?,
                correo: p.correo.as_deref().map(DireccionCorreo::new).transpose()?,
                sitio_web: p.sitio_web.filter(|s| !s.trim().is_empty()),
            },
            horario: p.horario,
            destacada_hasta: p.destacada_hasta,
            publicada: p.publicada.unwrap_or(true),
        })
    }
}

/// Crea o reemplaza la ficha del directorio del anunciante.
#[utoipa::path(
    put,
    path = "/publicidad/anunciantes/{anunciante_id}/ficha",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("anunciante_id" = Uuid, Path, description = "Anunciante")),
    request_body = FichaPeticion,
    responses(
        (status = OK, description = "Ficha guardada", body = FichaRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.directorio.gestionar`", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "El anunciante no existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Categoría restringida o inexistente, cobertura imposible o datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn guardar_ficha(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<FichaPeticion>,
) -> ApiResult<Json<FichaRespuesta>> {
    let f = uc::guardar_ficha(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        AnuncianteId::from_uuid(id),
        p.try_into()?,
    )
    .await?;
    Ok(Json(f.into()))
}

/// Cambio de estado de verificación.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EstadoAnunciantePeticion {
    /// `verificado`, `rechazado`, `suspendido` o `pendiente`.
    #[schema(example = "suspendido")]
    pub estado: String,
    /// Obligatorio para rechazar o suspender.
    #[schema(example = "Quejas por publicidad engañosa")]
    pub motivo: Option<String>,
}

/// Verifica, rechaza, suspende o reactiva un anunciante.
#[utoipa::path(
    post,
    path = "/publicidad/anunciantes/{anunciante_id}/estado",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("anunciante_id" = Uuid, Path, description = "Anunciante")),
    request_body = EstadoAnunciantePeticion,
    responses(
        (status = OK, description = "Estado cambiado", body = AnuncianteRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.moderar`", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "Transición no permitida (`transicion_invalida`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin motivo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn cambiar_estado(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<EstadoAnunciantePeticion>,
) -> ApiResult<Json<AnuncianteRespuesta>> {
    let a = uc::cambiar_verificacion(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        AnuncianteId::from_uuid(id),
        EstadoVerificacion::try_from(p.estado.as_str())?,
        p.motivo.as_deref(),
    )
    .await?;
    Ok(Json(anunciante_respuesta(a, None)))
}

// ---------------------------------------------------------------------------------------------
// Plataforma: moderación de reseñas
// ---------------------------------------------------------------------------------------------

/// Reseña con reportes sin atender.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResenaReportadaRespuesta {
    pub resena_id: Uuid,
    pub copropiedad_id: Uuid,
    pub copropiedad: String,
    pub anunciante_id: Uuid,
    pub nombre_comercial: String,
    pub calificacion: u8,
    pub texto: Option<String>,
    pub reportes: u32,
    pub motivos: Vec<String>,
    pub creada_en: DateTime<Utc>,
}

impl From<ResenaReportada> for ResenaReportadaRespuesta {
    fn from(r: ResenaReportada) -> Self {
        Self {
            resena_id: r.resena_id.as_uuid(),
            copropiedad_id: r.tenant_id.as_uuid(),
            copropiedad: r.copropiedad,
            anunciante_id: r.anunciante_id.as_uuid(),
            nombre_comercial: r.nombre_comercial,
            calificacion: r.calificacion.valor(),
            texto: r.texto,
            reportes: r.reportes,
            motivos: r.motivos,
            creada_en: r.creada_en,
        }
    }
}

/// Cola de moderación: reseñas publicadas con reportes sin atender, de todas las copropiedades.
#[utoipa::path(
    get,
    path = "/publicidad/moderacion/resenas",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(ParametrosPagina),
    responses(
        (status = OK, description = "Reseñas reportadas", body = PaginaRespuesta<ResenaReportadaRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.moderar`", body = ErrorRespuesta),
    )
)]
pub async fn resenas_reportadas(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<ResenaReportadaRespuesta>>> {
    let pagina = uc::resenas_reportadas(&state.plataforma, &op.ctx, p.validar()?).await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Decisión sobre una reseña reportada.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ModeracionPeticion {
    /// Copropiedad de la reseña (viene en la cola de moderación).
    pub copropiedad_id: Uuid,
    /// `true` la oculta (exige `motivo`); `false` la mantiene. En ambos casos los reportes
    /// quedan atendidos.
    pub ocultar: bool,
    #[schema(example = "Lenguaje ofensivo")]
    pub motivo: Option<String>,
}

/// Atiende los reportes de una reseña: la oculta con motivo o la mantiene.
#[utoipa::path(
    post,
    path = "/publicidad/moderacion/resenas/{resena_id}",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("resena_id" = Uuid, Path, description = "Reseña")),
    request_body = ModeracionPeticion,
    responses(
        (status = NO_CONTENT, description = "Moderada"),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.moderar`", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe en esa copropiedad", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Ocultar sin motivo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn moderar(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<ModeracionPeticion>,
) -> ApiResult<StatusCode> {
    let decision = if p.ocultar {
        DecisionModeracion::Ocultar {
            motivo: p.motivo.unwrap_or_default(),
        }
    } else {
        DecisionModeracion::Mantener
    };
    uc::moderar_resena(
        &state.unidad,
        &RelojSistema,
        &op.ctx,
        TenantId::from_uuid(p.copropiedad_id),
        ResenaId::from_uuid(id),
        decision,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
