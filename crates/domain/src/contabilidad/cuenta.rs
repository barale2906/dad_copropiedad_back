//! Plan de cuentas: códigos del PUC (Decreto 2650 de 1993) y auxiliares de la entidad.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Naturaleza de una cuenta: el lado en que aumenta su saldo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Naturaleza {
    Debito,
    Credito,
}

enum_texto!(Naturaleza {
    Debito => "debito",
    Credito => "credito",
});

/// Nivel de una cuenta según el largo de su código (1, 2, 4, 6 dígitos; los auxiliares de la
/// entidad agregan dos dígitos por nivel: 8, 10 o 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NivelCuenta {
    Clase,
    Grupo,
    Cuenta,
    Subcuenta,
    Auxiliar,
}

enum_texto!(NivelCuenta {
    Clase => "clase",
    Grupo => "grupo",
    Cuenta => "cuenta",
    Subcuenta => "subcuenta",
    Auxiliar => "auxiliar",
});

/// Código de cuenta válido: solo dígitos, empieza por 1–9 y tiene 1, 2, 4, 6, 8, 10 o 12
/// dígitos.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct CodigoCuenta(String);

impl CodigoCuenta {
    /// Valida el código.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no cumple el formato.
    pub fn new(codigo: &str) -> Result<Self, DominioError> {
        let digitos = codigo.chars().all(|c| c.is_ascii_digit());
        let largo_valido = matches!(codigo.len(), 1 | 2 | 4 | 6 | 8 | 10 | 12);
        if digitos && largo_valido && !codigo.starts_with('0') {
            Ok(Self(codigo.to_owned()))
        } else {
            Err(DominioError::DatoInvalido {
                campo: "codigo",
                regla: "dígitos que empiezan por 1–9, con 1, 2, 4, 6, 8, 10 o 12 dígitos",
            })
        }
    }

    /// El código como texto.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Nivel según el largo.
    pub fn nivel(&self) -> NivelCuenta {
        match self.0.len() {
            1 => NivelCuenta::Clase,
            2 => NivelCuenta::Grupo,
            4 => NivelCuenta::Cuenta,
            6 => NivelCuenta::Subcuenta,
            _ => NivelCuenta::Auxiliar,
        }
    }

    /// Cuenta padre (la del nivel inmediatamente superior); `None` para una clase.
    pub fn padre(&self) -> Option<CodigoCuenta> {
        let largo = match self.0.len() {
            1 => return None,
            2 => 1,
            n => n - 2,
        };
        Some(Self(self.0[..largo].to_owned()))
    }

    /// Primer dígito: la clase del PUC (1 activo … 9 cuentas de orden acreedoras).
    pub fn clase(&self) -> u8 {
        self.0.as_bytes()[0] - b'0'
    }

    /// ¿Es una cuenta de resultado (ingresos, gastos o costos, clases 4 a 7)? Se cancelan en el
    /// cierre anual.
    pub fn es_de_resultado(&self) -> bool {
        (4..=7).contains(&self.clase())
    }

    /// ¿Puede recibir movimientos? Las clases y los grupos nunca; desde el nivel cuenta lo
    /// decide la entidad.
    pub fn admite_movimiento(&self) -> bool {
        self.nivel() >= NivelCuenta::Cuenta
    }

    /// ¿El código empieza por `prefijo`?
    pub fn empieza_por(&self, prefijo: &str) -> bool {
        self.0.starts_with(prefijo)
    }
}

impl fmt::Display for CodigoCuenta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CodigoCuenta {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let texto = String::deserialize(d)?;
        CodigoCuenta::new(&texto).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(codigo: &str) -> CodigoCuenta {
        CodigoCuenta::new(codigo).expect("código válido")
    }

    #[test]
    fn niveles_y_padres_del_puc_y_auxiliares() {
        assert_eq!(c("1").nivel(), NivelCuenta::Clase);
        assert_eq!(c("11").nivel(), NivelCuenta::Grupo);
        assert_eq!(c("1105").nivel(), NivelCuenta::Cuenta);
        assert_eq!(c("110505").nivel(), NivelCuenta::Subcuenta);
        assert_eq!(c("11050501").nivel(), NivelCuenta::Auxiliar);
        assert_eq!(c("1").padre(), None);
        assert_eq!(c("11").padre(), Some(c("1")));
        assert_eq!(c("1105").padre(), Some(c("11")));
        assert_eq!(c("11050501").padre(), Some(c("110505")));
    }

    #[test]
    fn codigos_invalidos() {
        for malo in ["", "0", "123", "11A5", "0105", "1234567890123"] {
            assert!(CodigoCuenta::new(malo).is_err(), "{malo}");
        }
    }

    #[test]
    fn clase_resultado_y_movimiento() {
        assert!(c("4135").es_de_resultado());
        assert!(c("5105").es_de_resultado());
        assert!(!c("1105").es_de_resultado());
        assert!(!c("3605").es_de_resultado());
        assert!(!c("11").admite_movimiento());
        assert!(c("1105").admite_movimiento());
    }
}
