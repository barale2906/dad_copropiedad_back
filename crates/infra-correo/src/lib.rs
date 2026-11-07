//! Adaptador de correo saliente por SMTP. Implementa el puerto
//! [`application::puertos::EnvioCorreo`].
//!
//! En producción se usa el servidor de correo de HestiaCP del VPS; en desarrollo, Mailpit
//! (`docker compose`, interfaz en <http://localhost:8025>). Cambiar de proveedor es cambiar las
//! variables `SMTP_*` ([`shared::SmtpConfig`]); la aplicación no se entera.
//!
//! TLS con rustls (sin OpenSSL). El transporte está detrás del trait [`Transporte`] para probar
//! la construcción de los mensajes sin un servidor real.

use std::{error::Error as StdError, future::Future, time::Duration};

use application::{
    AppError, AppResult,
    puertos::{EnvioCorreo, correo::CorreoSaliente},
};
use lettre::{
    Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart, SinglePart, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use shared::{SeguridadSmtp, SmtpConfig};

/// Tiempo máximo de una conversación SMTP antes de darla por fallida (el worker reintenta).
const TIEMPO_MAXIMO: Duration = Duration::from_secs(20);

/// Errores al preparar o enviar un correo.
#[derive(Debug, thiserror::Error)]
pub enum ErrorCorreo {
    #[error("remitente inválido en SMTP_REMITENTE: {0}")]
    Remitente(#[source] lettre::address::AddressError),

    #[error("configuración SMTP inválida: {0}")]
    Configuracion(#[source] lettre::transport::smtp::Error),

    #[error("no se pudo construir el mensaje: {0}")]
    Mensaje(String),

    #[error("el servidor SMTP rechazó o no recibió el mensaje: {0}")]
    Envio(#[source] Box<dyn StdError + Send + Sync>),
}

/// Medio que entrega un [`Message`] ya construido. Devuelve la referencia del servidor si la da.
pub trait Transporte: Send + Sync {
    fn entregar(
        &self,
        mensaje: Message,
    ) -> impl Future<Output = Result<Option<String>, ErrorCorreo>> + Send;
}

impl Transporte for AsyncSmtpTransport<Tokio1Executor> {
    async fn entregar(&self, mensaje: Message) -> Result<Option<String>, ErrorCorreo> {
        let respuesta = self
            .send(mensaje)
            .await
            .map_err(|e| ErrorCorreo::Envio(Box::new(e)))?;
        // Típicamente "2.0.0 Ok: queued as ABC123": sirve para rastrear el envío en el servidor.
        Ok(respuesta.message().next().map(str::to_owned))
    }
}

/// Envío de correo con un remitente fijo.
#[derive(Debug, Clone)]
pub struct CorreoSmtp<T> {
    transporte: T,
    remitente: Mailbox,
}

impl CorreoSmtp<AsyncSmtpTransport<Tokio1Executor>> {
    /// Construye el cliente SMTP a partir de la configuración. No abre conexión todavía.
    ///
    /// # Errors
    ///
    /// [`ErrorCorreo`] si el remitente o el host no son válidos.
    ///
    /// # Panics
    ///
    /// Si se llama fuera de un runtime de Tokio (el pool de conexiones de lettre lo necesita).
    pub fn desde_config(config: &SmtpConfig) -> Result<Self, ErrorCorreo> {
        let constructor = match config.seguridad {
            SeguridadSmtp::Starttls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
                    .map_err(ErrorCorreo::Configuracion)?
            }
            SeguridadSmtp::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                .map_err(ErrorCorreo::Configuracion)?,
            SeguridadSmtp::Ninguna => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
            }
        };
        let mut constructor = constructor.port(config.puerto).timeout(Some(TIEMPO_MAXIMO));
        if let (Some(usuario), Some(contrasena)) = (&config.usuario, &config.contrasena) {
            constructor =
                constructor.credentials(Credentials::new(usuario.clone(), contrasena.clone()));
        }
        Self::new(constructor.build(), &config.remitente)
    }
}

impl<T: Transporte> CorreoSmtp<T> {
    /// Cliente con un transporte dado (las pruebas usan uno en memoria).
    ///
    /// # Errors
    ///
    /// [`ErrorCorreo::Remitente`] si `remitente` no es una dirección válida.
    pub fn new(transporte: T, remitente: &str) -> Result<Self, ErrorCorreo> {
        let remitente = remitente.parse().map_err(ErrorCorreo::Remitente)?;
        Ok(Self {
            transporte,
            remitente,
        })
    }

    fn construir(&self, correo: &CorreoSaliente) -> Result<Message, ErrorCorreo> {
        let para: Address = correo
            .para
            .as_str()
            .parse()
            .map_err(|e: lettre::address::AddressError| ErrorCorreo::Mensaje(e.to_string()))?;
        let constructor = Message::builder()
            .from(self.remitente.clone())
            .to(Mailbox::new(None, para))
            .subject(&correo.asunto);

        let resultado = match &correo.html {
            Some(html) => constructor.multipart(MultiPart::alternative_plain_html(
                correo.texto.clone(),
                html.clone(),
            )),
            None => constructor.singlepart(
                SinglePart::builder()
                    .header(ContentType::TEXT_PLAIN)
                    .body(correo.texto.clone()),
            ),
        };
        resultado.map_err(|e| ErrorCorreo::Mensaje(e.to_string()))
    }
}

impl<T: Transporte> EnvioCorreo for CorreoSmtp<T> {
    async fn enviar(&self, correo: &CorreoSaliente) -> AppResult<Option<String>> {
        let mensaje = self.construir(correo).map_err(AppError::infra)?;
        self.transporte
            .entregar(mensaje)
            .await
            .map_err(AppError::infra)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use domain::DireccionCorreo;

    use super::*;

    /// Guarda los mensajes en memoria, ya formateados como los vería el servidor.
    #[derive(Default, Clone)]
    struct TransporteEnMemoria(Arc<Mutex<Vec<String>>>);

    impl Transporte for TransporteEnMemoria {
        async fn entregar(&self, mensaje: Message) -> Result<Option<String>, ErrorCorreo> {
            let texto = String::from_utf8_lossy(&mensaje.formatted()).into_owned();
            self.0.lock().expect("mutex").push(texto);
            Ok(Some("en-memoria".into()))
        }
    }

    fn correo(html: Option<&str>) -> CorreoSaliente {
        CorreoSaliente {
            para: DireccionCorreo::new("residente@correo.co").expect("válida"),
            asunto: "Reserva aprobada".into(),
            texto: "Su reserva fue aprobada.".into(),
            html: html.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn envia_texto_plano_con_remitente_y_destino() {
        let transporte = TransporteEnMemoria::default();
        let cliente = CorreoSmtp::new(transporte.clone(), "DAD Copropiedad <no-responder@dad.co>")
            .expect("remitente válido");

        let referencia = cliente.enviar(&correo(None)).await.expect("se envía");

        assert_eq!(referencia.as_deref(), Some("en-memoria"));
        let enviado = transporte.0.lock().expect("mutex")[0].clone();
        assert!(
            enviado.contains("From: \"DAD Copropiedad\" <no-responder@dad.co>"),
            "{enviado}"
        );
        assert!(enviado.contains("To: residente@correo.co"), "{enviado}");
        assert!(enviado.contains("Subject: Reserva aprobada"), "{enviado}");
        assert!(enviado.contains("Content-Type: text/plain"), "{enviado}");
    }

    #[tokio::test]
    async fn con_html_envia_alternativa_texto_y_html() {
        let transporte = TransporteEnMemoria::default();
        let cliente =
            CorreoSmtp::new(transporte.clone(), "no-responder@dad.co").expect("remitente válido");

        cliente
            .enviar(&correo(Some("<p>Su reserva fue <b>aprobada</b>.</p>")))
            .await
            .expect("se envía");

        let enviado = transporte.0.lock().expect("mutex")[0].clone();
        assert!(enviado.contains("multipart/alternative"), "{enviado}");
        assert!(enviado.contains("text/html"), "{enviado}");
    }

    #[test]
    fn remitente_invalido_se_rechaza_al_crear_el_cliente() {
        assert!(matches!(
            CorreoSmtp::new(TransporteEnMemoria::default(), "no es un correo"),
            Err(ErrorCorreo::Remitente(_))
        ));
    }

    #[tokio::test]
    async fn configuracion_de_desarrollo_construye_el_cliente() {
        let config = SmtpConfig {
            host: "localhost".into(),
            puerto: 1025,
            seguridad: SeguridadSmtp::Ninguna,
            usuario: None,
            contrasena: None,
            remitente: "DAD Copropiedad <no-responder@localhost.dev>".into(),
        };
        assert!(CorreoSmtp::desde_config(&config).is_ok());
    }
}
