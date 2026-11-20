//! Pruebas funcionales de la consola sobre las entidades (plan 20, fase 2): listado y estado de
//! PH y empresas, gestión de usuarios de cualquier entidad y acceso de soporte de solo lectura.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{MIGRACIONES, Ph, ph};
use serde_json::json;
use sqlx::PgPool;

async fn empresa(ph: &Ph) -> String {
    let (s, v) = ph
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/entidades",
            Some(&ph.root),
            Some(json!({ "nombre": "Gimnasio Fuerza", "nit": "900123456-8",
                         "administrador": { "nombre": "Gerente", "correo": "gerente@gym.co" } })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().expect("id").to_owned()
}

/// Cuenta de la plataforma con el rol plantilla `soporte` y una sesión con segundo factor.
async fn soporte(ph: &Ph) -> String {
    let id = ph.api.registrar("laura").await;
    sqlx::query(
        "INSERT INTO asignaciones_plataforma (id, cuenta_id, rol_id, vigente_desde)
         VALUES (gen_random_uuid(), $1::uuid, uuid_rol_plantilla('soporte'), now())",
    )
    .bind(&id)
    .execute(&ph.api.duenio)
    .await
    .expect("rol soporte");
    ph.api.token_con_segundo_factor("laura").await
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn lista_ph_y_empresas_y_suspende_una_empresa(pool: PgPool) {
    let ph = ph(pool).await;
    let gym = empresa(&ph).await;

    let (s, todas) = ph
        .api
        .pedir(
            Method::GET,
            "/api/v1/plataforma/entidades",
            Some(&ph.root),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{todas}");
    let tipos: Vec<_> = todas["datos"]
        .as_array()
        .expect("datos")
        .iter()
        .map(|e| e["tipo"].as_str().expect("tipo").to_owned())
        .collect();
    assert!(tipos.contains(&"propiedad_horizontal".into()) && tipos.contains(&"empresa".into()));
    let (_, buscadas) = ph
        .api
        .pedir(
            Method::GET,
            "/api/v1/plataforma/entidades?q=gimnasio",
            Some(&ph.root),
            None,
        )
        .await;
    assert_eq!(buscadas["datos"].as_array().expect("datos").len(), 1);

    let estado = format!("/api/v1/plataforma/entidades/{gym}/estado");
    for (hacia, esperado) in [
        ("activa", "activa"),
        ("suspendida", "suspendida"),
        ("activa", "activa"),
    ] {
        let (s, v) = ph
            .api
            .pedir(
                Method::POST,
                &estado,
                Some(&ph.root),
                Some(json!({ "estado": hacia })),
            )
            .await;
        assert_eq!(
            (s, v["estado"].as_str()),
            (StatusCode::OK, Some(esperado)),
            "{v}"
        );
    }
    let (_, una) = ph
        .api
        .pedir(
            Method::GET,
            "/api/v1/plataforma/entidades?q=gimnasio",
            Some(&ph.root),
            None,
        )
        .await;
    assert_eq!(una["datos"][0]["estado"], "activa");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn solo_con_permiso_la_plataforma_gestiona_usuarios_de_una_entidad(pool: PgPool) {
    let ph = ph(pool).await;
    let laura = soporte(&ph).await;
    let base = format!("/api/v1/plataforma/entidades/{}", ph.id);

    let (s, _) = ph
        .api
        .pedir(Method::GET, &format!("{base}/usuarios"), Some(&laura), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (s, roles) = ph
        .api
        .pedir(Method::GET, &format!("{base}/roles"), Some(&ph.root), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{roles}");
    let contador = roles
        .as_array()
        .expect("roles")
        .iter()
        .find(|r| r["codigo"] == "contador")
        .and_then(|r| r["id"].as_str())
        .expect("contador")
        .to_owned();
    let (s, creado) = ph
        .api
        .pedir(
            Method::POST,
            &format!("{base}/usuarios"),
            Some(&ph.root),
            Some(json!({ "nombre": "Luis", "correo": "luis@correo.co", "rol_id": contador })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{creado}");
    let (_, usuarios) = ph
        .api
        .pedir(
            Method::GET,
            &format!("{base}/usuarios"),
            Some(&ph.root),
            None,
        )
        .await;
    assert!(
        usuarios["datos"]
            .as_array()
            .expect("datos")
            .iter()
            .any(|u| u["correo"] == "luis@correo.co")
    );
    let gestiones: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM auditoria_plataforma WHERE accion = 'plataforma.entidad.gestionar_usuarios'",
    )
    .fetch_one(&ph.api.duenio)
    .await
    .expect("auditoría");
    assert!(gestiones >= 3);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn soporte_entra_en_solo_lectura_y_sin_datos_sensibles(pool: PgPool) {
    let ph = ph(pool).await;
    let laura = soporte(&ph).await;
    let capacidades = format!("{}/capacidades", ph.entidad);

    let (s, _) = ph
        .api
        .pedir(Method::GET, &capacidades, Some(&laura), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, v) = ph
        .api
        .pedir(
            Method::POST,
            &format!("/api/v1/plataforma/entidades/{}/soporte", ph.id),
            Some(&laura),
            Some(json!({ "motivo": "Ticket 4521" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    let (s, c) = ph
        .api
        .pedir(Method::GET, &capacidades, Some(&laura), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{c}");
    assert_eq!(c["modo_soporte"], true);
    let permisos: Vec<&str> = c["permisos"]
        .as_array()
        .expect("permisos")
        .iter()
        .filter_map(|p| p.as_str())
        .collect();
    assert!(permisos.contains(&"personas.ver"));
    assert!(!permisos.contains(&"personas.datos_sensibles.ver"));
    assert!(!permisos.iter().any(|p| !p.contains(".ver")));
    let (s, _) = ph
        .api
        .pedir(
            Method::GET,
            &format!("{}/personas", ph.base),
            Some(&laura),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = ph
        .api
        .pedir(
            Method::PATCH,
            &ph.entidad,
            Some(&laura),
            Some(json!({ "nombre": "Cambiado por soporte" })),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let en_la_entidad: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM auditoria WHERE tenant_id = $1::uuid AND accion = 'plataforma.soporte.abrir'",
    )
    .bind(&ph.id)
    .fetch_one(&ph.api.duenio)
    .await
    .expect("auditoría");
    assert_eq!(en_la_entidad, 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn filtra_las_entidades_por_tipo_y_estado_y_da_una_por_id(pool: PgPool) {
    let ph = ph(pool).await;
    let gym = empresa(&ph).await;
    let pedir = |ruta: String| {
        let ph = &ph;
        async move { ph.api.pedir(Method::GET, &ruta, Some(&ph.root), None).await }
    };
    let ids = |v: &serde_json::Value| -> Vec<String> {
        v["datos"]
            .as_array()
            .expect("datos")
            .iter()
            .map(|e| e["id"].as_str().expect("id").to_owned())
            .collect()
    };

    let (s, empresas) = pedir("/api/v1/plataforma/entidades?tipo=empresa".into()).await;
    assert_eq!(s, StatusCode::OK, "{empresas}");
    assert_eq!(ids(&empresas), vec![gym.clone()]);
    let (_, phs) = pedir("/api/v1/plataforma/entidades?tipo=propiedad_horizontal".into()).await;
    assert_eq!(ids(&phs), vec![ph.id.clone()]);
    let (_, retiradas) = pedir("/api/v1/plataforma/entidades?estado=retirada".into()).await;
    assert!(ids(&retiradas).is_empty());
    let (s, v) = pedir("/api/v1/plataforma/entidades?tipo=gimnasio".into()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");

    let (s, una) = pedir(format!("/api/v1/plataforma/entidades/{gym}")).await;
    assert_eq!(s, StatusCode::OK, "{una}");
    assert_eq!(
        (una["nombre"].as_str(), una["tipo"].as_str()),
        (Some("Gimnasio Fuerza"), Some("empresa"))
    );
    let (s, _) = pedir(format!(
        "/api/v1/plataforma/entidades/{}",
        uuid::Uuid::now_v7()
    ))
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn lista_las_asignaciones_de_plataforma_de_una_cuenta_para_terminarlas(pool: PgPool) {
    let ph = ph(pool).await;
    let _ = soporte(&ph).await;
    let laura =
        sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM cuentas WHERE usuario = 'laura'")
            .fetch_one(&ph.api.duenio)
            .await
            .expect("cuenta");
    let ruta = format!("/api/v1/plataforma/asignaciones?cuenta_id={laura}");

    let (s, v) = ph.api.pedir(Method::GET, &ruta, Some(&ph.root), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let lista = v.as_array().expect("lista");
    assert_eq!(lista.len(), 1, "{v}");
    assert!(lista[0]["vigente_hasta"].is_null());
    let id = lista[0]["id"].as_str().expect("id");

    let (s, _) = ph
        .api
        .pedir(
            Method::DELETE,
            &format!("/api/v1/plataforma/asignaciones/{id}"),
            Some(&ph.root),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, v) = ph.api.pedir(Method::GET, &ruta, Some(&ph.root), None).await;
    assert!(
        v[0]["vigente_hasta"].is_string(),
        "queda en el historial: {v}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_permiso_de_roles_no_ve_las_asignaciones_de_plataforma(pool: PgPool) {
    let ph = ph(pool).await;
    let laura = soporte(&ph).await;

    let (s, v) = ph
        .api
        .pedir(
            Method::GET,
            "/api/v1/plataforma/asignaciones",
            Some(&laura),
            None,
        )
        .await;

    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["codigo"], json!("sin_permiso"));
}
