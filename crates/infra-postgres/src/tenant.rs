use domain::TenantId;
use sqlx::{PgPool, Postgres, Transaction};

/// Inicia una transacción con `app.tenant_id` fijado **solo para esa transacción**
/// (`set_config(…, true)` equivale a `SET LOCAL`, pero admite parámetros enlazados).
///
/// Al hacer COMMIT/ROLLBACK el valor desaparece, así que una conexión devuelta al pool nunca
/// arrastra el tenant de la petición anterior.
pub async fn transaccion_tenant(
    pool: &PgPool,
    tenant: TenantId,
) -> Result<Transaction<'static, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await?;
    tracing::trace!(tenant_id = %tenant, "transacción de tenant iniciada");
    Ok(tx)
}
