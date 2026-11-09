//! Botones «Autorizar» / «Rechazar» de la plantilla `visita_anunciada`. El gateway llama a
//! [`atender_respuesta_visita`] antes que a la asamblea y al chat.
//!
//! El id del botón lleva la copropiedad y la visita (`vis:{tenant}:{visita}:{a|r}`): la
//! respuesta se aplica a **esa** visita y solo si el teléfono es de alguien que puede decidir
//! por el bien privado. Meta reintenta webhooks: el `wamid` hace la respuesta idempotente.

use domain::{CanalRespuestaVisita, TenantId, VisitaId};
use uuid::Uuid;

use super::{
    TxAcceso, abrir,
    visitas::{ResultadoDecision, decidir_en, persona_por_telefono_del_bien},
};
use crate::{
    AppResult, Contexto,
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        RegistroOutbox, Reloj, RepositorioAcceso, SesionesConversacion, Transaccion,
        UnidadDeTrabajo,
        mensajeria::{ContenidoEntrante, ContenidoSaliente, MensajeEntrante, MensajeSaliente},
    },
};

const PREFIJO: &str = "vis:";

/// `payload` de un botón de la plantilla.
pub fn boton_visita(tenant: TenantId, visita: VisitaId, autorizar: bool) -> String {
    format!(
        "{PREFIJO}{tenant}:{visita}:{}",
        if autorizar { "a" } else { "r" }
    )
}

/// Interpreta el `payload` de un botón de visita.
pub fn leer_boton_visita(id: &str) -> Option<(TenantId, VisitaId, bool)> {
    let partes: Vec<&str> = id.strip_prefix(PREFIJO)?.split(':').collect();
    let [t, v, d] = partes.as_slice() else {
        return None;
    };
    let autorizar = match *d {
        "a" => true,
        "r" => false,
        _ => return None,
    };
    Some((
        TenantId::from_uuid(Uuid::try_parse(t).ok()?),
        VisitaId::from_uuid(Uuid::try_parse(v).ok()?),
        autorizar,
    ))
}

/// Resultado de atender un mensaje entrante.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultadoVisitaWhatsapp {
    /// No es un botón de visita: sigue la asamblea o el chat.
    NoAplica,
    Repetido,
    Atendido {
        respuestas: usize,
    },
}

/// Atiende un toque sobre «Autorizar» o «Rechazar».
///
/// # Errors
///
/// [`crate::AppError::Infraestructura`] si falla la base de datos.
pub async fn atender_respuesta_visita<U, S>(
    unidad: &U,
    sesiones: &S,
    reloj: &impl Reloj,
    m: &MensajeEntrante,
) -> AppResult<ResultadoVisitaWhatsapp>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
    S: SesionesConversacion,
{
    let ContenidoEntrante::Respuesta { id, .. } = &m.contenido else {
        return Ok(ResultadoVisitaWhatsapp::NoAplica);
    };
    let Some((tenant, visita, autorizar)) = leer_boton_visita(id) else {
        return Ok(ResultadoVisitaWhatsapp::NoAplica);
    };
    if sesiones.ya_procesado(&m.wamid).await? {
        return Ok(ResultadoVisitaWhatsapp::Repetido);
    }
    let ctx = Contexto::new(tenant).con_request_id(Some(m.wamid.clone()));
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, &ctx).await?;
    let texto = match tx.visita(visita).await? {
        None => "No encontramos esa visita.".to_owned(),
        Some(v) => {
            match persona_por_telefono_del_bien(&mut tx, v.bien_privado_id, &m.telefono, ahora)
                .await?
            {
                None => {
                    "Este número no está registrado para decidir sobre las visitas de ese inmueble."
                        .to_owned()
                }
                Some(p) => {
                    match decidir_en(
                        &mut tx,
                        &ctx,
                        visita,
                        Some(p),
                        autorizar,
                        CanalRespuestaVisita::Whatsapp,
                        ahora,
                    )
                    .await?
                    {
                        ResultadoDecision::Decidida(v) if autorizar => {
                            format!(
                                "✅ Autorizó el ingreso de {}. La portería ya lo sabe.",
                                v.visitante_nombre
                            )
                        }
                        ResultadoDecision::Decidida(v) => {
                            format!(
                                "🚫 Rechazó el ingreso de {}. La portería ya lo sabe.",
                                v.visitante_nombre
                            )
                        }
                        ResultadoDecision::YaDecidida(v) => format!(
                            "Esta visita ya fue atendida (estado: {}).",
                            v.estado.as_str().replace('_', " ")
                        ),
                    }
                }
            }
        }
    };
    tx.encolar(EventoSaliente {
        prioridad: PrioridadSalida::Confirmacion,
        mensaje: MensajeOutbox::WhatsApp(MensajeSaliente {
            linea: m.linea.clone(),
            telefono: m.telefono.clone(),
            contenido: ContenidoSaliente::Texto { cuerpo: texto },
        }),
    })
    .await?;
    tx.confirmar().await?;
    sesiones.marcar_procesado(&m.wamid).await?;
    Ok(ResultadoVisitaWhatsapp::Atendido { respuestas: 1 })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn boton_ida_y_vuelta() {
        let (t, v) = (TenantId::nuevo(), VisitaId::nuevo());
        assert_eq!(
            leer_boton_visita(&boton_visita(t, v, true)),
            Some((t, v, true))
        );
        assert_eq!(
            leer_boton_visita(&boton_visita(t, v, false)),
            Some((t, v, false))
        );
        assert_eq!(leer_boton_visita("vis:x:y:a"), None);
        assert_eq!(leer_boton_visita(&format!("vt:{t}:{v}:a")), None);
        assert_eq!(leer_boton_visita(&format!("vis:{t}:{v}:z")), None);
    }
}
