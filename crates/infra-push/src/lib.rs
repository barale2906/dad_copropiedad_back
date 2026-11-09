//! Notificaciones push de las apps Flutter (Android e iOS) con **Firebase Cloud Messaging HTTP
//! v1**: implementa el puerto [`EnvioPush`].
//!
//! - Autenticación con una **cuenta de servicio** de Google (archivo JSON en
//!   `FCM_CREDENCIALES`): se firma un JWT RS256 con su clave privada y se cambia por un token de
//!   acceso de una hora, que se reutiliza hasta un minuto antes de vencer.
//! - Un token de dispositivo que FCM ya no reconoce (`UNREGISTERED`, `INVALID_ARGUMENT` sobre el
//!   token, 404) se informa como [`ErrorPush::TokenInvalido`]: el worker lo desactiva y envía el
//!   respaldo por WhatsApp. Lo demás es [`ErrorPush::Temporal`] y se reintenta.
//! - El cuerpo del error puede incluir el token (dato personal): va a la fila del outbox, nunca a
//!   los logs.
//! - TLS con rustls + ring y las raíces de `webpki-roots` (sin OpenSSL), como `infra-whatsapp`.

use std::{sync::Arc, time::Duration};

use application::puertos::{
    EnvioPush,
    acceso::{ErrorPush, NotificacionPush},
};
use base64::{Engine, engine::general_purpose::STANDARD, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, TimeDelta, Utc};
use ring::{
    rand::SystemRandom,
    signature::{RSA_PKCS1_SHA256, RsaKeyPair},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

/// Base de la API de FCM.
pub const FCM_API_URL_PREDETERMINADA: &str = "https://fcm.googleapis.com";
/// Alcance OAuth de FCM.
const ALCANCE: &str = "https://www.googleapis.com/auth/firebase.messaging";
/// Tiempo máximo de cada llamada.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// Margen para renovar el token de acceso antes de que venza.
const MARGEN_RENOVACION: TimeDelta = TimeDelta::seconds(60);

/// Error al preparar el cliente.
#[derive(Debug, thiserror::Error)]
pub enum ErrorConfiguracion {
    #[error("no se pudo leer la cuenta de servicio de Firebase: {0}")]
    Lectura(String),
    #[error("cuenta de servicio de Firebase inválida: {0}")]
    Credenciales(&'static str),
    #[error("no se pudo crear el cliente HTTP: {0}")]
    Http(String),
}

/// Campos que se usan del JSON de la cuenta de servicio.
#[derive(Deserialize)]
struct CuentaServicio {
    project_id: String,
    client_email: String,
    private_key: String,
    token_uri: String,
}

struct TokenAcceso {
    valor: String,
    vence: DateTime<Utc>,
}

/// Cliente de FCM. Clonarlo comparte la caché del token de acceso.
#[derive(Clone)]
pub struct ClienteFcm {
    http: reqwest::Client,
    proyecto: Arc<str>,
    correo: Arc<str>,
    token_uri: Arc<str>,
    api_url: Arc<str>,
    clave: Arc<RsaKeyPair>,
    token: Arc<Mutex<Option<TokenAcceso>>>,
}

impl std::fmt::Debug for ClienteFcm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClienteFcm")
            .field("proyecto", &self.proyecto)
            .field("clave", &"<oculta>")
            .finish_non_exhaustive()
    }
}

fn der_de_pem(pem: &str) -> Result<Vec<u8>, ErrorConfiguracion> {
    let cuerpo: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .map(str::trim)
        .collect();
    STANDARD
        .decode(cuerpo)
        .map_err(|_| ErrorConfiguracion::Credenciales("la clave privada no es PEM válido"))
}

impl ClienteFcm {
    /// Lee el JSON de la cuenta de servicio.
    ///
    /// # Errors
    ///
    /// [`ErrorConfiguracion`] si el archivo no existe, no es una cuenta de servicio o la clave no
    /// es RSA PKCS#8.
    pub fn desde_archivo(ruta: &str, api_url: &str) -> Result<Self, ErrorConfiguracion> {
        let json = std::fs::read_to_string(ruta)
            .map_err(|e| ErrorConfiguracion::Lectura(e.to_string()))?;
        Self::desde_json(&json, api_url)
    }

    /// # Errors
    ///
    /// Los de [`ClienteFcm::desde_archivo`].
    pub fn desde_json(json: &str, api_url: &str) -> Result<Self, ErrorConfiguracion> {
        let c: CuentaServicio = serde_json::from_str(json).map_err(|_| {
            ErrorConfiguracion::Credenciales(
                "faltan project_id, client_email, private_key o token_uri",
            )
        })?;
        let clave = RsaKeyPair::from_pkcs8(&der_de_pem(&c.private_key)?)
            .map_err(|_| ErrorConfiguracion::Credenciales("la clave privada no es RSA PKCS#8"))?;
        let mut raices = rustls::RootCertStore::empty();
        raices.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| ErrorConfiguracion::Http(e.to_string()))?
        .with_root_certificates(raices)
        .with_no_client_auth();
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| ErrorConfiguracion::Http(e.to_string()))?;
        Ok(Self {
            http,
            proyecto: c.project_id.into(),
            correo: c.client_email.into(),
            token_uri: c.token_uri.into(),
            api_url: api_url.trim_end_matches('/').into(),
            clave: Arc::new(clave),
            token: Arc::default(),
        })
    }

    /// JWT RS256 de la cuenta de servicio (RFC 7523) para pedir el token de acceso.
    fn afirmacion(&self, ahora: DateTime<Utc>) -> Result<String, ErrorPush> {
        let cabecera = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let reclamos = json!({
            "iss": &*self.correo,
            "scope": ALCANCE,
            "aud": &*self.token_uri,
            "iat": ahora.timestamp(),
            "exp": (ahora + TimeDelta::hours(1)).timestamp(),
        });
        let carga = URL_SAFE_NO_PAD.encode(reclamos.to_string());
        let firmable = format!("{cabecera}.{carga}");
        let mut firma = vec![0u8; self.clave.public().modulus_len()];
        self.clave
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                firmable.as_bytes(),
                &mut firma,
            )
            .map_err(|_| ErrorPush::Temporal("no se pudo firmar el JWT de Firebase".into()))?;
        Ok(format!("{firmable}.{}", URL_SAFE_NO_PAD.encode(firma)))
    }

    async fn token_acceso(&self) -> Result<String, ErrorPush> {
        let mut guardado = self.token.lock().await;
        let ahora = Utc::now();
        if let Some(t) = guardado.as_ref()
            && t.vence - MARGEN_RENOVACION > ahora
        {
            return Ok(t.valor.clone());
        }
        let cuerpo = format!(
            "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion={}",
            self.afirmacion(ahora)?
        );
        let r = self
            .http
            .post(&*self.token_uri)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(cuerpo)
            .send()
            .await
            .map_err(|e| ErrorPush::Temporal(format!("token de Firebase: {e}")))?;
        if !r.status().is_success() {
            return Err(ErrorPush::Temporal(format!(
                "token de Firebase: HTTP {}",
                r.status().as_u16()
            )));
        }
        #[derive(Deserialize)]
        struct Respuesta {
            access_token: String,
            expires_in: i64,
        }
        let t: Respuesta = r.json().await.map_err(|_| {
            ErrorPush::Temporal("respuesta inesperada del token de Firebase".into())
        })?;
        let valor = t.access_token.clone();
        *guardado = Some(TokenAcceso {
            valor: t.access_token,
            vence: ahora + TimeDelta::seconds(t.expires_in),
        });
        Ok(valor)
    }
}

/// Cuerpo de `messages:send`: notificación visible + datos para la app, prioridad alta (avisos
/// de portería que no pueden esperar).
pub fn cuerpo_mensaje(n: &NotificacionPush) -> Value {
    json!({
        "message": {
            "token": n.token,
            "notification": { "title": n.titulo, "body": n.cuerpo },
            "data": n.datos,
            "android": { "priority": "high" },
            "apns": { "headers": { "apns-priority": "10" } },
        }
    })
}

/// Clasifica una respuesta de error de FCM.
pub fn clasificar_error(estado: u16, cuerpo: &str) -> ErrorPush {
    let v: Value = serde_json::from_str(cuerpo).unwrap_or(Value::Null);
    let estado_api = v["error"]["status"].as_str().unwrap_or_default();
    let codigo_fcm = v["error"]["details"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|d| d["errorCode"].as_str())
        .unwrap_or_default();
    let token_malo = estado == 404
        || matches!(
            codigo_fcm,
            "UNREGISTERED" | "INVALID_ARGUMENT" | "SENDER_ID_MISMATCH"
        )
        || (estado == 400 && estado_api == "INVALID_ARGUMENT");
    if token_malo {
        ErrorPush::TokenInvalido
    } else {
        ErrorPush::Temporal(format!(
            "FCM respondió {estado}: {}",
            cuerpo.chars().take(300).collect::<String>()
        ))
    }
}

impl EnvioPush for ClienteFcm {
    async fn enviar(&self, n: &NotificacionPush) -> Result<String, ErrorPush> {
        let token = self.token_acceso().await?;
        let url = format!(
            "{}/v1/projects/{}/messages:send",
            self.api_url, self.proyecto
        );
        let r = self
            .http
            .post(url)
            .bearer_auth(token)
            .json(&cuerpo_mensaje(n))
            .send()
            .await
            .map_err(|e| ErrorPush::Temporal(format!("red con FCM: {e}")))?;
        let estado = r.status().as_u16();
        let cuerpo = r.text().await.unwrap_or_default();
        if (200..300).contains(&estado) {
            let v: Value = serde_json::from_str(&cuerpo).unwrap_or(Value::Null);
            return Ok(v["name"].as_str().unwrap_or_default().to_owned());
        }
        Err(clasificar_error(estado, &cuerpo))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clasifica_tokens_invalidos() {
        let unregistered = r#"{"error":{"code":404,"status":"NOT_FOUND","details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":"UNREGISTERED"}]}}"#;
        assert_eq!(
            clasificar_error(404, unregistered),
            ErrorPush::TokenInvalido
        );
        let invalido = r#"{"error":{"code":400,"status":"INVALID_ARGUMENT"}}"#;
        assert_eq!(clasificar_error(400, invalido), ErrorPush::TokenInvalido);
        assert!(matches!(
            clasificar_error(503, "{}"),
            ErrorPush::Temporal(_)
        ));
        assert!(matches!(
            clasificar_error(429, r#"{"error":{"status":"RESOURCE_EXHAUSTED"}}"#),
            ErrorPush::Temporal(_)
        ));
    }

    #[test]
    fn cuerpo_con_datos_y_prioridad_alta() {
        let mut datos = std::collections::BTreeMap::new();
        datos.insert("visita_id".to_owned(), "v1".to_owned());
        let n = NotificacionPush {
            token: "t".into(),
            titulo: "Visita".into(),
            cuerpo: "Hola".into(),
            datos,
            respaldo: None,
        };
        let c = cuerpo_mensaje(&n);
        assert_eq!(c["message"]["token"], "t");
        assert_eq!(c["message"]["data"]["visita_id"], "v1");
        assert_eq!(c["message"]["android"]["priority"], "high");
    }
}
