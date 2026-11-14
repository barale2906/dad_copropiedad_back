//! Pruebas funcionales de lo tributario y los documentos electrónicos (plan 15) sobre el router
//! real y PostgreSQL, con el proveedor de facturación simulado: lo que harían la plataforma, el
//! contador y el gerente de una empresa cualquiera desde la web.

mod comun;

use axum::http::{Method, StatusCode};
use base64::{Engine, engine::general_purpose::STANDARD};
use comun::{Api, MIGRACIONES, api};
use serde_json::{Value, json};
use sqlx::PgPool;

const FACTURA_UBL: &str =
    include_str!("../../../crates/infra-facturacion/tests/fixtures/factura.xml");

struct Empresa {
    api: Api,
    root: String,
    gerente: String,
    id: String,
    cliente: String,
    proveedor: String,
}

impl Empresa {
    async fn pedir(
        &self,
        metodo: Method,
        sufijo: &str,
        cuerpo: Option<Value>,
    ) -> (StatusCode, Value) {
        self.api
            .pedir(
                metodo,
                &format!("/api/v1/entidades/{}/contabilidad{sufijo}", self.id),
                Some(&self.gerente),
                cuerpo,
            )
            .await
    }

    async fn crear(&self, sufijo: &str, cuerpo: Value) -> Value {
        let (s, v) = self.pedir(Method::POST, sufijo, Some(cuerpo)).await;
        assert!(s.is_success(), "{sufijo}: {s} {v}");
        v
    }

    async fn plataforma(&self, sufijo: &str, cuerpo: Value) -> Value {
        let (s, v) = self
            .api
            .pedir(
                Method::POST,
                &format!("/api/v1/plataforma{sufijo}"),
                Some(&self.root),
                Some(cuerpo),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{sufijo}: {v}");
        v
    }

    /// Perfil tributario confirmado: responsable de IVA, agente de retención y facturador
    /// electrónico por proveedor.
    async fn confirmar_perfil(&self) {
        let perfil = self
            .crear(
                "/tributario/perfiles",
                json!({ "vigente_desde": "2026-01-01", "parametros": {
                    "agente_retenedor": true, "responsable_iva": true,
                    "gran_contribuyente": false, "autorretenedor": false,
                    "contribuyente_renta": true, "facturador_electronico": true,
                    "obligado_rub": true, "municipios_ica": ["11001"],
                    "actividades_ica": ["9311"], "canal_facturacion": "proveedor" } }),
            )
            .await;
        assert_eq!(perfil["estado"], json!("borrador"));
        let confirmado = self
            .crear(
                &format!("/tributario/perfiles/{}/confirmacion", id(&perfil)),
                json!({}),
            )
            .await;
        assert_eq!(confirmado["estado"], json!("confirmado"));
    }

    /// Factura de venta emitida con una línea de alquiler (con o sin IVA).
    async fn venta(&self, valor: i64, tarifa_iva: &str) -> String {
        let f = self
            .crear(
                "/ventas/facturas",
                json!({ "persona_id": self.cliente, "fecha": "2026-03-02",
                        "vencimiento": "2026-04-01", "descripcion": "Alquiler del salón",
                        "lineas": [{ "descripcion": "Salón", "cuenta": "4170",
                                     "cantidad": "1", "valor_unitario": valor.to_string(),
                                     "tarifa_iva": tarifa_iva,
                                     "clasificacion": "alquiler_servicio" }] }),
            )
            .await;
        let f = id(&f);
        self.crear(&format!("/ventas/facturas/{f}/emision"), json!({}))
            .await;
        f
    }
}

fn id(v: &Value) -> String {
    v["id"]
        .as_str()
        .or_else(|| v["persona_id"].as_str())
        .expect("id")
        .to_owned()
}

/// Empresa con contabilidad y facturación electrónica, NIT, plan de cuentas, reglas de venta y
/// compra con IVA, un cliente, el proveedor de la factura UBL de prueba y la tarifa general de
/// IVA en el catálogo. Sin perfil tributario confirmado.
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
    let entidad = v["id"].as_str().expect("id").to_owned();
    for modulo in ["contabilidad", "facturacion_electronica"] {
        let (s, v) = api
            .pedir(
                Method::POST,
                &format!("/api/v1/plataforma/copropiedades/{entidad}/modulos"),
                Some(&root),
                Some(json!({ "modulo": modulo })),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    sqlx::query("UPDATE entidades SET nit = '900123456-1' WHERE id = $1::uuid")
        .bind(&entidad)
        .execute(&api.duenio)
        .await
        .expect("nit");
    let mut e = Empresa {
        gerente: api.token("gerente").await,
        root,
        id: entidad,
        api,
        cliente: String::new(),
        proveedor: String::new(),
    };
    e.plataforma(
        "/tributario/tarifas-iva",
        json!({ "codigo": "general", "nombre": "Tarifa general", "tratamiento": "gravado",
                "tarifa": "19", "fuente": "Estatuto Tributario, art. 468",
                "vigente_desde": "2026-01-01" }),
    )
    .await;
    for (codigo, movimiento, tercero) in [
        ("1", false, false),
        ("13", false, false),
        ("1305", true, true),
        ("2", false, false),
        ("23", false, false),
        ("2335", true, true),
        ("24", false, false),
        ("2408", true, false),
        ("4", false, false),
        ("41", false, false),
        ("4170", true, false),
        ("5", false, false),
        ("51", false, false),
        ("5135", true, false),
    ] {
        e.crear(
            "/cuentas",
            json!({ "codigo": codigo, "nombre": format!("Cuenta {codigo}"),
                    "acepta_movimiento": movimiento, "exige_tercero": tercero }),
        )
        .await;
    }
    for codigo in ["FC", "FV"] {
        e.crear(
            "/tipos-comprobante",
            json!({ "codigo": codigo, "nombre": format!("Tipo {codigo}"), "manual": false }),
        )
        .await;
    }
    let (s, v) = e
        .pedir(
            Method::PUT,
            "/configuracion",
            Some(json!({ "marco_tecnico": "grupo_3", "vigente_desde": "2026-01-01" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let linea = |concepto: &str, cara: &str, cuenta: &str, tercero: bool| {
        json!({ "concepto": concepto, "cara": cara, "cuenta": cuenta,
                "tercero": if tercero { "documento" } else { "ninguno" }, "centro": "ninguno" })
    };
    for (evento, tipo, lineas) in [
        (
            "cxp.factura",
            "FC",
            vec![
                linea("gasto", "debito", "5135", false),
                linea("iva_descontable", "debito", "2408", false),
                linea("por_pagar", "credito", "2335", true),
            ],
        ),
        (
            "ventas.factura",
            "FV",
            vec![
                linea("por_cobrar", "debito", "1305", true),
                linea("ingreso", "credito", "4170", false),
                linea("iva_generado", "credito", "2408", false),
            ],
        ),
    ] {
        e.crear(
            "/reglas",
            json!({ "evento": evento, "tipo_comprobante": tipo, "vigente_desde": "2026-01-01",
                    "lineas": lineas }),
        )
        .await;
    }
    e.cliente = id(&e
        .crear(
            "/terceros",
            json!({ "tipo": "juridica", "nombre": "Cliente S.A.S.", "documento": "800111222" }),
        )
        .await);
    e.proveedor = id(&e
        .crear(
            "/terceros",
            json!({ "tipo": "juridica", "nombre": "Proveedor de Aseo S.A.S.",
                    "documento": "900373115", "fiscal": { "regimen_iva": "responsable" } }),
        )
        .await);
    e
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_perfil_confirmado_no_hay_iva_y_con_el_solo_en_alquiler_o_venta(pool: PgPool) {
    let e = empresa(pool).await;
    let factura = |clasificacion: &str| {
        json!({ "persona_id": e.cliente, "fecha": "2026-03-02", "vencimiento": "2026-04-01",
                "descripcion": "Venta", "lineas": [{ "descripcion": "Línea", "cuenta": "4170",
                "cantidad": "1", "valor_unitario": "100000", "tarifa_iva": "19",
                "clasificacion": clasificacion }] })
    };

    let (s, v) = e
        .pedir(
            Method::POST,
            "/ventas/facturas",
            Some(factura("alquiler_servicio")),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "sin perfil: {v}");

    e.confirmar_perfil().await;
    let (s, v) = e
        .pedir(Method::POST, "/ventas/facturas", Some(factura("expensa")))
        .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "una expensa no lleva IVA: {v}"
    );
    let (s, v) = e
        .pedir(
            Method::POST,
            "/ventas/facturas",
            Some(factura("alquiler_servicio")),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["iva"], json!("19000.00"));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_perfil_confirmado_no_se_vuelve_a_confirmar(pool: PgPool) {
    let e = empresa(pool).await;
    let perfil = e
        .crear(
            "/tributario/perfiles",
            json!({ "vigente_desde": "2026-01-01", "parametros": {
                "agente_retenedor": false, "responsable_iva": false, "gran_contribuyente": false,
                "autorretenedor": false, "contribuyente_renta": true,
                "facturador_electronico": false, "obligado_rub": false,
                "canal_facturacion": "ninguno" } }),
        )
        .await;
    let ruta = format!("/tributario/perfiles/{}/confirmacion", id(&perfil));
    e.crear(&ruta, json!({})).await;

    let (s, v) = e.pedir(Method::POST, &ruta, Some(json!({}))).await;

    assert_eq!(s, StatusCode::CONFLICT, "{v}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_perfil_incoherente_con_el_rut_se_rechaza(pool: PgPool) {
    let e = empresa(pool).await;

    let (s, v) = e
        .pedir(
            Method::POST,
            "/tributario/perfiles",
            Some(json!({ "vigente_desde": "2026-01-01", "parametros": {
                "responsabilidades": ["R-99-PN"], "agente_retenedor": false,
                "responsable_iva": true, "gran_contribuyente": false, "autorretenedor": false,
                "contribuyente_renta": true, "facturador_electronico": false,
                "obligado_rub": false, "canal_facturacion": "ninguno" } })),
        )
        .await;

    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn factura_electronica_numerada_idempotente_y_anulada_con_nota_credito(pool: PgPool) {
    let e = empresa(pool).await;
    e.confirmar_perfil().await;
    let factura = e.venta(100_000, "19").await;
    let fe = format!("/ventas/facturas/{factura}/factura-electronica");

    let (s, v) = e.pedir(Method::POST, &fe, None).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "sin resolución: {v}");

    let resolucion = e
        .crear(
            "/tributario/resoluciones",
            json!({ "tipo": "factura", "prefijo": "FE", "desde": 1, "hasta": 100,
                    "numero_resolucion": "18764000000001", "fecha_resolucion": "2026-01-01",
                    "vigente_desde": "2026-01-01", "vigente_hasta": "2027-12-31",
                    "clave_tecnica": "clave-de-prueba" }),
        )
        .await;
    assert_eq!(resolucion["tiene_clave_tecnica"], json!(true));
    assert!(
        resolucion.get("clave_tecnica").is_none(),
        "la clave técnica no sale"
    );

    let primero = e.crear(&fe, json!({})).await;
    assert_eq!(primero["estado"], json!("validado"), "{primero}");
    assert_eq!(primero["numero"], json!(1));
    assert_eq!(primero["proveedor"], json!("simulado"));
    let cufe = primero["cufe"].as_str().expect("cufe").to_owned();
    assert_eq!(cufe.len(), 96);
    let otra_vez = e.crear(&fe, json!({})).await;
    assert_eq!(
        otra_vez["id"], primero["id"],
        "doble envío: el mismo documento"
    );
    assert_eq!(otra_vez["cufe"], json!(cufe));

    let segunda = e.venta(50_000, "0").await;
    let siguiente = e
        .crear(
            &format!("/ventas/facturas/{segunda}/factura-electronica"),
            json!({}),
        )
        .await;
    assert_eq!(siguiente["numero"], json!(2), "numera en orden");

    let (s, v) = e
        .pedir(
            Method::POST,
            &format!("/ventas/facturas/{factura}/anulacion"),
            Some(json!({ "motivo": "Error de digitación" })),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "con FE validada se anula con nota crédito: {v}"
    );

    let nota = e
        .crear(
            &format!("/ventas/facturas/{factura}/nota-credito"),
            json!({ "motivo": "Se facturó un valor equivocado" }),
        )
        .await;
    assert_eq!(nota["tipo"], json!("nota_credito"));
    assert_eq!(nota["estado"], json!("validado"));
    assert_eq!(nota["referencia_id"], primero["id"]);
    let (_, venta) = e
        .pedir(Method::GET, &format!("/ventas/facturas/{factura}"), None)
        .await;
    assert_eq!(venta["estado"], json!("anulada"));

    let (s, detalle) = e
        .pedir(
            Method::GET,
            &format!("/facturacion/documentos/{}", id(&primero)),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let eventos: Vec<&str> = detalle["eventos"]
        .as_array()
        .expect("eventos")
        .iter()
        .filter_map(|x| x["tipo"].as_str())
        .collect();
    assert_eq!(eventos, ["creado", "validado"]);
    let (s, lista) = e
        .pedir(Method::GET, "/facturacion/documentos?tipo=factura", None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(lista.as_array().expect("lista").len(), 2);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_perfil_de_facturador_no_emite(pool: PgPool) {
    let e = empresa(pool).await;
    let factura = e.venta(100_000, "0").await;

    let (s, v) = e
        .pedir(
            Method::POST,
            &format!("/ventas/facturas/{factura}/factura-electronica"),
            None,
        )
        .await;

    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_representacion_grafica_es_un_pdf(pool: PgPool) {
    let e = empresa(pool).await;
    e.confirmar_perfil().await;
    e.crear(
        "/tributario/resoluciones",
        json!({ "tipo": "factura", "prefijo": "FE", "desde": 1, "hasta": 100,
                "numero_resolucion": "18764000000001", "fecha_resolucion": "2026-01-01",
                "vigente_desde": "2026-01-01", "vigente_hasta": "2027-12-31" }),
    )
    .await;
    let factura = e.venta(100_000, "19").await;
    let fe = e
        .crear(
            &format!("/ventas/facturas/{factura}/factura-electronica"),
            json!({}),
        )
        .await;

    let (s, tipo, cuerpo) = e
        .api
        .pedir_texto(
            &format!(
                "/api/v1/entidades/{}/contabilidad/facturacion/documentos/{}/pdf",
                e.id,
                id(&fe)
            ),
            &e.gerente,
        )
        .await;

    assert_eq!(s, StatusCode::OK);
    assert_eq!(tipo, "application/pdf");
    assert!(cuerpo.starts_with("%PDF"));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn factura_recibida_por_xml_sin_duplicados_y_eventos_radian_en_orden(pool: PgPool) {
    let e = empresa(pool).await;
    let carga = json!({ "nombre_archivo": "fv-SETP990000123.xml",
                        "archivo_base64": STANDARD.encode(FACTURA_UBL) });

    let recibida = e.crear("/facturacion/recibidas", carga.clone()).await;
    assert_eq!(recibida["estado"], json!("pendiente"));
    assert_eq!(recibida["emisor_nit"], json!("900373115"));
    assert_eq!(
        recibida["persona_id"],
        json!(e.proveedor),
        "asociada por NIT"
    );
    assert_eq!(recibida["total"], json!("1190000.00"));
    let (s, v) = e
        .pedir(Method::POST, "/facturacion/recibidas", Some(carga))
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "la misma factura no entra dos veces: {v}"
    );

    let eventos = format!("/facturacion/recibidas/{}/eventos", id(&recibida));
    let (s, v) = e
        .pedir(
            Method::POST,
            &eventos,
            Some(json!({ "tipo": "aceptacion" })),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "sin acuse ni recibo: {v}"
    );
    let acuse = e.crear(&eventos, json!({ "tipo": "acuse" })).await;
    assert_eq!(acuse["estado"], json!("enviado"));
    let (s, _) = e
        .pedir(Method::POST, &eventos, Some(json!({ "tipo": "acuse" })))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "un evento no se repite");
    e.crear(&eventos, json!({ "tipo": "recibo_bien" })).await;
    let (s, v) = e
        .pedir(Method::POST, &eventos, Some(json!({ "tipo": "reclamo" })))
        .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "reclamo sin motivo: {v}"
    );

    let rechazada = e
        .crear(
            &format!("/facturacion/recibidas/{}/rechazo", id(&recibida)),
            json!({ "motivo": "No corresponde a un servicio prestado" }),
        )
        .await;
    assert_eq!(rechazada["estado"], json!("rechazada"));
    let (_, detalle) = e
        .pedir(
            Method::GET,
            &format!("/facturacion/recibidas/{}", id(&recibida)),
            None,
        )
        .await;
    assert_eq!(detalle["eventos"].as_array().expect("eventos").len(), 2);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_archivo_que_no_es_factura_se_rechaza(pool: PgPool) {
    let e = empresa(pool).await;

    let (s, v) = e
        .pedir(
            Method::POST,
            "/facturacion/recibidas",
            Some(json!({ "nombre_archivo": "otra.xml",
                         "archivo_base64": STANDARD.encode("<?xml version=\"1.0\"?><Nota/>") })),
        )
        .await;

    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn prepara_el_iva_y_la_exogena_es_reproducible(pool: PgPool) {
    let e = empresa(pool).await;
    e.confirmar_perfil().await;
    e.venta(100_000, "19").await;
    e.venta(50_000, "0").await;

    let (s, iva) = e
        .pedir(
            Method::GET,
            "/tributario/iva?desde=2026-03-01&hasta=2026-04-30",
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{iva}");
    assert_eq!(iva["total_generado"], json!("19000.00"));
    assert_eq!(iva["responsable"], json!(true));

    let formato = e
        .plataforma(
            "/tributario/formatos-exogena",
            json!({ "anio": 2026, "codigo": "1007", "nombre": "Ingresos recibidos",
                    "resolucion": "Resolución de prueba", "cuantia_menor": "60000",
                    "campos": [{ "concepto": "4001", "cuenta_prefijo": "1305",
                                 "columna": "ingreso_bruto", "lado": "debito" }] }),
        )
        .await;
    assert_eq!(formato["version"], json!(1));
    let generar = json!({ "formato_id": id(&formato) });
    let primera = e.crear("/tributario/exogena", generar.clone()).await;
    let segunda = e.crear("/tributario/exogena", generar).await;
    assert_eq!(
        primera["generacion"]["sha256"], segunda["generacion"]["sha256"],
        "los mismos datos y la misma versión dan el mismo archivo"
    );
    let csv = primera["csv"].as_str().expect("csv");
    assert!(csv.contains("800111222"), "{csv}");
    let (_, lista) = e
        .pedir(Method::GET, "/tributario/exogena?anio=2026", None)
        .await;
    assert_eq!(lista.as_array().expect("lista").len(), 2);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_ica_exige_perfil_confirmado(pool: PgPool) {
    let e = empresa(pool).await;

    let (s, v) = e
        .pedir(
            Method::GET,
            "/tributario/ica?desde=2026-01-01&hasta=2026-12-31",
            None,
        )
        .await;

    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn beneficiario_final_con_participacion_solo_por_propiedad(pool: PgPool) {
    let e = empresa(pool).await;
    let (s, v) = e
        .pedir(
            Method::POST,
            "/tributario/beneficiarios",
            Some(json!({ "persona_id": e.cliente, "criterio": "control",
                         "porcentaje": "30", "vigente_desde": "2026-01-01" })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");

    let b = e
        .crear(
            "/tributario/beneficiarios",
            json!({ "persona_id": e.cliente, "criterio": "propiedad", "porcentaje": "30",
                    "vigente_desde": "2026-01-01" }),
        )
        .await;
    let terminar = format!("/tributario/beneficiarios/{}/terminacion", id(&b));
    let (s, _) = e
        .pedir(
            Method::POST,
            &terminar,
            Some(json!({ "hasta": "2026-06-30" })),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = e
        .pedir(
            Method::POST,
            &terminar,
            Some(json!({ "hasta": "2026-07-31" })),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "ya terminó");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_modulo_no_hay_facturacion_electronica(pool: PgPool) {
    let e = empresa(pool).await;
    sqlx::query("DELETE FROM modulos_habilitados WHERE modulo = 'facturacion_electronica'")
        .execute(&e.api.duenio)
        .await
        .expect("quitar módulo");

    // Emitir, reintentar y RADIAN pasan por el proveedor: exigen el módulo. Leer el historial y
    // el buzón de recibidas no (toda entidad recibe facturas electrónicas).
    let (s, v) = e
        .pedir(
            Method::POST,
            &format!("/facturacion/documentos/{}/reintento", uuid::Uuid::now_v7()),
            None,
        )
        .await;

    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["codigo"], json!("modulo_no_habilitado"));
}
