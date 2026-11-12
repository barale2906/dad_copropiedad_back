//! Unidad de trabajo sobre PostgreSQL: cada [`TransaccionPg`] es una transacción con
//! `app.tenant_id` fijado ([`transaccion_tenant`]) que implementa los puertos de persistencia
//! que se usan dentro de ella: auditoría y outbox.

use application::{
    AppError, AppResult, Contexto,
    auditoria::EventoAuditoria,
    outbox::EventoSaliente,
    puertos::{
        RegistroAuditoria, RegistroOutbox, Transaccion, UnidadDeTrabajo,
        auditoria::{FiltroAuditoria, LecturaAuditoria, RegistroAuditado},
    },
};
use sqlx::{PgPool, Postgres};

use crate::transaccion_tenant;

/// Implementación de [`UnidadDeTrabajo`] con el pool de la aplicación (rol `dad_app`).
#[derive(Debug, Clone)]
pub struct UnidadDeTrabajoPg {
    pool: PgPool,
}

impl UnidadDeTrabajoPg {
    /// Unidad de trabajo sobre el pool de la app (`dad_app`).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Transacción abierta en el tenant del contexto. Soltarla sin confirmar hace ROLLBACK.
#[derive(Debug)]
pub struct TransaccionPg {
    pub(crate) tx: sqlx::Transaction<'static, Postgres>,
    pub(crate) ctx: Contexto,
}

impl UnidadDeTrabajo for UnidadDeTrabajoPg {
    type Transaccion = TransaccionPg;

    async fn iniciar(&self, ctx: &Contexto) -> AppResult<TransaccionPg> {
        let tx = transaccion_tenant(&self.pool, ctx.tenant_id)
            .await
            .map_err(AppError::infra)?;
        Ok(TransaccionPg {
            tx,
            ctx: ctx.clone(),
        })
    }
}

impl Transaccion for TransaccionPg {
    async fn confirmar(self) -> AppResult<()> {
        self.tx.commit().await.map_err(|e| {
            // Restricciones diferidas que se evalúan al confirmar (p. ej. los titulares de un
            // bien privado). El mensaje del trigger no lleva datos personales.
            match e.as_database_error() {
                Some(d) if d.code().is_some_and(|c| c.starts_with('V')) => {
                    AppError::Conflicto(d.message().to_owned())
                }
                _ => AppError::infra(e),
            }
        })
    }
}

impl RegistroAuditoria for TransaccionPg {
    async fn registrar(&mut self, evento: EventoAuditoria) -> AppResult<()> {
        // `ocurrido_en` lo pone la BD (hora de la transacción): es la hora oficial del registro.
        sqlx::query!(
            "INSERT INTO auditoria
                 (tenant_id, actor_id, accion, entidad, entidad_id, resultado, detalle, request_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            self.ctx.tenant_id.as_uuid(),
            self.ctx.actor_id.map(|a| a.as_uuid()),
            evento.accion.as_str(),
            evento.entidad,
            evento.entidad_id,
            evento.resultado.as_str(),
            evento.detalle.a_json(),
            self.ctx.request_id.as_deref(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

impl RegistroOutbox for TransaccionPg {
    async fn encolar(&mut self, evento: EventoSaliente) -> AppResult<()> {
        let payload = serde_json::to_value(&evento.mensaje).map_err(AppError::infra)?;
        sqlx::query!(
            "INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload)
             VALUES ($1, $2, $3, $4, $5, $6)",
            self.ctx.tenant_id.as_uuid(),
            evento.mensaje.canal().as_str(),
            evento.prioridad.as_i16(),
            evento.mensaje.linea(),
            evento.mensaje.destino(),
            payload,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

impl LecturaAuditoria for TransaccionPg {
    async fn consultar_auditoria(
        &mut self,
        f: &FiltroAuditoria,
        antes_de: Option<i64>,
        limite: i64,
    ) -> AppResult<Vec<RegistroAuditado>> {
        // Un prefijo terminado en punto filtra por módulo (`acceso.`); el LIKE se arma escapando
        // los comodines para que la entrada del usuario no los use.
        let (exacta, prefijo) = match f.accion.as_deref() {
            Some(a) if a.ends_with('.') => (
                None,
                Some(format!(
                    "{}%",
                    a.replace('\\', "\\\\")
                        .replace('%', "\\%")
                        .replace('_', "\\_")
                )),
            ),
            Some(a) => (Some(a.to_owned()), None),
            None => (None, None),
        };
        let filas = sqlx::query!(
            "SELECT id, ocurrido_en, actor_id, accion, entidad, entidad_id, resultado, detalle,
                    request_id
             FROM auditoria
             WHERE ($1::text IS NULL OR entidad = $1)
               AND ($2::uuid IS NULL OR entidad_id = $2)
               AND ($3::text IS NULL OR accion = $3)
               AND ($4::text IS NULL OR accion LIKE $4)
               AND ($5::uuid IS NULL OR actor_id = $5)
               AND ($6::timestamptz IS NULL OR ocurrido_en >= $6)
               AND ($7::timestamptz IS NULL OR ocurrido_en < $7)
               AND ($8::bigint IS NULL OR id < $8)
             ORDER BY id DESC LIMIT $9",
            f.entidad,
            f.entidad_id,
            exacta,
            prefijo,
            f.actor_id.map(domain::CuentaId::as_uuid),
            f.desde,
            f.hasta,
            antes_de,
            limite,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(filas
            .into_iter()
            .map(|r| RegistroAuditado {
                id: r.id,
                ocurrido_en: r.ocurrido_en,
                actor_id: r.actor_id.map(domain::CuentaId::from_uuid),
                accion: r.accion,
                entidad: r.entidad,
                entidad_id: r.entidad_id,
                resultado: r.resultado,
                detalle: r.detalle,
                request_id: r.request_id,
            })
            .collect())
    }
}
