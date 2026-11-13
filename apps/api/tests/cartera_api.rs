//! Pruebas funcionales de la cartera de la PH (plan 14 fases 1–4) sobre el router real y
//! PostgreSQL: lo que haría el administrador de una copropiedad desde la web (configurar,
//! presupuestar, liquidar, recaudar y consultar).

mod comun;

use axum::http::{Method, StatusCode};
use comun::{MIGRACIONES, Ph, ph};
use serde_json::{Value, json};
use sqlx::PgPool;

async fn crear(e: &Ph, ruta: &str, cuerpo: Value) -> Value {
    let (s, v) = e.pedir(Method::POST, ruta, Some(cuerpo)).await;
    assert!(s.is_success(), "{ruta}: {s} {v}");
    v
}

async fn contable(
    e: &Ph,
    metodo: Method,
    ruta: &str,
    cuerpo: Option<Value>,
) -> (StatusCode, Value) {
    e.api
        .pedir(
            metodo,
            &format!("/api/v1/entidades/{}/contabilidad{ruta}", e.id),
            Some(&e.admin),
            cuerpo,
        )
        .await
}

/// Copropiedad con contabilidad y cartera: plan de cuentas, reglas `ph.*`, banco, un bien con
/// su dueño y cuadro de coeficientes, concepto de administración, política y presupuesto.
/// Devuelve la copropiedad, el bien y la cuenta de tesorería.
async fn copropiedad(pool: PgPool) -> (Ph, String, String) {
    let e = ph(pool).await;
    e.habilitar("contabilidad").await;
    e.habilitar("contabilidad_ph").await;
    for (codigo, movimiento, tercero) in [
        ("1", false, false),
        ("11", false, false),
        ("1110", true, false),
        ("13", false, false),
        ("1305", true, true),
        ("2", false, false),
        ("28", false, false),
        ("2805", true, false),
        ("4", false, false),
        ("41", false, false),
        ("4170", true, false),
        ("5", false, false),
        ("51", false, false),
        ("5135", true, false),
    ] {
        let (s, v) = contable(
            &e,
            Method::POST,
            "/cuentas",
            Some(
                json!({ "codigo": codigo, "nombre": format!("Cuenta {codigo}"),
                         "acepta_movimiento": movimiento, "exige_tercero": tercero }),
            ),
        )
        .await;
        assert!(s.is_success(), "{s} {v}");
    }
    for codigo in ["RC", "LQ"] {
        let (s, v) = contable(
            &e,
            Method::POST,
            "/tipos-comprobante",
            Some(json!({ "codigo": codigo, "nombre": format!("Tipo {codigo}"), "manual": false })),
        )
        .await;
        assert!(s.is_success(), "{s} {v}");
    }
    let (s, v) = contable(
        &e,
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
            "ph.liquidacion",
            "LQ",
            vec![
                linea("administracion", "debito", "1305", true),
                linea("administracion", "credito", "4170", false),
            ],
        ),
        (
            "ph.recaudo",
            "RC",
            vec![
                linea("recibido", "debito", "1110", false),
                linea("administracion", "credito", "1305", true),
                linea("sin_aplicar", "credito", "2805", false),
            ],
        ),
    ] {
        let (s, v) = contable(
            &e,
            Method::POST,
            "/reglas",
            Some(json!({ "evento": evento, "tipo_comprobante": tipo,
                         "vigente_desde": "2026-01-01", "lineas": lineas })),
        )
        .await;
        assert!(s.is_success(), "{s} {v}");
    }
    let (s, banco) = contable(
        &e,
        Method::POST,
        "/tesoreria/cuentas",
        Some(
            json!({ "tipo": "banco", "nombre": "Recaudo", "banco": "Banco de prueba",
                     "numero_cuenta": "123-456789-01", "tipo_cuenta": "ahorros",
                     "cuenta_contable": "1110" }),
        ),
    )
    .await;
    assert!(s.is_success(), "{s} {banco}");
    let (bien, _) = e.bien_con_dueno().await;
    let cuadro = crear(&e, "/cuadros-coeficientes", json!({})).await;
    let cuadro = cuadro["id"].as_str().expect("cuadro").to_owned();
    let (s, v) = e
        .pedir(
            Method::PUT,
            &format!("/cuadros-coeficientes/{cuadro}/coeficientes"),
            Some(json!({ "coeficientes": [{ "bien_privado_id": bien, "coeficiente": "1" }] })),
        )
        .await;
    assert!(s.is_success(), "{s} {v}");
    crear(
        &e,
        &format!("/cuadros-coeficientes/{cuadro}/activar"),
        json!({}),
    )
    .await;
    // El cuadro rige desde su activación (hoy): la liquidación de prueba es de diciembre.
    let concepto = crear(
        &e,
        "/cartera/conceptos",
        json!({ "codigo": "administracion", "nombre": "Administración", "tipo": "ordinaria",
                "criterio": "coeficiente", "genera_interes": true,
                "concepto_contable": "administracion" }),
    )
    .await;
    crear(
        &e,
        "/cartera/politicas",
        json!({ "vigente_desde": "2026-01-01", "dia_vencimiento": 10, "redondeo": "100",
                "factor_mora": "1.5", "imputacion": "intereses_primero",
                "porcentaje_fondo": "1" }),
    )
    .await;
    let cuenta_gasto: String = sqlx::query_scalar(
        "SELECT id::text FROM cuentas_contables WHERE tenant_id = $1::uuid AND codigo = '5135'",
    )
    .bind(&e.id)
    .fetch_one(&e.api.duenio)
    .await
    .expect("cuenta de gasto");
    let presupuesto = crear(
        &e,
        "/cartera/presupuestos",
        json!({ "anio": 2026, "rubros": [{
            "concepto_cobro_id": concepto["id"], "cuenta_contable_id": cuenta_gasto,
            "nombre": "Administración", "valores_mensuales": vec!["450000"; 12] }] }),
    )
    .await;
    assert_eq!(presupuesto["estado"], json!("borrador"));
    assert_eq!(presupuesto["total_anual"], json!("5400000.00"));
    let aprobado = crear(
        &e,
        &format!(
            "/cartera/presupuestos/{}/aprobacion",
            presupuesto["id"].as_str().expect("id")
        ),
        json!({ "vigente_desde": "2026-01-01" }),
    )
    .await;
    assert_eq!(aprobado["estado"], json!("vigente"));
    let banco = banco["id"].as_str().expect("banco").to_owned();
    (e, bien, banco)
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn liquidar_recaudar_y_consultar_la_cartera_de_punta_a_punta(pool: PgPool) {
    let (e, bien, banco) = copropiedad(pool).await;

    let vista = crear(
        &e,
        "/cartera/liquidaciones/vista-previa",
        json!({ "periodo": "2026-12-01" }),
    )
    .await;
    assert_eq!(vista["total"], json!("450000.00"));

    let liquidado = crear(
        &e,
        "/cartera/liquidaciones",
        json!({ "periodo": "2026-12-15" }),
    )
    .await;
    assert_eq!(liquidado["liquidacion"]["periodo"], json!("2026-12-01"));
    assert_eq!(
        liquidado["cuentas_cobro"][0]["numero"],
        json!("CB-2026-00001")
    );
    let (s, _) = e
        .pedir(
            Method::POST,
            "/cartera/liquidaciones",
            Some(json!({ "periodo": "2026-12-01" })),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "reliquidar no duplica");

    let cuenta = liquidado["cuentas_cobro"][0]["id"]
        .as_str()
        .expect("cuenta");
    let (s, tipo, _) = e
        .api
        .pedir_texto(
            &format!("{}/cartera/cuentas-cobro/{cuenta}/pdf", e.base),
            &e.admin,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(tipo, "application/pdf");

    let recaudo = crear(
        &e,
        "/cartera/recaudos",
        json!({ "bien_privado_id": bien, "cuenta_tesoreria_id": banco, "fecha": "2026-12-05",
                "valor": "500000", "medio": "transferencia" }),
    )
    .await;
    assert_eq!(recaudo["anticipo"], json!("50000.00"));
    assert!(recaudo["recibo"]["aplicaciones"][0]["obligacion_id"].is_string());

    let (s, estado) = e
        .pedir(
            Method::GET,
            &format!("/cartera/bienes/{bien}/estado-cuenta?corte=2026-12-31"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{estado}");
    assert_eq!(estado["saldo"], json!("0.00"));
    assert_eq!(estado["anticipos"], json!("50000.00"));
    let (s, cuadre) = e.pedir(Method::GET, "/cartera/cuadre", None).await;
    assert_eq!(s, StatusCode::OK, "{cuadre}");
    assert_eq!(cuadre[0]["diferencia"], json!("0.00"));
    let (s, saldos) = e.pedir(Method::GET, "/cartera/saldos", None).await;
    assert_eq!(s, StatusCode::OK, "{saldos}");
    assert_eq!(saldos, json!([]));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_modulo_de_cartera_responde_403(pool: PgPool) {
    let e = ph(pool).await;
    e.habilitar("contabilidad").await;

    let (s, v) = e.pedir(Method::GET, "/cartera/conceptos", None).await;

    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"]["codigo"], json!("modulo_no_habilitado"));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_politica_que_supera_el_tope_de_mora_se_rechaza(pool: PgPool) {
    let (e, _, _) = copropiedad(pool).await;

    let (s, v) = e
        .pedir(
            Method::POST,
            "/cartera/politicas",
            Some(
                json!({ "vigente_desde": "2027-01-01", "dia_vencimiento": 10, "redondeo": "1",
                         "factor_mora": "1.6", "imputacion": "intereses_primero",
                         "porcentaje_fondo": "1" }),
            ),
        )
        .await;

    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}
