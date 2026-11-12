//! Horarios semanales con excepciones por fecha. Componente reutilizable: lo usan los puntos de
//! atención de la copropiedad (plan 03) y los bienes comunes reservables (plan 04).
//!
//! Los festivos de Colombia ([`crate::calendario`]) cierran salvo que la franja diga
//! `aplica_festivos`; una excepción de la fecha manda sobre todo lo demás.

use chrono::{Datelike, NaiveDate, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};

use crate::{DominioError, calendario};

/// Franja de atención de un día de la semana, `[inicio, fin)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FranjaSemanal {
    pub dia: Weekday,
    pub inicio: NaiveTime,
    pub fin: NaiveTime,
    /// ¿También rige si el día es festivo?
    pub aplica_festivos: bool,
}

/// Excepción de una fecha: cerrado (`franja: None`) o con una franja especial.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExcepcionHorario {
    pub fecha: NaiveDate,
    pub franja: Option<(NaiveTime, NaiveTime)>,
    pub motivo: Option<String>,
}

/// Horario válido: franjas con fin posterior al inicio y sin solaparse el mismo día; a lo sumo
/// una excepción por fecha.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Horario {
    franjas: Vec<FranjaSemanal>,
    excepciones: Vec<ExcepcionHorario>,
}

impl Horario {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si una franja termina antes de empezar, dos franjas del
    /// mismo día se solapan o una fecha tiene dos excepciones.
    pub fn new(
        mut franjas: Vec<FranjaSemanal>,
        mut excepciones: Vec<ExcepcionHorario>,
    ) -> Result<Self, DominioError> {
        let invalido = |regla| DominioError::DatoInvalido {
            campo: "horario",
            regla,
        };
        franjas.sort_by_key(|f| (f.dia.number_from_monday(), f.inicio));
        if franjas.iter().any(|f| f.fin <= f.inicio)
            || excepciones
                .iter()
                .any(|e| e.franja.is_some_and(|(i, f)| f <= i))
        {
            return Err(invalido("cada franja debe terminar después de empezar"));
        }
        if franjas
            .windows(2)
            .any(|w| w[0].dia == w[1].dia && w[1].inicio < w[0].fin)
        {
            return Err(invalido("las franjas de un mismo día no pueden solaparse"));
        }
        excepciones.sort_by_key(|e| e.fecha);
        if excepciones.windows(2).any(|w| w[0].fecha == w[1].fecha) {
            return Err(invalido("una fecha solo puede tener una excepción"));
        }
        Ok(Self {
            franjas,
            excepciones,
        })
    }

    /// Franjas semanales del horario.
    pub fn franjas(&self) -> &[FranjaSemanal] {
        &self.franjas
    }

    /// Excepciones por fecha (festivos, cierres).
    pub fn excepciones(&self) -> &[ExcepcionHorario] {
        &self.excepciones
    }

    /// Franjas `(inicio, fin)` que rigen en `fecha`.
    pub fn franjas_en(&self, fecha: NaiveDate) -> Vec<(NaiveTime, NaiveTime)> {
        if let Some(e) = self.excepciones.iter().find(|e| e.fecha == fecha) {
            return e.franja.into_iter().collect();
        }
        let festivo = calendario::es_festivo(fecha);
        self.franjas
            .iter()
            .filter(|f| f.dia == fecha.weekday() && (!festivo || f.aplica_festivos))
            .map(|f| (f.inicio, f.fin))
            .collect()
    }

    /// ¿Está abierto en `fecha` a la `hora`?
    pub fn abierto(&self, fecha: NaiveDate, hora: NaiveTime) -> bool {
        self.franjas_en(fecha)
            .iter()
            .any(|(i, f)| *i <= hora && hora < *f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(hh: u32, mm: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(hh, mm, 0).expect("hora válida")
    }

    fn d(a: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, dd).expect("fecha válida")
    }

    fn oficina() -> Horario {
        Horario::new(
            vec![
                FranjaSemanal {
                    dia: Weekday::Mon,
                    inicio: h(8, 0),
                    fin: h(12, 0),
                    aplica_festivos: false,
                },
                FranjaSemanal {
                    dia: Weekday::Mon,
                    inicio: h(14, 0),
                    fin: h(18, 0),
                    aplica_festivos: false,
                },
            ],
            vec![ExcepcionHorario {
                fecha: d(2026, 11, 23),
                franja: None,
                motivo: Some("Inventario".into()),
            }],
        )
        .expect("válido")
    }

    #[test]
    fn abre_en_sus_franjas_y_cierra_al_mediodia() {
        let lunes = d(2026, 11, 9);
        assert!(oficina().abierto(lunes, h(9, 0)));
        assert!(!oficina().abierto(lunes, h(12, 0)));
        assert!(oficina().abierto(lunes, h(14, 0)));
        assert!(!oficina().abierto(d(2026, 11, 10), h(9, 0)));
    }

    #[test]
    fn festivo_cierra_y_excepcion_manda() {
        // 16 de noviembre de 2026: lunes festivo (Independencia de Cartagena, trasladado).
        assert!(oficina().franjas_en(d(2026, 11, 16)).is_empty());
        assert!(oficina().franjas_en(d(2026, 11, 23)).is_empty());
    }

    #[test]
    fn rechaza_franjas_invertidas_o_solapadas() {
        let f = |i, fin| FranjaSemanal {
            dia: Weekday::Tue,
            inicio: i,
            fin,
            aplica_festivos: false,
        };
        assert!(Horario::new(vec![f(h(10, 0), h(9, 0))], vec![]).is_err());
        assert!(Horario::new(vec![f(h(8, 0), h(12, 0)), f(h(11, 0), h(13, 0))], vec![]).is_err());
        let e = ExcepcionHorario {
            fecha: d(2026, 1, 2),
            franja: None,
            motivo: None,
        };
        assert!(Horario::new(vec![], vec![e.clone(), e]).is_err());
    }
}
