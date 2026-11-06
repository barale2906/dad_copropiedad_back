use std::time::Duration;

use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::InfraPgError;

/// Abre el pool de la aplicación.
pub async fn conectar(url: &str, max_conexiones: u32) -> Result<PgPool, InfraPgError> {
    Ok(PgPoolOptions::new()
        .max_connections(max_conexiones)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await?)
}

/// Falla si el rol conectado puede saltarse RLS (superusuario o `BYPASSRLS`).
pub async fn verificar_rol_restringido(pool: &PgPool) -> Result<(), InfraPgError> {
    let (rol, privilegiado): (String, bool) = sqlx::query_as(
        "SELECT rolname::text, (rolsuper OR rolbypassrls) FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await?;
    if privilegiado {
        return Err(InfraPgError::RolPrivilegiado(rol));
    }
    Ok(())
}

/// Aplica las migraciones pendientes con una conexión del **dueño del esquema** y la cierra.
pub async fn migrar(url_dueno: &str) -> Result<(), InfraPgError> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url_dueno)
        .await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    pool.close().await;
    Ok(())
}

/// Sonda de disponibilidad (readiness).
pub async fn ping(pool: &PgPool) -> Result<(), InfraPgError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await?;
    Ok(())
}
