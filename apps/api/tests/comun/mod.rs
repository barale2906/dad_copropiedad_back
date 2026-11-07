//! Ayudantes de las pruebas funcionales de la API: router real sobre PostgreSQL con el rol
//! `dad_app`, peticiones JSON y cuentas de prueba.
#![allow(dead_code)]

use api::{routes, state::AppState};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use infra_archivos::{AlmacenLocal, FirmanteEnlaces};
use serde_json::{Value, json};
use sqlx::{Executor, PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;

pub static MIGRACIONES: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
pub const URL_PUBLICA: &str = "http://localhost:8080";
pub const CONTRASENA: &str = "una frase bien larga";

pub struct Api {
    pub router: Router,
    pub duenio: PgPool,
    _dir: DirTemporal,
}

struct DirTemporal(std::path::PathBuf);

impl Drop for DirTemporal {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub async fn api(pool: PgPool) -> Api {
    let app = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|c, _| {
            Box::pin(async move {
                c.execute("SET ROLE dad_app").await?;
                Ok(())
            })
        })
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("pool dad_app");
    let dir = std::env::temp_dir().join(format!("dad-api-id-{}", uuid::Uuid::new_v4()));
    let enlaces = FirmanteEnlaces::new(&[9; 32], URL_PUBLICA).expect("firmante");
    let almacen = AlmacenLocal::abrir(&dir, enlaces).expect("almacén");
    let config = shared::AppConfig {
        host: "127.0.0.1".into(),
        port: 0,
        database_url: String::new(),
        migrations_database_url: None,
        database_max_connections: 1,
        run_migrations: false,
        request_timeout: std::time::Duration::from_secs(10),
        log_format: shared::LogFormat::Pretty,
        docs_api_habilitada: false,
    };
    Api {
        router: routes::router(AppState::new(app, almacen, true), &config),
        duenio: pool,
        _dir: DirTemporal(dir),
    }
}

impl Api {
    pub async fn pedir(
        &self,
        metodo: Method,
        uri: &str,
        token: Option<&str>,
        cuerpo: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(metodo).uri(uri);
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let body = match cuerpo {
            Some(v) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let res = self
            .router
            .clone()
            .oneshot(req.body(body).expect("petición"))
            .await
            .expect("router");
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.expect("cuerpo");
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    /// Envía un cuerpo binario (archivo) y devuelve el estado y el JSON de la respuesta.
    pub async fn subir(&self, uri: &str, token: &str, contenido: Vec<u8>) -> (StatusCode, Value) {
        let req = Request::post(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(contenido))
            .expect("petición");
        let res = self.router.clone().oneshot(req).await.expect("router");
        let status = res.status();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.expect("cuerpo");
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    pub async fn registrar(&self, usuario: &str) -> String {
        let (s, v) = self
            .pedir(
                Method::POST,
                "/api/v1/auth/registro",
                None,
                Some(json!({
                    "nombre": format!("Persona {usuario}"),
                    "usuario": usuario,
                    "correo": format!("{usuario}@correo.co"),
                    "contrasena": CONTRASENA,
                })),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        v["id"].as_str().expect("id").to_owned()
    }

    pub async fn login(&self, usuario: &str) -> Value {
        let (s, v) = self
            .pedir(
                Method::POST,
                "/api/v1/auth/sesion",
                None,
                Some(json!({ "identificador": usuario, "contrasena": CONTRASENA })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }

    pub async fn token(&self, usuario: &str) -> String {
        self.login(usuario).await["tokens"]["token_acceso"]
            .as_str()
            .expect("token")
            .to_owned()
    }

    pub async fn hacer_superusuario(&self, cuenta: &str) {
        sqlx::query(
            "INSERT INTO asignaciones_plataforma (id, cuenta_id, rol_id, vigente_desde)
             VALUES (gen_random_uuid(), $1::uuid, uuid_rol_plantilla('superusuario'), now())",
        )
        .bind(cuenta)
        .execute(&self.duenio)
        .await
        .expect("superusuario");
    }

    /// Activa el segundo factor de la cuenta y devuelve un token de una sesión que lo verificó.
    pub async fn token_con_segundo_factor(&self, usuario: &str) -> String {
        let token = self.token(usuario).await;
        let (s, v) = self
            .pedir(
                Method::POST,
                "/api/v1/yo/segundo-factor",
                Some(&token),
                None,
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let clave = infra_seguridad::base32::decodificar(v["secreto"].as_str().expect("secreto"))
            .expect("base32");
        let codigo = || {
            infra_seguridad::codigo_totp(&clave, chrono::Utc::now().timestamp() / 30)
                .expect("código")
        };
        let (s, _) = self
            .pedir(
                Method::POST,
                "/api/v1/yo/segundo-factor/confirmar",
                Some(&token),
                Some(json!({ "codigo": codigo() })),
            )
            .await;
        assert_eq!(s, StatusCode::NO_CONTENT);

        let inicio = self.login(usuario).await;
        assert_eq!(inicio["resultado"], "segundo_factor");
        let (s, v) = self
            .pedir(
                Method::POST,
                "/api/v1/auth/sesion/segundo-factor",
                None,
                Some(json!({ "desafio": inicio["desafio"], "codigo": codigo() })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["token_acceso"].as_str().expect("token").to_owned()
    }
}
