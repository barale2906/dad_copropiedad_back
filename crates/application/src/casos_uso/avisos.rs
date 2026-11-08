//! Avisos de la gestión administrativa (tickets, órdenes de trabajo, obligaciones): se encolan
//! en el outbox dentro de la transacción del hecho que los origina.
//!
//! - A una **persona** (el solicitante de un ticket): por correo si lo tiene y, si se pide, por
//!   WhatsApp con una plantilla aprobada (fuera de una conversación solo se pueden enviar
//!   plantillas: ventana de 24 h de Meta).
//! - A las **cuentas con un permiso** (urgencias, vencimientos): por correo y, si registraron
//!   su teléfono como persona de la PH, por WhatsApp.
//!
//! Nunca se envía por WhatsApp información sensible de otros residentes (plan 07b): las
//! plantillas llevan solo el número del ticket, el estado y un texto corto.

use domain::{Permiso, PersonaId};

use crate::{
    AppResult,
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        RegistroOutbox, RepositorioPersonas, RepositorioTickets,
        correo::CorreoSaliente,
        mensajeria::{ContenidoSaliente, MensajeSaliente},
    },
};

/// Idioma de las plantillas registradas en Meta.
pub const IDIOMA_PLANTILLAS: &str = "es";

/// Plantilla de utilidad «ticket_actualizado». Parámetros: número, estado, detalle.
pub const PLANTILLA_TICKET_ACTUALIZADO: &str = "ticket_actualizado";
/// Plantilla de utilidad «reporte_urgente». Parámetros: número, asunto, ubicación.
pub const PLANTILLA_REPORTE_URGENTE: &str = "reporte_urgente";

/// Máximo de caracteres de un parámetro de plantilla (Meta rechaza textos largos).
const MAX_PARAMETRO: usize = 200;

/// Contenido de un aviso.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aviso {
    pub prioridad: PrioridadSalida,
    pub asunto: String,
    pub texto: String,
    /// Plantilla de WhatsApp y sus parámetros; `None` = solo correo.
    pub plantilla: Option<(&'static str, Vec<String>)>,
}

fn parametro(texto: &str) -> String {
    let limpio: String = texto
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    limpio.chars().take(MAX_PARAMETRO).collect()
}

fn whatsapp(linea: &str, telefono: &str, plantilla: &(&'static str, Vec<String>)) -> MensajeOutbox {
    MensajeOutbox::WhatsApp(MensajeSaliente {
        linea: linea.to_owned(),
        telefono: telefono.to_owned(),
        contenido: ContenidoSaliente::Plantilla {
            nombre: plantilla.0.to_owned(),
            idioma: IDIOMA_PLANTILLAS.to_owned(),
            parametros: plantilla.1.iter().map(|p| parametro(p)).collect(),
        },
    })
}

/// Avisa a una persona de la copropiedad. No hace nada si no tiene medios de contacto.
pub(crate) async fn avisar_persona<T>(
    tx: &mut T,
    persona: Option<PersonaId>,
    aviso: &Aviso,
) -> AppResult<()>
where
    T: RepositorioPersonas + RepositorioTickets + RegistroOutbox,
{
    let Some(p) = persona else { return Ok(()) };
    let Some(persona) = tx.persona(p).await? else {
        return Ok(());
    };
    if let Some(correo) = persona.correo {
        tx.encolar(EventoSaliente {
            prioridad: aviso.prioridad,
            mensaje: MensajeOutbox::Correo(CorreoSaliente {
                para: correo,
                asunto: aviso.asunto.clone(),
                texto: aviso.texto.clone(),
                html: None,
            }),
        })
        .await?;
    }
    if let (Some(plantilla), Some(telefono)) = (&aviso.plantilla, persona.telefono)
        && let Some(linea) = tx.linea_predeterminada().await?
    {
        tx.encolar(EventoSaliente {
            prioridad: aviso.prioridad,
            mensaje: whatsapp(&linea, telefono.as_str(), plantilla),
        })
        .await?;
    }
    Ok(())
}

/// Avisa a todas las cuentas que tienen el permiso en la copropiedad. Devuelve a cuántas.
pub(crate) async fn avisar_con_permiso<T>(
    tx: &mut T,
    permiso: Permiso,
    aviso: &Aviso,
) -> AppResult<usize>
where
    T: RepositorioTickets + RegistroOutbox,
{
    let destinatarios = tx.destinatarios_con_permiso(permiso).await?;
    let linea = match aviso.plantilla {
        Some(_) => tx.linea_predeterminada().await?,
        None => None,
    };
    for d in &destinatarios {
        if let Some(correo) = &d.correo {
            tx.encolar(EventoSaliente {
                prioridad: aviso.prioridad,
                mensaje: MensajeOutbox::Correo(CorreoSaliente {
                    para: correo.clone(),
                    asunto: aviso.asunto.clone(),
                    texto: aviso.texto.clone(),
                    html: None,
                }),
            })
            .await?;
        }
        if let (Some(plantilla), Some(linea), Some(telefono)) =
            (&aviso.plantilla, &linea, &d.telefono)
        {
            tx.encolar(EventoSaliente {
                prioridad: aviso.prioridad,
                mensaje: whatsapp(linea, telefono.as_str(), plantilla),
            })
            .await?;
        }
    }
    Ok(destinatarios.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parametros_sin_saltos_y_acotados() {
        assert_eq!(parametro("línea\nnueva"), "línea nueva");
        assert_eq!(parametro(&"x".repeat(500)).chars().count(), MAX_PARAMETRO);
    }
}
