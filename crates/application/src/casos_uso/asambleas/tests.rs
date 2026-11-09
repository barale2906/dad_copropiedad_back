//! Pruebas de los casos de uso de asambleas con dobles en memoria.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, TimeZone, Timelike, Utc};
use domain::{
    BienPrivadoId, Canal, CargoEleccion, ClaveIdempotencia, CodigoBienPrivado, CodigoRotulo,
    Coeficiente, ConjuntoPermisos, Convocatoria, CuentaId, Decision, EfectoMora, EstadoReunion,
    MetodoRegistro, Modalidad, Modulo, MotivoSalida, OpcionId, Permiso, PersonaId, Porcentaje,
    PreguntaId, ReglaMayoria, ReunionId, Telefono, TenantId, TipoBienPrivado, TipoPersona,
    TipoPregunta, TipoReunion, TipoTitularidad, TipoVoto, TitularidadId, UsoBienPrivado,
};
use rust_decimal::{Decimal, dec};

use super::{
    registro::{
        PersonaAsistente, PoderRecibido, RegistroAsistente, ResultadoRegistro, registrar_asistente,
        registrar_reingreso, registrar_salida, votar_con_papeleta,
    },
    reuniones::{
        DatosPunto, EnvioConvocatoriaOpciones, NuevaReunion, abrir_registro, convocar,
        convocar_segunda, crear_reunion, guardar_punto, instalar,
    },
    votaciones::{
        DefinicionPregunta, NuevaOpcion, SolicitudVoto, Votante, abrir_pregunta, aplicar_eleccion,
        cerrar_pregunta, cerrar_vencidas, crear_pregunta, detalle_votos, emitir_voto,
    },
    whatsapp::{
        ResultadoAsamblea, ServiciosAsamblea, atender_mensaje_asamblea, id_voto, payload_confirmar,
    },
};
use crate::{
    AppError, AppResult, Contexto,
    outbox::MensajeOutbox,
    puertos::{
        SesionesConversacion,
        asambleas::{
            AcuseEntrega, AsistenciaTelefono, ConfiguracionReunion, DirectorioAsambleas,
            PreguntaGuardada, Reunion,
        },
        copropiedades::{BienPrivado, CargoVigente, Persona, Titularidad},
        mensajeria::{ContenidoEntrante, ContenidoSaliente, MensajeEntrante},
        reloj::RelojFijo,
        sesiones::{CierreTurno, SesionGuardada},
        unidad_trabajo::memoria::UnidadEnMemoria,
    },
};

/// Lunes 9 de noviembre de 2026, 08:00 en Bogotá.
fn ahora() -> DateTime<Utc> {
    domain::reserva::Tz::America__Bogota
        .with_ymd_and_hms(2026, 11, 9, 8, 0, 0)
        .single()
        .expect("fecha")
        .with_timezone(&Utc)
}

const COEFICIENTES: [Decimal; 5] = [dec!(0.30), dec!(0.25), dec!(0.20), dec!(0.15), dec!(0.10)];

struct Entorno {
    unidad: UnidadEnMemoria,
    ctx: Contexto,
    bienes: Vec<BienPrivadoId>,
    /// Representante de cada bien (con teléfono y cuenta).
    duenos: Vec<PersonaId>,
    cuentas: Vec<CuentaId>,
}

fn telefono(i: usize) -> String {
    format!("57300111000{i}")
}

fn entorno() -> Entorno {
    let unidad = UnidadEnMemoria::default();
    unidad.habilitar(Modulo::Asambleas);
    let mut bienes = Vec::new();
    let mut duenos = Vec::new();
    let mut cuentas = Vec::new();
    unidad.con_datos(|d| {
        for (i, coef) in COEFICIENTES.iter().enumerate() {
            let id = BienPrivadoId::nuevo();
            d.bienes.insert(
                id,
                BienPrivado {
                    id,
                    codigo: CodigoBienPrivado::new(&format!("T1-{}01", i + 1)).expect("código"),
                    tipo: TipoBienPrivado::Apartamento,
                    agrupacion_id: None,
                    numero: None,
                    piso: None,
                    area_privada_m2: None,
                    matricula_inmobiliaria: None,
                    cedula_catastral: None,
                    uso: UsoBienPrivado::Residencial,
                    activo: true,
                    bien_principal_id: None,
                    coeficiente: Some(Coeficiente::new(*coef).expect("coeficiente")),
                },
            );
            let persona = PersonaId::nuevo();
            let cuenta = CuentaId::nuevo();
            let mut p = Persona::basica(
                persona,
                TipoPersona::Natural,
                format!("Propietario {i}"),
                None,
            );
            p.telefono = Some(Telefono::new(&telefono(i)).expect("teléfono"));
            p.cuenta_id = Some(cuenta);
            d.personas.push(p);
            d.titularidades.push(Titularidad {
                id: TitularidadId::nuevo(),
                bien_privado_id: id,
                persona_id: persona,
                tipo: TipoTitularidad::Propietario,
                porcentaje: Porcentaje::new(dec!(100)).expect("porcentaje"),
                es_representante: true,
                vigente_desde: ahora() - Duration::days(400),
                vigente_hasta: None,
                soporte_archivo_id: None,
            });
            bienes.push(id);
            duenos.push(persona);
            cuentas.push(cuenta);
        }
        d.asambleas.linea = Some("PNID".into());
    });
    Entorno {
        unidad,
        ctx: Contexto::new(TenantId::nuevo()),
        bienes,
        duenos,
        cuentas,
    }
}

fn reloj() -> RelojFijo {
    RelojFijo(ahora())
}

fn nueva(modalidad: Modalidad, config: ConfiguracionReunion) -> NuevaReunion {
    NuevaReunion {
        titulo: "Asamblea ordinaria 2027".into(),
        tipo: TipoReunion::Ordinaria,
        convocatoria: Convocatoria::Primera,
        modalidad,
        fecha_programada: ahora() + Duration::days(20),
        lugar: Some("Salón comunal".into()),
        enlace_video: None,
        plazo_respuesta: None,
        configuracion: config,
    }
}

/// Reunión convocada y en registro.
async fn en_registro(e: &Entorno, config: ConfiguracionReunion) -> Reunion {
    en_registro_modalidad(e, Modalidad::Presencial, config).await
}

async fn en_registro_modalidad(
    e: &Entorno,
    modalidad: Modalidad,
    config: ConfiguracionReunion,
) -> Reunion {
    let r = crear_reunion(&e.unidad, &reloj(), &e.ctx, nueva(modalidad, config))
        .await
        .expect("crea");
    guardar_punto(
        &e.unidad,
        &reloj(),
        &e.ctx,
        r.id,
        None,
        DatosPunto {
            orden: 1,
            titulo: "Verificación del quórum".into(),
            descripcion: None,
        },
    )
    .await
    .expect("punto");
    convocar(
        &e.unidad,
        &reloj(),
        &e.ctx,
        r.id,
        EnvioConvocatoriaOpciones::default(),
    )
    .await
    .expect("convoca");
    abrir_registro(&e.unidad, &reloj(), &e.ctx, r.id)
        .await
        .expect("registro")
}

fn registro(
    r: &Reunion,
    persona: PersonaId,
    propios: Vec<BienPrivadoId>,
    poderes: Vec<BienPrivadoId>,
) -> RegistroAsistente {
    RegistroAsistente {
        reunion_id: r.id,
        persona: PersonaAsistente::Existente(persona),
        telefono: None,
        bienes_propios: propios,
        poderes: poderes
            .into_iter()
            .map(|b| PoderRecibido {
                bien_privado_id: b,
                soporte_archivo_id: None,
            })
            .collect(),
        rotulo: None,
        enviar_confirmacion: true,
        metodo: MetodoRegistro::Mesa,
    }
}

async fn registrar(
    e: &Entorno,
    r: &Reunion,
    i: usize,
    poderes: &[usize],
) -> AppResult<ResultadoRegistro> {
    registrar_asistente(
        &e.unidad,
        &reloj(),
        &e.ctx,
        registro(
            r,
            e.duenos[i],
            vec![e.bienes[i]],
            poderes.iter().map(|p| e.bienes[*p]).collect(),
        ),
    )
    .await
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
        con_abstencion: true,
        punto_id: None,
        temporizador_segundos: None,
    }
}

async fn pregunta_abierta(e: &Entorno, r: &Reunion, d: DefinicionPregunta) -> PreguntaGuardada {
    let p = crear_pregunta(&e.unidad, &reloj(), &e.ctx, r.id, d)
        .await
        .expect("pregunta");
    abrir_pregunta(&e.unidad, &reloj(), &e.ctx, r.id, p.pregunta.id)
        .await
        .expect("abre")
}

fn voto(
    r: &Reunion,
    p: &PreguntaGuardada,
    asistente: domain::AsistenteId,
    opcion: usize,
    clave: &str,
) -> SolicitudVoto {
    SolicitudVoto {
        reunion_id: r.id,
        pregunta_id: p.pregunta.id,
        votante: Votante::Asistente(asistente),
        opciones: vec![p.pregunta.opciones[opcion].id],
        canal: Canal::Web,
        idempotency_key: ClaveIdempotencia::new(clave).expect("clave"),
        wamid: None,
        confirmar: true,
    }
}

fn participaciones(e: &Entorno, p: PreguntaId) -> Vec<domain::ParticipacionNueva> {
    e.unidad
        .datos()
        .asambleas
        .participaciones
        .into_iter()
        .filter(|x| x.pregunta_id == p)
        .collect()
}

fn textos_outbox(e: &Entorno) -> Vec<String> {
    e.unidad
        .outbox()
        .into_iter()
        .filter_map(|(_, ev)| match ev.mensaje {
            MensajeOutbox::WhatsApp(m) => match m.contenido {
                ContenidoSaliente::Texto { cuerpo } => Some(cuerpo),
                _ => None,
            },
            MensajeOutbox::Correo(_) | MensajeOutbox::Push(_) => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Convocatoria e instalación
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn convocar_toma_el_padron_y_deja_constancia_de_cada_envio() {
    let e = entorno();
    let r = en_registro(&e, ConfiguracionReunion::default()).await;
    let d = e.unidad.datos().asambleas;
    assert_eq!(d.padron.iter().filter(|(x, _)| *x == r.id).count(), 5);
    assert_eq!(
        d.envios.len(),
        5,
        "una plantilla por representante con WhatsApp"
    );
    assert!(e.unidad.outbox().iter().any(|(_, ev)| matches!(
        &ev.mensaje,
        MensajeOutbox::WhatsApp(m) if matches!(&m.contenido, ContenidoSaliente::Plantilla { nombre, .. } if nombre == "convocatoria_asamblea")
    )));
}

#[tokio::test]
async fn convocar_exige_antelacion_de_quince_dias_y_orden_del_dia() {
    let e = entorno();
    let mut n = nueva(Modalidad::Presencial, ConfiguracionReunion::default());
    n.fecha_programada = ahora() + Duration::days(10);
    let r = crear_reunion(&e.unidad, &reloj(), &e.ctx, n)
        .await
        .expect("crea");
    guardar_punto(
        &e.unidad,
        &reloj(),
        &e.ctx,
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
    let err = convocar(
        &e.unidad,
        &reloj(),
        &e.ctx,
        r.id,
        EnvioConvocatoriaOpciones::default(),
    )
    .await;
    assert!(matches!(err, Err(AppError::Validacion(m)) if m.contains("15 días")));

    let r = crear_reunion(
        &e.unidad,
        &reloj(),
        &e.ctx,
        nueva(Modalidad::Presencial, ConfiguracionReunion::default()),
    )
    .await
    .expect("crea");
    let err = convocar(
        &e.unidad,
        &reloj(),
        &e.ctx,
        r.id,
        EnvioConvocatoriaOpciones::default(),
    )
    .await;
    assert!(matches!(err, Err(AppError::Validacion(m)) if m.contains("orden del día")));
}

#[tokio::test]
async fn no_instala_sin_quorum_y_convoca_la_segunda_al_tercer_dia_habil() {
    let e = entorno();
    let r = en_registro(&e, ConfiguracionReunion::default()).await;
    registrar(&e, &r, 1, &[]).await.expect("registra 0.25");
    let err = instalar(&e.unidad, &reloj(), &e.ctx, r.id).await;
    assert!(matches!(err, Err(AppError::Conflicto(m)) if m.contains("no hay quórum")));

    let segunda = convocar_segunda(
        &e.unidad,
        &reloj(),
        &e.ctx,
        r.id,
        EnvioConvocatoriaOpciones::default(),
    )
    .await
    .expect("segunda");
    assert_eq!(segunda.reunion.convocatoria, Convocatoria::Segunda);
    assert_eq!(segunda.reunion.estado, EstadoReunion::Convocada);
    let local = segunda
        .reunion
        .fecha_programada
        .with_timezone(&domain::reserva::Tz::America__Bogota);
    assert_eq!(local.hour(), 20, "a las 8:00 p. m. (art. 41)");
    let original = e
        .unidad
        .datos()
        .asambleas
        .reuniones
        .into_iter()
        .find(|x| x.id == r.id)
        .expect("primera");
    assert_eq!(original.estado, EstadoReunion::Cancelada);
}

#[tokio::test]
async fn instala_con_mas_de_la_mitad_y_toma_la_foto() {
    let e = entorno();
    let r = en_registro(&e, ConfiguracionReunion::default()).await;
    registrar(&e, &r, 0, &[1]).await.expect("0.30 + 0.25");
    let r = instalar(&e.unidad, &reloj(), &e.ctx, r.id)
        .await
        .expect("instala");
    assert_eq!(r.estado, EstadoReunion::Instalada);
    let fotos = e.unidad.datos().asambleas.fotos;
    assert_eq!(fotos.len(), 1);
    assert_eq!(fotos[0].1.quorum.coeficientes.representados, dec!(0.55));
    assert_eq!(fotos[0].1.quorum.unidades.representados, dec!(2));
}

// ---------------------------------------------------------------------------------------------
// Registro y poderes
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn un_bien_solo_esta_en_un_asistente_presente() {
    let e = entorno();
    let r = en_registro(&e, ConfiguracionReunion::default()).await;
    registrar(&e, &r, 0, &[])
        .await
        .expect("propietario de T1-101");
    // Otra persona intenta traer un poder de T1-101 mientras su dueño está presente.
    let err = registrar(&e, &r, 1, &[0]).await;
    assert!(matches!(err, Err(AppError::Conflicto(m)) if m.contains("T1-101")));
}

#[tokio::test]
async fn poder_respeta_el_maximo_y_los_cargos_prohibidos() {
    let e = entorno();
    let config = ConfiguracionReunion {
        max_bienes_apoderado: Some(1),
        ..ConfiguracionReunion::default()
    };
    let r = en_registro(&e, config).await;
    let err = registrar(&e, &r, 0, &[1, 2]).await;
    assert!(matches!(err, Err(AppError::Conflicto(m)) if m.contains("máximo 1")));

    e.unidad.con_datos(|d| {
        d.asambleas.cargos_vigentes.push(CargoVigente {
            rol: "administrador".into(),
            rol_nombre: "Administrador".into(),
            cuenta_id: e.cuentas[3],
            nombre: "Propietario 3".into(),
            organizacion: None,
            vigente_desde: ahora() - Duration::days(30),
        });
    });
    let err = registrar(&e, &r, 3, &[4]).await;
    assert!(matches!(err, Err(AppError::Conflicto(m)) if m.contains("no puede ser apoderado")));
}

#[tokio::test]
async fn soporte_del_poder_obligatorio_si_la_asamblea_lo_exige() {
    let e = entorno();
    let r = en_registro(
        &e,
        ConfiguracionReunion {
            exigir_soporte_poder: true,
            ..ConfiguracionReunion::default()
        },
    )
    .await;
    assert!(matches!(
        registrar(&e, &r, 0, &[1]).await,
        Err(AppError::Validacion(_))
    ));
    assert!(registrar(&e, &r, 0, &[]).await.is_ok());
}

#[tokio::test]
async fn registro_en_mesa_envia_la_plantilla_de_confirmacion_y_entrega_la_papeleta() {
    let e = entorno();
    let r = en_registro(&e, ConfiguracionReunion::default()).await;
    let res = registrar(&e, &r, 0, &[1]).await.expect("registra");
    assert!(res.token_papeleta.is_some());
    assert!(res.asistente.presente);
    assert_eq!(res.asistente.linea_whatsapp.as_deref(), Some("PNID"));
    assert!(
        res.asistente.whatsapp_confirmado_en.is_none(),
        "confirma el asistente"
    );
    let plantilla = e
        .unidad
        .outbox()
        .into_iter()
        .find_map(|(_, ev)| match ev.mensaje {
            MensajeOutbox::WhatsApp(m) => match m.contenido {
                ContenidoSaliente::Plantilla {
                    nombre, botones, ..
                } if nombre == "confirmar_asistencia" => Some(botones),
                _ => None,
            },
            MensajeOutbox::Correo(_) | MensajeOutbox::Push(_) => None,
        });
    assert_eq!(
        plantilla,
        Some(vec![payload_confirmar(e.ctx.tenant_id, res.asistente.id)])
    );
}

// ---------------------------------------------------------------------------------------------
// Votación
// ---------------------------------------------------------------------------------------------

async fn instalada_con(
    e: &Entorno,
    config: ConfiguracionReunion,
) -> (Reunion, domain::AsistenteId) {
    let r = en_registro(e, config).await;
    let a = registrar(e, &r, 0, &[1, 2])
        .await
        .expect("apoderado de 3 bienes")
        .asistente
        .id;
    let r = instalar(&e.unidad, &reloj(), &e.ctx, r.id)
        .await
        .expect("instala");
    (r, a)
}

#[tokio::test]
async fn un_voto_del_apoderado_registra_una_participacion_por_cada_bien() {
    let e = entorno();
    let (r, a) = instalada_con(&e, ConfiguracionReunion::default()).await;
    let p = pregunta_abierta(&e, &r, si_no()).await;
    let res = emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, a, 0, "k1"))
        .await
        .expect("vota");
    assert!(res.registrado);
    assert_eq!(res.bienes.len(), 3);
    let parts = participaciones(&e, p.pregunta.id);
    assert_eq!(parts.len(), 3);
    let suma: Decimal = parts.iter().map(|x| x.coeficiente.valor()).sum();
    assert_eq!(suma, dec!(0.75), "coeficientes copiados del padrón");
    assert_eq!(parts.iter().filter(|x| x.apoderado_id.is_some()).count(), 2);
    // La auditoría no guarda la opción (DetalleAuditoria lo rechazaría) y la confirmación
    // tampoco la menciona.
    assert!(textos_outbox(&e).iter().all(|t| !t.contains("«Sí»")));
}

#[tokio::test]
async fn reintento_con_la_misma_clave_devuelve_el_original_y_otra_clave_ya_voto() {
    let e = entorno();
    let (r, a) = instalada_con(&e, ConfiguracionReunion::default()).await;
    let p = pregunta_abierta(&e, &r, si_no()).await;
    emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, a, 0, "k1"))
        .await
        .expect("vota");
    let reintento = emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, a, 1, "k1"))
        .await
        .expect("reintento");
    assert!(!reintento.registrado);
    assert_eq!(participaciones(&e, p.pregunta.id).len(), 3);
    let err = emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, a, 1, "k2")).await;
    assert!(matches!(err, Err(AppError::BienPrivadoYaVoto)));
}

#[tokio::test]
async fn moroso_sin_voto_cuenta_para_el_quorum_pero_no_vota() {
    let e = entorno();
    let config = ConfiguracionReunion {
        efecto_mora: EfectoMora::SinVoto,
        ..ConfiguracionReunion::default()
    };
    let r = en_registro(&e, config).await;
    super::registro::fijar_morosos(&e.unidad, &reloj(), &e.ctx, r.id, &["t1-301".into()])
        .await
        .expect("morosos");
    let res = registrar(&e, &r, 0, &[1, 2]).await.expect("registra");
    assert!(res.avisos.iter().any(|a| a.contains("T1-301")));
    let r = instalar(&e.unidad, &reloj(), &e.ctx, r.id)
        .await
        .expect("instala");
    let q = super::quorum::quorum_actual(&e.unidad, &reloj(), &e.ctx, r.id)
        .await
        .expect("quórum");
    assert_eq!(
        q.coeficientes.representados,
        dec!(0.75),
        "el moroso cuenta para el quórum"
    );
    let p = pregunta_abierta(&e, &r, si_no()).await;
    let res = emitir_voto(
        &e.unidad,
        &reloj(),
        &e.ctx,
        voto(&r, &p, res.asistente.id, 0, "k"),
    )
    .await
    .expect("vota");
    assert_eq!(res.bienes.len(), 2, "con un moroso sin voto: 2 de 3");
    assert_eq!(res.omitidos, vec!["T1-301".to_owned()]);
}

#[tokio::test]
async fn moroso_sin_voto_ni_quorum_no_cuenta_para_el_quorum() {
    let e = entorno();
    let config = ConfiguracionReunion {
        efecto_mora: EfectoMora::SinVotoNiQuorum,
        ..ConfiguracionReunion::default()
    };
    let r = en_registro(&e, config).await;
    super::registro::fijar_morosos(&e.unidad, &reloj(), &e.ctx, r.id, &["T1-101".into()])
        .await
        .expect("morosos");
    registrar(&e, &r, 0, &[1])
        .await
        .expect("registra 0.30 moroso + 0.25");
    let err = instalar(&e.unidad, &reloj(), &e.ctx, r.id).await;
    assert!(
        matches!(err, Err(AppError::Conflicto(_))),
        "solo cuenta 0.25"
    );
}

#[tokio::test]
async fn retiro_saca_del_quorum_y_de_las_preguntas_siguientes_y_reingreso_habilita() {
    let e = entorno();
    let (r, a) = instalada_con(&e, ConfiguracionReunion::default()).await;
    let otro = registrar(&e, &r, 3, &[])
        .await
        .expect("registra")
        .asistente
        .id;
    registrar_salida(
        &e.unidad,
        &reloj(),
        &e.ctx,
        r.id,
        otro,
        MotivoSalida::SeRetiro,
        None,
    )
    .await
    .expect("sale");
    let q = super::quorum::quorum_actual(&e.unidad, &reloj(), &e.ctx, r.id)
        .await
        .expect("quórum");
    assert_eq!(q.coeficientes.representados, dec!(0.75));
    let p = pregunta_abierta(&e, &r, si_no()).await;
    let err = emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, otro, 0, "x")).await;
    assert!(
        matches!(err, Err(AppError::NoInscrito)),
        "su rótulo o enlace ya no vota"
    );

    registrar_reingreso(&e.unidad, &reloj(), &e.ctx, r.id, otro)
        .await
        .expect("reingresa");
    let res = emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, otro, 1, "y"))
        .await
        .expect("vota");
    assert_eq!(res.bienes, vec!["T1-401".to_owned()]);
    emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, a, 0, "z"))
        .await
        .expect("vota");
}

#[tokio::test]
async fn pregunta_de_otra_reunion_no_existe_para_esta() {
    let e = entorno();
    let (r, a) = instalada_con(&e, ConfiguracionReunion::default()).await;
    let p = pregunta_abierta(&e, &r, si_no()).await;
    let mut s = voto(&r, &p, a, 0, "k");
    s.reunion_id = ReunionId::nuevo();
    assert!(matches!(
        emitir_voto(&e.unidad, &reloj(), &e.ctx, s).await,
        Err(AppError::NoEncontrado(_))
    ));
}

#[tokio::test]
async fn pregunta_cerrada_rechaza_el_voto_tardio_y_guarda_el_escrutinio() {
    let e = entorno();
    let (r, a) = instalada_con(&e, ConfiguracionReunion::default()).await;
    let p = pregunta_abierta(&e, &r, si_no()).await;
    emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, a, 0, "k"))
        .await
        .expect("vota");
    let cerrada = cerrar_pregunta(&e.unidad, &reloj(), &e.ctx, r.id, p.pregunta.id)
        .await
        .expect("cierra");
    let escrutinio = cerrada.resultado.expect("escrutinio");
    assert_eq!(escrutinio.decision, Decision::Aprobada);
    assert_eq!(escrutinio.habilitados.coeficiente, dec!(0.75));
    let err = emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, a, 1, "tarde")).await;
    assert!(matches!(
        err,
        Err(AppError::Dominio(domain::DominioError::PreguntaNoAbierta(
            _
        )))
    ));
}

#[tokio::test]
async fn temporizador_vencido_cierra_la_pregunta() {
    let e = entorno();
    let (r, _) = instalada_con(&e, ConfiguracionReunion::default()).await;
    let p = pregunta_abierta(
        &e,
        &r,
        DefinicionPregunta {
            temporizador_segundos: Some(60),
            ..si_no()
        },
    )
    .await;
    assert_eq!(
        cerrar_vencidas(&e.unidad, &reloj(), &e.ctx)
            .await
            .expect("aún no"),
        0
    );
    let despues = RelojFijo(ahora() + Duration::seconds(61));
    assert_eq!(
        cerrar_vencidas(&e.unidad, &despues, &e.ctx)
            .await
            .expect("cierra"),
        1
    );
    let guardada = e
        .unidad
        .datos()
        .asambleas
        .preguntas
        .into_iter()
        .find(|x| x.pregunta.id == p.pregunta.id)
        .expect("p");
    assert_eq!(guardada.pregunta.estado, domain::EstadoPregunta::Cerrada);
}

#[tokio::test]
async fn rotulo_vota_en_el_puesto_y_doble_escaneo_no_duplica() {
    let e = entorno();
    let r = en_registro(&e, ConfiguracionReunion::default()).await;
    let mut reg = registro(&r, e.duenos[0], vec![e.bienes[0]], vec![e.bienes[1]]);
    reg.rotulo = Some(CodigoRotulo::new("R-0001").expect("rótulo"));
    registrar_asistente(&e.unidad, &reloj(), &e.ctx, reg)
        .await
        .expect("registra");
    let r = instalar(&e.unidad, &reloj(), &e.ctx, r.id)
        .await
        .expect("instala");
    let p = pregunta_abierta(&e, &r, si_no()).await;
    let lectura = |clave: &str| SolicitudVoto {
        votante: Votante::Rotulo(CodigoRotulo::new("r-0001").expect("rótulo")),
        canal: Canal::Rotulo,
        ..voto(&r, &p, domain::AsistenteId::nuevo(), 0, clave)
    };
    // Sin permiso de operador no se registra.
    let sin_permiso = Contexto::de_cuenta(
        e.ctx.tenant_id,
        CuentaId::nuevo(),
        ConjuntoPermisos::vacio(),
    );
    assert!(matches!(
        emitir_voto(&e.unidad, &reloj(), &sin_permiso, lectura("a")).await,
        Err(AppError::SinPermiso(Permiso::AsambleasVotosRegistrar))
    ));
    emitir_voto(&e.unidad, &reloj(), &e.ctx, lectura("puesto-1"))
        .await
        .expect("primer puesto");
    let err = emitir_voto(&e.unidad, &reloj(), &e.ctx, lectura("puesto-2")).await;
    assert!(matches!(err, Err(AppError::BienPrivadoYaVoto)));
    assert_eq!(participaciones(&e, p.pregunta.id).len(), 2);
}

#[tokio::test]
async fn reservado_exige_permiso_para_el_detalle_y_audita_cada_consulta() {
    let e = entorno();
    let (r, a) = instalada_con(&e, ConfiguracionReunion::default()).await;
    let p = pregunta_abierta(
        &e,
        &r,
        DefinicionPregunta {
            tipo_voto: TipoVoto::Reservado,
            ..si_no()
        },
    )
    .await;
    emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, a, 0, "k"))
        .await
        .expect("vota");
    let ver = Contexto::de_cuenta(
        e.ctx.tenant_id,
        CuentaId::nuevo(),
        [Permiso::AsambleasVer].into_iter().collect(),
    );
    assert!(matches!(
        detalle_votos(&e.unidad, &reloj(), &ver, r.id, p.pregunta.id).await,
        Err(AppError::SinPermiso(Permiso::AsambleasVotosVerDetalle))
    ));
    let antes = e.unidad.confirmados().len();
    let detalle = detalle_votos(&e.unidad, &reloj(), &e.ctx, r.id, p.pregunta.id)
        .await
        .expect("detalle");
    assert_eq!(detalle.len(), 3);
    let eventos = e.unidad.confirmados();
    assert_eq!(eventos.len(), antes + 1);
    assert_eq!(
        eventos.last().expect("evento").1.accion.as_str(),
        "asambleas.detalle.consultar"
    );
}

#[tokio::test]
async fn eleccion_por_cociente_crea_los_roles_de_los_elegidos_con_cuenta() {
    let e = entorno();
    let (r, a) = instalada_con(&e, ConfiguracionReunion::default()).await;
    let b = registrar(&e, &r, 3, &[4])
        .await
        .expect("registra")
        .asistente
        .id;
    let sin_cuenta = PersonaId::nuevo();
    e.unidad.con_datos(|d| {
        d.personas.push(Persona::basica(
            sin_cuenta,
            TipoPersona::Natural,
            "Sin cuenta".into(),
            None,
        ));
    });
    let p = pregunta_abierta(
        &e,
        &r,
        DefinicionPregunta {
            enunciado: "Elección del consejo de administración".into(),
            tipo: TipoPregunta::Eleccion,
            cupos: Some(3),
            cargo: Some(CargoEleccion::Consejero),
            opciones: vec![
                NuevaOpcion {
                    texto: "Plancha 1".into(),
                    en_blanco: false,
                    candidatos: vec![e.duenos[0], e.duenos[1], sin_cuenta],
                },
                NuevaOpcion {
                    texto: "Plancha 2".into(),
                    en_blanco: false,
                    candidatos: vec![e.duenos[3], e.duenos[4]],
                },
            ],
            ..si_no()
        },
    )
    .await;
    // Plancha 1: 0.75; plancha 2: 0.25 → cociente 1/3: 2 y 0 por cociente; residuos 0.0833 y
    // 0.25 → el tercer cupo es de la plancha 2.
    emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, a, 0, "a"))
        .await
        .expect("vota");
    emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, b, 1, "b"))
        .await
        .expect("vota");
    cerrar_pregunta(&e.unidad, &reloj(), &e.ctx, r.id, p.pregunta.id)
        .await
        .expect("cierra");
    let elegidos = aplicar_eleccion(&e.unidad, &reloj(), &e.ctx, r.id, p.pregunta.id, None)
        .await
        .expect("aplica");
    assert_eq!(elegidos.len(), 3);
    assert_eq!(
        elegidos.iter().map(|x| x.persona_id).collect::<Vec<_>>(),
        vec![e.duenos[0], e.duenos[1], e.duenos[3]]
    );
    assert!(elegidos.iter().all(|x| x.rol_asignado));
    let asignaciones = e.unidad.datos().asignaciones;
    assert_eq!(
        asignaciones
            .iter()
            .filter(|(_, x)| x.origen == domain::OrigenAsignacion::Asamblea)
            .count(),
        3
    );
    assert!(matches!(
        aplicar_eleccion(&e.unidad, &reloj(), &e.ctx, r.id, p.pregunta.id, None).await,
        Err(AppError::Conflicto(_))
    ));
}

#[tokio::test]
async fn papeleta_virtual_ingresa_y_vota_por_web() {
    let e = entorno();
    let r = en_registro_modalidad(&e, Modalidad::Virtual, ConfiguracionReunion::default()).await;
    let mut reg = registro(&r, e.duenos[0], vec![e.bienes[0]], vec![e.bienes[1]]);
    reg.metodo = MetodoRegistro::Prerregistro;
    let res = registrar_asistente(&e.unidad, &reloj(), &e.ctx, reg)
        .await
        .expect("prerregistro");
    assert!(
        !res.asistente.presente,
        "el prerregistro no cuenta para el quórum"
    );
    let token = res.token_papeleta.expect("token");
    super::registro::ingresar_con_papeleta(&e.unidad, &reloj(), &e.ctx, &token)
        .await
        .expect("ingresa");
    let r = instalar(&e.unidad, &reloj(), &e.ctx, r.id)
        .await
        .expect("instala con 0.55");
    let p = pregunta_abierta(&e, &r, si_no()).await;
    let resultado = votar_con_papeleta(
        &e.unidad,
        &reloj(),
        &e.ctx,
        &token,
        p.pregunta.id,
        vec![p.pregunta.opciones[0].id],
        ClaveIdempotencia::new("web-1").expect("clave"),
    )
    .await
    .expect("vota");
    assert_eq!(resultado.bienes.len(), 2);
    assert!(matches!(
        votar_con_papeleta(
            &e.unidad,
            &reloj(),
            &e.ctx,
            "token-falso",
            p.pregunta.id,
            vec![],
            ClaveIdempotencia::new("x").expect("clave")
        )
        .await,
        Err(AppError::NoAutenticado)
    ));
}

#[tokio::test]
async fn consulta_escrita_habilita_a_los_representantes_y_cierra_al_vencer() {
    let e = entorno();
    let mut n = nueva(Modalidad::Escrita, ConfiguracionReunion::default());
    n.fecha_programada = ahora();
    n.plazo_respuesta = Some(ahora() + Duration::days(20));
    let r = crear_reunion(&e.unidad, &reloj(), &e.ctx, n)
        .await
        .expect("crea");
    let p = crear_pregunta(&e.unidad, &reloj(), &e.ctx, r.id, si_no())
        .await
        .expect("pregunta");
    let c = convocar(
        &e.unidad,
        &reloj(),
        &e.ctx,
        r.id,
        EnvioConvocatoriaOpciones::default(),
    )
    .await
    .expect("convoca");
    assert_eq!(c.reunion.estado, EstadoReunion::Instalada);
    let d = e.unidad.datos().asambleas;
    assert_eq!(d.asistentes.len(), 5);
    let guardada = d
        .preguntas
        .iter()
        .find(|x| x.pregunta.id == p.pregunta.id)
        .expect("p");
    assert_eq!(guardada.pregunta.estado, domain::EstadoPregunta::Abierta);
    assert_eq!(guardada.cierra_en, r.plazo_respuesta);

    let despues = RelojFijo(ahora() + Duration::days(21));
    cerrar_vencidas(&e.unidad, &despues, &e.ctx)
        .await
        .expect("cierra");
    let r = e
        .unidad
        .datos()
        .asambleas
        .reuniones
        .into_iter()
        .find(|x| x.id == r.id)
        .expect("r");
    assert_eq!(r.estado, EstadoReunion::Cerrada);
}

#[tokio::test]
async fn vista_previa_del_acta_trae_el_detalle_nominal_y_la_advertencia_de_unidades() {
    let e = entorno();
    let (r, a) = instalada_con(&e, ConfiguracionReunion::default()).await;
    let p = pregunta_abierta(
        &e,
        &r,
        DefinicionPregunta {
            base_computo: Some(domain::BaseComputo::Unidades),
            ..si_no()
        },
    )
    .await;
    emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p, a, 0, "k"))
        .await
        .expect("vota");
    cerrar_pregunta(&e.unidad, &reloj(), &e.ctx, r.id, p.pregunta.id)
        .await
        .expect("cierra");
    let acta = super::acta::vista_previa_acta(&e.unidad, &reloj(), &e.ctx, r.id)
        .await
        .expect("acta");
    assert_eq!(acta.preguntas.len(), 1);
    assert_eq!(acta.preguntas[0].detalle.len(), 3);
    assert!(acta.advertencias.iter().any(|a| a.contains("unidades")));
    assert_eq!(acta.asistentes[0].poderes.len(), 2);
}

// ---------------------------------------------------------------------------------------------
// WhatsApp
// ---------------------------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Sesiones(Arc<Mutex<Vec<String>>>);

impl SesionesConversacion for Sesiones {
    async fn ya_procesado(&self, wamid: &str) -> AppResult<bool> {
        Ok(self.0.lock().expect("mutex").iter().any(|w| w == wamid))
    }

    async fn marcar_procesado(&self, wamid: &str) -> AppResult<bool> {
        self.0.lock().expect("mutex").push(wamid.into());
        Ok(true)
    }

    async fn obtener(
        &self,
        _: &str,
        _: &str,
        _: DateTime<Utc>,
    ) -> AppResult<Option<SesionGuardada>> {
        Ok(None)
    }

    async fn cerrar_turno(&self, _: &CierreTurno) -> AppResult<bool> {
        Ok(true)
    }
}

struct Directorio(Vec<AsistenciaTelefono>);

impl DirectorioAsambleas for Directorio {
    async fn asistencias_por_telefono(&self, telefono: &str) -> AppResult<Vec<AsistenciaTelefono>> {
        Ok(self
            .0
            .iter()
            .filter(|_| !telefono.is_empty())
            .cloned()
            .collect())
    }

    async fn copropiedades_con_asambleas_pendientes(&self) -> AppResult<Vec<TenantId>> {
        Ok(Vec::new())
    }

    async fn registrar_acuse(&self, _: &AcuseEntrega) -> AppResult<()> {
        Ok(())
    }

    async fn actualizar_calidad_linea(&self, _: &str, _: bool, _: Option<&str>) -> AppResult<()> {
        Ok(())
    }
}

fn entrante(telefono: &str, wamid: &str, contenido: ContenidoEntrante) -> MensajeEntrante {
    MensajeEntrante {
        linea: "PNID".into(),
        telefono: telefono.into(),
        wamid: wamid.into(),
        recibido_en: ahora(),
        contenido,
    }
}

#[tokio::test]
async fn whatsapp_confirmar_vota_por_boton_y_el_reintento_de_meta_no_duplica() {
    let e = entorno();
    let (r, a) = instalada_con(&e, ConfiguracionReunion::default()).await;
    let p = pregunta_abierta(&e, &r, si_no()).await;
    let sesiones = Sesiones::default();
    let directorio = Directorio(Vec::new());
    let s = ServiciosAsamblea {
        unidad: &e.unidad,
        directorio: &directorio,
        sesiones: &sesiones,
        reloj: &reloj(),
    };
    let tel = telefono(0);
    // «Confirmar» de la plantilla: abre la ventana y le llegan las preguntas abiertas.
    let confirmar = entrante(
        &tel,
        "wamid.c",
        ContenidoEntrante::Respuesta {
            id: payload_confirmar(e.ctx.tenant_id, a),
            titulo: "Confirmar".into(),
        },
    );
    assert!(matches!(
        atender_mensaje_asamblea(&s, &confirmar)
            .await
            .expect("confirma"),
        ResultadoAsamblea::Atendido { respuestas: 2 }
    ));
    // Botón con la pregunta y la opción en el id.
    let boton = entrante(
        &tel,
        "wamid.v",
        ContenidoEntrante::Respuesta {
            id: id_voto(e.ctx.tenant_id, p.pregunta.id, p.pregunta.opciones[0].id),
            titulo: "1. Sí".into(),
        },
    );
    atender_mensaje_asamblea(&s, &boton).await.expect("vota");
    assert_eq!(
        atender_mensaje_asamblea(&s, &boton)
            .await
            .expect("reintento"),
        ResultadoAsamblea::Repetido
    );
    let parts = participaciones(&e, p.pregunta.id);
    assert_eq!(parts.len(), 3);
    assert!(
        parts
            .iter()
            .all(|x| x.canal == Canal::Whatsapp && x.wamid.as_deref() == Some("wamid.v"))
    );
    assert!(
        textos_outbox(&e)
            .iter()
            .any(|t| t.starts_with("✅ Voto registrado"))
    );

    // Desde otro teléfono el mismo botón no vota.
    let ajeno = entrante(
        &telefono(4),
        "wamid.x",
        ContenidoEntrante::Respuesta {
            id: id_voto(e.ctx.tenant_id, p.pregunta.id, p.pregunta.opciones[1].id),
            titulo: "2. No".into(),
        },
    );
    atender_mensaje_asamblea(&s, &ajeno)
        .await
        .expect("responde");
    assert_eq!(participaciones(&e, p.pregunta.id).len(), 3);
}

#[tokio::test]
async fn whatsapp_texto_libre_solo_con_una_pregunta_pendiente() {
    let e = entorno();
    let (r, a) = instalada_con(&e, ConfiguracionReunion::default()).await;
    let p1 = pregunta_abierta(&e, &r, si_no()).await;
    let p2 = pregunta_abierta(
        &e,
        &r,
        DefinicionPregunta {
            enunciado: "¿Aprueba la reforma?".into(),
            ..si_no()
        },
    )
    .await;
    let sesiones = Sesiones::default();
    let directorio = Directorio(vec![AsistenciaTelefono {
        tenant_id: e.ctx.tenant_id,
        copropiedad: "PH".into(),
        reunion_id: r.id,
        titulo: r.titulo.clone(),
        asistente_id: a,
        linea: Some("PNID".into()),
        confirmado: true,
    }]);
    let s = ServiciosAsamblea {
        unidad: &e.unidad,
        directorio: &directorio,
        sesiones: &sesiones,
        reloj: &reloj(),
    };
    let tel = telefono(0);
    // Dos pendientes: «sí» no vota; se reenvían las preguntas.
    let si = entrante(
        &tel,
        "wamid.1",
        ContenidoEntrante::Texto {
            cuerpo: "sí".into(),
        },
    );
    assert!(matches!(
        atender_mensaje_asamblea(&s, &si).await.expect("responde"),
        ResultadoAsamblea::Atendido { respuestas: 2 }
    ));
    assert!(participaciones(&e, p1.pregunta.id).is_empty());
    emitir_voto(&e.unidad, &reloj(), &e.ctx, voto(&r, &p1, a, 0, "web"))
        .await
        .expect("vota p1");
    // Ahora solo queda p2: «2» vota «No».
    let dos = entrante(
        &tel,
        "wamid.2",
        ContenidoEntrante::Texto { cuerpo: "2".into() },
    );
    atender_mensaje_asamblea(&s, &dos).await.expect("vota");
    let detalle = e.unidad.datos().asambleas.votos;
    assert!(
        detalle
            .iter()
            .filter(|v| v.pregunta_id == p2.pregunta.id)
            .all(|v| v.opcion_id == p2.pregunta.opciones[1].id)
    );
    assert_eq!(participaciones(&e, p2.pregunta.id).len(), 3);
    // «menú» va al chat de la copropiedad.
    let menu = entrante(
        &tel,
        "wamid.3",
        ContenidoEntrante::Texto {
            cuerpo: "menú".into(),
        },
    );
    assert_eq!(
        atender_mensaje_asamblea(&s, &menu)
            .await
            .expect("no aplica"),
        ResultadoAsamblea::NoAplica
    );
}

#[test]
fn opciones_de_texto_entiende_numeros_y_si_no() {
    let p = PreguntaGuardada {
        pregunta: domain::Pregunta {
            id: PreguntaId::nuevo(),
            reunion_id: ReunionId::nuevo(),
            enunciado: "x".into(),
            tipo: TipoPregunta::SeleccionMultiple,
            tipo_voto: TipoVoto::Nominal,
            base_computo: domain::BaseComputo::Coeficiente,
            regla_mayoria: ReglaMayoria::Simple,
            max_selecciones: 2,
            cupos: None,
            cargo: None,
            estado: domain::EstadoPregunta::Abierta,
            opciones: ["Sí", "No", "Otra"]
                .iter()
                .enumerate()
                .map(|(i, t)| domain::OpcionPregunta {
                    id: OpcionId::nuevo(),
                    orden: u16::try_from(i + 1).expect("orden"),
                    texto: (*t).into(),
                    en_blanco: false,
                })
                .collect(),
        },
        orden: 1,
        punto_id: None,
        temporizador_segundos: None,
        cierra_en: None,
        abierta_en: None,
        cerrada_en: None,
        resultado: None,
        candidatos: Vec::new(),
        roles_aplicados_en: None,
    };
    use super::whatsapp::opciones_de_texto as o;
    let ids = |v: &[usize]| {
        v.iter()
            .map(|i| p.pregunta.opciones[*i].id)
            .collect::<Vec<_>>()
    };
    assert_eq!(o(&p, "1,3"), Some(ids(&[0, 2])));
    assert_eq!(o(&p, " Sí "), Some(ids(&[0])));
    assert_eq!(o(&p, "no"), Some(ids(&[1])));
    assert_eq!(o(&p, "9"), None);
    assert_eq!(o(&p, "hola"), None);
}
