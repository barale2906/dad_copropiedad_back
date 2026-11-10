//! Pruebas de los casos de uso de control de acceso con dobles en memoria.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, NaiveDate, TimeDelta, TimeZone, Utc};
use domain::{
    Almacen, BienPrivadoId, CalidadResidencia, CanalRespuestaVisita, CodigoBienPrivado,
    ConfiguracionAcceso, ConjuntoPermisos, CuentaId, EstadoCorrespondencia, EstadoCredencial,
    EstadoVisita, Modulo, Permiso, PersonaId, Porcentaje, PuntoAccesoId, ResidenciaId,
    ResultadoAcceso, ResultadoLlamada, SentidoAcceso, Telefono, TenantId, TipoBienPrivado,
    TipoCorrespondencia, TipoCredencial, TipoPersona, TipoPuntoAcceso, TipoTitularidad, TipoVisita,
    TitularidadId, UsoBienPrivado,
};
use rust_decimal::dec;

use super::{
    correspondencia::{self as corr, NuevaCorrespondencia, NuevaEntrega, QuienRecogeEntrada},
    credenciales::{self as cred, NuevaCredencial, NuevoConsentimiento},
    porteria, puentes,
    tareas::{aplicar_retencion, escalar_avisos},
    visitas::{self, NuevaPreautorizacion, NuevaVisita, ResultadoDecision},
    whatsapp::{ResultadoVisitaWhatsapp, atender_respuesta_visita, boton_visita},
};
use crate::{
    AppError, AppResult, Contexto, SolicitudPagina,
    outbox::MensajeOutbox,
    puertos::{
        DirectorioAcceso, FirmaQr, SesionesConversacion,
        acceso::{FiltroVisitas, TitularAcceso},
        archivos::memoria::AlmacenEnMemoria,
        censo::Residencia,
        copropiedades::{BienPrivado, Persona, Titularidad},
        mensajeria::{ContenidoEntrante, ContenidoSaliente, MensajeEntrante},
        reloj::RelojFijo,
        sesiones::{CierreTurno, SesionGuardada},
        unidad_trabajo::memoria::UnidadEnMemoria,
    },
};

/// Lunes 9 de noviembre de 2026, 15:00 en Bogotá.
fn t0() -> DateTime<Utc> {
    domain::reserva::Tz::America__Bogota
        .with_ymd_and_hms(2026, 11, 9, 15, 0, 0)
        .single()
        .expect("fecha")
        .with_timezone(&Utc)
}

/// PNG mínimo válido (firma o foto).
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0,
    1, 0, 0, 0, 1, 8, 2, 0, 0, 0, 0x90, 0x77, 0x53, 0xDE,
];

/// Firma de prueba: SHA-256 del tenant y la carga, repetido (64 bytes). No es Ed25519, pero
/// cumple el contrato: solo verifica lo que firmó para ese tenant.
struct FirmaFalsa;

impl FirmaQr for FirmaFalsa {
    fn firmar(&self, tenant: TenantId, carga: &[u8]) -> Vec<u8> {
        use sha2::{Digest, Sha256};
        let h = Sha256::digest([tenant.as_uuid().as_bytes().as_slice(), carga].concat());
        [h.as_slice(), h.as_slice()].concat()
    }

    fn verificar(&self, tenant: TenantId, carga: &[u8], firma: &[u8]) -> bool {
        self.firmar(tenant, carga) == firma
    }

    fn clave_publica(&self, _tenant: TenantId) -> [u8; 32] {
        [9; 32]
    }
}

#[derive(Default, Clone)]
struct Sesiones(Arc<Mutex<Vec<String>>>);

impl SesionesConversacion for Sesiones {
    async fn ya_procesado(&self, wamid: &str) -> AppResult<bool> {
        Ok(self.0.lock().expect("mutex").iter().any(|w| w == wamid))
    }

    async fn marcar_procesado(&self, wamid: &str) -> AppResult<bool> {
        self.0.lock().expect("mutex").push(wamid.to_owned());
        Ok(true)
    }

    async fn obtener(
        &self,
        _l: &str,
        _t: &str,
        _a: DateTime<Utc>,
    ) -> AppResult<Option<SesionGuardada>> {
        Ok(None)
    }

    async fn cerrar_turno(&self, _c: &CierreTurno) -> AppResult<bool> {
        Ok(true)
    }
}

struct Directorio {
    tenant: TenantId,
    puentes: Vec<(String, domain::PuenteId)>,
}

impl DirectorioAcceso for Directorio {
    async fn copropiedades_con_acceso_pendiente(
        &self,
        _a: DateTime<Utc>,
    ) -> AppResult<Vec<TenantId>> {
        Ok(vec![self.tenant])
    }

    async fn copropiedades_con_control_acceso(&self) -> AppResult<Vec<TenantId>> {
        Ok(vec![self.tenant])
    }

    async fn puente_por_credencial(
        &self,
        huella: &str,
    ) -> AppResult<Option<(TenantId, domain::PuenteId)>> {
        Ok(self
            .puentes
            .iter()
            .find(|(h, _)| h == huella)
            .map(|(_, p)| (self.tenant, *p)))
    }

    async fn preparar_particiones(&self, _m: u32) -> AppResult<()> {
        Ok(())
    }
}

/// Bien T1-101 con:
/// - `responsable`: residente responsable, con app (cuenta + token) y teléfono;
/// - `hijo`: residente menor de edad, con teléfono;
/// - `tenedora`: residente adulta, solo WhatsApp;
/// - `dueno`: propietario que no vive allí, con teléfono.
struct Entorno {
    unidad: UnidadEnMemoria,
    almacen: AlmacenEnMemoria,
    tenant: TenantId,
    bien: BienPrivadoId,
    responsable: PersonaId,
    cuenta_responsable: CuentaId,
    hijo: PersonaId,
    tenedora: PersonaId,
    cuenta_tenedora: CuentaId,
    dueno: PersonaId,
    cuenta_dueno: CuentaId,
}

fn persona(
    id: PersonaId,
    nombre: &str,
    telefono: &str,
    cuenta: Option<CuentaId>,
    nacimiento: Option<NaiveDate>,
) -> Persona {
    let mut p = Persona::basica(id, TipoPersona::Natural, nombre.into(), None);
    p.telefono = Some(Telefono::new(telefono).expect("teléfono"));
    p.cuenta_id = cuenta;
    p.fecha_nacimiento = nacimiento;
    p
}

fn residencia(bien: BienPrivadoId, p: PersonaId, responsable: bool) -> Residencia {
    Residencia {
        id: ResidenciaId::nuevo(),
        bien_privado_id: bien,
        persona_id: p,
        calidad: CalidadResidencia::Familiar,
        es_responsable: responsable,
        autorizado_por: None,
        contrato_archivo_id: None,
        vigente_desde: t0() - TimeDelta::days(30),
        vigente_hasta: None,
        reservas_autorizadas: true,
    }
}

fn entorno() -> Entorno {
    let unidad = UnidadEnMemoria::default();
    unidad.habilitar(Modulo::ControlAcceso);
    let e = Entorno {
        unidad,
        almacen: AlmacenEnMemoria::new(Almacen::Local),
        tenant: TenantId::nuevo(),
        bien: BienPrivadoId::nuevo(),
        responsable: PersonaId::nuevo(),
        cuenta_responsable: CuentaId::nuevo(),
        hijo: PersonaId::nuevo(),
        tenedora: PersonaId::nuevo(),
        cuenta_tenedora: CuentaId::nuevo(),
        dueno: PersonaId::nuevo(),
        cuenta_dueno: CuentaId::nuevo(),
    };
    let (bien, r, h, t, d) = (e.bien, e.responsable, e.hijo, e.tenedora, e.dueno);
    let (cr, ct, cd) = (e.cuenta_responsable, e.cuenta_tenedora, e.cuenta_dueno);
    e.unidad.con_datos(|x| {
        x.bienes.insert(
            bien,
            BienPrivado {
                id: bien,
                codigo: CodigoBienPrivado::new("T1-101").expect("código"),
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
                coeficiente: None,
            },
        );
        x.personas.push(persona(
            r,
            "Rita Responsable",
            "573001000001",
            Some(cr),
            None,
        ));
        x.personas.push(persona(
            h,
            "Hugo Hijo",
            "573001000002",
            None,
            NaiveDate::from_ymd_opt(2014, 1, 1),
        ));
        x.personas
            .push(persona(t, "Tina Tenedora", "573001000003", Some(ct), None));
        x.personas
            .push(persona(d, "Dario Dueño", "573001000004", Some(cd), None));
        x.residencias.push(residencia(bien, r, true));
        x.residencias.push(residencia(bien, h, false));
        x.residencias.push(residencia(bien, t, false));
        x.titularidades.push(Titularidad {
            id: TitularidadId::nuevo(),
            bien_privado_id: bien,
            persona_id: d,
            tipo: TipoTitularidad::Propietario,
            porcentaje: Porcentaje::new(dec!(100)).expect("porcentaje"),
            es_representante: true,
            vigente_desde: t0() - TimeDelta::days(400),
            vigente_hasta: None,
            soporte_archivo_id: None,
        });
        x.gestion.linea = Some("PNID".into());
        x.acceso.tokens.push((cr, "token-rita".into()));
    });
    e
}

impl Entorno {
    fn porteria(&self) -> Contexto {
        Contexto::de_cuenta(
            self.tenant,
            CuentaId::nuevo(),
            [
                Permiso::AccesoRegistrar,
                Permiso::AccesoBitacoraVer,
                Permiso::AccesoCorrespondenciaEntregar,
                Permiso::AccesoMinutaRegistrar,
                Permiso::AccesoMinutaVer,
            ]
            .into_iter()
            .collect(),
        )
    }

    fn residente(&self, cuenta: CuentaId) -> Contexto {
        Contexto::de_cuenta(
            self.tenant,
            cuenta,
            [Permiso::AccesoVisitantesAutorizar].into_iter().collect(),
        )
    }

    fn admin(&self) -> Contexto {
        Contexto::de_cuenta(
            self.tenant,
            CuentaId::nuevo(),
            ConjuntoPermisos::todos(domain::Ambito::Copropiedad),
        )
    }

    fn sistema(&self) -> Contexto {
        Contexto::new(self.tenant)
    }

    fn directorio(&self) -> Directorio {
        Directorio {
            tenant: self.tenant,
            puentes: Vec::new(),
        }
    }

    /// Destinos de los avisos encolados (token push o teléfono), en orden.
    fn destinos(&self) -> Vec<String> {
        self.unidad
            .outbox()
            .into_iter()
            .map(|(_, e)| e.mensaje.destino().to_owned())
            .collect()
    }

    fn visita(&self) -> NuevaVisita {
        NuevaVisita {
            bien_privado_id: self.bien,
            tipo: TipoVisita::Domicilio,
            visitante_nombre: "Mensajero Rappi".into(),
            visitante_documento: None,
            placa: None,
            observacion: None,
        }
    }

    async fn anunciar(&self) -> crate::puertos::acceso::Visita {
        visitas::anunciar_visita(
            &self.unidad,
            &RelojFijo(t0()),
            &self.porteria(),
            self.visita(),
        )
        .await
        .expect("anunciar")
    }
}

// ---------------------------------------------------------------------------------------------
// Visitas
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn el_anuncio_avisa_solo_al_primer_respondiente_por_la_app() {
    let e = entorno();

    let v = e.anunciar().await;

    assert_eq!(v.estado, EstadoVisita::Anunciada);
    assert_eq!(v.avisos_enviados, 1);
    assert_eq!(v.siguiente_paso_en, Some(t0() + TimeDelta::minutes(2)));
    let outbox = e.unidad.outbox();
    assert_eq!(outbox.len(), 1, "un solo aviso: el push de la responsable");
    let MensajeOutbox::Push(n) = &outbox[0].1.mensaje else {
        panic!("debía ser push")
    };
    assert_eq!(n.token, "token-rita");
    assert_eq!(n.datos.get("visita_id"), Some(&v.id.to_string()));
    let respaldo = n.respaldo.as_ref().expect("respaldo por WhatsApp");
    assert_eq!(respaldo.telefono, "573001000001");
    let ContentoBotones(botones) = botones(&respaldo.contenido);
    assert_eq!(
        botones,
        vec![
            boton_visita(e.tenant, v.id, true),
            boton_visita(e.tenant, v.id, false)
        ]
    );
}

struct ContentoBotones(Vec<String>);

fn botones(c: &ContenidoSaliente) -> ContentoBotones {
    match c {
        ContenidoSaliente::Plantilla { botones, .. } => ContentoBotones(botones.clone()),
        _ => ContentoBotones(Vec::new()),
    }
}

#[tokio::test]
async fn a_los_dos_minutos_se_avisa_a_los_adultos_y_a_los_cinco_hay_que_llamar() {
    let e = entorno();
    let v = e.anunciar().await;

    let r = escalar_avisos(
        &e.directorio(),
        &e.unidad,
        &RelojFijo(t0() + TimeDelta::minutes(2)),
    )
    .await
    .expect("escalar");

    assert_eq!(r.segundos_avisos, 1);
    // Push a la responsable y WhatsApp a la tenedora; nunca al menor ni al dueño no residente.
    assert_eq!(
        e.destinos(),
        vec!["token-rita", "token-rita", "573001000003"]
    );
    let r = escalar_avisos(
        &e.directorio(),
        &e.unidad,
        &RelojFijo(t0() + TimeDelta::minutes(5)),
    )
    .await
    .expect("escalar");
    assert_eq!(r.llamadas, 1);
    let x = e
        .unidad
        .datos()
        .acceso
        .visitas
        .into_iter()
        .find(|x| x.id == v.id)
        .expect("visita");
    assert_eq!(x.estado, EstadoVisita::RequiereLlamada);
    let contactos =
        visitas::contactos_para_llamar(&e.unidad, &RelojFijo(t0()), &e.porteria(), v.id)
            .await
            .expect("contactos");
    assert_eq!(
        contactos.iter().map(|c| c.persona_id).collect::<Vec<_>>(),
        vec![e.responsable, e.tenedora]
    );
}

#[tokio::test]
async fn el_escalamiento_no_repite_avisos() {
    let e = entorno();
    e.anunciar().await;
    let reloj = RelojFijo(t0() + TimeDelta::minutes(3));
    escalar_avisos(&e.directorio(), &e.unidad, &reloj)
        .await
        .expect("escalar");
    let antes = e.unidad.outbox().len();

    let r = escalar_avisos(&e.directorio(), &e.unidad, &reloj)
        .await
        .expect("escalar");

    assert_eq!(r.segundos_avisos, 0);
    assert_eq!(e.unidad.outbox().len(), antes);
}

#[tokio::test]
async fn sin_nadie_a_quien_avisar_la_visita_queda_para_llamar() {
    let e = entorno();
    e.unidad.con_datos(|d| {
        d.residencias.clear();
        d.titularidades.clear();
    });

    let v = e.anunciar().await;

    assert_eq!(v.estado, EstadoVisita::RequiereLlamada);
    assert!(e.unidad.outbox().is_empty());
}

#[tokio::test]
async fn la_primera_respuesta_gana() {
    let e = entorno();
    let v = e.anunciar().await;
    let reloj = RelojFijo(t0() + TimeDelta::seconds(30));

    let r1 = visitas::decidir_visita(
        &e.unidad,
        &reloj,
        &e.residente(e.cuenta_responsable),
        v.id,
        true,
        CanalRespuestaVisita::App,
    )
    .await
    .expect("decidir");
    let r2 = visitas::decidir_visita(
        &e.unidad,
        &reloj,
        &e.residente(e.cuenta_tenedora),
        v.id,
        false,
        CanalRespuestaVisita::App,
    )
    .await
    .expect("decidir");

    let ResultadoDecision::Decidida(x) = r1 else {
        panic!("debía decidir")
    };
    assert_eq!(x.estado, EstadoVisita::Autorizada);
    assert_eq!(x.decidida_por, Some(e.responsable));
    assert!(matches!(r2, ResultadoDecision::YaDecidida(y) if y.estado == EstadoVisita::Autorizada));
    let r = escalar_avisos(
        &e.directorio(),
        &e.unidad,
        &RelojFijo(t0() + TimeDelta::minutes(5)),
    )
    .await
    .expect("escalar");
    assert_eq!(
        (r.segundos_avisos, r.llamadas),
        (0, 0),
        "decidida: no escala"
    );
}

#[tokio::test]
async fn el_propietario_no_residente_no_decide_salvo_que_viva_solo_una_persona_mayor() {
    let e = entorno();
    let v = e.anunciar().await;
    let reloj = RelojFijo(t0());

    let r = visitas::decidir_visita(
        &e.unidad,
        &reloj,
        &e.residente(e.cuenta_dueno),
        v.id,
        true,
        CanalRespuestaVisita::App,
    )
    .await;
    assert!(matches!(
        r,
        Err(AppError::SinPermiso(Permiso::AccesoVisitantesAutorizar))
    ));

    let (resp, hijo, ten) = (e.responsable, e.hijo, e.tenedora);
    e.unidad.con_datos(|d| {
        d.residencias.retain(|r| r.persona_id == resp);
        d.residencias
            .retain(|r| r.persona_id != hijo && r.persona_id != ten);
        if let Some(p) = d.personas.iter_mut().find(|p| p.id == resp) {
            p.fecha_nacimiento = NaiveDate::from_ymd_opt(1950, 6, 1);
        }
    });
    let r = visitas::decidir_visita(
        &e.unidad,
        &reloj,
        &e.residente(e.cuenta_dueno),
        v.id,
        true,
        CanalRespuestaVisita::App,
    )
    .await
    .expect("decidir");
    assert!(matches!(r, ResultadoDecision::Decidida(_)));
}

#[tokio::test]
async fn sin_el_modulo_no_hay_porteria() {
    let e = entorno();
    e.unidad.con_datos(|d| d.modulos.clear());

    let r = visitas::anunciar_visita(&e.unidad, &RelojFijo(t0()), &e.porteria(), e.visita()).await;

    assert!(matches!(
        r,
        Err(AppError::ModuloNoHabilitado(Modulo::ControlAcceso))
    ));
}

#[tokio::test]
async fn la_llamada_decide_la_visita() {
    let e = entorno();
    let v = e.anunciar().await;

    let x = visitas::registrar_llamada(
        &e.unidad,
        &RelojFijo(t0()),
        &e.porteria(),
        v.id,
        ResultadoLlamada::NoContesto,
        None,
    )
    .await
    .expect("llamada");

    assert_eq!(x.estado, EstadoVisita::SinRespuesta);
    let r =
        visitas::registrar_ingreso(&e.unidad, &RelojFijo(t0()), &e.porteria(), v.id, None).await;
    assert!(
        matches!(r, Err(AppError::Conflicto(_))),
        "sin autorización no ingresa"
    );
}

fn mensaje(boton: String, telefono: &str, wamid: &str) -> MensajeEntrante {
    MensajeEntrante {
        linea: "PNID".into(),
        telefono: telefono.into(),
        wamid: wamid.into(),
        recibido_en: t0(),
        contenido: ContenidoEntrante::Respuesta {
            id: boton,
            titulo: "Autorizar".into(),
        },
    }
}

#[tokio::test]
async fn autorizar_por_whatsapp_es_idempotente_y_valida_el_telefono() {
    let e = entorno();
    let v = e.anunciar().await;
    let sesiones = Sesiones::default();
    let reloj = RelojFijo(t0());

    let ajeno = mensaje(
        boton_visita(e.tenant, v.id, true),
        "573009999999",
        "wamid.1",
    );
    atender_respuesta_visita(&e.unidad, &sesiones, &reloj, &ajeno)
        .await
        .expect("atender");
    let x = e.unidad.datos().acceso.visitas[0].clone();
    assert_eq!(
        x.estado,
        EstadoVisita::Anunciada,
        "un teléfono ajeno no decide"
    );

    let menor = mensaje(
        boton_visita(e.tenant, v.id, true),
        "573001000002",
        "wamid.2",
    );
    atender_respuesta_visita(&e.unidad, &sesiones, &reloj, &menor)
        .await
        .expect("atender");
    assert_eq!(
        e.unidad.datos().acceso.visitas[0].estado,
        EstadoVisita::Anunciada,
        "un menor no decide"
    );

    let ok = mensaje(
        boton_visita(e.tenant, v.id, true),
        "573001000003",
        "wamid.3",
    );
    let r = atender_respuesta_visita(&e.unidad, &sesiones, &reloj, &ok)
        .await
        .expect("atender");
    assert_eq!(r, ResultadoVisitaWhatsapp::Atendido { respuestas: 1 });
    let x = e.unidad.datos().acceso.visitas[0].clone();
    assert_eq!(
        (x.estado, x.decidida_por, x.canal_decision),
        (
            EstadoVisita::Autorizada,
            Some(e.tenedora),
            Some(CanalRespuestaVisita::Whatsapp)
        )
    );

    let r = atender_respuesta_visita(&e.unidad, &sesiones, &reloj, &ok)
        .await
        .expect("atender");
    assert_eq!(r, ResultadoVisitaWhatsapp::Repetido);
    let otro = mensaje("vt:no-es-visita".into(), "573001000003", "wamid.4");
    assert_eq!(
        atender_respuesta_visita(&e.unidad, &sesiones, &reloj, &otro)
            .await
            .expect("atender"),
        ResultadoVisitaWhatsapp::NoAplica
    );
}

#[tokio::test]
async fn preautorizacion_con_qr_respeta_usos_y_ventana() {
    let e = entorno();
    let ctx = e.residente(e.cuenta_responsable);
    let (v, qr) = visitas::preautorizar_visita(
        &e.unidad,
        &RelojFijo(t0()),
        &FirmaFalsa,
        &ctx,
        NuevaPreautorizacion {
            visita: e.visita(),
            desde: t0(),
            hasta: t0() + TimeDelta::hours(5),
            usos: 2,
        },
    )
    .await
    .expect("preautorizar");
    assert_eq!(v.estado, EstadoVisita::Autorizada);
    assert_eq!(
        visitas::qr_de_visita(&e.unidad, &RelojFijo(t0()), &FirmaFalsa, &ctx, v.id)
            .await
            .expect("qr"),
        qr
    );

    let porteria = e.porteria();
    let reloj = RelojFijo(t0() + TimeDelta::hours(1));
    let x = visitas::ingreso_por_qr(&e.unidad, &reloj, &FirmaFalsa, &porteria, &qr, None)
        .await
        .expect("ingreso");
    assert_eq!((x.estado, x.usos), (EstadoVisita::Ingresada, 1));
    let x = visitas::registrar_salida(&e.unidad, &reloj, &porteria, v.id, None)
        .await
        .expect("salida");
    assert_eq!(x.estado, EstadoVisita::Autorizada, "queda un uso");
    visitas::ingreso_por_qr(&e.unidad, &reloj, &FirmaFalsa, &porteria, &qr, None)
        .await
        .expect("segundo ingreso");
    let x = visitas::registrar_salida(&e.unidad, &reloj, &porteria, v.id, None)
        .await
        .expect("salida");
    assert_eq!(x.estado, EstadoVisita::Finalizada);
    let r = visitas::ingreso_por_qr(&e.unidad, &reloj, &FirmaFalsa, &porteria, &qr, None).await;
    assert!(r.is_err(), "sin usos");
}

#[tokio::test]
async fn un_qr_alterado_o_de_otra_ph_no_sirve() {
    let e = entorno();
    let (_, qr) = visitas::preautorizar_visita(
        &e.unidad,
        &RelojFijo(t0()),
        &FirmaFalsa,
        &e.residente(e.cuenta_responsable),
        NuevaPreautorizacion {
            visita: e.visita(),
            desde: t0(),
            hasta: t0() + TimeDelta::hours(5),
            usos: 1,
        },
    )
    .await
    .expect("preautorizar");
    let mut alterado = qr.clone();
    alterado.pop();
    alterado.push(if qr.ends_with('0') { '1' } else { '0' });

    let r = visitas::ingreso_por_qr(
        &e.unidad,
        &RelojFijo(t0()),
        &FirmaFalsa,
        &e.porteria(),
        &alterado,
        None,
    )
    .await;
    assert!(matches!(r, Err(AppError::Validacion(_))));
    let otra = Contexto::de_cuenta(
        TenantId::nuevo(),
        CuentaId::nuevo(),
        [Permiso::AccesoRegistrar].into_iter().collect(),
    );
    let r =
        visitas::ingreso_por_qr(&e.unidad, &RelojFijo(t0()), &FirmaFalsa, &otra, &qr, None).await;
    assert!(matches!(r, Err(AppError::Validacion(_))));
}

#[tokio::test]
async fn la_autorizacion_de_un_anuncio_vence_si_no_ingresa() {
    let e = entorno();
    let v = e.anunciar().await;
    visitas::decidir_visita(
        &e.unidad,
        &RelojFijo(t0()),
        &e.residente(e.cuenta_responsable),
        v.id,
        true,
        CanalRespuestaVisita::App,
    )
    .await
    .expect("decidir");

    let r = escalar_avisos(
        &e.directorio(),
        &e.unidad,
        &RelojFijo(t0() + TimeDelta::hours(2)),
    )
    .await
    .expect("escalar");

    assert_eq!(r.vencidas, 1);
    assert_eq!(
        e.unidad.datos().acceso.visitas[0].estado,
        EstadoVisita::Vencida
    );
}

#[tokio::test]
async fn un_residente_solo_ve_las_visitas_de_su_bien() {
    let e = entorno();
    e.anunciar().await;
    let ajeno = Contexto::de_cuenta(
        e.tenant,
        CuentaId::nuevo(),
        [Permiso::AccesoVisitantesAutorizar].into_iter().collect(),
    );
    let filtro = FiltroVisitas {
        bien_privado_id: Some(e.bien),
        ..FiltroVisitas::default()
    };

    let r = visitas::listar_visitas(
        &e.unidad,
        &RelojFijo(t0()),
        &ajeno,
        filtro.clone(),
        SolicitudPagina::default(),
    )
    .await;
    assert!(matches!(r, Err(AppError::SinPermiso(_))));
    let p = visitas::listar_visitas(
        &e.unidad,
        &RelojFijo(t0()),
        &e.porteria(),
        FiltroVisitas::default(),
        SolicitudPagina::default(),
    )
    .await
    .expect("listar");
    assert_eq!(p.datos.len(), 1);
}

// ---------------------------------------------------------------------------------------------
// Correspondencia
// ---------------------------------------------------------------------------------------------

fn paquete(e: &Entorno) -> NuevaCorrespondencia {
    NuevaCorrespondencia {
        bien_privado_id: e.bien,
        tipo: TipoCorrespondencia::Paquete,
        remitente: Some("Servientrega".into()),
        guia: Some("123".into()),
        ubicacion: None,
        foto: None,
    }
}

async fn recibir(e: &Entorno, cuando: DateTime<Utc>) -> crate::puertos::acceso::Correspondencia {
    corr::registrar_correspondencia(
        &e.unidad,
        &e.almacen,
        &RelojFijo(cuando),
        &e.porteria(),
        paquete(e),
    )
    .await
    .expect("registrar")
}

#[tokio::test]
async fn la_correspondencia_se_avisa_cada_24_horas_a_todos_hasta_el_tope() {
    let e = entorno();
    e.unidad.con_datos(|d| {
        d.acceso.configuracion = Some(ConfiguracionAcceso {
            dias_avisos_correspondencia: 2,
            ..ConfiguracionAcceso::default()
        });
    });
    let c = recibir(&e, t0()).await;
    assert_eq!(c.avisos_enviados, 1);
    assert_eq!(
        e.destinos(),
        vec!["token-rita"],
        "primer aviso: primer respondiente"
    );

    let r = escalar_avisos(
        &e.directorio(),
        &e.unidad,
        &RelojFijo(t0() + TimeDelta::hours(23)),
    )
    .await
    .expect("escalar");
    assert_eq!(r.avisos_correspondencia, 0);
    let r = escalar_avisos(
        &e.directorio(),
        &e.unidad,
        &RelojFijo(t0() + TimeDelta::hours(24)),
    )
    .await
    .expect("escalar");
    assert_eq!(r.avisos_correspondencia, 1);
    assert_eq!(e.destinos()[1..], ["token-rita", "573001000003"]);
    escalar_avisos(
        &e.directorio(),
        &e.unidad,
        &RelojFijo(t0() + TimeDelta::hours(48)),
    )
    .await
    .expect("escalar");
    let x = e.unidad.datos().acceso.correspondencia[0].clone();
    assert_eq!(x.avisos_enviados, 3);
    assert_eq!(x.siguiente_aviso_en, None, "se agotaron los días");

    let r = escalar_avisos(
        &e.directorio(),
        &e.unidad,
        &RelojFijo(t0() + TimeDelta::hours(49)),
    )
    .await
    .expect("escalar");
    assert_eq!(r.alertas_administracion, 1);
    let r = escalar_avisos(
        &e.directorio(),
        &e.unidad,
        &RelojFijo(t0() + TimeDelta::hours(50)),
    )
    .await
    .expect("escalar");
    assert_eq!(r.alertas_administracion, 0, "la alerta sale una vez");
}

#[tokio::test]
async fn un_paquete_de_noche_se_avisa_al_terminar_el_silencio() {
    let e = entorno();
    let noche = t0() + TimeDelta::hours(8); // 23:00 en Bogotá

    let c = recibir(&e, noche).await;

    assert_eq!(c.avisos_enviados, 0);
    assert!(e.unidad.outbox().is_empty());
    let manana = t0() + TimeDelta::hours(16); // 07:00 del día siguiente
    assert_eq!(c.siguiente_aviso_en, Some(manana));
    let r = escalar_avisos(&e.directorio(), &e.unidad, &RelojFijo(manana))
        .await
        .expect("escalar");
    assert_eq!(r.avisos_correspondencia, 1);
    assert_eq!(e.destinos(), vec!["token-rita"]);
}

#[tokio::test]
async fn la_entrega_exige_firma_o_foto_y_alguien_del_bien() {
    let e = entorno();
    let c = recibir(&e, t0()).await;
    let porteria = e.porteria();
    let reloj = RelojFijo(t0() + TimeDelta::hours(1));

    let sin_soporte = NuevaEntrega {
        quien: QuienRecogeEntrada::Registrada(e.tenedora),
        firma: None,
        foto: None,
    };
    let r =
        corr::entregar_correspondencia(&e.unidad, &e.almacen, &reloj, &porteria, c.id, sin_soporte)
            .await;
    assert!(matches!(r, Err(AppError::Dominio(_))));
    let extrano = NuevaEntrega {
        quien: QuienRecogeEntrada::Registrada(PersonaId::nuevo()),
        firma: Some(PNG.to_vec()),
        foto: None,
    };
    let r = corr::entregar_correspondencia(&e.unidad, &e.almacen, &reloj, &porteria, c.id, extrano)
        .await;
    assert!(matches!(r, Err(AppError::Validacion(_))));
    assert!(
        e.almacen.claves().is_empty(),
        "la firma del intento fallido se borró"
    );

    let ok = NuevaEntrega {
        quien: QuienRecogeEntrada::Registrada(e.tenedora),
        firma: Some(PNG.to_vec()),
        foto: None,
    };
    let d = corr::entregar_correspondencia(&e.unidad, &e.almacen, &reloj, &porteria, c.id, ok)
        .await
        .expect("entregar");
    assert_eq!(d.correspondencia.estado, EstadoCorrespondencia::Entregada);
    let entrega = d.entrega.expect("entrega");
    assert_eq!(entrega.persona_id, Some(e.tenedora));
    assert_eq!(entrega.soportes_hasta, reloj.0 + TimeDelta::days(365));
    assert_eq!(e.almacen.claves().len(), 1);
    let otra = NuevaEntrega {
        quien: QuienRecogeEntrada::Registrada(e.tenedora),
        firma: Some(PNG.to_vec()),
        foto: None,
    };
    let r =
        corr::entregar_correspondencia(&e.unidad, &e.almacen, &reloj, &porteria, c.id, otra).await;
    assert!(r.is_err(), "no se entrega dos veces");

    let r = escalar_avisos(
        &e.directorio(),
        &e.unidad,
        &RelojFijo(t0() + TimeDelta::days(3)),
    )
    .await
    .expect("escalar");
    assert_eq!(
        r.avisos_correspondencia + r.alertas_administracion,
        0,
        "entregada: sin más avisos"
    );
}

#[tokio::test]
async fn un_tercero_puede_recoger_con_documento_y_foto() {
    let e = entorno();
    let c = recibir(&e, t0()).await;
    let entrega = NuevaEntrega {
        quien: QuienRecogeEntrada::Tercero {
            nombre: "Vecino Amable".into(),
            documento: "1.023.456".into(),
        },
        firma: None,
        foto: Some(PNG.to_vec()),
    };

    let d = corr::entregar_correspondencia(
        &e.unidad,
        &e.almacen,
        &RelojFijo(t0()),
        &e.porteria(),
        c.id,
        entrega,
    )
    .await
    .expect("entregar");

    let entrega = d.entrega.expect("entrega");
    assert_eq!(entrega.tercero_nombre.as_deref(), Some("Vecino Amable"));
    assert!(entrega.foto_archivo_id.is_some() && entrega.firma_archivo_id.is_none());
    let vista = corr::ver_correspondencia(
        &e.unidad,
        &RelojFijo(t0()),
        &e.residente(e.cuenta_responsable),
        c.id,
    )
    .await
    .expect("ver");
    assert_eq!(
        vista.entrega.and_then(|x| x.tercero_documento),
        None,
        "el residente no ve el documento del tercero"
    );
}

#[tokio::test]
async fn los_soportes_se_borran_al_vencer_su_plazo() {
    let e = entorno();
    let c = recibir(&e, t0()).await;
    let entrega = NuevaEntrega {
        quien: QuienRecogeEntrada::Registrada(e.responsable),
        firma: Some(PNG.to_vec()),
        foto: None,
    };
    corr::entregar_correspondencia(
        &e.unidad,
        &e.almacen,
        &RelojFijo(t0()),
        &e.porteria(),
        c.id,
        entrega,
    )
    .await
    .expect("entregar");

    let r = aplicar_retencion(
        &e.directorio(),
        &e.unidad,
        &e.almacen,
        &RelojFijo(t0() + TimeDelta::days(364)),
    )
    .await
    .expect("retención");
    assert_eq!(r.soportes_purgados, 0);
    let r = aplicar_retencion(
        &e.directorio(),
        &e.unidad,
        &e.almacen,
        &RelojFijo(t0() + TimeDelta::days(366)),
    )
    .await
    .expect("retención");
    assert_eq!(r.soportes_purgados, 1);
    assert!(e.almacen.claves().is_empty());
    let r = aplicar_retencion(
        &e.directorio(),
        &e.unidad,
        &e.almacen,
        &RelojFijo(t0() + TimeDelta::days(367)),
    )
    .await
    .expect("retención");
    assert_eq!(r.soportes_purgados, 0, "no se purga dos veces");
}

#[tokio::test]
async fn el_listado_para_entregar_trae_adultos_y_propietarios() {
    let e = entorno();

    let v = corr::personas_para_entrega(&e.unidad, &RelojFijo(t0()), &e.porteria(), e.bien)
        .await
        .expect("listado");

    let ids: Vec<_> = v.iter().map(|p| (p.persona_id, p.vinculo)).collect();
    assert_eq!(
        ids,
        vec![
            (e.responsable, "residente"),
            (e.tenedora, "residente"),
            (e.dueno, "propietario")
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// Portería: configuración, minuta y costo
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn la_configuracion_se_valida_y_la_edita_la_administracion() {
    let e = entorno();
    let mala = ConfiguracionAcceso {
        retencion_soportes_dias: 5,
        ..ConfiguracionAcceso::default()
    };
    assert!(matches!(
        porteria::guardar_configuracion(&e.unidad, &RelojFijo(t0()), &e.admin(), mala).await,
        Err(AppError::Dominio(_))
    ));
    let buena = ConfiguracionAcceso {
        retencion_soportes_dias: 180,
        ..ConfiguracionAcceso::default()
    };
    assert!(matches!(
        porteria::guardar_configuracion(&e.unidad, &RelojFijo(t0()), &e.porteria(), buena).await,
        Err(AppError::SinPermiso(Permiso::AccesoConfigurar))
    ));
    porteria::guardar_configuracion(&e.unidad, &RelojFijo(t0()), &e.admin(), buena)
        .await
        .expect("guardar");
    assert_eq!(
        porteria::ver_configuracion(&e.unidad, &RelojFijo(t0()), &e.porteria())
            .await
            .expect("ver")
            .retencion_soportes_dias,
        180
    );
}

#[tokio::test]
async fn la_minuta_guarda_novedades_con_adjuntos() {
    let e = entorno();
    let n = porteria::NuevaNovedad {
        tipo: domain::TipoNovedadMinuta::Ronda,
        texto: "Ronda sin novedad".into(),
        punto_acceso_id: None,
        adjuntos: vec![PNG.to_vec()],
    };

    let x = porteria::registrar_novedad(&e.unidad, &e.almacen, &RelojFijo(t0()), &e.porteria(), n)
        .await
        .expect("minuta");

    assert_eq!(x.adjuntos.len(), 1);
    let p = porteria::consultar_minuta(
        &e.unidad,
        &RelojFijo(t0()),
        &e.porteria(),
        Default::default(),
        SolicitudPagina::default(),
    )
    .await
    .expect("minuta");
    assert_eq!(p.datos.len(), 1);
    let r = porteria::consultar_minuta(
        &e.unidad,
        &RelojFijo(t0()),
        &e.residente(e.cuenta_responsable),
        Default::default(),
        SolicitudPagina::default(),
    )
    .await;
    assert!(matches!(
        r,
        Err(AppError::SinPermiso(Permiso::AccesoMinutaVer))
    ));
}

#[tokio::test]
async fn el_costo_cuenta_solo_las_plantillas_de_whatsapp() {
    let e = entorno();
    e.anunciar().await;
    escalar_avisos(
        &e.directorio(),
        &e.unidad,
        &RelojFijo(t0() + TimeDelta::minutes(2)),
    )
    .await
    .expect("escalar");

    let c = porteria::costo_avisos(
        &e.unidad,
        &RelojFijo(t0()),
        &e.admin(),
        t0(),
        t0() + TimeDelta::days(1),
        "COP",
    )
    .await
    .expect("costo");

    assert_eq!((c.conteo.push_enviados, c.conteo.whatsapp_enviados), (2, 1));
}

// ---------------------------------------------------------------------------------------------
// Credenciales, autorizaciones y biometría
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn una_tarjeta_se_guarda_por_huella_y_autoriza_a_un_residente() {
    let e = entorno();
    let admin = e.admin();
    let punto = PuntoAccesoId::nuevo();
    let emitida = cred::emitir_credencial(
        &e.unidad,
        &RelojFijo(t0()),
        &admin,
        NuevaCredencial {
            titular: TitularAcceso::Persona(e.responsable),
            tipo: TipoCredencial::TarjetaRfid,
            valor: Some("0A:1B:2C:3D".into()),
            vigente_desde: None,
            vigente_hasta: None,
        },
    )
    .await
    .expect("emitir");
    assert!(!emitida.credencial.huella.contains("0A1B2C3D"));
    assert!(emitida.secreto.is_none());

    let v = cred::verificar_en_porteria(
        &e.unidad,
        &RelojFijo(t0()),
        &FirmaFalsa,
        &e.porteria(),
        "0a1b2c3d",
        TipoCredencial::TarjetaRfid,
        punto,
    )
    .await
    .expect("verificar");
    assert_eq!(
        v.resultado,
        ResultadoAcceso::Permitido,
        "residente vigente: autorización derivada"
    );

    cred::cambiar_estado_credencial(
        &e.unidad,
        &RelojFijo(t0()),
        &admin,
        emitida.credencial.id,
        EstadoCredencial::Revocada,
    )
    .await
    .expect("revocar");
    let v = cred::verificar_en_porteria(
        &e.unidad,
        &RelojFijo(t0()),
        &FirmaFalsa,
        &e.porteria(),
        "0a1b2c3d",
        TipoCredencial::TarjetaRfid,
        punto,
    )
    .await
    .expect("verificar");
    assert_eq!(v.resultado, ResultadoAcceso::Denegado);
    assert_eq!(
        e.unidad.datos().acceso.eventos.len(),
        2,
        "cada intento queda en la bitácora"
    );
}

#[tokio::test]
async fn terminar_la_residencia_retira_la_autorizacion_derivada() {
    let e = entorno();
    let admin = e.admin();
    cred::emitir_credencial(
        &e.unidad,
        &RelojFijo(t0()),
        &admin,
        NuevaCredencial {
            titular: TitularAcceso::Persona(e.tenedora),
            tipo: TipoCredencial::Pin,
            valor: Some("4321".into()),
            vigente_desde: None,
            vigente_hasta: None,
        },
    )
    .await
    .expect("emitir");
    let t = e.tenedora;
    e.unidad.con_datos(|d| {
        for r in d.residencias.iter_mut().filter(|r| r.persona_id == t) {
            r.vigente_hasta = Some(t0() - TimeDelta::hours(1));
        }
    });

    let v = cred::verificar_en_porteria(
        &e.unidad,
        &RelojFijo(t0()),
        &FirmaFalsa,
        &e.porteria(),
        "4321",
        TipoCredencial::Pin,
        PuntoAccesoId::nuevo(),
    )
    .await
    .expect("verificar");

    assert_eq!(v.resultado, ResultadoAcceso::Denegado);
}

#[tokio::test]
async fn no_hay_dos_credenciales_vigentes_con_el_mismo_identificador() {
    let e = entorno();
    let n = NuevaCredencial {
        titular: TitularAcceso::Persona(e.responsable),
        tipo: TipoCredencial::Pin,
        valor: Some("1234".into()),
        vigente_desde: None,
        vigente_hasta: None,
    };
    cred::emitir_credencial(&e.unidad, &RelojFijo(t0()), &e.admin(), n.clone())
        .await
        .expect("emitir");

    let r = cred::emitir_credencial(&e.unidad, &RelojFijo(t0()), &e.admin(), n).await;

    assert!(matches!(r, Err(AppError::Conflicto(_))));
}

#[tokio::test]
async fn los_qr_permanentes_se_entregan_una_vez_y_se_rotan() {
    let e = entorno();
    let emitida = cred::emitir_credencial(
        &e.unidad,
        &RelojFijo(t0()),
        &e.admin(),
        NuevaCredencial {
            titular: TitularAcceso::Persona(e.responsable),
            tipo: TipoCredencial::QrPermanente,
            valor: None,
            vigente_desde: None,
            vigente_hasta: None,
        },
    )
    .await
    .expect("emitir");
    let secreto = emitida.secreto.expect("texto del QR");

    let nueva = cred::rotar_qr(
        &e.unidad,
        &RelojFijo(t0()),
        &e.admin(),
        emitida.credencial.id,
    )
    .await
    .expect("rotar");

    assert_ne!(nueva.secreto.as_deref(), Some(secreto.as_str()));
    let v = cred::verificar_en_porteria(
        &e.unidad,
        &RelojFijo(t0()),
        &FirmaFalsa,
        &e.porteria(),
        &secreto,
        TipoCredencial::QrPermanente,
        PuntoAccesoId::nuevo(),
    )
    .await
    .expect("verificar");
    assert_eq!(
        v.resultado,
        ResultadoAcceso::Denegado,
        "el QR anterior quedó revocado"
    );
}

#[tokio::test]
async fn la_biometria_exige_consentimiento_y_revocarlo_borra_la_plantilla() {
    let e = entorno();
    let admin = e.admin();
    let n = NuevaCredencial {
        titular: TitularAcceso::Persona(e.responsable),
        tipo: TipoCredencial::BiometriaRef,
        valor: Some("enrol-77".into()),
        vigente_desde: None,
        vigente_hasta: None,
    };
    let r = cred::emitir_credencial(&e.unidad, &RelojFijo(t0()), &admin, n.clone()).await;
    assert!(
        matches!(r, Err(AppError::Validacion(_))),
        "sin consentimiento"
    );

    cred::registrar_consentimiento(
        &e.unidad,
        &e.almacen,
        &RelojFijo(t0()),
        &admin,
        NuevoConsentimiento {
            persona_id: e.responsable,
            version_texto: "v1".into(),
            soporte: None,
        },
    )
    .await
    .expect("consentimiento");
    let c = cred::emitir_credencial(&e.unidad, &RelojFijo(t0()), &admin, n)
        .await
        .expect("emitir");

    let propia = e.residente(e.cuenta_responsable);
    let revocadas =
        cred::revocar_consentimiento(&e.unidad, &RelojFijo(t0()), &propia, e.responsable)
            .await
            .expect("revocar");
    assert_eq!(revocadas, 1);
    let cambios = {
        let mut tx = crate::puertos::UnidadDeTrabajo::iniciar(&e.unidad, &e.sistema())
            .await
            .expect("tx");
        crate::puertos::RepositorioAcceso::cambios_desde(&mut tx, 0, 100, t0())
            .await
            .expect("cambios")
    };
    let s = cambios
        .credenciales
        .iter()
        .find(|x| x.id == c.credencial.id)
        .expect("sincronizada");
    assert!(!s.activa && s.borrar_plantilla);
}

// ---------------------------------------------------------------------------------------------
// Puentes
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn un_puente_se_autentica_sincroniza_y_reporta_sin_duplicar() {
    let e = entorno();
    let admin = e.admin();
    let punto = puentes::guardar_punto(
        &e.unidad,
        &RelojFijo(t0()),
        &admin,
        None,
        puentes::DatosPunto {
            nombre: "Portería principal".into(),
            tipo: TipoPuntoAcceso::Peatonal,
            sentido: domain::SentidoAcceso::Ambos,
            recurso_id: None,
            activo: true,
        },
    )
    .await
    .expect("punto");
    let (puente, token) = puentes::registrar_puente(&e.unidad, &RelojFijo(t0()), &admin, "Mini PC")
        .await
        .expect("puente");
    let directorio = Directorio {
        tenant: e.tenant,
        puentes: vec![(puente.credencial_hash.clone(), puente.id)],
    };
    assert!(
        puentes::autenticar_puente(&directorio, "token-falso-que-no-existe-0000000000", None)
            .await
            .is_err()
    );
    let p = puentes::autenticar_puente(&directorio, &token, None)
        .await
        .expect("autenticar");

    let s = puentes::sincronizar(
        &e.unidad,
        &RelojFijo(t0()),
        &FirmaFalsa,
        &p,
        0,
        Some("0.1.0"),
    )
    .await
    .expect("sincronizar");
    assert_eq!(s.puntos.len(), 1);
    assert_eq!(s.clave_publica, [9; 32]);

    let evento = || puentes::EventoReportado {
        id_local: "ev-1".into(),
        dispositivo_id: None,
        punto_acceso_id: Some(punto.id),
        credencial_id: None,
        visita_id: None,
        sentido: SentidoAcceso::Entrada,
        resultado: ResultadoAcceso::Permitido,
        motivo: None,
        placa: Some("abc123".into()),
        foto: Some(PNG.to_vec()),
        ocurrido_en: t0(),
    };
    let r = puentes::reportar_eventos(
        &e.unidad,
        &e.almacen,
        &RelojFijo(t0()),
        &p,
        vec![evento()],
        vec![],
    )
    .await
    .expect("reportar");
    assert_eq!((r.nuevos, r.repetidos), (1, 0));
    let r = puentes::reportar_eventos(
        &e.unidad,
        &e.almacen,
        &RelojFijo(t0()),
        &p,
        vec![evento()],
        vec![],
    )
    .await
    .expect("reportar");
    assert_eq!((r.nuevos, r.repetidos), (0, 1));
    assert_eq!(
        e.almacen.claves().len(),
        1,
        "la foto del reintento no queda huérfana"
    );

    let r = aplicar_retencion(
        &e.directorio(),
        &e.unidad,
        &e.almacen,
        &RelojFijo(t0() + TimeDelta::days(31)),
    )
    .await
    .expect("retención");
    assert_eq!(r.fotos_purgadas, 1);
    assert!(e.almacen.claves().is_empty(), "fotos de placas: 30 días");
}

#[tokio::test]
async fn la_configuracion_de_un_dispositivo_no_lleva_secretos() {
    let e = entorno();
    let admin = e.admin();
    let punto = puentes::guardar_punto(
        &e.unidad,
        &RelojFijo(t0()),
        &admin,
        None,
        puentes::DatosPunto {
            nombre: "Vehicular".into(),
            tipo: TipoPuntoAcceso::Vehicular,
            sentido: domain::SentidoAcceso::Entrada,
            recurso_id: None,
            activo: true,
        },
    )
    .await
    .expect("punto");
    let (puente, _) = puentes::registrar_puente(&e.unidad, &RelojFijo(t0()), &admin, "Mini PC")
        .await
        .expect("puente");
    let datos = |config| puentes::DatosDispositivo {
        puente_id: puente.id,
        punto_acceso_id: punto.id,
        tipo: domain::TipoDispositivoAcceso::CamaraLpr,
        fabricante: "Hikvision".into(),
        modelo: None,
        config,
        activo: true,
    };

    let r = puentes::guardar_dispositivo(
        &e.unidad,
        &RelojFijo(t0()),
        &admin,
        None,
        datos(serde_json::json!({"ip": "10.0.0.5", "auth": {"Password": "x"}})),
    )
    .await;
    assert!(matches!(r, Err(AppError::Validacion(_))));
    puentes::guardar_dispositivo(
        &e.unidad,
        &RelojFijo(t0()),
        &admin,
        None,
        datos(serde_json::json!({"ip": "10.0.0.5", "canal": 1})),
    )
    .await
    .expect("dispositivo");
}

#[tokio::test]
async fn un_puente_retirado_ya_no_sincroniza() {
    let e = entorno();
    let admin = e.admin();
    let (puente, token) = puentes::registrar_puente(&e.unidad, &RelojFijo(t0()), &admin, "Mini PC")
        .await
        .expect("puente");
    let directorio = Directorio {
        tenant: e.tenant,
        puentes: vec![(puente.credencial_hash.clone(), puente.id)],
    };
    let p = puentes::autenticar_puente(&directorio, &token, None)
        .await
        .expect("autenticar");

    puentes::retirar_puente(&e.unidad, &RelojFijo(t0()), &admin, puente.id)
        .await
        .expect("retirar");

    let r = puentes::sincronizar(&e.unidad, &RelojFijo(t0()), &FirmaFalsa, &p, 0, None).await;
    assert!(matches!(r, Err(AppError::NoAutenticado)));
}

/// Tablet de la portería con la app: cuenta con `acceso.registrar` y un token push.
fn con_tablet(e: &Entorno) -> String {
    let cuenta = CuentaId::nuevo();
    e.unidad.con_datos(|d| {
        d.gestion.destinatarios.push((
            Permiso::AccesoRegistrar,
            crate::puertos::tickets::Destinatario {
                cuenta_id: cuenta,
                correo: None,
                telefono: None,
            },
        ));
        d.acceso.tokens.push((cuenta, "tablet-1".into()));
    });
    "tablet-1".into()
}

fn push_a(e: &Entorno, token: &str) -> Vec<crate::puertos::acceso::NotificacionPush> {
    e.unidad
        .outbox()
        .into_iter()
        .filter_map(|(_, ev)| match ev.mensaje {
            MensajeOutbox::Push(p) if p.token == token => Some(p),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn la_porteria_recibe_push_cuando_hay_que_llamar() {
    let e = entorno();
    let tablet = con_tablet(&e);
    e.unidad.con_datos(|d| {
        d.residencias.clear();
        d.titularidades.clear();
    });

    let v = e.anunciar().await;

    assert_eq!(v.estado, EstadoVisita::RequiereLlamada);
    let p = push_a(&e, &tablet);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].datos["tipo"], "porteria_visita");
    assert_eq!(p[0].datos["estado"], "requiere_llamada");
    assert!(
        p[0].respaldo.is_none(),
        "la portería usa la app, sin WhatsApp"
    );
}

#[tokio::test]
async fn la_porteria_recibe_push_cuando_un_residente_decide() {
    let e = entorno();
    let tablet = con_tablet(&e);
    let v = e.anunciar().await;
    assert!(
        push_a(&e, &tablet).is_empty(),
        "anunciar no avisa a la portería"
    );

    visitas::decidir_visita(
        &e.unidad,
        &RelojFijo(t0() + TimeDelta::seconds(30)),
        &e.residente(e.cuenta_responsable),
        v.id,
        false,
        CanalRespuestaVisita::App,
    )
    .await
    .expect("decidir");

    let p = push_a(&e, &tablet);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].datos["estado"], "rechazada");
    assert!(p[0].titulo.starts_with("Visita rechazada"));
}
