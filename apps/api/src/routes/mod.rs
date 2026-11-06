//! Definición de rutas y middlewares HTTP.

mod health;

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

use crate::state::AppState;

const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

pub fn router(state: AppState, config: &AppConfig) -> Router {
    Router::new()
        .route("/health", get(health::liveness))
        .route("/ready", get(health::readiness))
        .nest("/api/v1", api_v1())
        .layer(
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

/// API versionada. Aquí se montan los routers por contexto (reuniones, votaciones, registro…).
/// Los handlers solo traducen HTTP ↔ casos de uso de `application`; nada de reglas de negocio.
fn api_v1() -> Router<AppState> {
    Router::new()
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request};
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    use super::*;

    fn config_prueba() -> AppConfig {
        AppConfig {
            host: "127.0.0.1".into(),
            port: 0,
            database_url: "postgres://x:x@localhost:1/x".into(),
            migrations_database_url: None,
            database_max_connections: 1,
            run_migrations: false,
            request_timeout: std::time::Duration::from_secs(5),
            log_format: shared::LogFormat::Pretty,
        }
    }

    #[tokio::test]
    async fn health_responde_200_y_request_id() {
        let cfg = config_prueba();
        let db = PgPoolOptions::new()
            .connect_lazy(&cfg.database_url)
            .unwrap();
        let app = router(AppState { db }, &cfg);

        let res = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert!(res.headers().contains_key(REQUEST_ID));
    }
}
