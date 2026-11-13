//! Pruebas de integración del núcleo contable (plan 12, ADR 0017) como `dad_app`: los casos de
//! uso contra PostgreSQL y las garantías de la base de datos (QA-001 a QA-004, 011, 017, 018).

use application::{
    AppError, Contexto, ContextoPlataforma,
    casos_uso::contabilidad::{
        comprobantes::{
            self as uc, BorradorComprobante, DocumentoOrigen, LineaEntrada, SaldosIniciales,
        },
        configuracion::{
            self as conf, DatosTipoComprobante, NuevaConfiguracion, NuevaCuenta, NuevaLineaRegla,
            NuevaRegla, NuevoTercero,
        },
        periodos, plantillas, reportes,
    },
    puertos::{
        Reloj, Transaccion, UnidadDeTrabajo,
        contabilidad::{AgrupacionSaldos, AmbitoPlantilla, DatosFiscales, EstadoPlantilla, Origen},
    },
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use domain::{
    Ambito, ConjuntoPermisos, CuentaId, Documento, DominioError, PersonaId, TenantId, TipoPersona,
    contabilidad::{
        Cara, CodigoCuenta, CuentaPlantilla, EstadoPeriodo, ImporteDocumento, MarcoTecnico, Monto,
        Naturaleza, OrigenCentro, OrigenTercero, PeriodoContable, PlantillaContable,
    },
};
use infra_postgres::{UnidadDeTrabajoPg, UnidadDeTrabajoPlataformaPg};
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

mod comun;
use comun::{MIGRACIONES, crear_copropiedad, insertar_entidad, pool_como_app};

struct RelojFijo;

impl Reloj for RelojFijo {
    fn ahora(&self) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 13, 12, 0, 0)
            .single()
            .expect("fecha")
    }
}

struct Entorno {
    pool: PgPool,
    unidad: UnidadDeTrabajoPg,
    tenant: TenantId,
    contador: Contexto,
    aprobador: Contexto,
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

fn linea(cuenta: &str, debito: i64, credito: i64, persona: Option<PersonaId>) -> LineaEntrada {
    LineaEntrada {
        cuenta: cod(cuenta),
        persona,
        centro_costo: None,
        debito: m(debito),
        credito: m(credito),
        base: None,
        descripcion: None,
    }
}

async fn crear_cuenta(pool: &PgPool, usuario: &str) -> CuentaId {
    let c = CuentaId::nuevo();
    sqlx::query(
        "INSERT INTO cuentas (id, nombre_mostrar, usuario, hash_contrasena, contrasena_actualizada_en, creada_en)
         VALUES ($1, 'Contador', $2, 'x', now(), now())",
    )
    .bind(c.as_uuid())
    .bind(usuario)
    .execute(pool)
    .await
    .expect("cuenta");
    c
}

async fn habilitar_contabilidad(pool: &PgPool, tenant: TenantId) {
    sqlx::query(
        "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
         VALUES (uuidv7(), $1, 'contabilidad', now() - interval '1 day')",
    )
    .bind(tenant.as_uuid())
    .execute(pool)
    .await
    .expect("módulo");
}

/// Entidad sin vertical con la contabilidad habilitada, plan de cuentas mínimo, tipos de
/// comprobante y configuración desde el 1 de enero de 2026.
async fn entorno(pool: PgPool) -> Entorno {
    let tenant = insertar_entidad(&pool, "Gimnasio Fuerza").await;
    habilitar_contabilidad(&pool, tenant).await;
    let todos = ConjuntoPermisos::todos(Ambito::Copropiedad);
    let contador =
        Contexto::de_cuenta(tenant, crear_cuenta(&pool, "contador").await, todos.clone());
    let aprobador = Contexto::de_cuenta(tenant, crear_cuenta(&pool, "aprobador").await, todos);
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);
    for (codigo, movimiento, tercero) in [
        ("1", false, false),
        ("11", false, false),
        ("1105", true, false),
        ("13", false, false),
        ("1305", true, true),
        ("3", false, false),
        ("36", false, false),
        ("3605", true, false),
        ("4", false, false),
        ("41", false, false),
        ("4170", true, false),
        ("5", false, false),
        ("51", false, false),
        ("5105", true, false),
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
    for (codigo, manual) in [("CC", true), ("AP", false), ("CI", false), ("FV", false)] {
        conf::guardar_tipo_comprobante(
            &unidad,
            &RelojFijo,
            &contador,
            None,
            DatosTipoComprobante {
                codigo: codigo.into(),
                nombre: format!("Tipo {codigo}"),
                manual,
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
    Entorno {
        pool,
        unidad,
        tenant,
        contador,
        aprobador,
    }
}

impl Entorno {
    async fn contabilizar(
        &self,
        fecha: NaiveDate,
        lineas: Vec<LineaEntrada>,
    ) -> Result<application::puertos::contabilidad::Comprobante, AppError> {
        let b = uc::crear_borrador(
            &self.unidad,
            &RelojFijo,
            &self.contador,
            BorradorComprobante {
                tipo: "CC".into(),
                fecha,
                descripcion: "Movimiento de prueba".into(),
                lineas,
            },
        )
        .await?;
        uc::contabilizar(&self.unidad, &RelojFijo, &self.contador, b.id).await
    }

    async fn saldo(&self, cuenta: &str, hasta: NaiveDate) -> Decimal {
        reportes::balance_prueba(
            &self.unidad,
            &RelojFijo,
            &self.contador,
            fecha(2026, 1, 1),
            hasta,
            AgrupacionSaldos::default(),
        )
        .await
        .expect("balance")
        .into_iter()
        .filter(|f| f.codigo.as_str() == cuenta)
        .map(|f| f.saldo_final())
        .sum()
    }

    async fn tercero(&self) -> PersonaId {
        conf::crear_tercero(
            &self.unidad,
            &RelojFijo,
            &self.contador,
            NuevoTercero {
                tipo: TipoPersona::Juridica,
                nombre: "Cliente S.A.S.".into(),
                tipo_documento: None,
                documento: Documento::new("900123456").expect("documento"),
                fiscal: DatosFiscales::default(),
            },
        )
        .await
        .expect("tercero")
        .persona
        .id
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn comprobante_manual_se_contabiliza_con_consecutivo_y_cuadra_el_balance(pool: PgPool) {
    let e = entorno(pool).await;

    let primero = e
        .contabilizar(
            fecha(2026, 3, 5),
            vec![linea("1105", 1000, 0, None), linea("4170", 0, 1000, None)],
        )
        .await
        .expect("primero");
    let segundo = e
        .contabilizar(
            fecha(2026, 3, 6),
            vec![linea("5105", 300, 0, None), linea("1105", 0, 300, None)],
        )
        .await
        .expect("segundo");

    assert_eq!((primero.numero, segundo.numero), (Some(1), Some(2)));
    assert_eq!(
        e.saldo("1105", fecha(2026, 3, 31)).await,
        Decimal::from(700)
    );
    let diario = reportes::libro_diario(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        fecha(2026, 3, 1),
        fecha(2026, 3, 31),
    )
    .await
    .expect("diario");
    assert_eq!(diario.len(), 2);
    let mayor = reportes::libro_mayor(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        &cod("1105"),
        None,
        fecha(2026, 3, 6),
        fecha(2026, 3, 31),
    )
    .await
    .expect("mayor");
    assert_eq!(
        (mayor.saldo_inicial, mayor.saldo_final),
        (Decimal::from(1000), Decimal::from(700))
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_comprobante_que_no_cuadra_no_se_contabiliza(pool: PgPool) {
    let e = entorno(pool).await;

    let err = e
        .contabilizar(
            fecha(2026, 3, 5),
            vec![linea("1105", 1000, 0, None), linea("4170", 0, 999, None)],
        )
        .await
        .expect_err("no cuadra");

    assert!(
        matches!(err, AppError::Dominio(DominioError::ReglaContable(_))),
        "{err:?}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_base_de_datos_tambien_exige_la_partida_doble(pool: PgPool) {
    let e = entorno(pool).await;
    let b = uc::crear_borrador(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        BorradorComprobante {
            tipo: "CC".into(),
            fecha: fecha(2026, 3, 5),
            descripcion: "Descuadrado".into(),
            lineas: vec![linea("1105", 10, 0, None), linea("4170", 0, 9, None)],
        },
    )
    .await
    .expect("un borrador puede estar incompleto");

    // Saltándose la aplicación: el trigger diferido lo rechaza al confirmar.
    let app = pool_como_app(&e.pool).await;
    let mut tx = app.begin().await.expect("tx");
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(e.tenant.as_uuid().to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant");
    sqlx::query("UPDATE comprobantes SET estado = 'contabilizado', numero = 99, contabilizado_en = now() WHERE id = $1")
        .bind(b.id.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("el UPDATE pasa; la verificación es al confirmar");
    let err = tx.commit().await.expect_err("partida doble");

    assert!(err.to_string().contains("partida doble"), "{err}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn lo_contabilizado_no_se_edita_ni_se_borra(pool: PgPool) {
    let e = entorno(pool).await;
    let c = e
        .contabilizar(
            fecha(2026, 3, 5),
            vec![linea("1105", 50, 0, None), linea("4170", 0, 50, None)],
        )
        .await
        .expect("ok");
    let app = pool_como_app(&e.pool).await;

    let mut resultados = Vec::new();
    for sql in [
        "UPDATE comprobantes SET descripcion = 'otra' WHERE id = $1",
        "DELETE FROM comprobantes WHERE id = $1",
        "UPDATE lineas_comprobante SET debito = 49 WHERE comprobante_id = $1 AND orden = 1",
        "DELETE FROM lineas_comprobante WHERE comprobante_id = $1",
    ] {
        let mut tx = app.begin().await.expect("tx");
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(e.tenant.as_uuid().to_string())
            .execute(&mut *tx)
            .await
            .expect("tenant");
        resultados.push(
            sqlx::query(sql)
                .bind(c.id.as_uuid())
                .execute(&mut *tx)
                .await
                .is_err(),
        );
    }
    let editar = uc::editar_borrador(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        c.id,
        BorradorComprobante {
            tipo: "CC".into(),
            fecha: fecha(2026, 3, 5),
            descripcion: "otra".into(),
            lineas: vec![],
        },
    )
    .await;

    assert_eq!(
        resultados,
        vec![true; 4],
        "la base de datos rechaza cada cambio"
    );
    assert!(matches!(editar, Err(AppError::Conflicto(_))));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn periodo_cerrado_rechaza_y_la_reapertura_la_aprueba_otra_cuenta(pool: PgPool) {
    let e = entorno(pool).await;
    let marzo = periodos::abrir_periodo(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        PeriodoContable::new(2026, 3).expect("periodo"),
    )
    .await
    .expect("abrir");
    periodos::cambiar_estado(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        marzo.id,
        EstadoPeriodo::Cerrado,
    )
    .await
    .expect("cerrar");

    let cerrado = e
        .contabilizar(
            fecha(2026, 3, 5),
            vec![linea("1105", 1, 0, None), linea("4170", 0, 1, None)],
        )
        .await;
    let directo = periodos::cambiar_estado(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        marzo.id,
        EstadoPeriodo::Abierto,
    )
    .await;
    let solicitud = periodos::solicitar_reapertura(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        marzo.id,
        "Ajuste de una factura",
        None,
    )
    .await
    .expect("solicitud");
    let propia =
        periodos::decidir_reapertura(&e.unidad, &RelojFijo, &e.contador, solicitud.id, true).await;
    periodos::decidir_reapertura(&e.unidad, &RelojFijo, &e.aprobador, solicitud.id, true)
        .await
        .expect("otra cuenta aprueba");
    let reabierto = e
        .contabilizar(
            fecha(2026, 3, 5),
            vec![linea("1105", 1, 0, None), linea("4170", 0, 1, None)],
        )
        .await;

    assert!(
        matches!(
            cerrado,
            Err(AppError::Dominio(DominioError::ReglaContable(_)))
        ),
        "{cerrado:?}"
    );
    assert!(matches!(directo, Err(AppError::Conflicto(_))));
    assert!(
        matches!(propia, Err(AppError::Conflicto(_))),
        "segregación de funciones"
    );
    reabierto.expect("el periodo reabierto recibe comprobantes");
    let auditadas: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM auditoria WHERE accion LIKE 'contabilidad.reapertura.%'",
    )
    .fetch_one(&e.pool)
    .await
    .expect("conteo");
    assert_eq!(auditadas, 2);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_contabilizador_aplica_la_regla_y_es_idempotente_por_origen(pool: PgPool) {
    let e = entorno(pool).await;
    let cliente = e.tercero().await;
    conf::crear_regla(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        NuevaRegla {
            evento: "ventas.factura".into(),
            tipo_comprobante: "FV".into(),
            descripcion: None,
            vigente_desde: fecha(2026, 1, 1),
            lineas: vec![
                NuevaLineaRegla {
                    concepto: "venta".into(),
                    cara: Cara::Debito,
                    cuenta: cod("1305"),
                    tercero: OrigenTercero::Documento,
                    centro: OrigenCentro::Ninguno,
                    centro_costo: None,
                },
                NuevaLineaRegla {
                    concepto: "venta".into(),
                    cara: Cara::Credito,
                    cuenta: cod("4170"),
                    tercero: OrigenTercero::Ninguno,
                    centro: OrigenCentro::Ninguno,
                    centro_costo: None,
                },
            ],
        },
    )
    .await
    .expect("regla");
    let origen = Origen {
        tipo: "ventas.factura".into(),
        id: Uuid::now_v7(),
    };
    let documento = || DocumentoOrigen {
        evento: "ventas.factura".into(),
        origen: origen.clone(),
        fecha: fecha(2026, 4, 10),
        descripcion: "Factura FV-1".into(),
        importes: vec![ImporteDocumento {
            concepto: "venta".into(),
            monto: m(250),
            persona: Some(cliente),
            centro_costo: None,
            descripcion: None,
        }],
    };

    let mut ids = Vec::new();
    for _ in 0..2 {
        // El módulo que produce el documento contabiliza en su propia transacción.
        let mut tx = e.unidad.iniciar(&e.contador).await.expect("tx");
        ids.push(
            uc::contabilizar_documento(&mut tx, &RelojFijo, &e.contador, documento())
                .await
                .expect("contabiliza"),
        );
        tx.confirmar().await.expect("confirma");
    }

    assert_eq!(ids[0], ids[1], "un reintento devuelve el mismo comprobante");
    let c = uc::comprobante(&e.unidad, &RelojFijo, &e.contador, ids[0])
        .await
        .expect("comprobante");
    assert_eq!(c.lineas[0].persona, Some(cliente));
    assert_eq!(c.lineas[1].persona, None);
    assert_eq!(
        e.saldo("1305", fecha(2026, 4, 30)).await,
        Decimal::from(250)
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_cuenta_que_exige_tercero_no_acepta_lineas_sin_el(pool: PgPool) {
    let e = entorno(pool).await;

    let err = e
        .contabilizar(
            fecha(2026, 3, 5),
            vec![linea("1305", 10, 0, None), linea("4170", 0, 10, None)],
        )
        .await
        .expect_err("falta el tercero");

    assert!(matches!(err, AppError::Dominio(DominioError::ReglaContable(t)) if t.contains("1305")));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_reverso_anula_el_efecto_y_no_se_revierte_dos_veces(pool: PgPool) {
    let e = entorno(pool).await;
    let c = e
        .contabilizar(
            fecha(2026, 3, 5),
            vec![linea("1105", 80, 0, None), linea("4170", 0, 80, None)],
        )
        .await
        .expect("ok");

    let reverso = uc::revertir(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        c.id,
        "Se registró dos veces",
        Some(fecha(2026, 3, 20)),
    )
    .await
    .expect("revierte");
    let otra_vez = uc::revertir(&e.unidad, &RelojFijo, &e.contador, c.id, "Otra vez", None).await;

    assert_eq!(reverso.revierte_a, Some(c.id));
    assert_eq!(e.saldo("1105", fecha(2026, 3, 31)).await, Decimal::ZERO);
    assert!(matches!(otra_vez, Err(AppError::Conflicto(_))));
    let original = uc::comprobante(&e.unidad, &RelojFijo, &e.contador, c.id)
        .await
        .expect("original");
    assert_eq!(original.revertido_por, Some(reverso.id));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn saldos_iniciales_y_cierre_anual_con_estados_financieros(pool: PgPool) {
    let e = entorno(pool).await;
    uc::registrar_saldos_iniciales(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        SaldosIniciales {
            fecha: fecha(2026, 1, 1),
            tipo: "AP".into(),
            descripcion: "Apertura".into(),
            lineas: vec![linea("1105", 500, 0, None), linea("3605", 0, 500, None)],
        },
    )
    .await
    .expect("apertura");
    e.contabilizar(
        fecha(2026, 3, 5),
        vec![linea("1105", 1000, 0, None), linea("4170", 0, 1000, None)],
    )
    .await
    .expect("ingreso");
    e.contabilizar(
        fecha(2026, 3, 6),
        vec![linea("5105", 600, 0, None), linea("1105", 0, 600, None)],
    )
    .await
    .expect("gasto");
    conf::reemplazar_rubros(
        &e.unidad,
        &RelojFijo,
        &e.contador,
        vec![
            domain::contabilidad::RubroPlantilla {
                estado: "situacion_financiera".into(),
                codigo: "efectivo".into(),
                nombre: "Efectivo".into(),
                orden: 1,
                prefijos: vec!["11".into()],
            },
            domain::contabilidad::RubroPlantilla {
                estado: "resultados".into(),
                codigo: "ingresos".into(),
                nombre: "Ingresos".into(),
                orden: 1,
                prefijos: vec!["4".into()],
            },
        ],
    )
    .await
    .expect("rubros");

    let sin_cerrar = uc::cierre_anual(&e.unidad, &RelojFijo, &e.contador, 2026, "CI").await;
    for p in periodos::periodos(&e.unidad, &RelojFijo, &e.contador, Some(2026))
        .await
        .expect("periodos")
    {
        periodos::cambiar_estado(
            &e.unidad,
            &RelojFijo,
            &e.contador,
            p.id,
            EstadoPeriodo::Cerrado,
        )
        .await
        .expect("cerrar");
    }
    let cierre = uc::cierre_anual(&e.unidad, &RelojFijo, &e.contador, 2026, "CI")
        .await
        .expect("cierre");
    let repetido = uc::cierre_anual(&e.unidad, &RelojFijo, &e.contador, 2026, "CI").await;
    let estados =
        reportes::estados_financieros(&e.unidad, &RelojFijo, &e.contador, fecha(2026, 12, 31))
            .await
            .expect("estados");

    assert!(
        matches!(sin_cerrar, Err(AppError::Conflicto(_))),
        "{sin_cerrar:?}"
    );
    assert_eq!(cierre.mes_de_prueba(), 13);
    assert!(matches!(repetido, Err(AppError::Conflicto(_))));
    assert_eq!(e.saldo("4170", fecha(2026, 12, 31)).await, Decimal::ZERO);
    assert_eq!(e.saldo("5105", fecha(2026, 12, 31)).await, Decimal::ZERO);
    assert_eq!(
        e.saldo("3605", fecha(2026, 12, 31)).await,
        Decimal::from(-900),
        "500 de apertura + 400 de excedente"
    );
    assert_eq!(estados.resultado_ejercicio, Decimal::from(400));
    assert!(estados.cierre_contabilizado);
    assert_eq!(estados.situacion_financiera[0].valor, Decimal::from(900));
    assert_eq!(
        estados.resultados[0].valor,
        Decimal::from(1000),
        "el estado de resultados no muestra el cierre"
    );
}

trait MesDePrueba {
    fn mes_de_prueba(&self) -> u8;
}

impl MesDePrueba for application::puertos::contabilidad::Comprobante {
    fn mes_de_prueba(&self) -> u8 {
        self.periodo.mes
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn aplicar_plantilla_crea_lo_que_falta_y_la_vista_previa_no_cambia_nada(pool: PgPool) {
    let e = entorno(pool).await;
    let plataforma = UnidadDeTrabajoPlataformaPg::new(pool_como_app(&e.pool).await);
    let sistema = ContextoPlataforma::sistema();
    let cuenta = |codigo: &str, movimiento: bool| CuentaPlantilla {
        codigo: cod(codigo),
        nombre: format!("Plantilla {codigo}"),
        naturaleza: Naturaleza::Debito,
        acepta_movimiento: movimiento,
        exige_tercero: false,
        exige_centro_costo: false,
    };
    let p = plantillas::crear_plantilla(
        &plataforma,
        &RelojFijo,
        &sistema,
        AmbitoPlantilla::Plataforma,
        "Estándar gimnasio",
        "gimnasio",
        PlantillaContable {
            cuentas: vec![
                cuenta("1", false),
                cuenta("11", false),
                cuenta("1105", true),
                cuenta("1110", true),
            ],
            ..Default::default()
        },
    )
    .await
    .expect("plantilla");
    let borrador = conf::aplicar_plantilla(&e.unidad, &RelojFijo, &e.contador, p.id, true).await;
    plantillas::cambiar_estado_plantilla(
        &plataforma,
        &RelojFijo,
        &sistema,
        p.id,
        EstadoPlantilla::Publicada,
    )
    .await
    .expect("publica");

    let vista = conf::aplicar_plantilla(&e.unidad, &RelojFijo, &e.contador, p.id, true)
        .await
        .expect("vista previa");
    let antes = conf::cuentas(&e.unidad, &RelojFijo, &e.contador)
        .await
        .expect("cuentas")
        .len();
    let aplicada = conf::aplicar_plantilla(&e.unidad, &RelojFijo, &e.contador, p.id, false)
        .await
        .expect("aplica");
    let despues = conf::cuentas(&e.unidad, &RelojFijo, &e.contador)
        .await
        .expect("cuentas");

    assert!(
        matches!(borrador, Err(AppError::NoEncontrado(_))),
        "un borrador no se aplica"
    );
    assert_eq!((vista.cuentas_creadas, vista.cuentas_existentes), (1, 3));
    assert_eq!(antes, 14, "la vista previa no cambió nada");
    assert_eq!(aplicada.cuentas_creadas, 1);
    assert!(despues.iter().any(|c| c.codigo.as_str() == "1110"));
    let exportada = conf::exportar_plantilla(&e.unidad, &RelojFijo, &e.contador)
        .await
        .expect("exporta");
    assert_eq!(exportada.cuentas.len(), 15);
    assert_eq!(exportada.cuenta_resultado, Some(cod("3605")));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn otra_entidad_no_ve_la_contabilidad(pool: PgPool) {
    let e = entorno(pool).await;
    let c = e
        .contabilizar(
            fecha(2026, 3, 5),
            vec![linea("1105", 5, 0, None), linea("4170", 0, 5, None)],
        )
        .await
        .expect("ok");
    let otra = insertar_entidad(&e.pool, "Otra empresa").await;
    habilitar_contabilidad(&e.pool, otra).await;
    let ajeno = Contexto::de_cuenta(
        otra,
        e.contador.actor_id.expect("actor"),
        ConjuntoPermisos::todos(Ambito::Copropiedad),
    );

    let ver = uc::comprobante(&e.unidad, &RelojFijo, &ajeno, c.id).await;
    let cuentas = conf::cuentas(&e.unidad, &RelojFijo, &ajeno)
        .await
        .expect("cuentas");

    assert!(matches!(ver, Err(AppError::NoEncontrado(_))));
    assert!(cuentas.is_empty());
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_modulo_no_hay_contabilidad(pool: PgPool) {
    let tenant = insertar_entidad(&pool, "Sin módulo").await;
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);
    let ctx = Contexto::de_cuenta(
        tenant,
        crear_cuenta(&pool, "sinmodulo").await,
        ConjuntoPermisos::todos(Ambito::Copropiedad),
    );

    let err = conf::cuentas(&unidad, &RelojFijo, &ctx).await;

    assert!(matches!(
        err,
        Err(AppError::ModuloNoHabilitado(domain::Modulo::Contabilidad))
    ));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_copropiedad_lleva_su_contabilidad_con_el_mismo_id(pool: PgPool) {
    let ph = crear_copropiedad(&pool).await;
    habilitar_contabilidad(&pool, ph).await;
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);
    let ctx = Contexto::de_cuenta(
        ph,
        crear_cuenta(&pool, "contadorph").await,
        ConjuntoPermisos::todos(Ambito::Copropiedad),
    );

    let clase = conf::crear_cuenta(
        &unidad,
        &RelojFijo,
        &ctx,
        NuevaCuenta {
            codigo: cod("1"),
            nombre: "Activo".into(),
            naturaleza: None,
            acepta_movimiento: false,
            exige_tercero: false,
            exige_centro_costo: false,
        },
    )
    .await
    .expect("la PH usa el núcleo contable como cualquier entidad");

    assert_eq!(
        clase.naturaleza,
        Naturaleza::Debito,
        "tomada del catálogo PUC"
    );
}
