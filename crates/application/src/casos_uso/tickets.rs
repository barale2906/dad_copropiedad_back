//! Tickets (plan 07b, módulo vendible `gestion_administrativa`): radicación por web, WhatsApp,
//! correo o en persona; clasificación, asignación, estados, mensajes públicos y notas internas,
//! plazos en horas o días hábiles, alertas de vencimiento, cierre automático y encuesta.
//!
//! Invariantes:
//! 1. Las transiciones solo por [`EstadoTicket::transicionar`]; cada una deja un evento
//!    append-only.
//! 2. Radicar es idempotente por `idempotency_key` (el `wamid` en WhatsApp): un reintento
//!    devuelve el ticket original.
//! 3. El solicitante solo ve sus tickets y los mensajes públicos; nunca las notas internas.
//! 4. La identidad de quien reporta convivencia solo la ve quien tiene `tickets.convivencia.ver`.
//! 5. Mientras se espera al solicitante el plazo no corre; al responder se cuenta de nuevo
//!    (Ley 1755 de 2015, art. 17).

use chrono::{DateTime, Datelike, Utc};
use domain::{
    ActivoId, ArchivoId, BienPrivadoId, Calificacion, CanalTicket, CategoriaTicketId, EstadoTicket,
    MensajeTicketId, Modulo, NumeroTicket, Permiso, PersonaId, Plazo, PrioridadTicket, RecursoId,
    TicketId, TipoTicket,
    copropiedad::{texto_obligatorio, texto_opcional},
    reserva::Tz,
    ticket::vencimiento,
};

use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{
        avisos::{
            Aviso, PLANTILLA_REPORTE_URGENTE, PLANTILLA_TICKET_ACTUALIZADO, avisar_con_permiso,
            avisar_persona,
        },
        modulos::exigir_modulo_en,
        reservas::zona,
    },
    outbox::PrioridadSalida,
    puertos::{
        RegistroAuditoria, RegistroOutbox, Reloj, RepositorioCenso, RepositorioCopropiedad,
        RepositorioModulos, RepositorioPersonas, RepositorioRoles, RepositorioTickets, Transaccion,
        UnidadDeTrabajo,
        tickets::{
            AdjuntoTicket, CategoriaTicket, ConfiguracionTickets, Encuesta, EventoTicket,
            FiltroTickets, MensajeTicket, Responsable, SerieConsecutivo, TableroTickets, Ticket,
            VisibilidadTickets,
        },
    },
};

pub(crate) const ACCION_RADICAR: AccionAuditoria = AccionAuditoria::new("tickets.ticket.radicar");
pub(crate) const ACCION_CLASIFICAR: AccionAuditoria =
    AccionAuditoria::new("tickets.ticket.clasificar");
pub(crate) const ACCION_ASIGNAR: AccionAuditoria = AccionAuditoria::new("tickets.ticket.asignar");
pub(crate) const ACCION_TRANSICIONAR: AccionAuditoria =
    AccionAuditoria::new("tickets.ticket.transicionar");
pub(crate) const ACCION_COMENTAR: AccionAuditoria = AccionAuditoria::new("tickets.ticket.comentar");
pub(crate) const ACCION_CALIFICAR: AccionAuditoria =
    AccionAuditoria::new("tickets.ticket.calificar");
pub(crate) const ACCION_CONFIGURAR: AccionAuditoria =
    AccionAuditoria::new("tickets.configuracion.editar");
pub(crate) const ACCION_CATEGORIA: AccionAuditoria =
    AccionAuditoria::new("tickets.categoria.editar");

/// Puertos que usan los casos de uso de tickets dentro de la transacción de la copropiedad.
pub trait TxTickets:
    RepositorioTickets
    + RepositorioModulos
    + RepositorioCopropiedad
    + RepositorioPersonas
    + RepositorioCenso
    + RepositorioRoles
    + RegistroOutbox
    + RegistroAuditoria
{
}

impl<T> TxTickets for T where
    T: RepositorioTickets
        + RepositorioModulos
        + RepositorioCopropiedad
        + RepositorioPersonas
        + RepositorioCenso
        + RepositorioRoles
        + RegistroOutbox
        + RegistroAuditoria
{
}

async fn abrir<U>(unidad: &U, reloj: &impl Reloj, ctx: &Contexto) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::GestionAdministrativa).await?;
    Ok(tx)
}

fn tiene(ctx: &Contexto, p: Permiso) -> bool {
    ctx.exigir(p).is_ok()
}

fn texto_estado(e: EstadoTicket) -> String {
    e.as_str().replace('_', " ")
}

/// Tickets de la cuenta autenticada en todas sus copropiedades, los movidos más recientemente
/// primero («mis solicitudes» de la app).
///
/// # Errors
///
/// [`AppError::NoAutenticado`] sin cuenta; [`AppError::Validacion`] si `limite` no está entre 1
/// y 100.
pub async fn tickets_de_mi_cuenta<U>(
    unidad: &U,
    ctx: &crate::ContextoPlataforma,
    limite: u32,
) -> AppResult<Vec<crate::puertos::tickets::TicketDeCuenta>>
where
    U: crate::puertos::UnidadDeTrabajoPlataforma,
    U::Transaccion: crate::puertos::tickets::RepositorioTicketsDeCuenta,
{
    let cuenta = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    if !(1..=100).contains(&limite) {
        return Err(AppError::Validacion("`limite` va de 1 a 100".into()));
    }
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    crate::puertos::tickets::RepositorioTicketsDeCuenta::tickets_de_cuenta(&mut tx, cuenta, limite)
        .await
}

// ---------------------------------------------------------------------------------------------
// Configuración y categorías
// ---------------------------------------------------------------------------------------------

/// Configuración de tickets (jornada, reapertura y plazos).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tickets.configurar` ni `tickets.asignar`;
/// [`AppError::ModuloNoHabilitado`].
pub async fn ver_configuracion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<ConfiguracionTickets>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    if !tiene(ctx, Permiso::TicketsAsignar) {
        ctx.exigir(Permiso::TicketsConfigurar)?;
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.configuracion_tickets().await
}

/// Reemplaza la configuración de tickets. Los tickets ya radicados conservan su vencimiento.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tickets.configurar`; [`AppError::Dominio`] si la jornada es
/// inválida; [`AppError::Validacion`] si los días de reapertura no están entre 1 y 60.
pub async fn guardar_configuracion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    config: ConfiguracionTickets,
) -> AppResult<ConfiguracionTickets>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    ctx.exigir(Permiso::TicketsConfigurar)?;
    config.jornada.validar()?;
    if !(1..=60).contains(&config.dias_reapertura) {
        return Err(AppError::Validacion(
            "`dias_reapertura` debe estar entre 1 y 60".into(),
        ));
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.guardar_configuracion_tickets(&config).await?;
    let detalle = DetalleAuditoria::vacio().con("reglas_plazo", config.plazos.reglas().len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CONFIGURAR, "copropiedad", ctx.tenant_id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(config)
}

/// Categorías de tickets (las inactivas solo para quien configura).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tickets.crear`; [`AppError::ModuloNoHabilitado`].
pub async fn listar_categorias<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<CategoriaTicket>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    let configura = tiene(ctx, Permiso::TicketsConfigurar);
    if !configura {
        ctx.exigir(Permiso::TicketsCrear)?;
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    Ok(tx
        .categorias_ticket()
        .await?
        .into_iter()
        .filter(|c| configura || c.activa)
        .collect())
}

/// Crea (`id = None`) o edita una categoría.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tickets.configurar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si el nombre ya existe.
pub async fn guardar_categoria<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: Option<CategoriaTicketId>,
    nombre: String,
    activa: bool,
) -> AppResult<CategoriaTicket>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    ctx.exigir(Permiso::TicketsConfigurar)?;
    let nombre = texto_obligatorio("nombre", &nombre, 60)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let nueva = id.is_none();
    if let Some(id) = id
        && !tx.categorias_ticket().await?.iter().any(|c| c.id == id)
    {
        return Err(AppError::NoEncontrado("categoría"));
    }
    let categoria = CategoriaTicket {
        id: id.unwrap_or_else(CategoriaTicketId::nuevo),
        nombre,
        activa,
    };
    tx.guardar_categoria_ticket(&categoria, nueva).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_CATEGORIA,
        "categoria_ticket",
        categoria.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(categoria)
}

// ---------------------------------------------------------------------------------------------
// Radicación
// ---------------------------------------------------------------------------------------------

/// Datos de un ticket nuevo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoTicket {
    pub tipo: TipoTicket,
    pub categoria_id: Option<CategoriaTicketId>,
    /// `None` = media.
    pub prioridad: Option<PrioridadTicket>,
    pub canal: CanalTicket,
    pub asunto: String,
    pub descripcion: String,
    pub bien_privado_id: Option<BienPrivadoId>,
    pub ubicacion: Option<String>,
    pub recurso_id: Option<RecursoId>,
    pub activo_id: Option<ActivoId>,
    pub adjuntos: Vec<ArchivoId>,
    /// La genera el cliente (o es el `wamid` del mensaje de WhatsApp).
    pub idempotency_key: String,
    /// Persona por la que se radica (la administración que registra un caso presencial o por
    /// correo, o el chat de WhatsApp). `None` = la persona de la cuenta que radica.
    pub solicitante: Option<PersonaId>,
}

/// Máximo de adjuntos al radicar o en un mensaje.
pub const MAX_ADJUNTOS: usize = 10;

pub(crate) async fn calcular_vencimiento<T: TxTickets>(
    tx: &mut T,
    config: &ConfiguracionTickets,
    plazo: Option<Plazo>,
    desde: DateTime<Utc>,
) -> AppResult<Option<DateTime<Utc>>> {
    let Some(plazo) = plazo else { return Ok(None) };
    let tz = zona(tx).await?;
    Ok(Some(vencimiento(desde, plazo, &config.jornada, tz)?))
}

fn validar_clave(clave: &str) -> AppResult<String> {
    let clave = clave.trim();
    if clave.is_empty() || clave.len() > 200 {
        return Err(AppError::Validacion(
            "`idempotency_key` debe tener de 1 a 200 caracteres".into(),
        ));
    }
    Ok(clave.to_owned())
}

/// Radica un ticket. Si es urgente avisa de inmediato a quienes tienen
/// `tickets.urgencias.recibir`.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `tickets.crear` (o sin `tickets.asignar` para radicar por
///   otra persona); [`AppError::ModuloNoHabilitado`].
/// - [`AppError::Validacion`] si faltan datos, sobran adjuntos, la categoría está inactiva o la
///   cuenta no tiene vínculo con el bien privado.
/// - [`AppError::NoEncontrado`] si la persona, la categoría o un adjunto no existen.
pub async fn radicar_ticket<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: NuevoTicket,
) -> AppResult<Ticket>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    ctx.exigir(Permiso::TicketsCrear)?;
    let clave = validar_clave(&n.idempotency_key)?;
    let asunto = texto_obligatorio("asunto", &n.asunto, 120)?;
    let descripcion = texto_obligatorio("descripcion", &n.descripcion, 5000)?;
    let ubicacion = texto_opcional("ubicacion", n.ubicacion.as_deref(), 200)?;
    if n.adjuntos.len() > MAX_ADJUNTOS {
        return Err(AppError::Validacion(format!(
            "a lo sumo {MAX_ADJUNTOS} adjuntos"
        )));
    }
    let ahora = reloj.ahora();
    let gestiona = tiene(ctx, Permiso::TicketsAsignar);
    let mut tx = abrir(unidad, reloj, ctx).await?;
    if let Some(existente) = tx.ticket_por_clave(&clave).await? {
        return Ok(existente);
    }

    let (persona, cuenta) = match (n.solicitante, ctx.actor_id) {
        (Some(p), actor) => {
            if actor.is_some() && !gestiona {
                return Err(AppError::SinPermiso(Permiso::TicketsAsignar));
            }
            let persona = tx
                .persona(p)
                .await?
                .ok_or(AppError::NoEncontrado("persona"))?;
            (Some(p), persona.cuenta_id)
        }
        (None, Some(cuenta)) => (tx.persona_de_cuenta(cuenta).await?, Some(cuenta)),
        (None, None) => {
            return Err(AppError::Validacion(
                "indique la persona que radica el ticket".into(),
            ));
        }
    };
    if let (Some(bien), Some(cuenta), false, true) = (
        n.bien_privado_id,
        ctx.actor_id,
        gestiona,
        n.solicitante.is_none(),
    ) && !tx.cuenta_vinculada_a_bien(cuenta, bien).await?
    {
        return Err(AppError::Validacion(
            "no tiene vínculo con ese bien privado".into(),
        ));
    }
    if let Some(c) = n.categoria_id
        && !tx
            .categorias_ticket()
            .await?
            .iter()
            .any(|x| x.id == c && x.activa)
    {
        return Err(AppError::NoEncontrado("categoría"));
    }

    let config = tx.configuracion_tickets().await?;
    let prioridad = n.prioridad.unwrap_or(PrioridadTicket::Media);
    let plazo = config.plazos.plazo_para(n.tipo, prioridad);
    let vence_en = calcular_vencimiento(&mut tx, &config, plazo, ahora).await?;
    let tz = zona(&mut tx).await?;
    let anio = ahora.with_timezone(&tz).year();
    let consecutivo = tx
        .siguiente_consecutivo(SerieConsecutivo::Ticket, anio)
        .await?;
    let ticket = Ticket {
        id: TicketId::nuevo(),
        numero: NumeroTicket { anio, consecutivo },
        tipo: n.tipo,
        categoria_id: n.categoria_id,
        prioridad,
        canal: n.canal,
        estado: EstadoTicket::Abierto,
        asunto,
        descripcion,
        solicitante_persona_id: persona,
        solicitante_cuenta_id: cuenta,
        bien_privado_id: n.bien_privado_id,
        ubicacion,
        recurso_id: n.recurso_id,
        activo_id: n.activo_id,
        asignado: None,
        plazo,
        vence_en,
        alerta_vencimiento_en: None,
        resuelto_en: None,
        reabrible_hasta: None,
        cerrado_en: None,
        duplicado_de: None,
        confidencial: n.tipo.es_confidencial(),
        idempotency_key: clave,
        encuesta: None,
        creado_en: ahora,
        actualizado_en: ahora,
    };
    tx.insertar_ticket(&ticket).await?;
    for archivo_id in &n.adjuntos {
        tx.insertar_adjunto_ticket(
            ticket.id,
            &AdjuntoTicket {
                archivo_id: *archivo_id,
                mensaje_id: None,
                interno: false,
            },
        )
        .await?;
    }
    tx.insertar_evento_ticket(&EventoTicket {
        ticket_id: ticket.id,
        desde: None,
        hacia: EstadoTicket::Abierto,
        actor_cuenta_id: ctx.actor_id,
        motivo: None,
        ocurrido_en: ahora,
    })
    .await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", ticket.tipo.as_str())?
        .con("prioridad", ticket.prioridad.as_str())?
        .con("canal", ticket.canal.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RADICAR, "ticket", ticket.id.as_uuid()).con_detalle(detalle),
    )
    .await?;

    let numero = ticket.numero.to_string();
    if prioridad == PrioridadTicket::Urgente {
        let donde = ticket
            .ubicacion
            .clone()
            .unwrap_or_else(|| "sin ubicación".into());
        avisar_con_permiso(
            &mut tx,
            Permiso::TicketsUrgenciasRecibir,
            &Aviso {
                prioridad: PrioridadSalida::Confirmacion,
                asunto: format!("Reporte urgente {numero}"),
                texto: format!(
                    "Se radicó el reporte urgente {numero}: {}.\nUbicación: {donde}.",
                    ticket.asunto
                ),
                plantilla: Some((
                    PLANTILLA_REPORTE_URGENTE,
                    vec![numero.clone(), ticket.asunto.clone(), donde],
                )),
            },
        )
        .await?;
    }
    if ticket.canal != CanalTicket::Whatsapp {
        // En WhatsApp el chat responde en la conversación.
        avisar_persona(
            &mut tx,
            ticket.solicitante_persona_id,
            &Aviso {
                prioridad: PrioridadSalida::Confirmacion,
                asunto: format!("Radicamos su solicitud {numero}"),
                texto: format!(
                    "Recibimos su solicitud «{}» con el número {numero}.{}",
                    ticket.asunto,
                    ticket
                        .vence_en
                        .map(|v| format!(
                            " La respuesta está prevista a más tardar el {}.",
                            v.with_timezone(&tz).format("%Y-%m-%d %H:%M")
                        ))
                        .unwrap_or_default()
                ),
                plantilla: None,
            },
        )
        .await?;
    }
    tx.confirmar().await?;
    Ok(ticket)
}

// ---------------------------------------------------------------------------------------------
// Consulta
// ---------------------------------------------------------------------------------------------

async fn visibilidad<T: TxTickets>(
    tx: &mut T,
    ctx: &Contexto,
    ahora: DateTime<Utc>,
) -> AppResult<VisibilidadTickets> {
    let organizaciones = match ctx.actor_id {
        Some(c) if tiene(ctx, Permiso::TicketsAtender) => {
            tx.organizaciones_de_cuenta(c, ahora).await?
        }
        _ => Vec::new(),
    };
    Ok(VisibilidadTickets {
        todos: tiene(ctx, Permiso::TicketsVerTodos),
        convivencia: tiene(ctx, Permiso::TicketsConvivenciaVer),
        cuenta: ctx.actor_id,
        organizaciones,
    })
}

fn es_solicitante(ctx: &Contexto, t: &Ticket) -> bool {
    ctx.actor_id.is_some() && ctx.actor_id == t.solicitante_cuenta_id
}

fn es_responsable(v: &VisibilidadTickets, t: &Ticket) -> bool {
    match t.asignado {
        Some(Responsable::Cuenta(c)) => v.cuenta == Some(c),
        Some(Responsable::Organizacion(o)) => v.organizaciones.contains(&o),
        None => false,
    }
}

fn puede_ver(v: &VisibilidadTickets, ctx: &Contexto, t: &Ticket) -> bool {
    v.todos || (v.convivencia && t.confidencial) || es_solicitante(ctx, t) || es_responsable(v, t)
}

/// ¿Atiende el ticket (lo tiene asignado, o es un reporte de convivencia y tiene el permiso)?
fn atiende(ctx: &Contexto, v: &VisibilidadTickets, t: &Ticket) -> bool {
    (tiene(ctx, Permiso::TicketsAtender) && es_responsable(v, t))
        || (t.confidencial && v.convivencia)
}

/// Ve las notas internas: quien gestiona, atiende o supervisa todos los tickets.
fn ve_internos(ctx: &Contexto, v: &VisibilidadTickets, t: &Ticket) -> bool {
    tiene(ctx, Permiso::TicketsAsignar) || v.todos || atiende(ctx, v, t)
}

/// Ticket tal como lo ve quien consulta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketVista {
    pub ticket: Ticket,
    /// Se ocultó la identidad del solicitante (reporte de convivencia sin el permiso).
    pub identidad_oculta: bool,
    pub vencido: bool,
}

fn vista(
    ctx: &Contexto,
    v: &VisibilidadTickets,
    mut t: Ticket,
    ahora: DateTime<Utc>,
) -> TicketVista {
    let ocultar = t.confidencial && !v.convivencia && !es_solicitante(ctx, &t);
    if ocultar {
        t.solicitante_persona_id = None;
        t.solicitante_cuenta_id = None;
        t.bien_privado_id = None;
    }
    TicketVista {
        vencido: t.vencido(ahora),
        ticket: t,
        identidad_oculta: ocultar,
    }
}

/// Filtros del listado que elige quien consulta.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConsultaTickets {
    pub estado: Option<EstadoTicket>,
    pub tipo: Option<TipoTicket>,
    pub prioridad: Option<PrioridadTicket>,
    pub bien_privado_id: Option<BienPrivadoId>,
    pub solo_vencidos: bool,
    /// Solo los que radicó la cuenta.
    pub mios: bool,
}

/// Tickets visibles para quien consulta, más recientes primero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tickets.crear` ni otro permiso de tickets;
/// [`AppError::ModuloNoHabilitado`].
pub async fn listar_tickets<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    c: ConsultaTickets,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<TicketVista>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    let alguno = [
        Permiso::TicketsCrear,
        Permiso::TicketsVerTodos,
        Permiso::TicketsAtender,
        Permiso::TicketsConvivenciaVer,
    ]
    .iter()
    .any(|p| tiene(ctx, *p));
    if !alguno {
        return Err(AppError::SinPermiso(Permiso::TicketsCrear));
    }
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let v = visibilidad(&mut tx, ctx, ahora).await?;
    let filtro = FiltroTickets {
        estado: c.estado,
        tipo: c.tipo,
        prioridad: c.prioridad,
        solicitante_cuenta: if c.mios { ctx.actor_id } else { None },
        solicitante_persona: None,
        // Sin la identidad no se filtra por bien privado en los confidenciales (no se revela
        // quién reportó): el repositorio filtra por la columna y la vista la oculta.
        bien_privado_id: c.bien_privado_id,
        vencidos_a: c.solo_vencidos.then_some(ahora),
    };
    let pagina = tx.tickets(&v, &filtro, pagina).await?;
    Ok(Pagina {
        datos: pagina
            .datos
            .into_iter()
            .filter(|t| {
                // Filtrar por bien privado no debe permitir deducir al denunciante.
                c.bien_privado_id.is_none() || !t.confidencial || v.convivencia
            })
            .map(|t| vista(ctx, &v, t, ahora))
            .collect(),
        siguiente: pagina.siguiente,
    })
}

/// Tickets que radicó una persona (lo usa el chat de WhatsApp con el contexto del sistema).
///
/// # Errors
///
/// [`AppError::ModuloNoHabilitado`].
pub async fn tickets_de_persona<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    persona: PersonaId,
    limite: u32,
) -> AppResult<Vec<Ticket>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    ctx.exigir(Permiso::TicketsVerTodos)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let v = VisibilidadTickets {
        todos: true,
        ..VisibilidadTickets::default()
    };
    let filtro = FiltroTickets {
        solicitante_persona: Some(persona),
        ..FiltroTickets::default()
    };
    Ok(tx
        .tickets(&v, &filtro, SolicitudPagina::new(Some(limite), None)?)
        .await?
        .datos)
}

/// Mensaje tal como lo ve quien consulta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MensajeVista {
    pub mensaje: MensajeTicket,
    /// Lo escribió el solicitante (útil cuando su identidad está oculta).
    pub del_solicitante: bool,
}

/// Ticket con su conversación, adjuntos e historial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetalleTicket {
    pub ticket: TicketVista,
    pub mensajes: Vec<MensajeVista>,
    pub adjuntos: Vec<AdjuntoTicket>,
    pub eventos: Vec<EventoTicket>,
}

async fn cargar_visible<T: TxTickets>(
    tx: &mut T,
    ctx: &Contexto,
    id: TicketId,
    ahora: DateTime<Utc>,
) -> AppResult<(Ticket, VisibilidadTickets)> {
    let v = visibilidad(tx, ctx, ahora).await?;
    let t = tx
        .ticket(id)
        .await?
        .filter(|t| puede_ver(&v, ctx, t))
        // Un ticket ajeno "no existe" para quien no puede verlo.
        .ok_or(AppError::NoEncontrado("ticket"))?;
    Ok((t, v))
}

/// Un ticket con sus mensajes (sin notas internas para el solicitante), adjuntos e historial.
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si no existe o no es visible para la cuenta.
pub async fn ver_ticket<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: TicketId,
) -> AppResult<DetalleTicket>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let (t, v) = cargar_visible(&mut tx, ctx, id, ahora).await?;
    let internos = ve_internos(ctx, &v, &t);
    let vista_t = vista(ctx, &v, t.clone(), ahora);
    let ocultar = vista_t.identidad_oculta;
    let del_solicitante = |cuenta: Option<domain::CuentaId>, persona: Option<PersonaId>| {
        (cuenta.is_some() && cuenta == t.solicitante_cuenta_id)
            || (persona.is_some() && persona == t.solicitante_persona_id)
    };
    let mensajes = tx
        .mensajes_ticket(id, internos)
        .await?
        .into_iter()
        .map(|mut m| {
            let suyo = del_solicitante(m.autor_cuenta_id, m.autor_persona_id);
            if suyo && ocultar {
                m.autor_cuenta_id = None;
                m.autor_persona_id = None;
            }
            MensajeVista {
                mensaje: m,
                del_solicitante: suyo,
            }
        })
        .collect();
    let eventos = tx
        .eventos_ticket(id)
        .await?
        .into_iter()
        .map(|mut e| {
            if ocultar && del_solicitante(e.actor_cuenta_id, None) {
                e.actor_cuenta_id = None;
            }
            e
        })
        .collect();
    Ok(DetalleTicket {
        ticket: vista_t,
        mensajes,
        adjuntos: tx.adjuntos_ticket(id, internos).await?,
        eventos,
    })
}

// ---------------------------------------------------------------------------------------------
// Gestión
// ---------------------------------------------------------------------------------------------

/// Cambios de clasificación (`None` = sin cambio).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Clasificacion {
    pub tipo: Option<TipoTicket>,
    pub categoria_id: Option<CategoriaTicketId>,
    pub prioridad: Option<PrioridadTicket>,
}

/// Reclasifica un ticket. Si cambia el tipo o la prioridad, el vencimiento se recalcula desde
/// la radicación con el plazo nuevo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tickets.asignar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si el ticket ya terminó.
pub async fn clasificar_ticket<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: TicketId,
    c: Clasificacion,
) -> AppResult<TicketVista>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    ctx.exigir(Permiso::TicketsAsignar)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let (mut t, v) = cargar_visible(&mut tx, ctx, id, ahora).await?;
    if !t.estado.pendiente() {
        return Err(AppError::Conflicto("el ticket ya no está pendiente".into()));
    }
    if let Some(cat) = c.categoria_id
        && !tx
            .categorias_ticket()
            .await?
            .iter()
            .any(|x| x.id == cat && x.activa)
    {
        return Err(AppError::NoEncontrado("categoría"));
    }
    let cambia_plazo =
        c.tipo.is_some_and(|x| x != t.tipo) || c.prioridad.is_some_and(|x| x != t.prioridad);
    if let Some(tipo) = c.tipo {
        if tipo.es_confidencial() != t.confidencial {
            return Err(AppError::Validacion(
                "un reporte de convivencia no cambia de tipo (protege al denunciante)".into(),
            ));
        }
        t.tipo = tipo;
    }
    if let Some(p) = c.prioridad {
        t.prioridad = p;
    }
    if c.categoria_id.is_some() {
        t.categoria_id = c.categoria_id;
    }
    if cambia_plazo {
        let config = tx.configuracion_tickets().await?;
        t.plazo = config.plazos.plazo_para(t.tipo, t.prioridad);
        t.vence_en = calcular_vencimiento(&mut tx, &config, t.plazo, t.creado_en).await?;
        t.alerta_vencimiento_en = None;
    }
    t.actualizado_en = ahora;
    tx.actualizar_ticket(&t).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", t.tipo.as_str())?
        .con("prioridad", t.prioridad.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CLASIFICAR, "ticket", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(vista(ctx, &v, t, ahora))
}

pub(crate) async fn registrar_transicion<T: TxTickets>(
    tx: &mut T,
    ctx: &Contexto,
    t: &mut Ticket,
    hacia: EstadoTicket,
    motivo: Option<String>,
    ahora: DateTime<Utc>,
) -> AppResult<()> {
    let desde = t.estado;
    t.estado = desde.transicionar(hacia)?;
    let config = tx.configuracion_tickets().await?;
    match hacia {
        EstadoTicket::Resuelto => {
            t.resuelto_en = Some(ahora);
            let tz: Tz = zona(tx).await?;
            t.reabrible_hasta = Some(vencimiento(
                ahora,
                Plazo::new(config.dias_reapertura, domain::UnidadPlazo::DiasHabiles)?,
                &config.jornada,
                tz,
            )?);
        }
        EstadoTicket::Reabierto => {
            t.resuelto_en = None;
            t.reabrible_hasta = None;
        }
        EstadoTicket::Cerrado | EstadoTicket::Rechazado | EstadoTicket::Duplicado => {
            t.cerrado_en = Some(ahora);
            t.reabrible_hasta = None;
        }
        _ => {}
    }
    if desde == EstadoTicket::EnEsperaSolicitante || hacia == EstadoTicket::Reabierto {
        // Ley 1755, art. 17: al completar la información el término se cuenta de nuevo.
        t.vence_en = calcular_vencimiento(tx, &config, t.plazo, ahora).await?;
        t.alerta_vencimiento_en = None;
    }
    t.actualizado_en = ahora;
    tx.actualizar_ticket(t).await?;
    tx.insertar_evento_ticket(&EventoTicket {
        ticket_id: t.id,
        desde: Some(desde),
        hacia,
        actor_cuenta_id: ctx.actor_id,
        motivo,
        ocurrido_en: ahora,
    })
    .await
}

pub(crate) fn aviso_al_solicitante(t: &Ticket, detalle: &str) -> Aviso {
    let numero = t.numero.to_string();
    let estado = texto_estado(t.estado);
    Aviso {
        prioridad: PrioridadSalida::Evento,
        asunto: format!("Su solicitud {numero} está {estado}"),
        texto: format!(
            "Su solicitud «{}» ({numero}) está {estado}.\n{detalle}",
            t.asunto
        ),
        plantilla: (t.canal == CanalTicket::Whatsapp).then(|| {
            (
                PLANTILLA_TICKET_ACTUALIZADO,
                vec![numero, estado, detalle.to_owned()],
            )
        }),
    }
}

/// Asigna el ticket a una cuenta (que debe poder atender tickets) o a una organización con
/// contrato vigente. Desde `abierto`, `en_revision` o `reabierto` pasa a `asignado`.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tickets.asignar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si el responsable no puede atender; [`AppError::Dominio`] si el
/// estado no admite asignación.
pub async fn asignar_ticket<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: TicketId,
    responsable: Responsable,
) -> AppResult<TicketVista>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    ctx.exigir(Permiso::TicketsAsignar)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let (mut t, v) = cargar_visible(&mut tx, ctx, id, ahora).await?;
    if !t.estado.admite_asignacion() {
        return Err(AppError::Dominio(
            domain::DominioError::TransicionInvalida {
                entidad: "ticket",
                desde: t.estado.as_str(),
                hacia: EstadoTicket::Asignado.as_str(),
            },
        ));
    }
    match responsable {
        Responsable::Cuenta(c) => {
            if !tx
                .permisos_de_cuenta(c, ahora)
                .await?
                .contiene(Permiso::TicketsAtender)
            {
                return Err(AppError::Validacion(
                    "la cuenta no tiene el permiso `tickets.atender` en la copropiedad".into(),
                ));
            }
        }
        Responsable::Organizacion(o) => {
            if !tx.organizacion_con_contrato(o, ahora).await? {
                return Err(AppError::Validacion(
                    "la organización no tiene un contrato de servicio vigente".into(),
                ));
            }
        }
    }
    t.asignado = Some(responsable);
    if matches!(
        t.estado,
        EstadoTicket::Abierto | EstadoTicket::EnRevision | EstadoTicket::Reabierto
    ) {
        registrar_transicion(&mut tx, ctx, &mut t, EstadoTicket::Asignado, None, ahora).await?;
    } else {
        t.actualizado_en = ahora;
        tx.actualizar_ticket(&t).await?;
    }
    let detalle = match responsable {
        Responsable::Cuenta(c) => DetalleAuditoria::vacio().con("cuenta_id", c.to_string())?,
        Responsable::Organizacion(o) => {
            DetalleAuditoria::vacio().con("organizacion_id", o.to_string())?
        }
    };
    tx.registrar(EventoAuditoria::ok(ACCION_ASIGNAR, "ticket", id.as_uuid()).con_detalle(detalle))
        .await?;
    if let Responsable::Cuenta(c) = responsable {
        let numero = t.numero.to_string();
        for d in tx
            .destinatarios_con_permiso(Permiso::TicketsAtender)
            .await?
            .into_iter()
            .filter(|d| d.cuenta_id == c)
        {
            if let Some(correo) = d.correo {
                tx.encolar(crate::outbox::EventoSaliente {
                    prioridad: PrioridadSalida::Evento,
                    mensaje: crate::outbox::MensajeOutbox::Correo(
                        crate::puertos::correo::CorreoSaliente {
                            para: correo,
                            asunto: format!("Se le asignó el ticket {numero}"),
                            texto: format!(
                                "Se le asignó el ticket {numero} ({}, prioridad {}).",
                                t.tipo.as_str().replace('_', " "),
                                t.prioridad.as_str()
                            ),
                            html: None,
                        },
                    ),
                })
                .await?;
            }
        }
    }
    tx.confirmar().await?;
    Ok(vista(ctx, &v, t, ahora))
}

/// Cambio de estado pedido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CambioEstadoTicket {
    pub hacia: EstadoTicket,
    /// Obligatorio al resolver (es la respuesta al solicitante, que queda como mensaje
    /// público), rechazar o pedir información al solicitante.
    pub motivo: Option<String>,
    /// Obligatorio al marcar `duplicado`.
    pub duplicado_de: Option<TicketId>,
}

/// Cambia el estado de un ticket según el diagrama y el papel de quien lo pide:
///
/// - `en_revision`, `rechazado`, `duplicado`: quien gestiona (`tickets.asignar`).
/// - `en_progreso`, `en_espera_*`, `resuelto`: quien gestiona o quien lo atiende.
/// - `cerrado`: quien gestiona o el solicitante (acepta la solución).
/// - `reabierto`: el solicitante dentro del plazo de reapertura, o quien gestiona.
/// - `asignado`: con [`asignar_ticket`].
///
/// # Errors
///
/// [`AppError::SinPermiso`] si su papel no lo permite; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si la transición no está en el diagrama; [`AppError::Validacion`]
/// si falta el motivo o el ticket original; [`AppError::Conflicto`] si venció el plazo de
/// reapertura.
pub async fn transicionar_ticket<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: TicketId,
    cambio: CambioEstadoTicket,
) -> AppResult<TicketVista>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    let motivo = texto_opcional("motivo", cambio.motivo.as_deref(), 5000)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let (mut t, v) = cargar_visible(&mut tx, ctx, id, ahora).await?;
    let gestiona = tiene(ctx, Permiso::TicketsAsignar);
    let solicitante = es_solicitante(ctx, &t);
    use EstadoTicket::*;
    let permitido = match cambio.hacia {
        EnRevision | Rechazado | Duplicado => gestiona,
        EnProgreso | EnEsperaTercero | EnEsperaSolicitante | Resuelto => {
            gestiona || atiende(ctx, &v, &t)
        }
        Cerrado | Reabierto => gestiona || solicitante,
        Asignado => {
            return Err(AppError::Validacion(
                "para asignar use la operación de asignación".into(),
            ));
        }
        Abierto => false,
    };
    if !permitido {
        return Err(AppError::SinPermiso(if solicitante {
            Permiso::TicketsAsignar
        } else {
            Permiso::TicketsAtender
        }));
    }
    let exige_motivo = matches!(cambio.hacia, Resuelto | Rechazado | EnEsperaSolicitante);
    if exige_motivo && motivo.is_none() {
        return Err(AppError::Validacion(
            "indique el `motivo` (la respuesta o lo que se necesita del solicitante)".into(),
        ));
    }
    if cambio.hacia == Duplicado {
        let original = cambio.duplicado_de.ok_or_else(|| {
            AppError::Validacion("indique `duplicado_de` (el ticket original)".into())
        })?;
        if original == id || tx.ticket(original).await?.is_none() {
            return Err(AppError::NoEncontrado("ticket original"));
        }
        t.duplicado_de = Some(original);
    }
    if cambio.hacia == Reabierto && !gestiona && t.reabrible_hasta.is_none_or(|h| ahora > h) {
        return Err(AppError::Conflicto(
            "venció el plazo para reabrir; radique una nueva solicitud".into(),
        ));
    }
    let desde = t.estado;
    registrar_transicion(&mut tx, ctx, &mut t, cambio.hacia, motivo.clone(), ahora).await?;
    if cambio.hacia == Resuelto
        && let Some(respuesta) = &motivo
    {
        tx.insertar_mensaje_ticket(&MensajeTicket {
            id: MensajeTicketId::nuevo(),
            ticket_id: id,
            autor_cuenta_id: ctx.actor_id,
            autor_persona_id: None,
            canal: CanalTicket::Web,
            interno: false,
            texto: respuesta.clone(),
            adjuntos: Vec::new(),
            creado_en: ahora,
        })
        .await?;
    }
    let detalle = DetalleAuditoria::vacio()
        .con("desde", desde.as_str())?
        .con("hacia", cambio.hacia.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_TRANSICIONAR, "ticket", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    let avisar = matches!(
        cambio.hacia,
        Resuelto | EnEsperaSolicitante | Rechazado | Duplicado
    ) || (cambio.hacia == Cerrado && !solicitante);
    if avisar && !solicitante {
        let detalle = motivo.unwrap_or_default();
        let aviso = aviso_al_solicitante(&t, &detalle);
        avisar_persona(&mut tx, t.solicitante_persona_id, &aviso).await?;
    }
    if cambio.hacia == Reabierto && solicitante {
        avisar_con_permiso(
            &mut tx,
            Permiso::TicketsAsignar,
            &Aviso {
                prioridad: PrioridadSalida::Evento,
                asunto: format!("Se reabrió el ticket {}", t.numero),
                texto: format!(
                    "El solicitante reabrió el ticket {} («{}»).",
                    t.numero, t.asunto
                ),
                plantilla: None,
            },
        )
        .await?;
    }
    tx.confirmar().await?;
    Ok(vista(ctx, &v, t, ahora))
}

/// Mensaje nuevo en un ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoMensaje {
    pub texto: String,
    /// Nota interna (solo la administración y quien atiende).
    pub interno: bool,
    pub adjuntos: Vec<ArchivoId>,
    pub canal: CanalTicket,
}

/// Agrega un mensaje. El solicitante solo escribe mensajes públicos; si el ticket esperaba su
/// respuesta, vuelve a `en_progreso` y el plazo se cuenta de nuevo. Una respuesta pública de la
/// administración le llega al solicitante.
///
/// # Errors
///
/// [`AppError::NoEncontrado`]; [`AppError::SinPermiso`] si no es el solicitante ni atiende el
/// ticket (o si el solicitante intenta una nota interna); [`AppError::Conflicto`] si el ticket
/// terminó.
pub async fn comentar_ticket<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: TicketId,
    n: NuevoMensaje,
) -> AppResult<MensajeTicket>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    let texto = texto_obligatorio("texto", &n.texto, 5000)?;
    if n.adjuntos.len() > MAX_ADJUNTOS {
        return Err(AppError::Validacion(format!(
            "a lo sumo {MAX_ADJUNTOS} adjuntos"
        )));
    }
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let (mut t, v) = cargar_visible(&mut tx, ctx, id, ahora).await?;
    if t.estado.es_final() {
        return Err(AppError::Conflicto("el ticket ya terminó".into()));
    }
    let solicitante = es_solicitante(ctx, &t);
    let personal = tiene(ctx, Permiso::TicketsAsignar) || atiende(ctx, &v, &t);
    if !(solicitante || personal) || (n.interno && !personal) {
        return Err(AppError::SinPermiso(Permiso::TicketsAtender));
    }
    let mensaje = MensajeTicket {
        id: MensajeTicketId::nuevo(),
        ticket_id: id,
        autor_cuenta_id: ctx.actor_id,
        autor_persona_id: if solicitante {
            t.solicitante_persona_id
        } else {
            None
        },
        canal: n.canal,
        interno: n.interno,
        texto,
        adjuntos: n.adjuntos.clone(),
        creado_en: ahora,
    };
    tx.insertar_mensaje_ticket(&mensaje).await?;
    for archivo_id in &n.adjuntos {
        tx.insertar_adjunto_ticket(
            id,
            &AdjuntoTicket {
                archivo_id: *archivo_id,
                mensaje_id: Some(mensaje.id),
                interno: n.interno,
            },
        )
        .await?;
    }
    if solicitante && t.estado == EstadoTicket::EnEsperaSolicitante {
        registrar_transicion(&mut tx, ctx, &mut t, EstadoTicket::EnProgreso, None, ahora).await?;
    }
    let detalle = DetalleAuditoria::vacio().con("interno", n.interno)?;
    tx.registrar(EventoAuditoria::ok(ACCION_COMENTAR, "ticket", id.as_uuid()).con_detalle(detalle))
        .await?;
    if personal && !solicitante && !n.interno {
        let aviso = aviso_al_solicitante(&t, &mensaje.texto);
        avisar_persona(&mut tx, t.solicitante_persona_id, &aviso).await?;
    }
    tx.confirmar().await?;
    Ok(mensaje)
}

/// Encuesta de satisfacción del solicitante (una vez, con el ticket resuelto o cerrado).
///
/// # Errors
///
/// [`AppError::NoEncontrado`]; [`AppError::SinPermiso`] si no es el solicitante;
/// [`AppError::Conflicto`] si aún no está resuelto o ya calificó.
pub async fn calificar_ticket<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: TicketId,
    calificacion: Calificacion,
    comentario: Option<String>,
) -> AppResult<Ticket>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    let comentario = texto_opcional("comentario", comentario.as_deref(), 1000)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let (mut t, _) = cargar_visible(&mut tx, ctx, id, ahora).await?;
    if !es_solicitante(ctx, &t) {
        return Err(AppError::SinPermiso(Permiso::TicketsCrear));
    }
    if !matches!(t.estado, EstadoTicket::Resuelto | EstadoTicket::Cerrado) {
        return Err(AppError::Conflicto(
            "la encuesta se responde con el ticket resuelto".into(),
        ));
    }
    if t.encuesta.is_some() {
        return Err(AppError::Conflicto("la encuesta ya fue respondida".into()));
    }
    t.encuesta = Some(Encuesta {
        calificacion,
        comentario,
    });
    t.actualizado_en = ahora;
    tx.actualizar_ticket(&t).await?;
    let detalle = DetalleAuditoria::vacio().con("calificacion", calificacion.valor())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CALIFICAR, "ticket", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(t)
}

/// Indicadores del tablero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tickets.ver_todos`; [`AppError::ModuloNoHabilitado`].
pub async fn tablero_tickets<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<TableroTickets>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    ctx.exigir(Permiso::TicketsVerTodos)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.tablero_tickets(reloj.ahora()).await
}

// ---------------------------------------------------------------------------------------------
// Tareas programadas (worker de gestión del gateway, contexto del sistema)
// ---------------------------------------------------------------------------------------------

/// Resultado de las tareas de tickets de una copropiedad.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumenTareasTickets {
    pub cerrados: u32,
    pub alertados: u32,
}

/// Cierra los tickets resueltos cuyo plazo de reapertura pasó (invitando a la encuesta) y alerta
/// los vencidos al responsable y a quien gestiona (escalamiento).
///
/// # Errors
///
/// Errores de persistencia; [`AppError::ModuloNoHabilitado`] si la PH ya no tiene el módulo.
pub async fn tareas_tickets<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<ResumenTareasTickets>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTickets,
{
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut resumen = ResumenTareasTickets::default();
    for mut t in tx.tickets_por_cerrar(ahora).await? {
        registrar_transicion(
            &mut tx,
            ctx,
            &mut t,
            EstadoTicket::Cerrado,
            Some("cierre automático: venció el plazo de reapertura".into()),
            ahora,
        )
        .await?;
        if t.encuesta.is_none() {
            let aviso = aviso_al_solicitante(
                &t,
                "Cuéntenos cómo lo atendimos: califique la atención en la plataforma.",
            );
            avisar_persona(&mut tx, t.solicitante_persona_id, &aviso).await?;
        }
        resumen.cerrados += 1;
    }
    for mut t in tx.tickets_vencidos_sin_alerta(ahora).await? {
        t.alerta_vencimiento_en = Some(ahora);
        tx.actualizar_ticket(&t).await?;
        avisar_con_permiso(
            &mut tx,
            Permiso::TicketsAsignar,
            &Aviso {
                prioridad: PrioridadSalida::Evento,
                asunto: format!("Ticket vencido {}", t.numero),
                texto: format!(
                    "El ticket {} («{}», prioridad {}) superó su plazo de respuesta y sigue {}.",
                    t.numero,
                    t.asunto,
                    t.prioridad.as_str(),
                    texto_estado(t.estado)
                ),
                plantilla: None,
            },
        )
        .await?;
        resumen.alertados += 1;
    }
    if resumen != ResumenTareasTickets::default() {
        tx.confirmar().await?;
    }
    Ok(resumen)
}

#[cfg(test)]
mod tests;
