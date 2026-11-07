//! Ayudantes compartidos por las pruebas de integración de `infra-postgres`.

use domain::TenantId;
use sqlx::{Executor, PgPool, postgres::PgPoolOptions};

/// Migraciones del repositorio (las aplica `#[sqlx::test]` en una BD aislada por prueba).
pub const MIGRACIONES: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Pool sobre la misma BD de la prueba, pero con el rol restringido de la aplicación
/// (`SET ROLE dad_app` en cada conexión), igual que en producción.
pub async fn pool_como_app(pool: &PgPool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|conn, _| {
            Box::pin(async move {
                conn.execute("SET ROLE dad_app").await?;
                Ok(())
            })
        })
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("pool con rol dad_app")
}

/// Crea una copropiedad (tenant) con el dueño del esquema.
pub async fn crear_copropiedad(pool: &PgPool) -> TenantId {
    let tenant = TenantId::nuevo();
    sqlx::query("INSERT INTO copropiedades (id, nombre) VALUES ($1, 'PH de prueba')")
        .bind(tenant.as_uuid())
        .execute(pool)
        .await
        .expect("copropiedad");
    tenant
}
