//! Partida doble: solo se contabiliza un [`AsientoBalanceado`].

use serde::{Deserialize, Serialize};

use super::Monto;
use crate::{CentroCostoId, CuentaContableId, DominioError, PersonaId};

/// Línea de un asiento: una cuenta, una sola cara (débito o crédito) y, si la cuenta lo exige,
/// el tercero (`persona`) y el centro de costo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineaAsiento {
    pub cuenta: CuentaContableId,
    pub persona: Option<PersonaId>,
    pub centro_costo: Option<CentroCostoId>,
    pub debito: Monto,
    pub credito: Monto,
    /// Base gravable de un impuesto o retención (plan 15); informativa.
    pub base: Option<Monto>,
    pub descripcion: Option<String>,
}

impl LineaAsiento {
    /// Línea al débito.
    pub fn debito(cuenta: CuentaContableId, monto: Monto) -> Self {
        Self::nueva(cuenta, monto, Monto::CERO)
    }

    /// Línea al crédito.
    pub fn credito(cuenta: CuentaContableId, monto: Monto) -> Self {
        Self::nueva(cuenta, Monto::CERO, monto)
    }

    fn nueva(cuenta: CuentaContableId, debito: Monto, credito: Monto) -> Self {
        Self {
            cuenta,
            persona: None,
            centro_costo: None,
            debito,
            credito,
            base: None,
            descripcion: None,
        }
    }

    /// Con tercero.
    pub fn con_persona(mut self, persona: Option<PersonaId>) -> Self {
        self.persona = persona;
        self
    }

    /// Con centro de costo.
    pub fn con_centro(mut self, centro: Option<CentroCostoId>) -> Self {
        self.centro_costo = centro;
        self
    }

    /// Con descripción.
    pub fn con_descripcion(mut self, descripcion: Option<String>) -> Self {
        self.descripcion = descripcion;
        self
    }

    /// La misma línea con las caras invertidas (para el reverso).
    fn invertida(&self) -> Self {
        Self {
            debito: self.credito,
            credito: self.debito,
            ..self.clone()
        }
    }
}

/// Lo que el asiento necesita saber de cada cuenta para validarse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReglasCuenta {
    pub codigo: String,
    pub acepta_movimiento: bool,
    pub activa: bool,
    pub exige_tercero: bool,
    pub exige_centro_costo: bool,
}

/// Asiento que cumple la partida doble: al menos dos líneas, cada una con una sola cara
/// distinta de cero, y suma de débitos = suma de créditos exactamente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AsientoBalanceado {
    lineas: Vec<LineaAsiento>,
}

fn error(texto: impl Into<String>) -> DominioError {
    DominioError::ReglaContable(texto.into())
}

impl AsientoBalanceado {
    /// Construye el asiento si cumple la partida doble.
    ///
    /// # Errors
    ///
    /// [`DominioError::ReglaContable`] si tiene menos de dos líneas, una línea tiene las dos
    /// caras o ninguna, o los débitos no igualan a los créditos.
    pub fn nuevo(lineas: Vec<LineaAsiento>) -> Result<Self, DominioError> {
        if lineas.len() < 2 {
            return Err(error("un asiento tiene al menos dos líneas"));
        }
        if let Some(i) = lineas
            .iter()
            .position(|l| l.debito.es_cero() == l.credito.es_cero())
        {
            return Err(error(format!(
                "la línea {} debe tener valor en una sola cara (débito o crédito)",
                i + 1
            )));
        }
        let debitos: Monto = lineas.iter().map(|l| l.debito).sum();
        let creditos: Monto = lineas.iter().map(|l| l.credito).sum();
        if debitos != creditos {
            return Err(error(format!(
                "los débitos ({}) no igualan a los créditos ({})",
                debitos.valor(),
                creditos.valor()
            )));
        }
        Ok(Self { lineas })
    }

    /// Las líneas, en orden.
    pub fn lineas(&self) -> &[LineaAsiento] {
        &self.lineas
    }

    /// Total del asiento (suma de débitos = suma de créditos).
    pub fn total(&self) -> Monto {
        self.lineas.iter().map(|l| l.debito).sum()
    }

    /// Asiento con las caras invertidas: anula el efecto de este.
    pub fn reverso(&self) -> Self {
        Self {
            lineas: self.lineas.iter().map(LineaAsiento::invertida).collect(),
        }
    }

    /// Valida las líneas contra el plan de cuentas: cuentas existentes, activas y de
    /// movimiento, con el tercero y el centro de costo que cada una exige.
    ///
    /// # Errors
    ///
    /// [`DominioError::ReglaContable`] con la primera cuenta que no admite la línea.
    pub fn validar_contra(
        &self,
        cuenta: impl Fn(CuentaContableId) -> Option<ReglasCuenta>,
    ) -> Result<(), DominioError> {
        for (i, l) in self.lineas.iter().enumerate() {
            let r = cuenta(l.cuenta)
                .ok_or_else(|| error(format!("la cuenta de la línea {} no existe", i + 1)))?;
            let problema = if !r.activa {
                Some("está inactiva")
            } else if !r.acepta_movimiento {
                Some("no acepta movimientos")
            } else if r.exige_tercero && l.persona.is_none() {
                Some("exige tercero")
            } else if r.exige_centro_costo && l.centro_costo.is_none() {
                Some("exige centro de costo")
            } else {
                None
            };
            if let Some(p) = problema {
                return Err(error(format!(
                    "la cuenta {} {p} (línea {})",
                    r.codigo,
                    i + 1
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;

    fn m(valor: i64) -> Monto {
        Monto::new(Decimal::from(valor)).expect("monto")
    }

    fn reglas(codigo: &str) -> ReglasCuenta {
        ReglasCuenta {
            codigo: codigo.into(),
            acepta_movimiento: true,
            activa: true,
            exige_tercero: false,
            exige_centro_costo: false,
        }
    }

    #[test]
    fn balanceado_con_dos_o_mas_lineas() {
        let (caja, ingreso) = (CuentaContableId::nuevo(), CuentaContableId::nuevo());
        let asiento = AsientoBalanceado::nuevo(vec![
            LineaAsiento::debito(caja, m(100)),
            LineaAsiento::credito(ingreso, m(60)),
            LineaAsiento::credito(ingreso, m(40)),
        ])
        .expect("cuadra");
        assert_eq!(asiento.total(), m(100));
    }

    #[test]
    fn debitos_distintos_de_creditos_se_rechaza() {
        let c = CuentaContableId::nuevo();
        let err = AsientoBalanceado::nuevo(vec![
            LineaAsiento::debito(c, m(100)),
            LineaAsiento::credito(c, m(99)),
        ])
        .expect_err("no cuadra");
        assert!(matches!(err, DominioError::ReglaContable(t) if t.contains("débitos")));
    }

    #[test]
    fn una_linea_o_lineas_sin_cara_se_rechazan() {
        let c = CuentaContableId::nuevo();
        assert!(AsientoBalanceado::nuevo(vec![LineaAsiento::debito(c, m(1))]).is_err());
        let en_cero = LineaAsiento::debito(c, Monto::CERO);
        assert!(AsientoBalanceado::nuevo(vec![en_cero.clone(), en_cero]).is_err());
        let dos_caras = LineaAsiento {
            credito: m(1),
            ..LineaAsiento::debito(c, m(1))
        };
        assert!(AsientoBalanceado::nuevo(vec![dos_caras, LineaAsiento::credito(c, m(1))]).is_err());
    }

    #[test]
    fn el_reverso_invierte_las_caras() {
        let (a, b) = (CuentaContableId::nuevo(), CuentaContableId::nuevo());
        let asiento = AsientoBalanceado::nuevo(vec![
            LineaAsiento::debito(a, m(5)),
            LineaAsiento::credito(b, m(5)),
        ])
        .expect("cuadra");
        let reverso = asiento.reverso();
        assert_eq!(reverso.lineas()[0].credito, m(5));
        assert_eq!(reverso.lineas()[1].debito, m(5));
        assert_eq!(reverso.reverso(), asiento);
    }

    #[test]
    fn la_cuenta_que_exige_tercero_lo_pide() {
        let (cxc, ingreso) = (CuentaContableId::nuevo(), CuentaContableId::nuevo());
        let asiento = AsientoBalanceado::nuevo(vec![
            LineaAsiento::debito(cxc, m(5)),
            LineaAsiento::credito(ingreso, m(5)),
        ])
        .expect("cuadra");
        let err = asiento
            .validar_contra(|id| {
                Some(if id == cxc {
                    ReglasCuenta {
                        exige_tercero: true,
                        ..reglas("130505")
                    }
                } else {
                    reglas("413505")
                })
            })
            .expect_err("falta el tercero");
        assert!(matches!(err, DominioError::ReglaContable(t) if t.contains("130505")));
        assert!(asiento.validar_contra(|_| Some(reglas("1105"))).is_ok());
    }

    #[test]
    fn cuentas_inactivas_o_sin_movimiento_se_rechazan() {
        let c = CuentaContableId::nuevo();
        let asiento = AsientoBalanceado::nuevo(vec![
            LineaAsiento::debito(c, m(1)),
            LineaAsiento::credito(c, m(1)),
        ])
        .expect("cuadra");
        let inactiva = ReglasCuenta {
            activa: false,
            ..reglas("1105")
        };
        let titulo = ReglasCuenta {
            acepta_movimiento: false,
            ..reglas("11")
        };
        assert!(asiento.validar_contra(|_| Some(inactiva.clone())).is_err());
        assert!(asiento.validar_contra(|_| Some(titulo.clone())).is_err());
        assert!(asiento.validar_contra(|_| None).is_err());
    }
}
