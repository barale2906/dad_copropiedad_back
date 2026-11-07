//! Pruebas funcionales del plan 04 (bienes comunes, reservas, parqueaderos y sorteos) sobre el
//! router real y PostgreSQL (rol `dad_app`).

mod comun;

use axum::http::{Method, StatusCode};
use chrono::{Duration, TimeZone, Utc};
use comun::{MIGRACIONES, Ph, ph};
use serde_json::{Value, json};
use sqlx::PgPool;

/// Inicio y fin (RFC 3339) de una franja local de Bogotá dentro de `dias` días.
fn franja(dias: i64, h1: u32, h2: u32) -> (String, String) {
    let tz = chrono_tz_bogota();
    let fecha = (Utc::now() + Duration::days(dias))
        .with_timezone(&tz)
        .date_naive();
    let a = |h: u32| {
        tz.from_local_datetime(&fecha.and_hms_opt(h, 0, 0).expect("hora"))
            .single()
            .expect("local")
            .with_timezone(&Utc)
            .to_rfc3339()
    };
    (a(h1), a(h2))
}

fn chrono_tz_bogota() -> chrono::FixedOffset {
    chrono::FixedOffset::west_opt(5 * 3600).expect("UTC-5")
}

fn horario() -> Value {
    json!({ "franjas": (1..=7).map(|d| json!({
        "dia": d, "inicio": "06:00:00", "fin": "22:00:00", "aplica_festivos": true
    })).collect::<Vec<_>>() })
}

fn politica(modo: &str) -> Value {
    json!({
        "modo": modo, "duracion_minima_min": 60, "duracion_maxima_min": 240,
        "granularidad_min": 60, "anticipacion_minima_horas": 0, "anticipacion_maxima_dias": 60,
        "alistamiento_min": 60, "quien": "ambos", "tenedor_requiere_autorizacion": false,
        "tarifa": "150000", "cobro_por": "reserva", "deposito": "0",
        "cancelacion_sin_penalidad_horas": 48, "penalidad_porcentaje": "50",
        "max_invitados": 50, "metodo_entrega": "entrega_con_inventario"
    })
}

async fn salon(ph: &Ph) -> String {
    let (s, v) = ph
        .pedir(
            Method::POST,
            "/recursos",
            Some(json!({
                "tipo": "bien_comun", "nombre": "Salón social", "politica": politica("franja_con_aprobacion"),
                "horario": horario(),
                "bien_comun": { "categoria": "salon_social", "es_esencial": false }
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v["id"].as_str().expect("id").to_owned()
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_modulo_no_hay_bienes_comunes(pool: PgPool) {
    let ph = ph(pool).await;
    let (s, v) = ph.pedir(Method::GET, "/recursos", None).await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::FORBIDDEN, &json!("modulo_no_habilitado"))
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_propietario_reserva_el_salon_y_la_administracion_aprueba(pool: PgPool) {
    let ph = ph(pool).await;
    ph.habilitar("bienes_comunes").await;
    let (bien, _) = ph.bien_con_dueno().await;
    let salon = salon(&ph).await;
    let dueno = ph.api.token("dueno").await;
    let (inicio, fin) = franja(5, 14, 18);
    let peticion = json!({
        "recurso_id": salon, "bien_privado_id": bien, "inicio": inicio, "fin": fin,
        "invitados": 20, "idempotency_key": "dueno-1"
    });

    let (s, r) = ph
        .api
        .pedir(
            Method::POST,
            &format!("{}/reservas", ph.base),
            Some(&dueno),
            Some(peticion.clone()),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(
        (r["estado"].as_str(), r["valor"].as_str()),
        (Some("solicitada"), Some("150000.00"))
    );
    // Reintento con la misma clave: la misma reserva.
    let (_, otra) = ph
        .api
        .pedir(
            Method::POST,
            &format!("{}/reservas", ph.base),
            Some(&dueno),
            Some(peticion),
        )
        .await;
    assert_eq!(otra["id"], r["id"]);

    // El alistamiento de 1 h deja libre desde las 19:00.
    let dia = &inicio[..10];
    let (_, libres) = ph
        .pedir(
            Method::GET,
            &format!("/recursos/{salon}/disponibilidad?desde={dia}&hasta={dia}"),
            None,
        )
        .await;
    assert_eq!(libres.as_array().map(Vec::len), Some(2), "{libres}");

    let id = r["id"].as_str().expect("id");
    let (s, v) = ph
        .pedir(Method::POST, &format!("/reservas/{id}/aprobar"), None)
        .await;
    assert_eq!((s, &v["estado"]), (StatusCode::OK, &json!("aprobada")));
    let avisos: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE canal = 'correo' AND destino = 'dueno@correo.co'
           AND payload->>'asunto' = 'Reserva aprobada'",
    )
    .fetch_one(&ph.api.duenio)
    .await
    .expect("outbox");
    assert_eq!(avisos, 1, "aviso de aprobación en el outbox");

    // Entrega y recibo con daños: queda con novedad y un cargo pendiente.
    let (s, _) = ph
        .pedir(
            Method::POST,
            &format!("/reservas/{id}/entrega"),
            Some(json!({
                "checklist": [{ "elemento": "Sillas", "cantidad": 40, "estado": "bueno" }]
            })),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = ph
        .pedir(
            Method::POST,
            &format!("/reservas/{id}/recibo"),
            Some(json!({
                "novedad": "Dos sillas rotas", "valor_danos": "120000"
            })),
        )
        .await;
    assert_eq!(
        (s, &v["estado"]),
        (StatusCode::OK, &json!("con_novedad")),
        "{v}"
    );
    let cargo: String = sqlx::query_scalar("SELECT valor::text FROM cargos_pendientes")
        .fetch_one(&ph.api.duenio)
        .await
        .expect("cargo");
    assert_eq!(cargo, "120000.00");
    let (_, mias) = ph
        .api
        .pedir(
            Method::GET,
            &format!("{}/reservas?mias=true", ph.base),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(mias.as_array().map(Vec::len), Some(1));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn reserva_fuera_de_horario_o_de_bien_ajeno_se_rechaza(pool: PgPool) {
    let ph = ph(pool).await;
    ph.habilitar("bienes_comunes").await;
    ph.bien_con_dueno().await;
    let otro = ph
        .crear_bien(json!({ "tipo": "apartamento", "numero": "202" }))
        .await;
    let salon = salon(&ph).await;
    let dueno = ph.api.token("dueno").await;
    let (inicio, fin) = franja(5, 21, 23);
    let (s, v) = ph
        .api
        .pedir(
            Method::POST,
            &format!("{}/reservas", ph.base),
            Some(&dueno),
            Some(json!({
                "recurso_id": salon, "bien_privado_id": otro["id"], "inicio": inicio, "fin": fin,
                "idempotency_key": "x"
            })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sorteo_de_parqueaderos_verificable(pool: PgPool) {
    let ph = ph(pool).await;
    ph.habilitar("bienes_comunes").await;
    let (bien, _) = ph.bien_con_dueno().await;
    let mut parqueaderos = Vec::new();
    for c in ["C1", "C2"] {
        let (s, v) = ph
            .pedir(
                Method::POST,
                "/parqueaderos",
                Some(json!({
                    "codigo": c, "tipo_vehiculo": "carro", "naturaleza": "comunal"
                })),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        parqueaderos.push(v["id"].clone());
    }
    let desde = (Utc::now() - Duration::minutes(1)).to_rfc3339();
    let hasta = (Utc::now() + Duration::days(210)).to_rfc3339();
    let (s, sorteo) = ph
        .pedir(
            Method::POST,
            "/sorteos",
            Some(json!({
                "nombre": "Semestre 1", "recursos": parqueaderos,
                "vigencia_desde": desde, "vigencia_hasta": hasta
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{sorteo}");
    assert_eq!(
        sorteo["semilla"],
        Value::Null,
        "la semilla no se revela antes"
    );
    let id = sorteo["id"].as_str().expect("id");

    let dueno = ph.api.token("dueno").await;
    let (s, v) = ph
        .api
        .pedir(
            Method::POST,
            &format!("{}/sorteos/{id}/inscripciones", ph.base),
            Some(&dueno),
            Some(json!({ "bien_privado_id": bien })),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let (s, r) = ph
        .pedir(Method::POST, &format!("/sorteos/{id}/ejecutar"), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["asignaciones"].as_array().map(Vec::len), Some(1));
    let semilla = r["sorteo"]["semilla"].as_str().expect("semilla revelada");
    use sha2::Digest;
    assert_eq!(
        hex::encode(sha2::Sha256::digest(semilla.as_bytes())),
        sorteo["compromiso"].as_str().expect("compromiso"),
        "la semilla revelada corresponde al compromiso publicado"
    );
    let (_, lista) = ph.pedir(Method::GET, "/parqueaderos", None).await;
    let asignados = lista
        .as_array()
        .expect("lista")
        .iter()
        .filter(|p| !p["asignacion"].is_null())
        .count();
    assert_eq!(asignados, 1);
}
