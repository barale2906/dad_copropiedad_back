//! Payload del webhook de WhatsApp Cloud API y su normalización a [`MensajeEntrante`].
//!
//! Solo se modelan los campos que usamos; el resto se ignora (`serde` no falla por campos
//! desconocidos). Además de los mensajes, se normalizan los acuses de entrega (`statuses`: la
//! mesa ve «no llegó: entregue rótulo») y los cambios de calidad de una línea
//! (`phone_number_quality_update`: una línea degradada deja de recibir conversaciones nuevas).

use application::puertos::{
    asambleas::AcuseEntrega,
    mensajeria::{ContenidoEntrante, MensajeEntrante},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

/// Cuerpo de una notificación del webhook de Meta. Se lee con [`Notificacion::desde_json`] y se
/// consume con sus métodos, que devuelven tipos de la aplicación (mensajes, acuses, calidades).
#[derive(Debug, Deserialize)]
pub struct Notificacion {
    #[serde(default)]
    entry: Vec<Entrada>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Entrada {
    #[serde(default)]
    pub changes: Vec<Cambio>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Cambio {
    pub field: String,
    pub value: Valor,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Valor {
    /// Ausente en `phone_number_quality_update`.
    pub metadata: Option<Metadatos>,
    #[serde(default)]
    pub messages: Vec<Mensaje>,
    #[serde(default)]
    pub statuses: Vec<EstadoMensaje>,
    /// `phone_number_quality_update`: número visible de la línea.
    pub display_phone_number: Option<String>,
    /// `phone_number_quality_update`: `FLAGGED`, `UNFLAGGED`, `DOWNGRADE` o `UPGRADE`.
    pub event: Option<String>,
}

/// Acuse de entrega de un mensaje saliente.
#[derive(Debug, Deserialize)]
pub(crate) struct EstadoMensaje {
    pub id: String,
    pub status: String,
    pub timestamp: Option<String>,
    pub pricing: Option<Precio>,
    #[serde(default)]
    pub errors: Vec<ErrorEstado>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Precio {
    pub billable: Option<bool>,
    pub category: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorEstado {
    pub code: Option<i32>,
}

/// Cambio de calidad de una línea.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CambioCalidad {
    pub numero: String,
    pub degradada: bool,
    pub calidad: Option<&'static str>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Metadatos {
    pub phone_number_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Mensaje {
    pub from: String,
    pub id: String,
    pub timestamp: String,
    #[serde(rename = "type")]
    pub tipo: String,
    pub text: Option<Texto>,
    pub interactive: Option<Interactivo>,
    /// Respuesta rápida de una plantilla.
    pub button: Option<BotonPlantilla>,
    pub image: Option<Imagen>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Imagen {
    pub id: String,
    pub caption: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Texto {
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Interactivo {
    pub button_reply: Option<Respuesta>,
    pub list_reply: Option<Respuesta>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Respuesta {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BotonPlantilla {
    pub payload: String,
    pub text: String,
}

impl Notificacion {
    /// Deserializa el cuerpo crudo (ya verificada su firma `X-Hub-Signature-256`).
    ///
    /// # Errors
    ///
    /// Si el cuerpo no es el JSON de una notificación.
    pub fn desde_json(cuerpo: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(cuerpo)
    }

    fn cambios(&self, campo: &'static str) -> impl Iterator<Item = &Cambio> {
        self.entry
            .iter()
            .flat_map(|e| &e.changes)
            .filter(move |c| c.field == campo)
    }

    /// Mensajes entrantes normalizados (una notificación puede traer varios).
    pub fn mensajes(&self) -> Vec<MensajeEntrante> {
        self.cambios("messages")
            .filter_map(|c| c.value.metadata.as_ref().map(|m| (m, &c.value.messages)))
            .flat_map(|(m, mensajes)| mensajes.iter().map(|x| normalizar(&m.phone_number_id, x)))
            .collect()
    }

    /// Acuses de entrega (`sent`, `delivered`, `read`, `failed`).
    pub fn acuses(&self) -> Vec<AcuseEntrega> {
        self.cambios("messages")
            .filter_map(|c| c.value.metadata.as_ref().map(|m| (m, &c.value.statuses)))
            .flat_map(|(m, estados)| {
                estados
                    .iter()
                    .filter(|e| {
                        matches!(e.status.as_str(), "sent" | "delivered" | "read" | "failed")
                    })
                    .map(|e| AcuseEntrega {
                        wamid: e.id.clone(),
                        linea: m.phone_number_id.clone(),
                        estado: e.status.clone(),
                        categoria: e.pricing.as_ref().and_then(|p| p.category.clone()),
                        facturable: e.pricing.as_ref().and_then(|p| p.billable),
                        error_codigo: e.errors.first().and_then(|x| x.code),
                        ocurrido_en: instante(e.timestamp.as_deref()),
                    })
            })
            .collect()
    }

    /// Cambios de calidad de las líneas.
    pub fn calidades(&self) -> Vec<CambioCalidad> {
        self.cambios("phone_number_quality_update")
            .filter_map(|c| {
                let numero = c.value.display_phone_number.clone()?;
                let (degradada, calidad) = match c.value.event.as_deref()? {
                    "FLAGGED" => (true, Some("amarilla")),
                    "DOWNGRADE" => (true, Some("roja")),
                    "UNFLAGGED" | "UPGRADE" => (false, Some("verde")),
                    _ => return None,
                };
                Some(CambioCalidad {
                    numero,
                    degradada,
                    calidad,
                })
            })
            .collect()
    }
}

fn instante(segundos: Option<&str>) -> DateTime<Utc> {
    segundos
        .and_then(|t| t.parse::<i64>().ok())
        .and_then(|s| DateTime::from_timestamp(s, 0))
        .unwrap_or_else(Utc::now)
}

fn normalizar(linea: &str, m: &Mensaje) -> MensajeEntrante {
    let recibido_en = instante(Some(&m.timestamp));

    let contenido = match (
        m.tipo.as_str(),
        m.text.as_ref(),
        m.interactive.as_ref(),
        m.button.as_ref(),
        m.image.as_ref(),
    ) {
        ("image", .., Some(i)) => ContenidoEntrante::Imagen {
            media_id: i.id.clone(),
            leyenda: i.caption.clone(),
        },
        ("text", Some(t), ..) => ContenidoEntrante::Texto {
            cuerpo: t.body.clone(),
        },
        ("interactive", _, Some(i), ..) => {
            match i.button_reply.as_ref().or(i.list_reply.as_ref()) {
                Some(r) => ContenidoEntrante::Respuesta {
                    id: r.id.clone(),
                    titulo: r.title.clone(),
                },
                None => ContenidoEntrante::NoSoportado {
                    tipo_original: "interactive".into(),
                },
            }
        }
        ("button", _, _, Some(b), _) => ContenidoEntrante::Respuesta {
            id: b.payload.clone(),
            titulo: b.text.clone(),
        },
        (otro, ..) => ContenidoEntrante::NoSoportado {
            tipo_original: otro.to_owned(),
        },
    };

    MensajeEntrante {
        linea: linea.to_owned(),
        telefono: m.from.clone(),
        wamid: m.id.clone(),
        recibido_en,
        contenido,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EJEMPLO: &str = r#"{
      "object": "whatsapp_business_account",
      "entry": [{
        "id": "WABA",
        "changes": [{
          "field": "messages",
          "value": {
            "messaging_product": "whatsapp",
            "metadata": { "display_phone_number": "573000000000", "phone_number_id": "PNID-1" },
            "contacts": [{ "wa_id": "573001112233", "profile": { "name": "Ana" } }],
            "messages": [
              { "from": "573001112233", "id": "wamid.A", "timestamp": "1798000000",
                "type": "interactive",
                "interactive": { "type": "button_reply", "button_reply": { "id": "op-si", "title": "Sí" } } },
              { "from": "573001112233", "id": "wamid.B", "timestamp": "1798000001",
                "type": "interactive",
                "interactive": { "type": "list_reply", "list_reply": { "id": "reu-2", "title": "Torre 2" } } },
              { "from": "573001112233", "id": "wamid.C", "timestamp": "1798000002",
                "type": "text", "text": { "body": "hola" } },
              { "from": "573001112233", "id": "wamid.D", "timestamp": "1798000003",
                "type": "image", "image": { "id": "img", "caption": "fuga" } },
              { "from": "573001112233", "id": "wamid.E", "timestamp": "1798000004",
                "type": "audio", "audio": { "id": "aud" } }
            ]
          }
        }]
      }]
    }"#;

    #[test]
    fn normaliza_botones_listas_texto_y_no_soportados() {
        let msgs = Notificacion::desde_json(EJEMPLO.as_bytes())
            .unwrap()
            .mensajes();
        assert_eq!(msgs.len(), 5);
        assert!(msgs.iter().all(|m| m.linea == "PNID-1"));
        assert_eq!(msgs[0].wamid, "wamid.A");
        assert_eq!(
            msgs[0].contenido,
            ContenidoEntrante::Respuesta {
                id: "op-si".into(),
                titulo: "Sí".into()
            }
        );
        assert!(
            matches!(&msgs[1].contenido, ContenidoEntrante::Respuesta { id, .. } if id == "reu-2")
        );
        assert_eq!(
            msgs[2].contenido,
            ContenidoEntrante::Texto {
                cuerpo: "hola".into()
            }
        );
        assert_eq!(
            msgs[3].contenido,
            ContenidoEntrante::Imagen {
                media_id: "img".into(),
                leyenda: Some("fuga".into())
            }
        );
        assert!(matches!(
            &msgs[4].contenido,
            ContenidoEntrante::NoSoportado { tipo_original } if tipo_original == "audio"
        ));
        assert_eq!(msgs[0].recibido_en.timestamp(), 1_798_000_000);
    }

    #[test]
    fn acuses_de_entrega_con_categoria_y_error() {
        let json = r#"{"entry":[{"changes":[{"field":"messages","value":{
            "metadata":{"phone_number_id":"P"},
            "statuses":[
              {"id":"wamid.X","status":"delivered","timestamp":"1798000000",
               "pricing":{"billable":true,"category":"utility"}},
              {"id":"wamid.Y","status":"failed","timestamp":"1798000001",
               "errors":[{"code":131026,"title":"Message undeliverable"}]},
              {"id":"wamid.Z","status":"deleted"}
            ]}}]}]}"#;
        let n = Notificacion::desde_json(json.as_bytes()).unwrap();
        assert!(n.mensajes().is_empty());
        let acuses = n.acuses();
        assert_eq!(acuses.len(), 2, "solo los estados conocidos");
        assert_eq!(acuses[0].categoria.as_deref(), Some("utility"));
        assert_eq!(acuses[0].facturable, Some(true));
        assert_eq!(acuses[1].estado, "failed");
        assert_eq!(acuses[1].error_codigo, Some(131_026));
        assert_eq!(acuses[1].linea, "P");
    }

    #[test]
    fn cambio_de_calidad_de_una_linea() {
        let json = r#"{"entry":[{"changes":[{"field":"phone_number_quality_update","value":{
            "display_phone_number":"16505551111","event":"FLAGGED","current_limit":"TIER_10K"}}]}]}"#;
        let n = Notificacion::desde_json(json.as_bytes()).unwrap();
        assert!(n.mensajes().is_empty() && n.acuses().is_empty());
        assert_eq!(
            n.calidades(),
            vec![CambioCalidad {
                numero: "16505551111".into(),
                degradada: true,
                calidad: Some("amarilla"),
            }]
        );
    }
}
