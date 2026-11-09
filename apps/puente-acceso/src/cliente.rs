//! Cliente HTTPS del puente hacia `/api/v1/puentes` (solo conexiones salientes; TLS con rustls +
//! ring y las raíces de `webpki-roots`, como el resto del proyecto).

use std::{sync::Arc, time::Duration};

use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{EventoPendiente, Sincronizacion};

/// Error al hablar con la nube.
#[derive(Debug, thiserror::Error)]
pub enum ErrorNube {
    #[error("no se pudo crear el cliente HTTP: {0}")]
    Construccion(String),
    #[error("error de red: {0}")]
    Red(#[from] reqwest::Error),
    #[error("la API respondió {0}")]
    Http(u16),
}

/// Resultado de un reporte.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Reporte {
    pub nuevos: u32,
    pub repetidos: u32,
    pub borrados_confirmados: u32,
}

/// Cliente de la API de puentes.
#[derive(Clone)]
pub struct ClienteNube {
    http: reqwest::Client,
    base: Arc<str>,
    token: Arc<str>,
}

impl std::fmt::Debug for ClienteNube {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClienteNube")
            .field("base", &self.base)
            .field("token", &"<oculto>")
            .finish()
    }
}

impl ClienteNube {
    /// `base` es la raíz de la API con versión (`https://api.dominio.co/api/v1`).
    ///
    /// # Errors
    ///
    /// [`ErrorNube::Construccion`] si no se puede preparar TLS.
    pub fn new(base: &str, token: &str) -> Result<Self, ErrorNube> {
        let mut raices = rustls::RootCertStore::empty();
        raices.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| ErrorNube::Construccion(e.to_string()))?
        .with_root_certificates(raices)
        .with_no_client_auth();
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| ErrorNube::Construccion(e.to_string()))?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').into(),
            token: token.into(),
        })
    }

    /// Cambios desde el cursor (también cuenta como latido).
    ///
    /// # Errors
    ///
    /// [`ErrorNube`] si la API no responde 200.
    pub async fn sincronizar(
        &self,
        cursor: i64,
        version: &str,
    ) -> Result<Sincronizacion, ErrorNube> {
        let r = self
            .http
            .get(format!(
                "{}/puentes/sincronizacion?cursor={cursor}&version={}",
                self.base,
                version
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
                    .collect::<String>()
            ))
            .bearer_auth(&*self.token)
            .send()
            .await?;
        if !r.status().is_success() {
            return Err(ErrorNube::Http(r.status().as_u16()));
        }
        Ok(r.json().await?)
    }

    /// Reporta un lote de eventos y confirmaciones de borrado biométrico.
    ///
    /// # Errors
    ///
    /// [`ErrorNube`] si la API no responde 200.
    pub async fn reportar(
        &self,
        eventos: &[EventoPendiente],
        borrados: &[Uuid],
    ) -> Result<Reporte, ErrorNube> {
        let r = self
            .http
            .post(format!("{}/puentes/eventos", self.base))
            .bearer_auth(&*self.token)
            .json(&json!({ "eventos": eventos, "borrados_biometricos": borrados }))
            .send()
            .await?;
        if !r.status().is_success() {
            return Err(ErrorNube::Http(r.status().as_u16()));
        }
        Ok(r.json().await?)
    }
}
