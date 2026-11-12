//! Pruebas funcionales de la publicidad de la plataforma (plan 08, fases 3 a 6): un comercio
//! se registra en el portal, la moderación lo verifica y aprueba su campaña, y solo el
//! propietario que autorizó la publicidad (frente a la plataforma) ve la pieza, la reporta y
//! obtiene el cupón; la administración de la PH no puede autorizar por él.

mod comun;

use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use comun::{MIGRACIONES, Ph, ph};
use serde_json::{Value, json};
use sqlx::PgPool;

/// Registra a «rosa» como anunciante, la verifica y publica su ficha. Devuelve (token, id).
async fn anunciante(p: &Ph) -> (String, String) {
    p.api.registrar("rosa").await;
    let rosa = p.api.token("rosa").await;
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            "/api/v1/anunciantes",
            Some(&rosa),
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
    assert_eq!(v["estado"], "pendiente");
    let id = v["id"].as_str().expect("id").to_owned();
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("/api/v1/plataforma/publicidad/anunciantes/{id}/estado"),
            Some(&p.root),
            Some(json!({ "estado": "verificado" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = p
        .api
        .pedir(
            Method::PUT,
            &format!("/api/v1/anunciantes/{id}/ficha"),
            Some(&rosa),
            Some(json!({ "categorias": ["costura"], "cobertura": "nacional" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    (rosa, id)
}

async fn plataforma(p: &Ph, metodo: Method, ruta: &str, cuerpo: Option<Value>) -> Value {
    let (s, v) = p
        .api
        .pedir(
            metodo,
            &format!("/api/v1/plataforma/publicidad{ruta}"),
            Some(&p.root),
            cuerpo,
        )
        .await;
    assert!(s.is_success(), "{ruta}: {s} {v}");
    v
}

/// Tarifa CPC, política v1, campaña con una oferta con cupón, moderada y activa.
async fn campana_activa(p: &Ph, rosa: &str, id: &str) -> (String, String) {
    let hoy = Utc::now().date_naive();
    plataforma(
        p,
        Method::POST,
        "/tarifas",
        Some(json!({ "modelo": "cpc", "valor": "500", "vigente_desde": hoy })),
    )
    .await;
    let base = format!("/api/v1/anunciantes/{id}/campanas");
    let (s, c) = p
        .api
        .pedir(
            Method::POST,
            &base,
            Some(rosa),
            Some(json!({
                "nombre": "Noviembre", "objetivo": "cupones", "categoria": "costura",
                "inicio": Utc::now() - Duration::hours(1), "fin": Utc::now() + Duration::days(20),
                "modelo": "cpc", "presupuesto_total": "50000",
                "segmentacion": { "estratos": [] }
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let campana = c["id"].as_str().expect("id").to_owned();
    let (s, cr) = p
        .api
        .pedir(
            Method::POST,
            &format!("{base}/{campana}/creativos"),
            Some(rosa),
            Some(json!({
                "formato": "oferta_cupon", "titulo": "10 % en arreglos",
                "texto": "Presente el código en el local",
                "condiciones": "Válido hasta fin de mes", "cupon_codigo": "rosa10"
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{cr}");
    let creativo = cr["id"].as_str().expect("id").to_owned();
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{base}/{campana}/estado"),
            Some(rosa),
            Some(json!({ "accion": "enviar_a_revision" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let pendientes = plataforma(p, Method::GET, "/moderacion/creativos", None).await;
    assert_eq!(pendientes["datos"][0]["id"], creativo.as_str());
    plataforma(
        p,
        Method::POST,
        &format!("/moderacion/creativos/{creativo}"),
        Some(json!({ "decision": "aprobar" })),
    )
    .await;
    let aprobada = plataforma(
        p,
        Method::POST,
        &format!("/moderacion/campanas/{campana}"),
        Some(json!({ "decision": "aprobar" })),
    )
    .await;
    assert_eq!(aprobada["tarifa"], "500.0000");
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{base}/{campana}/estado"),
            Some(rosa),
            Some(json!({ "accion": "activar" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["estado"], "activa");
    (campana, creativo)
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn solo_el_propietario_que_autorizo_ve_la_oferta_y_obtiene_el_cupon(pool: PgPool) {
    let p = ph(pool).await;
    p.habilitar("publicidad").await;
    let (rosa, id) = anunciante(&p).await;
    let (campana, creativo) = campana_activa(&p, &rosa, &id).await;
    plataforma(
        &p,
        Method::POST,
        "/politica",
        Some(json!({ "texto": "Política de publicidad de la plataforma" })),
    )
    .await;
    p.bien_con_dueno().await;
    let dueno = p.api.token("dueno").await;
    let piezas = format!("{}/publicidad/piezas", p.base);

    let (s, v) = p.api.pedir(Method::GET, &piezas, Some(&dueno), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!([]), "sin autorización no hay piezas");

    let (s, v) = p
        .api
        .pedir(
            Method::GET,
            &format!("{}/publicidad/autorizacion", p.base),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["autorizada"], false);
    assert_eq!(v["politica"]["version"], 1);
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{}/publicidad/autorizacion", p.base),
            Some(&dueno),
            Some(json!({ "version": 1 })),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");

    let (_, v) = p.api.pedir(Method::GET, &piezas, Some(&dueno), None).await;
    assert_eq!(v[0]["creativo_id"], creativo.as_str());
    assert_eq!(v[0]["tiene_cupon"], true);
    assert!(
        v[0].get("cupon_codigo").is_none(),
        "el código se pide aparte"
    );

    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{}/publicidad/creativos/{creativo}/cupon", p.base),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["codigo"], "ROSA10");

    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("/api/v1/anunciantes/{id}/cupones/redimir"),
            Some(&rosa),
            Some(json!({ "codigo": "ROSA10" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["usos"], 1);

    let (s, m) = p
        .api
        .pedir(
            Method::GET,
            &format!("/api/v1/anunciantes/{id}/campanas/{campana}/metricas"),
            Some(&rosa),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert_eq!(
        (m["clics"].as_u64(), m["redenciones"].as_u64()),
        (Some(1), Some(1))
    );
    let texto = m.to_string();
    assert!(
        !texto.contains("Ana") && !texto.contains("dueno"),
        "las métricas no tienen datos de residentes: {texto}"
    );

    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{}/publicidad/creativos/{creativo}/reporte", p.base),
            Some(&dueno),
            Some(json!({ "motivo": "No respetaron el descuento" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let cola = plataforma(&p, Method::GET, "/moderacion/reportes", None).await;
    assert_eq!(cola["datos"][0]["reportes"], 1);

    let (s, _) = p
        .api
        .pedir(
            Method::DELETE,
            &format!("{}/publicidad/autorizacion", p.base),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, v) = p.api.pedir(Method::GET, &piezas, Some(&dueno), None).await;
    assert_eq!(v, json!([]), "revocar la quita");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_administracion_de_la_ph_no_autoriza_publicidad_por_el_residente(pool: PgPool) {
    let p = ph(pool).await;
    let (_, persona) = p.bien_con_dueno().await;
    let (s, v) = p
        .pedir(
            Method::POST,
            "/proteccion-datos/politicas",
            Some(json!({ "texto": "Política de la PH" })),
        )
        .await;
    assert!(s.is_success(), "{v}");

    let (s, v) = p
        .pedir(
            Method::POST,
            &format!("/personas/{persona}/autorizaciones-tratamiento"),
            Some(json!({ "finalidades": ["publicidad"], "medio": "escrito" })),
        )
        .await;

    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["codigo"], "validacion");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_modulo_la_ph_no_muestra_publicidad(pool: PgPool) {
    let p = ph(pool).await;
    let (rosa, id) = anunciante(&p).await;
    campana_activa(&p, &rosa, &id).await;
    plataforma(
        &p,
        Method::POST,
        "/politica",
        Some(json!({ "texto": "Política" })),
    )
    .await;
    p.bien_con_dueno().await;
    let dueno = p.api.token("dueno").await;
    p.api
        .pedir(
            Method::POST,
            &format!("{}/publicidad/autorizacion", p.base),
            Some(&dueno),
            Some(json!({ "version": 1 })),
        )
        .await;

    let (s, v) = p
        .api
        .pedir(
            Method::GET,
            &format!("{}/publicidad/piezas", p.base),
            Some(&dueno),
            None,
        )
        .await;

    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!([]));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn otra_cuenta_no_ve_el_portal_de_un_anunciante(pool: PgPool) {
    let p = ph(pool).await;
    let (_, id) = anunciante(&p).await;
    p.api.registrar("curioso").await;
    let curioso = p.api.token("curioso").await;

    let (s, v) = p
        .api
        .pedir(
            Method::GET,
            &format!("/api/v1/anunciantes/{id}/leads"),
            Some(&curioso),
            None,
        )
        .await;

    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    let (s, v) = p
        .api
        .pedir(Method::GET, "/api/v1/anunciantes", Some(&curioso), None)
        .await;
    assert_eq!((s, v), (StatusCode::OK, json!([])));
}
