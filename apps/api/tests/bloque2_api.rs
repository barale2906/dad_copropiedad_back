//! Pruebas funcionales del plan 10 (bloque 2) sobre el router completo con PostgreSQL real.

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
