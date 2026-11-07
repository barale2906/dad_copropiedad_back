//! Patrón Outbox: los mensajes salientes se graban en la **misma transacción** que el hecho que
//! los origina (con [`crate::puertos::RegistroOutbox`]). Un worker los consume después
//! ([`crate::casos_uso::despachar_salida`]); si el proceso se cae, el mensaje sigue pendiente y
//! no se pierde.
//!
//! El outbox es multicanal: WhatsApp, correo y (más adelante) push.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::puertos::{correo::CorreoSaliente, mensajeria::MensajeSaliente};

/// Prioridad de la cola de salida (menor número = sale primero). En WhatsApp, siempre dentro
/// del límite de 80 mensajes/segundo por número.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrioridadSalida {
    /// Respuesta inmediata a una acción del usuario: confirmación de voto, código de acceso.
    Confirmacion = 1,
    /// Hecho nuevo que el usuario debe conocer: pregunta abierta, cambio de estado de un ticket.
    Evento = 2,
    /// Recordatorios programados: reservas, convocatorias, vencimientos.
    Recordatorio = 3,
    /// Publicidad (con consentimiento); nunca adelanta a lo anterior.
    Publicidad = 9,
}

impl PrioridadSalida {
    /// Valor persistido en `outbox.prioridad` (`SMALLINT`).
    pub const fn as_i16(self) -> i16 {
        self as i16
    }
}

/// Canal por el que sale un mensaje. El texto es contrato con el `CHECK` de `outbox.canal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Canal {
    WhatsApp,
    Correo,
    /// Notificaciones de las apps móviles (previsto).
    Push,
}

impl Canal {
    /// Texto persistido en `outbox.canal`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WhatsApp => "whatsapp",
            Self::Correo => "correo",
            Self::Push => "push",
        }
    }
}

/// Contenido de un mensaje encolado. Se persiste completo en `outbox.payload` (JSONB).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "canal", rename_all = "snake_case")]
pub enum MensajeOutbox {
    #[serde(rename = "whatsapp")]
    WhatsApp(MensajeSaliente),
    Correo(CorreoSaliente),
}

impl MensajeOutbox {
    pub const fn canal(&self) -> Canal {
        match self {
            Self::WhatsApp(_) => Canal::WhatsApp,
            Self::Correo(_) => Canal::Correo,
        }
    }

    /// Destino según el canal: teléfono o dirección de correo (dato personal).
    pub fn destino(&self) -> &str {
        match self {
            Self::WhatsApp(m) => &m.telefono,
            Self::Correo(c) => c.para.as_str(),
        }
    }

    /// Número de WhatsApp emisor; `None` en los demás canales.
    pub fn linea(&self) -> Option<&str> {
        match self {
            Self::WhatsApp(m) => Some(&m.linea),
            Self::Correo(_) => None,
        }
    }
}

/// Evento a encolar en la tabla `outbox`.
///
/// **Secreto del voto**: una confirmación de voto nunca incluye la opción elegida.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventoSaliente {
    pub prioridad: PrioridadSalida,
    pub mensaje: MensajeOutbox,
}

/// Reintentos de envío: espera exponencial (`base · 2^intentos`, con tope) y un máximo de
/// intentos, tras el cual el mensaje queda `fallido` para revisión manual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoliticaReintentos {
    pub max_intentos: u32,
    pub espera_base: Duration,
    pub espera_maxima: Duration,
}

impl PoliticaReintentos {
    /// Espera antes del siguiente intento, sabiendo cuántos intentos fallidos lleva el mensaje
    /// (sin contar el actual).
    pub fn espera(&self, intentos_previos: u32) -> Duration {
        let factor = 2u32.saturating_pow(intentos_previos);
        self.espera_base
            .saturating_mul(factor)
            .min(self.espera_maxima)
    }
}

impl Default for PoliticaReintentos {
    /// 8 intentos: 30 s, 1 min, 2 min, 4 min, 8 min, 16 min, 32 min, 1 h.
    fn default() -> Self {
        Self {
            max_intentos: 8,
            espera_base: Duration::from_secs(30),
            espera_maxima: Duration::from_secs(60 * 60),
        }
    }
}

#[cfg(test)]
mod tests {
    use domain::DireccionCorreo;

    use super::{PrioridadSalida::*, *};
    use crate::puertos::mensajeria::ContenidoSaliente;

    #[test]
    fn confirmaciones_salen_primero_y_publicidad_al_final() {
        let mut v = [Publicidad, Recordatorio, Confirmacion, Evento];
        v.sort();
        assert_eq!(v, [Confirmacion, Evento, Recordatorio, Publicidad]);
        assert_eq!(Confirmacion.as_i16(), 1);
        assert_eq!(Publicidad.as_i16(), 9);
    }

    #[test]
    fn espera_crece_exponencialmente_con_tope() {
        let p = PoliticaReintentos::default();
        assert_eq!(p.espera(0), Duration::from_secs(30));
        assert_eq!(p.espera(1), Duration::from_secs(60));
        assert_eq!(p.espera(3), Duration::from_secs(240));
        assert_eq!(p.espera(10), Duration::from_secs(3600));
        assert_eq!(p.espera(u32::MAX), Duration::from_secs(3600));
    }

    #[test]
    fn canal_destino_y_linea_segun_el_mensaje() {
        let wa = MensajeOutbox::WhatsApp(MensajeSaliente {
            linea: "123".into(),
            telefono: "573001234567".into(),
            contenido: ContenidoSaliente::Texto {
                cuerpo: "hola".into(),
            },
        });
        let correo = MensajeOutbox::Correo(CorreoSaliente {
            para: DireccionCorreo::new("a@b.co").expect("válida"),
            asunto: "Asunto".into(),
            texto: "Texto".into(),
            html: None,
        });

        assert_eq!(
            (wa.canal(), wa.destino(), wa.linea()),
            (Canal::WhatsApp, "573001234567", Some("123"))
        );
        assert_eq!(
            (correo.canal(), correo.destino(), correo.linea()),
            (Canal::Correo, "a@b.co", None)
        );
    }

    #[test]
    fn payload_lleva_el_canal_y_se_recupera_igual() {
        let correo = MensajeOutbox::Correo(CorreoSaliente {
            para: DireccionCorreo::new("a@b.co").expect("válida"),
            asunto: "Asunto".into(),
            texto: "Texto".into(),
            html: Some("<p>Texto</p>".into()),
        });

        let json = serde_json::to_value(&correo).expect("serializa");
        assert_eq!(json["canal"], "correo");
        let de_vuelta: MensajeOutbox = serde_json::from_value(json).expect("deserializa");
        assert_eq!(de_vuelta, correo);
    }
}
