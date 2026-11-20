//! Integración de la nómina (plan 16 fase 1) sobre PostgreSQL como `dad_app`: catálogo laboral
//! de plataforma versionado y de solo inserción, alta de empleados con su persona, contratos que
//! no se cruzan, versiones que avanzan en el tiempo y se validan contra el catálogo vigente,
//! prórroga, terminación, salarios reservados y las garantías de los triggers.

mod comun;

use application::{
    AppError, Contexto, ContextoPlataforma, SolicitudPagina,
    casos_uso::nomina::{
        catalogo::{self, NuevaAdministradora, NuevoParametro},
        empleados::{
            self as uc, CondicionesVersion, NuevaPersonaEmpleado, NuevoContrato, NuevoEmpleado,
            PersonaEmpleado,
        },
    },
    puertos::{
        RelojSistema,
        nomina::{CuentaNomina, FiltroEmpleados},
    },
};
use chrono::NaiveDate;
use comun::{MIGRACIONES, insertar_entidad, pool_como_app};
use domain::{
    AdministradoraId, Ambito, ConjuntoPermisos, CuentaId, Documento, DominioError, Nit, Permiso,
    TenantId, TipoDocumento,
    contabilidad::{Monto, tesoreria::TipoCuentaBancaria},
    nomina::{
        ClaseRiesgo, CodigoEmpleado, EstadoEmpleado, EtapaAprendizaje, Jornada, MedioPago,
        MotivoTerminacion, ParametroLaboral, PeriodicidadPago, TipoAdministradora,
        TipoContratoLaboral, TipoSalario,
    },
};
use infra_postgres::{UnidadDeTrabajoPg, UnidadDeTrabajoPlataformaPg, transaccion_tenant};
use rust_decimal::{Decimal, dec};
use sqlx::{AssertSqlSafe, PgPool};

fn dia(a: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(a, m, d).expect("fecha")
}

struct Afiliadoras {
    eps: AdministradoraId,
    afp: AdministradoraId,
    cesantias: AdministradoraId,
    ccf: AdministradoraId,
    arl: AdministradoraId,
}

struct Entorno {
    duenio: PgPool,
    plataforma: UnidadDeTrabajoPlataformaPg,
    unidad: UnidadDeTrabajoPg,
    tenant: TenantId,
    operador: ContextoPlataforma,
    ctx: Contexto,
    adm: Afiliadoras,
}

impl Entorno {
    /// Entidad sin vertical con el módulo de nómina y el catálogo de 2026 cargado.
    async fn new(pool: &PgPool) -> Self {
        let app = pool_como_app(pool).await;
        let tenant = insertar_entidad(pool, "Gimnasio de prueba").await;
        sqlx::query("UPDATE entidades SET estado = 'activa' WHERE id = $1")
            .bind(tenant.as_uuid())
            .execute(pool)
            .await
            .expect("activa");
        sqlx::query(
            "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
             VALUES (gen_random_uuid(), $1, 'nomina', now() - interval '1 day')",
        )
        .bind(tenant.as_uuid())
        .execute(pool)
        .await
        .expect("módulo");
        let plataforma = UnidadDeTrabajoPlataformaPg::new(app.clone());
        let operador = ContextoPlataforma::de_cuenta(
            CuentaId::nuevo(),
            ConjuntoPermisos::todos(Ambito::Plataforma),
            None,
        );
        for (p, v) in [
            (ParametroLaboral::SalarioMinimo, dec!(1500000)),
            (ParametroLaboral::JornadaMaximaSemanal, dec!(42)),
            (ParametroLaboral::SalarioIntegralMinimo, dec!(13)),
            (ParametroLaboral::ApoyoAprendizLectiva, dec!(75)),
            (ParametroLaboral::ApoyoAprendizProductiva, dec!(100)),
        ] {
            catalogo::crear_parametro(
                &plataforma,
                &RelojSistema,
                &operador,
                NuevoParametro {
                    parametro: p,
                    valor: v,
                    fuente: "Valor de prueba".into(),
                    vigente_desde: dia(2026, 1, 1),
                },
            )
            .await
            .expect("parámetro");
        }
        let mut ids = Vec::new();
        for (i, tipo) in [
            TipoAdministradora::Eps,
            TipoAdministradora::Afp,
            TipoAdministradora::Cesantias,
            TipoAdministradora::Ccf,
            TipoAdministradora::Arl,
        ]
        .into_iter()
        .enumerate()
        {
            let a = catalogo::crear_administradora(
                &plataforma,
                &RelojSistema,
                &operador,
                NuevaAdministradora {
                    tipo,
                    codigo: format!("ADM{i}"),
                    nit: Nit::new(&format!("90000000{i}")).expect("nit"),
                    nombre: format!("Administradora {tipo}"),
                },
            )
            .await
            .expect("administradora");
            ids.push(a.id);
        }
        Self {
            duenio: pool.clone(),
            plataforma,
            unidad: UnidadDeTrabajoPg::new(app),
            tenant,
            operador,
            ctx: Contexto::new(tenant),
            adm: Afiliadoras {
                eps: ids[0],
                afp: ids[1],
                cesantias: ids[2],
                ccf: ids[3],
                arl: ids[4],
            },
        }
    }

    fn condiciones(&self, salario: Decimal) -> CondicionesVersion {
        CondicionesVersion {
            cargo: "Instructor".into(),
            salario: Monto::new(salario).expect("monto"),
            tipo_salario: TipoSalario::Ordinario,
            etapa_aprendizaje: None,
            jornada: Jornada::Completa,
            horas_semanales: None,
            periodicidad: PeriodicidadPago::Mensual,
            eps_id: self.adm.eps,
            afp_id: Some(self.adm.afp),
            cesantias_id: Some(self.adm.cesantias),
            ccf_id: Some(self.adm.ccf),
            arl_id: Some(self.adm.arl),
            clase_riesgo: Some(ClaseRiesgo::I),
            pensionado: false,
            medio_pago: MedioPago::Transferencia,
            cuenta: Some(CuentaNomina {
                banco: "Banco de prueba".into(),
                tipo: TipoCuentaBancaria::Ahorros,
                numero: "0012345678".into(),
            }),
        }
    }

    fn fijo(&self, inicio: NaiveDate, fin: NaiveDate) -> NuevoContrato {
        NuevoContrato {
            tipo: TipoContratoLaboral::TerminoFijo,
            fecha_inicio: inicio,
            fecha_fin: Some(fin),
            descripcion_obra: None,
            condiciones: self.condiciones(dec!(2000000)),
        }
    }

    fn nuevo(&self, codigo: &str, documento: &str, contrato: NuevoContrato) -> NuevoEmpleado {
        NuevoEmpleado {
            codigo: CodigoEmpleado::new(codigo).expect("código"),
            persona: PersonaEmpleado::Nueva(NuevaPersonaEmpleado {
                nombre: "Ana Pérez".into(),
                tipo_documento: TipoDocumento::Cc,
                documento: Documento::new(documento).expect("documento"),
                correo: None,
                telefono: None,
            }),
            contrato,
        }
    }

    /// Ejecuta SQL como `dad_app` en la entidad y confirma.
    async fn sql(&self, sql: &str) -> Result<(), sqlx::Error> {
        let mut tx = transaccion_tenant(&self.duenio, self.tenant).await?;
        sqlx::query("SET LOCAL ROLE dad_app")
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql(AssertSqlSafe(sql.to_owned()))
            .execute(&mut *tx)
            .await?;
        tx.commit().await
    }
}

fn regla_laboral(e: AppError) -> String {
    match e {
        AppError::Dominio(DominioError::ReglaLaboral(t)) => t,
        otro => panic!("se esperaba una regla laboral: {otro:?}"),
    }
}

fn sqlstate(e: &sqlx::Error) -> String {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| c.into_owned())
        .unwrap_or_default()
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_catalogo_vale_la_version_vigente_y_no_se_modifica(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let nuevo = |valor, desde| NuevoParametro {
        parametro: ParametroLaboral::SalarioMinimo,
        valor,
        fuente: "Decreto de prueba".into(),
        vigente_desde: desde,
    };
    catalogo::crear_parametro(
        &e.plataforma,
        &RelojSistema,
        &e.operador,
        nuevo(dec!(1600000), dia(2027, 1, 1)),
    )
    .await
    .expect("versión 2027");

    let repetida = catalogo::crear_parametro(
        &e.plataforma,
        &RelojSistema,
        &e.operador,
        nuevo(dec!(1700000), dia(2027, 1, 1)),
    )
    .await
    .expect_err("misma fecha");
    let vigente_2026 =
        catalogo::catalogo_vigente(&e.unidad, &RelojSistema, &e.ctx, Some(dia(2026, 12, 31)))
            .await
            .expect("catálogo 2026");
    let completo = catalogo::catalogo_plataforma(&e.plataforma, &e.operador)
        .await
        .expect("catálogo completo");
    let editar = e
        .sql("UPDATE parametros_laborales SET valor = 1 WHERE parametro = 'salario_minimo'")
        .await
        .expect_err("solo inserción");
    let desde_entidad = e
        .sql(
            "INSERT INTO parametros_laborales (id, parametro, valor, fuente, vigente_desde,
                 creado_en)
             VALUES (gen_random_uuid(), 'icbf', 3, 'Entidad', '2026-01-01', now())",
        )
        .await
        .expect_err("solo la plataforma escribe");

    assert!(matches!(repetida, AppError::Conflicto(_)), "{repetida:?}");
    let salario = |ps: &[application::puertos::nomina::VersionParametroLaboral]| {
        ps.iter()
            .filter(|p| p.parametro == ParametroLaboral::SalarioMinimo)
            .map(|p| p.valor)
            .collect::<Vec<_>>()
    };
    assert_eq!(salario(&vigente_2026.parametros), vec![dec!(1500000)]);
    assert_eq!(salario(&completo.parametros).len(), 2);
    assert_eq!(vigente_2026.administradoras.len(), 5);
    assert!(editar.to_string().contains("permission denied"), "{editar}");
    assert!(
        desde_entidad.to_string().contains("row-level security"),
        "{desde_entidad}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_empleado_nace_con_su_contrato_y_su_persona(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let ficha = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.nuevo("E-1", "52123456", e.fijo(dia(2026, 2, 1), dia(2027, 1, 31))),
    )
    .await
    .expect("alta");

    let leida = uc::ficha_empleado(&e.unidad, &RelojSistema, &e.ctx, ficha.empleado.id)
        .await
        .expect("ficha");
    let lista = uc::empleados(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        FiltroEmpleados {
            texto: Some("perez".into()),
            estado: None,
        },
        SolicitudPagina::primera(),
    )
    .await
    .expect("listado");
    let personas: i64 = sqlx::query_scalar("SELECT count(*) FROM personas WHERE tenant_id = $1")
        .bind(e.tenant.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("personas");

    assert_eq!(leida.empleado.id, ficha.empleado.id);
    assert_eq!(
        leida.contratos[0].versiones[0].id,
        ficha.contratos[0].versiones[0].id
    );
    assert_eq!(leida.contratos.len(), 1);
    let v = &leida.contratos[0].versiones[0];
    assert_eq!(v.salario.map(Monto::valor), Some(dec!(2000000.00)));
    assert_eq!(
        v.cuenta.as_ref().map(|c| c.numero.as_str()),
        Some("0012345678")
    );
    assert_eq!(personas, 1);
    assert_eq!(lista.datos.len(), 1);
    assert_eq!(lista.datos[0].cargo.as_deref(), Some("Instructor"));
    assert_eq!(lista.datos[0].fecha_ingreso, Some(dia(2026, 2, 1)));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_permiso_de_salarios_no_se_ven_salario_ni_cuenta(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let ficha = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.nuevo("E-1", "52123456", e.fijo(dia(2026, 2, 1), dia(2027, 1, 31))),
    )
    .await
    .expect("alta");
    let solo_ver = Contexto::de_cuenta(
        e.tenant,
        CuentaId::nuevo(),
        [Permiso::NominaVer].into_iter().collect(),
    );

    let leida = uc::ficha_empleado(&e.unidad, &RelojSistema, &solo_ver, ficha.empleado.id)
        .await
        .expect("ficha");
    let crear = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &solo_ver,
        e.nuevo("E-2", "52999999", e.fijo(dia(2026, 2, 1), dia(2027, 1, 31))),
    )
    .await
    .expect_err("sin gestionar");

    let v = &leida.contratos[0].versiones[0];
    assert_eq!((v.salario, v.cuenta.is_none()), (None, true));
    assert_eq!(v.cargo, "Instructor");
    assert!(matches!(
        crear,
        AppError::SinPermiso(Permiso::NominaEmpleadosGestionar)
    ));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_persona_se_reutiliza_pero_no_se_emplea_dos_veces(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let primera = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.nuevo("E-1", "52123456", e.fijo(dia(2026, 2, 1), dia(2027, 1, 31))),
    )
    .await
    .expect("alta");

    let otra_vez = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.nuevo(
            "E-2",
            "52.123.456",
            e.fijo(dia(2027, 2, 1), dia(2028, 1, 31)),
        ),
    )
    .await
    .expect_err("ya es empleada");
    let mismo_codigo = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.nuevo(
            "E-1",
            "1010101010",
            e.fijo(dia(2026, 2, 1), dia(2027, 1, 31)),
        ),
    )
    .await
    .expect_err("código repetido");

    assert!(matches!(otra_vez, AppError::Conflicto(_)), "{otra_vez:?}");
    assert!(
        matches!(mismo_codigo, AppError::Conflicto(_)),
        "{mismo_codigo:?}"
    );
    assert_eq!(
        primera.documento.as_ref().map(Documento::as_str),
        Some("52123456")
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn los_contratos_de_un_empleado_no_se_cruzan(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let ficha = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.nuevo("E-1", "52123456", e.fijo(dia(2026, 2, 1), dia(2026, 7, 31))),
    )
    .await
    .expect("alta");
    let empleado = ficha.empleado.id;

    let cruzado = uc::crear_contrato(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        empleado,
        e.fijo(dia(2026, 7, 1), dia(2026, 12, 31)),
    )
    .await
    .expect_err("se cruza");
    let siguiente = uc::crear_contrato(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        empleado,
        e.fijo(dia(2026, 8, 1), dia(2026, 12, 31)),
    )
    .await;

    assert!(matches!(cruzado, AppError::Conflicto(_)), "{cruzado:?}");
    assert!(siguiente.is_ok(), "{siguiente:?}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_modificacion_es_una_version_nueva_validada_con_el_catalogo_de_su_fecha(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    catalogo::crear_parametro(
        &e.plataforma,
        &RelojSistema,
        &e.operador,
        NuevoParametro {
            parametro: ParametroLaboral::SalarioMinimo,
            valor: dec!(2100000),
            fuente: "Decreto de prueba".into(),
            vigente_desde: dia(2027, 1, 1),
        },
    )
    .await
    .expect("mínimo 2027");
    let ficha = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.nuevo("E-1", "52123456", e.fijo(dia(2026, 2, 1), dia(2027, 6, 30))),
    )
    .await
    .expect("alta");
    let contrato = ficha.contratos[0].contrato.id;

    let bajo_el_minimo = uc::modificar_contrato(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        contrato,
        dia(2027, 1, 1),
        e.condiciones(dec!(2000000)),
    )
    .await
    .expect_err("2027 exige más");
    let ajustada = uc::modificar_contrato(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        contrato,
        dia(2027, 1, 1),
        e.condiciones(dec!(2100000)),
    )
    .await
    .expect("versión 2027");
    let hacia_atras = uc::modificar_contrato(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        contrato,
        dia(2026, 6, 1),
        e.condiciones(dec!(2500000)),
    )
    .await
    .expect_err("antes de la última");
    let leida = uc::ficha_empleado(&e.unidad, &RelojSistema, &e.ctx, ficha.empleado.id)
        .await
        .expect("ficha");

    assert!(regla_laboral(bajo_el_minimo).contains("salario mínimo"));
    assert_eq!(ajustada.vigente_desde, dia(2027, 1, 1));
    assert!(
        matches!(hacia_atras, AppError::Validacion(_)),
        "{hacia_atras:?}"
    );
    assert_eq!(leida.contratos[0].versiones.len(), 2);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_contrato_se_prorroga_y_al_terminar_queda_cerrado(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let ficha = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.nuevo("E-1", "52123456", e.fijo(dia(2025, 2, 1), dia(2025, 7, 31))),
    )
    .await
    .expect_err("sin catálogo de 2025");
    assert!(regla_laboral(ficha).contains("catálogo laboral"));

    let ficha = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.nuevo("E-1", "52123456", e.fijo(dia(2026, 2, 1), dia(2026, 7, 31))),
    )
    .await
    .expect("alta");
    let contrato = ficha.contratos[0].contrato.id;
    uc::modificar_contrato(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        contrato,
        dia(2026, 5, 1),
        e.condiciones(dec!(2200000)),
    )
    .await
    .expect("aumento");

    let fuera = uc::terminar_contrato(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        contrato,
        dia(2026, 9, 30),
        MotivoTerminacion::Renuncia,
    )
    .await
    .expect_err("después del fin");
    let prorrogado =
        uc::prorrogar_contrato(&e.unidad, &RelojSistema, &e.ctx, contrato, dia(2027, 1, 31))
            .await
            .expect("prórroga");
    let antes_de_la_version = uc::terminar_contrato(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        contrato,
        dia(2026, 4, 30),
        MotivoTerminacion::Renuncia,
    )
    .await
    .expect_err("hay una versión posterior");
    let terminado = uc::terminar_contrato(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        contrato,
        dia(2026, 9, 30),
        MotivoTerminacion::Renuncia,
    )
    .await
    .expect("terminación");
    let despues = uc::modificar_contrato(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        contrato,
        dia(2026, 10, 1),
        e.condiciones(dec!(2300000)),
    )
    .await
    .expect_err("terminado");
    let retirados = uc::empleados(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        FiltroEmpleados {
            texto: None,
            estado: Some(EstadoEmpleado::Retirado),
        },
        SolicitudPagina::primera(),
    )
    .await
    .expect("retirados");

    assert!(regla_laboral(fuera).contains("prorróguelo"));
    assert_eq!(prorrogado.fecha_fin, Some(dia(2027, 1, 31)));
    assert!(regla_laboral(antes_de_la_version).contains("versión posterior"));
    assert_eq!(
        terminado.motivo_terminacion,
        Some(MotivoTerminacion::Renuncia)
    );
    assert!(matches!(despues, AppError::Conflicto(_)), "{despues:?}");
    // Hoy (después de 2026-09-30) el empleado ya está retirado.
    assert_eq!(retirados.datos.len(), 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_aprendiz_lleva_apoyo_y_sus_afiliaciones(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let mut lectiva = e.condiciones(dec!(1125000));
    lectiva.tipo_salario = TipoSalario::ApoyoSostenimiento;
    lectiva.etapa_aprendizaje = Some(EtapaAprendizaje::Lectiva);
    lectiva.afp_id = None;
    lectiva.cesantias_id = None;
    lectiva.ccf_id = None;
    lectiva.arl_id = None;
    lectiva.clase_riesgo = None;
    let contrato = |c: CondicionesVersion| NuevoContrato {
        tipo: TipoContratoLaboral::Aprendizaje,
        fecha_inicio: dia(2026, 2, 1),
        fecha_fin: Some(dia(2027, 7, 31)),
        descripcion_obra: None,
        condiciones: c,
    };

    let ficha = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.nuevo("A-1", "1010101010", contrato(lectiva.clone())),
    )
    .await
    .expect("aprendiz");
    let mut productiva = lectiva.clone();
    productiva.salario = Monto::new(dec!(1500000)).expect("monto");
    productiva.etapa_aprendizaje = Some(EtapaAprendizaje::Productiva);
    let sin_arl = uc::modificar_contrato(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        ficha.contratos[0].contrato.id,
        dia(2026, 8, 1),
        productiva.clone(),
    )
    .await
    .expect_err("la etapa productiva lleva ARL");
    productiva.arl_id = Some(e.adm.arl);
    productiva.clase_riesgo = Some(ClaseRiesgo::I);
    let con_arl = uc::modificar_contrato(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        ficha.contratos[0].contrato.id,
        dia(2026, 8, 1),
        productiva,
    )
    .await;
    let mut eps_equivocada = lectiva;
    eps_equivocada.eps_id = e.adm.afp;
    let otra_eps = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.nuevo("A-2", "2020202020", contrato(eps_equivocada)),
    )
    .await
    .expect_err("no es una EPS");

    assert!(regla_laboral(sin_arl).contains("ARL"));
    assert!(con_arl.is_ok(), "{con_arl:?}");
    assert!(matches!(otra_eps, AppError::Validacion(_)), "{otra_eps:?}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_modulo_no_hay_nomina(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    sqlx::query("DELETE FROM modulos_habilitados WHERE tenant_id = $1")
        .bind(e.tenant.as_uuid())
        .execute(&pool)
        .await
        .expect("sin módulo");

    let r = uc::empleados(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        FiltroEmpleados::default(),
        SolicitudPagina::primera(),
    )
    .await
    .expect_err("sin módulo");

    assert!(
        matches!(r, AppError::ModuloNoHabilitado(domain::Modulo::Nomina)),
        "{r:?}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_base_protege_contratos_y_versiones(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let ficha = uc::crear_empleado(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        e.nuevo(
            "E-1",
            "52123456",
            e.fijo(dia(2026, 2, 1), dia(2026, 12, 31)),
        ),
    )
    .await
    .expect("alta");
    let empleado = ficha.empleado.id.as_uuid();
    let contrato = ficha.contratos[0].contrato.id.as_uuid();
    let tenant = e.tenant.as_uuid();

    let sin_version = e
        .sql(&format!(
            "INSERT INTO contratos_laborales (id, tenant_id, empleado_id, tipo, fecha_inicio,
                 creado_en)
             VALUES (gen_random_uuid(), '{tenant}', '{empleado}', 'termino_indefinido',
                     '2027-01-01', now())"
        ))
        .await
        .expect_err("sin primera versión");
    let editar_version = e
        .sql(&format!(
            "UPDATE versiones_contrato_laboral SET salario = 1 WHERE contrato_id = '{contrato}'"
        ))
        .await
        .expect_err("solo inserción");
    let borrar = e
        .sql(&format!(
            "DELETE FROM contratos_laborales WHERE id = '{contrato}'"
        ))
        .await
        .expect_err("no se borra");
    let cambiar_inicio = e
        .sql(&format!(
            "UPDATE contratos_laborales SET fecha_inicio = '2026-01-01' WHERE id = '{contrato}'"
        ))
        .await
        .expect_err("el inicio no cambia");

    assert!(
        sin_version.to_string().contains("primera versión"),
        "{sin_version}"
    );
    assert!(
        editar_version.to_string().contains("permission denied"),
        "{editar_version}"
    );
    // `dad_app` no tiene DELETE; el trigger también lo rechaza para el dueño del esquema.
    assert_eq!(sqlstate(&borrar), "42501");
    assert_eq!(sqlstate(&cambiar_inicio), "V0010");
}

// ---------------------------------------------------------------------------------------------
// Fase 2 · Liquidación
// ---------------------------------------------------------------------------------------------

mod liquidacion {
    use application::casos_uso::nomina::{
        catalogo::NuevaTablaLaboral,
        liquidacion as ul,
        novedades::{self as un, NuevaNovedad},
    };
    use domain::nomina::{
        PeriodicidadPago,
        liquidacion::{ConceptoNomina, TablaLaboral, TipoNovedad, Tramo},
        periodo::EstadoPeriodoNomina,
    };

    use super::*;

    /// Completa el catálogo de 2026 con lo que necesita la liquidación (valores de prueba).
    async fn catalogo_completo(e: &Entorno) {
        for (p, v) in [
            (ParametroLaboral::AuxilioTransporte, dec!(200000)),
            (ParametroLaboral::TopeAuxilioTransporte, dec!(2)),
            (ParametroLaboral::RecargoNocturno, dec!(35)),
            (ParametroLaboral::HoraExtraDiurna, dec!(25)),
            (ParametroLaboral::HoraExtraNocturna, dec!(75)),
            (ParametroLaboral::RecargoDominicalFestivo, dec!(90)),
            (ParametroLaboral::SaludEmpleador, dec!(8.5)),
            (ParametroLaboral::SaludEmpleado, dec!(4)),
            (ParametroLaboral::PensionEmpleador, dec!(12)),
            (ParametroLaboral::PensionEmpleado, dec!(4)),
            (ParametroLaboral::CajaCompensacion, dec!(4)),
            (ParametroLaboral::Icbf, dec!(3)),
            (ParametroLaboral::Sena, dec!(2)),
            (ParametroLaboral::ArlClase1, dec!(0.522)),
            (ParametroLaboral::FactorSalarioIntegral, dec!(70)),
            (ParametroLaboral::LimitePagosNoSalariales, dec!(40)),
            (ParametroLaboral::IncapacidadEps, dec!(66.67)),
            (ParametroLaboral::TopeIbc, dec!(25)),
            (ParametroLaboral::TopeExoneracionAportes, dec!(10)),
            (ParametroLaboral::RentaExentaLaboral, dec!(25)),
            (ParametroLaboral::TopeRentaExentaLaboral, dec!(790)),
            (ParametroLaboral::LimiteBeneficiosRetencion, dec!(40)),
            (ParametroLaboral::TopeBeneficiosRetencion, dec!(1340)),
            (ParametroLaboral::DiasCesantias, dec!(30)),
            (ParametroLaboral::DiasPrima, dec!(30)),
            (ParametroLaboral::InteresesCesantias, dec!(12)),
            (ParametroLaboral::DiasVacaciones, dec!(15)),
        ] {
            catalogo::crear_parametro(
                &e.plataforma,
                &RelojSistema,
                &e.operador,
                NuevoParametro {
                    parametro: p,
                    valor: v,
                    fuente: "Valor de prueba".into(),
                    vigente_desde: dia(2026, 1, 1),
                },
            )
            .await
            .expect("parámetro");
        }
        let t =
            |desde: Decimal, hasta: Option<Decimal>, tarifa: Decimal, adicional: Decimal| Tramo {
                desde,
                hasta,
                tarifa,
                adicional,
            };
        for (tabla, tramos) in [
            (
                TablaLaboral::RetencionSalarios,
                vec![
                    t(dec!(0), Some(dec!(95)), dec!(0), dec!(0)),
                    t(dec!(95), Some(dec!(150)), dec!(19), dec!(0)),
                    t(dec!(150), Some(dec!(360)), dec!(28), dec!(10)),
                    t(dec!(360), None, dec!(33), dec!(69)),
                ],
            ),
            (
                TablaLaboral::FondoSolidaridad,
                vec![
                    t(dec!(0), Some(dec!(4)), dec!(0), dec!(0)),
                    t(dec!(4), None, dec!(1), dec!(0)),
                ],
            ),
        ] {
            catalogo::crear_tabla_laboral(
                &e.plataforma,
                &RelojSistema,
                &e.operador,
                NuevaTablaLaboral {
                    tabla,
                    vigente_desde: dia(2026, 1, 1),
                    fuente: "Tabla de prueba".into(),
                    tramos,
                },
            )
            .await
            .expect("tabla");
        }
    }

    fn cuenta(tenant: TenantId) -> Contexto {
        Contexto::de_cuenta(
            tenant,
            CuentaId::nuevo(),
            ConjuntoPermisos::todos(Ambito::Copropiedad),
        )
    }

    async fn empleado(e: &Entorno, salario: Decimal) -> domain::ContratoLaboralId {
        let mut contrato = e.fijo(dia(2026, 2, 1), dia(2027, 1, 31));
        contrato.condiciones = e.condiciones(salario);
        uc::crear_empleado(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.nuevo("E-1", "52123456", contrato),
        )
        .await
        .expect("alta")
        .contratos[0]
            .contrato
            .id
    }

    fn horas(tipo: TipoNovedad, fecha: NaiveDate, horas: Decimal) -> NuevaNovedad {
        NuevaNovedad {
            tipo,
            desde: fecha,
            hasta: None,
            cantidad: Some(horas),
            descripcion: None,
            soporte_archivo_id: None,
        }
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn el_mes_se_liquida_lo_aprueba_otra_cuenta_y_se_paga(pool: PgPool) {
        let e = Entorno::new(&pool).await;
        catalogo_completo(&e).await;
        let contrato = empleado(&e, dec!(2100000)).await;
        let liquidador = cuenta(e.tenant);
        let aprobador = cuenta(e.tenant);
        un::registrar_novedad(
            &e.unidad,
            &RelojSistema,
            &liquidador,
            contrato,
            horas(TipoNovedad::HoraExtraDiurna, dia(2026, 3, 4), dec!(10)),
        )
        .await
        .expect("horas extra");
        let periodo = ul::crear_periodo(
            &e.unidad,
            &RelojSistema,
            &liquidador,
            PeriodicidadPago::Mensual,
            dia(2026, 3, 1),
        )
        .await
        .expect("periodo");

        let liquidado = ul::liquidar_periodo(&e.unidad, &RelojSistema, &liquidador, periodo.id)
            .await
            .expect("liquida");
        let misma_cuenta = ul::aprobar_periodo(&e.unidad, &RelojSistema, &liquidador, periodo.id)
            .await
            .expect_err("cuatro ojos");
        let aprobado = ul::aprobar_periodo(&e.unidad, &RelojSistema, &aprobador, periodo.id)
            .await
            .expect("aprueba");
        let tarde = un::registrar_novedad(
            &e.unidad,
            &RelojSistema,
            &liquidador,
            contrato,
            horas(TipoNovedad::HoraExtraDiurna, dia(2026, 3, 5), dec!(2)),
        )
        .await
        .expect_err("periodo aprobado");
        let otra_vez = ul::liquidar_periodo(&e.unidad, &RelojSistema, &liquidador, periodo.id)
            .await
            .expect_err("aprobado");
        let pagado = ul::marcar_pagado(
            &e.unidad,
            &RelojSistema,
            &liquidador,
            periodo.id,
            dia(2026, 3, 31),
        )
        .await
        .expect("pago");
        let leido = ul::periodo(&e.unidad, &RelojSistema, &e.ctx, periodo.id)
            .await
            .expect("periodo");

        let l = &liquidado.liquidaciones[0].liquidacion;
        let valor = |c| {
            l.lineas
                .iter()
                .find(|x| x.concepto == c)
                .map(|x| x.valor.valor())
        };
        assert_eq!(liquidado.liquidaciones.len(), 1);
        assert_eq!(valor(ConceptoNomina::Basico), Some(dec!(2100000.00)));
        // 10 × 10.000 × 1,25.
        assert_eq!(
            valor(ConceptoNomina::HoraExtraDiurna),
            Some(dec!(125000.00))
        );
        assert_eq!(l.ibc.valor(), dec!(2225000.00));
        assert!(
            matches!(misma_cuenta, AppError::Conflicto(_)),
            "{misma_cuenta:?}"
        );
        assert_eq!(aprobado.estado, EstadoPeriodoNomina::Aprobado);
        assert!(super::regla_laboral(tarde).contains("aprobado"));
        assert!(matches!(
            otra_vez,
            AppError::Dominio(DominioError::TransicionInvalida { .. })
        ));
        assert_eq!(pagado.estado, EstadoPeriodoNomina::Pagado);
        let leida = &leido.liquidaciones[0].liquidacion;
        assert_eq!((leida.neto, leida.ibc), (l.neto, l.ibc));
        assert_eq!(leida.lineas.len(), l.lineas.len());
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn una_novedad_despues_de_liquidar_exige_liquidar_de_nuevo(pool: PgPool) {
        let e = Entorno::new(&pool).await;
        catalogo_completo(&e).await;
        let contrato = empleado(&e, dec!(2100000)).await;
        let liquidador = cuenta(e.tenant);
        let aprobador = cuenta(e.tenant);
        let periodo = ul::crear_periodo(
            &e.unidad,
            &RelojSistema,
            &liquidador,
            PeriodicidadPago::Mensual,
            dia(2026, 3, 1),
        )
        .await
        .expect("periodo");
        ul::liquidar_periodo(&e.unidad, &RelojSistema, &liquidador, periodo.id)
            .await
            .expect("liquida");
        let n = un::registrar_novedad(
            &e.unidad,
            &RelojSistema,
            &liquidador,
            contrato,
            horas(TipoNovedad::HoraExtraNocturna, dia(2026, 3, 6), dec!(3)),
        )
        .await
        .expect("novedad");

        let pendiente = ul::aprobar_periodo(&e.unidad, &RelojSistema, &aprobador, periodo.id)
            .await
            .expect_err("hay una novedad nueva");
        un::anular_novedad(
            &e.unidad,
            &RelojSistema,
            &liquidador,
            n.id,
            "Registrada por error",
        )
        .await
        .expect("anula");
        let de_nuevo = ul::liquidar_periodo(&e.unidad, &RelojSistema, &liquidador, periodo.id)
            .await
            .expect("liquida otra vez");
        let aprobado = ul::aprobar_periodo(&e.unidad, &RelojSistema, &aprobador, periodo.id).await;

        assert!(matches!(pendiente, AppError::Conflicto(_)), "{pendiente:?}");
        assert!(
            de_nuevo.liquidaciones[0]
                .liquidacion
                .lineas
                .iter()
                .all(|l| l.concepto != ConceptoNomina::HoraExtraNocturna),
            "la anulada no entra"
        );
        assert!(aprobado.is_ok(), "{aprobado:?}");
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn la_segunda_quincena_retiene_con_lo_de_la_primera(pool: PgPool) {
        let e = Entorno::new(&pool).await;
        catalogo_completo(&e).await;
        let mut contrato = e.fijo(dia(2026, 2, 1), dia(2027, 1, 31));
        contrato.condiciones = e.condiciones(dec!(12000000));
        contrato.condiciones.periodicidad = PeriodicidadPago::Quincenal;
        uc::crear_empleado(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.nuevo("E-1", "52123456", contrato),
        )
        .await
        .expect("alta");
        let liquidador = cuenta(e.tenant);
        let mut retenciones = Vec::new();
        for desde in [dia(2026, 3, 1), dia(2026, 3, 16)] {
            let p = ul::crear_periodo(
                &e.unidad,
                &RelojSistema,
                &liquidador,
                PeriodicidadPago::Quincenal,
                desde,
            )
            .await
            .expect("periodo");
            let l = ul::liquidar_periodo(&e.unidad, &RelojSistema, &liquidador, p.id)
                .await
                .expect("liquida");
            retenciones.push(
                l.liquidaciones[0]
                    .liquidacion
                    .lineas
                    .iter()
                    .find(|x| x.concepto == ConceptoNomina::RetencionFuente)
                    .map(|x| x.valor.valor()),
            );
        }
        let mensual = ul::crear_periodo(
            &e.unidad,
            &RelojSistema,
            &liquidador,
            PeriodicidadPago::Mensual,
            dia(2026, 3, 1),
        )
        .await
        .expect("periodo mensual");
        let sin_contratos = ul::liquidar_periodo(&e.unidad, &RelojSistema, &liquidador, mensual.id)
            .await
            .expect("liquida");

        assert_eq!(retenciones[0], None, "la primera quincena no retiene");
        assert!(
            retenciones[1].is_some_and(|r| r > Decimal::ZERO),
            "{retenciones:?}"
        );
        assert!(
            sin_contratos.liquidaciones.is_empty(),
            "el contrato quincenal no entra al periodo mensual"
        );
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn las_incapacidades_solo_las_ve_quien_ve_datos_sensibles(pool: PgPool) {
        let e = Entorno::new(&pool).await;
        let contrato = empleado(&e, dec!(2100000)).await;
        un::registrar_novedad(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            contrato,
            NuevaNovedad {
                tipo: TipoNovedad::IncapacidadComun,
                desde: dia(2026, 3, 10),
                hasta: Some(dia(2026, 3, 12)),
                cantidad: None,
                descripcion: None,
                soporte_archivo_id: None,
            },
        )
        .await
        .expect("incapacidad");
        un::registrar_novedad(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            contrato,
            NuevaNovedad {
                tipo: TipoNovedad::BonificacionSalarial,
                desde: dia(2026, 3, 20),
                hasta: None,
                cantidad: Some(dec!(300000)),
                descripcion: None,
                soporte_archivo_id: None,
            },
        )
        .await
        .expect("bonificación");
        let restringido = Contexto::de_cuenta(
            e.tenant,
            CuentaId::nuevo(),
            [Permiso::NominaVer].into_iter().collect(),
        );

        let todas = un::novedades(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            None,
            dia(2026, 3, 1),
            dia(2026, 3, 31),
        )
        .await
        .expect("todas");
        let vistas = un::novedades(
            &e.unidad,
            &RelojSistema,
            &restringido,
            None,
            dia(2026, 3, 1),
            dia(2026, 3, 31),
        )
        .await
        .expect("restringidas");

        assert_eq!(todas.len(), 2);
        assert_eq!(vistas.len(), 1);
        assert_eq!(
            vistas[0].novedad.cantidad,
            Decimal::ZERO,
            "sin salarios no ve el valor"
        );
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn sin_las_tablas_vigentes_no_se_liquida(pool: PgPool) {
        let e = Entorno::new(&pool).await;
        empleado(&e, dec!(2100000)).await;
        let p = ul::crear_periodo(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            PeriodicidadPago::Mensual,
            dia(2026, 3, 1),
        )
        .await
        .expect("periodo");
        let fuera = ul::crear_periodo(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            PeriodicidadPago::Mensual,
            dia(2026, 3, 16),
        )
        .await
        .expect_err("no empieza el día 1");

        let r = ul::liquidar_periodo(&e.unidad, &RelojSistema, &e.ctx, p.id)
            .await
            .expect_err("sin tablas");

        assert!(super::regla_laboral(r).contains("tabla"));
        assert!(super::regla_laboral(fuera).contains("día 1"));
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn la_base_protege_lo_aprobado(pool: PgPool) {
        let e = Entorno::new(&pool).await;
        catalogo_completo(&e).await;
        empleado(&e, dec!(2100000)).await;
        let liquidador = cuenta(e.tenant);
        let p = ul::crear_periodo(
            &e.unidad,
            &RelojSistema,
            &liquidador,
            PeriodicidadPago::Mensual,
            dia(2026, 3, 1),
        )
        .await
        .expect("periodo");
        ul::liquidar_periodo(&e.unidad, &RelojSistema, &liquidador, p.id)
            .await
            .expect("liquida");
        ul::aprobar_periodo(&e.unidad, &RelojSistema, &cuenta(e.tenant), p.id)
            .await
            .expect("aprueba");
        let id = p.id.as_uuid();

        let borrar = e
            .sql(&format!(
                "DELETE FROM liquidaciones_nomina WHERE periodo_id = '{id}'"
            ))
            .await
            .expect_err("aprobado");
        let reabrir = e
            .sql(&format!(
                "UPDATE periodos_nomina SET estado = 'liquidado', aprobado_en = NULL,
                     aprobado_por = NULL WHERE id = '{id}'"
            ))
            .await
            .expect_err("no vuelve atrás");

        assert_eq!(sqlstate(&borrar), "V0010");
        assert_eq!(sqlstate(&reabrir), "V0010");
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn la_prima_de_junio_cubre_el_semestre_trabajado(pool: PgPool) {
        let e = Entorno::new(&pool).await;
        catalogo_completo(&e).await;
        let mut contrato = e.fijo(dia(2026, 4, 1), dia(2027, 3, 31));
        contrato.condiciones = e.condiciones(dec!(1500000));
        uc::crear_empleado(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.nuevo("E-1", "52123456", contrato),
        )
        .await
        .expect("alta");
        let p = ul::crear_periodo(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            PeriodicidadPago::Mensual,
            dia(2026, 6, 1),
        )
        .await
        .expect("junio");

        let l = ul::liquidar_periodo(&e.unidad, &RelojSistema, &e.ctx, p.id)
            .await
            .expect("liquida");

        let linea = l.liquidaciones[0]
            .liquidacion
            .lineas
            .iter()
            .find(|x| x.concepto == ConceptoNomina::Prima)
            .copied()
            .expect("prima");
        // Abril a junio: 90 días × (1.500.000 + 200.000) / 360.
        assert_eq!(
            (linea.cantidad, linea.valor.valor()),
            (dec!(90), dec!(425000.00))
        );
        // La prima no entra al IBC.
        assert_eq!(l.liquidaciones[0].liquidacion.ibc.valor(), dec!(1500000.00));
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn el_retiro_sin_justa_causa_liquida_prestaciones_e_indemnizacion(pool: PgPool) {
        let e = Entorno::new(&pool).await;
        catalogo_completo(&e).await;
        let mut contrato = e.fijo(dia(2026, 1, 1), dia(2026, 12, 31));
        contrato.condiciones = e.condiciones(dec!(3000000));
        let ficha = uc::crear_empleado(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.nuevo("E-1", "52123456", contrato),
        )
        .await
        .expect("alta");
        let contrato = ficha.contratos[0].contrato.id;
        un::registrar_novedad(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            contrato,
            NuevaNovedad {
                tipo: TipoNovedad::Vacaciones,
                // Lunes 2 a viernes 6 de marzo: 5 días hábiles.
                desde: dia(2026, 3, 2),
                hasta: Some(dia(2026, 3, 6)),
                cantidad: None,
                descripcion: None,
                soporte_archivo_id: None,
            },
        )
        .await
        .expect("vacaciones");
        uc::terminar_contrato(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            contrato,
            dia(2026, 6, 15),
            MotivoTerminacion::SinJustaCausa,
        )
        .await
        .expect("terminación");
        let p = ul::crear_periodo(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            PeriodicidadPago::Mensual,
            dia(2026, 6, 1),
        )
        .await
        .expect("junio");

        let l = ul::liquidar_periodo(&e.unidad, &RelojSistema, &e.ctx, p.id)
            .await
            .expect("liquida");

        let q = &l.liquidaciones[0].liquidacion;
        let valor = |c| {
            q.lineas
                .iter()
                .find(|x| x.concepto == c)
                .map(|x| x.valor.valor())
        };
        assert_eq!(q.dias, 15, "retiro a mitad de mes");
        assert_eq!(valor(ConceptoNomina::Basico), Some(dec!(1500000.00)));
        // 165 días (enero a 15 de junio) × (3.000.000 + 200.000 de auxilio: hasta dos mínimos)
        // / 360.
        assert_eq!(valor(ConceptoNomina::Cesantias), Some(dec!(1466667.00)));
        assert_eq!(valor(ConceptoNomina::Prima), Some(dec!(1466667.00)));
        // 1.466.666,67 × 12 % × 165 / 360.
        assert_eq!(
            valor(ConceptoNomina::InteresesCesantias),
            Some(dec!(80667.00))
        );
        // Causó 6,88 días; disfrutó 5: 1,88 × 100.000.
        assert_eq!(
            valor(ConceptoNomina::VacacionesCompensadas),
            Some(dec!(188000.00))
        );
        // Término fijo: lo que faltaba (16 de junio a 31 de diciembre = 195 días).
        assert_eq!(
            valor(ConceptoNomina::Indemnizacion),
            Some(dec!(19500000.00))
        );
        assert_eq!(
            q.ibc.valor(),
            dec!(1500000.00),
            "las prestaciones no son IBC"
        );
    }

    #[sqlx::test(migrator = "MIGRACIONES")]
    async fn las_cesantias_del_ano_se_reportan_por_fondo(pool: PgPool) {
        let e = Entorno::new(&pool).await;
        catalogo_completo(&e).await;
        let mut contrato = e.fijo(dia(2026, 7, 1), dia(2027, 6, 30));
        contrato.condiciones = e.condiciones(dec!(1500000));
        uc::crear_empleado(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            e.nuevo("E-1", "52123456", contrato),
        )
        .await
        .expect("alta");

        let r = application::casos_uso::nomina::prestaciones::cesantias_del_ano(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            2026,
        )
        .await
        .expect("cesantías");

        assert_eq!(r.len(), 1);
        // 180 días × 1.700.000 / 360 = 850.000; intereses 850.000 × 12 % × 180 / 360.
        assert_eq!(
            (r[0].dias, r[0].cesantias, r[0].intereses),
            (180, dec!(850000), dec!(51000))
        );
        assert_eq!(r[0].fondo_id, Some(e.adm.cesantias));
    }
}
