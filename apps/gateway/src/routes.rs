//! Rutas del gateway: sondas y webhook de WhatsApp Cloud API.

use std::collections::HashMap;

use application::{
    casos_uso::chat::{ResultadoTurno, ServiciosChat, atender_mensaje},
    puertos::RelojSistema,
};
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
        // Se responde 200 de inmediato y el turno se atiende aparte: Meta reintenta si tardamos,
        // y el turno es idempotente por `wamid`.
        let state = state.clone();
        tokio::spawn(async move { atender(&state, mensaje).await });
        // Votaciones (plan 06): el menú del chat sumará la opción de votar en asamblea.
    }
    StatusCode::OK
}

async fn atender(state: &GatewayState, mensaje: application::puertos::mensajeria::MensajeEntrante) {
    let servicios = ServiciosChat {
        unidad: &state.unidad,
        sesiones: &state.sesiones,
        directorio: &state.directorio,
        mensajeria: &state.mensajeria,
        almacen: state.archivos.as_ref(),
        reloj: &RelojSistema,
    };
    let wamid = mensaje.wamid.clone();
    match atender_mensaje(&servicios, mensaje).await {
        Ok(ResultadoTurno::Atendido { respuestas }) => {
            tracing::debug!(%wamid, respuestas, "turno del chat atendido");
            if respuestas > 0 {
                state.hay_salida.notify_one();
            }
        }
        Ok(ResultadoTurno::Repetido) => tracing::debug!(%wamid, "mensaje repetido: se ignora"),
        Err(e) => tracing::error!(%wamid, error = %e, "no se pudo cerrar el turno del chat"),
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use axum::{body::Body, http::Request};
    use hmac::{Hmac, KeyInit, Mac};
    use serde_json::Value;
    use sha2::Sha256;
    use shared::{LogFormat, WhatsappConfig};
    use sqlx::{Executor, PgPool, postgres::PgPoolOptions};
    use tower::ServiceExt;

    use super::*;

    fn app() -> Router {
        let db = PgPoolOptions::new()
            .connect_lazy("postgres://x:x@localhost:1/x")
            .unwrap();
        app_con(db)
    }

    fn app_con(db: PgPool) -> Router {
        let config = AppConfig {
            host: "127.0.0.1".into(),
            port: 0,
            database_url: "postgres://x:x@localhost:1/x".into(),
            migrations_database_url: None,
            database_max_connections: 1,
            run_migrations: false,
            request_timeout: Duration::from_secs(5),
            log_format: LogFormat::Pretty,
            docs_api_habilitada: false,
        };
        let enlaces =
            infra_archivos::FirmanteEnlaces::new(&[7; 32], "http://localhost").expect("firmante");
        let archivos =
            infra_archivos::AlmacenLocal::abrir(std::env::temp_dir(), enlaces).expect("almacén");
        let state = GatewayState {
            unidad: infra_postgres::UnidadDeTrabajoPg::new(db.clone()),
            sesiones: infra_postgres::SesionesChatPg::new(db.clone()),
            directorio: infra_postgres::DirectorioTelefonosPg::new(db.clone()),
            mensajeria: crate::mensajeria::MensajeriaGateway::new(None),
            archivos: Arc::new(archivos),
            hay_salida: Arc::default(),
            db,
            whatsapp: Arc::new(WhatsappConfig {
                verify_token: "token-verificacion".into(),
                app_secret: "secreto".into(),
                access_token: None,
                api_url: shared::WHATSAPP_API_URL_PREDETERMINADA.into(),
            }),
        };
        router(state, &config)
    }

    fn firmar(cuerpo: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secreto").unwrap();
        mac.update(cuerpo);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    static MIGRACIONES: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

    /// Copropiedad activa con la gestión administrativa y un propietario con WhatsApp.
    async fn ph_con_telefono(duenio: &PgPool, telefono: &str) {
        let mut tx = duenio.begin().await.unwrap();
        sqlx::query(
            "WITH ph AS (
                     INSERT INTO copropiedades (id, nombre, estado)
                     VALUES (gen_random_uuid(), 'Conjunto Alameda', 'activa') RETURNING id),
                 m AS (
                     INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
                     SELECT gen_random_uuid(), id, 'gestion_administrativa', now() - interval '1 day'
                     FROM ph),
                 b AS (
                     INSERT INTO bienes_privados (id, tenant_id, codigo)
                     SELECT gen_random_uuid(), id, 'T1-101' FROM ph RETURNING id, tenant_id),
                 p AS (
                     INSERT INTO personas (id, tenant_id, nombre, telefono)
                     SELECT gen_random_uuid(), id, 'Ana', $1 FROM ph RETURNING id)
                 INSERT INTO titularidades (id, tenant_id, bien_privado_id, persona_id, tipo,
                                            porcentaje, es_representante, vigente_desde)
                 SELECT gen_random_uuid(), b.tenant_id, b.id, p.id, 'propietario', 100, TRUE,
                        now() - interval '1 day'
                 FROM b, p",
        )
        .bind(telefono)
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }

    fn mensaje_de_texto(telefono: &str, wamid: &str, texto: &str) -> String {
        json!({
            "object": "whatsapp_business_account",
            "entry": [{ "id": "WABA", "changes": [{ "field": "messages", "value": {
                "messaging_product": "whatsapp",
                "metadata": { "display_phone_number": "573000000000", "phone_number_id": "PNID" },
                "messages": [{ "from": telefono, "id": wamid, "timestamp": "1798000000",
                               "type": "text", "text": { "body": texto } }]
            }}]}]
        })
        .to_string()
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn un_mensaje_de_whatsapp_recibe_el_menu_por_el_outbox(duenio: PgPool) {
        let telefono = "573001112233";
        ph_con_telefono(&duenio, telefono).await;
        let app_pool = PgPoolOptions::new()
            .max_connections(4)
            .after_connect(|c, _| {
                Box::pin(async move {
                    c.execute("SET ROLE dad_app").await?;
                    Ok(())
                })
            })
            .connect_with((*duenio.connect_options()).clone())
            .await
            .unwrap();
        let cuerpo = mensaje_de_texto(telefono, "wamid.hola", "hola");
        let enviar = || {
            app_con(app_pool.clone()).oneshot(
                Request::post("/webhook/whatsapp")
                    .header(CABECERA_FIRMA, firmar(cuerpo.as_bytes()))
                    .body(Body::from(cuerpo.clone()))
                    .unwrap(),
            )
        };
        assert_eq!(enviar().await.unwrap().status(), StatusCode::OK);
        // Meta reintenta el mismo webhook.
        assert_eq!(enviar().await.unwrap().status(), StatusCode::OK);

        // El turno se atiende aparte: se espera a que la respuesta llegue al outbox.
        for _ in 0..100 {
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE canal = 'whatsapp'")
                .fetch_one(&duenio)
                .await
                .unwrap();
            if n > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let respuestas: Vec<Value> = sqlx::query_scalar(
            "SELECT payload FROM outbox WHERE canal = 'whatsapp' AND destino = $1",
        )
        .bind(telefono)
        .fetch_all(&duenio)
        .await
        .unwrap();
        assert_eq!(respuestas.len(), 1, "un solo menú aunque Meta reintente");
        assert_eq!(respuestas[0]["contenido"]["tipo"], "lista");
        assert!(
            respuestas[0]["contenido"]["cuerpo"]
                .as_str()
                .is_some_and(|c| c.contains("T1-101"))
        );
        let sesiones: i64 = sqlx::query_scalar("SELECT count(*) FROM sesiones_chat")
            .fetch_one(&duenio)
            .await
            .unwrap();
        assert_eq!(sesiones, 1);
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
