//! Componentes transversales compartidos por todos los módulos de DAD_COPROPIEDAD.
//!
//! Este crate NO depende de Axum ni de la base de datos: configuración, telemetría y apagado.

pub mod apagado;
pub mod config;
pub mod telemetry;

pub use config::{
    AccesoConfig, AppConfig, ArchivosConfig, ConfigError, FCM_API_URL_PREDETERMINADA, LogFormat,
    PuenteConfig, PushConfig, SeguridadConfig, SeguridadSmtp, SmtpConfig, TipoAlmacen,
    WHATSAPP_API_URL_PREDETERMINADA, WhatsappConfig,
};
