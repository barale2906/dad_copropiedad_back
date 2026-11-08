//! Pruebas de los casos de uso de tickets con dobles en memoria.

use chrono::{DateTime, Duration, TimeZone, Utc};
use domain::{
    AsignacionRolId, BienPrivadoId, Calificacion, ConjuntoPermisos, CuentaId, DireccionCorreo,
    Documento, OrigenAsignacion, Permiso, PersonaId, Porcentaje, RolId, Telefono, TenantId,
    TipoPersona, TipoTitularidad, TitularidadId, permisos::ROLES_PLANTILLA,
};
use rust_decimal::dec;

use super::*;
use crate::{
    outbox::MensajeOutbox,
    puertos::{
        copropiedades::{Persona, Titularidad},
        identidad::Asignacion,
        mensajeria::ContenidoSaliente,
        reloj::RelojFijo,
        tickets::Destinatario,
        unidad_trabajo::memoria::UnidadEnMemoria,
    },
};

/// Miércoles 4 de noviembre de 2026, 10:00 en Bogotá.
fn ahora() -> DateTime<Utc> {
    domain::reserva::Tz::America__Bogota
        .with_ymd_and_hms(2026, 11, 4, 10, 0, 0)
        .single()
        .expect("fecha")
        .with_timezone(&Utc)
}

struct Entorno {
    unidad: UnidadEnMemoria,
    tenant: TenantId,
    bien: BienPrivadoId,
    /// Propietario del bien, con cuenta, correo y teléfono.
    dueno: PersonaId,
    cuenta_dueno: CuentaId,
}

fn rol(codigo: &str) -> RolId {
    let i = ROLES_PLANTILLA
        .iter()
        .position(|r| r.codigo == codigo)
        .expect("plantilla");
    RolId::from_uuid(uuid::Uuid::from_u128(i as u128 + 1))
}

fn entorno() -> Entorno {
    let unidad = UnidadEnMemoria::default();
    unidad.habilitar(Modulo::GestionAdministrativa);
    let tenant = TenantId::nuevo();
    let bien = BienPrivadoId::nuevo();
    let dueno = PersonaId::nuevo();
    let cuenta_dueno = CuentaId::nuevo();
    unidad.con_datos(|d| {
        let mut p = Persona::basica(
            dueno,
            TipoPersona::Natural,
            "Ana Dueña".into(),
            Some(Documento::new("52123456").expect("válido")),
        );
        p.cuenta_id = Some(cuenta_dueno);
        p.correo = Some(DireccionCorreo::new("ana@correo.co").expect("válido"));
        p.telefono = Some(Telefono::new("573001112233").expect("válido"));
        d.personas.push(p);
        d.titularidades.push(Titularidad {
            id: TitularidadId::nuevo(),
            bien_privado_id: bien,
            persona_id: dueno,
            tipo: TipoTitularidad::Propietario,
            porcentaje: Porcentaje::new(dec!(100)).expect("válido"),
            es_representante: true,
            vigente_desde: ahora() - Duration::days(400),
            vigente_hasta: None,
            soporte_archivo_id: None,
        });
        d.gestion.linea = Some("PNID".into());
    });
    Entorno {
        unidad,
        tenant,
        bien,
        dueno,
        cuenta_dueno,
    }
}

impl Entorno {
    fn ctx(&self, cuenta: CuentaId, permisos: &[Permiso]) -> Contexto {
        Contexto::de_cuenta(self.tenant, cuenta, permisos.iter().copied().collect())
    }

    fn residente(&self) -> Contexto {
        self.ctx(self.cuenta_dueno, &[Permiso::TicketsCrear])
    }

    fn admin(&self) -> Contexto {
        Contexto::de_cuenta(
            self.tenant,
            CuentaId::nuevo(),
            ConjuntoPermisos::todos(domain::Ambito::Copropiedad),
        )
    }

    /// Asistente: ve todos y gestiona, pero no ve la identidad en convivencia.
    fn asistente(&self) -> Contexto {
        self.ctx(
            CuentaId::nuevo(),
            &[
                Permiso::TicketsCrear,
                Permiso::TicketsAsignar,
                Permiso::TicketsVerTodos,
                Permiso::TicketsAtender,
            ],
        )
    }

    /// Cuenta con el rol plantilla `mantenimiento` (puede atender tickets).
    fn tecnico(&self) -> (CuentaId, Contexto) {
        let c = CuentaId::nuevo();
        let tenant = self.tenant;
        self.unidad.con_datos(|d| {
            d.asignaciones.push((
                tenant,
                Asignacion {
                    id: AsignacionRolId::nuevo(),
                    cuenta_id: c,
                    rol_id: rol("mantenimiento"),
                    vigente_desde: ahora() - Duration::days(1),
                    vigente_hasta: None,
                    origen: OrigenAsignacion::Manual,
                    origen_id: None,
                    asignado_por: None,
                },
            ));
        });
        (
            c,
            self.ctx(
                c,
                &[Permiso::TicketsAtender, Permiso::MantenimientoEjecutar],
            ),
        )
    }

    async fn radicar(&self, ctx: &Contexto, tipo: TipoTicket, clave: &str) -> AppResult<Ticket> {
        radicar_ticket(
            &self.unidad,
            &RelojFijo(ahora()),
            ctx,
            NuevoTicket {
                tipo,
                categoria_id: None,
                prioridad: None,
                canal: CanalTicket::Web,
                asunto: "Fuga en el baño".into(),
                descripcion: "Gotea el techo del baño principal".into(),
                bien_privado_id: Some(self.bien),
                ubicacion: Some("Torre 1, apto 101".into()),
                recurso_id: None,
                activo_id: None,
                adjuntos: Vec::new(),
                idempotency_key: clave.into(),
                solicitante: None,
            },
        )
        .await
    }
}

async fn transicion(
    e: &Entorno,
    ctx: &Contexto,
    id: TicketId,
    hacia: EstadoTicket,
    motivo: Option<&str>,
    en: DateTime<Utc>,
) -> AppResult<TicketVista> {
    transicionar_ticket(
        &e.unidad,
        &RelojFijo(en),
        ctx,
        id,
        CambioEstadoTicket {
            hacia,
            motivo: motivo.map(Into::into),
            duplicado_de: None,
        },
    )
    .await
}

#[tokio::test]
async fn radicar_es_idempotente_por_clave() {
    let e = entorno();
    let a = e
        .radicar(&e.residente(), TipoTicket::Peticion, "clave-1")
        .await
        .expect("radica");
    let b = e
        .radicar(&e.residente(), TipoTicket::Peticion, "clave-1")
        .await
        .expect("reintento");

    assert_eq!(a.id, b.id);
    let datos = e.unidad.datos();
    assert_eq!(datos.gestion.tickets.len(), 1);
    assert_eq!(datos.gestion.eventos.len(), 1);
    assert_eq!(a.numero.to_string(), "PQR-2026-00001");
    assert_eq!(a.solicitante_persona_id, Some(e.dueno));
}

#[tokio::test]
async fn el_consecutivo_avanza_por_copropiedad_y_anio() {
    let e = entorno();
    let a = e
        .radicar(&e.residente(), TipoTicket::Queja, "a")
        .await
        .expect("a");
    let b = e
        .radicar(&e.residente(), TipoTicket::Reclamo, "b")
        .await
        .expect("b");
    assert_eq!((a.numero.consecutivo, b.numero.consecutivo), (1, 2));
}

#[tokio::test]
async fn peticion_vence_en_quince_dias_habiles_y_felicitacion_no_vence() {
    let e = entorno();
    let p = e
        .radicar(&e.residente(), TipoTicket::Peticion, "p")
        .await
        .expect("p");
    // Desde el miércoles 4: 15 días hábiles (sin contar el festivo del 16) → 26 de noviembre, 17:00.
    let esperado = domain::reserva::Tz::America__Bogota
        .with_ymd_and_hms(2026, 11, 26, 17, 0, 0)
        .single()
        .expect("fecha")
        .with_timezone(&Utc);
    assert_eq!(p.vence_en, Some(esperado));
    let f = e
        .radicar(&e.residente(), TipoTicket::Felicitacion, "f")
        .await
        .expect("f");
    assert_eq!((f.plazo, f.vence_en), (None, None));
}

#[tokio::test]
async fn sin_vinculo_con_el_bien_no_radica() {
    let e = entorno();
    let extrano = e.ctx(CuentaId::nuevo(), &[Permiso::TicketsCrear]);
    let r = e.radicar(&extrano, TipoTicket::Peticion, "x").await;
    assert!(matches!(r, Err(AppError::Validacion(_))), "{r:?}");
}

#[tokio::test]
async fn sin_el_modulo_no_hay_tickets() {
    let e = entorno();
    e.unidad.con_datos(|d| d.modulos.clear());
    let r = e.radicar(&e.residente(), TipoTicket::Peticion, "x").await;
    assert!(matches!(r, Err(AppError::ModuloNoHabilitado(_))), "{r:?}");
}

#[tokio::test]
async fn urgente_avisa_de_inmediato_a_quien_recibe_urgencias() {
    let e = entorno();
    let c = CuentaId::nuevo();
    e.unidad.con_datos(|d| {
        d.gestion.destinatarios.push((
            Permiso::TicketsUrgenciasRecibir,
            Destinatario {
                cuenta_id: c,
                correo: Some(DireccionCorreo::new("turno@ph.co").expect("válido")),
                telefono: Some(Telefono::new("573009998877").expect("válido")),
            },
        ));
    });
    radicar_ticket(
        &e.unidad,
        &RelojFijo(ahora()),
        &e.residente(),
        NuevoTicket {
            tipo: TipoTicket::SolicitudMantenimiento,
            categoria_id: None,
            prioridad: Some(PrioridadTicket::Urgente),
            canal: CanalTicket::Web,
            asunto: "Ascensor detenido".into(),
            descripcion: "Hay personas adentro".into(),
            bien_privado_id: Some(e.bien),
            ubicacion: Some("Torre 2".into()),
            recurso_id: None,
            activo_id: None,
            adjuntos: Vec::new(),
            idempotency_key: "u".into(),
            solicitante: None,
        },
    )
    .await
    .expect("radica");

    let outbox = e.unidad.outbox();
    let plantilla = outbox.iter().find_map(|(_, ev)| match &ev.mensaje {
        MensajeOutbox::WhatsApp(m) if m.telefono == "573009998877" => Some(m.contenido.clone()),
        _ => None,
    });
    assert!(
        matches!(&plantilla, Some(ContenidoSaliente::Plantilla { nombre, .. }) if nombre == "reporte_urgente"),
        "{plantilla:?}"
    );
    assert!(outbox.iter().any(|(_, ev)| ev.prioridad == PrioridadSalida::Confirmacion
        && matches!(&ev.mensaje, MensajeOutbox::Correo(c) if c.para.as_str() == "turno@ph.co")));
}

#[tokio::test]
async fn el_solicitante_no_ve_las_notas_internas() {
    let e = entorno();
    let t = e
        .radicar(&e.residente(), TipoTicket::Peticion, "n")
        .await
        .expect("t");
    let reloj = RelojFijo(ahora());
    for (interno, texto) in [(true, "nota interna"), (false, "respuesta pública")] {
        comentar_ticket(
            &e.unidad,
            &reloj,
            &e.asistente(),
            t.id,
            NuevoMensaje {
                texto: texto.into(),
                interno,
                adjuntos: Vec::new(),
                canal: CanalTicket::Web,
            },
        )
        .await
        .expect("comenta");
    }

    let suyo = ver_ticket(&e.unidad, &reloj, &e.residente(), t.id)
        .await
        .expect("ve");
    let textos: Vec<_> = suyo
        .mensajes
        .iter()
        .map(|m| m.mensaje.texto.as_str())
        .collect();
    assert_eq!(textos, vec!["respuesta pública"]);
    let staff = ver_ticket(&e.unidad, &reloj, &e.asistente(), t.id)
        .await
        .expect("ve");
    assert_eq!(staff.mensajes.len(), 2);

    // El solicitante no puede escribir notas internas.
    let r = comentar_ticket(
        &e.unidad,
        &reloj,
        &e.residente(),
        t.id,
        NuevoMensaje {
            texto: "x".into(),
            interno: true,
            adjuntos: Vec::new(),
            canal: CanalTicket::Web,
        },
    )
    .await;
    assert!(matches!(r, Err(AppError::SinPermiso(_))));
}

#[tokio::test]
async fn un_ticket_ajeno_no_existe_para_otro_residente() {
    let e = entorno();
    let t = e
        .radicar(&e.residente(), TipoTicket::Queja, "q")
        .await
        .expect("t");
    let otro = e.ctx(CuentaId::nuevo(), &[Permiso::TicketsCrear]);
    let r = ver_ticket(&e.unidad, &RelojFijo(ahora()), &otro, t.id).await;
    assert!(matches!(r, Err(AppError::NoEncontrado(_))));
    let lista = listar_tickets(
        &e.unidad,
        &RelojFijo(ahora()),
        &otro,
        ConsultaTickets::default(),
        SolicitudPagina::primera(),
    )
    .await
    .expect("lista");
    assert!(lista.datos.is_empty());
}

#[tokio::test]
async fn convivencia_oculta_al_denunciante_sin_el_permiso_del_comite() {
    let e = entorno();
    let t = e
        .radicar(&e.residente(), TipoTicket::ReporteConvivencia, "c")
        .await
        .expect("t");
    assert!(t.confidencial);
    let reloj = RelojFijo(ahora());

    let asistente = ver_ticket(&e.unidad, &reloj, &e.asistente(), t.id)
        .await
        .expect("ve");
    assert!(asistente.ticket.identidad_oculta);
    assert_eq!(
        (
            asistente.ticket.ticket.solicitante_persona_id,
            asistente.ticket.ticket.solicitante_cuenta_id,
            asistente.ticket.ticket.bien_privado_id
        ),
        (None, None, None)
    );

    let comite = e.ctx(
        CuentaId::nuevo(),
        &[Permiso::TicketsConvivenciaVer, Permiso::TicketsAtender],
    );
    let visto = ver_ticket(&e.unidad, &reloj, &comite, t.id)
        .await
        .expect("ve");
    assert!(!visto.ticket.identidad_oculta);
    assert_eq!(visto.ticket.ticket.solicitante_persona_id, Some(e.dueno));

    // Tampoco se filtra por bien privado para deducir al denunciante.
    let por_bien = listar_tickets(
        &e.unidad,
        &reloj,
        &e.asistente(),
        ConsultaTickets {
            bien_privado_id: Some(e.bien),
            ..ConsultaTickets::default()
        },
        SolicitudPagina::primera(),
    )
    .await
    .expect("lista");
    assert!(por_bien.datos.is_empty());

    // Y la respuesta de una gestión tampoco lo revela.
    let asignado = transicion(
        &e,
        &e.asistente(),
        t.id,
        EstadoTicket::EnRevision,
        None,
        ahora(),
    )
    .await
    .expect("revisa");
    assert!(asignado.identidad_oculta && asignado.ticket.solicitante_cuenta_id.is_none());
}

#[tokio::test]
async fn el_residente_no_puede_rechazar_ni_resolver() {
    let e = entorno();
    let t = e
        .radicar(&e.residente(), TipoTicket::Queja, "r")
        .await
        .expect("t");
    for hacia in [EstadoTicket::Rechazado, EstadoTicket::EnRevision] {
        let r = transicion(&e, &e.residente(), t.id, hacia, Some("no"), ahora()).await;
        assert!(matches!(r, Err(AppError::SinPermiso(_))), "{hacia}: {r:?}");
    }
}

#[tokio::test]
async fn asignar_exige_quien_pueda_atender() {
    let e = entorno();
    let t = e
        .radicar(&e.residente(), TipoTicket::Peticion, "a")
        .await
        .expect("t");
    let r = asignar_ticket(
        &e.unidad,
        &RelojFijo(ahora()),
        &e.asistente(),
        t.id,
        Responsable::Cuenta(CuentaId::nuevo()),
    )
    .await;
    assert!(matches!(r, Err(AppError::Validacion(_))), "{r:?}");

    let (tecnico, _) = e.tecnico();
    let v = asignar_ticket(
        &e.unidad,
        &RelojFijo(ahora()),
        &e.asistente(),
        t.id,
        Responsable::Cuenta(tecnico),
    )
    .await
    .expect("asigna");
    assert_eq!(v.ticket.estado, EstadoTicket::Asignado);
}

#[tokio::test]
async fn ciclo_completo_con_espera_resolucion_reapertura_y_encuesta() {
    let e = entorno();
    let t = e
        .radicar(&e.residente(), TipoTicket::SolicitudMantenimiento, "c")
        .await
        .expect("t");
    let (tecnico, ctx_tecnico) = e.tecnico();
    asignar_ticket(
        &e.unidad,
        &RelojFijo(ahora()),
        &e.asistente(),
        t.id,
        Responsable::Cuenta(tecnico),
    )
    .await
    .expect("asigna");
    transicion(
        &e,
        &ctx_tecnico,
        t.id,
        EstadoTicket::EnProgreso,
        None,
        ahora(),
    )
    .await
    .expect("progreso");
    // Pedir información exige motivo.
    let sin_motivo = transicion(
        &e,
        &ctx_tecnico,
        t.id,
        EstadoTicket::EnEsperaSolicitante,
        None,
        ahora(),
    )
    .await;
    assert!(matches!(sin_motivo, Err(AppError::Validacion(_))));
    transicion(
        &e,
        &ctx_tecnico,
        t.id,
        EstadoTicket::EnEsperaSolicitante,
        Some("Envíe una foto"),
        ahora(),
    )
    .await
    .expect("espera");

    // El solicitante responde dos días después: vuelve a en_progreso y el plazo se cuenta otra vez.
    let despues = ahora() + Duration::days(2);
    comentar_ticket(
        &e.unidad,
        &RelojFijo(despues),
        &e.residente(),
        t.id,
        NuevoMensaje {
            texto: "Aquí está".into(),
            interno: false,
            adjuntos: Vec::new(),
            canal: CanalTicket::Web,
        },
    )
    .await
    .expect("responde");
    let tras_respuesta = e.unidad.datos().gestion.tickets[0].clone();
    assert_eq!(tras_respuesta.estado, EstadoTicket::EnProgreso);
    assert!(tras_respuesta.vence_en > t.vence_en);

    let resuelto = transicion(
        &e,
        &ctx_tecnico,
        t.id,
        EstadoTicket::Resuelto,
        Some("Se cambió el empaque"),
        despues,
    )
    .await
    .expect("resuelve")
    .ticket;
    assert!(resuelto.reabrible_hasta.is_some());
    assert!(
        e.unidad
            .datos()
            .gestion
            .mensajes
            .iter()
            .any(|m| m.texto == "Se cambió el empaque" && !m.interno)
    );
    assert!(e.unidad.outbox().iter().any(
        |(_, ev)| matches!(&ev.mensaje, MensajeOutbox::Correo(c) if c.para.as_str() == "ana@correo.co" && c.asunto.contains("resuelto"))
    ));

    // Reabre dentro del plazo; luego vuelve a resolverse.
    transicion(
        &e,
        &e.residente(),
        t.id,
        EstadoTicket::Reabierto,
        None,
        despues,
    )
    .await
    .expect("reabre");
    transicion(
        &e,
        &ctx_tecnico,
        t.id,
        EstadoTicket::Resuelto,
        Some("Listo"),
        despues,
    )
    .await
    .expect("resuelve otra vez");

    // Fuera del plazo de reapertura ya no puede.
    let tarde = despues + Duration::days(30);
    let r = transicion(
        &e,
        &e.residente(),
        t.id,
        EstadoTicket::Reabierto,
        None,
        tarde,
    )
    .await;
    assert!(matches!(r, Err(AppError::Conflicto(_))), "{r:?}");

    // Encuesta: solo el solicitante, una vez.
    let cinco = Calificacion::new(5).expect("válida");
    let otro = calificar_ticket(
        &e.unidad,
        &RelojFijo(tarde),
        &ctx_tecnico,
        t.id,
        cinco,
        None,
    )
    .await;
    assert!(matches!(otro, Err(AppError::SinPermiso(_))));
    calificar_ticket(
        &e.unidad,
        &RelojFijo(tarde),
        &e.residente(),
        t.id,
        cinco,
        None,
    )
    .await
    .expect("califica");
    let dos = calificar_ticket(
        &e.unidad,
        &RelojFijo(tarde),
        &e.residente(),
        t.id,
        cinco,
        None,
    )
    .await;
    assert!(matches!(dos, Err(AppError::Conflicto(_))));
}

#[tokio::test]
async fn las_tareas_cierran_resueltos_vencidos_y_alertan_una_vez() {
    let e = entorno();
    let sistema = Contexto::new(e.tenant);
    let resuelto = e
        .radicar(&e.residente(), TipoTicket::Peticion, "r")
        .await
        .expect("r");
    transicion(
        &e,
        &e.asistente(),
        resuelto.id,
        EstadoTicket::EnRevision,
        None,
        ahora(),
    )
    .await
    .expect("revisa");
    transicion(
        &e,
        &e.asistente(),
        resuelto.id,
        EstadoTicket::Resuelto,
        Some("Hecho"),
        ahora(),
    )
    .await
    .expect("resuelve");
    let vencido = e
        .radicar(&e.residente(), TipoTicket::Queja, "v")
        .await
        .expect("v");

    let mes = ahora() + Duration::days(30);
    let r = tareas_tickets(&e.unidad, &RelojFijo(mes), &sistema)
        .await
        .expect("tareas");
    assert_eq!(
        r,
        ResumenTareasTickets {
            cerrados: 1,
            alertados: 1
        }
    );
    let datos = e.unidad.datos();
    let t = |id| {
        datos
            .gestion
            .tickets
            .iter()
            .find(|t| t.id == id)
            .cloned()
            .expect("t")
    };
    assert_eq!(t(resuelto.id).estado, EstadoTicket::Cerrado);
    assert!(t(vencido.id).alerta_vencimiento_en.is_some());

    let otra = tareas_tickets(&e.unidad, &RelojFijo(mes), &sistema)
        .await
        .expect("tareas");
    assert_eq!(otra, ResumenTareasTickets::default());
}

#[tokio::test]
async fn reclasificar_recalcula_el_vencimiento_desde_la_radicacion() {
    let e = entorno();
    let t = e
        .radicar(&e.residente(), TipoTicket::SolicitudMantenimiento, "m")
        .await
        .expect("t");
    let urgente = clasificar_ticket(
        &e.unidad,
        &RelojFijo(ahora()),
        &e.asistente(),
        t.id,
        Clasificacion {
            prioridad: Some(PrioridadTicket::Urgente),
            ..Clasificacion::default()
        },
    )
    .await
    .expect("clasifica")
    .ticket;
    // 4 horas hábiles desde las 10:00 del miércoles → 14:00.
    assert_eq!(urgente.vence_en, Some(ahora() + Duration::hours(4)));
    let r = clasificar_ticket(
        &e.unidad,
        &RelojFijo(ahora()),
        &e.asistente(),
        t.id,
        Clasificacion {
            tipo: Some(TipoTicket::ReporteConvivencia),
            ..Clasificacion::default()
        },
    )
    .await;
    assert!(matches!(r, Err(AppError::Validacion(_))));
}

#[tokio::test]
async fn configuracion_valida_jornada_y_reapertura() {
    let e = entorno();
    let mut c = ConfiguracionTickets::predeterminada();
    c.dias_reapertura = 0;
    let r = guardar_configuracion(&e.unidad, &RelojFijo(ahora()), &e.admin(), c).await;
    assert!(matches!(r, Err(AppError::Validacion(_))));
    let r = guardar_configuracion(
        &e.unidad,
        &RelojFijo(ahora()),
        &e.residente(),
        ConfiguracionTickets::predeterminada(),
    )
    .await;
    assert!(matches!(r, Err(AppError::SinPermiso(_))));
}

#[tokio::test]
async fn categorias_inactivas_no_se_usan_al_radicar() {
    let e = entorno();
    let reloj = RelojFijo(ahora());
    let c = guardar_categoria(
        &e.unidad,
        &reloj,
        &e.admin(),
        None,
        "Plomería".into(),
        false,
    )
    .await
    .expect("crea");
    let repetida =
        guardar_categoria(&e.unidad, &reloj, &e.admin(), None, "plomería".into(), true).await;
    assert!(matches!(repetida, Err(AppError::Conflicto(_))));
    let visibles = listar_categorias(&e.unidad, &reloj, &e.residente())
        .await
        .expect("lista");
    assert!(visibles.is_empty());
    let r = radicar_ticket(
        &e.unidad,
        &reloj,
        &e.residente(),
        NuevoTicket {
            tipo: TipoTicket::SolicitudMantenimiento,
            categoria_id: Some(c.id),
            prioridad: None,
            canal: CanalTicket::Web,
            asunto: "x".into(),
            descripcion: "y".into(),
            bien_privado_id: None,
            ubicacion: None,
            recurso_id: None,
            activo_id: None,
            adjuntos: Vec::new(),
            idempotency_key: "k".into(),
            solicitante: None,
        },
    )
    .await;
    assert!(matches!(r, Err(AppError::NoEncontrado(_))));
}
