//! Tickets sobre la transacción del tenant: consecutivos, configuración, categorías, tickets,
//! mensajes, adjuntos, historial de estados y destinatarios de avisos internos.

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::{
        RepositorioTickets,
        tickets::{
            AdjuntoTicket, CategoriaTicket, ConfiguracionTickets, Destinatario, Encuesta,
            EventoTicket, FiltroTickets, MensajeTicket, Responsable, SerieConsecutivo,
            TableroTickets, Ticket, VisibilidadTickets,
        },
    },
};
use chrono::{DateTime, Utc};
use domain::{
    ActivoId, ArchivoId, BienPrivadoId, Calificacion, CanalTicket, CategoriaTicketId, CuentaId,
    DireccionCorreo, EstadoTicket, JornadaHabil, MensajeTicketId, NumeroTicket, OrganizacionId,
    Permiso, PersonaId, Plazo, PlazosTicket, PrioridadTicket, RecursoId, Telefono, TicketId,
    TipoTicket, UnidadPlazo,
};
use uuid::Uuid;

use crate::{TransaccionPg, copropiedades::error_escritura};

/// Estados en los que ya no corre el plazo (para las consultas de vencimiento).
const SIN_PLAZO: &[&str] = &[
    "resuelto",
    "cerrado",
    "rechazado",
    "duplicado",
    "en_espera_solicitante",
];

struct FilaTicket {
    id: Uuid,
    anio: i16,
    consecutivo: i32,
    tipo: String,
    categoria_id: Option<Uuid>,
    prioridad: String,
    canal: String,
    estado: String,
    asunto: String,
    descripcion: String,
    solicitante_persona_id: Option<Uuid>,
    solicitante_cuenta_id: Option<Uuid>,
    bien_privado_id: Option<Uuid>,
    ubicacion: Option<String>,
    recurso_id: Option<Uuid>,
    activo_id: Option<Uuid>,
    asignado_cuenta_id: Option<Uuid>,
    asignado_organizacion_id: Option<Uuid>,
    plazo_cantidad: Option<i32>,
    plazo_unidad: Option<String>,
    vence_en: Option<DateTime<Utc>>,
    alerta_vencimiento_en: Option<DateTime<Utc>>,
    resuelto_en: Option<DateTime<Utc>>,
    reabrible_hasta: Option<DateTime<Utc>>,
    cerrado_en: Option<DateTime<Utc>>,
    duplicado_de: Option<Uuid>,
    confidencial: bool,
    idempotency_key: String,
    calificacion: Option<i16>,
    comentario_encuesta: Option<String>,
    creado_en: DateTime<Utc>,
    actualizado_en: DateTime<Utc>,
}

fn entero<T: TryFrom<i64>>(v: i64, campo: &'static str) -> AppResult<T> {
    T::try_from(v).map_err(|_| AppError::Validacion(format!("`{campo}` fuera de rango")))
}

fn plazo_de(cantidad: Option<i32>, unidad: Option<String>) -> AppResult<Option<Plazo>> {
    match (cantidad, unidad) {
        (Some(c), Some(u)) => Ok(Some(Plazo::new(
            entero(i64::from(c), "plazo_cantidad")?,
            UnidadPlazo::try_from(u.as_str())?,
        )?)),
        _ => Ok(None),
    }
}

impl TryFrom<FilaTicket> for Ticket {
    type Error = AppError;

    fn try_from(f: FilaTicket) -> AppResult<Self> {
        let encuesta = match f.calificacion {
            Some(c) => Some(Encuesta {
                calificacion: Calificacion::new(entero(i64::from(c), "calificacion")?)?,
                comentario: f.comentario_encuesta,
            }),
            None => None,
        };
        Ok(Self {
            id: TicketId::from_uuid(f.id),
            numero: NumeroTicket {
                anio: i32::from(f.anio),
                consecutivo: entero(i64::from(f.consecutivo), "consecutivo")?,
            },
            tipo: TipoTicket::try_from(f.tipo.as_str())?,
            categoria_id: f.categoria_id.map(CategoriaTicketId::from_uuid),
            prioridad: PrioridadTicket::try_from(f.prioridad.as_str())?,
            canal: CanalTicket::try_from(f.canal.as_str())?,
            estado: EstadoTicket::try_from(f.estado.as_str())?,
            asunto: f.asunto,
            descripcion: f.descripcion,
            solicitante_persona_id: f.solicitante_persona_id.map(PersonaId::from_uuid),
            solicitante_cuenta_id: f.solicitante_cuenta_id.map(CuentaId::from_uuid),
            bien_privado_id: f.bien_privado_id.map(BienPrivadoId::from_uuid),
            ubicacion: f.ubicacion,
            recurso_id: f.recurso_id.map(RecursoId::from_uuid),
            activo_id: f.activo_id.map(ActivoId::from_uuid),
            asignado: Responsable::desde_columnas(
                f.asignado_cuenta_id.map(CuentaId::from_uuid),
                f.asignado_organizacion_id.map(OrganizacionId::from_uuid),
            ),
            plazo: plazo_de(f.plazo_cantidad, f.plazo_unidad)?,
            vence_en: f.vence_en,
            alerta_vencimiento_en: f.alerta_vencimiento_en,
            resuelto_en: f.resuelto_en,
            reabrible_hasta: f.reabrible_hasta,
            cerrado_en: f.cerrado_en,
            duplicado_de: f.duplicado_de.map(TicketId::from_uuid),
            confidencial: f.confidencial,
            idempotency_key: f.idempotency_key,
            encuesta,
            creado_en: f.creado_en,
            actualizado_en: f.actualizado_en,
        })
    }
}

fn a_tickets(filas: Vec<FilaTicket>) -> AppResult<Vec<Ticket>> {
    filas.into_iter().map(Ticket::try_from).collect()
}

struct FilaMensaje {
    id: Uuid,
    ticket_id: Uuid,
    autor_cuenta_id: Option<Uuid>,
    autor_persona_id: Option<Uuid>,
    canal: String,
    interno: bool,
    texto: String,
    creado_en: DateTime<Utc>,
}

struct FilaAdjunto {
    archivo_id: Uuid,
    mensaje_id: Option<Uuid>,
    interno: bool,
}

impl From<FilaAdjunto> for AdjuntoTicket {
    fn from(f: FilaAdjunto) -> Self {
        Self {
            archivo_id: ArchivoId::from_uuid(f.archivo_id),
            mensaje_id: f.mensaje_id.map(MensajeTicketId::from_uuid),
            interno: f.interno,
        }
    }
}

impl RepositorioTickets for TransaccionPg {
    async fn siguiente_consecutivo(
        &mut self,
        serie: SerieConsecutivo,
        anio: i32,
    ) -> AppResult<u32> {
        self.siguiente_en_serie(serie.as_str(), anio).await
    }

    async fn configuracion_tickets(&mut self) -> AppResult<ConfiguracionTickets> {
        let fila = sqlx::query!(
            "SELECT jornada_inicio, jornada_fin, incluye_sabado, dias_reapertura
             FROM configuracion_tickets"
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let reglas = sqlx::query!("SELECT tipo, prioridad, cantidad, unidad FROM plazos_ticket")
            .fetch_all(&mut *self.tx)
            .await
            .map_err(AppError::infra)?
            .into_iter()
            .map(|r| {
                Ok((
                    TipoTicket::try_from(r.tipo.as_str())?,
                    r.prioridad
                        .as_deref()
                        .map(PrioridadTicket::try_from)
                        .transpose()?,
                    Plazo::new(
                        entero(i64::from(r.cantidad), "cantidad")?,
                        UnidadPlazo::try_from(r.unidad.as_str())?,
                    )?,
                ))
            })
            .collect::<AppResult<Vec<_>>>()?;
        let mut config = ConfiguracionTickets::predeterminada();
        if let Some(f) = fila {
            config.jornada = JornadaHabil {
                inicio: f.jornada_inicio,
                fin: f.jornada_fin,
                incluye_sabado: f.incluye_sabado,
            };
            config.dias_reapertura = entero(i64::from(f.dias_reapertura), "dias_reapertura")?;
        }
        config.plazos = PlazosTicket::new(reglas);
        Ok(config)
    }

    async fn guardar_configuracion_tickets(&mut self, c: &ConfiguracionTickets) -> AppResult<()> {
        let dias: i16 = entero(i64::from(c.dias_reapertura), "dias_reapertura")?;
        sqlx::query!(
            "INSERT INTO configuracion_tickets
                 (tenant_id, jornada_inicio, jornada_fin, incluye_sabado, dias_reapertura)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (tenant_id) DO UPDATE SET
                 jornada_inicio = EXCLUDED.jornada_inicio, jornada_fin = EXCLUDED.jornada_fin,
                 incluye_sabado = EXCLUDED.incluye_sabado,
                 dias_reapertura = EXCLUDED.dias_reapertura, actualizado_en = now()",
            self.ctx.tenant_id.as_uuid(),
            c.jornada.inicio,
            c.jornada.fin,
            c.jornada.incluye_sabado,
            dias,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query!("DELETE FROM plazos_ticket")
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        for (tipo, prioridad, plazo) in c.plazos.reglas() {
            sqlx::query!(
                "INSERT INTO plazos_ticket (tenant_id, tipo, prioridad, cantidad, unidad)
                 VALUES ($1, $2, $3, $4, $5)",
                self.ctx.tenant_id.as_uuid(),
                tipo.as_str(),
                prioridad.map(PrioridadTicket::as_str),
                entero::<i32>(i64::from(plazo.cantidad), "cantidad")?,
                plazo.unidad.as_str(),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_escritura(e, "regla de plazo repetida", "copropiedad"))?;
        }
        Ok(())
    }

    async fn categorias_ticket(&mut self) -> AppResult<Vec<CategoriaTicket>> {
        Ok(sqlx::query!(
            "SELECT id, nombre, activa FROM categorias_ticket ORDER BY normalizar(nombre), id"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| CategoriaTicket {
            id: CategoriaTicketId::from_uuid(r.id),
            nombre: r.nombre,
            activa: r.activa,
        })
        .collect())
    }

    async fn guardar_categoria_ticket(
        &mut self,
        c: &CategoriaTicket,
        nueva: bool,
    ) -> AppResult<()> {
        let r = if nueva {
            sqlx::query!(
                "INSERT INTO categorias_ticket (id, tenant_id, nombre, activa) VALUES ($1, $2, $3, $4)",
                c.id.as_uuid(),
                self.ctx.tenant_id.as_uuid(),
                c.nombre,
                c.activa,
            )
            .execute(&mut *self.tx)
            .await
        } else {
            sqlx::query!(
                "UPDATE categorias_ticket SET nombre = $2, activa = $3 WHERE id = $1",
                c.id.as_uuid(),
                c.nombre,
                c.activa,
            )
            .execute(&mut *self.tx)
            .await
        };
        r.map_err(|e| error_escritura(e, "ya existe una categoría con ese nombre", "categoría"))?;
        Ok(())
    }

    async fn ticket(&mut self, id: TicketId) -> AppResult<Option<Ticket>> {
        sqlx::query_as!(
            FilaTicket,
            "SELECT id, anio, consecutivo, tipo, categoria_id, prioridad, canal, estado, asunto,
                    descripcion, solicitante_persona_id, solicitante_cuenta_id, bien_privado_id,
                    ubicacion, recurso_id, activo_id, asignado_cuenta_id,
                    asignado_organizacion_id, plazo_cantidad, plazo_unidad, vence_en,
                    alerta_vencimiento_en, resuelto_en, reabrible_hasta, cerrado_en,
                    duplicado_de, confidencial, idempotency_key, calificacion,
                    comentario_encuesta, creado_en, actualizado_en
             FROM tickets WHERE id = $1",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Ticket::try_from)
        .transpose()
    }

    async fn ticket_por_clave(&mut self, clave: &str) -> AppResult<Option<Ticket>> {
        sqlx::query_as!(
            FilaTicket,
            "SELECT id, anio, consecutivo, tipo, categoria_id, prioridad, canal, estado, asunto,
                    descripcion, solicitante_persona_id, solicitante_cuenta_id, bien_privado_id,
                    ubicacion, recurso_id, activo_id, asignado_cuenta_id,
                    asignado_organizacion_id, plazo_cantidad, plazo_unidad, vence_en,
                    alerta_vencimiento_en, resuelto_en, reabrible_hasta, cerrado_en,
                    duplicado_de, confidencial, idempotency_key, calificacion,
                    comentario_encuesta, creado_en, actualizado_en
             FROM tickets WHERE idempotency_key = $1",
            clave
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Ticket::try_from)
        .transpose()
    }

    async fn insertar_ticket(&mut self, t: &Ticket) -> AppResult<()> {
        let (cuenta, org) = Responsable::columnas(t.asignado);
        sqlx::query!(
            "INSERT INTO tickets (id, tenant_id, anio, consecutivo, tipo, categoria_id, prioridad,
                                  canal, estado, asunto, descripcion, solicitante_persona_id,
                                  solicitante_cuenta_id, bien_privado_id, ubicacion, recurso_id,
                                  activo_id, asignado_cuenta_id, asignado_organizacion_id,
                                  plazo_cantidad, plazo_unidad, vence_en, confidencial,
                                  idempotency_key, creado_en, actualizado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18, $19, $20, $21, $22, $23, $24, $25, $26)",
            t.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            entero::<i16>(i64::from(t.numero.anio), "anio")?,
            entero::<i32>(i64::from(t.numero.consecutivo), "consecutivo")?,
            t.tipo.as_str(),
            t.categoria_id.map(CategoriaTicketId::as_uuid),
            t.prioridad.as_str(),
            t.canal.as_str(),
            t.estado.as_str(),
            t.asunto,
            t.descripcion,
            t.solicitante_persona_id.map(PersonaId::as_uuid),
            t.solicitante_cuenta_id.map(CuentaId::as_uuid),
            t.bien_privado_id.map(BienPrivadoId::as_uuid),
            t.ubicacion,
            t.recurso_id.map(RecursoId::as_uuid),
            t.activo_id.map(ActivoId::as_uuid),
            cuenta.map(CuentaId::as_uuid),
            org.map(OrganizacionId::as_uuid),
            t.plazo
                .map(|p| entero::<i32>(i64::from(p.cantidad), "plazo"))
                .transpose()?,
            t.plazo.map(|p| p.unidad.as_str()),
            t.vence_en,
            t.confidencial,
            t.idempotency_key,
            t.creado_en,
            t.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "ya existe un ticket con esa clave de idempotencia",
                "persona, bien privado, categoría, recurso o activo",
            )
        })?;
        Ok(())
    }

    async fn actualizar_ticket(&mut self, t: &Ticket) -> AppResult<()> {
        let (cuenta, org) = Responsable::columnas(t.asignado);
        sqlx::query!(
            "UPDATE tickets SET
                 tipo = $2, categoria_id = $3, prioridad = $4, estado = $5,
                 asignado_cuenta_id = $6, asignado_organizacion_id = $7, plazo_cantidad = $8,
                 plazo_unidad = $9, vence_en = $10, alerta_vencimiento_en = $11,
                 resuelto_en = $12, reabrible_hasta = $13, cerrado_en = $14, duplicado_de = $15,
                 calificacion = $16, comentario_encuesta = $17, actualizado_en = $18
             WHERE id = $1",
            t.id.as_uuid(),
            t.tipo.as_str(),
            t.categoria_id.map(CategoriaTicketId::as_uuid),
            t.prioridad.as_str(),
            t.estado.as_str(),
            cuenta.map(CuentaId::as_uuid),
            org.map(OrganizacionId::as_uuid),
            t.plazo
                .map(|p| entero::<i32>(i64::from(p.cantidad), "plazo"))
                .transpose()?,
            t.plazo.map(|p| p.unidad.as_str()),
            t.vence_en,
            t.alerta_vencimiento_en,
            t.resuelto_en,
            t.reabrible_hasta,
            t.cerrado_en,
            t.duplicado_de.map(TicketId::as_uuid),
            t.encuesta
                .as_ref()
                .map(|e| i16::from(e.calificacion.valor())),
            t.encuesta.as_ref().and_then(|e| e.comentario.clone()),
            t.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "ticket inválido", "categoría o ticket original"))?;
        Ok(())
    }

    async fn tickets(
        &mut self,
        v: &VisibilidadTickets,
        f: &FiltroTickets,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<Ticket>> {
        let orgs: Vec<Uuid> = v.organizaciones.iter().map(|o| o.as_uuid()).collect();
        let sin_plazo: Vec<String> = SIN_PLAZO.iter().map(|s| (*s).to_owned()).collect();
        let filas = sqlx::query_as!(
            FilaTicket,
            "SELECT id, anio, consecutivo, tipo, categoria_id, prioridad, canal, estado, asunto,
                    descripcion, solicitante_persona_id, solicitante_cuenta_id, bien_privado_id,
                    ubicacion, recurso_id, activo_id, asignado_cuenta_id,
                    asignado_organizacion_id, plazo_cantidad, plazo_unidad, vence_en,
                    alerta_vencimiento_en, resuelto_en, reabrible_hasta, cerrado_en,
                    duplicado_de, confidencial, idempotency_key, calificacion,
                    comentario_encuesta, creado_en, actualizado_en
             FROM tickets WHERE ($1 OR ($2 AND confidencial) OR solicitante_cuenta_id = $3
                        OR asignado_cuenta_id = $3 OR asignado_organizacion_id = ANY($4))
                   AND ($5::text IS NULL OR estado = $5)
                   AND ($6::text IS NULL OR tipo = $6)
                   AND ($7::text IS NULL OR prioridad = $7)
                   AND ($8::uuid IS NULL OR solicitante_cuenta_id = $8)
                   AND ($9::uuid IS NULL OR solicitante_persona_id = $9)
                   AND ($10::uuid IS NULL OR bien_privado_id = $10)
                   AND ($11::timestamptz IS NULL OR (vence_en <= $11 AND estado <> ALL($12)))
                   AND ($13::uuid IS NULL OR id < $13)
                 ORDER BY id DESC
                 LIMIT $14",
            v.todos,
            v.convivencia,
            v.cuenta.map(CuentaId::as_uuid),
            &orgs,
            f.estado.map(EstadoTicket::as_str),
            f.tipo.map(TipoTicket::as_str),
            f.prioridad.map(PrioridadTicket::as_str),
            f.solicitante_cuenta.map(CuentaId::as_uuid),
            f.solicitante_persona.map(PersonaId::as_uuid),
            f.bien_privado_id.map(BienPrivadoId::as_uuid),
            f.vencidos_a,
            &sin_plazo,
            pagina.despues(),
            pagina.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(Pagina::desde_filas(a_tickets(filas)?, pagina, |t| {
            t.id.as_uuid()
        }))
    }

    async fn insertar_mensaje_ticket(&mut self, m: &MensajeTicket) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO ticket_mensajes (id, tenant_id, ticket_id, autor_cuenta_id,
                                          autor_persona_id, canal, interno, texto, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            m.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            m.ticket_id.as_uuid(),
            m.autor_cuenta_id.map(CuentaId::as_uuid),
            m.autor_persona_id.map(PersonaId::as_uuid),
            m.canal.as_str(),
            m.interno,
            m.texto,
            m.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "mensaje repetido", "ticket o persona"))?;
        Ok(())
    }

    async fn mensajes_ticket(
        &mut self,
        ticket: TicketId,
        incluir_internos: bool,
    ) -> AppResult<Vec<MensajeTicket>> {
        let filas = sqlx::query_as!(
            FilaMensaje,
            "SELECT id, ticket_id, autor_cuenta_id, autor_persona_id, canal, interno, texto,
                    creado_en
             FROM ticket_mensajes
             WHERE ticket_id = $1 AND ($2 OR NOT interno)
             ORDER BY creado_en, id",
            ticket.as_uuid(),
            incluir_internos,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let adjuntos = self.adjuntos_ticket(ticket, incluir_internos).await?;
        filas
            .into_iter()
            .map(|f| {
                let id = MensajeTicketId::from_uuid(f.id);
                Ok(MensajeTicket {
                    id,
                    ticket_id: TicketId::from_uuid(f.ticket_id),
                    autor_cuenta_id: f.autor_cuenta_id.map(CuentaId::from_uuid),
                    autor_persona_id: f.autor_persona_id.map(PersonaId::from_uuid),
                    canal: CanalTicket::try_from(f.canal.as_str())?,
                    interno: f.interno,
                    texto: f.texto,
                    adjuntos: adjuntos
                        .iter()
                        .filter(|a| a.mensaje_id == Some(id))
                        .map(|a| a.archivo_id)
                        .collect(),
                    creado_en: f.creado_en,
                })
            })
            .collect()
    }

    async fn insertar_adjunto_ticket(
        &mut self,
        ticket: TicketId,
        a: &AdjuntoTicket,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO ticket_adjuntos (tenant_id, ticket_id, mensaje_id, archivo_id, interno)
             VALUES ($1, $2, $3, $4, $5)",
            self.ctx.tenant_id.as_uuid(),
            ticket.as_uuid(),
            a.mensaje_id.map(MensajeTicketId::as_uuid),
            a.archivo_id.as_uuid(),
            a.interno,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "el archivo ya está adjunto al ticket", "archivo"))?;
        Ok(())
    }

    async fn adjuntos_ticket(
        &mut self,
        ticket: TicketId,
        incluir_internos: bool,
    ) -> AppResult<Vec<AdjuntoTicket>> {
        Ok(sqlx::query_as!(
            FilaAdjunto,
            "SELECT archivo_id, mensaje_id, interno FROM ticket_adjuntos
             WHERE ticket_id = $1 AND ($2 OR NOT interno)
             ORDER BY creado_en, archivo_id",
            ticket.as_uuid(),
            incluir_internos,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Into::into)
        .collect())
    }

    async fn insertar_evento_ticket(&mut self, e: &EventoTicket) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO ticket_eventos (id, tenant_id, ticket_id, desde, hacia, actor_cuenta_id,
                                         motivo, ocurrido_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            Uuid::now_v7(),
            self.ctx.tenant_id.as_uuid(),
            e.ticket_id.as_uuid(),
            e.desde.map(EstadoTicket::as_str),
            e.hacia.as_str(),
            e.actor_cuenta_id.map(CuentaId::as_uuid),
            e.motivo
                .as_deref()
                .map(|m| m.chars().take(500).collect::<String>()),
            e.ocurrido_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn eventos_ticket(&mut self, ticket: TicketId) -> AppResult<Vec<EventoTicket>> {
        sqlx::query!(
            "SELECT desde, hacia, actor_cuenta_id, motivo, ocurrido_en FROM ticket_eventos
             WHERE ticket_id = $1 ORDER BY ocurrido_en, id",
            ticket.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| {
            Ok(EventoTicket {
                ticket_id: ticket,
                desde: r.desde.as_deref().map(EstadoTicket::try_from).transpose()?,
                hacia: EstadoTicket::try_from(r.hacia.as_str())?,
                actor_cuenta_id: r.actor_cuenta_id.map(CuentaId::from_uuid),
                motivo: r.motivo,
                ocurrido_en: r.ocurrido_en,
            })
        })
        .collect()
    }

    async fn tickets_por_cerrar(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<Ticket>> {
        let filas = sqlx::query_as!(
            FilaTicket,
            "SELECT id, anio, consecutivo, tipo, categoria_id, prioridad, canal, estado, asunto,
                    descripcion, solicitante_persona_id, solicitante_cuenta_id, bien_privado_id,
                    ubicacion, recurso_id, activo_id, asignado_cuenta_id,
                    asignado_organizacion_id, plazo_cantidad, plazo_unidad, vence_en,
                    alerta_vencimiento_en, resuelto_en, reabrible_hasta, cerrado_en,
                    duplicado_de, confidencial, idempotency_key, calificacion,
                    comentario_encuesta, creado_en, actualizado_en
             FROM tickets WHERE estado = 'resuelto' AND reabrible_hasta <= $1 ORDER BY id LIMIT 500",
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        a_tickets(filas)
    }

    async fn tickets_vencidos_sin_alerta(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<Ticket>> {
        let sin_plazo: Vec<String> = SIN_PLAZO.iter().map(|s| (*s).to_owned()).collect();
        let filas = sqlx::query_as!(
            FilaTicket,
            "SELECT id, anio, consecutivo, tipo, categoria_id, prioridad, canal, estado, asunto,
                    descripcion, solicitante_persona_id, solicitante_cuenta_id, bien_privado_id,
                    ubicacion, recurso_id, activo_id, asignado_cuenta_id,
                    asignado_organizacion_id, plazo_cantidad, plazo_unidad, vence_en,
                    alerta_vencimiento_en, resuelto_en, reabrible_hasta, cerrado_en,
                    duplicado_de, confidencial, idempotency_key, calificacion,
                    comentario_encuesta, creado_en, actualizado_en
             FROM tickets WHERE vence_en <= $1 AND alerta_vencimiento_en IS NULL AND estado <> ALL($2)
                 ORDER BY id LIMIT 500",
            ahora,
            &sin_plazo,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        a_tickets(filas)
    }

    async fn tablero_tickets(&mut self, ahora: DateTime<Utc>) -> AppResult<TableroTickets> {
        let por_estado = sqlx::query!(
            r#"SELECT estado, count(*) AS "n!" FROM tickets
               WHERE estado NOT IN ('resuelto', 'cerrado', 'rechazado', 'duplicado')
               GROUP BY estado ORDER BY estado"#
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| {
            Ok((
                EstadoTicket::try_from(r.estado.as_str())?,
                entero(r.n, "conteo")?,
            ))
        })
        .collect::<AppResult<Vec<_>>>()?;
        let sin_plazo: Vec<String> = SIN_PLAZO.iter().map(|s| (*s).to_owned()).collect();
        let r = sqlx::query!(
            r#"SELECT
                 (SELECT count(*) FROM tickets WHERE vence_en <= $1 AND estado <> ALL($2))
                     AS "vencidos!",
                 (SELECT avg(EXTRACT(EPOCH FROM (e.primero - t.creado_en)))::float8 / 3600
                  FROM tickets t
                  JOIN LATERAL (SELECT min(ocurrido_en) AS primero FROM ticket_eventos e
                                WHERE e.ticket_id = t.id AND e.desde IS NOT NULL) e ON TRUE
                  WHERE e.primero IS NOT NULL) AS respuesta,
                 (SELECT avg(EXTRACT(EPOCH FROM (resuelto_en - creado_en)))::float8 / 3600
                  FROM tickets WHERE resuelto_en IS NOT NULL) AS solucion,
                 (SELECT avg(calificacion)::float8 FROM tickets WHERE calificacion IS NOT NULL)
                     AS satisfaccion"#,
            ahora,
            &sin_plazo,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(TableroTickets {
            por_estado,
            vencidos: entero(r.vencidos, "vencidos")?,
            horas_promedio_respuesta: r.respuesta,
            horas_promedio_solucion: r.solucion,
            satisfaccion_promedio: r.satisfaccion,
        })
    }

    async fn persona_de_cuenta(&mut self, cuenta: CuentaId) -> AppResult<Option<PersonaId>> {
        Ok(sqlx::query_scalar!(
            "SELECT id FROM personas WHERE cuenta_id = $1 ORDER BY id LIMIT 1",
            cuenta.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(PersonaId::from_uuid))
    }

    async fn organizaciones_de_cuenta(
        &mut self,
        cuenta: CuentaId,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<OrganizacionId>> {
        Ok(sqlx::query_scalar!(
            "SELECT DISTINCT c.organizacion_id FROM asignaciones_rol a
             JOIN contratos_servicio c ON c.id = a.origen_id
             WHERE a.cuenta_id = $1 AND a.origen = 'contrato_servicio'
               AND a.vigente_desde <= $2 AND (a.vigente_hasta IS NULL OR a.vigente_hasta > $2)
               AND c.estado = 'vigente' AND c.vigente_desde <= $2
               AND (c.vigente_hasta IS NULL OR c.vigente_hasta > $2)",
            cuenta.as_uuid(),
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(OrganizacionId::from_uuid)
        .collect())
    }

    async fn organizacion_con_contrato(
        &mut self,
        org: OrganizacionId,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (
                 SELECT 1 FROM contratos_servicio
                 WHERE organizacion_id = $1 AND estado = 'vigente' AND vigente_desde <= $2
                   AND (vigente_hasta IS NULL OR vigente_hasta > $2)) AS "existe!""#,
            org.as_uuid(),
            ahora,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn destinatarios_con_permiso(
        &mut self,
        permiso: Permiso,
    ) -> AppResult<Vec<Destinatario>> {
        Ok(sqlx::query!(
            r#"SELECT cuenta_id AS "cuenta_id!", correo, telefono
               FROM destinatarios_con_permiso($1)"#,
            permiso.as_str(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| Destinatario {
            cuenta_id: CuentaId::from_uuid(r.cuenta_id),
            correo: r.correo.and_then(|c| DireccionCorreo::new(&c).ok()),
            telefono: r.telefono.and_then(|t| Telefono::new(&t).ok()),
        })
        .collect())
    }

    async fn linea_predeterminada(&mut self) -> AppResult<Option<String>> {
        sqlx::query_scalar!("SELECT linea_whatsapp_predeterminada()")
            .fetch_one(&mut *self.tx)
            .await
            .map_err(AppError::infra)
    }
}

impl application::puertos::tickets::RepositorioTicketsDeCuenta for crate::TransaccionPlataformaPg {
    async fn tickets_de_cuenta(
        &mut self,
        cuenta: CuentaId,
        limite: u32,
    ) -> AppResult<Vec<application::puertos::tickets::TicketDeCuenta>> {
        sqlx::query!(
            r#"SELECT tenant_id AS "tenant_id!", copropiedad AS "copropiedad!", ticket_id AS "ticket_id!",
                      anio AS "anio!", consecutivo AS "consecutivo!", tipo AS "tipo!", estado AS "estado!",
                      asunto AS "asunto!", creado_en AS "creado_en!", actualizado_en AS "actualizado_en!"
               FROM tickets_de_cuenta($1, $2)"#,
            cuenta.as_uuid(),
            i32::try_from(limite).unwrap_or(100),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(application::puertos::tickets::TicketDeCuenta {
                tenant_id: domain::TenantId::from_uuid(f.tenant_id),
                copropiedad: f.copropiedad,
                ticket_id: TicketId::from_uuid(f.ticket_id),
                numero: NumeroTicket {
                    anio: i32::from(f.anio),
                    consecutivo: u32::try_from(f.consecutivo).unwrap_or(0),
                },
                tipo: TipoTicket::try_from(f.tipo.as_str())?,
                estado: EstadoTicket::try_from(f.estado.as_str())?,
                asunto: f.asunto,
                creado_en: f.creado_en,
                actualizado_en: f.actualizado_en,
            })
        })
        .collect()
    }
}
