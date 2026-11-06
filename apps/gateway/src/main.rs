//! Gateway de WhatsApp de DAD_COPROPIEDAD.
//!
//! Responsabilidades (ver `AGENTS.md`):
//! - Webhook de Meta: verificación de suscripción y de firma `X-Hub-Signature-256`.
//! - Enrutamiento: sesión por (línea, teléfono) resuelta contra las reuniones activas.
//! - Cola de salida (outbox) con prioridad y limitador de 80 mps por línea.
//! - Al comienzo también hace de worker (envíos y actas); se separa solo si la carga lo pide.

mod routes;
mod state;

use std::sync::Arc;

use anyhow::Context;
use shared::{AppConfig, WhatsappConfig, telemetry};
use tokio::net::TcpListener;

use crate::state::GatewayState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::cargar("GATEWAY_PORT", 8081).context("configuración inválida")?;
    let whatsapp = WhatsappConfig::from_env().context("configuración de WhatsApp inválida")?;
    telemetry::init(config.log_format);
    tracing::info!(
        ?config,
        "iniciando dad_copropiedad_gateway v{}",
        env!("CARGO_PKG_VERSION")
    );

    let db = infra_postgres::conectar(&config.database_url, config.database_max_connections)
        .await
        .context("no se pudo conectar a PostgreSQL (¿está arriba `docker compose up -d`?)")?;
    infra_postgres::verificar_rol_restringido(&db)
        .await
        .context("APP_DATABASE_URL debe usar el rol restringido `dad_app`")?;

    let state = GatewayState {
        db,
        whatsapp: Arc::new(whatsapp),
    };
    let app = routes::router(state, &config);
    let addr = config.socket_addr()?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("no se pudo abrir el puerto {addr}"))?;

    tracing::info!(%addr, "gateway escuchando");
    axum::serve(listener, app)
        .with_graceful_shutdown(shared::apagado::senal_apagado())
        .await
        .context("error del servidor HTTP")?;

    tracing::info!("gateway detenido correctamente");
    Ok(())
}
