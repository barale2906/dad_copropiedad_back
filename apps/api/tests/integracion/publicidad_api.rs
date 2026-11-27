//! Pruebas funcionales de la publicidad de la plataforma (plan 08, fases 3 a 6; plan 20): el
//! personal de la plataforma da de alta el comercio y carga su campaña, **otra cuenta** la
//! modera, y solo el propietario que autorizó la publicidad (frente a la plataforma) ve la pieza,
//! la reporta y obtiene el cupón, que el comercio redime con su enlace (sin cuenta). La
//! administración de la PH no puede autorizar por él ni controla la publicidad.

use crate::comun;

use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use comun::{MIGRACIONES, Ph, ph};
use serde_json::{Value, json};
use sqlx::PgPool;

/// Personal comercial de la plataforma: carga la pauta (no la modera).
async fn gestora(p: &Ph) -> String {
    p.api.operador_con_rol("gestora", "comercial").await
}

/// La gestora da de alta a «Modistería Rosa» (queda verificada) y publica su ficha.
async fn comercio(p: &Ph, gestora: &str) -> String {
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/publicidad/anunciantes",
            Some(gestora),
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
            Some(gestora),
            Some(json!({ "categorias": ["costura"], "cobertura": "nacional" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    id
}

/// Petición a `/plataforma/publicidad…` como el superusuario (modera).
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

/// Tarifa CPC, campaña con una oferta con cupón cargada por la gestora, moderada por el
/// superusuario y activa.
async fn campana_activa(p: &Ph, gestora: &str, id: &str) -> (String, String) {
    let hoy = Utc::now().date_naive();
    plataforma(
        p,
        Method::POST,
        "/tarifas",
        Some(json!({ "modelo": "cpc", "valor": "500", "vigente_desde": hoy })),
    )
    .await;
    let base = format!("/api/v1/plataforma/publicidad/anunciantes/{id}/campanas");
    let (s, c) = p
        .api
        .pedir(
            Method::POST,
            &base,
            Some(gestora),
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
            Some(gestora),
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
            Some(gestora),
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
            Some(gestora),
            Some(json!({ "accion": "activar" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["estado"], "activa");
    (campana, creativo)
}

async fn autorizar(p: &Ph, token: &str) {
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{}/publicidad/autorizacion", p.entidad),
            Some(token),
            Some(json!({ "version": 1 })),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn solo_el_propietario_que_autorizo_ve_la_oferta_y_el_comercio_la_redime(pool: PgPool) {
    let p = ph(pool).await;
    let gestora = gestora(&p).await;
    let id = comercio(&p, &gestora).await;
    let (campana, creativo) = campana_activa(&p, &gestora, &id).await;
    plataforma(
        &p,
        Method::POST,
        "/politica",
        Some(json!({ "texto": "Política de publicidad de la plataforma" })),
    )
    .await;
    p.bien_con_dueno().await;
    let dueno = p.api.token("dueno").await;
    let piezas = format!("{}/publicidad/piezas", p.entidad);

    let (s, v) = p.api.pedir(Method::GET, &piezas, Some(&dueno), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!([]), "sin autorización no hay piezas");

    let (s, v) = p
        .api
        .pedir(
            Method::GET,
            &format!("{}/publicidad/autorizacion", p.entidad),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["autorizada"], false);
    assert_eq!(v["politica"]["version"], 1);
    autorizar(&p, &dueno).await;
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
            &format!("{}/publicidad/creativos/{creativo}/cupon", p.entidad),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["codigo"], "ROSA10");

    // El comercio redime con su enlace (sin cuenta); el token se vuelve a consultar igual.
    let enlace = format!("/api/v1/plataforma/publicidad/anunciantes/{id}/enlace-cupones");
    let (s, generado) = p
        .api
        .pedir(Method::POST, &enlace, Some(&gestora), None)
        .await;
    assert_eq!(s, StatusCode::CREATED, "{generado}");
    let (_, vigente) = p
        .api
        .pedir(Method::GET, &enlace, Some(&gestora), None)
        .await;
    assert_eq!(vigente["token"], generado["token"]);
    let token = generado["token"].as_str().expect("token");
    let redimir = "/api/v1/publicidad/cupones/redimir";
    let (s, _) = p
        .api
        .publicar_con_cabecera(redimir, "X-Comercio", "otro", json!({ "codigo": "ROSA10" }))
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, v) = p
        .api
        .publicar_con_cabecera(redimir, "X-Comercio", token, json!({ "codigo": "ROSA10" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["usos"], 1);

    let (s, m) = p
        .api
        .pedir(
            Method::GET,
            &format!("/api/v1/plataforma/publicidad/anunciantes/{id}/campanas/{campana}/metricas"),
            Some(&gestora),
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
            &format!("{}/publicidad/creativos/{creativo}/reporte", p.entidad),
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
            &format!("{}/publicidad/autorizacion", p.entidad),
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
async fn una_entidad_excluida_por_la_plataforma_no_muestra_publicidad(pool: PgPool) {
    let p = ph(pool).await;
    let gestora = gestora(&p).await;
    let id = comercio(&p, &gestora).await;
    campana_activa(&p, &gestora, &id).await;
    plataforma(
        &p,
        Method::POST,
        "/politica",
        Some(json!({ "texto": "Política" })),
    )
    .await;
    p.bien_con_dueno().await;
    let dueno = p.api.token("dueno").await;
    autorizar(&p, &dueno).await;
    let piezas = format!("{}/publicidad/piezas", p.entidad);
    let (_, v) = p.api.pedir(Method::GET, &piezas, Some(&dueno), None).await;
    assert_eq!(
        v.as_array().map(Vec::len),
        Some(1),
        "sin habilitar nada: {v}"
    );

    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/publicidad/exclusiones",
            Some(&gestora),
            Some(json!({ "entidad_id": p.id, "motivo": "El contrato no permite publicidad" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    let (s, v) = p.api.pedir(Method::GET, &piezas, Some(&dueno), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!([]));
    let (_, capacidades) = p
        .api
        .pedir(
            Method::GET,
            &format!("{}/capacidades", p.entidad),
            Some(&p.admin),
            None,
        )
        .await;
    assert!(
        !capacidades["modulos"]
            .as_array()
            .expect("módulos")
            .iter()
            .any(|m| m == "publicidad"),
        "la publicidad no es un módulo"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_permiso_de_gestion_no_se_ven_las_campanas_de_un_comercio(pool: PgPool) {
    let p = ph(pool).await;
    let gestora = gestora(&p).await;
    let id = comercio(&p, &gestora).await;
    let soporte = p.api.operador_con_rol("curioso", "soporte").await;

    let (s, v) = p
        .api
        .pedir(
            Method::GET,
            &format!("/api/v1/plataforma/publicidad/anunciantes/{id}/leads"),
            Some(&soporte),
            None,
        )
        .await;

    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (s, _) = p
        .api
        .pedir(Method::GET, "/api/v1/anunciantes", Some(&gestora), None)
        .await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "el portal del anunciante ya no existe"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_empleado_de_una_empresa_que_autorizo_ve_el_directorio_y_las_piezas(pool: PgPool) {
    let p = ph(pool).await;
    let gestora = gestora(&p).await;
    let id = comercio(&p, &gestora).await;
    campana_activa(&p, &gestora, &id).await;
    plataforma(
        &p,
        Method::POST,
        "/politica",
        Some(json!({ "texto": "Política de publicidad" })),
    )
    .await;
    // Empresa con su administrador nuevo; él crea a su empleada.
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/entidades",
            Some(&p.root),
            Some(json!({ "nombre": "Oficina Central",
                         "administrador": { "nombre": "Gerente", "correo": "gerente@oficina.co" } })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let empresa = format!("/api/v1/entidades/{}", v["id"].as_str().expect("id"));
    p.api.definir_contrasena("gerente@oficina.co").await;
    let gerente = p.api.token("gerente@oficina.co").await;
    let (_, roles) = p
        .api
        .pedir(
            Method::GET,
            &format!("{empresa}/roles"),
            Some(&gerente),
            None,
        )
        .await;
    let asistente = roles
        .as_array()
        .expect("roles")
        .iter()
        .find(|r| r["codigo"] == "asistente_administrativo")
        .and_then(|r| r["id"].as_str())
        .expect("rol")
        .to_owned();
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{empresa}/usuarios"),
            Some(&gerente),
            Some(json!({ "nombre": "Eva", "correo": "eva@oficina.co", "rol_id": asistente })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    p.api.definir_contrasena("eva@oficina.co").await;
    let eva = p.api.token("eva@oficina.co").await;

    let (s, directorio) = p
        .api
        .pedir(
            Method::GET,
            &format!("{empresa}/directorio"),
            Some(&eva),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{directorio}");
    assert_eq!(directorio["datos"][0]["anunciante_id"], id.as_str());
    let piezas = format!("{empresa}/publicidad/piezas");
    let (_, v) = p.api.pedir(Method::GET, &piezas, Some(&eva), None).await;
    assert_eq!(v, json!([]), "sin autorización no hay piezas");
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{empresa}/publicidad/autorizacion"),
            Some(&eva),
            Some(json!({ "version": 1 })),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (s, v) = p.api.pedir(Method::GET, &piezas, Some(&eva), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v.as_array().map(Vec::len), Some(1), "{v}");
}
