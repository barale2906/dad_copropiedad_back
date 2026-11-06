use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Regla de aprobación de una pregunta, calculada sobre coeficientes (no sobre número de votos).
///
/// Referencia: Ley 675 de 2001 (Colombia), arts. 45 y 46. **Pendiente validación jurídica**
/// de los casos particulares (reglamento de cada copropiedad, segunda convocatoria, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReglaMayoria {
    /// Más de la mitad de los coeficientes **representados** en la sesión (art. 45).
    Simple,
    /// Al menos el 70 % de los coeficientes que **integran** la copropiedad (art. 46).
    Calificada70,
}

enum_texto!(ReglaMayoria {
    Simple => "simple",
    Calificada70 => "calificada_70",
});

impl ReglaMayoria {
    /// Determina si la decisión queda aprobada.
    ///
    /// - `a_favor`: suma de coeficientes que votaron por la opción.
    /// - `representados`: suma de coeficientes presentes o representados (quórum de la sesión).
    /// - `total`: suma de coeficientes de toda la copropiedad (normalmente 1).
    pub fn aprobada(self, a_favor: Decimal, representados: Decimal, total: Decimal) -> bool {
        match self {
            Self::Simple => a_favor * Decimal::TWO > representados,
            Self::Calificada70 => a_favor >= total * Decimal::new(70, 2),
        }
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::ReglaMayoria;

    #[test]
    fn simple_exige_mas_de_la_mitad_de_representados() {
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
}
