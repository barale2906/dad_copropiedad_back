//! Comunicados de la administración (plan 10, fase 4; módulo `gestion_administrativa`).
//!
//! - La administración (`comunicados.gestionar`) redacta un borrador, lo publica de inmediato o
//!   programado, lo retira o lo anula antes de que salga, y ve el tablero de lectura.
//! - Al enviarse se fija la lista de destinatarios ([`RepositorioComunicados::destinatarios_comunicado`]):
//!   adultos de la audiencia y, si la finalidad es `comunicaciones`, solo quien la autorizó
//!   (Ley 1581). Cada uno recibe push y correo si los tiene; WhatsApp (plantilla
//!   [`PLANTILLA_COMUNICADO`]) solo si es importante y no tiene la app
//!   ([`domain::comunicado::canales_comunicado`]). Todo va al outbox con el concepto `comunicados`.
//! - Los programados los envía el worker de gestión ([`enviar_programados`]).
//! - Cada persona ve sus comunicados en la cartelera; abrir uno marca la lectura y el botón
//!   «Enterado» queda registrado si el comunicado lo pide.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use domain::{
    ArchivoId, Audiencia, ComunicadoId, DireccionCorreo, EstadoComunicado, FinalidadTratamiento,
    Modulo, Permiso,
    comunicado::{canales_comunicado, finalidad_comunicado},
    copropiedad::texto_obligatorio,
};

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{avisos::IDIOMA_PLANTILLAS, modulos::exigir_modulo_en, tickets::TxTickets},
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    paginacion::{Pagina, SolicitudPagina},
    puertos::{
        AlmacenArchivos, RegistroAuditoria, Reloj, RepositorioArchivos, Transaccion,
        UnidadDeTrabajo,
        acceso::NotificacionPush,
        archivos::DescargaArchivo,
        comunicados::{
            Comunicado, ComunicadoConResumen, ComunicadoRecibido, EntregaComunicado,
            RepositorioComunicados, ResumenLectura,
        },
        correo::CorreoSaliente,
        mensajeria::{ContenidoSaliente, MensajeSaliente},
    },
};

pub(crate) const ACCION_CREAR: AccionAuditoria =
    AccionAuditoria::new("comunicados.comunicado.crear");
pub(crate) const ACCION_EDITAR: AccionAuditoria =
    AccionAuditoria::new("comunicados.comunicado.editar");
pub(crate) const ACCION_PROGRAMAR: AccionAuditoria =
    AccionAuditoria::new("comunicados.comunicado.programar");
pub(crate) const ACCION_ENVIAR: AccionAuditoria =
    AccionAuditoria::new("comunicados.comunicado.enviar");
pub(crate) const ACCION_RETIRAR: AccionAuditoria =
    AccionAuditoria::new("comunicados.comunicado.retirar");
pub(crate) const ACCION_ANULAR: AccionAuditoria =
    AccionAuditoria::new("comunicados.comunicado.anular");

/// Plantilla de WhatsApp (aprobar en Meta): «{{1}} publicó un comunicado importante: {{2}}.
/// Consúltelo en la app o en su correo.»
pub const PLANTILLA_COMUNICADO: &str = "comunicado_importante";

/// Caracteres máximos del título.
pub const MAX_TITULO: usize = 150;
/// Caracteres máximos del cuerpo.
pub const MAX_CUERPO: usize = 10_000;
/// Adjuntos máximos por comunicado.
pub const MAX_ADJUNTOS: usize = 10;
/// Un comunicado se programa a lo sumo con un año de anticipación.
const MAX_ANTICIPACION_DIAS: i64 = 366;
/// Vigencia de los enlaces de descarga de los adjuntos.
pub const VIGENCIA_ENLACE_ADJUNTO: Duration = Duration::minutes(5);

/// Puertos que usan los casos de uso de comunicados.
pub trait TxComunicados: TxTickets + RepositorioComunicados + RepositorioArchivos {}

impl<T> TxComunicados for T where T: TxTickets + RepositorioComunicados + RepositorioArchivos {}

/// Contenido de un comunicado (crear o editar el borrador).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoComunicado {
    pub titulo: String,
    pub cuerpo: String,
    pub finalidad: FinalidadTratamiento,
    pub audiencia: Audiencia,
    pub importante: bool,
    pub requiere_confirmacion: bool,
    pub adjuntos: Vec<ArchivoId>,
}

/// Contenido validado.
struct Contenido {
    titulo: String,
    cuerpo: String,
    finalidad: FinalidadTratamiento,
    audiencia: Audiencia,
    adjuntos: Vec<ArchivoId>,
}

/// El cuerpo admite saltos de línea y tabulaciones; ningún otro carácter de control.
fn cuerpo_valido(cuerpo: &str) -> AppResult<String> {
    let c = cuerpo.trim();
    if c.is_empty() {
        return Err(AppError::Validacion("`cuerpo` es obligatorio".into()));
    }
    if c.chars().count() > MAX_CUERPO
        || c.chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(AppError::Validacion(format!(
            "`cuerpo`: máximo {MAX_CUERPO} caracteres y sin caracteres no permitidos"
        )));
    }
    Ok(c.replace("\r\n", "\n"))
}

fn validar(n: NuevoComunicado) -> AppResult<Contenido> {
    let mut adjuntos = Vec::with_capacity(n.adjuntos.len());
    for a in n.adjuntos {
        if !adjuntos.contains(&a) {
            adjuntos.push(a);
        }
    }
    if adjuntos.len() > MAX_ADJUNTOS {
        return Err(AppError::Validacion(format!(
            "máximo {MAX_ADJUNTOS} adjuntos por comunicado"
        )));
    }
    Ok(Contenido {
        titulo: texto_obligatorio("titulo", &n.titulo, MAX_TITULO)?,
        cuerpo: cuerpo_valido(&n.cuerpo)?,
        finalidad: finalidad_comunicado(n.finalidad)?,
        audiencia: n.audiencia.validar()?,
        adjuntos,
    })
}

async fn abrir<U>(unidad: &U, reloj: &impl Reloj, ctx: &Contexto) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::GestionAdministrativa).await?;
    Ok(tx)
}

async fn abrir_gestion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    ctx.exigir(Permiso::ComunicadosGestionar)?;
    abrir(unidad, reloj, ctx).await
}

async fn bloqueado<T: TxComunicados>(tx: &mut T, id: ComunicadoId) -> AppResult<Comunicado> {
    tx.bloquear_comunicado(id)
        .await?
        .ok_or(AppError::NoEncontrado("comunicado"))
}

fn detalle(c: &Comunicado) -> AppResult<DetalleAuditoria> {
    DetalleAuditoria::vacio()
        .con("finalidad", c.finalidad.as_str())?
        .con("estado", c.estado.as_str())?
        .con("importante", c.importante)
}

// ---------------------------------------------------------------------------------------------
// Administración
// ---------------------------------------------------------------------------------------------

/// Crea un comunicado en borrador.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `comunicados.gestionar`; [`AppError::ModuloNoHabilitado`];
/// [`AppError::Validacion`] si el contenido, la finalidad, la audiencia o un adjunto no son
/// válidos.
pub async fn crear_comunicado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoComunicado,
) -> AppResult<Comunicado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    let (importante, requiere_confirmacion) = (nuevo.importante, nuevo.requiere_confirmacion);
    let v = validar(nuevo)?;
    let mut tx = abrir_gestion(unidad, reloj, ctx).await?;
    let ahora = reloj.ahora();
    let c = Comunicado {
        id: ComunicadoId::nuevo(),
        titulo: v.titulo,
        cuerpo: v.cuerpo,
        finalidad: v.finalidad,
        audiencia: v.audiencia,
        importante,
        requiere_confirmacion,
        estado: EstadoComunicado::Borrador,
        programado_para: None,
        enviado_en: None,
        adjuntos: v.adjuntos,
        creado_por: ctx.actor_id,
        creado_en: ahora,
        actualizado_en: ahora,
    };
    tx.insertar_comunicado(&c).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR, "comunicado", c.id.as_uuid()).con_detalle(detalle(&c)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Reemplaza el contenido de un borrador.
///
/// # Errors
///
/// Como [`crear_comunicado`]; [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si ya no es
/// borrador.
pub async fn editar_comunicado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComunicadoId,
    nuevo: NuevoComunicado,
) -> AppResult<Comunicado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    let (importante, requiere_confirmacion) = (nuevo.importante, nuevo.requiere_confirmacion);
    let v = validar(nuevo)?;
    let mut tx = abrir_gestion(unidad, reloj, ctx).await?;
    let mut c = bloqueado(&mut tx, id).await?;
    if !c.estado.editable() {
        return Err(AppError::Conflicto(
            "solo se edita un borrador: retire el comunicado programado primero".into(),
        ));
    }
    c.titulo = v.titulo;
    c.cuerpo = v.cuerpo;
    c.finalidad = v.finalidad;
    c.audiencia = v.audiencia;
    c.importante = importante;
    c.requiere_confirmacion = requiere_confirmacion;
    c.adjuntos = v.adjuntos;
    c.actualizado_en = reloj.ahora();
    tx.actualizar_comunicado(&c).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EDITAR, "comunicado", c.id.as_uuid()).con_detalle(detalle(&c)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Publica un borrador: sin fecha (o con una que ya pasó) se envía de inmediato; con fecha
/// futura queda programado y lo envía el worker de gestión.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::ModuloNoHabilitado`]; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si no es borrador; [`AppError::Validacion`] si se programa a más de
/// un año.
pub async fn publicar_comunicado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComunicadoId,
    programado_para: Option<DateTime<Utc>>,
) -> AppResult<Comunicado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    let ahora = reloj.ahora();
    if programado_para.is_some_and(|p| p > ahora + Duration::days(MAX_ANTICIPACION_DIAS)) {
        return Err(AppError::Validacion(
            "un comunicado se programa con máximo un año de anticipación".into(),
        ));
    }
    let mut tx = abrir_gestion(unidad, reloj, ctx).await?;
    let mut c = bloqueado(&mut tx, id).await?;
    c.estado = c.estado.transicionar(EstadoComunicado::Programado)?;
    c.programado_para = Some(programado_para.unwrap_or(ahora).max(ahora));
    c.actualizado_en = ahora;
    if programado_para.is_some_and(|p| p > ahora) {
        tx.actualizar_comunicado(&c).await?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_PROGRAMAR, "comunicado", c.id.as_uuid())
                .con_detalle(detalle(&c)?),
        )
        .await?;
    } else {
        enviar(&mut tx, ctx.tenant_id, &mut c, ahora).await?;
    }
    tx.confirmar().await?;
    Ok(c)
}

/// Devuelve un comunicado programado a borrador para editarlo.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::ModuloNoHabilitado`]; [`AppError::NoEncontrado`];
/// [`domain::DominioError::TransicionInvalida`] si no está programado.
pub async fn retirar_comunicado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComunicadoId,
) -> AppResult<Comunicado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    cambiar_estado(
        unidad,
        reloj,
        ctx,
        id,
        EstadoComunicado::Borrador,
        ACCION_RETIRAR,
    )
    .await
}

/// Anula un borrador o un programado (uno enviado no se anula).
///
/// # Errors
///
/// Como [`retirar_comunicado`].
pub async fn anular_comunicado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComunicadoId,
) -> AppResult<Comunicado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    cambiar_estado(
        unidad,
        reloj,
        ctx,
        id,
        EstadoComunicado::Anulado,
        ACCION_ANULAR,
    )
    .await
}

async fn cambiar_estado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComunicadoId,
    hacia: EstadoComunicado,
    accion: AccionAuditoria,
) -> AppResult<Comunicado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    let mut tx = abrir_gestion(unidad, reloj, ctx).await?;
    let mut c = bloqueado(&mut tx, id).await?;
    c.estado = c.estado.transicionar(hacia)?;
    if hacia == EstadoComunicado::Borrador {
        c.programado_para = None;
    }
    c.actualizado_en = reloj.ahora();
    tx.actualizar_comunicado(&c).await?;
    tx.registrar(
        EventoAuditoria::ok(accion, "comunicado", c.id.as_uuid()).con_detalle(detalle(&c)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Comunicados de la PH con sus contadores de entrega y lectura.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::ModuloNoHabilitado`].
pub async fn listar_comunicados<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<ComunicadoConResumen>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    let mut tx = abrir_gestion(unidad, reloj, ctx).await?;
    tx.comunicados(pagina).await
}

/// Un comunicado con su tablero de entrega y lectura.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::ModuloNoHabilitado`]; [`AppError::NoEncontrado`].
pub async fn ver_comunicado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComunicadoId,
) -> AppResult<ComunicadoConResumen>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    let mut tx = abrir_gestion(unidad, reloj, ctx).await?;
    let comunicado = tx
        .comunicado(id)
        .await?
        .ok_or(AppError::NoEncontrado("comunicado"))?;
    let resumen = tx.resumen_lectura(id).await?;
    Ok(ComunicadoConResumen {
        comunicado,
        resumen,
    })
}

// ---------------------------------------------------------------------------------------------
// Envío
// ---------------------------------------------------------------------------------------------

/// Envía el comunicado bloqueado `c`: fija los destinatarios, encola los mensajes y lo marca
/// enviado. Devuelve el resumen de la entrega.
async fn enviar<T: TxComunicados>(
    tx: &mut T,
    tenant: domain::TenantId,
    c: &mut Comunicado,
    ahora: DateTime<Utc>,
) -> AppResult<ResumenLectura> {
    c.estado = c.estado.transicionar(EstadoComunicado::Enviado)?;
    c.enviado_en = Some(ahora);
    c.actualizado_en = ahora;
    let destinatarios = tx
        .destinatarios_comunicado(&c.audiencia, c.finalidad, ahora)
        .await?;
    let linea = if c.importante {
        tx.linea_predeterminada().await?
    } else {
        None
    };
    let ph = tx.ficha().await?.map(|f| f.nombre).unwrap_or_default();
    let prioridad = if c.importante {
        PrioridadSalida::Evento
    } else {
        PrioridadSalida::Recordatorio
    };
    let mut datos = BTreeMap::new();
    datos.insert("tipo".to_owned(), "comunicado".to_owned());
    datos.insert("copropiedad_id".to_owned(), tenant.to_string());
    datos.insert("comunicado_id".to_owned(), c.id.to_string());

    let mut resumen = ResumenLectura::default();
    let mut entregas = Vec::with_capacity(destinatarios.len());
    for d in destinatarios {
        let correo = d
            .correo
            .as_deref()
            .and_then(|x| DireccionCorreo::new(x).ok());
        let whatsapp = match (&linea, &d.telefono) {
            (Some(l), Some(t)) => Some(MensajeSaliente {
                linea: l.clone(),
                telefono: t.clone(),
                contenido: ContenidoSaliente::Plantilla {
                    nombre: PLANTILLA_COMUNICADO.to_owned(),
                    idioma: IDIOMA_PLANTILLAS.to_owned(),
                    parametros: vec![parametro(&ph), parametro(&c.titulo)],
                    botones: Vec::new(),
                },
            }),
            _ => None,
        };
        let canales = canales_comunicado(
            c.importante,
            !d.tokens.is_empty(),
            correo.is_some(),
            whatsapp.is_some(),
        );
        if canales.push {
            for (i, token) in d.tokens.iter().enumerate() {
                let n = NotificacionPush {
                    token: token.clone(),
                    titulo: corto(&c.titulo, 80),
                    cuerpo: corto(&c.cuerpo.replace('\n', " "), 160),
                    datos: datos.clone(),
                    // Si la app ya no está, el importante se desvía por WhatsApp (ADR 0011).
                    respaldo: if i == 0 { whatsapp.clone() } else { None },
                };
                encolar(tx, prioridad, MensajeOutbox::Push(n)).await?;
            }
        }
        if let (true, Some(para)) = (canales.correo, correo) {
            let correo = CorreoSaliente {
                para,
                asunto: format!("{ph}: {}", c.titulo),
                texto: format!(
                    "{}\n\n{}\n\n—\n{ph}. Consulte los adjuntos y confirme la lectura en la app.",
                    c.titulo, c.cuerpo
                ),
                html: None,
            };
            encolar(tx, prioridad, MensajeOutbox::Correo(correo)).await?;
        }
        if let (true, Some(m)) = (canales.whatsapp, whatsapp) {
            encolar(tx, prioridad, MensajeOutbox::WhatsApp(m)).await?;
        }
        resumen.destinatarios += 1;
        resumen.por_push += u32::from(canales.push);
        resumen.por_correo += u32::from(canales.correo);
        resumen.por_whatsapp += u32::from(canales.whatsapp);
        resumen.sin_canal += u32::from(!canales.alguno());
        entregas.push(EntregaComunicado {
            persona: d.persona,
            canales,
        });
    }
    tx.actualizar_comunicado(c).await?;
    tx.insertar_entregas(c.id, &entregas).await?;
    let det = detalle(c)?
        .con("destinatarios", resumen.destinatarios)?
        .con("por_whatsapp", resumen.por_whatsapp)?;
    tx.registrar(EventoAuditoria::ok(ACCION_ENVIAR, "comunicado", c.id.as_uuid()).con_detalle(det))
        .await?;
    Ok(resumen)
}

async fn encolar<T: TxComunicados>(
    tx: &mut T,
    prioridad: PrioridadSalida,
    mensaje: MensajeOutbox,
) -> AppResult<()> {
    tx.encolar_comunicado(&EventoSaliente { prioridad, mensaje })
        .await
}

fn parametro(texto: &str) -> String {
    texto
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(200)
        .collect()
}

fn corto(texto: &str, max: usize) -> String {
    let t: String = texto.chars().take(max).collect();
    if texto.chars().count() > max {
        format!("{t}…")
    } else {
        t
    }
}

/// Envía los comunicados programados cuya hora llegó (worker de gestión, contexto del sistema).
/// Devuelve cuántos envió.
///
/// # Errors
///
/// [`AppError::ModuloNoHabilitado`] si la PH ya no tiene el módulo; errores de infraestructura.
pub async fn enviar_programados<U>(unidad: &U, reloj: &impl Reloj, ctx: &Contexto) -> AppResult<u32>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let ahora = reloj.ahora();
    let mut enviados = 0;
    for id in tx.programados_vencidos(ahora).await? {
        let mut c = bloqueado(&mut tx, id).await?;
        if c.estado == EstadoComunicado::Programado {
            enviar(&mut tx, ctx.tenant_id, &mut c, ahora).await?;
            enviados += 1;
        }
    }
    tx.confirmar().await?;
    Ok(enviados)
}

// ---------------------------------------------------------------------------------------------
// Cartelera de cada persona
// ---------------------------------------------------------------------------------------------

/// Adjunto con su enlace de descarga temporal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdjuntoComunicado {
    pub archivo_id: ArchivoId,
    pub nombre: String,
    pub tipo_mime: String,
    pub url: String,
}

/// Comunicado abierto por su destinatario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComunicadoAbierto {
    pub recibido: ComunicadoRecibido,
    pub adjuntos: Vec<AdjuntoComunicado>,
}

async fn persona_actor<T: TxComunicados>(
    tx: &mut T,
    ctx: &Contexto,
) -> AppResult<Option<domain::PersonaId>> {
    let cuenta = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    tx.persona_de_cuenta(cuenta).await
}

/// Comunicados que recibió la persona de la cuenta (su cartelera). Sin persona vinculada, la
/// cartelera está vacía.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] sin cuenta; [`AppError::ModuloNoHabilitado`].
pub async fn mis_comunicados<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<ComunicadoRecibido>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    match persona_actor(&mut tx, ctx).await? {
        Some(p) => tx.recibidos(p, pagina).await,
        None => Ok(Pagina {
            datos: Vec::new(),
            siguiente: None,
        }),
    }
}

/// Abre un comunicado recibido: registra la primera lectura y entrega los adjuntos con enlaces
/// de descarga de [`VIGENCIA_ENLACE_ADJUNTO`].
///
/// # Errors
///
/// [`AppError::NoAutenticado`]; [`AppError::ModuloNoHabilitado`]; [`AppError::NoEncontrado`]
/// si no es destinatario.
pub async fn abrir_mi_comunicado<U, A>(
    unidad: &U,
    reloj: &impl Reloj,
    almacen: &A,
    ctx: &Contexto,
    id: ComunicadoId,
) -> AppResult<ComunicadoAbierto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
    A: AlmacenArchivos,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let persona = persona_actor(&mut tx, ctx)
        .await?
        .ok_or(AppError::NoEncontrado("comunicado"))?;
    let ahora = reloj.ahora();
    let mut r = tx
        .recibido(persona, id)
        .await?
        .ok_or(AppError::NoEncontrado("comunicado"))?;
    if r.leido_en.is_none() {
        tx.marcar_leido(persona, id, ahora).await?;
        r.leido_en = Some(ahora);
    }
    let mut adjuntos = Vec::with_capacity(r.comunicado.adjuntos.len());
    for a in &r.comunicado.adjuntos {
        if let Some(archivo) = tx.buscar(*a).await? {
            let url = almacen.url_temporal(&DescargaArchivo {
                clave: archivo.clave,
                tipo: archivo.tipo,
                nombre: archivo.nombre_original.clone(),
                vence_en: ahora + VIGENCIA_ENLACE_ADJUNTO,
            })?;
            adjuntos.push(AdjuntoComunicado {
                archivo_id: archivo.id,
                nombre: archivo.nombre_original.as_str().to_owned(),
                tipo_mime: archivo.tipo.as_str().to_owned(),
                url,
            });
        }
    }
    tx.confirmar().await?;
    Ok(ComunicadoAbierto {
        recibido: r,
        adjuntos,
    })
}

/// Registra el «Enterado» del destinatario (idempotente).
///
/// # Errors
///
/// [`AppError::NoAutenticado`]; [`AppError::ModuloNoHabilitado`]; [`AppError::NoEncontrado`]
/// si no es destinatario; [`AppError::Conflicto`] si el comunicado no pide confirmación.
pub async fn confirmar_enterado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComunicadoId,
) -> AppResult<ComunicadoRecibido>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxComunicados,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let persona = persona_actor(&mut tx, ctx)
        .await?
        .ok_or(AppError::NoEncontrado("comunicado"))?;
    let mut r = tx
        .recibido(persona, id)
        .await?
        .ok_or(AppError::NoEncontrado("comunicado"))?;
    if !r.comunicado.requiere_confirmacion {
        return Err(AppError::Conflicto(
            "este comunicado no pide confirmación de «Enterado»".into(),
        ));
    }
    if r.enterado_en.is_none() {
        let ahora = reloj.ahora();
        tx.marcar_enterado(persona, id, ahora).await?;
        r.enterado_en = Some(ahora);
        r.leido_en.get_or_insert(ahora);
        tx.confirmar().await?;
    }
    Ok(r)
}

#[cfg(test)]
mod tests;
