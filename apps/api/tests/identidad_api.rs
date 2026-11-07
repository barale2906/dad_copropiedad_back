//! Pruebas funcionales de identidad sobre el router real y PostgreSQL (rol `dad_app`): lo que
//! vería un cliente web o móvil.

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

static MIGRACIONES: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
const URL_PUBLICA: &str = "http://localhost:8080";
const CONTRASENA: &str = "una frase bien larga";

struct Api {
    router: Router,
    duenio: PgPool,
    _dir: DirTemporal,
}

struct DirTemporal(std::path::PathBuf);

impl Drop for DirTemporal {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn api(pool: PgPool) -> Api {
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
    async fn pedir(
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

    async fn registrar(&self, usuario: &str) -> String {
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

    async fn login(&self, usuario: &str) -> Value {
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

    async fn token(&self, usuario: &str) -> String {
        self.login(usuario).await["tokens"]["token_acceso"]
            .as_str()
            .expect("token")
            .to_owned()
    }

    async fn hacer_superusuario(&self, cuenta: &str) {
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
    async fn token_con_segundo_factor(&self, usuario: &str) -> String {
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

#[sqlx::test(migrator = "MIGRACIONES")]
async fn registro_inicio_de_sesion_perfil_y_401(pool: PgPool) {
    let api = api(pool).await;
    let id = api.registrar("ana").await;

    let token = api.token("ana@correo.co").await;
    let (s, perfil) = api
        .pedir(Method::GET, "/api/v1/yo", Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{perfil}");
    assert_eq!(perfil["id"], id.as_str());
    assert_eq!(perfil["correo_verificado"], false);
    let (s, mias) = api
        .pedir(Method::GET, "/api/v1/yo/copropiedades", Some(&token), None)
        .await;
    assert_eq!((s, mias), (StatusCode::OK, json!([])));

    let (s, v) = api.pedir(Method::GET, "/api/v1/yo", None, None).await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::UNAUTHORIZED, &json!("no_autenticado"))
    );
    let (s, v) = api
        .pedir(
            Method::POST,
            "/api/v1/auth/sesion",
            None,
            Some(json!({ "identificador": "ana", "contrasena": "equivocada-pero-larga" })),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(v["error"]["codigo"], "credenciales_invalidas");

    // Cerrar sesión invalida el token.
    let (s, _) = api
        .pedir(Method::DELETE, "/api/v1/auth/sesion", Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = api
        .pedir(Method::GET, "/api/v1/yo", Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn plataforma_exige_segundo_factor_y_da_de_alta_una_copropiedad(pool: PgPool) {
    let api = api(pool).await;
    let root = api.registrar("root").await;
    api.hacer_superusuario(&root).await;
    let admin = api.registrar("admin").await;
    let alta = json!({ "nombre": "Conjunto Alameda", "administrador_cuenta_id": admin });

    let sin_2fa = api.token("root").await;
    let (s, v) = api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/copropiedades",
            Some(&sin_2fa),
            Some(alta.clone()),
        )
        .await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::FORBIDDEN, &json!("segundo_factor_requerido"))
    );

    let con_2fa = api.token_con_segundo_factor("root").await;
    let (s, ph) = api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/copropiedades",
            Some(&con_2fa),
            Some(alta),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{ph}");

    let token_admin = api.token("admin").await;
    let (_, mias) = api
        .pedir(
            Method::GET,
            "/api/v1/yo/copropiedades",
            Some(&token_admin),
            None,
        )
        .await;
    assert_eq!(mias[0]["copropiedad_id"], ph["id"]);
    assert_eq!(mias[0]["roles"], json!(["administrador"]));
    // Una cuenta sin roles de plataforma no opera la plataforma.
    let (s, _) = api
        .pedir(
            Method::GET,
            "/api/v1/plataforma/copropiedades",
            Some(&token_admin),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn permisos_por_copropiedad_y_aislamiento(pool: PgPool) {
    let api = api(pool).await;
    let root = api.registrar("root").await;
    api.hacer_superusuario(&root).await;
    let admin = api.registrar("admin").await;
    let vecino = api.registrar("vecino").await;
    let token_root = api.token_con_segundo_factor("root").await;
    let (_, ph) = api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/copropiedades",
            Some(&token_root),
            Some(json!({ "nombre": "Conjunto Alameda", "administrador_cuenta_id": admin })),
        )
        .await;
    let base = format!("/api/v1/copropiedades/{}", ph["id"].as_str().expect("id"));
    let token_admin = api.token("admin").await;
    let token_vecino = api.token("vecino").await;

    // Sin roles en la PH: para el vecino la copropiedad no existe.
    let (s, v) = api
        .pedir(
            Method::GET,
            &format!("{base}/roles"),
            Some(&token_vecino),
            None,
        )
        .await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::NOT_FOUND, &json!("no_encontrado"))
    );

    // El administrador lo busca y le asigna el rol de propietario.
    let (s, encontrada) = api
        .pedir(
            Method::GET,
            &format!("{base}/cuentas?identificador=vecino"),
            Some(&token_admin),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(encontrada["id"], vecino.as_str());
    let (_, roles) = api
        .pedir(
            Method::GET,
            &format!("{base}/roles"),
            Some(&token_admin),
            None,
        )
        .await;
    let propietario = roles
        .as_array()
        .expect("lista")
        .iter()
        .find(|r| r["codigo"] == "propietario")
        .expect("plantilla")["id"]
        .clone();
    let (s, v) = api
        .pedir(
            Method::POST,
            &format!("{base}/asignaciones"),
            Some(&token_admin),
            Some(json!({ "cuenta_id": vecino, "rol_id": propietario })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    // Ahora entra, pero el rol de propietario no ve los roles.
    let (s, v) = api
        .pedir(
            Method::GET,
            &format!("{base}/roles"),
            Some(&token_vecino),
            None,
        )
        .await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::FORBIDDEN, &json!("sin_permiso"))
    );
    // El superusuario entra a cualquier PH.
    let (s, _) = api
        .pedir(
            Method::GET,
            &format!("{base}/roles"),
            Some(&token_root),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn subir_un_archivo_y_descargarlo_con_su_enlace(pool: PgPool) {
    let api = api(pool).await;
    let root = api.registrar("root").await;
    api.hacer_superusuario(&root).await;
    let admin = api.registrar("admin").await;
    let token_root = api.token_con_segundo_factor("root").await;
    let (_, ph) = api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/copropiedades",
            Some(&token_root),
            Some(json!({ "nombre": "Conjunto Alameda", "administrador_cuenta_id": admin })),
        )
        .await;
    let base = format!("/api/v1/copropiedades/{}", ph["id"].as_str().expect("id"));
    let token = api.token("admin").await;

    let res = api
        .router
        .clone()
        .oneshot(
            Request::post(format!(
                "{base}/archivos?nombre=Reglamento.pdf&clasificacion=interno"
            ))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(&b"%PDF-1.7\nreglamento"[..]))
            .expect("petición"),
        )
        .await
        .expect("router");
    assert_eq!(res.status(), StatusCode::CREATED);
    let archivo: Value =
        serde_json::from_slice(&to_bytes(res.into_body(), usize::MAX).await.expect("cuerpo"))
            .expect("json");
    assert_eq!(archivo["tipo"], "application/pdf");

    let (s, enlace) = api
        .pedir(
            Method::GET,
            &format!(
                "{base}/archivos/{}/enlace",
                archivo["id"].as_str().expect("id")
            ),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{enlace}");
    let url = enlace["url"].as_str().expect("url");
    let ruta = url.strip_prefix(URL_PUBLICA).expect("de la API");
    let res = api
        .router
        .clone()
        .oneshot(Request::get(ruta).body(Body::empty()).expect("petición"))
        .await
        .expect("router");
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), usize::MAX).await.expect("cuerpo");
    assert_eq!(&bytes[..], b"%PDF-1.7\nreglamento");
}
