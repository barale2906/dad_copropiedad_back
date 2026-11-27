//! Pruebas funcionales de los extractores y la paginación: toda petición inválida responde
//! con el formato uniforme `{ "error": { "codigo", "mensaje" } }`.
//!
//! Aún no hay endpoints de negocio con listados, así que se monta un handler mínimo con los
//! mismos tipos que usarán los endpoints reales (`Query<ParametrosPagina>`, `Json`, `Path`).

use api::{
    error::ApiResult,
    extractores::{Json, Path, Query},
    paginacion::{PaginaRespuesta, ParametrosPagina},
};
use application::Pagina;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    response::Response,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

#[derive(Deserialize)]
struct Entrada {
    nombre: String,
}

async fn listar(Query(p): Query<ParametrosPagina>) -> ApiResult<Json<PaginaRespuesta<String>>> {
    let solicitud = p.validar()?;
    let filas: Vec<Uuid> = (0..3).map(|_| Uuid::now_v7()).collect();
    let pagina = Pagina::desde_filas(filas, solicitud, |id| *id).mapear(|id| id.to_string());
    Ok(Json(pagina.into()))
}

async fn crear(Json(entrada): Json<Entrada>) -> StatusCode {
    if entrada.nombre.is_empty() {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::CREATED
    }
}

async fn ver(Path(_id): Path<Uuid>) -> StatusCode {
    StatusCode::OK
}

fn app() -> Router {
    Router::new()
        .route("/items", get(listar).post(crear))
        .route("/items/{id}", get(ver))
        .route("/eco", post(crear))
}

async fn enviar(req: Request<Body>) -> (StatusCode, Value) {
    let res: Response = app().oneshot(req).await.expect("el router no falla");
    let status = res.status();
    let bytes = to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("cuerpo legible");
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

fn get_(uri: &str) -> Request<Body> {
    Request::get(uri)
        .body(Body::empty())
        .expect("petición válida")
}

#[tokio::test]
async fn pagina_con_cursor_cuando_hay_mas_resultados() {
    let (status, json) = enviar(get_("/items?limite=2")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["datos"].as_array().map(Vec::len), Some(2));
    assert_eq!(json["siguiente"], json["datos"][1]);
}

#[tokio::test]
async fn limite_fuera_de_rango_es_validacion_422() {
    let (status, json) = enviar(get_("/items?limite=0")).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json["error"]["codigo"], "validacion");
}

#[tokio::test]
async fn parametro_con_tipo_incorrecto_es_peticion_invalida_400() {
    for uri in [
        "/items?limite=muchos",
        "/items?despues=no-es-uuid",
        "/items/no-es-uuid",
    ] {
        let (status, json) = enviar(get_(uri)).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(json["error"]["codigo"], "peticion_invalida", "{uri}");
    }
}

#[tokio::test]
async fn json_mal_formado_es_peticion_invalida_400() {
    let req = Request::post("/eco")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{ nombre: "))
        .expect("petición válida");

    let (status, json) = enviar(req).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"]["codigo"], "peticion_invalida");
}

#[tokio::test]
async fn json_valido_pasa_al_handler() {
    let req = Request::post("/eco")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{ "nombre": "Torre 3" }"#))
        .expect("petición válida");

    let (status, _) = enviar(req).await;

    assert_eq!(status, StatusCode::CREATED);
}
