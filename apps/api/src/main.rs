//! Punto de entrada del servicio HTTP de DAD_COPROPIEDAD.

use anyhow::Context;
use shared::{AppConfig, telemetry};
use tokio::net::TcpListener;

use api::{routes, state::AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::from_env().context("configuración inválida")?;
    telemetry::init(config.log_format);
    tracing::info!(
        ?config,
        "iniciando dad_copropiedad_api v{}",
        env!("CARGO_PKG_VERSION")
    );

    if config.run_migrations
        && let Some(url) = &config.migrations_database_url
    {
        infra_postgres::migrar(url)
            .await
            .context("falló la ejecución de migraciones")?;
        tracing::info!("migraciones aplicadas");
    }

    let db = infra_postgres::conectar(&config.database_url, config.database_max_connections)
        .await
        .context("no se pudo conectar a PostgreSQL (¿está arriba `docker compose up -d`?)")?;
    infra_postgres::verificar_rol_restringido(&db)
        .await
        .context("APP_DATABASE_URL debe usar el rol restringido `dad_app`")?;

    let app = routes::router(AppState { db }, &config);
    let addr = config.socket_addr()?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("no se pudo abrir el puerto {addr}"))?;

    tracing::info!(%addr, "servidor escuchando");
    axum::serve(listener, app)
        .with_graceful_shutdown(shared::apagado::senal_apagado())
        .await
        .context("error del servidor HTTP")?;

    tracing::info!("servidor detenido correctamente");
    Ok(())
}
