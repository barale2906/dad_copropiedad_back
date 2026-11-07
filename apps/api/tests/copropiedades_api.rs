//! Pruebas funcionales del plan 03 sobre el router real y PostgreSQL (rol `dad_app`): lo que
//! haría la administración desde la web.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{MIGRACIONES, ph};
use serde_json::{Value, json};
use sqlx::PgPool;

#[sqlx::test(migrator = "MIGRACIONES")]
async fn cuadro_que_no_suma_uno_se_rechaza_y_corregido_se_activa(pool: PgPool) {
    let ph = ph(pool).await;

    // Nomenclatura propia de la PH: torre 3, apartamento 506 → "3506".
    let (s, v) = ph
        .pedir(
            Method::PATCH,
            "",
            Some(json!({ "plantilla_codigo": "{agrupacion}{numero}" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["nit"], "800197268-4");
    let (s, torre) = ph
        .pedir(
            Method::POST,
            "/agrupaciones",
            Some(json!({ "tipo": "edificio", "nombre": "Torre 3", "codigo": "3" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{torre}");
    let mut ids = Vec::new();
    for numero in ["506", "507", "508"] {
        let b = ph
            .crear_bien(
                json!({ "tipo": "apartamento", "agrupacion_id": torre["id"], "numero": numero }),
            )
            .await;
        assert_eq!(b["codigo"], format!("3{numero}"));
        assert_eq!(b["coeficiente"], Value::Null);
        ids.push(b["id"].clone());
    }

    let (s, cuadro) = ph
        .pedir(Method::POST, "/cuadros-coeficientes", Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{cuadro}");
    let ruta = format!(
        "/cuadros-coeficientes/{}",
        cuadro["id"].as_str().expect("id")
    );
    let carga = |valores: [&str; 3]| {
        json!({ "coeficientes": ids.iter().zip(valores).map(|(id, c)| json!({
            "bien_privado_id": id, "coeficiente": c
        })).collect::<Vec<_>>() })
    };
    let (s, revision) = ph
        .pedir(
            Method::PUT,
            &format!("{ruta}/coeficientes"),
            Some(carga(["33,333333 %", "33,333333 %", "33,333333 %"])),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{revision}");
    assert_eq!(revision["diferencia"], "0.00000001");
    assert_eq!(revision["valido"], false);

    let (s, v) = ph
        .pedir(Method::POST, &format!("{ruta}/activar"), None)
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["codigo"], "coeficientes_no_suman_uno");

    ph.pedir(
        Method::PUT,
        &format!("{ruta}/coeficientes"),
        Some(carga(["0.33333333", "0.33333333", "0.33333334"])),
    )
    .await;
    let (s, v) = ph
        .pedir(Method::POST, &format!("{ruta}/activar"), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["estado"], "vigente");

    let (_, ficha) = ph
        .pedir(
            Method::GET,
            &format!("/bienes-privados/{}", ids[2].as_str().expect("id")),
            None,
        )
        .await;
    assert_eq!(ficha["bien_privado"]["coeficiente"], "0.33333334");
    let (_, v) = ph.pedir(Method::GET, "", None).await;
    assert_eq!(v["advertencias"], json!([]), "{v}");
    assert_eq!(v["cargos"][0]["rol"], "administrador");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn transferir_la_titularidad_retira_el_rol_al_vendedor(pool: PgPool) {
    let ph = ph(pool).await;
    let vendedor = ph.api.registrar("vendedor").await;
    let comprador = ph.api.registrar("comprador").await;
    let bien = ph
        .crear_bien(json!({ "tipo": "casa", "numero": "12" }))
        .await;
    let ruta = format!("/bienes-privados/{}", bien["id"].as_str().expect("id"));
    let titular = |doc: &str, cuenta: &str| {
        json!({ "titulares": [{
            "persona": { "nombre": format!("Persona {doc}"), "documento": doc, "cuenta_id": cuenta },
            "porcentaje": "100"
        }]})
    };

    let (s, v) = ph
        .pedir(
            Method::POST,
            &format!("{ruta}/titulares"),
            Some(titular("52123456", &vendedor)),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let token_vendedor = ph.api.token("vendedor").await;
    let (s, mios) = ph
        .api
        .pedir(
            Method::GET,
            &format!("{}/mis-bienes-privados", ph.base),
            Some(&token_vendedor),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{mios}");
    assert_eq!(mios[0]["codigo"], "12");
    // Como titular ve la ficha de su bien sin `bienes_privados.ver`.
    let (s, _) = ph
        .api
        .pedir(
            Method::GET,
            &format!("{}{ruta}", ph.base),
            Some(&token_vendedor),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    let (s, v) = ph
        .pedir(
            Method::POST,
            &format!("{ruta}/transferencias"),
            Some(titular("79123456", &comprador)),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    // El vendedor ya no tiene ningún rol en la PH: para él "no existe".
    let (s, v) = ph
        .api
        .pedir(Method::GET, &ph.base, Some(&token_vendedor), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    let token_comprador = ph.api.token("comprador").await;
    let (s, v) = ph
        .api
        .pedir(Method::GET, &ph.base, Some(&token_comprador), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, historial) = ph
        .pedir(Method::GET, &format!("{ruta}/titulares"), None)
        .await;
    assert_eq!(historial.as_array().map(Vec::len), Some(2));
    assert_eq!(historial[0]["titularidad"]["vigente_hasta"], Value::Null);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn titulares_que_no_suman_cien_se_rechazan(pool: PgPool) {
    let ph = ph(pool).await;
    let bien = ph
        .crear_bien(json!({ "tipo": "apartamento", "numero": "101" }))
        .await;
    let (s, v) = ph
        .pedir(
            Method::POST,
            &format!("/bienes-privados/{}/titulares", bien["id"].as_str().expect("id")),
            Some(json!({ "titulares": [
                { "persona": { "nombre": "Ana", "documento": "111" }, "porcentaje": "60", "es_representante": true },
                { "persona": { "nombre": "Luis", "documento": "222" }, "porcentaje": "30" },
            ]})),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["codigo"], "validacion");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn importar_xlsx_con_errores_devuelve_la_vista_previa_por_fila(pool: PgPool) {
    let ph = ph(pool).await;
    let ruta = format!(
        "{}/importaciones/bienes-privados?nombre=bienes.xlsx",
        ph.base
    );

    let con_errores = include_bytes!(
        "../../../crates/infra-planillas/tests/fixtures/bienes_privados_con_errores.xlsx"
    );
    let (s, v) = ph.api.subir(&ruta, &ph.admin, con_errores.to_vec()).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let imp = &v["importacion"];
    assert_eq!(imp["estado"], "con_errores");
    assert_eq!(
        (imp["total_filas"].as_u64(), imp["filas_validas"].as_u64()),
        (Some(3), Some(1))
    );
    let filas: Vec<_> = imp["errores"]
        .as_array()
        .expect("errores")
        .iter()
        .map(|e| (e["fila"].as_u64(), e["columna"].as_str().map(str::to_owned)))
        .collect();
    assert!(
        filas.contains(&(Some(3), Some("coeficiente".into()))),
        "{filas:?}"
    );
    assert!(filas.contains(&(Some(4), Some("tipo".into()))), "{filas:?}");
    let (s, v) = ph
        .pedir(
            Method::POST,
            &format!(
                "/importaciones/{}/confirmar",
                imp["id"].as_str().expect("id")
            ),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");

    let valido =
        include_bytes!("../../../crates/infra-planillas/tests/fixtures/bienes_privados.xlsx");
    let (s, v) = ph.api.subir(&ruta, &ph.admin, valido.to_vec()).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["importacion"]["estado"], "validada");
    assert_eq!(v["filas"][0]["codigo"], "3-506");
    assert_eq!(v["filas"][0]["agrupacion_nueva"], true);
    let (s, aplicada) = ph
        .pedir(
            Method::POST,
            &format!(
                "/importaciones/{}/confirmar",
                v["importacion"]["id"].as_str().expect("id")
            ),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{aplicada}");
    assert_eq!(aplicada["resumen"]["bienes_privados"], 2);
    assert_eq!(aplicada["resumen"]["personas_nuevas"], 2);

    // Los coeficientes del archivo quedan en un borrador que suma 1 y se puede activar.
    let borrador = aplicada["resumen"]["cuadro_borrador_id"]
        .as_str()
        .expect("borrador");
    let (s, v) = ph
        .pedir(
            Method::POST,
            &format!("/cuadros-coeficientes/{borrador}/activar"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, lista) = ph.pedir(Method::GET, "/bienes-privados?q=506", None).await;
    assert_eq!(lista["datos"][0]["coeficiente"], "0.00523000", "{lista}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn planilla_que_no_es_csv_ni_xlsx_se_rechaza(pool: PgPool) {
    let ph = ph(pool).await;
    let ruta = format!("{}/importaciones/bienes-privados?nombre=x.pdf", ph.base);
    let (s, v) = ph
        .api
        .subir(&ruta, &ph.admin, b"%PDF-1.7\x00\x01".to_vec())
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (s, _) = ph
        .pedir(
            Method::GET,
            "/importaciones/bienes-privados/plantilla",
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn puntos_de_atencion_y_ciclo_de_vida_de_la_copropiedad(pool: PgPool) {
    let ph = ph(pool).await;
    let (s, v) = ph
        .pedir(
            Method::POST,
            "/puntos-atencion",
            Some(json!({
                "tipo": "oficina_administracion",
                "nombre": "Oficina",
                "horario": { "franjas": [
                    { "dia": 1, "inicio": "08:00:00", "fin": "12:00:00" },
                    { "dia": 1, "inicio": "11:00:00", "fin": "13:00:00" }
                ]}
            })),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "franjas solapadas: {v}"
    );
    let (s, v) = ph
        .pedir(
            Method::POST,
            "/puntos-atencion",
            Some(json!({
                "tipo": "oficina_administracion",
                "nombre": "Oficina",
                "horario": { "franjas": [{ "dia": 1, "inicio": "08:00:00", "fin": "12:00:00" }],
                             "excepciones": [{ "fecha": "2026-12-24" }] }
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (_, lista) = ph.pedir(Method::GET, "/puntos-atencion", None).await;
    assert_eq!(lista[0]["horario"]["franjas"][0]["dia"], 1);
    assert_eq!(lista[0]["horario"]["excepciones"][0]["fecha"], "2026-12-24");

    assert_eq!(ph.estado("activa", &ph.root).await.0, StatusCode::OK);
    assert_eq!(ph.estado("retirada", &ph.root).await.0, StatusCode::OK);
    let (s, v) = ph.estado("activa", &ph.root).await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::CONFLICT, &json!("conflicto_estado"))
    );
    let (s, v) = ph.estado("suspendida", &ph.admin).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "el administrador no opera la plataforma: {v}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_rol_en_la_copropiedad_no_se_ve_nada(pool: PgPool) {
    let ph = ph(pool).await;
    ph.api.registrar("extrano").await;
    let token = ph.api.token("extrano").await;
    for ruta in [
        "",
        "/bienes-privados",
        "/agrupaciones",
        "/cuadros-coeficientes",
    ] {
        let (s, _) = ph
            .api
            .pedir(
                Method::GET,
                &format!("{}{ruta}", ph.base),
                Some(&token),
                None,
            )
            .await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{ruta}");
    }
}
