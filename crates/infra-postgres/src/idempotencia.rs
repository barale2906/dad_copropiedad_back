//! Peticiones idempotentes de la API en PostgreSQL (plan 10, fase 6; ámbito de plataforma).

use application::{
    AppError, AppResult,
    puertos::{
        RegistroIdempotencia,
        idempotencia::{Reserva, RespuestaGuardada, VIGENCIA_IDEMPOTENCIA},
    },
};
use chrono::{DateTime, Utc};
use domain::{ClaveIdempotencia, CuentaId};
use sqlx::{PgPool, Postgres, Transaction};

async fn plataforma(pool: &PgPool) -> AppResult<Transaction<'static, Postgres>> {
    let mut tx = pool.begin().await.map_err(AppError::infra)?;
    sqlx::query("SELECT set_config('app.plataforma', 'on', true)")
        .execute(&mut *tx)
        .await
        .map_err(AppError::infra)?;
    Ok(tx)
}

/// Registro de `Idempotency-Key` (rol `dad_app`).
#[derive(Debug, Clone)]
pub struct IdempotenciaPg {
    pool: PgPool,
}

impl IdempotenciaPg {
    /// Registro sobre el pool de la app (`dad_app`).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl RegistroIdempotencia for IdempotenciaPg {
    async fn reservar(
        &self,
        cuenta: CuentaId,
        clave: &ClaveIdempotencia,
        huella: &str,
        ahora: DateTime<Utc>,
    ) -> AppResult<Reserva> {
        let mut tx = plataforma(&self.pool).await?;
        sqlx::query!(
            "DELETE FROM peticiones_idempotentes
             WHERE cuenta_id = $1 AND clave = $2 AND creada_en <= $3",
            cuenta.as_uuid(),
            clave.as_str(),
            ahora - VIGENCIA_IDEMPOTENCIA,
        )
        .execute(&mut *tx)
        .await
        .map_err(AppError::infra)?;
        let nueva = sqlx::query!(
            "INSERT INTO peticiones_idempotentes (cuenta_id, clave, huella, creada_en)
             VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
            cuenta.as_uuid(),
            clave.as_str(),
            huella,
            ahora,
        )
        .execute(&mut *tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected()
            == 1;
        let reserva = if nueva {
            Reserva::Nueva
        } else {
            let f = sqlx::query!(
                "SELECT huella, estado, tipo, cuerpo FROM peticiones_idempotentes
                 WHERE cuenta_id = $1 AND clave = $2",
                cuenta.as_uuid(),
                clave.as_str(),
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(AppError::infra)?;
            match (f.huella == huella, f.estado) {
                (false, _) => Reserva::OtraPeticion,
                (true, None) => Reserva::EnCurso,
                (true, Some(estado)) => Reserva::Repetida(RespuestaGuardada {
                    estado: u16::try_from(estado).map_err(AppError::infra)?,
                    tipo: f.tipo,
                    cuerpo: f.cuerpo.unwrap_or_default(),
                }),
            }
        };
        tx.commit().await.map_err(AppError::infra)?;
        Ok(reserva)
    }

    async fn guardar(
        &self,
        cuenta: CuentaId,
        clave: &ClaveIdempotencia,
        r: &RespuestaGuardada,
    ) -> AppResult<()> {
        let mut tx = plataforma(&self.pool).await?;
        sqlx::query!(
            "UPDATE peticiones_idempotentes SET estado = $3, tipo = $4, cuerpo = $5
             WHERE cuenta_id = $1 AND clave = $2",
            cuenta.as_uuid(),
            clave.as_str(),
            i16::try_from(r.estado).map_err(AppError::infra)?,
            r.tipo,
            &r.cuerpo,
        )
        .execute(&mut *tx)
        .await
        .map_err(AppError::infra)?;
        tx.commit().await.map_err(AppError::infra)
    }

    async fn liberar(&self, cuenta: CuentaId, clave: &ClaveIdempotencia) -> AppResult<()> {
        let mut tx = plataforma(&self.pool).await?;
        sqlx::query!(
            "DELETE FROM peticiones_idempotentes WHERE cuenta_id = $1 AND clave = $2",
            cuenta.as_uuid(),
            clave.as_str(),
        )
        .execute(&mut *tx)
        .await
        .map_err(AppError::infra)?;
        tx.commit().await.map_err(AppError::infra)
    }

    async fn purgar(&self, ahora: DateTime<Utc>) -> AppResult<u64> {
        let mut tx = plataforma(&self.pool).await?;
        let n = sqlx::query!(
            "DELETE FROM peticiones_idempotentes WHERE creada_en <= $1",
            ahora - VIGENCIA_IDEMPOTENCIA,
        )
        .execute(&mut *tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        tx.commit().await.map_err(AppError::infra)?;
        Ok(n)
    }
}
