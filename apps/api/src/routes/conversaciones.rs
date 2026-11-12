//! Chat de WhatsApp, lado web (plan 07b, módulo `gestion_administrativa`):
//! `/preguntas-frecuentes` que responde el chat y la bandeja `/conversaciones` escaladas a la
//! administración, dentro de `/api/v1/copropiedades/{copropiedad_id}`.

use application::{
    casos_uso::conversaciones::{self as uc, DatosPregunta},
    puertos::{
        Reloj as _, RelojSistema,
        chat::{Conversacion, EstadoConversacion, MensajeConversacion, PreguntaFrecuente},
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{ArchivoId, ConversacionId, CuentaId, PersonaId, PreguntaFrecuenteId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_CONVERSACIONES,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas del chat (lado web).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_preguntas, crear_pregunta))
        .routes(routes!(editar_pregunta))
        .routes(routes!(listar_conversaciones))
        .routes(routes!(ver_conversacion))
        .routes(routes!(responder))
        .routes(routes!(cerrar))
}

/// Pregunta frecuente.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreguntaFrecuenteRespuesta {
    pub id: Uuid,
    #[schema(example = "Horario de la piscina")]
    pub pregunta: String,
    pub respuesta: String,
    pub orden: i16,
    pub activa: bool,
    pub archivo_id: Option<Uuid>,
}

impl From<PreguntaFrecuente> for PreguntaFrecuenteRespuesta {
    fn from(p: PreguntaFrecuente) -> Self {
        Self {
            id: p.id.as_uuid(),
            pregunta: p.pregunta,
            respuesta: p.respuesta,
            orden: p.orden,
            activa: p.activa,
            archivo_id: p.archivo_id.map(ArchivoId::as_uuid),
        }
    }
}

/// Datos de una pregunta frecuente.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PreguntaFrecuentePeticion {
    /// Hasta 72 caracteres (se muestra en la lista de WhatsApp).
    pub pregunta: String,
    /// Hasta 4000 caracteres; admite saltos de línea.
    pub respuesta: String,
    #[serde(default)]
    pub orden: i16,
    #[serde(default = "verdadero")]
    pub activa: bool,
    /// Documento enlazable (reglamento, manual de convivencia).
    pub archivo_id: Option<Uuid>,
}

const fn verdadero() -> bool {
    true
}

impl From<PreguntaFrecuentePeticion> for DatosPregunta {
    fn from(p: PreguntaFrecuentePeticion) -> Self {
        Self {
            pregunta: p.pregunta,
            respuesta: p.respuesta,
            orden: p.orden,
            activa: p.activa,
            archivo_id: p.archivo_id.map(ArchivoId::from_uuid),
        }
    }
}

/// Preguntas frecuentes (las inactivas solo para quien las edita).
#[utoipa::path(
    get,
    path = "/preguntas-frecuentes",
    tag = ETIQUETA_CONVERSACIONES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Preguntas", body = Vec<PreguntaFrecuenteRespuesta>),
        (status = FORBIDDEN, description = "Sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_preguntas(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<PreguntaFrecuenteRespuesta>>> {
    let v = uc::listar_preguntas(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea una pregunta frecuente.
#[utoipa::path(
    post,
    path = "/preguntas-frecuentes",
    tag = ETIQUETA_CONVERSACIONES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = PreguntaFrecuentePeticion,
    responses(
        (status = CREATED, description = "Pregunta creada", body = PreguntaFrecuenteRespuesta),
        (status = FORBIDDEN, description = "Sin `preguntas_frecuentes.editar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Textos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_pregunta(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<PreguntaFrecuentePeticion>,
) -> ApiResult<(StatusCode, Json<PreguntaFrecuenteRespuesta>)> {
    let r = uc::guardar_pregunta(&state.unidad, &RelojSistema, &en.ctx, None, p.into()).await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Reemplaza una pregunta frecuente.
#[utoipa::path(
    put,
    path = "/preguntas-frecuentes/{pregunta_id}",
    tag = ETIQUETA_CONVERSACIONES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("pregunta_id" = Uuid, Path, description = "Pregunta"),
    ),
    request_body = PreguntaFrecuentePeticion,
    responses(
        (status = OK, description = "Pregunta", body = PreguntaFrecuenteRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn editar_pregunta(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PreguntaFrecuentePeticion>,
) -> ApiResult<Json<PreguntaFrecuenteRespuesta>> {
    let r = uc::guardar_pregunta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        Some(PreguntaFrecuenteId::from_uuid(ruta.1)),
        p.into(),
    )
    .await?;
    Ok(Json(r.into()))
}

/// Conversación de la bandeja.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConversacionRespuesta {
    pub id: Uuid,
    /// Teléfono del residente (dato personal).
    pub telefono: String,
    pub persona_id: Option<Uuid>,
    #[schema(value_type = crate::esquemas::EstadoConversacion)]
    pub estado: EstadoConversacion,
    pub atendida_por: Option<Uuid>,
    pub abierta_en: DateTime<Utc>,
    pub ultimo_entrante_en: DateTime<Utc>,
    /// Se puede responder con texto libre (ventana de 24 h de WhatsApp).
    pub ventana_abierta: bool,
    pub cerrada_en: Option<DateTime<Utc>>,
}

impl From<Conversacion> for ConversacionRespuesta {
    fn from(c: Conversacion) -> Self {
        Self {
            ventana_abierta: c.ventana_abierta(RelojSistema.ahora()),
            id: c.id.as_uuid(),
            telefono: c.telefono,
            persona_id: c.persona_id.map(PersonaId::as_uuid),
            estado: c.estado,
            atendida_por: c.atendida_por.map(CuentaId::as_uuid),
            abierta_en: c.abierta_en,
            ultimo_entrante_en: c.ultimo_entrante_en,
            cerrada_en: c.cerrada_en,
        }
    }
}

/// Mensaje de una conversación.
#[derive(Debug, Serialize, ToSchema)]
pub struct MensajeConversacionRespuesta {
    /// Lo escribió el residente.
    pub entrante: bool,
    pub texto: String,
    pub autor_cuenta_id: Option<Uuid>,
    pub creado_en: DateTime<Utc>,
}

impl From<MensajeConversacion> for MensajeConversacionRespuesta {
    fn from(m: MensajeConversacion) -> Self {
        Self {
            entrante: m.entrante,
            texto: m.texto,
            autor_cuenta_id: m.autor_cuenta_id.map(CuentaId::as_uuid),
            creado_en: m.creado_en,
        }
    }
}

/// Conversación con sus mensajes.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleConversacionRespuesta {
    pub conversacion: ConversacionRespuesta,
    pub mensajes: Vec<MensajeConversacionRespuesta>,
}

/// Filtro de la bandeja.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroConversaciones {
    #[param(value_type = Option<crate::esquemas::EstadoConversacion>)]
    pub estado: Option<EstadoConversacion>,
}

/// Bandeja de conversaciones, más recientes primero.
#[utoipa::path(
    get,
    path = "/conversaciones",
    tag = ETIQUETA_CONVERSACIONES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroConversaciones, ParametrosPagina),
    responses(
        (status = OK, description = "Conversaciones", body = PaginaRespuesta<ConversacionRespuesta>),
        (status = FORBIDDEN, description = "Sin `conversaciones.atender` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_conversaciones(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroConversaciones>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<ConversacionRespuesta>>> {
    let pagina = uc::listar_conversaciones(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        f.estado,
        p.validar()?,
    )
    .await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

fn id_conversacion(ruta: (Uuid, Uuid)) -> ConversacionId {
    ConversacionId::from_uuid(ruta.1)
}

/// Una conversación con sus mensajes.
#[utoipa::path(
    get,
    path = "/conversaciones/{conversacion_id}",
    tag = ETIQUETA_CONVERSACIONES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("conversacion_id" = Uuid, Path, description = "Conversación"),
    ),
    responses(
        (status = OK, description = "Conversación", body = DetalleConversacionRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver_conversacion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DetalleConversacionRespuesta>> {
    let (c, mensajes) =
        uc::ver_conversacion(&state.unidad, &RelojSistema, &en.ctx, id_conversacion(ruta)).await?;
    Ok(Json(DetalleConversacionRespuesta {
        conversacion: c.into(),
        mensajes: mensajes.into_iter().map(Into::into).collect(),
    }))
}

/// Respuesta de la administración.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RespuestaPeticion {
    pub texto: String,
}

/// Responde por WhatsApp (dentro de la ventana de 24 h desde el último mensaje del residente).
#[utoipa::path(
    post,
    path = "/conversaciones/{conversacion_id}/mensajes",
    tag = ETIQUETA_CONVERSACIONES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("conversacion_id" = Uuid, Path, description = "Conversación"),
    ),
    request_body = RespuestaPeticion,
    responses(
        (status = CREATED, description = "Respuesta encolada", body = MensajeConversacionRespuesta),
        (status = CONFLICT, description = "Conversación cerrada o ventana de 24 h vencida (`conflicto`)", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `conversaciones.atender` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn responder(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<RespuestaPeticion>,
) -> ApiResult<(StatusCode, Json<MensajeConversacionRespuesta>)> {
    let m = uc::responder_conversacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_conversacion(ruta),
        p.texto,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(m.into())))
}

/// Cierra la conversación (el chat vuelve al menú en el siguiente mensaje del residente).
#[utoipa::path(
    post,
    path = "/conversaciones/{conversacion_id}/cierre",
    tag = ETIQUETA_CONVERSACIONES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("conversacion_id" = Uuid, Path, description = "Conversación"),
    ),
    responses(
        (status = OK, description = "Conversación cerrada", body = ConversacionRespuesta),
        (status = CONFLICT, description = "Ya estaba cerrada (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn cerrar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ConversacionRespuesta>> {
    let c = uc::cerrar_conversacion(&state.unidad, &RelojSistema, &en.ctx, id_conversacion(ruta))
        .await?;
    Ok(Json(c.into()))
}
