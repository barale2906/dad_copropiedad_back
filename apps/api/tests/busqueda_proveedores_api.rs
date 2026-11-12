//! Pruebas funcionales de la búsqueda de proveedores (plan 08, fase 2): un propietario busca
//! desde la app y autoriza el contacto (el aviso al proveedor y el «¿Le sirvió?» quedan en el
//! outbox); la administración genera el enlace público para los grupos de WhatsApp y cualquiera
//! con el enlace ve el directorio sin sesión.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{MIGRACIONES, Ph, ph};
use serde_json::json;
use sqlx::PgPool;

const CABECERA: &str = "x-enlace-directorio";

/// Da de alta «Modistería Rosa» (cobertura nacional, con WhatsApp).
async fn anunciante(p: &Ph) {
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/publicidad/anunciantes",
            Some(&p.root),
            Some(json!({
                "organizacion": {
                    "nit": "900123456-8", "razon_social": "Rosa Pérez",
                    "naturaleza": "natural_con_rut", "tipos": ["otro"]
                },
                "nombre_comercial": "Modistería Rosa",
                "municipio_dane": "11001",
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().expect("id").to_owned();
    let (s, v) = p
        .api
        .pedir(
            Method::PUT,
            &format!("/api/v1/plataforma/publicidad/anunciantes/{id}/ficha"),
            Some(&p.root),
            Some(json!({
                "categorias": ["costura"],
                "cobertura": "nacional",
                "whatsapp": "+57 310 999 8877",
            })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

async fn linea(p: &Ph) {
    sqlx::query(
        "INSERT INTO lineas_whatsapp (id, phone_number_id, numero_visible, nombre)
         VALUES (gen_random_uuid(), 'PNID', '+57 300 000 0000', 'Línea')",
    )
    .execute(&p.api.duenio)
    .await
    .expect("línea");
}

async fn celular(p: &Ph, persona: &str) {
    sqlx::query("UPDATE personas SET telefono = '573001112233' WHERE id = $1::uuid")
        .bind(persona)
        .execute(&p.api.duenio)
        .await
        .expect("celular");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn propietario_busca_y_autoriza_que_lo_contacten(pool: PgPool) {
    let p = ph(pool).await;
    anunciante(&p).await;
    linea(&p).await;
    let (_, persona) = p.bien_con_dueno().await;
    celular(&p, &persona).await;
    let dueno = p.api.token("dueno").await;

    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{}/directorio/busquedas", p.base),
            Some(&dueno),
            Some(json!({ "texto": "Busco una costurera, por favor" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["categoria"]["codigo"], "costura");
    assert_eq!(v["sugerencias"][0]["nombre_comercial"], "Modistería Rosa");
    assert_eq!(v["contactables"], true);
    let solicitud = v["solicitud_id"].as_str().expect("id").to_owned();
    let pendientes: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox WHERE concepto = 'directorio'")
            .fetch_one(&p.api.duenio)
            .await
            .expect("outbox");
    assert_eq!(pendientes, 0, "buscar no comparte nada con el proveedor");

    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{}/directorio/busquedas/{solicitud}/contacto", p.base),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["proveedores"], json!(["Modistería Rosa"]));
    assert_eq!(v["seguimiento"], true);
    let destinos: Vec<String> = sqlx::query_scalar(
        "SELECT destino FROM outbox WHERE concepto = 'directorio' ORDER BY prioridad",
    )
    .fetch_all(&p.api.duenio)
    .await
    .expect("outbox");
    assert_eq!(destinos, ["573109998877", "573001112233"]);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_celular_en_el_censo_no_se_autoriza_el_contacto(pool: PgPool) {
    let p = ph(pool).await;
    anunciante(&p).await;
    linea(&p).await;
    p.bien_con_dueno().await;
    let dueno = p.api.token("dueno").await;
    let (_, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{}/directorio/busquedas", p.base),
            Some(&dueno),
            Some(json!({ "texto": "busco modista" })),
        )
        .await;
    let solicitud = v["solicitud_id"].as_str().expect("id").to_owned();

    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{}/directorio/busquedas/{solicitud}/contacto", p.base),
            Some(&dueno),
            None,
        )
        .await;

    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["codigo"], "validacion");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn buscar_exige_una_persona_del_censo_y_un_texto_entendible(pool: PgPool) {
    let p = ph(pool).await;
    p.bien_con_dueno().await;
    let dueno = p.api.token("dueno").await;

    let (s, v) = p
        .pedir(
            Method::POST,
            "/directorio/busquedas",
            Some(json!({ "texto": "busco costurera" })),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "administrador sin persona: {v}"
    );

    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{}/directorio/busquedas", p.base),
            Some(&dueno),
            Some(json!({ "texto": "busco" })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_enlace_publico_muestra_el_directorio_sin_sesion(pool: PgPool) {
    let p = ph(pool).await;
    anunciante(&p).await;
    linea(&p).await;

    let (s, v) = p
        .pedir(Method::POST, "/directorio/enlace-publico", None)
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(
        v["whatsapp_url"],
        "https://wa.me/573000000000?text=Busco%20"
    );
    let token = v["token"].as_str().expect("token").to_owned();
    let publico = format!("{}/directorio/publico", p.base);

    let (s, v) = p.api.pedir_con_cabecera(&publico, CABECERA, &token).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["copropiedad"].is_string());
    assert_eq!(
        v["fichas"]["datos"][0]["nombre_comercial"],
        "Modistería Rosa"
    );

    let (s, _) = p.api.pedir(Method::GET, &publico, None, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "sin token");
    let (s, v) = p.api.pedir_con_cabecera(&publico, CABECERA, "otro").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{v}");
    assert_eq!(v["error"]["codigo"], "no_autenticado");

    let (s, _) = p
        .pedir(Method::DELETE, "/directorio/enlace-publico", None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = p.api.pedir_con_cabecera(&publico, CABECERA, &token).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "revocado");
    let (s, v) = p
        .pedir(Method::GET, "/directorio/enlace-publico", None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["activo"], false);
    let (s, _) = p
        .pedir(Method::DELETE, "/directorio/enlace-publico", None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_propietario_no_genera_el_enlace_publico(pool: PgPool) {
    let p = ph(pool).await;
    p.bien_con_dueno().await;
    let dueno = p.api.token("dueno").await;

    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{}/directorio/enlace-publico", p.base),
            Some(&dueno),
            None,
        )
        .await;

    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["codigo"], "sin_permiso");
}
