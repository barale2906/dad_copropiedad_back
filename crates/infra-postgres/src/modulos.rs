//! Repositorio de módulos habilitados (`modulos_habilitados`) sobre la transacción del tenant.

use application::{
    AppError, AppResult,
    puertos::{RepositorioModulos, modulos::PeriodoModulo},
};
use chrono::{DateTime, Utc};
use domain::Modulo;
use uuid::Uuid;

use crate::TransaccionPg;

impl RepositorioModulos for TransaccionPg {
    async fn modulo_habilitado(&mut self, modulo: Modulo, ahora: DateTime<Utc>) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (
                   SELECT 1 FROM modulos_habilitados
                   WHERE modulo = $1 AND desde <= $2 AND (hasta IS NULL OR $2 < hasta)
               ) AS "habilitado!""#,
            modulo.as_str(),
            ahora,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn modulos_habilitados(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<Modulo>> {
        sqlx::query_scalar!(
            "SELECT DISTINCT modulo FROM modulos_habilitados
             WHERE desde <= $1 AND (hasta IS NULL OR $1 < hasta)",
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .iter()
        .map(|m| Ok(Modulo::try_from(m.as_str())?))
        .collect()
    }

    async fn insertar_periodo(&mut self, p: &PeriodoModulo) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde, hasta)
             VALUES ($1, $2, $3, $4, $5)",
            Uuid::now_v7(),
            self.ctx.tenant_id.as_uuid(),
            p.modulo.as_str(),
            p.desde,
            p.hasta,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            crate::copropiedades::error_escritura(e, "periodo de módulo repetido", "copropiedad")
        })?;
        Ok(())
    }

    async fn terminar_periodos(&mut self, modulo: Modulo, hasta: DateTime<Utc>) -> AppResult<u64> {
        // Los periodos que empiezan después de `hasta` no se tocan (cerrarlos violaría
        // `hasta > desde`); quien termina un módulo los revisa aparte.
        let filas = sqlx::query!(
            "UPDATE modulos_habilitados SET hasta = $2
             WHERE modulo = $1 AND desde < $2 AND (hasta IS NULL OR hasta > $2)",
            modulo.as_str(),
            hasta,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        Ok(filas)
    }
}
