//! Cuadros de coeficientes de copropiedad versionados (Ley 675, arts. 25–28).
//!
//! Los coeficientes solo cambian por reforma del reglamento (escritura pública): no se editan
//! uno por uno, se prepara un cuadro **borrador** completo y se **activa**. Activar exige que la
//! suma sea **exactamente 1** (sin tolerancia, con `Decimal`), que estén todos los bienes
//! privados activos y que ninguno tenga coeficiente ≤ 0 (lo garantiza [`Coeficiente`]).
//!
//! ```text
//! Borrador ──activar──▶ Vigente ──(se activa otro)──▶ Historico
//! ```

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{BienPrivadoId, Coeficiente, DominioError};

/// Estado de un cuadro de coeficientes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoCuadro {
    /// En preparación: se cargan y corrigen coeficientes.
    Borrador,
    /// El que rige: lo usan las votaciones y las expensas. Solo uno por copropiedad.
    Vigente,
    /// Rigió antes; se conserva como historia legal.
    Historico,
}

enum_texto!(EstadoCuadro {
    Borrador => "borrador",
    Vigente => "vigente",
    Historico => "historico",
});

impl EstadoCuadro {
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] fuera de borrador → vigente → histórico.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        match (self, hacia) {
            (Self::Borrador, Self::Vigente) | (Self::Vigente, Self::Historico) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "cuadro de coeficientes",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }

    /// Solo un borrador admite cambios en sus coeficientes.
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no es borrador.
    pub fn exigir_borrador(self) -> Result<(), DominioError> {
        if self == Self::Borrador {
            Ok(())
        } else {
            Err(DominioError::TransicionInvalida {
                entidad: "cuadro de coeficientes",
                desde: self.as_str(),
                hacia: "editar",
            })
        }
    }
}

/// Para qué sirve el cuadro. El de votaciones y expensas es siempre [`TipoCuadro::Copropiedad`];
/// los módulos de contribución (art. 29, conjuntos mixtos) quedan previstos para contabilidad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoCuadro {
    Copropiedad,
    ModuloContribucion,
}

enum_texto!(TipoCuadro {
    Copropiedad => "copropiedad",
    ModuloContribucion => "modulo_contribucion",
});

/// Resultado de revisar un cuadro contra los bienes privados activos. Sirve de herramienta de
/// **ajuste de redondeo**: muestra cuánto falta o sobra; la corrección es siempre humana.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionCuadro {
    pub suma: Decimal,
    /// `1 - suma`: positiva si falta, negativa si sobra.
    pub diferencia: Decimal,
    /// Bienes privados activos sin coeficiente en el cuadro.
    pub faltantes: Vec<BienPrivadoId>,
    /// Bienes del cuadro que no están activos (inactivos o inexistentes).
    pub sobrantes: Vec<BienPrivadoId>,
    /// Bien con el mayor coeficiente: donde suele cargarse el ajuste de redondeo (sugerencia).
    pub sugerido_para_ajuste: Option<BienPrivadoId>,
}

impl RevisionCuadro {
    /// Revisa los coeficientes del cuadro contra los bienes privados activos.
    pub fn revisar(
        coeficientes: &BTreeMap<BienPrivadoId, Coeficiente>,
        activos: &BTreeSet<BienPrivadoId>,
    ) -> Self {
        let suma: Decimal = coeficientes.values().map(|c| c.valor()).sum();
        let faltantes = activos
            .iter()
            .filter(|b| !coeficientes.contains_key(b))
            .copied()
            .collect();
        let sobrantes = coeficientes
            .keys()
            .filter(|b| !activos.contains(b))
            .copied()
            .collect();
        let sugerido_para_ajuste = coeficientes
            .iter()
            .max_by_key(|(id, c)| (**c, std::cmp::Reverse(**id)))
            .map(|(id, _)| *id);
        Self {
            suma,
            diferencia: Decimal::ONE - suma,
            faltantes,
            sobrantes,
            sugerido_para_ajuste,
        }
    }

    /// ¿Se puede activar?
    pub fn es_valido(&self) -> bool {
        self.exigir_valido().is_ok()
    }

    /// # Errors
    ///
    /// [`DominioError::CuadroIncompleto`] si faltan bienes activos o sobran inactivos (cuentan
    /// como faltantes del lado contrario), o [`DominioError::CoeficientesNoSumanUno`] si la suma
    /// no es exactamente 1.
    pub fn exigir_valido(&self) -> Result<(), DominioError> {
        if !self.faltantes.is_empty() || !self.sobrantes.is_empty() {
            return Err(DominioError::CuadroIncompleto {
                faltantes: self.faltantes.len() + self.sobrantes.len(),
            });
        }
        if self.suma != Decimal::ONE {
            return Err(DominioError::CoeficientesNoSumanUno {
                suma: self.suma,
                diferencia: self.diferencia,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;
    use uuid::Uuid;

    use super::*;

    fn bien(n: u128) -> BienPrivadoId {
        BienPrivadoId::from_uuid(Uuid::from_u128(n))
    }

    fn cuadro(valores: &[(u128, Decimal)]) -> BTreeMap<BienPrivadoId, Coeficiente> {
        valores
            .iter()
            .map(|(n, v)| (bien(*n), Coeficiente::new(*v).expect("válido")))
            .collect()
    }

    fn activos(n: &[u128]) -> BTreeSet<BienPrivadoId> {
        n.iter().map(|x| bien(*x)).collect()
    }

    #[test]
    fn suma_exacta_es_valida() {
        let r = RevisionCuadro::revisar(
            &cuadro(&[
                (1, dec!(0.33333333)),
                (2, dec!(0.33333333)),
                (3, dec!(0.33333334)),
            ]),
            &activos(&[1, 2, 3]),
        );
        assert_eq!(r.suma, Decimal::ONE);
        assert!(r.es_valido());
        assert_eq!(r.sugerido_para_ajuste, Some(bien(3)));
    }

    #[test]
    fn faltante_de_redondeo_se_rechaza_sin_tolerancia() {
        let r = RevisionCuadro::revisar(
            &cuadro(&[
                (1, dec!(0.33333333)),
                (2, dec!(0.33333333)),
                (3, dec!(0.33333333)),
            ]),
            &activos(&[1, 2, 3]),
        );
        assert_eq!(r.diferencia, dec!(0.00000001));
        assert_eq!(
            r.exigir_valido(),
            Err(DominioError::CoeficientesNoSumanUno {
                suma: dec!(0.99999999),
                diferencia: dec!(0.00000001)
            })
        );
    }

    #[test]
    fn sobrante_se_rechaza() {
        let r = RevisionCuadro::revisar(
            &cuadro(&[(1, dec!(0.5)), (2, dec!(0.50000001))]),
            &activos(&[1, 2]),
        );
        assert_eq!(r.diferencia, dec!(-0.00000001));
        assert!(matches!(
            r.exigir_valido(),
            Err(DominioError::CoeficientesNoSumanUno { .. })
        ));
    }

    #[test]
    fn faltan_o_sobran_bienes() {
        let r = RevisionCuadro::revisar(&cuadro(&[(1, dec!(1))]), &activos(&[1, 2]));
        assert_eq!(r.faltantes, vec![bien(2)]);
        assert_eq!(
            r.exigir_valido(),
            Err(DominioError::CuadroIncompleto { faltantes: 1 })
        );

        let r = RevisionCuadro::revisar(&cuadro(&[(1, dec!(0.5)), (9, dec!(0.5))]), &activos(&[1]));
        assert_eq!(r.sobrantes, vec![bien(9)]);
        assert!(!r.es_valido());
    }

    #[test]
    fn maquina_de_estados() {
        use EstadoCuadro::*;
        assert_eq!(Borrador.transicionar(Vigente), Ok(Vigente));
        assert_eq!(Vigente.transicionar(Historico), Ok(Historico));
        for (d, h) in [
            (Borrador, Historico),
            (Historico, Vigente),
            (Vigente, Borrador),
        ] {
            assert!(d.transicionar(h).is_err(), "{d} -> {h}");
        }
        assert!(Borrador.exigir_borrador().is_ok());
        assert!(Vigente.exigir_borrador().is_err());
    }
}
