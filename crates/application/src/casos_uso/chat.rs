//! Asistente de WhatsApp de la copropiedad (plan 07b): menú con información (preguntas
//! frecuentes), reservas de bienes comunes, parqueadero de visitantes, citas con la
//! administración, reporte de daños con foto, estado de las solicitudes y paso a la bandeja
//! humana.
//!
//! La conversación es una **máquina de estados** ([`Paso`]) que se guarda entre mensajes en
//! [`SesionesConversacion`]; el gateway solo traduce el webhook y llama a [`atender_mensaje`].
//!
//! Reglas:
//! - La copropiedad se resuelve por el teléfono (cualquier línea atiende cualquier PH); si el
//!   teléfono tiene varios vínculos se pregunta con una lista.
//! - «menú» siempre vuelve al inicio.
//! - Todo lo que crea datos es idempotente por `wamid` (Meta reintenta los webhooks).
//! - Nunca se envía información de otros residentes; las respuestas solo hablan de lo propio.
//! - Reservar exige que la persona tenga cuenta en la plataforma: se usan sus permisos, igual
//!   que en la web.
//! - Autorizar una visita (QR temporal), ver los paquetes en portería y votar en una asamblea no
//!   exigen cuenta: la persona ya la identificó su teléfono (plan 10, fase 1).

use chrono::{DateTime, Duration, NaiveDate, Utc};
use domain::{
    ArchivoId, CanalReserva, CanalTicket, CategoriaTicketId, ClasificacionArchivo, ConversacionId,
    EstadoReserva, Franja, ModalidadCita, NaturalezaParqueadero, NombreArchivo, Placa,
    PrioridadTicket, RecursoId, TipoRecurso, TipoTicket, reserva::Tz,
};
use serde::{Deserialize, Serialize};

use crate::{
    AppError, AppResult, Contexto,
    casos_uso::{
        acceso::{
            TxAcceso,
            correspondencia::listar_correspondencia,
            visitas::{NuevaPreautorizacion, NuevaVisita, preautorizar_para_persona},
        },
        archivos::{ArchivoSubido, subir_archivo},
        asambleas::{TxAsambleas, whatsapp::reenviar_preguntas_abiertas},
        conversaciones::{
            EntradaConversacion, TxChat, conversacion_sigue_abierta, listar_preguntas,
            registrar_entrante,
        },
        reservas::{SolicitudReserva, disponibilidad, listar_recursos, solicitar_reserva, zona},
        roles::permisos_en_copropiedad,
        tickets::{NuevoTicket, TxTickets, listar_categorias, radicar_ticket, tickets_de_persona},
    },
    puertos::{
        AlmacenArchivos, FirmaQr, GeneradorImagenQr, Mensajeria, Reloj, RepositorioArchivos,
        RepositorioReservas, UnidadDeTrabajo,
        acceso::FiltroCorrespondencia,
        asambleas::DirectorioAsambleas,
        chat::{DirectorioTelefonos, VinculoTelefono},
        mensajeria::{
            ContenidoEntrante, ContenidoSaliente, Eleccion, MensajeEntrante, MensajeSaliente,
        },
        sesiones::{CierreTurno, SesionGuardada, SesionesConversacion},
    },
};

/// Vida de una sesión inactiva en el menú.
pub const VIDA_SESION: Duration = Duration::minutes(30);
/// Vida de una sesión en la bandeja humana (ventana de Meta).
pub const VIDA_SESION_HUMANA: Duration = Duration::hours(24);
/// Días que se ofrecen para reservar.
pub const DIAS_RESERVA: i64 = 14;
/// Filas de una lista de WhatsApp (una se reserva para «Volver al menú»).
const FILAS: usize = ContenidoSaliente::MAX_FILAS_LISTA - 1;

const ID_MENU: &str = "menu";
const ID_SIN_FOTO: &str = "sin_foto";
const ID_SI: &str = "si";
const ID_NO: &str = "no";

/// Paso de la conversación (se serializa en la sesión).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "paso", rename_all = "snake_case")]
pub enum Paso {
    Menu,
    ElegirVinculo,
    Faq,
    ReporteCategoria,
    ReporteDescripcion {
        categoria: Option<CategoriaTicketId>,
        nombre: Option<String>,
    },
    ReporteFoto {
        categoria: Option<CategoriaTicketId>,
        nombre: Option<String>,
        descripcion: String,
    },
    ReporteUrgente {
        categoria: Option<CategoriaTicketId>,
        nombre: Option<String>,
        descripcion: String,
        foto: Option<ArchivoId>,
    },
    ReservaRecurso {
        tipo: TipoRecurso,
        visitantes: bool,
    },
    ReservaDia {
        reserva: BorradorReserva,
    },
    ReservaFranja {
        reserva: BorradorReserva,
        dia: NaiveDate,
    },
    ReservaPlaca {
        reserva: BorradorReserva,
    },
    ReservaMotivo {
        reserva: BorradorReserva,
    },
    ReservaConfirmar {
        reserva: BorradorReserva,
    },
    Humano {
        conversacion: ConversacionId,
    },
    /// Autorizar una visita: nombre del visitante.
    VisitaNombre,
    /// Autorizar una visita: día de la visita.
    VisitaDia {
        nombre: String,
    },
}

/// Reserva en construcción.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BorradorReserva {
    pub recurso: RecursoId,
    pub nombre: String,
    pub tipo: TipoRecurso,
    pub visitantes: bool,
    pub duracion_min: u32,
    pub granularidad_min: u32,
    pub inicio: Option<DateTime<Utc>>,
    pub placa: Option<String>,
    pub motivo: Option<String>,
}

/// Estado persistido de la conversación.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EstadoChat {
    pub vinculo: Option<VinculoTelefono>,
    pub paso: Paso,
}

impl Default for EstadoChat {
    fn default() -> Self {
        Self {
            vinculo: None,
            paso: Paso::Menu,
        }
    }
}

/// Adaptadores que necesita el chat.
pub struct ServiciosChat<'a, U, S, D, M, A, R, DA, F, Q> {
    pub unidad: &'a U,
    pub sesiones: &'a S,
    pub directorio: &'a D,
    pub mensajeria: &'a M,
    pub almacen: &'a A,
    pub reloj: &'a R,
    /// Asistencias por teléfono (opción «Votar»).
    pub asambleas: &'a DA,
    /// Firma de los QR de visitas.
    pub firma: &'a F,
    /// Imagen PNG del QR que se envía al residente.
    pub qr: &'a Q,
}

/// Puertos de la transacción de copropiedad que usa el chat.
pub trait TxConversacional:
    TxTickets + TxChat + RepositorioReservas + RepositorioArchivos + TxAcceso + TxAsambleas
{
}

impl<T> TxConversacional for T where
    T: TxTickets + TxChat + RepositorioReservas + RepositorioArchivos + TxAcceso + TxAsambleas
{
}

/// Resultado de un turno.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultadoTurno {
    /// Ya se había procesado (reintento de Meta).
    Repetido,
    Atendido {
        respuestas: usize,
    },
}

struct Turno {
    estado: Option<EstadoChat>,
    respuestas: Vec<ContenidoSaliente>,
}

impl Turno {
    fn responder(&mut self, c: ContenidoSaliente) {
        self.respuestas.push(c);
    }

    fn texto(&mut self, t: impl Into<String>) {
        self.responder(ContenidoSaliente::Texto { cuerpo: t.into() });
    }
}

fn normalizar(t: &str) -> String {
    t.trim()
        .to_lowercase()
        .replace(['á', 'à'], "a")
        .replace('é', "e")
        .replace('í', "i")
        .replace('ó', "o")
        .replace('ú', "u")
}

fn pide_menu(c: &ContenidoEntrante) -> bool {
    match c {
        ContenidoEntrante::Texto { cuerpo } => matches!(
            normalizar(cuerpo).as_str(),
            "menu" | "inicio" | "hola" | "0" | "volver" | "cancelar" | "salir"
        ),
        ContenidoEntrante::Respuesta { id, .. } => id == ID_MENU,
        _ => false,
    }
}

fn corto(t: &str, max: usize) -> String {
    if t.chars().count() <= max {
        t.to_owned()
    } else {
        let mut s: String = t.chars().take(max - 1).collect();
        s.push('…');
        s
    }
}

fn boton_menu() -> ContenidoSaliente {
    ContenidoSaliente::Botones {
        cuerpo: "¿Algo más?".into(),
        botones: vec![Eleccion::new(ID_MENU, "Volver al menú")],
    }
}

fn lista(cuerpo: String, boton: &str, mut filas: Vec<Eleccion>) -> ContenidoSaliente {
    filas.truncate(FILAS);
    filas.push(Eleccion::new(ID_MENU, "Volver al menú"));
    ContenidoSaliente::Lista {
        cuerpo,
        boton: boton.into(),
        filas,
    }
}

/// Menú principal. WhatsApp admite 10 filas: si sobran, salen primero las de menos uso (la
/// opción «Votar» solo aparece con una asamblea en curso).
fn menu(v: &VinculoTelefono, varios: bool, votar: bool) -> ContenidoSaliente {
    let mut filas = Vec::new();
    if votar {
        filas.push(
            Eleccion::new("m:votar", "Votar en la asamblea").con_descripcion("Preguntas abiertas"),
        );
    }
    filas.extend([
        Eleccion::new("m:visita", "Autorizar una visita").con_descripcion("Le enviamos el QR"),
        Eleccion::new("m:paquetes", "Mis paquetes").con_descripcion("Correspondencia en portería"),
        Eleccion::new("m:reportar", "Reportar un daño").con_descripcion("Con foto si quiere"),
        Eleccion::new("m:estado", "Mis solicitudes"),
        Eleccion::new("m:humano", "Contactar administración"),
        Eleccion::new("m:reservar", "Reservar bien común"),
        Eleccion::new("m:visitante", "Parqueadero visitante"),
        Eleccion::new("m:cita", "Cita con administración"),
        Eleccion::new("m:info", "Información de la PH")
            .con_descripcion("Horarios, reglas y preguntas frecuentes"),
    ]);
    filas.truncate(ContenidoSaliente::MAX_FILAS_LISTA - usize::from(varios));
    if varios {
        filas.push(Eleccion::new("m:cambiar", "Cambiar bien privado"));
    }
    ContenidoSaliente::Lista {
        cuerpo: format!(
            "Hola. Está en {} · {}.\n¿En qué le ayudamos?",
            v.copropiedad, v.codigo
        ),
        boton: "Ver opciones".into(),
        filas,
    }
}

/// ¿El teléfono es asistente de una asamblea en curso? (Si la consulta falla, no se ofrece.)
async fn votacion_abierta<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    m: &MensajeEntrante,
) -> bool
where
    DA: DirectorioAsambleas,
{
    s.asambleas
        .asistencias_por_telefono(&m.telefono)
        .await
        .is_ok_and(|a| !a.is_empty())
}

fn mensaje_error(e: &AppError) -> String {
    match e {
        AppError::Dominio(_)
        | AppError::Validacion(_)
        | AppError::Conflicto(_)
        | AppError::NoEncontrado(_) => format!("No fue posible: {e}."),
        AppError::ModuloNoHabilitado(_) => {
            "Este servicio no está habilitado en su copropiedad.".into()
        }
        AppError::SinPermiso(_) => "Su perfil no tiene acceso a esta opción.".into(),
        _ => "Tuvimos un problema. Intente de nuevo en unos minutos.".into(),
    }
}

/// Atiende un mensaje entrante y cierra el turno (sesión, respuestas y `wamid` procesado en
/// una transacción).
///
/// # Errors
///
/// Solo errores de las sesiones (base de datos de plataforma). Los errores de los casos de uso
/// se responden al usuario con un mensaje seguro.
pub async fn atender_mensaje<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    m: MensajeEntrante,
) -> AppResult<ResultadoTurno>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    S: SesionesConversacion,
    D: DirectorioTelefonos,
    M: Mensajeria,
    A: AlmacenArchivos,
    R: Reloj,
    DA: DirectorioAsambleas,
    F: FirmaQr,
    Q: GeneradorImagenQr,
{
    if s.sesiones.ya_procesado(&m.wamid).await? {
        return Ok(ResultadoTurno::Repetido);
    }
    let ahora = s.reloj.ahora();
    let guardada = s.sesiones.obtener(&m.linea, &m.telefono, ahora).await?;
    let estado = guardada
        .and_then(|g| serde_json::from_value::<EstadoChat>(g.estado).ok())
        .unwrap_or_default();
    let mut turno = Turno {
        estado: Some(estado),
        respuestas: Vec::new(),
    };
    if let Err(e) = conversar(s, &m, &mut turno).await {
        tracing::warn!(wamid = %m.wamid, error = %e, "error atendiendo un mensaje del chat");
        turno.texto(mensaje_error(&e));
        if !matches!(
            turno.estado.as_ref().map(|e| &e.paso),
            Some(Paso::Humano { .. })
        ) && let Some(e) = &mut turno.estado
        {
            e.paso = Paso::Menu;
        }
    }
    let humano = matches!(
        turno.estado.as_ref().map(|e| &e.paso),
        Some(Paso::Humano { .. })
    );
    let expira_en = ahora
        + if humano {
            VIDA_SESION_HUMANA
        } else {
            VIDA_SESION
        };
    let sesion = turno
        .estado
        .as_ref()
        .map(|e| {
            serde_json::to_value(e)
                .map(|estado| SesionGuardada {
                    estado,
                    ultimo_entrante_en: ahora,
                })
                .map_err(AppError::infra)
        })
        .transpose()?;
    let respuestas: Vec<MensajeSaliente> = turno
        .respuestas
        .into_iter()
        .map(|contenido| MensajeSaliente {
            linea: m.linea.clone(),
            telefono: m.telefono.clone(),
            contenido,
        })
        .collect();
    let n = respuestas.len();
    let cierre = CierreTurno {
        linea: m.linea.clone(),
        telefono: m.telefono.clone(),
        wamid: m.wamid.clone(),
        sesion,
        expira_en,
        respuestas,
    };
    if s.sesiones.cerrar_turno(&cierre).await? {
        Ok(ResultadoTurno::Atendido { respuestas: n })
    } else {
        Ok(ResultadoTurno::Repetido)
    }
}

async fn conversar<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    m: &MensajeEntrante,
    turno: &mut Turno,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    S: SesionesConversacion,
    D: DirectorioTelefonos,
    M: Mensajeria,
    A: AlmacenArchivos,
    R: Reloj,
    DA: DirectorioAsambleas,
    F: FirmaQr,
    Q: GeneradorImagenQr,
{
    let vinculos = s.directorio.vinculos_por_telefono(&m.telefono).await?;
    if vinculos.is_empty() {
        turno.estado = None;
        turno.texto(
            "Este número no está registrado en una copropiedad con este servicio. Pida a la \
             administración que actualice su teléfono en el censo.",
        );
        return Ok(());
    }
    let mut estado = turno.estado.take().unwrap_or_default();
    if estado
        .vinculo
        .as_ref()
        .is_some_and(|v| !vinculos.contains(v))
    {
        estado = EstadoChat::default();
    }
    if estado.vinculo.is_none() {
        if let [unico] = vinculos.as_slice() {
            estado.vinculo = Some(unico.clone());
        } else {
            let elegido = match (&estado.paso, &m.contenido) {
                (Paso::ElegirVinculo, ContenidoEntrante::Respuesta { id, .. }) => id
                    .strip_prefix("v:")
                    .and_then(|i| i.parse::<usize>().ok())
                    .and_then(|i| vinculos.get(i).cloned()),
                _ => None,
            };
            match elegido {
                Some(v) => {
                    turno.responder(menu(&v, true, votacion_abierta(s, m).await));
                    estado.vinculo = Some(v);
                    estado.paso = Paso::Menu;
                }
                None => {
                    let filas = vinculos
                        .iter()
                        .take(ContenidoSaliente::MAX_FILAS_LISTA)
                        .enumerate()
                        .map(|(i, v)| {
                            Eleccion::new(format!("v:{i}"), corto(&v.codigo, 24))
                                .con_descripcion(corto(&v.copropiedad, 72))
                        })
                        .collect();
                    estado.paso = Paso::ElegirVinculo;
                    turno.responder(ContenidoSaliente::Lista {
                        cuerpo: "Su número está registrado en varios bienes privados. ¿Por cuál \
                                 nos escribe?"
                            .into(),
                        boton: "Elegir".into(),
                        filas,
                    });
                }
            }
            turno.estado = Some(estado);
            return Ok(());
        }
    }
    let r = procesar(s, m, &mut estado, vinculos.len() > 1, turno).await;
    // El estado se conserva aunque falle un caso de uso (el error se responde aparte).
    turno.estado = Some(estado);
    r
}

/// Atiende el mensaje con el vínculo ya resuelto y deja en `estado` el paso siguiente.
async fn procesar<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    m: &MensajeEntrante,
    estado: &mut EstadoChat,
    varios: bool,
    turno: &mut Turno,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    S: SesionesConversacion,
    D: DirectorioTelefonos,
    M: Mensajeria,
    A: AlmacenArchivos,
    R: Reloj,
    DA: DirectorioAsambleas,
    F: FirmaQr,
    Q: GeneradorImagenQr,
{
    let Some(vinculo) = estado.vinculo.clone() else {
        return Ok(());
    };
    let ctx = Contexto::new(vinculo.tenant_id).con_request_id(Some(m.wamid.clone()));

    // En la bandeja humana todo va a la conversación, salvo «menú».
    if let Paso::Humano { conversacion } = estado.paso {
        let abierta = conversacion_sigue_abierta(s.unidad, &ctx, conversacion).await?;
        if abierta && !pide_menu(&m.contenido) {
            match &m.contenido {
                ContenidoEntrante::Texto { cuerpo } => {
                    registrar_entrante(
                        s.unidad,
                        s.reloj,
                        &ctx,
                        EntradaConversacion {
                            linea: m.linea.clone(),
                            telefono: m.telefono.clone(),
                            persona_id: Some(vinculo.persona_id),
                            wamid: m.wamid.clone(),
                            texto: Some(corto(cuerpo, 4096)),
                        },
                    )
                    .await?;
                }
                _ => turno.texto(
                    "En esta conversación por ahora solo recibimos texto. Escriba «menú» para \
                     volver a las opciones.",
                ),
            }
            return Ok(());
        }
        estado.paso = Paso::Menu;
        turno.responder(menu(&vinculo, varios, votacion_abierta(s, m).await));
        return Ok(());
    }

    if pide_menu(&m.contenido) {
        estado.paso = Paso::Menu;
        turno.responder(menu(&vinculo, varios, votacion_abierta(s, m).await));
        return Ok(());
    }

    let paso = std::mem::replace(&mut estado.paso, Paso::Menu);
    estado.paso = avanzar(s, m, &ctx, &vinculo, varios, paso, turno).await?;
    if estado.paso == Paso::ElegirVinculo {
        estado.vinculo = None;
    }
    Ok(())
}

fn respuesta_id(c: &ContenidoEntrante) -> Option<&str> {
    match c {
        ContenidoEntrante::Respuesta { id, .. } => Some(id),
        _ => None,
    }
}

fn texto_de(c: &ContenidoEntrante) -> Option<&str> {
    match c {
        ContenidoEntrante::Texto { cuerpo } => Some(cuerpo.trim()).filter(|t| !t.is_empty()),
        _ => None,
    }
}

/// Atiende el mensaje en el paso actual y devuelve el siguiente paso.
#[allow(clippy::too_many_lines)]
async fn avanzar<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    m: &MensajeEntrante,
    ctx: &Contexto,
    v: &VinculoTelefono,
    varios: bool,
    paso: Paso,
    turno: &mut Turno,
) -> AppResult<Paso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    S: SesionesConversacion,
    D: DirectorioTelefonos,
    M: Mensajeria,
    A: AlmacenArchivos,
    R: Reloj,
    DA: DirectorioAsambleas,
    F: FirmaQr,
    Q: GeneradorImagenQr,
{
    let id = respuesta_id(&m.contenido);
    match paso {
        Paso::Menu | Paso::ElegirVinculo => match id {
            Some("m:info") => mostrar_faq(s, ctx, turno).await,
            Some("m:reportar") => {
                let categorias = listar_categorias(s.unidad, s.reloj, ctx).await?;
                let mut filas: Vec<Eleccion> = categorias
                    .iter()
                    .take(FILAS - 1)
                    .map(|c| Eleccion::new(format!("c:{}", c.id), corto(&c.nombre, 24)))
                    .collect();
                filas.push(Eleccion::new("c:otro", "Otro"));
                turno.responder(lista(
                    "¿Qué tipo de daño quiere reportar?".into(),
                    "Elegir",
                    filas,
                ));
                Ok(Paso::ReporteCategoria)
            }
            Some("m:estado") => {
                let tickets = tickets_de_persona(s.unidad, s.reloj, ctx, v.persona_id, 5).await?;
                if tickets.is_empty() {
                    turno.texto("No tiene solicitudes radicadas.");
                } else {
                    let lineas: Vec<String> = tickets
                        .iter()
                        .map(|t| {
                            format!(
                                "• {} · {} · {}",
                                t.numero,
                                t.estado.as_str().replace('_', " "),
                                corto(&t.asunto, 60)
                            )
                        })
                        .collect();
                    turno.texto(format!("Sus últimas solicitudes:\n{}", lineas.join("\n")));
                }
                turno.responder(boton_menu());
                Ok(Paso::Menu)
            }
            Some("m:reservar") => {
                elegir_recurso(s, ctx, v, TipoRecurso::BienComun, false, turno).await
            }
            Some("m:visitante") => {
                elegir_recurso(s, ctx, v, TipoRecurso::Parqueadero, true, turno).await
            }
            Some("m:cita") => {
                elegir_recurso(s, ctx, v, TipoRecurso::AgendaAdministracion, false, turno).await
            }
            Some("m:humano") => {
                let c = registrar_entrante(
                    s.unidad,
                    s.reloj,
                    ctx,
                    EntradaConversacion {
                        linea: m.linea.clone(),
                        telefono: m.telefono.clone(),
                        persona_id: Some(v.persona_id),
                        wamid: m.wamid.clone(),
                        texto: None,
                    },
                )
                .await?;
                turno.texto(
                    "Escriba su mensaje: la administración le responderá por aquí en su horario \
                     de atención. Escriba «menú» para volver a las opciones.",
                );
                Ok(Paso::Humano { conversacion: c.id })
            }
            Some("m:visita") => {
                turno.texto(
                    "Escriba el nombre de la persona que lo visita (como aparece en su documento).",
                );
                Ok(Paso::VisitaNombre)
            }
            Some("m:paquetes") => {
                mostrar_paquetes(s, ctx, v, turno).await?;
                Ok(Paso::Menu)
            }
            Some("m:votar") => {
                let n = reenviar_preguntas_abiertas(s.unidad, s.asambleas, s.reloj, m).await?;
                if n == 0 {
                    turno.texto("No tiene preguntas abiertas por votar en este momento.");
                    turno.responder(boton_menu());
                } else {
                    turno.texto(format!("Le enviamos {n} pregunta(s) para votar."));
                }
                Ok(Paso::Menu)
            }
            Some("m:cambiar") if varios => {
                turno.texto("Escriba cualquier mensaje para elegir el bien privado.");
                Ok(Paso::ElegirVinculo)
            }
            _ => {
                turno.responder(menu(v, varios, votacion_abierta(s, m).await));
                Ok(Paso::Menu)
            }
        },
        Paso::Faq => {
            let preguntas = listar_preguntas(s.unidad, s.reloj, ctx).await?;
            match id
                .and_then(|i| i.strip_prefix("f:"))
                .and_then(|i| preguntas.iter().find(|p| p.id.to_string() == i))
            {
                Some(p) => {
                    turno.texto(p.respuesta.clone());
                    turno.responder(ContenidoSaliente::Botones {
                        cuerpo: "¿Algo más?".into(),
                        botones: vec![
                            Eleccion::new("m:info", "Otra pregunta"),
                            Eleccion::new(ID_MENU, "Volver al menú"),
                        ],
                    });
                    Ok(Paso::Menu)
                }
                None => mostrar_faq(s, ctx, turno).await,
            }
        }
        Paso::ReporteCategoria => {
            let elegido = id.and_then(|i| i.strip_prefix("c:"));
            let categorias = listar_categorias(s.unidad, s.reloj, ctx).await?;
            let (categoria, nombre) = match elegido {
                Some("otro") => (None, None),
                Some(i) => match categorias.iter().find(|c| c.id.to_string() == i) {
                    Some(c) => (Some(c.id), Some(c.nombre.clone())),
                    None => (None, None),
                },
                None => {
                    turno.texto("Elija una opción de la lista o escriba «menú».");
                    return Ok(Paso::ReporteCategoria);
                }
            };
            turno.texto(
                "Describa el daño y dónde está (torre, piso, zona común). Sea lo más preciso \
                 posible.",
            );
            Ok(Paso::ReporteDescripcion { categoria, nombre })
        }
        Paso::ReporteDescripcion { categoria, nombre } => match texto_de(&m.contenido) {
            Some(t) => {
                turno.responder(ContenidoSaliente::Botones {
                    cuerpo: "Si quiere, envíe ahora una foto del daño.".into(),
                    botones: vec![Eleccion::new(ID_SIN_FOTO, "Sin foto")],
                });
                Ok(Paso::ReporteFoto {
                    categoria,
                    nombre,
                    descripcion: corto(t, 5000),
                })
            }
            None => {
                turno.texto("Escriba la descripción en un mensaje de texto.");
                Ok(Paso::ReporteDescripcion { categoria, nombre })
            }
        },
        Paso::ReporteFoto {
            categoria,
            nombre,
            descripcion,
        } => {
            let foto = match &m.contenido {
                ContenidoEntrante::Imagen { media_id, .. } => {
                    Some(guardar_foto(s, ctx, media_id).await?)
                }
                ContenidoEntrante::Respuesta { id, .. } if id == ID_SIN_FOTO => None,
                _ => {
                    turno.responder(ContenidoSaliente::Botones {
                        cuerpo: "Envíe una foto o toque «Sin foto».".into(),
                        botones: vec![Eleccion::new(ID_SIN_FOTO, "Sin foto")],
                    });
                    return Ok(Paso::ReporteFoto {
                        categoria,
                        nombre,
                        descripcion,
                    });
                }
            };
            turno.responder(preguntar_urgencia());
            Ok(Paso::ReporteUrgente {
                categoria,
                nombre,
                descripcion,
                foto,
            })
        }
        Paso::ReporteUrgente {
            categoria,
            nombre,
            descripcion,
            foto,
        } => {
            let urgente = match id {
                Some(ID_SI) => true,
                Some(ID_NO) => false,
                _ => {
                    turno.responder(preguntar_urgencia());
                    return Ok(Paso::ReporteUrgente {
                        categoria,
                        nombre,
                        descripcion,
                        foto,
                    });
                }
            };
            let ticket = radicar_ticket(
                s.unidad,
                s.reloj,
                ctx,
                NuevoTicket {
                    tipo: TipoTicket::SolicitudMantenimiento,
                    categoria_id: categoria,
                    prioridad: Some(if urgente {
                        PrioridadTicket::Urgente
                    } else {
                        PrioridadTicket::Media
                    }),
                    canal: CanalTicket::Whatsapp,
                    asunto: corto(
                        &match &nombre {
                            Some(n) => format!("Daño: {n}"),
                            None => "Reporte de daño".into(),
                        },
                        120,
                    ),
                    descripcion,
                    bien_privado_id: Some(v.bien_privado_id),
                    ubicacion: None,
                    recurso_id: None,
                    activo_id: None,
                    adjuntos: foto.into_iter().collect(),
                    idempotency_key: m.wamid.clone(),
                    solicitante: Some(v.persona_id),
                },
            )
            .await?;
            turno.texto(format!(
                "Radicamos su reporte {}.{} Puede consultar el estado en «Mis solicitudes».",
                ticket.numero,
                if urgente {
                    " Avisamos de inmediato al personal encargado."
                } else {
                    ""
                }
            ));
            turno.responder(boton_menu());
            Ok(Paso::Menu)
        }
        Paso::ReservaRecurso { tipo, visitantes } => {
            let Some(rid) = id.and_then(|i| i.strip_prefix("r:")) else {
                return elegir_recurso(s, ctx, v, tipo, visitantes, turno).await;
            };
            let (ctx_cuenta, recursos) = recursos_de(s, ctx, v, tipo, visitantes).await?;
            let Some(r) = recursos.into_iter().find(|r| r.id.to_string() == rid) else {
                return elegir_recurso(s, ctx, v, tipo, visitantes, turno).await;
            };
            let reserva = BorradorReserva {
                recurso: r.id,
                nombre: r.nombre.clone(),
                tipo,
                visitantes,
                duracion_min: r.politica.duracion_minima_min,
                granularidad_min: r.politica.granularidad_min.max(1),
                inicio: None,
                placa: None,
                motivo: None,
            };
            elegir_dia(s, &ctx_cuenta, reserva, turno).await
        }
        Paso::ReservaDia { reserva } => {
            let Some(dia) = id
                .and_then(|i| i.strip_prefix("d:"))
                .and_then(|d| d.parse::<NaiveDate>().ok())
            else {
                let ctx_cuenta = contexto_cuenta(s, v).await?;
                return elegir_dia(s, &ctx_cuenta, reserva, turno).await;
            };
            let ctx_cuenta = contexto_cuenta(s, v).await?;
            elegir_franja(s, &ctx_cuenta, reserva, dia, turno).await
        }
        Paso::ReservaFranja { mut reserva, dia } => {
            let Some(inicio) = id
                .and_then(|i| i.strip_prefix("h:"))
                .and_then(|h| DateTime::parse_from_rfc3339(h).ok())
                .map(|h| h.with_timezone(&Utc))
            else {
                let ctx_cuenta = contexto_cuenta(s, v).await?;
                return elegir_franja(s, &ctx_cuenta, reserva, dia, turno).await;
            };
            reserva.inicio = Some(inicio);
            if reserva.visitantes {
                turno.texto("Escriba la placa del vehículo del visitante.");
                Ok(Paso::ReservaPlaca { reserva })
            } else if reserva.tipo == TipoRecurso::AgendaAdministracion {
                turno.texto("Cuéntenos brevemente el motivo de la cita.");
                Ok(Paso::ReservaMotivo { reserva })
            } else {
                confirmar_reserva(s, ctx, reserva, turno).await
            }
        }
        Paso::ReservaPlaca { mut reserva } => match texto_de(&m.contenido).map(Placa::new) {
            Some(Ok(p)) => {
                reserva.placa = Some(p.as_str().to_owned());
                confirmar_reserva(s, ctx, reserva, turno).await
            }
            _ => {
                turno.texto("Escriba una placa válida (por ejemplo ABC123).");
                Ok(Paso::ReservaPlaca { reserva })
            }
        },
        Paso::ReservaMotivo { mut reserva } => match texto_de(&m.contenido) {
            Some(t) => {
                reserva.motivo = Some(corto(t, 500));
                confirmar_reserva(s, ctx, reserva, turno).await
            }
            None => {
                turno.texto("Escriba el motivo en un mensaje de texto.");
                Ok(Paso::ReservaMotivo { reserva })
            }
        },
        Paso::ReservaConfirmar { reserva } => match id {
            Some(ID_SI) => {
                let ctx_cuenta = contexto_cuenta(s, v).await?;
                let inicio = reserva
                    .inicio
                    .ok_or_else(|| AppError::Validacion("falta la hora".into()))?;
                let franja = Franja::new(
                    inicio,
                    inicio + Duration::minutes(i64::from(reserva.duracion_min)),
                )?;
                let placa = reserva.placa.as_deref().map(Placa::new).transpose()?;
                let hecha = solicitar_reserva(
                    s.unidad,
                    s.reloj,
                    &ctx_cuenta,
                    SolicitudReserva {
                        recurso_id: reserva.recurso,
                        bien_privado_id: v.bien_privado_id,
                        franja,
                        invitados: 0,
                        idempotency_key: m.wamid.clone(),
                        canal: CanalReserva::Whatsapp,
                        modalidad: (reserva.tipo == TipoRecurso::AgendaAdministracion)
                            .then_some(ModalidadCita::Presencial),
                        motivo: reserva.motivo.clone(),
                        placa_visitante: placa,
                    },
                )
                .await?;
                turno.texto(if hecha.estado == EstadoReserva::Aprobada {
                    format!("Listo: su reserva de {} quedó confirmada.", reserva.nombre)
                } else {
                    format!(
                        "Su solicitud de {} quedó registrada; la administración la revisará y le \
                         avisará.",
                        reserva.nombre
                    )
                });
                turno.responder(boton_menu());
                Ok(Paso::Menu)
            }
            Some(ID_NO) => {
                turno.texto("Cancelamos la solicitud.");
                turno.responder(menu(v, varios, votacion_abierta(s, m).await));
                Ok(Paso::Menu)
            }
            _ => confirmar_reserva(s, ctx, reserva, turno).await,
        },
        Paso::Humano { conversacion } => Ok(Paso::Humano { conversacion }),
        Paso::VisitaNombre => match texto_de(&m.contenido) {
            Some(nombre) if (2..=120).contains(&nombre.chars().count()) => {
                let tz = zona_de_chat(s, ctx).await?;
                let hoy = s.reloj.ahora().with_timezone(&tz).date_naive();
                let filas = (0..DIAS_VISITA)
                    .filter_map(|i| hoy.checked_add_days(chrono::Days::new(i)))
                    .map(|d| Eleccion::new(format!("vd:{d}"), etiqueta_dia(d, hoy)))
                    .collect();
                turno.responder(lista(
                    format!("¿Qué día llega {}?", corto(nombre, 60)),
                    "Elegir día",
                    filas,
                ));
                Ok(Paso::VisitaDia {
                    nombre: nombre.to_owned(),
                })
            }
            _ => {
                turno.texto("Escriba solo el nombre del visitante (o «menú» para salir).");
                Ok(Paso::VisitaNombre)
            }
        },
        Paso::VisitaDia { nombre } => {
            let dia = id
                .and_then(|i| i.strip_prefix("vd:"))
                .and_then(|d| d.parse::<NaiveDate>().ok());
            let Some(dia) = dia else {
                turno.texto("Elija el día de la lista.");
                return Ok(Paso::VisitaDia { nombre });
            };
            autorizar_visita(s, ctx, v, &nombre, dia, turno).await?;
            Ok(Paso::Menu)
        }
    }
}

fn preguntar_urgencia() -> ContenidoSaliente {
    ContenidoSaliente::Botones {
        cuerpo: "¿Es urgente? (fuga de agua, sin agua o luz, ascensor detenido con personas, \
                 riesgo para alguien)"
            .into(),
        botones: vec![
            Eleccion::new(ID_SI, "Sí, es urgente"),
            Eleccion::new(ID_NO, "No es urgente"),
        ],
    }
}

async fn mostrar_faq<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    ctx: &Contexto,
    turno: &mut Turno,
) -> AppResult<Paso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    R: Reloj,
    DA: DirectorioAsambleas,
    F: FirmaQr,
    Q: GeneradorImagenQr,
{
    let preguntas = listar_preguntas(s.unidad, s.reloj, ctx).await?;
    if preguntas.is_empty() {
        turno.texto("La administración aún no ha publicado información en el chat.");
        turno.responder(boton_menu());
        return Ok(Paso::Menu);
    }
    let filas = preguntas
        .iter()
        .map(|p| {
            Eleccion::new(format!("f:{}", p.id), corto(&p.pregunta, 24))
                .con_descripcion(corto(&p.pregunta, 72))
        })
        .collect();
    turno.responder(lista("¿Qué quiere saber?".into(), "Ver temas", filas));
    Ok(Paso::Faq)
}

async fn guardar_foto<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    ctx: &Contexto,
    media_id: &str,
) -> AppResult<ArchivoId>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    M: Mensajeria,
    A: AlmacenArchivos,
    R: Reloj,
    DA: DirectorioAsambleas,
    F: FirmaQr,
    Q: GeneradorImagenQr,
{
    let media = s.mensajeria.descargar_media(media_id).await?;
    let archivo = subir_archivo(
        s.unidad,
        s.almacen,
        s.reloj,
        ctx,
        ArchivoSubido {
            nombre_original: NombreArchivo::new("foto-whatsapp")?,
            // Fotos de un ticket: pueden mostrar personas o su vivienda.
            clasificacion: ClasificacionArchivo::Personal,
            contenido: media.contenido,
        },
    )
    .await?;
    Ok(archivo.id)
}

async fn contexto_cuenta<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    v: &VinculoTelefono,
) -> AppResult<Contexto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    R: Reloj,
    DA: DirectorioAsambleas,
    F: FirmaQr,
    Q: GeneradorImagenQr,
{
    let cuenta = v.cuenta_id.ok_or_else(|| {
        AppError::Validacion(
            "para reservar desde el chat necesita una cuenta en la plataforma; pida a la \
             administración que la vincule"
                .into(),
        )
    })?;
    let permisos = permisos_en_copropiedad(s.unidad, s.reloj, v.tenant_id, cuenta, false).await?;
    Ok(Contexto::de_cuenta(v.tenant_id, cuenta, permisos))
}

async fn recursos_de<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    _ctx: &Contexto,
    v: &VinculoTelefono,
    tipo: TipoRecurso,
    visitantes: bool,
) -> AppResult<(Contexto, Vec<crate::puertos::reservas::Recurso>)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    R: Reloj,
    DA: DirectorioAsambleas,
    F: FirmaQr,
    Q: GeneradorImagenQr,
{
    let ctx_cuenta = contexto_cuenta(s, v).await?;
    let recursos = listar_recursos(s.unidad, s.reloj, &ctx_cuenta, Some(tipo))
        .await?
        .into_iter()
        .filter(|r| r.estado == domain::EstadoRecurso::Disponible)
        .filter(|r| {
            !visitantes
                || r.parqueadero
                    .as_ref()
                    .is_some_and(|p| p.naturaleza == NaturalezaParqueadero::Visitantes)
        })
        .collect();
    Ok((ctx_cuenta, recursos))
}

async fn elegir_recurso<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    ctx: &Contexto,
    v: &VinculoTelefono,
    tipo: TipoRecurso,
    visitantes: bool,
    turno: &mut Turno,
) -> AppResult<Paso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    R: Reloj,
    DA: DirectorioAsambleas,
    F: FirmaQr,
    Q: GeneradorImagenQr,
{
    let (_, recursos) = recursos_de(s, ctx, v, tipo, visitantes).await?;
    if recursos.is_empty() {
        turno.texto("No hay opciones disponibles para reservar en este momento.");
        turno.responder(boton_menu());
        return Ok(Paso::Menu);
    }
    let filas = recursos
        .iter()
        .map(|r| {
            let e = Eleccion::new(format!("r:{}", r.id), corto(&r.nombre, 24));
            match &r.ubicacion {
                Some(u) => e.con_descripcion(corto(u, 72)),
                None => e,
            }
        })
        .collect();
    let cuerpo = match tipo {
        TipoRecurso::AgendaAdministracion => "¿Con quién quiere la cita?",
        TipoRecurso::Parqueadero => "¿Qué parqueadero de visitantes quiere?",
        _ => "¿Qué quiere reservar?",
    };
    turno.responder(lista(cuerpo.into(), "Elegir", filas));
    Ok(Paso::ReservaRecurso { tipo, visitantes })
}

async fn tz_de<U>(unidad: &U, ctx: &Contexto) -> AppResult<Tz>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
{
    let mut tx = unidad.iniciar(ctx).await?;
    zona(&mut tx).await
}

/// Inicios posibles de la duración mínima dentro de las franjas libres, cada `granularidad`.
fn inicios(
    libres: &[Franja],
    duracion: Duration,
    paso: Duration,
    desde: DateTime<Utc>,
) -> Vec<DateTime<Utc>> {
    let mut v = Vec::new();
    for f in libres {
        let mut i = f.inicio();
        while i + duracion <= f.fin() {
            if i >= desde {
                v.push(i);
            }
            i += paso;
        }
    }
    v
}

async fn elegir_dia<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    ctx_cuenta: &Contexto,
    reserva: BorradorReserva,
    turno: &mut Turno,
) -> AppResult<Paso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    R: Reloj,
    DA: DirectorioAsambleas,
    F: FirmaQr,
    Q: GeneradorImagenQr,
{
    let ahora = s.reloj.ahora();
    let tz = tz_de(s.unidad, ctx_cuenta).await?;
    let hoy = ahora.with_timezone(&tz).date_naive();
    let libres = disponibilidad(
        s.unidad,
        s.reloj,
        ctx_cuenta,
        reserva.recurso,
        hoy,
        hoy + Duration::days(DIAS_RESERVA - 1),
    )
    .await?;
    let posibles = inicios(
        &libres,
        Duration::minutes(i64::from(reserva.duracion_min)),
        Duration::minutes(i64::from(reserva.granularidad_min)),
        ahora,
    );
    let mut dias: Vec<(NaiveDate, usize)> = Vec::new();
    for i in &posibles {
        let d = i.with_timezone(&tz).date_naive();
        match dias.last_mut() {
            Some((x, n)) if *x == d => *n += 1,
            _ => dias.push((d, 1)),
        }
    }
    if dias.is_empty() {
        turno.texto(format!(
            "{} no tiene horarios libres en los próximos {DIAS_RESERVA} días.",
            reserva.nombre
        ));
        turno.responder(boton_menu());
        return Ok(Paso::Menu);
    }
    let filas = dias
        .iter()
        .map(|(d, n)| {
            Eleccion::new(format!("d:{d}"), d.format("%Y-%m-%d").to_string())
                .con_descripcion(format!("{n} horario(s) libre(s)"))
        })
        .collect();
    turno.responder(lista(
        format!("¿Qué día quiere {}?", reserva.nombre),
        "Ver días",
        filas,
    ));
    Ok(Paso::ReservaDia { reserva })
}

async fn elegir_franja<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    ctx_cuenta: &Contexto,
    reserva: BorradorReserva,
    dia: NaiveDate,
    turno: &mut Turno,
) -> AppResult<Paso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    R: Reloj,
    DA: DirectorioAsambleas,
    F: FirmaQr,
    Q: GeneradorImagenQr,
{
    let ahora = s.reloj.ahora();
    let tz = tz_de(s.unidad, ctx_cuenta).await?;
    let libres = disponibilidad(s.unidad, s.reloj, ctx_cuenta, reserva.recurso, dia, dia).await?;
    let duracion = Duration::minutes(i64::from(reserva.duracion_min));
    let posibles: Vec<DateTime<Utc>> = inicios(
        &libres,
        duracion,
        Duration::minutes(i64::from(reserva.granularidad_min)),
        ahora,
    )
    .into_iter()
    .filter(|i| i.with_timezone(&tz).date_naive() == dia)
    .collect();
    if posibles.is_empty() {
        turno.texto("Ese día ya no tiene horarios libres.");
        return elegir_dia(s, ctx_cuenta, reserva, turno).await;
    }
    let filas = posibles
        .iter()
        .map(|i| {
            let a = i.with_timezone(&tz);
            let b = (*i + duracion).with_timezone(&tz);
            Eleccion::new(
                format!("h:{}", i.to_rfc3339()),
                format!("{}–{}", a.format("%H:%M"), b.format("%H:%M")),
            )
        })
        .collect();
    turno.responder(lista(
        format!("Horarios libres el {dia}:"),
        "Ver horarios",
        filas,
    ));
    Ok(Paso::ReservaFranja { reserva, dia })
}

async fn confirmar_reserva<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    ctx: &Contexto,
    reserva: BorradorReserva,
    turno: &mut Turno,
) -> AppResult<Paso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    R: Reloj,
    DA: DirectorioAsambleas,
    F: FirmaQr,
    Q: GeneradorImagenQr,
{
    let tz = tz_de(s.unidad, ctx).await?;
    let cuando = reserva
        .inicio
        .map(|i| {
            i.with_timezone(&tz)
                .format("%Y-%m-%d a las %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let mut resumen = format!("¿Confirma {} el {cuando}?", reserva.nombre);
    if let Some(p) = &reserva.placa {
        resumen.push_str(&format!("\nPlaca del visitante: {p}"));
    }
    turno.responder(ContenidoSaliente::Botones {
        cuerpo: resumen,
        botones: vec![
            Eleccion::new(ID_SI, "Confirmar"),
            Eleccion::new(ID_NO, "Cancelar"),
        ],
    });
    Ok(Paso::ReservaConfirmar { reserva })
}

#[cfg(test)]
mod tests;

/// Días que se ofrecen para una visita desde el chat (hoy y los 6 siguientes).
const DIAS_VISITA: u64 = 7;

fn etiqueta_dia(d: NaiveDate, hoy: NaiveDate) -> String {
    match (d - hoy).num_days() {
        0 => "Hoy".into(),
        1 => "Mañana".into(),
        _ => d.format("%d/%m/%Y").to_string(),
    }
}

async fn zona_de_chat<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    ctx: &Contexto,
) -> AppResult<Tz>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
{
    let mut tx = s.unidad.iniciar(ctx).await?;
    zona(&mut tx).await
}

/// Pre-autoriza la visita para todo el día elegido (un ingreso) y envía el QR.
async fn autorizar_visita<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    ctx: &Contexto,
    v: &VinculoTelefono,
    nombre: &str,
    dia: NaiveDate,
    turno: &mut Turno,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    R: Reloj,
    F: FirmaQr,
    A: AlmacenArchivos,
    Q: GeneradorImagenQr,
{
    let tz = zona_de_chat(s, ctx).await?;
    let local = |d: NaiveDate| {
        use chrono::TimeZone;
        tz.from_local_datetime(&d.and_time(chrono::NaiveTime::MIN))
            .earliest()
            .map(|x| x.with_timezone(&Utc))
    };
    let siguiente = dia
        .succ_opt()
        .ok_or(AppError::Validacion("fecha inválida".into()))?;
    let (Some(inicio), Some(fin)) = (local(dia), local(siguiente)) else {
        return Err(AppError::Validacion("fecha inválida".into()));
    };
    let desde = inicio.max(s.reloj.ahora());
    let (visita, qr) = preautorizar_para_persona(
        s.unidad,
        s.reloj,
        s.firma,
        ctx,
        v.persona_id,
        NuevaPreautorizacion {
            visita: NuevaVisita {
                bien_privado_id: v.bien_privado_id,
                tipo: domain::TipoVisita::Visita,
                visitante_nombre: nombre.to_owned(),
                visitante_documento: None,
                placa: None,
                observacion: None,
            },
            desde,
            hasta: fin,
            usos: 1,
        },
    )
    .await?;
    turno.texto(format!(
        "✅ Visita de {} autorizada para el {} ({}). Comparta este código con su visitante: en \
         portería lo leen o lo digitan.",
        corto(&visita.visitante_nombre, 60),
        dia.format("%d/%m/%Y"),
        v.codigo
    ));
    match imagen_qr(s, ctx, &qr).await {
        Ok(url) => turno.responder(ContenidoSaliente::Imagen {
            url,
            leyenda: Some(format!(
                "QR de la visita de {}",
                corto(&visita.visitante_nombre, 60)
            )),
        }),
        Err(e) => {
            tracing::warn!(error = %e, "no se pudo preparar la imagen del QR; se envía el texto")
        }
    }
    turno.texto(qr);
    turno.responder(boton_menu());
    Ok(())
}

/// Vigencia del enlace con el que Meta descarga la imagen del QR.
const VIGENCIA_ENLACE_QR: Duration = Duration::hours(1);

/// Guarda el PNG del QR como archivo de la PH y devuelve un enlace firmado de vida corta.
async fn imagen_qr<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    ctx: &Contexto,
    texto: &str,
) -> AppResult<String>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    A: AlmacenArchivos,
    R: Reloj,
    Q: GeneradorImagenQr,
{
    use crate::casos_uso::archivos::{borrar_huerfano, nuevo_archivo, registrar_en};
    let subido = ArchivoSubido {
        nombre_original: NombreArchivo::new("qr-visita.png")?,
        clasificacion: ClasificacionArchivo::Personal,
        contenido: s.qr.png(texto)?,
    };
    let tipo = domain::TipoArchivo::validar(&subido.contenido)?;
    let archivo = nuevo_archivo(ctx, s.almacen, s.reloj, &subido, tipo);
    s.almacen.guardar(&archivo.clave, &subido.contenido).await?;
    let registro = async {
        let mut tx = s.unidad.iniciar(ctx).await?;
        registrar_en(&mut tx, &archivo).await?;
        crate::puertos::Transaccion::confirmar(tx).await
    }
    .await;
    if let Err(e) = registro {
        borrar_huerfano(s.almacen, ctx, &archivo).await;
        return Err(e);
    }
    s.almacen
        .url_temporal(&crate::puertos::archivos::DescargaArchivo {
            clave: archivo.clave,
            tipo: archivo.tipo,
            nombre: archivo.nombre_original,
            vence_en: s.reloj.ahora() + VIGENCIA_ENLACE_QR,
        })
}

/// Correspondencia que espera en portería para el bien.
async fn mostrar_paquetes<U, S, D, M, A, R, DA, F, Q>(
    s: &ServiciosChat<'_, U, S, D, M, A, R, DA, F, Q>,
    ctx: &Contexto,
    v: &VinculoTelefono,
    turno: &mut Turno,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxConversacional,
    R: Reloj,
{
    let pagina = listar_correspondencia(
        s.unidad,
        s.reloj,
        ctx,
        FiltroCorrespondencia {
            estado: Some(domain::EstadoCorrespondencia::Recibida),
            bien_privado_id: Some(v.bien_privado_id),
            bienes: None,
        },
        crate::SolicitudPagina::new(Some(10), None)?,
    )
    .await?;
    if pagina.datos.is_empty() {
        turno.texto(format!(
            "No hay correspondencia pendiente para {}.",
            v.codigo
        ));
    } else {
        let tz = zona_de_chat(s, ctx).await?;
        let lineas: Vec<String> = pagina
            .datos
            .iter()
            .map(|c| {
                format!(
                    "• {}{} · desde el {}",
                    c.tipo.as_str(),
                    c.remitente
                        .as_deref()
                        .map(|r| format!(" de {}", corto(r, 40)))
                        .unwrap_or_default(),
                    c.recibida_en.with_timezone(&tz).format("%d/%m %H:%M")
                )
            })
            .collect();
        turno.texto(format!(
            "Correspondencia en portería para {}:\n{}\nRecójala con su documento.",
            v.codigo,
            lineas.join("\n")
        ));
    }
    turno.responder(boton_menu());
    Ok(())
}
