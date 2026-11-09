//! Rutas del gateway: sondas y webhook de WhatsApp Cloud API.

use std::collections::HashMap;

use application::{
    casos_uso::{
        acceso::whatsapp::{ResultadoVisitaWhatsapp, atender_respuesta_visita},
        asambleas::whatsapp::{ResultadoAsamblea, ServiciosAsamblea, atender_mensaje_asamblea},
        chat::{ResultadoTurno, ServiciosChat, atender_mensaje},
    },
    puertos::{DirectorioAsambleas, RelojSistema},
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
    }
    let acuses = notificacion.acuses();
    let calidades = notificacion.calidades();
    if !acuses.is_empty() || !calidades.is_empty() {
        let state = state.clone();
        tokio::spawn(async move {
            for a in acuses {
                if let Err(e) = state.asambleas.registrar_acuse(&a).await {
                    tracing::warn!(wamid = %a.wamid, error = %e, "no se pudo registrar el acuse");
                }
            }
            for c in calidades {
                tracing::warn!(degradada = c.degradada, calidad = ?c.calidad, "cambio de calidad de una línea");
                if let Err(e) = state
                    .asambleas
                    .actualizar_calidad_linea(&c.numero, c.degradada, c.calidad)
                    .await
                {
                    tracing::warn!(error = %e, "no se pudo actualizar la calidad de la línea");
                }
            }
        });
    }
    StatusCode::OK
}

/// Los botones de visitas van primero; luego la asamblea (votar, confirmar asistencia) y al
/// final el chat de la copropiedad.
async fn atender(state: &GatewayState, mensaje: application::puertos::mensajeria::MensajeEntrante) {
    match atender_respuesta_visita(&state.unidad, &state.sesiones, &RelojSistema, &mensaje).await {
        Ok(ResultadoVisitaWhatsapp::NoAplica) => {}
        Ok(ResultadoVisitaWhatsapp::Repetido) => {
            tracing::debug!(wamid = %mensaje.wamid, "respuesta de visita repetida");
            return;
        }
        Ok(ResultadoVisitaWhatsapp::Atendido { .. }) => {
            state.hay_salida.notify_one();
            return;
        }
        Err(e) => {
            tracing::error!(wamid = %mensaje.wamid, error = %e, "no se pudo atender la respuesta de visita");
            return;
        }
    }
    let servicios = ServiciosAsamblea {
        unidad: &state.unidad,
        directorio: &state.asambleas,
        sesiones: &state.sesiones,
        reloj: &RelojSistema,
    };
    match atender_mensaje_asamblea(&servicios, &mensaje).await {
        Ok(ResultadoAsamblea::NoAplica) => atender_chat(state, mensaje).await,
        Ok(ResultadoAsamblea::Repetido) => {
            tracing::debug!(wamid = %mensaje.wamid, "mensaje de asamblea repetido");
        }
        Ok(ResultadoAsamblea::Atendido { respuestas }) => {
            tracing::debug!(wamid = %mensaje.wamid, respuestas, "mensaje de asamblea atendido");
            state.hay_salida.notify_one();
        }
        Err(e) => {
            tracing::error!(wamid = %mensaje.wamid, error = %e, "no se pudo atender el mensaje de asamblea");
        }
    }
}

async fn atender_chat(
    state: &GatewayState,
    mensaje: application::puertos::mensajeria::MensajeEntrante,
) {
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

    use uuid::Uuid;

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
            asambleas: infra_postgres::DirectorioAsambleasPg::new(db.clone()),
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

    async fn pool_app(duenio: &PgPool) -> PgPool {
        PgPoolOptions::new()
            .max_connections(4)
            .after_connect(|c, _| {
                Box::pin(async move {
                    c.execute("SET ROLE dad_app").await?;
                    Ok(())
                })
            })
            .connect_with((*duenio.connect_options()).clone())
            .await
            .unwrap()
    }

    async fn esperar_conteo(duenio: &PgPool, sql: &str, esperado: i64) -> i64 {
        let mut n = 0;
        for _ in 0..100 {
            n = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
                .fetch_one(duenio)
                .await
                .unwrap();
            if n >= esperado {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        n
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn un_acuse_de_entrega_por_el_webhook_queda_registrado(duenio: PgPool) {
        let cuerpo = json!({
            "object": "whatsapp_business_account",
            "entry": [{ "id": "WABA", "changes": [{ "field": "messages", "value": {
                "messaging_product": "whatsapp",
                "metadata": { "display_phone_number": "573000000000", "phone_number_id": "PNID" },
                "statuses": [{ "id": "wamid.salida", "status": "failed", "timestamp": "1798000000",
                               "errors": [{ "code": 131026 }] }]
            }}]}]
        })
        .to_string();
        let res = app_con(pool_app(&duenio).await)
            .oneshot(
                Request::post("/webhook/whatsapp")
                    .header(CABECERA_FIRMA, firmar(cuerpo.as_bytes()))
                    .body(Body::from(cuerpo))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let n = esperar_conteo(
            &duenio,
            "SELECT count(*) FROM whatsapp_estados WHERE wamid = 'wamid.salida' AND error_codigo = 131026",
            1,
        )
        .await;
        assert_eq!(n, 1);
    }

    /// Asamblea instalada con una pregunta sí/no abierta y un asistente presente que confirmó
    /// WhatsApp. Devuelve (tenant, pregunta, opción «Sí»).
    async fn asamblea_con_pregunta_abierta(duenio: &PgPool, telefono: &str) -> (Uuid, Uuid, Uuid) {
        let (t, b, p, r, a, q, o) = (
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let sql = format!(
            "INSERT INTO copropiedades (id, nombre, estado) VALUES ('{t}', 'Conjunto', 'activa');
             INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
                 VALUES (gen_random_uuid(), '{t}', 'asambleas', now() - interval '1 day');
             INSERT INTO bienes_privados (id, tenant_id, codigo, coeficiente) VALUES ('{b}', '{t}', 'T1-101', 0.5);
             INSERT INTO personas (id, tenant_id, nombre, telefono) VALUES ('{p}', '{t}', 'Ana', '{telefono}');
             INSERT INTO reuniones (id, tenant_id, titulo, tipo, estado, fecha_programada)
                 VALUES ('{r}', '{t}', 'Asamblea', 'ordinaria', 'instalada', now());
             INSERT INTO padron_reunion (tenant_id, reunion_id, bien_privado_id, coeficiente, representante_persona_id)
                 VALUES ('{t}', '{r}', '{b}', 0.5, '{p}');
             INSERT INTO asistentes (id, tenant_id, reunion_id, persona_id, metodo, telefono, linea_whatsapp,
                                     whatsapp_confirmado_en)
                 VALUES ('{a}', '{t}', '{r}', '{p}', 'mesa', '{telefono}', 'PNID', now());
             INSERT INTO asistente_bienes (id, tenant_id, reunion_id, asistente_id, bien_privado_id, calidad, desde)
                 VALUES (gen_random_uuid(), '{t}', '{r}', '{a}', '{b}', 'propietario', now());
             INSERT INTO asistente_presencias (id, tenant_id, reunion_id, asistente_id, entrada_en)
                 VALUES (gen_random_uuid(), '{t}', '{r}', '{a}', now());
             INSERT INTO preguntas (id, tenant_id, reunion_id, orden, enunciado, tipo, tipo_voto, base_computo,
                                    regla_mayoria, estado, abierta_en)
                 VALUES ('{q}', '{t}', '{r}', 1, '¿Aprueba?', 'si_no', 'nominal', 'coeficiente', 'simple',
                         'abierta', now());
             INSERT INTO opciones (id, tenant_id, pregunta_id, orden, texto) VALUES
                 ('{o}', '{t}', '{q}', 1, 'Sí'), (gen_random_uuid(), '{t}', '{q}', 2, 'No');
             INSERT INTO pregunta_habilitados (tenant_id, pregunta_id, bien_privado_id, coeficiente, desde)
                 VALUES ('{t}', '{q}', '{b}', 0.5, now());"
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(duenio)
            .await
            .unwrap();
        (t, q, o)
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn un_boton_de_voto_por_el_webhook_registra_la_participacion(duenio: PgPool) {
        let telefono = "573009998877";
        let (t, q, o) = asamblea_con_pregunta_abierta(&duenio, telefono).await;
        let cuerpo = json!({
            "object": "whatsapp_business_account",
            "entry": [{ "id": "WABA", "changes": [{ "field": "messages", "value": {
                "messaging_product": "whatsapp",
                "metadata": { "display_phone_number": "573000000000", "phone_number_id": "PNID" },
                "messages": [{ "from": telefono, "id": "wamid.voto", "timestamp": "1798000000",
                               "type": "interactive",
                               "interactive": { "type": "button_reply",
                                                "button_reply": { "id": format!("vt:{t}:{q}:{o}"), "title": "1. Sí" } } }]
            }}]}]
        })
        .to_string();
        let app = app_con(pool_app(&duenio).await);
        for _ in 0..2 {
            // Meta reintenta el mismo webhook.
            let res = app
                .clone()
                .oneshot(
                    Request::post("/webhook/whatsapp")
                        .header(CABECERA_FIRMA, firmar(cuerpo.as_bytes()))
                        .body(Body::from(cuerpo.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
        }
        let sql = format!(
            "SELECT count(*) FROM participaciones WHERE pregunta_id = '{q}' AND wamid = 'wamid.voto'"
        );
        assert_eq!(esperar_conteo(&duenio, &sql, 1).await, 1);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(esperar_conteo(&duenio, &sql, 1).await, 1, "sin duplicados");
        let confirmaciones: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM outbox WHERE destino = $1 AND payload->'contenido'->>'cuerpo' LIKE '✅ Voto registrado%'",
        )
        .bind(telefono)
        .fetch_one(&duenio)
        .await
        .unwrap();
        assert_eq!(confirmaciones, 1);
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
