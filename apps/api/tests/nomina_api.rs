//! Pruebas funcionales de la nómina (plan 16 fase 1, ADR 0025): la plataforma carga el
//! catálogo laboral; un gimnasio que **solo contrató la nómina** da de alta a su instructor con
//! su contrato, lo modifica, lo prorroga y lo termina, e importa a su aprendiz desde una
//! planilla con vista previa.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{Api, MIGRACIONES, api};
use serde_json::{Value, json};
use sqlx::PgPool;

struct Gimnasio {
    api: Api,
    root: String,
    gerente: String,
    id: String,
    eps: String,
    afp: String,
    cesantias: String,
    ccf: String,
    arl: String,
}

impl Gimnasio {
    async fn pedir(
        &self,
        metodo: Method,
        sufijo: &str,
        cuerpo: Option<Value>,
    ) -> (StatusCode, Value) {
        self.api
            .pedir(
                metodo,
                &format!("/api/v1/entidades/{}/nomina{sufijo}", self.id),
                Some(&self.gerente),
                cuerpo,
            )
            .await
    }

    fn condiciones(&self, salario: &str) -> Value {
        json!({
            "cargo": "Instructor",
            "salario": salario,
            "tipo_salario": "ordinario",
            "jornada": "completa",
            "periodicidad": "mensual",
            "eps_id": self.eps,
            "afp_id": self.afp,
            "cesantias_id": self.cesantias,
            "ccf_id": self.ccf,
            "arl_id": self.arl,
            "clase_riesgo": "1",
            "medio_pago": "transferencia",
            "cuenta": { "banco": "Banco X", "tipo": "ahorros", "numero": "0012345678" },
        })
    }
}

/// Empresa sin vertical con solo el módulo de nómina y el catálogo de 2026 cargado por la
/// plataforma.
async fn gimnasio(pool: PgPool) -> Gimnasio {
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
            Some(json!({ "nombre": "Gimnasio Fuerte", "administrador_cuenta_id": gerente_id })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().expect("id").to_owned();
    let (s, v) = api
        .pedir(
            Method::POST,
            &format!("/api/v1/plataforma/copropiedades/{id}/modulos"),
            Some(&root),
            Some(json!({ "modulo": "nomina" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    for (parametro, valor) in [
        ("salario_minimo", "1500000"),
        ("jornada_maxima_semanal", "42"),
        ("salario_integral_minimo", "13"),
        ("apoyo_aprendiz_lectiva", "75"),
        ("apoyo_aprendiz_productiva", "100"),
    ] {
        let (s, v) = api
            .pedir(
                Method::POST,
                "/api/v1/plataforma/nomina/parametros",
                Some(&root),
                Some(json!({ "parametro": parametro, "valor": valor,
                             "fuente": "Valor de prueba", "vigente_desde": "2026-01-01" })),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{parametro}: {v}");
    }
    let mut ids = Vec::new();
    for (i, (tipo, codigo)) in [
        ("eps", "EPS037"),
        ("afp", "230301"),
        ("cesantias", "230301"),
        ("ccf", "CCF22"),
        ("arl", "14-23"),
    ]
    .into_iter()
    .enumerate()
    {
        let (s, v) = api
            .pedir(
                Method::POST,
                "/api/v1/plataforma/nomina/administradoras",
                Some(&root),
                Some(
                    json!({ "tipo": tipo, "codigo": codigo, "nit": format!("90000000{i}"),
                             "nombre": format!("Administradora {tipo}") }),
                ),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{tipo}: {v}");
        ids.push(v["id"].as_str().expect("id").to_owned());
    }
    Gimnasio {
        gerente: api.token("gerente").await,
        root,
        id,
        eps: ids[0].clone(),
        afp: ids[1].clone(),
        cesantias: ids[2].clone(),
        ccf: ids[3].clone(),
        arl: ids[4].clone(),
        api,
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_gimnasio_contrata_modifica_prorroga_y_termina(pool: PgPool) {
    let g = gimnasio(pool).await;
    let (s, catalogo) = g
        .pedir(Method::GET, "/catalogo?fecha=2026-06-01", None)
        .await;
    assert_eq!(s, StatusCode::OK, "{catalogo}");
    assert_eq!(
        catalogo["administradoras"].as_array().map(Vec::len),
        Some(5)
    );
    let minimo = catalogo["parametros"]
        .as_array()
        .expect("parámetros")
        .iter()
        .find(|p| p["parametro"] == "salario_minimo")
        .expect("salario mínimo");
    assert_eq!(minimo["unidad"], "pesos");

    let (s, ficha) = g
        .pedir(
            Method::POST,
            "/empleados",
            Some(json!({
                "codigo": "E-001",
                "persona": { "nombre": "Ana Pérez", "tipo_documento": "CC",
                             "documento": "52.123.456", "correo": "ana@correo.co" },
                "contrato": {
                    "tipo": "termino_fijo",
                    "fecha_inicio": "2026-02-01",
                    "fecha_fin": "2026-07-31",
                    "condiciones": g.condiciones("2000000"),
                },
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{ficha}");
    let empleado = ficha["id"].as_str().expect("id").to_owned();
    let contrato = ficha["contratos"][0]["contrato"]["id"]
        .as_str()
        .expect("contrato")
        .to_owned();
    assert_eq!(ficha["documento"], "52123456");
    assert_eq!(
        ficha["contratos"][0]["versiones"][0]["salario"],
        "2000000.00"
    );

    let (s, v) = g
        .pedir(
            Method::POST,
            &format!("/contratos/{contrato}/versiones"),
            Some(json!({ "vigente_desde": "2026-05-01",
                         "condiciones": g.condiciones("1400000") })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "bajo el mínimo: {v}");
    assert_eq!(v["error"]["codigo"], "validacion");
    let (s, v) = g
        .pedir(
            Method::POST,
            &format!("/contratos/{contrato}/versiones"),
            Some(json!({ "vigente_desde": "2026-05-01",
                         "condiciones": g.condiciones("2200000") })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, v) = g
        .pedir(
            Method::POST,
            &format!("/contratos/{contrato}/prorroga"),
            Some(json!({ "fecha_fin": "2027-01-31" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["fecha_fin"], "2027-01-31");
    let (s, v) = g
        .pedir(
            Method::POST,
            &format!("/empleados/{empleado}/contratos"),
            Some(
                json!({ "tipo": "termino_indefinido", "fecha_inicio": "2026-12-01",
                         "condiciones": g.condiciones("2000000") }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "se cruza: {v}");
    let (s, v) = g
        .pedir(
            Method::POST,
            &format!("/contratos/{contrato}/terminacion"),
            Some(json!({ "fecha": "2026-09-30", "motivo": "renuncia" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["motivo_terminacion"], "renuncia");

    let (s, lista) = g.pedir(Method::GET, "/empleados?q=perez", None).await;
    assert_eq!(s, StatusCode::OK, "{lista}");
    assert_eq!(lista["datos"][0]["estado"], "retirado");
    assert_eq!(lista["datos"][0]["cargo"], "Instructor");
    let (s, ficha) = g
        .pedir(Method::GET, &format!("/empleados/{empleado}"), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{ficha}");
    assert_eq!(
        ficha["contratos"][0]["versiones"].as_array().map(Vec::len),
        Some(2)
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_aprendiz_llega_por_planilla_con_vista_previa(pool: PgPool) {
    let g = gimnasio(pool).await;
    let (s, tipo, plantilla) = g
        .api
        .pedir_texto(
            &format!("/api/v1/entidades/{}/nomina/importaciones/plantilla", g.id),
            &g.gerente,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(tipo.starts_with("text/csv"), "{tipo}");
    let encabezados = plantilla.lines().next().expect("encabezados");
    let csv = format!(
        "{encabezados}\n\
         A-1;Luis Gómez;CC;1010101010;;;aprendizaje;2026-02-01;2027-07-31;;Aprendiz;1125000;;lectiva;;;;EPS037;;;;;;;;;;\n\
         A-2;Sara Ruiz;CC;2020202020;;;aprendizaje;2026-02-01;2027-07-31;;Aprendiz;900000;;lectiva;;;;EPS037;;;;;;;;;;\n"
    );
    let ruta = format!(
        "/api/v1/entidades/{}/nomina/importaciones?nombre=empleados.csv",
        g.id
    );
    let (s, v) = g.api.subir(&ruta, &g.gerente, csv.into_bytes()).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["importacion"]["estado"], "con_errores");
    assert_eq!(v["importacion"]["errores"][0]["fila"], 3);
    assert!(
        !v.to_string().contains("900000"),
        "los errores no repiten el valor: {v}"
    );

    let csv = format!(
        "{encabezados}\n\
         A-1;Luis Gómez;CC;1010101010;;;aprendizaje;2026-02-01;2027-07-31;;Aprendiz;1125000;;lectiva;;;;EPS037;;;;;;;;;;\n"
    );
    let (s, v) = g.api.subir(&ruta, &g.gerente, csv.into_bytes()).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["importacion"]["estado"], "validada", "{v}");
    assert_eq!(v["filas"][0]["tipo_contrato"], "aprendizaje");
    let importacion = v["importacion"]["id"].as_str().expect("id").to_owned();
    let (s, lista) = g.pedir(Method::GET, "/empleados", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(lista["datos"].as_array().map(Vec::len), Some(0), "nada aún");

    let (s, v) = g
        .pedir(
            Method::POST,
            &format!("/importaciones/{importacion}/confirmar"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["resumen"]["empleados"], 1);
    assert_eq!(v["resumen"]["personas_nuevas"], 1);
    let (s, v) = g
        .pedir(
            Method::POST,
            &format!("/importaciones/{importacion}/confirmar"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "ya aplicada: {v}");
    let (_, lista) = g.pedir(Method::GET, "/empleados?estado=activo", None).await;
    assert_eq!(lista["datos"][0]["codigo"], "A-1");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_modulo_la_nomina_responde_403(pool: PgPool) {
    let g = gimnasio(pool).await;
    sqlx::query("DELETE FROM modulos_habilitados WHERE tenant_id = $1::uuid")
        .bind(&g.id)
        .execute(&g.api.duenio)
        .await
        .expect("sin módulo");

    let (s, v) = g.pedir(Method::GET, "/empleados", None).await;

    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["codigo"], "modulo_no_habilitado");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn solo_la_plataforma_carga_el_catalogo(pool: PgPool) {
    let g = gimnasio(pool).await;
    let (s, v) = g
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/nomina/parametros",
            Some(&g.gerente),
            Some(
                json!({ "parametro": "icbf", "valor": "3", "fuente": "Gerente",
                         "vigente_desde": "2026-01-01" }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");

    let (s, v) = g
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/nomina/parametros",
            Some(&g.root),
            Some(
                json!({ "parametro": "inicio_jornada_nocturna", "valor": "19.5",
                         "fuente": "Ley 2466 de 2025", "vigente_desde": "2026-01-01" }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "hora no entera: {v}");
    let (s, catalogo) = g
        .api
        .pedir(
            Method::GET,
            "/api/v1/plataforma/nomina/catalogo",
            Some(&g.root),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{catalogo}");
    let eps = catalogo["administradoras"]
        .as_array()
        .expect("administradoras")
        .iter()
        .find(|a| a["tipo"] == "eps")
        .expect("eps")["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let (s, v) = g
        .api
        .pedir(
            Method::PUT,
            &format!("/api/v1/plataforma/nomina/administradoras/{eps}"),
            Some(&g.root),
            Some(json!({ "nombre": "EPS en liquidación", "activa": false })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, vigente) = g.pedir(Method::GET, "/catalogo", None).await;
    assert_eq!(vigente["administradoras"].as_array().map(Vec::len), Some(4));
}
