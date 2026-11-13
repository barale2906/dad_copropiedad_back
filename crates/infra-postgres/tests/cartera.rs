//! Pruebas de integración de la vertical contable de la PH (plan 14 fases 1–4, ADR 0019) como
//! `dad_app`: liquidación de expensas, cuentas de cobro, recaudo con imputación, pronto pago y
//! anticipos, notas crédito, cartera inicial, módulos de contribución, recaudo por referencia y
//! cuadre contra el mayor.

use application::{
    AppError, Contexto, ContextoPlataforma,
    casos_uso::{
        coeficientes::{self, NuevoCuadro, RefBienPrivado},
        contabilidad::{
            conciliacion::{self as conc, FormatoEntrada, ImportacionExtracto},
            configuracion::{
                self as conf, DatosTipoComprobante, NuevaConfiguracion, NuevaCuenta,
                NuevaLineaRegla, NuevaRegla,
            },
            tesoreria::{self as tes, NuevaCuentaTesoreria, conceptos},
        },
        contabilidad_ph::{
            configuracion::{self as cfg, NuevoConcepto},
            eventos, informes, integraciones,
            liquidacion::{self as liq, NuevaCuota},
            mora, portal,
            presupuesto::{self as pres, NuevoRubro},
            recaudo::{self as rec, FilaCarteraInicial, NuevoCobro, NuevoRecaudo},
        },
    },
    puertos::{
        AlmacenArchivos, LectorPlanillas, Reloj, archivos::DescargaArchivo, cartera::ConceptoCobro,
        tesoreria::CuentaTesoreria,
    },
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use domain::{
    Almacen, Ambito, BienPrivadoId, ClaveArchivo, Coeficiente, ConjuntoPermisos, CuentaContableId,
    CuentaId, DominioError, NombreArchivo, TenantId, TipoArchivo,
    contabilidad::{
        Cara, CodigoCuenta, MarcoTecnico, Monto, OrigenCentro, OrigenTercero,
        extracto::{MapeoColumnas, SeparadorDecimal},
        tesoreria::{MedioRecibo, TipoCuentaBancaria, TipoCuentaTesoreria},
    },
    contabilidad_ph::{
        CriterioDistribucion, EstadoLiquidacionExpensas, OrdenImputacion, ParametrosPolitica,
        TipoConceptoCobro,
    },
};
use infra_postgres::{UnidadDeTrabajoPg, UnidadDeTrabajoPlataformaPg};
use rust_decimal::{Decimal, dec};
use sqlx::PgPool;
use uuid::Uuid;

mod comun;
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};

struct RelojFijo;

impl Reloj for RelojFijo {
    fn ahora(&self) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 13, 12, 0, 0)
            .single()
            .expect("fecha")
    }
}

struct AlmacenNulo;

impl AlmacenArchivos for AlmacenNulo {
    fn almacen(&self) -> Almacen {
        Almacen::Local
    }

    async fn guardar(&self, _: &ClaveArchivo, _: &[u8]) -> application::AppResult<()> {
        Ok(())
    }

    async fn obtener(&self, _: &ClaveArchivo) -> application::AppResult<Vec<u8>> {
        Ok(Vec::new())
    }

    async fn eliminar(&self, _: &ClaveArchivo) -> application::AppResult<()> {
        Ok(())
    }

    fn url_temporal(&self, _: &DescargaArchivo) -> application::AppResult<String> {
        Ok(String::new())
    }
}

struct LectorCsv;

impl LectorPlanillas for LectorCsv {
    fn leer(&self, _: TipoArchivo, contenido: &[u8]) -> application::AppResult<Vec<Vec<String>>> {
        Ok(String::from_utf8_lossy(contenido)
            .lines()
            .map(|l| l.split(',').map(str::to_owned).collect())
            .collect())
    }
}

struct Entorno {
    pool: PgPool,
    unidad: UnidadDeTrabajoPg,
    contador: Contexto,
    banco: CuentaTesoreria,
    /// Bienes 101, 102 y 103 (coeficientes 0.5, 0.3 y 0.2).
    bienes: [BienPrivadoId; 3],
    administracion: ConceptoCobro,
    multa: ConceptoCobro,
}

fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(a, m, d).expect("fecha")
}

fn m(valor: i64) -> Monto {
    Monto::new(Decimal::from(valor)).expect("monto")
}

fn cod(c: &str) -> CodigoCuenta {
    CodigoCuenta::new(c).expect("código")
}

fn politica() -> ParametrosPolitica {
    ParametrosPolitica {
        dia_vencimiento: 10,
        redondeo: dec!(100),
        pronto_pago: Some((dec!(10), 5)),
        factor_mora: dec!(1.5),
        tasa_mora_mensual: None,
        imputacion: OrdenImputacion::InteresesPrimero,
        permite_elegir_obligacion: false,
        porcentaje_fondo: dec!(1),
        fondo_suspendido: false,
    }
}

async fn crear_cuenta(pool: &PgPool, usuario: &str) -> CuentaId {
    let c = CuentaId::nuevo();
    sqlx::query(
        "INSERT INTO cuentas (id, nombre_mostrar, usuario, hash_contrasena, contrasena_actualizada_en, creada_en)
         VALUES ($1, 'Persona', $2, 'x', now(), now())",
    )
    .bind(c.as_uuid())
    .bind(usuario)
    .execute(pool)
    .await
    .expect("cuenta");
    c
}

async fn cuenta_contable(pool: &PgPool, tenant: TenantId, codigo: &str) -> CuentaContableId {
    let id: Uuid =
        sqlx::query_scalar("SELECT id FROM cuentas_contables WHERE tenant_id = $1 AND codigo = $2")
            .bind(tenant.as_uuid())
            .bind(codigo)
            .fetch_one(pool)
            .await
            .expect("cuenta contable");
    CuentaContableId::from_uuid(id)
}

fn regla(evento: &str, tipo: &str, lineas: &[(&str, Cara, &str, bool)]) -> NuevaRegla {
    NuevaRegla {
        evento: evento.into(),
        tipo_comprobante: tipo.into(),
        descripcion: None,
        vigente_desde: fecha(2026, 1, 1),
        lineas: lineas
            .iter()
            .map(|(concepto, cara, cuenta, tercero)| NuevaLineaRegla {
                concepto: (*concepto).into(),
                cara: *cara,
                cuenta: cod(cuenta),
                tercero: if *tercero {
                    OrigenTercero::Documento
                } else {
                    OrigenTercero::Ninguno
                },
                centro: OrigenCentro::Ninguno,
                centro_costo: None,
            })
            .collect(),
    }
}

fn concepto(
    codigo: &str,
    tipo: TipoConceptoCobro,
    criterio: CriterioDistribucion,
    contable: &str,
) -> NuevoConcepto {
    NuevoConcepto {
        codigo: codigo.into(),
        nombre: codigo.into(),
        tipo,
        criterio,
        modulo: None,
        valor_fijo: None,
        tarifas: Vec::new(),
        genera_interes: tipo == TipoConceptoCobro::Ordinaria,
        concepto_contable: contable.into(),
    }
}

/// Bienes con propietario representante y el cuadro de coeficientes vigente desde enero.
async fn bienes_y_cuadro(pool: &PgPool, tenant: TenantId) -> [BienPrivadoId; 3] {
    let bienes = [Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7()];
    let cuadro = Uuid::now_v7();
    for (i, (bien, codigo, coeficiente)) in [
        (bienes[0], "101", dec!(0.5)),
        (bienes[1], "102", dec!(0.3)),
        (bienes[2], "103", dec!(0.2)),
    ]
    .into_iter()
    .enumerate()
    {
        let persona = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO bienes_privados (id, tenant_id, codigo, area_privada_m2)
             VALUES ($1, $2, $3, 60)",
        )
        .bind(bien)
        .bind(tenant.as_uuid())
        .bind(codigo)
        .execute(pool)
        .await
        .expect("bien");
        sqlx::query("INSERT INTO personas (id, tenant_id, nombre, correo) VALUES ($1, $2, $3, $4)")
            .bind(persona)
            .bind(tenant.as_uuid())
            .bind(format!("Propietario {codigo}"))
            .bind(format!("p{i}@correo.co"))
            .execute(pool)
            .await
            .expect("persona");
        sqlx::query(
            "INSERT INTO titularidades (id, tenant_id, bien_privado_id, persona_id, tipo,
                 porcentaje, es_representante, vigente_desde)
             VALUES (gen_random_uuid(), $1, $2, $3, 'propietario', 100, true, '2025-01-01')",
        )
        .bind(tenant.as_uuid())
        .bind(bien)
        .bind(persona)
        .execute(pool)
        .await
        .expect("titular");
        if i == 0 {
            sqlx::query(
                "INSERT INTO cuadros_coeficientes (id, tenant_id, version) VALUES ($1, $2, 1)",
            )
            .bind(cuadro)
            .bind(tenant.as_uuid())
            .execute(pool)
            .await
            .expect("cuadro");
        }
        sqlx::query(
            "INSERT INTO coeficientes_bien_privado (tenant_id, cuadro_id, bien_privado_id, coeficiente)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(tenant.as_uuid())
        .bind(cuadro)
        .bind(bien)
        .bind(coeficiente)
        .execute(pool)
        .await
        .expect("coeficiente");
    }
    sqlx::query(
        "UPDATE cuadros_coeficientes SET estado = 'vigente', vigente_desde = '2026-01-01' WHERE id = $1",
    )
    .bind(cuadro)
    .execute(pool)
    .await
    .expect("activar cuadro");
    bienes.map(BienPrivadoId::from_uuid)
}

/// Copropiedad con contabilidad y cartera, plan de cuentas, reglas `ph.*`, banco, tres bienes,
/// conceptos (administración, fondo, multa), política y presupuesto 2026 de 1.000.000 al mes.
async fn entorno(pool: PgPool) -> Entorno {
    let tenant = crear_copropiedad(&pool).await;
    for modulo in ["contabilidad", "contabilidad_ph"] {
        sqlx::query(
            "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
             VALUES (uuidv7(), $1, $2, now() - interval '1 day')",
        )
        .bind(tenant.as_uuid())
        .bind(modulo)
        .execute(&pool)
        .await
        .expect("módulo");
    }
    let todos = ConjuntoPermisos::todos(Ambito::Copropiedad);
    let contador = Contexto::de_cuenta(tenant, crear_cuenta(&pool, "contador").await, todos);
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);
    for (codigo, movimiento, tercero) in [
        ("1", false, false),
        ("11", false, false),
        ("1110", true, false),
        ("13", false, false),
        ("1305", true, true),
        ("2", false, false),
        ("28", false, false),
        ("2805", true, false),
        ("3", false, false),
        ("36", false, false),
        ("3605", true, false),
        ("4", false, false),
        ("41", false, false),
        ("4170", true, false),
        ("4175", true, false),
        ("42", false, false),
        ("4210", true, false),
        ("5", false, false),
        ("51", false, false),
        ("5135", true, false),
        ("53", false, false),
        ("5305", true, false),
    ] {
        conf::crear_cuenta(
            &unidad,
            &RelojFijo,
            &contador,
            NuevaCuenta {
                codigo: cod(codigo),
                nombre: format!("Cuenta {codigo}"),
                naturaleza: None,
                acepta_movimiento: movimiento,
                exige_tercero: tercero,
                exige_centro_costo: false,
            },
        )
        .await
        .expect("cuenta");
    }
    for codigo in ["RC", "CC", "LQ"] {
        conf::guardar_tipo_comprobante(
            &unidad,
            &RelojFijo,
            &contador,
            None,
            DatosTipoComprobante {
                codigo: codigo.into(),
                nombre: format!("Tipo {codigo}"),
                manual: codigo == "CC",
                activo: true,
            },
        )
        .await
        .expect("tipo");
    }
    conf::guardar_configuracion(
        &unidad,
        &RelojFijo,
        &contador,
        NuevaConfiguracion {
            marco_tecnico: MarcoTecnico::Grupo3,
            cuenta_resultado: Some(cod("3605")),
            soporte_archivo_id: None,
            vigente_desde: fecha(2026, 1, 1),
        },
    )
    .await
    .expect("configuración");
    use Cara::{Credito as C, Debito as D};
    for r in [
        regla(
            eventos::LIQUIDACION,
            "LQ",
            &[
                ("administracion", D, "1305", true),
                ("administracion", C, "4170", false),
                ("fondo", D, "1305", true),
                ("fondo", C, "4175", false),
                ("extraordinaria", D, "1305", true),
                ("extraordinaria", C, "2805", false),
                ("multa", D, "1305", true),
                ("multa", C, "4170", false),
            ],
        ),
        regla(
            eventos::COBRO,
            "CC",
            &[("multa", D, "1305", true), ("multa", C, "4170", false)],
        ),
        regla(
            eventos::RECAUDO,
            "RC",
            &[
                (conceptos::RECIBIDO, D, "1110", false),
                ("administracion", C, "1305", true),
                ("fondo", C, "1305", true),
                ("extraordinaria", C, "1305", true),
                ("multa", C, "1305", true),
                ("intereses", C, "1305", true),
                (conceptos::SIN_APLICAR, C, "2805", false),
            ],
        ),
        regla(
            eventos::INTERESES,
            "CC",
            &[
                ("intereses", D, "1305", true),
                ("intereses", C, "4210", false),
            ],
        ),
        regla(
            eventos::APLICACION,
            "CC",
            &[
                (conceptos::SIN_APLICAR, D, "2805", false),
                ("administracion", C, "1305", true),
                ("fondo", C, "1305", true),
                ("multa", C, "1305", true),
            ],
        ),
        regla(
            eventos::PRONTO_PAGO,
            "CC",
            &[
                ("administracion", D, "5305", false),
                ("administracion", C, "1305", true),
            ],
        ),
        regla(
            eventos::NOTA_CREDITO,
            "CC",
            &[
                ("administracion", D, "4170", false),
                ("administracion", C, "1305", true),
                ("multa", D, "4170", false),
                ("multa", C, "1305", true),
            ],
        ),
    ] {
        conf::crear_regla(&unidad, &RelojFijo, &contador, r)
            .await
            .expect("regla");
    }
    let banco = tes::crear_cuenta_tesoreria(
        &unidad,
        &RelojFijo,
        &contador,
        NuevaCuentaTesoreria {
            tipo: TipoCuentaTesoreria::Banco,
            nombre: "Cuenta de recaudo".into(),
            banco: Some("Banco de prueba".into()),
            numero_cuenta: Some("123-456789-01".into()),
            tipo_cuenta: Some(TipoCuentaBancaria::Ahorros),
            cuenta_contable: cod("1110"),
            destinacion: None,
        },
    )
    .await
    .expect("banco");
    let bienes = bienes_y_cuadro(&pool, tenant).await;
    let administracion = cfg::crear_concepto_cobro(
        &unidad,
        &RelojFijo,
        &contador,
        concepto(
            "administracion",
            TipoConceptoCobro::Ordinaria,
            CriterioDistribucion::Coeficiente,
            "administracion",
        ),
    )
    .await
    .expect("administración");
    cfg::crear_concepto_cobro(
        &unidad,
        &RelojFijo,
        &contador,
        concepto(
            "fondo_imprevistos",
            TipoConceptoCobro::FondoImprevistos,
            CriterioDistribucion::Coeficiente,
            "fondo",
        ),
    )
    .await
    .expect("fondo");
    let multa = cfg::crear_concepto_cobro(
        &unidad,
        &RelojFijo,
        &contador,
        concepto(
            "multa",
            TipoConceptoCobro::Multa,
            CriterioDistribucion::Coeficiente,
            "multa",
        ),
    )
    .await
    .expect("multa");
    cfg::crear_politica_cartera(
        &unidad,
        &RelojFijo,
        &contador,
        fecha(2026, 1, 1),
        politica(),
        None,
    )
    .await
    .expect("política");
    let gasto = cuenta_contable(&pool, tenant, "5135").await;
    let p = pres::crear_presupuesto(
        &unidad,
        &RelojFijo,
        &contador,
        2026,
        Some("Presupuesto 2026"),
        vec![NuevoRubro {
            concepto: administracion.id,
            cuenta: gasto,
            centro_costo: None,
            nombre: "Gastos de administración".into(),
            valores_mensuales: vec![m(1_000_000); 12],
        }],
    )
    .await
    .expect("presupuesto");
    pres::aprobar_presupuesto(
        &unidad,
        &RelojFijo,
        &contador,
        p.id,
        fecha(2026, 1, 1),
        None,
    )
    .await
    .expect("aprobar");
    cfg::crear_concepto_cobro(
        &unidad,
        &RelojFijo,
        &contador,
        concepto(
            "intereses",
            TipoConceptoCobro::InteresMora,
            CriterioDistribucion::Coeficiente,
            "intereses",
        ),
    )
    .await
    .expect("intereses");
    sqlx::query(
        "INSERT INTO tasas_referencia (id, vigente_desde, vigente_hasta, tasa_efectiva_anual,
             resolucion, fuente, creada_en)
         VALUES (uuidv7(), '2026-01-01', '2026-12-31', 20, 'Resolución de prueba', 'Prueba', now())",
    )
    .execute(&pool)
    .await
    .expect("tasa");
    Entorno {
        pool,
        unidad,
        contador,
        banco,
        bienes,
        administracion,
        multa,
    }
}

impl Entorno {
    async fn liquidar(&self, periodo: NaiveDate) -> application::AppResult<liq::Liquidado> {
        liq::liquidar_expensas(&self.unidad, &RelojFijo, &self.contador, periodo).await
    }

    async fn saldo(&self, bien: BienPrivadoId) -> Decimal {
        rec::estado_cuenta(
            &self.unidad,
            &RelojFijo,
            &self.contador,
            bien,
            Some(fecha(2026, 12, 31)),
        )
        .await
        .expect("estado de cuenta")
        .saldo
    }

    async fn recaudar(
        &self,
        bien: BienPrivadoId,
        dia: NaiveDate,
        valor: i64,
    ) -> application::AppResult<rec::Recaudo> {
        rec::recaudar(
            &self.unidad,
            &RelojFijo,
            &self.contador,
            NuevoRecaudo {
                bien,
                cuenta_tesoreria: self.banco.id,
                fecha: dia,
                valor: m(valor),
                medio: MedioRecibo::Transferencia,
                referencia: None,
                descripcion: None,
                soporte_archivo_id: None,
                elegidas: Vec::new(),
            },
        )
        .await
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn liquidar_reparte_contabiliza_y_crea_una_cuenta_de_cobro_por_bien(pool: PgPool) {
    let e = entorno(pool).await;

    let l = e.liquidar(fecha(2026, 11, 13)).await.expect("liquidar");

    assert_eq!(l.liquidacion.periodo, fecha(2026, 11, 1));
    assert_eq!(l.liquidacion.total.valor(), dec!(1010000));
    assert_eq!(l.liquidacion.vencimiento, fecha(2026, 11, 10));
    assert_eq!(l.cuentas_cobro.len(), 3);
    let numeros: Vec<u32> = l.cuentas_cobro.iter().map(|c| c.numero).collect();
    assert_eq!(numeros, vec![1, 2, 3]);
    let cuenta_101 = l
        .cuentas_cobro
        .iter()
        .find(|c| c.bien_privado_id == e.bienes[0])
        .expect("cuenta del 101");
    assert_eq!(cuenta_101.cargos.valor(), dec!(505000));
    assert_eq!(
        cuenta_101.descuento_pronto_pago.map(Monto::valor),
        Some(dec!(50000))
    );
    assert_eq!(e.saldo(e.bienes[1]).await, dec!(303000));
    let cuadre = rec::cuadre_cartera(&e.unidad, &RelojFijo, &e.contador)
        .await
        .expect("cuadre");
    assert_eq!(cuadre.len(), 1);
    assert_eq!(cuadre[0].saldo_mayor, dec!(1010000));
    assert_eq!(cuadre[0].diferencia, Decimal::ZERO);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn reliquidar_el_mismo_periodo_no_duplica(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");

    let otra = e.liquidar(fecha(2026, 11, 20)).await;

    assert!(matches!(otra, Err(AppError::Conflicto(_))), "{otra:?}");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM obligaciones")
        .fetch_one(&e.pool)
        .await
        .expect("contar");
    assert_eq!(n, 6);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_vista_previa_no_guarda_nada(pool: PgPool) {
    let e = entorno(pool).await;

    let vista =
        liq::vista_previa_liquidacion(&e.unidad, &RelojFijo, &e.contador, fecha(2026, 11, 1))
            .await
            .expect("vista previa");

    assert_eq!(vista.total, dec!(1010000));
    assert_eq!(vista.por_concepto.len(), 2);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM liquidaciones_expensas")
        .fetch_one(&e.pool)
        .await
        .expect("contar");
    assert_eq!(n, 0);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_pago_se_imputa_y_lo_que_sobra_se_aplica_en_la_siguiente_liquidacion(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("noviembre");

    let r = e
        .recaudar(e.bienes[0], fecha(2026, 11, 20), 600_000)
        .await
        .expect("recaudo");

    assert_eq!(r.aplicado.len(), 2);
    assert_eq!(r.anticipo, dec!(95000));
    assert_eq!(r.descuento, None);
    assert_eq!(e.saldo(e.bienes[0]).await, Decimal::ZERO);

    let dic = e.liquidar(fecha(2026, 12, 1)).await.expect("diciembre");

    assert_eq!(dic.anticipos_aplicados, 1);
    assert_eq!(e.saldo(e.bienes[0]).await, dec!(410000));
    let cuadre = rec::cuadre_cartera(&e.unidad, &RelojFijo, &e.contador)
        .await
        .expect("cuadre");
    assert_eq!(cuadre[0].diferencia, Decimal::ZERO);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_pronto_pago_descuenta_y_anular_el_recaudo_lo_devuelve(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");

    let r = e
        .recaudar(e.bienes[1], fecha(2026, 11, 4), 273_000)
        .await
        .expect("recaudo");

    assert_eq!(r.descuento.map(Monto::valor), Some(dec!(30000)));
    assert_eq!(r.anticipo, Decimal::ZERO);
    assert_eq!(e.saldo(e.bienes[1]).await, Decimal::ZERO);

    let normal =
        tes::anular_recibo(&e.unidad, &RelojFijo, &e.contador, r.recibo.id, "Prueba").await;
    assert!(matches!(normal, Err(AppError::Conflicto(_))), "{normal:?}");

    rec::anular_recaudo(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        r.recibo.id,
        "Pago duplicado",
    )
    .await
    .expect("anular");

    assert_eq!(e.saldo(e.bienes[1]).await, dec!(303000));
    let cuadre = rec::cuadre_cartera(&e.unidad, &RelojFijo, &e.contador)
        .await
        .expect("cuadre");
    assert_eq!(cuadre[0].diferencia, Decimal::ZERO);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_pagar_a_tiempo_no_hay_pronto_pago(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");

    let r = e
        .recaudar(e.bienes[1], fecha(2026, 11, 6), 273_000)
        .await
        .expect("recaudo");

    assert_eq!(r.descuento, None);
    assert_eq!(e.saldo(e.bienes[1]).await, dec!(30000));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_liquidacion_sin_pagos_se_anula_y_se_puede_repetir(pool: PgPool) {
    let e = entorno(pool).await;
    let l = e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");

    let anulada = liq::anular_liquidacion(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        l.liquidacion.id,
        "Presupuesto equivocado",
    )
    .await
    .expect("anular");

    assert_eq!(anulada.estado, EstadoLiquidacionExpensas::Anulada);
    assert_eq!(e.saldo(e.bienes[0]).await, Decimal::ZERO);
    let otra = e.liquidar(fecha(2026, 11, 1)).await.expect("repetir");
    assert_eq!(otra.liquidacion.total.valor(), dec!(1010000));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_liquidacion_con_pagos_no_se_anula(pool: PgPool) {
    let e = entorno(pool).await;
    let l = e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");
    e.recaudar(e.bienes[2], fecha(2026, 11, 20), 1_000)
        .await
        .expect("recaudo");

    let r = liq::anular_liquidacion(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        l.liquidacion.id,
        "Presupuesto equivocado",
    )
    .await;

    assert!(matches!(r, Err(AppError::Conflicto(_))), "{r:?}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn cobro_manual_y_nota_credito(pool: PgPool) {
    let e = entorno(pool).await;

    let cobro = rec::registrar_cobro(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        NuevoCobro {
            bien: e.bienes[2],
            concepto: e.multa.id,
            valor: m(50_000),
            fecha: fecha(2026, 11, 13),
            vencimiento: fecha(2026, 11, 30),
            descripcion: "Ruido después de las 10 p. m.".into(),
        },
    )
    .await
    .expect("cobro");
    rec::registrar_nota_credito(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        cobro.obligacion.id,
        m(20_000),
        "Rebaja aprobada por el consejo",
        None,
    )
    .await
    .expect("nota");
    let excesiva = rec::registrar_nota_credito(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        cobro.obligacion.id,
        m(40_000),
        "Rebaja aprobada por el consejo",
        None,
    )
    .await;
    let con_ajustes = rec::anular_cobro(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        cobro.obligacion.id,
        "Error",
    )
    .await;
    let ordinario = rec::registrar_cobro(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        NuevoCobro {
            bien: e.bienes[2],
            concepto: e.administracion.id,
            valor: m(50_000),
            fecha: fecha(2026, 11, 13),
            vencimiento: fecha(2026, 11, 30),
            descripcion: "Administración a mano".into(),
        },
    )
    .await;

    assert_eq!(e.saldo(e.bienes[2]).await, dec!(30000));
    assert!(
        matches!(excesiva, Err(AppError::Validacion(_))),
        "{excesiva:?}"
    );
    assert!(
        matches!(con_ajustes, Err(AppError::Conflicto(_))),
        "{con_ajustes:?}"
    );
    assert!(
        matches!(ordinario, Err(AppError::Validacion(_))),
        "{ordinario:?}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_cartera_inicial_con_errores_no_carga_nada(pool: PgPool) {
    let e = entorno(pool).await;
    let fila = |bien: &str| FilaCarteraInicial {
        bien: bien.into(),
        concepto: "administracion".into(),
        periodo: fecha(2026, 9, 1),
        valor: m(120_000),
        vencimiento: fecha(2026, 9, 10),
    };

    let con_error = rec::cargar_cartera_inicial(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        vec![fila("101"), fila("999")],
        false,
    )
    .await
    .expect("validar");
    let buena =
        rec::cargar_cartera_inicial(&e.unidad, &RelojFijo, &e.contador, vec![fila("101")], false)
            .await
            .expect("cargar");

    assert_eq!(con_error.errores, vec![(2, "el bien no existe".to_owned())]);
    assert!(!con_error.cargada);
    assert!(buena.cargada);
    let estado = rec::estado_cuenta(&e.unidad, &RelojFijo, &e.contador, e.bienes[0], None)
        .await
        .expect("estado");
    assert_eq!(estado.saldo, dec!(120000));
    assert_eq!(estado.vencido, dec!(120000));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_modulo_de_contribucion_solo_cobra_a_sus_bienes(pool: PgPool) {
    let e = entorno(pool).await;
    let modulo = cfg::guardar_modulo_contribucion(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        None,
        "Torre con ascensor",
        None,
        true,
    )
    .await
    .expect("módulo");
    let cuadro = coeficientes::crear_cuadro(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        NuevoCuadro {
            modulo: Some(modulo.id),
            ..NuevoCuadro::default()
        },
    )
    .await
    .expect("cuadro del módulo");
    let mitad = Coeficiente::new(dec!(0.5)).expect("coeficiente");
    coeficientes::cargar_coeficientes(
        &e.unidad,
        &e.contador,
        cuadro.id,
        vec![
            (RefBienPrivado::Id(e.bienes[0]), mitad),
            (RefBienPrivado::Id(e.bienes[1]), mitad),
        ],
    )
    .await
    .expect("cargar");
    coeficientes::activar_cuadro(&e.unidad, &RelojFijo, &e.contador, cuadro.id)
        .await
        .expect("activar");
    let mut ascensor = concepto(
        "ascensor",
        TipoConceptoCobro::Ordinaria,
        CriterioDistribucion::Modulo,
        "administracion",
    );
    ascensor.modulo = Some(modulo.id);
    let ascensor = cfg::crear_concepto_cobro(&e.unidad, &RelojFijo, &e.contador, ascensor)
        .await
        .expect("concepto del módulo");
    let gasto = cuenta_contable(&e.pool, e.contador.tenant_id, "5135").await;
    let rubro = |concepto, valor| NuevoRubro {
        concepto,
        cuenta: gasto,
        centro_costo: None,
        nombre: "Rubro".into(),
        valores_mensuales: vec![m(valor); 12],
    };
    let v2 = pres::crear_presupuesto(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        2026,
        None,
        vec![
            rubro(e.administracion.id, 1_000_000),
            rubro(ascensor.id, 200_000),
        ],
    )
    .await
    .expect("presupuesto v2");
    pres::aprobar_presupuesto(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        v2.id,
        fecha(2026, 11, 1),
        None,
    )
    .await
    .expect("aprobar v2");

    let l = e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");

    let (_, obligaciones) = liq::liquidacion(&e.unidad, &RelojFijo, &e.contador, l.liquidacion.id)
        .await
        .expect("detalle");
    let del_ascensor: Vec<_> = obligaciones
        .iter()
        .filter(|o| o.obligacion.concepto_cobro_id == ascensor.id)
        .collect();
    assert_eq!(del_ascensor.len(), 2);
    assert!(
        del_ascensor
            .iter()
            .all(|o| o.obligacion.bien_privado_id != e.bienes[2])
    );
    assert_eq!(
        del_ascensor
            .iter()
            .map(|o| o.obligacion.valor.valor())
            .sum::<Decimal>(),
        dec!(200000)
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_cuota_extraordinaria_se_cobra_en_su_calendario(pool: PgPool) {
    let e = entorno(pool).await;
    let extra = cfg::crear_concepto_cobro(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        concepto(
            "fachada",
            TipoConceptoCobro::Extraordinaria,
            CriterioDistribucion::Coeficiente,
            "extraordinaria",
        ),
    )
    .await
    .expect("concepto");
    liq::crear_cuota_extraordinaria(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        NuevaCuota {
            concepto: extra.id,
            finalidad: "Pintura de la fachada".into(),
            valor_total: m(3_000_000),
            numero_cuotas: 2,
            primer_periodo: fecha(2026, 11, 1),
            destinacion_especifica: true,
            pregunta: None,
        },
    )
    .await
    .expect("cuota");

    let l = e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");

    assert_eq!(l.liquidacion.total.valor(), dec!(2510000));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_recaudo_bancario_se_identifica_por_la_referencia_del_bien(pool: PgPool) {
    let e = entorno(pool).await;
    cfg::guardar_datos_pago(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.bienes[0],
        Some("101"),
        None,
    )
    .await
    .expect("referencia");
    let repetida = cfg::guardar_datos_pago(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.bienes[1],
        Some("101"),
        None,
    )
    .await;
    assert!(
        matches!(repetida, Err(AppError::Conflicto(_))),
        "{repetida:?}"
    );
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");
    let csv = "Fecha,Descripcion,Referencia,Valor
2026-11-08,Recaudo,0000000101,505000
2026-11-09,Consignacion,999,1000";
    conc::importar_extracto(
        &e.unidad,
        &AlmacenNulo,
        &LectorCsv,
        &RelojFijo,
        &e.contador,
        ImportacionExtracto {
            cuenta_tesoreria: e.banco.id,
            nombre: NombreArchivo::new("extracto.csv").expect("nombre"),
            contenido: csv.as_bytes().to_vec(),
            formato: FormatoEntrada::Planilla(MapeoColumnas {
                fila_encabezado: 1,
                fecha: "Fecha".into(),
                formato_fecha: "%Y-%m-%d".into(),
                descripcion: "Descripcion".into(),
                referencia: Some("Referencia".into()),
                valor: Some("Valor".into()),
                debito: None,
                credito: None,
                separador_decimal: SeparadorDecimal::Punto,
            }),
            saldo_inicial: None,
            saldo_final: None,
        },
    )
    .await
    .expect("extracto");

    let r = rec::recaudar_por_referencia(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.banco.id,
        fecha(2026, 11, 1),
        fecha(2026, 11, 30),
    )
    .await
    .expect("recaudar");

    assert_eq!(r.recaudos.len(), 1);
    assert_eq!(r.sin_identificar, 1);
    assert_eq!(e.saldo(e.bienes[0]).await, Decimal::ZERO);
    let conciliados: i64 =
        sqlx::query_scalar("SELECT count(*) FROM movimientos_extracto WHERE estado = 'conciliado'")
            .fetch_one(&e.pool)
            .await
            .expect("contar");
    assert_eq!(conciliados, 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_presupuesto_ni_politica_no_se_liquida(pool: PgPool) {
    let e = entorno(pool).await;

    let antes_de_la_politica = e.liquidar(fecha(2025, 12, 1)).await;
    let mut excesiva = politica();
    excesiva.factor_mora = dec!(1.6);
    let tope = cfg::crear_politica_cartera(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        fecha(2027, 1, 1),
        excesiva,
        None,
    )
    .await;

    assert!(
        matches!(antes_de_la_politica, Err(AppError::Validacion(_))),
        "{antes_de_la_politica:?}"
    );
    assert!(
        matches!(
            tope,
            Err(AppError::Dominio(DominioError::DatoInvalido { .. }))
        ),
        "{tope:?}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_ejecucion_presupuestal_compara_contra_el_mayor(pool: PgPool) {
    let e = entorno(pool).await;
    let p = pres::presupuestos(&e.unidad, &RelojFijo, &e.contador, Some(2026))
        .await
        .expect("presupuestos")
        .remove(0);

    let ejecucion = pres::ejecucion_presupuestal(&e.unidad, &RelojFijo, &e.contador, p.id, 11)
        .await
        .expect("ejecución");

    assert_eq!(ejecucion.rubros.len(), 1);
    assert_eq!(ejecucion.rubros[0].presupuestado, dec!(11000000));
    assert_eq!(ejecucion.rubros[0].ejecutado, Decimal::ZERO);
    assert_eq!(ejecucion.rubros[0].cuenta_codigo, "5135");
    let sin_permiso = pres::presupuestos(
        &e.unidad,
        &RelojFijo,
        &Contexto::de_cuenta(
            e.contador.tenant_id,
            CuentaId::nuevo(),
            ConjuntoPermisos::vacio(),
        ),
        None,
    )
    .await;
    assert!(
        matches!(sin_permiso, Err(AppError::SinPermiso(_))),
        "{sin_permiso:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// Fase 5: mora, acuerdos, edades y certificados
// ---------------------------------------------------------------------------------------------

struct PdfFalso;

impl application::puertos::GeneradorActas for PdfFalso {
    async fn generar(
        &self,
        _: &application::puertos::actas::DatosActa,
    ) -> application::AppResult<Vec<u8>> {
        Ok(b"%PDF".to_vec())
    }

    async fn rotulos(
        &self,
        _: &application::puertos::actas::HojaRotulos,
    ) -> application::AppResult<Vec<u8>> {
        Ok(b"%PDF".to_vec())
    }

    async fn tabla(
        &self,
        _: &application::puertos::actas::TablaPdf,
    ) -> application::AppResult<Vec<u8>> {
        Ok(b"%PDF-tabla".to_vec())
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn los_intereses_se_causan_por_tramos_de_saldo_y_no_se_repiten(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");
    e.recaudar(e.bienes[0], fecha(2026, 11, 20), 300_000)
        .await
        .expect("abono");

    let c = mora::causar_intereses(&e.unidad, &RelojFijo, &e.contador, fecha(2026, 11, 30))
        .await
        .expect("causar");

    assert_eq!(c.calculo.bienes.len(), 3);
    let del_101 = c
        .calculo
        .bienes
        .iter()
        .find(|b| b.bien_id == e.bienes[0])
        .expect("101");
    let tramos: Vec<(u32, Decimal)> = del_101.detalle.iter().map(|d| (d.dias, d.base)).collect();
    assert_eq!(tramos, vec![(9, dec!(500000.00)), (11, dec!(200000.00))]);
    assert!(
        del_101
            .detalle
            .iter()
            .all(|d| d.tasa_efectiva_anual == dec!(30))
    );
    let otra =
        mora::causar_intereses(&e.unidad, &RelojFijo, &e.contador, fecha(2026, 11, 30)).await;
    assert!(matches!(otra, Err(AppError::Conflicto(_))), "{otra:?}");
    let (_, detalle) = mora::causacion(&e.unidad, &RelojFijo, &e.contador, c.causacion.id)
        .await
        .expect("detalle");
    assert_eq!(detalle.len(), 4);
    assert_eq!(e.saldo(e.bienes[0]).await, dec!(205000) + del_101.total);
    let cuadre = rec::cuadre_cartera(&e.unidad, &RelojFijo, &e.contador)
        .await
        .expect("cuadre");
    assert!(cuadre.iter().all(|c| c.diferencia.is_zero()));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_pago_cubre_primero_los_intereses(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");
    let c = mora::causar_intereses(&e.unidad, &RelojFijo, &e.contador, fecha(2026, 11, 30))
        .await
        .expect("causar");
    let interes_102 = c
        .calculo
        .bienes
        .iter()
        .find(|b| b.bien_id == e.bienes[1])
        .expect("102")
        .total;

    let r = e
        .recaudar(e.bienes[1], fecha(2026, 11, 30), 1_000)
        .await
        .expect("abono");

    let (_, obligaciones) = (
        0,
        rec::estado_cuenta(&e.unidad, &RelojFijo, &e.contador, e.bienes[1], None)
            .await
            .expect("estado")
            .obligaciones,
    );
    let interes = obligaciones
        .iter()
        .find(|o| o.obligacion.origen == domain::contabilidad_ph::OrigenObligacion::Interes)
        .expect("obligación de intereses");
    assert_eq!(r.aplicado[0].0, interes.obligacion.id);
    assert_eq!(interes.saldo, (interes_102 - dec!(1000)).max(Decimal::ZERO));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_acuerdo_aprobado_por_otra_cuenta_suspende_la_mora(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");
    let a = mora::proponer_acuerdo(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        mora::NuevoAcuerdo {
            bien: e.bienes[1],
            numero_cuotas: 3,
            primer_vencimiento: fecha(2026, 12, 15),
            fecha: None,
            observaciones: None,
        },
    )
    .await
    .expect("proponer");
    assert_eq!(a.valor.valor(), dec!(303000));
    assert_eq!(a.cuotas.len(), 3);

    let misma = mora::aprobar_acuerdo(&e.unidad, &RelojFijo, &e.contador, a.id).await;
    assert!(matches!(misma, Err(AppError::Conflicto(_))), "{misma:?}");
    let otra = Contexto::de_cuenta(
        e.contador.tenant_id,
        crear_cuenta(&e.pool, "tesorero").await,
        ConjuntoPermisos::todos(Ambito::Copropiedad),
    );
    mora::aprobar_acuerdo(&e.unidad, &RelojFijo, &otra, a.id)
        .await
        .expect("aprobar");
    let c = mora::causar_intereses(&e.unidad, &RelojFijo, &e.contador, fecha(2026, 11, 30))
        .await
        .expect("causar");

    let del_102 = c
        .calculo
        .bienes
        .iter()
        .find(|b| b.bien_id == e.bienes[1])
        .expect("102");
    assert_eq!(del_102.detalle.iter().map(|d| d.dias).sum::<u32>(), 2);
    let otro = mora::proponer_acuerdo(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        mora::NuevoAcuerdo {
            bien: e.bienes[1],
            numero_cuotas: 1,
            primer_vencimiento: fecha(2026, 12, 15),
            fecha: None,
            observaciones: None,
        },
    )
    .await;
    assert!(matches!(otro, Err(AppError::Conflicto(_))), "{otro:?}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_causacion_sin_pagos_se_anula_y_se_repite(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");
    let c = mora::causar_intereses(&e.unidad, &RelojFijo, &e.contador, fecha(2026, 11, 30))
        .await
        .expect("causar");

    mora::anular_causacion(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        c.causacion.id,
        "Tasa equivocada",
    )
    .await
    .expect("anular");

    assert_eq!(e.saldo(e.bienes[2]).await, dec!(202000));
    let otra = mora::causar_intereses(&e.unidad, &RelojFijo, &e.contador, fecha(2026, 11, 30))
        .await
        .expect("repetir");
    assert_eq!(otra.calculo.total, c.calculo.total);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_tasa_de_referencia_no_se_causan_intereses(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");

    let r = mora::causar_intereses(&e.unidad, &RelojFijo, &e.contador, fecha(2027, 1, 31)).await;

    assert!(
        matches!(r, Err(AppError::Dominio(DominioError::ReglaContable(_)))),
        "{r:?}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_cartera_por_edades_clasifica_por_dias_de_vencida(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");

    let (filas, total) = mora::cartera_por_edades(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        Some(fecha(2026, 12, 31)),
    )
    .await
    .expect("edades");

    assert_eq!(filas.len(), 3);
    assert_eq!(total.de_31_a_60, dec!(1010000));
    assert_eq!(total.total, dec!(1010000));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_certificado_se_verifica_por_su_codigo(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");

    let paz = mora::emitir_certificado(
        &e.unidad,
        &PdfFalso,
        &RelojFijo,
        &e.contador,
        e.bienes[0],
        domain::contabilidad_ph::mora::TipoCertificado::PazYSalvo,
    )
    .await;
    let deuda = mora::emitir_certificado(
        &e.unidad,
        &PdfFalso,
        &RelojFijo,
        &e.contador,
        e.bienes[0],
        domain::contabilidad_ph::mora::TipoCertificado::Deuda,
    )
    .await
    .expect("deuda");

    assert!(matches!(paz, Err(AppError::Conflicto(_))), "{paz:?}");
    assert_eq!(deuda.certificado.saldo, dec!(505000));
    assert_eq!(deuda.codigo.len(), 24);
    let valido = mora::verificar_certificado(
        &e.unidad,
        e.contador.tenant_id,
        &deuda.codigo.to_lowercase().replace('-', " "),
    )
    .await
    .expect("verificar");
    assert_eq!(
        valido.map(|v| v.tipo),
        Some(domain::contabilidad_ph::mora::TipoCertificado::Deuda)
    );
    let falso =
        mora::verificar_certificado(&e.unidad, e.contador.tenant_id, "0000-0000-0000-0000-0000")
            .await
            .expect("verificar");
    assert_eq!(falso, None);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_plataforma_carga_tasas_sin_solapes(pool: PgPool) {
    let plataforma = UnidadDeTrabajoPlataformaPg::new(pool_como_app(&pool).await);
    let tasa = |desde, hasta| mora::NuevaTasa {
        vigente_desde: desde,
        vigente_hasta: hasta,
        tasa_efectiva_anual: dec!(18.5),
        resolucion: "Resolución 0001".into(),
        fuente: "Superintendencia Financiera".into(),
    };

    mora::crear_tasa_referencia(
        &plataforma,
        &RelojFijo,
        &ContextoPlataforma::sistema(),
        tasa(fecha(2027, 1, 1), fecha(2027, 1, 31)),
    )
    .await
    .expect("enero");
    let solapada = mora::crear_tasa_referencia(
        &plataforma,
        &RelojFijo,
        &ContextoPlataforma::sistema(),
        tasa(fecha(2027, 1, 15), fecha(2027, 2, 28)),
    )
    .await;

    assert!(
        matches!(solapada, Err(AppError::Conflicto(_))),
        "{solapada:?}"
    );
    let tasas = mora::tasas_referencia_plataforma(&plataforma, &ContextoPlataforma::sistema())
        .await
        .expect("tasas");
    assert_eq!(tasas.len(), 1);
}

// ---------------------------------------------------------------------------------------------
// Fases 6 y 7: portal, integraciones e informes
// ---------------------------------------------------------------------------------------------

/// Cuenta del propietario del bien 101 (su persona queda vinculada a ella) sin permisos de
/// cartera.
async fn propietario_101(e: &Entorno) -> Contexto {
    let cuenta = crear_cuenta(&e.pool, "propietario101").await;
    sqlx::query(
        "UPDATE personas SET cuenta_id = $1 WHERE id = (SELECT persona_id FROM titularidades
             WHERE bien_privado_id = $2 AND vigente_hasta IS NULL)",
    )
    .bind(cuenta.as_uuid())
    .bind(e.bienes[0].as_uuid())
    .execute(&e.pool)
    .await
    .expect("vincular cuenta");
    Contexto::de_cuenta(e.contador.tenant_id, cuenta, ConjuntoPermisos::vacio())
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_propietario_ve_solo_lo_suyo_y_saca_su_paz_y_salvo(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");
    let yo = propietario_101(&e).await;

    let mios = portal::mis_bienes(&e.unidad, &RelojFijo, &yo)
        .await
        .expect("mis bienes");
    let ajeno = portal::mi_estado_cuenta(&e.unidad, &RelojFijo, &yo, e.bienes[1], None).await;
    let estado = portal::mi_estado_cuenta(&e.unidad, &RelojFijo, &yo, e.bienes[0], None)
        .await
        .expect("estado");
    let cuentas = portal::mis_cuentas_cobro(&e.unidad, &RelojFijo, &yo, e.bienes[0])
        .await
        .expect("cuentas");
    let pdf = portal::mi_cuenta_cobro_pdf(&e.unidad, &PdfFalso, &RelojFijo, &yo, cuentas[0].id)
        .await
        .expect("pdf");
    let con_deuda =
        portal::mi_paz_y_salvo(&e.unidad, &PdfFalso, &RelojFijo, &yo, e.bienes[0]).await;
    e.recaudar(e.bienes[0], fecha(2026, 11, 13), 505_000)
        .await
        .expect("pago");
    let paz = portal::mi_paz_y_salvo(&e.unidad, &PdfFalso, &RelojFijo, &yo, e.bienes[0])
        .await
        .expect("paz y salvo");

    assert_eq!(
        mios.iter().map(|b| b.id).collect::<Vec<_>>(),
        vec![e.bienes[0]]
    );
    assert!(matches!(ajeno, Err(AppError::NoEncontrado(_))), "{ajeno:?}");
    assert_eq!(estado.saldo, dec!(505000));
    assert_eq!(cuentas.len(), 1);
    assert!(!pdf.is_empty());
    assert!(
        matches!(con_deuda, Err(AppError::Conflicto(_))),
        "{con_deuda:?}"
    );
    assert_eq!(
        paz.certificado.tipo,
        domain::contabilidad_ph::mora::TipoCertificado::PazYSalvo
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_mora_restringe_y_libera_las_reservas(pool: PgPool) {
    let e = entorno(pool).await;
    sqlx::query(
        "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
         VALUES (uuidv7(), $1, 'bienes_comunes', now() - interval '1 day')",
    )
    .bind(e.contador.tenant_id.as_uuid())
    .execute(&e.pool)
    .await
    .expect("módulo");
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");

    let primera = integraciones::sincronizar_restricciones_mora(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        m(300_000),
    )
    .await
    .expect("sincronizar");
    e.recaudar(e.bienes[0], fecha(2026, 11, 13), 505_000)
        .await
        .expect("pago");
    let segunda = integraciones::sincronizar_restricciones_mora(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        m(300_000),
    )
    .await
    .expect("sincronizar");

    assert_eq!((primera.restringidos, primera.levantadas), (2, 0));
    assert_eq!((segunda.restringidos, segunda.levantadas), (0, 1));
    let vigentes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM restricciones_reserva WHERE hasta IS NULL OR hasta > now()",
    )
    .fetch_one(&e.pool)
    .await
    .expect("contar");
    assert_eq!(vigentes, 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn informes_del_fondo_flujo_proyecto_y_asamblea(pool: PgPool) {
    let e = entorno(pool).await;
    e.liquidar(fecha(2026, 11, 1)).await.expect("liquidar");
    e.recaudar(e.bienes[0], fecha(2026, 11, 13), 505_000)
        .await
        .expect("pago");

    let fondo = informes::estado_fondo(&e.unidad, &RelojFijo, &e.contador, None)
        .await
        .expect("fondo");
    let flujo = informes::flujo_caja(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        fecha(2026, 10, 1),
        fecha(2026, 11, 30),
    )
    .await
    .expect("flujo");
    let proyecto =
        informes::proyectar_presupuesto(&e.unidad, &RelojFijo, &e.contador, 2026, dec!(10))
            .await
            .expect("proyecto");
    let pdf =
        informes::informe_asamblea_pdf(&e.unidad, &PdfFalso, &RelojFijo, &e.contador, 2026, 11)
            .await
            .expect("informe");

    assert_eq!(fondo.liquidado, dec!(10000));
    assert_eq!(fondo.recaudado, dec!(5000));
    assert_eq!(fondo.porcentaje, dec!(0.08));
    assert!(!fondo.llego_a_la_mitad);
    assert_eq!(flujo.len(), 2);
    assert_eq!(flujo[1].entradas, dec!(505000));
    assert_eq!(flujo[1].saldo_final, dec!(505000));
    assert_eq!(proyecto.anio, 2027);
    assert_eq!(
        proyecto.rubros[0].valores_mensuales[0].valor(),
        dec!(1100000)
    );
    assert!(!pdf.is_empty());
}
