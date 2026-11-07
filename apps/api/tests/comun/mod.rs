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

    /// GET que devuelve texto (CSV): estado, `Content-Type` y cuerpo.
    pub async fn pedir_texto(&self, uri: &str, token: &str) -> (StatusCode, String, String) {
        let req = Request::get(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .expect("petición");
        let res = self.router.clone().oneshot(req).await.expect("router");
        let status = res.status();
        let tipo = res
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = to_bytes(res.into_body(), usize::MAX).await.expect("cuerpo");
        (status, tipo, String::from_utf8_lossy(&bytes).into_owned())
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

/// Copropiedad dada de alta por la plataforma con su administrador.
pub struct Ph {
    pub api: Api,
    pub base: String,
    pub admin: String,
    pub root: String,
    pub id: String,
}

pub async fn ph(pool: PgPool) -> Ph {
    let api = api(pool).await;
    let root_id = api.registrar("root").await;
    api.hacer_superusuario(&root_id).await;
    let admin_id = api.registrar("admin").await;
    let root = api.token_con_segundo_factor("root").await;
    let (s, v) = api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/copropiedades",
            Some(&root),
            Some(json!({
                "nombre": "Conjunto Alameda",
                "administrador_cuenta_id": admin_id,
                "nit": "800.197.268",
                "tipo": "residencial",
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().expect("id").to_owned();
    Ph {
        admin: api.token("admin").await,
        base: format!("/api/v1/copropiedades/{id}"),
        root,
        id,
        api,
    }
}

impl Ph {
    pub async fn pedir(
        &self,
        metodo: Method,
        ruta: &str,
        cuerpo: Option<Value>,
    ) -> (StatusCode, Value) {
        self.api
            .pedir(
                metodo,
                &format!("{}{ruta}", self.base),
                Some(&self.admin),
                cuerpo,
            )
            .await
    }

    pub async fn estado(&self, estado: &str, token: &str) -> (StatusCode, Value) {
        self.api
            .pedir(
                Method::POST,
                &format!("/api/v1/plataforma/copropiedades/{}/estado", self.id),
                Some(token),
                Some(json!({ "estado": estado })),
            )
            .await
    }

    pub async fn crear_bien(&self, cuerpo: Value) -> Value {
        let (s, v) = self
            .pedir(Method::POST, "/bienes-privados", Some(cuerpo))
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        v
    }

    /// Bien privado `101` con un titular que tiene cuenta (`dueno`, correo
    /// `dueno@correo.co`). Devuelve el id del bien y el de la persona titular.
    pub async fn bien_con_dueno(&self) -> (String, String) {
        let cuenta = self.api.registrar("dueno").await;
        let bien = self
            .crear_bien(json!({ "tipo": "apartamento", "numero": "101" }))
            .await;
        let id = bien["id"].as_str().expect("id").to_owned();
        let (s, v) = self
            .pedir(
                Method::POST,
                &format!("/bienes-privados/{id}/titulares"),
                Some(json!({ "titulares": [{
                    "persona": { "nombre": "Ana Dueña", "documento": "52123456",
                                 "correo": "dueno@correo.co", "cuenta_id": cuenta },
                    "porcentaje": "100"
                }]})),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        (id, v[0]["persona_id"].as_str().expect("persona").to_owned())
    }

    /// Habilita un módulo vendible para la copropiedad.
    pub async fn habilitar(&self, modulo: &str) {
        sqlx::query(
            "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
             VALUES (gen_random_uuid(), $1::uuid, $2, now() - interval '1 day')",
        )
        .bind(&self.id)
        .bind(modulo)
        .execute(&self.api.duenio)
        .await
        .expect("módulo");
    }
}
