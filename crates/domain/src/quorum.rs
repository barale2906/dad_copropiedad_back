//! Quórum de la asamblea (Ley 675 de 2001, arts. 39, 41 y 45).
//!
//! El quórum es un hecho propio (registro en mesa, ingreso virtual), no una pregunta: suma los
//! bienes privados de los asistentes **presentes**, por coeficiente y por unidades.
//! **Pendiente validación jurídica**: reglamentos particulares y reuniones no presenciales
//! (art. 42).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{BaseComputo, DominioError};

/// Convocatoria en la que se reúne la asamblea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Convocatoria {
    /// Exige más de la mitad de los coeficientes (art. 45).
    Primera,
    /// Reunión de segunda convocatoria (art. 41): sesiona y decide con cualquier número plural
    /// de propietarios.
    Segunda,
    /// Reunión por derecho propio (art. 39, parágrafo): mismo quórum que la segunda.
    DerechoPropio,
}

enum_texto!(Convocatoria {
    Primera => "primera",
    Segunda => "segunda",
    DerechoPropio => "derecho_propio",
});

/// Proporción representada de una base (coeficientes o unidades).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quorum {
    /// Lo presente o representado.
    pub representados: Decimal,
    /// Toda la copropiedad (el padrón de la reunión).
    pub total: Decimal,
}

impl Quorum {
    /// # Errors
    ///
    /// [`DominioError::CoeficienteInvalido`] si el total no es positivo o lo representado está
    /// fuera de `[0, total]`.
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

    /// ¿Más de la mitad?
    pub fn mayoria_absoluta(&self) -> bool {
        self.representados * Decimal::TWO > self.total
    }
}

/// Quórum en ambas bases, como se muestra en vivo y queda en las fotos del acta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuorumReunion {
    pub coeficientes: Quorum,
    pub unidades: Quorum,
    /// Personas presentes (propietarios y apoderados).
    pub asistentes: u32,
}

impl QuorumReunion {
    /// # Errors
    ///
    /// [`DominioError::CoeficienteInvalido`] si los totales son inconsistentes.
    pub fn new(
        coeficiente_presente: Decimal,
        coeficiente_total: Decimal,
        unidades_presentes: u32,
        unidades_total: u32,
        asistentes: u32,
    ) -> Result<Self, DominioError> {
        Ok(Self {
            coeficientes: Quorum::new(coeficiente_presente, coeficiente_total)?,
            unidades: Quorum::new(
                Decimal::from(unidades_presentes),
                Decimal::from(unidades_total),
            )?,
            asistentes,
        })
    }

    /// Quórum de la base elegida.
    pub fn en(&self, base: BaseComputo) -> Quorum {
        match base {
            BaseComputo::Coeficiente => self.coeficientes,
            BaseComputo::Unidades => self.unidades,
        }
    }

    /// ¿Hay quórum para deliberar y decidir?
    ///
    /// - Primera convocatoria: más de la mitad de la base (art. 45).
    /// - Segunda o derecho propio: «cualquier número plural de propietarios» (arts. 39 y 41):
    ///   al menos dos bienes privados representados.
    pub fn hay_quorum(&self, convocatoria: Convocatoria, base: BaseComputo) -> bool {
        match convocatoria {
            Convocatoria::Primera => self.en(base).mayoria_absoluta(),
            Convocatoria::Segunda | Convocatoria::DerechoPropio => {
                self.unidades.representados >= Decimal::TWO
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    fn quorum(coef: Decimal, unidades: u32) -> QuorumReunion {
        QuorumReunion::new(coef, dec!(1), unidades, 100, unidades).unwrap()
    }

    #[test]
    fn primera_convocatoria_exige_mas_de_la_mitad() {
        let c = BaseComputo::Coeficiente;
        assert!(quorum(dec!(0.50000001), 10).hay_quorum(Convocatoria::Primera, c));
        assert!(!quorum(dec!(0.5), 90).hay_quorum(Convocatoria::Primera, c));
        // Por unidades decide el número de bienes, no el coeficiente.
        assert!(quorum(dec!(0.2), 51).hay_quorum(Convocatoria::Primera, BaseComputo::Unidades));
        assert!(!quorum(dec!(0.9), 50).hay_quorum(Convocatoria::Primera, BaseComputo::Unidades));
    }

    #[test]
    fn segunda_y_derecho_propio_con_numero_plural() {
        for conv in [Convocatoria::Segunda, Convocatoria::DerechoPropio] {
            assert!(quorum(dec!(0.02), 2).hay_quorum(conv, BaseComputo::Coeficiente));
            assert!(!quorum(dec!(0.4), 1).hay_quorum(conv, BaseComputo::Coeficiente));
        }
    }

    #[test]
    fn rechaza_datos_inconsistentes() {
        assert!(Quorum::new(dec!(1.1), dec!(1)).is_err());
        assert!(Quorum::new(dec!(0.1), dec!(0)).is_err());
        assert!(QuorumReunion::new(dec!(0.1), dec!(1), 5, 0, 1).is_err());
    }

    #[test]
    fn fraccion() {
        assert_eq!(
            Quorum::new(dec!(0.25), dec!(1)).unwrap().fraccion(),
            dec!(0.25)
        );
    }
}
