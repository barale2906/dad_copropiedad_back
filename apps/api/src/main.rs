//! Punto de entrada del servicio HTTP de DAD_COPROPIEDAD.

use anyhow::Context;
use infra_archivos::AlmacenLocal;
use shared::{AppConfig, ArchivosConfig, SeguridadConfig, telemetry};
use tokio::net::TcpListener;

use api::{en_vivo, routes, state::AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::from_env().context("configuración inválida")?;
    let config_archivos =
        ArchivosConfig::from_env().context("configuración de archivos inválida")?;
    telemetry::init(config.log_format);
    tracing::info!(
        ?config,
        ?config_archivos,
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

    let archivos =
        AlmacenLocal::desde_config(&config_archivos).context("almacén de archivos inválido")?;

    let seguridad = SeguridadConfig::from_env().context("configuración de seguridad inválida")?;
    if !seguridad.plataforma_exige_segundo_factor {
        tracing::warn!("PLATAFORMA_EXIGE_SEGUNDO_FACTOR=false: solo aceptable en desarrollo");
    }
    let state = AppState::new(db, archivos, seguridad.plataforma_exige_segundo_factor);
    // Tablero en vivo: oyente de `pg_notify` y emisor cada ~500 ms, hasta el apagado.
    let (avisar_apagado, apagado) = tokio::sync::watch::channel(false);
    let esperar = |mut rx: tokio::sync::watch::Receiver<bool>| async move {
        let _ = rx.wait_for(|a| *a).await;
    };
    let oyente = tokio::spawn(en_vivo::escuchar(
        state.db.clone(),
        std::sync::Arc::clone(&state.en_vivo),
        esperar(apagado.clone()),
    ));
    let emisor = tokio::spawn(en_vivo::emitir(
        std::sync::Arc::clone(&state.en_vivo),
        state.unidad.clone(),
        esperar(apagado.clone()),
    ));
    let app = routes::router(state, &config);
    let addr = config.socket_addr()?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("no se pudo abrir el puerto {addr}"))?;

    tracing::info!(%addr, "servidor escuchando");
    axum::serve(listener, app)
        .with_graceful_shutdown(shared::apagado::senal_apagado())
        .await
        .context("error del servidor HTTP")?;
    let _ = avisar_apagado.send(true);
    for t in [oyente, emisor] {
        if let Err(e) = t.await {
            tracing::error!(error = %e, "una tarea del tablero en vivo terminó con error");
        }
    }

    tracing::info!("servidor detenido correctamente");
    Ok(())
}
