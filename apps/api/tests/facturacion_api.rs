//! Pruebas funcionales de la facturación que se vende sola (plan 18, ADR 0022): una empresa que
//! **solo contrató la facturación electrónica** —sin contabilidad— crea su cliente, confirma su
//! perfil tributario, arma su catálogo, factura, emite la factura electrónica, marca el pago y
//! entrega los libros a su contador; la plataforma ve su consumo.

mod comun;

use axum::http::{Method, StatusCode};
use comun::{Api, MIGRACIONES, api};
use serde_json::{Value, json};
use sqlx::PgPool;

struct Tienda {
    api: Api,
    root: String,
    gerente: String,
    id: String,
}

impl Tienda {
    async fn pedir(
        &self,
        metodo: Method,
        sufijo: &str,
        cuerpo: Option<Value>,
    ) -> (StatusCode, Value) {
        self.api
            .pedir(
                metodo,
                &format!("/api/v1/entidades/{}{sufijo}", self.id),
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
}

fn id(v: &Value) -> String {
    v["id"]
        .as_str()
        .or_else(|| v["persona_id"].as_str())
        .expect("id")
        .to_owned()
}

/// Empresa sin vertical con solo el módulo de facturación electrónica, NIT y la tarifa general
/// de IVA en el catálogo de la plataforma.
async fn tienda(pool: PgPool) -> Tienda {
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
            Some(json!({ "nombre": "Tienda La Esquina", "administrador_cuenta_id": gerente_id })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let entidad = v["id"].as_str().expect("id").to_owned();
    let (s, v) = api
        .pedir(
            Method::POST,
            &format!("/api/v1/plataforma/copropiedades/{entidad}/modulos"),
            Some(&root),
            Some(json!({ "modulo": "facturacion_electronica" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    sqlx::query("UPDATE entidades SET nit = '900123456-8' WHERE id = $1::uuid")
        .bind(&entidad)
        .execute(&api.duenio)
        .await
        .expect("nit");
    let (s, v) = api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/tributario/tarifas-iva",
            Some(&root),
            Some(json!({ "codigo": "general", "nombre": "Tarifa general",
                         "tratamiento": "gravado", "tarifa": "19",
                         "fuente": "Estatuto Tributario, art. 468",
                         "vigente_desde": "2026-01-01" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    Tienda {
        gerente: api.token("gerente").await,
        root,
        id: entidad,
        api,
    }
}

/// Cliente con correo de facturación, perfil tributario confirmado y resolución de numeración:
/// lo mínimo para emitir. Devuelve el id del cliente.
async fn facturador(t: &Tienda) -> String {
    let cliente = id(&t
        .crear(
            "/terceros",
            json!({ "tipo": "juridica", "nombre": "Cliente SAS", "documento": "800111222",
                    "fiscal": { "regimen_iva": "responsable", "responsabilidades": [],
                                "correo_facturacion": "facturas@cliente.co" } }),
        )
        .await);
    let perfil = t
        .crear(
            "/tributario/perfiles",
            json!({ "vigente_desde": "2026-01-01", "parametros": {
                "agente_retenedor": false, "responsable_iva": true,
                "gran_contribuyente": false, "autorretenedor": false,
                "contribuyente_renta": true, "facturador_electronico": true,
                "obligado_rub": false, "municipios_ica": [], "actividades_ica": [],
                "canal_facturacion": "proveedor" } }),
        )
        .await;
    t.crear(
        &format!("/tributario/perfiles/{}/confirmacion", id(&perfil)),
        json!({}),
    )
    .await;
    t.crear(
        "/tributario/resoluciones",
        json!({ "tipo": "factura", "prefijo": "FE", "desde": 1, "hasta": 100,
                "numero_resolucion": "18764000000001", "fecha_resolucion": "2026-01-01",
                "vigente_desde": "2026-01-01", "vigente_hasta": "2027-12-31",
                "clave_tecnica": "clave-de-prueba" }),
    )
    .await;
    cliente
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_empresa_sin_contabilidad_factura_de_punta_a_punta(pool: PgPool) {
    let t = tienda(pool).await;

    // Sin contabilidad, la contabilidad no responde; la facturación sí.
    let (s, v) = t.pedir(Method::GET, "/contabilidad/cuentas", None).await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::FORBIDDEN, &json!("modulo_no_habilitado"))
    );

    // Cliente, perfil tributario y resolución: todo en las rutas comunes de la entidad.
    let cliente = facturador(&t).await;

    // Catálogo: una línea solo con el ítem toma descripción, precio e IVA.
    let item = t
        .crear(
            "/facturacion/items",
            json!({ "codigo": "ASE-H", "descripcion": "Asesoría por hora", "unidad": "HUR",
                    "precio": "100000", "tarifa_iva": "19", "clasificacion": "venta" }),
        )
        .await;
    let (s, v) = t
        .pedir(
            Method::POST,
            "/facturacion/items",
            Some(
                json!({ "codigo": "MAL", "descripcion": "Con cuenta", "clasificacion": "venta",
                         "cuenta": "4135" }),
            ),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "sin contabilidad no hay cuentas: {v}"
    );

    let nueva = || {
        json!({ "persona_id": cliente, "fecha": "2026-11-14", "vencimiento": "2026-11-30",
                "descripcion": "Asesoría", "lineas": [{ "item_id": item["id"], "cantidad": "2" }] })
    };
    let factura = t.crear("/facturacion/facturas", nueva()).await;
    assert_eq!(factura["total"], json!("238000.00"), "{factura}");
    assert_eq!(factura["lineas"][0]["cuenta_id"], Value::Null);
    let f = id(&factura);
    let emitida = t
        .crear(&format!("/facturacion/facturas/{f}/emision"), json!({}))
        .await;
    assert_eq!(emitida["numero"], json!(1));
    assert_eq!(
        emitida["comprobante_id"],
        Value::Null,
        "sin contabilidad no se contabiliza"
    );

    // Factura electrónica (proveedor simulado) y su PDF.
    let fe = t
        .crear(
            &format!("/facturacion/facturas/{f}/factura-electronica"),
            json!({}),
        )
        .await;
    assert_eq!(fe["estado"], json!("validado"), "{fe}");
    let (s, tipo, _) = t
        .api
        .pedir_texto(
            &format!(
                "/api/v1/entidades/{}/facturacion/documentos/{}/pdf",
                t.id,
                id(&fe)
            ),
            &t.gerente,
        )
        .await;
    assert_eq!((s, tipo.as_str()), (StatusCode::OK, "application/pdf"));

    // Al validarse, el adquirente recibe por correo el ZIP con el XML y el PDF, y la misma
    // entrega se puede descargar.
    let (asunto, adjunto): (String, String) = sqlx::query_as(
        "SELECT payload->>'asunto', payload->'adjuntos'->0->>'tipo' FROM outbox
          WHERE canal = 'correo' AND destino = 'facturas@cliente.co'",
    )
    .fetch_one(&t.api.duenio)
    .await
    .expect("correo al adquirente");
    assert!(asunto.contains("FE1"), "{asunto}");
    assert_eq!(adjunto, "application/zip");
    let (s, tipo, zip) = t
        .api
        .pedir_texto(
            &format!(
                "/api/v1/entidades/{}/facturacion/documentos/{}/paquete",
                t.id,
                id(&fe)
            ),
            &t.gerente,
        )
        .await;
    assert_eq!((s, tipo.as_str()), (StatusCode::OK, "application/zip"));
    assert!(zip.starts_with("PK"), "es un ZIP");

    // Pago marcado a mano.
    let pagada = t
        .crear(
            &format!("/facturacion/facturas/{f}/pago"),
            json!({ "fecha": "2026-11-20", "medio": "transferencia", "referencia": "TRX-9" }),
        )
        .await;
    assert_eq!(pagada["estado"], json!("pagada"));

    // Libro de ventas para el contador, con el CUFE y su huella.
    let (s, _, csv) = t
        .api
        .pedir_texto(
            &format!(
                "/api/v1/entidades/{}/facturacion/libro-ventas?desde=2026-11-01&hasta=2026-11-30",
                t.id
            ),
            &t.gerente,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        csv.starts_with("fecha,numero_interno,numero_dian,cufe"),
        "{csv}"
    );
    assert!(csv.contains("FE1") && csv.contains(fe["cufe"].as_str().expect("cufe")));

    // La plataforma ve el consumo de la empresa (para cobrar el módulo).
    let (s, consumo) = t
        .api
        .pedir(
            Method::GET,
            "/api/v1/plataforma/facturacion/consumo?desde=2026-01-01&hasta=2099-12-31",
            Some(&t.root),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{consumo}");
    assert_eq!(consumo[0]["entidad_id"], json!(t.id));
    assert_eq!(consumo[0]["tipo"], json!("factura"));
    assert_eq!(consumo[0]["cantidad"], json!(1));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_contabilidad_no_se_contabiliza_lo_pendiente(pool: PgPool) {
    let t = tienda(pool).await;

    let (s, v) = t
        .pedir(
            Method::POST,
            "/facturacion/contabilizacion",
            Some(json!({ "desde": "2026-01-01", "hasta": "2026-12-31" })),
        )
        .await;

    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::FORBIDDEN, &json!("modulo_no_habilitado"))
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_ningun_modulo_la_facturacion_no_responde(pool: PgPool) {
    let t = tienda(pool).await;
    sqlx::query("DELETE FROM modulos_habilitados WHERE tenant_id = $1::uuid")
        .bind(&t.id)
        .execute(&t.api.duenio)
        .await
        .expect("sin módulos");

    let (s, v) = t.pedir(Method::GET, "/facturacion/items", None).await;

    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::FORBIDDEN, &json!("modulo_no_habilitado"))
    );
}

/// Con la DIAN caída la factura queda en contingencia y nadie recibe correo; el worker la
/// reintenta y, al validarse, el adquirente recibe el ZIP.
#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_factura_en_contingencia_se_valida_al_reintentar(pool: PgPool) {
    use application::{
        Contexto,
        casos_uso::facturacion::documentos::{
            Expedicion, emitir_factura_electronica, reintentar_contingencias,
        },
        puertos::RelojSistema,
    };
    use domain::{FacturaVentaId, TenantId};
    use infra_facturacion::{EmpaquetadorZip, ProveedorSimulado};
    use infra_pdf::GeneradorTypst;
    use infra_postgres::{UnidadDeTrabajoPg, UnidadDeTrabajoPlataformaPg};
    use infra_seguridad::CofreAesGcm;

    let t = tienda(pool).await;
    let cliente = facturador(&t).await;
    let factura = t
        .crear(
            "/facturacion/facturas",
            json!({ "persona_id": cliente, "fecha": "2026-11-14", "vencimiento": "2026-11-30",
                    "descripcion": "Asesoría", "lineas": [{ "descripcion": "Asesoría",
                    "cantidad": "1", "valor_unitario": "100000" }] }),
        )
        .await;
    let f = id(&factura);
    t.crear(&format!("/facturacion/facturas/{f}/emision"), json!({}))
        .await;
    let correos = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM outbox WHERE canal = 'correo' AND destino = 'facturas@cliente.co'",
        )
        .fetch_one(&t.api.duenio)
        .await
        .expect("conteo")
    };
    let unidad = UnidadDeTrabajoPg::new(t.api.app.clone());
    let cofre = CofreAesGcm::new(None);
    let medios = |proveedor| Expedicion {
        proveedor,
        cofre: &cofre,
        generador: &GeneradorTypst,
        empaquetador: &EmpaquetadorZip,
        pie: None,
    };
    let ctx = Contexto::new(TenantId::from(uuid::Uuid::parse_str(&t.id).expect("uuid")));
    let factura_id = FacturaVentaId::from(uuid::Uuid::parse_str(&f).expect("uuid"));

    let caido = ProveedorSimulado::caido();
    let d = emitir_factura_electronica(
        &unidad,
        &medios(&caido),
        &t.api.almacen,
        &RelojSistema,
        &ctx,
        factura_id,
    )
    .await
    .expect("contingencia");
    assert_eq!(d.estado.as_str(), "contingencia");
    assert_eq!(correos().await, 0, "sin respuesta de la DIAN no se entrega");

    let disponible = ProveedorSimulado::new();
    let resumen = reintentar_contingencias(
        (
            &UnidadDeTrabajoPlataformaPg::new(t.api.app.clone()),
            &unidad,
        ),
        &medios(&disponible),
        &t.api.almacen,
        &RelojSistema,
        100,
    )
    .await
    .expect("reintento");
    assert_eq!((resumen.reintentados, resumen.validados), (1, 1));
    assert_eq!(correos().await, 1, "al validarse se entrega el ZIP");

    let (_, fe) = t
        .pedir(
            Method::GET,
            &format!("/facturacion/documentos/{}", d.id),
            None,
        )
        .await;
    assert_eq!(fe["documento"]["estado"], json!("validado"), "{fe}");
}

/// Una empresa que no es PH ve y edita su ficha en `/entidades/{id}` (nombre, NIT, contacto).
#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_empresa_ve_y_edita_su_ficha(pool: PgPool) {
    let t = tienda(pool).await;

    let (s, ficha) = t.pedir(Method::GET, "", None).await;
    assert_eq!(s, StatusCode::OK, "{ficha}");
    assert_eq!(ficha["nombre"], json!("Tienda La Esquina"));
    assert_eq!(ficha["nit"], json!("900123456-8"));
    assert_eq!(ficha["zona_horaria"], json!("America/Bogota"));

    let (s, editada) = t
        .pedir(
            Method::PATCH,
            "",
            Some(
                json!({ "razon_social": "Tienda La Esquina SAS", "telefono": "573001234567",
                         "correo": "contacto@laesquina.co", "direccion": "Calle 1 # 2-3" }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{editada}");
    assert_eq!(editada["razon_social"], json!("Tienda La Esquina SAS"));
    assert_eq!(editada["correo"], json!("contacto@laesquina.co"));
    assert_eq!(
        editada["nombre"],
        json!("Tienda La Esquina"),
        "lo omitido no cambia"
    );

    let (s, v) = t
        .pedir(Method::PATCH, "", Some(json!({ "zona_horaria": "Bogota" })))
        .await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("validacion"))
    );
    let (_, otra) = t.pedir(Method::GET, "", None).await;
    assert_eq!(
        otra["razon_social"],
        json!("Tienda La Esquina SAS"),
        "se guardó"
    );
}
