//! Gateway de WhatsApp de DAD_COPROPIEDAD.
//!
//! Responsabilidades (ver `AGENTS.md`):
//! - Webhook de Meta: verificación de suscripción y de firma `X-Hub-Signature-256`.
//! - Enrutamiento: sesión por (línea, teléfono) resuelta contra las reuniones activas.
//! - Cola de salida (outbox) con prioridad y limitador de 80 mps por línea.
//! - Chat de WhatsApp de la copropiedad (`casos_uso::chat`): menú, reportes, reservas y bandeja
//!   humana; las respuestas salen por el outbox.
//! - Al comienzo también hace de worker (envíos y actas); se separa solo si la carga lo pide.
//!   Despacha correo y WhatsApp ([`worker_salida`]) con el ritmo por línea de [`mensajeria`].
//! - Encola los recordatorios de reservas ([`worker_recordatorios`]) y ejecuta las tareas de
//!   tickets y mantenimiento ([`worker_gestion`]).
//! - Asambleas por WhatsApp (`casos_uso::asambleas::whatsapp`) antes del chat: confirmar
//!   asistencia y votar con botones cuyo id lleva la pregunta; registra los acuses de entrega y
//!   la calidad de las líneas; cierra las preguntas con temporizador ([`worker_asambleas`]).
//! - Control de acceso (`casos_uso::acceso`): botones «Autorizar» / «Rechazar» de las visitas
//!   antes que la asamblea; escalamiento de visitas y correspondencia y retención
//!   ([`worker_acceso`]); push a las apps con Firebase (respaldo por WhatsApp).

mod mensajeria;
mod routes;
mod state;
mod worker_acceso;
mod worker_asambleas;
mod worker_gestion;
mod worker_recordatorios;
mod worker_salida;

use std::sync::Arc;

use anyhow::Context;
use infra_archivos::AlmacenLocal;
use infra_correo::CorreoSmtp;
use infra_postgres::{
    ColaSalidaPg, DirectorioAccesoPg, DirectorioAsambleasPg, DirectorioGestionPg,
    DirectorioRecordatoriosPg, DirectorioTelefonosPg, SesionesChatPg, UnidadDeTrabajoPg,
};
use infra_push::ClienteFcm;
use infra_whatsapp::ClienteWhatsapp;
use shared::{
    AccesoConfig, AppConfig, ArchivosConfig, PushConfig, SmtpConfig, WhatsappConfig, telemetry,
};
use tokio::{
    net::TcpListener,
    sync::{Notify, watch},
};

use crate::{mensajeria::MensajeriaGateway, state::GatewayState};

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

    let correo = match SmtpConfig::from_env().context("configuración SMTP inválida")? {
        Some(smtp) => {
            tracing::info!(?smtp, "envío de correo activado");
            Some(CorreoSmtp::desde_config(&smtp).context("cliente SMTP inválido")?)
        }
        None => {
            tracing::warn!("SMTP_HOST no está definido: el envío de correo queda desactivado");
            None
        }
    };
    let cliente_whatsapp = match &whatsapp.access_token {
        Some(token) => Some(
            ClienteWhatsapp::new(&whatsapp.api_url, token)
                .context("cliente de WhatsApp Cloud API inválido")?,
        ),
        None => {
            tracing::warn!(
                "WHATSAPP_ACCESS_TOKEN no está definido: no se envían mensajes de WhatsApp"
            );
            None
        }
    };
    let mensajeria = MensajeriaGateway::new(cliente_whatsapp);
    let archivos = AlmacenLocal::desde_config(
        &ArchivosConfig::from_env().context("configuración de archivos inválida")?,
    )
    .context("almacén de archivos inválido")?;
    let acceso = AccesoConfig::from_env().context("configuración de control de acceso inválida")?;
    let firma_qr = Arc::new(
        infra_seguridad::FirmaQrEd25519::new(acceso.qr_secreto.as_bytes())
            .context("ACCESO_QR_SECRETO inválido")?,
    );
    let push = PushConfig::from_env().context("configuración de push inválida")?;
    let fcm = match &push.credenciales {
        Some(ruta) => {
            let c = ClienteFcm::desde_archivo(ruta, &push.api_url)
                .context("cuenta de servicio de Firebase (FCM_CREDENCIALES) inválida")?;
            tracing::info!("envío push (Firebase Cloud Messaging) activado");
            Some(c)
        }
        None => {
            tracing::warn!(
                "FCM_CREDENCIALES no está definido: los avisos de portería salen por WhatsApp"
            );
            None
        }
    };
    let archivos = Arc::new(archivos);
    let hay_salida = Arc::new(Notify::new());

    let worker = (correo.is_some() || mensajeria.activa() || fcm.is_some()).then(|| {
        let cola = ColaSalidaPg::new(db.clone());
        let whatsapp = mensajeria.activa().then(|| mensajeria.clone());
        let aviso = Arc::clone(&hay_salida);
        let apagado = esperar_apagado(apagado.clone());
        tokio::spawn(async move {
            worker_salida::ejecutar(cola, correo, whatsapp, fcm, &aviso, apagado).await;
        })
    });

    let acceso = tokio::spawn(worker_acceso::ejecutar(
        DirectorioAccesoPg::new(db.clone()),
        UnidadDeTrabajoPg::new(db.clone()),
        Arc::clone(&archivos),
        Arc::clone(&hay_salida),
        esperar_apagado(apagado.clone()),
    ));

    let recordatorios = tokio::spawn(worker_recordatorios::ejecutar(
        DirectorioRecordatoriosPg::new(db.clone()),
        UnidadDeTrabajoPg::new(db.clone()),
        esperar_apagado(apagado.clone()),
    ));

    let gestion = tokio::spawn(worker_gestion::ejecutar(
        DirectorioGestionPg::new(db.clone()),
        UnidadDeTrabajoPg::new(db.clone()),
        SesionesChatPg::new(db.clone()),
        infra_postgres::IdempotenciaPg::new(db.clone()),
        esperar_apagado(apagado.clone()),
    ));

    let asambleas = tokio::spawn(worker_asambleas::ejecutar(
        DirectorioAsambleasPg::new(db.clone()),
        UnidadDeTrabajoPg::new(db.clone()),
        Arc::clone(&hay_salida),
        esperar_apagado(apagado.clone()),
    ));

    let state = GatewayState {
        unidad: UnidadDeTrabajoPg::new(db.clone()),
        sesiones: SesionesChatPg::new(db.clone()),
        directorio: DirectorioTelefonosPg::new(db.clone()),
        asambleas: DirectorioAsambleasPg::new(db.clone()),
        mensajeria,
        archivos,
        hay_salida,
        firma_qr,
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

    if let Err(e) = gestion.await {
        tracing::error!(error = %e, "el worker de gestión terminó con error");
    }

    if let Err(e) = asambleas.await {
        tracing::error!(error = %e, "el worker de asambleas terminó con error");
    }
    if let Err(e) = acceso.await {
        tracing::error!(error = %e, "el worker de control de acceso terminó con error");
    }

    tracing::info!("gateway detenido correctamente");
    Ok(())
}

/// Se resuelve cuando llega la señal de apagado (o si el emisor desaparece).
async fn esperar_apagado(mut apagado: watch::Receiver<bool>) {
    let _ = apagado.wait_for(|apagar| *apagar).await;
}
