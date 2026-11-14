//! Garantías del esquema de la facturación independiente (plan 18, ADR 0022): una entidad sin
//! contabilidad emite facturas sin cuenta ni comprobante, la marca de pago manual solo cabe en
//! una factura pagada y sin contabilizar, y el catálogo de productos y servicios queda aislado
//! por entidad.
//!
//! Las primeras pruebas siembran como dueño del esquema (sin RLS) y operan como `dad_app`
//! dentro de `transaccion_tenant`, igual que en producción; el SQL dinámico (`AssertSqlSafe`)
//! solo interpola UUIDs generados por la propia prueba. Las de los casos de uso recorren el
//! flujo de una empresa que **solo contrató la facturación** (sin contabilidad).

mod comun;

use application::{
    AppError, Contexto,
    casos_uso::{
        contabilidad::{
            configuracion::{self as conf, NuevoTercero},
            cuentas_por_pagar::{self as cxp, NuevaFacturaProveedor, NuevaLineaFacturaProveedor},
        },
        facturacion::{
            items::{self, DatosItem},
            ventas::{self, NuevaFacturaVenta, NuevaLineaVenta, NuevoPagoManual},
        },
    },
    puertos::{Reloj, contabilidad::DatosFiscales, facturacion::FiltroItems},
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use comun::{MIGRACIONES, insertar_entidad, pool_como_app};
use domain::{
    Ambito, ConjuntoPermisos, CuentaId, Documento, Modulo, PersonaId, TenantId, TipoPersona,
    contabilidad::{CodigoCuenta, Monto, tesoreria::EstadoFacturaVenta},
    facturacion::MedioPagoManual,
    tributario::ClasificacionTributaria,
};
use infra_postgres::{UnidadDeTrabajoPg, transaccion_tenant};
use rust_decimal::Decimal;
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

/// Una empresa sin vertical ni contabilidad, con un cliente.
struct Empresa {
    tenant: TenantId,
    cliente: Uuid,
}

async fn sembrar(pool: &PgPool) -> Empresa {
    let e = Empresa {
        tenant: TenantId::nuevo(),
        cliente: Uuid::now_v7(),
    };
    let (t, c) = (e.tenant.as_uuid(), e.cliente);
    sqlx::raw_sql(AssertSqlSafe(format!(
        "INSERT INTO entidades (id, nombre, estado) VALUES ('{t}', 'Tienda {t}', 'activa');
         INSERT INTO personas (id, tenant_id, nombre) VALUES ('{c}', '{t}', 'Cliente');"
    )))
    .execute(pool)
    .await
    .expect("siembra");
    e
}

/// Ejecuta `sql` como `dad_app` en la transacción del tenant y la confirma.
async fn como_app(pool: &PgPool, tenant: TenantId, sql: String) -> Result<(), sqlx::Error> {
    let mut tx = transaccion_tenant(pool, tenant).await?;
    sqlx::query("SET LOCAL ROLE dad_app")
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut *tx).await?;
    tx.commit().await
}

/// Factura de 100 000 en el estado dado, con número y emisión si no es borrador, y una línea
/// sin cuenta contable.
fn factura(e: &Empresa, id: Uuid, estado: &str, extra: &str) -> String {
    let (t, c) = (e.tenant.as_uuid(), e.cliente);
    let (numero, emitida) = if estado == "borrador" {
        ("NULL", "NULL")
    } else {
        ("1", "now()")
    };
    let pagado = if estado == "pagada" { "100000" } else { "0" };
    format!(
        "INSERT INTO facturas_venta (id, tenant_id, persona_id, numero, fecha, vencimiento,
             descripcion, subtotal, iva, total, pagado, estado, creada_en, emitida_en{cols})
         VALUES ('{id}', '{t}', '{c}', {numero}, '2026-11-14', '2026-11-30', 'Venta', 100000, 0,
                 100000, {pagado}, '{estado}', now(), {emitida}{vals});
         INSERT INTO lineas_factura_venta (tenant_id, factura_id, orden, descripcion, cantidad,
             valor_unitario, subtotal, iva)
         VALUES ('{t}', '{id}', 1, 'Servicio', 1, 100000, 100000, 0);",
        cols = if extra.is_empty() {
            ""
        } else {
            ", pago_manual_fecha, pago_manual_medio, pago_manual_por"
        },
        vals = extra,
    )
}

fn es_violacion_de_check(r: Result<(), sqlx::Error>) -> bool {
    matches!(r, Err(sqlx::Error::Database(d)) if d.code().as_deref() == Some("23514"))
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_factura_emitida_sin_contabilidad_no_exige_cuenta_ni_comprobante(pool: PgPool) {
    let e = sembrar(&pool).await;

    let r = como_app(&pool, e.tenant, factura(&e, Uuid::now_v7(), "emitida", "")).await;

    assert!(r.is_ok(), "{r:?}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_factura_emitida_exige_su_numero(pool: PgPool) {
    let e = sembrar(&pool).await;
    let sql = factura(&e, Uuid::now_v7(), "emitida", "").replacen(
        ", 1, '2026-11-14'",
        ", NULL, '2026-11-14'",
        1,
    );

    let r = como_app(&pool, e.tenant, sql).await;

    assert!(es_violacion_de_check(r), "sin número no se emite");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_pago_manual_solo_cabe_en_una_factura_pagada(pool: PgPool) {
    let e = sembrar(&pool).await;
    let pago = format!(", '2026-11-20', 'transferencia', '{}'", Uuid::now_v7());

    let emitida = como_app(
        &pool,
        e.tenant,
        factura(&e, Uuid::now_v7(), "emitida", &pago),
    )
    .await;
    let pagada = como_app(
        &pool,
        e.tenant,
        factura(&e, Uuid::now_v7(), "pagada", &pago),
    )
    .await;

    assert!(
        es_violacion_de_check(emitida),
        "una emitida no lleva pago manual"
    );
    assert!(pagada.is_ok(), "{pagada:?}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_pago_manual_va_completo(pool: PgPool) {
    let e = sembrar(&pool).await;
    let id = Uuid::now_v7();
    let sql = format!(
        "{} UPDATE facturas_venta SET pago_manual_fecha = '2026-11-20' WHERE id = '{id}';",
        factura(&e, id, "pagada", "")
    );

    let r = como_app(&pool, e.tenant, sql).await;

    assert!(es_violacion_de_check(r), "fecha sin medio ni responsable");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_numero_interno_de_la_factura_usa_los_consecutivos(pool: PgPool) {
    let e = sembrar(&pool).await;
    let t = e.tenant.as_uuid();

    let r = como_app(
        &pool,
        e.tenant,
        format!("INSERT INTO consecutivos (tenant_id, serie, anio, ultimo) VALUES ('{t}', 'factura_venta', 2026, 1);"),
    )
    .await;

    assert!(r.is_ok(), "{r:?}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_catalogo_de_items_queda_aislado_por_entidad(pool: PgPool) {
    let a = sembrar(&pool).await;
    let b = sembrar(&pool).await;
    for (e, codigo) in [(&a, "SRV-1"), (&b, "SRV-1")] {
        let t = e.tenant.as_uuid();
        como_app(
            &pool,
            e.tenant,
            format!(
                "INSERT INTO items_facturacion (id, tenant_id, codigo, descripcion, creado_en,
                     actualizado_en)
                 VALUES ('{}', '{t}', '{codigo}', 'Asesoría', now(), now());",
                Uuid::now_v7()
            ),
        )
        .await
        .expect("el mismo código sirve en otra entidad");
    }

    let mut tx = transaccion_tenant(&pool, a.tenant).await.expect("tx");
    sqlx::query("SET LOCAL ROLE dad_app")
        .execute(&mut *tx)
        .await
        .expect("rol");
    let visibles: i64 = sqlx::query_scalar("SELECT count(*) FROM items_facturacion")
        .fetch_one(&mut *tx)
        .await
        .expect("conteo");

    assert_eq!(visibles, 1, "la entidad solo ve su catálogo");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_item_con_iva_debe_ser_venta_o_servicio(pool: PgPool) {
    let e = sembrar(&pool).await;
    let t = e.tenant.as_uuid();

    let r = como_app(
        &pool,
        e.tenant,
        format!(
            "INSERT INTO items_facturacion (id, tenant_id, codigo, descripcion, tarifa_iva,
                 clasificacion, creado_en, actualizado_en)
             VALUES ('{}', '{t}', 'EXP', 'Expensa', 19, 'expensa', now(), now());",
            Uuid::now_v7()
        ),
    )
    .await;

    assert!(es_violacion_de_check(r), "una expensa no lleva IVA");
}

// ---------------------------------------------------------------------------------------------
// Casos de uso: una empresa que solo contrató la facturación
// ---------------------------------------------------------------------------------------------

struct RelojFijo;

impl Reloj for RelojFijo {
    fn ahora(&self) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 14, 12, 0, 0)
            .single()
            .expect("fecha")
    }
}

fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(a, m, d).expect("fecha")
}

fn pesos(v: i64) -> Monto {
    Monto::new(Decimal::from(v)).expect("monto")
}

struct Tienda {
    unidad: UnidadDeTrabajoPg,
    facturador: Contexto,
    cliente: PersonaId,
}

/// Empresa sin vertical con `modulos` habilitados, un facturador y un cliente.
async fn tienda(pool: PgPool, modulos: &[Modulo]) -> Tienda {
    let tenant = insertar_entidad(&pool, "Tienda La Esquina").await;
    for m in modulos {
        sqlx::query(
            "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
             VALUES (uuidv7(), $1, $2, '2026-01-01T00:00:00Z')",
        )
        .bind(tenant.as_uuid())
        .bind(m.as_str())
        .execute(&pool)
        .await
        .expect("módulo");
    }
    let facturador = Contexto::de_cuenta(
        tenant,
        CuentaId::nuevo(),
        ConjuntoPermisos::todos(Ambito::Copropiedad),
    );
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);
    let cliente = if modulos.is_empty() {
        PersonaId::nuevo()
    } else {
        conf::crear_tercero(
            &unidad,
            &RelojFijo,
            &facturador,
            NuevoTercero {
                tipo: TipoPersona::Juridica,
                nombre: "Cliente SAS".into(),
                tipo_documento: None,
                documento: Documento::new("900555444").expect("documento"),
                fiscal: DatosFiscales::default(),
            },
        )
        .await
        .expect("el facturador crea su cliente sin contabilidad")
        .persona
        .id
    };
    Tienda {
        unidad,
        facturador,
        cliente,
    }
}

fn datos_item(precio: i64) -> DatosItem {
    DatosItem {
        descripcion: "Asesoría por hora".into(),
        unidad: Some("HUR".into()),
        precio: Some(pesos(precio)),
        tarifa_iva: Decimal::ZERO,
        clasificacion: ClasificacionTributaria::AlquilerServicio,
        cuenta: None,
        activo: true,
    }
}

fn nueva_factura(t: &Tienda, lineas: Vec<NuevaLineaVenta>) -> NuevaFacturaVenta {
    NuevaFacturaVenta {
        persona: t.cliente,
        fecha: fecha(2026, 11, 14),
        vencimiento: fecha(2026, 11, 30),
        descripcion: "Servicios de noviembre".into(),
        origen: None,
        lineas,
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_empresa_solo_con_facturacion_factura_desde_su_catalogo_sin_contabilizar(pool: PgPool) {
    // Preparar
    let t = tienda(pool.clone(), &[Modulo::FacturacionElectronica]).await;
    let item = items::crear_item(
        &t.unidad,
        &RelojFijo,
        &t.facturador,
        "ASE-H",
        datos_item(80_000),
    )
    .await
    .expect("ítem");
    let linea = NuevaLineaVenta {
        item: Some(item.id),
        cantidad: Decimal::from(3),
        ..NuevaLineaVenta::default()
    };

    // Ejecutar
    let mut emitidas = Vec::new();
    for _ in 0..2 {
        let f = ventas::crear_factura_venta(
            &t.unidad,
            &RelojFijo,
            &t.facturador,
            nueva_factura(&t, vec![linea.clone()]),
        )
        .await
        .expect("borrador");
        emitidas.push(
            ventas::emitir_factura_venta(&t.unidad, &RelojFijo, &t.facturador, f.id)
                .await
                .expect("emite sin contabilidad"),
        );
    }

    // Verificar: la línea salió del catálogo, la numeración es la propia y nada se contabilizó.
    let primera = &emitidas[0];
    assert_eq!(primera.lineas[0].descripcion, "Asesoría por hora");
    assert_eq!(primera.lineas[0].item_id, Some(item.id));
    assert_eq!(primera.lineas[0].cuenta_id, None);
    assert_eq!(primera.total, pesos(240_000));
    assert_eq!(
        emitidas.iter().map(|f| f.numero).collect::<Vec<_>>(),
        vec![Some(1), Some(2)]
    );
    assert!(emitidas.iter().all(|f| f.comprobante_id.is_none()));
    let comprobantes: i64 = sqlx::query_scalar("SELECT count(*) FROM comprobantes")
        .fetch_one(&pool)
        .await
        .expect("conteo");
    assert_eq!(comprobantes, 0, "sin contabilidad no hay asientos");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_contabilidad_el_pago_se_marca_a_mano(pool: PgPool) {
    let t = tienda(pool, &[Modulo::FacturacionElectronica]).await;
    let linea = NuevaLineaVenta {
        descripcion: Some("Producto".into()),
        cantidad: Decimal::ONE,
        valor_unitario: Some(pesos(50_000)),
        ..NuevaLineaVenta::default()
    };
    let f = ventas::crear_factura_venta(
        &t.unidad,
        &RelojFijo,
        &t.facturador,
        nueva_factura(&t, vec![linea]),
    )
    .await
    .expect("borrador");
    let f = ventas::emitir_factura_venta(&t.unidad, &RelojFijo, &t.facturador, f.id)
        .await
        .expect("emitida");
    let pago = |dia| NuevoPagoManual {
        fecha: fecha(2026, 11, dia),
        medio: MedioPagoManual::Transferencia,
        referencia: Some("TRX-1".into()),
    };

    let antes = ventas::marcar_pagada(&t.unidad, &RelojFijo, &t.facturador, f.id, pago(1)).await;
    let pagada = ventas::marcar_pagada(&t.unidad, &RelojFijo, &t.facturador, f.id, pago(20))
        .await
        .expect("pagada");

    assert!(
        matches!(antes, Err(AppError::Validacion(_))),
        "antes de la factura: {antes:?}"
    );
    assert_eq!(pagada.estado, EstadoFacturaVenta::Pagada);
    assert_eq!(pagada.pagado, pagada.total);
    let marca = pagada.pago_manual.expect("marca");
    assert_eq!(marca.medio, MedioPagoManual::Transferencia);
    assert_eq!(Some(marca.por), t.facturador.actor_id);
    let leida = ventas::factura_venta(&t.unidad, &RelojFijo, &t.facturador, f.id)
        .await
        .expect("lee");
    assert_eq!(
        leida.pago_manual.map(|p| p.referencia),
        Some(Some("TRX-1".into()))
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_contabilidad_la_factura_no_lleva_cuentas(pool: PgPool) {
    let t = tienda(pool, &[Modulo::FacturacionElectronica]).await;
    let linea = NuevaLineaVenta {
        descripcion: Some("Producto".into()),
        cuenta: Some(CodigoCuenta::new("4135").expect("código")),
        cantidad: Decimal::ONE,
        valor_unitario: Some(pesos(50_000)),
        ..NuevaLineaVenta::default()
    };

    let r = ventas::crear_factura_venta(
        &t.unidad,
        &RelojFijo,
        &t.facturador,
        nueva_factura(&t, vec![linea]),
    )
    .await;
    let item = items::crear_item(
        &t.unidad,
        &RelojFijo,
        &t.facturador,
        "X",
        DatosItem {
            cuenta: Some(CodigoCuenta::new("4135").expect("código")),
            ..datos_item(1_000)
        },
    )
    .await;

    assert!(matches!(r, Err(AppError::Validacion(_))), "{r:?}");
    assert!(matches!(item, Err(AppError::Validacion(_))), "{item:?}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_linea_sin_descripcion_ni_item_se_rechaza(pool: PgPool) {
    let t = tienda(pool, &[Modulo::FacturacionElectronica]).await;
    let linea = NuevaLineaVenta {
        cantidad: Decimal::ONE,
        valor_unitario: Some(pesos(1_000)),
        ..NuevaLineaVenta::default()
    };

    let r = ventas::crear_factura_venta(
        &t.unidad,
        &RelojFijo,
        &t.facturador,
        nueva_factura(&t, vec![linea]),
    )
    .await;

    assert!(matches!(r, Err(AppError::Validacion(_))), "{r:?}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_facturacion_ni_contabilidad_no_se_factura(pool: PgPool) {
    let t = tienda(pool, &[]).await;

    let r = items::items(&t.unidad, &RelojFijo, &t.facturador, FiltroItems::default()).await;

    assert!(
        matches!(
            r,
            Err(AppError::ModuloNoHabilitado(Modulo::FacturacionElectronica))
        ),
        "{r:?}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_catalogo_se_busca_sin_tildes_y_oculta_los_inactivos(pool: PgPool) {
    let t = tienda(pool, &[Modulo::FacturacionElectronica]).await;
    let activo = items::crear_item(&t.unidad, &RelojFijo, &t.facturador, "A", datos_item(1_000))
        .await
        .expect("ítem");
    let inactivo = items::crear_item(&t.unidad, &RelojFijo, &t.facturador, "B", datos_item(2_000))
        .await
        .expect("ítem");
    items::actualizar_item(
        &t.unidad,
        &RelojFijo,
        &t.facturador,
        inactivo.id,
        DatosItem {
            activo: false,
            ..datos_item(2_000)
        },
    )
    .await
    .expect("desactiva");
    let repetido =
        items::crear_item(&t.unidad, &RelojFijo, &t.facturador, "A", datos_item(1_000)).await;

    let encontrados = items::items(
        &t.unidad,
        &RelojFijo,
        &t.facturador,
        FiltroItems {
            texto: Some("ASESORIA".into()),
            con_inactivos: false,
        },
    )
    .await
    .expect("busca");
    let linea = NuevaLineaVenta {
        item: Some(inactivo.id),
        cantidad: Decimal::ONE,
        ..NuevaLineaVenta::default()
    };
    let con_inactivo = ventas::crear_factura_venta(
        &t.unidad,
        &RelojFijo,
        &t.facturador,
        nueva_factura(&t, vec![linea]),
    )
    .await;

    assert_eq!(
        encontrados.iter().map(|i| i.id).collect::<Vec<_>>(),
        vec![activo.id]
    );
    assert!(
        matches!(repetido, Err(AppError::Conflicto(_))),
        "{repetido:?}"
    );
    assert!(
        matches!(con_inactivo, Err(AppError::Validacion(_))),
        "{con_inactivo:?}"
    );
}

fn compra(t: &Tienda, retenciones: Vec<String>) -> NuevaFacturaProveedor {
    NuevaFacturaProveedor {
        persona: t.cliente,
        prefijo: String::new(),
        numero: "77".into(),
        fecha: fecha(2026, 11, 14),
        vencimiento: fecha(2026, 11, 30),
        descripcion: "Compra a no obligado".into(),
        origen: None,
        cufe: None,
        archivo_id: None,
        lineas: vec![NuevaLineaFacturaProveedor {
            descripcion: "Servicio de aseo".into(),
            cuenta: None,
            centro_costo: None,
            base: pesos(300_000),
            iva: Monto::CERO,
            iva_descontable: false,
            retenciones,
        }],
        excepcion_duplicado: None,
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_contabilidad_la_compra_se_registra_sin_cuentas_ni_retenciones(pool: PgPool) {
    let t = tienda(pool, &[Modulo::FacturacionElectronica]).await;

    let con_retencion = cxp::registrar_factura_proveedor(
        &t.unidad,
        &RelojFijo,
        &t.facturador,
        compra(&t, vec!["servicios".into()]),
    )
    .await;
    let registrada =
        cxp::registrar_factura_proveedor(&t.unidad, &RelojFijo, &t.facturador, compra(&t, vec![]))
            .await
            .expect("registrada para el documento soporte");

    assert!(
        matches!(con_retencion, Err(AppError::Validacion(_))),
        "{con_retencion:?}"
    );
    assert_eq!(registrada.lineas[0].cuenta_id, None);
    assert!(registrada.comprobante_id.is_none());
}

// ---------------------------------------------------------------------------------------------
// Habilitación con el motor propio (plan 18, parte B)
// ---------------------------------------------------------------------------------------------

mod habilitacion {
    use std::sync::Mutex;

    use application::{
        AppResult,
        casos_uso::facturacion::habilitacion::{self as hab, DatosSoftware},
        puertos::{
            cofre::Cofre,
            tributario::{
                CertificadoLeido, CredencialesFacturador, DocumentoAEmitir, HabilitacionDian,
                LectorCertificados, RangoAutorizado, RespuestaProveedor,
            },
        },
    };
    use domain::{
        facturacion::{AmbienteDian, EstadoEnvioSet, EstadoHabilitacion},
        tributario::EstadoDocumentoElectronico,
    };

    use super::*;

    /// Cofre de prueba: antepone el contexto (así se nota si se descifra con otro).
    struct CofrePrueba;

    impl Cofre for CofrePrueba {
        fn cifrar(&self, contexto: &str, datos: &[u8]) -> AppResult<Vec<u8>> {
            let mut v = contexto.as_bytes().to_vec();
            v.push(0);
            v.extend_from_slice(datos);
            Ok(v)
        }

        fn descifrar(&self, contexto: &str, cifrado: &[u8]) -> AppResult<Vec<u8>> {
            let prefijo = [contexto.as_bytes(), &[0]].concat();
            cifrado
                .strip_prefix(prefijo.as_slice())
                .map(<[u8]>::to_vec)
                .ok_or_else(|| AppError::Validacion("otro contexto".into()))
        }
    }

    struct LectorPrueba;

    impl LectorCertificados for LectorPrueba {
        fn leer(&self, _p12: &[u8], contrasena: &str) -> AppResult<CertificadoLeido> {
            if contrasena != "clave-p12" {
                return Err(AppError::Validacion("contraseña errada".into()));
            }
            Ok(CertificadoLeido {
                sujeto: "CN=Tienda La Esquina".into(),
                emisor: "CN=CA de prueba".into(),
                serial: "123".into(),
                vigente_desde: Utc
                    .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                    .single()
                    .expect("f"),
                vigente_hasta: Utc
                    .with_ymd_and_hms(2026, 12, 1, 0, 0, 0)
                    .single()
                    .expect("f"),
            })
        }
    }

    /// DIAN de prueba: acepta todo y recuerda las credenciales que recibió.
    #[derive(Default)]
    struct DianPrueba {
        recibidas: Mutex<Vec<CredencialesFacturador>>,
    }

    impl HabilitacionDian for DianPrueba {
        async fn enviar_set_pruebas(&self, d: &DocumentoAEmitir) -> AppResult<RespuestaProveedor> {
            let c = d.credenciales.clone().expect("credenciales");
            self.recibidas.lock().expect("candado").push(c);
            Ok(RespuestaProveedor {
                estado: EstadoDocumentoElectronico::Enviado,
                id_externo: Some(format!("zip-{}", d.numero)),
                cufe: Some("c".repeat(96)),
                xml: None,
                mensaje: None,
            })
        }

        async fn consultar_set(
            &self,
            _c: &CredencialesFacturador,
            _clave: &str,
        ) -> AppResult<RespuestaProveedor> {
            Ok(RespuestaProveedor {
                estado: EstadoDocumentoElectronico::Validado,
                id_externo: None,
                cufe: None,
                xml: None,
                mensaje: Some("Procesado Correctamente".into()),
            })
        }

        async fn rangos_numeracion(
            &self,
            c: &CredencialesFacturador,
            _nit: &str,
        ) -> AppResult<Vec<RangoAutorizado>> {
            assert_eq!(c.ambiente, AmbienteDian::Produccion);
            Ok(vec![RangoAutorizado {
                resolucion: "18764000000099".into(),
                fecha_resolucion: Some(fecha(2026, 11, 1)),
                prefijo: "TLE".into(),
                desde: 1,
                hasta: 5000,
                vigente_desde: fecha(2026, 11, 1),
                vigente_hasta: fecha(2027, 11, 1),
                clave_tecnica: "clave-real".into(),
            }])
        }
    }

    async fn con_nit(t: &Tienda, pool: &PgPool) {
        sqlx::query("UPDATE entidades SET nit = '900123456-1' WHERE id = $1")
            .bind(t.facturador.tenant_id.as_uuid())
            .execute(pool)
            .await
            .expect("nit");
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn la_entidad_se_habilita_de_punta_a_punta_y_sus_secretos_quedan_cifrados(pool: PgPool) {
        // Preparar
        let t = tienda(pool.clone(), &[Modulo::FacturacionElectronica]).await;
        con_nit(&t, &pool).await;
        let dian = DianPrueba::default();
        let datos = DatosSoftware {
            software_id: "56f2ae4e-9812".into(),
            pin: "12345".into(),
            test_set_id: Some("set-1".into()),
        };

        // Ejecutar: software → certificado → set → consulta → producción → rangos.
        hab::registrar_software(&t.unidad, &CofrePrueba, &RelojFijo, &t.facturador, datos)
            .await
            .expect("software");
        let sin_certificado = hab::enviar_set_pruebas(
            &t.unidad,
            (&dian, &CofrePrueba),
            &RelojFijo,
            &t.facturador,
            2,
        )
        .await;
        let errada = hab::cargar_certificado(
            &t.unidad,
            (&CofrePrueba, &LectorPrueba),
            &RelojFijo,
            &t.facturador,
            b"p12",
            "otra",
        )
        .await;
        let certificado = hab::cargar_certificado(
            &t.unidad,
            (&CofrePrueba, &LectorPrueba),
            &RelojFijo,
            &t.facturador,
            b"p12",
            "clave-p12",
        )
        .await
        .expect("certificado");
        let antes = hab::pasar_a_produccion(&t.unidad, &RelojFijo, &t.facturador).await;
        let enviados = hab::enviar_set_pruebas(
            &t.unidad,
            (&dian, &CofrePrueba),
            &RelojFijo,
            &t.facturador,
            2,
        )
        .await
        .expect("set");
        let consultado =
            hab::consultar_set_pruebas(&t.unidad, (&dian, &CofrePrueba), &RelojFijo, &t.facturador)
                .await
                .expect("consulta");
        let produccion = hab::pasar_a_produccion(&t.unidad, &RelojFijo, &t.facturador)
            .await
            .expect("producción");
        let rangos =
            hab::sincronizar_rangos(&t.unidad, (&dian, &CofrePrueba), &RelojFijo, &t.facturador)
                .await
                .expect("rangos");
        let otra_vez =
            hab::sincronizar_rangos(&t.unidad, (&dian, &CofrePrueba), &RelojFijo, &t.facturador)
                .await
                .expect("rangos");

        // Verificar
        assert!(matches!(sin_certificado, Err(AppError::Validacion(_))));
        assert!(matches!(errada, Err(AppError::Validacion(_))));
        assert!(
            certificado.por_vencer,
            "vence en menos de 30 días frente al reloj fijo"
        );
        assert!(antes.is_err(), "sin set aceptado no hay producción");
        assert_eq!(
            enviados.iter().map(|e| e.numero).collect::<Vec<_>>(),
            vec![990_000_001, 990_000_002]
        );
        assert!(
            consultado
                .envios
                .iter()
                .all(|e| e.estado == EstadoEnvioSet::Aceptado)
        );
        assert_eq!(consultado.estado, Some(EstadoHabilitacion::Habilitado));
        assert_eq!(produccion.estado, Some(EstadoHabilitacion::Produccion));
        assert_eq!(produccion.ambiente, Some(AmbienteDian::Produccion));
        assert_eq!(rangos.len(), 1, "el rango nuevo queda como resolución");
        assert!(otra_vez.is_empty(), "sincronizar otra vez no duplica");
        let recibidas = dian.recibidas.lock().expect("candado").clone();
        assert!(
            recibidas
                .iter()
                .all(|c| c.pin == "12345" && c.contrasena == "clave-p12")
        );
        assert_eq!(
            recibidas
                .iter()
                .map(|c| c.consecutivo_archivo)
                .collect::<Vec<_>>(),
            vec![1, 2],
            "cada archivo enviado lleva su consecutivo del año"
        );
        // En la base nada queda en claro.
        let (pin, clave): (Vec<u8>, Vec<u8>) = sqlx::query_as(
            "SELECT c.pin_cifrado, f.contrasena_cifrada FROM configuraciones_facturacion c
             JOIN certificados_firma f ON f.id = c.certificado_id",
        )
        .fetch_one(&pool)
        .await
        .expect("secretos");
        assert!(
            pin.starts_with(b"facturacion.pin:"),
            "el PIN pasó por el cofre"
        );
        assert!(clave.starts_with(b"facturacion.contrasena:"));
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn en_produccion_el_software_ya_no_se_cambia(pool: PgPool) {
        let t = tienda(pool.clone(), &[Modulo::FacturacionElectronica]).await;
        sqlx::query(
            "INSERT INTO configuraciones_facturacion (tenant_id, software_id, pin_cifrado,
                 ambiente, estado, actualizado_en)
             VALUES ($1, 'sw', '\\x00', 'produccion', 'produccion', now())",
        )
        .bind(t.facturador.tenant_id.as_uuid())
        .execute(&pool)
        .await
        .expect("en producción");

        let r = hab::registrar_software(
            &t.unidad,
            &CofrePrueba,
            &RelojFijo,
            &t.facturador,
            DatosSoftware {
                software_id: "otro".into(),
                pin: "1".into(),
                test_set_id: None,
            },
        )
        .await;

        assert!(matches!(r, Err(AppError::Dominio(_))), "{r:?}");
    }
}
