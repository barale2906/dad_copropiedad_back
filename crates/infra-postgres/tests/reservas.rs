//! Integración del plan 04 sobre PostgreSQL como `dad_app`: EXCLUDE de franjas y de
//! asignaciones, concurrencia (exclusivos y aforo), idempotencia, sorteos, recordatorios y RLS.

mod comun;

use application::{
    AppError, Contexto,
    casos_uso::{
        bienes_privados::{NuevoBienPrivado, crear_bien_privado},
        parqueaderos::{
            NuevoParqueadero, NuevoSorteo, SolicitudAsignacionParqueadero, asignar_parqueadero,
            crear_parqueadero, crear_sorteo, ejecutar_sorteo, inscribirse_sorteo,
        },
        reservas::{
            NuevoRecurso, SolicitudReserva, crear_recurso, recordar_reservas, solicitar_reserva,
        },
    },
    puertos::{RelojSistema, RepositorioReservas, UnidadDeTrabajo, reservas::DetalleBienComun},
};
use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc, Weekday};
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};
use domain::{
    BienPrivadoId, CanalReserva, CategoriaBienComun, Franja, FranjaSemanal, Horario,
    ModoAsignacion, NaturalezaParqueadero, NumeroBienPrivado, OrigenAsignacionParqueadero,
    PoliticaReserva, TipoBienPrivado, TipoParqueadero, TipoRecurso, UsoBienPrivado, reserva::Tz,
    sorteo::CriteriosSorteo,
};
use infra_postgres::{DirectorioRecordatoriosPg, UnidadDeTrabajoPg};
use sqlx::PgPool;

struct RelojEn(DateTime<Utc>);

impl application::puertos::Reloj for RelojEn {
    fn ahora(&self) -> DateTime<Utc> {
        self.0
    }
}

struct Entorno {
    duenio: PgPool,
    app: PgPool,
    unidad: UnidadDeTrabajoPg,
    ctx: Contexto,
    bien: BienPrivadoId,
}

async fn bien(unidad: &UnidadDeTrabajoPg, ctx: &Contexto, n: &str) -> BienPrivadoId {
    crear_bien_privado(
        unidad,
        ctx,
        NuevoBienPrivado {
            tipo: TipoBienPrivado::Apartamento,
            agrupacion_id: None,
            numero: Some(NumeroBienPrivado::new(n).expect("válido")),
            codigo: None,
            piso: None,
            area_privada_m2: None,
            matricula_inmobiliaria: None,
            cedula_catastral: None,
            uso: UsoBienPrivado::Residencial,
        },
    )
    .await
    .expect("bien")
    .id
}

async fn entorno(pool: PgPool) -> Entorno {
    let tenant = crear_copropiedad(&pool).await;
    sqlx::query(
        "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
         VALUES (gen_random_uuid(), $1, 'bienes_comunes', now() - interval '1 day')",
    )
    .bind(tenant.as_uuid())
    .execute(&pool)
    .await
    .expect("módulo");
    let app = pool_como_app(&pool).await;
    let unidad = UnidadDeTrabajoPg::new(app.clone());
    let ctx = Contexto::new(tenant);
    let bien = bien(&unidad, &ctx, "101").await;
    Entorno {
        duenio: pool,
        app,
        unidad,
        ctx,
        bien,
    }
}

fn todos_los_dias() -> Horario {
    use Weekday::*;
    Horario::new(
        [Mon, Tue, Wed, Thu, Fri, Sat, Sun]
            .iter()
            .map(|d| FranjaSemanal {
                dia: *d,
                inicio: NaiveTime::from_hms_opt(6, 0, 0).expect("hora"),
                fin: NaiveTime::from_hms_opt(22, 0, 0).expect("hora"),
                aplica_festivos: true,
            })
            .collect(),
        vec![],
    )
    .expect("válido")
}

/// Franja local de Bogotá dentro de `dias` días a las `h1`–`h2` horas.
fn franja(dias: i64, h1: u32, h2: u32) -> Franja {
    let fecha = (Utc::now() + Duration::days(dias))
        .with_timezone(&Tz::America__Bogota)
        .date_naive();
    let a = |h: u32| -> DateTime<Utc> {
        Tz::America__Bogota
            .from_local_datetime(&fecha.and_hms_opt(h, 0, 0).expect("hora"))
            .single()
            .expect("hora local")
            .with_timezone(&Utc)
    };
    Franja::new(a(h1), a(h2)).expect("franja")
}

async fn salon(e: &Entorno, politica: PoliticaReserva) -> application::puertos::reservas::Recurso {
    crear_recurso(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        NuevoRecurso {
            tipo: TipoRecurso::BienComun,
            nombre: "Salón".into(),
            ubicacion: None,
            descripcion: None,
            politica,
            horario: todos_los_dias(),
            bien_comun: Some(DetalleBienComun {
                categoria: CategoriaBienComun::SalonSocial,
                area_m2: None,
                es_esencial: false,
                reglas: None,
                reglas_archivo_id: None,
            }),
        },
    )
    .await
    .expect("recurso")
}

fn solicitud(
    recurso: domain::RecursoId,
    bien: BienPrivadoId,
    f: Franja,
    clave: &str,
) -> SolicitudReserva {
    SolicitudReserva {
        recurso_id: recurso,
        bien_privado_id: bien,
        franja: f,
        invitados: 0,
        idempotency_key: clave.into(),
        canal: CanalReserva::Web,
        modalidad: None,
        motivo: None,
        placa_visitante: None,
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn dos_solicitudes_simultaneas_de_la_misma_franja_dejan_una_reserva(pool: PgPool) {
    let e = entorno(pool).await;
    let r = salon(&e, PoliticaReserva::default()).await;
    let f = franja(3, 14, 16);
    let tareas: Vec<_> = (0..6)
        .map(|i| {
            let unidad = e.unidad.clone();
            let ctx = e.ctx.clone();
            let s = solicitud(r.id, e.bien, f, &format!("c{i}"));
            tokio::spawn(async move { solicitar_reserva(&unidad, &RelojSistema, &ctx, s).await })
        })
        .collect();
    let mut ok = 0;
    for t in tareas {
        match t.await.expect("tarea") {
            Ok(_) => ok += 1,
            Err(AppError::Conflicto(_)) => {}
            Err(otro) => panic!("error inesperado: {otro:?}"),
        }
    }
    assert_eq!(ok, 1);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM reservas")
        .fetch_one(&e.duenio)
        .await
        .expect("conteo");
    assert_eq!(n, 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_cupo_de_aforo_no_se_supera_con_solicitudes_simultaneas(pool: PgPool) {
    let e = entorno(pool).await;
    let gimnasio = salon(
        &e,
        PoliticaReserva {
            modo: ModoAsignacion::FranjaInmediata,
            aforo: Some(2),
            ..PoliticaReserva::default()
        },
    )
    .await;
    let f = franja(2, 7, 8);
    let tareas: Vec<_> = (0..5)
        .map(|i| {
            let unidad = e.unidad.clone();
            let ctx = e.ctx.clone();
            let s = solicitud(gimnasio.id, e.bien, f, &format!("g{i}"));
            tokio::spawn(async move { solicitar_reserva(&unidad, &RelojSistema, &ctx, s).await })
        })
        .collect();
    let mut ok = 0;
    for t in tareas {
        if t.await.expect("tarea").is_ok() {
            ok += 1;
        }
    }
    assert_eq!(ok, 2, "aforo de 2 personas");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn solicitar_es_idempotente_en_la_bd(pool: PgPool) {
    let e = entorno(pool).await;
    let r = salon(&e, PoliticaReserva::default()).await;
    let s = solicitud(r.id, e.bien, franja(3, 10, 12), "misma");
    let a = solicitar_reserva(&e.unidad, &RelojSistema, &e.ctx, s.clone())
        .await
        .expect("1");
    let b = solicitar_reserva(&e.unidad, &RelojSistema, &e.ctx, s)
        .await
        .expect("2");
    assert_eq!(a.id, b.id);
    let mut tx = e.unidad.iniciar(&e.ctx).await.expect("tx");
    assert_eq!(
        tx.reserva(a.id).await.expect("lee").expect("existe").franja,
        a.franja
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sorteo_persistido_asigna_sin_solapes(pool: PgPool) {
    let e = entorno(pool).await;
    let mut parqueaderos = Vec::new();
    for c in ["C1", "C2"] {
        parqueaderos.push(
            crear_parqueadero(
                &e.unidad,
                &RelojSistema,
                &e.ctx,
                NuevoParqueadero {
                    codigo: c.into(),
                    tipo_vehiculo: TipoParqueadero::Carro,
                    naturaleza: NaturalezaParqueadero::Comunal,
                    bien_privado_id: None,
                    ubicacion: None,
                    politica: None,
                    horario: Horario::default(),
                },
            )
            .await
            .expect("parqueadero")
            .id,
        );
    }
    let desde = Utc::now() + Duration::days(10);
    let s = crear_sorteo(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        NuevoSorteo {
            nombre: "Semestre".into(),
            recursos: parqueaderos.clone(),
            vigencia_desde: desde,
            vigencia_hasta: desde + Duration::days(180),
            criterios: CriteriosSorteo::default(),
        },
    )
    .await
    .expect("sorteo");
    let mut bienes = vec![e.bien];
    for n in ["102", "103"] {
        bienes.push(bien(&e.unidad, &e.ctx, n).await);
    }
    for b in &bienes {
        inscribirse_sorteo(&e.unidad, &RelojSistema, &e.ctx, s.id, *b, false)
            .await
            .expect("inscribe");
    }
    let r = ejecutar_sorteo(&e.unidad, &RelojSistema, &e.ctx, s.id)
        .await
        .expect("ejecuta");
    assert_eq!(r.asignaciones.len(), 2);
    let semilla: String = sqlx::query_scalar("SELECT semilla FROM sorteos WHERE id = $1")
        .bind(s.id.as_uuid())
        .fetch_one(&e.duenio)
        .await
        .expect("semilla");
    assert_eq!(semilla, s.semilla.as_str());
    // La BD rechaza una segunda asignación del mismo parqueadero en el periodo.
    assert!(matches!(
        asignar_parqueadero(
            &e.unidad,
            &RelojSistema,
            &e.ctx,
            SolicitudAsignacionParqueadero {
                recurso_id: parqueaderos[0],
                bien_privado_id: bienes[0],
                vehiculo_id: None,
                origen: OrigenAsignacionParqueadero::Manual,
                desde: Some(desde + Duration::days(30)),
                hasta: Some(desde + Duration::days(40)),
            }
        )
        .await,
        Err(AppError::Conflicto(_))
    ));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn recordatorios_y_aislamiento_entre_copropiedades(pool: PgPool) {
    let e = entorno(pool).await;
    let r = salon(
        &e,
        PoliticaReserva {
            modo: ModoAsignacion::FranjaInmediata,
            anticipacion_minima_horas: 0,
            ..PoliticaReserva::default()
        },
    )
    .await;
    let f = franja(2, 10, 11);
    solicitar_reserva(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        solicitud(r.id, e.bien, f, "rec"),
    )
    .await
    .expect("reserva");
    let directorio = DirectorioRecordatoriosPg::new(e.app.clone());
    // Reloj fijo dos horas antes de la reserva: queda dentro de la ventana de 24 h.
    let reloj = RelojEn(f.inicio() - Duration::hours(2));
    assert_eq!(
        recordar_reservas(&directorio, &e.unidad, &reloj)
            .await
            .expect("recuerda"),
        1
    );
    assert_eq!(
        recordar_reservas(&directorio, &e.unidad, &reloj)
            .await
            .expect("recuerda"),
        0,
        "una sola vez"
    );
    let correos: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE canal = 'correo'")
        .fetch_one(&e.duenio)
        .await
        .expect("outbox");
    assert_eq!(
        correos, 0,
        "el solicitante (sistema) no tiene correo: se marca sin encolar"
    );

    let otra = Contexto::new(crear_copropiedad(&e.duenio).await);
    let mut tx = e.unidad.iniciar(&otra).await.expect("tx");
    assert!(tx.recursos(None).await.expect("lee").is_empty());
    assert!(
        tx.reservas(&Default::default())
            .await
            .expect("lee")
            .is_empty()
    );
}
