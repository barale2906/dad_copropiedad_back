use serde::{Deserialize, Serialize};

use crate::{DominioError, OpcionId, PreguntaId, ReglaMayoria, ReunionId};

/// Ciclo de vida de una pregunta sometida a votación.
///
/// ```text
/// Borrador ──abrir──▶ Abierta ──cerrar──▶ Cerrada
///    │                   │
///    └──────anular───────┴──▶ Anulada
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoPregunta {
    Borrador,
    Abierta,
    Cerrada,
    Anulada,
}

enum_texto!(EstadoPregunta {
    Borrador => "borrador",
    Abierta => "abierta",
    Cerrada => "cerrada",
    Anulada => "anulada",
});

impl EstadoPregunta {
    /// Valida y aplica una transición de estado. Única fuente de verdad de las transiciones.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoPregunta::{Abierta, Anulada, Borrador, Cerrada};
        match (self, hacia) {
            (Borrador, Abierta) | (Abierta, Cerrada) | (Borrador | Abierta, Anulada) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "pregunta",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }

    /// Verifica que la pregunta acepte votos.
    pub fn exigir_abierta(self) -> Result<(), DominioError> {
        if self == Self::Abierta {
            Ok(())
        } else {
            Err(DominioError::PreguntaNoAbierta(self))
        }
    }
}

/// Modalidad del voto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoVoto {
    /// Se registra qué bien privado eligió qué opción (permite actas nominales).
    Nominal,
    /// Solo se registra **que** el bien privado votó; la opción elegida queda desvinculada.
    Secreto,
}

enum_texto!(TipoVoto {
    Nominal => "nominal",
    Secreto => "secreto",
});

/// Pregunta tal como la carga el repositorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pregunta {
    pub id: PreguntaId,
    pub reunion_id: ReunionId,
    pub enunciado: String,
    pub tipo_voto: TipoVoto,
    pub regla_mayoria: ReglaMayoria,
    pub estado: EstadoPregunta,
    pub opciones: Vec<OpcionId>,
}

impl Pregunta {
    /// Única forma de obtener una [`PreguntaAbierta`]. Falla si la pregunta no está abierta.
    pub fn para_votar(&self) -> Result<PreguntaAbierta<'_>, DominioError> {
        self.estado.exigir_abierta()?;
        Ok(PreguntaAbierta(self))
    }
}

/// Prueba, verificada por el compilador, de que la pregunta estaba abierta al validarla.
///
/// No tiene constructor público: solo [`Pregunta::para_votar`] la crea. Como la emisión de
/// votos ([`PreguntaAbierta::emitir`](crate::voto)) solo existe sobre este tipo, «votar en una
/// pregunta cerrada» no compila. La base de datos vuelve a verificar el estado dentro de la
/// transacción (la pregunta pudo cerrarse entre la lectura y la escritura).
#[derive(Debug, Clone, Copy)]
pub struct PreguntaAbierta<'a>(&'a Pregunta);

impl PreguntaAbierta<'_> {
    pub fn pregunta(&self) -> &Pregunta {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::EstadoPregunta::{self, *};

    #[test]
    fn transiciones_validas() {
        assert_eq!(Borrador.transicionar(Abierta).unwrap(), Abierta);
        assert_eq!(Abierta.transicionar(Cerrada).unwrap(), Cerrada);
        assert_eq!(Borrador.transicionar(Anulada).unwrap(), Anulada);
        assert_eq!(Abierta.transicionar(Anulada).unwrap(), Anulada);
    }

    #[test]
    fn transiciones_invalidas() {
        for (desde, hacia) in [
            (Borrador, Cerrada),
            (Cerrada, Abierta),
            (Cerrada, Anulada),
            (Anulada, Abierta),
            (Abierta, Borrador),
        ] {
            assert!(
                desde.transicionar(hacia).is_err(),
                "{desde} -> {hacia} debería fallar"
            );
        }
    }

    #[test]
    fn solo_abierta_recibe_votos() {
        assert!(Abierta.exigir_abierta().is_ok());
        for e in [Borrador, Cerrada, Anulada] {
            assert!(e.exigir_abierta().is_err());
        }
    }

    #[test]
    fn ida_y_vuelta_texto() {
        for e in EstadoPregunta::TODOS {
            assert_eq!(EstadoPregunta::try_from(e.as_str()).unwrap(), *e);
        }
        assert!(EstadoPregunta::try_from("otra").is_err());
    }
}
