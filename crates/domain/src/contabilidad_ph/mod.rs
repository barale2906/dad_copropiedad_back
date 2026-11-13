//! Vertical contable de la propiedad horizontal (plan 14, ADR 0019): conceptos de cobro, política
//! de cartera, liquidación de expensas con un redondeo que cuadra al peso, imputación de pagos y
//! descuento por pronto pago.
//!
//! Lógica pura. Usa el núcleo contable ([`Monto`]) y los tipos de la PH; el núcleo nunca usa
//! este módulo (ADR 0017).

pub mod mora;

use std::collections::{BTreeMap, BTreeSet};

use chrono::{Datelike, NaiveDate};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

use crate::{
    BienPrivadoId, ConceptoCobroId, CuotaExtraordinariaId, DominioError, ModuloContribucionId,
    ObligacionCobroId, TipoBienPrivado, contabilidad::Monto,
};

/// Clase de concepto de cobro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoConceptoCobro {
    /// Expensa ordinaria: se liquida cada mes con el presupuesto (o su valor fijo).
    Ordinaria,
    /// Fondo de imprevistos (art. 35): porcentaje de lo ordinario, en línea aparte.
    FondoImprevistos,
    /// Cuota extraordinaria aprobada por la asamblea (según su calendario).
    Extraordinaria,
    /// Uso de un bien común (salón, parqueadero de visitantes).
    UsoBienComun,
    Multa,
    /// Intereses de mora (art. 30).
    InteresMora,
    /// Daños (p. ej. los de una reserva).
    Dano,
    Otro,
}

enum_texto!(TipoConceptoCobro {
    Ordinaria => "ordinaria",
    FondoImprevistos => "fondo_imprevistos",
    Extraordinaria => "extraordinaria",
    UsoBienComun => "uso_bien_comun",
    Multa => "multa",
    InteresMora => "interes_mora",
    Dano => "dano",
    Otro => "otro",
});

impl TipoConceptoCobro {
    /// ¿Se cobra en un cobro manual? Lo ordinario, el fondo y las cuotas extraordinarias solo
    /// salen de la liquidación.
    pub fn admite_cobro_manual(self) -> bool {
        !matches!(
            self,
            TipoConceptoCobro::Ordinaria
                | TipoConceptoCobro::FondoImprevistos
                | TipoConceptoCobro::Extraordinaria
        )
    }
}

/// Cómo se reparte un concepto entre los bienes privados.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterioDistribucion {
    /// Coeficiente de copropiedad del cuadro vigente.
    Coeficiente,
    /// Coeficiente del cuadro del módulo de contribución.
    Modulo,
    /// El mismo valor mensual para cada bien.
    ValorFijo,
    /// Valor mensual según el tipo de bien.
    PorTipoBien,
    /// Proporcional al área privada.
    Area,
}

enum_texto!(CriterioDistribucion {
    Coeficiente => "coeficiente",
    Modulo => "modulo",
    ValorFijo => "valor_fijo",
    PorTipoBien => "por_tipo_bien",
    Area => "area",
});

impl CriterioDistribucion {
    /// ¿Reparte un total (presupuesto o cuota) en proporción a un peso? Los demás fijan el valor
    /// de cada bien.
    pub fn reparte_total(self) -> bool {
        matches!(
            self,
            CriterioDistribucion::Coeficiente
                | CriterioDistribucion::Modulo
                | CriterioDistribucion::Area
        )
    }
}

/// Orden en que un pago cubre las obligaciones (Código Civil art. 1653: intereses antes que
/// capital, salvo que el acreedor consienta otra cosa).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrdenImputacion {
    /// Todos los intereses (de la más antigua a la más reciente) y luego el capital.
    InteresesPrimero,
    /// Periodo por periodo desde el más antiguo; en cada uno, intereses antes que capital.
    PorAntiguedad,
}

enum_texto!(OrdenImputacion {
    InteresesPrimero => "intereses_primero",
    PorAntiguedad => "por_antiguedad",
});

fn transicion<E: Copy>(
    entidad: &'static str,
    desde: E,
    hacia: E,
    texto: fn(E) -> &'static str,
    permitida: bool,
) -> Result<E, DominioError> {
    if permitida {
        Ok(hacia)
    } else {
        Err(DominioError::TransicionInvalida {
            entidad,
            desde: texto(desde),
            hacia: texto(hacia),
        })
    }
}

/// Estado de un presupuesto: borrador → vigente (aprobado por la asamblea) → reemplazado (por
/// otra versión del mismo año).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoPresupuesto {
    Borrador,
    Vigente,
    Reemplazado,
}

enum_texto!(EstadoPresupuesto {
    Borrador => "borrador",
    Vigente => "vigente",
    Reemplazado => "reemplazado",
});

impl EstadoPresupuesto {
    /// borrador → vigente → reemplazado.
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(self, hacia: EstadoPresupuesto) -> Result<Self, DominioError> {
        use EstadoPresupuesto::{Borrador, Reemplazado, Vigente};
        let ok = matches!((self, hacia), (Borrador, Vigente) | (Vigente, Reemplazado));
        transicion("presupuesto", self, hacia, Self::as_str, ok)
    }
}

/// Estado de una liquidación de expensas: se contabiliza al liquidarse; anular = reverso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoLiquidacionExpensas {
    Contabilizada,
    Anulada,
}

enum_texto!(EstadoLiquidacionExpensas {
    Contabilizada => "contabilizada",
    Anulada => "anulada",
});

/// De dónde viene una obligación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrigenObligacion {
    /// Liquidación mensual (ordinaria, fondo y cuotas extraordinarias).
    Liquidacion,
    /// Cargo pendiente de una reserva (daños, penalidades) que entró en una liquidación.
    Cargo,
    /// Cartera inicial (su contabilidad está en el comprobante de saldos iniciales).
    SaldoInicial,
    /// Cobro manual (multas, uso de bienes comunes) con su propio comprobante.
    Manual,
    /// Intereses de mora de una causación (nunca generan intereses).
    Interes,
}

enum_texto!(OrigenObligacion {
    Liquidacion => "liquidacion",
    Cargo => "cargo",
    SaldoInicial => "saldo_inicial",
    Manual => "manual",
    Interes => "interes",
});

/// Estado de una obligación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoObligacion {
    Vigente,
    Anulada,
}

enum_texto!(EstadoObligacion {
    Vigente => "vigente",
    Anulada => "anulada",
});

/// Clase de ajuste que reduce una obligación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoAjuste {
    /// Corrección o condonación con motivo (documento correctivo).
    NotaCredito,
    /// Descuento por pronto pago ganado por un recibo.
    ProntoPago,
}

enum_texto!(TipoAjuste {
    NotaCredito => "nota_credito",
    ProntoPago => "pronto_pago",
});

/// Clase de aviso de la cartera (uno por cuenta de cobro y clase).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoAvisoCartera {
    /// Antes del vencimiento.
    Recordatorio,
    /// Después del vencimiento, si sigue debiendo.
    Mora,
}

enum_texto!(TipoAvisoCartera {
    Recordatorio => "recordatorio",
    Mora => "mora",
});

/// Estado de una cuota extraordinaria.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoCuotaExtraordinaria {
    Vigente,
    Anulada,
}

enum_texto!(EstadoCuotaExtraordinaria {
    Vigente => "vigente",
    Anulada => "anulada",
});

fn invalido(campo: &'static str, regla: &'static str) -> DominioError {
    DominioError::DatoInvalido { campo, regla }
}

// ---------------------------------------------------------------------------------------------
// Periodos y política de cartera
// ---------------------------------------------------------------------------------------------

/// Primer día del mes de la fecha: así se identifica un periodo de liquidación.
pub fn periodo_de(fecha: NaiveDate) -> NaiveDate {
    fecha.with_day(1).unwrap_or(fecha)
}

/// Meses desde `desde` hasta `hasta` (negativo si `hasta` es anterior).
pub fn meses_entre(desde: NaiveDate, hasta: NaiveDate) -> i32 {
    (hasta.year() - desde.year()) * 12 + hasta.month0() as i32 - desde.month0() as i32
}

/// El día `dia` (1 a 28) del periodo.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si el día no está entre 1 y 28.
pub fn dia_del_periodo(periodo: NaiveDate, dia: u8) -> Result<NaiveDate, DominioError> {
    if !(1..=28).contains(&dia) {
        return Err(invalido("dia", "entre 1 y 28"));
    }
    periodo_de(periodo)
        .with_day(u32::from(dia))
        .ok_or_else(|| invalido("dia", "entre 1 y 28"))
}

/// Parámetros de una versión de la política de cartera (las variables las elige el contador o
/// la asamblea; aquí solo se validan los topes legales).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParametrosPolitica {
    /// Día del mes en que vence la cuenta de cobro (1 a 28).
    pub dia_vencimiento: u8,
    /// Múltiplo al que se redondea cada cobro (1 = al peso).
    pub redondeo: Decimal,
    /// Descuento por pronto pago sobre lo ordinario del periodo: porcentaje y día límite.
    pub pronto_pago: Option<(Decimal, u8)>,
    /// Veces el interés bancario corriente (Ley 675 art. 30: a lo sumo 1,5).
    pub factor_mora: Decimal,
    /// Tasa mensual inferior aprobada por la asamblea (%), si la hay.
    pub tasa_mora_mensual: Option<Decimal>,
    pub imputacion: OrdenImputacion,
    /// Consentimiento del acreedor para que el pagador elija qué obligación cubre.
    pub permite_elegir_obligacion: bool,
    /// Porcentaje del fondo de imprevistos sobre lo ordinario.
    pub porcentaje_fondo: Decimal,
    /// La asamblea suspendió el fondo (art. 35, al llegar al 50 % del presupuesto).
    pub fondo_suspendido: bool,
}

impl ParametrosPolitica {
    /// Topes legales y rangos.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] con la regla incumplida.
    pub fn validar(&self) -> Result<(), DominioError> {
        if !(1..=28).contains(&self.dia_vencimiento) {
            return Err(invalido("dia_vencimiento", "entre 1 y 28"));
        }
        if self.redondeo < Decimal::ONE || self.redondeo.normalize().scale() > 0 {
            return Err(invalido(
                "redondeo",
                "un múltiplo entero de al menos 1 peso",
            ));
        }
        if let Some((porcentaje, dia)) = self.pronto_pago {
            if porcentaje <= Decimal::ZERO || porcentaje > Decimal::ONE_HUNDRED {
                return Err(invalido(
                    "pronto_pago_porcentaje",
                    "mayor que 0 y hasta 100",
                ));
            }
            if !(1..=28).contains(&dia) || dia > self.dia_vencimiento {
                return Err(invalido(
                    "pronto_pago_dia",
                    "entre 1 y 28 y no posterior al vencimiento",
                ));
            }
        }
        if self.factor_mora <= Decimal::ZERO || self.factor_mora > Decimal::new(15, 1) {
            return Err(invalido(
                "factor_mora",
                "mayor que 0 y a lo sumo 1,5 (Ley 675, art. 30)",
            ));
        }
        if self.tasa_mora_mensual.is_some_and(|t| t <= Decimal::ZERO) {
            return Err(invalido("tasa_mora_mensual", "mayor que 0"));
        }
        if self.porcentaje_fondo < Decimal::ZERO || self.porcentaje_fondo > Decimal::ONE_HUNDRED {
            return Err(invalido("porcentaje_fondo", "entre 0 y 100"));
        }
        if !self.fondo_suspendido && self.porcentaje_fondo < Decimal::ONE {
            return Err(invalido(
                "porcentaje_fondo",
                "al menos 1 % salvo que la asamblea lo suspenda (Ley 675, art. 35)",
            ));
        }
        Ok(())
    }

    /// Porcentaje del fondo que se liquida (`None` si está suspendido o es cero).
    pub fn fondo_a_liquidar(&self) -> Option<Decimal> {
        (!self.fondo_suspendido && self.porcentaje_fondo > Decimal::ZERO)
            .then_some(self.porcentaje_fondo)
    }
}

/// ¿El fondo acumulado llegó a la mitad del presupuesto ordinario anual? (art. 35: la asamblea
/// puede suspender los aportes; se avisa, no se suspende solo).
pub fn fondo_llego_a_la_mitad(acumulado: Decimal, presupuesto_anual: Decimal) -> bool {
    presupuesto_anual > Decimal::ZERO && acumulado * Decimal::TWO >= presupuesto_anual
}

/// Valores de enero a diciembre de un rubro del presupuesto.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si no son doce.
pub fn valores_mensuales(valores: &[Monto]) -> Result<[Monto; 12], DominioError> {
    <[Monto; 12]>::try_from(valores).map_err(|_| invalido("valores_mensuales", "doce valores"))
}

// ---------------------------------------------------------------------------------------------
// Reparto con redondeo que cuadra
// ---------------------------------------------------------------------------------------------

/// Reparte `total` en proporción a los pesos, en múltiplos de `multiplo`, de modo que la suma
/// sea exactamente `total` (método del mayor residuo): cada parte se redondea hacia abajo al
/// múltiplo y lo que falta se entrega, de a un múltiplo, a las de mayor residuo (a igual
/// residuo, a la de menor clave); el último pedazo menor que el múltiplo va a la siguiente.
///
/// Los pesos en cero reciben cero. El resultado conserva el orden de `pesos`.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si el total es negativo, el múltiplo no es positivo o no hay
/// pesos positivos.
pub fn repartir<K: Copy + Ord>(
    total: Decimal,
    pesos: &[(K, Decimal)],
    multiplo: Decimal,
) -> Result<Vec<(K, Decimal)>, DominioError> {
    if total < Decimal::ZERO {
        return Err(invalido("total", "no puede ser negativo"));
    }
    if multiplo <= Decimal::ZERO {
        return Err(invalido("redondeo", "mayor que cero"));
    }
    if pesos.iter().any(|(_, p)| *p < Decimal::ZERO) {
        return Err(invalido("peso", "no puede ser negativo"));
    }
    let suma: Decimal = pesos.iter().map(|(_, p)| *p).sum();
    if suma <= Decimal::ZERO {
        return Err(invalido("bienes", "no hay a quién repartir"));
    }
    let mut partes: Vec<(K, Decimal, Decimal)> = pesos
        .iter()
        .map(|(k, p)| {
            let exacto = total * *p / suma;
            let base = (exacto / multiplo).floor() * multiplo;
            (*k, base, exacto - base)
        })
        .collect();
    let mut resto = total - partes.iter().map(|(_, b, _)| *b).sum::<Decimal>();
    let mut orden: Vec<usize> = (0..partes.len())
        .filter(|i| pesos[*i].1 > Decimal::ZERO)
        .collect();
    orden.sort_by(|a, b| {
        partes[*b]
            .2
            .cmp(&partes[*a].2)
            .then(partes[*a].0.cmp(&partes[*b].0))
    });
    for i in orden.iter().cycle().take(orden.len() * 2) {
        if resto <= Decimal::ZERO {
            break;
        }
        let entrega = resto.min(multiplo);
        partes[*i].1 += entrega;
        resto -= entrega;
    }
    Ok(partes
        .into_iter()
        .map(|(k, v, _)| (k, v.round_dp(2)))
        .collect())
}

/// Valor de la cuota extraordinaria en el periodo: el total en `numero_cuotas` partes que
/// cuadran (la diferencia del redondeo va a las primeras); `None` fuera del calendario.
pub fn cuota_del_periodo(
    valor_total: Decimal,
    numero_cuotas: u16,
    primer_periodo: NaiveDate,
    periodo: NaiveDate,
    multiplo: Decimal,
) -> Option<Decimal> {
    let indice = meses_entre(primer_periodo, periodo);
    let n = i32::from(numero_cuotas);
    if numero_cuotas == 0 || !(0..n).contains(&indice) {
        return None;
    }
    let pesos: Vec<(i32, Decimal)> = (0..n).map(|i| (i, Decimal::ONE)).collect();
    repartir(valor_total, &pesos, multiplo)
        .ok()?
        .into_iter()
        .find(|(i, _)| *i == indice)
        .map(|(_, v)| v)
}

// ---------------------------------------------------------------------------------------------
// Liquidación de expensas
// ---------------------------------------------------------------------------------------------

/// Bien privado activo que se liquida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BienLiquidable {
    pub id: BienPrivadoId,
    pub tipo: TipoBienPrivado,
    pub area_m2: Option<Decimal>,
}

/// Concepto de cobro activo con lo necesario para repartirlo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptoLiquidable {
    pub id: ConceptoCobroId,
    pub codigo: String,
    pub tipo: TipoConceptoCobro,
    pub criterio: CriterioDistribucion,
    pub modulo: Option<ModuloContribucionId>,
    pub valor_fijo: Option<Decimal>,
    /// Valor mensual por tipo de bien (criterio [`CriterioDistribucion::PorTipoBien`]).
    pub tarifas: Vec<(TipoBienPrivado, Decimal)>,
}

/// Cuota extraordinaria que cae en el periodo, con su valor ya calculado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuotaDelPeriodo {
    pub id: CuotaExtraordinariaId,
    pub concepto: ConceptoCobroId,
    pub valor: Decimal,
}

/// Lo que necesita la liquidación de un periodo.
#[derive(Debug, Clone, Default)]
pub struct DatosLiquidacion {
    pub bienes: Vec<BienLiquidable>,
    pub conceptos: Vec<ConceptoLiquidable>,
    /// Total del mes por concepto ordinario, según el presupuesto vigente.
    pub presupuesto_mes: BTreeMap<ConceptoCobroId, Decimal>,
    /// Cuadro de la copropiedad vigente en el periodo.
    pub coeficientes: BTreeMap<BienPrivadoId, Decimal>,
    /// Cuadro de cada módulo de contribución vigente en el periodo.
    pub coeficientes_modulo: BTreeMap<ModuloContribucionId, BTreeMap<BienPrivadoId, Decimal>>,
    pub cuotas: Vec<CuotaDelPeriodo>,
    /// Múltiplo del redondeo (política).
    pub redondeo: Decimal,
    /// Porcentaje del fondo de imprevistos (`None` = suspendido).
    pub porcentaje_fondo: Option<Decimal>,
}

/// Cobro de un concepto a un bien en el periodo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cobro {
    pub bien: BienPrivadoId,
    pub concepto: ConceptoCobroId,
    pub cuota: Option<CuotaExtraordinariaId>,
    pub valor: Decimal,
}

fn regla_liquidacion(texto: String) -> DominioError {
    DominioError::ReglaContable(texto)
}

impl DatosLiquidacion {
    fn concepto(&self, id: ConceptoCobroId) -> Result<&ConceptoLiquidable, DominioError> {
        self.conceptos
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| regla_liquidacion("concepto de cobro inactivo o inexistente".into()))
    }

    /// Pesos de los bienes activos según el criterio (los bienes fuera del cuadro no pesan).
    fn pesos(&self, c: &ConceptoLiquidable) -> Result<Vec<(BienPrivadoId, Decimal)>, DominioError> {
        let activos: BTreeSet<BienPrivadoId> = self.bienes.iter().map(|b| b.id).collect();
        let de_cuadro = |cuadro: &BTreeMap<BienPrivadoId, Decimal>| {
            cuadro
                .iter()
                .filter(|(b, _)| activos.contains(b))
                .map(|(b, p)| (*b, *p))
                .collect::<Vec<_>>()
        };
        let pesos = match c.criterio {
            CriterioDistribucion::Coeficiente => de_cuadro(&self.coeficientes),
            CriterioDistribucion::Modulo => {
                let cuadro = c
                    .modulo
                    .and_then(|m| self.coeficientes_modulo.get(&m))
                    .ok_or_else(|| {
                        regla_liquidacion(format!(
                            "el módulo del concepto `{}` no tiene cuadro vigente",
                            c.codigo
                        ))
                    })?;
                de_cuadro(cuadro)
            }
            CriterioDistribucion::Area => {
                let sin_area = self.bienes.iter().filter(|b| b.area_m2.is_none()).count();
                if sin_area > 0 {
                    return Err(regla_liquidacion(format!(
                        "el concepto `{}` se reparte por área y {sin_area} bienes no la tienen",
                        c.codigo
                    )));
                }
                self.bienes
                    .iter()
                    .map(|b| (b.id, b.area_m2.unwrap_or_default()))
                    .collect()
            }
            CriterioDistribucion::ValorFijo | CriterioDistribucion::PorTipoBien => {
                return Err(regla_liquidacion(format!(
                    "el concepto `{}` no reparte un total",
                    c.codigo
                )));
            }
        };
        if pesos.is_empty() {
            return Err(regla_liquidacion(format!(
                "no hay bienes con coeficiente para el concepto `{}`",
                c.codigo
            )));
        }
        Ok(pesos)
    }

    fn repartir_concepto(
        &self,
        c: &ConceptoLiquidable,
        total: Decimal,
    ) -> Result<Vec<(BienPrivadoId, Decimal)>, DominioError> {
        repartir(total, &self.pesos(c)?, self.redondeo)
    }

    /// Valor de un concepto ordinario por bien.
    fn ordinaria(
        &self,
        c: &ConceptoLiquidable,
    ) -> Result<Vec<(BienPrivadoId, Decimal)>, DominioError> {
        match c.criterio {
            CriterioDistribucion::ValorFijo => {
                let valor = c.valor_fijo.unwrap_or_default();
                Ok(self.bienes.iter().map(|b| (b.id, valor)).collect())
            }
            CriterioDistribucion::PorTipoBien => Ok(self
                .bienes
                .iter()
                .filter_map(|b| {
                    c.tarifas
                        .iter()
                        .find(|(t, _)| *t == b.tipo)
                        .map(|(_, v)| (b.id, *v))
                })
                .collect()),
            _ => match self.presupuesto_mes.get(&c.id) {
                Some(total) if *total > Decimal::ZERO => self.repartir_concepto(c, *total),
                _ => Ok(Vec::new()),
            },
        }
    }
}

/// Liquida el periodo: lo ordinario por concepto, el fondo de imprevistos como porcentaje de lo
/// ordinario de cada bien (en línea aparte) y las cuotas extraordinarias del calendario. Cada
/// total se reparte con [`repartir`], así que la suma por concepto cuadra al peso.
///
/// # Errors
///
/// [`DominioError::ReglaContable`] si un concepto no se puede repartir (módulo sin cuadro,
/// bienes sin área, cuota con un criterio que no reparte); [`DominioError::DatoInvalido`] de
/// [`repartir`].
pub fn liquidar(d: &DatosLiquidacion) -> Result<Vec<Cobro>, DominioError> {
    let mut cobros = Vec::new();
    let mut ordinario_por_bien: BTreeMap<BienPrivadoId, Decimal> = BTreeMap::new();
    for c in d
        .conceptos
        .iter()
        .filter(|c| c.tipo == TipoConceptoCobro::Ordinaria)
    {
        for (bien, valor) in d.ordinaria(c)? {
            *ordinario_por_bien.entry(bien).or_default() += valor;
            cobros.push(Cobro {
                bien,
                concepto: c.id,
                cuota: None,
                valor,
            });
        }
    }
    if let Some(porcentaje) = d.porcentaje_fondo {
        let base: Vec<(BienPrivadoId, Decimal)> =
            ordinario_por_bien.iter().map(|(b, v)| (*b, *v)).collect();
        let total_base: Decimal = base.iter().map(|(_, v)| *v).sum();
        let total = (total_base * porcentaje / Decimal::ONE_HUNDRED)
            .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
        if total > Decimal::ZERO {
            for c in d
                .conceptos
                .iter()
                .filter(|c| c.tipo == TipoConceptoCobro::FondoImprevistos)
            {
                for (bien, valor) in repartir(total, &base, d.redondeo)? {
                    cobros.push(Cobro {
                        bien,
                        concepto: c.id,
                        cuota: None,
                        valor,
                    });
                }
            }
        }
    }
    for cuota in &d.cuotas {
        let c = d.concepto(cuota.concepto)?;
        if !c.criterio.reparte_total() {
            return Err(regla_liquidacion(format!(
                "la cuota extraordinaria del concepto `{}` necesita un criterio que reparta",
                c.codigo
            )));
        }
        for (bien, valor) in d.repartir_concepto(c, cuota.valor)? {
            cobros.push(Cobro {
                bien,
                concepto: c.id,
                cuota: Some(cuota.id),
                valor,
            });
        }
    }
    cobros.retain(|c| c.valor > Decimal::ZERO);
    Ok(cobros)
}

// ---------------------------------------------------------------------------------------------
// Imputación de pagos y pronto pago
// ---------------------------------------------------------------------------------------------

/// Obligación con saldo, candidata a recibir un pago.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pendiente {
    pub id: ObligacionCobroId,
    pub periodo: NaiveDate,
    /// ¿Es de intereses de mora? (va antes que el capital).
    pub es_interes: bool,
    pub saldo: Decimal,
}

/// Distribuye un pago entre las obligaciones según el orden de la política. Las de `primero`
/// (las que eligió el pagador, si la política lo permite) van antes, en su orden. Devuelve lo
/// aplicado a cada una y lo que sobra (anticipo).
pub fn imputar(
    pago: Decimal,
    pendientes: &[Pendiente],
    orden: OrdenImputacion,
    primero: &[ObligacionCobroId],
) -> (Vec<(ObligacionCobroId, Decimal)>, Decimal) {
    let mut lista: Vec<&Pendiente> = pendientes
        .iter()
        .filter(|p| p.saldo > Decimal::ZERO)
        .collect();
    lista.sort_by_key(|p| {
        let elegido = primero
            .iter()
            .position(|e| *e == p.id)
            .unwrap_or(usize::MAX);
        match orden {
            OrdenImputacion::InteresesPrimero => (elegido, !p.es_interes, p.periodo, p.id),
            OrdenImputacion::PorAntiguedad => (elegido, false, p.periodo, p.id),
        }
    });
    if orden == OrdenImputacion::PorAntiguedad {
        // Dentro de cada periodo (y fuera de las elegidas), intereses antes que capital.
        lista.sort_by_key(|p| {
            let elegido = primero
                .iter()
                .position(|e| *e == p.id)
                .unwrap_or(usize::MAX);
            (elegido, p.periodo, !p.es_interes, p.id)
        });
    }
    let mut resto = pago.max(Decimal::ZERO);
    let mut aplicado = Vec::new();
    for p in lista {
        if resto <= Decimal::ZERO {
            break;
        }
        let valor = resto.min(p.saldo);
        aplicado.push((p.id, valor));
        resto -= valor;
    }
    (aplicado, resto)
}

/// Descuento por pronto pago: el porcentaje sobre la base, redondeado al peso.
pub fn descuento_pronto_pago(porcentaje: Decimal, base: Decimal) -> Decimal {
    let mut v = (base * porcentaje / Decimal::ONE_HUNDRED)
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero);
    v.rescale(2);
    v
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;
    use uuid::Uuid;

    use super::*;

    fn bien(n: u128) -> BienPrivadoId {
        BienPrivadoId::from_uuid(Uuid::from_u128(n))
    }

    fn concepto(n: u128) -> ConceptoCobroId {
        ConceptoCobroId::from_uuid(Uuid::from_u128(n))
    }

    fn obligacion(n: u128) -> ObligacionCobroId {
        ObligacionCobroId::from_uuid(Uuid::from_u128(n))
    }

    fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, d).expect("fecha")
    }

    fn politica() -> ParametrosPolitica {
        ParametrosPolitica {
            dia_vencimiento: 10,
            redondeo: dec!(100),
            pronto_pago: Some((dec!(5), 5)),
            factor_mora: dec!(1.5),
            tasa_mora_mensual: None,
            imputacion: OrdenImputacion::InteresesPrimero,
            permite_elegir_obligacion: false,
            porcentaje_fondo: dec!(1),
            fondo_suspendido: false,
        }
    }

    fn bienes_tres() -> Vec<BienLiquidable> {
        (1..=3)
            .map(|n| BienLiquidable {
                id: bien(n),
                tipo: if n == 3 {
                    TipoBienPrivado::Local
                } else {
                    TipoBienPrivado::Apartamento
                },
                area_m2: Some(Decimal::from(n as u64 * 10)),
            })
            .collect()
    }

    fn concepto_liquidable(
        n: u128,
        tipo: TipoConceptoCobro,
        criterio: CriterioDistribucion,
    ) -> ConceptoLiquidable {
        ConceptoLiquidable {
            id: concepto(n),
            codigo: format!("c{n}"),
            tipo,
            criterio,
            modulo: None,
            valor_fijo: None,
            tarifas: Vec::new(),
        }
    }

    fn tercios() -> BTreeMap<BienPrivadoId, Decimal> {
        [
            (bien(1), dec!(0.33333333)),
            (bien(2), dec!(0.33333333)),
            (bien(3), dec!(0.33333334)),
        ]
        .into()
    }

    #[test]
    fn repartir_cuadra_al_peso_con_el_mayor_residuo() {
        let partes = repartir(
            dec!(1000000),
            &[(1, Decimal::ONE), (2, Decimal::ONE), (3, Decimal::ONE)],
            dec!(100),
        )
        .expect("reparte");

        assert_eq!(
            partes.iter().map(|(_, v)| *v).sum::<Decimal>(),
            dec!(1000000)
        );
        assert_eq!(
            partes,
            vec![(1, dec!(333400)), (2, dec!(333300)), (3, dec!(333300))]
        );
    }

    #[test]
    fn repartir_entrega_el_pedazo_menor_que_el_multiplo() {
        let partes =
            repartir(dec!(1050), &[(1, dec!(0.5)), (2, dec!(0.5))], dec!(100)).expect("reparte");

        assert_eq!(partes.iter().map(|(_, v)| *v).sum::<Decimal>(), dec!(1050));
        assert_eq!(partes, vec![(1, dec!(550)), (2, dec!(500))]);
    }

    #[test]
    fn repartir_sin_pesos_es_un_error() {
        assert!(repartir(dec!(10), &[(1, Decimal::ZERO)], Decimal::ONE).is_err());
        assert!(repartir(dec!(-1), &[(1, Decimal::ONE)], Decimal::ONE).is_err());
        assert!(repartir(dec!(10), &[(1, Decimal::ONE)], Decimal::ZERO).is_err());
    }

    #[test]
    fn la_cuota_extraordinaria_sigue_su_calendario_y_cuadra() {
        let valores: Vec<Decimal> = [fecha(2026, 11, 1), fecha(2026, 12, 1), fecha(2027, 1, 1)]
            .into_iter()
            .filter_map(|p| cuota_del_periodo(dec!(1000), 3, fecha(2026, 11, 1), p, Decimal::ONE))
            .collect();

        assert_eq!(valores, vec![dec!(334), dec!(333), dec!(333)]);
        assert_eq!(
            cuota_del_periodo(
                dec!(1000),
                3,
                fecha(2026, 11, 1),
                fecha(2027, 2, 1),
                Decimal::ONE
            ),
            None
        );
        assert_eq!(
            cuota_del_periodo(
                dec!(1000),
                3,
                fecha(2026, 11, 1),
                fecha(2026, 10, 1),
                Decimal::ONE
            ),
            None
        );
    }

    #[test]
    fn liquidar_reparte_por_coeficiente_y_agrega_el_fondo_en_linea_aparte() {
        let d = DatosLiquidacion {
            bienes: bienes_tres(),
            conceptos: vec![
                concepto_liquidable(
                    1,
                    TipoConceptoCobro::Ordinaria,
                    CriterioDistribucion::Coeficiente,
                ),
                concepto_liquidable(
                    2,
                    TipoConceptoCobro::FondoImprevistos,
                    CriterioDistribucion::Coeficiente,
                ),
            ],
            presupuesto_mes: [(concepto(1), dec!(900000))].into(),
            coeficientes: tercios(),
            redondeo: dec!(100),
            porcentaje_fondo: Some(dec!(1)),
            ..DatosLiquidacion::default()
        };

        let cobros = liquidar(&d).expect("liquida");

        let total = |c: u128| -> Decimal {
            cobros
                .iter()
                .filter(|x| x.concepto == concepto(c))
                .map(|x| x.valor)
                .sum()
        };
        assert_eq!(total(1), dec!(900000));
        assert_eq!(total(2), dec!(9000));
        assert_eq!(cobros.len(), 6);
    }

    #[test]
    fn el_fondo_suspendido_no_se_liquida() {
        let d = DatosLiquidacion {
            bienes: bienes_tres(),
            conceptos: vec![
                concepto_liquidable(
                    1,
                    TipoConceptoCobro::Ordinaria,
                    CriterioDistribucion::Coeficiente,
                ),
                concepto_liquidable(
                    2,
                    TipoConceptoCobro::FondoImprevistos,
                    CriterioDistribucion::Coeficiente,
                ),
            ],
            presupuesto_mes: [(concepto(1), dec!(900000))].into(),
            coeficientes: tercios(),
            redondeo: Decimal::ONE,
            porcentaje_fondo: None,
            ..DatosLiquidacion::default()
        };

        let cobros = liquidar(&d).expect("liquida");

        assert!(cobros.iter().all(|c| c.concepto == concepto(1)));
    }

    #[test]
    fn el_modulo_de_contribucion_no_toca_bienes_fuera_del_modulo() {
        let modulo = ModuloContribucionId::from_uuid(Uuid::from_u128(9));
        let mut c = concepto_liquidable(
            1,
            TipoConceptoCobro::Ordinaria,
            CriterioDistribucion::Modulo,
        );
        c.modulo = Some(modulo);
        let d = DatosLiquidacion {
            bienes: bienes_tres(),
            conceptos: vec![c],
            presupuesto_mes: [(concepto(1), dec!(1000))].into(),
            coeficientes_modulo: [(modulo, [(bien(1), dec!(0.5)), (bien(2), dec!(0.5))].into())]
                .into(),
            redondeo: Decimal::ONE,
            ..DatosLiquidacion::default()
        };

        let cobros = liquidar(&d).expect("liquida");

        assert_eq!(cobros.len(), 2);
        assert!(cobros.iter().all(|c| c.bien != bien(3)));
    }

    #[test]
    fn un_modulo_sin_cuadro_vigente_no_se_liquida() {
        let mut c = concepto_liquidable(
            1,
            TipoConceptoCobro::Ordinaria,
            CriterioDistribucion::Modulo,
        );
        c.modulo = Some(ModuloContribucionId::from_uuid(Uuid::from_u128(9)));
        let d = DatosLiquidacion {
            bienes: bienes_tres(),
            conceptos: vec![c],
            presupuesto_mes: [(concepto(1), dec!(1000))].into(),
            redondeo: Decimal::ONE,
            ..DatosLiquidacion::default()
        };

        assert!(matches!(liquidar(&d), Err(DominioError::ReglaContable(_))));
    }

    #[test]
    fn valor_fijo_tipo_de_bien_y_area() {
        let mut fijo = concepto_liquidable(
            1,
            TipoConceptoCobro::Ordinaria,
            CriterioDistribucion::ValorFijo,
        );
        fijo.valor_fijo = Some(dec!(5000));
        let mut por_tipo = concepto_liquidable(
            2,
            TipoConceptoCobro::Ordinaria,
            CriterioDistribucion::PorTipoBien,
        );
        por_tipo.tarifas = vec![(TipoBienPrivado::Local, dec!(20000))];
        let area = concepto_liquidable(3, TipoConceptoCobro::Ordinaria, CriterioDistribucion::Area);
        let d = DatosLiquidacion {
            bienes: bienes_tres(),
            conceptos: vec![fijo, por_tipo, area],
            presupuesto_mes: [(concepto(3), dec!(600))].into(),
            redondeo: Decimal::ONE,
            ..DatosLiquidacion::default()
        };

        let cobros = liquidar(&d).expect("liquida");

        let de = |c: u128| -> Vec<(BienPrivadoId, Decimal)> {
            cobros
                .iter()
                .filter(|x| x.concepto == concepto(c))
                .map(|x| (x.bien, x.valor))
                .collect()
        };
        assert_eq!(de(1).len(), 3);
        assert_eq!(de(2), vec![(bien(3), dec!(20000))]);
        assert_eq!(
            de(3),
            vec![
                (bien(1), dec!(100)),
                (bien(2), dec!(200)),
                (bien(3), dec!(300))
            ]
        );
    }

    #[test]
    fn la_cuota_extraordinaria_se_reparte_por_su_criterio() {
        let cuota = CuotaExtraordinariaId::from_uuid(Uuid::from_u128(7));
        let d = DatosLiquidacion {
            bienes: bienes_tres(),
            conceptos: vec![concepto_liquidable(
                4,
                TipoConceptoCobro::Extraordinaria,
                CriterioDistribucion::Coeficiente,
            )],
            coeficientes: tercios(),
            cuotas: vec![CuotaDelPeriodo {
                id: cuota,
                concepto: concepto(4),
                valor: dec!(100000),
            }],
            redondeo: Decimal::ONE,
            ..DatosLiquidacion::default()
        };

        let cobros = liquidar(&d).expect("liquida");

        assert!(cobros.iter().all(|c| c.cuota == Some(cuota)));
        assert_eq!(
            cobros.iter().map(|c| c.valor).sum::<Decimal>(),
            dec!(100000)
        );
    }

    #[test]
    fn imputa_intereses_antes_que_capital() {
        let pendientes = [
            Pendiente {
                id: obligacion(1),
                periodo: fecha(2026, 9, 1),
                es_interes: false,
                saldo: dec!(100),
            },
            Pendiente {
                id: obligacion(2),
                periodo: fecha(2026, 10, 1),
                es_interes: true,
                saldo: dec!(10),
            },
            Pendiente {
                id: obligacion(3),
                periodo: fecha(2026, 10, 1),
                es_interes: false,
                saldo: dec!(100),
            },
        ];

        let (aplicado, sobra) = imputar(
            dec!(150),
            &pendientes,
            OrdenImputacion::InteresesPrimero,
            &[],
        );

        assert_eq!(
            aplicado,
            vec![
                (obligacion(2), dec!(10)),
                (obligacion(1), dec!(100)),
                (obligacion(3), dec!(40))
            ]
        );
        assert_eq!(sobra, Decimal::ZERO);
    }

    #[test]
    fn por_antiguedad_cubre_periodo_por_periodo_y_deja_anticipo() {
        let pendientes = [
            Pendiente {
                id: obligacion(3),
                periodo: fecha(2026, 10, 1),
                es_interes: false,
                saldo: dec!(100),
            },
            Pendiente {
                id: obligacion(2),
                periodo: fecha(2026, 10, 1),
                es_interes: true,
                saldo: dec!(10),
            },
            Pendiente {
                id: obligacion(1),
                periodo: fecha(2026, 9, 1),
                es_interes: false,
                saldo: dec!(100),
            },
        ];

        let (aplicado, sobra) =
            imputar(dec!(250), &pendientes, OrdenImputacion::PorAntiguedad, &[]);

        assert_eq!(
            aplicado,
            vec![
                (obligacion(1), dec!(100)),
                (obligacion(2), dec!(10)),
                (obligacion(3), dec!(100))
            ]
        );
        assert_eq!(sobra, dec!(40));
    }

    #[test]
    fn la_obligacion_elegida_va_primero() {
        let pendientes = [
            Pendiente {
                id: obligacion(1),
                periodo: fecha(2026, 9, 1),
                es_interes: false,
                saldo: dec!(100),
            },
            Pendiente {
                id: obligacion(2),
                periodo: fecha(2026, 10, 1),
                es_interes: false,
                saldo: dec!(100),
            },
        ];

        let (aplicado, _) = imputar(
            dec!(100),
            &pendientes,
            OrdenImputacion::InteresesPrimero,
            &[obligacion(2)],
        );

        assert_eq!(aplicado, vec![(obligacion(2), dec!(100))]);
    }

    #[test]
    fn descuento_de_pronto_pago_al_peso() {
        assert_eq!(descuento_pronto_pago(dec!(5), dec!(333333)), dec!(16667.00));
    }

    #[test]
    fn la_politica_respeta_los_topes_legales() {
        assert!(politica().validar().is_ok());
        let mut p = politica();
        p.factor_mora = dec!(1.6);
        assert!(p.validar().is_err());
        let mut p = politica();
        p.porcentaje_fondo = dec!(0.5);
        assert!(p.validar().is_err());
        p.fondo_suspendido = true;
        assert!(p.validar().is_ok());
        assert_eq!(p.fondo_a_liquidar(), None);
        let mut p = politica();
        p.pronto_pago = Some((dec!(5), 15));
        assert!(p.validar().is_err());
        let mut p = politica();
        p.redondeo = dec!(0.5);
        assert!(p.validar().is_err());
    }

    #[test]
    fn periodos_y_fechas() {
        assert_eq!(periodo_de(fecha(2026, 11, 13)), fecha(2026, 11, 1));
        assert_eq!(meses_entre(fecha(2026, 11, 1), fecha(2027, 2, 1)), 3);
        assert_eq!(
            dia_del_periodo(fecha(2026, 2, 1), 28).expect("día"),
            fecha(2026, 2, 28)
        );
        assert!(dia_del_periodo(fecha(2026, 2, 1), 30).is_err());
    }

    #[test]
    fn alerta_del_fondo_a_la_mitad() {
        assert!(fondo_llego_a_la_mitad(dec!(50), dec!(100)));
        assert!(!fondo_llego_a_la_mitad(dec!(49), dec!(100)));
        assert!(!fondo_llego_a_la_mitad(dec!(10), Decimal::ZERO));
    }

    #[test]
    fn presupuesto_y_estados() {
        let doce = vec![Monto::CERO; 12];
        assert!(valores_mensuales(&doce).is_ok());
        assert!(valores_mensuales(&doce[..11]).is_err());
        assert_eq!(
            EstadoPresupuesto::Borrador.transicionar(EstadoPresupuesto::Vigente),
            Ok(EstadoPresupuesto::Vigente)
        );
        assert!(
            EstadoPresupuesto::Reemplazado
                .transicionar(EstadoPresupuesto::Vigente)
                .is_err()
        );
        assert!(TipoConceptoCobro::Multa.admite_cobro_manual());
        assert!(!TipoConceptoCobro::Ordinaria.admite_cobro_manual());
    }
}
