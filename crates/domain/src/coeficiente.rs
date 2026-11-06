use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Coeficiente de copropiedad de una unidad privada, expresado como **fracción** (0 < c ≤ 1).
///
/// Ejemplo: un apartamento con 0,5230 % de coeficiente se representa como `0.005230`.
/// En BD se guarda en `NUMERIC(10,8)`; nunca usar `f64` para coeficientes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "Decimal", into = "Decimal")]
pub struct Coeficiente(Decimal);

impl Coeficiente {
    /// Máximo de decimales admitidos (coincide con la escala de la columna en BD).
    pub const ESCALA: u32 = 8;

    pub fn new(valor: Decimal) -> Result<Self, DominioError> {
        if valor <= Decimal::ZERO || valor > Decimal::ONE {
            return Err(DominioError::CoeficienteInvalido(format!(
                "{valor} debe estar en el rango (0, 1]"
            )));
        }
        if valor.normalize().scale() > Self::ESCALA {
            return Err(DominioError::CoeficienteInvalido(format!(
                "{valor} tiene más de {} decimales",
                Self::ESCALA
            )));
        }
        Ok(Self(valor))
    }

    pub const fn valor(self) -> Decimal {
        self.0
    }
}

impl TryFrom<Decimal> for Coeficiente {
    type Error = DominioError;

    fn try_from(valor: Decimal) -> Result<Self, Self::Error> {
        Self::new(valor)
    }
}

impl From<Coeficiente> for Decimal {
    fn from(c: Coeficiente) -> Self {
        c.0
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::Coeficiente;

    #[test]
    fn acepta_rango_valido() {
        assert!(Coeficiente::new(dec!(0.005230)).is_ok());
        assert!(Coeficiente::new(dec!(1)).is_ok());
    }

    #[test]
    fn rechaza_fuera_de_rango_o_escala() {
        assert!(Coeficiente::new(dec!(0)).is_err());
        assert!(Coeficiente::new(dec!(-0.1)).is_err());
        assert!(Coeficiente::new(dec!(1.00000001)).is_err());
        assert!(Coeficiente::new(dec!(0.000000001)).is_err());
    }
}
