//! Funcionales del cobro de la plataforma (plan 19): catálogo → suscripción → corte →
//! aprobación → factura en la entidad de la plataforma → pago; lo que ve la entidad; permisos
//! de la consola; la entidad suspendida queda en solo lectura; y el anunciante se factura con el
//! mismo facturador.

use crate::comun;

use axum::http::{Method, StatusCode};
use comun::{MIGRACIONES, Ph, ph};
use serde_json::{Value, json};
use sqlx::PgPool;

const COBROS: &str = "/api/v1/plataforma/cobros";

fn id(v: &Value) -> String {
    v["id"].as_str().expect("id").to_owned()
}

impl Ph {
    async fn consola(
        &self,
        metodo: Method,
        ruta: &str,
        cuerpo: Option<Value>,
    ) -> (StatusCode, Value) {
        self.api
            .pedir(metodo, &format!("{COBROS}{ruta}"), Some(&self.root), cuerpo)
            .await
    }
}

/// La entidad de la plataforma (la que factura), con el módulo de facturación y NIT.
async fn facturador_de_la_plataforma(p: &Ph) -> String {
    let gisla = p.api.registrar("gisla").await;
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            "/api/v1/plataforma/entidades",
            Some(&p.root),
            Some(json!({ "nombre": "GISLA SAS", "administrador_cuenta_id": gisla })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let entidad = id(&v);
    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("/api/v1/plataforma/copropiedades/{entidad}/modulos"),
            Some(&p.root),
            Some(json!({ "modulo": "facturacion_electronica" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    sqlx::query(
        "UPDATE entidades SET nit = '900123456-8', es_plataforma = true WHERE id = $1::uuid",
    )
    .bind(&entidad)
    .execute(&p.api.duenio)
    .await
    .expect("plataforma");
    entidad
}

/// Producto plano de $100.000 al mes y la suscripción mensual de la PH desde enero.
async fn suscripcion_de_la_ph(p: &Ph) -> String {
    let (s, producto) = p
        .consola(
            Method::POST,
            "/productos",
            Some(
                json!({ "codigo": "asambleas", "nombre": "Asambleas y votaciones",
                         "descripcion_factura": "{producto} — {periodo}",
                         "modulo": "asambleas" }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{producto}");
    let (s, v) = p
        .consola(
            Method::POST,
            &format!("/productos/{}/tarifas", id(&producto)),
            Some(json!({ "modelo": "plana", "valor": "100000", "vigente_desde": "2026-01-01" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (s, suscripcion) = p
        .consola(
            Method::POST,
            "/suscripciones",
            Some(
                json!({ "entidad_id": p.id, "ciclo_meses": 1, "inicio": "2026-01-01",
                         "correo_facturacion": "pagos@alameda.co" }),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{suscripcion}");
    assert_eq!(suscripcion["plazo_pago_dias"], 15);
    let (s, v) = p
        .consola(
            Method::POST,
            &format!("/suscripciones/{}/lineas", id(&suscripcion)),
            Some(json!({ "producto_id": id(&producto), "desde": "2026-01-01" })),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["producto"], "asambleas");
    id(&suscripcion)
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_corte_se_aprueba_factura_y_paga(pool: PgPool) {
    let p = ph(pool).await;
    let gisla = facturador_de_la_plataforma(&p).await;
    let suscripcion = suscripcion_de_la_ph(&p).await;

    let (s, corte) = p
        .consola(
            Method::POST,
            "/cortes",
            Some(json!({ "suscripcion_id": suscripcion, "desde": "2026-01-01" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corte}");
    assert_eq!(corte["estado"], "borrador");
    assert_eq!(corte["total"], "100000");
    assert_eq!(
        corte["lineas"][0]["descripcion"],
        "Asambleas y votaciones — 2026-01-01 a 2026-01-31"
    );
    let corte_id = id(&corte);
    let (s, v) = p
        .consola(Method::POST, &format!("/cortes/{corte_id}/aprobar"), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = p
        .consola(Method::POST, &format!("/cortes/{corte_id}/facturar"), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["estado"], "facturado");
    let factura = v["factura_venta_id"].as_str().expect("factura").to_owned();
    let (en_gisla, total): (String, rust_decimal::Decimal) =
        sqlx::query_as("SELECT tenant_id::text, total FROM facturas_venta WHERE id = $1::uuid")
            .bind(&factura)
            .fetch_one(&p.api.duenio)
            .await
            .expect("factura de venta");
    assert_eq!(en_gisla, gisla, "factura la entidad de la plataforma");
    assert_eq!(total, rust_decimal::Decimal::from(100_000));
    let (s, v) = p
        .consola(Method::POST, &format!("/cortes/{corte_id}/facturar"), None)
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "no se factura dos veces: {v}");

    let (s, v) = p
        .consola(
            Method::POST,
            &format!("/cortes/{corte_id}/pago"),
            Some(json!({ "fecha": "2026-02-10" })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["estado"], "pagado");

    let (s, mia) = p.pedir_entidad(Method::GET, "/suscripcion", None).await;
    assert_eq!(s, StatusCode::OK, "{mia}");
    assert_eq!(mia["lineas"][0]["producto"], "asambleas");
    assert_eq!(mia["cortes"][0]["estado"], "pagado");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn mientras_no_factura_la_plataforma_se_marca_la_factura_externa(pool: PgPool) {
    let p = ph(pool).await;
    let suscripcion = suscripcion_de_la_ph(&p).await;
    let (_, corte) = p
        .consola(
            Method::POST,
            "/cortes",
            Some(json!({ "suscripcion_id": suscripcion, "desde": "2026-01-01" })),
        )
        .await;
    let corte_id = id(&corte);

    let (s, v) = p
        .consola(
            Method::POST,
            &format!("/cortes/{corte_id}/factura-externa"),
            Some(json!({ "numero": "FE-1203", "fecha": "2026-02-01" })),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "solo un corte aprobado se factura: {v}"
    );
    p.consola(Method::POST, &format!("/cortes/{corte_id}/aprobar"), None)
        .await;
    let (s, v) = p
        .consola(Method::POST, &format!("/cortes/{corte_id}/facturar"), None)
        .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "sin entidad de la plataforma: {v}"
    );
    let (s, v) = p
        .consola(
            Method::POST,
            &format!("/cortes/{corte_id}/factura-externa"),
            Some(json!({ "numero": "FE-1203", "fecha": "2026-02-01" })),
        )
        .await;

    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["factura_externa"], "FE-1203");
    assert_eq!(v["vencimiento"], "2026-02-16");
    let (s, v) = p
        .consola(Method::GET, "/cortes?estado=facturado", None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["datos"].as_array().map(Vec::len), Some(1));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_consola_reparte_los_permisos_por_rol(pool: PgPool) {
    let p = ph(pool).await;
    let suscripcion = suscripcion_de_la_ph(&p).await;
    let comercial = p.api.operador_con_rol("vendedor", "comercial").await;
    let (_, corte) = p
        .consola(
            Method::POST,
            "/cortes",
            Some(json!({ "suscripcion_id": suscripcion, "desde": "2026-01-01" })),
        )
        .await;

    let (s, v) = p
        .api
        .pedir(
            Method::POST,
            &format!("{COBROS}/cortes/{}/aprobar", id(&corte)),
            Some(&comercial),
            None,
        )
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "el comercial no aprueba cortes: {v}"
    );
    let (s, v) = p
        .api
        .pedir(
            Method::GET,
            &format!("{COBROS}/productos"),
            Some(&comercial),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "el comercial gestiona el catálogo: {v}");
    let (s, v) = p
        .api
        .pedir(
            Method::GET,
            &format!("{COBROS}/productos"),
            Some(&p.admin),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "la entidad no ve la consola: {v}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_entidad_suspendida_solo_lee(pool: PgPool) {
    let p = ph(pool).await;
    let (s, v) = p.estado("activa", &p.root).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = p.estado("suspendida", &p.root).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (s, caps) = p.pedir_entidad(Method::GET, "/capacidades", None).await;
    assert_eq!(s, StatusCode::OK, "{caps}");
    assert_eq!(caps["entidad_suspendida"], true);
    let (s, v) = p
        .pedir(
            Method::POST,
            "/bienes-privados",
            Some(json!({ "tipo": "apartamento", "numero": "101" })),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "suspendida no escribe: {v}");
    let (s, v) = p.pedir(Method::GET, "/bienes-privados", None).await;
    assert_eq!(s, StatusCode::OK, "suspendida sí lee: {v}");

    let (s, v) = p.estado("activa", &p.root).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, caps) = p.pedir_entidad(Method::GET, "/capacidades", None).await;
    assert_eq!(caps["entidad_suspendida"], false);
    p.crear_bien(json!({ "tipo": "apartamento", "numero": "101" }))
        .await;
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_anunciante_se_cobra_con_el_mismo_facturador(pool: PgPool) {
    let p = ph(pool).await;
    let gisla = facturador_de_la_plataforma(&p).await;
    let liquidacion: String = sqlx::query_scalar(
        "WITH o AS (
             INSERT INTO organizaciones (id, nit, razon_social, naturaleza, tipos, correo_contacto)
             VALUES (gen_random_uuid(), '901234567-1', 'Pizzería Don Luigi SAS', 'juridica',
                     ARRAY['anunciante'], 'pagos@luigi.co') RETURNING id),
         a AS (
             INSERT INTO anunciantes (id, organizacion_id, nombre_comercial, estado_verificacion,
                                      creado_en, actualizado_en)
             SELECT gen_random_uuid(), o.id, 'Don Luigi', 'verificado', now(), now() FROM o
             RETURNING id)
         INSERT INTO liquidaciones (id, anunciante_id, periodo, variable, fijos, leads, valor_leads,
                                    valor, costo_whatsapp, estado, calculada_en, actualizada_en)
         SELECT gen_random_uuid(), a.id, '2026-01-01', 30000, 0, 0, 0, 30000, 0, 'calculada',
                now(), now() FROM a
         RETURNING id::text",
    )
    .fetch_one(&p.api.duenio)
    .await
    .expect("liquidación");

    let (s, corte) = p
        .consola(
            Method::POST,
            &format!("/liquidaciones/{liquidacion}/corte"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{corte}");
    assert_eq!(corte["liquidacion_id"], liquidacion.as_str());
    let corte_id = id(&corte);
    p.consola(Method::POST, &format!("/cortes/{corte_id}/aprobar"), None)
        .await;
    let (s, v) = p
        .consola(Method::POST, &format!("/cortes/{corte_id}/facturar"), None)
        .await;

    assert_eq!(s, StatusCode::OK, "{v}");
    let (en, cliente): (String, String) = sqlx::query_as(
        "SELECT f.tenant_id::text, pe.nombre FROM facturas_venta f
         JOIN personas pe ON pe.tenant_id = f.tenant_id AND pe.id = f.persona_id
         WHERE f.id = $1::uuid",
    )
    .bind(v["factura_venta_id"].as_str().expect("factura"))
    .fetch_one(&p.api.duenio)
    .await
    .expect("factura");
    assert_eq!(en, gisla);
    assert_eq!(cliente, "Pizzería Don Luigi SAS");
}
