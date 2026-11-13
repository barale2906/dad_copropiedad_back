//! Integración de los planes 07b y 07c sobre PostgreSQL como `dad_app`: tickets (consecutivo
//! concurrente, idempotencia, RLS, historial append-only), funciones `SECURITY DEFINER` del chat
//! y de los avisos, sesiones del chat por turno y generación idempotente de órdenes.

mod comun;

use application::{
    AppError, Contexto, SolicitudPagina,
    casos_uso::{
        bienes_privados::{NuevoBienPrivado, crear_bien_privado},
        conversaciones::{EntradaConversacion, registrar_entrante},
        mantenimiento::{
            DatosPlan, NuevoActivo, generar_ordenes_preventivas, guardar_plan, registrar_activo,
            tablero_mantenimiento, tareas_gestion,
        },
        tickets::{ConsultaTickets, NuevoTicket, listar_tickets, radicar_ticket},
        titulares::{DatosPersona, RefPersona, SolicitudTitular, registrar_titulares},
    },
    puertos::{
        DirectorioGestion, DirectorioTelefonos, RelojSistema, RepositorioTickets,
        SesionesConversacion, UnidadDeTrabajo,
        mensajeria::{ContenidoSaliente, MensajeSaliente},
        sesiones::{CierreTurno, SesionGuardada},
    },
};
use chrono::{Duration, Utc};
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};
use domain::{
    BienPrivadoId, CanalTicket, ClaseActivo, CodigoActivo, CuentaId, DireccionCorreo, Documento,
    Frecuencia, NumeroBienPrivado, Permiso, PersonaId, Porcentaje, Recurrencia, Telefono, TenantId,
    TipoBienPrivado, TipoDocumento, TipoPersona, TipoTicket, TipoTitularidad, UsoBienPrivado,
};
use infra_postgres::{
    DirectorioGestionPg, DirectorioTelefonosPg, SesionesChatPg, UnidadDeTrabajoPg,
    transaccion_tenant,
};
use sqlx::PgPool;

const TELEFONO: &str = "573001112233";

struct Entorno {
    duenio: PgPool,
    app: PgPool,
    unidad: UnidadDeTrabajoPg,
    tenant: TenantId,
    bien: BienPrivadoId,
    persona: PersonaId,
}

async fn habilitar(pool: &PgPool, tenant: TenantId) {
    sqlx::query(
        "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
         VALUES (gen_random_uuid(), $1, 'gestion_administrativa', now() - interval '1 day')",
    )
    .bind(tenant.as_uuid())
    .execute(pool)
    .await
    .expect("módulo");
}

async fn entorno(pool: PgPool) -> Entorno {
    let tenant = crear_copropiedad(&pool).await;
    habilitar(&pool, tenant).await;
    let app = pool_como_app(&pool).await;
    let unidad = UnidadDeTrabajoPg::new(app.clone());
    let ctx = Contexto::new(tenant);
    let bien = crear_bien_privado(
        &unidad,
        &ctx,
        NuevoBienPrivado {
            tipo: TipoBienPrivado::Apartamento,
            agrupacion_id: None,
            numero: Some(NumeroBienPrivado::new("101").expect("válido")),
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
    .id;
    let persona = registrar_titulares(
        &unidad,
        &RelojSistema,
        &ctx,
        bien,
        vec![SolicitudTitular {
            persona: RefPersona::Nueva(DatosPersona {
                tipo: TipoPersona::Natural,
                nombre: "Ana Dueña".into(),
                documento: Documento::new("52123456").expect("válido"),
                telefono: Some(Telefono::new(TELEFONO).expect("válido")),
                correo: Some(DireccionCorreo::new("ana@correo.co").expect("válido")),
                cuenta_id: None,
                tipo_documento: Some(TipoDocumento::Cc),
            }),
            tipo: TipoTitularidad::Propietario,
            porcentaje: Porcentaje::new(100.into()).expect("válido"),
            es_representante: true,
        }],
    )
    .await
    .expect("titular")[0]
        .persona_id;
    Entorno {
        duenio: pool,
        app,
        unidad,
        tenant,
        bien,
        persona,
    }
}

fn nuevo(e: &Entorno, clave: &str) -> NuevoTicket {
    NuevoTicket {
        tipo: TipoTicket::SolicitudMantenimiento,
        categoria_id: None,
        prioridad: None,
        canal: CanalTicket::Whatsapp,
        asunto: "Fuga".into(),
        descripcion: "Gotea el techo".into(),
        bien_privado_id: Some(e.bien),
        ubicacion: None,
        recurso_id: None,
        activo_id: None,
        adjuntos: Vec::new(),
        idempotency_key: clave.into(),
        solicitante: Some(e.persona),
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn radicaciones_simultaneas_reciben_consecutivos_distintos(pool: PgPool) {
    let e = entorno(pool).await;
    let ctx = Contexto::new(e.tenant);
    let tareas: Vec<_> = (0..8)
        .map(|i| {
            let unidad = e.unidad.clone();
            let ctx = ctx.clone();
            let n = nuevo(&e, &format!("wamid.{i}"));
            tokio::spawn(async move { radicar_ticket(&unidad, &RelojSistema, &ctx, n).await })
        })
        .collect();
    let mut numeros = Vec::new();
    for t in tareas {
        numeros.push(t.await.expect("tarea").expect("radica").numero.consecutivo);
    }
    numeros.sort_unstable();
    assert_eq!(numeros, (1..=8).collect::<Vec<u32>>());
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_mismo_wamid_nunca_crea_dos_tickets(pool: PgPool) {
    let e = entorno(pool).await;
    let ctx = Contexto::new(e.tenant);
    let a = radicar_ticket(&e.unidad, &RelojSistema, &ctx, nuevo(&e, "wamid.X"))
        .await
        .expect("radica");
    let b = radicar_ticket(&e.unidad, &RelojSistema, &ctx, nuevo(&e, "wamid.X"))
        .await
        .expect("reintento");
    assert_eq!(a.id, b.id);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets")
        .fetch_one(&e.duenio)
        .await
        .expect("conteo");
    assert_eq!(n, 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn otra_copropiedad_no_ve_los_tickets(pool: PgPool) {
    let e = entorno(pool).await;
    radicar_ticket(
        &e.unidad,
        &RelojSistema,
        &Contexto::new(e.tenant),
        nuevo(&e, "w"),
    )
    .await
    .expect("radica");
    let otra = crear_copropiedad(&e.duenio).await;
    habilitar(&e.duenio, otra).await;
    let lista = listar_tickets(
        &e.unidad,
        &RelojSistema,
        &Contexto::new(otra),
        ConsultaTickets::default(),
        SolicitudPagina::primera(),
    )
    .await
    .expect("lista");
    assert!(lista.datos.is_empty());
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn historiales_son_de_solo_insercion(pool: PgPool) {
    let e = entorno(pool).await;
    radicar_ticket(
        &e.unidad,
        &RelojSistema,
        &Contexto::new(e.tenant),
        nuevo(&e, "w"),
    )
    .await
    .expect("radica");
    for sql in [
        "UPDATE ticket_eventos SET motivo = 'x'",
        "DELETE FROM ticket_eventos",
        "UPDATE ticket_mensajes SET texto = 'x'",
        "DELETE FROM movimientos_inventario",
    ] {
        let mut tx = transaccion_tenant(&e.app, e.tenant).await.expect("tx");
        let err = sqlx::query(sql).execute(&mut *tx).await.expect_err(sql);
        assert!(
            err.to_string().contains("permission denied"),
            "{sql}: {err}"
        );
    }
    // Ni siquiera el dueño del esquema puede reescribir el historial (trigger).
    let err = sqlx::query("UPDATE ticket_eventos SET motivo = 'x'")
        .execute(&e.duenio)
        .await
        .expect_err("trigger");
    assert!(err.to_string().contains("solo inserción"), "{err}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn vinculos_por_telefono_exige_ph_activa_y_modulo(pool: PgPool) {
    let e = entorno(pool).await;
    let directorio = DirectorioTelefonosPg::new(e.app.clone());
    // La copropiedad de prueba nace `en_configuracion`.
    assert!(
        directorio
            .vinculos_por_telefono(TELEFONO)
            .await
            .expect("consulta")
            .is_empty()
    );
    sqlx::query("UPDATE entidades SET estado = 'activa' WHERE id = $1")
        .bind(e.tenant.as_uuid())
        .execute(&e.duenio)
        .await
        .expect("activa");
    let v = directorio
        .vinculos_por_telefono(TELEFONO)
        .await
        .expect("consulta");
    assert_eq!(v.len(), 1);
    assert_eq!(
        (v[0].tenant_id, v[0].persona_id, v[0].bien_privado_id),
        (e.tenant, e.persona, e.bien)
    );
    assert!(v[0].es_propietario);
    assert!(
        directorio
            .vinculos_por_telefono("573000000000")
            .await
            .expect("consulta")
            .is_empty()
    );
    // El rol de la aplicación no lee directamente las tablas de otro tenant.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM personas")
        .fetch_one(&e.app)
        .await
        .expect("conteo");
    assert_eq!(n, 0);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn destinatarios_con_permiso_devuelve_las_cuentas_del_rol(pool: PgPool) {
    let e = entorno(pool).await;
    let cuenta = CuentaId::nuevo();
    sqlx::query(
        "INSERT INTO cuentas (id, nombre_mostrar, correo, hash_contrasena, contrasena_actualizada_en, creada_en)
         VALUES ($1, 'Admin', 'admin@ph.co', 'x', now(), now())",
    )
    .bind(cuenta.as_uuid())
    .execute(&e.duenio)
    .await
    .expect("cuenta");
    sqlx::query(
        "INSERT INTO asignaciones_rol (id, tenant_id, cuenta_id, rol_id, vigente_desde, origen)
         VALUES (gen_random_uuid(), $1, $2, uuid_rol_plantilla('administrador'), now() - interval '1 hour', 'manual')",
    )
    .bind(e.tenant.as_uuid())
    .bind(cuenta.as_uuid())
    .execute(&e.duenio)
    .await
    .expect("asignación");

    let mut tx = e
        .unidad
        .iniciar(&Contexto::new(e.tenant))
        .await
        .expect("tx");
    let urgencias = tx
        .destinatarios_con_permiso(Permiso::TicketsUrgenciasRecibir)
        .await
        .expect("consulta");
    assert_eq!(urgencias.len(), 1);
    assert_eq!(urgencias[0].cuenta_id, cuenta);
    assert_eq!(
        urgencias[0].correo.as_ref().map(|c| c.as_str()),
        Some("admin@ph.co")
    );
    // Nadie tiene `tickets.convivencia.ver` salvo el administrador; un permiso sin titulares.
    let nadie = tx
        .destinatarios_con_permiso(Permiso::PlataformaAuditoriaVer)
        .await
        .expect("consulta");
    assert!(nadie.is_empty());
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_turno_del_chat_se_cierra_una_sola_vez(pool: PgPool) {
    let app = pool_como_app(&pool).await;
    let sesiones = SesionesChatPg::new(app.clone());
    let ahora = Utc::now();
    let cierre = CierreTurno {
        linea: "PNID".into(),
        telefono: TELEFONO.into(),
        wamid: "wamid.1".into(),
        sesion: Some(SesionGuardada {
            estado: serde_json::json!({ "paso": "menu" }),
            ultimo_entrante_en: ahora,
        }),
        expira_en: ahora + Duration::minutes(30),
        respuestas: vec![MensajeSaliente {
            linea: "PNID".into(),
            telefono: TELEFONO.into(),
            contenido: ContenidoSaliente::Texto {
                cuerpo: "Hola".into(),
            },
        }],
    };
    assert!(!sesiones.ya_procesado("wamid.1").await.expect("consulta"));
    assert!(sesiones.cerrar_turno(&cierre).await.expect("cierra"));
    assert!(!sesiones.cerrar_turno(&cierre).await.expect("repetido"));
    assert!(sesiones.ya_procesado("wamid.1").await.expect("consulta"));

    let s = sesiones
        .obtener("PNID", TELEFONO, ahora)
        .await
        .expect("obtiene")
        .expect("sesión");
    assert_eq!(s.estado["paso"], "menu");
    // Vencida, ya no se devuelve.
    assert!(
        sesiones
            .obtener("PNID", TELEFONO, ahora + Duration::hours(1))
            .await
            .expect("obtiene")
            .is_none()
    );
    let respuestas: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE tenant_id IS NULL AND canal = 'whatsapp' AND prioridad = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("conteo");
    assert_eq!(respuestas, 1);
    assert!(
        sesiones
            .purgar(ahora + Duration::hours(1))
            .await
            .expect("purga")
            >= 1
    );
    // Sin `app.plataforma` el rol de la aplicación no ve las sesiones.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM sesiones_chat")
        .fetch_one(&app)
        .await
        .expect("conteo");
    assert_eq!(n, 0);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_mensaje_repetido_no_se_duplica_en_la_bandeja(pool: PgPool) {
    let e = entorno(pool).await;
    let ctx = Contexto::new(e.tenant);
    let entrada = EntradaConversacion {
        linea: "PNID".into(),
        telefono: TELEFONO.into(),
        persona_id: Some(e.persona),
        wamid: "wamid.b".into(),
        texto: Some("Hola".into()),
    };
    let a = registrar_entrante(&e.unidad, &RelojSistema, &ctx, entrada.clone())
        .await
        .expect("registra");
    let b = registrar_entrante(&e.unidad, &RelojSistema, &ctx, entrada)
        .await
        .expect("reintento");
    assert_eq!(a.id, b.id);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM conversacion_mensajes")
        .fetch_one(&e.duenio)
        .await
        .expect("conteo");
    assert_eq!(n, 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn las_ordenes_preventivas_no_se_repiten_y_el_worker_las_encuentra(pool: PgPool) {
    let e = entorno(pool).await;
    let ctx = Contexto::new(e.tenant);
    let activo = registrar_activo(
        &e.unidad,
        &RelojSistema,
        &ctx,
        NuevoActivo {
            codigo: CodigoActivo::new("BOMBA-01").expect("válido"),
            nombre: "Bomba".into(),
            clase: ClaseActivo::Equipo,
            marca: None,
            modelo: None,
            serie: None,
            ubicacion: None,
            agrupacion_id: None,
            recurso_id: None,
            fecha_compra: None,
            valor: None,
            garantia_vence: None,
            proveedor_organizacion_id: None,
        },
    )
    .await
    .expect("activo");
    let hoy = Utc::now()
        .with_timezone(&domain::reserva::Tz::America__Bogota)
        .date_naive();
    guardar_plan(
        &e.unidad,
        &RelojSistema,
        &ctx,
        None,
        DatosPlan {
            activo_id: Some(activo.id),
            ubicacion: None,
            actividad: "Revisión semanal".into(),
            recurrencia: Recurrencia::new(Frecuencia::Semanas, 1, hoy).expect("válida"),
            anticipacion_dias: 7,
            responsable: None,
            duracion_estimada_min: None,
            checklist: vec!["Presión".into()],
            activo: true,
        },
    )
    .await
    .expect("plan");

    let directorio = DirectorioGestionPg::new(e.app.clone());
    assert_eq!(
        directorio
            .copropiedades_con_gestion_pendiente()
            .await
            .expect("consulta"),
        vec![e.tenant]
    );
    let r = tareas_gestion(&directorio, &e.unidad, &RelojSistema)
        .await
        .expect("tareas");
    assert_eq!(r.ordenes_generadas, 2, "hoy y en 7 días");
    let otra = generar_ordenes_preventivas(&e.unidad, &RelojSistema, &ctx)
        .await
        .expect("genera");
    assert_eq!(otra, 0);

    // La BD también impide la orden repetida.
    let err = sqlx::query(
        "INSERT INTO ordenes_trabajo (id, tenant_id, anio, consecutivo, origen, tipo, plan_id,
                                      descripcion, programada_para, creado_en, actualizado_en)
         SELECT gen_random_uuid(), tenant_id, anio, 99, origen, tipo, plan_id, descripcion,
                programada_para, now(), now()
         FROM ordenes_trabajo LIMIT 1",
    )
    .execute(&e.duenio)
    .await
    .expect_err("única");
    assert!(err.to_string().contains("ux_orden_por_periodo"), "{err}");

    let t = tablero_mantenimiento(&e.unidad, &RelojSistema, &ctx)
        .await
        .expect("tablero");
    assert_eq!(t.preventivas_programadas, 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_el_modulo_los_tickets_responden_modulo_no_habilitado(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);
    let r = listar_tickets(
        &unidad,
        &RelojSistema,
        &Contexto::new(tenant),
        ConsultaTickets::default(),
        SolicitudPagina::primera(),
    )
    .await;
    assert!(matches!(r, Err(AppError::ModuloNoHabilitado(_))));
}
