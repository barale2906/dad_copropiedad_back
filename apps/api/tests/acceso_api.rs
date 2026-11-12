//! Pruebas funcionales del control de acceso (plan 05) sobre el router completo con PostgreSQL
//! real: portería (visitas, QR, correspondencia), residentes, puentes con token de máquina y
//! dispositivos push de la app.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{MIGRACIONES, Ph, ph};
use serde_json::{Value, json};
use sqlx::PgPool;

/// PNG mínimo en Base64 (firma o foto).
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

async fn con_rol(ph: &Ph, usuario: &str, codigo_rol: &str) -> String {
    let cuenta = ph.api.registrar(usuario).await;
    let (_, roles) = ph.pedir(Method::GET, "/roles", None).await;
    let rol = roles
        .as_array()
        .and_then(|r| r.iter().find(|r| r["codigo"] == codigo_rol))
        .and_then(|r| r["id"].as_str())
        .expect("rol plantilla")
        .to_owned();
    let (s, v) = ph
        .pedir(
            Method::POST,
            "/asignaciones",
            Some(json!({ "cuenta_id": cuenta, "rol_id": rol })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    ph.api.token(usuario).await
}

async fn como(
    ph: &Ph,
    token: &str,
    metodo: Method,
    ruta: &str,
    cuerpo: Option<Value>,
) -> (StatusCode, Value) {
    ph.api
        .pedir(metodo, &format!("{}{ruta}", ph.base), Some(token), cuerpo)
        .await
}

/// PH con el módulo, un bien con dueño (cuenta `dueno`, no residente) y un vigilante.
struct Escenario {
    ph: Ph,
    bien: String,
    persona: String,
    vigilante: String,
    dueno: String,
}

async fn escenario(pool: PgPool) -> Escenario {
    let ph = ph(pool).await;
    ph.habilitar("control_acceso").await;
    let (bien, persona) = ph.bien_con_dueno().await;
    let vigilante = con_rol(&ph, "porteria1", "vigilante").await;
    let dueno = ph.api.token("dueno").await;
    Escenario {
        ph,
        bien,
        persona,
        vigilante,
        dueno,
    }
}

impl Escenario {
    async fn anunciar(&self) -> Value {
        let (s, v) = como(
            &self.ph,
            &self.vigilante,
            Method::POST,
            "/visitas",
            Some(json!({ "bien_privado_id": self.bien, "tipo": "domicilio", "visitante_nombre": "Mensajero" })),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        v
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_modulo_no_hay_porteria(pool: PgPool) {
    let ph = ph(pool).await;

    let (s, v) = ph.pedir(Method::GET, "/visitas", None).await;

    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::FORBIDDEN, &json!("modulo_no_habilitado"))
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_porteria_anuncia_el_dueno_decide_una_vez_y_la_visita_entra_y_sale(pool: PgPool) {
    let e = escenario(pool).await;
    let (s, _) =
        e.ph.api
            .pedir(
                Method::POST,
                "/api/v1/yo/dispositivos-push",
                Some(&e.dueno),
                Some(json!({ "plataforma": "android", "token": "fcm-token-del-dueno-0001" })),
            )
            .await;
    assert_eq!(s, StatusCode::CREATED);
    let v = e.anunciar().await;
    let id = v["id"].as_str().expect("id");
    assert_eq!(v["estado"], "anunciada");
    assert_eq!(v["avisos_enviados"], 1);
    let canales: Vec<String> = sqlx::query_scalar(
        "SELECT canal FROM outbox WHERE concepto = 'control_acceso' AND tenant_id = $1::uuid",
    )
    .bind(&e.ph.id)
    .fetch_all(&e.ph.api.duenio)
    .await
    .expect("outbox");
    assert_eq!(canales, vec!["push"], "con app, el aviso sale por push");

    let (s, r) = como(
        &e.ph,
        &e.dueno,
        Method::POST,
        &format!("/visitas/{id}/respuesta"),
        Some(json!({ "autorizar": true })),
    )
    .await;
    assert_eq!(
        (s, &r["decidida_ahora"]),
        (StatusCode::OK, &json!(true)),
        "{r}"
    );
    let (_, r) = como(
        &e.ph,
        &e.dueno,
        Method::POST,
        &format!("/visitas/{id}/respuesta"),
        Some(json!({ "autorizar": false })),
    )
    .await;
    assert_eq!(r["decidida_ahora"], false, "la primera decisión gana");
    assert_eq!(r["visita"]["estado"], "autorizada");

    let (s, x) = como(
        &e.ph,
        &e.vigilante,
        Method::POST,
        &format!("/visitas/{id}/ingreso"),
        Some(json!({})),
    )
    .await;
    assert_eq!(
        (s, &x["estado"]),
        (StatusCode::OK, &json!("ingresada")),
        "{x}"
    );
    let (_, dentro) = como(
        &e.ph,
        &e.vigilante,
        Method::GET,
        "/visitas?dentro=true",
        None,
    )
    .await;
    assert_eq!(dentro["datos"].as_array().map(Vec::len), Some(1));
    let (s, x) = como(
        &e.ph,
        &e.vigilante,
        Method::POST,
        &format!("/visitas/{id}/salida"),
        Some(json!({})),
    )
    .await;
    assert_eq!((s, &x["estado"]), (StatusCode::OK, &json!("finalizada")));
    let (s, d) = como(
        &e.ph,
        &e.dueno,
        Method::GET,
        &format!("/visitas/{id}"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert!(d["eventos"].as_array().is_some_and(|ev| ev.len() >= 4));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn alguien_ajeno_al_bien_no_ve_ni_decide(pool: PgPool) {
    let e = escenario(pool).await;
    let v = e.anunciar().await;
    let ajeno = con_rol(&e.ph, "vecino", "residente").await;

    let (s, _) = como(
        &e.ph,
        &ajeno,
        Method::GET,
        &format!("/visitas?bien_privado_id={}", e.bien),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = como(
        &e.ph,
        &ajeno,
        Method::POST,
        &format!("/visitas/{}/respuesta", v["id"].as_str().expect("id")),
        Some(json!({ "autorizar": true })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = como(
        &e.ph,
        &e.vigilante,
        Method::PUT,
        "/acceso/configuracion",
        Some(json!({
            "retencion_soportes_dias": 365, "dias_avisos_correspondencia": 7,
            "retencion_eventos_dias": 730, "retencion_fotos_placas_dias": 30
        })),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "la portería no configura");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_preautorizacion_entrega_un_qr_de_usos_contados(pool: PgPool) {
    let e = escenario(pool).await;
    let ahora = chrono::Utc::now();
    let (s, v) = como(&e.ph, &e.dueno, Method::POST, "/visitas/preautorizaciones", Some(json!({
        "bien_privado_id": e.bien, "tipo": "visita", "visitante_nombre": "Tía Rosa",
        "desde": ahora - chrono::TimeDelta::minutes(1), "hasta": ahora + chrono::TimeDelta::hours(3), "usos": 1
    }))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let qr = v["qr"].as_str().expect("qr").to_owned();
    assert!(qr.starts_with("DQ1."));

    let (s, x) = como(
        &e.ph,
        &e.vigilante,
        Method::POST,
        "/visitas/ingreso-qr",
        Some(json!({ "qr": qr })),
    )
    .await;
    assert_eq!(
        (s, &x["estado"]),
        (StatusCode::OK, &json!("ingresada")),
        "{x}"
    );
    let (s, _) = como(
        &e.ph,
        &e.vigilante,
        Method::POST,
        "/visitas/ingreso-qr",
        Some(json!({ "qr": qr })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "un solo uso");
    let (s, _) = como(
        &e.ph,
        &e.vigilante,
        Method::POST,
        "/visitas/ingreso-qr",
        Some(json!({ "qr": "DQ1.00.00" })),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_correspondencia_se_entrega_con_firma_a_alguien_del_bien(pool: PgPool) {
    let e = escenario(pool).await;
    let (s, c) = como(&e.ph, &e.vigilante, Method::POST, "/correspondencia", Some(json!({
        "bien_privado_id": e.bien, "tipo": "paquete", "remitente": "Servientrega", "foto_base64": PNG
    }))).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let id = c["id"].as_str().expect("id").to_owned();
    assert!(c["foto_archivo_id"].is_string());

    let (s, lista) = como(
        &e.ph,
        &e.vigilante,
        Method::GET,
        &format!(
            "/correspondencia/personas-para-entrega?bien_privado_id={}",
            e.bien
        ),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{lista}");
    assert_eq!(lista[0]["persona_id"], json!(e.persona));
    let (s, x) = como(
        &e.ph,
        &e.vigilante,
        Method::POST,
        &format!("/correspondencia/{id}/entrega"),
        Some(json!({ "persona_id": e.persona })),
    )
    .await;
    assert_eq!(
        (s, &x["error"]["codigo"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("validacion")),
        "sin firma ni foto"
    );
    let (s, x) = como(
        &e.ph,
        &e.vigilante,
        Method::POST,
        &format!("/correspondencia/{id}/entrega"),
        Some(json!({ "persona_id": e.persona, "firma_base64": PNG })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{x}");
    assert_eq!(x["correspondencia"]["estado"], "entregada");
    assert!(x["entrega"]["firma_archivo_id"].is_string());
    let (s, _) = como(
        &e.ph,
        &e.vigilante,
        Method::POST,
        &format!("/correspondencia/{id}/entrega"),
        Some(json!({ "persona_id": e.persona, "firma_base64": PNG })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "no se entrega dos veces");

    let (s, mia) = como(
        &e.ph,
        &e.dueno,
        Method::GET,
        &format!("/correspondencia?bien_privado_id={}", e.bien),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{mia}");
    assert_eq!(mia["datos"][0]["estado"], "entregada");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_puente_sincroniza_y_reporta_con_su_token(pool: PgPool) {
    let e = escenario(pool).await;
    assert_eq!(e.ph.estado("activa", &e.ph.root).await.0, StatusCode::OK);
    let (s, punto) =
        e.ph.pedir(
            Method::POST,
            "/puntos-acceso",
            Some(json!({ "nombre": "Portería", "tipo": "peatonal", "sentido": "ambos" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{punto}");
    let (s, p) =
        e.ph.pedir(
            Method::POST,
            "/puentes",
            Some(json!({ "nombre": "Mini PC" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let token = p["token"].as_str().expect("token").to_owned();
    let (s, cred) =
        e.ph.pedir(
            Method::POST,
            "/credenciales",
            Some(json!({
                "titular": { "persona_id": e.persona }, "tipo": "pin", "valor": "4321"
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{cred}");

    let (s, _) =
        e.ph.api
            .pedir(Method::GET, "/api/v1/puentes/sincronizacion", None, None)
            .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, sync) =
        e.ph.api
            .pedir(
                Method::GET,
                "/api/v1/puentes/sincronizacion?cursor=0",
                Some(&token),
                None,
            )
            .await;
    assert_eq!(s, StatusCode::OK, "{sync}");
    assert_eq!(sync["copropiedad_id"], json!(e.ph.id));
    assert_eq!(sync["clave_publica_qr"].as_str().map(str::len), Some(64));
    let c = &sync["credenciales"][0];
    assert_eq!(
        (c["tipo"].as_str(), c["activa"].as_bool()),
        (Some("pin"), Some(true))
    );
    assert!(!c["huella"].as_str().unwrap_or_default().contains("4321"));

    let reporte = json!({ "eventos": [{
        "id_local": "ev-1", "punto_acceso_id": punto["id"], "sentido": "entrada",
        "resultado": "permitido", "ocurrido_en": chrono::Utc::now()
    }]});
    let (s, r) =
        e.ph.api
            .pedir(
                Method::POST,
                "/api/v1/puentes/eventos",
                Some(&token),
                Some(reporte.clone()),
            )
            .await;
    assert_eq!((s, &r["nuevos"]), (StatusCode::OK, &json!(1)), "{r}");
    let (_, r) =
        e.ph.api
            .pedir(
                Method::POST,
                "/api/v1/puentes/eventos",
                Some(&token),
                Some(reporte),
            )
            .await;
    assert_eq!((&r["nuevos"], &r["repetidos"]), (&json!(0), &json!(1)));
    let (s, _) =
        e.ph.api
            .pedir(
                Method::POST,
                "/api/v1/puentes/latido",
                Some(&token),
                Some(json!({ "version": "0.1.0" })),
            )
            .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, bitacora) = e.ph.pedir(Method::GET, "/eventos-acceso", None).await;
    assert_eq!(bitacora["datos"].as_array().map(Vec::len), Some(1));

    let (s, v) = como(
        &e.ph,
        &e.vigilante,
        Method::POST,
        "/credenciales/verificacion",
        Some(json!({
            "texto": "4321", "tipo": "pin", "punto_acceso_id": punto["id"]
        })),
    )
    .await;
    assert_eq!(
        (s, &v["resultado"]),
        (StatusCode::OK, &json!("permitido")),
        "{v}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_app_registra_y_retira_su_dispositivo_push(pool: PgPool) {
    let e = escenario(pool).await;
    let (s, d) = e
        .ph
        .api
        .pedir(
            Method::POST,
            "/api/v1/yo/dispositivos-push",
            Some(&e.dueno),
            Some(json!({
                "plataforma": "android", "token": "fcm-token-de-prueba-123456", "nombre": "Teléfono"
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    assert!(d.get("token").is_none(), "el token no se devuelve");
    let (_, lista) =
        e.ph.api
            .pedir(
                Method::GET,
                "/api/v1/yo/dispositivos-push",
                Some(&e.dueno),
                None,
            )
            .await;
    assert_eq!(lista.as_array().map(Vec::len), Some(1));
    let ruta = format!(
        "/api/v1/yo/dispositivos-push/{}",
        d["id"].as_str().expect("id")
    );
    assert_eq!(
        e.ph.api
            .pedir(Method::DELETE, &ruta, Some(&e.dueno), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        e.ph.api
            .pedir(Method::DELETE, &ruta, Some(&e.dueno), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (s, _) =
        e.ph.api
            .pedir(
                Method::POST,
                "/api/v1/yo/dispositivos-push",
                Some(&e.dueno),
                Some(json!({
                    "plataforma": "ios", "token": "corto"
                })),
            )
            .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_configuracion_valida_sus_plazos(pool: PgPool) {
    let e = escenario(pool).await;

    let (s, _) =
        e.ph.pedir(
            Method::PUT,
            "/acceso/configuracion",
            Some(json!({
                "retencion_soportes_dias": 5, "dias_avisos_correspondencia": 7,
                "retencion_eventos_dias": 730, "retencion_fotos_placas_dias": 30
            })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, c) =
        e.ph.pedir(
            Method::PUT,
            "/acceso/configuracion",
            Some(json!({
                "retencion_soportes_dias": 180, "dias_avisos_correspondencia": 5,
                "silencio_inicio": "22:00:00", "silencio_fin": "06:00:00",
                "retencion_eventos_dias": 365, "retencion_fotos_placas_dias": 15
            })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{c}");
    let (_, c) = como(
        &e.ph,
        &e.vigilante,
        Method::GET,
        "/acceso/configuracion",
        None,
    )
    .await;
    assert_eq!(
        (
            c["retencion_soportes_dias"].as_u64(),
            c["silencio_inicio"].as_str()
        ),
        (Some(180), Some("22:00:00"))
    );
}

// ---------------------------------------------------------------------------------------------
// Biometría, credenciales y autorizaciones manuales (contra PostgreSQL)
// ---------------------------------------------------------------------------------------------

async fn emitir_biometrica(e: &Escenario) -> (StatusCode, Value) {
    e.ph.pedir(
        Method::POST,
        "/credenciales",
        Some(json!({
            "titular": { "persona_id": e.persona }, "tipo": "biometria_ref", "valor": "plantilla-1"
        })),
    )
    .await
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_biometria_exige_consentimiento_vigente_y_revocarlo_revoca_la_credencial(pool: PgPool) {
    let e = escenario(pool).await;
    let ruta_consentimiento = format!("/personas/{}/consentimiento-biometrico", e.persona);

    // Sin consentimiento se rechaza por eso, no por otro dato faltante.
    let por_consentimiento = |v: &Value| {
        v["error"]["mensaje"]
            .as_str()
            .is_some_and(|m| m.contains("consentimiento"))
    };
    let (s, v) = emitir_biometrica(&e).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(por_consentimiento(&v), "{v}");

    let (s, v) =
        e.ph.pedir(
            Method::POST,
            &ruta_consentimiento,
            Some(json!({ "version_texto": "v1" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, v) =
        e.ph.pedir(
            Method::POST,
            &ruta_consentimiento,
            Some(json!({ "version_texto": "v1" })),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "un solo consentimiento vigente: {v}"
    );
    let (s, credencial) = emitir_biometrica(&e).await;
    assert_eq!(s, StatusCode::CREATED, "{credencial}");

    let (s, v) = e.ph.pedir(Method::DELETE, &ruta_consentimiento, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, credenciales) =
        e.ph.pedir(
            Method::GET,
            &format!("/credenciales?persona_id={}", e.persona),
            None,
        )
        .await;
    let biometrica = credenciales
        .as_array()
        .and_then(|c| c.iter().find(|c| c["id"] == credencial["credencial"]["id"]))
        .expect("la credencial sigue en el historial");
    assert_eq!(biometrica["estado"], "revocada");
    let (s, v) = emitir_biometrica(&e).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(por_consentimiento(&v), "{v}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_vigilante_no_enrola_biometria(pool: PgPool) {
    let e = escenario(pool).await;

    let (s, v) = como(
        &e.ph,
        &e.vigilante,
        Method::POST,
        &format!("/personas/{}/consentimiento-biometrico", e.persona),
        Some(json!({ "version_texto": "v1" })),
    )
    .await;

    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::FORBIDDEN, &json!("sin_permiso"))
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_tarjeta_suspendida_se_reactiva_y_una_revocada_ya_no(pool: PgPool) {
    let e = escenario(pool).await;
    let (s, tarjeta) = e.ph
        .pedir(
            Method::POST,
            "/credenciales",
            Some(json!({ "titular": { "persona_id": e.persona }, "tipo": "tarjeta_rfid", "valor": "04A1B2C3" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tarjeta}");
    let estado = |nuevo: &'static str| {
        let (ph, id) = (
            &e.ph,
            tarjeta["credencial"]["id"].as_str().expect("id").to_owned(),
        );
        async move {
            ph.pedir(
                Method::POST,
                &format!("/credenciales/{id}/estado"),
                Some(json!({ "estado": nuevo })),
            )
            .await
        }
    };

    assert_eq!(estado("suspendida").await.1["estado"], "suspendida");
    assert_eq!(estado("activa").await.1["estado"], "activa");
    assert_eq!(estado("revocada").await.1["estado"], "revocada");
    let (s, v) = estado("activa").await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::CONFLICT, &json!("conflicto_estado")),
        "{v}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_autorizacion_manual_se_lista_y_al_revocarla_queda_con_fecha(pool: PgPool) {
    let e = escenario(pool).await;
    let ruta = format!("/autorizaciones-acceso?persona_id={}", e.persona);
    let (s, a) =
        e.ph.pedir(
            Method::POST,
            "/autorizaciones-acceso",
            Some(json!({
                "titular": { "persona_id": e.persona },
                "vigente_desde": chrono::Utc::now().to_rfc3339(),
                "motivo": "contratista de la remodelación",
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{a}");
    let id = a["id"].as_str().expect("id");

    let (_, lista) = e.ph.pedir(Method::GET, &ruta, None).await;
    assert!(
        lista.as_array().is_some_and(|l| l
            .iter()
            .any(|x| x["id"] == a["id"] && x["revocada_en"].is_null())),
        "{lista}"
    );
    let (s, _) =
        e.ph.pedir(
            Method::POST,
            &format!("/autorizaciones-acceso/{id}/revocacion"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (_, lista) = e.ph.pedir(Method::GET, &ruta, None).await;
    let revocada = lista
        .as_array()
        .and_then(|l| l.iter().find(|x| x["id"] == a["id"]));
    assert!(
        revocada.is_none_or(|x| x["revocada_en"].is_string()),
        "{lista}"
    );
}
