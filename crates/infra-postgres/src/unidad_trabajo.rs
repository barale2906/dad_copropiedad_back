//! Unidad de trabajo sobre PostgreSQL: cada [`TransaccionPg`] es una transacción con
//! `app.tenant_id` fijado ([`transaccion_tenant`]) que implementa los puertos de persistencia
//! que se usan dentro de ella, empezando por la auditoría.

use application::{
    AppError, AppResult, Contexto,
    auditoria::EventoAuditoria,
    puertos::{RegistroAuditoria, Transaccion, UnidadDeTrabajo},
};
use sqlx::{PgPool, Postgres};

use crate::transaccion_tenant;

/// Implementación de [`UnidadDeTrabajo`] con el pool de la aplicación (rol `dad_app`).
#[derive(Debug, Clone)]
pub struct UnidadDeTrabajoPg {
    pool: PgPool,
}

impl UnidadDeTrabajoPg {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Transacción abierta en el tenant del contexto. Soltarla sin confirmar hace ROLLBACK.
#[derive(Debug)]
pub struct TransaccionPg {
    tx: sqlx::Transaction<'static, Postgres>,
    ctx: Contexto,
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
        self.tx.commit().await.map_err(AppError::infra)
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
