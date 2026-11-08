//! Lado web del chat de WhatsApp (plan 07b, módulo `gestion_administrativa`): preguntas
//! frecuentes que responde el chat y la bandeja de conversaciones escaladas a la
//! administración.
//!
//! Reglas:
//! - Responder con texto libre solo dentro de la ventana de 24 h desde el último mensaje del
//!   residente (regla de Meta); fuera de ella hay que esperar a que escriba de nuevo.
//! - Las respuestas salen por el outbox dentro de la transacción que registra el mensaje.

use domain::{ArchivoId, ConversacionId, Modulo, Permiso, PreguntaFrecuenteId};

use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, EventoAuditoria},
    casos_uso::modulos::exigir_modulo_en,
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        RegistroAuditoria, RegistroOutbox, Reloj, RepositorioChat, RepositorioModulos, Transaccion,
        UnidadDeTrabajo,
        chat::{Conversacion, EstadoConversacion, MensajeConversacion, PreguntaFrecuente},
        mensajeria::{ContenidoSaliente, MensajeSaliente},
    },
};

pub const ACCION_PREGUNTA: AccionAuditoria =
    AccionAuditoria::new("conversaciones.pregunta_frecuente.editar");
pub const ACCION_RESPONDER: AccionAuditoria =
    AccionAuditoria::new("conversaciones.conversacion.responder");
pub const ACCION_CERRAR: AccionAuditoria =
    AccionAuditoria::new("conversaciones.conversacion.cerrar");

/// Puertos de estos casos de uso.
pub trait TxChat:
    RepositorioChat + RepositorioModulos + RegistroOutbox + RegistroAuditoria
{
}

impl<T> TxChat for T where
    T: RepositorioChat + RepositorioModulos + RegistroOutbox + RegistroAuditoria
{
}

async fn abrir<U>(unidad: &U, reloj: &impl Reloj, ctx: &Contexto) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxChat,
{
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::GestionAdministrativa).await?;
    Ok(tx)
}

/// Preguntas frecuentes (las inactivas solo para quien las edita).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `copropiedad.datos.ver`; [`AppError::ModuloNoHabilitado`].
pub async fn listar_preguntas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<PreguntaFrecuente>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxChat,
{
    let edita = ctx.exigir(Permiso::PreguntasFrecuentesEditar).is_ok();
    if !edita {
        ctx.exigir(Permiso::CopropiedadDatosVer)?;
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.preguntas_frecuentes(!edita).await
}

/// Datos de una pregunta frecuente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosPregunta {
    pub pregunta: String,
    pub respuesta: String,
    pub orden: i16,
    pub activa: bool,
    pub archivo_id: Option<ArchivoId>,
}

/// Máximo de caracteres del título: WhatsApp muestra 72 en la descripción de una fila.
pub const MAX_PREGUNTA: usize = 72;

/// Crea (`id = None`) o reemplaza una pregunta frecuente.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `preguntas_frecuentes.editar`; [`AppError::NoEncontrado`];
/// errores de validación.
pub async fn guardar_pregunta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: Option<PreguntaFrecuenteId>,
    d: DatosPregunta,
) -> AppResult<PreguntaFrecuente>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxChat,
{
    use domain::copropiedad::texto_obligatorio;
    ctx.exigir(Permiso::PreguntasFrecuentesEditar)?;
    let p = PreguntaFrecuente {
        id: id.unwrap_or_else(PreguntaFrecuenteId::nuevo),
        pregunta: texto_obligatorio("pregunta", &d.pregunta, MAX_PREGUNTA)?,
        // La respuesta puede tener saltos de línea: se valida solo la longitud.
        respuesta: {
            let r = d.respuesta.trim();
            if r.is_empty() || r.chars().count() > 4000 {
                return Err(AppError::Validacion(
                    "`respuesta` debe tener de 1 a 4000 caracteres".into(),
                ));
            }
            r.to_owned()
        },
        orden: d.orden,
        activa: d.activa,
        archivo_id: d.archivo_id,
    };
    let mut tx = abrir(unidad, reloj, ctx).await?;
    if let Some(id) = id {
        tx.pregunta_frecuente(id)
            .await?
            .ok_or(AppError::NoEncontrado("pregunta frecuente"))?;
    }
    tx.guardar_pregunta_frecuente(&p, id.is_none()).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PREGUNTA,
        "pregunta_frecuente",
        p.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Conversaciones de la bandeja, más recientes primero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `conversaciones.atender`; [`AppError::ModuloNoHabilitado`].
pub async fn listar_conversaciones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    estado: Option<EstadoConversacion>,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Conversacion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxChat,
{
    ctx.exigir(Permiso::ConversacionesAtender)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.conversaciones(estado, pagina).await
}

/// Una conversación con sus mensajes.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `conversaciones.atender`; [`AppError::NoEncontrado`].
pub async fn ver_conversacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ConversacionId,
) -> AppResult<(Conversacion, Vec<MensajeConversacion>)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxChat,
{
    ctx.exigir(Permiso::ConversacionesAtender)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let c = tx
        .conversacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("conversación"))?;
    let mensajes = tx.mensajes_conversacion(id).await?;
    Ok((c, mensajes))
}

/// Responde desde la web: el mensaje sale por WhatsApp (outbox) y queda en la conversación.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `conversaciones.atender`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si la conversación está cerrada o pasó la ventana de 24 h.
pub async fn responder_conversacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ConversacionId,
    texto: String,
) -> AppResult<MensajeConversacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxChat,
{
    ctx.exigir(Permiso::ConversacionesAtender)?;
    let texto = texto.trim().to_owned();
    if texto.is_empty() || texto.chars().count() > 4096 {
        return Err(AppError::Validacion(
            "`texto` debe tener de 1 a 4096 caracteres".into(),
        ));
    }
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut c = tx
        .conversacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("conversación"))?;
    if c.estado == EstadoConversacion::Cerrada {
        return Err(AppError::Conflicto("la conversación está cerrada".into()));
    }
    if !c.ventana_abierta(ahora) {
        return Err(AppError::Conflicto(
            "pasaron más de 24 horas desde el último mensaje del residente; WhatsApp solo \
             permite responder cuando vuelva a escribir"
                .into(),
        ));
    }
    let m = MensajeConversacion {
        conversacion_id: id,
        entrante: false,
        texto: texto.clone(),
        autor_cuenta_id: ctx.actor_id,
        wamid: None,
        creado_en: ahora,
    };
    tx.insertar_mensaje_conversacion(&m).await?;
    tx.encolar(EventoSaliente {
        prioridad: PrioridadSalida::Confirmacion,
        mensaje: MensajeOutbox::WhatsApp(MensajeSaliente {
            linea: c.linea.clone(),
            telefono: c.telefono.clone(),
            contenido: ContenidoSaliente::Texto { cuerpo: texto },
        }),
    })
    .await?;
    if c.atendida_por.is_none() {
        c.atendida_por = ctx.actor_id;
        tx.actualizar_conversacion(&c).await?;
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_RESPONDER,
        "conversacion",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(m)
}

/// Texto que recibe el residente al cerrar la conversación.
pub const TEXTO_CIERRE: &str =
    "La administración cerró esta conversación. Escriba «menú» para ver las opciones.";

/// Cierra la conversación (el chat vuelve al menú en el siguiente mensaje del residente) y le
/// avisa si la ventana de 24 h sigue abierta.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `conversaciones.atender`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya estaba cerrada.
pub async fn cerrar_conversacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ConversacionId,
) -> AppResult<Conversacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxChat,
{
    ctx.exigir(Permiso::ConversacionesAtender)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut c = tx
        .conversacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("conversación"))?;
    if c.estado == EstadoConversacion::Cerrada {
        return Err(AppError::Conflicto(
            "la conversación ya estaba cerrada".into(),
        ));
    }
    c.estado = EstadoConversacion::Cerrada;
    c.cerrada_en = Some(ahora);
    tx.actualizar_conversacion(&c).await?;
    if c.ventana_abierta(ahora) {
        tx.encolar(EventoSaliente {
            prioridad: PrioridadSalida::Confirmacion,
            mensaje: MensajeOutbox::WhatsApp(MensajeSaliente {
                linea: c.linea.clone(),
                telefono: c.telefono.clone(),
                contenido: ContenidoSaliente::Texto {
                    cuerpo: TEXTO_CIERRE.into(),
                },
            }),
        })
        .await?;
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_CERRAR,
        "conversacion",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Registra un mensaje entrante en la conversación abierta del teléfono (o abre una). Lo usa el
/// chat con el contexto del sistema. Devuelve la conversación.
///
/// # Errors
///
/// Errores de persistencia; [`AppError::ModuloNoHabilitado`].
pub async fn registrar_entrante<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    entrada: EntradaConversacion,
) -> AppResult<Conversacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxChat,
{
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut c = match tx
        .conversacion_abierta(&entrada.linea, &entrada.telefono)
        .await?
    {
        Some(c) => c,
        None => {
            let c = Conversacion {
                id: ConversacionId::nuevo(),
                linea: entrada.linea.clone(),
                telefono: entrada.telefono.clone(),
                persona_id: entrada.persona_id,
                estado: EstadoConversacion::Abierta,
                atendida_por: None,
                abierta_en: ahora,
                ultimo_entrante_en: ahora,
                cerrada_en: None,
            };
            tx.insertar_conversacion(&c).await?;
            c
        }
    };
    if let Some(texto) = entrada.texto {
        let nuevo = tx
            .insertar_mensaje_conversacion(&MensajeConversacion {
                conversacion_id: c.id,
                entrante: true,
                texto,
                autor_cuenta_id: None,
                wamid: Some(entrada.wamid),
                creado_en: ahora,
            })
            .await?;
        if nuevo {
            c.ultimo_entrante_en = ahora.max(c.ultimo_entrante_en);
            tx.actualizar_conversacion(&c).await?;
        }
    }
    tx.confirmar().await?;
    Ok(c)
}

/// Mensaje entrante que va a la bandeja.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntradaConversacion {
    pub linea: String,
    pub telefono: String,
    pub persona_id: Option<domain::PersonaId>,
    pub wamid: String,
    /// `None` = solo abrir la conversación.
    pub texto: Option<String>,
}

/// ¿La conversación sigue abierta? (el chat decide si sigue en modo humano).
///
/// # Errors
///
/// Errores de persistencia.
pub async fn conversacion_sigue_abierta<U>(
    unidad: &U,
    ctx: &Contexto,
    id: ConversacionId,
) -> AppResult<bool>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxChat,
{
    let mut tx = unidad.iniciar(ctx).await?;
    Ok(tx
        .conversacion(id)
        .await?
        .is_some_and(|c| c.estado == EstadoConversacion::Abierta))
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::*;

    #[test]
    fn ventana_de_24_horas() {
        let ahora = Utc::now();
        let c = Conversacion {
            id: ConversacionId::nuevo(),
            linea: "L".into(),
            telefono: "573001112233".into(),
            persona_id: None,
            estado: EstadoConversacion::Abierta,
            atendida_por: None,
            abierta_en: ahora,
            ultimo_entrante_en: ahora - Duration::hours(23),
            cerrada_en: None,
        };
        assert!(c.ventana_abierta(ahora));
        assert!(!c.ventana_abierta(ahora + Duration::hours(2)));
    }
}
