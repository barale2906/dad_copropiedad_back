//! Pruebas funcionales de identidad sobre el router real y PostgreSQL (rol `dad_app`): lo que
//! vería un cliente web o móvil.

mod comun;

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use comun::{Api, MIGRACIONES, URL_PUBLICA, api};
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
        .pedir(Method::GET, "/api/v1/yo/entidades", Some(&token), None)
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
            "/api/v1/yo/entidades",
            Some(&token_admin),
            None,
        )
        .await;
    assert_eq!(mias[0]["entidad_id"], ph["id"]);
    assert_eq!(mias[0]["tipo"], "propiedad_horizontal");
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
    let entidad = format!("/api/v1/entidades/{}", ph["id"].as_str().expect("id"));
    let token_admin = api.token("admin").await;
    let token_vecino = api.token("vecino").await;

    // Sin roles en la PH: para el vecino la copropiedad no existe.
    let (s, v) = api
        .pedir(
            Method::GET,
            &format!("{entidad}/roles"),
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
            &format!("{entidad}/cuentas?identificador=vecino"),
            Some(&token_admin),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(encontrada["id"], vecino.as_str());
    let (_, roles) = api
        .pedir(
            Method::GET,
            &format!("{entidad}/roles"),
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
            &format!("{entidad}/asignaciones"),
            Some(&token_admin),
            Some(json!({ "cuenta_id": vecino, "rol_id": propietario })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    // Ahora entra, pero el rol de propietario no ve los roles.
    let (s, v) = api
        .pedir(
            Method::GET,
            &format!("{entidad}/roles"),
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
            &format!("{entidad}/roles"),
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
    let entidad = format!("/api/v1/entidades/{}", ph["id"].as_str().expect("id"));
    let token = api.token("admin").await;

    let res = api
        .router
        .clone()
        .oneshot(
            Request::post(format!(
                "{entidad}/archivos?nombre=Reglamento.pdf&clasificacion=interno"
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
                "{entidad}/archivos/{}/enlace",
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

#[sqlx::test(migrator = "MIGRACIONES")]
async fn al_iniciar_sesion_llegan_las_entidades_para_elegir_en_cual_trabajar(pool: PgPool) {
    // Preparar: una cuenta que administra una PH y una empresa sin vertical.
    let api = api(pool).await;
    let root = api.registrar("root").await;
    api.hacer_superusuario(&root).await;
    let ana = api.registrar("ana").await;
    let plataforma = api.token_con_segundo_factor("root").await;
    let mut ids = Vec::new();
    for (ruta, nombre) in [
        ("/api/v1/plataforma/copropiedades", "Conjunto Alameda"),
        ("/api/v1/plataforma/entidades", "Gimnasio Fuerza"),
    ] {
        let (s, v) = api
            .pedir(
                Method::POST,
                ruta,
                Some(&plataforma),
                Some(json!({ "nombre": nombre, "administrador_cuenta_id": ana })),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        ids.push(v["id"].clone());
    }

    // Ejecutar
    let inicio = api.login("ana").await;

    // Verificar: ordenadas por nombre, cada una con su tipo.
    let entidades = &inicio["entidades"];
    assert_eq!(entidades.as_array().map(Vec::len), Some(2), "{inicio}");
    assert_eq!(entidades[0]["entidad_id"], ids[0]);
    assert_eq!(entidades[0]["tipo"], "propiedad_horizontal");
    assert_eq!(entidades[1]["entidad_id"], ids[1]);
    assert_eq!(entidades[1]["tipo"], "empresa");
    assert_eq!(entidades[1]["roles"], json!(["administrador"]));

    // Al entrar, las capacidades repiten el tipo para armar los menús.
    let token = inicio["tokens"]["token_acceso"].as_str().expect("token");
    for (id, tipo) in [(&ids[0], "propiedad_horizontal"), (&ids[1], "empresa")] {
        let id = id.as_str().expect("id");
        let (s, c) = api
            .pedir(
                Method::GET,
                &format!("/api/v1/entidades/{id}/capacidades"),
                Some(token),
                None,
            )
            .await;
        assert_eq!((s, &c["tipo"]), (StatusCode::OK, &json!(tipo)), "{c}");
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_entidad_retirada_ya_no_se_ofrece_al_entrar(pool: PgPool) {
    let api = api(pool).await;
    let root = api.registrar("root").await;
    api.hacer_superusuario(&root).await;
    let ana = api.registrar("ana").await;
    let plataforma = api.token_con_segundo_factor("root").await;
    let (s, v) = api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/entidades",
            Some(&plataforma),
            Some(json!({ "nombre": "Tienda Vieja", "administrador_cuenta_id": ana })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    sqlx::query("UPDATE entidades SET estado = 'retirada' WHERE id = $1::uuid")
        .bind(v["id"].as_str().expect("id"))
        .execute(&api.duenio)
        .await
        .expect("retirar");

    let inicio = api.login("ana").await;

    assert_eq!(inicio["entidades"], json!([]));
}

/// Superusuario y plataforma ya designada: la empresa de GISLA con su contadora.
async fn plataforma_designada(api: &Api) -> (String, Value) {
    let root = api.registrar("root").await;
    api.hacer_superusuario(&root).await;
    let contadora = api.registrar("contadora").await;
    let token = api.token_con_segundo_factor("root").await;
    let (s, gisla) = api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/entidades",
            Some(&token),
            Some(json!({ "nombre": "GISLA SAS", "administrador_cuenta_id": contadora })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{gisla}");
    let (s, v) = api
        .pedir(
            Method::PUT,
            "/api/v1/plataforma/entidad-plataforma",
            Some(&token),
            Some(json!({ "entidad_id": gisla["id"] })),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    (token, gisla["id"].clone())
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn quien_opera_la_plataforma_la_recibe_como_entidad_al_entrar(pool: PgPool) {
    let api = api(pool).await;
    let (token_root, gisla) = plataforma_designada(&api).await;

    // El superusuario (con segundo factor) no tiene roles dentro de la empresa, pero sí de
    // plataforma; la contadora entra directo con su rol en la empresa.
    let (s, root) = api
        .pedir(Method::GET, "/api/v1/yo/entidades", Some(&token_root), None)
        .await;
    let contadora = api.login("contadora").await;

    assert_eq!(s, StatusCode::OK);
    assert_eq!(root[0]["entidad_id"], gisla);
    assert_eq!(root[0]["tipo"], "plataforma");
    assert_eq!(root[0]["roles"], json!(["superusuario"]));
    assert_eq!(contadora["entidades"][0]["tipo"], "plataforma");
    assert_eq!(contadora["entidades"][0]["roles"], json!(["administrador"]));
    let token = contadora["tokens"]["token_acceso"].as_str().expect("token");
    let id = gisla.as_str().expect("id");
    let (s, c) = api
        .pedir(
            Method::GET,
            &format!("/api/v1/entidades/{id}/capacidades"),
            Some(token),
            None,
        )
        .await;
    assert_eq!((s, &c["tipo"]), (StatusCode::OK, &json!("plataforma")));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_plataforma_es_una_sola_y_nunca_una_copropiedad(pool: PgPool) {
    let api = api(pool).await;
    let (token, gisla) = plataforma_designada(&api).await;
    let admin = api.registrar("admin").await;
    let mut otras = Vec::new();
    for ruta in [
        "/api/v1/plataforma/copropiedades",
        "/api/v1/plataforma/entidades",
    ] {
        let (s, v) = api
            .pedir(
                Method::POST,
                ruta,
                Some(&token),
                Some(json!({ "nombre": "Otra", "administrador_cuenta_id": admin })),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        otras.push(v["id"].clone());
    }
    let designar = |id: Value| {
        api.pedir(
            Method::PUT,
            "/api/v1/plataforma/entidad-plataforma",
            Some(&token),
            Some(json!({ "entidad_id": id })),
        )
    };

    let (s, v) = designar(otras[0].clone()).await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::CONFLICT, &json!("conflicto")),
        "una PH no puede ser la plataforma"
    );
    let (s, _) = designar(json!(uuid::Uuid::now_v7())).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = designar(otras[1].clone()).await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let marcadas: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM entidades WHERE es_plataforma")
            .fetch_all(&api.duenio)
            .await
            .expect("consulta");
    assert_eq!(marcadas.len(), 1, "solo una entidad es la plataforma");
    assert_ne!(json!(marcadas[0]), gisla, "la anterior pierde la marca");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_permiso_no_se_designa_la_plataforma(pool: PgPool) {
    let api = api(pool).await;
    let soporte = api.registrar("soporte").await;
    sqlx::query(
        "INSERT INTO asignaciones_plataforma (id, cuenta_id, rol_id, vigente_desde)
         VALUES (gen_random_uuid(), $1::uuid, uuid_rol_plantilla('soporte'), now())",
    )
    .bind(&soporte)
    .execute(&api.duenio)
    .await
    .expect("rol de soporte");
    let token = api.token_con_segundo_factor("soporte").await;

    let (s, v) = api
        .pedir(
            Method::PUT,
            "/api/v1/plataforma/entidad-plataforma",
            Some(&token),
            Some(json!({ "entidad_id": uuid::Uuid::now_v7() })),
        )
        .await;

    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::FORBIDDEN, &json!("sin_permiso"))
    );
}
