//! Pruebas de integración de tesorería, cuentas por pagar, facturas de venta y conciliación
//! (plan 13, ADR 0018) como `dad_app`: los casos de uso contra PostgreSQL y las garantías de la
//! base de datos.

use application::{
    AppError, Contexto, ContextoPlataforma,
    casos_uso::contabilidad::{
        conciliacion::{self as conc, FormatoEntrada, ImportacionExtracto},
        configuracion::{
            self as conf, DatosTipoComprobante, NuevaConfiguracion, NuevaCuenta, NuevaLineaRegla,
            NuevaRegla, NuevoTercero,
        },
        cuentas_por_pagar::{
            self as cxp, NuevaFacturaProveedor, NuevaLineaFacturaProveedor, NuevoConceptoRetencion,
        },
        reportes,
        tesoreria::{
            self as tes, EventoPasarelaVerificado, NuevaCuentaTesoreria, NuevoEgreso, NuevoGasto,
            NuevoRecibo, conceptos, eventos,
        },
        tributario,
        ventas::{self, NuevaFacturaVenta, NuevaLineaVenta},
    },
    puertos::{
        AlmacenArchivos, LectorPlanillas, Reloj,
        archivos::DescargaArchivo,
        contabilidad::{AgrupacionSaldos, DatosFiscales},
        tesoreria::{CuentaTesoreria, FiltroRecibos},
    },
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use domain::{
    Almacen, Ambito, ClaveArchivo, ConjuntoPermisos, CuentaId, Documento, DominioError,
    NombreArchivo, PersonaId, TenantId, TipoArchivo, TipoPersona,
    contabilidad::{
        Cara, CodigoCuenta, MarcoTecnico, Monto, OrigenCentro, OrigenTercero, RegimenIva,
        ResponsabilidadFiscal,
        extracto::{MapeoColumnas, SeparadorDecimal},
        tesoreria::{
            EstadoEgreso, EstadoEventoPasarela, EstadoFacturaProveedor, EstadoFacturaVenta,
            EstadoMovimientoExtracto, EstadoRecibo, ImpuestoRetencion, MedioEgreso, MedioRecibo,
            TipoCuentaBancaria, TipoCuentaTesoreria,
        },
        tributario::{CanalFacturacion, ClasificacionTributaria, ParametrosPerfil},
    },
};
use infra_postgres::{UnidadDeTrabajoPg, UnidadDeTrabajoPlataformaPg};
use rust_decimal::Decimal;
use sqlx::PgPool;

mod comun;
use comun::{MIGRACIONES, insertar_entidad, pool_como_app};

struct RelojFijo;

impl Reloj for RelojFijo {
    fn ahora(&self) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 13, 12, 0, 0)
            .single()
            .expect("fecha")
    }
}

/// Almacén que no guarda nada (las pruebas no leen los bytes).
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

/// CSV separado por comas, sin comillas (suficiente para las pruebas).
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
    tenant: TenantId,
    contador: Contexto,
    tesorero: Contexto,
    banco: CuentaTesoreria,
}

fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(a, m, d).expect("fecha")
}

fn m(valor: i64) -> Monto {
    Monto::new(Decimal::from(valor)).expect("monto")
}

fn d(valor: i64) -> Decimal {
    Decimal::from(valor)
}

fn cod(c: &str) -> CodigoCuenta {
    CodigoCuenta::new(c).expect("código")
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

/// Perfil tributario confirmado de agente de retención y responsable de IVA (plan 15: sin él no
/// se practican retenciones ni se factura con IVA).
async fn perfil_agente_retenedor(unidad: &UnidadDeTrabajoPg, contador: &Contexto) {
    let p = tributario::crear_perfil_tributario(
        unidad,
        &RelojFijo,
        contador,
        fecha(2026, 1, 1),
        ParametrosPerfil {
            responsabilidades: vec![],
            agente_retenedor: true,
            responsable_iva: true,
            gran_contribuyente: false,
            autorretenedor: false,
            contribuyente_renta: true,
            facturador_electronico: false,
            obligado_rub: false,
            municipios_ica: vec![],
            actividades_ica: vec![],
            canal_facturacion: CanalFacturacion::Ninguno,
        },
        None,
        None,
    )
    .await
    .expect("perfil");
    tributario::confirmar_perfil_tributario(unidad, &RelojFijo, contador, p.id)
        .await
        .expect("confirmar perfil");
}

/// Entidad con contabilidad, plan de cuentas, tipos, reglas de los eventos del plan 13 y una
/// cuenta bancaria.
async fn entorno(pool: PgPool) -> Entorno {
    let tenant = insertar_entidad(&pool, "Gimnasio Fuerza").await;
    sqlx::query(
        "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
         VALUES (uuidv7(), $1, 'contabilidad', now() - interval '1 day')",
    )
    .bind(tenant.as_uuid())
    .execute(&pool)
    .await
    .expect("módulo");
    sqlx::query("UPDATE entidades SET nit = '900123456-1' WHERE id = $1")
        .bind(tenant.as_uuid())
        .execute(&pool)
        .await
        .expect("nit");
    let todos = ConjuntoPermisos::todos(Ambito::Copropiedad);
    let contador =
        Contexto::de_cuenta(tenant, crear_cuenta(&pool, "contador").await, todos.clone());
    let tesorero = Contexto::de_cuenta(tenant, crear_cuenta(&pool, "tesorero").await, todos);
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);
    for (codigo, movimiento, tercero) in [
        ("1", false, false),
        ("11", false, false),
        ("1110", true, false),
        ("13", false, false),
        ("1305", true, true),
        ("2", false, false),
        ("23", false, false),
        ("2335", true, true),
        ("2365", false, false),
        ("236525", true, true),
        ("236599", true, true),
        ("2367", true, true),
        ("2368", true, true),
        ("24", false, false),
        ("2408", true, false),
        ("28", false, false),
        ("2805", true, false),
        ("3", false, false),
        ("36", false, false),
        ("3605", true, false),
        ("4", false, false),
        ("41", false, false),
        ("4170", true, false),
        ("5", false, false),
        ("51", false, false),
        ("5135", true, false),
        ("5195", true, false),
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
    for codigo in ["RC", "CE", "FC", "FV", "CC"] {
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
            eventos::RECIBO,
            "RC",
            &[
                (conceptos::RECIBIDO, D, "1110", false),
                (conceptos::APLICADO, C, "1305", true),
                (conceptos::SIN_APLICAR, C, "2805", false),
            ],
        ),
        regla(
            eventos::APLICACION,
            "CC",
            &[
                (conceptos::SIN_APLICAR, D, "2805", false),
                (conceptos::APLICADO, C, "1305", true),
            ],
        ),
        regla(
            eventos::EGRESO,
            "CE",
            &[
                (conceptos::PAGADO_FACTURAS, D, "2335", true),
                (conceptos::GASTO, D, "5195", false),
                (conceptos::GIRADO, C, "1110", false),
            ],
        ),
        regla(
            eventos::FACTURA_PROVEEDOR,
            "FC",
            &[
                (conceptos::GASTO, D, "5135", false),
                (conceptos::IVA_DESCONTABLE, D, "2408", false),
                ("retencion_renta", C, "236599", true),
                ("retencion_iva", C, "2367", true),
                ("retencion_ica", C, "2368", true),
                (conceptos::POR_PAGAR, C, "2335", true),
            ],
        ),
        regla(
            eventos::FACTURA_VENTA,
            "FV",
            &[
                (conceptos::POR_COBRAR, D, "1305", true),
                (conceptos::INGRESO, C, "4170", false),
                (conceptos::IVA_GENERADO, C, "2408", false),
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
            nombre: "Cuenta corriente".into(),
            banco: Some("Banco de prueba".into()),
            numero_cuenta: Some("123-456789-01".into()),
            tipo_cuenta: Some(TipoCuentaBancaria::Corriente),
            cuenta_contable: cod("1110"),
            destinacion: None,
        },
    )
    .await
    .expect("banco");
    perfil_agente_retenedor(&unidad, &contador).await;
    Entorno {
        pool,
        unidad,
        tenant,
        contador,
        tesorero,
        banco,
    }
}

impl Entorno {
    async fn tercero(&self, nombre: &str, documento: &str, fiscal: DatosFiscales) -> PersonaId {
        conf::crear_tercero(
            &self.unidad,
            &RelojFijo,
            &self.contador,
            NuevoTercero {
                tipo: TipoPersona::Juridica,
                nombre: nombre.into(),
                tipo_documento: None,
                documento: Documento::new(documento).expect("documento"),
                fiscal,
            },
        )
        .await
        .expect("tercero")
        .persona
        .id
    }

    async fn saldo(&self, cuenta: &str) -> Decimal {
        reportes::balance_prueba(
            &self.unidad,
            &RelojFijo,
            &self.contador,
            fecha(2026, 1, 1),
            fecha(2026, 12, 31),
            AgrupacionSaldos::default(),
        )
        .await
        .expect("balance")
        .into_iter()
        .filter(|f| f.codigo.as_str() == cuenta)
        .map(|f| f.saldo_final())
        .sum()
    }

    async fn factura_venta(
        &self,
        cliente: PersonaId,
        valor: i64,
        vence: NaiveDate,
    ) -> domain::FacturaVentaId {
        let f = ventas::crear_factura_venta(
            &self.unidad,
            &RelojFijo,
            &self.contador,
            NuevaFacturaVenta {
                persona: cliente,
                fecha: fecha(2026, 1, 2),
                vencimiento: vence,
                descripcion: "Alquiler del salón".into(),
                origen: None,
                lineas: vec![NuevaLineaVenta {
                    descripcion: "Salón social".into(),
                    cuenta: cod("4170"),
                    centro_costo: None,
                    cantidad: d(1),
                    valor_unitario: m(valor),
                    tarifa_iva: d(0),
                    clasificacion: ClasificacionTributaria::AlquilerServicio,
                }],
            },
        )
        .await
        .expect("factura");
        ventas::emitir_factura_venta(&self.unidad, &RelojFijo, &self.contador, f.id)
            .await
            .expect("emitir")
            .id
    }

    fn recibo(&self, persona: Option<PersonaId>, valor: i64) -> NuevoRecibo {
        NuevoRecibo {
            cuenta_tesoreria: self.banco.id,
            persona,
            fecha: fecha(2026, 3, 10),
            valor: m(valor),
            medio: MedioRecibo::Transferencia,
            referencia: Some("TRF-1".into()),
            descripcion: "Pago del cliente".into(),
            soporte_archivo_id: None,
            aplicaciones: Vec::new(),
        }
    }

    async fn concepto(
        &self,
        codigo: &str,
        impuesto: ImpuestoRetencion,
        tarifa: &str,
        desde: NaiveDate,
    ) {
        let plataforma = UnidadDeTrabajoPlataformaPg::new(pool_como_app(&self.pool).await);
        cxp::crear_concepto_retencion(
            &plataforma,
            &ContextoPlataforma::sistema(),
            NuevoConceptoRetencion {
                codigo: codigo.into(),
                impuesto,
                nombre: format!("Concepto {codigo}"),
                base_minima_uvt: if impuesto == ImpuestoRetencion::Iva {
                    d(0)
                } else {
                    d(4)
                },
                tarifa: tarifa.parse().expect("tarifa"),
                cuenta_sugerida: (impuesto == ImpuestoRetencion::Renta).then(|| cod("236525")),
                municipio_dane: None,
                fuente: "Catálogo de prueba".into(),
                vigente_desde: desde,
            },
        )
        .await
        .expect("concepto");
    }

    fn factura_proveedor(
        &self,
        proveedor: PersonaId,
        numero: &str,
        f: NaiveDate,
    ) -> NuevaFacturaProveedor {
        NuevaFacturaProveedor {
            persona: proveedor,
            prefijo: "FE".into(),
            numero: numero.into(),
            fecha: f,
            vencimiento: f,
            descripcion: "Mantenimiento de equipos".into(),
            origen: None,
            cufe: None,
            archivo_id: None,
            lineas: vec![NuevaLineaFacturaProveedor {
                descripcion: "Servicio".into(),
                cuenta: cod("5135"),
                centro_costo: None,
                base: m(1_000_000),
                iva: m(190_000),
                iva_descontable: true,
                retenciones: vec!["servicios".into(), "reteiva".into()],
            }],
            excepcion_duplicado: None,
        }
    }
}

fn responsable() -> DatosFiscales {
    DatosFiscales {
        regimen_iva: Some(RegimenIva::Responsable),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------------------------
// Recibos y facturas de venta
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "MIGRACIONES")]
async fn recibo_aplicado_paga_la_factura_y_cuadra_la_cartera(pool: PgPool) {
    let e = entorno(pool).await;
    let cliente = e
        .tercero("Cliente", "800111222", DatosFiscales::default())
        .await;
    let factura = e.factura_venta(cliente, 500_000, fecha(2026, 3, 31)).await;

    let recibo = tes::registrar_recibo(
        &e.unidad,
        &RelojFijo,
        &e.tesorero,
        NuevoRecibo {
            aplicaciones: vec![(factura, m(500_000))],
            ..e.recibo(Some(cliente), 500_000)
        },
    )
    .await
    .expect("recibo");

    let f = ventas::factura_venta(&e.unidad, &RelojFijo, &e.contador, factura)
        .await
        .expect("factura");
    assert_eq!(f.estado, EstadoFacturaVenta::Pagada);
    assert_eq!(f.numero, Some(1));
    assert_eq!(recibo.aplicaciones.len(), 1);
    assert_eq!(e.saldo("1110").await, d(500_000));
    assert_eq!(e.saldo("1305").await, d(0), "la cartera queda en cero");
    assert_eq!(e.saldo("4170").await, d(-500_000));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn no_se_aplica_mas_que_el_saldo_de_la_factura(pool: PgPool) {
    let e = entorno(pool).await;
    let cliente = e
        .tercero("Cliente", "800111222", DatosFiscales::default())
        .await;
    let factura = e.factura_venta(cliente, 100_000, fecha(2026, 3, 31)).await;

    let r = tes::registrar_recibo(
        &e.unidad,
        &RelojFijo,
        &e.tesorero,
        NuevoRecibo {
            aplicaciones: vec![(factura, m(150_000))],
            ..e.recibo(Some(cliente), 150_000)
        },
    )
    .await;

    assert!(matches!(r, Err(AppError::Validacion(_))), "{r:?}");
    assert_eq!(e.saldo("1110").await, d(0), "nada quedó a medias");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn recibo_no_identificado_se_aplica_despues_y_al_anularlo_todo_vuelve(pool: PgPool) {
    let e = entorno(pool).await;
    let cliente = e
        .tercero("Cliente", "800111222", DatosFiscales::default())
        .await;
    let factura = e.factura_venta(cliente, 300_000, fecha(2026, 3, 31)).await;
    let recibo = tes::registrar_recibo(&e.unidad, &RelojFijo, &e.tesorero, e.recibo(None, 300_000))
        .await
        .expect("recibo");
    assert_eq!(e.saldo("2805").await, d(-300_000), "queda por identificar");

    let aplicado = tes::aplicar_recibo(
        &e.unidad,
        &RelojFijo,
        &e.tesorero,
        recibo.id,
        factura,
        m(300_000),
        Some(fecha(2026, 3, 20)),
    )
    .await
    .expect("aplicar");

    assert_eq!(aplicado.persona_id, Some(cliente), "queda identificado");
    assert_eq!(e.saldo("2805").await, d(0));
    assert_eq!(e.saldo("1305").await, d(0));

    let anulado = tes::anular_recibo(
        &e.unidad,
        &RelojFijo,
        &e.tesorero,
        recibo.id,
        "Consignación errada",
    )
    .await
    .expect("anular");

    assert_eq!(anulado.estado, EstadoRecibo::Anulado);
    let f = ventas::factura_venta(&e.unidad, &RelojFijo, &e.contador, factura)
        .await
        .expect("factura");
    assert_eq!(f.estado, EstadoFacturaVenta::Emitida);
    assert_eq!(e.saldo("1110").await, d(0));
    assert_eq!(
        e.saldo("1305").await,
        d(300_000),
        "la factura vuelve a deberse"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn factura_de_venta_con_recaudo_no_se_anula_y_la_cartera_va_por_edades(pool: PgPool) {
    let e = entorno(pool).await;
    let cliente = e
        .tercero("Cliente", "800111222", DatosFiscales::default())
        .await;
    let vieja = e.factura_venta(cliente, 100_000, fecha(2026, 1, 15)).await;
    let nueva = e.factura_venta(cliente, 200_000, fecha(2026, 12, 31)).await;
    tes::registrar_recibo(
        &e.unidad,
        &RelojFijo,
        &e.tesorero,
        NuevoRecibo {
            aplicaciones: vec![(nueva, m(50_000))],
            ..e.recibo(Some(cliente), 50_000)
        },
    )
    .await
    .expect("abono");

    let anular =
        ventas::anular_factura_venta(&e.unidad, &RelojFijo, &e.contador, nueva, "Error").await;
    let cartera = ventas::cartera_por_edades(&e.unidad, &RelojFijo, &e.contador, fecha(2026, 5, 1))
        .await
        .expect("cartera");

    assert!(
        matches!(
            anular,
            Err(AppError::Dominio(DominioError::TransicionInvalida { .. }))
        ),
        "{anular:?}"
    );
    assert_eq!(cartera.len(), 1);
    assert_eq!(cartera[0].total, d(250_000));
    let tramos: Vec<_> = cartera[0]
        .tramos
        .iter()
        .map(|(t, v)| (t.as_str(), *v))
        .collect();
    assert_eq!(
        tramos,
        vec![("corriente", d(150_000)), ("mas_de_90", d(100_000))]
    );
    ventas::anular_factura_venta(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        vieja,
        "Error de digitación",
    )
    .await
    .expect("sin recaudos se anula");
    assert_eq!(e.saldo("1305").await, d(150_000));
}

// ---------------------------------------------------------------------------------------------
// Cuentas por pagar y egresos
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "MIGRACIONES")]
async fn factura_duplicada_se_rechaza_salvo_excepcion_autorizada(pool: PgPool) {
    let e = entorno(pool).await;
    e.concepto(
        "servicios",
        ImpuestoRetencion::Renta,
        "4",
        fecha(2026, 1, 1),
    )
    .await;
    e.concepto("reteiva", ImpuestoRetencion::Iva, "15", fecha(2026, 1, 1))
        .await;
    let proveedor = e.tercero("Proveedor", "900555666", responsable()).await;
    cxp::registrar_factura_proveedor(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.factura_proveedor(proveedor, "100", fecha(2026, 3, 5)),
    )
    .await
    .expect("primera");

    let repetida = cxp::registrar_factura_proveedor(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.factura_proveedor(proveedor, "100", fecha(2026, 3, 6)),
    )
    .await;
    let con_excepcion = cxp::registrar_factura_proveedor(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        NuevaFacturaProveedor {
            excepcion_duplicado: Some("El proveedor reinició su numeración".into()),
            ..e.factura_proveedor(proveedor, "100", fecha(2026, 3, 6))
        },
    )
    .await;

    assert!(
        matches!(repetida, Err(AppError::Conflicto(_))),
        "{repetida:?}"
    );
    let f = con_excepcion.expect("con excepción");
    assert!(f.excepcion_autorizada_por.is_some());
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_causacion_usa_la_tarifa_vigente_en_la_fecha_de_la_factura(pool: PgPool) {
    let e = entorno(pool).await;
    e.concepto(
        "servicios",
        ImpuestoRetencion::Renta,
        "4",
        fecha(2026, 1, 1),
    )
    .await;
    e.concepto(
        "servicios",
        ImpuestoRetencion::Renta,
        "6",
        fecha(2026, 4, 1),
    )
    .await;
    e.concepto("reteiva", ImpuestoRetencion::Iva, "15", fecha(2026, 1, 1))
        .await;
    let proveedor = e.tercero("Proveedor", "900555666", responsable()).await;

    let marzo = cxp::registrar_factura_proveedor(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.factura_proveedor(proveedor, "1", fecha(2026, 3, 31)),
    )
    .await
    .expect("marzo");
    let abril = cxp::registrar_factura_proveedor(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.factura_proveedor(proveedor, "2", fecha(2026, 4, 1)),
    )
    .await
    .expect("abril");
    cxp::causar_factura_proveedor(&e.unidad, &RelojFijo, &e.contador, marzo.id)
        .await
        .expect("causar");

    let renta = |f: &application::puertos::tesoreria::FacturaProveedor| {
        f.lineas[0]
            .retenciones
            .iter()
            .find(|r| r.impuesto == ImpuestoRetencion::Renta)
            .map(|r| r.valor.valor())
    };
    assert_eq!(renta(&marzo), Some(d(40_000)));
    assert_eq!(renta(&abril), Some(d(60_000)));
    assert_eq!(marzo.retenciones.valor(), d(40_000 + 28_500));
    assert_eq!(marzo.neto_pagar.valor(), d(1_190_000 - 68_500));
    assert_eq!(
        e.saldo("236525").await,
        d(-40_000),
        "va a la cuenta sugerida del catálogo"
    );
    assert_eq!(e.saldo("236599").await, d(0));
    assert_eq!(e.saldo("2367").await, d(-28_500));
    assert_eq!(e.saldo("2408").await, d(190_000));
    assert_eq!(e.saldo("2335").await, d(-1_121_500));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn autorretenedor_no_tiene_retencion_de_renta_y_el_certificado_suma(pool: PgPool) {
    let e = entorno(pool).await;
    e.concepto(
        "servicios",
        ImpuestoRetencion::Renta,
        "4",
        fecha(2026, 1, 1),
    )
    .await;
    e.concepto("reteiva", ImpuestoRetencion::Iva, "15", fecha(2026, 1, 1))
        .await;
    let autorretenedor = e
        .tercero(
            "Autorretenedor",
            "900777888",
            DatosFiscales {
                responsabilidades: vec![ResponsabilidadFiscal::Autorretenedor],
                ..responsable()
            },
        )
        .await;
    let proveedor = e.tercero("Proveedor", "900555666", responsable()).await;
    let auto = cxp::registrar_factura_proveedor(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.factura_proveedor(autorretenedor, "1", fecha(2026, 3, 5)),
    )
    .await
    .expect("factura");
    for numero in ["1", "2"] {
        let f = cxp::registrar_factura_proveedor(
            &e.unidad,
            &RelojFijo,
            &e.contador,
            e.factura_proveedor(proveedor, numero, fecha(2026, 3, 5)),
        )
        .await
        .expect("factura");
        cxp::causar_factura_proveedor(&e.unidad, &RelojFijo, &e.contador, f.id)
            .await
            .expect("causar");
    }

    let certificado = cxp::certificado_retenciones(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        proveedor,
        fecha(2026, 1, 1),
        fecha(2026, 12, 31),
    )
    .await
    .expect("certificado");

    assert!(
        auto.lineas[0]
            .retenciones
            .iter()
            .all(|r| r.impuesto != ImpuestoRetencion::Renta)
    );
    assert_eq!(certificado.entidad_nit.as_deref(), Some("900123456-1"));
    assert_eq!(certificado.filas.len(), 2);
    assert_eq!(certificado.total_retenido, d(2 * (40_000 + 28_500)));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn egreso_lo_autoriza_otra_cuenta_y_al_pagar_salda_la_factura(pool: PgPool) {
    let e = entorno(pool).await;
    e.concepto(
        "servicios",
        ImpuestoRetencion::Renta,
        "4",
        fecha(2026, 1, 1),
    )
    .await;
    e.concepto("reteiva", ImpuestoRetencion::Iva, "15", fecha(2026, 1, 1))
        .await;
    let proveedor = e.tercero("Proveedor", "900555666", responsable()).await;
    let factura = cxp::registrar_factura_proveedor(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.factura_proveedor(proveedor, "100", fecha(2026, 3, 5)),
    )
    .await
    .expect("factura");
    cxp::causar_factura_proveedor(&e.unidad, &RelojFijo, &e.contador, factura.id)
        .await
        .expect("causar");
    let egreso = tes::registrar_egreso(
        &e.unidad,
        &RelojFijo,
        &e.tesorero,
        NuevoEgreso {
            cuenta_tesoreria: e.banco.id,
            persona: proveedor,
            fecha: fecha(2026, 3, 15),
            medio: MedioEgreso::Transferencia,
            referencia: None,
            descripcion: "Pago factura 100".into(),
            soporte_archivo_id: None,
            facturas: vec![(factura.id, factura.neto_pagar)],
            gastos: vec![NuevoGasto {
                cuenta: cod("5195"),
                persona: None,
                centro_costo: None,
                valor: m(5_000),
                descripcion: "Comisión bancaria".into(),
            }],
        },
    )
    .await
    .expect("egreso");

    let por_quien_registro =
        tes::autorizar_egreso(&e.unidad, &RelojFijo, &e.tesorero, egreso.id).await;
    let sin_autorizar =
        tes::pagar_egreso(&e.unidad, &RelojFijo, &e.tesorero, egreso.id, None).await;
    tes::autorizar_egreso(&e.unidad, &RelojFijo, &e.contador, egreso.id)
        .await
        .expect("autorizar");
    let pagado = tes::pagar_egreso(&e.unidad, &RelojFijo, &e.tesorero, egreso.id, None)
        .await
        .expect("pagar");

    assert!(matches!(por_quien_registro, Err(AppError::Conflicto(_))));
    assert!(matches!(
        sin_autorizar,
        Err(AppError::Dominio(DominioError::TransicionInvalida { .. }))
    ));
    assert_eq!(pagado.estado, EstadoEgreso::Pagado);
    assert_eq!(pagado.valor.valor(), d(1_121_500 + 5_000));
    let f = cxp::factura_proveedor(&e.unidad, &RelojFijo, &e.contador, factura.id)
        .await
        .expect("factura");
    assert_eq!(f.estado, EstadoFacturaProveedor::Pagada);
    assert_eq!(e.saldo("2335").await, d(0));
    assert_eq!(e.saldo("1110").await, d(-1_126_500));

    tes::anular_egreso(
        &e.unidad,
        &RelojFijo,
        &e.tesorero,
        egreso.id,
        "Transferencia devuelta",
    )
    .await
    .expect("anular");
    let f = cxp::factura_proveedor(&e.unidad, &RelojFijo, &e.contador, factura.id)
        .await
        .expect("factura");
    assert_eq!(f.estado, EstadoFacturaProveedor::Causada);
    assert_eq!(e.saldo("2335").await, d(-1_121_500));
    assert_eq!(e.saldo("1110").await, d(0));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_base_de_datos_exige_que_otro_autorice_el_egreso(pool: PgPool) {
    let e = entorno(pool).await;
    let proveedor = e.tercero("Proveedor", "900555666", responsable()).await;
    let egreso = tes::registrar_egreso(
        &e.unidad,
        &RelojFijo,
        &e.tesorero,
        NuevoEgreso {
            cuenta_tesoreria: e.banco.id,
            persona: proveedor,
            fecha: fecha(2026, 3, 15),
            medio: MedioEgreso::Cheque,
            referencia: Some("000123".into()),
            descripcion: "Anticipo".into(),
            soporte_archivo_id: None,
            facturas: Vec::new(),
            gastos: vec![NuevoGasto {
                cuenta: cod("5195"),
                persona: Some(proveedor),
                centro_costo: None,
                valor: m(10_000),
                descripcion: "Anticipo".into(),
            }],
        },
    )
    .await
    .expect("egreso");

    let r = sqlx::query(
        "UPDATE egresos SET estado = 'autorizado', autorizado_por = registrado_por,
             autorizado_en = now() WHERE id = $1",
    )
    .bind(egreso.id.as_uuid())
    .execute(&e.pool)
    .await;

    assert!(r.is_err(), "CHECK autorizado_por <> registrado_por");
}

// ---------------------------------------------------------------------------------------------
// Extractos y conciliación
// ---------------------------------------------------------------------------------------------

fn mapeo() -> MapeoColumnas {
    MapeoColumnas {
        fila_encabezado: 1,
        fecha: "Fecha".into(),
        formato_fecha: "%Y-%m-%d".into(),
        descripcion: "Descripcion".into(),
        referencia: Some("Referencia".into()),
        valor: Some("Valor".into()),
        debito: None,
        credito: None,
        separador_decimal: SeparadorDecimal::Punto,
    }
}

impl Entorno {
    async fn importar(
        &self,
        csv: &str,
    ) -> Result<application::puertos::tesoreria::ExtractoBancario, AppError> {
        conc::importar_extracto(
            &self.unidad,
            &AlmacenNulo,
            &LectorCsv,
            &RelojFijo,
            &self.contador,
            ImportacionExtracto {
                cuenta_tesoreria: self.banco.id,
                nombre: NombreArchivo::new("extracto.csv").expect("nombre"),
                contenido: csv.as_bytes().to_vec(),
                formato: FormatoEntrada::Planilla(mapeo()),
                saldo_inicial: Some(d(0)),
                saldo_final: Some(d(380_000)),
            },
        )
        .await
    }
}

const EXTRACTO: &str = "Fecha,Descripcion,Referencia,Valor
2026-03-11,Transferencia recibida,TRF-1,500000
2026-03-12,Consignacion sin identificar,,80000
2026-03-15,Comision,,-200000";

#[sqlx::test(migrator = "MIGRACIONES")]
async fn reimportar_el_mismo_extracto_no_duplica(pool: PgPool) {
    let e = entorno(pool).await;

    let primero = e.importar(EXTRACTO).await.expect("importar");
    let segundo = e.importar(EXTRACTO).await;

    assert_eq!(primero.movimientos, 3);
    assert_eq!(primero.desde, fecha(2026, 3, 11));
    assert!(
        matches!(segundo, Err(AppError::Conflicto(_))),
        "{segundo:?}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn concilia_automatica_y_manualmente_y_el_informe_cuadra(pool: PgPool) {
    let e = entorno(pool).await;
    let cliente = e
        .tercero("Cliente", "800111222", DatosFiscales::default())
        .await;
    tes::registrar_recibo(
        &e.unidad,
        &RelojFijo,
        &e.tesorero,
        e.recibo(Some(cliente), 500_000),
    )
    .await
    .expect("recibo");
    let proveedor = e.tercero("Proveedor", "900555666", responsable()).await;
    let egreso = tes::registrar_egreso(
        &e.unidad,
        &RelojFijo,
        &e.tesorero,
        NuevoEgreso {
            cuenta_tesoreria: e.banco.id,
            persona: proveedor,
            fecha: fecha(2026, 3, 14),
            medio: MedioEgreso::Transferencia,
            referencia: None,
            descripcion: "Gastos".into(),
            soporte_archivo_id: None,
            facturas: Vec::new(),
            gastos: vec![
                NuevoGasto {
                    cuenta: cod("5195"),
                    persona: None,
                    centro_costo: None,
                    valor: m(150_000),
                    descripcion: "Gasto A".into(),
                },
                NuevoGasto {
                    cuenta: cod("5195"),
                    persona: None,
                    centro_costo: None,
                    valor: m(50_000),
                    descripcion: "Gasto B".into(),
                },
            ],
        },
    )
    .await
    .expect("egreso");
    tes::autorizar_egreso(&e.unidad, &RelojFijo, &e.contador, egreso.id)
        .await
        .expect("autorizar");
    tes::pagar_egreso(
        &e.unidad,
        &RelojFijo,
        &e.tesorero,
        egreso.id,
        Some(fecha(2026, 3, 13)),
    )
    .await
    .expect("pagar");
    e.importar(EXTRACTO).await.expect("importar");

    let automaticas = conc::conciliar_automaticamente_cuenta(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.banco.id,
        fecha(2026, 3, 1),
        fecha(2026, 3, 31),
        3,
    )
    .await
    .expect("automática");

    assert_eq!(
        automaticas.len(),
        2,
        "el recibo y el egreso casan por valor y fecha"
    );
    let informe = conc::informe_conciliacion(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.banco.id,
        fecha(2026, 3, 31),
    )
    .await
    .expect("informe");
    assert_eq!(informe.saldo_libros, d(300_000));
    assert_eq!(
        informe.extracto_pendientes.len(),
        1,
        "la consignación sin identificar"
    );
    assert_eq!(informe.diferencia, Some(d(0)));

    let pendiente = informe.extracto_pendientes[0].id;
    let recibo = conc::recibo_no_identificado(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        pendiente,
        "Consignación por identificar",
    )
    .await
    .expect("recibo no identificado");
    assert_eq!(recibo.persona_id, None);
    let informe = conc::informe_conciliacion(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.banco.id,
        fecha(2026, 3, 31),
    )
    .await
    .expect("informe");
    assert!(informe.extracto_pendientes.is_empty());
    assert!(informe.libros_pendientes.is_empty());
    assert_eq!(informe.saldo_libros, d(380_000));
    assert_eq!(informe.diferencia, Some(d(0)));

    let deshecha =
        conc::deshacer_conciliacion(&e.unidad, &RelojFijo, &e.contador, automaticas[0].id)
            .await
            .expect("deshacer");
    let otra_vez =
        conc::deshacer_conciliacion(&e.unidad, &RelojFijo, &e.contador, automaticas[0].id).await;
    assert!(matches!(otra_vez, Err(AppError::Conflicto(_))));
    let movs = conc::movimientos(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.banco.id,
        fecha(2026, 3, 1),
        fecha(2026, 3, 31),
        Some(EstadoMovimientoExtracto::Pendiente),
    )
    .await
    .expect("movimientos");
    assert_eq!(movs.len(), 1);
    assert_eq!(movs[0].id, deshecha.movimientos[0]);

    let lineas = conc::lineas_libros(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.banco.id,
        fecha(2026, 3, 1),
        fecha(2026, 3, 31),
        true,
    )
    .await
    .expect("líneas");
    let mal = conc::conciliar_manual(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.banco.id,
        vec![movs[0].id],
        vec![
            (lineas[0].comprobante_id, lineas[0].orden),
            (lineas[0].comprobante_id, lineas[0].orden),
        ],
    )
    .await;
    assert!(
        matches!(mal, Err(AppError::Validacion(_))),
        "partidas repetidas"
    );
    conc::conciliar_manual(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.banco.id,
        vec![movs[0].id],
        lineas.iter().map(|l| (l.comprobante_id, l.orden)).collect(),
    )
    .await
    .expect("manual");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn las_conciliaciones_no_se_modifican(pool: PgPool) {
    let e = entorno(pool).await;
    let cliente = e
        .tercero("Cliente", "800111222", DatosFiscales::default())
        .await;
    tes::registrar_recibo(
        &e.unidad,
        &RelojFijo,
        &e.tesorero,
        e.recibo(Some(cliente), 500_000),
    )
    .await
    .expect("recibo");
    e.importar(EXTRACTO).await.expect("importar");
    let hechas = conc::conciliar_automaticamente_cuenta(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        e.banco.id,
        fecha(2026, 3, 1),
        fecha(2026, 3, 31),
        3,
    )
    .await
    .expect("automática");

    let r = sqlx::query("DELETE FROM conciliaciones WHERE id = $1")
        .bind(hechas[0].id.as_uuid())
        .execute(&e.pool)
        .await;

    assert!(r.is_err(), "append-only");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn importa_un_recaudo_asobancaria(pool: PgPool) {
    let e = entorno(pool).await;
    let relleno = |s: String| format!("{s:<162}");
    let texto = [
        relleno(format!(
            "01{:0>10}20260305{:0>3}{:0>17}20260306",
            "900123456", "7", "1"
        )),
        relleno(format!("05{:0>13}{:0>4}", "77", "1")),
        relleno(format!(
            "06{:0>48}{:0>14}0102{:0>6}{:0>6}",
            "1001", 15_000_000, 1, 1
        )),
        relleno(format!("08{:0>9}{:0>18}{:0>4}", 1, 15_000_000, 1)),
        format!("09{:0>9}{:0>18}", 1, 15_000_000),
    ]
    .join("\n");

    let extracto = conc::importar_extracto(
        &e.unidad,
        &AlmacenNulo,
        &LectorCsv,
        &RelojFijo,
        &e.contador,
        ImportacionExtracto {
            cuenta_tesoreria: e.banco.id,
            nombre: NombreArchivo::new("recaudo.txt").expect("nombre"),
            contenido: texto.into_bytes(),
            formato: FormatoEntrada::Asobancaria2001,
            saldo_inicial: None,
            saldo_final: None,
        },
    )
    .await
    .expect("importar");

    assert_eq!(extracto.version_lector, "asobancaria_2001-1");
    assert_eq!(extracto.movimientos, 1);
}

// ---------------------------------------------------------------------------------------------
// Pasarela de pagos
// ---------------------------------------------------------------------------------------------

#[sqlx::test(migrator = "MIGRACIONES")]
async fn evento_repetido_de_la_pasarela_crea_un_solo_recibo_y_el_reverso_lo_anula(pool: PgPool) {
    let e = entorno(pool).await;
    let cliente = e
        .tercero("Cliente", "800111222", DatosFiscales::default())
        .await;
    let sistema = Contexto::new(e.tenant);
    let evento = |externo: &str, estado| EventoPasarelaVerificado {
        proveedor: "pasarela_prueba".into(),
        evento_externo: externo.into(),
        referencia: "PAGO-77".into(),
        estado,
        valor: m(120_000),
        comision: m(0),
        retenciones: m(0),
        cuenta_tesoreria: e.banco.id,
        persona: Some(cliente),
        ocurrido_en: Utc
            .with_ymd_and_hms(2026, 3, 10, 15, 0, 0)
            .single()
            .expect("fecha"),
    };

    let primero = tes::registrar_evento_pasarela(
        &e.unidad,
        &RelojFijo,
        &sistema,
        evento("ev-1", EstadoEventoPasarela::Aprobado),
    )
    .await
    .expect("evento");
    let reintento = tes::registrar_evento_pasarela(
        &e.unidad,
        &RelojFijo,
        &sistema,
        evento("ev-1", EstadoEventoPasarela::Aprobado),
    )
    .await
    .expect("reintento");

    assert_eq!(primero.id, reintento.id);
    let recibos = tes::recibos(&e.unidad, &RelojFijo, &e.contador, FiltroRecibos::default())
        .await
        .expect("recibos");
    assert_eq!(recibos.len(), 1);
    assert_eq!(recibos[0].medio, MedioRecibo::Pasarela);

    tes::registrar_evento_pasarela(
        &e.unidad,
        &RelojFijo,
        &sistema,
        evento("ev-2", EstadoEventoPasarela::Reversado),
    )
    .await
    .expect("reverso");
    let r = tes::recibo(&e.unidad, &RelojFijo, &e.contador, recibos[0].id)
        .await
        .expect("recibo");
    assert_eq!(r.estado, EstadoRecibo::Anulado);
    assert_eq!(e.saldo("1110").await, d(0));
}
