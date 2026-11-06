//! Rutas del gateway: sondas y webhook de WhatsApp Cloud API.

use std::collections::HashMap;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use infra_whatsapp::{
    firma::{CABECERA_FIRMA, verificar_firma},
    webhook::Notificacion,
};
use serde_json::{Value, json};
use shared::AppConfig;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer};

use crate::state::GatewayState;

/// Meta envía notificaciones pequeñas; se limita el cuerpo para no aceptar abusos.
const LIMITE_CUERPO: usize = 256 * 1024;

pub fn router(state: GatewayState, config: &AppConfig) -> Router {
    Router::new()
        .route("/health", get(liveness))
        .route("/ready", get(readiness))
        .route(
            "/webhook/whatsapp",
            get(verificar_suscripcion).post(recibir),
        )
        .layer(RequestBodyLimitLayer::new(LIMITE_CUERPO))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            config.request_timeout,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn liveness() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

async fn readiness(State(state): State<GatewayState>) -> impl IntoResponse {
    match infra_postgres::ping(&state.db).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "status": "ok", "database": "ok" })),
        ),
        Err(e) => {
            tracing::warn!(error = %e, "readiness: PostgreSQL no responde");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "error", "database": "no_disponible" })),
            )
        }
    }
}

/// `GET`: Meta verifica la suscripción con `hub.mode`, `hub.verify_token` y `hub.challenge`.
async fn verificar_suscripcion(
    State(state): State<GatewayState>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let modo = params.get("hub.mode").map(String::as_str);
    let token = params.get("hub.verify_token").map(String::as_str);
    match (modo, token, params.get("hub.challenge")) {
        (Some("subscribe"), Some(t), Some(challenge)) if t == state.whatsapp.verify_token => {
            tracing::info!("suscripción de webhook verificada");
            (StatusCode::OK, challenge.clone())
        }
        _ => (StatusCode::FORBIDDEN, String::new()),
    }
}

/// `POST`: notificación de Meta. Se valida la firma sobre el cuerpo **crudo** y se responde
/// rápido (Meta reintenta si tardamos); el procesamiento es idempotente por `wamid`.
async fn recibir(
    State(state): State<GatewayState>,
    cabeceras: HeaderMap,
    cuerpo: Bytes,
) -> StatusCode {
    let firma = cabeceras.get(CABECERA_FIRMA).and_then(|v| v.to_str().ok());
    if let Err(e) = verificar_firma(state.whatsapp.app_secret.as_bytes(), &cuerpo, firma) {
        tracing::warn!(error = %e, "webhook rechazado: firma inválida");
        return StatusCode::UNAUTHORIZED;
    }

    let notificacion = match Notificacion::desde_json(&cuerpo) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, "webhook con JSON inválido");
            return StatusCode::BAD_REQUEST;
        }
    };

    for mensaje in notificacion.mensajes() {
        // Nunca loguear el teléfono ni el contenido (datos personales / posible voto).
        tracing::info!(linea = %mensaje.linea, wamid = %mensaje.wamid, "mensaje entrante");
        // Porción vertical pendiente:
        //   1. DirectorioReuniones::activas_por_telefono + SesionesConversacion (Redis)
        //   2. casos_uso::resolver_reunion → lista de reuniones si hay varias
        //   3. casos_uso::EmitirVoto (idempotency_key = wamid) → confirmación vía outbox
    }
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use axum::{body::Body, http::Request};
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    use shared::{LogFormat, WhatsappConfig};
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    use super::*;

    fn app() -> Router {
        let config = AppConfig {
            host: "127.0.0.1".into(),
            port: 0,
            database_url: "postgres://x:x@localhost:1/x".into(),
            migrations_database_url: None,
            database_max_connections: 1,
            run_migrations: false,
            request_timeout: Duration::from_secs(5),
            log_format: LogFormat::Pretty,
        };
        let db = PgPoolOptions::new()
            .connect_lazy(&config.database_url)
            .unwrap();
        let state = GatewayState {
            db,
            whatsapp: Arc::new(WhatsappConfig {
                verify_token: "token-verificacion".into(),
                app_secret: "secreto".into(),
            }),
        };
        router(state, &config)
    }

    fn firmar(cuerpo: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secreto").unwrap();
        mac.update(cuerpo);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[tokio::test]
    async fn verificacion_de_suscripcion() {
        let ok = app()
            .oneshot(
                Request::get("/webhook/whatsapp?hub.mode=subscribe&hub.verify_token=token-verificacion&hub.challenge=123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);

        let mal = app()
            .oneshot(
                Request::get(
                    "/webhook/whatsapp?hub.mode=subscribe&hub.verify_token=otro&hub.challenge=123",
                )
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mal.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn webhook_exige_firma_valida() {
        let cuerpo = br#"{"object":"whatsapp_business_account","entry":[]}"#;
        let enviar = |firma: Option<String>| {
            let mut req = Request::post("/webhook/whatsapp");
            if let Some(f) = firma {
                req = req.header(CABECERA_FIRMA, f);
            }
            app().oneshot(req.body(Body::from(&cuerpo[..])).unwrap())
        };

        assert_eq!(
            enviar(Some(firmar(cuerpo))).await.unwrap().status(),
            StatusCode::OK
        );
        assert_eq!(
            enviar(Some(firmar(b"otro"))).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            enviar(None).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
