//! Cobro de la plataforma a sus clientes (plan 19, ADR 0024): reglas puras para calcular el
//! corte de una suscripción, decidir si se aprueba solo y llevar el calendario de los avisos de
//! cobro y de la suspensión por mora.
//!
//! - **Modelos de tarifa** ([`ModeloTarifa`]): plana por periodo, por unidad medida, por paquete
//!   con excedente, por consumo y cobro único. Los precios los define la plataforma: el sistema no
//!   impone descuentos ni topes (un precio pactado reemplaza la tarifa en esa línea).
//! - **Prorrateo por días** de lo periódico (plana, por unidad y la base del paquete) según los
//!   días en que la línea estuvo vigente dentro del periodo; el consumo y lo único no se prorratean.
//! - Montos en pesos, **redondeados al peso** línea por línea.
//! - Mora: factura con vencimiento a [`PLAZO_PAGO_POR_DEFECTO`] días; recordatorio
//!   [`DIAS_RECORDATORIO`] días antes, después del vencimiento cada [`DIAS_HABILES_ENTRE_AVISOS`]
//!   días hábiles, y suspensión a los [`DIAS_PARA_SUSPENDER`] días de vencida.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

use crate::{DominioError, calendario::dias_habiles_entre};

/// Días entre la factura y su vencimiento si la suscripción no dice otra cosa.
pub const PLAZO_PAGO_POR_DEFECTO: u16 = 15;
/// Días antes del vencimiento en que sale el recordatorio de pago.
pub const DIAS_RECORDATORIO: i64 = 5;
/// Días hábiles entre dos avisos de una factura vencida.
pub const DIAS_HABILES_ENTRE_AVISOS: u32 = 5;
/// Días de vencida tras los que la entidad se suspende sola (la plataforma puede antes, a mano).
pub const DIAS_PARA_SUSPENDER: i64 = 30;
/// Días máximos de prueba gratuita.
pub const MAXIMO_DIAS_PRUEBA: u16 = 30;

fn invalido(campo: &'static str, regla: &'static str) -> DominioError {
    DominioError::DatoInvalido { campo, regla }
}

/// Cómo se cobra un producto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeloTarifa {
    /// Valor fijo por periodo (prorrateado si la línea no estuvo todo el periodo).
    Plana,
    /// Valor por unidad medida al corte (p. ej. bienes privados), prorrateado.
    PorUnidad,
    /// Valor fijo con unidades incluidas; lo que pase se cobra como excedente.
    PorPaquete,
    /// Valor por unidad consumida en el periodo (p. ej. mensajes), sin prorrateo.
    PorConsumo,
    /// Una sola vez, en el periodo en que empieza la línea (implementación, capacitación).
    Unica,
}

enum_texto!(ModeloTarifa {
    Plana => "plana",
    PorUnidad => "por_unidad",
    PorPaquete => "por_paquete",
    PorConsumo => "por_consumo",
    Unica => "unica",
});

impl ModeloTarifa {
    /// ¿Necesita una métrica medida?
    pub const fn mide(self) -> bool {
        matches!(self, Self::PorUnidad | Self::PorPaquete | Self::PorConsumo)
    }
}

/// Métrica medida por entidad (`modulo.cosa`, p. ej. `ph.bienes_privados`). El facturador no
/// conoce las verticales: solo nombres de métricas.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Metrica(String);

impl Metrica {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no es `modulo.cosa` en minúsculas (hasta 60 caracteres).
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let v = valor.trim();
        let parte = |p: &str| {
            !p.is_empty()
                && p.starts_with(|c: char| c.is_ascii_lowercase())
                && p.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        };
        match v.split_once('.') {
            Some((a, b)) if v.len() <= 60 && parte(a) && parte(b) => Ok(Self(v.to_owned())),
            _ => Err(invalido("metrica", "modulo.cosa en minúsculas")),
        }
    }

    /// Texto de la métrica.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Metrica {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<Metrica> for String {
    fn from(m: Metrica) -> Self {
        m.0
    }
}

/// Tarifa de un producto (versionada por fecha de vigencia).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tarifa {
    pub modelo: ModeloTarifa,
    /// Valor por periodo, por unidad o del paquete, según el modelo (COP).
    pub valor: Decimal,
    /// Unidades incluidas en el paquete.
    pub incluidas: Option<u32>,
    /// Valor de cada unidad de excedente del paquete.
    pub valor_excedente: Option<Decimal>,
    pub metrica: Option<Metrica>,
}

impl Tarifa {
    /// Valida la coherencia del modelo con sus datos.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si el valor es negativo, falta o sobra la métrica, o un
    /// paquete no trae unidades incluidas y valor del excedente.
    pub fn validada(self) -> Result<Self, DominioError> {
        if self.valor < Decimal::ZERO {
            return Err(invalido("valor", "no puede ser negativo"));
        }
        if self.modelo.mide() != self.metrica.is_some() {
            return Err(invalido(
                "metrica",
                "obligatoria en por_unidad, por_paquete y por_consumo, y solo en ellos",
            ));
        }
        let paquete = self.modelo == ModeloTarifa::PorPaquete;
        if paquete != (self.incluidas.is_some() && self.valor_excedente.is_some()) {
            return Err(invalido(
                "incluidas",
                "un paquete trae unidades incluidas y valor del excedente (y solo él)",
            ));
        }
        if self.valor_excedente.is_some_and(|v| v < Decimal::ZERO) {
            return Err(invalido("valor_excedente", "no puede ser negativo"));
        }
        Ok(self)
    }
}

/// Periodo de un corte: `[desde, hasta)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Periodo {
    pub desde: NaiveDate,
    /// Excluido.
    pub hasta: NaiveDate,
}

impl Periodo {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si termina antes de empezar o el mismo día.
    pub fn new(desde: NaiveDate, hasta: NaiveDate) -> Result<Self, DominioError> {
        if hasta <= desde {
            return Err(invalido("periodo", "debe terminar después de empezar"));
        }
        Ok(Self { desde, hasta })
    }

    /// Días del periodo.
    pub fn dias(&self) -> i64 {
        (self.hasta - self.desde).num_days()
    }

    /// Días en que `[desde, hasta)` (hasta `None` = sin fin) se cruza con el periodo.
    pub fn dias_en(&self, desde: NaiveDate, hasta: Option<NaiveDate>) -> i64 {
        let inicio = desde.max(self.desde);
        let fin = hasta.map_or(self.hasta, |h| h.min(self.hasta));
        (fin - inicio).num_days().max(0)
    }

    /// `AAAA-MM-DD a AAAA-MM-DD` (último día incluido) para el texto de la factura.
    pub fn texto(&self) -> String {
        let ultimo = self.hasta.pred_opt().unwrap_or(self.hasta);
        format!("{} a {}", self.desde, ultimo)
    }
}

/// Frecuencia con que se corta una suscripción.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "tipo", content = "meses")]
pub enum Ciclo {
    /// Cada N meses (1 = mensual, 12 = anual).
    Meses(u8),
    /// Cuando ocurre el hecho medido (se corta a pedido).
    PorEvento,
}

impl Ciclo {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si son 0 o más de 12 meses.
    pub fn validado(self) -> Result<Self, DominioError> {
        match self {
            Self::Meses(m) if !(1..=12).contains(&m) => Err(invalido("ciclo", "de 1 a 12 meses")),
            otro => Ok(otro),
        }
    }

    /// Periodo que empieza en `desde` (un ciclo por evento no tiene periodo fijo).
    pub fn periodo_desde(self, desde: NaiveDate) -> Option<Periodo> {
        match self {
            Self::Meses(m) => {
                let hasta = desde.checked_add_months(chrono::Months::new(u32::from(m)))?;
                Periodo::new(desde, hasta).ok()
            }
            Self::PorEvento => None,
        }
    }
}

/// Línea de una suscripción con la tarifa vigente al corte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineaACobrar {
    /// Código del producto (identifica la línea en el corte).
    pub producto: String,
    pub nombre: String,
    /// Plantilla del texto de la factura: `{producto}`, `{periodo}`, `{cantidad}`, `{incluidas}`.
    pub plantilla: String,
    pub tarifa: Tarifa,
    /// Precio pactado que reemplaza el `valor` de la tarifa (descuento comercial).
    pub precio_pactado: Option<Decimal>,
    pub desde: NaiveDate,
    pub hasta: Option<NaiveDate>,
}

/// Línea calculada del corte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineaCorte {
    pub producto: String,
    /// Texto final de la factura.
    pub descripcion: String,
    pub cantidad: Decimal,
    pub valor_unitario: Decimal,
    /// `cantidad × valor_unitario` prorrateado y redondeado al peso.
    pub valor: Decimal,
}

fn al_peso(v: Decimal) -> Decimal {
    v.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
}

fn texto(l: &LineaACobrar, periodo: &Periodo, cantidad: Decimal) -> String {
    l.plantilla
        .replace("{producto}", &l.nombre)
        .replace("{periodo}", &periodo.texto())
        .replace("{cantidad}", &cantidad.normalize().to_string())
        .replace(
            "{incluidas}",
            &l.tarifa
                .incluidas
                .map(|i| i.to_string())
                .unwrap_or_default(),
        )
}

/// Calcula las líneas del corte del periodo. `mediciones` trae la cantidad de cada métrica de la
/// entidad en el periodo (lo que falte cuenta 0). Las líneas sin valor no salen.
pub fn calcular_corte(
    periodo: &Periodo,
    lineas: &[LineaACobrar],
    mediciones: &BTreeMap<Metrica, u64>,
) -> Vec<LineaCorte> {
    let total_dias = Decimal::from(periodo.dias());
    let mut v = Vec::new();
    for l in lineas {
        let dias = periodo.dias_en(l.desde, l.hasta);
        if dias == 0 && l.tarifa.modelo != ModeloTarifa::PorConsumo {
            continue;
        }
        let factor = Decimal::from(dias) / total_dias;
        let precio = l.precio_pactado.unwrap_or(l.tarifa.valor);
        let medido = l
            .tarifa
            .metrica
            .as_ref()
            .and_then(|m| mediciones.get(m))
            .copied()
            .unwrap_or(0);
        let mut agregar = |producto: &str,
                           descripcion: String,
                           cantidad: Decimal,
                           unitario: Decimal,
                           valor: Decimal| {
            let valor = al_peso(valor);
            if valor > Decimal::ZERO {
                v.push(LineaCorte {
                    producto: producto.to_owned(),
                    descripcion,
                    cantidad,
                    valor_unitario: unitario,
                    valor,
                });
            }
        };
        match l.tarifa.modelo {
            ModeloTarifa::Plana => {
                agregar(
                    &l.producto,
                    texto(l, periodo, Decimal::ONE),
                    Decimal::ONE,
                    precio,
                    precio * factor,
                );
            }
            ModeloTarifa::PorUnidad => {
                let cantidad = Decimal::from(medido);
                agregar(
                    &l.producto,
                    texto(l, periodo, cantidad),
                    cantidad,
                    precio,
                    cantidad * precio * factor,
                );
            }
            ModeloTarifa::PorPaquete => {
                let cantidad = Decimal::from(medido);
                agregar(
                    &l.producto,
                    texto(l, periodo, cantidad),
                    Decimal::ONE,
                    precio,
                    precio * factor,
                );
                let incluidas = u64::from(l.tarifa.incluidas.unwrap_or(0));
                let excedente = medido.saturating_sub(incluidas);
                if excedente > 0 {
                    let unitario = l.tarifa.valor_excedente.unwrap_or(Decimal::ZERO);
                    let exceso = Decimal::from(excedente);
                    agregar(
                        &format!("{}.excedente", l.producto),
                        format!("Excedente de {}: {} unidades", l.nombre, excedente),
                        exceso,
                        unitario,
                        exceso * unitario,
                    );
                }
            }
            ModeloTarifa::PorConsumo => {
                let cantidad = Decimal::from(medido);
                agregar(
                    &l.producto,
                    texto(l, periodo, cantidad),
                    cantidad,
                    precio,
                    cantidad * precio,
                );
            }
            ModeloTarifa::Unica => {
                if periodo.desde <= l.desde && l.desde < periodo.hasta {
                    agregar(
                        &l.producto,
                        texto(l, periodo, Decimal::ONE),
                        Decimal::ONE,
                        precio,
                        precio,
                    );
                }
            }
        }
    }
    v
}

/// Total del corte.
pub fn total(lineas: &[LineaCorte]) -> Decimal {
    lineas.iter().map(|l| l.valor).sum()
}

/// ¿El corte se aprueba solo? Cuando cobra lo mismo que el anterior (mismos productos,
/// cantidades y valores); lo demás lo aprueba una persona.
pub fn igual_al_anterior(actual: &[LineaCorte], anterior: Option<&[LineaCorte]>) -> bool {
    let clave = |v: &[LineaCorte]| {
        let mut x: Vec<(String, Decimal, Decimal)> = v
            .iter()
            .map(|l| (l.producto.clone(), l.cantidad, l.valor))
            .collect();
        x.sort();
        x
    };
    anterior.is_some_and(|a| !actual.is_empty() && clave(actual) == clave(a))
}

/// Valida los días de prueba gratuita (0 = sin prueba).
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si pasan de [`MAXIMO_DIAS_PRUEBA`].
pub fn validar_dias_prueba(dias: u16) -> Result<u16, DominioError> {
    if dias > MAXIMO_DIAS_PRUEBA {
        return Err(invalido("dias_prueba", "hasta 30 días"));
    }
    Ok(dias)
}

/// Estado de una suscripción.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoSuscripcion {
    Activa,
    /// Suspendida por la plataforma (p. ej. por mora): la entidad queda en solo lectura.
    Suspendida,
    /// Terminada (estado final): no genera más cortes.
    Terminada,
}

enum_texto!(EstadoSuscripcion {
    Activa => "activa",
    Suspendida => "suspendida",
    Terminada => "terminada",
});

/// Estado de un corte de cobro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoCorte {
    /// Calculado; se puede recalcular, aprobar o descartar.
    Borrador,
    /// Listo para facturar («por facturar»).
    Aprobado,
    /// Con factura (propia o emitida por fuera); corre el plazo de pago.
    Facturado,
    Pagado,
    /// Descartado con motivo (estado final).
    Descartado,
}

enum_texto!(EstadoCorte {
    Borrador => "borrador",
    Aprobado => "aprobado",
    Facturado => "facturado",
    Pagado => "pagado",
    Descartado => "descartado",
});

impl EstadoCorte {
    /// Borrador → aprobado → facturado → pagado; borrador o aprobado → descartado.
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si el diagrama no la admite.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoCorte::*;
        match (self, hacia) {
            (Borrador, Aprobado)
            | (Aprobado, Facturado)
            | (Facturado, Pagado)
            | (Borrador | Aprobado, Descartado) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "corte",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }
}

/// Qué toca hacer hoy con una factura de la plataforma sin pagar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccionMora {
    /// Nada todavía.
    Esperar,
    /// Recordatorio antes del vencimiento.
    Recordar,
    /// Aviso de factura vencida.
    AvisarVencida,
    /// Suspender la entidad (solo lectura) por mora.
    Suspender,
}

/// Calendario de cobro de una factura vencida o por vencer: recordatorio
/// [`DIAS_RECORDATORIO`] días antes del vencimiento, aviso cada [`DIAS_HABILES_ENTRE_AVISOS`]
/// días hábiles después de vencida y suspensión a los [`DIAS_PARA_SUSPENDER`] días (si no está
/// ya suspendida). `ultimo_aviso` = día del último aviso enviado.
pub fn accion_de_mora(
    hoy: NaiveDate,
    vencimiento: NaiveDate,
    ultimo_aviso: Option<NaiveDate>,
    ya_suspendida: bool,
) -> AccionMora {
    let vencida_hace = (hoy - vencimiento).num_days();
    if vencida_hace >= DIAS_PARA_SUSPENDER && !ya_suspendida {
        return AccionMora::Suspender;
    }
    if vencida_hace > 0 {
        let toca = ultimo_aviso.is_none_or(|u| {
            u <= vencimiento || dias_habiles_entre(u, hoy) >= DIAS_HABILES_ENTRE_AVISOS
        });
        return if toca {
            AccionMora::AvisarVencida
        } else {
            AccionMora::Esperar
        };
    }
    let faltan = (vencimiento - hoy).num_days();
    if faltan <= DIAS_RECORDATORIO
        && ultimo_aviso.is_none_or(|u| (vencimiento - u).num_days() > DIAS_RECORDATORIO)
    {
        AccionMora::Recordar
    } else {
        AccionMora::Esperar
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    fn d(m: u32, dia: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, dia).expect("fecha")
    }

    fn noviembre() -> Periodo {
        Periodo::new(d(11, 1), d(12, 1)).expect("periodo")
    }

    fn linea(modelo: ModeloTarifa, valor: Decimal, metrica: Option<&str>) -> LineaACobrar {
        LineaACobrar {
            producto: "asambleas".into(),
            nombre: "Asambleas".into(),
            plantilla: "{producto} — {periodo}: {cantidad}".into(),
            tarifa: Tarifa {
                modelo,
                valor,
                incluidas: None,
                valor_excedente: None,
                metrica: metrica.map(|m| Metrica::new(m).expect("métrica")),
            },
            precio_pactado: None,
            desde: d(1, 1),
            hasta: None,
        }
    }

    fn medido(m: &str, n: u64) -> BTreeMap<Metrica, u64> {
        [(Metrica::new(m).expect("métrica"), n)]
            .into_iter()
            .collect()
    }

    #[test]
    fn mes_completo_cobra_la_tarifa_plana_con_su_texto() {
        let c = calcular_corte(
            &noviembre(),
            &[linea(ModeloTarifa::Plana, dec!(90000), None)],
            &BTreeMap::new(),
        );
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].valor, dec!(90000));
        assert_eq!(c[0].descripcion, "Asambleas — 2026-11-01 a 2026-11-30: 1");
    }

    #[test]
    fn alta_y_baja_a_mitad_de_mes_se_prorratean_al_peso() {
        let mut alta = linea(ModeloTarifa::Plana, dec!(100000), None);
        alta.desde = d(11, 16);
        let mut baja = linea(ModeloTarifa::Plana, dec!(100000), None);
        baja.hasta = Some(d(11, 11));
        let c = calcular_corte(&noviembre(), &[alta, baja], &BTreeMap::new());
        assert_eq!(c[0].valor, dec!(50000), "15 de 30 días");
        assert_eq!(c[1].valor, dec!(33333), "10 de 30 días, al peso");
    }

    #[test]
    fn por_unidad_multiplica_lo_medido() {
        let c = calcular_corte(
            &noviembre(),
            &[linea(
                ModeloTarifa::PorUnidad,
                dec!(500),
                Some("ph.bienes_privados"),
            )],
            &medido("ph.bienes_privados", 320),
        );
        assert_eq!((c[0].cantidad, c[0].valor), (dec!(320), dec!(160000)));
    }

    #[test]
    fn el_paquete_sin_usar_cobra_la_base_y_el_excedente_aparte() {
        let mut p = linea(
            ModeloTarifa::PorPaquete,
            dec!(80000),
            Some("facturacion.documentos"),
        );
        p.tarifa.incluidas = Some(200);
        p.tarifa.valor_excedente = Some(dec!(300));
        let sin_usar = calcular_corte(&noviembre(), std::slice::from_ref(&p), &BTreeMap::new());
        assert_eq!(sin_usar.len(), 1);
        assert_eq!(sin_usar[0].valor, dec!(80000));
        let con_exceso = calcular_corte(&noviembre(), &[p], &medido("facturacion.documentos", 212));
        assert_eq!(con_exceso.len(), 2);
        assert_eq!(con_exceso[1].valor, dec!(3600), "12 × 300");
        assert_eq!(total(&con_exceso), dec!(83600));
    }

    #[test]
    fn consumo_y_unica_no_se_prorratean_y_la_unica_sale_una_vez() {
        let consumo = linea(
            ModeloTarifa::PorConsumo,
            dec!(80),
            Some("whatsapp.mensajes"),
        );
        let mut unica = linea(ModeloTarifa::Unica, dec!(500000), None);
        unica.producto = "implementacion".into();
        unica.desde = d(11, 20);
        let c = calcular_corte(
            &noviembre(),
            &[consumo, unica.clone()],
            &medido("whatsapp.mensajes", 812),
        );
        assert_eq!(c[0].valor, dec!(64960));
        assert_eq!(c[1].valor, dec!(500000));
        let diciembre = Periodo::new(
            d(12, 1),
            NaiveDate::from_ymd_opt(2027, 1, 1).expect("fecha"),
        )
        .expect("periodo");
        assert!(calcular_corte(&diciembre, &[unica], &BTreeMap::new()).is_empty());
    }

    #[test]
    fn el_precio_pactado_reemplaza_la_tarifa() {
        let mut l = linea(ModeloTarifa::Plana, dec!(90000), None);
        l.precio_pactado = Some(dec!(70000));
        assert_eq!(
            calcular_corte(&noviembre(), &[l], &BTreeMap::new())[0].valor,
            dec!(70000)
        );
    }

    #[test]
    fn tarifas_incoherentes_se_rechazan() {
        let base = linea(ModeloTarifa::Plana, dec!(1), None).tarifa;
        assert!(base.clone().validada().is_ok());
        assert!(
            Tarifa {
                metrica: Some(Metrica::new("ph.bienes").expect("m")),
                ..base.clone()
            }
            .validada()
            .is_err()
        );
        assert!(
            Tarifa {
                modelo: ModeloTarifa::PorPaquete,
                metrica: Some(Metrica::new("a.b").expect("m")),
                ..base.clone()
            }
            .validada()
            .is_err()
        );
        assert!(
            Tarifa {
                valor: dec!(-1),
                ..base
            }
            .validada()
            .is_err()
        );
        assert!(Metrica::new("Sin.Punto").is_err() && Metrica::new("sinpunto").is_err());
    }

    #[test]
    fn se_aprueba_solo_si_cobra_lo_mismo_que_el_anterior() {
        let c = calcular_corte(
            &noviembre(),
            &[linea(ModeloTarifa::Plana, dec!(90000), None)],
            &BTreeMap::new(),
        );
        assert!(igual_al_anterior(&c, Some(&c)));
        assert!(
            !igual_al_anterior(&c, None),
            "el primero lo revisa una persona"
        );
        let mut otro = c.clone();
        otro[0].valor = dec!(1);
        assert!(!igual_al_anterior(&c, Some(&otro)));
    }

    #[test]
    fn calendario_de_mora() {
        let vence = d(11, 30);
        assert_eq!(
            accion_de_mora(d(11, 20), vence, None, false),
            AccionMora::Esperar
        );
        assert_eq!(
            accion_de_mora(d(11, 25), vence, None, false),
            AccionMora::Recordar
        );
        assert_eq!(
            accion_de_mora(d(11, 26), vence, Some(d(11, 25)), false),
            AccionMora::Esperar
        );
        assert_eq!(
            accion_de_mora(d(12, 1), vence, Some(d(11, 25)), false),
            AccionMora::AvisarVencida
        );
        assert_eq!(
            accion_de_mora(d(12, 3), vence, Some(d(12, 1)), false),
            AccionMora::Esperar
        );
        // El 8 de diciembre es festivo: el quinto día hábil después del 1 es el 9.
        assert_eq!(
            accion_de_mora(d(12, 8), vence, Some(d(12, 1)), false),
            AccionMora::Esperar
        );
        assert_eq!(
            accion_de_mora(d(12, 9), vence, Some(d(12, 1)), false),
            AccionMora::AvisarVencida
        );
        assert_eq!(
            accion_de_mora(d(12, 30), vence, Some(d(12, 22)), false),
            AccionMora::Suspender
        );
        assert_ne!(
            accion_de_mora(d(12, 30), vence, Some(d(12, 22)), true),
            AccionMora::Suspender
        );
    }

    #[test]
    fn el_corte_sigue_su_diagrama() {
        use EstadoCorte::*;
        assert!(Borrador.transicionar(Aprobado).is_ok());
        assert!(Aprobado.transicionar(Facturado).is_ok());
        assert!(Facturado.transicionar(Pagado).is_ok());
        assert!(
            Facturado.transicionar(Descartado).is_err(),
            "lo facturado no se descarta"
        );
        assert!(
            Borrador.transicionar(Facturado).is_err(),
            "sin aprobar no se factura"
        );
    }

    #[test]
    fn ciclos_y_prueba() {
        assert_eq!(
            Ciclo::Meses(1).periodo_desde(d(1, 31)).map(|p| p.hasta),
            Some(d(2, 28))
        );
        assert!(Ciclo::Meses(13).validado().is_err());
        assert!(Ciclo::PorEvento.periodo_desde(d(1, 1)).is_none());
        assert!(validar_dias_prueba(30).is_ok() && validar_dias_prueba(31).is_err());
    }
}
