//! Cliente HTTP de la prueba: cada petición queda medida como una [`Muestra`] con el nombre de
//! su operación, para resumirlas al final.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use reqwest::Method;
use serde_json::Value;

/// Una petición medida.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Muestra {
    /// Nombre corto de la operación (`votar`, `tablero`, …).
    pub operacion: &'static str,
    /// Duración de punta a punta en microsegundos.
    pub micros: u64,
    /// Código HTTP; `0` si la petición no llegó a responder (red, tiempo agotado).
    pub estado: u16,
}

impl Muestra {
    /// Si la respuesta fue 2xx.
    pub const fn exitosa(&self) -> bool {
        self.estado >= 200 && self.estado < 300
    }
}

/// Respuesta ya leída.
#[derive(Debug, Clone)]
pub struct Respuesta {
    /// Código HTTP (`0` sin respuesta).
    pub estado: u16,
    /// Cuerpo JSON (`Null` si no era JSON).
    pub cuerpo: Value,
}

impl Respuesta {
    /// El cuerpo si el estado es uno de `esperados`; si no, un error con el contexto.
    ///
    /// # Errors
    ///
    /// Si el estado no es el esperado.
    pub fn exigir(self, esperados: &[u16], que: &str) -> anyhow::Result<Value> {
        if esperados.contains(&self.estado) {
            Ok(self.cuerpo)
        } else {
            bail!("{que}: HTTP {} {}", self.estado, self.cuerpo)
        }
    }
}

/// Cuerpo de una petición.
#[derive(Debug, Clone, Copy)]
pub enum Cuerpo<'a> {
    /// Sin cuerpo.
    Nada,
    /// JSON.
    Json(&'a Value),
    /// Bytes crudos (`application/octet-stream`).
    Bytes(&'a [u8]),
}

/// Cliente contra la URL base de la API, que acumula las muestras.
#[derive(Debug, Clone)]
pub struct Cliente {
    http: reqwest::Client,
    base: String,
    muestras: Arc<Mutex<Vec<Muestra>>>,
}

impl Cliente {
    /// Cliente para `base` (p. ej. `http://127.0.0.1:8080`). TLS con rustls y raíces de webpki,
    /// como el resto del proyecto (sin OpenSSL).
    ///
    /// # Errors
    ///
    /// Si no se puede construir el cliente HTTP.
    pub fn new(base: &str) -> anyhow::Result<Self> {
        let raices =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .context("versiones de TLS")?
        .with_root_certificates(raices)
        .with_no_client_auth();
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(256)
            .build()
            .context("cliente HTTP")?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            muestras: Arc::default(),
        })
    }

    /// Petición JSON (o sin cuerpo) con el token opcional.
    pub async fn pedir(
        &self,
        operacion: &'static str,
        metodo: Method,
        ruta: &str,
        token: Option<&str>,
        cuerpo: Option<&Value>,
    ) -> Respuesta {
        let cuerpo = cuerpo.map_or(Cuerpo::Nada, Cuerpo::Json);
        self.pedir_con(operacion, metodo, ruta, token, &[], cuerpo)
            .await
    }

    /// Petición con cabeceras adicionales (p. ej. `Idempotency-Key`).
    pub async fn pedir_con(
        &self,
        operacion: &'static str,
        metodo: Method,
        ruta: &str,
        token: Option<&str>,
        cabeceras: &[(&str, String)],
        cuerpo: Cuerpo<'_>,
    ) -> Respuesta {
        let mut req = self.http.request(metodo, format!("{}{ruta}", self.base));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        for (nombre, valor) in cabeceras {
            req = req.header(*nombre, valor);
        }
        req = match cuerpo {
            Cuerpo::Nada => req,
            Cuerpo::Json(v) => req.json(v),
            Cuerpo::Bytes(b) => req
                .header("content-type", "application/octet-stream")
                .body(b.to_vec()),
        };
        let inicio = Instant::now();
        let (estado, cuerpo) = match req.send().await {
            Ok(r) => {
                let estado = r.status().as_u16();
                let cuerpo = r.json::<Value>().await.unwrap_or(Value::Null);
                (estado, cuerpo)
            }
            Err(_) => (0, Value::Null),
        };
        let micros = u64::try_from(inicio.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.muestras
            .lock()
            .expect("el mutex de muestras no se envenena: nadie entra en pánico con él tomado")
            .push(Muestra {
                operacion,
                micros,
                estado,
            });
        Respuesta { estado, cuerpo }
    }

    /// Muestras tomadas hasta ahora.
    pub fn muestras(&self) -> Vec<Muestra> {
        self.muestras
            .lock()
            .expect("el mutex de muestras no se envenena")
            .clone()
    }

    /// Descarta las muestras (las de la preparación no cuentan en el informe).
    pub fn reiniciar_muestras(&self) {
        self.muestras
            .lock()
            .expect("el mutex de muestras no se envenena")
            .clear();
    }
}
