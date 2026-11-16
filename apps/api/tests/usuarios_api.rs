//! Pruebas funcionales de los usuarios (plan 20) sobre el router real y PostgreSQL (rol
//! `dad_app`): sin registro abierto, la plataforma y cada entidad crean los usuarios, que
//! definen su contraseña con el código que les llega al correo.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{Api, MIGRACIONES, api_sin_registro};
use serde_json::{Value, json};
use sqlx::PgPool;

/// Superusuario sembrado en la BD (sin registro abierto no hay otra forma, como `dad_admin`).
async fn superusuario(api: &Api) -> String {
    sqlx::query(
        "INSERT INTO cuentas (id, nombre_mostrar, usuario, correo, hash_contrasena,
                              contrasena_actualizada_en, creada_en)
         VALUES (gen_random_uuid(), 'Root', 'root', 'root@plataforma.co', NULL, NULL, now())",
    )
    .execute(&api.duenio)
    .await
    .expect("cuenta root");
    let id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM cuentas WHERE usuario = 'root'")
        .fetch_one(&api.duenio)
        .await
        .expect("id");
    api.hacer_superusuario(&id.to_string()).await;
    // Sin código de por medio: la contraseña de prueba se define como la definiría `dad_admin`.
    sqlx::query(
        "INSERT INTO codigos_verificacion (id, cuenta_id, proposito, hash_codigo, expira_en,
                                           creado_en)
         VALUES (gen_random_uuid(), $1, 'definir_contrasena',
                 encode(sha256('111111'::bytea), 'hex'), now() + interval '1 hour', now())",
    )
    .bind(id)
    .execute(&api.duenio)
    .await
    .expect("código");
    let (s, v) = api
        .pedir(
            Method::POST,
            "/api/v1/auth/contrasena/restablecer",
            None,
            Some(json!({ "correo": "root@plataforma.co", "codigo": "111111",
                         "contrasena_nueva": comun::CONTRASENA })),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    api.token_con_segundo_factor("root").await
}

async fn alta_ph(api: &Api, root: &str, administrador: Value) -> (StatusCode, Value) {
    api.pedir(
        Method::POST,
        "/api/v1/plataforma/copropiedades",
        Some(root),
        Some(json!({ "nombre": "Conjunto Alameda", "administrador": administrador })),
    )
    .await
}

async fn rol_id(api: &Api, entidad: &str, token: &str, codigo: &str) -> String {
    let (s, roles) = api
        .pedir(Method::GET, &format!("{entidad}/roles"), Some(token), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{roles}");
    roles
        .as_array()
        .expect("lista")
        .iter()
        .find(|r| r["codigo"] == codigo)
        .and_then(|r| r["id"].as_str())
        .expect("rol")
        .to_owned()
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn registro_cerrado_no_crea_cuentas(pool: PgPool) {
    let api = api_sin_registro(pool).await;

    let (s, v) = api
        .pedir(
            Method::POST,
            "/api/v1/auth/registro",
            None,
            Some(json!({ "nombre": "Ana", "correo": "ana@correo.co",
                         "contrasena": comun::CONTRASENA })),
        )
        .await;

    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    let cuentas: i64 = sqlx::query_scalar("SELECT count(*) FROM cuentas")
        .fetch_one(&api.duenio)
        .await
        .expect("conteo");
    assert_eq!(cuentas, 0);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn alta_con_administrador_nuevo_y_el_administrador_crea_su_equipo(pool: PgPool) {
    let api = api_sin_registro(pool).await;
    let root = superusuario(&api).await;

    let (s, ph) = alta_ph(
        &api,
        &root,
        json!({ "nombre": "Ana Administradora", "correo": "ana@correo.co" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{ph}");
    let entidad = format!("/api/v1/entidades/{}", ph["id"].as_str().expect("id"));
    // Sin contraseña todavía: no entra.
    let (s, _) = api
        .pedir(
            Method::POST,
            "/api/v1/auth/sesion",
            None,
            Some(json!({ "identificador": "ana@correo.co", "contrasena": comun::CONTRASENA })),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    api.definir_contrasena("ana@correo.co").await;
    let ana = api.token("ana@correo.co").await;

    let contador = rol_id(&api, &entidad, &ana, "contador").await;
    let (s, creado) = api
        .pedir(
            Method::POST,
            &format!("{entidad}/usuarios"),
            Some(&ana),
            Some(
                json!({ "nombre": "Luis Contador", "correo": "luis@correo.co",
                         "rol_id": contador, "usuario": "luis" }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{creado}");
    assert_eq!(creado["cuenta_nueva"], true);

    let (s, usuarios) = api
        .pedir(
            Method::GET,
            &format!("{entidad}/usuarios"),
            Some(&ana),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{usuarios}");
    let luis = usuarios["datos"]
        .as_array()
        .expect("datos")
        .iter()
        .find(|u| u["correo"] == "luis@correo.co")
        .expect("luis");
    assert_eq!(luis["contrasena_pendiente"], true);
    assert_eq!(luis["roles"], json!(["contador"]));

    let (s, _) = api
        .pedir(
            Method::POST,
            &format!(
                "{entidad}/usuarios/{}/codigo-contrasena",
                creado["cuenta_id"].as_str().expect("id")
            ),
            Some(&ana),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    api.definir_contrasena("luis@correo.co").await;
    let luis = api.token("luis").await;
    let (s, _) = api
        .pedir(
            Method::GET,
            &format!("{entidad}/capacidades"),
            Some(&luis),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn correo_existente_recibe_el_rol_en_otra_entidad_sin_cuenta_nueva(pool: PgPool) {
    let api = api_sin_registro(pool).await;
    let root = superusuario(&api).await;
    let (_, primera) = alta_ph(
        &api,
        &root,
        json!({ "nombre": "Ana", "correo": "ana@correo.co" }),
    )
    .await;
    api.definir_contrasena("ana@correo.co").await;

    let (s, segunda) = alta_ph(
        &api,
        &root,
        json!({ "nombre": "Ana con otro nombre", "correo": "ana@correo.co" }),
    )
    .await;

    assert_eq!(s, StatusCode::CREATED, "{segunda}");
    let ana = api.token("ana@correo.co").await;
    let (s, mias) = api
        .pedir(Method::GET, "/api/v1/yo/entidades", Some(&ana), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(mias.as_array().expect("lista").len(), 2, "{mias} {primera}");
    let asunto: String = sqlx::query_scalar(
        "SELECT payload->>'asunto' FROM outbox WHERE destino = 'ana@correo.co' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&api.duenio)
    .await
    .expect("aviso");
    assert_eq!(asunto, "Tiene un acceso nuevo en DAD Copropiedad");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn alta_exige_un_solo_administrador(pool: PgPool) {
    let api = api_sin_registro(pool).await;
    let root = superusuario(&api).await;

    let (s, v) = api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/entidades",
            Some(&root),
            Some(json!({ "nombre": "Gimnasio Fuerza" })),
        )
        .await;

    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_plataforma_crea_su_usuario_con_rol_y_lo_bloquea_y_desbloquea(pool: PgPool) {
    let api = api_sin_registro(pool).await;
    let root = superusuario(&api).await;
    let (s, roles) = api
        .pedir(Method::GET, "/api/v1/plataforma/roles", Some(&root), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let soporte = roles
        .as_array()
        .expect("lista")
        .iter()
        .find(|r| r["codigo"] == "soporte")
        .and_then(|r| r["id"].as_str())
        .expect("rol soporte")
        .to_owned();

    let (s, creado) = api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/cuentas",
            Some(&root),
            Some(
                json!({ "nombre": "Laura Soporte", "correo": "laura@plataforma.co",
                         "rol_plataforma_id": soporte }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{creado}");
    assert_eq!(creado["cuenta_nueva"], true);
    assert!(creado["asignacion_id"].is_string());

    let consulta = "/api/v1/plataforma/cuentas?identificador=laura@plataforma.co";
    let (s, cuenta) = api.pedir(Method::GET, consulta, Some(&root), None).await;
    assert_eq!(s, StatusCode::OK, "{cuenta}");
    assert_eq!(cuenta["contrasena_pendiente"], true);
    api.definir_contrasena("laura@plataforma.co").await;
    let id = creado["cuenta_id"].as_str().expect("id");

    let (s, _) = api
        .pedir(
            Method::POST,
            &format!("/api/v1/plataforma/cuentas/{id}/bloquear"),
            Some(&root),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, cuenta) = api.pedir(Method::GET, consulta, Some(&root), None).await;
    assert_eq!(cuenta["estado"], "bloqueada");
    let (s, v) = api
        .pedir(
            Method::POST,
            &format!("/api/v1/plataforma/cuentas/{id}/desbloquear"),
            Some(&root),
            Some(json!({ "motivo": "Confirmó su identidad" })),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (s, v) = api
        .pedir(
            Method::POST,
            &format!("/api/v1/plataforma/cuentas/{id}/desbloquear"),
            Some(&root),
            Some(json!({ "motivo": "Otra vez" })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let _ = api.token("laura@plataforma.co").await;
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn carga_masiva_de_usuarios_con_vista_previa_y_confirmacion(pool: PgPool) {
    let api = api_sin_registro(pool).await;
    let root = superusuario(&api).await;
    let (_, ph) = alta_ph(
        &api,
        &root,
        json!({ "nombre": "Ana", "correo": "ana@correo.co" }),
    )
    .await;
    let entidad = format!("/api/v1/entidades/{}", ph["id"].as_str().expect("id"));
    api.definir_contrasena("ana@correo.co").await;
    let ana = api.token("ana@correo.co").await;

    let con_error =
        "nombre;correo;rol\nLuis;luis@correo.co;contador\nMarta;marta@correo.co;no_existe\n";
    let (s, vista) = api
        .subir(
            &format!("{entidad}/usuarios/importaciones?nombre=usuarios.csv"),
            &ana,
            con_error.as_bytes().to_vec(),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{vista}");
    assert_eq!(vista["importacion"]["estado"], "con_errores");
    assert_eq!(vista["importacion"]["errores"][0]["fila"], 3);

    let valida = "nombre;correo;rol;telefono\nLuis;luis@correo.co;contador;573001234567\n\
                  Marta;marta@correo.co;vigilante;\n";
    let (s, vista) = api
        .subir(
            &format!("{entidad}/usuarios/importaciones?nombre=usuarios.csv"),
            &ana,
            valida.as_bytes().to_vec(),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{vista}");
    assert_eq!(vista["importacion"]["estado"], "validada");
    assert_eq!(vista["filas"].as_array().expect("filas").len(), 2);
    let id = vista["importacion"]["id"].as_str().expect("id");

    let (s, aplicada) = api
        .pedir(
            Method::POST,
            &format!("{entidad}/usuarios/importaciones/{id}/confirmar"),
            Some(&ana),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{aplicada}");
    assert_eq!(aplicada["resumen"]["cuentas_nuevas"], 2);
    let (_, usuarios) = api
        .pedir(
            Method::GET,
            &format!("{entidad}/usuarios"),
            Some(&ana),
            None,
        )
        .await;
    assert_eq!(usuarios["datos"].as_array().expect("datos").len(), 3);
    let (s, _) = api
        .pedir(
            Method::POST,
            &format!("{entidad}/usuarios/importaciones/{id}/confirmar"),
            Some(&ana),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
}
