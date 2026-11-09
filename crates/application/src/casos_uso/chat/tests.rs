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

struct Entorno {
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
    e.enviar(elige("m:humano")).await;
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

#[test]
fn menu_y_comandos_reconocidos() {
    assert!(pide_menu(&texto("  Menú ")));
    assert!(pide_menu(&elige(ID_MENU)));
    assert!(!pide_menu(&texto("menudo problema")));
    assert_eq!(corto("abcdef", 4), "abc…");
}
