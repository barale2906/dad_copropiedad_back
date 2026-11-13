//! Estados financieros y cierre anual a partir de saldos por cuenta.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::{AsientoBalanceado, CodigoCuenta, LineaAsiento, Monto};
use crate::{CentroCostoId, CuentaContableId, DominioError, PersonaId};

/// Saldo de una cuenta de movimiento hasta una fecha: débitos y créditos acumulados.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaldoCuenta {
    pub codigo: CodigoCuenta,
    pub debitos: Decimal,
    pub creditos: Decimal,
}

impl SaldoCuenta {
    /// Débitos − créditos (positivo = saldo débito).
    pub fn neto(&self) -> Decimal {
        self.debitos - self.creditos
    }
}

/// Rubro con sus prefijos de cuenta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rubro {
    pub codigo: String,
    pub nombre: String,
    pub orden: u16,
    pub prefijos: Vec<String>,
}

/// Valor de un rubro en el estado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValorRubro {
    pub codigo: String,
    pub nombre: String,
    pub valor: Decimal,
}

/// Clases del PUC de naturaleza débito (activo, gastos, costos, orden deudoras).
fn clase_debito(clase: u8) -> bool {
    matches!(clase, 1 | 5 | 6 | 7 | 8)
}

/// Valor de cada rubro, en su orden. Cada cuenta suma con el signo natural del rubro (el de la
/// clase de su primer prefijo): así una cuenta correctora (p. ej. depreciación acumulada, saldo
/// crédito dentro del activo) resta. Una cuenta que cae en varios prefijos del mismo rubro
/// cuenta una sola vez.
pub fn valores_rubros(rubros: &[Rubro], saldos: &[SaldoCuenta]) -> Vec<ValorRubro> {
    let mut ordenados: Vec<&Rubro> = rubros.iter().collect();
    ordenados.sort_by_key(|r| (r.orden, r.codigo.clone()));
    ordenados
        .into_iter()
        .map(|r| {
            let debito = r
                .prefijos
                .first()
                .and_then(|p| p.bytes().next())
                .is_some_and(|b| clase_debito(b.wrapping_sub(b'0')));
            let neto: Decimal = saldos
                .iter()
                .filter(|s| r.prefijos.iter().any(|p| s.codigo.empieza_por(p)))
                .map(SaldoCuenta::neto)
                .sum();
            ValorRubro {
                codigo: r.codigo.clone(),
                nombre: r.nombre.clone(),
                valor: if debito { neto } else { -neto },
            }
        })
        .collect()
}

/// Resultado del ejercicio: ingresos (clase 4) menos gastos y costos (clases 5, 6 y 7).
/// Positivo = excedente o utilidad.
pub fn resultado_del_ejercicio(saldos: &[SaldoCuenta]) -> Decimal {
    saldos
        .iter()
        .filter(|s| s.codigo.es_de_resultado())
        .map(|s| -s.neto())
        .sum()
}

/// Saldo de una cuenta de resultado por tercero y centro de costo, para el cierre anual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaldoResultado {
    pub cuenta: CuentaContableId,
    pub persona: Option<PersonaId>,
    pub centro_costo: Option<CentroCostoId>,
    /// Débitos − créditos del año.
    pub neto: Decimal,
}

fn monto(valor: Decimal) -> Result<Monto, DominioError> {
    Monto::new(valor.abs())
}

/// Asiento de cierre anual: lleva a cero cada saldo de resultado y lleva la diferencia a la
/// cuenta de excedente o déficit del ejercicio.
///
/// # Errors
///
/// [`DominioError::ReglaContable`] si no hay saldos que cerrar.
pub fn asiento_de_cierre(
    saldos: &[SaldoResultado],
    cuenta_resultado: CuentaContableId,
) -> Result<AsientoBalanceado, DominioError> {
    let mut lineas = Vec::new();
    let mut total = Decimal::ZERO;
    for s in saldos.iter().filter(|s| !s.neto.is_zero()) {
        let m = monto(s.neto)?;
        let linea = if s.neto > Decimal::ZERO {
            LineaAsiento::credito(s.cuenta, m)
        } else {
            LineaAsiento::debito(s.cuenta, m)
        };
        lineas.push(linea.con_persona(s.persona).con_centro(s.centro_costo));
        total += s.neto;
    }
    if lineas.is_empty() {
        return Err(DominioError::ReglaContable(
            "no hay saldos de resultado que cerrar".into(),
        ));
    }
    // total > 0: hubo más gasto que ingreso (déficit) → débito a la cuenta de resultado.
    if !total.is_zero() {
        let m = monto(total)?;
        lineas.push(if total > Decimal::ZERO {
            LineaAsiento::debito(cuenta_resultado, m)
        } else {
            LineaAsiento::credito(cuenta_resultado, m)
        });
    }
    AsientoBalanceado::nuevo(lineas)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saldo(codigo: &str, debitos: i64, creditos: i64) -> SaldoCuenta {
        SaldoCuenta {
            codigo: CodigoCuenta::new(codigo).expect("código"),
            debitos: Decimal::from(debitos),
            creditos: Decimal::from(creditos),
        }
    }

    fn rubro(codigo: &str, orden: u16, prefijos: &[&str]) -> Rubro {
        Rubro {
            codigo: codigo.into(),
            nombre: codigo.into(),
            orden,
            prefijos: prefijos.iter().map(|p| (*p).to_owned()).collect(),
        }
    }

    #[test]
    fn rubros_con_signo_natural_y_cuentas_correctoras() {
        let saldos = [
            saldo("110505", 1000, 200),
            saldo("152405", 500, 0),
            saldo("159205", 0, 100),
            saldo("220505", 0, 300),
        ];
        let rubros = [
            rubro("ppe", 2, &["15"]),
            rubro("efectivo", 1, &["11"]),
            rubro("proveedores", 3, &["22"]),
        ];

        let v = valores_rubros(&rubros, &saldos);

        assert_eq!(v[0].codigo, "efectivo");
        assert_eq!(v[0].valor, Decimal::from(800));
        assert_eq!(v[1].valor, Decimal::from(400), "la depreciación resta");
        assert_eq!(
            v[2].valor,
            Decimal::from(300),
            "el pasivo se muestra positivo"
        );
    }

    #[test]
    fn resultado_es_ingresos_menos_gastos_y_costos() {
        let saldos = [
            saldo("4170", 0, 1000),
            saldo("5105", 600, 0),
            saldo("6135", 100, 0),
            saldo("1105", 999, 0),
        ];
        assert_eq!(resultado_del_ejercicio(&saldos), Decimal::from(300));
    }

    #[test]
    fn el_cierre_lleva_a_cero_los_resultados_contra_el_excedente() {
        let (ingreso, gasto, excedente) = (
            CuentaContableId::nuevo(),
            CuentaContableId::nuevo(),
            CuentaContableId::nuevo(),
        );
        let saldos = [
            SaldoResultado {
                cuenta: ingreso,
                persona: None,
                centro_costo: None,
                neto: Decimal::from(-1000),
            },
            SaldoResultado {
                cuenta: gasto,
                persona: None,
                centro_costo: None,
                neto: Decimal::from(700),
            },
        ];

        let asiento = asiento_de_cierre(&saldos, excedente).expect("cuadra");

        let l = asiento.lineas();
        assert_eq!(
            (l[0].cuenta, l[0].debito.valor()),
            (ingreso, Decimal::from(1000))
        );
        assert_eq!(
            (l[1].cuenta, l[1].credito.valor()),
            (gasto, Decimal::from(700))
        );
        assert_eq!(
            (l[2].cuenta, l[2].credito.valor()),
            (excedente, Decimal::from(300)),
            "excedente al crédito"
        );
    }

    #[test]
    fn sin_saldos_no_hay_cierre() {
        let c = CuentaContableId::nuevo();
        let cero = SaldoResultado {
            cuenta: c,
            persona: None,
            centro_costo: None,
            neto: Decimal::ZERO,
        };
        assert!(asiento_de_cierre(&[cero], c).is_err());
    }
}
