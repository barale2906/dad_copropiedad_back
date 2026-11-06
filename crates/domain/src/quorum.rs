use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Convocatoria en la que se reúne la asamblea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Convocatoria {
    Primera,
    /// Reunión por derecho propio o de segunda convocatoria (Ley 675, art. 41): sesiona y
    /// decide con cualquier número plural de propietarios.
    Segunda,
}

/// Quórum de una reunión, calculado sobre coeficientes.
///
/// **Pendiente validación jurídica**: reglamentos particulares, número plural de asistentes en
/// segunda convocatoria y reuniones no presenciales (art. 42).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quorum {
    /// Suma de coeficientes presentes o representados.
    pub representados: Decimal,
    /// Suma de coeficientes de toda la copropiedad (normalmente 1).
    pub total: Decimal,
}

impl Quorum {
    pub fn new(representados: Decimal, total: Decimal) -> Result<Self, DominioError> {
        if total <= Decimal::ZERO || representados < Decimal::ZERO || representados > total {
            return Err(DominioError::CoeficienteInvalido(format!(
                "quórum inconsistente: representados={representados}, total={total}"
            )));
        }
        Ok(Self {
            representados,
            total,
        })
    }

    /// Fracción representada (0..=1).
    pub fn fraccion(&self) -> Decimal {
        self.representados / self.total
    }

    /// Primera convocatoria: más de la mitad de los coeficientes (Ley 675, art. 45).
    pub fn hay_quorum(&self, convocatoria: Convocatoria) -> bool {
        match convocatoria {
            Convocatoria::Primera => self.representados * Decimal::TWO > self.total,
            Convocatoria::Segunda => self.representados > Decimal::ZERO,
        }
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    #[test]
    fn primera_convocatoria_exige_mas_de_la_mitad() {
        assert!(
            Quorum::new(dec!(0.50000001), dec!(1))
                .unwrap()
                .hay_quorum(Convocatoria::Primera)
        );
        assert!(
            !Quorum::new(dec!(0.5), dec!(1))
                .unwrap()
                .hay_quorum(Convocatoria::Primera)
        );
    }

    #[test]
    fn segunda_convocatoria_con_cualquier_numero() {
        assert!(
            Quorum::new(dec!(0.1), dec!(1))
                .unwrap()
                .hay_quorum(Convocatoria::Segunda)
        );
    }

    #[test]
    fn rechaza_datos_inconsistentes() {
        assert!(Quorum::new(dec!(1.1), dec!(1)).is_err());
        assert!(Quorum::new(dec!(0.1), dec!(0)).is_err());
    }
}
