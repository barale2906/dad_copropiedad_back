//! Pruebas funcionales del plan 10 (bloque 2) sobre el router completo con PostgreSQL real:
//! «mis solicitudes» entre PH, comunicados de la administración y campañas del censo.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{MIGRACIONES, ph};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrator = "MIGRACIONES")]
async fn mis_solicitudes_reune_los_tickets_de_todas_mis_copropiedades(pool: PgPool) {
    let ph = ph(pool).await;
    ph.habilitar("gestion_administrativa").await;
    assert_eq!(ph.estado("activa", &ph.root).await.0, StatusCode::OK);
    let (bien, _) = ph.bien_con_dueno().await;
    let dueno = ph.api.token("dueno").await;
    let (s, t) = ph
        .api
        .pedir(
            Method::POST,
            &format!("{}/tickets", ph.base),
            Some(&dueno),
            Some(json!({
                "tipo": "solicitud_mantenimiento", "asunto": "Fuga en el baño",
                "descripcion": "Gotea el techo", "bien_privado_id": bien,
                "idempotency_key": "clave-1"
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");

    let (s, mios) = ph
        .api
        .pedir(Method::GET, "/api/v1/yo/tickets", Some(&dueno), None)
        .await;

    assert_eq!(s, StatusCode::OK, "{mios}");
    assert_eq!(mios.as_array().map(Vec::len), Some(1));
    assert_eq!(mios[0]["copropiedad_id"], json!(ph.id));
    assert_eq!(mios[0]["numero"], t["numero"]);
    assert!(mios[0].get("descripcion").is_none(), "sin el detalle");
    let (_, del_admin) = ph
        .api
        .pedir(Method::GET, "/api/v1/yo/tickets", Some(&ph.admin), None)
        .await;
    assert_eq!(
        del_admin.as_array().map(Vec::len),
        Some(0),
        "solo los propios"
    );
    let (s, _) = ph
        .api
        .pedir(
            Method::GET,
            "/api/v1/yo/tickets?limite=0",
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_comunicado_llega_a_la_cartelera_y_cuenta_su_lectura(pool: PgPool) {
    let ph = ph(pool).await;
    ph.habilitar("gestion_administrativa").await;
    assert_eq!(ph.estado("activa", &ph.root).await.0, StatusCode::OK);
    ph.bien_con_dueno().await;
    let dueno = ph.api.token("dueno").await;
    let cuerpo = json!({
        "titulo": "Asamblea ordinaria", "cuerpo": "Convocatoria.\nOrden del día adjunto.",
        "finalidad": "administracion", "audiencia": { "tipo": "propietarios" },
        "requiere_confirmacion": true
    });

    // El propietario no puede redactar comunicados.
    let (s, _) = ph
        .api
        .pedir(
            Method::POST,
            &format!("{}/comunicados", ph.base),
            Some(&dueno),
            Some(cuerpo.clone()),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (s, c) = ph.pedir(Method::POST, "/comunicados", Some(cuerpo)).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    assert_eq!(c["estado"], "borrador");
    let id = c["id"].as_str().expect("id").to_owned();
    let (s, e) = ph
        .pedir(
            Method::POST,
            &format!("/comunicados/{id}/publicar"),
            Some(json!({})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{e}");
    assert_eq!(e["estado"], "enviado");
    let (s, _) = ph
        .pedir(Method::POST, &format!("/comunicados/{id}/anular"), None)
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "lo enviado no se anula");

    let (s, mios) = ph
        .api
        .pedir(
            Method::GET,
            &format!("{}/mis-comunicados", ph.base),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{mios}");
    assert_eq!(mios["datos"].as_array().map(Vec::len), Some(1));
    assert!(mios["datos"][0]["leido_en"].is_null());
    let (s, abierto) = ph
        .api
        .pedir(
            Method::GET,
            &format!("{}/mis-comunicados/{id}", ph.base),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{abierto}");
    assert!(abierto["leido_en"].is_string());
    let (s, ent) = ph
        .api
        .pedir(
            Method::POST,
            &format!("{}/mis-comunicados/{id}/enterado", ph.base),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{ent}");
    assert!(ent["enterado_en"].is_string());

    let (s, tablero) = ph
        .pedir(Method::GET, &format!("/comunicados/{id}"), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{tablero}");
    assert_eq!(tablero["resumen"]["destinatarios"], 1);
    assert_eq!(tablero["resumen"]["por_correo"], 1);
    assert_eq!(tablero["resumen"]["leidos"], 1);
    assert_eq!(tablero["resumen"]["enterados"], 1);
    let correos: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE concepto = 'comunicados' AND canal = 'correo'",
    )
    .fetch_one(&ph.api.duenio)
    .await
    .expect("outbox");
    assert_eq!(correos, 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_comunicado_con_audiencia_invalida_responde_422(pool: PgPool) {
    let ph = ph(pool).await;
    ph.habilitar("gestion_administrativa").await;
    let (s, v) = ph
        .pedir(
            Method::POST,
            "/comunicados",
            Some(json!({
                "titulo": "Aviso", "cuerpo": "Texto", "finalidad": "administracion",
                "audiencia": { "tipo": "bienes", "ids": [] }
            })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["codigo"], "validacion");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_campana_del_censo_llega_al_representante_y_sus_cambios_se_atienden(pool: PgPool) {
    let ph = ph(pool).await;
    let (bien, persona) = ph.bien_con_dueno().await;
    let (s, _) = ph.pedir(Method::POST, "/censo/campanas", None).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "sin línea de WhatsApp");
    sqlx::query(
        "INSERT INTO lineas_whatsapp (id, phone_number_id, numero_visible, nombre)
         VALUES (gen_random_uuid(), 'PNID', '+57 300', 'Línea')",
    )
    .execute(&ph.api.duenio)
    .await
    .expect("línea");
    sqlx::query("UPDATE personas SET telefono = '573001112233' WHERE id = $1::uuid")
        .bind(&persona)
        .execute(&ph.api.duenio)
        .await
        .expect("teléfono");

    let (s, c) = ph.pedir(Method::POST, "/censo/campanas", None).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    assert_eq!(c["enviados"], 1, "el representante, porque nadie reside");
    let campana = c["id"].as_str().expect("id").to_owned();
    let plantilla: String = sqlx::query_scalar(
        "SELECT payload->'contenido'->>'nombre' FROM outbox WHERE canal = 'whatsapp'",
    )
    .fetch_one(&ph.api.duenio)
    .await
    .expect("outbox");
    assert_eq!(plantilla, "actualizar_datos");

    // La respuesta llega por el chat del gateway; aquí se simula en la BD.
    sqlx::query(
        "INSERT INTO respuestas_censo (tenant_id, campana_id, persona_id, bien_privado_id,
             respuesta, detalle, respondida_en)
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, 'cambios', 'Carro nuevo ABC123', now())",
    )
    .bind(&ph.id)
    .bind(&campana)
    .bind(&persona)
    .bind(&bien)
    .execute(&ph.api.duenio)
    .await
    .expect("respuesta");
    let (s, r) = ph
        .pedir(
            Method::GET,
            &format!("/censo/campanas/{campana}/respuestas"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r[0]["detalle"], "Carro nuevo ABC123");
    assert_eq!(r[0]["nombre"], "Ana Dueña");
    let (s, _) = ph
        .pedir(
            Method::POST,
            &format!("/censo/campanas/{campana}/respuestas/{persona}/{bien}/atender"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, lista) = ph.pedir(Method::GET, "/censo/campanas", None).await;
    assert_eq!(lista[0]["con_cambios"], 1);
    assert_eq!(lista[0]["por_atender"], 0);
}
