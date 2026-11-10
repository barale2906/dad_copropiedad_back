//! Puerto de tickets (plan 07b): tickets, mensajes, historial de estados, configuración de
//! plazos y categorías, y los destinatarios de los avisos internos. Se usa dentro de la
//! transacción de una copropiedad.

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::{
    ActivoId, ArchivoId, BienPrivadoId, Calificacion, CanalTicket, CategoriaTicketId, CuentaId,
    DireccionCorreo, EstadoTicket, JornadaHabil, MensajeTicketId, NumeroTicket, OrganizacionId,
    Permiso, PersonaId, Plazo, PlazosTicket, PrioridadTicket, RecursoId, Telefono, TenantId,
    TicketId, TipoTicket,
};

use crate::{AppResult, Pagina, SolicitudPagina};

/// Responsable de un ticket, una orden de trabajo o un plan: una cuenta o una organización con
/// contrato de servicio en la copropiedad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Responsable {
    Cuenta(CuentaId),
    Organizacion(OrganizacionId),
}

impl Responsable {
    /// Columnas `(asignado_cuenta_id, asignado_organizacion_id)`.
    pub fn columnas(r: Option<Self>) -> (Option<CuentaId>, Option<OrganizacionId>) {
        match r {
            Some(Self::Cuenta(c)) => (Some(c), None),
            Some(Self::Organizacion(o)) => (None, Some(o)),
            None => (None, None),
        }
    }

    /// Inverso de [`Responsable::columnas`].
    pub fn desde_columnas(cuenta: Option<CuentaId>, org: Option<OrganizacionId>) -> Option<Self> {
        cuenta
            .map(Self::Cuenta)
            .or_else(|| org.map(Self::Organizacion))
    }
}

/// Encuesta de satisfacción al resolver o cerrar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encuesta {
    pub calificacion: Calificacion,
    pub comentario: Option<String>,
}

/// Ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub id: TicketId,
    pub numero: NumeroTicket,
    pub tipo: TipoTicket,
    pub categoria_id: Option<CategoriaTicketId>,
    pub prioridad: PrioridadTicket,
    pub canal: CanalTicket,
    pub estado: EstadoTicket,
    pub asunto: String,
    pub descripcion: String,
    pub solicitante_persona_id: Option<PersonaId>,
    pub solicitante_cuenta_id: Option<CuentaId>,
    pub bien_privado_id: Option<BienPrivadoId>,
    pub ubicacion: Option<String>,
    pub recurso_id: Option<RecursoId>,
    pub activo_id: Option<ActivoId>,
    pub asignado: Option<Responsable>,
    /// Plazo con que se calculó `vence_en` (se vuelve a contar al reanudar una espera).
    pub plazo: Option<Plazo>,
    pub vence_en: Option<DateTime<Utc>>,
    pub alerta_vencimiento_en: Option<DateTime<Utc>>,
    pub resuelto_en: Option<DateTime<Utc>>,
    /// Hasta cuándo el solicitante puede reabrirlo; después se cierra solo.
    pub reabrible_hasta: Option<DateTime<Utc>>,
    pub cerrado_en: Option<DateTime<Utc>>,
    pub duplicado_de: Option<TicketId>,
    /// La identidad del solicitante solo la ve quien tiene `tickets.convivencia.ver`.
    pub confidencial: bool,
    pub idempotency_key: String,
    pub encuesta: Option<Encuesta>,
    pub creado_en: DateTime<Utc>,
    pub actualizado_en: DateTime<Utc>,
}

impl Ticket {
    /// ¿Venció el plazo sin resolverse?
    pub fn vencido(&self, ahora: DateTime<Utc>) -> bool {
        self.estado.corre_plazo() && self.vence_en.is_some_and(|v| v <= ahora)
    }
}

/// Mensaje de un ticket. Las notas internas (`interno`) no las ve el solicitante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MensajeTicket {
    pub id: MensajeTicketId,
    pub ticket_id: TicketId,
    pub autor_cuenta_id: Option<CuentaId>,
    pub autor_persona_id: Option<PersonaId>,
    pub canal: CanalTicket,
    pub interno: bool,
    pub texto: String,
    pub adjuntos: Vec<ArchivoId>,
    pub creado_en: DateTime<Utc>,
}

/// Cambio de estado (historial append-only). `desde = None` es la radicación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventoTicket {
    pub ticket_id: TicketId,
    pub desde: Option<EstadoTicket>,
    pub hacia: EstadoTicket,
    pub actor_cuenta_id: Option<CuentaId>,
    pub motivo: Option<String>,
    pub ocurrido_en: DateTime<Utc>,
}

/// Adjunto de un ticket (al radicar o en un mensaje).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdjuntoTicket {
    pub archivo_id: ArchivoId,
    pub mensaje_id: Option<MensajeTicketId>,
    pub interno: bool,
}

/// Configuración de tickets de la copropiedad.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfiguracionTickets {
    pub jornada: JornadaHabil,
    pub dias_reapertura: u32,
    pub plazos: PlazosTicket,
}

impl ConfiguracionTickets {
    /// Valores por defecto (jornada de lunes a viernes, 5 días hábiles para reabrir, plazos de
    /// la Ley 1755).
    pub fn predeterminada() -> Self {
        Self {
            jornada: JornadaHabil::default(),
            dias_reapertura: domain::ticket::DIAS_REAPERTURA_PREDETERMINADOS,
            plazos: PlazosTicket::default(),
        }
    }
}

/// Categoría configurable (plomería, eléctrico, ascensores, ruido…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoriaTicket {
    pub id: CategoriaTicketId,
    pub nombre: String,
    pub activa: bool,
}

/// Qué tickets puede ver quien consulta (se combinan con OR).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VisibilidadTickets {
    /// `tickets.ver_todos`.
    pub todos: bool,
    /// `tickets.convivencia.ver`: los reportes de convivencia.
    pub convivencia: bool,
    /// Los que radicó o tiene asignados esta cuenta.
    pub cuenta: Option<CuentaId>,
    /// Los asignados a estas organizaciones (las de la cuenta, si atiende tickets).
    pub organizaciones: Vec<OrganizacionId>,
}

/// Filtros del listado.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroTickets {
    pub estado: Option<EstadoTicket>,
    pub tipo: Option<TipoTicket>,
    pub prioridad: Option<PrioridadTicket>,
    /// Solo los que radicó esta cuenta.
    pub solicitante_cuenta: Option<CuentaId>,
    /// Solo los que radicó esta persona (chat de WhatsApp).
    pub solicitante_persona: Option<PersonaId>,
    pub bien_privado_id: Option<BienPrivadoId>,
    /// Solo los vencidos a esta hora.
    pub vencidos_a: Option<DateTime<Utc>>,
}

/// Indicadores del tablero de tickets.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableroTickets {
    /// Pendientes por estado.
    pub por_estado: Vec<(EstadoTicket, u64)>,
    pub vencidos: u64,
    /// Horas promedio hasta la primera atención (primer cambio de estado tras la radicación).
    pub horas_promedio_respuesta: Option<f64>,
    /// Horas promedio hasta la resolución.
    pub horas_promedio_solucion: Option<f64>,
    /// Promedio de la encuesta de satisfacción.
    pub satisfaccion_promedio: Option<f64>,
}

/// Cuenta que recibe un aviso interno.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destinatario {
    pub cuenta_id: CuentaId,
    pub correo: Option<DireccionCorreo>,
    pub telefono: Option<Telefono>,
}

/// Serie de un consecutivo legible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerieConsecutivo {
    Ticket,
    OrdenTrabajo,
}

impl SerieConsecutivo {
    /// Texto de `consecutivos.serie`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ticket => "ticket",
            Self::OrdenTrabajo => "orden_trabajo",
        }
    }
}

pub trait RepositorioTickets: Send {
    /// Siguiente número de la serie en el año (atómico dentro de la transacción).
    fn siguiente_consecutivo(
        &mut self,
        serie: SerieConsecutivo,
        anio: i32,
    ) -> impl Future<Output = AppResult<u32>> + Send;

    /// Configuración de la copropiedad o la predeterminada si no hay.
    fn configuracion_tickets(
        &mut self,
    ) -> impl Future<Output = AppResult<ConfiguracionTickets>> + Send;

    /// Reemplaza la configuración y todas las reglas de plazo.
    fn guardar_configuracion_tickets(
        &mut self,
        c: &ConfiguracionTickets,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn categorias_ticket(&mut self)
    -> impl Future<Output = AppResult<Vec<CategoriaTicket>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya existe otra con el mismo nombre.
    fn guardar_categoria_ticket(
        &mut self,
        c: &CategoriaTicket,
        nueva: bool,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn ticket(&mut self, id: TicketId) -> impl Future<Output = AppResult<Option<Ticket>>> + Send;

    /// Ticket ya radicado con esa clave de idempotencia (o `wamid`).
    fn ticket_por_clave(
        &mut self,
        clave: &str,
    ) -> impl Future<Output = AppResult<Option<Ticket>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si la clave de idempotencia ya existe.
    fn insertar_ticket(&mut self, t: &Ticket) -> impl Future<Output = AppResult<()>> + Send;

    fn actualizar_ticket(&mut self, t: &Ticket) -> impl Future<Output = AppResult<()>> + Send;

    /// Tickets visibles que cumplen el filtro, más recientes primero (cursor por `id`
    /// descendente).
    fn tickets(
        &mut self,
        visibilidad: &VisibilidadTickets,
        filtro: &FiltroTickets,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Ticket>>> + Send;

    fn insertar_mensaje_ticket(
        &mut self,
        m: &MensajeTicket,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Mensajes en orden cronológico; sin las notas internas si `incluir_internos` es falso.
    fn mensajes_ticket(
        &mut self,
        ticket: TicketId,
        incluir_internos: bool,
    ) -> impl Future<Output = AppResult<Vec<MensajeTicket>>> + Send;

    fn insertar_adjunto_ticket(
        &mut self,
        ticket: TicketId,
        a: &AdjuntoTicket,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn adjuntos_ticket(
        &mut self,
        ticket: TicketId,
        incluir_internos: bool,
    ) -> impl Future<Output = AppResult<Vec<AdjuntoTicket>>> + Send;

    fn insertar_evento_ticket(
        &mut self,
        e: &EventoTicket,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn eventos_ticket(
        &mut self,
        ticket: TicketId,
    ) -> impl Future<Output = AppResult<Vec<EventoTicket>>> + Send;

    /// Resueltos cuyo plazo de reapertura ya pasó.
    fn tickets_por_cerrar(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<Ticket>>> + Send;

    /// Vencidos (con el plazo corriendo) que aún no se alertaron.
    fn tickets_vencidos_sin_alerta(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<Ticket>>> + Send;

    fn tablero_tickets(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<TableroTickets>> + Send;

    /// Persona de la copropiedad vinculada a la cuenta, si existe.
    fn persona_de_cuenta(
        &mut self,
        cuenta: CuentaId,
    ) -> impl Future<Output = AppResult<Option<PersonaId>>> + Send;

    /// Organizaciones para las que la cuenta trabaja en esta copropiedad (roles derivados de
    /// contratos de servicio vigentes).
    fn organizaciones_de_cuenta(
        &mut self,
        cuenta: CuentaId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<OrganizacionId>>> + Send;

    /// ¿La organización tiene un contrato de servicio vigente en la copropiedad?
    fn organizacion_con_contrato(
        &mut self,
        org: OrganizacionId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Cuentas con el permiso vigente en la copropiedad (avisos internos).
    fn destinatarios_con_permiso(
        &mut self,
        permiso: Permiso,
    ) -> impl Future<Output = AppResult<Vec<Destinatario>>> + Send;

    /// Línea de WhatsApp para avisos fuera de una conversación (`None` si no hay líneas).
    fn linea_predeterminada(&mut self) -> impl Future<Output = AppResult<Option<String>>> + Send;
}

/// Copropiedades con tareas pendientes del worker de gestión (función `SECURITY DEFINER`
/// `copropiedades_con_gestion_pendiente`: solo devuelve ids de tenant).
pub trait DirectorioGestion: Send + Sync {
    fn copropiedades_con_gestion_pendiente(
        &self,
    ) -> impl Future<Output = AppResult<Vec<TenantId>>> + Send;
}

/// Ticket de la cuenta en cualquiera de sus copropiedades (sin descripción ni mensajes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketDeCuenta {
    pub tenant_id: TenantId,
    pub copropiedad: String,
    pub ticket_id: TicketId,
    pub numero: NumeroTicket,
    pub tipo: TipoTicket,
    pub estado: EstadoTicket,
    pub asunto: String,
    pub creado_en: DateTime<Utc>,
    pub actualizado_en: DateTime<Utc>,
}

/// Tickets de una cuenta en todas sus copropiedades (transacción de plataforma, función
/// `SECURITY DEFINER` `tickets_de_cuenta`).
pub trait RepositorioTicketsDeCuenta: Send {
    fn tickets_de_cuenta(
        &mut self,
        cuenta: CuentaId,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<TicketDeCuenta>>> + Send;
}
