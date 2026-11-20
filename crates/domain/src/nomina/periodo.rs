//! Periodos de la nómina (plan 16 fase 2): fechas válidas por periodicidad y estados.

use chrono::{Datelike, Days, NaiveDate};
use serde::{Deserialize, Serialize};

use super::PeriodicidadPago;
use crate::DominioError;

/// Estado de un periodo de nómina.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoPeriodoNomina {
    /// Recibe novedades; aún no se liquida.
    Abierto,
    /// Liquidado: se puede volver a liquidar o aprobar.
    Liquidado,
    /// Aprobado por otra cuenta: sus liquidaciones ya no cambian.
    Aprobado,
    /// Pagado (estado final).
    Pagado,
}

enum_texto!(EstadoPeriodoNomina {
    Abierto => "abierto",
    Liquidado => "liquidado",
    Aprobado => "aprobado",
    Pagado => "pagado",
});

impl EstadoPeriodoNomina {
    /// Única forma de cambiar el estado: abierto → liquidado (y liquidar de nuevo) → aprobado →
    /// pagado.
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si la transición no está permitida.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoPeriodoNomina as E;
        match (self, hacia) {
            (E::Abierto | E::Liquidado, E::Liquidado)
            | (E::Liquidado, E::Aprobado)
            | (E::Aprobado, E::Pagado) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "periodo de nómina",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }
}

fn fin_de_mes(d: NaiveDate) -> Option<NaiveDate> {
    let primero = d.with_day(1)?;
    primero
        .checked_add_months(chrono::Months::new(1))?
        .checked_sub_days(Days::new(1))
}

/// Último día del periodo que empieza en `desde`: el mes completo (mensual) o la quincena del
/// 1 al 15 o del 16 al fin de mes (quincenal).
///
/// # Errors
///
/// [`DominioError::ReglaLaboral`] si `desde` no es el inicio de un periodo de esa periodicidad.
pub fn fin_de_periodo(
    periodicidad: PeriodicidadPago,
    desde: NaiveDate,
) -> Result<NaiveDate, DominioError> {
    let fin = fin_de_mes(desde);
    match (periodicidad, desde.day()) {
        (PeriodicidadPago::Mensual, 1) | (PeriodicidadPago::Quincenal, 16) => fin,
        (PeriodicidadPago::Quincenal, 1) => desde.with_day(15),
        _ => None,
    }
    .ok_or_else(|| {
        DominioError::ReglaLaboral(format!(
            "un periodo `{periodicidad}` empieza el día 1{}",
            if periodicidad == PeriodicidadPago::Quincenal {
                " o el 16"
            } else {
                ""
            }
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, d).expect("fecha")
    }

    #[test]
    fn las_fechas_del_periodo_dependen_de_la_periodicidad() {
        use PeriodicidadPago::{Mensual, Quincenal};
        assert_eq!(
            fin_de_periodo(Mensual, fecha(2026, 2, 1)),
            Ok(fecha(2026, 2, 28))
        );
        assert_eq!(
            fin_de_periodo(Quincenal, fecha(2026, 2, 1)),
            Ok(fecha(2026, 2, 15))
        );
        assert_eq!(
            fin_de_periodo(Quincenal, fecha(2026, 2, 16)),
            Ok(fecha(2026, 2, 28))
        );
        assert_eq!(
            fin_de_periodo(Mensual, fecha(2026, 12, 1)),
            Ok(fecha(2026, 12, 31))
        );
        assert!(fin_de_periodo(Mensual, fecha(2026, 2, 16)).is_err());
        assert!(fin_de_periodo(Quincenal, fecha(2026, 2, 10)).is_err());
    }

    #[test]
    fn el_periodo_avanza_y_no_vuelve_atras() {
        use EstadoPeriodoNomina as E;
        assert_eq!(E::Abierto.transicionar(E::Liquidado), Ok(E::Liquidado));
        assert_eq!(E::Liquidado.transicionar(E::Liquidado), Ok(E::Liquidado));
        assert_eq!(E::Liquidado.transicionar(E::Aprobado), Ok(E::Aprobado));
        assert_eq!(E::Aprobado.transicionar(E::Pagado), Ok(E::Pagado));
        assert!(E::Abierto.transicionar(E::Aprobado).is_err());
        assert!(E::Aprobado.transicionar(E::Liquidado).is_err());
        assert!(E::Pagado.transicionar(E::Aprobado).is_err());
    }
}
