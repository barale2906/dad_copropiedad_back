//! Mora de la cartera de la PH (plan 14 fase 5): tasa de mora a partir del interés bancario
//! corriente (Ley 675 art. 30, tope de usura del Código de Comercio art. 884), intereses diarios
//! sobre el capital vencido por tramos de tasa y de saldo, acuerdos de pago y certificados.
//!
//! Sin anatocismo: la base es siempre el capital de una obligación que genera intereses; las
//! obligaciones de intereses no los generan.

use chrono::{Months, NaiveDate};
use rust_decimal::{Decimal, MathematicalOps, RoundingStrategy};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::repartir;
use crate::DominioError;

/// Modalidad de la tasa de referencia certificada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModalidadTasa {
    /// Crédito de consumo y ordinario (la que usa la mora de las expensas).
    ConsumoOrdinario,
}

enum_texto!(ModalidadTasa {
    ConsumoOrdinario => "consumo_ordinario",
});

/// Estado de una causación de intereses: se contabiliza al causarse; anular = reverso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoCausacion {
    Contabilizada,
    Anulada,
}

enum_texto!(EstadoCausacion {
    Contabilizada => "contabilizada",
    Anulada => "anulada",
});

/// Estado de un acuerdo de pago.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoAcuerdo {
    /// Registrado; falta que otra cuenta lo apruebe.
    Propuesto,
    /// Aprobado: suspende la mora de lo que se debía mientras se cumpla.
    Vigente,
    Cumplido,
    /// Una cuota venció sin pagarse: la mora se reanuda.
    Incumplido,
    Anulado,
}

enum_texto!(EstadoAcuerdo {
    Propuesto => "propuesto",
    Vigente => "vigente",
    Cumplido => "cumplido",
    Incumplido => "incumplido",
    Anulado => "anulado",
});

impl EstadoAcuerdo {
    /// propuesto → vigente | anulado; vigente → cumplido | incumplido | anulado.
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(self, hacia: EstadoAcuerdo) -> Result<Self, DominioError> {
        use EstadoAcuerdo::{Anulado, Cumplido, Incumplido, Propuesto, Vigente};
        if matches!(
            (self, hacia),
            (Propuesto, Vigente | Anulado) | (Vigente, Cumplido | Incumplido | Anulado)
        ) {
            Ok(hacia)
        } else {
            Err(DominioError::TransicionInvalida {
                entidad: "acuerdo de pago",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            })
        }
    }

    /// ¿Ocupa el lugar del acuerdo activo del bien?
    pub fn activo(self) -> bool {
        matches!(self, EstadoAcuerdo::Propuesto | EstadoAcuerdo::Vigente)
    }
}

/// Clase de certificado de cartera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoCertificado {
    /// El bien no debe nada al corte.
    PazYSalvo,
    /// Lo que debe el bien al corte (la certificación del administrador es título ejecutivo,
    /// Ley 675 art. 48).
    Deuda,
}

enum_texto!(TipoCertificado {
    PazYSalvo => "paz_y_salvo",
    Deuda => "deuda",
});

fn invalido(campo: &'static str, regla: &'static str) -> DominioError {
    DominioError::DatoInvalido { campo, regla }
}

/// Factor máximo sobre el interés bancario corriente (Ley 675 art. 30; coincide con el tope de
/// usura del Código de Comercio art. 884).
pub fn factor_maximo() -> Decimal {
    Decimal::new(15, 1)
}

/// Tasa de mora efectiva anual (%) a partir del interés bancario corriente (%): el factor de la
/// política (a lo sumo 1,5) o la tasa mensual inferior aprobada por la asamblea si da menos.
pub fn tasa_mora_ea(
    interes_bancario_ea: Decimal,
    factor: Decimal,
    tasa_inferior_mensual: Option<Decimal>,
) -> Decimal {
    let tope = interes_bancario_ea * factor.min(factor_maximo());
    let inferior = tasa_inferior_mensual.map(|m| {
        ((Decimal::ONE + m / Decimal::ONE_HUNDRED).powi(12) - Decimal::ONE) * Decimal::ONE_HUNDRED
    });
    inferior.map_or(tope, |i| i.min(tope)).round_dp(4)
}

/// Tasa de mora de un tramo de días, con la tasa de referencia de donde sale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TramoTasa {
    pub desde: NaiveDate,
    /// Inclusive.
    pub hasta: NaiveDate,
    /// Efectiva anual, en %.
    pub tasa_ea: Decimal,
    pub referencia: Uuid,
}

/// Reducción del capital en una fecha (pago aplicado o ajuste): desde ese día no causa interés.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reduccion {
    pub fecha: NaiveDate,
    pub valor: Decimal,
}

/// Lo que causa una obligación entre dos fechas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalculoInteres<'a> {
    pub desde: NaiveDate,
    /// Inclusive.
    pub hasta: NaiveDate,
    /// Capital pendiente el día anterior a `desde`.
    pub saldo_inicial: Decimal,
    pub reducciones: &'a [Reduccion],
    pub tramos: &'a [TramoTasa],
    /// Días sin mora (acuerdo de pago vigente), inclusive.
    pub suspensiones: &'a [(NaiveDate, NaiveDate)],
}

/// Tramo de días con la misma base y la misma tasa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentoInteres {
    pub desde: NaiveDate,
    /// Inclusive.
    pub hasta: NaiveDate,
    pub dias: u32,
    pub base: Decimal,
    pub tasa_ea: Decimal,
    pub referencia: Uuid,
    /// `base × ((1 + tasa)^(días / 365) − 1)` con dos decimales.
    pub valor: Decimal,
}

fn valor_segmento(base: Decimal, tasa_ea: Decimal, dias: u32) -> Decimal {
    let exponente = Decimal::from(dias) / Decimal::from(365);
    let factor = (Decimal::ONE + tasa_ea / Decimal::ONE_HUNDRED).powd(exponente) - Decimal::ONE;
    let mut v = (base * factor).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    v.rescale(2);
    v
}

/// Interés de mora día a día: cada día con capital pendiente (después de las reducciones de ese
/// día o antes) y sin suspensión causa a la tasa del tramo que lo contiene; los días seguidos con
/// la misma base y tasa forman un segmento.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si `hasta` es anterior a `desde`;
/// [`DominioError::ReglaContable`] si un día con capital no tiene tasa de referencia.
pub fn intereses(c: &CalculoInteres<'_>) -> Result<Vec<SegmentoInteres>, DominioError> {
    if c.hasta < c.desde {
        return Err(invalido("hasta", "no puede ser anterior al inicio"));
    }
    let mut segmentos: Vec<SegmentoInteres> = Vec::new();
    let mut dia = c.desde;
    loop {
        let reducido: Decimal = c
            .reducciones
            .iter()
            .filter(|r| r.fecha <= dia)
            .map(|r| r.valor)
            .sum();
        let base = c.saldo_inicial - reducido;
        let suspendido = c.suspensiones.iter().any(|(d, h)| *d <= dia && dia <= *h);
        if base > Decimal::ZERO && !suspendido {
            let tramo = c
                .tramos
                .iter()
                .find(|t| t.desde <= dia && dia <= t.hasta)
                .ok_or_else(|| {
                    DominioError::ReglaContable(format!(
                        "no hay tasa de referencia para el {dia}: la plataforma debe cargarla"
                    ))
                })?;
            match segmentos.last_mut() {
                Some(s)
                    if s.base == base
                        && s.referencia == tramo.referencia
                        && s.hasta.succ_opt() == Some(dia) =>
                {
                    s.hasta = dia;
                    s.dias += 1;
                }
                _ => segmentos.push(SegmentoInteres {
                    desde: dia,
                    hasta: dia,
                    dias: 1,
                    base,
                    tasa_ea: tramo.tasa_ea,
                    referencia: tramo.referencia,
                    valor: Decimal::ZERO,
                }),
            }
        }
        if dia >= c.hasta {
            break;
        }
        dia = dia
            .succ_opt()
            .ok_or_else(|| invalido("hasta", "fecha fuera de rango"))?;
    }
    for s in &mut segmentos {
        s.valor = valor_segmento(s.base, s.tasa_ea, s.dias);
    }
    Ok(segmentos)
}

/// Cuota de un acuerdo de pago.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuotaAcuerdo {
    pub numero: u16,
    pub vencimiento: NaiveDate,
    pub valor: Decimal,
}

/// Cuotas mensuales iguales (la diferencia del redondeo al peso va a las primeras) desde el
/// primer vencimiento.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si el número de cuotas no está entre 1 y 60 o el valor no es
/// positivo.
pub fn cuotas_acuerdo(
    valor: Decimal,
    numero: u16,
    primer_vencimiento: NaiveDate,
) -> Result<Vec<CuotaAcuerdo>, DominioError> {
    if !(1..=60).contains(&numero) {
        return Err(invalido("numero_cuotas", "entre 1 y 60"));
    }
    if valor <= Decimal::ZERO {
        return Err(invalido("valor", "mayor que cero"));
    }
    let pesos: Vec<(u16, Decimal)> = (1..=numero).map(|n| (n, Decimal::ONE)).collect();
    repartir(valor, &pesos, Decimal::ONE)?
        .into_iter()
        .map(|(n, v)| {
            let vencimiento = primer_vencimiento
                .checked_add_months(Months::new(u32::from(n) - 1))
                .ok_or_else(|| invalido("primer_vencimiento", "fecha fuera de rango"))?;
            let mut valor = v;
            valor.rescale(2);
            Ok(CuotaAcuerdo {
                numero: n,
                vencimiento,
                valor,
            })
        })
        .collect()
}

/// Cómo va un acuerdo de pago en una fecha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SituacionAcuerdo {
    AlDia,
    /// Lo pagado cubre todas las cuotas.
    Cumplido,
    /// Una cuota venció sin cubrirse: la mora se reanuda desde el día siguiente.
    Incumplido {
        desde: NaiveDate,
    },
}

/// Situación del acuerdo en `hoy` según lo pagado desde su fecha (pagos con su fecha): una cuota
/// está cubierta si lo pagado hasta su vencimiento alcanza la suma de las cuotas hasta ella.
pub fn situacion_acuerdo(
    cuotas: &[CuotaAcuerdo],
    pagos: &[(NaiveDate, Decimal)],
    hoy: NaiveDate,
) -> SituacionAcuerdo {
    let total_pagado: Decimal = pagos.iter().map(|p| p.1).sum();
    let total_cuotas: Decimal = cuotas.iter().map(|c| c.valor).sum();
    if total_pagado >= total_cuotas {
        return SituacionAcuerdo::Cumplido;
    }
    let mut acumulado = Decimal::ZERO;
    let mut ordenadas: Vec<&CuotaAcuerdo> = cuotas.iter().collect();
    ordenadas.sort_by_key(|c| c.vencimiento);
    for c in ordenadas {
        acumulado += c.valor;
        if c.vencimiento >= hoy {
            break;
        }
        let pagado: Decimal = pagos
            .iter()
            .filter(|p| p.0 <= c.vencimiento)
            .map(|p| p.1)
            .sum();
        if pagado < acumulado {
            return SituacionAcuerdo::Incumplido {
                desde: c.vencimiento.succ_opt().unwrap_or(c.vencimiento),
            };
        }
    }
    SituacionAcuerdo::AlDia
}

/// Código de verificación legible (`ABCD-EF01-2345-6789-ABCD`, 80 bits) a partir de un token
/// aleatorio en hexadecimal.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si el token no tiene al menos 20 caracteres hexadecimales.
pub fn codigo_verificacion(token_hex: &str) -> Result<String, DominioError> {
    let limpio: String = token_hex
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(20)
        .collect::<String>()
        .to_ascii_uppercase();
    if limpio.len() < 20 {
        return Err(invalido("codigo", "al menos 20 dígitos hexadecimales"));
    }
    let grupos: Vec<&str> = (0..5).map(|i| &limpio[i * 4..i * 4 + 4]).collect();
    Ok(grupos.join("-"))
}

/// Código como lo escribió una persona (sin guiones ni espacios, en mayúsculas), para
/// compararlo por su huella.
pub fn normalizar_codigo(codigo: &str) -> String {
    codigo
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, d).expect("fecha")
    }

    fn tramo(desde: NaiveDate, hasta: NaiveDate, tasa: Decimal, n: u128) -> TramoTasa {
        TramoTasa {
            desde,
            hasta,
            tasa_ea: tasa,
            referencia: Uuid::from_u128(n),
        }
    }

    #[test]
    fn la_tasa_de_mora_no_supera_una_vez_y_media_el_interes_bancario() {
        assert_eq!(tasa_mora_ea(dec!(20), dec!(1.5), None), dec!(30));
        assert_eq!(tasa_mora_ea(dec!(20), dec!(2), None), dec!(30));
        assert_eq!(
            tasa_mora_ea(dec!(20), dec!(1.5), Some(dec!(1))),
            dec!(12.6825)
        );
        assert_eq!(tasa_mora_ea(dec!(20), dec!(1.5), Some(dec!(5))), dec!(30));
    }

    #[test]
    fn un_anio_completo_causa_la_tasa_efectiva_anual() {
        let tramos = [tramo(fecha(2025, 1, 1), fecha(2025, 12, 31), dec!(30), 1)];
        let s = intereses(&CalculoInteres {
            desde: fecha(2025, 1, 1),
            hasta: fecha(2025, 12, 31),
            saldo_inicial: dec!(100000),
            reducciones: &[],
            tramos: &tramos,
            suspensiones: &[],
        })
        .expect("intereses");

        assert_eq!(s.len(), 1);
        assert_eq!(s[0].dias, 365);
        assert_eq!(s[0].valor, dec!(30000.00));
    }

    #[test]
    fn el_pago_y_el_cambio_de_tasa_parten_los_tramos() {
        let tramos = [
            tramo(fecha(2026, 10, 1), fecha(2026, 10, 31), dec!(30), 1),
            tramo(fecha(2026, 11, 1), fecha(2026, 11, 30), dec!(27), 2),
        ];
        let reducciones = [Reduccion {
            fecha: fecha(2026, 10, 21),
            valor: dec!(40000),
        }];
        let s = intereses(&CalculoInteres {
            desde: fecha(2026, 10, 11),
            hasta: fecha(2026, 11, 10),
            saldo_inicial: dec!(100000),
            reducciones: &reducciones,
            tramos: &tramos,
            suspensiones: &[],
        })
        .expect("intereses");

        let resumen: Vec<(u32, Decimal, Decimal)> =
            s.iter().map(|x| (x.dias, x.base, x.tasa_ea)).collect();
        assert_eq!(
            resumen,
            vec![
                (10, dec!(100000), dec!(30)),
                (11, dec!(60000), dec!(30)),
                (10, dec!(60000), dec!(27))
            ]
        );
        assert!(s.iter().all(|x| x.valor > Decimal::ZERO));
    }

    #[test]
    fn pagado_todo_o_suspendido_no_causa() {
        let tramos = [tramo(fecha(2026, 1, 1), fecha(2026, 12, 31), dec!(30), 1)];
        let pagado = [Reduccion {
            fecha: fecha(2026, 3, 1),
            valor: dec!(100000),
        }];
        let s = intereses(&CalculoInteres {
            desde: fecha(2026, 3, 1),
            hasta: fecha(2026, 3, 31),
            saldo_inicial: dec!(100000),
            reducciones: &pagado,
            tramos: &tramos,
            suspensiones: &[],
        })
        .expect("intereses");
        assert!(s.is_empty());

        let suspendido = intereses(&CalculoInteres {
            desde: fecha(2026, 3, 1),
            hasta: fecha(2026, 3, 31),
            saldo_inicial: dec!(100000),
            reducciones: &[],
            tramos: &tramos,
            suspensiones: &[(fecha(2026, 3, 1), fecha(2026, 3, 20))],
        })
        .expect("intereses");
        assert_eq!(suspendido.len(), 1);
        assert_eq!(suspendido[0].dias, 11);
    }

    #[test]
    fn sin_tasa_de_referencia_no_se_causa() {
        let r = intereses(&CalculoInteres {
            desde: fecha(2026, 3, 1),
            hasta: fecha(2026, 3, 31),
            saldo_inicial: dec!(1),
            reducciones: &[],
            tramos: &[],
            suspensiones: &[],
        });

        assert!(matches!(r, Err(DominioError::ReglaContable(_))));
    }

    #[test]
    fn las_cuotas_del_acuerdo_cuadran_y_son_mensuales() {
        let c = cuotas_acuerdo(dec!(1000), 3, fecha(2026, 12, 15)).expect("cuotas");

        assert_eq!(
            c.iter().map(|x| x.valor).collect::<Vec<_>>(),
            vec![dec!(334.00), dec!(333.00), dec!(333.00)]
        );
        assert_eq!(c[2].vencimiento, fecha(2027, 2, 15));
        assert!(cuotas_acuerdo(dec!(1000), 0, fecha(2026, 12, 15)).is_err());
    }

    #[test]
    fn el_acuerdo_se_incumple_si_una_cuota_vence_sin_pago() {
        let c = cuotas_acuerdo(dec!(300), 3, fecha(2026, 1, 10)).expect("cuotas");

        assert_eq!(
            situacion_acuerdo(&c, &[(fecha(2026, 1, 9), dec!(100))], fecha(2026, 2, 5)),
            SituacionAcuerdo::AlDia
        );
        assert_eq!(
            situacion_acuerdo(&c, &[(fecha(2026, 1, 9), dec!(100))], fecha(2026, 2, 11)),
            SituacionAcuerdo::Incumplido {
                desde: fecha(2026, 2, 11)
            }
        );
        assert_eq!(
            situacion_acuerdo(&c, &[(fecha(2026, 1, 9), dec!(300))], fecha(2026, 2, 11)),
            SituacionAcuerdo::Cumplido
        );
    }

    #[test]
    fn transiciones_del_acuerdo() {
        assert!(
            EstadoAcuerdo::Propuesto
                .transicionar(EstadoAcuerdo::Vigente)
                .is_ok()
        );
        assert!(
            EstadoAcuerdo::Cumplido
                .transicionar(EstadoAcuerdo::Vigente)
                .is_err()
        );
        assert!(EstadoAcuerdo::Vigente.activo());
        assert!(!EstadoAcuerdo::Incumplido.activo());
    }

    #[test]
    fn el_codigo_de_verificacion_se_lee_y_se_normaliza() {
        let c = codigo_verificacion("0123456789abcdef0123456789abcdef").expect("código");

        assert_eq!(c, "0123-4567-89AB-CDEF-0123");
        assert_eq!(
            normalizar_codigo(" 0123 4567-89ab-cdef-0123 "),
            "0123456789ABCDEF0123"
        );
        assert!(codigo_verificacion("abc").is_err());
    }
}
