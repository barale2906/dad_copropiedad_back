//! Gateway de WhatsApp de DAD_COPROPIEDAD.
//!
//! Responsabilidades (ver `AGENTS.md`):
//! - Webhook de Meta: verificación de suscripción y de firma `X-Hub-Signature-256`.
//! - Enrutamiento: sesión por (línea, teléfono) resuelta contra las reuniones activas.
//! - Cola de salida (outbox) con prioridad y limitador de 80 mps por línea.
//! - Al comienzo también hace de worker (envíos y actas); se separa solo si la carga lo pide.
//!   Hoy despacha correo ([`worker_salida`]); WhatsApp se suma con su cliente de envío.
//! - Encola los recordatorios de reservas ([`worker_recordatorios`]).

mod routes;
mod state;
mod worker_recordatorios;
mod worker_salida;

use std::sync::Arc;

use anyhow::Context;
use infra_correo::CorreoSmtp;
use infra_postgres::{ColaSalidaPg, DirectorioRecordatoriosPg, UnidadDeTrabajoPg};
use shared::{AppConfig, SmtpConfig, WhatsappConfig, telemetry};
use tokio::{net::TcpListener, sync::watch};

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

    // Una sola señal de apagado para el servidor HTTP y el worker.
    let (avisar_apagado, apagado) = watch::channel(false);
    tokio::spawn(async move {
        shared::apagado::senal_apagado().await;
        let _ = avisar_apagado.send(true);
    });

    let worker = match SmtpConfig::from_env().context("configuración SMTP inválida")? {
        Some(smtp) => {
            tracing::info!(?smtp, "envío de correo activado");
            let correo = CorreoSmtp::desde_config(&smtp).context("cliente SMTP inválido")?;
            Some(tokio::spawn(worker_salida::ejecutar(
                ColaSalidaPg::new(db.clone()),
                correo,
                esperar_apagado(apagado.clone()),
            )))
        }
        None => {
            tracing::warn!("SMTP_HOST no está definido: el envío de correo queda desactivado");
            None
        }
    };

    let recordatorios = tokio::spawn(worker_recordatorios::ejecutar(
        DirectorioRecordatoriosPg::new(db.clone()),
        UnidadDeTrabajoPg::new(db.clone()),
        esperar_apagado(apagado.clone()),
    ));

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
        .with_graceful_shutdown(esperar_apagado(apagado))
        .await
        .context("error del servidor HTTP")?;

    if let Some(worker) = worker
        && let Err(e) = worker.await
    {
        tracing::error!(error = %e, "el worker de envíos terminó con error");
    }

    if let Err(e) = recordatorios.await {
        tracing::error!(error = %e, "el worker de recordatorios terminó con error");
    }

    tracing::info!("gateway detenido correctamente");
    Ok(())
}

/// Se resuelve cuando llega la señal de apagado (o si el emisor desaparece).
async fn esperar_apagado(mut apagado: watch::Receiver<bool>) {
    let _ = apagado.wait_for(|apagar| *apagar).await;
}
