//! Patrón Outbox: los mensajes salientes se graban en la **misma transacción** que el hecho que
//! los origina (p. ej. el voto). El gateway los consume con `FOR UPDATE SKIP LOCKED`; si el
//! proceso se cae, el mensaje sigue pendiente y no se pierde.

use serde::{Deserialize, Serialize};

use crate::puertos::mensajeria::MensajeSaliente;

/// Prioridad de la cola de salida (menor número = sale primero), siempre dentro del límite de
/// 80 mensajes/segundo por número de WhatsApp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrioridadSalida {
    ConfirmacionVoto = 1,
    PreguntaNueva = 2,
    Recordatorio = 3,
}

impl PrioridadSalida {
    /// Valor persistido en `outbox.prioridad` (`SMALLINT`).
    pub const fn as_i16(self) -> i16 {
        self as i16
    }
}

/// Evento a encolar en la tabla `outbox`.
///
/// **Secreto del voto**: una confirmación de voto nunca incluye la opción elegida.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventoSaliente {
    pub prioridad: PrioridadSalida,
    pub mensaje: MensajeSaliente,
}

#[cfg(test)]
mod tests {
    use super::PrioridadSalida::*;

    #[test]
    fn confirmaciones_salen_primero() {
        let mut v = [Recordatorio, ConfirmacionVoto, PreguntaNueva];
        v.sort();
        assert_eq!(v, [ConfirmacionVoto, PreguntaNueva, Recordatorio]);
        assert_eq!(ConfirmacionVoto.as_i16(), 1);
    }
}
