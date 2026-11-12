//! Asistentes de una asamblea: la persona presente (propietario o apoderado) que representa uno
//! o varios bienes privados y vota **una vez por todos** (decisión del 2026-11-08).

use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Cómo quedó registrado el asistente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetodoRegistro {
    /// Registro asistido en la mesa el día de la reunión (firma de la planilla).
    Mesa,
    /// Prerregistro de la administración antes de la reunión.
    Prerregistro,
    /// La persona ingresó con su enlace personal (papeleta virtual).
    Enlace,
    /// Consulta por escrito (art. 43): cada representante queda habilitado al convocar.
    Consulta,
}

enum_texto!(MetodoRegistro {
    Mesa => "mesa",
    Prerregistro => "prerregistro",
    Enlace => "enlace",
    Consulta => "consulta",
});

/// En qué calidad el asistente representa un bien privado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalidadRepresentacion {
    /// Es el representante del bien ante la copropiedad (titularidad con `es_representante`).
    Propietario,
    /// Representa el bien mediante poder (Ley 675, art. 37). Queda en el acta.
    Apoderado,
}

enum_texto!(CalidadRepresentacion {
    Propietario => "propietario",
    Apoderado => "apoderado",
});

/// Estado de un poder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoPoder {
    /// Recibido; la verificación del soporte puede quedar para después sin frenar la fila.
    Pendiente,
    Verificado,
    Rechazado,
    /// El poderdante lo revocó (o asistió en persona).
    Revocado,
}

enum_texto!(EstadoPoder {
    Pendiente => "pendiente",
    Verificado => "verificado",
    Rechazado => "rechazado",
    Revocado => "revocado",
});

impl EstadoPoder {
    /// ¿El apoderado puede representar el bien con este poder?
    pub const fn vigente(self) -> bool {
        matches!(self, Self::Pendiente | Self::Verificado)
    }

    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoPoder::{Pendiente, Rechazado, Revocado, Verificado};
        match (self, hacia) {
            (Pendiente, Verificado | Rechazado | Revocado) | (Verificado, Revocado) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "poder",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }
}

/// Motivo de salida de un asistente: sus bienes dejan el quórum y las preguntas que se abran
/// después; sus votos ya emitidos se conservan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotivoSalida {
    SeRetiro,
    DelegoPoder,
    ProblemaTecnico,
    Otro,
}

enum_texto!(MotivoSalida {
    SeRetiro => "se_retiro",
    DelegoPoder => "delego_poder",
    ProblemaTecnico => "problema_tecnico",
    Otro => "otro",
});

/// Momento de una foto del quórum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MomentoQuorum {
    Instalacion,
    Apertura,
    Cierre,
    Manual,
}

enum_texto!(MomentoQuorum {
    Instalacion => "instalacion",
    Apertura => "apertura",
    Cierre => "cierre",
    Manual => "manual",
});

/// Código de un rótulo preimpreso (Code 128) que la mesa liga a un asistente.
///
/// Se normaliza a mayúsculas; admite letras, dígitos y guion (hasta 32), lo que un lector USB
/// de código de barras «teclea» al escanear.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CodigoRotulo(String);

impl CodigoRotulo {
    /// Caracteres máximos del código.
    pub const LONGITUD_MAXIMA: usize = 32;

    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si está vacío, es muy largo o tiene otros caracteres.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let v = valor.trim().to_ascii_uppercase();
        if v.is_empty()
            || v.len() > Self::LONGITUD_MAXIMA
            || !v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(DominioError::DatoInvalido {
                campo: "rotulo",
                regla: "de 1 a 32 letras, dígitos o guiones",
            });
        }
        Ok(Self(v))
    }

    /// Código del paquete de rótulos genéricos: `prefijo` + número con ceros (p. ej. `R-0042`).
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si el resultado no es un código válido.
    pub fn numerado(prefijo: &str, numero: u32, digitos: usize) -> Result<Self, DominioError> {
        Self::new(&format!("{prefijo}{numero:0digitos$}"))
    }

    /// Código normalizado.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CodigoRotulo {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<CodigoRotulo> for String {
    fn from(c: CodigoRotulo) -> Self {
        c.0
    }
}

/// Cargos que pueden prohibirse como apoderados en una asamblea (códigos de rol plantilla).
///
/// La ley no lo prohíbe expresamente para todos; lo define el reglamento de cada PH (p. ej. el
/// administrador y el revisor fiscal suelen tener prohibido representar).
pub const CARGOS_PROHIBIBLES: &[&str] = &[
    "administrador",
    "asistente_administrativo",
    "contador",
    "revisor_fiscal",
    "presidente_consejo",
    "consejero",
    "comite_convivencia",
    "vigilante",
    "supervisor_seguridad",
    "aseo",
    "mantenimiento",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotulo_normaliza_y_valida() {
        assert_eq!(CodigoRotulo::new(" r-0042 ").unwrap().as_str(), "R-0042");
        assert!(CodigoRotulo::new("").is_err());
        assert!(CodigoRotulo::new("R 1").is_err());
        assert!(CodigoRotulo::new(&"A".repeat(33)).is_err());
        assert_eq!(
            CodigoRotulo::numerado("R-", 7, 4).unwrap().as_str(),
            "R-0007"
        );
    }

    #[test]
    fn poderes_vigentes_y_transiciones() {
        assert!(EstadoPoder::Pendiente.vigente() && EstadoPoder::Verificado.vigente());
        assert!(!EstadoPoder::Revocado.vigente());
        assert!(
            EstadoPoder::Pendiente
                .transicionar(EstadoPoder::Verificado)
                .is_ok()
        );
        assert!(
            EstadoPoder::Verificado
                .transicionar(EstadoPoder::Revocado)
                .is_ok()
        );
        assert!(
            EstadoPoder::Rechazado
                .transicionar(EstadoPoder::Verificado)
                .is_err()
        );
    }

    #[test]
    fn textos_ida_y_vuelta() {
        for m in MotivoSalida::TODOS {
            assert_eq!(MotivoSalida::try_from(m.as_str()).unwrap(), *m);
        }
        for m in MetodoRegistro::TODOS {
            assert_eq!(MetodoRegistro::try_from(m.as_str()).unwrap(), *m);
        }
    }
}
