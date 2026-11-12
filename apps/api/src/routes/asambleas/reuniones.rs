//! Reuniones: crear, configurar, orden del día, anexos, convocatoria, transiciones, quórum,
//! tablero y WebSocket en vivo.

use application::{
    casos_uso::asambleas::{
        quorum::{foto_manual, quorum_actual, tablero_en_vivo},
        reuniones::{
            self as uc, CambiosReunion, DatosPunto, EnvioConvocatoriaOpciones, NuevaReunion,
            ResultadoConvocatoria,
        },
    },
    puertos::RelojSistema,
};
use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::Response,
};
use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, Convocatoria, EstadoPunto, EstadoReunion, Modalidad, PuntoOrdenId, ReunionId,
    TipoDocumentoReunion, TipoReunion,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::{
    ConfiguracionAsambleaDto, ConstanciaRespuesta, DetalleReunionRespuesta, DocumentoDto,
    PuntoRespuesta, QuorumRespuesta, ReunionRespuesta, TableroRespuesta,
};
use crate::{
    en_vivo::tablero_json,
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_ASAMBLEAS,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::{EnCopropiedad, EnCopropiedadWs, SUBPROTOCOLO_WS},
    state::AppState,
};

pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar, crear))
        .routes(routes!(ver, editar))
        .routes(routes!(crear_punto))
        .routes(routes!(editar_punto, eliminar_punto))
        .routes(routes!(estado_punto))
        .routes(routes!(agregar_documento))
        .routes(routes!(quitar_documento))
        .routes(routes!(convocar))
        .routes(routes!(constancias))
        .routes(routes!(segunda_convocatoria))
        .routes(routes!(transicionar))
        .routes(routes!(quorum, foto_quorum))
        .routes(routes!(tablero))
        .routes(routes!(en_vivo))
}

/// Reunión nueva (o sus datos al editarla en borrador).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReunionPeticion {
    #[schema(example = "Asamblea general ordinaria 2027")]
    pub titulo: String,
    #[schema(value_type = crate::esquemas::TipoReunion)]
    pub tipo: TipoReunion,
    /// `primera` (por defecto), `segunda` o `derecho_propio`.
    #[schema(value_type = Option<crate::esquemas::Convocatoria>)]
    pub convocatoria: Option<Convocatoria>,
    /// `presencial`, `virtual`, `mixta` o `escrita` (consulta por escrito, art. 43).
    #[schema(value_type = crate::esquemas::Modalidad)]
    pub modalidad: Modalidad,
    pub fecha_programada: DateTime<Utc>,
    pub lugar: Option<String>,
    /// Enlace de Google Meet u otro proveedor.
    pub enlace_video: Option<String>,
    /// Solo en consultas por escrito: hasta un mes después de `fecha_programada`.
    pub plazo_respuesta: Option<DateTime<Utc>>,
    #[serde(default)]
    pub configuracion: ConfiguracionAsambleaDto,
}

impl From<ReunionPeticion> for NuevaReunion {
    fn from(p: ReunionPeticion) -> Self {
        Self {
            titulo: p.titulo,
            tipo: p.tipo,
            convocatoria: p.convocatoria.unwrap_or(Convocatoria::Primera),
            modalidad: p.modalidad,
            fecha_programada: p.fecha_programada,
            lugar: p.lugar,
            enlace_video: p.enlace_video,
            plazo_respuesta: p.plazo_respuesta,
            configuracion: p.configuracion.into(),
        }
    }
}

/// Filtro del listado.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroReuniones {
    /// Estado de la reunión.
    #[param(value_type = Option<crate::esquemas::EstadoReunion>)]
    pub estado: Option<EstadoReunion>,
    pub limite: Option<u32>,
    pub despues: Option<Uuid>,
}

/// Reuniones de la copropiedad, más recientes primero.
#[utoipa::path(
    get,
    path = "/asambleas",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroReuniones),
    responses(
        (status = OK, description = "Página de reuniones", body = PaginaRespuesta<ReunionRespuesta>),
        (status = FORBIDDEN, description = "Sin `asambleas.ver` o módulo no contratado", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroReuniones>,
) -> ApiResult<Json<PaginaRespuesta<ReunionRespuesta>>> {
    let pagina = ParametrosPagina {
        limite: f.limite,
        despues: f.despues,
    }
    .validar()?;
    let p = uc::listar_reuniones(&state.unidad, &RelojSistema, &en.ctx, f.estado, pagina).await?;
    Ok(Json(p.mapear(Into::into).into()))
}

/// Crea una reunión en borrador con su configuración (poderes, mora, base de cómputo).
#[utoipa::path(
    post,
    path = "/asambleas",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = ReunionPeticion,
    responses(
        (status = CREATED, description = "Reunión creada", body = ReunionRespuesta),
        (status = FORBIDDEN, description = "Sin `asambleas.convocar` o módulo no contratado", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos o configuración inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<ReunionPeticion>,
) -> ApiResult<(StatusCode, Json<ReunionRespuesta>)> {
    let r = uc::crear_reunion(&state.unidad, &RelojSistema, &en.ctx, p.into()).await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Reunión con su orden del día y anexos.
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    responses(
        (status = OK, description = "Reunión", body = DetalleReunionRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DetalleReunionRespuesta>> {
    let id = ruta.1;
    let d = uc::ver_reunion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
    )
    .await?;
    Ok(Json(DetalleReunionRespuesta {
        reunion: d.reunion.into(),
        puntos: d.puntos.into_iter().map(Into::into).collect(),
        documentos: d.documentos.into_iter().map(Into::into).collect(),
    }))
}

/// Cambios: los datos de la convocatoria solo en borrador; la configuración, hasta instalar.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EditarReunionPeticion {
    pub datos: Option<ReunionPeticion>,
    pub configuracion: Option<ConfiguracionAsambleaDto>,
}

/// Edita la reunión (la elección de la mora queda auditada).
#[utoipa::path(
    patch,
    path = "/asambleas/{reunion_id}",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    request_body = EditarReunionPeticion,
    responses(
        (status = OK, description = "Reunión actualizada", body = ReunionRespuesta),
        (status = CONFLICT, description = "El estado ya no permite el cambio (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn editar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<EditarReunionPeticion>,
) -> ApiResult<Json<ReunionRespuesta>> {
    let id = ruta.1;
    let r = uc::editar_reunion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        CambiosReunion {
            datos: p.datos.map(Into::into),
            configuracion: p.configuracion.map(Into::into),
        },
    )
    .await?;
    Ok(Json(r.into()))
}

/// Punto del orden del día.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PuntoPeticion {
    #[schema(example = 1)]
    pub orden: u16,
    #[schema(example = "Verificación del quórum")]
    pub titulo: String,
    pub descripcion: Option<String>,
}

impl From<PuntoPeticion> for DatosPunto {
    fn from(p: PuntoPeticion) -> Self {
        Self {
            orden: p.orden,
            titulo: p.titulo,
            descripcion: p.descripcion,
        }
    }
}

/// Agrega un punto al orden del día (en borrador).
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/puntos",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    request_body = PuntoPeticion,
    responses(
        (status = CREATED, description = "Punto creado", body = PuntoRespuesta),
        (status = CONFLICT, description = "Ya convocada u orden repetido (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_punto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PuntoPeticion>,
) -> ApiResult<(StatusCode, Json<PuntoRespuesta>)> {
    let id = ruta.1;
    let punto = uc::guardar_punto(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        None,
        p.into(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(punto.into())))
}

/// Edita un punto (en borrador).
#[utoipa::path(
    put,
    path = "/asambleas/{reunion_id}/puntos/{punto_id}",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("punto_id" = Uuid, Path, description = "Punto")),
    request_body = PuntoPeticion,
    responses(
        (status = OK, description = "Punto actualizado", body = PuntoRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn editar_punto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    Json(p): Json<PuntoPeticion>,
) -> ApiResult<Json<PuntoRespuesta>> {
    let (id, punto) = (ruta.1, ruta.2);
    let punto = uc::guardar_punto(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        Some(PuntoOrdenId::from_uuid(punto)),
        p.into(),
    )
    .await?;
    Ok(Json(punto.into()))
}

/// Elimina un punto (en borrador).
#[utoipa::path(
    delete,
    path = "/asambleas/{reunion_id}/puntos/{punto_id}",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("punto_id" = Uuid, Path, description = "Punto")),
    responses(
        (status = NO_CONTENT, description = "Eliminado"),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn eliminar_punto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let (id, punto) = (ruta.1, ruta.2);
    uc::eliminar_punto(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        PuntoOrdenId::from_uuid(punto),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Nuevo estado de un punto.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EstadoPuntoPeticion {
    /// `en_curso`, `terminado` o `aplazado`.
    #[schema(value_type = crate::esquemas::EstadoPunto)]
    pub estado: EstadoPunto,
}

/// Avanza un punto del orden del día durante la reunión.
#[utoipa::path(
    put,
    path = "/asambleas/{reunion_id}/puntos/{punto_id}/estado",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("punto_id" = Uuid, Path, description = "Punto")),
    request_body = EstadoPuntoPeticion,
    responses(
        (status = OK, description = "Punto actualizado", body = PuntoRespuesta),
        (status = CONFLICT, description = "Transición inválida o reunión no instalada (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn estado_punto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    Json(p): Json<EstadoPuntoPeticion>,
) -> ApiResult<Json<PuntoRespuesta>> {
    let (id, punto) = (ruta.1, ruta.2);
    let punto = uc::cambiar_estado_punto(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        PuntoOrdenId::from_uuid(punto),
        p.estado,
    )
    .await?;
    Ok(Json(punto.into()))
}

/// Anexo (archivo ya subido con `POST /archivos`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DocumentoPeticion {
    pub archivo_id: Uuid,
    #[schema(value_type = crate::esquemas::TipoDocumentoReunion)]
    pub tipo: TipoDocumentoReunion,
    #[schema(example = "Estados financieros 2026")]
    pub titulo: String,
}

/// Anexa un documento a la convocatoria.
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/documentos",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    request_body = DocumentoPeticion,
    responses(
        (status = CREATED, description = "Anexo agregado", body = DocumentoDto),
        (status = CONFLICT, description = "Ya anexo o reunión en curso (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn agregar_documento(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<DocumentoPeticion>,
) -> ApiResult<(StatusCode, Json<DocumentoDto>)> {
    let id = ruta.1;
    let d = uc::agregar_documento(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        ArchivoId::from_uuid(p.archivo_id),
        p.tipo,
        &p.titulo,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(d.into())))
}

/// Quita un anexo.
#[utoipa::path(
    delete,
    path = "/asambleas/{reunion_id}/documentos/{archivo_id}",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("archivo_id" = Uuid, Path, description = "Archivo")),
    responses(
        (status = NO_CONTENT, description = "Quitado"),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn quitar_documento(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let (id, archivo) = (ruta.1, ruta.2);
    uc::quitar_documento(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        ArchivoId::from_uuid(archivo),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Canales del envío de la convocatoria.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ConvocarPeticion {
    /// Plantilla `convocatoria_asamblea` (por defecto, sí).
    pub whatsapp: Option<bool>,
    /// Correo (por defecto, sí).
    pub correo: Option<bool>,
}

impl ConvocarPeticion {
    fn opciones(&self) -> EnvioConvocatoriaOpciones {
        EnvioConvocatoriaOpciones {
            whatsapp: self.whatsapp.unwrap_or(true),
            correo: self.correo.unwrap_or(true),
        }
    }
}

/// Resumen de la convocatoria.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConvocatoriaRespuesta {
    pub reunion: ReunionRespuesta,
    pub bienes_padron: u32,
    pub envios_whatsapp: u32,
    pub envios_correo: u32,
    /// Representantes sin teléfono ni correo: hay que convocarlos por otro medio.
    pub sin_contacto: u32,
}

impl From<ResultadoConvocatoria> for ConvocatoriaRespuesta {
    fn from(r: ResultadoConvocatoria) -> Self {
        Self {
            reunion: r.reunion.into(),
            bienes_padron: r.bienes_padron,
            envios_whatsapp: r.envios_whatsapp,
            envios_correo: r.envios_correo,
            sin_contacto: r.sin_contacto,
        }
    }
}

/// Convoca: fija el padrón (cuadro de coeficientes vigente y representantes) y envía la
/// convocatoria por WhatsApp y correo con constancia. Ordinarias de primera convocatoria: al
/// menos 15 días calendario de antelación (art. 39).
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/convocatoria",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    request_body = ConvocarPeticion,
    responses(
        (status = OK, description = "Convocada", body = ConvocatoriaRespuesta),
        (status = CONFLICT, description = "No está en borrador (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Antelación, orden del día o coeficientes (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn convocar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ConvocarPeticion>,
) -> ApiResult<Json<ConvocatoriaRespuesta>> {
    let id = ruta.1;
    let r = uc::convocar(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        p.opciones(),
    )
    .await?;
    Ok(Json(r.into()))
}

/// Constancias de envío de la convocatoria (estado y acuse de cada mensaje).
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/convocatoria/constancias",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    responses((status = OK, description = "Constancias", body = Vec<ConstanciaRespuesta>))
)]
pub async fn constancias(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<ConstanciaRespuesta>>> {
    let id = ruta.1;
    let v = uc::constancias_convocatoria(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Sin quórum en primera convocatoria: cancela la reunión y convoca la de segunda para el
/// tercer día hábil siguiente a las 8:00 p. m. (art. 41).
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/segunda-convocatoria",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión de primera convocatoria")),
    request_body = ConvocarPeticion,
    responses(
        (status = CREATED, description = "Segunda reunión convocada", body = ConvocatoriaRespuesta),
        (status = CONFLICT, description = "Tiene quórum o no está en registro (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn segunda_convocatoria(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ConvocarPeticion>,
) -> ApiResult<(StatusCode, Json<ConvocatoriaRespuesta>)> {
    let id = ruta.1;
    let r = uc::convocar_segunda(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        p.opciones(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Acción sobre la reunión.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccionReunion {
    AbrirRegistro,
    Instalar,
    Suspender,
    Reanudar,
    Cerrar,
    Cancelar,
}

/// Transición de la reunión.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TransicionPeticion {
    pub accion: AccionReunion,
    /// Obligatorio al cancelar.
    pub motivo: Option<String>,
}

/// Abre el registro, instala (exige quórum), suspende, reanuda, cierra (cierra las preguntas
/// abiertas y envía el resumen por WhatsApp) o cancela.
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/transiciones",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    request_body = TransicionPeticion,
    responses(
        (status = OK, description = "Reunión actualizada", body = ReunionRespuesta),
        (status = CONFLICT, description = "Transición inválida (`conflicto_estado`) o sin quórum (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn transicionar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<TransicionPeticion>,
) -> ApiResult<Json<ReunionRespuesta>> {
    let id = ruta.1;
    let (u, c, id) = (&state.unidad, &en.ctx, ReunionId::from_uuid(id));
    let r = match p.accion {
        AccionReunion::AbrirRegistro => uc::abrir_registro(u, &RelojSistema, c, id).await?,
        AccionReunion::Instalar => uc::instalar(u, &RelojSistema, c, id).await?,
        AccionReunion::Suspender => uc::suspender_o_reanudar(u, &RelojSistema, c, id, true).await?,
        AccionReunion::Reanudar => uc::suspender_o_reanudar(u, &RelojSistema, c, id, false).await?,
        AccionReunion::Cerrar => uc::cerrar_reunion(u, &RelojSistema, c, id).await?,
        AccionReunion::Cancelar => {
            uc::cancelar(
                u,
                &RelojSistema,
                c,
                id,
                p.motivo.as_deref().unwrap_or_default(),
            )
            .await?
        }
    };
    Ok(Json(r.into()))
}

/// Quórum actual por coeficiente y por unidades.
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/quorum",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    responses((status = OK, description = "Quórum", body = QuorumRespuesta))
)]
pub async fn quorum(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<QuorumRespuesta>> {
    let id = ruta.1;
    Ok(Json(
        quorum_actual(
            &state.unidad,
            &RelojSistema,
            &en.ctx,
            ReunionId::from_uuid(id),
        )
        .await?
        .into(),
    ))
}

/// Toma una foto manual del quórum para el acta.
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/quorum",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    responses((status = CREATED, description = "Foto tomada", body = QuorumRespuesta))
)]
pub async fn foto_quorum(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<(StatusCode, Json<QuorumRespuesta>)> {
    let id = ruta.1;
    let q = foto_manual(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(q.into())))
}

/// Tablero en vivo (el mismo que emite el WebSocket).
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/tablero",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    responses((status = OK, description = "Tablero", body = TableroRespuesta))
)]
pub async fn tablero(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<TableroRespuesta>> {
    let id = ruta.1;
    let t = tablero_en_vivo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
    )
    .await?;
    Ok(Json(t.into()))
}

/// WebSocket del tablero en vivo: al conectar envía el tablero y luego uno nuevo cada vez que
/// algo cambia (a lo sumo cada ~500 ms). El navegador envía el token como subprotocolo:
/// `new WebSocket(url, ["dad.v1", token])`.
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/en-vivo",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    responses(
        (status = SWITCHING_PROTOCOLS, description = "WebSocket abierto: mensajes JSON `TableroRespuesta`"),
        (status = FORBIDDEN, description = "Sin `asambleas.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn en_vivo(
    State(state): State<AppState>,
    EnCopropiedadWs(en): EnCopropiedadWs,
    Path(ruta): Path<(Uuid, Uuid)>,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    let reunion = ReunionId::from_uuid(ruta.1);
    // Verifica el permiso, la reunión y el módulo antes de abrir el socket.
    let inicial = tablero_json(&state.unidad, &en.ctx, reunion).await?;
    let rx = state.en_vivo.suscribir(en.ctx.tenant_id, reunion);
    Ok(ws
        .protocols([SUBPROTOCOLO_WS])
        .on_upgrade(move |socket| atender_socket(socket, inicial, rx)))
}

async fn atender_socket(
    mut socket: WebSocket,
    inicial: String,
    mut rx: tokio::sync::broadcast::Receiver<std::sync::Arc<str>>,
) {
    if socket.send(Message::Text(inicial.into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            m = rx.recv() => match m {
                Ok(json) => {
                    if socket.send(Message::Text(json.as_ref().into())).await.is_err() {
                        return;
                    }
                }
                // Si la pantalla se atrasó, el siguiente tablero la pone al día.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            },
            entrada = socket.recv() => match entrada {
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
        }
    }
}
