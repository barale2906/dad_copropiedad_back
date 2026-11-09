//! Mayorías y base de cómputo de las decisiones de la asamblea (Ley 675 de 2001, arts. 45 y 46).
//!
//! La ley decide por **coeficientes**; algunas copropiedades cuentan también (o deciden, si su
//! reglamento lo permite) por **unidades** (un bien privado = un voto). Por eso cada pregunta
//! declara su [`BaseComputo`] y el escrutinio calcula siempre ambas; solo una decide.
//! **Pendiente validación jurídica**: cuándo es válido decidir por unidades (riesgo de
//! impugnación, art. 49), reglamentos particulares y segunda convocatoria.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Con qué se pesan los votos y el quórum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseComputo {
    /// Suma de coeficientes de copropiedad (lo que exige la ley; por defecto).
    #[default]
    Coeficiente,
    /// Número de bienes privados. Se advierte en la interfaz y en el acta.
    Unidades,
}

enum_texto!(BaseComputo {
    Coeficiente => "coeficiente",
    Unidades => "unidades",
});

/// Sobre qué total se mide un umbral personalizado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseMayoria {
    /// Lo presente y habilitado para votar la pregunta.
    Presentes,
    /// Toda la copropiedad (el padrón de la reunión).
    Total,
}

enum_texto!(BaseMayoria {
    Presentes => "presentes",
    Total => "total",
});

/// Regla de aprobación de una pregunta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "regla", rename_all = "snake_case")]
pub enum ReglaMayoria {
    /// Más de la mitad de lo **presente** en la sesión (art. 45).
    Simple,
    /// Al menos el 70 % de lo que **integra** la copropiedad (art. 46).
    Calificada70,
    /// Umbral que fija el reglamento (p. ej. 60 % de los presentes): se alcanza con **al menos**
    /// `porcentaje` % de la base.
    Personalizada {
        porcentaje: Decimal,
        sobre: BaseMayoria,
    },
}

impl ReglaMayoria {
    /// Código persistido en `preguntas.regla_mayoria`.
    pub const fn codigo(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Calificada70 => "calificada_70",
            Self::Personalizada { .. } => "personalizada",
        }
    }

    /// Reconstruye la regla desde sus columnas (`regla_mayoria`, `umbral_porcentaje`,
    /// `umbral_sobre`).
    ///
    /// # Errors
    ///
    /// [`DominioError::ValorDesconocido`] o [`DominioError::DatoInvalido`] si las columnas no
    /// forman una regla válida.
    pub fn desde_columnas(
        codigo: &str,
        porcentaje: Option<Decimal>,
        sobre: Option<&str>,
    ) -> Result<Self, DominioError> {
        match (codigo, porcentaje, sobre) {
            ("simple", None, None) => Ok(Self::Simple),
            ("calificada_70", None, None) => Ok(Self::Calificada70),
            ("personalizada", Some(p), Some(s)) => {
                Self::personalizada(p, BaseMayoria::try_from(s)?)
            }
            ("simple" | "calificada_70" | "personalizada", ..) => Err(DominioError::DatoInvalido {
                campo: "regla_mayoria",
                regla: "el umbral solo aplica (y es obligatorio) en la regla personalizada",
            }),
            (otro, ..) => Err(DominioError::ValorDesconocido {
                tipo: "ReglaMayoria",
                valor: otro.to_owned(),
            }),
        }
    }

    /// Regla personalizada validada (porcentaje en (0, 100]).
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si el porcentaje está fuera de rango.
    pub fn personalizada(porcentaje: Decimal, sobre: BaseMayoria) -> Result<Self, DominioError> {
        if porcentaje <= Decimal::ZERO || porcentaje > Decimal::ONE_HUNDRED {
            return Err(DominioError::DatoInvalido {
                campo: "umbral_porcentaje",
                regla: "debe ser mayor que 0 y como máximo 100",
            });
        }
        Ok(Self::Personalizada { porcentaje, sobre })
    }

    /// `(umbral_porcentaje, umbral_sobre)` para persistir.
    pub const fn umbral(self) -> (Option<Decimal>, Option<BaseMayoria>) {
        match self {
            Self::Personalizada { porcentaje, sobre } => (Some(porcentaje), Some(sobre)),
            _ => (None, None),
        }
    }

    /// Determina si la decisión queda aprobada. Los tres valores van en la misma base
    /// (coeficientes o unidades).
    ///
    /// - `a_favor`: lo que votó por la opción.
    /// - `presentes`: lo habilitado para votar la pregunta (quórum de la decisión).
    /// - `total`: toda la copropiedad (padrón).
    pub fn aprobada(self, a_favor: Decimal, presentes: Decimal, total: Decimal) -> bool {
        match self {
            Self::Simple => a_favor * Decimal::TWO > presentes,
            Self::Calificada70 => a_favor >= total * Decimal::new(70, 2),
            Self::Personalizada { porcentaje, sobre } => {
                let base = match sobre {
                    BaseMayoria::Presentes => presentes,
                    BaseMayoria::Total => total,
                };
                a_favor > Decimal::ZERO && a_favor * Decimal::ONE_HUNDRED >= base * porcentaje
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    #[test]
    fn simple_exige_mas_de_la_mitad_de_presentes() {
        let r = ReglaMayoria::Simple;
        assert!(r.aprobada(dec!(0.30), dec!(0.55), dec!(1)));
        assert!(!r.aprobada(dec!(0.275), dec!(0.55), dec!(1))); // exactamente la mitad: no aprueba
    }

    #[test]
    fn calificada_exige_70_por_ciento_del_total() {
        let r = ReglaMayoria::Calificada70;
        assert!(r.aprobada(dec!(0.70), dec!(0.80), dec!(1)));
        assert!(!r.aprobada(dec!(0.699999), dec!(1), dec!(1)));
    }

    #[test]
    fn personalizada_sobre_presentes_o_total() {
        let presentes = ReglaMayoria::personalizada(dec!(60), BaseMayoria::Presentes).unwrap();
        assert!(presentes.aprobada(dec!(0.30), dec!(0.50), dec!(1)));
        assert!(!presentes.aprobada(dec!(0.29), dec!(0.50), dec!(1)));
        let total = ReglaMayoria::personalizada(dec!(60), BaseMayoria::Total).unwrap();
        assert!(!total.aprobada(dec!(0.30), dec!(0.50), dec!(1)));
        // Por unidades: 61 de 100 bienes.
        assert!(total.aprobada(dec!(61), dec!(70), dec!(100)));
        assert!(!total.aprobada(Decimal::ZERO, Decimal::ZERO, dec!(100)));
    }

    #[test]
    fn personalizada_valida_el_porcentaje() {
        assert!(ReglaMayoria::personalizada(dec!(0), BaseMayoria::Total).is_err());
        assert!(ReglaMayoria::personalizada(dec!(100.01), BaseMayoria::Total).is_err());
        assert!(ReglaMayoria::personalizada(dec!(100), BaseMayoria::Total).is_ok());
    }

    #[test]
    fn columnas_ida_y_vuelta() {
        for r in [
            ReglaMayoria::Simple,
            ReglaMayoria::Calificada70,
            ReglaMayoria::personalizada(dec!(66.67), BaseMayoria::Presentes).unwrap(),
        ] {
            let (p, s) = r.umbral();
            assert_eq!(
                ReglaMayoria::desde_columnas(r.codigo(), p, s.map(BaseMayoria::as_str)).unwrap(),
                r
            );
        }
        assert!(ReglaMayoria::desde_columnas("simple", Some(dec!(50)), Some("total")).is_err());
        assert!(ReglaMayoria::desde_columnas("personalizada", None, None).is_err());
        assert!(ReglaMayoria::desde_columnas("unanimidad", None, None).is_err());
    }
}
