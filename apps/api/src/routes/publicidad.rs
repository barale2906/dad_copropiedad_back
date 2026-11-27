//! Publicidad de la plataforma (plan 08, fases 3 a 6; ADR 0015).
//!
//! - Plataforma, `/api/v1/plataforma/publicidad/…`: el personal carga las campañas y piezas de
//!   cualquier comercio (`plataforma.publicidad.gestionar`; los comercios no usan la plataforma,
//!   plan 20), sus métricas y leads, el enlace de cupones del comercio y las exclusiones; la
//!   moderación (otra cuenta), reportes, tarifas, política, liquidaciones y pantallas.
//! - Comercio, `/api/v1/publicidad/cupones/redimir` (sin sesión, `X-Comercio`): redime cupones.
//! - Entidad, `/api/v1/entidades/{entidad_id}/publicidad/…`: autorización del titular, piezas,
//!   eventos, cupones y reportes (solo para quien autorizó), en PH y empresas.
//! - Copropiedad, `/api/v1/copropiedades/{copropiedad_id}/publicidad/pantalla`: la pantalla del
//!   lobby (sin sesión, `X-Pantalla`).

use application::{
    AppError, ContextoPlataforma,
    casos_uso::publicidad::{
        entrega::{self, EventoPieza},
        gestion::{self as portal, AccionCampana, DatosCampana},
        liquidacion, moderacion,
        moderacion::Decision,
    },
    puertos::{
        RelojSistema,
        publicidad::{
            Campana, Creativo, CreativoReportado, Lead, Liquidacion, PiezaPublicidad,
            PoliticaPublicidad, TarifaPublicidad,
        },
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AnuncianteId, ArchivoId, CampanaId, CodigoCategoria, CodigoDane, CreativoId, EstadoLiquidacion,
    FormatoCreativo, MedioAutorizacion, ModeloCobro, NombreArchivo, ObjetivoCampana, Segmentacion,
    TenantId, TipoCopropiedad, TipoEntidad,
    publicidad::{CodigoCupon, ContenidoCreativo, Cupon},
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    limite::LimitePublico,
    openapi::ETIQUETA_PUBLICIDAD,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::{EnEntidad, EnlaceComercio, OperadorPlataforma, PantallaLobby},
    state::AppState,
};

/// Redención de cupones por el comercio (se monta en `/api/v1/publicidad`).
pub fn rutas_comercio() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(redimir))
}

/// Plataforma (se monta en `/api/v1/plataforma`).
pub fn rutas_plataforma() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_campanas, crear_campana))
        .routes(routes!(ver_campana, editar_campana))
        .routes(routes!(cambiar_estado_campana))
        .routes(routes!(crear_creativo))
        .routes(routes!(subir_imagen))
        .routes(routes!(poner_logo))
        .routes(routes!(editar_creativo))
        .routes(routes!(metricas))
        .routes(routes!(leads))
        .routes(routes!(
            ver_enlace_comercio,
            generar_enlace_comercio,
            revocar_enlace_comercio
        ))
        .routes(routes!(exclusiones, excluir))
        .routes(routes!(incluir))
        .routes(routes!(creativos_pendientes))
        .routes(routes!(moderar_creativo))
        .routes(routes!(campanas_en_revision))
        .routes(routes!(moderar_campana))
        .routes(routes!(reportes_pendientes))
        .routes(routes!(atender_reportes))
        .routes(routes!(tarifas, fijar_tarifa))
        .routes(routes!(publicar_politica))
        .routes(routes!(liquidaciones, liquidar))
        .routes(routes!(estado_liquidacion))
        .routes(routes!(ver_pantalla, generar_pantalla))
}

/// Cualquier entidad, PH o empresa (se monta en `/api/v1/entidades/{entidad_id}`).
pub fn rutas_entidad() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(estado_autorizacion, autorizar, revocar))
        .routes(routes!(piezas))
        .routes(routes!(eventos))
        .routes(routes!(cupon))
        .routes(routes!(reportar))
}

/// Copropiedad (se monta en `/api/v1/copropiedades/{copropiedad_id}`): la pantalla del lobby.
pub fn rutas_copropiedad() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(pantalla))
}

fn invalido(campo: &str) -> ApiError {
    ApiError::App(AppError::Validacion(format!("`{campo}` inválido")))
}

// ---------------------------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------------------------

/// A qué entidades (PH y empresas) llega la campaña: solo atributos de la entidad, nunca de
/// personas. Cada criterio vacío no restringe. `tipos`, `bienes_*` y `estratos` son de la PH:
/// una empresa no los cumple.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct SegmentacionDto {
    /// Entidades elegidas una por una (vacío = todas las que cumplan lo demás).
    pub entidades: Vec<Uuid>,
    /// `propiedad_horizontal` y/o `empresa` (vacío = las dos).
    #[schema(value_type = Vec<crate::esquemas::TipoEntidad>)]
    pub tipos_entidad: Vec<String>,
    /// Radio en km desde el anunciante (1 a 100).
    pub radio_km: Option<u16>,
    /// Códigos DANE de municipios.
    #[schema(example = json!(["11001"]))]
    pub municipios: Vec<String>,
    /// `residencial`, `comercial` o `mixta`.
    #[schema(value_type = Vec<crate::esquemas::TipoCopropiedad>)]
    pub tipos: Vec<String>,
    pub bienes_min: Option<u32>,
    pub bienes_max: Option<u32>,
    /// Estratos de 1 a 6.
    pub estratos: Vec<u8>,
}

impl TryFrom<SegmentacionDto> for Segmentacion {
    type Error = ApiError;

    fn try_from(d: SegmentacionDto) -> Result<Self, ApiError> {
        Ok(Self {
            entidades: d.entidades.into_iter().map(TenantId::from_uuid).collect(),
            tipos_entidad: d
                .tipos_entidad
                .iter()
                .map(|t| TipoEntidad::try_from(t.as_str()))
                .collect::<Result<_, _>>()?,
            radio_km: d.radio_km,
            municipios: d
                .municipios
                .iter()
                .map(|m| CodigoDane::new(m))
                .collect::<Result<_, _>>()?,
            tipos: d
                .tipos
                .iter()
                .map(|t| TipoCopropiedad::try_from(t.as_str()))
                .collect::<Result<_, _>>()?,
            bienes_min: d.bienes_min,
            bienes_max: d.bienes_max,
            estratos: d.estratos,
        })
    }
}

impl From<Segmentacion> for SegmentacionDto {
    fn from(s: Segmentacion) -> Self {
        Self {
            entidades: s.entidades.iter().map(|t| t.as_uuid()).collect(),
            tipos_entidad: s
                .tipos_entidad
                .iter()
                .map(|t| t.as_str().to_owned())
                .collect(),
            radio_km: s.radio_km,
            municipios: s.municipios.iter().map(|m| m.as_str().to_owned()).collect(),
            tipos: s.tipos.iter().map(|t| t.as_str().to_owned()).collect(),
            bienes_min: s.bienes_min,
            bienes_max: s.bienes_max,
            estratos: s.estratos,
        }
    }
}

/// Datos de una campaña.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CampanaPeticion {
    #[schema(example = "Arreglos de noviembre")]
    pub nombre: String,
    /// `visibilidad`, `trafico`, `cupones` o `leads`.
    #[schema(value_type = crate::esquemas::ObjetivoCampana)]
    pub objetivo: String,
    /// Una de las categorías de su ficha.
    #[schema(example = "costura")]
    pub categoria: String,
    pub inicio: DateTime<Utc>,
    pub fin: DateTime<Utc>,
    /// `tarifa_fija`, `cpm`, `cpc`, `por_redencion`, `por_lead` o `directorio_destacado`.
    #[schema(value_type = crate::esquemas::ModeloCobro)]
    pub modelo: String,
    /// COP.
    #[schema(value_type = String, example = "200000")]
    pub presupuesto_total: Decimal,
    #[schema(value_type = Option<String>, example = "20000")]
    pub presupuesto_diario: Option<Decimal>,
    #[serde(default)]
    pub segmentacion: SegmentacionDto,
}

impl TryFrom<CampanaPeticion> for DatosCampana {
    type Error = ApiError;

    fn try_from(p: CampanaPeticion) -> Result<Self, ApiError> {
        Ok(Self {
            nombre: p.nombre,
            objetivo: ObjetivoCampana::try_from(p.objetivo.as_str())?,
            categoria: CodigoCategoria::new(&p.categoria)?,
            inicio: p.inicio,
            fin: p.fin,
            modelo: ModeloCobro::try_from(p.modelo.as_str())?,
            presupuesto_total: p.presupuesto_total,
            presupuesto_diario: p.presupuesto_diario,
            segmentacion: p.segmentacion.try_into()?,
        })
    }
}

/// Campaña.
#[derive(Debug, Serialize, ToSchema)]
pub struct CampanaRespuesta {
    pub id: Uuid,
    pub anunciante_id: Uuid,
    pub nombre: String,
    #[schema(value_type = crate::esquemas::ObjetivoCampana)]
    pub objetivo: String,
    pub categoria: String,
    pub inicio: DateTime<Utc>,
    pub fin: DateTime<Utc>,
    #[schema(value_type = crate::esquemas::ModeloCobro)]
    pub modelo: String,
    #[schema(value_type = String)]
    pub presupuesto_total: Decimal,
    #[schema(value_type = Option<String>)]
    pub presupuesto_diario: Option<Decimal>,
    /// Tarifa fijada al aprobarla.
    #[schema(value_type = Option<String>)]
    pub tarifa: Option<Decimal>,
    #[schema(value_type = String)]
    pub consumido: Decimal,
    pub segmentacion: SegmentacionDto,
    /// El anunciante presta servicio a una copropiedad alcanzada: la moderación lo revisa.
    pub conflicto_interes: bool,
    /// `borrador`, `en_revision`, `aprobada`, `rechazada`, `activa`, `pausada` o `finalizada`.
    #[schema(value_type = crate::esquemas::EstadoCampana)]
    pub estado: String,
    pub motivo_estado: Option<String>,
    pub creada_en: DateTime<Utc>,
}

impl From<Campana> for CampanaRespuesta {
    fn from(c: Campana) -> Self {
        Self {
            id: c.id.as_uuid(),
            anunciante_id: c.anunciante_id.as_uuid(),
            nombre: c.nombre,
            objetivo: c.objetivo.as_str().to_owned(),
            categoria: c.categoria.as_str().to_owned(),
            inicio: c.inicio,
            fin: c.fin,
            modelo: c.modelo.as_str().to_owned(),
            presupuesto_total: c.presupuesto.total,
            presupuesto_diario: c.presupuesto.diario,
            tarifa: c.tarifa,
            consumido: c.consumido,
            segmentacion: c.segmentacion.into(),
            conflicto_interes: c.conflicto_interes,
            estado: c.estado.as_str().to_owned(),
            motivo_estado: c.motivo_estado,
            creada_en: c.creada_en,
        }
    }
}

/// Contenido de un creativo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreativoPeticion {
    /// `banner`, `tarjeta_destacada`, `oferta_cupon`, `mensaje_whatsapp`, `boletin_correo` o
    /// `pantalla_lobby`.
    #[schema(value_type = crate::esquemas::FormatoCreativo)]
    pub formato: String,
    #[schema(example = "Arreglos en 24 horas")]
    pub titulo: String,
    pub texto: String,
    pub boton: Option<String>,
    /// Solo `https://`.
    pub enlace: Option<String>,
    /// Obligatorias en promociones y cupones (Ley 1480).
    pub condiciones: Option<String>,
    #[serde(default)]
    pub es_promocion: bool,
    /// Solo en `oferta_cupon`: de 4 a 20 letras o dígitos.
    pub cupon_codigo: Option<String>,
    pub cupon_limite: Option<u32>,
    /// Solo en `mensaje_whatsapp`: plantilla de marketing aprobada en Meta.
    pub plantilla_whatsapp: Option<String>,
    /// Imagen subida antes con `POST /plataforma/publicidad/imagenes`. Cambiarla devuelve el
    /// creativo a moderación.
    pub imagen_archivo_id: Option<Uuid>,
}

impl TryFrom<CreativoPeticion> for ContenidoCreativo {
    type Error = ApiError;

    fn try_from(p: CreativoPeticion) -> Result<Self, ApiError> {
        Ok(Self {
            formato: FormatoCreativo::try_from(p.formato.as_str())?,
            titulo: p.titulo,
            texto: p.texto,
            boton: p.boton,
            enlace: p.enlace,
            condiciones: p.condiciones,
            es_promocion: p.es_promocion,
            cupon: p
                .cupon_codigo
                .as_deref()
                .map(|c| {
                    Ok::<_, ApiError>(Cupon {
                        codigo: CodigoCupon::new(c)?,
                        limite_usos: p.cupon_limite,
                    })
                })
                .transpose()?,
            plantilla_whatsapp: p.plantilla_whatsapp,
            imagen: p.imagen_archivo_id.map(ArchivoId::from_uuid),
        })
    }
}

/// Creativo con su estado de moderación.
#[derive(Debug, Serialize, ToSchema)]
pub struct CreativoRespuesta {
    pub id: Uuid,
    pub campana_id: Uuid,
    #[schema(value_type = crate::esquemas::FormatoCreativo)]
    pub formato: String,
    pub titulo: String,
    pub texto: String,
    pub boton: Option<String>,
    pub enlace: Option<String>,
    pub condiciones: Option<String>,
    pub es_promocion: bool,
    pub cupon_codigo: Option<String>,
    pub cupon_limite: Option<u32>,
    pub cupon_usos: u32,
    pub plantilla_whatsapp: Option<String>,
    pub imagen_archivo_id: Option<Uuid>,
    /// `pendiente`, `aprobado` o `rechazado`.
    #[schema(value_type = crate::esquemas::EstadoCreativo)]
    pub estado: String,
    pub motivo_estado: Option<String>,
}

impl From<Creativo> for CreativoRespuesta {
    fn from(c: Creativo) -> Self {
        let k = c.contenido;
        Self {
            id: c.id.as_uuid(),
            campana_id: c.campana_id.as_uuid(),
            formato: k.formato.as_str().to_owned(),
            titulo: k.titulo,
            texto: k.texto,
            boton: k.boton,
            enlace: k.enlace,
            condiciones: k.condiciones,
            es_promocion: k.es_promocion,
            cupon_codigo: k.cupon.as_ref().map(|x| x.codigo.as_str().to_owned()),
            cupon_limite: k.cupon.as_ref().and_then(|x| x.limite_usos),
            cupon_usos: c.cupon_usos,
            plantilla_whatsapp: k.plantilla_whatsapp,
            imagen_archivo_id: k.imagen.map(ArchivoId::as_uuid),
            estado: c.estado.as_str().to_owned(),
            motivo_estado: c.motivo_estado,
        }
    }
}

/// Campaña con sus creativos.
#[derive(Debug, Serialize, ToSchema)]
pub struct CampanaDetalleRespuesta {
    pub campana: CampanaRespuesta,
    pub creativos: Vec<CreativoRespuesta>,
}

/// Cambio de estado de una campaña.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AccionCampanaPeticion {
    /// `enviar_a_revision`, `activar`, `pausar` o `finalizar`.
    #[schema(example = "enviar_a_revision")]
    pub accion: String,
}

/// Métrica de un día y canal.
#[derive(Debug, Serialize, ToSchema)]
pub struct MetricaDiaRespuesta {
    pub dia: NaiveDate,
    /// `app`, `pantalla`, `whatsapp` o `correo`.
    #[schema(value_type = crate::esquemas::CanalPublicidad)]
    pub canal: String,
    pub impresiones: u32,
    pub clics: u32,
    pub redenciones: u32,
    #[schema(value_type = String)]
    pub costo: Decimal,
}

/// Métricas agregadas de una campaña (sin personas ni nombres de copropiedades).
#[derive(Debug, Serialize, ToSchema)]
pub struct MetricasRespuesta {
    /// Copropiedades que alcanza hoy.
    pub alcance: u64,
    pub impresiones: u32,
    pub clics: u32,
    pub redenciones: u32,
    #[schema(value_type = String)]
    pub consumido: Decimal,
    pub dias: Vec<MetricaDiaRespuesta>,
}

/// Contacto que un residente autorizó compartirle para una solicitud.
#[derive(Debug, Serialize, ToSchema)]
pub struct LeadRespuesta {
    pub solicitud_id: Uuid,
    pub creado_en: DateTime<Utc>,
    pub categoria: Option<String>,
    pub nombre: String,
    pub telefono: Option<String>,
}

impl From<Lead> for LeadRespuesta {
    fn from(l: Lead) -> Self {
        Self {
            solicitud_id: l.solicitud_id.as_uuid(),
            creado_en: l.creado_en,
            categoria: l.categoria.map(|c| c.as_str().to_owned()),
            nombre: l.nombre,
            telefono: l.telefono.map(|t| t.as_str().to_owned()),
        }
    }
}

/// Código del cupón que presenta el cliente.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RedimirPeticion {
    #[schema(example = "ROSA10")]
    pub codigo: String,
}

/// Cupón redimido.
#[derive(Debug, Serialize, ToSchema)]
pub struct RedencionRespuesta {
    pub creativo_id: Uuid,
    pub titulo: String,
    pub usos: u32,
    /// `null` = sin límite.
    pub restantes: Option<u32>,
}

// ---------------------------------------------------------------------------------------------
// Campañas de un comercio (personal de la plataforma)
// ---------------------------------------------------------------------------------------------

const PROHIBIDO_GESTION: &str = "Sin `plataforma.publicidad.gestionar` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)";

/// Campañas del anunciante.
#[utoipa::path(
    get,
    path = "/publicidad/anunciantes/{anunciante_id}/campanas",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("anunciante_id" = Uuid, Path, description = "Comercio"), ParametrosPagina),
    responses(
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = OK, description = "Campañas", body = PaginaRespuesta<CampanaRespuesta>),
        (status = NOT_FOUND, description = "El comercio no existe", body = ErrorRespuesta),
    )
)]
pub async fn listar_campanas(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<CampanaRespuesta>>> {
    let v = portal::campanas(
        &state.plataforma,
        &c.ctx,
        AnuncianteId::from_uuid(id),
        p.validar()?,
    )
    .await?;
    Ok(Json(v.mapear(Into::into).into()))
}

/// Crea una campaña en borrador.
#[utoipa::path(
    post,
    path = "/publicidad/anunciantes/{anunciante_id}/campanas",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("anunciante_id" = Uuid, Path, description = "Comercio")),
    request_body = CampanaPeticion,
    responses(
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = CREATED, description = "Campaña creada", body = CampanaRespuesta),
        (status = NOT_FOUND, description = "El comercio no existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_campana(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<CampanaPeticion>,
) -> ApiResult<(StatusCode, Json<CampanaRespuesta>)> {
    let x = portal::crear_campana(
        &state.plataforma,
        &RelojSistema,
        &c.ctx,
        AnuncianteId::from_uuid(id),
        p.try_into()?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(x.into())))
}

/// Campaña con sus creativos.
#[utoipa::path(
    get,
    path = "/publicidad/anunciantes/{anunciante_id}/campanas/{campana_id}",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(
        ("anunciante_id" = Uuid, Path, description = "Comercio"),
        ("campana_id" = Uuid, Path, description = "Campaña"),
    ),
    responses(
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = OK, description = "Campaña", body = CampanaDetalleRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver_campana(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<CampanaDetalleRespuesta>> {
    let (x, creativos) = portal::campana(
        &state.plataforma,
        &c.ctx,
        AnuncianteId::from_uuid(ruta.0),
        CampanaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(CampanaDetalleRespuesta {
        campana: x.into(),
        creativos: creativos.into_iter().map(Into::into).collect(),
    }))
}

/// Edita la campaña (una aprobada o pausada vuelve a borrador y a moderación).
#[utoipa::path(
    put,
    path = "/publicidad/anunciantes/{anunciante_id}/campanas/{campana_id}",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(
        ("anunciante_id" = Uuid, Path, description = "Comercio"),
        ("campana_id" = Uuid, Path, description = "Campaña"),
    ),
    request_body = CampanaPeticion,
    responses(
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = OK, description = "Campaña editada", body = CampanaRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos o estado que no lo permite", body = ErrorRespuesta),
    )
)]
pub async fn editar_campana(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<CampanaPeticion>,
) -> ApiResult<Json<CampanaRespuesta>> {
    let x = portal::editar_campana(
        &state.plataforma,
        &RelojSistema,
        &c.ctx,
        AnuncianteId::from_uuid(ruta.0),
        CampanaId::from_uuid(ruta.1),
        p.try_into()?,
    )
    .await?;
    Ok(Json(x.into()))
}

/// Envía a revisión, activa, pausa o finaliza la campaña.
#[utoipa::path(
    post,
    path = "/publicidad/anunciantes/{anunciante_id}/campanas/{campana_id}/estado",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(
        ("anunciante_id" = Uuid, Path, description = "Comercio"),
        ("campana_id" = Uuid, Path, description = "Campaña"),
    ),
    request_body = AccionCampanaPeticion,
    responses(
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = OK, description = "Estado cambiado", body = CampanaRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "Transición no permitida (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Acción desconocida o falta algo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn cambiar_estado_campana(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AccionCampanaPeticion>,
) -> ApiResult<Json<CampanaRespuesta>> {
    let accion = match p.accion.as_str() {
        "enviar_a_revision" => AccionCampana::EnviarARevision,
        "activar" => AccionCampana::Activar,
        "pausar" => AccionCampana::Pausar,
        "finalizar" => AccionCampana::Finalizar,
        _ => return Err(invalido("accion")),
    };
    let x = portal::cambiar_estado_campana(
        &state.plataforma,
        &RelojSistema,
        &c.ctx,
        AnuncianteId::from_uuid(ruta.0),
        CampanaId::from_uuid(ruta.1),
        accion,
    )
    .await?;
    Ok(Json(x.into()))
}

/// Agrega un creativo (queda pendiente de moderación).
#[utoipa::path(
    post,
    path = "/publicidad/anunciantes/{anunciante_id}/campanas/{campana_id}/creativos",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(
        ("anunciante_id" = Uuid, Path, description = "Comercio"),
        ("campana_id" = Uuid, Path, description = "Campaña"),
    ),
    request_body = CreativoPeticion,
    responses(
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = CREATED, description = "Creativo creado", body = CreativoRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "Campaña no editable o cupón repetido (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Contenido inválido (condiciones obligatorias en promociones)", body = ErrorRespuesta),
    )
)]
pub async fn crear_creativo(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<CreativoPeticion>,
) -> ApiResult<(StatusCode, Json<CreativoRespuesta>)> {
    let x = portal::guardar_creativo(
        &state.plataforma,
        &RelojSistema,
        &c.ctx,
        AnuncianteId::from_uuid(ruta.0),
        CampanaId::from_uuid(ruta.1),
        None,
        p.try_into()?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(x.into())))
}

/// Edita un creativo (si cambia, vuelve a moderación).
#[utoipa::path(
    put,
    path = "/publicidad/anunciantes/{anunciante_id}/campanas/{campana_id}/creativos/{creativo_id}",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(
        ("anunciante_id" = Uuid, Path, description = "Comercio"),
        ("campana_id" = Uuid, Path, description = "Campaña"),
        ("creativo_id" = Uuid, Path, description = "Creativo"),
    ),
    request_body = CreativoPeticion,
    responses(
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = OK, description = "Creativo guardado", body = CreativoRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "Campaña no editable (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn editar_creativo(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    Json(p): Json<CreativoPeticion>,
) -> ApiResult<Json<CreativoRespuesta>> {
    let x = portal::guardar_creativo(
        &state.plataforma,
        &RelojSistema,
        &c.ctx,
        AnuncianteId::from_uuid(ruta.0),
        CampanaId::from_uuid(ruta.1),
        Some(CreativoId::from_uuid(ruta.2)),
        p.try_into()?,
    )
    .await?;
    Ok(Json(x.into()))
}

/// Métricas agregadas de la campaña.
#[utoipa::path(
    get,
    path = "/publicidad/anunciantes/{anunciante_id}/campanas/{campana_id}/metricas",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(
        ("anunciante_id" = Uuid, Path, description = "Comercio"),
        ("campana_id" = Uuid, Path, description = "Campaña"),
    ),
    responses(
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = OK, description = "Métricas", body = MetricasRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn metricas(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<MetricasRespuesta>> {
    let m = portal::metricas(
        &state.plataforma,
        &c.ctx,
        AnuncianteId::from_uuid(ruta.0),
        CampanaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(MetricasRespuesta {
        alcance: m.alcance,
        impresiones: m.totales.impresiones,
        clics: m.totales.clics,
        redenciones: m.totales.redenciones,
        consumido: m.consumido,
        dias: m
            .dias
            .into_iter()
            .map(|d| MetricaDiaRespuesta {
                dia: d.dia,
                canal: d.canal.as_str().to_owned(),
                impresiones: d.conteo.impresiones,
                clics: d.conteo.clics,
                redenciones: d.conteo.redenciones,
                costo: d.costo,
            })
            .collect(),
    }))
}

/// Contactos que los residentes autorizaron compartirle al comercio (nombre y celular, para esa
/// solicitud).
#[utoipa::path(
    get,
    path = "/publicidad/anunciantes/{anunciante_id}/leads",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("anunciante_id" = Uuid, Path, description = "Comercio"), ParametrosPagina),
    responses(
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = OK, description = "Contactos", body = PaginaRespuesta<LeadRespuesta>),
        (status = NOT_FOUND, description = "El comercio no existe", body = ErrorRespuesta),
    )
)]
pub async fn leads(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<LeadRespuesta>>> {
    let v = portal::leads(
        &state.plataforma,
        &c.ctx,
        AnuncianteId::from_uuid(id),
        p.validar()?,
    )
    .await?;
    Ok(Json(v.mapear(Into::into).into()))
}

/// Redime un cupón en el mostrador del comercio, con el token de **su enlace** en `X-Comercio`
/// (sin sesión: los comercios no tienen cuenta en la plataforma).
#[utoipa::path(
    post,
    path = "/cupones/redimir",
    tag = ETIQUETA_PUBLICIDAD,
    params(("X-Comercio" = String, Header, description = "Token del enlace de redención del comercio")),
    request_body = RedimirPeticion,
    responses(
        (status = OK, description = "Redimido", body = RedencionRespuesta),
        (status = UNAUTHORIZED, description = "Sin enlace o enlace revocado (`no_autenticado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Código desconocido", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Oferta vencida o sin usos (`validacion`)", body = ErrorRespuesta),
        (status = TOO_MANY_REQUESTS, description = "Demasiadas peticiones desde esta IP (`demasiadas_peticiones`, cabecera `Retry-After`)", body = ErrorRespuesta),
    )
)]
pub async fn redimir(
    _limite: LimitePublico,
    State(state): State<AppState>,
    comercio: EnlaceComercio,
    Json(p): Json<RedimirPeticion>,
) -> ApiResult<Json<RedencionRespuesta>> {
    let r = portal::redimir_cupon(
        &state.plataforma,
        &RelojSistema,
        &ContextoPlataforma::anonimo(None),
        &comercio.token,
        &p.codigo,
    )
    .await?;
    Ok(Json(RedencionRespuesta {
        creativo_id: r.creativo_id.as_uuid(),
        titulo: r.titulo,
        usos: r.usos,
        restantes: r.restantes,
    }))
}

/// Token de un enlace (el frontend arma la URL; ver la guía del frontend).
#[derive(Debug, Serialize, ToSchema)]
pub struct EnlaceTokenRespuesta {
    /// `null` = no hay enlace vigente (o es anterior a que se guardaran cifrados: genere otro).
    pub token: Option<String>,
}

/// El enlace de redención del comercio, sin cambiarlo.
#[utoipa::path(
    get,
    path = "/publicidad/anunciantes/{anunciante_id}/enlace-cupones",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("anunciante_id" = Uuid, Path, description = "Comercio")),
    responses(
        (status = OK, description = "Token vigente o `null`", body = EnlaceTokenRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "El comercio no existe", body = ErrorRespuesta),
    )
)]
pub async fn ver_enlace_comercio(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<EnlaceTokenRespuesta>> {
    let token = portal::enlace_comercio(
        &state.plataforma,
        state.cofre.as_ref(),
        &c.ctx,
        AnuncianteId::from_uuid(id),
    )
    .await?;
    Ok(Json(EnlaceTokenRespuesta { token }))
}

/// Genera (o reemplaza, el anterior deja de servir) el enlace con que el comercio redime cupones.
#[utoipa::path(
    post,
    path = "/publicidad/anunciantes/{anunciante_id}/enlace-cupones",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("anunciante_id" = Uuid, Path, description = "Comercio")),
    responses(
        (status = CREATED, description = "Token nuevo", body = EnlaceTokenRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "El comercio no existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "El servidor no tiene clave del Cofre (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn generar_enlace_comercio(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<EnlaceTokenRespuesta>)> {
    let token = portal::generar_enlace_comercio(
        &state.plataforma,
        state.cofre.as_ref(),
        &RelojSistema,
        &c.ctx,
        AnuncianteId::from_uuid(id),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(EnlaceTokenRespuesta { token: Some(token) }),
    ))
}

/// Revoca el enlace del comercio.
#[utoipa::path(
    delete,
    path = "/publicidad/anunciantes/{anunciante_id}/enlace-cupones",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("anunciante_id" = Uuid, Path, description = "Comercio")),
    responses(
        (status = NO_CONTENT, description = "Revocado"),
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "El comercio no existe o no tenía enlace", body = ErrorRespuesta),
    )
)]
pub async fn revocar_enlace_comercio(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    portal::revocar_enlace_comercio(&state.plataforma, &c.ctx, AnuncianteId::from_uuid(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Entidad excluida de la publicidad.
#[derive(Debug, Serialize, ToSchema)]
pub struct ExclusionRespuesta {
    pub entidad_id: Uuid,
    pub motivo: String,
    pub creada_en: DateTime<Utc>,
}

/// Exclusión de una entidad.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ExclusionPeticion {
    pub entidad_id: Uuid,
    #[schema(example = "El contrato con la PH no permite publicidad")]
    pub motivo: String,
}

/// Entidades excluidas de la publicidad (no reciben piezas ni destacadas).
#[utoipa::path(
    get,
    path = "/publicidad/exclusiones",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Exclusiones", body = Vec<ExclusionRespuesta>),
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
    )
)]
pub async fn exclusiones(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
) -> ApiResult<Json<Vec<ExclusionRespuesta>>> {
    let v = portal::exclusiones(&state.plataforma, &c.ctx).await?;
    Ok(Json(
        v.into_iter()
            .map(|x| ExclusionRespuesta {
                entidad_id: x.tenant_id.as_uuid(),
                motivo: x.motivo,
                creada_en: x.creada_en,
            })
            .collect(),
    ))
}

/// Excluye una entidad de la publicidad. Solo la plataforma lo decide: la entidad no
/// controla la publicidad. Su directorio sigue funcionando.
#[utoipa::path(
    post,
    path = "/publicidad/exclusiones",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    request_body = ExclusionPeticion,
    responses(
        (status = CREATED, description = "Excluida", body = ExclusionRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La entidad no existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin motivo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn excluir(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Json(p): Json<ExclusionPeticion>,
) -> ApiResult<(StatusCode, Json<ExclusionRespuesta>)> {
    let x = portal::excluir_entidad(
        &state.plataforma,
        &RelojSistema,
        &c.ctx,
        TenantId::from_uuid(p.entidad_id),
        &p.motivo,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(ExclusionRespuesta {
            entidad_id: x.tenant_id.as_uuid(),
            motivo: x.motivo,
            creada_en: x.creada_en,
        }),
    ))
}

/// Vuelve a incluir una entidad en la publicidad.
#[utoipa::path(
    delete,
    path = "/publicidad/exclusiones/{entidad_id}",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = NO_CONTENT, description = "Incluida"),
        (status = FORBIDDEN, description = PROHIBIDO_GESTION, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No estaba excluida", body = ErrorRespuesta),
    )
)]
pub async fn incluir(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    portal::incluir_entidad(&state.plataforma, &c.ctx, TenantId::from_uuid(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Plataforma
// ---------------------------------------------------------------------------------------------

/// Decisión de la moderación.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DecisionPeticion {
    /// `aprobar` o `rechazar`.
    #[schema(example = "aprobar")]
    pub decision: String,
    /// Obligatorio al rechazar.
    pub motivo: Option<String>,
}

impl TryFrom<DecisionPeticion> for Decision {
    type Error = ApiError;

    fn try_from(p: DecisionPeticion) -> Result<Self, ApiError> {
        match p.decision.as_str() {
            "aprobar" => Ok(Self::Aprobar),
            "rechazar" => Ok(Self::Rechazar {
                motivo: p.motivo.unwrap_or_default(),
            }),
            _ => Err(invalido("decision")),
        }
    }
}

/// Creativos pendientes de moderación.
#[utoipa::path(
    get,
    path = "/publicidad/moderacion/creativos",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(ParametrosPagina),
    responses(
        (status = OK, description = "Pendientes", body = PaginaRespuesta<CreativoRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.moderar`", body = ErrorRespuesta),
    )
)]
pub async fn creativos_pendientes(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<CreativoRespuesta>>> {
    let v = moderacion::creativos_pendientes(&state.plataforma, &op.0.ctx, p.validar()?).await?;
    Ok(Json(v.mapear(Into::into).into()))
}

/// Aprueba o rechaza un creativo.
#[utoipa::path(
    post,
    path = "/publicidad/moderacion/creativos/{creativo_id}",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("creativo_id" = Uuid, Path, description = "Creativo")),
    request_body = DecisionPeticion,
    responses(
        (status = OK, description = "Moderado", body = CreativoRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.moderar`", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está pendiente", body = ErrorRespuesta),
    )
)]
pub async fn moderar_creativo(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<DecisionPeticion>,
) -> ApiResult<Json<CreativoRespuesta>> {
    let x = moderacion::moderar_creativo(
        &state.plataforma,
        &RelojSistema,
        &op.0.ctx,
        CreativoId::from_uuid(id),
        p.try_into()?,
    )
    .await?;
    Ok(Json(x.into()))
}

/// Campañas en revisión (con el conflicto de interés marcado).
#[utoipa::path(
    get,
    path = "/publicidad/moderacion/campanas",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(ParametrosPagina),
    responses(
        (status = OK, description = "En revisión", body = PaginaRespuesta<CampanaRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.moderar`", body = ErrorRespuesta),
    )
)]
pub async fn campanas_en_revision(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<CampanaRespuesta>>> {
    let v = moderacion::campanas_en_revision(&state.plataforma, &op.0.ctx, p.validar()?).await?;
    Ok(Json(v.mapear(Into::into).into()))
}

/// Aprueba (fija la tarifa vigente) o rechaza una campaña.
#[utoipa::path(
    post,
    path = "/publicidad/moderacion/campanas/{campana_id}",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("campana_id" = Uuid, Path, description = "Campaña")),
    request_body = DecisionPeticion,
    responses(
        (status = OK, description = "Moderada", body = CampanaRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.moderar`", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Falta verificar al anunciante, aprobar creativos o una tarifa", body = ErrorRespuesta),
    )
)]
pub async fn moderar_campana(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<DecisionPeticion>,
) -> ApiResult<Json<CampanaRespuesta>> {
    let x = moderacion::moderar_campana(
        &state.plataforma,
        &RelojSistema,
        &op.0.ctx,
        CampanaId::from_uuid(id),
        p.try_into()?,
    )
    .await?;
    Ok(Json(x.into()))
}

/// Creativo reportado por residentes.
#[derive(Debug, Serialize, ToSchema)]
pub struct CreativoReportadoRespuesta {
    pub creativo_id: Uuid,
    pub campana_id: Uuid,
    pub anunciante_id: Uuid,
    pub nombre_comercial: String,
    pub titulo: String,
    pub reportes: u32,
    pub motivos: Vec<String>,
    pub primero_en: DateTime<Utc>,
}

impl From<CreativoReportado> for CreativoReportadoRespuesta {
    fn from(r: CreativoReportado) -> Self {
        Self {
            creativo_id: r.creativo_id.as_uuid(),
            campana_id: r.campana_id.as_uuid(),
            anunciante_id: r.anunciante_id.as_uuid(),
            nombre_comercial: r.nombre_comercial,
            titulo: r.titulo,
            reportes: r.reportes,
            motivos: r.motivos,
            primero_en: r.primero_en,
        }
    }
}

/// Creativos con reportes sin atender, de todas las copropiedades.
#[utoipa::path(
    get,
    path = "/publicidad/moderacion/reportes",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(ParametrosPagina),
    responses(
        (status = OK, description = "Reportados", body = PaginaRespuesta<CreativoReportadoRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.moderar`", body = ErrorRespuesta),
    )
)]
pub async fn reportes_pendientes(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<CreativoReportadoRespuesta>>> {
    let v = moderacion::reportes_pendientes(&state.plataforma, &op.0.ctx, p.validar()?).await?;
    Ok(Json(v.mapear(Into::into).into()))
}

/// Cómo se atienden los reportes de un creativo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AtenderReportesPeticion {
    /// `true` lo retira (deja de entregarse); `false` lo mantiene.
    pub retirar: bool,
    /// Obligatorio al retirarlo.
    pub motivo: Option<String>,
}

/// Reportes atendidos.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReportesAtendidosRespuesta {
    pub atendidos: u64,
}

/// Atiende los reportes de un creativo: lo retira o lo mantiene.
#[utoipa::path(
    post,
    path = "/publicidad/moderacion/reportes/{creativo_id}",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("creativo_id" = Uuid, Path, description = "Creativo")),
    request_body = AtenderReportesPeticion,
    responses(
        (status = OK, description = "Atendidos", body = ReportesAtendidosRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.moderar`", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn atender_reportes(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<AtenderReportesPeticion>,
) -> ApiResult<Json<ReportesAtendidosRespuesta>> {
    let motivo = p.retirar.then(|| p.motivo.unwrap_or_default());
    let n = moderacion::atender_reportes(
        &state.plataforma,
        &RelojSistema,
        &op.0.ctx,
        CreativoId::from_uuid(id),
        motivo.as_deref(),
    )
    .await?;
    Ok(Json(ReportesAtendidosRespuesta { atendidos: n }))
}

/// Tarifa de un modelo de cobro.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TarifaPublicidadDto {
    #[schema(value_type = crate::esquemas::ModeloCobro)]
    pub modelo: String,
    /// COP.
    #[schema(value_type = String, example = "800")]
    pub valor: Decimal,
    pub vigente_desde: NaiveDate,
}

impl From<TarifaPublicidad> for TarifaPublicidadDto {
    fn from(t: TarifaPublicidad) -> Self {
        Self {
            modelo: t.modelo.as_str().to_owned(),
            valor: t.valor,
            vigente_desde: t.vigente_desde,
        }
    }
}

/// Todas las tarifas.
#[utoipa::path(
    get,
    path = "/publicidad/tarifas",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Tarifas", body = Vec<TarifaPublicidadDto>),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.liquidar`", body = ErrorRespuesta),
    )
)]
pub async fn tarifas(
    State(state): State<AppState>,
    op: OperadorPlataforma,
) -> ApiResult<Json<Vec<TarifaPublicidadDto>>> {
    let v = liquidacion::tarifas(&state.plataforma, &op.0.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Fija la tarifa de un modelo desde una fecha (hoy o futura).
#[utoipa::path(
    post,
    path = "/publicidad/tarifas",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    request_body = TarifaPublicidadDto,
    responses(
        (status = CREATED, description = "Tarifa fijada", body = TarifaPublicidadDto),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.liquidar`", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya hay una tarifa del modelo desde ese día", body = ErrorRespuesta),
    )
)]
pub async fn fijar_tarifa(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Json(p): Json<TarifaPublicidadDto>,
) -> ApiResult<(StatusCode, Json<TarifaPublicidadDto>)> {
    let t = liquidacion::fijar_tarifa(
        &state.plataforma,
        &RelojSistema,
        &op.0.ctx,
        TarifaPublicidad {
            modelo: ModeloCobro::try_from(p.modelo.as_str())?,
            valor: p.valor,
            vigente_desde: p.vigente_desde,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(t.into())))
}

/// Política de publicidad de la plataforma.
#[derive(Debug, Serialize, ToSchema)]
pub struct PoliticaPublicidadRespuesta {
    pub version: u32,
    pub texto: String,
    pub vigente_desde: DateTime<Utc>,
}

impl From<PoliticaPublicidad> for PoliticaPublicidadRespuesta {
    fn from(p: PoliticaPublicidad) -> Self {
        Self {
            version: p.version,
            texto: p.texto,
            vigente_desde: p.vigente_desde,
        }
    }
}

/// Texto de una versión nueva de la política.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PoliticaPublicidadPeticion {
    pub texto: String,
}

/// Publica una versión nueva de la política de publicidad (rige desde ya).
#[utoipa::path(
    post,
    path = "/publicidad/politica",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    request_body = PoliticaPublicidadPeticion,
    responses(
        (status = CREATED, description = "Publicada", body = PoliticaPublicidadRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.liquidar`", body = ErrorRespuesta),
    )
)]
pub async fn publicar_politica(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Json(p): Json<PoliticaPublicidadPeticion>,
) -> ApiResult<(StatusCode, Json<PoliticaPublicidadRespuesta>)> {
    let x = liquidacion::publicar_politica(&state.plataforma, &RelojSistema, &op.0.ctx, &p.texto)
        .await?;
    Ok((StatusCode::CREATED, Json(x.into())))
}

/// Mes a liquidar o consultar.
#[derive(Debug, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct MesQuery {
    /// Cualquier día del mes.
    pub mes: NaiveDate,
}

/// Liquidación de un anunciante en un mes (sin reparto a las copropiedades).
#[derive(Debug, Serialize, ToSchema)]
pub struct LiquidacionRespuesta {
    pub id: Uuid,
    pub anunciante_id: Uuid,
    pub periodo: NaiveDate,
    #[schema(value_type = String)]
    pub variable: Decimal,
    #[schema(value_type = String)]
    pub fijos: Decimal,
    pub leads: u32,
    #[schema(value_type = String)]
    pub valor_leads: Decimal,
    #[schema(value_type = String)]
    pub valor: Decimal,
    /// Costo de WhatsApp para la plataforma (informativo).
    #[schema(value_type = String)]
    pub costo_whatsapp: Decimal,
    /// `calculada`, `facturada` o `pagada`.
    #[schema(value_type = crate::esquemas::EstadoLiquidacion)]
    pub estado: String,
}

impl From<Liquidacion> for LiquidacionRespuesta {
    fn from(l: Liquidacion) -> Self {
        Self {
            id: l.id.as_uuid(),
            anunciante_id: l.anunciante_id.as_uuid(),
            periodo: l.periodo,
            variable: l.variable,
            fijos: l.fijos,
            leads: l.leads,
            valor_leads: l.valor_leads,
            valor: l.valor,
            costo_whatsapp: l.costo_whatsapp,
            estado: l.estado.as_str().to_owned(),
        }
    }
}

/// Liquidaciones de un mes.
#[utoipa::path(
    get,
    path = "/publicidad/liquidaciones",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(MesQuery),
    responses(
        (status = OK, description = "Liquidaciones", body = Vec<LiquidacionRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.liquidar`", body = ErrorRespuesta),
    )
)]
pub async fn liquidaciones(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Query(q): Query<MesQuery>,
) -> ApiResult<Json<Vec<LiquidacionRespuesta>>> {
    let v = liquidacion::liquidaciones(&state.plataforma, &op.0.ctx, q.mes).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Calcula o recalcula las liquidaciones de un mes ya cerrado.
#[utoipa::path(
    post,
    path = "/publicidad/liquidaciones",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    request_body = MesQuery,
    responses(
        (status = OK, description = "Liquidaciones", body = Vec<LiquidacionRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.liquidar`", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "El mes no ha terminado", body = ErrorRespuesta),
    )
)]
pub async fn liquidar(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Json(q): Json<MesQuery>,
) -> ApiResult<Json<Vec<LiquidacionRespuesta>>> {
    let v = liquidacion::liquidar_mes(&state.plataforma, &RelojSistema, &op.0.ctx, q.mes).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Estado nuevo de una liquidación.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EstadoLiquidacionPeticion {
    pub mes: NaiveDate,
    /// `facturada` o `pagada`.
    #[schema(value_type = crate::esquemas::EstadoLiquidacion)]
    pub estado: String,
}

/// Marca la liquidación del anunciante facturada o pagada.
#[utoipa::path(
    post,
    path = "/publicidad/liquidaciones/{anunciante_id}/estado",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("anunciante_id" = Uuid, Path, description = "Anunciante")),
    request_body = EstadoLiquidacionPeticion,
    responses(
        (status = OK, description = "Estado cambiado", body = LiquidacionRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.liquidar`", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn estado_liquidacion(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<EstadoLiquidacionPeticion>,
) -> ApiResult<Json<LiquidacionRespuesta>> {
    let l = liquidacion::cambiar_estado_liquidacion(
        &state.plataforma,
        &RelojSistema,
        &op.0.ctx,
        AnuncianteId::from_uuid(id),
        p.mes,
        EstadoLiquidacion::try_from(p.estado.as_str())?,
    )
    .await?;
    Ok(Json(l.into()))
}

/// Token de una pantalla.
#[derive(Debug, Serialize, ToSchema)]
pub struct PantallaRespuesta {
    pub token: String,
}

/// El token vigente de la pantalla del lobby, sin cambiarlo.
#[utoipa::path(
    get,
    path = "/publicidad/pantallas/{copropiedad_id}",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Token vigente o `null` (genere uno)", body = EnlaceTokenRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.modulos.gestionar`", body = ErrorRespuesta),
    )
)]
pub async fn ver_pantalla(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<EnlaceTokenRespuesta>> {
    let token = moderacion::token_pantalla(
        &state.unidad,
        state.cofre.as_ref(),
        &op.0.ctx,
        TenantId::from_uuid(id),
    )
    .await?;
    Ok(Json(EnlaceTokenRespuesta { token }))
}

/// Genera el token de la pantalla del lobby de una copropiedad (reemplaza el anterior).
#[utoipa::path(
    post,
    path = "/publicidad/pantallas/{copropiedad_id}",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = CREATED, description = "Token", body = PantallaRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.modulos.gestionar`", body = ErrorRespuesta),
    )
)]
pub async fn generar_pantalla(
    State(state): State<AppState>,
    op: OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<PantallaRespuesta>)> {
    let token = moderacion::generar_pantalla(
        &state.unidad,
        state.cofre.as_ref(),
        &RelojSistema,
        &op.0.ctx,
        TenantId::from_uuid(id),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(PantallaRespuesta { token })))
}

// ---------------------------------------------------------------------------------------------
// Entidad (PH o empresa)
// ---------------------------------------------------------------------------------------------

/// Autorización de publicidad de la cuenta.
#[derive(Debug, Serialize, ToSchema)]
pub struct AutorizacionPublicidadRespuesta {
    pub autorizada: bool,
    /// Política de la plataforma que se acepta al autorizar.
    pub politica: Option<PoliticaPublicidadRespuesta>,
}

/// Aceptación de la política de publicidad.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AutorizarPublicidadPeticion {
    /// Versión de la política que leyó y acepta (debe ser la vigente).
    pub version: u32,
    /// `app` (por defecto) o `web`.
    #[schema(value_type = Option<crate::esquemas::MedioAutorizacion>)]
    pub medio: Option<String>,
}

/// ¿La cuenta autorizó recibir publicidad de la plataforma? Con la política vigente.
#[utoipa::path(
    get,
    path = "/publicidad/autorizacion",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad (PH o empresa)")),
    responses(
        (status = OK, description = "Estado", body = AutorizacionPublicidadRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "La cuenta no está en el censo", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "No exige permisos: basta un rol en la copropiedad (sin persona en la copropiedad, 422)", body = ErrorRespuesta),
    )
)]
pub async fn estado_autorizacion(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<AutorizacionPublicidadRespuesta>> {
    let e = entrega::autorizacion_publicidad(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(AutorizacionPublicidadRespuesta {
        autorizada: e.autorizada,
        politica: e.politica.map(Into::into),
    }))
}

/// El titular autoriza recibir publicidad de la plataforma (solo él; la PH no puede).
#[utoipa::path(
    post,
    path = "/publicidad/autorizacion",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad (PH o empresa)")),
    request_body = AutorizarPublicidadPeticion,
    responses(
        (status = NO_CONTENT, description = "Autorizada"),
        (status = UNPROCESSABLE_ENTITY, description = "Versión de la política no vigente o la cuenta no está en el censo", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "No exige permisos: solo el titular autoriza (sin persona en la copropiedad, 422)", body = ErrorRespuesta),
    )
)]
pub async fn autorizar(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<AutorizarPublicidadPeticion>,
) -> ApiResult<StatusCode> {
    let medio = match p.medio.as_deref() {
        None => MedioAutorizacion::App,
        Some(m) => MedioAutorizacion::try_from(m)?,
    };
    entrega::autorizar_publicidad(&state.unidad, &RelojSistema, &en.ctx, p.version, medio).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Retira la autorización de publicidad.
#[utoipa::path(
    delete,
    path = "/publicidad/autorizacion",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad (PH o empresa)")),
    responses(
        (status = NO_CONTENT, description = "Revocada (o no había)"),
        (status = UNPROCESSABLE_ENTITY, description = "La cuenta no está en el censo", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "No exige permisos: solo el titular retira su autorización", body = ErrorRespuesta),
    )
)]
pub async fn revocar(State(state): State<AppState>, en: EnEntidad) -> ApiResult<StatusCode> {
    entrega::revocar_publicidad(&state.unidad, &RelojSistema, &en.ctx, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Pieza de publicidad para mostrar.
#[derive(Debug, Serialize, ToSchema)]
pub struct PiezaRespuesta {
    pub creativo_id: Uuid,
    pub anunciante_id: Uuid,
    pub nombre_comercial: String,
    #[schema(value_type = crate::esquemas::FormatoCreativo)]
    pub formato: String,
    pub titulo: String,
    pub texto: String,
    pub boton: Option<String>,
    pub enlace: Option<String>,
    pub condiciones: Option<String>,
    pub es_promocion: bool,
    /// Si es una oferta con cupón (el código se pide con `…/cupon`).
    pub tiene_cupon: bool,
    pub vence_en: DateTime<Utc>,
    /// Enlace firmado de la imagen (vale una hora; no lleva sesión). `null` si no tiene.
    pub imagen_url: Option<String>,
}

impl From<PiezaPublicidad> for PiezaRespuesta {
    fn from(p: PiezaPublicidad) -> Self {
        let k = p.contenido;
        Self {
            creativo_id: p.candidata.creativo.as_uuid(),
            anunciante_id: p.anunciante_id.as_uuid(),
            nombre_comercial: p.nombre_comercial,
            formato: k.formato.as_str().to_owned(),
            titulo: k.titulo,
            texto: k.texto,
            boton: k.boton,
            enlace: k.enlace,
            condiciones: k.condiciones,
            es_promocion: k.es_promocion,
            tiene_cupon: k.cupon.is_some(),
            vence_en: p.candidata.fin,
            imagen_url: None,
        }
    }
}

/// Piezas con el enlace firmado de su imagen.
fn piezas_respuesta(
    state: &AppState,
    piezas: Vec<PiezaPublicidad>,
) -> ApiResult<Vec<PiezaRespuesta>> {
    piezas
        .into_iter()
        .map(|p| {
            let imagen = match (p.contenido.imagen, p.imagen_tipo) {
                (Some(id), Some(tipo)) => Some(portal::enlace_imagen(
                    state.archivos.as_ref(),
                    &RelojSistema,
                    id,
                    tipo,
                )?),
                _ => None,
            };
            let mut r = PiezaRespuesta::from(p);
            r.imagen_url = imagen;
            Ok(r)
        })
        .collect()
}

/// Piezas para el residente (vacío si no autorizó la publicidad o la PH no tiene el módulo).
#[utoipa::path(
    get,
    path = "/publicidad/piezas",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad (PH o empresa)")),
    responses(
        (status = OK, description = "Piezas", body = Vec<PiezaRespuesta>),
        (status = FORBIDDEN, description = "Sin `directorio.ver`", body = ErrorRespuesta),
    )
)]
pub async fn piezas(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<PiezaRespuesta>>> {
    let v = entrega::piezas_para_residente(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(piezas_respuesta(&state, v)?))
}

/// Eventos de una pieza.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EventoDto {
    pub creativo_id: Uuid,
    #[serde(default)]
    pub impresiones: u32,
    #[serde(default)]
    pub clics: u32,
}

/// Lote de eventos (agregados, anónimos).
#[derive(Debug, Deserialize, ToSchema)]
pub struct EventosPeticion {
    pub eventos: Vec<EventoDto>,
}

/// Eventos aceptados (dentro del presupuesto).
#[derive(Debug, Serialize, ToSchema)]
pub struct EventosRespuesta {
    pub impresiones: u32,
    pub clics: u32,
}

/// Registra impresiones y clics de las piezas (solo de quien autorizó; hasta 100 por
/// contador).
#[utoipa::path(
    post,
    path = "/publicidad/eventos",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad (PH o empresa)")),
    request_body = EventosPeticion,
    responses(
        (status = OK, description = "Aceptados", body = EventosRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Demasiados eventos (`validacion`)", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `directorio.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn eventos(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<EventosPeticion>,
) -> ApiResult<Json<EventosRespuesta>> {
    let eventos: Vec<EventoPieza> = p
        .eventos
        .into_iter()
        .map(|e| EventoPieza {
            creativo: CreativoId::from_uuid(e.creativo_id),
            impresiones: e.impresiones,
            clics: e.clics,
        })
        .collect();
    let a = entrega::registrar_eventos(&state.unidad, &RelojSistema, &en.ctx, &eventos).await?;
    Ok(Json(EventosRespuesta {
        impresiones: a.impresiones,
        clics: a.clics,
    }))
}

/// Cupón de una oferta.
#[derive(Debug, Serialize, ToSchema)]
pub struct CuponRespuesta {
    #[schema(example = "ROSA10")]
    pub codigo: String,
    pub condiciones: Option<String>,
    pub vence_en: DateTime<Utc>,
    pub nombre_comercial: String,
}

/// El residente obtiene el código de una oferta para presentarlo en el comercio.
#[utoipa::path(
    post,
    path = "/publicidad/creativos/{creativo_id}/cupon",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(
        ("entidad_id" = Uuid, Path, description = "Entidad (PH o empresa)"),
        ("creativo_id" = Uuid, Path, description = "Oferta"),
    ),
    responses(
        (status = OK, description = "Cupón", body = CuponRespuesta),
        (status = NOT_FOUND, description = "La oferta no se muestra en la PH o no autorizó la publicidad", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `directorio.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn cupon(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<CuponRespuesta>> {
    let c = entrega::obtener_cupon(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CreativoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(CuponRespuesta {
        codigo: c.codigo,
        condiciones: c.condiciones,
        vence_en: c.vence_en,
        nombre_comercial: c.nombre_comercial,
    }))
}

/// Motivo del reporte de un anuncio.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReporteAnuncioPeticion {
    /// Hasta 500 caracteres.
    #[schema(example = "Publicidad engañosa")]
    pub motivo: String,
}

/// ¿Se registró el reporte? (`false` si ya lo había reportado).
#[derive(Debug, Serialize, ToSchema)]
pub struct ReporteAnuncioRespuesta {
    pub nuevo: bool,
}

/// Reporta un anuncio para la moderación de la plataforma.
#[utoipa::path(
    post,
    path = "/publicidad/creativos/{creativo_id}/reporte",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(
        ("entidad_id" = Uuid, Path, description = "Entidad (PH o empresa)"),
        ("creativo_id" = Uuid, Path, description = "Anuncio"),
    ),
    request_body = ReporteAnuncioPeticion,
    responses(
        (status = OK, description = "Reportado", body = ReporteAnuncioRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin motivo", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `directorio.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn reportar(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ReporteAnuncioPeticion>,
) -> ApiResult<Json<ReporteAnuncioRespuesta>> {
    let nuevo = entrega::reportar_anuncio(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CreativoId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(Json(ReporteAnuncioRespuesta { nuevo }))
}

/// Cartelera de la pantalla del lobby (sin sesión: el token de la pantalla en `X-Pantalla`).
#[utoipa::path(
    get,
    path = "/publicidad/pantalla",
    tag = ETIQUETA_PUBLICIDAD,
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("X-Pantalla" = String, Header, description = "Token de la pantalla"),
    ),
    responses(
        (status = OK, description = "Piezas", body = Vec<PiezaRespuesta>),
        (status = UNAUTHORIZED, description = "Token inválido (`no_autenticado`)", body = ErrorRespuesta),
        (status = TOO_MANY_REQUESTS, description = "Demasiadas peticiones", body = ErrorRespuesta),
    )
)]
pub async fn pantalla(
    _limite: LimitePublico,
    State(state): State<AppState>,
    p: PantallaLobby,
) -> ApiResult<Json<Vec<PiezaRespuesta>>> {
    let v = entrega::piezas_pantalla(&state.unidad, &RelojSistema, p.tenant, &p.token).await?;
    Ok(Json(piezas_respuesta(&state, v)?))
}

// ---------------------------------------------------------------------------------------------
// Imágenes de la publicidad
// ---------------------------------------------------------------------------------------------

/// Imagen a subir (en Base64 dentro del JSON, como los demás archivos de la API).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ImagenPeticion {
    #[schema(example = "promo-diciembre.png")]
    pub nombre_archivo: String,
    /// JPEG, PNG o WebP de hasta 2 MB.
    pub archivo_base64: String,
}

/// Imagen subida.
#[derive(Debug, Serialize, ToSchema)]
pub struct ImagenRespuesta {
    /// Va en `imagen_archivo_id` del creativo o en el logo del comercio.
    pub id: Uuid,
    #[schema(example = "image/png")]
    pub tipo: String,
    pub tamano_bytes: u64,
    /// Enlace firmado para verla (vale una hora).
    pub url: String,
}

/// Sube una imagen de la publicidad (creativo o logo de un comercio).
#[utoipa::path(
    post,
    path = "/publicidad/imagenes",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    request_body = ImagenPeticion,
    responses(
        (status = CREATED, description = "Imagen subida", body = ImagenRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.publicidad.gestionar` ni `plataforma.directorio.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es JPEG, PNG o WebP, o pasa de 2 MB", body = ErrorRespuesta),
    )
)]
pub async fn subir_imagen(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Json(p): Json<ImagenPeticion>,
) -> ApiResult<(StatusCode, Json<ImagenRespuesta>)> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let contenido = STANDARD.decode(p.archivo_base64.trim()).map_err(|_| {
        ApiError::App(AppError::Validacion(
            "`archivo_base64` no es Base64 válido".into(),
        ))
    })?;
    let nombre = NombreArchivo::new(&p.nombre_archivo).map_err(|e| ApiError::App(e.into()))?;
    let a = portal::subir_imagen(
        &state.plataforma,
        state.archivos.as_ref(),
        &RelojSistema,
        &c.ctx,
        nombre,
        contenido,
    )
    .await?;
    let url = portal::enlace_imagen(state.archivos.as_ref(), &RelojSistema, a.id, a.tipo)?;
    Ok((
        StatusCode::CREATED,
        Json(ImagenRespuesta {
            id: a.id.as_uuid(),
            tipo: a.tipo.as_str().to_owned(),
            tamano_bytes: a.tamano_bytes,
            url,
        }),
    ))
}

/// Logo de un comercio (`null` lo quita).
#[derive(Debug, Deserialize, ToSchema)]
pub struct LogoPeticion {
    /// Imagen subida con `POST /plataforma/publicidad/imagenes`.
    pub archivo_id: Option<Uuid>,
}

/// Logo vigente del comercio.
#[derive(Debug, Serialize, ToSchema)]
pub struct LogoRespuesta {
    pub anunciante_id: Uuid,
    pub logo_archivo_id: Option<Uuid>,
}

/// Pone o quita el logo de un comercio.
#[utoipa::path(
    put,
    path = "/publicidad/anunciantes/{anunciante_id}/logo",
    tag = ETIQUETA_PUBLICIDAD,
    security(("bearer" = [])),
    params(("anunciante_id" = Uuid, Path, description = "Comercio")),
    request_body = LogoPeticion,
    responses(
        (status = OK, description = "Logo guardado", body = LogoRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.directorio.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe el comercio o la imagen", body = ErrorRespuesta),
    )
)]
pub async fn poner_logo(
    State(state): State<AppState>,
    OperadorPlataforma(c): OperadorPlataforma,
    Path(anunciante): Path<Uuid>,
    Json(p): Json<LogoPeticion>,
) -> ApiResult<Json<LogoRespuesta>> {
    let a = portal::poner_logo(
        &state.plataforma,
        &RelojSistema,
        &c.ctx,
        AnuncianteId::from_uuid(anunciante),
        p.archivo_id.map(ArchivoId::from_uuid),
    )
    .await?;
    Ok(Json(LogoRespuesta {
        anunciante_id: a.id.as_uuid(),
        logo_archivo_id: a.logo.map(ArchivoId::as_uuid),
    }))
}
