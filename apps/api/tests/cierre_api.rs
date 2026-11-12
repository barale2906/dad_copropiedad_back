//! Pruebas funcionales del plan 09: protección de datos del titular (Ley 1581), consulta de
//! auditoría y límite de peticiones por IP.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{MIGRACIONES, api_con_limites, ph};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_titular_autoriza_consulta_y_reclama_y_la_administracion_responde(pool: PgPool) {
    let ph = ph(pool).await;
    let (_, persona) = ph.bien_con_dueno().await;
    let dueno = ph.api.token("dueno").await;
    let como_dueno = |m: Method, ruta: String, cuerpo| {
        let api = &ph.api;
        let url = format!("{}{ruta}", ph.base);
        let t = dueno.clone();
        async move { api.pedir(m, &url, Some(&t), cuerpo).await }
    };

    let (s, v) = como_dueno(
        Method::POST,
        format!("/personas/{persona}/autorizaciones-tratamiento"),
        Some(json!({
            "finalidades": ["comunicaciones"], "medio": "app"
        })),
    )
    .await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("validacion")),
        "sin política vigente"
    );

    let (s, p) = ph
        .pedir(
            Method::POST,
            "/proteccion-datos/politicas",
            Some(json!({ "texto": "Política de tratamiento" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let (s, vigente) = como_dueno(Method::GET, "/proteccion-datos/politica".into(), None).await;
    assert_eq!((s, &vigente["version"]), (StatusCode::OK, &json!(1)));
    let (s, v) = como_dueno(
        Method::POST,
        format!("/personas/{persona}/autorizaciones-tratamiento"),
        Some(json!({
            "finalidades": ["comunicaciones", "control_acceso"], "medio": "app"
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, v) = como_dueno(
        Method::POST,
        format!("/personas/{persona}/autorizaciones-tratamiento/revocacion"),
        Some(json!({
            "finalidades": ["control_acceso"]
        })),
    )
    .await;
    assert_eq!((s, &v["revocadas"]), (StatusCode::OK, &json!(1)));

    let (s, d) = como_dueno(Method::GET, "/proteccion-datos/mis-datos".into(), None).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(d["persona"]["id"], json!(persona));
    assert_eq!(d["autorizaciones"].as_array().map(Vec::len), Some(2));

    let (s, sol) = como_dueno(
        Method::POST,
        "/proteccion-datos/solicitudes".into(),
        Some(json!({
            "persona_id": persona, "tipo": "rectificacion", "detalle": "Cambió mi celular"
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{sol}");
    let id = sol["id"].as_str().expect("id").to_owned();
    let (s, _) = como_dueno(
        Method::POST,
        format!("/proteccion-datos/solicitudes/{id}/tramite"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "el titular no tramita");

    let (_, x) = ph
        .pedir(
            Method::POST,
            &format!("/proteccion-datos/solicitudes/{id}/tramite"),
            None,
        )
        .await;
    assert_eq!(x["estado"], "en_tramite");
    let (s, x) = ph
        .pedir(
            Method::POST,
            &format!("/proteccion-datos/solicitudes/{id}/prorroga"),
            Some(json!({ "motivo": "Verificación" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{x}");
    assert!(x["prorrogada_hasta"].is_string());
    let (s, _) = ph
        .pedir(
            Method::POST,
            &format!("/proteccion-datos/solicitudes/{id}/prorroga"),
            Some(json!({ "motivo": "Otra" })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "una sola prórroga");
    let (s, x) = ph
        .pedir(
            Method::POST,
            &format!("/proteccion-datos/solicitudes/{id}/respuesta"),
            Some(json!({ "respuesta": "Actualizado" })),
        )
        .await;
    assert_eq!((s, &x["estado"]), (StatusCode::OK, &json!("respondida")));

    let (_, mias) = como_dueno(Method::GET, "/proteccion-datos/solicitudes".into(), None).await;
    assert_eq!(mias["datos"][0]["respuesta"], "Actualizado");
    ph.api.registrar("vecino").await;
    let vecino = ph.api.token("vecino").await;
    let (s, _) = ph
        .api
        .pedir(
            Method::GET,
            &format!("{}/proteccion-datos/mis-datos", ph.base),
            Some(&vecino),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "sin rol en la PH no entra");

    // La auditoría registró todo y la consulta el administrador (sin datos personales).
    let (s, a) = ph
        .pedir(
            Method::GET,
            "/auditoria?accion=proteccion_datos.&limite=50",
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{a}");
    let acciones: Vec<&str> = a["datos"]
        .as_array()
        .expect("datos")
        .iter()
        .filter_map(|e| e["accion"].as_str())
        .collect();
    for esperada in [
        "proteccion_datos.solicitud.responder",
        "proteccion_datos.solicitud.prorrogar",
        "proteccion_datos.titular.consultar",
        "proteccion_datos.autorizacion.otorgar",
        "proteccion_datos.politica.publicar",
    ] {
        assert!(
            acciones.contains(&esperada),
            "falta {esperada} en {acciones:?}"
        );
    }
    assert!(
        !a.to_string().contains("Cambió mi celular"),
        "el detalle no llega a la auditoría"
    );
    let (s, p1) = ph
        .pedir(
            Method::GET,
            "/auditoria?accion=proteccion_datos.&limite=1",
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let siguiente = p1["siguiente"].as_str().expect("cursor");
    let (_, p2) = ph
        .pedir(
            Method::GET,
            &format!("/auditoria?accion=proteccion_datos.&limite=1&despues={siguiente}"),
            None,
        )
        .await;
    assert_ne!(p1["datos"][0]["id"], p2["datos"][0]["id"]);
    let (s, _) = como_dueno(Method::GET, "/auditoria".into(), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "sin auditoria.ver");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_inicio_de_sesion_se_limita_por_ip(pool: PgPool) {
    let api = api_con_limites(
        pool,
        api::limite::Limites {
            auth_por_minuto: 3,
            publico_por_minuto: 0,
            confiar_proxy: false,
        },
    )
    .await;
    let intento = json!({ "identificador": "nadie", "contrasena": "contrasena-equivocada" });

    for _ in 0..3 {
        let (s, _) = api
            .pedir(
                Method::POST,
                "/api/v1/auth/sesion",
                None,
                Some(intento.clone()),
            )
            .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    let (s, v) = api
        .pedir(Method::POST, "/api/v1/auth/sesion", None, Some(intento))
        .await;

    assert_eq!(
        (s, &v["error"]["codigo"]),
        (
            StatusCode::TOO_MANY_REQUESTS,
            &json!("demasiadas_peticiones")
        )
    );
    // Las rutas autenticadas no cuentan en ese límite.
    let (s, _) = api
        .pedir(Method::GET, "/api/v1/yo", Some("token-inexistente"), None)
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}
