//! Proyección de resultados en vivo (se mantiene en memoria y se emite cada ~500 ms).

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::Serialize;

use crate::{Coeficiente, DominioError, OpcionId, PreguntaId, ReglaMayoria};

/// Acumulado de coeficientes por opción de una pregunta.
///
/// La verdad está en PostgreSQL; esta proyección se reconstruye desde la BD al arrancar y luego
/// se actualiza con cada voto confirmado. El WebSocket emite instantáneas, no un mensaje por voto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProyeccionResultados {
    pub pregunta_id: PreguntaId,
    pub por_opcion: BTreeMap<OpcionId, Decimal>,
    /// Número de votos aplicados (se incrementa con cada voto; sirve para detectar cambios).
    pub version: u64,
}

impl ProyeccionResultados {
    pub fn new(pregunta_id: PreguntaId, opciones: &[OpcionId]) -> Self {
        Self {
            pregunta_id,
            por_opcion: opciones.iter().map(|o| (*o, Decimal::ZERO)).collect(),
            version: 0,
        }
    }

    pub fn aplicar(
        &mut self,
        opcion_id: OpcionId,
        coeficiente: Coeficiente,
    ) -> Result<(), DominioError> {
        let acumulado = self
            .por_opcion
            .get_mut(&opcion_id)
            .ok_or(DominioError::OpcionInvalida(opcion_id))?;
        *acumulado += coeficiente.valor();
        self.version += 1;
        Ok(())
    }

    /// Suma de coeficientes que ya votaron.
    pub fn total_votado(&self) -> Decimal {
        self.por_opcion.values().copied().sum()
    }

    pub fn aprobada(
        &self,
        opcion_id: OpcionId,
        regla: ReglaMayoria,
        representados: Decimal,
        total: Decimal,
    ) -> bool {
        self.por_opcion
            .get(&opcion_id)
            .is_some_and(|a_favor| regla.aprobada(*a_favor, representados, total))
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    #[test]
    fn suma_coeficientes_no_votos() {
        let (si, no) = (OpcionId::nuevo(), OpcionId::nuevo());
        let mut p = ProyeccionResultados::new(PreguntaId::nuevo(), &[si, no]);
        p.aplicar(si, Coeficiente::new(dec!(0.40)).unwrap())
            .unwrap();
        p.aplicar(no, Coeficiente::new(dec!(0.05)).unwrap())
            .unwrap();
        p.aplicar(no, Coeficiente::new(dec!(0.05)).unwrap())
            .unwrap();

        assert_eq!(p.por_opcion[&si], dec!(0.40));
        assert_eq!(p.por_opcion[&no], dec!(0.10));
        assert_eq!(p.total_votado(), dec!(0.50));
        assert_eq!(p.version, 3);
        assert!(p.aprobada(si, ReglaMayoria::Simple, dec!(0.60), dec!(1)));
    }

    #[test]
    fn rechaza_opcion_desconocida() {
        let mut p = ProyeccionResultados::new(PreguntaId::nuevo(), &[OpcionId::nuevo()]);
        assert!(
            p.aplicar(OpcionId::nuevo(), Coeficiente::new(dec!(0.1)).unwrap())
                .is_err()
        );
    }
}
