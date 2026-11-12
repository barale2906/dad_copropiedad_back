//! Titularidad de los bienes privados: quiénes son sus titulares, en qué porcentaje y quién
//! los representa ante la copropiedad.
//!
//! Reglas del conjunto de titulares **vigentes** de un bien privado:
//! - Por cada clase de derecho ([`ClaseDerecho`]) los porcentajes suman exactamente 100 %.
//! - Una persona no aparece dos veces con el mismo tipo.
//! - Hay **exactamente un** representante ante la copropiedad (votos, notificaciones); con un
//!   solo titular, es él.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{DominioError, PersonaId};

/// Calidad en que una persona es titular.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoTitularidad {
    Propietario,
    /// Tiene la nuda propiedad; el goce lo tiene un usufructuario.
    NudoPropietario,
    Usufructuario,
    /// Bien en fiducia: el titular es el patrimonio autónomo.
    Fideicomiso,
    /// Bien en arrendamiento financiero: el titular es la entidad.
    Leasing,
}

enum_texto!(TipoTitularidad {
    Propietario => "propietario",
    NudoPropietario => "nudo_propietario",
    Usufructuario => "usufructuario",
    Fideicomiso => "fideicomiso",
    Leasing => "leasing",
});

/// Clase de derecho sobre el bien: los porcentajes suman 100 % dentro de cada clase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ClaseDerecho {
    /// Propietario, nudo propietario, fideicomiso o leasing.
    Dominio,
    /// Usufructuario.
    Goce,
}

impl TipoTitularidad {
    /// Clase de derecho del tipo de titularidad (los vigentes suman 100 % por clase).
    pub const fn clase(self) -> ClaseDerecho {
        match self {
            Self::Usufructuario => ClaseDerecho::Goce,
            _ => ClaseDerecho::Dominio,
        }
    }
}

/// Persona natural o jurídica.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoPersona {
    Natural,
    Juridica,
}

enum_texto!(TipoPersona {
    Natural => "natural",
    Juridica => "juridica",
});

/// Porcentaje de propiedad en (0, 100] con hasta 4 decimales.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "Decimal", into = "Decimal")]
pub struct Porcentaje(Decimal);

impl Porcentaje {
    /// Cien por ciento.
    pub const CIEN: Self = Self(Decimal::ONE_HUNDRED);

    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] fuera de (0, 100] o con más de 4 decimales.
    pub fn new(valor: Decimal) -> Result<Self, DominioError> {
        if valor <= Decimal::ZERO || valor > Decimal::ONE_HUNDRED || valor.normalize().scale() > 4 {
            return Err(DominioError::DatoInvalido {
                campo: "porcentaje",
                regla: "entre 0 y 100 con máximo 4 decimales",
            });
        }
        Ok(Self(valor))
    }

    /// Porcentaje mayor que 0 y hasta 100.
    pub const fn valor(self) -> Decimal {
        self.0
    }
}

impl TryFrom<Decimal> for Porcentaje {
    type Error = DominioError;

    fn try_from(v: Decimal) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}

impl From<Porcentaje> for Decimal {
    fn from(p: Porcentaje) -> Self {
        p.0
    }
}

/// Un titular dentro del conjunto que se registra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitularNuevo {
    pub persona_id: PersonaId,
    pub tipo: TipoTitularidad,
    pub porcentaje: Porcentaje,
    pub es_representante: bool,
}

/// Conjunto de titulares válido (solo se construye cumpliendo las reglas del módulo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConjuntoTitulares(Vec<TitularNuevo>);

impl ConjuntoTitulares {
    /// Valida el conjunto. Con un solo titular se le marca representante aunque no venga así.
    ///
    /// # Errors
    ///
    /// [`DominioError::TitularesInvalidos`] si está vacío, una persona se repite con el mismo
    /// tipo, alguna clase no suma 100 % o no hay exactamente un representante.
    pub fn new(mut titulares: Vec<TitularNuevo>) -> Result<Self, DominioError> {
        if titulares.is_empty() {
            return Err(DominioError::TitularesInvalidos(
                "debe haber al menos un titular",
            ));
        }
        let mut vistos = BTreeSet::new();
        if !titulares
            .iter()
            .all(|t| vistos.insert((t.persona_id, t.tipo)))
        {
            return Err(DominioError::TitularesInvalidos(
                "una persona aparece dos veces con el mismo tipo",
            ));
        }
        let mut sumas: BTreeMap<ClaseDerecho, Decimal> = BTreeMap::new();
        for t in &titulares {
            *sumas.entry(t.tipo.clase()).or_default() += t.porcentaje.valor();
        }
        if sumas.values().any(|s| *s != Decimal::ONE_HUNDRED) {
            return Err(DominioError::TitularesInvalidos(
                "los porcentajes deben sumar 100 % (por separado dominio y usufructo)",
            ));
        }
        if let [unico] = titulares.as_mut_slice() {
            unico.es_representante = true;
        }
        if titulares.iter().filter(|t| t.es_representante).count() != 1 {
            return Err(DominioError::TitularesInvalidos(
                "con varios titulares se designa exactamente un representante ante la copropiedad",
            ));
        }
        Ok(Self(titulares))
    }

    /// Titulares del conjunto.
    pub fn titulares(&self) -> &[TitularNuevo] {
        &self.0
    }

    /// Representante del bien ante la copropiedad.
    pub fn representante(&self) -> PersonaId {
        self.0
            .iter()
            .find(|t| t.es_representante)
            .map(|t| t.persona_id)
            .expect("ConjuntoTitulares siempre tiene exactamente un representante")
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    fn t(n: u128, tipo: TipoTitularidad, p: Decimal, repr: bool) -> TitularNuevo {
        TitularNuevo {
            persona_id: PersonaId::from_uuid(uuid::Uuid::from_u128(n)),
            tipo,
            porcentaje: Porcentaje::new(p).expect("válido"),
            es_representante: repr,
        }
    }

    use TipoTitularidad::*;

    #[test]
    fn un_titular_es_representante() {
        let c = ConjuntoTitulares::new(vec![t(1, Propietario, dec!(100), false)]).expect("válido");
        assert!(c.titulares()[0].es_representante);
    }

    #[test]
    fn copropietarios_suman_cien_con_un_representante() {
        let c = ConjuntoTitulares::new(vec![
            t(1, Propietario, dec!(50), true),
            t(2, Propietario, dec!(50), false),
        ])
        .expect("válido");
        assert_eq!(
            c.representante(),
            PersonaId::from_uuid(uuid::Uuid::from_u128(1))
        );
    }

    #[test]
    fn nuda_propiedad_y_usufructo_suman_por_separado() {
        assert!(
            ConjuntoTitulares::new(vec![
                t(1, NudoPropietario, dec!(100), false),
                t(2, Usufructuario, dec!(100), true),
            ])
            .is_ok()
        );
    }

    #[test]
    fn rechaza_conjuntos_invalidos() {
        let casos = [
            vec![],
            vec![
                t(1, Propietario, dec!(60), true),
                t(2, Propietario, dec!(30), false),
            ],
            vec![
                t(1, Propietario, dec!(50), false),
                t(2, Propietario, dec!(50), false),
            ],
            vec![
                t(1, Propietario, dec!(50), true),
                t(2, Propietario, dec!(50), true),
            ],
            vec![
                t(1, Propietario, dec!(50), true),
                t(1, Propietario, dec!(50), false),
            ],
            vec![
                t(1, NudoPropietario, dec!(100), true),
                t(2, Usufructuario, dec!(50), false),
            ],
        ];
        for c in casos {
            assert!(ConjuntoTitulares::new(c.clone()).is_err(), "{c:?}");
        }
    }

    #[test]
    fn porcentaje_en_rango() {
        assert!(Porcentaje::new(dec!(33.3333)).is_ok());
        assert!(Porcentaje::new(dec!(33.33333)).is_err());
        assert!(Porcentaje::new(dec!(0)).is_err());
        assert!(Porcentaje::new(dec!(100.01)).is_err());
    }
}
