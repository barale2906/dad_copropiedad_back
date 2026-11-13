//! Integración del plan 06 sobre PostgreSQL como `dad_app`: flujo completo de una asamblea,
//! votos concurrentes (un solo registro por bien), doble escaneo del mismo rótulo, dos
//! asambleas simultáneas con el mismo teléfono, avisos en vivo (`pg_notify`), reparto de
//! líneas, acuses de entrega, temporizador y líneas/tarifas de plataforma.

mod comun;

use std::sync::Arc;

use application::{
    AppError, Contexto, ContextoPlataforma,
    casos_uso::asambleas::{
        lineas::{crear_linea, guardar_tarifa, listar_lineas},
        quorum::tablero_en_vivo,
        registro::{PersonaAsistente, PoderRecibido, RegistroAsistente, registrar_asistente},
        reportes::{costo_whatsapp, reporte_asistencia},
        reuniones::{
            DatosPunto, EnvioConvocatoriaOpciones, NuevaReunion, abrir_registro, convocar,
            crear_reunion, guardar_punto, instalar,
        },
        votaciones::{
            DefinicionPregunta, SolicitudVoto, Votante, abrir_pregunta, cerrar_pregunta,
            cerrar_vencidas, crear_pregunta, emitir_voto,
        },
        whatsapp::{ServiciosAsamblea, atender_mensaje_asamblea, id_voto, payload_confirmar},
    },
    puertos::{
        DirectorioAsambleas, RelojSistema, RepositorioAsambleas, UnidadDeTrabajo,
        asambleas::{AcuseEntrega, ConfiguracionReunion, PreguntaGuardada, Reunion},
        mensajeria::{ContenidoEntrante, MensajeEntrante},
    },
};
use chrono::{Duration, Utc};
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};
use domain::{
    AsistenteId, BienPrivadoId, Canal, ClaveIdempotencia, Convocatoria, Decision, MetodoRegistro,
    Modalidad, PersonaId, ReglaMayoria, TenantId, TipoPregunta, TipoReunion, TipoVoto,
};
use infra_postgres::{
    CANAL_EN_VIVO, DirectorioAsambleasPg, SesionesChatPg, UnidadDeTrabajoPg,
    UnidadDeTrabajoPlataformaPg,
};
use rust_decimal::Decimal;
use sqlx::{PgPool, postgres::PgListener};
use uuid::Uuid;

struct Ph {
    tenant: TenantId,
    ctx: Contexto,
    bienes: Vec<BienPrivadoId>,
    personas: Vec<PersonaId>,
}

fn telefono(base: u32, i: usize) -> String {
    format!("57{base:03}{i:07}")
}

/// Copropiedad activa con el módulo de asambleas y `n` bienes de coeficiente `coef`, cada uno
/// con un propietario representante con teléfono `57{base}{i}`.
async fn ph(duenio: &PgPool, n: usize, coef: Decimal, base: u32) -> Ph {
    let tenant = crear_copropiedad(duenio).await;
    sqlx::query("UPDATE entidades SET estado = 'activa' WHERE id = $1")
        .bind(tenant.as_uuid())
        .execute(duenio)
        .await
        .expect("copropiedad activa");
    sqlx::query(
        "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
         VALUES (gen_random_uuid(), $1, 'asambleas', now() - interval '1 day')",
    )
    .bind(tenant.as_uuid())
    .execute(duenio)
    .await
    .expect("módulo");
    let bienes: Vec<Uuid> = (0..n).map(|_| Uuid::now_v7()).collect();
    let personas: Vec<Uuid> = (0..n).map(|_| Uuid::now_v7()).collect();
    let codigos: Vec<String> = (0..n).map(|i| format!("B-{i:04}")).collect();
    let telefonos: Vec<String> = (0..n).map(|i| telefono(base, i)).collect();
    let mut tx = duenio.begin().await.expect("tx");
    sqlx::query(
        "INSERT INTO bienes_privados (id, tenant_id, codigo, coeficiente)
         SELECT b, $1, c, $4 FROM UNNEST($2::uuid[], $3::text[]) AS x (b, c)",
    )
    .bind(tenant.as_uuid())
    .bind(&bienes)
    .bind(&codigos)
    .bind(coef)
    .execute(&mut *tx)
    .await
    .expect("bienes");
    sqlx::query(
        "INSERT INTO personas (id, tenant_id, nombre, telefono)
         SELECT p, $1, 'Propietario ' || c, t FROM UNNEST($2::uuid[], $3::text[], $4::text[]) AS x (p, c, t)",
    )
    .bind(tenant.as_uuid())
    .bind(&personas)
    .bind(&codigos)
    .bind(&telefonos)
    .execute(&mut *tx)
    .await
    .expect("personas");
    sqlx::query(
        "INSERT INTO titularidades (id, tenant_id, bien_privado_id, persona_id, tipo, porcentaje,
                                    es_representante, vigente_desde)
         SELECT gen_random_uuid(), $1, b, p, 'propietario', 100, TRUE, now() - interval '1 day'
         FROM UNNEST($2::uuid[], $3::uuid[]) AS x (b, p)",
    )
    .bind(tenant.as_uuid())
    .bind(&bienes)
    .bind(&personas)
    .execute(&mut *tx)
    .await
    .expect("titularidades");
    tx.commit().await.expect("commit");
    Ph {
        tenant,
        ctx: Contexto::new(tenant),
        bienes: bienes.into_iter().map(BienPrivadoId::from_uuid).collect(),
        personas: personas.into_iter().map(PersonaId::from_uuid).collect(),
    }
}

async fn linea(duenio: &PgPool, pnid: &str) {
    sqlx::query(
        "INSERT INTO lineas_whatsapp (id, phone_number_id, numero_visible, nombre)
         VALUES (gen_random_uuid(), $1, $2, 'Línea')",
    )
    .bind(pnid)
    .bind(format!("+57 {pnid}"))
    .execute(duenio)
    .await
    .expect("línea");
}

fn nueva() -> NuevaReunion {
    NuevaReunion {
        titulo: "Asamblea ordinaria 2027".into(),
        tipo: TipoReunion::Ordinaria,
        convocatoria: Convocatoria::Primera,
        modalidad: Modalidad::Presencial,
        fecha_programada: Utc::now() + Duration::days(20),
        lugar: Some("Salón comunal".into()),
        enlace_video: None,
        plazo_respuesta: None,
        configuracion: ConfiguracionReunion::default(),
    }
}

async fn en_registro(u: &UnidadDeTrabajoPg, p: &Ph) -> Reunion {
    let r = crear_reunion(u, &RelojSistema, &p.ctx, nueva())
        .await
        .expect("crea");
    guardar_punto(
        u,
        &RelojSistema,
        &p.ctx,
        r.id,
        None,
        DatosPunto {
            orden: 1,
            titulo: "Quórum".into(),
            descripcion: None,
        },
    )
    .await
    .expect("punto");
    convocar(
        u,
        &RelojSistema,
        &p.ctx,
        r.id,
        EnvioConvocatoriaOpciones::default(),
    )
    .await
    .expect("convoca");
    abrir_registro(u, &RelojSistema, &p.ctx, r.id)
        .await
        .expect("registro")
}

async fn registrar(
    u: &UnidadDeTrabajoPg,
    p: &Ph,
    r: &Reunion,
    i: usize,
    poderes: &[usize],
) -> AsistenteId {
    registrar_asistente(
        u,
        &RelojSistema,
        &p.ctx,
        RegistroAsistente {
            reunion_id: r.id,
            persona: PersonaAsistente::Existente(p.personas[i]),
            telefono: None,
            bienes_propios: vec![p.bienes[i]],
            poderes: poderes
                .iter()
                .map(|j| PoderRecibido {
                    bien_privado_id: p.bienes[*j],
                    soporte_archivo_id: None,
                })
                .collect(),
            rotulo: None,
            enviar_confirmacion: true,
            metodo: MetodoRegistro::Mesa,
        },
    )
    .await
    .expect("registra")
    .asistente
    .id
}

fn si_no() -> DefinicionPregunta {
    DefinicionPregunta {
        enunciado: "¿Aprueba el presupuesto 2027?".into(),
        tipo: TipoPregunta::SiNo,
        tipo_voto: TipoVoto::Nominal,
        base_computo: None,
        regla_mayoria: ReglaMayoria::Simple,
        max_selecciones: 1,
        cupos: None,
        cargo: None,
        opciones: Vec::new(),
        con_abstencion: false,
        punto_id: None,
        temporizador_segundos: None,
    }
}

async fn abierta(u: &UnidadDeTrabajoPg, p: &Ph, r: &Reunion) -> PreguntaGuardada {
    let q = crear_pregunta(u, &RelojSistema, &p.ctx, r.id, si_no())
        .await
        .expect("pregunta");
    abrir_pregunta(u, &RelojSistema, &p.ctx, r.id, q.pregunta.id)
        .await
        .expect("abre")
}

fn voto(
    r: &Reunion,
    q: &PreguntaGuardada,
    a: AsistenteId,
    opcion: usize,
    clave: &str,
) -> SolicitudVoto {
    SolicitudVoto {
        reunion_id: r.id,
        pregunta_id: q.pregunta.id,
        votante: Votante::Asistente(a),
        opciones: vec![q.pregunta.opciones[opcion].id],
        canal: Canal::Web,
        idempotency_key: ClaveIdempotencia::new(clave).expect("clave"),
        wamid: None,
        confirmar: false,
    }
}

async fn contar(duenio: &PgPool, sql: &str, id: Uuid) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .bind(id)
        .fetch_one(duenio)
        .await
        .expect("conteo")
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn flujo_completo_de_una_asamblea(duenio: PgPool) {
    linea(&duenio, "PNID-1").await;
    let p = ph(&duenio, 5, Decimal::new(20, 2), 300).await;
    let u = UnidadDeTrabajoPg::new(pool_como_app(&duenio).await);
    let r = en_registro(&u, &p).await;
    assert_eq!(
        contar(
            &duenio,
            "SELECT count(*) FROM padron_reunion WHERE reunion_id = $1",
            r.id.as_uuid()
        )
        .await,
        5
    );
    assert_eq!(
        contar(
            &duenio,
            "SELECT count(*) FROM envios_convocatoria WHERE reunion_id = $1",
            r.id.as_uuid()
        )
        .await,
        5
    );

    let a = registrar(&u, &p, &r, 0, &[1, 2]).await;
    let r = instalar(&u, &RelojSistema, &p.ctx, r.id)
        .await
        .expect("instala con 0.60");
    let q = abierta(&u, &p, &r).await;
    let res = emitir_voto(&u, &RelojSistema, &p.ctx, voto(&r, &q, a, 0, "web-1"))
        .await
        .expect("vota");
    assert_eq!(res.bienes.len(), 3);
    let reintento = emitir_voto(&u, &RelojSistema, &p.ctx, voto(&r, &q, a, 1, "web-1"))
        .await
        .expect("reintento");
    assert!(!reintento.registrado);
    assert!(matches!(
        emitir_voto(&u, &RelojSistema, &p.ctx, voto(&r, &q, a, 1, "web-2")).await,
        Err(AppError::BienPrivadoYaVoto)
    ));
    assert_eq!(
        contar(
            &duenio,
            "SELECT count(*) FROM participaciones WHERE pregunta_id = $1",
            q.pregunta.id.as_uuid()
        )
        .await,
        3
    );

    let tablero = tablero_en_vivo(&u, &RelojSistema, &p.ctx, r.id)
        .await
        .expect("tablero");
    assert!(tablero.hay_quorum);
    assert_eq!(tablero.preguntas_abiertas.len(), 1);
    assert_eq!(
        tablero.preguntas_abiertas[0].escrutinio.votantes.unidades,
        3
    );

    let cerrada = cerrar_pregunta(&u, &RelojSistema, &p.ctx, r.id, q.pregunta.id)
        .await
        .expect("cierra");
    assert_eq!(
        cerrada.resultado.expect("escrutinio").decision,
        Decision::Aprobada
    );
    // El escrutinio queda guardado en la fila (JSONB) y se lee igual.
    let mut tx = u.iniciar(&p.ctx).await.expect("tx");
    let leida = tx
        .pregunta(q.pregunta.id)
        .await
        .expect("lee")
        .expect("existe");
    assert_eq!(
        leida.resultado.expect("guardado").decision,
        Decision::Aprobada
    );
    drop(tx);

    let asistencia = reporte_asistencia(&u, &RelojSistema, &p.ctx, r.id)
        .await
        .expect("reporte");
    assert_eq!(asistencia.len(), 3);
    // Costo: marca enviados los mensajes de la asamblea y los valoriza.
    sqlx::query("UPDATE outbox SET estado = 'enviado' WHERE reunion_id = $1")
        .bind(r.id.as_uuid())
        .execute(&duenio)
        .await
        .expect("enviados");
    let costo = costo_whatsapp(&u, &RelojSistema, &p.ctx, r.id, "COP")
        .await
        .expect("costo");
    assert_eq!(
        costo.plantillas, 6,
        "5 convocatorias + 1 confirmar_asistencia"
    );
    assert!(costo.valor_estimado > Decimal::ZERO);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn seiscientos_votos_simultaneos_quedan_uno_por_bien(duenio: PgPool) {
    let n = 600;
    let p = Arc::new(ph(&duenio, n, Decimal::new(1, 3), 301).await);
    let app = sqlx::postgres::PgPoolOptions::new()
        .max_connections(16)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::Executor::execute(c, "SET ROLE dad_app").await?;
                Ok(())
            })
        })
        .connect_with((*duenio.connect_options()).clone())
        .await
        .expect("pool");
    let u = UnidadDeTrabajoPg::new(app);
    let r = en_registro(&u, &p).await;
    // Asistentes sembrados de una vez (la mesa no es lo que se prueba aquí).
    sqlx::query(
        "WITH a AS (
             INSERT INTO asistentes (id, tenant_id, reunion_id, persona_id, metodo)
             SELECT gen_random_uuid(), $1, $2, representante_persona_id, 'mesa'
             FROM padron_reunion WHERE reunion_id = $2
             RETURNING id, persona_id),
         b AS (
             INSERT INTO asistente_bienes (id, tenant_id, reunion_id, asistente_id, bien_privado_id, calidad, desde)
             SELECT gen_random_uuid(), $1, $2, a.id, pr.bien_privado_id, 'propietario', now()
             FROM a JOIN padron_reunion pr ON pr.reunion_id = $2 AND pr.representante_persona_id = a.persona_id)
         INSERT INTO asistente_presencias (id, tenant_id, reunion_id, asistente_id, entrada_en)
         SELECT gen_random_uuid(), $1, $2, a.id, now() FROM a",
    )
    .bind(p.tenant.as_uuid())
    .bind(r.id.as_uuid())
    .execute(&duenio)
    .await
    .expect("asistentes");
    let r = instalar(&u, &RelojSistema, &p.ctx, r.id)
        .await
        .expect("instala");
    let q = abierta(&u, &p, &r).await;
    let asistentes: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM asistentes WHERE reunion_id = $1")
            .bind(r.id.as_uuid())
            .fetch_all(&duenio)
            .await
            .expect("ids");
    assert_eq!(asistentes.len(), n);

    // Cada asistente vota dos veces a la vez con claves distintas (doble toque / dos canales).
    let mut tareas = Vec::new();
    for (i, a) in asistentes.iter().enumerate() {
        for intento in 0..2 {
            let (u, p, r, q) = (u.clone(), Arc::clone(&p), r.clone(), q.clone());
            let a = AsistenteId::from_uuid(*a);
            tareas.push(tokio::spawn(async move {
                emitir_voto(
                    &u,
                    &RelojSistema,
                    &p.ctx,
                    voto(&r, &q, a, (i + intento) % 2, &format!("{a}-{intento}")),
                )
                .await
            }));
        }
    }
    let mut registrados = 0;
    for t in tareas {
        match t.await.expect("tarea") {
            Ok(res) if res.registrado => registrados += 1,
            Ok(_) | Err(AppError::BienPrivadoYaVoto) => {}
            Err(e) => panic!("error inesperado: {e}"),
        }
    }
    assert_eq!(registrados, n);
    assert_eq!(
        contar(
            &duenio,
            "SELECT count(*) FROM participaciones WHERE pregunta_id = $1",
            q.pregunta.id.as_uuid()
        )
        .await,
        i64::try_from(n).expect("n")
    );
    assert_eq!(
        contar(
            &duenio,
            "SELECT count(*) FROM votos WHERE pregunta_id = $1",
            q.pregunta.id.as_uuid()
        )
        .await,
        i64::try_from(n).expect("n")
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn doble_escaneo_del_mismo_rotulo_en_dos_puestos_registra_un_voto(duenio: PgPool) {
    let p = ph(&duenio, 3, Decimal::new(30, 2), 302).await;
    let u = UnidadDeTrabajoPg::new(pool_como_app(&duenio).await);
    let r = en_registro(&u, &p).await;
    let a = registrar(&u, &p, &r, 0, &[1]).await;
    application::casos_uso::asambleas::registro::asignar_rotulo(
        &u,
        &RelojSistema,
        &p.ctx,
        r.id,
        a,
        Some(domain::CodigoRotulo::new("R-0007").expect("rótulo")),
    )
    .await
    .expect("rótulo");
    let r = instalar(&u, &RelojSistema, &p.ctx, r.id)
        .await
        .expect("instala");
    let q = abierta(&u, &p, &r).await;
    let lectura = |clave: &'static str| SolicitudVoto {
        votante: Votante::Rotulo(domain::CodigoRotulo::new("R-0007").expect("rótulo")),
        canal: Canal::Rotulo,
        ..voto(&r, &q, a, 0, clave)
    };
    let (x, y) = tokio::join!(
        emitir_voto(&u, &RelojSistema, &p.ctx, lectura("puesto-si")),
        emitir_voto(&u, &RelojSistema, &p.ctx, lectura("puesto-no"))
    );
    assert_eq!(
        [&x, &y]
            .iter()
            .filter(|r| r.as_ref().is_ok_and(|v| v.registrado))
            .count(),
        1
    );
    assert_eq!(
        contar(
            &duenio,
            "SELECT count(*) FROM participaciones WHERE pregunta_id = $1",
            q.pregunta.id.as_uuid()
        )
        .await,
        2
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn dos_asambleas_simultaneas_con_el_mismo_telefono_no_mezclan_votos(duenio: PgPool) {
    linea(&duenio, "PNID-2").await;
    let a = ph(&duenio, 2, Decimal::new(50, 2), 303).await;
    let b = ph(&duenio, 2, Decimal::new(50, 2), 303).await; // mismos teléfonos en otra PH
    let app = pool_como_app(&duenio).await;
    let u = UnidadDeTrabajoPg::new(app.clone());
    let ra = en_registro(&u, &a).await;
    let rb = en_registro(&u, &b).await;
    let asis_a = registrar(&u, &a, &ra, 0, &[1]).await;
    let asis_b = registrar(&u, &b, &rb, 0, &[1]).await;
    let ra = instalar(&u, &RelojSistema, &a.ctx, ra.id)
        .await
        .expect("instala A");
    let rb = instalar(&u, &RelojSistema, &b.ctx, rb.id)
        .await
        .expect("instala B");
    let qa = abierta(&u, &a, &ra).await;
    let qb = abierta(&u, &b, &rb).await;

    let directorio = DirectorioAsambleasPg::new(app.clone());
    let asistencias = directorio
        .asistencias_por_telefono(&telefono(303, 0))
        .await
        .expect("directorio");
    assert_eq!(asistencias.len(), 2);
    let sesiones = SesionesChatPg::new(app.clone());
    let s = ServiciosAsamblea {
        unidad: &u,
        directorio: &directorio,
        sesiones: &sesiones,
        reloj: &RelojSistema,
    };
    let mensaje = |wamid: &str, contenido| MensajeEntrante {
        linea: "PNID-2".into(),
        telefono: telefono(303, 0),
        wamid: wamid.into(),
        recibido_en: Utc::now(),
        contenido,
    };
    // Confirma en ambas.
    for (t, asis) in [(a.tenant, asis_a), (b.tenant, asis_b)] {
        atender_mensaje_asamblea(
            &s,
            &mensaje(
                &format!("wamid.c.{t}"),
                ContenidoEntrante::Respuesta {
                    id: payload_confirmar(t, asis),
                    titulo: "Confirmar".into(),
                },
            ),
        )
        .await
        .expect("confirma");
    }
    // Texto libre con dos pendientes (una por asamblea): no vota.
    atender_mensaje_asamblea(
        &s,
        &mensaje(
            "wamid.t",
            ContenidoEntrante::Texto {
                cuerpo: "sí".into(),
            },
        ),
    )
    .await
    .expect("texto");
    for q in [&qa, &qb] {
        assert_eq!(
            contar(
                &duenio,
                "SELECT count(*) FROM participaciones WHERE pregunta_id = $1",
                q.pregunta.id.as_uuid()
            )
            .await,
            0
        );
    }
    // El botón de la PH A vota solo en A.
    atender_mensaje_asamblea(
        &s,
        &mensaje(
            "wamid.va",
            ContenidoEntrante::Respuesta {
                id: id_voto(a.tenant, qa.pregunta.id, qa.pregunta.opciones[0].id),
                titulo: "1. Sí".into(),
            },
        ),
    )
    .await
    .expect("vota A");
    assert_eq!(
        contar(
            &duenio,
            "SELECT count(*) FROM participaciones WHERE pregunta_id = $1",
            qa.pregunta.id.as_uuid()
        )
        .await,
        2
    );
    assert_eq!(
        contar(
            &duenio,
            "SELECT count(*) FROM participaciones WHERE pregunta_id = $1",
            qb.pregunta.id.as_uuid()
        )
        .await,
        0
    );
    // Ahora solo queda la pregunta de B pendiente: el texto «no» vota en B.
    atender_mensaje_asamblea(
        &s,
        &mensaje(
            "wamid.tb",
            ContenidoEntrante::Texto {
                cuerpo: "no".into(),
            },
        ),
    )
    .await
    .expect("vota B");
    assert_eq!(
        contar(
            &duenio,
            "SELECT count(*) FROM participaciones WHERE pregunta_id = $1",
            qb.pregunta.id.as_uuid()
        )
        .await,
        2
    );
    // Reintento de Meta del mismo wamid: sin duplicados.
    atender_mensaje_asamblea(
        &s,
        &mensaje(
            "wamid.tb",
            ContenidoEntrante::Texto {
                cuerpo: "no".into(),
            },
        ),
    )
    .await
    .expect("reintento");
    assert_eq!(
        contar(
            &duenio,
            "SELECT count(*) FROM participaciones WHERE pregunta_id = $1",
            qb.pregunta.id.as_uuid()
        )
        .await,
        2
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn los_cambios_se_avisan_en_vivo_al_confirmar(duenio: PgPool) {
    let p = ph(&duenio, 2, Decimal::new(50, 2), 304).await;
    let u = UnidadDeTrabajoPg::new(pool_como_app(&duenio).await);
    let mut escucha = PgListener::connect_with(&duenio).await.expect("listener");
    escucha.listen(CANAL_EN_VIVO).await.expect("listen");
    let r = en_registro(&u, &p).await;
    // Una transacción que no se confirma no avisa.
    {
        let mut tx = u.iniciar(&p.ctx).await.expect("tx");
        tx.avisar_en_vivo(r.id).await.expect("aviso");
    }
    registrar(&u, &p, &r, 0, &[]).await;
    let mut vistos = Vec::new();
    while let Ok(Ok(n)) =
        tokio::time::timeout(std::time::Duration::from_millis(300), escucha.recv()).await
    {
        vistos.push(n.payload().to_owned());
    }
    assert!(
        vistos
            .iter()
            .all(|v| v == &format!("{}:{}", p.tenant, r.id))
    );
    assert!(!vistos.is_empty());
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn elegir_linea_reparte_y_el_acuse_fallido_llega_a_la_mesa(duenio: PgPool) {
    linea(&duenio, "PNID-A").await;
    linea(&duenio, "PNID-B").await;
    let p = ph(&duenio, 4, Decimal::new(25, 2), 305).await;
    let app = pool_como_app(&duenio).await;
    let u = UnidadDeTrabajoPg::new(app.clone());
    let r = en_registro(&u, &p).await;
    // Las 4 convocatorias se repartieron entre las dos líneas.
    let por_linea: Vec<(String, i64)> = sqlx::query_as(
        "SELECT linea, count(*) FROM outbox WHERE reunion_id = $1 AND canal = 'whatsapp' GROUP BY linea ORDER BY linea",
    )
    .bind(r.id.as_uuid())
    .fetch_all(&duenio)
    .await
    .expect("reparto");
    assert_eq!(por_linea, vec![("PNID-A".into(), 2), ("PNID-B".into(), 2)]);

    let a = registrar(&u, &p, &r, 0, &[]).await;
    let outbox: i64 =
        sqlx::query_scalar("SELECT confirmacion_outbox_id FROM asistentes WHERE id = $1")
            .bind(a.as_uuid())
            .fetch_one(&duenio)
            .await
            .expect("outbox");
    sqlx::query(
        "UPDATE outbox SET estado = 'enviado', referencia_salida = 'wamid.salida' WHERE id = $1",
    )
    .bind(outbox)
    .execute(&duenio)
    .await
    .expect("enviado");
    let directorio = DirectorioAsambleasPg::new(app);
    let acuse = |estado: &str| AcuseEntrega {
        wamid: "wamid.salida".into(),
        linea: "PNID-A".into(),
        estado: estado.into(),
        categoria: Some("utility".into()),
        facturable: Some(true),
        error_codigo: (estado == "failed").then_some(131_026),
        ocurrido_en: Utc::now(),
    };
    directorio
        .registrar_acuse(&acuse("sent"))
        .await
        .expect("sent");
    directorio
        .registrar_acuse(&acuse("failed"))
        .await
        .expect("failed");
    directorio
        .registrar_acuse(&acuse("failed"))
        .await
        .expect("idempotente");
    let filas = application::casos_uso::asambleas::registro::asistentes_de_reunion(
        &u,
        &RelojSistema,
        &p.ctx,
        r.id,
    )
    .await
    .expect("mesa");
    assert_eq!(filas[0].entrega_confirmacion.as_deref(), Some("failed"));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn temporizador_vencido_se_detecta_y_se_cierra(duenio: PgPool) {
    let p = ph(&duenio, 2, Decimal::new(50, 2), 306).await;
    let app = pool_como_app(&duenio).await;
    let u = UnidadDeTrabajoPg::new(app.clone());
    let r = en_registro(&u, &p).await;
    registrar(&u, &p, &r, 0, &[1]).await;
    let r = instalar(&u, &RelojSistema, &p.ctx, r.id)
        .await
        .expect("instala");
    let q = abierta(&u, &p, &r).await;
    sqlx::query("UPDATE preguntas SET cierra_en = now() - interval '1 second' WHERE id = $1")
        .bind(q.pregunta.id.as_uuid())
        .execute(&duenio)
        .await
        .expect("vence");
    let pendientes = DirectorioAsambleasPg::new(app)
        .copropiedades_con_asambleas_pendientes()
        .await
        .expect("pendientes");
    assert_eq!(pendientes, vec![p.tenant]);
    assert_eq!(
        cerrar_vencidas(&u, &RelojSistema, &p.ctx)
            .await
            .expect("cierra"),
        1
    );
    let estado: String = sqlx::query_scalar("SELECT estado FROM preguntas WHERE id = $1")
        .bind(q.pregunta.id.as_uuid())
        .fetch_one(&duenio)
        .await
        .expect("estado");
    assert_eq!(estado, "cerrada");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn lineas_y_tarifas_de_plataforma(duenio: PgPool) {
    let plataforma = UnidadDeTrabajoPlataformaPg::new(pool_como_app(&duenio).await);
    let ctx = ContextoPlataforma::sistema();
    crear_linea(&plataforma, &ctx, "PNID-9", "+57 300 000 0009", "Línea 9")
        .await
        .expect("crea");
    assert!(matches!(
        crear_linea(&plataforma, &ctx, "PNID-9", "+57 300 000 0009", "Otra").await,
        Err(AppError::Conflicto(_))
    ));
    let lineas = listar_lineas(&plataforma, &ctx).await.expect("lista");
    assert_eq!(lineas.len(), 1);
    assert_eq!(lineas[0].franquicia_servicio_mes, 1000);
    guardar_tarifa(
        &plataforma,
        &ctx,
        "CO",
        "utilidad",
        "COP",
        Decimal::new(310, 2),
        chrono::NaiveDate::from_ymd_opt(2027, 1, 1).expect("fecha"),
    )
    .await
    .expect("tarifa");
    let sin_permiso = ContextoPlataforma::anonimo(None);
    assert!(matches!(
        listar_lineas(&plataforma, &sin_permiso).await,
        Err(AppError::SinPermiso(_))
    ));
}

/// Encola `n` mensajes de WhatsApp de plataforma por `linea` (`plantilla` o `texto`).
async fn mensajes(duenio: &PgPool, linea: &str, tipo: &str, n: i32) {
    sqlx::query(
        "INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload)
         SELECT NULL, 'whatsapp', 2, $1, '573000000000', jsonb_build_object('contenido', jsonb_build_object('tipo', $2::text))
         FROM generate_series(1, $3)",
    )
    .bind(linea)
    .bind(tipo)
    .bind(n)
    .execute(duenio)
    .await
    .expect("outbox");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_rotacion_cuenta_solo_mensajes_de_servicio_y_los_avisos_rotan_igual(duenio: PgPool) {
    linea(&duenio, "PNID-A").await;
    linea(&duenio, "PNID-B").await;
    let app = pool_como_app(&duenio).await;
    let elegida = |sql: &'static str| {
        let app = app.clone();
        async move {
            sqlx::query_scalar::<_, String>(sql)
                .fetch_one(&app)
                .await
                .expect("línea")
        }
    };
    // A envió 500 plantillas (se pagan aparte) y B 10 mensajes de servicio: A conserva toda su
    // franquicia gratuita, así que la siguiente conversación sale por A.
    mensajes(&duenio, "PNID-A", "plantilla", 500).await;
    mensajes(&duenio, "PNID-B", "texto", 10).await;
    assert_eq!(elegida("SELECT elegir_linea_whatsapp()").await, "PNID-A");
    // A gasta 995 de servicio (le quedan 5; a B, 990): ahora sale por B, también para los
    // avisos de tickets.
    mensajes(&duenio, "PNID-A", "botones", 995).await;
    assert_eq!(elegida("SELECT elegir_linea_whatsapp()").await, "PNID-B");
    assert_eq!(
        elegida("SELECT linea_whatsapp_predeterminada()").await,
        "PNID-B"
    );

    let plataforma = UnidadDeTrabajoPlataformaPg::new(app);
    let lineas = listar_lineas(&plataforma, &ContextoPlataforma::sistema())
        .await
        .expect("líneas");
    let a = lineas
        .iter()
        .find(|l| l.phone_number_id == "PNID-A")
        .expect("A");
    assert_eq!(
        (a.servicio_mes, a.plantillas_mes, a.franquicia_disponible()),
        (995, 500, 5)
    );
    let b = lineas
        .iter()
        .find(|l| l.phone_number_id == "PNID-B")
        .expect("B");
    assert_eq!(
        (b.servicio_mes, b.plantillas_mes, b.franquicia_disponible()),
        (10, 0, 990)
    );
}
