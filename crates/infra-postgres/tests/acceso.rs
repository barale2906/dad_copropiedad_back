//! Integración del plan 05 sobre PostgreSQL como `dad_app`: visitas con avisos marcados para el
//! costo, decisión única bajo concurrencia, RLS, historiales append-only, eventos idempotentes y
//! particionados, versión de sincronización movida por el censo, funciones `SECURITY DEFINER`
//! (puentes, tokens push, desvío a WhatsApp, purga de eventos).

mod comun;

use application::{
    AppError, Contexto, SolicitudPagina,
    casos_uso::{
        acceso::{
            credenciales::{NuevaCredencial, emitir_credencial},
            puentes::{autenticar_puente, registrar_puente},
            tareas::escalar_avisos,
            visitas::{NuevaVisita, ResultadoDecision, anunciar_visita, decidir_visita},
        },
        bienes_privados::{NuevoBienPrivado, crear_bien_privado},
        titulares::{DatosPersona, RefPersona, SolicitudTitular, registrar_titulares},
    },
    outbox::{Canal, EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        ColaSalida, DirectorioAcceso, LoteSalida, Reloj, RelojSistema, RepositorioAcceso,
        Transaccion, UnidadDeTrabajo,
        acceso::{EventoAcceso, FiltroVisitas, NotificacionPush, TitularAcceso},
        mensajeria::{ContenidoSaliente, MensajeSaliente},
    },
};
use chrono::{DateTime, TimeDelta, Utc};
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};
use domain::{
    BienPrivadoId, CanalRespuestaVisita, ConjuntoPermisos, CuentaId, Documento, EventoAccesoId,
    NumeroBienPrivado, Permiso, PersonaId, Porcentaje, PuenteId, ResultadoAcceso, SentidoAcceso,
    Telefono, TenantId, TipoBienPrivado, TipoCredencial, TipoDocumento, TipoPersona,
    TipoTitularidad, TipoVisita, UsoBienPrivado,
};
use infra_postgres::{ColaSalidaPg, DirectorioAccesoPg, UnidadDeTrabajoPg, transaccion_tenant};
use sqlx::PgPool;

const TELEFONO: &str = "573001112233";

struct RelojFijo(DateTime<Utc>);

impl Reloj for RelojFijo {
    fn ahora(&self) -> DateTime<Utc> {
        self.0
    }
}

struct Entorno {
    duenio: PgPool,
    app: PgPool,
    unidad: UnidadDeTrabajoPg,
    tenant: TenantId,
    bien: BienPrivadoId,
    persona: PersonaId,
    cuenta: CuentaId,
    vigilante: CuentaId,
}

async fn crear_cuenta(pool: &PgPool, usuario: &str) -> CuentaId {
    let c = CuentaId::nuevo();
    sqlx::query(
        "INSERT INTO cuentas (id, nombre_mostrar, usuario, hash_contrasena, contrasena_actualizada_en, creada_en)
         VALUES ($1, 'Ana', $2, 'x', now(), now())",
    )
    .bind(c.as_uuid())
    .bind(usuario)
    .execute(pool)
    .await
    .expect("cuenta");
    c
}

async fn entorno(pool: PgPool) -> Entorno {
    let tenant = crear_copropiedad(&pool).await;
    sqlx::query(
        "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
         VALUES (gen_random_uuid(), $1, 'control_acceso', now() - interval '1 day')",
    )
    .bind(tenant.as_uuid())
    .execute(&pool)
    .await
    .expect("módulo");
    sqlx::query("INSERT INTO lineas_whatsapp (id, phone_number_id, numero_visible, nombre) VALUES (gen_random_uuid(), 'PNID', '+57 300', 'Línea')")
        .execute(&pool)
        .await
        .expect("línea");
    sqlx::query("UPDATE copropiedades SET estado = 'activa' WHERE id = $1")
        .bind(tenant.as_uuid())
        .execute(&pool)
        .await
        .expect("activa");
    let cuenta = crear_cuenta(&pool, "ana").await;
    let vigilante = crear_cuenta(&pool, "porteria1").await;
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
                correo: None,
                cuenta_id: Some(cuenta),
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
        cuenta,
        vigilante,
    }
}

impl Entorno {
    fn porteria(&self) -> Contexto {
        Contexto::de_cuenta(
            self.tenant,
            self.vigilante,
            [Permiso::AccesoRegistrar].into_iter().collect(),
        )
    }

    fn duena(&self) -> Contexto {
        Contexto::de_cuenta(
            self.tenant,
            self.cuenta,
            [Permiso::AccesoVisitantesAutorizar].into_iter().collect(),
        )
    }

    fn visita(&self) -> NuevaVisita {
        NuevaVisita {
            bien_privado_id: self.bien,
            tipo: TipoVisita::Visita,
            visitante_nombre: "Carlos Visitante".into(),
            visitante_documento: Some("1020304050".into()),
            placa: Some("abc123".into()),
            observacion: None,
        }
    }

    async fn avisos(&self) -> Vec<(String, String)> {
        sqlx::query_as("SELECT canal, destino FROM outbox WHERE tenant_id = $1 AND concepto = 'control_acceso' ORDER BY id")
            .bind(self.tenant.as_uuid())
            .fetch_all(&self.duenio)
            .await
            .expect("outbox")
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_anuncio_avisa_por_whatsapp_marcado_para_el_costo_y_escala(pool: PgPool) {
    let e = entorno(pool).await;
    let t0 = Utc::now();

    let v = anunciar_visita(&e.unidad, &RelojFijo(t0), &e.porteria(), e.visita())
        .await
        .expect("anunciar");

    // La dueña no vive allí, pero sin residentes el aviso va a la representante de los titulares.
    assert_eq!(
        e.avisos().await,
        vec![("whatsapp".to_owned(), TELEFONO.to_owned())]
    );
    let directorio = DirectorioAccesoPg::new(e.app.clone());
    let r = escalar_avisos(
        &directorio,
        &e.unidad,
        &RelojFijo(t0 + TimeDelta::minutes(2)),
    )
    .await
    .expect("escalar");
    assert_eq!(r.segundos_avisos, 1);
    let r = escalar_avisos(
        &directorio,
        &e.unidad,
        &RelojFijo(t0 + TimeDelta::minutes(5)),
    )
    .await
    .expect("escalar");
    assert_eq!(r.llamadas, 1);
    let estado: String = sqlx::query_scalar("SELECT estado FROM visitas WHERE id = $1")
        .bind(v.id.as_uuid())
        .fetch_one(&e.duenio)
        .await
        .expect("estado");
    assert_eq!(estado, "requiere_llamada");
    let eventos: i64 =
        sqlx::query_scalar("SELECT count(*) FROM visita_eventos WHERE visita_id = $1")
            .bind(v.id.as_uuid())
            .fetch_one(&e.duenio)
            .await
            .expect("eventos");
    assert_eq!(
        eventos, 4,
        "anunciada, aviso 1, aviso 2, escalada a llamada"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn dos_respuestas_simultaneas_deciden_una_sola_vez(pool: PgPool) {
    let e = entorno(pool).await;
    // La dueña no residente decide porque el bien no tiene residentes adultos.
    let v = anunciar_visita(&e.unidad, &RelojSistema, &e.porteria(), e.visita())
        .await
        .expect("anunciar");
    let tareas: Vec<_> = [true, false]
        .into_iter()
        .map(|autorizar| {
            let (unidad, ctx) = (e.unidad.clone(), e.duena());
            tokio::spawn(async move {
                decidir_visita(
                    &unidad,
                    &RelojSistema,
                    &ctx,
                    v.id,
                    autorizar,
                    CanalRespuestaVisita::App,
                )
                .await
            })
        })
        .collect();
    let mut decididas = 0;
    for t in tareas {
        if let ResultadoDecision::Decidida(_) = t.await.expect("tarea").expect("decidir") {
            decididas += 1;
        }
    }
    assert_eq!(decididas, 1);
    let decisiones: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM visita_eventos WHERE visita_id = $1 AND tipo IN ('autorizada', 'rechazada')",
    )
    .bind(v.id.as_uuid())
    .fetch_one(&e.duenio)
    .await
    .expect("eventos");
    assert_eq!(decisiones, 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn otra_copropiedad_no_ve_las_visitas(pool: PgPool) {
    let e = entorno(pool).await;
    anunciar_visita(&e.unidad, &RelojSistema, &e.porteria(), e.visita())
        .await
        .expect("anunciar");
    let otra = crear_copropiedad(&e.duenio).await;

    let mut tx = e.unidad.iniciar(&Contexto::new(otra)).await.expect("tx");
    let p = tx
        .visitas(&FiltroVisitas::default(), SolicitudPagina::default())
        .await
        .expect("visitas");

    assert!(p.datos.is_empty());
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn historiales_append_only(pool: PgPool) {
    let e = entorno(pool).await;
    let v = anunciar_visita(&e.unidad, &RelojSistema, &e.porteria(), e.visita())
        .await
        .expect("anunciar");
    let mut tx = transaccion_tenant(&e.app, e.tenant).await.expect("tx");

    let r = sqlx::query("UPDATE visita_eventos SET nivel = 9 WHERE visita_id = $1")
        .bind(v.id.as_uuid())
        .execute(&mut *tx)
        .await;
    assert!(r.is_err(), "visita_eventos no admite UPDATE");
    drop(tx);

    let mut tx = transaccion_tenant(&e.app, e.tenant).await.expect("tx");
    sqlx::query(
        "INSERT INTO eventos_acceso (id, tenant_id, sentido, resultado, ocurrido_en) VALUES ($1, $2, 'entrada', 'manual', now())",
    )
    .bind(EventoAccesoId::nuevo().as_uuid())
    .bind(e.tenant.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("insertar");
    tx.commit().await.expect("confirmar");
    let mut tx = transaccion_tenant(&e.app, e.tenant).await.expect("tx");
    let r = sqlx::query("UPDATE eventos_acceso SET motivo = 'x'")
        .execute(&mut *tx)
        .await;
    assert!(r.is_err(), "eventos_acceso no admite UPDATE");
    drop(tx);
    let r = sqlx::query("DELETE FROM eventos_acceso WHERE tenant_id = $1")
        .bind(e.tenant.as_uuid())
        .execute(&e.duenio)
        .await;
    assert!(r.is_err(), "ni el dueño borra eventos recientes");
}

fn evento(puente: Option<PuenteId>, id_local: &str, ocurrido: DateTime<Utc>) -> EventoAcceso {
    EventoAcceso {
        id: EventoAccesoId::nuevo(),
        punto_acceso_id: None,
        dispositivo_id: None,
        puente_id: puente,
        id_local: Some(id_local.into()),
        credencial_id: None,
        visita_id: None,
        sentido: SentidoAcceso::Entrada,
        resultado: ResultadoAcceso::Permitido,
        motivo: None,
        placa: None,
        registrado_por: None,
        ocurrido_en: ocurrido,
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_puente_se_autentica_y_sus_reportes_no_se_duplican(pool: PgPool) {
    let e = entorno(pool).await;
    let admin = Contexto::new(e.tenant);
    let (puente, token) = registrar_puente(&e.unidad, &RelojSistema, &admin, "Mini PC")
        .await
        .expect("puente");
    let directorio = DirectorioAccesoPg::new(e.app.clone());

    let p = autenticar_puente(&directorio, &token, None)
        .await
        .expect("autenticar");
    assert_eq!((p.ctx.tenant_id, p.puente), (e.tenant, puente.id));
    assert!(matches!(
        autenticar_puente(&directorio, &"0".repeat(64), None).await,
        Err(AppError::NoAutenticado)
    ));

    let ahora = Utc::now();
    let mut tx = e.unidad.iniciar(&admin).await.expect("tx");
    assert!(
        tx.insertar_evento_acceso(&evento(Some(puente.id), "ev-1", ahora))
            .await
            .expect("evento")
    );
    assert!(
        !tx.insertar_evento_acceso(&evento(Some(puente.id), "ev-1", ahora))
            .await
            .expect("evento")
    );
    tx.confirmar().await.expect("confirmar");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn los_eventos_se_purgan_solo_despues_de_un_anio(pool: PgPool) {
    let e = entorno(pool).await;
    let ahora = Utc::now();
    for (dias, local) in [(800_i64, "viejo"), (10, "reciente")] {
        sqlx::query(
            "INSERT INTO eventos_acceso (id, tenant_id, sentido, resultado, ocurrido_en, id_local, puente_id)
             VALUES ($1, $2, 'entrada', 'manual', $3, NULL, NULL)",
        )
        .bind(EventoAccesoId::nuevo().as_uuid())
        .bind(e.tenant.as_uuid())
        .bind(ahora - TimeDelta::days(dias))
        .execute(&e.duenio)
        .await
        .expect(local);
    }

    let mut tx = e
        .unidad
        .iniciar(&Contexto::new(e.tenant))
        .await
        .expect("tx");
    let borrados = tx.purgar_eventos_acceso(100).await.expect("purgar");
    tx.confirmar().await.expect("confirmar");

    assert_eq!(borrados, 1, "el plazo nunca baja de 365 días");
    let quedan: i64 =
        sqlx::query_scalar("SELECT count(*) FROM eventos_acceso WHERE tenant_id = $1")
            .bind(e.tenant.as_uuid())
            .fetch_one(&e.duenio)
            .await
            .expect("conteo");
    assert_eq!(quedan, 1);
    DirectorioAccesoPg::new(e.app.clone())
        .preparar_particiones(6)
        .await
        .expect("particiones");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn vender_el_bien_retira_la_autorizacion_derivada_en_la_siguiente_sincronizacion(
    pool: PgPool,
) {
    let e = entorno(pool).await;
    let ctx = Contexto::de_cuenta(
        e.tenant,
        CuentaId::nuevo(),
        ConjuntoPermisos::todos(domain::Ambito::Copropiedad),
    );
    let c = emitir_credencial(
        &e.unidad,
        &RelojSistema,
        &ctx,
        NuevaCredencial {
            titular: TitularAcceso::Persona(e.persona),
            tipo: TipoCredencial::Pin,
            valor: Some("4321".into()),
            vigente_desde: None,
            vigente_hasta: None,
        },
    )
    .await
    .expect("emitir")
    .credencial;
    let ahora = Utc::now();
    let mut tx = e.unidad.iniciar(&ctx).await.expect("tx");
    let primero = tx.cambios_desde(0, 100, ahora).await.expect("cambios");
    let s = primero
        .credenciales
        .iter()
        .find(|x| x.id == c.id)
        .expect("credencial");
    assert!(
        s.activa && !s.ventanas.is_empty(),
        "propietaria: ventana derivada"
    );
    drop(tx);

    sqlx::query("UPDATE titularidades SET vigente_hasta = now() WHERE persona_id = $1")
        .bind(e.persona.as_uuid())
        .execute(&e.duenio)
        .await
        .expect("vender");

    let mut tx = e.unidad.iniciar(&ctx).await.expect("tx");
    let despues = tx
        .cambios_desde(primero.cursor, 100, Utc::now() + TimeDelta::seconds(1))
        .await
        .expect("cambios");
    let s = despues
        .credenciales
        .iter()
        .find(|x| x.id == c.id)
        .expect("vuelve a sincronizarse");
    assert!(s.ventanas.is_empty(), "sin titularidad no hay ventana");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_push_con_token_invalido_se_desvia_a_whatsapp(pool: PgPool) {
    let e = entorno(pool).await;
    sqlx::query(
        "INSERT INTO dispositivos_push (id, cuenta_id, plataforma, token, registrado_en)
         VALUES (gen_random_uuid(), $1, 'android', 'token-de-prueba-0123456789', now())",
    )
    .bind(e.cuenta.as_uuid())
    .execute(&e.duenio)
    .await
    .expect("dispositivo");
    let mut tx = e
        .unidad
        .iniciar(&Contexto::new(e.tenant))
        .await
        .expect("tx");
    assert_eq!(tx.tokens_push(&[e.cuenta]).await.expect("tokens").len(), 1);
    let respaldo = MensajeSaliente {
        linea: "PNID".into(),
        telefono: TELEFONO.into(),
        contenido: ContenidoSaliente::Texto {
            cuerpo: "Visita".into(),
        },
    };
    tx.encolar_aviso_acceso(&EventoSaliente {
        prioridad: PrioridadSalida::Confirmacion,
        mensaje: MensajeOutbox::Push(NotificacionPush {
            token: "token-de-prueba-0123456789".into(),
            titulo: "Visita".into(),
            cuerpo: "Visita".into(),
            datos: Default::default(),
            respaldo: Some(respaldo.clone()),
        }),
    })
    .await
    .expect("encolar");
    tx.confirmar().await.expect("confirmar");
    // Otra copropiedad no ve los tokens de una cuenta que no es persona suya.
    let otra = crear_copropiedad(&e.duenio).await;
    let mut tx = e.unidad.iniciar(&Contexto::new(otra)).await.expect("tx");
    assert!(
        tx.tokens_push(&[e.cuenta])
            .await
            .expect("tokens")
            .is_empty()
    );
    drop(tx);

    let cola = ColaSalidaPg::new(e.app.clone());
    let mut lote = cola.reclamar(&[Canal::Push], 10).await.expect("reclamar");
    let m = lote.tomar_mensajes().pop().expect("push pendiente");
    lote.desviar_push(m.id, Some(respaldo))
        .await
        .expect("desviar");
    lote.confirmar().await.expect("confirmar");

    assert_eq!(
        e.avisos().await,
        vec![
            ("push".to_owned(), "token-de-prueba-0123456789".to_owned()),
            ("whatsapp".to_owned(), TELEFONO.to_owned())
        ]
    );
    let (estado, activo): (String, bool) = sqlx::query_as(
        "SELECT o.estado, d.activo FROM outbox o JOIN dispositivos_push d ON d.token = o.destino WHERE o.canal = 'push'",
    )
    .fetch_one(&e.duenio)
    .await
    .expect("estado");
    assert_eq!((estado.as_str(), activo), ("fallido", false));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_entrega_exige_soporte_tambien_en_la_base(pool: PgPool) {
    let e = entorno(pool).await;
    let c = domain::CorrespondenciaId::nuevo();
    sqlx::query(
        "INSERT INTO correspondencia (id, tenant_id, bien_privado_id, tipo, estado, recibida_en)
         VALUES ($1, $2, $3, 'paquete', 'recibida', now())",
    )
    .bind(c.as_uuid())
    .bind(e.tenant.as_uuid())
    .bind(e.bien.as_uuid())
    .execute(&e.duenio)
    .await
    .expect("correspondencia");

    let r = sqlx::query(
        "INSERT INTO entregas_correspondencia (tenant_id, correspondencia_id, persona_id, entregada_en, soportes_hasta)
         VALUES ($1, $2, $3, now(), now())",
    )
    .bind(e.tenant.as_uuid())
    .bind(c.as_uuid())
    .bind(e.persona.as_uuid())
    .execute(&e.duenio)
    .await;

    assert!(r.is_err(), "sin firma ni foto");
}
