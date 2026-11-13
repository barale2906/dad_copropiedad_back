//! Reglas de contabilización: cómo un documento origen se vuelve asiento.
//!
//! Un módulo operativo no escribe asientos: entrega importes por **concepto** (p. ej.
//! `expensa_ordinaria`, `iva_generado`) y la regla vigente del evento dice a qué cuenta y a qué
//! cara va cada concepto. Un concepto puede tener una línea al débito y otra al crédito (se
//! cuadra solo) o repartirse entre varios conceptos que juntos cuadran.

use serde::{Deserialize, Serialize};

use super::{AsientoBalanceado, LineaAsiento, Monto, es_identificador};
use crate::{CentroCostoId, CuentaContableId, DominioError, PersonaId};

/// Cara de una línea de regla.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cara {
    Debito,
    Credito,
}

enum_texto!(Cara {
    Debito => "debito",
    Credito => "credito",
});

/// De dónde sale el tercero de la línea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrigenTercero {
    /// El que trae el importe del documento.
    Documento,
    Ninguno,
}

enum_texto!(OrigenTercero {
    Documento => "documento",
    Ninguno => "ninguno",
});

/// De dónde sale el centro de costo de la línea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrigenCentro {
    Documento,
    /// Uno fijo de la regla.
    Fijo,
    Ninguno,
}

enum_texto!(OrigenCentro {
    Documento => "documento",
    Fijo => "fijo",
    Ninguno => "ninguno",
});

/// Línea de una regla: concepto → cuenta y cara.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineaRegla {
    pub concepto: String,
    pub cara: Cara,
    pub cuenta: CuentaContableId,
    pub tercero: OrigenTercero,
    pub centro: OrigenCentro,
    /// Solo con [`OrigenCentro::Fijo`].
    pub centro_fijo: Option<CentroCostoId>,
}

/// Regla de contabilización de un evento (una versión).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReglaContable {
    pub evento: String,
    pub lineas: Vec<LineaRegla>,
}

/// Importe de un documento origen por concepto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImporteDocumento {
    pub concepto: String,
    pub monto: Monto,
    pub persona: Option<PersonaId>,
    pub centro_costo: Option<CentroCostoId>,
    pub descripcion: Option<String>,
}

fn error(texto: impl Into<String>) -> DominioError {
    DominioError::ReglaContable(texto.into())
}

impl ReglaContable {
    /// Valida la regla: evento y conceptos bien formados, al menos dos líneas, centro fijo solo
    /// donde corresponde y sin líneas repetidas (concepto + cara + cuenta).
    ///
    /// # Errors
    ///
    /// [`DominioError::ReglaContable`] con el problema.
    pub fn validar(&self) -> Result<(), DominioError> {
        if !es_identificador(&self.evento, true, 80) {
            return Err(error(
                "el evento usa minúsculas, dígitos, `_` y `.` y empieza por letra",
            ));
        }
        if self.lineas.len() < 2 {
            return Err(error("una regla tiene al menos dos líneas"));
        }
        for (i, l) in self.lineas.iter().enumerate() {
            if !es_identificador(&l.concepto, false, 60) {
                return Err(error(format!(
                    "el concepto de la línea {} es inválido",
                    i + 1
                )));
            }
            if (l.centro == OrigenCentro::Fijo) != l.centro_fijo.is_some() {
                return Err(error(format!(
                    "la línea {} lleva centro de costo fijo solo si su origen es `fijo`",
                    i + 1
                )));
            }
            if self.lineas[..i]
                .iter()
                .any(|o| o.concepto == l.concepto && o.cara == l.cara && o.cuenta == l.cuenta)
            {
                return Err(error(format!("la línea {} está repetida", i + 1)));
            }
        }
        Ok(())
    }

    /// Arma el asiento de los importes: cada importe genera una línea por cada línea de la
    /// regla con su concepto. Los importes en cero se omiten.
    ///
    /// # Errors
    ///
    /// [`DominioError::ReglaContable`] si un concepto no está en la regla o el asiento no cuadra.
    pub fn aplicar(
        &self,
        importes: &[ImporteDocumento],
    ) -> Result<AsientoBalanceado, DominioError> {
        let mut lineas = Vec::new();
        for imp in importes.iter().filter(|i| !i.monto.es_cero()) {
            let de_concepto: Vec<&LineaRegla> = self
                .lineas
                .iter()
                .filter(|l| l.concepto == imp.concepto)
                .collect();
            if de_concepto.is_empty() {
                return Err(error(format!(
                    "el concepto `{}` no está en la regla de `{}`",
                    imp.concepto, self.evento
                )));
            }
            for l in de_concepto {
                let linea = match l.cara {
                    Cara::Debito => LineaAsiento::debito(l.cuenta, imp.monto),
                    Cara::Credito => LineaAsiento::credito(l.cuenta, imp.monto),
                };
                let persona = match l.tercero {
                    OrigenTercero::Documento => imp.persona,
                    OrigenTercero::Ninguno => None,
                };
                let centro = match l.centro {
                    OrigenCentro::Documento => imp.centro_costo,
                    OrigenCentro::Fijo => l.centro_fijo,
                    OrigenCentro::Ninguno => None,
                };
                lineas.push(
                    linea
                        .con_persona(persona)
                        .con_centro(centro)
                        .con_descripcion(imp.descripcion.clone()),
                );
            }
        }
        AsientoBalanceado::nuevo(lineas)
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;

    fn m(valor: i64) -> Monto {
        Monto::new(Decimal::from(valor)).expect("monto")
    }

    fn linea(concepto: &str, cara: Cara, cuenta: CuentaContableId) -> LineaRegla {
        LineaRegla {
            concepto: concepto.into(),
            cara,
            cuenta,
            tercero: OrigenTercero::Documento,
            centro: OrigenCentro::Ninguno,
            centro_fijo: None,
        }
    }

    fn importe(concepto: &str, valor: i64, persona: Option<PersonaId>) -> ImporteDocumento {
        ImporteDocumento {
            concepto: concepto.into(),
            monto: m(valor),
            persona,
            centro_costo: None,
            descripcion: None,
        }
    }

    #[test]
    fn un_concepto_con_dos_caras_se_cuadra_solo() {
        let (cxc, ingreso) = (CuentaContableId::nuevo(), CuentaContableId::nuevo());
        let regla = ReglaContable {
            evento: "ventas.factura".into(),
            lineas: vec![
                linea("venta", Cara::Debito, cxc),
                LineaRegla {
                    tercero: OrigenTercero::Ninguno,
                    ..linea("venta", Cara::Credito, ingreso)
                },
            ],
        };
        regla.validar().expect("válida");
        let cliente = PersonaId::nuevo();

        let asiento = regla
            .aplicar(&[importe("venta", 100, Some(cliente))])
            .expect("cuadra");

        assert_eq!(asiento.lineas()[0].persona, Some(cliente));
        assert_eq!(
            asiento.lineas()[1].persona,
            None,
            "el ingreso no lleva tercero"
        );
        assert_eq!(asiento.total(), m(100));
    }

    #[test]
    fn conceptos_que_juntos_cuadran_y_los_que_no() {
        let (banco, cxc, interes) = (
            CuentaContableId::nuevo(),
            CuentaContableId::nuevo(),
            CuentaContableId::nuevo(),
        );
        let regla = ReglaContable {
            evento: "cartera.recaudo".into(),
            lineas: vec![
                linea("recibido", Cara::Debito, banco),
                linea("capital", Cara::Credito, cxc),
                linea("interes", Cara::Credito, interes),
            ],
        };
        let ok = regla.aplicar(&[
            importe("recibido", 110, None),
            importe("capital", 100, None),
            importe("interes", 10, None),
            importe("interes", 0, None),
        ]);
        let descuadre = regla.aplicar(&[
            importe("recibido", 110, None),
            importe("capital", 100, None),
        ]);
        let desconocido = regla.aplicar(&[importe("multa", 5, None)]);

        assert_eq!(ok.expect("cuadra").lineas().len(), 3);
        assert!(descuadre.is_err());
        assert!(matches!(desconocido, Err(DominioError::ReglaContable(t)) if t.contains("multa")));
    }

    #[test]
    fn el_centro_fijo_va_en_la_linea() {
        let (a, b) = (CuentaContableId::nuevo(), CuentaContableId::nuevo());
        let centro = CentroCostoId::nuevo();
        let regla = ReglaContable {
            evento: "gastos.causacion".into(),
            lineas: vec![
                LineaRegla {
                    centro: OrigenCentro::Fijo,
                    centro_fijo: Some(centro),
                    ..linea("gasto", Cara::Debito, a)
                },
                linea("gasto", Cara::Credito, b),
            ],
        };
        let asiento = regla.aplicar(&[importe("gasto", 7, None)]).expect("cuadra");
        assert_eq!(asiento.lineas()[0].centro_costo, Some(centro));
    }

    #[test]
    fn reglas_mal_formadas() {
        let c = CuentaContableId::nuevo();
        let base = ReglaContable {
            evento: "x.y".into(),
            lineas: vec![linea("a", Cara::Debito, c), linea("aa", Cara::Credito, c)],
        };
        assert!(
            ReglaContable {
                evento: "Mal".into(),
                ..base.clone()
            }
            .validar()
            .is_err()
        );
        assert!(
            ReglaContable {
                lineas: vec![linea("aa", Cara::Debito, c), linea("aa", Cara::Debito, c)],
                ..base.clone()
            }
            .validar()
            .is_err(),
            "repetida"
        );
        let fijo_sin_centro = LineaRegla {
            centro: OrigenCentro::Fijo,
            ..linea("bb", Cara::Debito, c)
        };
        assert!(
            ReglaContable {
                lineas: vec![fijo_sin_centro, linea("cc", Cara::Credito, c)],
                ..base
            }
            .validar()
            .is_err()
        );
    }
}
