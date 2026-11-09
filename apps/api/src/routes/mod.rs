//! Definición de rutas y middlewares HTTP.
//!
//! Convención de rutas (decisión D2 de la hoja de ruta):
//! - `/api/v1/plataforma/…` — operaciones globales (módulo `plataforma`).
//! - `/api/v1/yo/…` — la cuenta autenticada (módulo `yo`).
//! - `/api/v1/copropiedades/{copropiedad_id}/…` — todo lo de una copropiedad (módulo `copropiedades`).
//! - `/api/v1/auth/…` — registro, inicio y cierre de sesión (módulo `auth`, sin sesión).
//! - `/api/v1/organizaciones/{organizacion_id}/…` — ficha global de una organización
//!   (módulo `organizaciones`), y `/api/v1/permisos` — catálogo.
//! - `/api/v1/archivos/descarga` — descarga con enlace firmado, sin sesión (módulo `archivos`).
//! - `/api/v1/puentes/…` — puentes de acceso con token de máquina, sin sesión (`acceso::puentes`).
//!
//! Cada handler se registra con `utoipa_axum::routes!` para que el contrato OpenAPI salga de
//! las mismas rutas que se sirven.

mod acceso;
mod archivos;
pub(crate) mod asambleas;
mod auth;
mod bienes_privados;
mod censo;
mod coeficientes;
mod conversaciones;
mod copropiedades;
mod ficha;
mod health;
mod importaciones;
mod mantenimiento;
mod organizaciones;
mod parqueaderos;
mod plataforma;
mod reservas;
mod roles;
mod tickets;
mod yo;

use axum::{
    Router,
    extract::Request,
    http::{HeaderName, StatusCode},
    routing::get,
};
use shared::AppConfig;
use tower::ServiceBuilder;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::{DefaultOnFailure, DefaultOnResponse, TraceLayer},
};
use tracing::Level;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_scalar::{Scalar, Servable};

pub use self::copropiedades::PARAMETRO_COPROPIEDAD;
use crate::{error::ruta_no_encontrada, openapi::ApiDoc, state::AppState};

const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// Ruta del contrato OpenAPI en JSON (solo con `API_DOCS_HABILITADA=true`).
pub const RUTA_OPENAPI: &str = "/api-docs/openapi.json";
/// Ruta de la documentación interactiva (solo con `API_DOCS_HABILITADA=true`).
pub const RUTA_DOCS: &str = "/docs";

/// Router HTTP completo, con middlewares y estado.
pub fn router(state: AppState, config: &AppConfig) -> Router {
    let (rutas, api) = rutas().split_for_parts();

    let mut app = rutas.fallback(ruta_no_encontrada);
    if config.docs_api_habilitada {
        app = app.merge(documentacion(api));
    }

    app.layer(
        ServiceBuilder::new()
            .layer(SetRequestIdLayer::new(REQUEST_ID, MakeRequestUuid))
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(|req: &Request| {
                        let request_id = req
                            .headers()
                            .get(REQUEST_ID)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("-");
                        tracing::info_span!(
                            "http",
                            method = %req.method(),
                            uri = %req.uri(),
                            request_id
                        )
                    })
                    .on_response(DefaultOnResponse::new().level(Level::INFO))
                    .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
            )
            .layer(TimeoutLayer::with_status_code(
                StatusCode::REQUEST_TIMEOUT,
                config.request_timeout,
            ))
            .layer(PropagateRequestIdLayer::new(REQUEST_ID)),
    )
    .with_state(state)
}

/// Contrato OpenAPI generado a partir de las rutas registradas.
pub fn openapi() -> utoipa::openapi::OpenApi {
    rutas().split_for_parts().1
}

/// Todas las rutas documentadas, sin middlewares ni estado.
fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(health::liveness))
        .routes(routes!(health::readiness))
        .nest("/api/v1", api_v1())
}

/// API versionada. Los handlers solo traducen HTTP ↔ casos de uso de `application`; nada de
/// reglas de negocio.
fn api_v1() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .nest("/plataforma", plataforma::rutas())
        .nest("/yo", yo::rutas().merge(acceso::push::rutas()))
        .nest("/puentes", acceso::puentes::rutas())
        .nest("/archivos", archivos::rutas())
        .nest("/auth", auth::rutas())
        .nest("/organizaciones", organizaciones::rutas_cuenta())
        .nest("/permisos", roles::rutas_catalogo())
        .nest(
            &format!("/copropiedades/{{{PARAMETRO_COPROPIEDAD}}}"),
            copropiedades::rutas(),
        )
}

/// Contrato en JSON y UI interactiva (Scalar).
fn documentacion(api: utoipa::openapi::OpenApi) -> Router<AppState> {
    let json = axum::Json(api.clone());
    Router::new()
        .route(RUTA_OPENAPI, get(move || async move { json }))
        .merge(Scalar::with_url(RUTA_DOCS, api))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
        response::Response,
    };
    use serde_json::Value;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    use super::*;

    fn config_prueba(docs_api_habilitada: bool) -> AppConfig {
        AppConfig {
            host: "127.0.0.1".into(),
            port: 0,
            database_url: "postgres://x:x@localhost:1/x".into(),
            migrations_database_url: None,
            database_max_connections: 1,
            run_migrations: false,
            request_timeout: std::time::Duration::from_secs(5),
            log_format: shared::LogFormat::Pretty,
            docs_api_habilitada,
        }
    }

    async fn pedir(docs_api_habilitada: bool, uri: &str) -> Response {
        let cfg = config_prueba(docs_api_habilitada);
        let db = PgPoolOptions::new()
            .connect_lazy(&cfg.database_url)
            .expect("URL de prueba válida");
        let enlaces =
            infra_archivos::FirmanteEnlaces::new(&[7; 32], "http://localhost").expect("válido");
        let archivos = infra_archivos::AlmacenLocal::abrir(std::env::temp_dir(), enlaces)
            .expect("directorio temporal");
        let state = AppState::new(
            db,
            archivos,
            true,
            infra_seguridad::FirmaQrEd25519::new(&[5; 32]).expect("secreto de prueba"),
        );
        router(state, &cfg)
            .oneshot(
                Request::get(uri)
                    .body(Body::empty())
                    .expect("petición válida"),
            )
            .await
            .expect("el router no falla")
    }

    async fn cuerpo_json(res: Response) -> Value {
        let bytes = to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("cuerpo legible");
        serde_json::from_slice(&bytes).expect("cuerpo JSON")
    }

    #[tokio::test]
    async fn health_responde_200_y_request_id() {
        let res = pedir(false, "/health").await;

        assert_eq!(res.status(), StatusCode::OK);
        assert!(res.headers().contains_key(REQUEST_ID));
        assert_eq!(cuerpo_json(res).await["status"], "ok");
    }

    #[tokio::test]
    async fn ruta_inexistente_devuelve_error_uniforme() {
        for uri in [
            "/no-existe",
            "/api/v1/yo/nada",
            "/api/v1/copropiedades/0190c1a2-0000-7000-8000-000000000000/nada",
        ] {
            let res = pedir(false, uri).await;

            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(
                cuerpo_json(res).await["error"]["codigo"],
                "no_encontrado",
                "{uri}"
            );
        }
    }

    #[tokio::test]
    async fn documentacion_solo_si_esta_habilitada() {
        assert_eq!(
            pedir(false, RUTA_OPENAPI).await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            pedir(false, RUTA_DOCS).await.status(),
            StatusCode::NOT_FOUND
        );

        let res = pedir(true, RUTA_OPENAPI).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            cuerpo_json(res).await["info"]["title"],
            "DAD_COPROPIEDAD API"
        );
        assert_eq!(pedir(true, RUTA_DOCS).await.status(), StatusCode::OK);
    }

    #[test]
    fn contrato_incluye_salud_y_esquema_de_error() {
        let doc = serde_json::to_value(openapi()).expect("serializa");

        assert!(doc["paths"]["/health"]["get"].is_object());
        assert!(doc["paths"]["/ready"]["get"].is_object());
        assert!(doc["components"]["schemas"]["ErrorRespuesta"].is_object());
        assert!(doc["components"]["schemas"]["CodigoError"].is_object());
    }
}
