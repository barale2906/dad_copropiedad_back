//! Pruebas funcionales de identidad sobre el router real y PostgreSQL (rol `dad_app`): lo que
//! vería un cliente web o móvil.

mod comun;

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use comun::{MIGRACIONES, URL_PUBLICA, api};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
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
