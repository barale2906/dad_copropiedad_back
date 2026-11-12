//! Casos de uso de protección de datos personales (Ley 1581 de 2012).
//!
//! - [`publicar_politica`], [`politica_vigente`], [`historial_politicas`]: la política de
//!   tratamiento de la copropiedad (versionada).
//! - [`registrar_autorizacion`] / [`revocar_autorizacion`] / [`autorizaciones_de`]:
//!   autorizaciones por finalidad.
//! - [`esta_autorizado`]: lo consultan los módulos antes de tratar datos para una finalidad
//!   (p. ej. publicidad o comunicaciones).
//! - [`radicar_solicitud`], [`tramitar_solicitud`], [`prorrogar_solicitud`],
//!   [`responder_solicitud`], [`listar_solicitudes`]: consultas y reclamos del titular con su
//!   vencimiento legal en días hábiles de Colombia.
//! - [`mis_datos`]: la consulta del titular (art. 14) sobre lo que la PH guarda de él.
//!
//! **Quién actúa**: el titular sobre sí mismo (su persona en la PH, vinculada a su cuenta) o la
//! administración con `proteccion_datos.gestionar`. La auditoría guarda ids, finalidades y
//! fechas, nunca los datos personales.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, AutorizacionId, EstadoSolicitudTitular, FinalidadTratamiento, MedioAutorizacion,
    Permiso, PersonaId, PoliticaTratamientoId, SolicitudTitularId, TipoSolicitudTitular,
    copropiedad::{texto_obligatorio, texto_opcional},
    proteccion_datos::{fecha_en_colombia, prorrogar_solicitud as nueva_prorroga},
};
use serde_json::Value;

use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioArchivos, RepositorioPersonas,
        RepositorioProteccionDatos, RepositorioTickets, Transaccion, UnidadDeTrabajo,
        copropiedades::Persona,
        proteccion_datos::{
            Autorizacion, AutorizacionRegistrada, FiltroSolicitudes, PoliticaTratamiento,
            SolicitudTitular,
        },
    },
};

pub(crate) const ACCION_PUBLICAR_POLITICA: AccionAuditoria =
    AccionAuditoria::new("proteccion_datos.politica.publicar");
pub(crate) const ACCION_OTORGAR: AccionAuditoria =
    AccionAuditoria::new("proteccion_datos.autorizacion.otorgar");
pub(crate) const ACCION_REVOCAR: AccionAuditoria =
    AccionAuditoria::new("proteccion_datos.autorizacion.revocar");
pub(crate) const ACCION_RADICAR: AccionAuditoria =
    AccionAuditoria::new("proteccion_datos.solicitud.radicar");
pub(crate) const ACCION_TRAMITAR: AccionAuditoria =
    AccionAuditoria::new("proteccion_datos.solicitud.tramitar");
pub(crate) const ACCION_PRORROGAR: AccionAuditoria =
    AccionAuditoria::new("proteccion_datos.solicitud.prorrogar");
pub(crate) const ACCION_RESPONDER: AccionAuditoria =
    AccionAuditoria::new("proteccion_datos.solicitud.responder");
pub(crate) const ACCION_CONSULTAR: AccionAuditoria =
    AccionAuditoria::new("proteccion_datos.titular.consultar");

/// Puertos que usan los casos de uso de protección de datos.
pub trait TxProteccionDatos:
    RepositorioProteccionDatos
    + RepositorioPersonas
    + RepositorioTickets
    + RepositorioArchivos
    + RegistroAuditoria
{
}

impl<T> TxProteccionDatos for T where
    T: RepositorioProteccionDatos
        + RepositorioPersonas
        + RepositorioTickets
        + RepositorioArchivos
        + RegistroAuditoria
{
}

fn gestiona(ctx: &Contexto) -> bool {
    ctx.exigir(Permiso::ProteccionDatosGestionar).is_ok()
}

/// Exige que el actor sea el titular `persona` o tenga `proteccion_datos.gestionar`.
async fn exigir_titular_o_gestion<T: TxProteccionDatos>(
    tx: &mut T,
    ctx: &Contexto,
    persona: PersonaId,
) -> AppResult<()> {
    if gestiona(ctx) {
        return Ok(());
    }
    let propia = match ctx.actor_id {
        Some(c) => tx.persona_de_cuenta(c).await? == Some(persona),
        None => false,
    };
    if propia {
        Ok(())
    } else {
        Err(AppError::SinPermiso(Permiso::ProteccionDatosGestionar))
    }
}

/// Persona de la PH vinculada a la cuenta del actor.
async fn persona_del_actor<T: TxProteccionDatos>(
    tx: &mut T,
    ctx: &Contexto,
) -> AppResult<PersonaId> {
    match ctx.actor_id {
        Some(c) => tx
            .persona_de_cuenta(c)
            .await?
            .ok_or(AppError::NoEncontrado("persona vinculada a la cuenta")),
        None => Err(AppError::NoAutenticado),
    }
}

/// Contenido de una política nueva: texto, archivo (PDF) o ambos.
#[derive(Debug, Clone)]
pub struct NuevaPolitica {
    pub texto: Option<String>,
    pub archivo_id: Option<ArchivoId>,
    /// `None` = desde ahora.
    pub vigente_desde: Option<DateTime<Utc>>,
}

/// Publica una versión nueva de la política (la siguiente a la última).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `proteccion_datos.gestionar`; [`AppError::Validacion`] si no
/// trae ni texto ni archivo, o si `vigente_desde` es pasado.
pub async fn publicar_politica<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaPolitica,
) -> AppResult<PoliticaTratamiento>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxProteccionDatos,
{
    ctx.exigir(Permiso::ProteccionDatosGestionar)?;
    let texto = nueva.texto.filter(|t| !t.trim().is_empty());
    if texto.is_none() && nueva.archivo_id.is_none() {
        return Err(AppError::Validacion(
            "la política necesita un texto o un archivo".into(),
        ));
    }
    let ahora = reloj.ahora();
    let vigente_desde = nueva.vigente_desde.unwrap_or(ahora);
    if vigente_desde < ahora {
        // Las autorizaciones ya otorgadas se dieron bajo otra versión: no se reescribe el pasado.
        return Err(AppError::Validacion(
            "la vigencia de una política no puede empezar en el pasado".into(),
        ));
    }

    let mut tx = unidad.iniciar(ctx).await?;
    if let Some(a) = nueva.archivo_id
        && tx.buscar(a).await?.is_none()
    {
        return Err(AppError::NoEncontrado("archivo"));
    }
    let politica = PoliticaTratamiento {
        id: PoliticaTratamientoId::nuevo(),
        version: tx.ultima_version_politica().await? + 1,
        texto,
        archivo_id: nueva.archivo_id,
        vigente_desde,
    };
    tx.insertar_politica(&politica).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("version", politica.version)?
        .con("vigente_desde", politica.vigente_desde.to_rfc3339())?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_PUBLICAR_POLITICA,
            "politica_tratamiento",
            politica.id.as_uuid(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(politica)
}

/// Política vigente: la puede leer cualquier cuenta de la PH (se lee antes de autorizar).
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn politica_vigente<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Option<PoliticaTratamiento>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxProteccionDatos,
{
    let mut tx = unidad.iniciar(ctx).await?;
    tx.politica_vigente(reloj.ahora()).await
}

/// Todas las versiones publicadas, la más reciente primero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `proteccion_datos.gestionar`.
pub async fn historial_politicas<U>(
    unidad: &U,
    ctx: &Contexto,
) -> AppResult<Vec<PoliticaTratamiento>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxProteccionDatos,
{
    ctx.exigir(Permiso::ProteccionDatosGestionar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    tx.historial_politicas().await
}

/// Autorización que llega del titular.
#[derive(Debug, Clone)]
pub struct AutorizacionOtorgada {
    pub persona_id: PersonaId,
    pub finalidades: BTreeSet<FinalidadTratamiento>,
    pub medio: MedioAutorizacion,
    pub evidencia_archivo_id: Option<ArchivoId>,
}

/// Registra la autorización del titular para cada finalidad, bajo la política vigente. Las
/// finalidades que ya estaban autorizadas no se duplican. Devuelve las que quedaron nuevas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] si no es el titular ni tiene `proteccion_datos.gestionar`;
/// [`AppError::Validacion`] si no hay finalidades o la copropiedad no tiene política vigente
/// (sin política no hay autorización informada, art. 12).
pub async fn registrar_autorizacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    otorgada: AutorizacionOtorgada,
) -> AppResult<Vec<FinalidadTratamiento>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxProteccionDatos,
{
    if otorgada.finalidades.is_empty() {
        return Err(AppError::Validacion(
            "indique al menos una finalidad".into(),
        ));
    }
    // La publicidad es de la plataforma (ADR 0015): la autoriza el titular frente a ella, con
    // la política de la plataforma; ni la administración ni la política de la PH la cubren.
    if otorgada
        .finalidades
        .contains(&FinalidadTratamiento::Publicidad)
    {
        return Err(AppError::Validacion(
            "la autorización de publicidad la da el titular a la plataforma, no la copropiedad"
                .into(),
        ));
    }
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_titular_o_gestion(&mut tx, ctx, otorgada.persona_id).await?;
    let politica = tx.politica_vigente(ahora).await?.ok_or_else(|| {
        AppError::Validacion("la copropiedad no tiene una política de tratamiento vigente".into())
    })?;

    let mut nuevas = Vec::new();
    for finalidad in otorgada.finalidades {
        let autorizacion = Autorizacion {
            id: AutorizacionId::nuevo(),
            persona_id: otorgada.persona_id,
            politica_id: Some(politica.id),
            finalidad,
            medio: otorgada.medio,
            evidencia_archivo_id: otorgada.evidencia_archivo_id,
            otorgada_en: ahora,
        };
        if tx.insertar_autorizacion(&autorizacion).await? {
            nuevas.push(finalidad);
        }
    }
    if !nuevas.is_empty() {
        let detalle = DetalleAuditoria::vacio()
            .con("finalidades", nombres(&nuevas))?
            .con("medio", otorgada.medio.as_str())?
            .con("politica_version", politica.version)?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_OTORGAR, "persona", otorgada.persona_id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
    }
    tx.confirmar().await?;
    Ok(nuevas)
}

/// Revoca la autorización del titular para esas finalidades (art. 8, lit. e). Devuelve cuántas
/// autorizaciones vigentes se revocaron.
///
/// # Errors
///
/// [`AppError::SinPermiso`] si no es el titular ni tiene `proteccion_datos.gestionar`;
/// [`AppError::Validacion`] si no se indica ninguna finalidad.
pub async fn revocar_autorizacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    persona_id: PersonaId,
    finalidades: &[FinalidadTratamiento],
) -> AppResult<u64>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxProteccionDatos,
{
    if finalidades.is_empty() {
        return Err(AppError::Validacion(
            "indique al menos una finalidad".into(),
        ));
    }
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_titular_o_gestion(&mut tx, ctx, persona_id).await?;
    let revocadas = tx
        .revocar_autorizaciones(persona_id, finalidades, reloj.ahora())
        .await?;
    if revocadas > 0 {
        let detalle = DetalleAuditoria::vacio().con("finalidades", nombres(finalidades))?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_REVOCAR, "persona", persona_id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
    }
    tx.confirmar().await?;
    Ok(revocadas)
}

/// Autorizaciones (vigentes y revocadas) de la persona.
///
/// # Errors
///
/// [`AppError::SinPermiso`] si no es el titular ni tiene `proteccion_datos.gestionar`.
pub async fn autorizaciones_de<U>(
    unidad: &U,
    ctx: &Contexto,
    persona_id: PersonaId,
) -> AppResult<Vec<AutorizacionRegistrada>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxProteccionDatos,
{
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_titular_o_gestion(&mut tx, ctx, persona_id).await?;
    tx.autorizaciones_de_persona(persona_id).await
}

/// ¿La persona tiene autorización vigente para la finalidad?
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn esta_autorizado<U>(
    unidad: &U,
    ctx: &Contexto,
    persona_id: PersonaId,
    finalidad: FinalidadTratamiento,
) -> AppResult<bool>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioProteccionDatos,
{
    let mut tx = unidad.iniciar(ctx).await?;
    Ok(tx
        .finalidades_autorizadas(persona_id)
        .await?
        .contains(&finalidad))
}

/// Radica una consulta o un reclamo del titular y calcula su vencimiento legal (10 o 15 días
/// hábiles desde el día de radicación en Colombia).
///
/// # Errors
///
/// [`AppError::SinPermiso`] si no es el titular ni tiene `proteccion_datos.gestionar`;
/// [`AppError::Dominio`] si el detalle es inválido; [`AppError::Infraestructura`] si falla la
/// base de datos o el vencimiento se sale del calendario.
pub async fn radicar_solicitud<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    persona_id: PersonaId,
    tipo: TipoSolicitudTitular,
    detalle: Option<String>,
) -> AppResult<SolicitudTitular>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxProteccionDatos,
{
    let detalle = texto_opcional("detalle", detalle.as_deref(), 2000)?;
    let radicada_en = reloj.ahora();
    let vence_el = tipo
        .vencimiento(fecha_en_colombia(radicada_en))
        .ok_or_else(|| AppError::infra(std::io::Error::other("vencimiento fuera de rango")))?;
    let solicitud = SolicitudTitular {
        id: SolicitudTitularId::nuevo(),
        persona_id,
        tipo,
        radicada_en,
        vence_el,
        estado: EstadoSolicitudTitular::Radicada,
        detalle,
        radicada_por: ctx.actor_id,
        prorrogada_hasta: None,
        motivo_prorroga: None,
        respuesta: None,
        respuesta_archivo_id: None,
        respondida_en: None,
        respondida_por: None,
    };

    let mut tx = unidad.iniciar(ctx).await?;
    exigir_titular_o_gestion(&mut tx, ctx, persona_id).await?;
    tx.insertar_solicitud(&solicitud).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", tipo.as_str())?
        .con("vence_el", vence_el.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RADICAR, "solicitud_titular", solicitud.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(solicitud)
}

async fn solicitud_de<T: TxProteccionDatos>(
    tx: &mut T,
    id: SolicitudTitularId,
) -> AppResult<SolicitudTitular> {
    tx.solicitud_para_actualizar(id)
        .await?
        .ok_or(AppError::NoEncontrado("solicitud"))
}

async fn auditar_solicitud<T: TxProteccionDatos>(
    tx: &mut T,
    accion: AccionAuditoria,
    s: &SolicitudTitular,
) -> AppResult<()> {
    let detalle = DetalleAuditoria::vacio()
        .con("estado", s.estado.as_str())?
        .con("vence", s.vence().to_string())?;
    tx.registrar(
        EventoAuditoria::ok(accion, "solicitud_titular", s.id.as_uuid()).con_detalle(detalle),
    )
    .await
}

/// Pasa la solicitud a `en_tramite` (la administración la está atendiendo).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `proteccion_datos.gestionar`; [`AppError::Dominio`] si ya se
/// respondió.
pub async fn tramitar_solicitud<U>(
    unidad: &U,
    ctx: &Contexto,
    id: SolicitudTitularId,
) -> AppResult<SolicitudTitular>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxProteccionDatos,
{
    ctx.exigir(Permiso::ProteccionDatosGestionar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut s = solicitud_de(&mut tx, id).await?;
    s.estado = s.estado.transicionar(EstadoSolicitudTitular::EnTramite)?;
    tx.actualizar_solicitud(&s).await?;
    auditar_solicitud(&mut tx, ACCION_TRAMITAR, &s).await?;
    tx.confirmar().await?;
    Ok(s)
}

/// Prorroga la solicitud una sola vez, antes del vencimiento y con motivo (arts. 14 y 15: 5
/// días hábiles más para consultas, 8 para reclamos). El motivo se informa al titular.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `proteccion_datos.gestionar`; [`AppError::Dominio`] si ya se
/// prorrogó, ya se respondió, el plazo venció o el motivo es inválido.
pub async fn prorrogar_solicitud<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: SolicitudTitularId,
    motivo: &str,
) -> AppResult<SolicitudTitular>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxProteccionDatos,
{
    ctx.exigir(Permiso::ProteccionDatosGestionar)?;
    let motivo = texto_obligatorio("motivo", motivo, 500)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut s = solicitud_de(&mut tx, id).await?;
    let hasta = nueva_prorroga(
        s.tipo,
        s.estado,
        fecha_en_colombia(s.radicada_en),
        s.vence_el,
        s.prorrogada_hasta.is_some(),
        fecha_en_colombia(reloj.ahora()),
    )?;
    s.prorrogada_hasta = Some(hasta);
    s.motivo_prorroga = Some(motivo);
    tx.actualizar_solicitud(&s).await?;
    auditar_solicitud(&mut tx, ACCION_PRORROGAR, &s).await?;
    tx.confirmar().await?;
    Ok(s)
}

/// Responde la solicitud con un texto, un archivo o ambos. En supresión y revocatoria la
/// administración ejecuta el cambio en el censo y deja constancia aquí.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `proteccion_datos.gestionar`; [`AppError::Validacion`] sin
/// texto ni archivo; [`AppError::NoEncontrado`] si el archivo no existe; [`AppError::Dominio`]
/// si ya se respondió.
pub async fn responder_solicitud<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: SolicitudTitularId,
    respuesta: Option<String>,
    archivo: Option<ArchivoId>,
) -> AppResult<SolicitudTitular>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxProteccionDatos,
{
    ctx.exigir(Permiso::ProteccionDatosGestionar)?;
    let respuesta = texto_opcional("respuesta", respuesta.as_deref(), 5000)?;
    if respuesta.is_none() && archivo.is_none() {
        return Err(AppError::Validacion(
            "la respuesta necesita un texto o un archivo".into(),
        ));
    }
    let mut tx = unidad.iniciar(ctx).await?;
    if let Some(a) = archivo
        && tx.buscar(a).await?.is_none()
    {
        return Err(AppError::NoEncontrado("archivo"));
    }
    let mut s = solicitud_de(&mut tx, id).await?;
    s.estado = s.estado.transicionar(EstadoSolicitudTitular::Respondida)?;
    s.respuesta = respuesta;
    s.respuesta_archivo_id = archivo;
    s.respondida_en = Some(reloj.ahora());
    s.respondida_por = ctx.actor_id;
    tx.actualizar_solicitud(&s).await?;
    auditar_solicitud(&mut tx, ACCION_RESPONDER, &s).await?;
    tx.confirmar().await?;
    Ok(s)
}

/// Solicitudes de la PH. La administración ve todas; el titular, las suyas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] si pide las de otra persona sin `proteccion_datos.gestionar`.
pub async fn listar_solicitudes<U>(
    unidad: &U,
    ctx: &Contexto,
    mut filtro: FiltroSolicitudes,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<SolicitudTitular>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxProteccionDatos,
{
    let mut tx = unidad.iniciar(ctx).await?;
    if !gestiona(ctx) {
        let propia = persona_del_actor(&mut tx, ctx).await?;
        if filtro.persona_id.is_some_and(|p| p != propia) {
            return Err(AppError::SinPermiso(Permiso::ProteccionDatosGestionar));
        }
        filtro.persona_id = Some(propia);
    }
    tx.solicitudes(&filtro, pagina).await
}

/// Lo que la PH trata de un titular (art. 14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosDelTitular {
    pub persona: Persona,
    pub autorizaciones: Vec<AutorizacionRegistrada>,
    pub solicitudes: Vec<SolicitudTitular>,
}

/// Consulta del titular: su ficha en la PH, sus autorizaciones y sus solicitudes. Sin
/// `persona` es la del actor; con ella, exige `proteccion_datos.gestionar`. Se audita.
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si la cuenta no tiene persona en la PH; [`AppError::SinPermiso`].
pub async fn mis_datos<U>(
    unidad: &U,
    ctx: &Contexto,
    persona: Option<PersonaId>,
) -> AppResult<DatosDelTitular>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxProteccionDatos,
{
    let mut tx = unidad.iniciar(ctx).await?;
    let id = match persona {
        Some(p) => {
            exigir_titular_o_gestion(&mut tx, ctx, p).await?;
            p
        }
        None => persona_del_actor(&mut tx, ctx).await?,
    };
    let ficha = tx
        .persona(id)
        .await?
        .ok_or(AppError::NoEncontrado("persona"))?;
    let autorizaciones = tx.autorizaciones_de_persona(id).await?;
    let solicitudes = tx
        .solicitudes(
            &FiltroSolicitudes {
                persona_id: Some(id),
                ..FiltroSolicitudes::default()
            },
            SolicitudPagina::new(Some(200), None)?,
        )
        .await?
        .datos;
    tx.registrar(EventoAuditoria::ok(
        ACCION_CONSULTAR,
        "persona",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(DatosDelTitular {
        persona: ficha,
        autorizaciones,
        solicitudes,
    })
}

fn nombres(finalidades: &[FinalidadTratamiento]) -> Value {
    Value::Array(
        finalidades
            .iter()
            .map(|f| Value::from(f.as_str()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone};
    use domain::TenantId;

    use super::*;
    use crate::puertos::unidad_trabajo::memoria::UnidadEnMemoria;

    /// Viernes 30 de octubre de 2026, 3 p. m. en Colombia.
    struct RelojFijo;

    impl Reloj for RelojFijo {
        fn ahora(&self) -> DateTime<Utc> {
            Utc.with_ymd_and_hms(2026, 10, 30, 20, 0, 0)
                .single()
                .expect("fecha válida")
        }
    }

    fn politica_de_texto() -> NuevaPolitica {
        NuevaPolitica {
            texto: Some("Política de tratamiento de datos del Conjunto Alameda".into()),
            archivo_id: None,
            vigente_desde: None,
        }
    }

    fn otorgada(persona: PersonaId, f: &[FinalidadTratamiento]) -> AutorizacionOtorgada {
        AutorizacionOtorgada {
            persona_id: persona,
            finalidades: f.iter().copied().collect(),
            medio: MedioAutorizacion::WhatsApp,
            evidencia_archivo_id: None,
        }
    }

    async fn con_politica() -> (UnidadEnMemoria, Contexto) {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        publicar_politica(&unidad, &RelojFijo, &ctx, politica_de_texto())
            .await
            .expect("publica");
        (unidad, ctx)
    }

    #[tokio::test]
    async fn cada_politica_publicada_es_una_version_nueva() {
        let (unidad, ctx) = con_politica().await;

        let segunda = publicar_politica(&unidad, &RelojFijo, &ctx, politica_de_texto())
            .await
            .expect("publica");

        assert_eq!(segunda.version, 2);
        assert_eq!(unidad.datos().politicas.len(), 2);
        let acciones: Vec<_> = unidad.confirmados().iter().map(|e| e.1.accion).collect();
        assert_eq!(acciones, vec![ACCION_PUBLICAR_POLITICA; 2]);
    }

    #[tokio::test]
    async fn politica_vacia_o_con_vigencia_pasada_se_rechaza() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let vacia = NuevaPolitica {
            texto: Some("   ".into()),
            archivo_id: None,
            vigente_desde: None,
        };
        let pasada = NuevaPolitica {
            vigente_desde: Some(RelojFijo.ahora() - chrono::Duration::days(1)),
            ..politica_de_texto()
        };

        for nueva in [vacia, pasada] {
            let err = publicar_politica(&unidad, &RelojFijo, &ctx, nueva)
                .await
                .expect_err("se rechaza");
            assert!(matches!(err, AppError::Validacion(_)));
        }
        assert!(unidad.datos().politicas.is_empty());
    }

    #[tokio::test]
    async fn sin_politica_vigente_no_se_puede_autorizar() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        // Una política que empieza a regir mañana todavía no sirve.
        publicar_politica(
            &unidad,
            &RelojFijo,
            &ctx,
            NuevaPolitica {
                vigente_desde: Some(RelojFijo.ahora() + chrono::Duration::days(1)),
                ..politica_de_texto()
            },
        )
        .await
        .expect("publica");

        let err = registrar_autorizacion(
            &unidad,
            &RelojFijo,
            &ctx,
            otorgada(PersonaId::nuevo(), &[FinalidadTratamiento::Comunicaciones]),
        )
        .await
        .expect_err("sin política vigente");

        assert!(matches!(err, AppError::Validacion(_)));
    }

    #[tokio::test]
    async fn la_copropiedad_no_autoriza_publicidad_por_el_titular() {
        let (unidad, ctx) = con_politica().await;

        let r = registrar_autorizacion(
            &unidad,
            &RelojFijo,
            &ctx,
            otorgada(PersonaId::nuevo(), &[FinalidadTratamiento::Publicidad]),
        )
        .await;

        assert!(matches!(r, Err(AppError::Validacion(_))));
    }

    #[tokio::test]
    async fn autorizacion_por_finalidad_sin_duplicados_y_auditada_sin_datos_personales() {
        let (unidad, ctx) = con_politica().await;
        let persona = PersonaId::nuevo();
        use FinalidadTratamiento::*;

        let primeras = registrar_autorizacion(
            &unidad,
            &RelojFijo,
            &ctx,
            otorgada(persona, &[Administracion, Comunicaciones]),
        )
        .await
        .expect("autoriza");
        let repetidas = registrar_autorizacion(
            &unidad,
            &RelojFijo,
            &ctx,
            otorgada(persona, &[Comunicaciones, ControlAcceso]),
        )
        .await
        .expect("autoriza");

        assert_eq!(primeras, vec![Administracion, Comunicaciones]);
        assert_eq!(repetidas, vec![ControlAcceso], "comunicaciones ya estaba");
        assert_eq!(unidad.datos().autorizaciones.len(), 3);
        assert!(
            esta_autorizado(&unidad, &ctx, persona, ControlAcceso)
                .await
                .expect("consulta")
        );
        assert!(
            !esta_autorizado(&unidad, &ctx, persona, Publicidad)
                .await
                .expect("consulta")
        );
        let ultimo = unidad.confirmados().pop().expect("auditado").1;
        assert_eq!(ultimo.accion, ACCION_OTORGAR);
        assert_eq!(
            ultimo.detalle.a_json()["finalidades"],
            serde_json::json!(["control_acceso"])
        );
    }

    #[tokio::test]
    async fn revocar_quita_solo_las_finalidades_indicadas() {
        let (unidad, ctx) = con_politica().await;
        let persona = PersonaId::nuevo();
        use FinalidadTratamiento::*;
        registrar_autorizacion(
            &unidad,
            &RelojFijo,
            &ctx,
            otorgada(persona, &[Comunicaciones, ControlAcceso]),
        )
        .await
        .expect("autoriza");

        let revocadas = revocar_autorizacion(&unidad, &RelojFijo, &ctx, persona, &[ControlAcceso])
            .await
            .expect("revoca");
        let otra_vez = revocar_autorizacion(&unidad, &RelojFijo, &ctx, persona, &[ControlAcceso])
            .await
            .expect("revoca");

        assert_eq!((revocadas, otra_vez), (1, 0));
        assert!(
            !esta_autorizado(&unidad, &ctx, persona, ControlAcceso)
                .await
                .expect("consulta")
        );
        assert!(
            esta_autorizado(&unidad, &ctx, persona, Comunicaciones)
                .await
                .expect("consulta")
        );
        // Se vuelve a autorizar después de revocar: queda una autorización nueva.
        let nuevas = registrar_autorizacion(
            &unidad,
            &RelojFijo,
            &ctx,
            otorgada(persona, &[ControlAcceso]),
        )
        .await
        .expect("autoriza");
        assert_eq!(nuevas, vec![ControlAcceso]);
    }

    #[tokio::test]
    async fn solicitud_vence_en_dias_habiles_de_colombia() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let persona = PersonaId::nuevo();

        let consulta = radicar_solicitud(
            &unidad,
            &RelojFijo,
            &ctx,
            persona,
            TipoSolicitudTitular::Consulta,
            None,
        )
        .await
        .expect("radica");
        let supresion = radicar_solicitud(
            &unidad,
            &RelojFijo,
            &ctx,
            persona,
            TipoSolicitudTitular::Supresion,
            None,
        )
        .await
        .expect("radica");

        assert_eq!(
            consulta.vence_el,
            NaiveDate::from_ymd_opt(2026, 11, 17).expect("fecha")
        );
        assert_eq!(
            supresion.vence_el,
            NaiveDate::from_ymd_opt(2026, 11, 24).expect("fecha")
        );
        assert_eq!(unidad.datos().solicitudes.len(), 2);
        assert_eq!(unidad.confirmados()[0].1.accion, ACCION_RADICAR);
    }

    /// PH con política vigente y una titular con cuenta.
    async fn con_titular() -> (UnidadEnMemoria, Contexto, PersonaId, Contexto) {
        let (unidad, admin) = con_politica().await;
        let persona = PersonaId::nuevo();
        let cuenta = domain::CuentaId::nuevo();
        unidad.con_datos(|d| {
            let mut p = Persona::basica(persona, domain::TipoPersona::Natural, "Ana".into(), None);
            p.cuenta_id = Some(cuenta);
            d.personas.push(p);
        });
        let titular =
            Contexto::de_cuenta(admin.tenant_id, cuenta, domain::ConjuntoPermisos::vacio());
        (unidad, admin, persona, titular)
    }

    #[tokio::test]
    async fn el_titular_actua_sobre_si_mismo_y_nadie_mas_sin_permiso() {
        use FinalidadTratamiento::*;
        let (unidad, _, persona, titular) = con_titular().await;

        let nuevas = registrar_autorizacion(
            &unidad,
            &RelojFijo,
            &titular,
            otorgada(persona, &[Comunicaciones]),
        )
        .await
        .expect("el titular autoriza");
        assert_eq!(nuevas, vec![Comunicaciones]);
        let ajeno = Contexto::de_cuenta(
            titular.tenant_id,
            domain::CuentaId::nuevo(),
            domain::ConjuntoPermisos::vacio(),
        );
        for r in [
            registrar_autorizacion(
                &unidad,
                &RelojFijo,
                &ajeno,
                otorgada(persona, &[ControlAcceso]),
            )
            .await
            .map(|_| ()),
            revocar_autorizacion(&unidad, &RelojFijo, &ajeno, persona, &[Comunicaciones])
                .await
                .map(|_| ()),
            radicar_solicitud(
                &unidad,
                &RelojFijo,
                &ajeno,
                persona,
                TipoSolicitudTitular::Consulta,
                None,
            )
            .await
            .map(|_| ()),
        ] {
            assert!(matches!(
                r,
                Err(AppError::SinPermiso(Permiso::ProteccionDatosGestionar))
            ));
        }
        let datos = mis_datos(&unidad, &titular, None).await.expect("mis datos");
        assert_eq!(datos.persona.id, persona);
        assert_eq!(datos.autorizaciones.len(), 1);
        assert!(mis_datos(&unidad, &ajeno, Some(persona)).await.is_err());
    }

    #[tokio::test]
    async fn la_solicitud_se_tramita_prorroga_una_vez_y_responde() {
        let (unidad, admin, persona, titular) = con_titular().await;
        let s = radicar_solicitud(
            &unidad,
            &RelojFijo,
            &titular,
            persona,
            TipoSolicitudTitular::Rectificacion,
            Some("Mi teléfono cambió".into()),
        )
        .await
        .expect("radica");
        assert_eq!(s.radicada_por, titular.actor_id);

        tramitar_solicitud(&unidad, &admin, s.id)
            .await
            .expect("trámite");
        let p = prorrogar_solicitud(
            &unidad,
            &RelojFijo,
            &admin,
            s.id,
            "Se verifica con la portería",
        )
        .await
        .expect("prórroga");
        assert!(p.vence() > s.vence_el);
        assert!(
            prorrogar_solicitud(&unidad, &RelojFijo, &admin, s.id, "otra")
                .await
                .is_err(),
            "una sola vez"
        );
        assert!(matches!(
            responder_solicitud(&unidad, &RelojFijo, &admin, s.id, None, None).await,
            Err(AppError::Validacion(_))
        ));
        let r = responder_solicitud(
            &unidad,
            &RelojFijo,
            &admin,
            s.id,
            Some("Actualizado".into()),
            None,
        )
        .await
        .expect("responde");
        assert_eq!(r.estado, EstadoSolicitudTitular::Respondida);
        assert!(
            responder_solicitud(&unidad, &RelojFijo, &admin, s.id, Some("x".into()), None)
                .await
                .is_err()
        );
        assert!(matches!(
            tramitar_solicitud(&unidad, &titular, s.id).await,
            Err(AppError::SinPermiso(_))
        ));
        let mias = listar_solicitudes(
            &unidad,
            &titular,
            FiltroSolicitudes::default(),
            SolicitudPagina::default(),
        )
        .await
        .expect("listar");
        assert_eq!(mias.datos.len(), 1);
    }

    #[tokio::test]
    async fn las_vencidas_se_filtran_por_su_vencimiento_con_prorroga() {
        let (unidad, admin, persona, _) = con_titular().await;
        let s = radicar_solicitud(
            &unidad,
            &RelojFijo,
            &admin,
            persona,
            TipoSolicitudTitular::Consulta,
            None,
        )
        .await
        .expect("radica");
        let filtro = |h| FiltroSolicitudes {
            vencen_hasta: Some(h),
            ..FiltroSolicitudes::default()
        };

        let antes = listar_solicitudes(
            &unidad,
            &admin,
            filtro(s.vence_el.pred_opt().expect("día")),
            SolicitudPagina::default(),
        )
        .await
        .expect("listar");
        let el_dia = listar_solicitudes(
            &unidad,
            &admin,
            filtro(s.vence_el),
            SolicitudPagina::default(),
        )
        .await
        .expect("listar");

        assert!(antes.datos.is_empty());
        assert_eq!(el_dia.datos.len(), 1);
    }
}
