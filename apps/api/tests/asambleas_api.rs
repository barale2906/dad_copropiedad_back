//! Pruebas funcionales del plan 06 (asambleas y votaciones) sobre el router real y PostgreSQL
//! (rol `dad_app`): flujo completo por HTTP, permisos del operador, papeleta virtual, hoja de
//! rótulos, acta en PDF, reportes y líneas de WhatsApp de la plataforma.

mod comun;

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use comun::{MIGRACIONES, Ph, ph};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

/// Copropiedad con el módulo de asambleas y `n` bienes de coeficiente `coef`, cada uno con un
/// propietario representante (`Propietario B-000i`, teléfono `573001000000 + i`).
async fn ph_con_bienes(pool: PgPool, n: usize, coef: &str) -> (Ph, Vec<String>, Vec<String>) {
    let ph = ph(pool).await;
    ph.habilitar("asambleas").await;
    let tenant: Uuid = ph.id.parse().expect("uuid");
    let bienes: Vec<Uuid> = (0..n).map(|_| Uuid::now_v7()).collect();
    let personas: Vec<Uuid> = (0..n).map(|_| Uuid::now_v7()).collect();
    let codigos: Vec<String> = (0..n).map(|i| format!("B-{i:04}")).collect();
    let telefonos: Vec<String> = (0..n).map(|i| format!("5730010000{i:02}")).collect();
    let db = &ph.api.duenio;
    sqlx::query(
        "INSERT INTO bienes_privados (id, tenant_id, codigo, coeficiente)
         SELECT b, $1, c, $4::numeric FROM UNNEST($2::uuid[], $3::text[]) AS x (b, c)",
    )
    .bind(tenant)
    .bind(&bienes)
    .bind(&codigos)
    .bind(coef)
    .execute(db)
    .await
    .expect("bienes");
    sqlx::query(
        "INSERT INTO personas (id, tenant_id, nombre, telefono)
         SELECT p, $1, 'Propietario ' || c, t FROM UNNEST($2::uuid[], $3::text[], $4::text[]) AS x (p, c, t)",
    )
    .bind(tenant)
    .bind(&personas)
    .bind(&codigos)
    .bind(&telefonos)
    .execute(db)
    .await
    .expect("personas");
    sqlx::query(
        "INSERT INTO titularidades (id, tenant_id, bien_privado_id, persona_id, tipo, porcentaje,
                                    es_representante, vigente_desde)
         SELECT gen_random_uuid(), $1, b, p, 'propietario', 100, TRUE, now() - interval '1 day'
         FROM UNNEST($2::uuid[], $3::uuid[]) AS x (b, p)",
    )
    .bind(tenant)
    .bind(&bienes)
    .bind(&personas)
    .execute(db)
    .await
    .expect("titularidades");
    sqlx::query(
        "INSERT INTO lineas_whatsapp (id, phone_number_id, numero_visible, nombre)
         VALUES (gen_random_uuid(), 'PNID', '+57 300', 'Línea')",
    )
    .execute(db)
    .await
    .expect("línea");
    (
        ph,
        bienes.iter().map(Uuid::to_string).collect(),
        personas.iter().map(Uuid::to_string).collect(),
    )
}

fn id(v: &Value) -> String {
    v["id"].as_str().expect("id").to_owned()
}

async fn ok(
    ph: &Ph,
    metodo: Method,
    ruta: &str,
    cuerpo: Option<Value>,
    esperado: StatusCode,
) -> Value {
    let (s, v) = ph.pedir(metodo, ruta, cuerpo).await;
    assert_eq!(s, esperado, "{ruta}: {v}");
    v
}

/// Reunión convocada y con el registro abierto.
async fn en_registro(ph: &Ph) -> String {
    let fecha = (chrono::Utc::now() + chrono::Duration::days(20)).to_rfc3339();
    let r = ok(
        ph,
        Method::POST,
        "/asambleas",
        Some(json!({
            "titulo": "Asamblea ordinaria 2027", "tipo": "ordinaria", "modalidad": "presencial",
            "fecha_programada": fecha, "lugar": "Salón comunal",
            "configuracion": { "max_bienes_apoderado": 2 }
        })),
        StatusCode::CREATED,
    )
    .await;
    let r = id(&r);
    ok(
        ph,
        Method::POST,
        &format!("/asambleas/{r}/puntos"),
        Some(json!({ "orden": 1, "titulo": "Quórum" })),
        StatusCode::CREATED,
    )
    .await;
    let c = ok(
        ph,
        Method::POST,
        &format!("/asambleas/{r}/convocatoria"),
        Some(json!({})),
        StatusCode::OK,
    )
    .await;
    assert_eq!(c["bienes_padron"], 3);
    ok(
        ph,
        Method::POST,
        &format!("/asambleas/{r}/transiciones"),
        Some(json!({ "accion": "abrir_registro" })),
        StatusCode::OK,
    )
    .await;
    r
}

async fn votar(
    ph: &Ph,
    token: &str,
    ruta: &str,
    clave: &str,
    cuerpo: Value,
) -> (StatusCode, Value) {
    let req = Request::post(format!("{}{ruta}", ph.base))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", clave)
        .body(Body::from(cuerpo.to_string()))
        .expect("petición");
    let res = ph.api.router.clone().oneshot(req).await.expect("router");
    let s = res.status();
    let b = to_bytes(res.into_body(), usize::MAX).await.expect("cuerpo");
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_modulo_no_hay_asambleas(pool: PgPool) {
    let ph = ph(pool).await;
    let (s, v) = ph.pedir(Method::GET, "/asambleas", None).await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::FORBIDDEN, &json!("modulo_no_habilitado"))
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn flujo_completo_de_una_asamblea_por_la_api(pool: PgPool) {
    let (ph, bienes, personas) = ph_con_bienes(pool, 3, "0.3").await;
    let r = en_registro(&ph).await;

    // La mesa busca el bien y registra al propietario con un poder y un rótulo.
    let m = ok(
        &ph,
        Method::GET,
        &format!("/asambleas/{r}/mesa?texto=b-0000"),
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(m[0]["representante_persona_id"], json!(personas[0]));
    let reg = ok(
        &ph,
        Method::POST,
        &format!("/asambleas/{r}/asistentes"),
        Some(json!({
            "persona_id": personas[0], "bienes_propios": [bienes[0]],
            "poderes": [{ "bien_privado_id": bienes[1] }], "rotulo": "r-0001"
        })),
        StatusCode::CREATED,
    )
    .await;
    assert_eq!(reg["asistente"]["presente"], true);
    assert_eq!(reg["asistente"]["rotulo"], "R-0001");
    assert_eq!(reg["asistente"]["bienes"].as_array().map(Vec::len), Some(2));
    let token = reg["token_papeleta"].as_str().expect("token").to_owned();
    // Un bien en un solo asistente.
    let (s, v) = ph
        .pedir(
            Method::POST,
            &format!("/asambleas/{r}/asistentes"),
            Some(json!({ "persona_id": personas[2], "bienes_propios": [bienes[2]], "poderes": [{ "bien_privado_id": bienes[1] }] })),
        )
        .await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::CONFLICT, &json!("conflicto")),
        "{v}"
    );

    let q = ok(
        &ph,
        Method::GET,
        &format!("/asambleas/{r}/quorum"),
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(q["unidades_presentes"], 2);
    ok(
        &ph,
        Method::POST,
        &format!("/asambleas/{r}/transiciones"),
        Some(json!({ "accion": "instalar" })),
        StatusCode::OK,
    )
    .await;

    // Pregunta sí/no abierta; el puesto de escaneo registra el voto del rótulo.
    let p = ok(
        &ph,
        Method::POST,
        &format!("/asambleas/{r}/preguntas"),
        Some(json!({ "enunciado": "¿Aprueba el presupuesto?", "tipo": "si_no", "con_abstencion": true })),
        StatusCode::CREATED,
    )
    .await;
    let pid = id(&p);
    let si = p["opciones"][0]["id"].clone();
    ok(
        &ph,
        Method::PUT,
        &format!("/asambleas/{r}/preguntas/{pid}/estado"),
        Some(json!({ "accion": "abrir" })),
        StatusCode::OK,
    )
    .await;
    let ruta_voto = format!("/asambleas/{r}/preguntas/{pid}/votos");
    let cuerpo = json!({ "rotulo": "R-0001", "opciones": [si], "canal": "rotulo" });
    let (s, v) = votar(&ph, &ph.admin, &ruta_voto, "lectura-1", cuerpo.clone()).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["registrado"], true);
    assert_eq!(v["bienes"].as_array().map(Vec::len), Some(2));
    let (_, reintento) = votar(&ph, &ph.admin, &ruta_voto, "lectura-1", cuerpo.clone()).await;
    assert_eq!(reintento["registrado"], false);
    let (s, v) = votar(&ph, &ph.admin, &ruta_voto, "lectura-2", cuerpo).await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::CONFLICT, &json!("ya_voto"))
    );

    let t = ok(
        &ph,
        Method::GET,
        &format!("/asambleas/{r}/tablero"),
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(t["hay_quorum"], true);
    assert_eq!(
        t["preguntas_abiertas"][0]["escrutinio"]["votantes"]["unidades"],
        2
    );

    // Papeleta virtual: el token es la credencial.
    let papeleta = |token: &str| {
        Request::get(format!("{}/papeleta", ph.base))
            .header("x-papeleta", token)
            .body(Body::empty())
            .expect("petición")
    };
    let res = ph
        .api
        .router
        .clone()
        .oneshot(papeleta(&token))
        .await
        .expect("router");
    assert_eq!(res.status(), StatusCode::OK);
    let b = to_bytes(res.into_body(), usize::MAX).await.expect("cuerpo");
    let v: Value = serde_json::from_slice(&b).expect("json");
    assert_eq!(v["preguntas_abiertas"][0]["ya_voto"], true);
    let res = ph
        .api
        .router
        .clone()
        .oneshot(papeleta("falso"))
        .await
        .expect("router");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Cierre, detalle nominal, cierre de la reunión y acta.
    let c = ok(
        &ph,
        Method::PUT,
        &format!("/asambleas/{r}/preguntas/{pid}/estado"),
        Some(json!({ "accion": "cerrar" })),
        StatusCode::OK,
    )
    .await;
    assert_eq!(c["estado"], "cerrada");
    let res = ok(
        &ph,
        Method::GET,
        &format!("/asambleas/{r}/preguntas/{pid}/resultados"),
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(res["escrutinio"]["decision"]["decision"], "aprobada");
    let d = ok(
        &ph,
        Method::GET,
        &format!("/asambleas/{r}/preguntas/{pid}/detalle"),
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(d.as_array().map(Vec::len), Some(2));
    ok(
        &ph,
        Method::POST,
        &format!("/asambleas/{r}/transiciones"),
        Some(json!({ "accion": "cerrar" })),
        StatusCode::OK,
    )
    .await;
    let acta = ok(
        &ph,
        Method::POST,
        &format!("/asambleas/{r}/acta"),
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(acta["acta_estado"], "generada");
    assert!(acta["acta_archivo_id"].is_string());
    let (s, v) = ph
        .pedir(
            Method::PUT,
            &format!("/asambleas/{r}/acta/estado"),
            Some(json!({ "paso": "aprobar" })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "sin el escaneo firmado: {v}");

    // Reportes.
    let (s, tipo, csv) = ph
        .api
        .pedir_texto(
            &format!("{}/asambleas/{r}/reportes/asistencia?formato=csv", ph.base),
            &ph.admin,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(tipo.starts_with("text/csv"));
    assert!(csv.contains("B-0001") && csv.contains("apoderado"));
    let costo = ok(
        &ph,
        Method::GET,
        &format!("/asambleas/{r}/reportes/costo-whatsapp"),
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(costo["moneda"], "COP");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn solo_el_operador_registra_votos_y_el_residente_no(pool: PgPool) {
    let (ph, bienes, personas) = ph_con_bienes(pool, 3, "0.3").await;
    let r = en_registro(&ph).await;
    let reg = ok(
        &ph,
        Method::POST,
        &format!("/asambleas/{r}/asistentes"),
        Some(json!({ "persona_id": personas[0], "bienes_propios": [bienes[0]], "poderes": [{ "bien_privado_id": bienes[1] }] })),
        StatusCode::CREATED,
    )
    .await;
    let asistente = reg["asistente"]["id"].clone();
    ok(
        &ph,
        Method::POST,
        &format!("/asambleas/{r}/transiciones"),
        Some(json!({ "accion": "instalar" })),
        StatusCode::OK,
    )
    .await;
    let p = ok(
        &ph,
        Method::POST,
        &format!("/asambleas/{r}/preguntas"),
        Some(json!({ "enunciado": "¿Aprueba?", "tipo": "si_no" })),
        StatusCode::CREATED,
    )
    .await;
    let pid = id(&p);
    ok(
        &ph,
        Method::PUT,
        &format!("/asambleas/{r}/preguntas/{pid}/estado"),
        Some(json!({ "accion": "abrir" })),
        StatusCode::OK,
    )
    .await;

    let asignar = |usuario: &'static str, rol: &'static str| {
        let ph = &ph;
        async move {
            let cuenta = ph.api.registrar(usuario).await;
            let (_, roles) = ph.pedir(Method::GET, "/roles", None).await;
            let rol = roles
                .as_array()
                .and_then(|v| v.iter().find(|x| x["codigo"] == rol))
                .map(id)
                .expect("rol");
            ok(
                ph,
                Method::POST,
                "/asignaciones",
                Some(json!({ "cuenta_id": cuenta, "rol_id": rol })),
                StatusCode::CREATED,
            )
            .await;
            ph.api.token(usuario).await
        }
    };
    let residente = asignar("residente1", "residente").await;
    let operador = asignar("operador1", "operador_asamblea").await;
    let cuerpo = json!({ "asistente_id": asistente, "opciones": [p["opciones"][1]["id"]], "canal": "presencial" });
    let ruta = format!("/asambleas/{r}/preguntas/{pid}/votos");
    let (s, v) = votar(&ph, &residente, &ruta, "k", cuerpo.clone()).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (s, v) = votar(&ph, &operador, &ruta, "k", cuerpo).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    // Sin `Idempotency-Key` la petición está mal formada.
    let (s, _) = ph
        .pedir(
            Method::POST,
            &ruta,
            Some(json!({ "asistente_id": asistente, "opciones": [], "canal": "presencial" })),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn hoja_de_rotulos_en_pdf(pool: PgPool) {
    let (ph, _, _) = ph_con_bienes(pool, 1, "1").await;
    let req = Request::post(format!("{}/asambleas/rotulos", ph.base))
        .header(header::AUTHORIZATION, format!("Bearer {}", ph.admin))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "prefijo": "R-", "desde": 1, "cantidad": 12 }).to_string(),
        ))
        .expect("petición");
    let res = ph.api.router.clone().oneshot(req).await.expect("router");
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/pdf");
    let b = to_bytes(res.into_body(), usize::MAX).await.expect("cuerpo");
    assert!(b.starts_with(b"%PDF-"));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_tablero_en_vivo_exige_sesion(pool: PgPool) {
    let (ph, _, _) = ph_con_bienes(pool, 1, "1").await;
    let r = Uuid::now_v7();
    let req = Request::get(format!("{}/asambleas/{r}/en-vivo", ph.base))
        .header(header::SEC_WEBSOCKET_PROTOCOL, "dad.v1, token-falso")
        .body(Body::empty())
        .expect("petición");
    let res = ph.api.router.clone().oneshot(req).await.expect("router");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn lineas_de_whatsapp_de_la_plataforma(pool: PgPool) {
    let ph = ph(pool).await;
    let (s, v) = ph
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/whatsapp/lineas",
            Some(&ph.root),
            Some(json!({ "phone_number_id": "PNID-7", "numero_visible": "+57 300 7", "nombre": "Línea 7" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, v) = ph
        .api
        .pedir(
            Method::GET,
            "/api/v1/plataforma/whatsapp/lineas",
            Some(&ph.root),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v[0]["estado"], "activa");
    let (s, _) = ph
        .api
        .pedir(
            Method::GET,
            "/api/v1/plataforma/whatsapp/lineas",
            Some(&ph.admin),
            None,
        )
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "el administrador de una PH no gestiona líneas"
    );
}
