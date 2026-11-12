//! Pruebas funcionales de la seguridad de las cuentas (ADR 0005) sobre el router real y
//! PostgreSQL (rol `dad_app`): recuperación y cambio de contraseña, verificación del correo,
//! límite de intentos de los códigos, cierre de sesiones y rotación del token de renovación.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{Api, CONTRASENA, MIGRACIONES, api};
use serde_json::{Value, json};
use sqlx::PgPool;

const NUEVA: &str = "otra frase todavía más larga";

/// Código del último correo enviado a `correo` (los códigos solo se guardan como hash; el
/// texto del correo encolado es la única copia en claro).
async fn ultimo_codigo(api: &Api, correo: &str) -> String {
    let texto: String = sqlx::query_scalar(
        "SELECT payload->>'texto' FROM outbox
         WHERE canal = 'correo' AND destino = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(correo)
    .fetch_one(&api.duenio)
    .await
    .expect("correo encolado");
    texto
        .split("es: ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .expect("código en el correo")
        .to_owned()
}

async fn correos_a(api: &Api, correo: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM outbox WHERE canal = 'correo' AND destino = $1")
        .bind(correo)
        .fetch_one(&api.duenio)
        .await
        .expect("conteo")
}

async fn iniciar(api: &Api, usuario: &str, contrasena: &str) -> (StatusCode, Value) {
    api.pedir(
        Method::POST,
        "/api/v1/auth/sesion",
        None,
        Some(json!({ "identificador": usuario, "contrasena": contrasena })),
    )
    .await
}

async fn perfil(api: &Api, token: &str) -> (StatusCode, Value) {
    api.pedir(Method::GET, "/api/v1/yo", Some(token), None)
        .await
}

async fn restablecer(api: &Api, correo: &str, codigo: &str) -> (StatusCode, Value) {
    api.pedir(
        Method::POST,
        "/api/v1/auth/contrasena/restablecer",
        None,
        Some(json!({ "correo": correo, "codigo": codigo, "contrasena_nueva": NUEVA })),
    )
    .await
}

async fn recuperar(api: &Api, correo: &str) -> StatusCode {
    api.pedir(
        Method::POST,
        "/api/v1/auth/contrasena/recuperar",
        None,
        Some(json!({ "correo": correo })),
    )
    .await
    .0
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn restablecer_la_contrasena_cambia_la_clave_y_cierra_las_sesiones(pool: PgPool) {
    let api = api(pool).await;
    api.registrar("ana").await;
    let previo = api.token("ana").await;

    assert_eq!(recuperar(&api, "ana@correo.co").await, StatusCode::ACCEPTED);
    let codigo = ultimo_codigo(&api, "ana@correo.co").await;
    let (s, v) = restablecer(&api, "ana@correo.co", &codigo).await;

    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(perfil(&api, &previo).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        iniciar(&api, "ana", CONTRASENA).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(iniciar(&api, "ana", NUEVA).await.0, StatusCode::OK);
    // Quien recibió el código controla el correo: queda verificado.
    let token = api.token_con("ana", NUEVA).await;
    assert_eq!(perfil(&api, &token).await.1["correo_verificado"], true);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn recuperar_no_revela_si_el_correo_existe(pool: PgPool) {
    let api = api(pool).await;

    assert_eq!(
        recuperar(&api, "nadie@correo.co").await,
        StatusCode::ACCEPTED
    );
    assert_eq!(correos_a(&api, "nadie@correo.co").await, 0);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_codigo_de_recuperacion_sirve_una_sola_vez(pool: PgPool) {
    let api = api(pool).await;
    api.registrar("ana").await;
    recuperar(&api, "ana@correo.co").await;
    let codigo = ultimo_codigo(&api, "ana@correo.co").await;

    assert_eq!(
        restablecer(&api, "ana@correo.co", &codigo).await.0,
        StatusCode::NO_CONTENT
    );
    let (s, v) = restablecer(&api, "ana@correo.co", &codigo).await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("validacion"))
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn cinco_codigos_errados_agotan_el_codigo_aunque_luego_llegue_el_correcto(pool: PgPool) {
    let api = api(pool).await;
    api.registrar("ana").await;
    recuperar(&api, "ana@correo.co").await;
    let codigo = ultimo_codigo(&api, "ana@correo.co").await;
    let errado = if codigo == "000000" {
        "111111"
    } else {
        "000000"
    };

    for _ in 0..5 {
        assert_eq!(
            restablecer(&api, "ana@correo.co", errado).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    // Cada intento fallido quedó contado (la transacción se confirma aunque falle).
    assert_eq!(
        restablecer(&api, "ana@correo.co", &codigo).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(iniciar(&api, "ana", CONTRASENA).await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_correo_se_verifica_con_el_codigo_del_registro_y_no_con_otro(pool: PgPool) {
    let api = api(pool).await;
    api.registrar("ana").await;
    let token = api.token("ana").await;
    let codigo = ultimo_codigo(&api, "ana@correo.co").await;
    let errado = if codigo == "000000" {
        "111111"
    } else {
        "000000"
    };

    let (s, _) = api
        .pedir(
            Method::POST,
            "/api/v1/yo/correo/verificar",
            Some(&token),
            Some(json!({ "codigo": errado })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(perfil(&api, &token).await.1["correo_verificado"], false);

    let (s, v) = api
        .pedir(
            Method::POST,
            "/api/v1/yo/correo/verificar",
            Some(&token),
            Some(json!({ "codigo": codigo })),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(perfil(&api, &token).await.1["correo_verificado"], true);
    // Ya verificado, no hay nada que reenviar.
    let (s, _) = api
        .pedir(
            Method::POST,
            "/api/v1/yo/correo/reenviar",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn reenviar_la_verificacion_manda_un_codigo_nuevo(pool: PgPool) {
    let api = api(pool).await;
    api.registrar("ana").await;
    let token = api.token("ana").await;
    let antes = correos_a(&api, "ana@correo.co").await;

    let (s, _) = api
        .pedir(
            Method::POST,
            "/api/v1/yo/correo/reenviar",
            Some(&token),
            None,
        )
        .await;

    assert_eq!(s, StatusCode::ACCEPTED);
    assert_eq!(correos_a(&api, "ana@correo.co").await, antes + 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn cambiar_la_contrasena_exige_la_actual_y_cierra_las_sesiones(pool: PgPool) {
    let api = api(pool).await;
    api.registrar("ana").await;
    let token = api.token("ana").await;
    let otro_equipo = api.token("ana").await;
    let cambio =
        |actual: &'static str| json!({ "contrasena_actual": actual, "contrasena_nueva": NUEVA });

    let (s, v) = api
        .pedir(
            Method::PUT,
            "/api/v1/yo/contrasena",
            Some(&token),
            Some(cambio("no es la clave")),
        )
        .await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::UNAUTHORIZED, &json!("credenciales_invalidas"))
    );
    assert_eq!(perfil(&api, &token).await.0, StatusCode::OK);

    let (s, _) = api
        .pedir(
            Method::PUT,
            "/api/v1/yo/contrasena",
            Some(&token),
            Some(cambio(CONTRASENA)),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(perfil(&api, &token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(perfil(&api, &otro_equipo).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(iniciar(&api, "ana", NUEVA).await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn cerrar_todas_las_sesiones_revoca_cada_equipo_de_la_cuenta_y_no_otras(pool: PgPool) {
    let api = api(pool).await;
    api.registrar("ana").await;
    api.registrar("beto").await;
    let (celular, portatil, de_beto) = (
        api.token("ana").await,
        api.token("ana").await,
        api.token("beto").await,
    );

    let (s, _) = api
        .pedir(Method::DELETE, "/api/v1/yo/sesiones", Some(&celular), None)
        .await;

    assert_eq!(s, StatusCode::OK);
    assert_eq!(perfil(&api, &celular).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(perfil(&api, &portatil).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(perfil(&api, &de_beto).await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn renovar_rota_el_token_y_reusar_uno_viejo_revoca_la_sesion(pool: PgPool) {
    let api = api(pool).await;
    api.registrar("ana").await;
    let inicio = api.login("ana").await;
    let primero = inicio["tokens"]["token_renovacion"].clone();
    let renovar = |token: Value| {
        let api = &api;
        async move {
            api.pedir(
                Method::POST,
                "/api/v1/auth/sesion/renovar",
                None,
                Some(json!({ "token_renovacion": token })),
            )
            .await
        }
    };

    let (s, nuevos) = renovar(primero.clone()).await;
    assert_eq!(s, StatusCode::OK, "{nuevos}");
    let acceso = nuevos["token_acceso"].as_str().expect("acceso").to_owned();
    assert_ne!(nuevos["token_renovacion"], primero);
    assert_eq!(perfil(&api, &acceso).await.0, StatusCode::OK);

    // El token ya usado vuelve a llegar (robo o dos pestañas): se revoca la sesión entera.
    assert_eq!(renovar(primero).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(perfil(&api, &acceso).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        renovar(nuevos["token_renovacion"].clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
}
