//! Chat de WhatsApp sobre PostgreSQL:
//!
//! - [`SesionesChatPg`] (plataforma, `app.plataforma = on`): sesiones por (línea, teléfono),
//!   `wamid` procesados y respuestas encoladas en el outbox de plataforma, en una transacción
//!   por turno.
//! - [`DirectorioTelefonosPg`]: vínculos del teléfono con copropiedades (función
//!   `SECURITY DEFINER` `vinculos_por_telefono`).
//! - [`TransaccionPg`] implementa [`RepositorioChat`] (preguntas frecuentes y bandeja).

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    outbox::{MensajeOutbox, PrioridadSalida},
    puertos::{
        DirectorioTelefonos, RepositorioChat, SesionesConversacion,
        chat::{
            Conversacion, EstadoConversacion, MensajeConversacion, PreguntaFrecuente,
            VinculoTelefono,
        },
        sesiones::{CierreTurno, SesionGuardada},
    },
};
use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, BienPrivadoId, ConversacionId, CuentaId, PersonaId, PreguntaFrecuenteId, TenantId,
};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{TransaccionPg, copropiedades::error_escritura};

async fn transaccion_plataforma(pool: &PgPool) -> AppResult<Transaction<'static, Postgres>> {
    let mut tx = pool.begin().await.map_err(AppError::infra)?;
    sqlx::query("SELECT set_config('app.plataforma', 'on', true)")
        .execute(&mut *tx)
        .await
        .map_err(AppError::infra)?;
    Ok(tx)
}

/// Sesiones del chat en PostgreSQL (rol `dad_app`, ámbito de plataforma).
#[derive(Debug, Clone)]
pub struct SesionesChatPg {
    pool: PgPool,
}

impl SesionesChatPg {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Borra las sesiones vencidas y los `wamid` de más de 30 días (Meta no reintenta tanto).
    /// Devuelve cuántas filas borró.
    ///
    /// # Errors
    ///
    /// Errores de la base de datos.
    pub async fn purgar(&self, ahora: DateTime<Utc>) -> AppResult<u64> {
        let mut tx = transaccion_plataforma(&self.pool).await?;
        let sesiones = sqlx::query!("DELETE FROM sesiones_chat WHERE expira_en <= $1", ahora)
            .execute(&mut *tx)
            .await
            .map_err(AppError::infra)?
            .rows_affected();
        let wamids = sqlx::query!(
            "DELETE FROM mensajes_entrantes WHERE recibido_en <= $1::timestamptz - interval '30 days'",
            ahora
        )
        .execute(&mut *tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        tx.commit().await.map_err(AppError::infra)?;
        Ok(sesiones + wamids)
    }
}

impl SesionesConversacion for SesionesChatPg {
    async fn ya_procesado(&self, wamid: &str) -> AppResult<bool> {
        let mut tx = transaccion_plataforma(&self.pool).await?;
        let existe = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM mensajes_entrantes WHERE wamid = $1) AS "existe!""#,
            wamid
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::infra)?;
        tx.commit().await.map_err(AppError::infra)?;
        Ok(existe)
    }

    async fn marcar_procesado(&self, wamid: &str) -> AppResult<bool> {
        let mut tx = transaccion_plataforma(&self.pool).await?;
        let nuevo = sqlx::query!(
            "INSERT INTO mensajes_entrantes (wamid) VALUES ($1) ON CONFLICT DO NOTHING",
            wamid
        )
        .execute(&mut *tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected()
            == 1;
        tx.commit().await.map_err(AppError::infra)?;
        Ok(nuevo)
    }

    async fn obtener(
        &self,
        linea: &str,
        telefono: &str,
        ahora: DateTime<Utc>,
    ) -> AppResult<Option<SesionGuardada>> {
        let mut tx = transaccion_plataforma(&self.pool).await?;
        let fila = sqlx::query!(
            "SELECT estado, ultimo_entrante_en FROM sesiones_chat
             WHERE linea = $1 AND telefono = $2 AND expira_en > $3",
            linea,
            telefono,
            ahora,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(AppError::infra)?;
        tx.commit().await.map_err(AppError::infra)?;
        Ok(fila.map(|f| SesionGuardada {
            estado: f.estado,
            ultimo_entrante_en: f.ultimo_entrante_en,
        }))
    }

    async fn cerrar_turno(&self, c: &CierreTurno) -> AppResult<bool> {
        let mut tx = transaccion_plataforma(&self.pool).await?;
        let nuevo = sqlx::query!(
            "INSERT INTO mensajes_entrantes (wamid) VALUES ($1) ON CONFLICT DO NOTHING",
            c.wamid
        )
        .execute(&mut *tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected()
            == 1;
        if !nuevo {
            return Ok(false);
        }
        match &c.sesion {
            Some(s) => {
                sqlx::query!(
                    "INSERT INTO sesiones_chat (linea, telefono, estado, ultimo_entrante_en, expira_en)
                     VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT (linea, telefono) DO UPDATE SET
                         estado = EXCLUDED.estado,
                         ultimo_entrante_en = EXCLUDED.ultimo_entrante_en,
                         expira_en = EXCLUDED.expira_en",
                    c.linea,
                    c.telefono,
                    s.estado,
                    s.ultimo_entrante_en,
                    c.expira_en,
                )
                .execute(&mut *tx)
                .await
                .map_err(AppError::infra)?;
            }
            None => {
                sqlx::query!(
                    "DELETE FROM sesiones_chat WHERE linea = $1 AND telefono = $2",
                    c.linea,
                    c.telefono
                )
                .execute(&mut *tx)
                .await
                .map_err(AppError::infra)?;
            }
        }
        for r in &c.respuestas {
            let mensaje = MensajeOutbox::WhatsApp(r.clone());
            let payload = serde_json::to_value(&mensaje).map_err(AppError::infra)?;
            sqlx::query!(
                "INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload)
                 VALUES (NULL, $1, $2, $3, $4, $5)",
                mensaje.canal().as_str(),
                PrioridadSalida::Confirmacion.as_i16(),
                mensaje.linea(),
                mensaje.destino(),
                payload,
            )
            .execute(&mut *tx)
            .await
            .map_err(AppError::infra)?;
        }
        tx.commit().await.map_err(AppError::infra)?;
        Ok(true)
    }
}

/// Directorio de teléfonos (función `vinculos_por_telefono`).
#[derive(Debug, Clone)]
pub struct DirectorioTelefonosPg {
    pool: PgPool,
}

impl DirectorioTelefonosPg {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl DirectorioTelefonos for DirectorioTelefonosPg {
    async fn vinculos_por_telefono(&self, telefono: &str) -> AppResult<Vec<VinculoTelefono>> {
        Ok(sqlx::query!(
            r#"SELECT tenant_id AS "tenant_id!", copropiedad AS "copropiedad!",
                      persona_id AS "persona_id!", cuenta_id, bien_privado_id AS "bien_privado_id!",
                      codigo AS "codigo!", vinculo AS "vinculo!"
               FROM vinculos_por_telefono($1)"#,
            telefono,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| VinculoTelefono {
            tenant_id: TenantId::from_uuid(r.tenant_id),
            copropiedad: r.copropiedad,
            persona_id: PersonaId::from_uuid(r.persona_id),
            cuenta_id: r.cuenta_id.map(CuentaId::from_uuid),
            bien_privado_id: BienPrivadoId::from_uuid(r.bien_privado_id),
            codigo: r.codigo,
            es_propietario: r.vinculo == "propietario",
        })
        .collect())
    }
}

struct FilaConversacion {
    id: Uuid,
    linea: String,
    telefono: String,
    persona_id: Option<Uuid>,
    estado: String,
    atendida_por: Option<Uuid>,
    abierta_en: DateTime<Utc>,
    ultimo_entrante_en: DateTime<Utc>,
    cerrada_en: Option<DateTime<Utc>>,
}

impl TryFrom<FilaConversacion> for Conversacion {
    type Error = AppError;

    fn try_from(f: FilaConversacion) -> AppResult<Self> {
        Ok(Self {
            id: ConversacionId::from_uuid(f.id),
            linea: f.linea,
            telefono: f.telefono,
            persona_id: f.persona_id.map(PersonaId::from_uuid),
            estado: match f.estado.as_str() {
                "abierta" => EstadoConversacion::Abierta,
                "cerrada" => EstadoConversacion::Cerrada,
                otro => {
                    return Err(AppError::Validacion(format!(
                        "estado de conversación desconocido: {otro}"
                    )));
                }
            },
            atendida_por: f.atendida_por.map(CuentaId::from_uuid),
            abierta_en: f.abierta_en,
            ultimo_entrante_en: f.ultimo_entrante_en,
            cerrada_en: f.cerrada_en,
        })
    }
}

struct FilaPregunta {
    id: Uuid,
    pregunta: String,
    respuesta: String,
    orden: i16,
    activa: bool,
    archivo_id: Option<Uuid>,
}

impl From<FilaPregunta> for PreguntaFrecuente {
    fn from(f: FilaPregunta) -> Self {
        Self {
            id: PreguntaFrecuenteId::from_uuid(f.id),
            pregunta: f.pregunta,
            respuesta: f.respuesta,
            orden: f.orden,
            activa: f.activa,
            archivo_id: f.archivo_id.map(ArchivoId::from_uuid),
        }
    }
}

impl RepositorioChat for TransaccionPg {
    async fn preguntas_frecuentes(
        &mut self,
        solo_activas: bool,
    ) -> AppResult<Vec<PreguntaFrecuente>> {
        Ok(sqlx::query_as!(
            FilaPregunta,
            "SELECT id, pregunta, respuesta, orden, activa, archivo_id FROM preguntas_frecuentes
             WHERE (NOT $1 OR activa) ORDER BY orden, id",
            solo_activas,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Into::into)
        .collect())
    }

    async fn pregunta_frecuente(
        &mut self,
        id: PreguntaFrecuenteId,
    ) -> AppResult<Option<PreguntaFrecuente>> {
        Ok(sqlx::query_as!(
            FilaPregunta,
            "SELECT id, pregunta, respuesta, orden, activa, archivo_id FROM preguntas_frecuentes
             WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Into::into))
    }

    async fn guardar_pregunta_frecuente(
        &mut self,
        p: &PreguntaFrecuente,
        nueva: bool,
    ) -> AppResult<()> {
        let r = if nueva {
            sqlx::query!(
                "INSERT INTO preguntas_frecuentes (id, tenant_id, pregunta, respuesta, orden, activa,
                                                   archivo_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                p.id.as_uuid(),
                self.ctx.tenant_id.as_uuid(),
                p.pregunta,
                p.respuesta,
                p.orden,
                p.activa,
                p.archivo_id.map(ArchivoId::as_uuid),
            )
            .execute(&mut *self.tx)
            .await
        } else {
            sqlx::query!(
                "UPDATE preguntas_frecuentes SET pregunta = $2, respuesta = $3, orden = $4,
                        activa = $5, archivo_id = $6, actualizado_en = now()
                 WHERE id = $1",
                p.id.as_uuid(),
                p.pregunta,
                p.respuesta,
                p.orden,
                p.activa,
                p.archivo_id.map(ArchivoId::as_uuid),
            )
            .execute(&mut *self.tx)
            .await
        };
        r.map_err(|e| error_escritura(e, "pregunta repetida", "archivo"))?;
        Ok(())
    }

    async fn conversacion_abierta(
        &mut self,
        linea: &str,
        telefono: &str,
    ) -> AppResult<Option<Conversacion>> {
        sqlx::query_as!(
            FilaConversacion,
            "SELECT id, linea, telefono, persona_id, estado, atendida_por, abierta_en,
                    ultimo_entrante_en, cerrada_en
             FROM conversaciones WHERE linea = $1 AND telefono = $2 AND estado = 'abierta'",
            linea,
            telefono,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Conversacion::try_from)
        .transpose()
    }

    async fn conversacion(&mut self, id: ConversacionId) -> AppResult<Option<Conversacion>> {
        sqlx::query_as!(
            FilaConversacion,
            "SELECT id, linea, telefono, persona_id, estado, atendida_por, abierta_en,
                    ultimo_entrante_en, cerrada_en
             FROM conversaciones WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Conversacion::try_from)
        .transpose()
    }

    async fn insertar_conversacion(&mut self, c: &Conversacion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO conversaciones (id, tenant_id, linea, telefono, persona_id, estado,
                                         atendida_por, abierta_en, ultimo_entrante_en, cerrada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.linea,
            c.telefono,
            c.persona_id.map(PersonaId::as_uuid),
            c.estado.as_str(),
            c.atendida_por.map(CuentaId::as_uuid),
            c.abierta_en,
            c.ultimo_entrante_en,
            c.cerrada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "ya hay una conversación abierta con ese teléfono",
                "persona",
            )
        })?;
        Ok(())
    }

    async fn actualizar_conversacion(&mut self, c: &Conversacion) -> AppResult<()> {
        sqlx::query!(
            "UPDATE conversaciones SET estado = $2, atendida_por = $3, ultimo_entrante_en = $4,
                    cerrada_en = $5
             WHERE id = $1",
            c.id.as_uuid(),
            c.estado.as_str(),
            c.atendida_por.map(CuentaId::as_uuid),
            c.ultimo_entrante_en,
            c.cerrada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn conversaciones(
        &mut self,
        estado: Option<EstadoConversacion>,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<Conversacion>> {
        let filas = sqlx::query_as!(
            FilaConversacion,
            "SELECT id, linea, telefono, persona_id, estado, atendida_por, abierta_en,
                    ultimo_entrante_en, cerrada_en
             FROM conversaciones
             WHERE ($1::text IS NULL OR estado = $1) AND ($2::uuid IS NULL OR id < $2)
             ORDER BY id DESC LIMIT $3",
            estado.map(EstadoConversacion::as_str),
            pagina.despues(),
            pagina.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Conversacion::try_from)
        .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(filas, pagina, |c| c.id.as_uuid()))
    }

    async fn insertar_mensaje_conversacion(&mut self, m: &MensajeConversacion) -> AppResult<bool> {
        let insertadas = sqlx::query!(
            "INSERT INTO conversacion_mensajes (id, tenant_id, conversacion_id, direccion, texto,
                                                autor_cuenta_id, wamid, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (wamid) WHERE wamid IS NOT NULL DO NOTHING",
            Uuid::now_v7(),
            self.ctx.tenant_id.as_uuid(),
            m.conversacion_id.as_uuid(),
            if m.entrante { "entrante" } else { "saliente" },
            m.texto,
            m.autor_cuenta_id.map(CuentaId::as_uuid),
            m.wamid,
            m.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "mensaje repetido", "conversación"))?
        .rows_affected();
        Ok(insertadas == 1)
    }

    async fn mensajes_conversacion(
        &mut self,
        id: ConversacionId,
    ) -> AppResult<Vec<MensajeConversacion>> {
        Ok(sqlx::query!(
            "SELECT direccion, texto, autor_cuenta_id, wamid, creado_en
             FROM conversacion_mensajes WHERE conversacion_id = $1 ORDER BY creado_en, id",
            id.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| MensajeConversacion {
            conversacion_id: id,
            entrante: r.direccion == "entrante",
            texto: r.texto,
            autor_cuenta_id: r.autor_cuenta_id.map(CuentaId::from_uuid),
            wamid: r.wamid,
            creado_en: r.creado_en,
        })
        .collect())
    }
}

/// Directorio del worker de gestión (función `copropiedades_con_gestion_pendiente`).
#[derive(Debug, Clone)]
pub struct DirectorioGestionPg {
    pool: PgPool,
}

impl DirectorioGestionPg {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl application::puertos::DirectorioGestion for DirectorioGestionPg {
    async fn copropiedades_con_gestion_pendiente(&self) -> AppResult<Vec<TenantId>> {
        Ok(sqlx::query_scalar!(
            r#"SELECT tenant_id AS "tenant_id!" FROM copropiedades_con_gestion_pendiente()"#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TenantId::from_uuid)
        .collect())
    }
}
