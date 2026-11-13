//! Periodos contables, sus estados y el marco técnico.

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Marco técnico contable de la entidad (Decreto 2420 de 2015 y Decreto 1670 de 2021). Lo
/// elige el contador con soporte; ninguno se asigna solo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarcoTecnico {
    #[serde(rename = "grupo_1")]
    Grupo1,
    #[serde(rename = "grupo_2")]
    Grupo2,
    #[serde(rename = "grupo_3")]
    Grupo3,
    Otro,
}

enum_texto!(MarcoTecnico {
    Grupo1 => "grupo_1",
    Grupo2 => "grupo_2",
    Grupo3 => "grupo_3",
    Otro => "otro",
});

/// Estado de un periodo. Solo en `Abierto` se contabiliza; reabrir uno cerrado exige una
/// reapertura aprobada por otra cuenta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoPeriodo {
    Abierto,
    EnRevision,
    Cerrado,
}

enum_texto!(EstadoPeriodo {
    Abierto => "abierto",
    EnRevision => "en_revision",
    Cerrado => "cerrado",
});

impl EstadoPeriodo {
    /// Transiciones permitidas: abierto ⇄ en revisión, ambos → cerrado, cerrado → abierto
    /// (reapertura).
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(self, hacia: EstadoPeriodo) -> Result<EstadoPeriodo, DominioError> {
        use EstadoPeriodo::{Abierto, Cerrado, EnRevision};
        match (self, hacia) {
            (Abierto, EnRevision | Cerrado)
            | (EnRevision, Abierto | Cerrado)
            | (Cerrado, Abierto) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "periodo contable",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }
}

/// Estado de una solicitud de reapertura.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoReapertura {
    Pendiente,
    Aprobada,
    Rechazada,
}

enum_texto!(EstadoReapertura {
    Pendiente => "pendiente",
    Aprobada => "aprobada",
    Rechazada => "rechazada",
});

/// Periodo contable: un mes del año, o el mes 13 (cierre anual, fechado el 31 de diciembre).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PeriodoContable {
    pub anio: i32,
    pub mes: u8,
}

/// Mes del cierre anual.
pub const MES_CIERRE: u8 = 13;

impl PeriodoContable {
    /// Valida el periodo.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si el año no está entre 2000 y 2999 o el mes entre 1 y 13.
    pub fn new(anio: i32, mes: u8) -> Result<Self, DominioError> {
        if !(2000..=2999).contains(&anio) {
            return Err(DominioError::DatoInvalido {
                campo: "anio",
                regla: "entre 2000 y 2999",
            });
        }
        if !(1..=MES_CIERRE).contains(&mes) {
            return Err(DominioError::DatoInvalido {
                campo: "mes",
                regla: "entre 1 y 13 (13 = cierre anual)",
            });
        }
        Ok(Self { anio, mes })
    }

    /// Periodo mensual de una fecha.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si el año está fuera de rango.
    pub fn de_fecha(fecha: NaiveDate) -> Result<Self, DominioError> {
        Self::new(fecha.year(), u8::try_from(fecha.month()).unwrap_or(1))
    }

    /// Periodo del cierre anual del año.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si el año está fuera de rango.
    pub fn cierre_anual(anio: i32) -> Result<Self, DominioError> {
        Self::new(anio, MES_CIERRE)
    }

    /// ¿Es el periodo del cierre anual?
    pub fn es_cierre(self) -> bool {
        self.mes == MES_CIERRE
    }

    /// Primer y último día del periodo (el de cierre es solo el 31 de diciembre).
    pub fn rango(self) -> (NaiveDate, NaiveDate) {
        let fecha = |m: u32, d: u32| {
            NaiveDate::from_ymd_opt(self.anio, m, d).expect("el periodo se validó al crearlo")
        };
        if self.es_cierre() {
            return (fecha(12, 31), fecha(12, 31));
        }
        let inicio = fecha(u32::from(self.mes), 1);
        let fin = inicio
            .checked_add_months(chrono::Months::new(1))
            .and_then(|d| d.pred_opt())
            .expect("el mes siguiente existe");
        (inicio, fin)
    }

    /// ¿La fecha cae en el periodo?
    pub fn contiene(self, fecha: NaiveDate) -> bool {
        let (inicio, fin) = self.rango();
        inicio <= fecha && fecha <= fin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, d).expect("fecha")
    }

    #[test]
    fn transiciones_del_periodo() {
        use EstadoPeriodo::{Abierto, Cerrado, EnRevision};
        assert_eq!(Abierto.transicionar(Cerrado), Ok(Cerrado));
        assert_eq!(EnRevision.transicionar(Abierto), Ok(Abierto));
        assert_eq!(Cerrado.transicionar(Abierto), Ok(Abierto));
        assert!(Cerrado.transicionar(EnRevision).is_err());
        assert!(Abierto.transicionar(Abierto).is_err());
    }

    #[test]
    fn rango_de_meses_y_del_cierre() {
        let feb = PeriodoContable::new(2028, 2).expect("ok");
        assert_eq!(feb.rango(), (fecha(2028, 2, 1), fecha(2028, 2, 29)));
        let cierre = PeriodoContable::cierre_anual(2026).expect("ok");
        assert!(cierre.contiene(fecha(2026, 12, 31)));
        assert!(!cierre.contiene(fecha(2026, 12, 30)));
        assert_eq!(
            PeriodoContable::de_fecha(fecha(2026, 11, 13)).expect("ok"),
            PeriodoContable {
                anio: 2026,
                mes: 11
            }
        );
    }

    #[test]
    fn periodos_invalidos() {
        assert!(PeriodoContable::new(2026, 0).is_err());
        assert!(PeriodoContable::new(2026, 14).is_err());
        assert!(PeriodoContable::new(1999, 1).is_err());
    }
}
