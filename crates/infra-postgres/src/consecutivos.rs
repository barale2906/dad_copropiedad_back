//! Consecutivos legibles por serie y año (`consecutivos`): tickets, órdenes de trabajo y tipos
//! de comprobante (`comprobante:<código>`). Atómico dentro de la transacción del tenant.

use application::{AppError, AppResult};

use crate::TransaccionPg;

impl TransaccionPg {
    /// Siguiente número de la serie en el año.
    pub(crate) async fn siguiente_en_serie(&mut self, serie: &str, anio: i32) -> AppResult<u32> {
        let anio = i16::try_from(anio)
            .map_err(|_| AppError::Validacion("`anio` fuera de rango".into()))?;
        let ultimo = sqlx::query_scalar!(
            "INSERT INTO consecutivos (tenant_id, serie, anio, ultimo) VALUES ($1, $2, $3, 1)
             ON CONFLICT (tenant_id, serie, anio) DO UPDATE SET ultimo = consecutivos.ultimo + 1
             RETURNING ultimo",
            self.ctx.tenant_id.as_uuid(),
            serie,
            anio,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        u32::try_from(ultimo).map_err(|_| AppError::Validacion("consecutivo fuera de rango".into()))
    }
}
