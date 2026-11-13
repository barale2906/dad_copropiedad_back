//! Pruebas del asistente de WhatsApp con dobles en memoria: sesiones, directorio de teléfonos,
//! mensajería (descarga de fotos) y almacén de archivos.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use domain::{
    Almacen, BienPrivadoId, CuentaId, EstadoTicket, Modulo, PersonaId, TenantId, TipoPersona,
};

use super::*;
use crate::{
    casos_uso::conversaciones::{cerrar_conversacion, responder_conversacion},
    puertos::{
        archivos::memoria::AlmacenEnMemoria, chat::EstadoConversacion, copropiedades::Persona,
        mensajeria::MediaDescargada, reloj::RelojFijo, unidad_trabajo::memoria::UnidadEnMemoria,
    },
};

const LINEA: &str = "PNID";
const TELEFONO: &str = "573001112233";
/// PNG mínimo (firma + IHDR) para pasar la validación por contenido.
const PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0,
    1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
];

fn ahora() -> DateTime<Utc> {
    domain::reserva::Tz::America__Bogota
        .with_ymd_and_hms(2026, 11, 4, 10, 0, 0)
        .single()
        .expect("fecha")
        .with_timezone(&Utc)
}

#[derive(Default, Clone)]
struct SesionesEnMemoria {
    sesion: Arc<Mutex<Option<SesionGuardada>>>,
    procesados: Arc<Mutex<Vec<String>>>,
    respuestas: Arc<Mutex<Vec<MensajeSaliente>>>,
}

impl SesionesConversacion for SesionesEnMemoria {
    async fn ya_procesado(&self, wamid: &str) -> AppResult<bool> {
        Ok(self
            .procesados
            .lock()
            .expect("mutex")
            .iter()
            .any(|w| w == wamid))
    }

    async fn marcar_procesado(&self, wamid: &str) -> AppResult<bool> {
        let mut p = self.procesados.lock().expect("mutex");
        if p.iter().any(|w| w == wamid) {
            return Ok(false);
        }
        p.push(wamid.to_owned());
        Ok(true)
    }

    async fn obtener(
        &self,
        _linea: &str,
        _telefono: &str,
        _ahora: DateTime<Utc>,
    ) -> AppResult<Option<SesionGuardada>> {
        Ok(self.sesion.lock().expect("mutex").clone())
    }

    async fn cerrar_turno(&self, c: &CierreTurno) -> AppResult<bool> {
        let mut p = self.procesados.lock().expect("mutex");
        if p.contains(&c.wamid) {
            return Ok(false);
        }
        p.push(c.wamid.clone());
        *self.sesion.lock().expect("mutex") = c.sesion.clone();
        self.respuestas
            .lock()
            .expect("mutex")
            .extend(c.respuestas.iter().cloned());
        Ok(true)
    }
}

struct Directorio(Vec<VinculoTelefono>);

impl DirectorioTelefonos for Directorio {
    async fn vinculos_por_telefono(&self, telefono: &str) -> AppResult<Vec<VinculoTelefono>> {
        Ok(if telefono == TELEFONO {
            self.0.clone()
        } else {
            Vec::new()
        })
    }
}

struct MetaFalsa;

impl Mensajeria for MetaFalsa {
    async fn enviar(&self, _m: &MensajeSaliente) -> AppResult<String> {
        Ok("wamid.salida".into())
    }

    async fn descargar_media(&self, _media_id: &str) -> AppResult<MediaDescargada> {
        Ok(MediaDescargada {
            contenido: PNG.to_vec(),
            mime: Some("image/png".into()),
        })
    }
}

/// Asistencias simuladas por teléfono (opción «Votar»).
#[derive(Default)]
struct Asambleas(Mutex<Vec<crate::puertos::asambleas::AsistenciaTelefono>>);

impl DirectorioAsambleas for Asambleas {
    async fn asistencias_por_telefono(
        &self,
        _telefono: &str,
    ) -> AppResult<Vec<crate::puertos::asambleas::AsistenciaTelefono>> {
        Ok(self.0.lock().expect("mutex").clone())
    }

    async fn copropiedades_con_asambleas_pendientes(&self) -> AppResult<Vec<TenantId>> {
        Ok(Vec::new())
    }

    async fn registrar_acuse(&self, _: &crate::puertos::asambleas::AcuseEntrega) -> AppResult<()> {
        Ok(())
    }

    async fn actualizar_calidad_linea(&self, _: &str, _: bool, _: Option<&str>) -> AppResult<()> {
        Ok(())
    }
}

/// Imagen de prueba: el PNG mínimo.
struct QrFalso;

impl GeneradorImagenQr for QrFalso {
    fn png(&self, _texto: &str) -> AppResult<Vec<u8>> {
        Ok(PNG.to_vec())
    }
}

/// Firma de prueba (64 bytes deterministas).
struct FirmaFalsa;

impl FirmaQr for FirmaFalsa {
    fn firmar(&self, _tenant: TenantId, carga: &[u8]) -> Vec<u8> {
        let mut f = carga.to_vec();
        f.resize(64, 7);
        f
    }

    fn verificar(&self, tenant: TenantId, carga: &[u8], firma: &[u8]) -> bool {
        self.firmar(tenant, carga) == firma
    }

    fn clave_publica(&self, _tenant: TenantId) -> [u8; 32] {
        [0; 32]
    }
}

struct Entorno {
    asambleas: Asambleas,
    unidad: UnidadEnMemoria,
    sesiones: SesionesEnMemoria,
    directorio: Directorio,
    almacen: AlmacenEnMemoria,
    tenant: TenantId,
    persona: PersonaId,
    n: Mutex<u32>,
}

fn vinculo(tenant: TenantId, persona: PersonaId, codigo: &str) -> VinculoTelefono {
    VinculoTelefono {
        tenant_id: tenant,
        copropiedad: "Conjunto Alameda".into(),
        persona_id: persona,
        cuenta_id: None,
        bien_privado_id: BienPrivadoId::nuevo(),
        codigo: codigo.into(),
        es_propietario: true,
    }
}

fn entorno(vinculos: usize) -> Entorno {
    let unidad = UnidadEnMemoria::default();
    unidad.habilitar(Modulo::GestionAdministrativa);
    let tenant = TenantId::nuevo();
    let persona = PersonaId::nuevo();
    unidad.con_datos(|d| {
        d.personas.push(Persona::basica(
            persona,
            TipoPersona::Natural,
            "Ana".into(),
            None,
        ));
    });
    let directorio = Directorio(
        (0..vinculos)
            .map(|i| vinculo(tenant, persona, &format!("T1-10{i}")))
            .collect(),
    );
    Entorno {
        asambleas: Asambleas::default(),
        unidad,
        sesiones: SesionesEnMemoria::default(),
        directorio,
        almacen: AlmacenEnMemoria::new(Almacen::Local),
        tenant,
        persona,
        n: Mutex::new(0),
    }
}

impl Entorno {
    async fn enviar(&self, contenido: ContenidoEntrante) -> Vec<ContenidoSaliente> {
        let wamid = {
            let mut n = self.n.lock().expect("mutex");
            *n += 1;
            format!("wamid.{n}")
        };
        self.enviar_con(&wamid, TELEFONO, contenido).await
    }

    async fn enviar_con(
        &self,
        wamid: &str,
        telefono: &str,
        contenido: ContenidoEntrante,
    ) -> Vec<ContenidoSaliente> {
        let antes = self.sesiones.respuestas.lock().expect("mutex").len();
        let s = ServiciosChat {
            unidad: &self.unidad,
            sesiones: &self.sesiones,
            directorio: &self.directorio,
            mensajeria: &MetaFalsa,
            almacen: &self.almacen,
            reloj: &RelojFijo(ahora()),
            asambleas: &self.asambleas,
            firma: &FirmaFalsa,
            qr: &QrFalso,
        };
        atender_mensaje(
            &s,
            MensajeEntrante {
                linea: LINEA.into(),
                telefono: telefono.into(),
                wamid: wamid.into(),
                recibido_en: ahora(),
                contenido,
            },
        )
        .await
        .expect("turno");
        self.sesiones.respuestas.lock().expect("mutex")[antes..]
            .iter()
            .map(|m| m.contenido.clone())
            .collect()
    }

    fn paso(&self) -> Option<Paso> {
        self.sesiones
            .sesion
            .lock()
            .expect("mutex")
            .as_ref()
            .and_then(|s| serde_json::from_value::<EstadoChat>(s.estado.clone()).ok())
            .map(|e| e.paso)
    }
}

fn texto(t: &str) -> ContenidoEntrante {
    ContenidoEntrante::Texto { cuerpo: t.into() }
}

fn elige(id: &str) -> ContenidoEntrante {
    ContenidoEntrante::Respuesta {
        id: id.into(),
        titulo: id.into(),
    }
}

fn ids(c: &ContenidoSaliente) -> Vec<String> {
    match c {
        ContenidoSaliente::Lista { filas, .. } => filas.iter().map(|f| f.id.clone()).collect(),
        ContenidoSaliente::Botones { botones, .. } => {
            botones.iter().map(|b| b.id.clone()).collect()
        }
        _ => Vec::new(),
    }
}

#[tokio::test]
async fn un_numero_desconocido_recibe_ayuda_y_no_deja_sesion() {
    let e = entorno(1);
    let r = e.enviar_con("w1", "573009999999", texto("hola")).await;
    assert!(
        matches!(&r[..], [ContenidoSaliente::Texto { cuerpo }] if cuerpo.contains("no está registrado"))
    );
    assert!(e.sesiones.sesion.lock().expect("mutex").is_none());
}

#[tokio::test]
async fn con_un_vinculo_muestra_el_menu() {
    let e = entorno(1);
    let r = e.enviar(texto("Hola")).await;
    assert_eq!(r.len(), 1);
    let opciones = ids(&r[0]);
    assert!(opciones.contains(&"m:reportar".to_owned()));
    assert!(!opciones.contains(&"m:cambiar".to_owned()));
    assert_eq!(e.paso(), Some(Paso::Menu));
}

#[tokio::test]
async fn con_varios_vinculos_pregunta_por_cual_escribe() {
    let e = entorno(2);
    let r = e.enviar(texto("hola")).await;
    assert_eq!(ids(&r[0]), vec!["v:0", "v:1"]);
    assert_eq!(e.paso(), Some(Paso::ElegirVinculo));
    let r = e.enviar(elige("v:1")).await;
    assert!(ids(&r[0]).contains(&"m:cambiar".to_owned()));
    assert_eq!(e.paso(), Some(Paso::Menu));
}

#[tokio::test]
async fn reportar_un_dano_con_foto_radica_un_ticket_una_sola_vez() {
    let e = entorno(1);
    e.enviar(texto("hola")).await;
    let r = e.enviar(elige("m:reportar")).await;
    assert!(ids(&r[0]).contains(&"c:otro".to_owned()));
    e.enviar(elige("c:otro")).await;
    // Un botón donde se espera texto vuelve a pedir la descripción.
    e.enviar(elige("algo")).await;
    assert!(matches!(e.paso(), Some(Paso::ReporteDescripcion { .. })));
    let r = e.enviar(texto("Fuga en el techo del baño")).await;
    assert_eq!(ids(&r[0]), vec![ID_SIN_FOTO]);
    e.enviar(ContenidoEntrante::Imagen {
        media_id: "m1".into(),
        leyenda: None,
    })
    .await;
    assert!(matches!(
        e.paso(),
        Some(Paso::ReporteUrgente { foto: Some(_), .. })
    ));

    let r = e.enviar_con("wamid.final", TELEFONO, elige(ID_SI)).await;
    assert!(
        matches!(&r[0], ContenidoSaliente::Texto { cuerpo } if cuerpo.contains("PQR-2026-00001") && cuerpo.contains("de inmediato"))
    );
    // Meta reintenta el mismo webhook: no hay un segundo ticket ni respuesta.
    let reintento = e.enviar_con("wamid.final", TELEFONO, elige(ID_SI)).await;
    assert!(reintento.is_empty());

    let datos = e.unidad.datos();
    assert_eq!(datos.gestion.tickets.len(), 1);
    let t = &datos.gestion.tickets[0];
    assert_eq!(
        (t.canal, t.prioridad, t.estado, t.solicitante_persona_id),
        (
            CanalTicket::Whatsapp,
            PrioridadTicket::Urgente,
            EstadoTicket::Abierto,
            Some(e.persona)
        )
    );
    assert_eq!(t.idempotency_key, "wamid.final");
    assert_eq!(datos.gestion.adjuntos.len(), 1);
    assert_eq!(e.almacen.claves().len(), 1);
    assert_eq!(e.paso(), Some(Paso::Menu));
}

#[tokio::test]
async fn mis_solicitudes_lista_solo_las_propias() {
    let e = entorno(1);
    e.enviar(texto("hola")).await;
    let r = e.enviar(elige("m:estado")).await;
    assert!(matches!(&r[0], ContenidoSaliente::Texto { cuerpo } if cuerpo.contains("No tiene")));
}

#[tokio::test]
async fn las_preguntas_frecuentes_responden_desde_la_lista() {
    let e = entorno(1);
    let id = domain::PreguntaFrecuenteId::nuevo();
    e.unidad.con_datos(|d| {
        d.gestion
            .preguntas
            .push(crate::puertos::chat::PreguntaFrecuente {
                id,
                pregunta: "Horario de la piscina".into(),
                respuesta: "De martes a domingo, 8:00 a 18:00".into(),
                orden: 0,
                activa: true,
                archivo_id: None,
            });
    });
    e.enviar(texto("hola")).await;
    let r = e.enviar(elige("m:info")).await;
    assert!(ids(&r[0]).contains(&format!("f:{id}")));
    let r = e.enviar(elige(&format!("f:{id}"))).await;
    assert!(matches!(&r[0], ContenidoSaliente::Texto { cuerpo } if cuerpo.contains("martes")));
}

#[tokio::test]
async fn reservar_sin_cuenta_explica_que_debe_registrarse() {
    let e = entorno(1);
    e.enviar(texto("hola")).await;
    let r = e.enviar(elige("m:reservar")).await;
    assert!(
        matches!(&r[0], ContenidoSaliente::Texto { cuerpo } if cuerpo.contains("cuenta en la plataforma")),
        "{r:?}"
    );
    assert_eq!(e.paso(), Some(Paso::Menu));
}

#[tokio::test]
async fn la_bandeja_humana_recibe_los_mensajes_y_menu_sale_de_ella() {
    let e = entorno(1);
    e.enviar(texto("hola")).await;
    let r = e.enviar(elige("m:humano")).await;
    assert_eq!(e.paso(), Some(Paso::Contacto));
    assert!(
        ids(&r[0]).contains(&"h:cita".to_owned()),
        "la cita está dentro del contacto"
    );
    e.enviar(elige("h:mensaje")).await;
    assert!(matches!(e.paso(), Some(Paso::Humano { .. })));
    let r = e.enviar(texto("Necesito un paz y salvo")).await;
    assert!(r.is_empty(), "en modo humano el chat no responde solo");

    let datos = e.unidad.datos();
    assert_eq!(datos.gestion.conversaciones.len(), 1);
    let c = datos.gestion.conversaciones[0].clone();
    assert_eq!(c.telefono, TELEFONO);
    assert_eq!(datos.gestion.mensajes_conversacion.len(), 1);

    // La administración responde desde la web por el outbox.
    let admin = Contexto::de_cuenta(
        e.tenant,
        CuentaId::nuevo(),
        [domain::Permiso::ConversacionesAtender]
            .into_iter()
            .collect(),
    );
    responder_conversacion(
        &e.unidad,
        &RelojFijo(ahora()),
        &admin,
        c.id,
        "Con gusto".into(),
    )
    .await
    .expect("responde");
    assert!(e.unidad.outbox().iter().any(|(_, ev)| matches!(
        &ev.mensaje,
        crate::outbox::MensajeOutbox::WhatsApp(m) if m.telefono == TELEFONO
    )));

    // Al cerrarla, el siguiente mensaje vuelve al menú.
    let cerrada = cerrar_conversacion(&e.unidad, &RelojFijo(ahora()), &admin, c.id)
        .await
        .expect("cierra");
    assert_eq!(cerrada.estado, EstadoConversacion::Cerrada);
    let r = e.enviar(texto("gracias")).await;
    assert!(ids(&r[0]).contains(&"m:reportar".to_owned()));
    assert_eq!(e.paso(), Some(Paso::Menu));
}

#[tokio::test]
async fn fuera_de_la_ventana_de_24_horas_no_se_responde_con_texto_libre() {
    let e = entorno(1);
    e.enviar(texto("hola")).await;
    e.enviar(elige("m:humano")).await;
    e.enviar(elige("h:mensaje")).await;
    let c = e.unidad.datos().gestion.conversaciones[0].id;
    let admin = Contexto::de_cuenta(
        e.tenant,
        CuentaId::nuevo(),
        [domain::Permiso::ConversacionesAtender]
            .into_iter()
            .collect(),
    );
    let r = responder_conversacion(
        &e.unidad,
        &RelojFijo(ahora() + chrono::Duration::hours(25)),
        &admin,
        c,
        "tarde".into(),
    )
    .await;
    assert!(matches!(r, Err(AppError::Conflicto(_))));
}

#[tokio::test]
async fn mi_saldo_reemplaza_la_cita_en_el_menu_y_sin_cartera_lo_dice() {
    let e = entorno(1);
    let r = e.enviar(texto("hola")).await;
    let opciones = ids(&r[0]);
    assert!(opciones.contains(&"m:saldo".to_owned()));
    assert!(!opciones.contains(&"m:cita".to_owned()));
    assert!(
        opciones.contains(&"m:info".to_owned()),
        "no se pierde ninguna opción"
    );

    let r = e.enviar(elige("m:saldo")).await;

    assert!(
        matches!(&r[0], ContenidoSaliente::Texto { cuerpo } if cuerpo.contains("no está disponible")),
        "{r:?}"
    );
    assert_eq!(e.paso(), Some(Paso::Menu));
}

#[test]
fn menu_y_comandos_reconocidos() {
    assert!(pide_menu(&texto("  Menú ")));
    assert!(pide_menu(&elige(ID_MENU)));
    assert!(!pide_menu(&texto("menudo problema")));
    assert_eq!(corto("abcdef", 4), "abc…");
}

/// Entorno con control de acceso: la persona del chat reside en el bien de su vínculo.
fn entorno_con_acceso() -> Entorno {
    let e = entorno(1);
    e.unidad.habilitar(Modulo::ControlAcceso);
    let (persona, bien) = (e.persona, e.directorio.0[0].bien_privado_id);
    e.unidad.con_datos(|d| {
        d.bienes.insert(
            bien,
            crate::puertos::copropiedades::BienPrivado {
                id: bien,
                codigo: domain::CodigoBienPrivado::new("T1-100").expect("código"),
                tipo: domain::TipoBienPrivado::Apartamento,
                agrupacion_id: None,
                numero: None,
                piso: None,
                area_privada_m2: None,
                matricula_inmobiliaria: None,
                cedula_catastral: None,
                uso: domain::UsoBienPrivado::Residencial,
                activo: true,
                bien_principal_id: None,
                coeficiente: None,
            },
        );
        d.residencias.push(crate::puertos::censo::Residencia {
            id: domain::ResidenciaId::nuevo(),
            bien_privado_id: bien,
            persona_id: persona,
            calidad: domain::CalidadResidencia::Familiar,
            es_responsable: true,
            autorizado_por: None,
            contrato_archivo_id: None,
            vigente_desde: ahora() - Duration::days(30),
            vigente_hasta: None,
            reservas_autorizadas: true,
        });
    });
    e
}

#[tokio::test]
async fn sin_asamblea_el_menu_no_ofrece_votar_y_con_ella_si() {
    let e = entorno(1);
    let menu = e.enviar(texto("hola")).await;
    assert!(!ids(&menu[0]).contains(&"m:votar".to_owned()));
    assert!(ids(&menu[0]).contains(&"m:visita".to_owned()));

    e.asambleas
        .0
        .lock()
        .expect("mutex")
        .push(crate::puertos::asambleas::AsistenciaTelefono {
            tenant_id: e.tenant,
            copropiedad: "Conjunto Alameda".into(),
            reunion_id: domain::ReunionId::nuevo(),
            titulo: "Asamblea ordinaria".into(),
            asistente_id: domain::AsistenteId::nuevo(),
            linea: None,
            confirmado: true,
        });
    let menu = e.enviar(texto("menú")).await;
    assert_eq!(ids(&menu[0])[0], "m:votar");
    assert!(ids(&menu[0]).len() <= ContenidoSaliente::MAX_FILAS_LISTA);
}

#[tokio::test]
async fn votar_sin_preguntas_abiertas_lo_explica() {
    let e = entorno(1);
    e.enviar(texto("hola")).await;

    let r = e.enviar(elige("m:votar")).await;

    assert!(
        matches!(&r[0], ContenidoSaliente::Texto { cuerpo } if cuerpo.contains("No tiene preguntas"))
    );
}

#[tokio::test]
async fn autorizar_una_visita_desde_el_chat_envia_el_qr_sin_cuenta() {
    let e = entorno_con_acceso();
    e.enviar(texto("hola")).await;
    e.enviar(elige("m:visita")).await;
    assert_eq!(e.paso(), Some(Paso::VisitaNombre));

    let dias = e.enviar(texto("Rosa Pérez")).await;
    let hoy = ahora()
        .with_timezone(&domain::reserva::Tz::America__Bogota)
        .date_naive();
    assert!(ids(&dias[0]).contains(&format!("vd:{hoy}")));
    let r = e.enviar(elige(&format!("vd:{hoy}"))).await;

    assert!(
        r.iter().any(
            |c| matches!(c, ContenidoSaliente::Texto { cuerpo } if cuerpo.starts_with("DQ1."))
        )
    );
    assert!(
        r.iter().any(
            |c| matches!(c, ContenidoSaliente::Imagen { url, .. } if url.starts_with("memoria://"))
        ),
        "el QR también va como imagen"
    );
    assert_eq!(e.almacen.claves().len(), 1, "el PNG quedó en el almacén");
    let visitas = e.unidad.datos().acceso.visitas;
    assert_eq!(visitas.len(), 1);
    assert_eq!(visitas[0].visitante_nombre, "Rosa Pérez");
    assert_eq!(visitas[0].decidida_por, Some(e.persona));
    assert_eq!(
        visitas[0].canal_decision,
        Some(domain::CanalRespuestaVisita::Whatsapp)
    );
    assert_eq!(e.paso(), Some(Paso::Menu));
}

#[tokio::test]
async fn quien_no_vive_ni_es_dueno_no_autoriza_visitas() {
    let e = entorno(1);
    e.unidad.habilitar(Modulo::ControlAcceso);
    e.enviar(texto("hola")).await;
    e.enviar(elige("m:visita")).await;
    e.enviar(texto("Rosa Pérez")).await;
    let hoy = ahora()
        .with_timezone(&domain::reserva::Tz::America__Bogota)
        .date_naive();

    let r = e.enviar(elige(&format!("vd:{hoy}"))).await;

    assert!(
        matches!(&r[0], ContenidoSaliente::Texto { cuerpo } if cuerpo.contains("no tiene acceso"))
    );
    assert!(e.unidad.datos().acceso.visitas.is_empty());
}

#[tokio::test]
async fn mis_paquetes_lista_la_correspondencia_pendiente() {
    let e = entorno_con_acceso();
    let bien = e.directorio.0[0].bien_privado_id;
    e.unidad.con_datos(|d| {
        d.acceso
            .correspondencia
            .push(crate::puertos::acceso::Correspondencia {
                id: domain::CorrespondenciaId::nuevo(),
                bien_privado_id: bien,
                tipo: domain::TipoCorrespondencia::Paquete,
                estado: domain::EstadoCorrespondencia::Recibida,
                remitente: Some("Servientrega".into()),
                guia: None,
                ubicacion: None,
                foto_archivo_id: None,
                recibida_por: None,
                recibida_en: ahora(),
                avisos_enviados: 1,
                ultimo_aviso_en: None,
                siguiente_aviso_en: None,
                alerta_administracion_en: None,
                cerrada_en: None,
                motivo_devolucion: None,
            });
    });
    e.enviar(texto("hola")).await;

    let r = e.enviar(elige("m:paquetes")).await;

    assert!(
        matches!(&r[0], ContenidoSaliente::Texto { cuerpo } if cuerpo.contains("Servientrega"))
    );
}

/// Campaña de actualización del censo ya lanzada para el segundo bien del teléfono.
fn con_campana(e: &Entorno) -> crate::casos_uso::campanas_censo::BotonCampana {
    let campana = domain::CampanaCensoId::nuevo();
    e.unidad.con_datos(|d| {
        d.campanas
            .campanas
            .push(crate::puertos::censo::CampanaCenso {
                id: campana,
                creada_por: None,
                creada_en: ahora(),
                enviados: 2,
            });
    });
    crate::casos_uso::campanas_censo::BotonCampana {
        tenant: e.tenant,
        campana,
        bien: e.directorio.0[1].bien_privado_id,
        cambios: false,
    }
}

#[tokio::test]
async fn el_boton_esta_bien_confirma_sin_preguntar_por_cual_bien() {
    let e = entorno(2);
    let b = con_campana(&e);
    let r = e.enviar(elige(&b.id())).await;

    assert!(matches!(&r[0], ContenidoSaliente::Texto { cuerpo } if cuerpo.contains("T1-101")));
    let respuestas = e.unidad.datos().campanas.respuestas;
    assert_eq!(respuestas.len(), 1);
    assert_eq!(respuestas[0].bien, b.bien, "el bien del botón, no otro");
    assert_eq!(
        respuestas[0].respuesta,
        domain::RespuestaCampanaCenso::Confirmado
    );
    assert_eq!(e.paso(), Some(Paso::Menu));
}

#[tokio::test]
async fn hay_cambios_muestra_lo_registrado_y_guarda_el_texto() {
    let e = entorno(2);
    let b = crate::casos_uso::campanas_censo::BotonCampana {
        cambios: true,
        ..con_campana(&e)
    };
    let r = e.enviar(elige(&b.id())).await;
    assert!(matches!(&r[0], ContenidoSaliente::Texto { cuerpo } if cuerpo.contains("Residentes")));
    assert!(matches!(e.paso(), Some(Paso::CensoCambios { .. })));

    e.enviar(texto("Ya no vive Juan.\nCarro nuevo ABC123"))
        .await;
    let respuestas = e.unidad.datos().campanas.respuestas;
    assert_eq!(
        respuestas[0].detalle.as_deref(),
        Some("Ya no vive Juan. Carro nuevo ABC123"),
        "en una sola línea"
    );
    assert_eq!(e.paso(), Some(Paso::Menu));
}

#[tokio::test]
async fn un_boton_de_campana_de_otro_bien_no_se_acepta() {
    let e = entorno(1);
    let b = crate::casos_uso::campanas_censo::BotonCampana {
        bien: BienPrivadoId::nuevo(),
        ..con_campana(&entorno(2))
    };
    e.enviar(elige(&b.id())).await;
    assert!(e.unidad.datos().campanas.respuestas.is_empty());
}

// --- Buscar un proveedor (plan 08, fase 2) ---

const WHATSAPP_PROVEEDOR: &str = "573109998877";

/// Entorno con una costurera en el directorio (con WhatsApp) y el celular de Ana en el censo.
fn entorno_con_directorio() -> (Entorno, domain::AnuncianteId) {
    use crate::puertos::directorio::{Categoria, Contacto, FichaDirectorio, ResumenCalificacion};
    let e = entorno(1);
    let anunciante = domain::AnuncianteId::nuevo();
    let costura = domain::CodigoCategoria::new("costura").expect("válido");
    e.unidad.con_datos(|d| {
        d.directorio.categorias = vec![Categoria {
            codigo: costura.clone(),
            padre: None,
            nombre: "Costura y modistería".into(),
            sinonimos: vec!["costurera".into(), "modista".into()],
            restringida: false,
            activa: true,
        }];
        d.directorio.fichas = vec![FichaDirectorio {
            anunciante_id: anunciante,
            nombre_comercial: "Modistería Rosa".into(),
            descripcion: None,
            categorias: vec![costura],
            contacto: Contacto {
                whatsapp: Some(domain::Telefono::new(WHATSAPP_PROVEEDOR).expect("válido")),
                ..Contacto::default()
            },
            horario: None,
            direccion: None,
            municipio: None,
            destacada: false,
            distancia_km: None,
            calificacion: ResumenCalificacion::default(),
        }];
        d.personas[0].telefono = Some(domain::Telefono::new(TELEFONO).expect("válido"));
    });
    (e, anunciante)
}

fn cuerpo(c: &ContenidoSaliente) -> String {
    match c {
        ContenidoSaliente::Texto { cuerpo }
        | ContenidoSaliente::Botones { cuerpo, .. }
        | ContenidoSaliente::Lista { cuerpo, .. } => cuerpo.clone(),
        _ => String::new(),
    }
}

fn plantillas_en_outbox(e: &Entorno) -> Vec<String> {
    e.unidad
        .outbox()
        .into_iter()
        .filter_map(|(_, ev)| match ev.mensaje {
            crate::outbox::MensajeOutbox::WhatsApp(MensajeSaliente {
                contenido: ContenidoSaliente::Plantilla { nombre, .. },
                ..
            }) => Some(nombre),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn el_menu_ofrece_buscar_un_proveedor() {
    let e = entorno(1);
    let r = e.enviar(texto("hola")).await;
    assert!(ids(&r[0]).contains(&"m:proveedor".to_owned()));
    assert!(ids(&r[0]).len() <= ContenidoSaliente::MAX_FILAS_LISTA);
}

#[tokio::test]
async fn busco_costurera_responde_con_fichas_sin_compartir_el_telefono() {
    let (e, _) = entorno_con_directorio();

    // Lo que trae el enlace de los grupos: sin pasar por el menú.
    let r = e.enviar(texto("Busco costurera")).await;

    assert!(cuerpo(&r[0]).contains("Modistería Rosa"));
    assert!(cuerpo(&r[0]).contains(WHATSAPP_PROVEEDOR));
    assert_eq!(ids(&r[1]), ["p:si", "p:no"]);
    assert!(matches!(e.paso(), Some(Paso::ProveedorContacto { .. })));
    assert!(
        e.unidad.outbox().is_empty(),
        "nada sale hacia el proveedor sin el «Sí»"
    );
}

#[tokio::test]
async fn desde_el_menu_pide_que_busca_y_sin_resultados_deja_intentar_otra_vez() {
    let (e, _) = entorno_con_directorio();
    e.enviar(texto("hola")).await;

    let r = e.enviar(elige("m:proveedor")).await;
    assert!(cuerpo(&r[0]).contains("¿Qué necesita?"));
    let r = e.enviar(texto("un cerrajero")).await;

    assert!(cuerpo(&r[0]).contains("No encontramos"));
    assert_eq!(e.paso(), Some(Paso::ProveedorBusqueda));
}

#[tokio::test]
async fn con_el_si_se_comparte_el_contacto_y_se_programa_el_le_sirvio() {
    let (e, _) = entorno_con_directorio();
    e.enviar(texto("necesito una modista")).await;

    let r = e.enviar(elige("p:si")).await;

    assert!(cuerpo(&r[0]).contains("Modistería Rosa"));
    assert_eq!(
        plantillas_en_outbox(&e),
        ["contacto_proveedor", "opinion_proveedor"]
    );
    assert_eq!(e.paso(), Some(Paso::Menu));
}

#[tokio::test]
async fn con_el_no_no_se_comparte_nada() {
    let (e, _) = entorno_con_directorio();
    e.enviar(texto("busco costurera")).await;

    let r = e.enviar(elige("p:no")).await;

    assert!(cuerpo(&r[0]).contains("no compartimos"));
    assert!(e.unidad.outbox().is_empty());
    assert!(e.unidad.datos().directorio.contactos.is_empty());
}

/// Busca, autoriza y devuelve el botón «Calificar» del «¿Le sirvió?».
async fn boton_le_sirvio(e: &Entorno) -> String {
    e.enviar(texto("busco costurera")).await;
    e.enviar(elige("p:si")).await;
    e.unidad
        .outbox()
        .into_iter()
        .find_map(|(_, ev)| match ev.mensaje {
            crate::outbox::MensajeOutbox::WhatsApp(MensajeSaliente {
                contenido: ContenidoSaliente::Plantilla { botones, .. },
                ..
            }) => botones.first().cloned(),
            _ => None,
        })
        .expect("botón")
}

#[tokio::test]
async fn el_le_sirvio_sin_cuenta_agradece_y_no_publica() {
    let (e, _) = entorno_con_directorio();
    let boton = boton_le_sirvio(&e).await;

    let r = e.enviar(elige(&boton)).await;
    assert_eq!(ids(&r[0])[0], "oc:5");
    let r = e.enviar(elige("oc:5")).await;

    assert!(cuerpo(&r[0]).contains("necesita una cuenta"));
    assert!(e.unidad.datos().directorio.resenas.is_empty());
}

#[tokio::test]
async fn el_le_sirvio_de_un_residente_con_cuenta_publica_la_resena() {
    use domain::{AsignacionRolId, OrigenAsignacion, RolId, permisos::ROLES_PLANTILLA};
    let (mut e, anunciante) = entorno_con_directorio();
    let cuenta = CuentaId::nuevo();
    e.directorio.0[0].cuenta_id = Some(cuenta);
    let i = ROLES_PLANTILLA
        .iter()
        .position(|r| r.codigo == "residente")
        .expect("plantilla");
    let tenant = e.tenant;
    e.unidad.con_datos(|d| {
        d.directorio.residentes.push(cuenta);
        d.asignaciones.push((
            tenant,
            crate::puertos::identidad::Asignacion {
                id: AsignacionRolId::nuevo(),
                cuenta_id: cuenta,
                rol_id: RolId::from_uuid(uuid::Uuid::from_u128(i as u128 + 1)),
                vigente_desde: ahora() - Duration::days(1),
                vigente_hasta: None,
                origen: OrigenAsignacion::Residencia,
                origen_id: None,
                asignado_por: None,
            },
        ));
    });
    let boton = boton_le_sirvio(&e).await;

    e.enviar(elige(&boton)).await;
    let r = e.enviar(elige("oc:4")).await;

    assert!(cuerpo(&r[0]).contains("¡Gracias!"));
    let resenas = e.unidad.datos().directorio.resenas;
    assert_eq!(resenas.len(), 1);
    assert_eq!(resenas[0].anunciante_id, anunciante);
    assert_eq!(resenas[0].calificacion.valor(), 4);
}

#[tokio::test]
async fn no_recibir_mas_revoca_la_autorizacion_de_publicidad() {
    use crate::casos_uso::publicidad::entrega::BotonBaja;
    use domain::{AutorizacionId, FinalidadTratamiento, MedioAutorizacion};
    let e = entorno(1);
    let persona = e.persona;
    e.unidad.con_datos(|d| {
        d.autorizaciones.push((
            crate::puertos::proteccion_datos::Autorizacion {
                id: AutorizacionId::nuevo(),
                persona_id: persona,
                politica_id: None,
                finalidad: FinalidadTratamiento::Publicidad,
                medio: MedioAutorizacion::App,
                evidencia_archivo_id: None,
                otorgada_en: ahora(),
            },
            None,
        ));
    });

    let r = e.enviar(elige(&BotonBaja { tenant: e.tenant }.id())).await;

    assert!(cuerpo(&r[0]).contains("no le enviaremos más publicidad"));
    assert!(e.unidad.datos().autorizaciones[0].1.is_some(), "revocada");
}

// ---------------------------------------------------------------------------------------------
// Reservas desde el chat
// ---------------------------------------------------------------------------------------------

/// Entorno del motor de reservas (bien con su propietaria y salón social) atendido por el chat:
/// la propietaria tiene cuenta y un rol con los permisos de reservar.
async fn entorno_con_reservas() -> Entorno {
    use crate::casos_uso::{
        reservas::tests::{entorno as entorno_reservas, salon},
        roles::{NuevoRol, SolicitudAsignacion, asignar_rol, crear_rol},
    };
    let er = entorno_reservas().await;
    er.unidad.habilitar(Modulo::GestionAdministrativa);
    salon(&er, domain::PoliticaReserva::default(), false).await;
    let cuenta = er.dueno.actor_id.expect("cuenta de la propietaria");
    let rol = crear_rol(
        &er.unidad,
        &er.ctx,
        NuevoRol {
            codigo: "reserva_chat".into(),
            nombre: "Reserva por chat".into(),
            descripcion: None,
            permisos: er.dueno.permisos.clone(),
        },
    )
    .await
    .expect("rol");
    asignar_rol(
        &er.unidad,
        &RelojFijo(ahora()),
        &er.ctx,
        SolicitudAsignacion::manual(cuenta, rol.id),
    )
    .await
    .expect("asignación");
    let mut persona = PersonaId::nuevo();
    er.unidad.con_datos(|d| {
        persona = d
            .personas
            .iter()
            .find(|p| p.cuenta_id == Some(cuenta))
            .expect("persona de la propietaria")
            .id;
    });
    let v = VinculoTelefono {
        cuenta_id: Some(cuenta),
        bien_privado_id: er.bien,
        ..vinculo(er.ctx.tenant_id, persona, "101")
    };
    Entorno {
        asambleas: Asambleas::default(),
        unidad: er.unidad,
        sesiones: SesionesEnMemoria::default(),
        directorio: Directorio(vec![v]),
        almacen: AlmacenEnMemoria::new(Almacen::Local),
        tenant: er.ctx.tenant_id,
        persona,
        n: Mutex::new(0),
    }
}

/// Primera opción de la última respuesta con ese prefijo de id.
fn opcion(r: &[ContenidoSaliente], prefijo: &str) -> String {
    r.iter()
        .flat_map(ids)
        .find(|i| i.starts_with(prefijo))
        .unwrap_or_else(|| panic!("sin opción {prefijo}: {r:?}"))
}

/// Recorre el menú hasta la confirmación; devuelve la respuesta que pide confirmar.
async fn hasta_confirmar(e: &Entorno) -> Vec<ContenidoSaliente> {
    e.enviar(texto("hola")).await;
    let recursos = e.enviar(elige("m:reservar")).await;
    let dias = e.enviar(elige(&opcion(&recursos, "r:"))).await;
    // El día siguiente: la anticipación mínima puede dejar sin franjas el de hoy.
    let manana = dias
        .iter()
        .flat_map(ids)
        .filter(|i| i.starts_with("d:"))
        .nth(1)
        .expect("días");
    let franjas = e.enviar(elige(&manana)).await;
    e.enviar(elige(&opcion(&franjas, "h:"))).await
}

fn reservas(e: &Entorno) -> usize {
    let mut n = 0;
    e.unidad.con_datos(|d| n = d.reservas.len());
    n
}

#[tokio::test]
async fn reservar_el_salon_por_whatsapp_recorre_recurso_dia_franja_y_confirmacion() {
    let e = entorno_con_reservas().await;

    let confirmar = hasta_confirmar(&e).await;
    assert_eq!(
        ids(&confirmar[0]),
        vec![ID_SI.to_owned(), ID_NO.to_owned()],
        "{confirmar:?}"
    );
    assert!(matches!(e.paso(), Some(Paso::ReservaConfirmar { .. })));
    let r = e.enviar(elige(ID_SI)).await;

    assert!(
        r.iter().any(
            |c| matches!(c, ContenidoSaliente::Texto { cuerpo } if cuerpo.contains("Salón social"))
        ),
        "{r:?}"
    );
    assert_eq!(reservas(&e), 1);
    assert_eq!(e.paso(), Some(Paso::Menu));
}

#[tokio::test]
async fn el_reintento_de_meta_de_la_confirmacion_no_duplica_la_reserva() {
    let e = entorno_con_reservas().await;
    hasta_confirmar(&e).await;

    e.enviar_con("wamid.si", TELEFONO, elige(ID_SI)).await;
    e.enviar_con("wamid.si", TELEFONO, elige(ID_SI)).await;

    assert_eq!(reservas(&e), 1);
}

#[tokio::test]
async fn responder_no_a_la_confirmacion_no_reserva_y_vuelve_al_menu() {
    let e = entorno_con_reservas().await;
    hasta_confirmar(&e).await;

    e.enviar(elige(ID_NO)).await;

    assert_eq!(reservas(&e), 0);
    assert_eq!(e.paso(), Some(Paso::Menu));
}

#[tokio::test]
async fn una_opcion_que_no_es_de_la_lista_repite_la_pregunta_sin_avanzar() {
    let e = entorno_con_reservas().await;
    e.enviar(texto("hola")).await;
    e.enviar(elige("m:reservar")).await;

    let r = e
        .enviar(elige("r:00000000-0000-0000-0000-000000000000"))
        .await;

    assert!(
        r.iter().flat_map(ids).any(|i| i.starts_with("r:")),
        "vuelve a listar: {r:?}"
    );
    assert!(matches!(e.paso(), Some(Paso::ReservaRecurso { .. })));
    let r = e.enviar(texto("mañana a las 3")).await;
    assert!(r.iter().flat_map(ids).any(|i| i.starts_with("r:")));
    assert_eq!(reservas(&e), 0);
}
