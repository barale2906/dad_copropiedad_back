//! Registro (inscripción) de bienes privados a una reunión.

use serde::{Deserialize, Serialize};

/// Cómo quedó inscrito el bien privado en la reunión.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetodoRegistro {
    /// Enlace prellenado enviado por WhatsApp/correo; el asistente confirma con botones.
    Enlace,
    /// Prerregistro hecho por la administración antes de la reunión.
    Prerregistro,
    /// Lectura del QR en la puerta el día de la reunión.
    QrPuerta,
}

enum_texto!(MetodoRegistro {
    Enlace => "enlace",
    Prerregistro => "prerregistro",
    QrPuerta => "qr_puerta",
});

/// Calidad en la que la persona representa al bien privado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolAsistente {
    Propietario,
    /// Representa al bien privado mediante poder (Ley 675, art. 37). Queda en el acta.
    Apoderado,
}

enum_texto!(RolAsistente {
    Propietario => "propietario",
    Apoderado => "apoderado",
});
