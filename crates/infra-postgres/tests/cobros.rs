//! Integración del cobro de la plataforma (plan 19) sobre PostgreSQL como `dad_app`: catálogo y
//! tarifas versionadas, suscripción única por entidad, cortes medidos con `medir_consumo`,
//! aprobación automática de un corte igual al anterior, calendario de mora con suspensión en
//! solo lectura (nunca con asamblea instalada) y reactivación al pagar. Las tablas son de
//! plataforma: una transacción de la entidad no las lee.

mod comun;

use application::{
    AppError, ContextoPlataforma,
    casos_uso::cobros::{self as uc, DatosProducto, NuevaSuscripcion, cortes, tareas},
    puertos::{Reloj, cobros::Pagador},
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};
use domain::{
    Ambito, ConjuntoPermisos, DireccionCorreo, EstadoCopropiedad, TenantId,
    cobro_plataforma::{Ciclo, EstadoCorte, Metrica, ModeloTarifa, Tarifa},
};
use infra_postgres::{UnidadDeTrabajoPg, UnidadDeTrabajoPlataformaPg};
use rust_decimal::dec;
use sqlx::PgPool;
use uuid::Uuid;

struct RelojFijo(DateTime<Utc>);

impl Reloj for RelojFijo {
    fn ahora(&self) -> DateTime<Utc> {
        self.0
    }
}

fn dia(a: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(a, m, d).expect("fecha")
}

fn a_mediodia(a: i32, m: u32, d: u32) -> RelojFijo {
    RelojFijo(
        Utc.with_ymd_and_hms(a, m, d, 17, 0, 0)
            .single()
            .expect("hora"),
    )
}

struct Entorno {
    owner: PgPool,
    plataforma: UnidadDeTrabajoPlataformaPg,
    unidad: UnidadDeTrabajoPg,
    tenant: TenantId,
    ctx: ContextoPlataforma,
}

impl Entorno {
    /// PH activa con dos bienes privados y una suscripción mensual desde el 1 de enero con un
    /// producto de $1.000 por bien privado.
    async fn new(pool: &PgPool) -> (Self, domain::SuscripcionId) {
        let app = pool_como_app(pool).await;
        let tenant = crear_copropiedad(pool).await;
        sqlx::query("UPDATE entidades SET estado = 'activa' WHERE id = $1")
            .bind(tenant.as_uuid())
            .execute(pool)
            .await
            .expect("activa");
        sqlx::query(
            "INSERT INTO bienes_privados (id, tenant_id, codigo)
             VALUES (gen_random_uuid(), $1, '101'), (gen_random_uuid(), $1, '102')",
        )
        .bind(tenant.as_uuid())
        .execute(pool)
        .await
        .expect("bienes");
        let operador = domain::CuentaId::nuevo();
        sqlx::query(
            "INSERT INTO cuentas (id, nombre_mostrar, usuario, hash_contrasena,
                                  contrasena_actualizada_en, creada_en)
             VALUES ($1, 'Operador', 'operador_cobros', 'x', now(), now())",
        )
        .bind(operador.as_uuid())
        .execute(pool)
        .await
        .expect("operador");
        let e = Self {
            owner: pool.clone(),
            plataforma: UnidadDeTrabajoPlataformaPg::new(app.clone()),
            unidad: UnidadDeTrabajoPg::new(app),
            tenant,
            ctx: ContextoPlataforma::de_cuenta(
                operador,
                ConjuntoPermisos::todos(Ambito::Plataforma),
                None,
            ),
        };
        let reloj = a_mediodia(2026, 1, 1);
        let p = uc::crear_producto(
            &e.plataforma,
            &reloj,
            &e.ctx,
            DatosProducto {
                codigo: "ph_base".into(),
                nombre: "Gestión de la PH".into(),
                descripcion_factura: "{producto} — {cantidad} bienes — {periodo}".into(),
                modulo: None,
                item_facturacion_id: None,
                activo: true,
            },
        )
        .await
        .expect("producto");
        uc::fijar_tarifa(
            &e.plataforma,
            &reloj,
            &e.ctx,
            p.id,
            Tarifa {
                modelo: ModeloTarifa::PorUnidad,
                valor: dec!(1000),
                incluidas: None,
                valor_excedente: None,
                metrica: Some(Metrica::new("ph.bienes_privados").expect("métrica")),
            },
            dia(2025, 1, 1),
        )
        .await
        .expect("tarifa");
        let s = uc::crear_suscripcion(
            &e.plataforma,
            &reloj,
            &e.ctx,
            NuevaSuscripcion {
                tenant_id: tenant,
                pagador: Pagador::MismaEntidad,
                ciclo: Ciclo::Meses(1),
                inicio: dia(2026, 1, 1),
                dias_prueba: 0,
                plazo_pago_dias: None,
                correo_facturacion: DireccionCorreo::new("pagos@ph.co").expect("correo"),
            },
        )
        .await
        .expect("suscripción");
        uc::agregar_linea(
            &e.plataforma,
            &reloj,
            &e.ctx,
            s.id,
            p.id,
            dia(2026, 1, 1),
            None,
        )
        .await
        .expect("línea");
        (e, s.id)
    }

    async fn estado_entidad(&self) -> String {
        sqlx::query_scalar("SELECT estado FROM entidades WHERE id = $1")
            .bind(self.tenant.as_uuid())
            .fetch_one(&self.owner)
            .await
            .expect("estado")
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_worker_corta_los_periodos_cerrados_con_la_medicion(pool: PgPool) {
    let (e, s) = Entorno::new(&pool).await;

    let n = tareas::generar_cortes(&e.plataforma, &a_mediodia(2026, 3, 10))
        .await
        .expect("cortes");

    assert_eq!(n, 2, "enero y febrero están cerrados; marzo no");
    let lista = cortes::listar(
        &e.plataforma,
        &e.ctx,
        &application::puertos::cobros::FiltroCortes {
            estado: None,
            suscripcion: Some(s),
        },
        application::SolicitudPagina::primera(),
    )
    .await
    .expect("lista");
    assert_eq!(lista.datos.len(), 2);
    for c in &lista.datos {
        assert_eq!(
            c.estado,
            EstadoCorte::Borrador,
            "sin anterior cobrado no se aprueba solo"
        );
        assert_eq!(c.total, dec!(2000), "dos bienes a $1.000");
        assert!(c.lineas[0].descripcion.contains("2 bienes"));
    }
    let medidas: i64 = sqlx::query_scalar("SELECT count(*) FROM mediciones_consumo")
        .fetch_one(&pool)
        .await
        .expect("mediciones");
    assert_eq!(medidas, 2, "una medición guardada por periodo");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_corte_igual_al_anterior_aprobado_queda_aprobado_solo(pool: PgPool) {
    let (e, s) = Entorno::new(&pool).await;
    let reloj = a_mediodia(2026, 3, 10);
    let enero = cortes::calcular(&e.plataforma, &reloj, &e.ctx, s, dia(2026, 1, 1), None)
        .await
        .expect("enero");
    cortes::aprobar(&e.plataforma, &reloj, &e.ctx, enero.id)
        .await
        .expect("aprobar");

    let febrero = cortes::calcular(&e.plataforma, &reloj, &e.ctx, s, dia(2026, 2, 1), None)
        .await
        .expect("febrero");

    assert_eq!(febrero.estado, EstadoCorte::Aprobado);
    assert_eq!(febrero.aprobado_por, None, "aprobado por el sistema");
    let otra = cortes::calcular(&e.plataforma, &reloj, &e.ctx, s, dia(2026, 2, 1), None)
        .await
        .expect("idempotente");
    assert_eq!(
        otra.id, febrero.id,
        "recalcular un corte aprobado devuelve el mismo"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_entidad_tiene_una_sola_suscripcion_vigente(pool: PgPool) {
    let (e, _) = Entorno::new(&pool).await;

    let r = uc::crear_suscripcion(
        &e.plataforma,
        &a_mediodia(2026, 1, 2),
        &e.ctx,
        NuevaSuscripcion {
            tenant_id: e.tenant,
            pagador: Pagador::MismaEntidad,
            ciclo: Ciclo::PorEvento,
            inicio: dia(2026, 1, 2),
            dias_prueba: 0,
            plazo_pago_dias: None,
            correo_facturacion: DireccionCorreo::new("otro@ph.co").expect("correo"),
        },
    )
    .await;

    assert!(matches!(r, Err(AppError::Conflicto(_))), "{r:?}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_entidad_no_lee_las_tablas_de_cobro(pool: PgPool) {
    let (e, _) = Entorno::new(&pool).await;
    let app = pool_como_app(&pool).await;
    let mut tx = app.begin().await.expect("tx");
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(e.tenant.as_uuid().to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant");

    let visibles: i64 = sqlx::query_scalar("SELECT count(*) FROM suscripciones")
        .fetch_one(&mut *tx)
        .await
        .expect("conteo");
    let medir = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT medir_consumo($1, 'ph.bienes_privados', '2026-01-01', '2026-02-01')",
    )
    .bind(e.tenant.as_uuid())
    .fetch_one(&mut *tx)
    .await;

    assert_eq!(visibles, 0);
    assert!(
        medir.is_err(),
        "medir_consumo exige una transacción de plataforma"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn las_tarifas_no_se_editan(pool: PgPool) {
    let (_e, _) = Entorno::new(&pool).await;

    let r = sqlx::query("UPDATE tarifas_producto SET valor = 1")
        .execute(&pool)
        .await;

    assert!(r.is_err(), "las tarifas son versionadas (solo inserción)");
}

/// Factura del corte de enero por fuera del sistema el 31 de enero (vence a los 15 días).
async fn enero_facturado(e: &Entorno, s: domain::SuscripcionId) -> domain::CorteFacturacionId {
    let reloj = a_mediodia(2026, 1, 31);
    let c = cortes::calcular(&e.plataforma, &reloj, &e.ctx, s, dia(2026, 1, 1), None)
        .await
        .expect("enero");
    cortes::aprobar(&e.plataforma, &reloj, &e.ctx, c.id)
        .await
        .expect("aprobar");
    let c = cortes::marcar_facturado_externo(&e.plataforma, &e.ctx, c.id, "FE-1", dia(2026, 1, 31))
        .await
        .expect("facturado");
    assert_eq!(c.vencimiento, Some(dia(2026, 2, 15)));
    c.id
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_mora_avisa_suspende_y_el_pago_reactiva(pool: PgPool) {
    let (e, s) = Entorno::new(&pool).await;
    let corte = enero_facturado(&e, s).await;

    let recordatorio = tareas::procesar_mora(&e.plataforma, &e.unidad, &a_mediodia(2026, 2, 10))
        .await
        .expect("recordatorio");
    let a_tiempo = tareas::procesar_mora(&e.plataforma, &e.unidad, &a_mediodia(2026, 2, 11))
        .await
        .expect("sin repetir");
    let vencida = tareas::procesar_mora(&e.plataforma, &e.unidad, &a_mediodia(2026, 2, 16))
        .await
        .expect("vencida");
    let suspension = tareas::procesar_mora(&e.plataforma, &e.unidad, &a_mediodia(2026, 3, 17))
        .await
        .expect("suspensión");

    assert_eq!(recordatorio.avisos, 1, "cinco días antes del vencimiento");
    assert_eq!(a_tiempo.avisos, 0, "un aviso por fecha del calendario");
    assert_eq!(vencida.avisos, 1, "al día siguiente del vencimiento");
    assert_eq!(suspension.suspendidas, 1, "30 días después del vencimiento");
    assert_eq!(e.estado_entidad().await, "suspendida");
    let correos: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE canal = 'correo' AND tenant_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("correos");
    // La factura externa la entrega quien la emitió: solo salen el recordatorio y el aviso.
    assert_eq!(correos, 2, "recordatorio y aviso de vencida");

    let pagado = cortes::marcar_pagado(
        &e.plataforma,
        &e.unidad,
        &a_mediodia(2026, 3, 18),
        &e.ctx,
        corte,
        dia(2026, 3, 18),
    )
    .await
    .expect("pago");

    assert_eq!(pagado.estado, EstadoCorte::Pagado);
    assert_eq!(
        e.estado_entidad().await,
        "activa",
        "el pago reactiva la entidad"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn no_se_suspende_con_una_asamblea_instalada(pool: PgPool) {
    let (e, s) = Entorno::new(&pool).await;
    enero_facturado(&e, s).await;
    sqlx::query(
        "INSERT INTO reuniones (id, tenant_id, titulo, tipo, estado, fecha_programada, instalada_en)
         VALUES ($1, $2, 'Asamblea', 'ordinaria', 'instalada', now(), now())",
    )
    .bind(Uuid::now_v7())
    .bind(e.tenant.as_uuid())
    .execute(&pool)
    .await
    .expect("reunión instalada");

    let ronda = tareas::procesar_mora(&e.plataforma, &e.unidad, &a_mediodia(2026, 3, 17))
        .await
        .expect("mora");

    assert_eq!(ronda.suspendidas, 0);
    assert_eq!(e.estado_entidad().await, "activa");
    let manual = application::casos_uso::plataforma::cambiar_estado_entidad(
        &e.plataforma,
        &e.unidad,
        &a_mediodia(2026, 3, 17),
        &e.ctx,
        e.tenant,
        EstadoCopropiedad::Suspendida,
    )
    .await;
    assert!(matches!(manual, Err(AppError::Conflicto(_))), "{manual:?}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_entidad_suspendida_queda_en_solo_lectura(pool: PgPool) {
    let (e, _) = Entorno::new(&pool).await;
    sqlx::query("UPDATE entidades SET estado = 'suspendida' WHERE id = $1")
        .bind(e.tenant.as_uuid())
        .execute(&pool)
        .await
        .expect("suspender");
    let cuenta = domain::CuentaId::nuevo();
    sqlx::query(
        "INSERT INTO cuentas (id, nombre_mostrar, usuario, hash_contrasena,
                              contrasena_actualizada_en, creada_en)
         VALUES ($1, 'Admin', 'admin_prueba', 'x', now(), now())",
    )
    .bind(cuenta.as_uuid())
    .execute(&pool)
    .await
    .expect("cuenta");
    sqlx::query(
        "INSERT INTO asignaciones_rol (id, tenant_id, cuenta_id, rol_id, origen, vigente_desde)
         VALUES (gen_random_uuid(), $1, $2, uuid_rol_plantilla('administrador'), 'manual',
                 now() - interval '1 day')",
    )
    .bind(e.tenant.as_uuid())
    .bind(cuenta.as_uuid())
    .execute(&pool)
    .await
    .expect("rol");

    let (permisos, suspendida) = application::casos_uso::roles::permisos_y_suspension(
        &e.unidad,
        &application::puertos::RelojSistema,
        e.tenant,
        cuenta,
        false,
    )
    .await
    .expect("permisos");

    assert!(suspendida);
    assert!(permisos.contiene(domain::Permiso::CensoVer));
    assert!(!permisos.contiene(domain::Permiso::CensoEditar));
}
