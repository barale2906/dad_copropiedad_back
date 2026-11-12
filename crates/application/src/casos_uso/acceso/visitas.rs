//! Visitas: anuncio en la portería con escalamiento (primer respondiente → todos los
//! residentes a los 2 min → llamada a los 5 min), pre-autorización con QR temporal firmado,
//! decisión, ingreso y salida.

use chrono::{DateTime, TimeDelta, Utc};
use domain::{
    BienPrivadoId, CanalRespuestaVisita, CargaQr, Documento, EstadoVisita, NivelAviso,
    OrigenVisita, Permiso, PersonaId, Placa, ResultadoAcceso, ResultadoLlamada, SentidoAcceso,
    TipoEventoVisita, TipoVisita, VisitaId,
    acceso::{destinatarios_aviso, momento_siguiente_paso, puede_decidir_visita},
    copropiedad::{texto_obligatorio, texto_opcional},
};

use super::{
    AvisoAcceso, PlantillaAviso, TxAcceso, abrir, avisar_personas, codigo_bien, corto, hoy,
    persona_del_actor, tiene, vinculados, zona_ph,
};
use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::acceso::whatsapp::boton_visita,
    outbox::PrioridadSalida,
    puertos::{
        FirmaQr, RegistroAuditoria, Reloj, RepositorioAcceso, RepositorioPersonas, Transaccion,
        UnidadDeTrabajo,
        acceso::{EventoAcceso, EventoVisita, FiltroVisitas, Visita},
    },
};

pub(crate) const ACCION_ANUNCIAR: AccionAuditoria = AccionAuditoria::new("acceso.visita.anunciar");
pub(crate) const ACCION_PREAUTORIZAR: AccionAuditoria =
    AccionAuditoria::new("acceso.visita.preautorizar");
pub(crate) const ACCION_DECIDIR: AccionAuditoria = AccionAuditoria::new("acceso.visita.decidir");
pub(crate) const ACCION_LLAMADA: AccionAuditoria =
    AccionAuditoria::new("acceso.visita.registrar_llamada");
pub(crate) const ACCION_CONTACTOS: AccionAuditoria =
    AccionAuditoria::new("acceso.visita.ver_contactos");
pub(crate) const ACCION_INGRESO: AccionAuditoria = AccionAuditoria::new("acceso.visita.ingresar");
pub(crate) const ACCION_SALIDA: AccionAuditoria = AccionAuditoria::new("acceso.visita.salir");
pub(crate) const ACCION_CANCELAR: AccionAuditoria = AccionAuditoria::new("acceso.visita.cancelar");

/// Plantilla de utilidad «visita_anunciada». Parámetros: visitante, tipo, bien privado.
/// Botones: «Autorizar», «Rechazar».
pub const PLANTILLA_VISITA_ANUNCIADA: &str = "visita_anunciada";

/// Tiempo que vale la autorización de una visita anunciada para ingresar.
pub const VIGENCIA_AUTORIZACION_ANUNCIO: TimeDelta = TimeDelta::hours(1);
/// Ventana máxima de una pre-autorización.
pub const VENTANA_MAXIMA_PREAUTORIZACION: TimeDelta = TimeDelta::days(31);
/// Usos máximos de un QR temporal.
pub const USOS_MAXIMOS_QR: u16 = 60;

/// Datos del visitante que la portería anuncia.
#[derive(Debug, Clone)]
pub struct NuevaVisita {
    pub bien_privado_id: BienPrivadoId,
    pub tipo: TipoVisita,
    pub visitante_nombre: String,
    pub visitante_documento: Option<String>,
    pub placa: Option<String>,
    pub observacion: Option<String>,
}

/// Pre-autorización de un residente.
#[derive(Debug, Clone)]
pub struct NuevaPreautorizacion {
    pub visita: NuevaVisita,
    pub desde: DateTime<Utc>,
    pub hasta: DateTime<Utc>,
    pub usos: u16,
}

/// Visita con su historial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetalleVisita {
    pub visita: Visita,
    pub eventos: Vec<EventoVisita>,
}

/// Contacto para la llamada de la portería (solo mientras la visita la requiere).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactoLlamada {
    pub persona_id: PersonaId,
    pub nombre: String,
    pub telefono: String,
}

/// Resultado de una decisión: la primera gana; las siguientes reciben la ya tomada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultadoDecision {
    Decidida(Visita),
    YaDecidida(Visita),
}

/// Datos del visitante ya validados: nombre, documento, placa y observación.
type DatosVisitante = (String, Option<Documento>, Option<Placa>, Option<String>);

fn validar_datos(n: &NuevaVisita) -> AppResult<DatosVisitante> {
    Ok((
        texto_obligatorio("visitante_nombre", &n.visitante_nombre, 120)?,
        n.visitante_documento
            .as_deref()
            .filter(|d| !d.trim().is_empty())
            .map(Documento::new)
            .transpose()?,
        n.placa
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .map(Placa::new)
            .transpose()?,
        texto_opcional("observacion", n.observacion.as_deref(), 300)?,
    ))
}

fn visita_nueva(
    ctx: &Contexto,
    n: &NuevaVisita,
    origen: OrigenVisita,
    estado: EstadoVisita,
    ahora: DateTime<Utc>,
) -> AppResult<Visita> {
    let (nombre, documento, placa, observacion) = validar_datos(n)?;
    Ok(Visita {
        id: VisitaId::nuevo(),
        bien_privado_id: n.bien_privado_id,
        origen,
        tipo: n.tipo,
        estado,
        visitante_nombre: nombre,
        visitante_documento: documento,
        placa,
        observacion,
        decidida_por: None,
        canal_decision: None,
        ventana_desde: None,
        ventana_hasta: None,
        usos_permitidos: 1,
        usos: 0,
        avisos_enviados: 0,
        siguiente_paso_en: None,
        registrada_por: ctx.actor_id,
        creada_en: ahora,
        decidida_en: None,
        ingreso_en: None,
        salida_en: None,
    })
}

pub(crate) fn evento(
    v: &Visita,
    tipo: TipoEventoVisita,
    ctx: &Contexto,
    ahora: DateTime<Utc>,
) -> EventoVisita {
    EventoVisita {
        visita_id: v.id,
        tipo,
        canal: None,
        persona_id: None,
        cuenta_id: ctx.actor_id,
        nivel: None,
        destinatarios: None,
        ocurrido_en: ahora,
    }
}

async fn auditar<T: TxAcceso>(tx: &mut T, accion: AccionAuditoria, v: &Visita) -> AppResult<()> {
    let detalle = DetalleAuditoria::vacio()
        .con("estado", v.estado.as_str())?
        .con("tipo", v.tipo.as_str())?
        .con("bien_privado_id", v.bien_privado_id.to_string())?;
    tx.registrar(EventoAuditoria::ok(accion, "visita", v.id.as_uuid()).con_detalle(detalle))
        .await
}

/// Aviso de visita para el bien (push con acciones y plantilla con botones).
pub(crate) async fn aviso_visita<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    v: &Visita,
) -> AppResult<AvisoAcceso> {
    let codigo = codigo_bien(tx, v.bien_privado_id).await?;
    let tipo = v.tipo.as_str().replace('_', " ");
    let mut datos = std::collections::BTreeMap::new();
    datos.insert("tipo".into(), "visita".into());
    datos.insert("copropiedad_id".into(), ctx.tenant_id.to_string());
    datos.insert("visita_id".into(), v.id.to_string());
    datos.insert("acciones".into(), "autorizar,rechazar".into());
    Ok(AvisoAcceso {
        prioridad: PrioridadSalida::Confirmacion,
        titulo: "Visita en portería".into(),
        cuerpo: format!(
            "{} ({tipo}) pide ingresar a {codigo}.",
            corto(&v.visitante_nombre, 60)
        ),
        datos,
        plantilla: PlantillaAviso {
            nombre: PLANTILLA_VISITA_ANUNCIADA,
            parametros: vec![v.visitante_nombre.clone(), tipo, codigo],
            botones: vec![
                boton_visita(ctx.tenant_id, v.id, true),
                boton_visita(ctx.tenant_id, v.id, false),
            ],
        },
    })
}

/// Envía el aviso del nivel y registra el evento. Devuelve a cuántas personas llegó.
pub(crate) async fn avisar_visita<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    v: &mut Visita,
    nivel: NivelAviso,
    ahora: DateTime<Utc>,
) -> AppResult<u32> {
    let tz = zona_ph(tx).await?;
    let (residentes, titulares) = vinculados(tx, v.bien_privado_id).await?;
    let personas = destinatarios_aviso(nivel, &residentes, &titulares, hoy(ahora, tz));
    let aviso = aviso_visita(tx, ctx, v).await?;
    let n = avisar_personas(tx, &personas, &aviso).await?;
    v.avisos_enviados += 1;
    let mut e = evento(v, TipoEventoVisita::AvisoEnviado, ctx, ahora);
    e.nivel = Some(v.avisos_enviados);
    e.destinatarios = Some(n);
    tx.insertar_evento_visita(&e).await?;
    Ok(n)
}

/// Qué pasó con la visita, para el aviso a la portería.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NovedadPorteria {
    /// Nadie respondió: hay que llamar.
    Llamar,
    /// Un residente decidió desde la app, la web o WhatsApp.
    Decidida,
}

/// Push a la portería (cuentas con `acceso.registrar`, plan 10 fase 7): la tablet deja de
/// depender de refrescar. Sin WhatsApp de respaldo: la portería usa la app. Devuelve a cuántos
/// dispositivos se envió.
pub(crate) async fn avisar_porteria<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    v: &Visita,
    novedad: NovedadPorteria,
) -> AppResult<u32> {
    let cuentas: Vec<domain::CuentaId> = tx
        .destinatarios_con_permiso(Permiso::AccesoRegistrar)
        .await?
        .into_iter()
        .map(|d| d.cuenta_id)
        .collect();
    if cuentas.is_empty() {
        return Ok(0);
    }
    let tokens = tx.tokens_push(&cuentas).await?;
    let codigo = codigo_bien(tx, v.bien_privado_id).await?;
    let visitante = corto(&v.visitante_nombre, 60);
    let (titulo, cuerpo) = match novedad {
        NovedadPorteria::Llamar => (
            format!("Llamar a {codigo}"),
            format!("Nadie respondió por la visita de {visitante}."),
        ),
        NovedadPorteria::Decidida if v.estado == EstadoVisita::Autorizada => (
            format!("Visita autorizada en {codigo}"),
            format!("Puede ingresar {visitante}."),
        ),
        NovedadPorteria::Decidida => (
            format!("Visita rechazada en {codigo}"),
            format!("No se autorizó el ingreso de {visitante}."),
        ),
    };
    let mut datos = std::collections::BTreeMap::new();
    datos.insert("tipo".into(), "porteria_visita".into());
    datos.insert("copropiedad_id".into(), ctx.tenant_id.to_string());
    datos.insert("visita_id".into(), v.id.to_string());
    datos.insert("estado".into(), v.estado.as_str().into());
    let mut n = 0;
    for (_, token) in tokens {
        tx.encolar_aviso_acceso(&crate::outbox::EventoSaliente {
            prioridad: PrioridadSalida::Confirmacion,
            mensaje: crate::outbox::MensajeOutbox::Push(crate::puertos::acceso::NotificacionPush {
                token,
                titulo: titulo.clone(),
                cuerpo: cuerpo.clone(),
                datos: datos.clone(),
                respaldo: None,
            }),
        })
        .await?;
        n += 1;
    }
    Ok(n)
}

/// Pasa la visita a `requiere_llamada`: la portería debe llamar.
pub(crate) async fn pedir_llamada<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    v: &mut Visita,
    ahora: DateTime<Utc>,
) -> AppResult<()> {
    v.estado = v.estado.transicionar(EstadoVisita::RequiereLlamada)?;
    v.siguiente_paso_en = None;
    tx.insertar_evento_visita(&evento(v, TipoEventoVisita::EscaladaLlamada, ctx, ahora))
        .await?;
    avisar_porteria(tx, ctx, v, NovedadPorteria::Llamar).await?;
    Ok(())
}

/// La portería anuncia una visita: aviso al primer respondiente y escalamiento programado. Si
/// nadie del bien tiene app ni WhatsApp, queda de una vez para llamar.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.registrar`; [`AppError::NoEncontrado`] si el bien no
/// existe; [`AppError::Dominio`] con datos inválidos.
pub async fn anunciar_visita<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: NuevaVisita,
) -> AppResult<Visita>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoRegistrar)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    codigo_bien(&mut tx, n.bien_privado_id).await?;
    let mut v = visita_nueva(
        ctx,
        &n,
        OrigenVisita::AnuncioPorteria,
        EstadoVisita::Anunciada,
        ahora,
    )?;
    tx.insertar_visita(&v).await?;
    tx.insertar_evento_visita(&evento(&v, TipoEventoVisita::Anunciada, ctx, ahora))
        .await?;
    let avisados =
        avisar_visita(&mut tx, ctx, &mut v, NivelAviso::PrimerRespondiente, ahora).await?;
    if avisados == 0 {
        pedir_llamada(&mut tx, ctx, &mut v, ahora).await?;
    } else {
        v.siguiente_paso_en = Some(momento_siguiente_paso(v.creada_en, v.avisos_enviados));
    }
    tx.actualizar_visita(&v).await?;
    auditar(&mut tx, ACCION_ANUNCIAR, &v).await?;
    tx.confirmar().await?;
    Ok(v)
}

/// ¿La cuenta del contexto puede decidir por el bien? (residente adulto o, si en el bien no
/// vive nadie de 18 a 59 años, un propietario).
async fn persona_que_decide<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    bien: BienPrivadoId,
    ahora: DateTime<Utc>,
) -> AppResult<PersonaId> {
    ctx.exigir(Permiso::AccesoVisitantesAutorizar)?;
    let persona = persona_del_actor(tx, ctx)
        .await?
        .ok_or(AppError::SinPermiso(Permiso::AccesoVisitantesAutorizar))?;
    let tz = zona_ph(tx).await?;
    let (residentes, titulares) = vinculados(tx, bien).await?;
    if puede_decidir_visita(persona, &residentes, &titulares, hoy(ahora, tz)) {
        Ok(persona)
    } else {
        Err(AppError::SinPermiso(Permiso::AccesoVisitantesAutorizar))
    }
}

/// Texto del QR temporal firmado de una visita pre-autorizada.
pub(crate) fn texto_qr(
    firma: &impl FirmaQr,
    ctx: &Contexto,
    v: &Visita,
) -> AppResult<Option<String>> {
    let (Some(desde), Some(hasta)) = (v.ventana_desde, v.ventana_hasta) else {
        return Ok(None);
    };
    if v.origen != OrigenVisita::Preautorizada {
        return Ok(None);
    }
    let carga = CargaQr::new(ctx.tenant_id, v.id, desde, hasta, v.usos_permitidos)?;
    Ok(Some(
        carga.texto(&firma.firmar(ctx.tenant_id, &carga.bytes())),
    ))
}

fn validar_ventana(p: &NuevaPreautorizacion, ahora: DateTime<Utc>) -> AppResult<()> {
    if p.hasta <= p.desde || p.hasta <= ahora || p.hasta - p.desde > VENTANA_MAXIMA_PREAUTORIZACION
    {
        return Err(AppError::Validacion(
            "la ventana debe terminar en el futuro, después de empezar, y durar a lo sumo 31 días"
                .into(),
        ));
    }
    if p.usos == 0 || p.usos > USOS_MAXIMOS_QR {
        return Err(AppError::Validacion(format!(
            "los usos van de 1 a {USOS_MAXIMOS_QR}"
        )));
    }
    Ok(())
}

/// Crea la pre-autorización dentro de una transacción abierta.
async fn preautorizar_en<T: TxAcceso>(
    tx: &mut T,
    firma: &impl FirmaQr,
    ctx: &Contexto,
    persona: PersonaId,
    canal: CanalRespuestaVisita,
    p: &NuevaPreautorizacion,
    ahora: DateTime<Utc>,
) -> AppResult<(Visita, String)> {
    let mut v = visita_nueva(
        ctx,
        &p.visita,
        OrigenVisita::Preautorizada,
        EstadoVisita::Autorizada,
        ahora,
    )?;
    v.decidida_por = Some(persona);
    v.canal_decision = Some(canal);
    v.decidida_en = Some(ahora);
    v.ventana_desde = Some(p.desde);
    v.ventana_hasta = Some(p.hasta);
    v.usos_permitidos = p.usos;
    v.siguiente_paso_en = Some(p.hasta);
    tx.insertar_visita(&v).await?;
    let mut e = evento(&v, TipoEventoVisita::Preautorizada, ctx, ahora);
    e.persona_id = Some(persona);
    e.canal = Some(canal);
    tx.insertar_evento_visita(&e).await?;
    auditar(tx, ACCION_PREAUTORIZAR, &v).await?;
    let qr = texto_qr(firma, ctx, &v)?.ok_or(AppError::Validacion("visita sin ventana".into()))?;
    Ok((v, qr))
}

/// Un residente pre-autoriza una visita: queda autorizada en su ventana y recibe un QR temporal
/// firmado (lo comparte desde la app).
///
/// # Errors
///
/// [`AppError::SinPermiso`] si la cuenta no puede decidir por el bien;
/// [`AppError::Validacion`] si la ventana o los usos están fuera de rango.
pub async fn preautorizar_visita<U>(
    unidad: &U,
    reloj: &impl Reloj,
    firma: &impl FirmaQr,
    ctx: &Contexto,
    p: NuevaPreautorizacion,
) -> AppResult<(Visita, String)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let ahora = reloj.ahora();
    validar_ventana(&p, ahora)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let persona = persona_que_decide(&mut tx, ctx, p.visita.bien_privado_id, ahora).await?;
    let r = preautorizar_en(
        &mut tx,
        firma,
        ctx,
        persona,
        CanalRespuestaVisita::App,
        &p,
        ahora,
    )
    .await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Pre-autorización desde el chat de WhatsApp: la persona ya la identificó el teléfono (no
/// necesita cuenta en la plataforma). Exige que pueda decidir por el bien (residente adulto o,
/// si no vive nadie de 18 a 59 años, propietario).
///
/// # Errors
///
/// [`AppError::SinPermiso`] si la persona no puede decidir por el bien; los de
/// [`preautorizar_visita`].
pub async fn preautorizar_para_persona<U>(
    unidad: &U,
    reloj: &impl Reloj,
    firma: &impl FirmaQr,
    ctx: &Contexto,
    persona: PersonaId,
    p: NuevaPreautorizacion,
) -> AppResult<(Visita, String)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let ahora = reloj.ahora();
    validar_ventana(&p, ahora)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let tz = zona_ph(&mut tx).await?;
    let (residentes, titulares) = vinculados(&mut tx, p.visita.bien_privado_id).await?;
    if !puede_decidir_visita(persona, &residentes, &titulares, hoy(ahora, tz)) {
        return Err(AppError::SinPermiso(Permiso::AccesoVisitantesAutorizar));
    }
    let r = preautorizar_en(
        &mut tx,
        firma,
        ctx,
        persona,
        CanalRespuestaVisita::Whatsapp,
        &p,
        ahora,
    )
    .await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Toma la decisión dentro de una transacción abierta (la usan la app, la web y WhatsApp).
pub(crate) async fn decidir_en<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    id: VisitaId,
    persona: Option<PersonaId>,
    autorizar: bool,
    canal: CanalRespuestaVisita,
    ahora: DateTime<Utc>,
) -> AppResult<ResultadoDecision> {
    let mut v = tx
        .visita_para_actualizar(id)
        .await?
        .ok_or(AppError::NoEncontrado("visita"))?;
    if !v.estado.espera_decision() {
        return Ok(ResultadoDecision::YaDecidida(v));
    }
    let (estado, tipo) = if autorizar {
        (EstadoVisita::Autorizada, TipoEventoVisita::Autorizada)
    } else {
        (EstadoVisita::Rechazada, TipoEventoVisita::Rechazada)
    };
    v.estado = v.estado.transicionar(estado)?;
    v.decidida_por = persona;
    v.canal_decision = Some(canal);
    v.decidida_en = Some(ahora);
    if autorizar {
        v.ventana_desde = Some(ahora);
        v.ventana_hasta = Some(ahora + VIGENCIA_AUTORIZACION_ANUNCIO);
        v.siguiente_paso_en = v.ventana_hasta;
    } else {
        v.siguiente_paso_en = None;
    }
    tx.actualizar_visita(&v).await?;
    let mut e = evento(&v, tipo, ctx, ahora);
    e.canal = Some(canal);
    e.persona_id = persona;
    tx.insertar_evento_visita(&e).await?;
    auditar(tx, ACCION_DECIDIR, &v).await?;
    avisar_porteria(tx, ctx, &v, NovedadPorteria::Decidida).await?;
    Ok(ResultadoDecision::Decidida(v))
}

/// Un residente autoriza o rechaza desde la app o la web. La primera respuesta gana.
///
/// # Errors
///
/// [`AppError::SinPermiso`] si la cuenta no puede decidir por el bien;
/// [`AppError::NoEncontrado`].
pub async fn decidir_visita<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: VisitaId,
    autorizar: bool,
    canal: CanalRespuestaVisita,
) -> AppResult<ResultadoDecision>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let bien = tx
        .visita(id)
        .await?
        .ok_or(AppError::NoEncontrado("visita"))?
        .bien_privado_id;
    let persona = persona_que_decide(&mut tx, ctx, bien, ahora).await?;
    let r = decidir_en(&mut tx, ctx, id, Some(persona), autorizar, canal, ahora).await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Nombres y teléfonos de quienes pueden decidir, para que la portería llame. Solo mientras la
/// visita está en `requiere_llamada`; cada consulta se audita.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.registrar`; [`AppError::Conflicto`] si la visita no
/// requiere llamada.
pub async fn contactos_para_llamar<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: VisitaId,
) -> AppResult<Vec<ContactoLlamada>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoRegistrar)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let v = tx
        .visita(id)
        .await?
        .ok_or(AppError::NoEncontrado("visita"))?;
    if v.estado != EstadoVisita::RequiereLlamada {
        return Err(AppError::Conflicto(
            "la visita no está esperando una llamada".into(),
        ));
    }
    let tz = zona_ph(&mut tx).await?;
    let (residentes, titulares) = vinculados(&mut tx, v.bien_privado_id).await?;
    let mut contactos = Vec::new();
    for p in destinatarios_aviso(NivelAviso::Todos, &residentes, &titulares, hoy(ahora, tz)) {
        if let Some(persona) = tx.persona(p).await?
            && let Some(t) = persona.telefono
        {
            contactos.push(ContactoLlamada {
                persona_id: p,
                nombre: persona.nombre,
                telefono: t.as_str().to_owned(),
            });
        }
    }
    let detalle = DetalleAuditoria::vacio().con("contactos", contactos.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CONTACTOS, "visita", v.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(contactos)
}

/// La portería registra el resultado de la llamada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.registrar`; [`AppError::Conflicto`] si la visita ya se
/// decidió; [`AppError::Validacion`] si la persona no puede decidir por el bien.
pub async fn registrar_llamada<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: VisitaId,
    resultado: ResultadoLlamada,
    persona: Option<PersonaId>,
) -> AppResult<Visita>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoRegistrar)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut v = tx
        .visita_para_actualizar(id)
        .await?
        .ok_or(AppError::NoEncontrado("visita"))?;
    if !v.estado.espera_decision() {
        return Err(AppError::Conflicto("la visita ya fue atendida".into()));
    }
    if let Some(p) = persona {
        let tz = zona_ph(&mut tx).await?;
        let (residentes, titulares) = vinculados(&mut tx, v.bien_privado_id).await?;
        if !puede_decidir_visita(p, &residentes, &titulares, hoy(ahora, tz)) {
            return Err(AppError::Validacion(
                "la persona no puede decidir por ese bien privado".into(),
            ));
        }
    }
    if v.estado == EstadoVisita::Anunciada {
        v.estado = v.estado.transicionar(EstadoVisita::RequiereLlamada)?;
    }
    v.estado = v.estado.transicionar(resultado.estado())?;
    v.decidida_por = persona;
    v.canal_decision = Some(CanalRespuestaVisita::Llamada);
    v.decidida_en = Some(ahora);
    v.siguiente_paso_en = None;
    if resultado == ResultadoLlamada::Autorizo {
        v.ventana_desde = Some(ahora);
        v.ventana_hasta = Some(ahora + VIGENCIA_AUTORIZACION_ANUNCIO);
        v.siguiente_paso_en = v.ventana_hasta;
    }
    tx.actualizar_visita(&v).await?;
    let mut e = evento(&v, TipoEventoVisita::Llamada, ctx, ahora);
    e.canal = Some(CanalRespuestaVisita::Llamada);
    e.persona_id = persona;
    tx.insertar_evento_visita(&e).await?;
    let detalle = DetalleAuditoria::vacio().con("resultado", resultado.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_LLAMADA, "visita", v.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(v)
}

fn exigir_ventana(v: &Visita, ahora: DateTime<Utc>) -> AppResult<()> {
    if v.estado != EstadoVisita::Autorizada {
        return Err(AppError::Conflicto(format!(
            "la visita está {}",
            v.estado.as_str().replace('_', " ")
        )));
    }
    let en_ventana =
        v.ventana_desde.is_none_or(|d| ahora >= d) && v.ventana_hasta.is_none_or(|h| ahora < h);
    if !en_ventana || v.usos >= v.usos_permitidos {
        return Err(AppError::Conflicto(
            "la autorización no está vigente o ya no tiene usos".into(),
        ));
    }
    Ok(())
}

/// Ingreso dentro de una transacción abierta (portería o QR).
pub(crate) async fn ingresar_en<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    v: &mut Visita,
    punto: Option<domain::PuntoAccesoId>,
    ahora: DateTime<Utc>,
) -> AppResult<()> {
    exigir_ventana(v, ahora)?;
    v.estado = v.estado.transicionar(EstadoVisita::Ingresada)?;
    v.usos += 1;
    v.ingreso_en = Some(ahora);
    tx.actualizar_visita(v).await?;
    tx.insertar_evento_visita(&evento(v, TipoEventoVisita::Ingreso, ctx, ahora))
        .await?;
    tx.insertar_evento_acceso(&EventoAcceso {
        id: domain::EventoAccesoId::nuevo(),
        punto_acceso_id: punto,
        dispositivo_id: None,
        puente_id: None,
        id_local: None,
        credencial_id: None,
        visita_id: Some(v.id),
        sentido: SentidoAcceso::Entrada,
        resultado: ResultadoAcceso::Manual,
        motivo: Some("visita autorizada".into()),
        placa: v.placa.clone(),
        registrado_por: ctx.actor_id,
        ocurrido_en: ahora,
    })
    .await?;
    auditar(tx, ACCION_INGRESO, v).await
}

/// La portería registra el ingreso de una visita autorizada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.registrar`; [`AppError::Conflicto`] si no está
/// autorizada, está fuera de su ventana o agotó sus usos.
pub async fn registrar_ingreso<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: VisitaId,
    punto: Option<domain::PuntoAccesoId>,
) -> AppResult<Visita>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoRegistrar)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut v = tx
        .visita_para_actualizar(id)
        .await?
        .ok_or(AppError::NoEncontrado("visita"))?;
    ingresar_en(&mut tx, ctx, &mut v, punto, ahora).await?;
    tx.confirmar().await?;
    Ok(v)
}

/// Ingreso leyendo el QR temporal con la cámara de la tablet: verifica la firma de la PH, la
/// ventana y los usos (invariante 7).
///
/// # Errors
///
/// [`AppError::Validacion`] si el QR no es válido o es de otra copropiedad; los de
/// [`registrar_ingreso`].
pub async fn ingreso_por_qr<U>(
    unidad: &U,
    reloj: &impl Reloj,
    firma: &impl FirmaQr,
    ctx: &Contexto,
    texto: &str,
    punto: Option<domain::PuntoAccesoId>,
) -> AppResult<Visita>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoRegistrar)?;
    let ahora = reloj.ahora();
    let (carga, sello) =
        CargaQr::leer_texto(texto).ok_or(AppError::Validacion("QR no reconocido".into()))?;
    if carga.tenant != ctx.tenant_id || !firma.verificar(ctx.tenant_id, &carga.bytes(), &sello) {
        return Err(AppError::Validacion(
            "QR inválido para esta copropiedad".into(),
        ));
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut v = tx
        .visita_para_actualizar(carga.visita)
        .await?
        .ok_or(AppError::NoEncontrado("visita"))?;
    if !carga.admite(ahora, u32::from(v.usos)) {
        return Err(AppError::Conflicto(
            "el QR está fuera de su ventana o ya no tiene usos".into(),
        ));
    }
    ingresar_en(&mut tx, ctx, &mut v, punto, ahora).await?;
    tx.confirmar().await?;
    Ok(v)
}

/// Salida de la visita. Una pre-autorización con usos y ventana vigentes vuelve a quedar
/// autorizada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.registrar`; [`AppError::Dominio`] si no estaba dentro.
pub async fn registrar_salida<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: VisitaId,
    punto: Option<domain::PuntoAccesoId>,
) -> AppResult<Visita>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoRegistrar)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut v = tx
        .visita_para_actualizar(id)
        .await?
        .ok_or(AppError::NoEncontrado("visita"))?;
    let quedan = v.usos < v.usos_permitidos && v.ventana_hasta.is_some_and(|h| ahora < h);
    let hacia = if quedan {
        EstadoVisita::Autorizada
    } else {
        EstadoVisita::Finalizada
    };
    v.estado = v.estado.transicionar(hacia)?;
    v.salida_en = Some(ahora);
    if !quedan {
        v.siguiente_paso_en = None;
    }
    tx.actualizar_visita(&v).await?;
    tx.insertar_evento_visita(&evento(&v, TipoEventoVisita::Salida, ctx, ahora))
        .await?;
    tx.insertar_evento_acceso(&EventoAcceso {
        id: domain::EventoAccesoId::nuevo(),
        punto_acceso_id: punto,
        dispositivo_id: None,
        puente_id: None,
        id_local: None,
        credencial_id: None,
        visita_id: Some(v.id),
        sentido: SentidoAcceso::Salida,
        resultado: ResultadoAcceso::Manual,
        motivo: None,
        placa: v.placa.clone(),
        registrado_por: ctx.actor_id,
        ocurrido_en: ahora,
    })
    .await?;
    auditar(&mut tx, ACCION_SALIDA, &v).await?;
    tx.confirmar().await?;
    Ok(v)
}

/// Cancela una visita que aún no ingresó (el residente que la autorizó o la portería).
///
/// # Errors
///
/// [`AppError::SinPermiso`] si no es la portería ni alguien que puede decidir por el bien;
/// [`AppError::Dominio`] si ya ingresó o terminó.
pub async fn cancelar_visita<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: VisitaId,
) -> AppResult<Visita>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut v = tx
        .visita_para_actualizar(id)
        .await?
        .ok_or(AppError::NoEncontrado("visita"))?;
    if !tiene(ctx, Permiso::AccesoRegistrar) {
        persona_que_decide(&mut tx, ctx, v.bien_privado_id, ahora).await?;
    }
    v.estado = v.estado.transicionar(EstadoVisita::Cancelada)?;
    v.siguiente_paso_en = None;
    tx.actualizar_visita(&v).await?;
    tx.insertar_evento_visita(&evento(&v, TipoEventoVisita::Cancelada, ctx, ahora))
        .await?;
    auditar(&mut tx, ACCION_CANCELAR, &v).await?;
    tx.confirmar().await?;
    Ok(v)
}

/// ¿Puede la cuenta ver las visitas del bien? La portería y la bitácora ven todo; un residente
/// o propietario, las de sus bienes.
async fn exigir_ver_bien<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    bien: Option<BienPrivadoId>,
) -> AppResult<()> {
    if tiene(ctx, Permiso::AccesoRegistrar) || tiene(ctx, Permiso::AccesoBitacoraVer) {
        return Ok(());
    }
    ctx.exigir(Permiso::AccesoVisitantesAutorizar)?;
    let (Some(b), Some(cuenta)) = (bien, ctx.actor_id) else {
        return Err(AppError::SinPermiso(Permiso::AccesoBitacoraVer));
    };
    if tx.cuenta_vinculada_a_bien(cuenta, b).await? {
        Ok(())
    } else {
        Err(AppError::SinPermiso(Permiso::AccesoBitacoraVer))
    }
}

/// Visitas (más recientes primero). Sin `acceso.bitacora.ver` hay que indicar un bien propio.
///
/// # Errors
///
/// [`AppError::SinPermiso`].
pub async fn listar_visitas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroVisitas,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Visita>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    exigir_ver_bien(&mut tx, ctx, filtro.bien_privado_id).await?;
    tx.visitas(&filtro, pagina).await
}

/// Visita con su historial.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`].
pub async fn ver_visita<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: VisitaId,
) -> AppResult<DetalleVisita>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let visita = tx
        .visita(id)
        .await?
        .ok_or(AppError::NoEncontrado("visita"))?;
    exigir_ver_bien(&mut tx, ctx, Some(visita.bien_privado_id)).await?;
    let eventos = tx.eventos_visita(id).await?;
    Ok(DetalleVisita { visita, eventos })
}

/// QR de una visita pre-autorizada (la firma Ed25519 es determinista: se regenera igual).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::Conflicto`] si no es una pre-autorización vigente.
pub async fn qr_de_visita<U>(
    unidad: &U,
    reloj: &impl Reloj,
    firma: &impl FirmaQr,
    ctx: &Contexto,
    id: VisitaId,
) -> AppResult<String>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let v = tx
        .visita(id)
        .await?
        .ok_or(AppError::NoEncontrado("visita"))?;
    exigir_ver_bien(&mut tx, ctx, Some(v.bien_privado_id)).await?;
    if v.estado != EstadoVisita::Autorizada {
        return Err(AppError::Conflicto(
            "la visita no tiene un QR vigente".into(),
        ));
    }
    texto_qr(firma, ctx, &v)?.ok_or(AppError::Conflicto(
        "la visita no tiene un QR vigente".into(),
    ))
}

/// Personas del bien que pueden decidir (para validar el teléfono de una respuesta de WhatsApp).
pub(crate) async fn persona_por_telefono_del_bien<T: TxAcceso>(
    tx: &mut T,
    bien: BienPrivadoId,
    telefono: &str,
    ahora: DateTime<Utc>,
) -> AppResult<Option<PersonaId>> {
    let tz = zona_ph(tx).await?;
    let (residentes, titulares) = vinculados(tx, bien).await?;
    for p in destinatarios_aviso(NivelAviso::Todos, &residentes, &titulares, hoy(ahora, tz)) {
        if tx
            .persona(p)
            .await?
            .and_then(|x| x.telefono)
            .is_some_and(|t| t.as_str() == telefono)
        {
            return Ok(Some(p));
        }
    }
    Ok(None)
}
