//! Pruebas funcionales de tesorería, cuentas por pagar, facturas de venta y conciliación
//! (plan 13) sobre el router real y PostgreSQL: lo que harían la plataforma, el contador y el
//! tesorero de una empresa cualquiera desde la web.

mod comun;

use axum::http::{Method, StatusCode};
use base64::{Engine, engine::general_purpose::STANDARD};
use comun::{Api, MIGRACIONES, api};
use serde_json::{Value, json};
use sqlx::PgPool;

struct Empresa {
    api: Api,
    gerente: String,
    contador: String,
    id: String,
    banco: String,
    cliente: String,
    proveedor: String,
}

impl Empresa {
    async fn pedir_como(
        &self,
        token: &str,
        metodo: Method,
        sufijo: &str,
        cuerpo: Option<Value>,
    ) -> (StatusCode, Value) {
        self.api
            .pedir(
                metodo,
                &format!("/api/v1/entidades/{}/contabilidad{sufijo}", self.id),
                Some(token),
                cuerpo,
            )
            .await
    }

    async fn pedir(
        &self,
        metodo: Method,
        sufijo: &str,
        cuerpo: Option<Value>,
    ) -> (StatusCode, Value) {
        self.pedir_como(&self.gerente.clone(), metodo, sufijo, cuerpo)
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

/// Empresa con contabilidad, plan de cuentas, reglas del plan 13, una cuenta bancaria, un
/// cliente, un proveedor, el concepto de retención de servicios y un contador (otra cuenta con
/// rol de administrador, para autorizar egresos).
async fn empresa(pool: PgPool) -> Empresa {
    let api = api(pool).await;
    let root_id = api.registrar("root").await;
    api.hacer_superusuario(&root_id).await;
    let gerente_id = api.registrar("gerente").await;
    let contador_id = api.registrar("contador").await;
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
    let (s, v) = api
        .pedir(
            Method::POST,
            &format!("/api/v1/plataforma/copropiedades/{entidad}/modulos"),
            Some(&root),
            Some(json!({ "modulo": "contabilidad" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    // Los roles de una entidad sin vertical aún no tienen rutas propias (deuda del plan 12).
    sqlx::query(
        "INSERT INTO asignaciones_rol (id, tenant_id, cuenta_id, rol_id, vigente_desde, origen)
         VALUES (uuidv7(), $1::uuid, $2::uuid, uuid_rol_plantilla('administrador'), now(), 'manual')",
    )
    .bind(&entidad)
    .bind(&contador_id)
    .execute(&api.duenio)
    .await
    .expect("rol del contador");
    let (s, v) = api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/retenciones",
            Some(&root),
            Some(json!({
                "codigo": "servicios",
                "impuesto": "renta",
                "nombre": "Servicios en general",
                "base_minima_uvt": "4",
                "tarifa": "4",
                "fuente": "Catálogo de prueba",
                "vigente_desde": "2026-01-01",
            })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    let mut e = Empresa {
        gerente: api.token("gerente").await,
        contador: api.token("contador").await,
        id: entidad,
        api,
        banco: String::new(),
        cliente: String::new(),
        proveedor: String::new(),
    };
    for (codigo, movimiento, tercero) in [
        ("1", false, false),
        ("11", false, false),
        ("1110", true, false),
        ("13", false, false),
        ("1305", true, true),
        ("2", false, false),
        ("23", false, false),
        ("2335", true, true),
        ("2365", true, true),
        ("28", false, false),
        ("2805", true, false),
        ("4", false, false),
        ("41", false, false),
        ("4170", true, false),
        ("5", false, false),
        ("51", false, false),
        ("5135", true, false),
        ("5195", true, false),
    ] {
        e.crear(
            "/cuentas",
            json!({ "codigo": codigo, "nombre": format!("Cuenta {codigo}"),
                    "acepta_movimiento": movimiento, "exige_tercero": tercero }),
        )
        .await;
    }
    for codigo in ["RC", "CE", "FC", "FV"] {
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
            "tesoreria.recibo",
            "RC",
            vec![
                linea("recibido", "debito", "1110", false),
                linea("aplicado", "credito", "1305", true),
                linea("sin_aplicar", "credito", "2805", false),
            ],
        ),
        (
            "tesoreria.egreso",
            "CE",
            vec![
                linea("pagado_facturas", "debito", "2335", true),
                linea("gasto", "debito", "5195", false),
                linea("girado", "credito", "1110", false),
            ],
        ),
        (
            "cxp.factura",
            "FC",
            vec![
                linea("gasto", "debito", "5135", false),
                linea("retencion_renta", "credito", "2365", true),
                linea("por_pagar", "credito", "2335", true),
            ],
        ),
        (
            "ventas.factura",
            "FV",
            vec![
                linea("por_cobrar", "debito", "1305", true),
                linea("ingreso", "credito", "4170", false),
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
    let banco = e
        .crear(
            "/tesoreria/cuentas",
            json!({ "tipo": "banco", "nombre": "Corriente", "banco": "Banco de prueba",
                    "numero_cuenta": "123-456789-01", "tipo_cuenta": "corriente",
                    "cuenta_contable": "1110" }),
        )
        .await;
    assert_eq!(
        banco["numero_enmascarado"],
        json!("****8901"),
        "el número no se guarda completo"
    );
    e.banco = id(&banco);
    e.cliente = id(&e
        .crear(
            "/terceros",
            json!({ "tipo": "juridica", "nombre": "Cliente S.A.S.", "documento": "800111222" }),
        )
        .await);
    e.proveedor = id(&e
        .crear(
            "/terceros",
            json!({ "tipo": "juridica", "nombre": "Proveedor S.A.S.", "documento": "900555666",
                    "fiscal": { "regimen_iva": "responsable", "correo_facturacion": "fe@proveedor.co" } }),
        )
        .await);
    e
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn venta_recaudo_compra_y_pago_de_punta_a_punta(pool: PgPool) {
    let e = empresa(pool).await;

    // Venta: borrador → emitir → recibo aplicado.
    let factura = e
        .crear(
            "/ventas/facturas",
            json!({ "persona_id": e.cliente, "fecha": "2026-03-01", "vencimiento": "2026-03-31",
                    "descripcion": "Alquiler del salón",
                    "lineas": [{ "descripcion": "Salón", "cuenta": "4170", "cantidad": "1",
                                 "valor_unitario": "350000" }] }),
        )
        .await;
    assert_eq!(factura["estado"], json!("borrador"));
    let factura_id = id(&factura);
    let (s, emitida) = e
        .pedir(
            Method::POST,
            &format!("/ventas/facturas/{factura_id}/emision"),
            None,
        )
        .await;
    assert_eq!(
        (s, &emitida["numero"]),
        (StatusCode::OK, &json!(1)),
        "{emitida}"
    );
    let recibo = e
        .crear(
            "/tesoreria/recibos",
            json!({ "cuenta_tesoreria_id": e.banco, "persona_id": e.cliente, "fecha": "2026-03-10",
                    "valor": "350000", "medio": "transferencia", "descripcion": "Pago salón",
                    "aplicaciones": [{ "factura_venta_id": factura_id, "valor": "350000" }] }),
        )
        .await;
    assert_eq!(recibo["sin_aplicar"], json!("0.00"));
    let (_, f) = e
        .pedir(Method::GET, &format!("/ventas/facturas/{factura_id}"), None)
        .await;
    assert_eq!(f["estado"], json!("pagada"));
    let (_, cartera) = e
        .pedir(Method::GET, "/ventas/cartera?corte=2026-04-30", None)
        .await;
    assert_eq!(cartera, json!([]));

    // Compra: registrar (con retención) → duplicado rechazado → causar.
    let nueva = json!({ "persona_id": e.proveedor, "prefijo": "FE", "numero": "77",
                        "fecha": "2026-03-05", "vencimiento": "2026-04-05",
                        "descripcion": "Mantenimiento",
                        "lineas": [{ "descripcion": "Servicio", "cuenta": "5135",
                                     "base": "1000000", "iva": "0",
                                     "retenciones": ["servicios"] }] });
    let compra = e.crear("/cuentas-por-pagar/facturas", nueva.clone()).await;
    assert_eq!(compra["retenciones"], json!("40000.00"));
    assert_eq!(compra["neto_pagar"], json!("960000.00"));
    let (s, v) = e
        .pedir(Method::POST, "/cuentas-por-pagar/facturas", Some(nueva))
        .await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::CONFLICT, &json!("conflicto"))
    );
    let compra_id = id(&compra);
    let (s, v) = e
        .pedir(
            Method::POST,
            &format!("/cuentas-por-pagar/facturas/{compra_id}/causacion"),
            None,
        )
        .await;
    assert_eq!(
        (s, &v["estado"]),
        (StatusCode::OK, &json!("causada")),
        "{v}"
    );

    // Egreso: lo registra el gerente, no puede autorizarlo él mismo; lo autoriza el contador.
    let egreso = e
        .crear(
            "/tesoreria/egresos",
            json!({ "cuenta_tesoreria_id": e.banco, "persona_id": e.proveedor,
                    "fecha": "2026-03-20", "medio": "transferencia", "descripcion": "Pago FE-77",
                    "facturas": [{ "factura_proveedor_id": compra_id, "valor": "960000" }] }),
        )
        .await;
    let egreso_id = id(&egreso);
    let (s, v) = e
        .pedir(
            Method::POST,
            &format!("/tesoreria/egresos/{egreso_id}/autorizacion"),
            None,
        )
        .await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::CONFLICT, &json!("conflicto"))
    );
    let (s, v) = e
        .pedir_como(
            &e.contador,
            Method::POST,
            &format!("/tesoreria/egresos/{egreso_id}/autorizacion"),
            None,
        )
        .await;
    assert_eq!(
        (s, &v["estado"]),
        (StatusCode::OK, &json!("autorizado")),
        "{v}"
    );
    let (s, v) = e
        .pedir(
            Method::POST,
            &format!("/tesoreria/egresos/{egreso_id}/pago"),
            Some(json!({})),
        )
        .await;
    assert_eq!((s, &v["estado"]), (StatusCode::OK, &json!("pagado")), "{v}");
    let (_, compra) = e
        .pedir(
            Method::GET,
            &format!("/cuentas-por-pagar/facturas/{compra_id}"),
            None,
        )
        .await;
    assert_eq!(compra["estado"], json!("pagada"));

    // Certificado de retenciones y su envío por correo.
    let (s, cert) = e
        .pedir(
            Method::GET,
            &format!(
                "/cuentas-por-pagar/certificados/{}?desde=2026-01-01&hasta=2026-12-31",
                e.proveedor
            ),
            None,
        )
        .await;
    assert_eq!(
        (s, &cert["total_retenido"]),
        (StatusCode::OK, &json!("40000.00")),
        "{cert}"
    );
    let (s, envio) = e
        .pedir(
            Method::POST,
            &format!(
                "/cuentas-por-pagar/certificados/{}/envio?desde=2026-01-01&hasta=2026-12-31",
                e.proveedor
            ),
            None,
        )
        .await;
    assert_eq!(
        (s, &envio["para"]),
        (StatusCode::ACCEPTED, &json!("fe@proveedor.co"))
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn importa_el_extracto_concilia_y_el_informe_cuadra(pool: PgPool) {
    let e = empresa(pool).await;
    e.crear(
        "/tesoreria/recibos",
        json!({ "cuenta_tesoreria_id": e.banco, "persona_id": e.cliente, "fecha": "2026-03-10",
                "valor": "200000", "medio": "consignacion", "descripcion": "Anticipo" }),
    )
    .await;
    let csv = "Fecha;Detalle;Valor\n10/03/2026;Consignación;200.000,00\n12/03/2026;Abono sin identificar;50.000,00\n";
    let peticion = json!({
        "nombre_archivo": "marzo.csv",
        "archivo_base64": STANDARD.encode(csv),
        "formato": "planilla",
        "mapeo": { "fecha": "Fecha", "formato_fecha": "%d/%m/%Y", "descripcion": "Detalle",
                   "valor": "Valor", "separador_decimal": "coma" },
        "saldo_final": "250000",
    });
    let ruta = format!("/tesoreria/cuentas/{}/extractos", e.banco);

    let (s, previa) = e
        .pedir(
            Method::POST,
            &format!("{ruta}/vista-previa"),
            Some(peticion.clone()),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{previa}");
    assert_eq!(previa["movimientos"].as_array().map(Vec::len), Some(2));
    let extracto = e.crear(&ruta, peticion.clone()).await;
    assert_eq!(extracto["movimientos"], json!(2));
    let (s, v) = e.pedir(Method::POST, &ruta, Some(peticion)).await;
    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::CONFLICT, &json!("conflicto")),
        "no se importa dos veces"
    );

    let (s, hechas) = e
        .pedir(
            Method::POST,
            &format!("/tesoreria/cuentas/{}/conciliacion-automatica", e.banco),
            Some(json!({ "desde": "2026-03-01", "hasta": "2026-03-31" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{hechas}");
    assert_eq!(hechas.as_array().map(Vec::len), Some(1));
    let (_, pendientes) = e
        .pedir(
            Method::GET,
            &format!("/tesoreria/cuentas/{}/movimientos?desde=2026-03-01&hasta=2026-03-31&estado=pendiente", e.banco),
            None,
        )
        .await;
    let movimiento = id(&pendientes[0]);
    let (s, recibo) = e
        .pedir(
            Method::POST,
            &format!("/tesoreria/movimientos/{movimiento}/recibo"),
            Some(json!({ "descripcion": "Abono por identificar" })),
        )
        .await;
    assert_eq!(
        (s, &recibo["persona_id"]),
        (StatusCode::CREATED, &Value::Null),
        "{recibo}"
    );
    let (s, informe) = e
        .pedir(
            Method::GET,
            &format!(
                "/tesoreria/cuentas/{}/informe-conciliacion?corte=2026-03-31",
                e.banco
            ),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{informe}");
    assert_eq!(informe["saldo_libros"], json!("250000.00"));
    assert_eq!(informe["diferencia"], json!("0.00"));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_permiso_de_tesoreria_no_registra_recibos(pool: PgPool) {
    let e = empresa(pool).await;
    let lector_id = e.api.registrar("lector").await;
    sqlx::query(
        "INSERT INTO asignaciones_rol (id, tenant_id, cuenta_id, rol_id, vigente_desde, origen)
         VALUES (uuidv7(), $1::uuid, $2::uuid, uuid_rol_plantilla('consejero'), now(), 'manual')",
    )
    .bind(&e.id)
    .bind(&lector_id)
    .execute(&e.api.duenio)
    .await
    .expect("rol");
    let lector = e.api.token("lector").await;

    let (s, v) = e
        .pedir_como(
            &lector,
            Method::POST,
            "/tesoreria/recibos",
            Some(
                json!({ "cuenta_tesoreria_id": e.banco, "fecha": "2026-03-10", "valor": "1",
                         "medio": "efectivo", "descripcion": "x" }),
            ),
        )
        .await;
    let (s_ver, _) = e
        .pedir_como(&lector, Method::GET, "/tesoreria/recibos", None)
        .await;

    assert_eq!(
        (s, &v["error"]["codigo"]),
        (StatusCode::FORBIDDEN, &json!("sin_permiso"))
    );
    assert_eq!(s_ver, StatusCode::OK, "con `contabilidad.ver` sí consulta");
}
