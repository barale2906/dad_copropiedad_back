//! Payload del webhook de WhatsApp Cloud API y su normalización a [`MensajeEntrante`].
//!
//! Solo se modelan los campos que usamos; el resto se ignora (`serde` no falla por campos
//! desconocidos). Los `statuses` (entregado/leído) se descartan por ahora.

use application::puertos::mensajeria::{ContenidoEntrante, MensajeEntrante};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Notificacion {
    #[serde(default)]
    pub entry: Vec<Entrada>,
}

#[derive(Debug, Deserialize)]
pub struct Entrada {
    #[serde(default)]
    pub changes: Vec<Cambio>,
}

#[derive(Debug, Deserialize)]
pub struct Cambio {
    pub field: String,
    pub value: Valor,
}

#[derive(Debug, Deserialize)]
pub struct Valor {
    pub metadata: Metadatos,
    #[serde(default)]
    pub messages: Vec<Mensaje>,
}

#[derive(Debug, Deserialize)]
pub struct Metadatos {
    pub phone_number_id: String,
}

#[derive(Debug, Deserialize)]
pub struct Mensaje {
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
pub struct Imagen {
    pub id: String,
    pub caption: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Texto {
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct Interactivo {
    pub button_reply: Option<Respuesta>,
    pub list_reply: Option<Respuesta>,
}

#[derive(Debug, Deserialize)]
pub struct Respuesta {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct BotonPlantilla {
    pub payload: String,
    pub text: String,
}

impl Notificacion {
    pub fn desde_json(cuerpo: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(cuerpo)
    }

    /// Mensajes entrantes normalizados (una notificación puede traer varios).
    pub fn mensajes(self) -> Vec<MensajeEntrante> {
        self.entry
            .into_iter()
            .flat_map(|e| e.changes)
            .filter(|c| c.field == "messages")
            .flat_map(|c| {
                let linea = c.value.metadata.phone_number_id;
                c.value
                    .messages
                    .into_iter()
                    .map(move |m| normalizar(&linea, m))
            })
            .collect()
    }
}

fn normalizar(linea: &str, m: Mensaje) -> MensajeEntrante {
    let recibido_en = m
        .timestamp
        .parse::<i64>()
        .ok()
        .and_then(|s| DateTime::from_timestamp(s, 0))
        .unwrap_or_else(Utc::now);

    let contenido = match (m.tipo.as_str(), m.text, m.interactive, m.button, m.image) {
        ("image", .., Some(i)) => ContenidoEntrante::Imagen {
            media_id: i.id,
            leyenda: i.caption,
        },
        ("text", Some(t), ..) => ContenidoEntrante::Texto { cuerpo: t.body },
        ("interactive", _, Some(i), ..) => match i.button_reply.or(i.list_reply) {
            Some(r) => ContenidoEntrante::Respuesta {
                id: r.id,
                titulo: r.title,
            },
            None => ContenidoEntrante::NoSoportado {
                tipo_original: "interactive".into(),
            },
        },
        ("button", _, _, Some(b), _) => ContenidoEntrante::Respuesta {
            id: b.payload,
            titulo: b.text,
        },
        (otro, ..) => ContenidoEntrante::NoSoportado {
            tipo_original: otro.to_owned(),
        },
    };

    MensajeEntrante {
        linea: linea.to_owned(),
        telefono: m.from,
        wamid: m.id,
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
    fn ignora_notificaciones_de_estado() {
        let json = r#"{"entry":[{"changes":[{"field":"messages","value":{
            "metadata":{"phone_number_id":"P"},"statuses":[{"id":"wamid.X","status":"read"}]}}]}]}"#;
        assert!(
            Notificacion::desde_json(json.as_bytes())
                .unwrap()
                .mensajes()
                .is_empty()
        );
    }
}
