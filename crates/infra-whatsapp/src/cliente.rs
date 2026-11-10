//! Cliente de envío de WhatsApp Cloud API: implementa [`Mensajeria`] (texto, botones, listas y
//! plantillas aprobadas) y la descarga de archivos recibidos.
//!
//! - Los textos se recortan a los límites de Meta (botones de 20 caracteres, filas de 24,
//!   descripciones de 72, cuerpo interactivo de 1024 y texto de 4096) para que un título largo
//!   no haga rechazar todo el mensaje.
//! - El error de un envío puede incluir el teléfono de destino: lo registra el worker en la
//!   fila del outbox, nunca en los logs.
//! - TLS con rustls + ring y las raíces de `webpki-roots` (sin OpenSSL ni el almacén del sistema).

use std::{sync::Arc, time::Duration};

use application::{
    AppError, AppResult,
    puertos::mensajeria::{ContenidoSaliente, MediaDescargada, MensajeSaliente, Mensajeria},
};
use serde::Deserialize;
use serde_json::{Value, json};

/// Tamaño máximo de un archivo descargado (Meta admite imágenes de hasta 5 MB).
pub const MAX_MEDIA_BYTES: usize = 10 * 1024 * 1024;
/// Tiempo máximo de cada llamada a la API.
pub const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum ErrorCliente {
    #[error("no se pudo crear el cliente HTTP: {0}")]
    Construccion(String),
    #[error("error de red con WhatsApp Cloud API: {0}")]
    Red(#[from] reqwest::Error),
    #[error("WhatsApp Cloud API respondió {estado}: {cuerpo}")]
    Api { estado: u16, cuerpo: String },
    #[error("respuesta inesperada de WhatsApp Cloud API")]
    Respuesta,
    #[error("el archivo supera {MAX_MEDIA_BYTES} bytes")]
    MediaGrande,
}

/// Cliente de WhatsApp Cloud API con un token de acceso del sistema.
#[derive(Clone)]
pub struct ClienteWhatsapp {
    http: reqwest::Client,
    /// Base de la Graph API con versión, p. ej. `https://graph.facebook.com/v23.0`.
    api_url: Arc<str>,
    token: Arc<str>,
}

impl std::fmt::Debug for ClienteWhatsapp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClienteWhatsapp")
            .field("api_url", &self.api_url)
            .field("token", &"<oculto>")
            .finish()
    }
}

impl ClienteWhatsapp {
    /// # Errors
    ///
    /// [`ErrorCliente::Construccion`] si no se puede preparar TLS.
    pub fn new(api_url: &str, token: &str) -> Result<Self, ErrorCliente> {
        let mut raices = rustls::RootCertStore::empty();
        raices.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| ErrorCliente::Construccion(e.to_string()))?
        .with_root_certificates(raices)
        .with_no_client_auth();
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| ErrorCliente::Construccion(e.to_string()))?;
        Ok(Self {
            http,
            api_url: api_url.trim_end_matches('/').into(),
            token: token.into(),
        })
    }

    async fn post(&self, url: String, cuerpo: &Value) -> Result<Value, ErrorCliente> {
        let res = self
            .http
            .post(url)
            .bearer_auth(&self.token)
            .json(cuerpo)
            .send()
            .await?;
        respuesta_json(res).await
    }

    async fn enviar_payload(&self, m: &MensajeSaliente) -> Result<String, ErrorCliente> {
        #[derive(Deserialize)]
        struct Enviado {
            id: String,
        }
        #[derive(Deserialize)]
        struct Respuesta {
            messages: Vec<Enviado>,
        }
        let v = self
            .post(
                format!("{}/{}/messages", self.api_url, m.linea),
                &payload(m),
            )
            .await?;
        let r: Respuesta = serde_json::from_value(v).map_err(|_| ErrorCliente::Respuesta)?;
        r.messages
            .into_iter()
            .next()
            .map(|e| e.id)
            .ok_or(ErrorCliente::Respuesta)
    }

    async fn descargar(&self, media_id: &str) -> Result<MediaDescargada, ErrorCliente> {
        #[derive(Deserialize)]
        struct Info {
            url: String,
            mime_type: Option<String>,
        }
        let res = self
            .http
            .get(format!("{}/{media_id}", self.api_url))
            .bearer_auth(&self.token)
            .send()
            .await?;
        let info: Info = serde_json::from_value(respuesta_json(res).await?)
            .map_err(|_| ErrorCliente::Respuesta)?;
        let res = self
            .http
            .get(&info.url)
            .bearer_auth(&self.token)
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(ErrorCliente::Api {
                estado: res.status().as_u16(),
                cuerpo: String::new(),
            });
        }
        if res
            .content_length()
            .is_some_and(|l| l > MAX_MEDIA_BYTES as u64)
        {
            return Err(ErrorCliente::MediaGrande);
        }
        let bytes = res.bytes().await?;
        if bytes.len() > MAX_MEDIA_BYTES {
            return Err(ErrorCliente::MediaGrande);
        }
        Ok(MediaDescargada {
            contenido: bytes.to_vec(),
            mime: info.mime_type,
        })
    }
}

async fn respuesta_json(res: reqwest::Response) -> Result<Value, ErrorCliente> {
    let estado = res.status();
    let texto = res.text().await?;
    if !estado.is_success() {
        return Err(ErrorCliente::Api {
            estado: estado.as_u16(),
            cuerpo: recortar(&texto, 500),
        });
    }
    serde_json::from_str(&texto).map_err(|_| ErrorCliente::Respuesta)
}

impl Mensajeria for ClienteWhatsapp {
    async fn enviar(&self, mensaje: &MensajeSaliente) -> AppResult<String> {
        self.enviar_payload(mensaje).await.map_err(AppError::infra)
    }

    async fn descargar_media(&self, media_id: &str) -> AppResult<MediaDescargada> {
        self.descargar(media_id).await.map_err(AppError::infra)
    }
}

fn recortar(t: &str, max: usize) -> String {
    if t.chars().count() <= max {
        t.to_owned()
    } else {
        let mut s: String = t.chars().take(max.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

/// Cuerpo JSON de `POST /{phone_number_id}/messages` para el mensaje.
pub fn payload(m: &MensajeSaliente) -> Value {
    let base = |tipo: &str, contenido: Value| {
        let mut v = json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": m.telefono,
            "type": tipo,
        });
        v[tipo] = contenido;
        v
    };
    match &m.contenido {
        ContenidoSaliente::Texto { cuerpo } => base(
            "text",
            json!({ "body": recortar(cuerpo, 4096), "preview_url": false }),
        ),
        ContenidoSaliente::Botones { cuerpo, botones } => base(
            "interactive",
            json!({
                "type": "button",
                "body": { "text": recortar(cuerpo, 1024) },
                "action": { "buttons": botones.iter().take(ContenidoSaliente::MAX_BOTONES).map(|b| json!({
                    "type": "reply",
                    "reply": { "id": recortar(&b.id, 256), "title": recortar(&b.titulo, 20) }
                })).collect::<Vec<_>>() }
            }),
        ),
        ContenidoSaliente::Lista {
            cuerpo,
            boton,
            filas,
        } => base(
            "interactive",
            json!({
                "type": "list",
                "body": { "text": recortar(cuerpo, 1024) },
                "action": {
                    "button": recortar(boton, 20),
                    "sections": [{
                        "title": "Opciones",
                        "rows": filas.iter().take(ContenidoSaliente::MAX_FILAS_LISTA).map(|f| {
                            let mut fila = json!({
                                "id": recortar(&f.id, 200),
                                "title": recortar(&f.titulo, 24),
                            });
                            if let Some(d) = &f.descripcion {
                                fila["description"] = json!(recortar(d, 72));
                            }
                            fila
                        }).collect::<Vec<_>>()
                    }]
                }
            }),
        ),
        ContenidoSaliente::Plantilla {
            nombre,
            idioma,
            parametros,
            botones,
        } => {
            let mut plantilla = json!({ "name": nombre, "language": { "code": idioma } });
            let mut componentes = Vec::new();
            if !parametros.is_empty() {
                componentes.push(json!({
                    "type": "body",
                    "parameters": parametros.iter().map(|p| json!({ "type": "text", "text": p })).collect::<Vec<_>>()
                }));
            }
            // Botones de respuesta rápida: el `payload` vuelve en el webhook al tocarlos.
            for (i, payload) in botones.iter().enumerate() {
                componentes.push(json!({
                    "type": "button",
                    "sub_type": "quick_reply",
                    "index": i.to_string(),
                    "parameters": [{ "type": "payload", "payload": payload }]
                }));
            }
            if !componentes.is_empty() {
                plantilla["components"] = Value::Array(componentes);
            }
            base("template", plantilla)
        }
        ContenidoSaliente::Imagen { url, leyenda } => {
            let mut imagen = json!({ "link": url });
            if let Some(l) = leyenda {
                imagen["caption"] = json!(recortar(l, 1024));
            }
            base("image", imagen)
        }
    }
}

#[cfg(test)]
mod tests {
    use application::puertos::mensajeria::Eleccion;
    use axum::{
        Json, Router,
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        routing::{get, post},
    };
    use tokio::net::TcpListener;

    use super::*;

    fn mensaje(contenido: ContenidoSaliente) -> MensajeSaliente {
        MensajeSaliente {
            linea: "PNID".into(),
            telefono: "573001112233".into(),
            contenido,
        }
    }

    #[test]
    fn payload_de_lista_recorta_titulos_y_descripciones() {
        let p = payload(&mensaje(ContenidoSaliente::Lista {
            cuerpo: "Elija".into(),
            boton: "Ver todas las opciones disponibles".into(),
            filas: vec![
                Eleccion::new("a", "Un título que es demasiado largo para una fila")
                    .con_descripcion("d".repeat(100)),
            ],
        }));
        assert_eq!(p["type"], "interactive");
        assert_eq!(p["interactive"]["type"], "list");
        let fila = &p["interactive"]["action"]["sections"][0]["rows"][0];
        assert_eq!(fila["title"].as_str().map(|t| t.chars().count()), Some(24));
        assert_eq!(
            fila["description"].as_str().map(|t| t.chars().count()),
            Some(72)
        );
        assert_eq!(
            p["interactive"]["action"]["button"]
                .as_str()
                .map(|t| t.chars().count()),
            Some(20)
        );
    }

    #[test]
    fn payload_de_imagen_por_enlace() {
        let i = payload(&mensaje(ContenidoSaliente::Imagen {
            url: "https://api.dominio.co/api/v1/archivos/descarga?f=x".into(),
            leyenda: Some("QR de la visita".into()),
        }));
        assert_eq!(i["type"], "image");
        assert_eq!(
            i["image"]["link"],
            "https://api.dominio.co/api/v1/archivos/descarga?f=x"
        );
        assert_eq!(i["image"]["caption"], "QR de la visita");
    }

    #[test]
    fn payload_de_botones_y_plantilla() {
        let b = payload(&mensaje(ContenidoSaliente::Botones {
            cuerpo: "¿Es urgente?".into(),
            botones: vec![Eleccion::new("si", "Sí"), Eleccion::new("no", "No")],
        }));
        assert_eq!(
            b["interactive"]["action"]["buttons"][1]["reply"]["id"],
            "no"
        );
        let t = payload(&mensaje(ContenidoSaliente::Plantilla {
            nombre: "ticket_actualizado".into(),
            idioma: "es".into(),
            parametros: vec!["PQR-2026-00001".into(), "resuelto".into()],
            botones: vec!["asis:x".into()],
        }));
        assert_eq!(t["type"], "template");
        assert_eq!(t["template"]["language"]["code"], "es");
        assert_eq!(
            t["template"]["components"][0]["parameters"][0]["text"],
            "PQR-2026-00001"
        );
        assert_eq!(t["template"]["components"][1]["sub_type"], "quick_reply");
        assert_eq!(
            t["template"]["components"][1]["parameters"][0]["payload"],
            "asis:x"
        );
        assert_eq!(t["to"], "573001112233");
    }

    #[derive(Clone)]
    struct Simulador {
        base: String,
    }

    async fn simulador() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("puerto");
        let base = format!("http://{}", listener.local_addr().expect("dirección"));
        let estado = Simulador { base: base.clone() };
        let app = Router::new()
            .route(
                "/{linea}/messages",
                post(
                    |Path(linea): Path<String>, h: HeaderMap, Json(v): Json<Value>| async move {
                        let autorizado = h
                            .get("authorization")
                            .and_then(|x| x.to_str().ok())
                            == Some("Bearer token-prueba");
                        if !autorizado || linea != "PNID" {
                            return (StatusCode::UNAUTHORIZED, Json(json!({ "error": {} })));
                        }
                        let id = format!("wamid.{}", v["type"].as_str().unwrap_or_default());
                        (StatusCode::OK, Json(json!({ "messages": [{ "id": id }] })))
                    },
                ),
            )
            .route(
                "/media-1",
                get(|State(s): State<Simulador>| async move {
                    Json(json!({ "url": format!("{}/descarga/1", s.base), "mime_type": "image/png" }))
                }),
            )
            .route("/descarga/1", get(|| async { vec![0x89u8, b'P', b'N', b'G'] }))
            .with_state(estado);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        base
    }

    #[tokio::test]
    async fn envia_y_devuelve_el_wamid() {
        let base = simulador().await;
        let cliente = ClienteWhatsapp::new(&base, "token-prueba").expect("cliente");
        let wamid = cliente
            .enviar(&mensaje(ContenidoSaliente::Texto {
                cuerpo: "hola".into(),
            }))
            .await
            .expect("envía");
        assert_eq!(wamid, "wamid.text");
    }

    #[tokio::test]
    async fn error_de_la_api_se_propaga() {
        let base = simulador().await;
        let cliente = ClienteWhatsapp::new(&base, "otro").expect("cliente");
        assert!(
            cliente
                .enviar(&mensaje(ContenidoSaliente::Texto {
                    cuerpo: "hola".into(),
                }))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn descarga_media_en_dos_pasos() {
        let base = simulador().await;
        let cliente = ClienteWhatsapp::new(&base, "token-prueba").expect("cliente");
        let media = cliente.descargar_media("media-1").await.expect("descarga");
        assert_eq!(media.contenido, vec![0x89, b'P', b'N', b'G']);
        assert_eq!(media.mime.as_deref(), Some("image/png"));
    }
}
