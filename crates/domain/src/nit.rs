//! NIT (Número de Identificación Tributaria) colombiano con dígito de verificación.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::DominioError;

/// NIT con su dígito de verificación (DV) calculado por el algoritmo de la DIAN (módulo 11 con
/// pesos primos). Se guarda como `"900123456-8"`: número sin puntos, guion y DV.
///
/// Para una persona natural con RUT el número es su cédula: **dato personal**.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Nit {
    numero: String,
    dv: u8,
}

impl Nit {
    const PESOS: [u32; 15] = [3, 7, 13, 17, 19, 23, 29, 37, 41, 43, 47, 53, 59, 67, 71];

    /// Acepta `900.123.456-8`, `900123456-8` o `9001234568`… siempre que el DV cuadre. Si no
    /// trae DV (`900123456`), lo calcula.
    ///
    /// # Errors
    ///
    /// [`DominioError::NitInvalido`] si no es numérico, tiene más de 15 dígitos o el DV no
    /// coincide.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let limpio: String = valor.chars().filter(|c| !matches!(c, '.' | ' ')).collect();
        let (numero, dv) = match limpio.split_once('-') {
            Some((n, d)) => (n.to_owned(), Some(d)),
            None => (limpio, None),
        };
        if numero.is_empty() || numero.len() > 15 || !numero.chars().all(|c| c.is_ascii_digit()) {
            return Err(DominioError::NitInvalido);
        }
        let numero = numero.trim_start_matches('0').to_owned();
        if numero.is_empty() {
            return Err(DominioError::NitInvalido);
        }
        let calculado = Self::digito_verificacion(&numero);
        match dv {
            None => Ok(Self {
                numero,
                dv: calculado,
            }),
            Some(d) if d.parse::<u8>().ok() == Some(calculado) && d.len() == 1 => Ok(Self {
                numero,
                dv: calculado,
            }),
            Some(_) => Err(DominioError::NitInvalido),
        }
    }

    fn digito_verificacion(numero: &str) -> u8 {
        let suma: u32 = numero
            .bytes()
            .rev()
            .zip(Self::PESOS)
            .map(|(b, peso)| u32::from(b - b'0') * peso)
            .sum();
        let residuo = suma % 11;
        let dv = if residuo > 1 { 11 - residuo } else { residuo };
        u8::try_from(dv).expect("el DV es menor que 11")
    }

    pub fn numero(&self) -> &str {
        &self.numero
    }

    pub const fn dv(&self) -> u8 {
        self.dv
    }
}

impl fmt::Display for Nit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.numero, self.dv)
    }
}

// Puede ser una cédula: no se muestra en `Debug`.
impl fmt::Debug for Nit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Nit(<oculto>)")
    }
}

impl TryFrom<String> for Nit {
    type Error = DominioError;

    fn try_from(valor: String) -> Result<Self, Self::Error> {
        Self::new(&valor)
    }
}

impl From<Nit> for String {
    fn from(n: Nit) -> Self {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calcula_y_valida_el_digito_de_verificacion() {
        // NIT de la DIAN: 800.197.268-4.
        let nit = Nit::new("800.197.268-4").expect("válido");
        assert_eq!(nit.to_string(), "800197268-4");
        assert_eq!(Nit::new("800197268").expect("sin DV").dv(), 4);
        assert_eq!(Nit::new("860001022-7").expect("válido").dv(), 7);
        assert_eq!(Nit::new("800197268-5"), Err(DominioError::NitInvalido));
    }

    #[test]
    fn rechaza_formatos_invalidos() {
        for invalido in ["", "abc", "000", "1234567890123456", "800197268-44"] {
            assert_eq!(
                Nit::new(invalido),
                Err(DominioError::NitInvalido),
                "{invalido:?}"
            );
        }
        assert!(!format!("{:?}", Nit::new("800197268").expect("válido")).contains("800"));
    }
}
