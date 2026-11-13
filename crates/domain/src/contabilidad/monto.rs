//! Dinero en la moneda funcional.

use std::{iter::Sum, ops::Add};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Valor monetario en la moneda funcional (COP): `Decimal` con dos decimales como máximo y
/// nunca negativo. Las caras del asiento (débito o crédito) dan el signo; un monto no lo lleva.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize)]
#[serde(transparent)]
pub struct Monto(Decimal);

impl Monto {
    /// Cero.
    pub const CERO: Monto = Monto(Decimal::ZERO);

    /// Valida el monto.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si es negativo o tiene más de dos decimales.
    pub fn new(valor: Decimal) -> Result<Self, DominioError> {
        if valor.is_sign_negative() && !valor.is_zero() {
            return Err(DominioError::DatoInvalido {
                campo: "monto",
                regla: "no puede ser negativo",
            });
        }
        if valor.normalize().scale() > 2 {
            return Err(DominioError::DatoInvalido {
                campo: "monto",
                regla: "máximo dos decimales",
            });
        }
        Ok(Self(valor.round_dp(2)))
    }

    /// El valor como `Decimal`.
    pub fn valor(self) -> Decimal {
        self.0
    }

    /// ¿Es cero?
    pub fn es_cero(self) -> bool {
        self.0.is_zero()
    }
}

impl Add for Monto {
    type Output = Monto;

    fn add(self, otro: Monto) -> Monto {
        Monto(self.0 + otro.0)
    }
}

impl Sum for Monto {
    fn sum<I: Iterator<Item = Monto>>(iter: I) -> Monto {
        iter.fold(Monto::CERO, Add::add)
    }
}

impl<'de> Deserialize<'de> for Monto {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Monto::new(<Decimal as Deserialize>::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<Decimal> for Monto {
    type Error = DominioError;

    fn try_from(valor: Decimal) -> Result<Self, DominioError> {
        Monto::new(valor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(texto: &str) -> Decimal {
        texto.parse().expect("decimal de prueba")
    }

    #[test]
    fn acepta_ceros_y_hasta_dos_decimales() {
        assert_eq!(Monto::new(dec("0")).expect("cero"), Monto::CERO);
        assert_eq!(
            Monto::new(dec("1500.50")).expect("ok").valor(),
            dec("1500.50")
        );
        assert_eq!(
            Monto::new(dec("10.500")).expect("ceros sobrantes").valor(),
            dec("10.50")
        );
    }

    #[test]
    fn rechaza_negativos_y_tres_decimales() {
        assert!(Monto::new(dec("-1")).is_err());
        assert!(Monto::new(dec("0.001")).is_err());
    }

    #[test]
    fn suma_exacta_sin_flotantes() {
        let total: Monto = [dec("0.10"), dec("0.20"), dec("0.70")]
            .into_iter()
            .map(|d| Monto::new(d).expect("ok"))
            .sum();
        assert_eq!(total.valor(), dec("1.00"));
    }

    #[test]
    fn deserializa_validando() {
        assert!(serde_json::from_str::<Monto>("\"12.34\"").is_ok());
        assert!(serde_json::from_str::<Monto>("\"-1\"").is_err());
    }
}
