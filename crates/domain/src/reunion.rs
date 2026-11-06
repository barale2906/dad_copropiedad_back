use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Ciclo de vida de una reunión de asamblea.
///
/// ```text
/// Convocada ──instalar──▶ Instalada ──cerrar──▶ Cerrada
///     │
///     └──cancelar──▶ Cancelada
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoReunion {
    /// Convocatoria enviada; se aceptan inscripciones (enlace, prerregistro, QR en puerta).
    Convocada,
    /// Quórum verificado y sesión instalada; sus preguntas pueden abrirse.
    Instalada,
    Cerrada,
    Cancelada,
}

enum_texto!(EstadoReunion {
    Convocada => "convocada",
    Instalada => "instalada",
    Cerrada => "cerrada",
    Cancelada => "cancelada",
});

impl EstadoReunion {
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoReunion::{Cancelada, Cerrada, Convocada, Instalada};
        match (self, hacia) {
            (Convocada, Instalada | Cancelada) | (Instalada, Cerrada) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "reunión",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }

    /// Una pregunta solo puede abrirse en una reunión instalada.
    pub fn exigir_instalada(self) -> Result<(), DominioError> {
        if self == Self::Instalada {
            Ok(())
        } else {
            Err(DominioError::ReunionNoInstalada(self))
        }
    }

    /// Reuniones en las que un inscrito puede interactuar (registrarse o votar).
    pub const fn activa(self) -> bool {
        matches!(self, Self::Convocada | Self::Instalada)
    }
}

/// Tipo de reunión según Ley 675 de 2001 (art. 39).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoReunion {
    Ordinaria,
    Extraordinaria,
}

enum_texto!(TipoReunion {
    Ordinaria => "ordinaria",
    Extraordinaria => "extraordinaria",
});

#[cfg(test)]
mod tests {
    use super::EstadoReunion::{self, *};

    #[test]
    fn transiciones() {
        assert_eq!(Convocada.transicionar(Instalada).unwrap(), Instalada);
        assert_eq!(Instalada.transicionar(Cerrada).unwrap(), Cerrada);
        assert_eq!(Convocada.transicionar(Cancelada).unwrap(), Cancelada);
        for (d, h) in [
            (Cerrada, Instalada),
            (Instalada, Convocada),
            (Cancelada, Instalada),
        ] {
            assert!(d.transicionar(h).is_err());
        }
    }

    #[test]
    fn solo_instalada_permite_abrir_preguntas() {
        assert!(Instalada.exigir_instalada().is_ok());
        assert!(Convocada.exigir_instalada().is_err());
    }

    #[test]
    fn ida_y_vuelta_texto() {
        for e in EstadoReunion::TODOS {
            assert_eq!(EstadoReunion::try_from(e.as_str()).unwrap(), *e);
        }
    }
}
