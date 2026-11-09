//! Cliente FCM contra un servidor simulado: token de acceso con JWT, envío y clasificación de
//! errores. La cuenta de servicio del fixture es de prueba (clave generada para estas pruebas).

use std::sync::{Arc, Mutex};

use application::puertos::{
    EnvioPush,
    acceso::{ErrorPush, NotificacionPush},
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use infra_push::ClienteFcm;
use serde_json::{Value, json};

#[derive(Default, Clone)]
struct Registro {
    tokens_pedidos: Arc<Mutex<u32>>,
    autorizaciones: Arc<Mutex<Vec<String>>>,
    cuerpos: Arc<Mutex<Vec<Value>>>,
}

async fn token(State(r): State<Registro>, cuerpo: Bytes) -> Json<Value> {
    let texto = String::from_utf8_lossy(&cuerpo);
    assert!(texto.starts_with(
        "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion="
    ));
    let jwt = texto.rsplit('=').next().expect("jwt");
    assert_eq!(jwt.split('.').count(), 3, "JWT de tres partes");
    *r.tokens_pedidos.lock().expect("mutex") += 1;
    Json(json!({"access_token": "acceso-1", "expires_in": 3600, "token_type": "Bearer"}))
}

async fn enviar(
    State(r): State<Registro>,
    h: HeaderMap,
    Json(v): Json<Value>,
) -> (StatusCode, Json<Value>) {
    r.autorizaciones.lock().expect("mutex").push(
        h.get("authorization")
            .and_then(|x| x.to_str().ok())
            .unwrap_or_default()
            .to_owned(),
    );
    r.cuerpos.lock().expect("mutex").push(v.clone());
    if v["message"]["token"] == "desinstalada" {
        return (
            StatusCode::NOT_FOUND,
            Json(
                json!({"error": {"code": 404, "status": "NOT_FOUND", "details": [{"errorCode": "UNREGISTERED"}]}}),
            ),
        );
    }
    if v["message"]["token"] == "saturado" {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": {"status": "UNAVAILABLE"}})),
        );
    }
    (
        StatusCode::OK,
        Json(json!({"name": "projects/dad-prueba/messages/123"})),
    )
}

async fn servidor() -> (String, Registro) {
    let registro = Registro::default();
    let app = Router::new()
        .route("/token", post(token))
        .route("/v1/projects/dad-prueba/messages:send", post(enviar))
        .with_state(registro.clone());
    let escucha = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("puerto");
    let url = format!("http://{}", escucha.local_addr().expect("dirección"));
    tokio::spawn(async move { axum::serve(escucha, app).await.expect("servidor") });
    (url, registro)
}

fn cliente(url: &str) -> ClienteFcm {
    let mut cuenta: Value =
        serde_json::from_str(include_str!("fixtures/cuenta_servicio_prueba.json"))
            .expect("fixture");
    cuenta["token_uri"] = json!(format!("{url}/token"));
    ClienteFcm::desde_json(&cuenta.to_string(), url).expect("cliente")
}

fn notificacion(token: &str) -> NotificacionPush {
    NotificacionPush {
        token: token.into(),
        titulo: "Visita en portería".into(),
        cuerpo: "Mensajero pide ingresar".into(),
        datos: [("visita_id".to_owned(), "v-1".to_owned())]
            .into_iter()
            .collect(),
        respaldo: None,
    }
}

#[tokio::test]
async fn envia_con_token_de_acceso_reutilizado() {
    let (url, registro) = servidor().await;
    let fcm = cliente(&url);

    let id = fcm
        .enviar(&notificacion("telefono-1"))
        .await
        .expect("envío");
    fcm.enviar(&notificacion("telefono-2"))
        .await
        .expect("envío");

    assert_eq!(id, "projects/dad-prueba/messages/123");
    assert_eq!(
        *registro.tokens_pedidos.lock().expect("mutex"),
        1,
        "el token de acceso se reutiliza"
    );
    assert!(
        registro
            .autorizaciones
            .lock()
            .expect("mutex")
            .iter()
            .all(|a| a == "Bearer acceso-1")
    );
    let cuerpos = registro.cuerpos.lock().expect("mutex");
    assert_eq!(cuerpos[0]["message"]["data"]["visita_id"], "v-1");
    assert_eq!(
        cuerpos[0]["message"]["notification"]["title"],
        "Visita en portería"
    );
}

#[tokio::test]
async fn distingue_token_invalido_de_falla_temporal() {
    let (url, _) = servidor().await;
    let fcm = cliente(&url);

    assert_eq!(
        fcm.enviar(&notificacion("desinstalada")).await,
        Err(ErrorPush::TokenInvalido)
    );
    assert!(matches!(
        fcm.enviar(&notificacion("saturado")).await,
        Err(ErrorPush::Temporal(_))
    ));
}

#[test]
fn rechaza_cuentas_de_servicio_invalidas() {
    assert!(ClienteFcm::desde_json("{}", "http://x").is_err());
    let sin_clave =
        json!({"project_id": "p", "client_email": "c", "private_key": "nada", "token_uri": "t"});
    assert!(ClienteFcm::desde_json(&sin_clave.to_string(), "http://x").is_err());
    assert!(format!("{:?}", cliente("http://x")).contains("oculta"));
}
