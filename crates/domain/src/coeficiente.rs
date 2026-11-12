//! Coeficiente de copropiedad (Ley 675, art. 25): fracción en `Decimal`, nunca en flotante.

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

    /// Coeficiente en el rango (0, 1] con a lo sumo [`Self::ESCALA`] decimales.
    ///
    /// # Errors
    ///
    /// [`DominioError::CoeficienteInvalido`] si está fuera del rango o tiene más decimales.
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

    /// Fracción del coeficiente (0,5 % = 0,005).
    pub const fn valor(self) -> Decimal {
        self.0
    }

    /// Lee un coeficiente escrito por una persona: **porcentaje** si termina en `%`
    /// (`"0,5230 %"` → `0.005230`) o **fracción** si no (`"0.00523"`). Acepta coma o punto
    /// decimal y espacios; no acepta separadores de miles.
    ///
    /// ```
    /// use domain::Coeficiente;
    /// use rust_decimal::Decimal;
    ///
    /// let c = Coeficiente::desde_texto("0,5230 %").expect("válido");
    /// assert_eq!(c.valor(), Decimal::new(523, 5));
    /// assert_eq!(Coeficiente::desde_texto("0.00523").expect("válido").valor(), Decimal::new(523, 5));
    /// ```
    ///
    /// # Errors
    ///
    /// [`DominioError::CoeficienteInvalido`] si no es un número, queda fuera de (0, 1] o tiene
    /// más de [`Self::ESCALA`] decimales como fracción.
    pub fn desde_texto(texto: &str) -> Result<Self, DominioError> {
        let limpio: String = texto.chars().filter(|c| !c.is_whitespace()).collect();
        let (numero, es_porcentaje) = match limpio.strip_suffix('%') {
            Some(n) => (n, true),
            None => (limpio.as_str(), false),
        };
        let numero = numero.replace(',', ".");
        if numero.is_empty() || numero.matches('.').count() > 1 {
            return Err(DominioError::CoeficienteInvalido(
                "escriba un número como 0,5230 % o 0.005230".into(),
            ));
        }
        let valor: Decimal = numero.parse().map_err(|_| {
            DominioError::CoeficienteInvalido("escriba un número como 0,5230 % o 0.005230".into())
        })?;
        let fraccion = if es_porcentaje {
            valor / Decimal::ONE_HUNDRED
        } else {
            valor
        };
        Self::new(fraccion.normalize())
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

#[cfg(test)]
mod pruebas_texto {
    use rust_decimal::dec;

    use super::Coeficiente;

    #[test]
    fn lee_porcentaje_y_fraccion_con_coma_o_punto() {
        for (texto, esperado) in [
            ("0,5230 %", dec!(0.00523)),
            ("0.523%", dec!(0.00523)),
            ("0,00523", dec!(0.00523)),
            ("100 %", dec!(1)),
            (" 1 ", dec!(1)),
        ] {
            assert_eq!(
                Coeficiente::desde_texto(texto).expect(texto).valor(),
                esperado,
                "{texto}"
            );
        }
    }

    #[test]
    fn rechaza_textos_invalidos() {
        for texto in [
            "",
            "%",
            "abc",
            "1.000,5",
            "0,5,2",
            "0 %",
            "101 %",
            "0.0000000001",
        ] {
            assert!(Coeficiente::desde_texto(texto).is_err(), "{texto:?}");
        }
    }
}
