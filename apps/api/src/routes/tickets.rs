//! Tickets (plan 07b, módulo `gestion_administrativa`): `/tickets`, `/tickets/{ticket_id}/…`,
//! `/tickets/configuracion`, `/tickets/categorias` y `/tickets/tablero`, dentro de
//! `/api/v1/copropiedades/{copropiedad_id}`.

use application::{
    casos_uso::tickets::{
        self as uc, CambioEstadoTicket, Clasificacion, ConsultaTickets, DetalleTicket,
        MensajeVista, NuevoMensaje, NuevoTicket, TicketVista,
    },
    puertos::{
        RelojSistema,
        tickets::{
            AdjuntoTicket, CategoriaTicket, ConfiguracionTickets, EventoTicket, MensajeTicket,
            Responsable, TableroTickets,
        },
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, NaiveTime, Utc};
use domain::{
    ActivoId, ArchivoId, BienPrivadoId, Calificacion, CanalTicket, CategoriaTicketId, CuentaId,
    EstadoTicket, JornadaHabil, OrganizacionId, PersonaId, Plazo, PlazosTicket, PrioridadTicket,
    RecursoId, TicketId, TipoTicket, UnidadPlazo,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_TICKETS,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de tickets.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(ver_configuracion, guardar_configuracion))
        .routes(routes!(listar_categorias, crear_categoria))
        .routes(routes!(editar_categoria))
        .routes(routes!(listar, radicar))
        .routes(routes!(tablero))
        .routes(routes!(ver, clasificar))
        .routes(routes!(asignar))
        .routes(routes!(transicionar))
        .routes(routes!(comentar))
        .routes(routes!(calificar))
}

fn id_ticket(ruta: (Uuid, Uuid)) -> TicketId {
    TicketId::from_uuid(ruta.1)
}

// ---------------------------------------------------------------------------------------------
// DTO comunes
// ---------------------------------------------------------------------------------------------

/// Responsable: una cuenta **o** una organización con contrato de servicio.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, ToSchema)]
pub struct ResponsableDto {
    pub cuenta_id: Option<Uuid>,
    pub organizacion_id: Option<Uuid>,
}

impl ResponsableDto {
    /// # Errors
    ///
    /// [`ApiError::PeticionInvalida`] si no trae exactamente uno de los dos.
    pub fn validar(self) -> ApiResult<Responsable> {
        match (self.cuenta_id, self.organizacion_id) {
            (Some(c), None) => Ok(Responsable::Cuenta(CuentaId::from_uuid(c))),
            (None, Some(o)) => Ok(Responsable::Organizacion(OrganizacionId::from_uuid(o))),
            _ => Err(ApiError::PeticionInvalida(
                "indique `cuenta_id` u `organizacion_id` (uno solo)".into(),
            )),
        }
    }

    /// Opcional: `None` si no trae ninguno.
    ///
    /// # Errors
    ///
    /// [`ApiError::PeticionInvalida`] si trae los dos.
    pub fn opcional(r: Option<Self>) -> ApiResult<Option<Responsable>> {
        r.filter(|r| r.cuenta_id.is_some() || r.organizacion_id.is_some())
            .map(Self::validar)
            .transpose()
    }
}

impl From<Responsable> for ResponsableDto {
    fn from(r: Responsable) -> Self {
        let (c, o) = Responsable::columnas(Some(r));
        Self {
            cuenta_id: c.map(CuentaId::as_uuid),
            organizacion_id: o.map(OrganizacionId::as_uuid),
        }
    }
}

/// Plazo de respuesta.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema)]
pub struct PlazoDto {
    #[schema(example = 15)]
    pub cantidad: u32,
    /// `horas`, `horas_habiles` o `dias_habiles`.
    #[schema(value_type = String, example = "dias_habiles")]
    pub unidad: UnidadPlazo,
}

impl From<Plazo> for PlazoDto {
    fn from(p: Plazo) -> Self {
        Self {
            cantidad: p.cantidad,
            unidad: p.unidad,
        }
    }
}

/// Encuesta de satisfacción.
#[derive(Debug, Serialize, ToSchema)]
pub struct EncuestaDto {
    #[schema(example = 5)]
    pub calificacion: u8,
    pub comentario: Option<String>,
}

/// Ticket. Si es un reporte de convivencia y la cuenta no tiene `tickets.convivencia.ver`, la
/// identidad del solicitante y su bien privado vienen vacíos (`identidad_oculta`).
#[derive(Debug, Serialize, ToSchema)]
pub struct TicketRespuesta {
    pub id: Uuid,
    /// Número legible por copropiedad y año.
    #[schema(example = "PQR-2026-00042")]
    pub numero: String,
    #[schema(value_type = String, example = "solicitud_mantenimiento")]
    pub tipo: TipoTicket,
    pub categoria_id: Option<Uuid>,
    #[schema(value_type = String, example = "media")]
    pub prioridad: PrioridadTicket,
    #[schema(value_type = String, example = "web")]
    pub canal: CanalTicket,
    #[schema(value_type = String, example = "abierto")]
    pub estado: EstadoTicket,
    pub asunto: String,
    pub descripcion: String,
    pub solicitante_persona_id: Option<Uuid>,
    pub solicitante_cuenta_id: Option<Uuid>,
    pub bien_privado_id: Option<Uuid>,
    pub ubicacion: Option<String>,
    pub recurso_id: Option<Uuid>,
    pub activo_id: Option<Uuid>,
    pub asignado: Option<ResponsableDto>,
    pub plazo: Option<PlazoDto>,
    pub vence_en: Option<DateTime<Utc>>,
    pub vencido: bool,
    pub resuelto_en: Option<DateTime<Utc>>,
    /// Hasta cuándo el solicitante puede reabrirlo; luego se cierra solo.
    pub reabrible_hasta: Option<DateTime<Utc>>,
    pub cerrado_en: Option<DateTime<Utc>>,
    pub duplicado_de: Option<Uuid>,
    pub confidencial: bool,
    pub identidad_oculta: bool,
    pub encuesta: Option<EncuestaDto>,
    pub creado_en: DateTime<Utc>,
    pub actualizado_en: DateTime<Utc>,
}

impl From<TicketVista> for TicketRespuesta {
    fn from(v: TicketVista) -> Self {
        let t = v.ticket;
        Self {
            id: t.id.as_uuid(),
            numero: t.numero.to_string(),
            tipo: t.tipo,
            categoria_id: t.categoria_id.map(CategoriaTicketId::as_uuid),
            prioridad: t.prioridad,
            canal: t.canal,
            estado: t.estado,
            asunto: t.asunto,
            descripcion: t.descripcion,
            solicitante_persona_id: t.solicitante_persona_id.map(PersonaId::as_uuid),
            solicitante_cuenta_id: t.solicitante_cuenta_id.map(CuentaId::as_uuid),
            bien_privado_id: t.bien_privado_id.map(BienPrivadoId::as_uuid),
            ubicacion: t.ubicacion,
            recurso_id: t.recurso_id.map(RecursoId::as_uuid),
            activo_id: t.activo_id.map(ActivoId::as_uuid),
            asignado: t.asignado.map(Into::into),
            plazo: t.plazo.map(Into::into),
            vence_en: t.vence_en,
            vencido: v.vencido,
            resuelto_en: t.resuelto_en,
            reabrible_hasta: t.reabrible_hasta,
            cerrado_en: t.cerrado_en,
            duplicado_de: t.duplicado_de.map(TicketId::as_uuid),
            confidencial: t.confidencial,
            identidad_oculta: v.identidad_oculta,
            encuesta: t.encuesta.map(|e| EncuestaDto {
                calificacion: e.calificacion.valor(),
                comentario: e.comentario,
            }),
            creado_en: t.creado_en,
            actualizado_en: t.actualizado_en,
        }
    }
}

fn vista_propia(t: application::puertos::tickets::Ticket) -> TicketRespuesta {
    let vencido = t.vencido(Utc::now());
    TicketVista {
        ticket: t,
        identidad_oculta: false,
        vencido,
    }
    .into()
}

// ---------------------------------------------------------------------------------------------
// Configuración
// ---------------------------------------------------------------------------------------------

/// Regla de plazo: por tipo y, opcionalmente, por prioridad.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReglaPlazoDto {
    #[schema(value_type = String, example = "queja")]
    pub tipo: TipoTicket,
    /// `null` = cualquier prioridad.
    #[schema(value_type = Option<String>, example = "urgente")]
    pub prioridad: Option<PrioridadTicket>,
    pub plazo: PlazoDto,
}

/// Configuración de tickets. Los tipos sin regla usan los plazos por defecto (Ley 1755: 15 días
/// hábiles para peticiones, 10 para documentos; mantenimiento según la prioridad).
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ConfiguracionTicketsDto {
    #[schema(value_type = String, example = "08:00:00")]
    pub jornada_inicio: NaiveTime,
    #[schema(value_type = String, example = "17:00:00")]
    pub jornada_fin: NaiveTime,
    pub incluye_sabado: bool,
    /// Días hábiles para reabrir un ticket resuelto.
    #[schema(example = 5)]
    pub dias_reapertura: u32,
    pub plazos: Vec<ReglaPlazoDto>,
}

impl From<ConfiguracionTickets> for ConfiguracionTicketsDto {
    fn from(c: ConfiguracionTickets) -> Self {
        Self {
            jornada_inicio: c.jornada.inicio,
            jornada_fin: c.jornada.fin,
            incluye_sabado: c.jornada.incluye_sabado,
            dias_reapertura: c.dias_reapertura,
            plazos: c
                .plazos
                .reglas()
                .iter()
                .map(|(tipo, prioridad, plazo)| ReglaPlazoDto {
                    tipo: *tipo,
                    prioridad: *prioridad,
                    plazo: (*plazo).into(),
                })
                .collect(),
        }
    }
}

/// Configuración de tickets de la copropiedad.
#[utoipa::path(
    get,
    path = "/tickets/configuracion",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Configuración", body = ConfiguracionTicketsDto),
        (status = FORBIDDEN, description = "Sin `tickets.configurar` (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn ver_configuracion(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<ConfiguracionTicketsDto>> {
    let c = uc::ver_configuracion(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(c.into()))
}

/// Reemplaza la configuración (jornada, reapertura y todas las reglas de plazo).
#[utoipa::path(
    put,
    path = "/tickets/configuracion",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = ConfiguracionTicketsDto,
    responses(
        (status = OK, description = "Configuración guardada", body = ConfiguracionTicketsDto),
        (status = FORBIDDEN, description = "Sin `tickets.configurar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Jornada o plazos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn guardar_configuracion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<ConfiguracionTicketsDto>,
) -> ApiResult<Json<ConfiguracionTicketsDto>> {
    let reglas = p
        .plazos
        .into_iter()
        .map(|r| {
            Ok((
                r.tipo,
                r.prioridad,
                Plazo::new(r.plazo.cantidad, r.plazo.unidad)?,
            ))
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let c = uc::guardar_configuracion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ConfiguracionTickets {
            jornada: JornadaHabil {
                inicio: p.jornada_inicio,
                fin: p.jornada_fin,
                incluye_sabado: p.incluye_sabado,
            },
            dias_reapertura: p.dias_reapertura,
            plazos: PlazosTicket::new(reglas),
        },
    )
    .await?;
    Ok(Json(c.into()))
}

// ---------------------------------------------------------------------------------------------
// Categorías
// ---------------------------------------------------------------------------------------------

/// Categoría de tickets.
#[derive(Debug, Serialize, ToSchema)]
pub struct CategoriaRespuesta {
    pub id: Uuid,
    #[schema(example = "Plomería")]
    pub nombre: String,
    pub activa: bool,
}

impl From<CategoriaTicket> for CategoriaRespuesta {
    fn from(c: CategoriaTicket) -> Self {
        Self {
            id: c.id.as_uuid(),
            nombre: c.nombre,
            activa: c.activa,
        }
    }
}

/// Datos de una categoría.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CategoriaPeticion {
    #[schema(example = "Plomería")]
    pub nombre: String,
    #[serde(default = "verdadero")]
    pub activa: bool,
}

const fn verdadero() -> bool {
    true
}

/// Categorías de tickets (las inactivas solo para quien configura).
#[utoipa::path(
    get,
    path = "/tickets/categorias",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Categorías", body = Vec<CategoriaRespuesta>),
        (status = FORBIDDEN, description = "Sin `tickets.crear` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_categorias(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<CategoriaRespuesta>>> {
    let v = uc::listar_categorias(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea una categoría.
#[utoipa::path(
    post,
    path = "/tickets/categorias",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = CategoriaPeticion,
    responses(
        (status = CREATED, description = "Categoría creada", body = CategoriaRespuesta),
        (status = CONFLICT, description = "Nombre repetido (`conflicto`)", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `tickets.configurar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_categoria(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<CategoriaPeticion>,
) -> ApiResult<(StatusCode, Json<CategoriaRespuesta>)> {
    let c = uc::guardar_categoria(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        None,
        p.nombre,
        p.activa,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Renombra, activa o inactiva una categoría.
#[utoipa::path(
    put,
    path = "/tickets/categorias/{categoria_id}",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("categoria_id" = Uuid, Path, description = "Categoría"),
    ),
    request_body = CategoriaPeticion,
    responses(
        (status = OK, description = "Categoría", body = CategoriaRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn editar_categoria(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<CategoriaPeticion>,
) -> ApiResult<Json<CategoriaRespuesta>> {
    let c = uc::guardar_categoria(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        Some(CategoriaTicketId::from_uuid(ruta.1)),
        p.nombre,
        p.activa,
    )
    .await?;
    Ok(Json(c.into()))
}

// ---------------------------------------------------------------------------------------------
// Tickets
// ---------------------------------------------------------------------------------------------

/// Filtros del listado.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroTicketsQuery {
    #[param(value_type = Option<String>, example = "abierto")]
    pub estado: Option<EstadoTicket>,
    #[param(value_type = Option<String>, example = "queja")]
    pub tipo: Option<TipoTicket>,
    #[param(value_type = Option<String>, example = "urgente")]
    pub prioridad: Option<PrioridadTicket>,
    pub bien_privado_id: Option<Uuid>,
    /// Solo los vencidos.
    #[serde(default)]
    pub vencidos: bool,
    /// Solo los que radicó la cuenta.
    #[serde(default)]
    pub mios: bool,
}

/// Tickets visibles para la cuenta, más recientes primero: todos con `tickets.ver_todos`; los
/// de convivencia con `tickets.convivencia.ver`; los asignados con `tickets.atender`; y los
/// propios.
#[utoipa::path(
    get,
    path = "/tickets",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroTicketsQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Tickets", body = PaginaRespuesta<TicketRespuesta>),
        (status = FORBIDDEN, description = "Sin permisos de tickets (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroTicketsQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<TicketRespuesta>>> {
    let pagina = uc::listar_tickets(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ConsultaTickets {
            estado: f.estado,
            tipo: f.tipo,
            prioridad: f.prioridad,
            bien_privado_id: f.bien_privado_id.map(BienPrivadoId::from_uuid),
            solo_vencidos: f.vencidos,
            mios: f.mios,
        },
        p.validar()?,
    )
    .await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Ticket nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RadicarPeticion {
    #[schema(value_type = String, example = "solicitud_mantenimiento")]
    pub tipo: TipoTicket,
    pub categoria_id: Option<Uuid>,
    /// `media` si se omite.
    #[schema(value_type = Option<String>, example = "alta")]
    pub prioridad: Option<PrioridadTicket>,
    /// `web` si se omite; `presencial` o `correo` cuando la administración registra un caso.
    #[schema(value_type = Option<String>, example = "web")]
    pub canal: Option<CanalTicket>,
    #[schema(example = "Fuga en el baño comunal del piso 3")]
    pub asunto: String,
    pub descripcion: String,
    pub bien_privado_id: Option<Uuid>,
    pub ubicacion: Option<String>,
    /// Bien común afectado.
    pub recurso_id: Option<Uuid>,
    pub activo_id: Option<Uuid>,
    /// Archivos ya subidos (`POST /archivos`).
    #[serde(default)]
    pub adjuntos: Vec<Uuid>,
    /// La genera el cliente; un reintento con la misma clave devuelve el ticket original.
    pub idempotency_key: String,
    /// Persona por la que radica la administración (casos presenciales o por correo); exige
    /// `tickets.asignar`.
    pub solicitante_persona_id: Option<Uuid>,
}

/// Radica un ticket. Si es urgente avisa de inmediato a quienes tienen
/// `tickets.urgencias.recibir`.
#[utoipa::path(
    post,
    path = "/tickets",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = RadicarPeticion,
    responses(
        (status = CREATED, description = "Ticket radicado (o el original si se reintenta)", body = TicketRespuesta),
        (status = FORBIDDEN, description = "Sin `tickets.crear` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Persona, categoría o adjunto inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos o sin vínculo con el bien privado (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn radicar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<RadicarPeticion>,
) -> ApiResult<(StatusCode, Json<TicketRespuesta>)> {
    let t = uc::radicar_ticket(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoTicket {
            tipo: p.tipo,
            categoria_id: p.categoria_id.map(CategoriaTicketId::from_uuid),
            prioridad: p.prioridad,
            canal: p.canal.unwrap_or(CanalTicket::Web),
            asunto: p.asunto,
            descripcion: p.descripcion,
            bien_privado_id: p.bien_privado_id.map(BienPrivadoId::from_uuid),
            ubicacion: p.ubicacion,
            recurso_id: p.recurso_id.map(RecursoId::from_uuid),
            activo_id: p.activo_id.map(ActivoId::from_uuid),
            adjuntos: p.adjuntos.into_iter().map(ArchivoId::from_uuid).collect(),
            idempotency_key: p.idempotency_key,
            solicitante: p.solicitante_persona_id.map(PersonaId::from_uuid),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(vista_propia(t))))
}

/// Pendientes por estado.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConteoEstado {
    #[schema(value_type = String, example = "en_progreso")]
    pub estado: EstadoTicket,
    pub cantidad: u64,
}

/// Indicadores del tablero.
#[derive(Debug, Serialize, ToSchema)]
pub struct TableroTicketsRespuesta {
    pub pendientes_por_estado: Vec<ConteoEstado>,
    pub vencidos: u64,
    /// Horas promedio hasta la primera atención.
    pub horas_promedio_respuesta: Option<f64>,
    /// Horas promedio hasta la resolución.
    pub horas_promedio_solucion: Option<f64>,
    /// Promedio de la encuesta (1 a 5).
    pub satisfaccion_promedio: Option<f64>,
}

impl From<TableroTickets> for TableroTicketsRespuesta {
    fn from(t: TableroTickets) -> Self {
        Self {
            pendientes_por_estado: t
                .por_estado
                .into_iter()
                .map(|(estado, cantidad)| ConteoEstado { estado, cantidad })
                .collect(),
            vencidos: t.vencidos,
            horas_promedio_respuesta: t.horas_promedio_respuesta,
            horas_promedio_solucion: t.horas_promedio_solucion,
            satisfaccion_promedio: t.satisfaccion_promedio,
        }
    }
}

/// Tablero: pendientes por estado, vencidos, tiempos promedio y satisfacción.
#[utoipa::path(
    get,
    path = "/tickets/tablero",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Indicadores", body = TableroTicketsRespuesta),
        (status = FORBIDDEN, description = "Sin `tickets.ver_todos` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn tablero(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<TableroTicketsRespuesta>> {
    let t = uc::tablero_tickets(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(t.into()))
}

/// Mensaje de un ticket.
#[derive(Debug, Serialize, ToSchema)]
pub struct MensajeRespuesta {
    pub id: Uuid,
    pub autor_cuenta_id: Option<Uuid>,
    pub autor_persona_id: Option<Uuid>,
    /// Lo escribió el solicitante.
    pub del_solicitante: bool,
    #[schema(value_type = String, example = "web")]
    pub canal: CanalTicket,
    /// Nota interna (el solicitante no la ve).
    pub interno: bool,
    pub texto: String,
    pub adjuntos: Vec<Uuid>,
    pub creado_en: DateTime<Utc>,
}

impl From<MensajeVista> for MensajeRespuesta {
    fn from(v: MensajeVista) -> Self {
        let m = v.mensaje;
        Self {
            id: m.id.as_uuid(),
            autor_cuenta_id: m.autor_cuenta_id.map(CuentaId::as_uuid),
            autor_persona_id: m.autor_persona_id.map(PersonaId::as_uuid),
            del_solicitante: v.del_solicitante,
            canal: m.canal,
            interno: m.interno,
            texto: m.texto,
            adjuntos: m.adjuntos.into_iter().map(ArchivoId::as_uuid).collect(),
            creado_en: m.creado_en,
        }
    }
}

impl From<MensajeTicket> for MensajeRespuesta {
    fn from(m: MensajeTicket) -> Self {
        MensajeVista {
            mensaje: m,
            del_solicitante: false,
        }
        .into()
    }
}

/// Adjunto de un ticket (descargar con `POST /archivos/{archivo_id}/enlace`).
#[derive(Debug, Serialize, ToSchema)]
pub struct AdjuntoRespuesta {
    pub archivo_id: Uuid,
    pub mensaje_id: Option<Uuid>,
    pub interno: bool,
}

impl From<AdjuntoTicket> for AdjuntoRespuesta {
    fn from(a: AdjuntoTicket) -> Self {
        Self {
            archivo_id: a.archivo_id.as_uuid(),
            mensaje_id: a.mensaje_id.map(|m| m.as_uuid()),
            interno: a.interno,
        }
    }
}

/// Cambio de estado del historial.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventoRespuesta {
    /// `null` = radicación.
    #[schema(value_type = Option<String>, example = "abierto")]
    pub desde: Option<EstadoTicket>,
    #[schema(value_type = String, example = "asignado")]
    pub hacia: EstadoTicket,
    pub actor_cuenta_id: Option<Uuid>,
    pub motivo: Option<String>,
    pub ocurrido_en: DateTime<Utc>,
}

impl From<EventoTicket> for EventoRespuesta {
    fn from(e: EventoTicket) -> Self {
        Self {
            desde: e.desde,
            hacia: e.hacia,
            actor_cuenta_id: e.actor_cuenta_id.map(CuentaId::as_uuid),
            motivo: e.motivo,
            ocurrido_en: e.ocurrido_en,
        }
    }
}

/// Ticket con su conversación, adjuntos e historial.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleRespuesta {
    pub ticket: TicketRespuesta,
    pub mensajes: Vec<MensajeRespuesta>,
    pub adjuntos: Vec<AdjuntoRespuesta>,
    pub eventos: Vec<EventoRespuesta>,
}

impl From<DetalleTicket> for DetalleRespuesta {
    fn from(d: DetalleTicket) -> Self {
        Self {
            ticket: d.ticket.into(),
            mensajes: d.mensajes.into_iter().map(Into::into).collect(),
            adjuntos: d.adjuntos.into_iter().map(Into::into).collect(),
            eventos: d.eventos.into_iter().map(Into::into).collect(),
        }
    }
}

/// Un ticket (sin notas internas para el solicitante).
#[utoipa::path(
    get,
    path = "/tickets/{ticket_id}",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("ticket_id" = Uuid, Path, description = "Ticket"),
    ),
    responses(
        (status = OK, description = "Ticket", body = DetalleRespuesta),
        (status = NOT_FOUND, description = "No existe o no es visible para la cuenta", body = ErrorRespuesta),
    )
)]
pub async fn ver(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DetalleRespuesta>> {
    let d = uc::ver_ticket(&state.unidad, &RelojSistema, &en.ctx, id_ticket(ruta)).await?;
    Ok(Json(d.into()))
}

/// Cambios de clasificación (los omitidos no cambian).
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ClasificarPeticion {
    #[schema(value_type = Option<String>, example = "reclamo")]
    pub tipo: Option<TipoTicket>,
    pub categoria_id: Option<Uuid>,
    #[schema(value_type = Option<String>, example = "alta")]
    pub prioridad: Option<PrioridadTicket>,
}

/// Reclasifica un ticket; si cambia el tipo o la prioridad se recalcula el vencimiento.
#[utoipa::path(
    patch,
    path = "/tickets/{ticket_id}",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("ticket_id" = Uuid, Path, description = "Ticket"),
    ),
    request_body = ClasificarPeticion,
    responses(
        (status = OK, description = "Ticket", body = TicketRespuesta),
        (status = FORBIDDEN, description = "Sin `tickets.asignar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya no está pendiente (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn clasificar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ClasificarPeticion>,
) -> ApiResult<Json<TicketRespuesta>> {
    let t = uc::clasificar_ticket(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_ticket(ruta),
        Clasificacion {
            tipo: p.tipo,
            categoria_id: p.categoria_id.map(CategoriaTicketId::from_uuid),
            prioridad: p.prioridad,
        },
    )
    .await?;
    Ok(Json(t.into()))
}

/// Asigna el ticket a una cuenta (con `tickets.atender`) o a una organización con contrato.
#[utoipa::path(
    post,
    path = "/tickets/{ticket_id}/asignacion",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("ticket_id" = Uuid, Path, description = "Ticket"),
    ),
    request_body = ResponsableDto,
    responses(
        (status = OK, description = "Ticket asignado", body = TicketRespuesta),
        (status = FORBIDDEN, description = "Sin `tickets.asignar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "El estado no admite asignación (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "El responsable no puede atender (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn asignar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ResponsableDto>,
) -> ApiResult<Json<TicketRespuesta>> {
    let t = uc::asignar_ticket(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_ticket(ruta),
        p.validar()?,
    )
    .await?;
    Ok(Json(t.into()))
}

/// Cambio de estado.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TransicionTicketPeticion {
    #[schema(value_type = String, example = "resuelto")]
    pub hacia: EstadoTicket,
    /// Obligatorio al resolver (es la respuesta al solicitante), rechazar o pedir información.
    pub motivo: Option<String>,
    /// Obligatorio al marcar `duplicado`.
    pub duplicado_de: Option<Uuid>,
}

/// Cambia el estado según el diagrama y el papel de la cuenta (gestiona, atiende o es el
/// solicitante, que puede cerrar o reabrir dentro del plazo).
#[utoipa::path(
    post,
    path = "/tickets/{ticket_id}/transiciones",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("ticket_id" = Uuid, Path, description = "Ticket"),
    ),
    request_body = TransicionTicketPeticion,
    responses(
        (status = OK, description = "Ticket", body = TicketRespuesta),
        (status = FORBIDDEN, description = "Su papel no permite ese cambio (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Transición no permitida (`conflicto_estado`) o venció la reapertura (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Falta el motivo o el original (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn transicionar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<TransicionTicketPeticion>,
) -> ApiResult<Json<TicketRespuesta>> {
    let t = uc::transicionar_ticket(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_ticket(ruta),
        CambioEstadoTicket {
            hacia: p.hacia,
            motivo: p.motivo,
            duplicado_de: p.duplicado_de.map(TicketId::from_uuid),
        },
    )
    .await?;
    Ok(Json(t.into()))
}

/// Mensaje nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MensajePeticion {
    pub texto: String,
    /// Nota interna (solo la administración y quien atiende).
    #[serde(default)]
    pub interno: bool,
    #[serde(default)]
    pub adjuntos: Vec<Uuid>,
}

/// Agrega un mensaje público o una nota interna. Si el ticket esperaba al solicitante y él
/// responde, vuelve a `en_progreso`.
#[utoipa::path(
    post,
    path = "/tickets/{ticket_id}/mensajes",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("ticket_id" = Uuid, Path, description = "Ticket"),
    ),
    request_body = MensajePeticion,
    responses(
        (status = CREATED, description = "Mensaje", body = MensajeRespuesta),
        (status = FORBIDDEN, description = "No es el solicitante ni atiende el ticket (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "El ticket terminó (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn comentar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<MensajePeticion>,
) -> ApiResult<(StatusCode, Json<MensajeRespuesta>)> {
    let m = uc::comentar_ticket(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_ticket(ruta),
        NuevoMensaje {
            texto: p.texto,
            interno: p.interno,
            adjuntos: p.adjuntos.into_iter().map(ArchivoId::from_uuid).collect(),
            canal: CanalTicket::Web,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(m.into())))
}

/// Encuesta de satisfacción.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EncuestaPeticion {
    /// De 1 a 5.
    #[schema(value_type = u8, example = 5)]
    pub calificacion: Calificacion,
    pub comentario: Option<String>,
}

/// El solicitante califica la atención (una vez, con el ticket resuelto o cerrado).
#[utoipa::path(
    post,
    path = "/tickets/{ticket_id}/encuesta",
    tag = ETIQUETA_TICKETS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("ticket_id" = Uuid, Path, description = "Ticket"),
    ),
    request_body = EncuestaPeticion,
    responses(
        (status = OK, description = "Ticket calificado", body = TicketRespuesta),
        (status = FORBIDDEN, description = "No es el solicitante (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Aún no está resuelto o ya calificó (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn calificar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<EncuestaPeticion>,
) -> ApiResult<Json<TicketRespuesta>> {
    let t = uc::calificar_ticket(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_ticket(ruta),
        p.calificacion,
        p.comentario,
    )
    .await?;
    Ok(Json(vista_propia(t)))
}
