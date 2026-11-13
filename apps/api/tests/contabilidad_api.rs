//! Pruebas funcionales del núcleo contable (plan 12) sobre el router real y PostgreSQL: lo que
//! harían la plataforma y el contador de una empresa cualquiera desde la web.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{Api, MIGRACIONES, api};
use serde_json::{Value, json};
use sqlx::PgPool;

struct Empresa {
    api: Api,
    root: String,
    gerente: String,
    id: String,
}

impl Empresa {
    fn ruta(&self, sufijo: &str) -> String {
        format!("/api/v1/entidades/{}{sufijo}", self.id)
    }

    async fn pedir(
        &self,
        metodo: Method,
        sufijo: &str,
        cuerpo: Option<Value>,
    ) -> (StatusCode, Value) {
        self.api
            .pedir(metodo, &self.ruta(sufijo), Some(&self.gerente), cuerpo)
            .await
    }

    async fn habilitar(&self, modulo: &str) -> (StatusCode, Value) {
        self.api
            .pedir(
                Method::POST,
                &format!("/api/v1/plataforma/copropiedades/{}/modulos", self.id),
                Some(&self.root),
                Some(json!({ "modulo": modulo })),
            )
            .await
    }
}

/// Empresa sin vertical dada de alta por la plataforma, con su gerente como administrador.
async fn empresa(pool: PgPool) -> Empresa {
    let api = api(pool).await;
    let root_id = api.registrar("root").await;
    api.hacer_superusuario(&root_id).await;
    let gerente_id = api.registrar("gerente").await;
    let root = api.token_con_segundo_factor("root").await;
    let (s, v) = api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/entidades",
            Some(&root),
            Some(json!({ "nombre": "Gimnasio Fuerza", "administrador_cuenta_id": gerente_id })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    Empresa {
        gerente: api.token("gerente").await,
        id: v["id"].as_str().expect("id").to_owned(),
        root,
        api,
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_empresa_lleva_su_contabilidad_de_punta_a_punta(pool: PgPool) {
    let e = empresa(pool).await;

    // Sin el módulo, la contabilidad no responde; la empresa no tiene rutas de copropiedad.
    let (s, v) = e.pedir(Method::GET, "/contabilidad/cuentas", None).await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::FORBIDDEN, &json!("modulo_no_habilitado"))
    );
    assert_eq!(e.habilitar("contabilidad").await.0, StatusCode::CREATED);
    let (s, _) = e
        .api
        .pedir(
            Method::GET,
            &format!("/api/v1/copropiedades/{}/capacidades", e.id),
            Some(&e.gerente),
            None,
        )
        .await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "una empresa no es una copropiedad"
    );
    let (_, capacidades) = e.pedir(Method::GET, "/capacidades", None).await;
    assert_eq!(capacidades["modulos"], json!(["contabilidad"]));

    for (codigo, movimiento) in [
        ("1", false),
        ("11", false),
        ("1105", true),
        ("4", false),
        ("41", false),
        ("4170", true),
    ] {
        let (s, v) = e
            .pedir(
                Method::POST,
                "/contabilidad/cuentas",
                Some(json!({ "codigo": codigo, "nombre": format!("Cuenta {codigo}"), "acepta_movimiento": movimiento })),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    let (s, v) = e
        .pedir(
            Method::POST,
            "/contabilidad/tipos-comprobante",
            Some(json!({ "codigo": "CC", "nombre": "Contable" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, v) = e
        .pedir(
            Method::PUT,
            "/contabilidad/configuracion",
            Some(json!({ "marco_tecnico": "grupo_3", "vigente_desde": "2026-01-01" })),
        )
        .await;
    assert_eq!((s, &v["version"]), (StatusCode::OK, &json!(1)), "{v}");

    let comprobante = |credito: &str| {
        json!({
            "tipo": "CC",
            "fecha": "2026-03-05",
            "descripcion": "Venta de contado",
            "lineas": [
                { "cuenta": "1105", "debito": "150000" },
                { "cuenta": "4170", "credito": credito },
            ],
        })
    };
    let (s, v) = e
        .pedir(
            Method::POST,
            "/contabilidad/comprobantes/previsualizar",
            Some(comprobante("149999")),
        )
        .await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("regla_contable")),
        "{v}"
    );
    let (s, borrador) = e
        .pedir(
            Method::POST,
            "/contabilidad/comprobantes",
            Some(comprobante("150000")),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{borrador}");
    let id = borrador["id"].as_str().expect("id");
    let (s, c) = e
        .pedir(
            Method::POST,
            &format!("/contabilidad/comprobantes/{id}/contabilizar"),
            None,
        )
        .await;
    assert_eq!(
        (s, &c["numero"], &c["estado"]),
        (StatusCode::OK, &json!(1), &json!("contabilizado")),
        "{c}"
    );
    let (s, _) = e
        .pedir(
            Method::PUT,
            &format!("/contabilidad/comprobantes/{id}"),
            Some(comprobante("150000")),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "lo contabilizado no se edita");

    let (s, balance) = e
        .pedir(
            Method::GET,
            "/contabilidad/reportes/balance-prueba?desde=2026-01-01&hasta=2026-12-31",
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{balance}");
    let caja = balance
        .as_array()
        .expect("filas")
        .iter()
        .find(|f| f["codigo"] == "1105")
        .expect("1105");
    assert_eq!(caja["saldo_final"], json!("150000.00"));
    let (s, tipo, csv) = e
        .api
        .pedir_texto(&e.ruta("/contabilidad/reportes/balance-prueba?desde=2026-01-01&hasta=2026-12-31&formato=csv"), &e.gerente)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(tipo.starts_with("text/csv"), "{tipo}");
    assert!(csv.starts_with("cuenta,nombre,"), "{csv}");
    let exportaciones: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM auditoria WHERE accion = 'contabilidad.libros.exportar'",
    )
    .fetch_one(&e.api.duenio)
    .await
    .expect("conteo");
    assert_eq!(
        exportaciones, 1,
        "la exportación queda en la auditoría (QA-017)"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn los_modulos_de_la_ph_no_se_habilitan_en_una_empresa(pool: PgPool) {
    let e = empresa(pool).await;

    let (s, v) = e.habilitar("asambleas").await;

    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::NOT_FOUND, &json!("no_encontrado")),
        "{v}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn quien_no_tiene_roles_en_la_entidad_no_la_ve(pool: PgPool) {
    let e = empresa(pool).await;
    e.api.registrar("extrano").await;
    let extrano = e.api.token("extrano").await;

    let (s, _) = e
        .api
        .pedir(Method::GET, &e.ruta("/capacidades"), Some(&extrano), None)
        .await;

    assert_eq!(s, StatusCode::NOT_FOUND);
}
