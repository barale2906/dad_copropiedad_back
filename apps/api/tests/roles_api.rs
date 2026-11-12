//! Pruebas funcionales de roles y asignaciones (ADR 0005) sobre el router real y PostgreSQL:
//! la regla de **no escalamiento** (nadie otorga, edita ni revoca lo que no tiene), las
//! plantillas inmutables y los mínimos que la plataforma y cada copropiedad deben conservar.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{Api, MIGRACIONES, Ph, ph};
use serde_json::{Value, json};
use sqlx::PgPool;

/// Rol propio de la copropiedad creado con `token`; devuelve su id.
async fn crear_rol(ph: &Ph, token: &str, codigo: &str, permisos: &[&str]) -> (StatusCode, Value) {
    ph.api
        .pedir(
            Method::POST,
            &format!("{}/roles", ph.base),
            Some(token),
            Some(json!({ "codigo": codigo, "nombre": codigo, "permisos": permisos })),
        )
        .await
}

async fn editar_permisos(
    ph: &Ph,
    token: &str,
    rol: &str,
    permisos: &[&str],
) -> (StatusCode, Value) {
    ph.api
        .pedir(
            Method::PUT,
            &format!("{}/roles/{rol}/permisos", ph.base),
            Some(token),
            Some(json!({ "permisos": permisos })),
        )
        .await
}

async fn asignar(ph: &Ph, token: &str, cuenta: &str, rol: &str) -> (StatusCode, Value) {
    ph.api
        .pedir(
            Method::POST,
            &format!("{}/asignaciones", ph.base),
            Some(token),
            Some(json!({ "cuenta_id": cuenta, "rol_id": rol })),
        )
        .await
}

async fn capacidades(ph: &Ph, token: &str) -> (StatusCode, Value) {
    ph.api
        .pedir(
            Method::GET,
            &format!("{}/capacidades", ph.base),
            Some(token),
            None,
        )
        .await
}

fn id(v: &Value) -> String {
    v["id"].as_str().expect("id").to_owned()
}

/// Cuenta nueva con un rol propio hecho por el administrador; devuelve (cuenta, token, rol).
async fn cuenta_con_rol(ph: &Ph, usuario: &str, permisos: &[&str]) -> (String, String, String) {
    let cuenta = ph.api.registrar(usuario).await;
    let (s, rol) = crear_rol(ph, &ph.admin, &format!("rol_{usuario}"), permisos).await;
    assert_eq!(s, StatusCode::CREATED, "{rol}");
    let (s, v) = asignar(ph, &ph.admin, &cuenta, &id(&rol)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    (cuenta, ph.api.token(usuario).await, id(&rol))
}

const GESTOR: &[&str] = &["roles.ver", "roles.gestionar", "roles.asignar", "censo.ver"];

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_gestor_de_roles_no_se_da_permisos_que_no_tiene(pool: PgPool) {
    let ph = ph(pool).await;
    let (_, gestor, rol_gestor) = cuenta_con_rol(&ph, "gestor", GESTOR).await;

    let (s, auxiliar) = crear_rol(&ph, &gestor, "auxiliar", &["censo.ver"]).await;
    assert_eq!(s, StatusCode::CREATED, "{auxiliar}");
    let sin_permiso = (StatusCode::FORBIDDEN, json!("sin_permiso"));
    for (rol, permisos) in [
        (id(&auxiliar), vec!["censo.ver", "censo.exportar"]),
        // Tampoco amplía su propio rol.
        (rol_gestor, [GESTOR, &["censo.exportar"]].concat()),
    ] {
        let (s, v) = editar_permisos(&ph, &gestor, &rol, &permisos).await;
        assert_eq!((s, v["error"]["codigo"].clone()), sin_permiso, "{v}");
    }
    let (s, v) = crear_rol(&ph, &gestor, "exportador", &["censo.exportar"]).await;
    assert_eq!((s, v["error"]["codigo"].clone()), sin_permiso);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_gestor_no_asigna_ni_revoca_roles_con_mas_permisos_que_los_suyos(pool: PgPool) {
    let ph = ph(pool).await;
    let (_, gestor, _) = cuenta_con_rol(&ph, "gestor", GESTOR).await;
    let (s, roles) = ph.pedir(Method::GET, "/roles", None).await;
    assert_eq!(s, StatusCode::OK);
    let administrador = roles
        .as_array()
        .and_then(|r| r.iter().find(|r| r["codigo"] == "administrador"))
        .map(id)
        .expect("plantilla administrador");
    let otra = ph.api.registrar("otra").await;

    let (s, _) = asignar(&ph, &gestor, &otra, &administrador).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (s, asignaciones) = ph
        .api
        .pedir(
            Method::GET,
            &format!("{}/asignaciones", ph.base),
            Some(&gestor),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{asignaciones}");
    let del_admin = asignaciones
        .as_array()
        .and_then(|a| a.iter().find(|a| a["rol_id"] == json!(administrador)))
        .map(id)
        .expect("asignación del administrador");
    let (s, _) = ph
        .api
        .pedir(
            Method::DELETE,
            &format!("{}/asignaciones/{del_admin}", ph.base),
            Some(&gestor),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn las_plantillas_no_se_editan(pool: PgPool) {
    let ph = ph(pool).await;
    let (_, roles) = ph.pedir(Method::GET, "/roles", None).await;
    let propietario = roles
        .as_array()
        .and_then(|r| r.iter().find(|r| r["codigo"] == "propietario"))
        .map(id)
        .expect("plantilla propietario");

    let (s, v) = editar_permisos(&ph, &ph.admin, &propietario, &["censo.ver"]).await;

    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::CONFLICT, &json!("conflicto"))
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn editar_y_revocar_un_rol_cambia_de_inmediato_lo_que_puede_su_titular(pool: PgPool) {
    let ph = ph(pool).await;
    let (cuenta, token, rol) = cuenta_con_rol(&ph, "auxiliar", &["censo.ver"]).await;
    assert_eq!(
        capacidades(&ph, &token).await.1["permisos"],
        json!(["censo.ver"])
    );

    let (s, v) = editar_permisos(&ph, &ph.admin, &rol, &["censo.ver", "censo.editar"]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, c) = capacidades(&ph, &token).await;
    assert_eq!(c["permisos"].as_array().map(Vec::len), Some(2), "{c}");

    let (_, asignaciones) = ph.pedir(Method::GET, "/asignaciones", None).await;
    let suya = asignaciones
        .as_array()
        .and_then(|a| a.iter().find(|a| a["cuenta_id"] == json!(cuenta)))
        .map(id)
        .expect("asignación");
    let (s, _) = ph
        .pedir(Method::DELETE, &format!("/asignaciones/{suya}"), None)
        .await;
    assert!(s.is_success(), "{s}");

    // Sin roles vigentes la copropiedad deja de existir para la cuenta.
    assert_eq!(capacidades(&ph, &token).await.0, StatusCode::NOT_FOUND);
    let (_, historial) = ph.pedir(Method::GET, "/asignaciones", None).await;
    let terminada = historial
        .as_array()
        .and_then(|a| a.iter().find(|a| a["id"] == json!(suya)))
        .expect("queda en el historial");
    assert!(terminada["vigente_hasta"].is_string(), "{terminada}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_copropiedad_no_se_queda_sin_administrador(pool: PgPool) {
    let ph = ph(pool).await;
    let (_, asignaciones) = ph.pedir(Method::GET, "/asignaciones", None).await;
    let unica = asignaciones
        .as_array()
        .map(|a| id(&a[0]))
        .expect("asignación del administrador");

    let (s, v) = ph
        .pedir(Method::DELETE, &format!("/asignaciones/{unica}"), None)
        .await;

    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::CONFLICT, &json!("conflicto"))
    );
}

// ---------------------------------------------------------------------------------------------
// Plataforma
// ---------------------------------------------------------------------------------------------

async fn rol_plataforma(api: &Api, codigo: &str) -> String {
    sqlx::query_scalar::<_, uuid::Uuid>("SELECT uuid_rol_plantilla($1)")
        .bind(codigo)
        .fetch_one(&api.duenio)
        .await
        .expect("plantilla de plataforma")
        .to_string()
}

/// Operador de plataforma con la plantilla `codigo`; devuelve (cuenta, token con segundo factor).
async fn operador(ph: &Ph, usuario: &str, codigo: &str) -> (String, String) {
    let cuenta = ph.api.registrar(usuario).await;
    let rol = rol_plataforma(&ph.api, codigo).await;
    let (s, v) = ph
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/asignaciones",
            Some(&ph.root),
            Some(json!({ "cuenta_id": cuenta, "rol_id": rol })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    (cuenta, ph.api.token_con_segundo_factor(usuario).await)
}

async fn bloquear(api: &Api, token: &str, cuenta: &str) -> StatusCode {
    api.pedir(
        Method::POST,
        &format!("/api/v1/plataforma/cuentas/{cuenta}/bloquear"),
        Some(token),
        None,
    )
    .await
    .0
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn soporte_bloquea_cuentas_pero_no_a_un_superusuario(pool: PgPool) {
    let ph = ph(pool).await;
    let (_, soporte) = operador(&ph, "soporte", "soporte").await;
    let root: String = sqlx::query_scalar("SELECT id::text FROM cuentas WHERE usuario = 'root'")
        .fetch_one(&ph.api.duenio)
        .await
        .expect("root");
    let objetivo = ph.api.registrar("objetivo").await;
    let sesion_previa = ph.api.token("objetivo").await;

    assert_eq!(
        bloquear(&ph.api, &soporte, &root).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        bloquear(&ph.api, &soporte, &objetivo).await,
        StatusCode::NO_CONTENT
    );

    let (s, _) = ph
        .api
        .pedir(Method::GET, "/api/v1/yo", Some(&sesion_previa), None)
        .await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "el bloqueo cierra sus sesiones"
    );
    let (s, _) = ph
        .api
        .pedir(
            Method::POST,
            "/api/v1/auth/sesion",
            None,
            Some(json!({ "identificador": "objetivo", "contrasena": comun::CONTRASENA })),
        )
        .await;
    assert!(
        s.is_client_error(),
        "una cuenta bloqueada no inicia sesión: {s}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_operador_sin_un_permiso_no_lo_pone_en_un_rol_ni_asigna_uno_mayor(pool: PgPool) {
    let ph = ph(pool).await;
    // Rol propio con permisos de roles pero sin crear copropiedades.
    let (s, rol) = ph
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/roles",
            Some(&ph.root),
            Some(json!({
                "codigo": "gestor_plataforma", "nombre": "Gestor",
                "permisos": ["plataforma.roles.gestionar", "plataforma.roles.asignar"],
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{rol}");
    let cuenta = ph.api.registrar("gestor").await;
    let (s, _) = ph
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/asignaciones",
            Some(&ph.root),
            Some(json!({ "cuenta_id": cuenta, "rol_id": id(&rol) })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let gestor = ph.api.token_con_segundo_factor("gestor").await;
    let plataforma = |metodo, ruta: String, cuerpo| {
        let (api, gestor) = (&ph.api, &gestor);
        async move { api.pedir(metodo, &ruta, Some(gestor), Some(cuerpo)).await.0 }
    };

    let mayor = json!({ "permisos": ["plataforma.roles.gestionar", "plataforma.roles.asignar", "plataforma.copropiedades.crear"] });
    assert_eq!(
        plataforma(
            Method::PUT,
            format!("/api/v1/plataforma/roles/{}/permisos", id(&rol)),
            mayor
        )
        .await,
        StatusCode::FORBIDDEN
    );
    let otra = ph.api.registrar("otra").await;
    let superusuario = rol_plataforma(&ph.api, "superusuario").await;
    assert_eq!(
        plataforma(
            Method::POST,
            "/api/v1/plataforma/asignaciones".into(),
            json!({ "cuenta_id": otra, "rol_id": superusuario })
        )
        .await,
        StatusCode::FORBIDDEN
    );
    // Un rol de plataforma no lleva permisos de copropiedad.
    assert_eq!(
        plataforma(
            Method::POST,
            "/api/v1/plataforma/roles".into(),
            json!({ "codigo": "mixto", "nombre": "Mixto", "permisos": ["censo.ver"] })
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_plataforma_no_se_queda_sin_superusuario(pool: PgPool) {
    let ph = ph(pool).await;
    let unica: String = sqlx::query_scalar(
        "SELECT a.id::text FROM asignaciones_plataforma a JOIN cuentas c ON c.id = a.cuenta_id
         WHERE c.usuario = 'root'",
    )
    .fetch_one(&ph.api.duenio)
    .await
    .expect("asignación del superusuario");

    let (s, v) = ph
        .api
        .pedir(
            Method::DELETE,
            &format!("/api/v1/plataforma/asignaciones/{unica}"),
            Some(&ph.root),
            None,
        )
        .await;

    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::CONFLICT, &json!("conflicto"))
    );
}
