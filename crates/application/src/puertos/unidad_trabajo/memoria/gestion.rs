//! Dobles en memoria de los repositorios de tickets, mantenimiento y chat (planes 07b y 07c).

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ActivoId, ConversacionId, CuentaId, EstadoOrden, EstadoTicket, ObligacionId, OrdenTrabajoId,
    OrganizacionId, Permiso, PersonaId, PlanMantenimientoId, PreguntaFrecuenteId, TicketId,
};
use rust_decimal::Decimal;

use super::TransaccionEnMemoria;
use crate::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::{
        RepositorioChat, RepositorioMantenimiento, RepositorioTickets,
        chat::{Conversacion, EstadoConversacion, MensajeConversacion, PreguntaFrecuente},
        mantenimiento::{
            Activo, ArchivoActivo, Cumplimiento, EvidenciaOrden, FiltroActivos, FiltroOrdenes,
            Movimiento, Obligacion, OrdenTrabajo, PlanMantenimiento, TableroMantenimiento,
        },
        tickets::{
            AdjuntoTicket, CategoriaTicket, ConfiguracionTickets, Destinatario, EventoTicket,
            FiltroTickets, MensajeTicket, Responsable, SerieConsecutivo, TableroTickets, Ticket,
            VisibilidadTickets,
        },
    },
};

/// Datos de gestión administrativa en memoria.
#[derive(Default, Clone)]
pub struct DatosGestion {
    pub consecutivos: Vec<(&'static str, i32, u32)>,
    pub configuracion: Option<ConfiguracionTickets>,
    pub categorias: Vec<CategoriaTicket>,
    pub tickets: Vec<Ticket>,
    pub mensajes: Vec<MensajeTicket>,
    pub adjuntos: Vec<(TicketId, AdjuntoTicket)>,
    pub eventos: Vec<EventoTicket>,
    /// Permisos de los destinatarios de avisos.
    pub destinatarios: Vec<(Permiso, Destinatario)>,
    pub linea: Option<String>,
    /// Organizaciones para las que trabaja cada cuenta.
    pub organizaciones_de: Vec<(CuentaId, OrganizacionId)>,
    pub con_contrato: Vec<OrganizacionId>,
    pub activos: Vec<Activo>,
    pub archivos_activo: Vec<(ActivoId, ArchivoActivo)>,
    pub movimientos: Vec<Movimiento>,
    pub planes: Vec<PlanMantenimiento>,
    pub ordenes: Vec<OrdenTrabajo>,
    pub evidencias: Vec<(OrdenTrabajoId, EvidenciaOrden)>,
    pub obligaciones: Vec<Obligacion>,
    pub cumplimientos: Vec<Cumplimiento>,
    pub preguntas: Vec<PreguntaFrecuente>,
    pub conversaciones: Vec<Conversacion>,
    pub mensajes_conversacion: Vec<MensajeConversacion>,
}

fn pagina_desc<T: Clone>(
    mut v: Vec<T>,
    p: SolicitudPagina,
    id: impl Fn(&T) -> uuid::Uuid,
) -> Pagina<T> {
    v.sort_by_key(|x| std::cmp::Reverse(id(x)));
    let v: Vec<T> = v
        .into_iter()
        .filter(|x| p.despues().is_none_or(|d| id(x) < d))
        .take(usize::try_from(p.filas_a_consultar()).unwrap_or(usize::MAX))
        .collect();
    Pagina::desde_filas(v, p, id)
}

fn visible(v: &VisibilidadTickets, t: &Ticket) -> bool {
    v.todos
        || (v.convivencia && t.confidencial)
        || (v.cuenta.is_some() && t.solicitante_cuenta_id == v.cuenta)
        || matches!(t.asignado, Some(Responsable::Cuenta(c)) if Some(c) == v.cuenta)
        || matches!(t.asignado, Some(Responsable::Organizacion(o)) if v.organizaciones.contains(&o))
}

impl RepositorioTickets for TransaccionEnMemoria {
    async fn siguiente_consecutivo(
        &mut self,
        serie: SerieConsecutivo,
        anio: i32,
    ) -> AppResult<u32> {
        let c = &mut self.datos.gestion.consecutivos;
        match c
            .iter_mut()
            .find(|(s, a, _)| *s == serie.as_str() && *a == anio)
        {
            Some((_, _, n)) => {
                *n += 1;
                Ok(*n)
            }
            None => {
                c.push((serie.as_str(), anio, 1));
                Ok(1)
            }
        }
    }

    async fn configuracion_tickets(&mut self) -> AppResult<ConfiguracionTickets> {
        Ok(self
            .datos
            .gestion
            .configuracion
            .clone()
            .unwrap_or_else(ConfiguracionTickets::predeterminada))
    }

    async fn guardar_configuracion_tickets(&mut self, c: &ConfiguracionTickets) -> AppResult<()> {
        self.datos.gestion.configuracion = Some(c.clone());
        Ok(())
    }

    async fn categorias_ticket(&mut self) -> AppResult<Vec<CategoriaTicket>> {
        Ok(self.datos.gestion.categorias.clone())
    }

    async fn guardar_categoria_ticket(
        &mut self,
        c: &CategoriaTicket,
        nueva: bool,
    ) -> AppResult<()> {
        let cats = &mut self.datos.gestion.categorias;
        if cats
            .iter()
            .any(|x| x.id != c.id && x.nombre.to_lowercase() == c.nombre.to_lowercase())
        {
            return Err(AppError::Conflicto("categoría repetida".into()));
        }
        if nueva {
            cats.push(c.clone());
        } else if let Some(x) = cats.iter_mut().find(|x| x.id == c.id) {
            *x = c.clone();
        }
        Ok(())
    }

    async fn ticket(&mut self, id: TicketId) -> AppResult<Option<Ticket>> {
        Ok(self
            .datos
            .gestion
            .tickets
            .iter()
            .find(|t| t.id == id)
            .cloned())
    }

    async fn ticket_por_clave(&mut self, clave: &str) -> AppResult<Option<Ticket>> {
        Ok(self
            .datos
            .gestion
            .tickets
            .iter()
            .find(|t| t.idempotency_key == clave)
            .cloned())
    }

    async fn insertar_ticket(&mut self, t: &Ticket) -> AppResult<()> {
        if self
            .datos
            .gestion
            .tickets
            .iter()
            .any(|x| x.idempotency_key == t.idempotency_key)
        {
            return Err(AppError::Conflicto("clave repetida".into()));
        }
        self.datos.gestion.tickets.push(t.clone());
        Ok(())
    }

    async fn actualizar_ticket(&mut self, t: &Ticket) -> AppResult<()> {
        if let Some(x) = self.datos.gestion.tickets.iter_mut().find(|x| x.id == t.id) {
            *x = t.clone();
        }
        Ok(())
    }

    async fn tickets(
        &mut self,
        v: &VisibilidadTickets,
        f: &FiltroTickets,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<Ticket>> {
        let filtrados = self
            .datos
            .gestion
            .tickets
            .iter()
            .filter(|t| visible(v, t))
            .filter(|t| f.estado.is_none_or(|e| t.estado == e))
            .filter(|t| f.tipo.is_none_or(|e| t.tipo == e))
            .filter(|t| f.prioridad.is_none_or(|e| t.prioridad == e))
            .filter(|t| {
                f.solicitante_cuenta
                    .is_none_or(|c| t.solicitante_cuenta_id == Some(c))
            })
            .filter(|t| {
                f.solicitante_persona
                    .is_none_or(|p| t.solicitante_persona_id == Some(p))
            })
            .filter(|t| {
                f.bien_privado_id
                    .is_none_or(|b| t.bien_privado_id == Some(b))
            })
            .filter(|t| f.vencidos_a.is_none_or(|a| t.vencido(a)))
            .cloned()
            .collect();
        Ok(pagina_desc(filtrados, pagina, |t| t.id.as_uuid()))
    }

    async fn insertar_mensaje_ticket(&mut self, m: &MensajeTicket) -> AppResult<()> {
        self.datos.gestion.mensajes.push(m.clone());
        Ok(())
    }

    async fn mensajes_ticket(
        &mut self,
        ticket: TicketId,
        incluir_internos: bool,
    ) -> AppResult<Vec<MensajeTicket>> {
        Ok(self
            .datos
            .gestion
            .mensajes
            .iter()
            .filter(|m| m.ticket_id == ticket && (incluir_internos || !m.interno))
            .cloned()
            .collect())
    }

    async fn insertar_adjunto_ticket(
        &mut self,
        ticket: TicketId,
        a: &AdjuntoTicket,
    ) -> AppResult<()> {
        self.datos.gestion.adjuntos.push((ticket, a.clone()));
        Ok(())
    }

    async fn adjuntos_ticket(
        &mut self,
        ticket: TicketId,
        incluir_internos: bool,
    ) -> AppResult<Vec<AdjuntoTicket>> {
        Ok(self
            .datos
            .gestion
            .adjuntos
            .iter()
            .filter(|(t, a)| *t == ticket && (incluir_internos || !a.interno))
            .map(|(_, a)| a.clone())
            .collect())
    }

    async fn insertar_evento_ticket(&mut self, e: &EventoTicket) -> AppResult<()> {
        self.datos.gestion.eventos.push(e.clone());
        Ok(())
    }

    async fn eventos_ticket(&mut self, ticket: TicketId) -> AppResult<Vec<EventoTicket>> {
        Ok(self
            .datos
            .gestion
            .eventos
            .iter()
            .filter(|e| e.ticket_id == ticket)
            .cloned()
            .collect())
    }

    async fn tickets_por_cerrar(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<Ticket>> {
        Ok(self
            .datos
            .gestion
            .tickets
            .iter()
            .filter(|t| {
                t.estado == EstadoTicket::Resuelto && t.reabrible_hasta.is_some_and(|h| h <= ahora)
            })
            .cloned()
            .collect())
    }

    async fn tickets_vencidos_sin_alerta(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<Ticket>> {
        Ok(self
            .datos
            .gestion
            .tickets
            .iter()
            .filter(|t| t.vencido(ahora) && t.alerta_vencimiento_en.is_none())
            .cloned()
            .collect())
    }

    async fn tablero_tickets(&mut self, ahora: DateTime<Utc>) -> AppResult<TableroTickets> {
        let t = &self.datos.gestion.tickets;
        let mut por_estado: Vec<(EstadoTicket, u64)> = Vec::new();
        for x in t.iter().filter(|x| x.estado.pendiente()) {
            match por_estado.iter_mut().find(|(e, _)| *e == x.estado) {
                Some((_, n)) => *n += 1,
                None => por_estado.push((x.estado, 1)),
            }
        }
        Ok(TableroTickets {
            por_estado,
            vencidos: t.iter().filter(|x| x.vencido(ahora)).count() as u64,
            ..TableroTickets::default()
        })
    }

    async fn persona_de_cuenta(&mut self, cuenta: CuentaId) -> AppResult<Option<PersonaId>> {
        Ok(self
            .datos
            .personas
            .iter()
            .find(|p| p.cuenta_id == Some(cuenta))
            .map(|p| p.id))
    }

    async fn organizaciones_de_cuenta(
        &mut self,
        cuenta: CuentaId,
        _ahora: DateTime<Utc>,
    ) -> AppResult<Vec<OrganizacionId>> {
        Ok(self
            .datos
            .gestion
            .organizaciones_de
            .iter()
            .filter(|(c, _)| *c == cuenta)
            .map(|(_, o)| *o)
            .collect())
    }

    async fn organizacion_con_contrato(
        &mut self,
        org: OrganizacionId,
        _ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        Ok(self.datos.gestion.con_contrato.contains(&org))
    }

    async fn destinatarios_con_permiso(
        &mut self,
        permiso: Permiso,
    ) -> AppResult<Vec<Destinatario>> {
        Ok(self
            .datos
            .gestion
            .destinatarios
            .iter()
            .filter(|(p, _)| *p == permiso)
            .map(|(_, d)| d.clone())
            .collect())
    }

    async fn linea_predeterminada(&mut self) -> AppResult<Option<String>> {
        Ok(self.datos.gestion.linea.clone())
    }
}

impl RepositorioMantenimiento for TransaccionEnMemoria {
    async fn activos(
        &mut self,
        f: &FiltroActivos,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<Activo>> {
        let mut v: Vec<Activo> = self
            .datos
            .gestion
            .activos
            .iter()
            .filter(|a| f.clase.is_none_or(|c| a.clase == c))
            .filter(|a| f.estado.is_none_or(|e| a.estado == e))
            .filter(|a| {
                f.texto.as_deref().is_none_or(|t| {
                    a.nombre.to_lowercase().contains(&t.to_lowercase())
                        || a.codigo.as_str().contains(&t.to_uppercase())
                })
            })
            .filter(|a| pagina.despues().is_none_or(|d| a.id.as_uuid() > d))
            .cloned()
            .collect();
        v.sort_by_key(|a| a.id);
        v.truncate(usize::try_from(pagina.filas_a_consultar()).unwrap_or(usize::MAX));
        Ok(Pagina::desde_filas(v, pagina, |a| a.id.as_uuid()))
    }

    async fn activo(&mut self, id: ActivoId) -> AppResult<Option<Activo>> {
        Ok(self
            .datos
            .gestion
            .activos
            .iter()
            .find(|a| a.id == id)
            .cloned())
    }

    async fn insertar_activo(&mut self, a: &Activo) -> AppResult<()> {
        if self
            .datos
            .gestion
            .activos
            .iter()
            .any(|x| x.codigo == a.codigo)
        {
            return Err(AppError::Conflicto("código repetido".into()));
        }
        self.datos.gestion.activos.push(a.clone());
        Ok(())
    }

    async fn actualizar_activo(&mut self, a: &Activo) -> AppResult<()> {
        if let Some(x) = self.datos.gestion.activos.iter_mut().find(|x| x.id == a.id) {
            *x = a.clone();
        }
        Ok(())
    }

    async fn insertar_archivo_activo(
        &mut self,
        activo: ActivoId,
        a: &ArchivoActivo,
    ) -> AppResult<()> {
        self.datos.gestion.archivos_activo.push((activo, *a));
        Ok(())
    }

    async fn archivos_activo(&mut self, activo: ActivoId) -> AppResult<Vec<ArchivoActivo>> {
        Ok(self
            .datos
            .gestion
            .archivos_activo
            .iter()
            .filter(|(a, _)| *a == activo)
            .map(|(_, x)| *x)
            .collect())
    }

    async fn insertar_movimiento(&mut self, m: &Movimiento) -> AppResult<()> {
        self.datos.gestion.movimientos.push(m.clone());
        Ok(())
    }

    async fn movimientos(&mut self, activo: ActivoId) -> AppResult<Vec<Movimiento>> {
        Ok(self
            .datos
            .gestion
            .movimientos
            .iter()
            .filter(|m| m.activo_id == activo)
            .cloned()
            .collect())
    }

    async fn planes(&mut self, solo_activos: bool) -> AppResult<Vec<PlanMantenimiento>> {
        Ok(self
            .datos
            .gestion
            .planes
            .iter()
            .filter(|p| !solo_activos || p.activo)
            .cloned()
            .collect())
    }

    async fn plan(&mut self, id: PlanMantenimientoId) -> AppResult<Option<PlanMantenimiento>> {
        Ok(self
            .datos
            .gestion
            .planes
            .iter()
            .find(|p| p.id == id)
            .cloned())
    }

    async fn insertar_plan(&mut self, p: &PlanMantenimiento) -> AppResult<()> {
        self.datos.gestion.planes.push(p.clone());
        Ok(())
    }

    async fn actualizar_plan(&mut self, p: &PlanMantenimiento) -> AppResult<()> {
        if let Some(x) = self.datos.gestion.planes.iter_mut().find(|x| x.id == p.id) {
            *x = p.clone();
        }
        Ok(())
    }

    async fn ordenes(
        &mut self,
        f: &FiltroOrdenes,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<OrdenTrabajo>> {
        let v = self
            .datos
            .gestion
            .ordenes
            .iter()
            .filter(|o| f.estado.is_none_or(|e| o.estado == e))
            .filter(|o| f.activo_id.is_none_or(|a| o.activo_id == Some(a)))
            .filter(|o| f.ticket_id.is_none_or(|t| o.ticket_id == Some(t)))
            .filter(|o| {
                f.asignadas_a
                    .as_ref()
                    .is_none_or(|(c, orgs)| match o.asignado {
                        Some(Responsable::Cuenta(x)) => x == *c,
                        Some(Responsable::Organizacion(x)) => orgs.contains(&x),
                        None => false,
                    })
            })
            .filter(|o| {
                f.vencidas_a
                    .is_none_or(|h| o.programada_para < h && o.estado.pendiente())
            })
            .cloned()
            .collect();
        Ok(pagina_desc(v, pagina, |o| o.id.as_uuid()))
    }

    async fn orden(&mut self, id: OrdenTrabajoId) -> AppResult<Option<OrdenTrabajo>> {
        Ok(self
            .datos
            .gestion
            .ordenes
            .iter()
            .find(|o| o.id == id)
            .cloned())
    }

    async fn existe_orden_de_plan(
        &mut self,
        plan: PlanMantenimientoId,
        fecha: NaiveDate,
    ) -> AppResult<bool> {
        Ok(self
            .datos
            .gestion
            .ordenes
            .iter()
            .any(|o| o.plan_id == Some(plan) && o.programada_para == fecha))
    }

    async fn insertar_orden(&mut self, o: &OrdenTrabajo) -> AppResult<()> {
        if o.plan_id.is_some()
            && self
                .datos
                .gestion
                .ordenes
                .iter()
                .any(|x| x.plan_id == o.plan_id && x.programada_para == o.programada_para)
        {
            return Err(AppError::Conflicto("orden repetida".into()));
        }
        self.datos.gestion.ordenes.push(o.clone());
        Ok(())
    }

    async fn actualizar_orden(&mut self, o: &OrdenTrabajo) -> AppResult<()> {
        if let Some(x) = self.datos.gestion.ordenes.iter_mut().find(|x| x.id == o.id) {
            *x = o.clone();
        }
        Ok(())
    }

    async fn insertar_evidencia(
        &mut self,
        orden: OrdenTrabajoId,
        e: &EvidenciaOrden,
    ) -> AppResult<()> {
        self.datos.gestion.evidencias.push((orden, *e));
        Ok(())
    }

    async fn evidencias(&mut self, orden: OrdenTrabajoId) -> AppResult<Vec<EvidenciaOrden>> {
        Ok(self
            .datos
            .gestion
            .evidencias
            .iter()
            .filter(|(o, _)| *o == orden)
            .map(|(_, e)| *e)
            .collect())
    }

    async fn obligaciones(&mut self, solo_activas: bool) -> AppResult<Vec<Obligacion>> {
        Ok(self
            .datos
            .gestion
            .obligaciones
            .iter()
            .filter(|o| !solo_activas || o.activa)
            .cloned()
            .collect())
    }

    async fn obligacion(&mut self, id: ObligacionId) -> AppResult<Option<Obligacion>> {
        Ok(self
            .datos
            .gestion
            .obligaciones
            .iter()
            .find(|o| o.id == id)
            .cloned())
    }

    async fn insertar_obligacion(&mut self, o: &Obligacion) -> AppResult<()> {
        if o.codigo_sugerido.is_some()
            && self
                .datos
                .gestion
                .obligaciones
                .iter()
                .any(|x| x.activa && x.codigo_sugerido == o.codigo_sugerido)
        {
            return Err(AppError::Conflicto("sugerida ya activa".into()));
        }
        self.datos.gestion.obligaciones.push(o.clone());
        Ok(())
    }

    async fn actualizar_obligacion(&mut self, o: &Obligacion) -> AppResult<()> {
        if let Some(x) = self
            .datos
            .gestion
            .obligaciones
            .iter_mut()
            .find(|x| x.id == o.id)
        {
            *x = o.clone();
        }
        Ok(())
    }

    async fn insertar_cumplimiento(&mut self, c: &Cumplimiento) -> AppResult<()> {
        self.datos.gestion.cumplimientos.push(c.clone());
        Ok(())
    }

    async fn cumplimientos(&mut self, obligacion: ObligacionId) -> AppResult<Vec<Cumplimiento>> {
        Ok(self
            .datos
            .gestion
            .cumplimientos
            .iter()
            .filter(|c| c.obligacion_id == obligacion)
            .cloned()
            .collect())
    }

    async fn tablero_mantenimiento(&mut self, hoy: NaiveDate) -> AppResult<TableroMantenimiento> {
        let o = &self.datos.gestion.ordenes;
        let preventivas = o.iter().filter(|x| {
            x.tipo == domain::TipoOrden::Preventivo
                && x.estado != EstadoOrden::Cancelada
                && x.programada_para <= hoy
        });
        Ok(TableroMantenimiento {
            preventivas_programadas: preventivas.clone().count() as u64,
            preventivas_cumplidas: preventivas.filter(|x| x.estado.cumplida()).count() as u64,
            ordenes_vencidas: o
                .iter()
                .filter(|x| x.estado.pendiente() && x.programada_para < hoy)
                .count() as u64,
            costo_por_activo: o
                .iter()
                .filter_map(|x| {
                    x.activo_id
                        .map(|a| (a, x.costo_mano_obra + x.costo_repuestos))
                })
                .filter(|(_, c)| *c > Decimal::ZERO)
                .map(|(a, c)| (a, String::new(), c))
                .collect(),
        })
    }
}

impl RepositorioChat for TransaccionEnMemoria {
    async fn preguntas_frecuentes(
        &mut self,
        solo_activas: bool,
    ) -> AppResult<Vec<PreguntaFrecuente>> {
        let mut v: Vec<_> = self
            .datos
            .gestion
            .preguntas
            .iter()
            .filter(|p| !solo_activas || p.activa)
            .cloned()
            .collect();
        v.sort_by_key(|p| (p.orden, p.id));
        Ok(v)
    }

    async fn pregunta_frecuente(
        &mut self,
        id: PreguntaFrecuenteId,
    ) -> AppResult<Option<PreguntaFrecuente>> {
        Ok(self
            .datos
            .gestion
            .preguntas
            .iter()
            .find(|p| p.id == id)
            .cloned())
    }

    async fn guardar_pregunta_frecuente(
        &mut self,
        p: &PreguntaFrecuente,
        nueva: bool,
    ) -> AppResult<()> {
        let v = &mut self.datos.gestion.preguntas;
        if nueva {
            v.push(p.clone());
        } else if let Some(x) = v.iter_mut().find(|x| x.id == p.id) {
            *x = p.clone();
        }
        Ok(())
    }

    async fn conversacion_abierta(
        &mut self,
        linea: &str,
        telefono: &str,
    ) -> AppResult<Option<Conversacion>> {
        Ok(self
            .datos
            .gestion
            .conversaciones
            .iter()
            .find(|c| {
                c.linea == linea
                    && c.telefono == telefono
                    && c.estado == EstadoConversacion::Abierta
            })
            .cloned())
    }

    async fn conversacion(&mut self, id: ConversacionId) -> AppResult<Option<Conversacion>> {
        Ok(self
            .datos
            .gestion
            .conversaciones
            .iter()
            .find(|c| c.id == id)
            .cloned())
    }

    async fn insertar_conversacion(&mut self, c: &Conversacion) -> AppResult<()> {
        self.datos.gestion.conversaciones.push(c.clone());
        Ok(())
    }

    async fn actualizar_conversacion(&mut self, c: &Conversacion) -> AppResult<()> {
        if let Some(x) = self
            .datos
            .gestion
            .conversaciones
            .iter_mut()
            .find(|x| x.id == c.id)
        {
            *x = c.clone();
        }
        Ok(())
    }

    async fn conversaciones(
        &mut self,
        estado: Option<EstadoConversacion>,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<Conversacion>> {
        let v = self
            .datos
            .gestion
            .conversaciones
            .iter()
            .filter(|c| estado.is_none_or(|e| c.estado == e))
            .cloned()
            .collect();
        Ok(pagina_desc(v, pagina, |c| c.id.as_uuid()))
    }

    async fn insertar_mensaje_conversacion(&mut self, m: &MensajeConversacion) -> AppResult<bool> {
        if m.wamid.is_some()
            && self
                .datos
                .gestion
                .mensajes_conversacion
                .iter()
                .any(|x| x.wamid == m.wamid)
        {
            return Ok(false);
        }
        self.datos.gestion.mensajes_conversacion.push(m.clone());
        Ok(true)
    }

    async fn mensajes_conversacion(
        &mut self,
        id: ConversacionId,
    ) -> AppResult<Vec<MensajeConversacion>> {
        Ok(self
            .datos
            .gestion
            .mensajes_conversacion
            .iter()
            .filter(|m| m.conversacion_id == id)
            .cloned()
            .collect())
    }
}
