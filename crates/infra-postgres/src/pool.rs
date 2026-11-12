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
///
/// # Errors
///
/// [`InfraPgError`] si no conecta o una migración falla (la que falla no queda aplicada).
pub async fn migrar(url_dueno: &str) -> Result<(), InfraPgError> {
    let pool = pool_dueno(url_dueno).await?;
    let resultado = sqlx::migrate!("../../migrations").run(&pool).await;
    pool.close().await;
    Ok(resultado?)
}

/// Revierte con el **dueño del esquema** las migraciones aplicadas posteriores a `version`,
/// de la más nueva a la más vieja (sus `.down.sql`), y cierra la conexión.
///
/// Es **destructivo**: un `.down.sql` puede borrar tablas o columnas con datos. Se usa solo
/// para deshacer un despliegue fallido y después de un respaldo (`docs/despliegue.md`).
///
/// # Errors
///
/// [`InfraPgError`] si no conecta o una reversión falla.
pub async fn revertir_hasta(url_dueno: &str, version: i64) -> Result<(), InfraPgError> {
    let pool = pool_dueno(url_dueno).await?;
    let resultado = revertir_en(&pool, version).await;
    pool.close().await;
    resultado
}

/// Como [`revertir_hasta`], sobre un pool ya abierto con el dueño del esquema.
///
/// # Errors
///
/// [`InfraPgError::Migracion`] si una reversión falla.
pub async fn revertir_en(pool: &PgPool, version: i64) -> Result<(), InfraPgError> {
    sqlx::migrate!("../../migrations")
        .undo(pool, version)
        .await?;
    Ok(())
}

async fn pool_dueno(url_dueno: &str) -> Result<PgPool, InfraPgError> {
    Ok(PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url_dueno)
        .await?)
}

/// Sonda de disponibilidad (readiness).
pub async fn ping(pool: &PgPool) -> Result<(), InfraPgError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await?;
    Ok(())
}
