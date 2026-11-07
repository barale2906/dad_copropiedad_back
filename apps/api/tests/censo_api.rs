//! Pruebas funcionales del censo (plan 07a) sobre el router real y PostgreSQL (rol `dad_app`).

mod comun;

use axum::http::{Method, StatusCode};
use comun::{MIGRACIONES, ph};
use serde_json::{Value, json};
use sqlx::PgPool;

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_propietario_propone_un_vehiculo_y_la_administracion_lo_aprueba(pool: PgPool) {
    let ph = ph(pool).await;
    let (bien, _) = ph.bien_con_dueno().await;
    let dueno = ph.api.token("dueno").await;
    let ruta = |r: &str| format!("{}{r}", ph.base);

    let (s, cambio) = ph
        .api
        .pedir(
            Method::POST,
            &ruta("/cambios-censo"),
            Some(&dueno),
            Some(json!({
                "bien_privado_id": bien,
                "tipo": "vehiculo",
                "datos": { "tipo": "carro", "placa": "xyz-987", "marca": "Mazda" }
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{cambio}");
    assert_eq!(cambio["estado"], "pendiente");
    // El propietario no ve el censo de la copropiedad, solo sus propuestas.
    let (s, _) = ph
        .api
        .pedir(Method::GET, &ruta("/vehiculos"), Some(&dueno), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, mias) = ph
        .api
        .pedir(
            Method::GET,
            &ruta("/cambios-censo?mias=true"),
            Some(&dueno),
            None,
        )
        .await;
    assert_eq!(
        (s, mias.as_array().map(Vec::len)),
        (StatusCode::OK, Some(1))
    );

    let (_, pendientes) = ph
        .pedir(Method::GET, "/cambios-censo?estado=pendiente", None)
        .await;
    assert_eq!(pendientes.as_array().map(Vec::len), Some(1));
    let (s, v) = ph
        .pedir(
            Method::POST,
            &format!(
                "/cambios-censo/{}/aprobar",
                cambio["id"].as_str().expect("id")
            ),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, vehiculos) = ph.pedir(Method::GET, "/vehiculos?placa=XYZ987", None).await;
    assert_eq!(vehiculos[0]["marca"], "Mazda", "{vehiculos}");
    let (s, v) = ph
        .pedir(
            Method::POST,
            &format!(
                "/cambios-censo/{}/rechazar",
                cambio["id"].as_str().expect("id")
            ),
            Some(json!({ "motivo": "tarde" })),
        )
        .await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::CONFLICT, &json!("conflicto_estado"))
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_tenedor_entra_con_su_residencia_y_sale_al_terminarla(pool: PgPool) {
    let ph = ph(pool).await;
    let (bien, dueno) = ph.bien_con_dueno().await;
    let inquilino = ph.api.registrar("inquilino").await;
    let residencia = |autorizado: Value| {
        json!({
            "bien_privado_id": bien,
            "persona": { "nombre": "Luis Tenedor", "documento": "79123456", "cuenta_id": inquilino },
            "calidad": "tenedor",
            "es_responsable": true,
            "autorizado_por": autorizado,
        })
    };
    let (s, v) = ph
        .pedir(Method::POST, "/residencias", Some(residencia(Value::Null)))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "sin autorización: {v}");
    let (s, r) = ph
        .pedir(Method::POST, "/residencias", Some(residencia(json!(dueno))))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(r["persona"]["nombre"], "Luis Tenedor");

    let token = ph.api.token("inquilino").await;
    let (s, _) = ph
        .api
        .pedir(Method::GET, &ph.base, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::OK, "el tenedor ve la ficha de la PH");

    let (s, _) = ph
        .pedir(
            Method::POST,
            &format!("/residencias/{}/terminar", r["id"].as_str().expect("id")),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = ph
        .api
        .pedir(Method::GET, &ph.base, Some(&token), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "sin residencia ya no tiene rol");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn libro_de_propietarios_en_csv(pool: PgPool) {
    let ph = ph(pool).await;
    ph.bien_con_dueno().await;
    let (s, tipo, texto) = ph
        .api
        .pedir_texto(
            &format!("{}/libros/propietarios?formato=csv", ph.base),
            &ph.admin,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(tipo.starts_with("text/csv"), "{tipo}");
    assert!(
        texto.starts_with("\u{feff}\"Bien privado\";\"Coeficiente\""),
        "{texto}"
    );
    assert!(texto.contains("\"101\";\"\";\"Ana Dueña\""), "{texto}");
    let (_, json) = ph.pedir(Method::GET, "/libros/propietarios", None).await;
    assert_eq!(json[0]["documento"], "52123456");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn alertas_de_mascotas_y_datos_de_menores(pool: PgPool) {
    let ph = ph(pool).await;
    let (bien, dueno) = ph.bien_con_dueno().await;
    let (s, m) = ph
        .pedir(
            Method::POST,
            "/mascotas",
            Some(json!({
                "bien_privado_id": bien, "especie": "perro", "nombre": "Rocky",
                "raza": "Rottweiler", "potencialmente_peligrosa": true, "vacunas_vencen": "2099-01-01"
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{m}");
    let (_, alertas) = ph.pedir(Method::GET, "/censo/alertas", None).await;
    assert_eq!(
        alertas["mascotas"][0]["alertas"],
        json!(["sin_poliza"]),
        "{alertas}"
    );

    let (s, v) = ph
        .pedir(
            Method::PATCH,
            &format!("/mascotas/{}", m["id"].as_str().expect("id")),
            Some(json!({ "estado": "trasladada" })),
        )
        .await;
    assert_eq!((s, &v["estado"]), (StatusCode::OK, &json!("trasladada")));
    let (_, alertas) = ph.pedir(Method::GET, "/censo/alertas", None).await;
    assert_eq!(alertas["mascotas"], json!([]));

    let (s, p) = ph
        .pedir(
            Method::PATCH,
            &format!("/personas/{dueno}"),
            Some(json!({ "fecha_nacimiento": "2015-03-01", "tipo_documento": "TI" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["es_menor"], true);
    let (_, lista) = ph.pedir(Method::GET, "/personas?q=dueña", None).await;
    assert_eq!(lista["datos"][0]["tipo_documento"], "TI", "{lista}");
}
