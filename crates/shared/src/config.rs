//! Configuración de la aplicación leída desde variables de entorno (y `.env` en desarrollo).

use std::{env, net::SocketAddr, str::FromStr, time::Duration};

/// Configuración ausente o inválida.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("falta la variable de entorno obligatoria `{0}`")]
    Missing(&'static str),
    #[error("valor inválido para `{var}`: {detail}")]
    Invalid { var: &'static str, detail: String },
}

/// Formato de los logs (`LOG_FORMAT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Legible para humanos (desarrollo local).
    Pretty,
    /// Una línea JSON por evento (producción / agregadores de logs).
    Json,
}

impl FromStr for LogFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pretty" => Ok(Self::Pretty),
            "json" => Ok(Self::Json),
            other => Err(format!(
                "se esperaba `pretty` o `json`, se recibió `{other}`"
            )),
        }
    }
}

/// Configuración común de los binarios (`api`, `gateway`).
#[derive(Clone)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    /// Conexión de la aplicación con el rol restringido `dad_app` (`APP_DATABASE_URL`).
    /// RLS solo aísla tenants si este rol no es superusuario ni tiene `BYPASSRLS`.
    pub database_url: String,
    /// Conexión del dueño del esquema (`DATABASE_URL`), usada solo para migraciones y por
    /// `sqlx-cli` / `#[sqlx::test]`. Obligatoria si `run_migrations = true`.
    pub migrations_database_url: Option<String>,
    pub database_max_connections: u32,
    pub run_migrations: bool,
    pub request_timeout: Duration,
    pub log_format: LogFormat,
    /// Publica la documentación interactiva (`/docs`) y el contrato (`/api-docs/openapi.json`).
    /// Desactivada por defecto: en producción solo se activa si el acceso está protegido.
    pub docs_api_habilitada: bool,
}

// `Debug` manual para no imprimir credenciales de la base de datos en los logs.
impl std::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database_url", &"<oculto>")
            .field(
                "migrations_database_url",
                &self.migrations_database_url.as_ref().map(|_| "<oculto>"),
            )
            .field("database_max_connections", &self.database_max_connections)
            .field("run_migrations", &self.run_migrations)
            .field("request_timeout", &self.request_timeout)
            .field("log_format", &self.log_format)
            .field("docs_api_habilitada", &self.docs_api_habilitada)
            .finish()
    }
}

impl AppConfig {
    /// Configuración del binario `api` (puerto en `APP_PORT`, 8080 por defecto).
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::cargar("APP_PORT", 8080)
    }

    /// Carga `.env` si existe (solo útil en desarrollo) y luego lee las variables de entorno.
    /// `var_puerto` permite que cada binario escuche en su propio puerto.
    pub fn cargar(var_puerto: &'static str, puerto_defecto: u16) -> Result<Self, ConfigError> {
        // En producción no existe `.env`; ignorar el error es intencional.
        let _ = dotenvy::dotenv();

        let config = Self {
            host: optional("APP_HOST").unwrap_or_else(|| "0.0.0.0".to_owned()),
            port: parse_or(var_puerto, puerto_defecto)?,
            database_url: required("APP_DATABASE_URL")?,
            migrations_database_url: optional("DATABASE_URL"),
            database_max_connections: parse_or("DATABASE_MAX_CONNECTIONS", 10)?,
            run_migrations: parse_or("RUN_MIGRATIONS", false)?,
            request_timeout: Duration::from_secs(parse_or("REQUEST_TIMEOUT_SECS", 30)?),
            log_format: parse_or("LOG_FORMAT", LogFormat::Pretty)?,
            docs_api_habilitada: parse_or("API_DOCS_HABILITADA", false)?,
        };
        if config.run_migrations && config.migrations_database_url.is_none() {
            return Err(ConfigError::Missing("DATABASE_URL"));
        }
        Ok(config)
    }

    /// Dirección donde escucha el servidor.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Invalid`] si `APP_HOST` y el puerto no forman una dirección.
    pub fn socket_addr(&self) -> Result<SocketAddr, ConfigError> {
        format!("{}:{}", self.host, self.port)
            .parse()
            .map_err(|e: std::net::AddrParseError| ConfigError::Invalid {
                var: "APP_HOST/puerto",
                detail: e.to_string(),
            })
    }
}

/// Credenciales de WhatsApp Cloud API (solo las necesita el `gateway`).
#[derive(Clone)]
pub struct WhatsappConfig {
    /// Token que Meta envía en `hub.verify_token` al suscribir el webhook.
    pub verify_token: String,
    /// *App secret* de Meta: clave HMAC de `X-Hub-Signature-256`.
    pub app_secret: String,
    /// Token de acceso (usuario del sistema) para enviar mensajes. `None` = el gateway recibe
    /// pero no envía por WhatsApp (los mensajes quedan pendientes en el outbox).
    pub access_token: Option<String>,
    /// Base de la Graph API con versión (`WHATSAPP_API_URL`).
    pub api_url: String,
}

/// Graph API por defecto (se fija la versión para que un cambio de Meta no cambie el contrato).
pub const WHATSAPP_API_URL_PREDETERMINADA: &str = "https://graph.facebook.com/v23.0";

impl std::fmt::Debug for WhatsappConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhatsappConfig")
            .field("verify_token", &"<oculto>")
            .field("app_secret", &"<oculto>")
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| "<oculto>"),
            )
            .field("api_url", &self.api_url)
            .finish()
    }
}

impl WhatsappConfig {
    /// Configuración de WhatsApp desde las variables `WHATSAPP_*`.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] si falta una variable obligatoria o es inválida.
    pub fn from_env() -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        Ok(Self {
            verify_token: required("WHATSAPP_VERIFY_TOKEN")?,
            app_secret: required("WHATSAPP_APP_SECRET")?,
            access_token: optional("WHATSAPP_ACCESS_TOKEN"),
            api_url: optional("WHATSAPP_API_URL")
                .unwrap_or_else(|| WHATSAPP_API_URL_PREDETERMINADA.into()),
        })
    }
}

/// Cifrado de la conexión SMTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeguridadSmtp {
    /// Conexión en claro que se cifra con STARTTLS (puerto 587). Recomendado.
    Starttls,
    /// TLS desde el inicio (puerto 465).
    Tls,
    /// Sin cifrado: **solo desarrollo** (Mailpit en `localhost:1025`).
    Ninguna,
}

impl FromStr for SeguridadSmtp {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "starttls" => Ok(Self::Starttls),
            "tls" => Ok(Self::Tls),
            "ninguna" => Ok(Self::Ninguna),
            otro => Err(format!(
                "se esperaba `starttls`, `tls` o `ninguna`, se recibió `{otro}`"
            )),
        }
    }
}

/// Servidor de correo saliente (en producción, el SMTP de HestiaCP del VPS).
#[derive(Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub puerto: u16,
    pub seguridad: SeguridadSmtp,
    /// Usuario y contraseña; ambos vacíos = sin autenticación (Mailpit).
    pub usuario: Option<String>,
    pub contrasena: Option<String>,
    /// Remitente de todos los correos, p. ej. `DAD Copropiedad <no-responder@dominio.co>`.
    pub remitente: String,
}

impl std::fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("puerto", &self.puerto)
            .field("seguridad", &self.seguridad)
            .field("usuario", &self.usuario.as_ref().map(|_| "<oculto>"))
            .field("contrasena", &self.contrasena.as_ref().map(|_| "<oculto>"))
            .field("remitente", &self.remitente)
            .finish()
    }
}

impl SmtpConfig {
    /// Lee `SMTP_*`. Devuelve `None` si `SMTP_HOST` no está definido: el envío de correo queda
    /// desactivado (el gateway lo advierte al arrancar).
    ///
    /// # Errors
    ///
    /// [`ConfigError`] si `SMTP_HOST` existe pero falta `SMTP_REMITENTE`, si un valor es
    /// inválido o si se define solo uno de usuario y contraseña.
    pub fn from_env() -> Result<Option<Self>, ConfigError> {
        let _ = dotenvy::dotenv();
        let Some(host) = optional("SMTP_HOST") else {
            return Ok(None);
        };
        let usuario = optional("SMTP_USUARIO");
        let contrasena = optional("SMTP_CONTRASENA");
        if usuario.is_some() != contrasena.is_some() {
            return Err(ConfigError::Invalid {
                var: "SMTP_USUARIO/SMTP_CONTRASENA",
                detail: "defina ambos o ninguno".into(),
            });
        }
        Ok(Some(Self {
            host,
            puerto: parse_or("SMTP_PUERTO", 587)?,
            seguridad: parse_or("SMTP_SEGURIDAD", SeguridadSmtp::Starttls)?,
            usuario,
            contrasena,
            remitente: required("SMTP_REMITENTE")?,
        }))
    }
}

/// Almacén de archivos configurado (`ARCHIVOS_ALMACEN`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoAlmacen {
    /// Disco del servidor, bajo `ARCHIVOS_DIR`.
    Local,
}

impl FromStr for TipoAlmacen {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "local" => Ok(Self::Local),
            "s3" => Err("el almacén `s3` aún no está implementado (ADR 0003)".into()),
            otro => Err(format!("se esperaba `local` o `s3`, se recibió `{otro}`")),
        }
    }
}

/// Archivos y adjuntos (ADR 0003).
#[derive(Clone)]
pub struct ArchivosConfig {
    pub almacen: TipoAlmacen,
    /// Directorio raíz del almacén local. Debe estar en la copia de seguridad del VPS.
    pub dir: String,
    /// Secreto HMAC de los enlaces de descarga (≥ 32 caracteres). Rotarlo invalida los enlaces
    /// vigentes, que duran minutos.
    pub secreto_enlaces: String,
    /// URL pública de la API con la que se arman los enlaces (p. ej. `https://api.dominio.co`).
    pub url_publica: String,
}

impl std::fmt::Debug for ArchivosConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchivosConfig")
            .field("almacen", &self.almacen)
            .field("dir", &self.dir)
            .field("secreto_enlaces", &"<oculto>")
            .field("url_publica", &self.url_publica)
            .finish()
    }
}

impl ArchivosConfig {
    /// Lee `ARCHIVOS_*`.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] si falta `ARCHIVOS_SECRETO_ENLACES` o `ARCHIVOS_URL_PUBLICA`, o si
    /// `ARCHIVOS_ALMACEN` no es un almacén disponible.
    pub fn from_env() -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        Ok(Self {
            almacen: parse_or("ARCHIVOS_ALMACEN", TipoAlmacen::Local)?,
            dir: optional("ARCHIVOS_DIR").unwrap_or_else(|| "./datos/archivos".into()),
            secreto_enlaces: required("ARCHIVOS_SECRETO_ENLACES")?,
            url_publica: required("ARCHIVOS_URL_PUBLICA")?,
        })
    }
}

/// Control de acceso (plan 05): secreto de los QR de visitas.
#[derive(Clone)]
pub struct AccesoConfig {
    /// Secreto maestro (≥ 32 caracteres) del que se deriva la clave Ed25519 de cada PH
    /// (`ACCESO_QR_SECRETO`). Rotarlo invalida los QR de visitas emitidos.
    pub qr_secreto: String,
}

impl std::fmt::Debug for AccesoConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccesoConfig")
            .field("qr_secreto", &"<oculto>")
            .finish()
    }
}

impl AccesoConfig {
    /// Lee `ACCESO_QR_SECRETO`.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] si falta o tiene menos de 32 caracteres.
    pub fn from_env() -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        let qr_secreto = required("ACCESO_QR_SECRETO")?;
        if qr_secreto.len() < 32 {
            return Err(ConfigError::Invalid {
                var: "ACCESO_QR_SECRETO",
                detail: "debe tener al menos 32 caracteres".into(),
            });
        }
        Ok(Self { qr_secreto })
    }
}

/// Base de la API de Firebase Cloud Messaging.
pub const FCM_API_URL_PREDETERMINADA: &str = "https://fcm.googleapis.com";

/// Notificaciones push de las apps Flutter (Firebase Cloud Messaging HTTP v1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushConfig {
    /// Ruta del JSON de la cuenta de servicio de Firebase (`FCM_CREDENCIALES`). `None` = el
    /// gateway no envía push (los avisos de acceso salen por WhatsApp).
    pub credenciales: Option<String>,
    /// Base de la API (`FCM_API_URL`).
    pub api_url: String,
}

impl PushConfig {
    /// Lee `FCM_*`.
    ///
    /// # Errors
    ///
    /// No falla hoy; se deja `Result` por consistencia con el resto de la configuración.
    pub fn from_env() -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        Ok(Self {
            credenciales: optional("FCM_CREDENCIALES"),
            api_url: optional("FCM_API_URL").unwrap_or_else(|| FCM_API_URL_PREDETERMINADA.into()),
        })
    }
}

/// Puente de acceso en sitio (`apps/puente-acceso`).
#[derive(Clone)]
pub struct PuenteConfig {
    /// Base de la API (`PUENTE_API_URL`, p. ej. `https://api.dominio.co/api/v1`).
    pub api_url: String,
    /// Token de máquina del puente (`PUENTE_TOKEN`), entregado al registrarlo.
    pub token: String,
    /// Directorio de la cache local y la cola de eventos (`PUENTE_DATOS_DIR`).
    pub datos_dir: String,
    /// Zona horaria IANA de la PH para evaluar horarios sin internet (`PUENTE_ZONA_HORARIA`).
    pub zona_horaria: String,
    /// Segundos entre sincronizaciones (`PUENTE_INTERVALO_SEGUNDOS`, 20 por defecto: una
    /// credencial revocada sale de la cache en menos de un minuto).
    pub intervalo_segundos: u64,
    pub log_format: LogFormat,
}

impl std::fmt::Debug for PuenteConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PuenteConfig")
            .field("api_url", &self.api_url)
            .field("token", &"<oculto>")
            .field("datos_dir", &self.datos_dir)
            .field("zona_horaria", &self.zona_horaria)
            .field("intervalo_segundos", &self.intervalo_segundos)
            .finish()
    }
}

impl PuenteConfig {
    /// Lee `PUENTE_*`.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] si falta `PUENTE_API_URL` o `PUENTE_TOKEN`, o un valor es inválido.
    pub fn from_env() -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        Ok(Self {
            api_url: required("PUENTE_API_URL")?,
            token: required("PUENTE_TOKEN")?,
            datos_dir: optional("PUENTE_DATOS_DIR").unwrap_or_else(|| "./datos/puente".into()),
            zona_horaria: optional("PUENTE_ZONA_HORARIA")
                .unwrap_or_else(|| "America/Bogota".into()),
            intervalo_segundos: parse_or("PUENTE_INTERVALO_SEGUNDOS", 20)?,
            log_format: parse_or("LOG_FORMAT", LogFormat::Pretty)?,
        })
    }
}

/// Límite de peticiones por IP de la API (`LIMITE_*`, `CONFIAR_PROXY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitesConfig {
    /// Registro, inicio de sesión, segundo factor y recuperar contraseña (10 por defecto).
    pub auth_por_minuto: u32,
    /// Papeleta, puentes y descarga con enlace (120 por defecto).
    pub publico_por_minuto: u32,
    /// Tomar la IP real de `X-Real-IP` (solo detrás de Nginx propio; `false` por defecto).
    pub confiar_proxy: bool,
}

impl LimitesConfig {
    /// # Errors
    ///
    /// [`ConfigError::Invalid`] si un valor no es un número o un booleano.
    pub fn from_env() -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        Ok(Self {
            auth_por_minuto: parse_or("LIMITE_AUTH_POR_MINUTO", 10)?,
            publico_por_minuto: parse_or("LIMITE_PUBLICO_POR_MINUTO", 120)?,
            confiar_proxy: parse_or("CONFIAR_PROXY", false)?,
        })
    }
}

/// Seguridad de las cuentas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeguridadConfig {
    /// Operar en el ámbito de plataforma exige una sesión con segundo factor (TOTP)
    /// (`PLATAFORMA_EXIGE_SEGUNDO_FACTOR`, `true` por defecto). Solo se apaga en desarrollo.
    pub plataforma_exige_segundo_factor: bool,
}

impl SeguridadConfig {
    /// # Errors
    ///
    /// [`ConfigError::Invalid`] si el valor no es un booleano.
    pub fn from_env() -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        Ok(Self {
            plataforma_exige_segundo_factor: parse_or("PLATAFORMA_EXIGE_SEGUNDO_FACTOR", true)?,
        })
    }
}

fn optional(var: &'static str) -> Option<String> {
    env::var(var).ok().filter(|v| !v.trim().is_empty())
}

fn required(var: &'static str) -> Result<String, ConfigError> {
    optional(var).ok_or(ConfigError::Missing(var))
}

fn parse_or<T>(var: &'static str, default: T) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    match optional(var) {
        None => Ok(default),
        Some(raw) => raw.parse().map_err(|e: T::Err| ConfigError::Invalid {
            var,
            detail: e.to_string(),
        }),
    }
}
