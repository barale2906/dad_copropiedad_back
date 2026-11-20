//! Prestaciones sociales y liquidación del contrato (plan 16 fase 4): reglas puras, sin IO.
//!
//! - **Base mensual** ([`base_mensual`]): el último salario, o el promedio del lapso si cambió en
//!   los últimos tres meses, más el auxilio de transporte (cesantías y prima) y el promedio de lo
//!   variable (horas extra, recargos, comisiones, bonificaciones salariales).
//! - **Cesantías** = base × días ÷ 360 × (días de cesantías por año ÷ 30); **intereses** =
//!   cesantías × tasa × días ÷ 360; **prima** = base × días del semestre ÷ 360 × (días de prima
//!   por año ÷ 30).
//! - **Vacaciones**: se causan días de vacaciones por año trabajado; al retiro, los pendientes se
//!   pagan al salario diario ([`vacaciones_pendientes`]).
//! - **Indemnización** sin justa causa (CST art. 64, [`indemnizacion`]): término fijo, el tiempo
//!   que falta; obra o labor, 15 días; indefinido, 30 días el primer año y 20 por cada año
//!   adicional (20 y 15 con salario de 10 salarios mínimos o más), proporcional.
//!
//! Marcadas para la revisión del especialista laboral: la base con lo variable, el aprendiz sin
//! prestaciones, los días hábiles de vacaciones (lunes a viernes) y la indemnización del aprendiz
//! (no se calcula).

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::{
    CatalogoLaboral, MotivoTerminacion, ParametroLaboral as P, TipoContratoLaboral,
    liquidacion::{ConceptoNomina, LineaLiquidacion, dias_comerciales},
};
use crate::{DominioError, contabilidad::Monto};

fn monto(v: Decimal) -> Result<Monto, DominioError> {
    Monto::new(v.round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero))
}

fn ano() -> Decimal {
    Decimal::from(360)
}

/// Lo que define la base mensual de las prestaciones de un lapso.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseLapso {
    /// Salario mensual vigente al final del lapso.
    pub salario_actual: Decimal,
    /// Promedio del salario mensual en el lapso (ponderado por días).
    pub salario_promedio: Decimal,
    /// ¿Cambió el salario en los últimos tres meses del lapso?
    pub cambio_reciente: bool,
    /// Auxilio de transporte mensual, si al final del lapso tiene derecho (cero si no).
    pub auxilio: Decimal,
    /// Devengos salariales variables del lapso (horas extra, recargos, comisiones,
    /// bonificaciones salariales).
    pub variables: Decimal,
    /// Días comerciales laborados del lapso.
    pub dias: u32,
}

/// Base mensual de las prestaciones. `con_auxilio` es falso en las vacaciones.
pub fn base_mensual(b: &BaseLapso, con_auxilio: bool) -> Decimal {
    let salario = if b.cambio_reciente {
        b.salario_promedio
    } else {
        b.salario_actual
    };
    let variable = if b.dias == 0 {
        Decimal::ZERO
    } else {
        b.variables / Decimal::from(b.dias) * Decimal::from(30)
    };
    salario
        + variable
        + if con_auxilio {
            b.auxilio
        } else {
            Decimal::ZERO
        }
}

/// Cesantías del lapso.
///
/// # Errors
///
/// [`DominioError::ReglaLaboral`] si falta el parámetro en el catálogo.
pub fn cesantias(b: &BaseLapso, c: &CatalogoLaboral) -> Result<Decimal, DominioError> {
    Ok(
        base_mensual(b, true) * Decimal::from(b.dias) / ano() * c.valor(P::DiasCesantias)?
            / Decimal::from(30),
    )
}

/// Intereses sobre las cesantías de un lapso de `dias`.
///
/// # Errors
///
/// [`DominioError::ReglaLaboral`] si falta el parámetro en el catálogo.
pub fn intereses(
    cesantias: Decimal,
    dias: u32,
    c: &CatalogoLaboral,
) -> Result<Decimal, DominioError> {
    Ok(
        cesantias * c.valor(P::InteresesCesantias)? / Decimal::ONE_HUNDRED * Decimal::from(dias)
            / ano(),
    )
}

/// Prima de servicios de los días del semestre.
///
/// # Errors
///
/// [`DominioError::ReglaLaboral`] si falta el parámetro en el catálogo.
pub fn prima(b: &BaseLapso, c: &CatalogoLaboral) -> Result<Decimal, DominioError> {
    Ok(
        base_mensual(b, true) * Decimal::from(b.dias) / ano() * c.valor(P::DiasPrima)?
            / Decimal::from(30),
    )
}

/// Días de vacaciones pendientes: los causados por los días trabajados menos los disfrutados.
///
/// # Errors
///
/// [`DominioError::ReglaLaboral`] si falta el parámetro en el catálogo.
pub fn dias_vacaciones_pendientes(
    dias_trabajados: u32,
    dias_disfrutados: Decimal,
    c: &CatalogoLaboral,
) -> Result<Decimal, DominioError> {
    let causados = Decimal::from(dias_trabajados) * c.valor(P::DiasVacaciones)? / ano();
    Ok((causados - dias_disfrutados).max(Decimal::ZERO).round_dp(2))
}

/// Valor de las vacaciones pendientes al salario diario (sin auxilio).
pub fn vacaciones_pendientes(b: &BaseLapso, dias_pendientes: Decimal) -> Decimal {
    base_mensual(b, false) / Decimal::from(30) * dias_pendientes
}

/// Datos de la terminación para la indemnización.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Terminacion {
    pub tipo: TipoContratoLaboral,
    pub motivo: MotivoTerminacion,
    pub inicio: NaiveDate,
    /// Fecha final pactada (término fijo).
    pub fin_pactado: Option<NaiveDate>,
    pub fecha: NaiveDate,
    /// Salario mensual a la terminación.
    pub salario: Decimal,
}

/// Días de salario de la indemnización por terminación sin justa causa (cero con otro motivo).
///
/// # Errors
///
/// [`DominioError::ReglaLaboral`] si falta el salario mínimo en el catálogo.
pub fn dias_indemnizacion(t: &Terminacion, c: &CatalogoLaboral) -> Result<Decimal, DominioError> {
    if t.motivo != MotivoTerminacion::SinJustaCausa {
        return Ok(Decimal::ZERO);
    }
    Ok(match t.tipo {
        TipoContratoLaboral::TerminoFijo => t
            .fin_pactado
            .filter(|f| *f > t.fecha)
            .and_then(|f| t.fecha.succ_opt().map(|d| dias_comerciales(d, f)))
            .map_or(Decimal::ZERO, Decimal::from),
        TipoContratoLaboral::ObraLabor => Decimal::from(15),
        TipoContratoLaboral::Aprendizaje => Decimal::ZERO,
        TipoContratoLaboral::TerminoIndefinido => {
            let alto = t.salario >= c.salarios_minimos(Decimal::from(10))?;
            let (primer, adicional) = if alto { (20, 15) } else { (30, 20) };
            let anos = Decimal::from(dias_comerciales(t.inicio, t.fecha)) / ano();
            let extra = (anos - Decimal::ONE).max(Decimal::ZERO);
            Decimal::from(primer) + Decimal::from(adicional) * extra
        }
    }
    .round_dp(2))
}

/// Valor de la indemnización.
///
/// # Errors
///
/// Los de [`dias_indemnizacion`].
pub fn indemnizacion(t: &Terminacion, c: &CatalogoLaboral) -> Result<Decimal, DominioError> {
    Ok(t.salario / Decimal::from(30) * dias_indemnizacion(t, c)?)
}

/// Línea de una prestación (redondeada al peso). `None` si el valor es cero.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si el valor es negativo.
pub fn linea(
    concepto: ConceptoNomina,
    cantidad: Decimal,
    valor: Decimal,
) -> Result<Option<LineaLiquidacion>, DominioError> {
    let valor = monto(valor.max(Decimal::ZERO))?;
    Ok((!valor.es_cero()).then_some(LineaLiquidacion {
        concepto,
        cantidad,
        valor,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nomina::VersionParametro;

    fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, d).expect("fecha")
    }

    fn dec(t: &str) -> Decimal {
        t.parse().expect("decimal")
    }

    fn catalogo() -> CatalogoLaboral {
        let v = |p, x: &str| VersionParametro {
            parametro: p,
            valor: dec(x),
            vigente_desde: fecha(2020, 1, 1),
        };
        CatalogoLaboral::vigente_en(
            &[
                v(P::SalarioMinimo, "1500000"),
                v(P::DiasCesantias, "30"),
                v(P::DiasPrima, "30"),
                v(P::InteresesCesantias, "12"),
                v(P::DiasVacaciones, "15"),
            ],
            fecha(2026, 12, 31),
        )
    }

    fn base(salario: &str, dias: u32) -> BaseLapso {
        BaseLapso {
            salario_actual: dec(salario),
            salario_promedio: dec(salario),
            cambio_reciente: false,
            auxilio: dec("200000"),
            variables: Decimal::ZERO,
            dias,
        }
    }

    /// Caso de referencia: semestre incompleto (ingreso el 1 de abril, prima de junio).
    #[test]
    fn prima_de_un_semestre_incompleto() {
        let b = base("1500000", 90);
        // (1.500.000 + 200.000) × 90 / 360.
        assert_eq!(prima(&b, &catalogo()).expect("prima"), dec("425000"));
    }

    #[test]
    fn cesantias_e_intereses_del_ano_completo() {
        let b = base("1500000", 360);
        let c = cesantias(&b, &catalogo()).expect("cesantías");
        assert_eq!(c, dec("1700000"));
        assert_eq!(
            intereses(c, 360, &catalogo()).expect("intereses"),
            dec("204000")
        );
    }

    /// Caso de referencia: salario variable (con horas extra y un aumento reciente: promedio).
    #[test]
    fn el_salario_variable_entra_por_su_promedio() {
        let b = BaseLapso {
            salario_actual: dec("2000000"),
            salario_promedio: dec("1800000"),
            cambio_reciente: true,
            auxilio: Decimal::ZERO,
            variables: dec("600000"),
            dias: 180,
        };
        // 1.800.000 + 600.000 / 180 × 30 = 1.900.000.
        assert_eq!(base_mensual(&b, true), dec("1900000"));
        let sin_cambio = BaseLapso {
            cambio_reciente: false,
            ..b
        };
        assert_eq!(base_mensual(&sin_cambio, true), dec("2100000"));
    }

    #[test]
    fn vacaciones_pendientes_al_retiro() {
        // 300 días trabajados causan 12,5 días; disfrutó 5.
        let pendientes =
            dias_vacaciones_pendientes(300, dec("5"), &catalogo()).expect("pendientes");
        assert_eq!(pendientes, dec("7.5"));
        assert_eq!(
            vacaciones_pendientes(&base("3000000", 300), pendientes),
            dec("750000")
        );
        assert_eq!(
            dias_vacaciones_pendientes(30, dec("15"), &catalogo()).expect("pendientes"),
            Decimal::ZERO
        );
    }

    /// Caso de referencia: retiro a mitad de mes de un contrato indefinido de año y medio, sin
    /// justa causa.
    #[test]
    fn indemnizacion_del_indefinido() {
        let t = Terminacion {
            tipo: TipoContratoLaboral::TerminoIndefinido,
            motivo: MotivoTerminacion::SinJustaCausa,
            inicio: fecha(2025, 1, 1),
            fin_pactado: None,
            fecha: fecha(2026, 6, 30),
            salario: dec("3000000"),
        };
        // 540 días = 1,5 años: 30 + 20 × 0,5 = 40 días.
        assert_eq!(dias_indemnizacion(&t, &catalogo()), Ok(dec("40")));
        assert_eq!(indemnizacion(&t, &catalogo()), Ok(dec("4000000")));
        let alto = Terminacion {
            salario: dec("15000000"),
            ..t
        };
        assert_eq!(dias_indemnizacion(&alto, &catalogo()), Ok(dec("27.5")));
        let renuncia = Terminacion {
            motivo: MotivoTerminacion::Renuncia,
            ..t
        };
        assert_eq!(indemnizacion(&renuncia, &catalogo()), Ok(Decimal::ZERO));
    }

    #[test]
    fn indemnizacion_del_termino_fijo_es_el_tiempo_que_falta() {
        let t = Terminacion {
            tipo: TipoContratoLaboral::TerminoFijo,
            motivo: MotivoTerminacion::SinJustaCausa,
            inicio: fecha(2026, 1, 1),
            fin_pactado: Some(fecha(2026, 12, 31)),
            fecha: fecha(2026, 9, 30),
            salario: dec("2000000"),
        };
        assert_eq!(dias_indemnizacion(&t, &catalogo()), Ok(dec("90")));
        let obra = Terminacion {
            tipo: TipoContratoLaboral::ObraLabor,
            fin_pactado: None,
            ..t
        };
        assert_eq!(dias_indemnizacion(&obra, &catalogo()), Ok(dec("15")));
    }

    #[test]
    fn una_linea_en_cero_no_se_agrega() {
        assert_eq!(
            linea(ConceptoNomina::Prima, Decimal::ZERO, Decimal::ZERO),
            Ok(None)
        );
        assert!(
            linea(ConceptoNomina::Prima, dec("90"), dec("425000.4"))
                .expect("línea")
                .is_some_and(|l| l.valor.valor() == dec("425000"))
        );
    }
}
