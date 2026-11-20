//! Nómina (plan 16, ADR 0025): reglas puras del módulo que se vende solo y sirve a cualquier
//! entidad (una PH con su todero, un gimnasio, la misma plataforma…).
//!
//! - **Catálogo laboral** ([`ParametroLaboral`], [`CatalogoLaboral`]): salario mínimo,
//!   recargos, aportes, prestaciones… Ningún valor vive en el código: la plataforma los carga
//!   versionados (vale la versión de mayor `vigente_desde` hasta la fecha) y cada uno dice su
//!   [`Unidad`].
//! - **Contratos** ([`TipoContratoLaboral`], [`CondicionesContrato`]): un contrato tiene fechas
//!   ([`validar_fechas_contrato`]) y condiciones versionadas que se validan contra el catálogo
//!   vigente en su fecha ([`validar_condiciones`]).
//!
//! Las reglas laborales se escribieron con la norma conocida y **las revisa el especialista
//! laboral** antes de cada entrega (aviso normativo del plan 16). Este módulo no conoce a la PH:
//! solo usa el núcleo contable ([`Monto`]) y lo común (prueba de fronteras).

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{DominioError, contabilidad::Monto};

pub mod liquidacion;
pub mod periodo;
pub mod prestaciones;

fn regla(texto: impl Into<String>) -> DominioError {
    DominioError::ReglaLaboral(texto.into())
}

// ---------------------------------------------------------------------------------------------
// Catálogo laboral
// ---------------------------------------------------------------------------------------------

/// Unidad en que se expresa el valor de un [`ParametroLaboral`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unidad {
    /// Pesos colombianos (dos decimales a lo sumo).
    Pesos,
    /// Porcentaje de 0 a 100 (`12.5` = 12,5 %).
    Porcentaje,
    /// Múltiplo del salario mínimo (`2` = dos salarios mínimos).
    SalariosMinimos,
    /// Horas a la semana.
    HorasSemana,
    /// Hora del día, de 0 a 23 (`19` = 7 p. m.).
    HoraDelDia,
    /// Días.
    Dias,
    /// Unidades de valor tributario al año.
    UvtAnuales,
}

enum_texto!(Unidad {
    Pesos => "pesos",
    Porcentaje => "porcentaje",
    SalariosMinimos => "salarios_minimos",
    HorasSemana => "horas_semana",
    HoraDelDia => "hora_del_dia",
    Dias => "dias",
    UvtAnuales => "uvt_anuales",
});

/// Parámetro del catálogo laboral. El texto es contrato con `parametros_laborales.parametro`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParametroLaboral {
    /// Salario mínimo mensual legal vigente (decreto anual).
    SalarioMinimo,
    /// Auxilio de transporte mensual (decreto anual).
    AuxilioTransporte,
    /// Hasta cuántos salarios mínimos se paga el auxilio de transporte.
    TopeAuxilioTransporte,
    /// Jornada máxima semanal (Ley 2101 de 2021, reducción gradual).
    JornadaMaximaSemanal,
    /// Hora en que empieza la jornada nocturna (Ley 2466 de 2025).
    InicioJornadaNocturna,
    /// Hora en que termina la jornada nocturna.
    FinJornadaNocturna,
    RecargoNocturno,
    HoraExtraDiurna,
    HoraExtraNocturna,
    /// Recargo dominical y festivo (Ley 2466 de 2025, gradual).
    RecargoDominicalFestivo,
    SaludEmpleador,
    SaludEmpleado,
    PensionEmpleador,
    PensionEmpleado,
    CajaCompensacion,
    Icbf,
    Sena,
    #[serde(rename = "arl_clase_1")]
    ArlClase1,
    #[serde(rename = "arl_clase_2")]
    ArlClase2,
    #[serde(rename = "arl_clase_3")]
    ArlClase3,
    #[serde(rename = "arl_clase_4")]
    ArlClase4,
    #[serde(rename = "arl_clase_5")]
    ArlClase5,
    /// Salario integral mínimo, en salarios mínimos.
    SalarioIntegralMinimo,
    /// Parte del salario integral que es base de aportes (factor salarial).
    FactorSalarioIntegral,
    /// Límite de los pagos no salariales sobre el total (Ley 1393 de 2010).
    LimitePagosNoSalariales,
    /// Días de incapacidad de origen común a cargo del empleador.
    DiasIncapacidadEmpleador,
    /// Porcentaje del salario que reconoce la EPS por incapacidad de origen común.
    IncapacidadEps,
    /// Días de salario de cesantías por año trabajado.
    DiasCesantias,
    /// Intereses anuales sobre las cesantías.
    InteresesCesantias,
    /// Días de salario de prima por año trabajado.
    DiasPrima,
    /// Días hábiles de vacaciones por año trabajado.
    DiasVacaciones,
    /// Apoyo de sostenimiento del aprendiz en etapa lectiva, en % del salario mínimo.
    ApoyoAprendizLectiva,
    /// Apoyo de sostenimiento del aprendiz en etapa productiva, en % del salario mínimo.
    ApoyoAprendizProductiva,
    /// Tope del ingreso base de cotización, en salarios mínimos.
    TopeIbc,
    /// Desde cuántos salarios mínimos de IBC no aplica la exoneración del art. 114-1 del ET.
    TopeExoneracionAportes,
    /// Renta exenta de los pagos laborales (art. 206 num. 10 del ET), en %.
    RentaExentaLaboral,
    /// Tope anual de esa renta exenta.
    TopeRentaExentaLaboral,
    /// Límite de las rentas exentas y deducciones sobre el ingreso depurado (art. 336 del ET).
    LimiteBeneficiosRetencion,
    /// Tope anual de las rentas exentas y deducciones.
    TopeBeneficiosRetencion,
}

enum_texto!(ParametroLaboral {
    SalarioMinimo => "salario_minimo",
    AuxilioTransporte => "auxilio_transporte",
    TopeAuxilioTransporte => "tope_auxilio_transporte",
    JornadaMaximaSemanal => "jornada_maxima_semanal",
    InicioJornadaNocturna => "inicio_jornada_nocturna",
    FinJornadaNocturna => "fin_jornada_nocturna",
    RecargoNocturno => "recargo_nocturno",
    HoraExtraDiurna => "hora_extra_diurna",
    HoraExtraNocturna => "hora_extra_nocturna",
    RecargoDominicalFestivo => "recargo_dominical_festivo",
    SaludEmpleador => "salud_empleador",
    SaludEmpleado => "salud_empleado",
    PensionEmpleador => "pension_empleador",
    PensionEmpleado => "pension_empleado",
    CajaCompensacion => "caja_compensacion",
    Icbf => "icbf",
    Sena => "sena",
    ArlClase1 => "arl_clase_1",
    ArlClase2 => "arl_clase_2",
    ArlClase3 => "arl_clase_3",
    ArlClase4 => "arl_clase_4",
    ArlClase5 => "arl_clase_5",
    SalarioIntegralMinimo => "salario_integral_minimo",
    FactorSalarioIntegral => "factor_salario_integral",
    LimitePagosNoSalariales => "limite_pagos_no_salariales",
    DiasIncapacidadEmpleador => "dias_incapacidad_empleador",
    IncapacidadEps => "incapacidad_eps",
    DiasCesantias => "dias_cesantias",
    InteresesCesantias => "intereses_cesantias",
    DiasPrima => "dias_prima",
    DiasVacaciones => "dias_vacaciones",
    ApoyoAprendizLectiva => "apoyo_aprendiz_lectiva",
    ApoyoAprendizProductiva => "apoyo_aprendiz_productiva",
    TopeIbc => "tope_ibc",
    TopeExoneracionAportes => "tope_exoneracion_aportes",
    RentaExentaLaboral => "renta_exenta_laboral",
    TopeRentaExentaLaboral => "tope_renta_exenta_laboral",
    LimiteBeneficiosRetencion => "limite_beneficios_retencion",
    TopeBeneficiosRetencion => "tope_beneficios_retencion",
});

impl ParametroLaboral {
    /// Unidad en que se expresa el valor.
    pub const fn unidad(self) -> Unidad {
        use ParametroLaboral as P;
        match self {
            P::SalarioMinimo | P::AuxilioTransporte => Unidad::Pesos,
            P::TopeAuxilioTransporte
            | P::SalarioIntegralMinimo
            | P::TopeIbc
            | P::TopeExoneracionAportes => Unidad::SalariosMinimos,
            P::TopeRentaExentaLaboral | P::TopeBeneficiosRetencion => Unidad::UvtAnuales,
            P::JornadaMaximaSemanal => Unidad::HorasSemana,
            P::InicioJornadaNocturna | P::FinJornadaNocturna => Unidad::HoraDelDia,
            P::DiasIncapacidadEmpleador | P::DiasCesantias | P::DiasPrima | P::DiasVacaciones => {
                Unidad::Dias
            }
            P::RecargoNocturno
            | P::HoraExtraDiurna
            | P::HoraExtraNocturna
            | P::RecargoDominicalFestivo
            | P::SaludEmpleador
            | P::SaludEmpleado
            | P::PensionEmpleador
            | P::PensionEmpleado
            | P::CajaCompensacion
            | P::Icbf
            | P::Sena
            | P::ArlClase1
            | P::ArlClase2
            | P::ArlClase3
            | P::ArlClase4
            | P::ArlClase5
            | P::FactorSalarioIntegral
            | P::LimitePagosNoSalariales
            | P::IncapacidadEps
            | P::InteresesCesantias
            | P::ApoyoAprendizLectiva
            | P::ApoyoAprendizProductiva
            | P::RentaExentaLaboral
            | P::LimiteBeneficiosRetencion => Unidad::Porcentaje,
        }
    }

    /// Valida el valor según la unidad y lo deja con la escala de la BD (cuatro decimales).
    ///
    /// # Errors
    ///
    /// [`DominioError::ReglaLaboral`] si el valor no cabe en la unidad (porcentaje de 0 a 100,
    /// hora del día entera de 0 a 23, pesos positivos con dos decimales…).
    pub fn validar_valor(self, valor: Decimal) -> Result<Decimal, DominioError> {
        let cien = Decimal::ONE_HUNDRED;
        let entero = valor.fract().is_zero();
        let valido = match self.unidad() {
            Unidad::Pesos => valor > Decimal::ZERO && valor.normalize().scale() <= 2,
            Unidad::Porcentaje => valor >= Decimal::ZERO && valor <= cien,
            Unidad::SalariosMinimos => valor > Decimal::ZERO && valor <= cien,
            Unidad::HorasSemana => valor > Decimal::ZERO && valor <= Decimal::from(168),
            Unidad::HoraDelDia => entero && valor >= Decimal::ZERO && valor <= Decimal::from(23),
            Unidad::Dias => valor >= Decimal::ZERO && valor <= Decimal::from(366),
            Unidad::UvtAnuales => valor > Decimal::ZERO && valor <= Decimal::from(100_000),
        };
        if !valido || valor.normalize().scale() > 4 {
            return Err(regla(format!(
                "el valor de `{self}` no es válido en {}",
                self.unidad()
            )));
        }
        Ok(valor.round_dp(4))
    }

    /// Tarifa de ARL de la clase de riesgo.
    pub const fn arl(clase: ClaseRiesgo) -> Self {
        match clase {
            ClaseRiesgo::I => Self::ArlClase1,
            ClaseRiesgo::Ii => Self::ArlClase2,
            ClaseRiesgo::Iii => Self::ArlClase3,
            ClaseRiesgo::Iv => Self::ArlClase4,
            ClaseRiesgo::V => Self::ArlClase5,
        }
    }
}

/// Versión de un parámetro: el valor y desde cuándo vale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionParametro {
    pub parametro: ParametroLaboral,
    pub valor: Decimal,
    pub vigente_desde: NaiveDate,
}

/// Valores del catálogo laboral vigentes en una fecha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogoLaboral {
    fecha: NaiveDate,
    valores: BTreeMap<ParametroLaboral, Decimal>,
}

impl CatalogoLaboral {
    /// Toma, de cada parámetro, la versión de mayor `vigente_desde` que no pase de `fecha`.
    pub fn vigente_en(versiones: &[VersionParametro], fecha: NaiveDate) -> Self {
        let mut elegidas: BTreeMap<ParametroLaboral, (NaiveDate, Decimal)> = BTreeMap::new();
        for v in versiones.iter().filter(|v| v.vigente_desde <= fecha) {
            let actual = elegidas.get(&v.parametro).map(|(d, _)| *d);
            if actual.is_none_or(|d| v.vigente_desde > d) {
                elegidas.insert(v.parametro, (v.vigente_desde, v.valor));
            }
        }
        Self {
            fecha,
            valores: elegidas.into_iter().map(|(p, (_, v))| (p, v)).collect(),
        }
    }

    /// Fecha del catálogo.
    pub fn fecha(&self) -> NaiveDate {
        self.fecha
    }

    /// Valor vigente del parámetro.
    ///
    /// # Errors
    ///
    /// [`DominioError::ReglaLaboral`] si la plataforma no ha cargado el parámetro para la fecha.
    pub fn valor(&self, parametro: ParametroLaboral) -> Result<Decimal, DominioError> {
        self.valores.get(&parametro).copied().ok_or_else(|| {
            regla(format!(
                "el catálogo laboral no tiene `{parametro}` vigente el {}",
                self.fecha
            ))
        })
    }

    /// `salarios` salarios mínimos vigentes, en pesos.
    ///
    /// # Errors
    ///
    /// Los de [`Self::valor`].
    pub fn salarios_minimos(&self, salarios: Decimal) -> Result<Decimal, DominioError> {
        Ok(self.valor(ParametroLaboral::SalarioMinimo)? * salarios)
    }
}

/// Tipo de administradora de la seguridad social.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoAdministradora {
    /// Entidad promotora de salud.
    Eps,
    /// Administradora de fondos de pensiones (incluye Colpensiones).
    Afp,
    /// Fondo de cesantías.
    Cesantias,
    /// Caja de compensación familiar.
    Ccf,
    /// Administradora de riesgos laborales.
    Arl,
}

enum_texto!(TipoAdministradora {
    Eps => "eps",
    Afp => "afp",
    Cesantias => "cesantias",
    Ccf => "ccf",
    Arl => "arl",
});

/// Valida el código PILA de una administradora: 2 a 12 mayúsculas, dígitos o guiones.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si no cumple.
pub fn validar_codigo_administradora(codigo: &str) -> Result<String, DominioError> {
    let c = codigo.trim().to_uppercase();
    if (2..=12).contains(&c.len())
        && c.chars()
            .all(|x| x.is_ascii_uppercase() || x.is_ascii_digit() || x == '-')
    {
        Ok(c)
    } else {
        Err(DominioError::DatoInvalido {
            campo: "codigo",
            regla: "de 2 a 12 mayúsculas, dígitos o guiones",
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Empleados y contratos
// ---------------------------------------------------------------------------------------------

/// Código interno de un empleado en la entidad: 1 a 30 caracteres `A-Z a-z 0-9 . _ -`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CodigoEmpleado(String);

impl CodigoEmpleado {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no cumple el formato.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let v = valor.trim();
        if (1..=30).contains(&v.len())
            && v.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            Ok(Self(v.to_owned()))
        } else {
            Err(DominioError::DatoInvalido {
                campo: "codigo",
                regla: "de 1 a 30 letras, dígitos, `.`, `_` o `-`",
            })
        }
    }

    /// Código tal como se guarda.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CodigoEmpleado {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, DominioError> {
        Self::new(&v)
    }
}

impl From<CodigoEmpleado> for String {
    fn from(c: CodigoEmpleado) -> Self {
        c.0
    }
}

/// Estado de un empleado, derivado de sus contratos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoEmpleado {
    /// Tiene un contrato vigente o que empieza después.
    Activo,
    /// Todos sus contratos terminaron o vencieron.
    Retirado,
}

enum_texto!(EstadoEmpleado {
    Activo => "activo",
    Retirado => "retirado",
});

/// Tipo de contrato laboral (código de la nómina electrónica de la DIAN en
/// [`Self::codigo_dian`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoContratoLaboral {
    TerminoFijo,
    TerminoIndefinido,
    ObraLabor,
    /// Contrato de aprendizaje (SENA): se paga un apoyo de sostenimiento.
    Aprendizaje,
}

enum_texto!(TipoContratoLaboral {
    TerminoFijo => "termino_fijo",
    TerminoIndefinido => "termino_indefinido",
    ObraLabor => "obra_labor",
    Aprendizaje => "aprendizaje",
});

impl TipoContratoLaboral {
    /// Código del tipo de contrato en la nómina electrónica.
    pub const fn codigo_dian(self) -> &'static str {
        match self {
            Self::TerminoFijo => "1",
            Self::TerminoIndefinido => "2",
            Self::ObraLabor => "3",
            Self::Aprendizaje => "4",
        }
    }

    /// ¿Lleva fecha final pactada?
    pub const fn lleva_fecha_fin(self) -> bool {
        matches!(self, Self::TerminoFijo | Self::Aprendizaje)
    }
}

/// Motivo de la terminación de un contrato (la indemnización se calcula con él en la fase 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotivoTerminacion {
    Renuncia,
    MutuoAcuerdo,
    /// Vence el término pactado (término fijo o aprendizaje).
    VencimientoTermino,
    /// Termina la obra o labor contratada.
    FinObra,
    JustaCausa,
    SinJustaCausa,
    PeriodoPrueba,
    Fallecimiento,
    Otro,
}

enum_texto!(MotivoTerminacion {
    Renuncia => "renuncia",
    MutuoAcuerdo => "mutuo_acuerdo",
    VencimientoTermino => "vencimiento_termino",
    FinObra => "fin_obra",
    JustaCausa => "justa_causa",
    SinJustaCausa => "sin_justa_causa",
    PeriodoPrueba => "periodo_prueba",
    Fallecimiento => "fallecimiento",
    Otro => "otro",
});

/// Tipo de salario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoSalario {
    Ordinario,
    /// Remunera el trabajo y las prestaciones (sin cesantías ni prima aparte).
    Integral,
    /// Apoyo de sostenimiento del contrato de aprendizaje.
    ApoyoSostenimiento,
}

enum_texto!(TipoSalario {
    Ordinario => "ordinario",
    Integral => "integral",
    ApoyoSostenimiento => "apoyo_sostenimiento",
});

/// Etapa del aprendiz SENA (cambia el apoyo y las afiliaciones).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EtapaAprendizaje {
    Lectiva,
    Productiva,
}

enum_texto!(EtapaAprendizaje {
    Lectiva => "lectiva",
    Productiva => "productiva",
});

/// Jornada laboral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Jornada {
    Completa,
    /// Tiempo parcial: lleva las horas semanales pactadas.
    Parcial,
}

enum_texto!(Jornada {
    Completa => "completa",
    Parcial => "parcial",
});

/// Periodicidad de pago de la nómina.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodicidadPago {
    Mensual,
    Quincenal,
}

enum_texto!(PeriodicidadPago {
    Mensual => "mensual",
    Quincenal => "quincenal",
});

/// Medio de pago del salario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MedioPago {
    /// Consignación a la cuenta del empleado (lleva banco, tipo y número).
    Transferencia,
    Cheque,
    Efectivo,
}

enum_texto!(MedioPago {
    Transferencia => "transferencia",
    Cheque => "cheque",
    Efectivo => "efectivo",
});

/// Clase de riesgo laboral (I a V) del puesto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ClaseRiesgo {
    #[serde(rename = "1")]
    I,
    #[serde(rename = "2")]
    Ii,
    #[serde(rename = "3")]
    Iii,
    #[serde(rename = "4")]
    Iv,
    #[serde(rename = "5")]
    V,
}

enum_texto!(ClaseRiesgo {
    I => "1",
    Ii => "2",
    Iii => "3",
    Iv => "4",
    V => "5",
});

impl ClaseRiesgo {
    /// Número de la clase (1 a 5), como se guarda en la BD.
    pub const fn numero(self) -> i16 {
        match self {
            Self::I => 1,
            Self::Ii => 2,
            Self::Iii => 3,
            Self::Iv => 4,
            Self::V => 5,
        }
    }

    /// Clase por su número.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no está entre 1 y 5.
    pub fn de_numero(n: i16) -> Result<Self, DominioError> {
        match n {
            1 => Ok(Self::I),
            2 => Ok(Self::Ii),
            3 => Ok(Self::Iii),
            4 => Ok(Self::Iv),
            5 => Ok(Self::V),
            _ => Err(DominioError::DatoInvalido {
                campo: "clase_riesgo",
                regla: "de 1 a 5",
            }),
        }
    }
}

/// Valida las fechas de un contrato nuevo según su tipo: término fijo y aprendizaje llevan
/// fecha final; indefinido y obra o labor, no; la obra lleva su descripción.
///
/// # Errors
///
/// [`DominioError::ReglaLaboral`] con la regla incumplida.
pub fn validar_fechas_contrato(
    tipo: TipoContratoLaboral,
    inicio: NaiveDate,
    fin: Option<NaiveDate>,
    descripcion_obra: Option<&str>,
) -> Result<(), DominioError> {
    match (tipo.lleva_fecha_fin(), fin) {
        (true, None) => return Err(regla(format!("el contrato `{tipo}` lleva fecha final"))),
        (false, Some(_)) => {
            return Err(regla(format!("el contrato `{tipo}` no lleva fecha final")));
        }
        (true, Some(f)) if f < inicio => {
            return Err(regla("la fecha final es anterior al inicio"));
        }
        _ => {}
    }
    if (tipo == TipoContratoLaboral::ObraLabor) != descripcion_obra.is_some() {
        return Err(regla(
            "solo el contrato por obra o labor lleva la descripción de la obra",
        ));
    }
    Ok(())
}

/// Valida la terminación de un contrato.
///
/// # Errors
///
/// [`DominioError::ReglaLaboral`] si la fecha cae fuera del contrato o el motivo no
/// corresponde al tipo (el vencimiento del término solo en término fijo o aprendizaje y el fin
/// de la obra solo en obra o labor).
pub fn validar_terminacion(
    tipo: TipoContratoLaboral,
    inicio: NaiveDate,
    fin: Option<NaiveDate>,
    fecha: NaiveDate,
    motivo: MotivoTerminacion,
) -> Result<(), DominioError> {
    if fecha < inicio || fin.is_some_and(|f| fecha > f) {
        return Err(regla(
            "la terminación cae fuera de las fechas del contrato (prorróguelo primero)",
        ));
    }
    let motivo_ok = match motivo {
        MotivoTerminacion::VencimientoTermino => tipo.lleva_fecha_fin(),
        MotivoTerminacion::FinObra => tipo == TipoContratoLaboral::ObraLabor,
        _ => true,
    };
    if motivo_ok {
        Ok(())
    } else {
        Err(regla(format!(
            "el motivo `{motivo}` no aplica a un contrato `{tipo}`"
        )))
    }
}

/// Valida una prórroga: solo de un contrato a término fijo y hacia adelante.
///
/// # Errors
///
/// [`DominioError::ReglaLaboral`] si no es a término fijo o la fecha no es posterior.
pub fn validar_prorroga(
    tipo: TipoContratoLaboral,
    fin_actual: Option<NaiveDate>,
    nueva: NaiveDate,
) -> Result<(), DominioError> {
    match (tipo, fin_actual) {
        (TipoContratoLaboral::TerminoFijo, Some(f)) if nueva > f => Ok(()),
        (TipoContratoLaboral::TerminoFijo, _) => {
            Err(regla("la prórroga lleva la fecha final más adelante"))
        }
        _ => Err(regla("solo se prorroga un contrato a término fijo")),
    }
}

/// Afiliaciones de una versión del contrato (qué administradoras tiene).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Afiliaciones {
    pub eps: bool,
    pub afp: bool,
    pub cesantias: bool,
    pub ccf: bool,
    pub arl: bool,
}

/// Condiciones de una versión del contrato que dependen de la norma.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CondicionesContrato {
    pub tipo_contrato: TipoContratoLaboral,
    pub salario: Monto,
    pub tipo_salario: TipoSalario,
    pub etapa_aprendizaje: Option<EtapaAprendizaje>,
    pub jornada: Jornada,
    /// Solo en jornada parcial.
    pub horas_semanales: Option<Decimal>,
    pub pensionado: bool,
    pub afiliaciones: Afiliaciones,
}

/// Valida las condiciones contra el catálogo laboral vigente en la fecha de la versión:
///
/// - el aprendizaje va con el apoyo de sostenimiento y su etapa; los demás contratos, no;
/// - la jornada parcial lleva horas por debajo de la jornada máxima; la completa, no;
/// - el salario ordinario no baja del mínimo (proporcional a las horas en jornada parcial), el
///   integral no baja de su mínimo en salarios mínimos y el apoyo no baja del porcentaje de su
///   etapa;
/// - afiliaciones: EPS siempre; pensiones salvo pensionados y aprendices; cesantías con salario
///   ordinario; caja de compensación salvo aprendices; ARL salvo el aprendiz en etapa lectiva.
///
/// # Errors
///
/// [`DominioError::ReglaLaboral`] con la regla incumplida (sin el valor del salario), o si
/// falta en el catálogo un parámetro que la regla necesita.
pub fn validar_condiciones(
    c: &CondicionesContrato,
    catalogo: &CatalogoLaboral,
) -> Result<(), DominioError> {
    use ParametroLaboral as P;
    let aprendiz = c.tipo_contrato == TipoContratoLaboral::Aprendizaje;
    if aprendiz != (c.tipo_salario == TipoSalario::ApoyoSostenimiento)
        || aprendiz != c.etapa_aprendizaje.is_some()
    {
        return Err(regla(
            "el contrato de aprendizaje va con el apoyo de sostenimiento y su etapa, y solo él",
        ));
    }
    let jornada_maxima = catalogo.valor(P::JornadaMaximaSemanal)?;
    let proporcion = match (c.jornada, c.horas_semanales) {
        (Jornada::Completa, None) => Decimal::ONE,
        (Jornada::Parcial, Some(h)) if h > Decimal::ZERO && h < jornada_maxima => {
            h / jornada_maxima
        }
        (Jornada::Parcial, Some(_)) => {
            return Err(regla(
                "las horas de la jornada parcial son menos que la jornada máxima",
            ));
        }
        _ => {
            return Err(regla("solo la jornada parcial lleva las horas semanales"));
        }
    };
    let salario = c.salario.valor();
    let (minimo, que) = match (c.tipo_salario, c.etapa_aprendizaje) {
        (TipoSalario::Ordinario, _) => (
            catalogo.salarios_minimos(Decimal::ONE)? * proporcion,
            "el salario mínimo",
        ),
        (TipoSalario::Integral, _) => (
            catalogo.salarios_minimos(catalogo.valor(P::SalarioIntegralMinimo)?)?,
            "el mínimo del salario integral",
        ),
        (TipoSalario::ApoyoSostenimiento, etapa) => {
            let p = if etapa == Some(EtapaAprendizaje::Productiva) {
                P::ApoyoAprendizProductiva
            } else {
                P::ApoyoAprendizLectiva
            };
            (
                catalogo.salarios_minimos(catalogo.valor(p)? / Decimal::ONE_HUNDRED)? * proporcion,
                "el apoyo de sostenimiento de la etapa",
            )
        }
    };
    if salario < minimo.round_dp(2) {
        return Err(regla(format!("la remuneración no alcanza {que}")));
    }
    let a = c.afiliaciones;
    let lectiva = c.etapa_aprendizaje == Some(EtapaAprendizaje::Lectiva);
    let exigidas = [
        (a.eps, true, "EPS"),
        (a.afp, !aprendiz && !c.pensionado, "fondo de pensiones"),
        (
            a.cesantias,
            c.tipo_salario == TipoSalario::Ordinario,
            "fondo de cesantías",
        ),
        (a.ccf, !aprendiz, "caja de compensación"),
        (a.arl, !lectiva, "ARL"),
    ];
    if let Some((_, _, nombre)) = exigidas.iter().find(|(tiene, exige, _)| *exige && !*tiene) {
        return Err(regla(format!("falta la afiliación a {nombre}")));
    }
    if a.cesantias && c.tipo_salario != TipoSalario::Ordinario {
        return Err(regla(
            "el salario integral y el apoyo de sostenimiento no llevan fondo de cesantías",
        ));
    }
    if a.arl && lectiva {
        return Err(regla("el aprendiz en etapa lectiva no se afilia a ARL"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, d).expect("fecha válida")
    }

    fn dec(t: &str) -> Decimal {
        t.parse().expect("decimal")
    }

    fn version(p: ParametroLaboral, valor: &str, desde: NaiveDate) -> VersionParametro {
        VersionParametro {
            parametro: p,
            valor: dec(valor),
            vigente_desde: desde,
        }
    }

    /// Catálogo de referencia para las pruebas (valores ilustrativos, no oficiales).
    fn catalogo() -> CatalogoLaboral {
        use ParametroLaboral as P;
        let d = fecha(2026, 1, 1);
        CatalogoLaboral::vigente_en(
            &[
                version(P::SalarioMinimo, "1500000", d),
                version(P::JornadaMaximaSemanal, "42", d),
                version(P::SalarioIntegralMinimo, "13", d),
                version(P::ApoyoAprendizLectiva, "75", d),
                version(P::ApoyoAprendizProductiva, "100", d),
            ],
            fecha(2026, 8, 1),
        )
    }

    fn todas() -> Afiliaciones {
        Afiliaciones {
            eps: true,
            afp: true,
            cesantias: true,
            ccf: true,
            arl: true,
        }
    }

    fn ordinario(salario: &str) -> CondicionesContrato {
        CondicionesContrato {
            tipo_contrato: TipoContratoLaboral::TerminoIndefinido,
            salario: Monto::new(dec(salario)).expect("monto"),
            tipo_salario: TipoSalario::Ordinario,
            etapa_aprendizaje: None,
            jornada: Jornada::Completa,
            horas_semanales: None,
            pensionado: false,
            afiliaciones: todas(),
        }
    }

    fn aprendiz(etapa: EtapaAprendizaje, salario: &str) -> CondicionesContrato {
        CondicionesContrato {
            tipo_contrato: TipoContratoLaboral::Aprendizaje,
            tipo_salario: TipoSalario::ApoyoSostenimiento,
            etapa_aprendizaje: Some(etapa),
            afiliaciones: Afiliaciones {
                eps: true,
                arl: etapa == EtapaAprendizaje::Productiva,
                ..Afiliaciones::default()
            },
            ..ordinario(salario)
        }
    }

    fn error(r: Result<(), DominioError>) -> String {
        match r {
            Err(DominioError::ReglaLaboral(t)) => t,
            otro => panic!("se esperaba una regla laboral: {otro:?}"),
        }
    }

    #[test]
    fn el_json_usa_el_mismo_texto_que_la_base() {
        for p in ParametroLaboral::TODOS {
            let json = serde_json::to_value(p).expect("json");
            assert_eq!(json, serde_json::Value::from(p.as_str()), "{p:?}");
        }
    }

    #[test]
    fn textos_persistidos_ida_y_vuelta() {
        for p in ParametroLaboral::TODOS {
            assert_eq!(ParametroLaboral::try_from(p.as_str()), Ok(*p));
        }
        for t in TipoContratoLaboral::TODOS {
            assert_eq!(TipoContratoLaboral::try_from(t.as_str()), Ok(*t));
        }
        for c in ClaseRiesgo::TODOS {
            assert_eq!(ClaseRiesgo::de_numero(c.numero()), Ok(*c));
        }
        assert!(ClaseRiesgo::de_numero(6).is_err());
    }

    #[test]
    fn el_catalogo_toma_la_ultima_version_hasta_la_fecha() {
        use ParametroLaboral as P;
        let versiones = [
            version(P::RecargoDominicalFestivo, "80", fecha(2025, 7, 1)),
            version(P::RecargoDominicalFestivo, "90", fecha(2026, 7, 1)),
            version(P::RecargoDominicalFestivo, "100", fecha(2027, 7, 1)),
        ];
        let c = CatalogoLaboral::vigente_en(&versiones, fecha(2026, 12, 31));
        assert_eq!(c.valor(P::RecargoDominicalFestivo), Ok(dec("90")));
        let antes = CatalogoLaboral::vigente_en(&versiones, fecha(2026, 6, 30));
        assert_eq!(antes.valor(P::RecargoDominicalFestivo), Ok(dec("80")));
    }

    #[test]
    fn sin_el_parametro_vigente_no_hay_valor() {
        let c = CatalogoLaboral::vigente_en(
            &[version(
                ParametroLaboral::SalarioMinimo,
                "1500000",
                fecha(2027, 1, 1),
            )],
            fecha(2026, 12, 31),
        );
        assert!(
            error(c.valor(ParametroLaboral::SalarioMinimo).map(|_| ())).contains("salario_minimo")
        );
    }

    #[test]
    fn cada_unidad_acota_su_valor() {
        use ParametroLaboral as P;
        assert!(P::SalarioMinimo.validar_valor(dec("1423500")).is_ok());
        assert!(P::SalarioMinimo.validar_valor(dec("0")).is_err());
        assert!(P::SaludEmpleado.validar_valor(dec("4")).is_ok());
        assert!(P::SaludEmpleado.validar_valor(dec("100.5")).is_err());
        assert!(P::InicioJornadaNocturna.validar_valor(dec("19")).is_ok());
        assert!(P::InicioJornadaNocturna.validar_valor(dec("19.5")).is_err());
        assert!(P::InicioJornadaNocturna.validar_valor(dec("24")).is_err());
        assert!(P::ArlClase1.validar_valor(dec("0.522")).is_ok());
        assert!(P::ArlClase1.validar_valor(dec("0.52211")).is_err());
        assert_eq!(P::arl(ClaseRiesgo::Iii), P::ArlClase3);
    }

    #[test]
    fn las_fechas_dependen_del_tipo_de_contrato() {
        use TipoContratoLaboral as T;
        let i = fecha(2026, 2, 1);
        let f = Some(fecha(2027, 1, 31));
        assert!(validar_fechas_contrato(T::TerminoFijo, i, f, None).is_ok());
        assert!(validar_fechas_contrato(T::TerminoFijo, i, None, None).is_err());
        assert!(validar_fechas_contrato(T::TerminoIndefinido, i, None, None).is_ok());
        assert!(validar_fechas_contrato(T::TerminoIndefinido, i, f, None).is_err());
        assert!(validar_fechas_contrato(T::ObraLabor, i, None, Some("Pintura")).is_ok());
        assert!(validar_fechas_contrato(T::ObraLabor, i, None, None).is_err());
        assert!(validar_fechas_contrato(T::Aprendizaje, i, f, None).is_ok());
        assert!(validar_fechas_contrato(T::TerminoFijo, i, Some(fecha(2026, 1, 1)), None).is_err());
    }

    #[test]
    fn la_terminacion_respeta_fechas_y_motivo() {
        use {MotivoTerminacion as M, TipoContratoLaboral as T};
        let i = fecha(2026, 2, 1);
        let f = Some(fecha(2026, 12, 31));
        assert!(
            validar_terminacion(
                T::TerminoFijo,
                i,
                f,
                fecha(2026, 12, 31),
                M::VencimientoTermino
            )
            .is_ok()
        );
        assert!(validar_terminacion(T::TerminoFijo, i, f, fecha(2027, 1, 1), M::Renuncia).is_err());
        assert!(
            validar_terminacion(T::TerminoFijo, i, f, fecha(2026, 1, 31), M::Renuncia).is_err()
        );
        assert!(
            validar_terminacion(
                T::TerminoIndefinido,
                i,
                None,
                fecha(2026, 5, 1),
                M::VencimientoTermino
            )
            .is_err()
        );
        assert!(validar_terminacion(T::ObraLabor, i, None, fecha(2026, 5, 1), M::FinObra).is_ok());
        assert!(
            validar_terminacion(T::TerminoIndefinido, i, None, fecha(2026, 5, 1), M::FinObra)
                .is_err()
        );
    }

    #[test]
    fn solo_se_prorroga_el_termino_fijo_hacia_adelante() {
        use TipoContratoLaboral as T;
        let f = Some(fecha(2026, 12, 31));
        assert!(validar_prorroga(T::TerminoFijo, f, fecha(2027, 12, 31)).is_ok());
        assert!(validar_prorroga(T::TerminoFijo, f, fecha(2026, 12, 31)).is_err());
        assert!(validar_prorroga(T::Aprendizaje, f, fecha(2027, 12, 31)).is_err());
        assert!(validar_prorroga(T::TerminoIndefinido, None, fecha(2027, 12, 31)).is_err());
    }

    #[test]
    fn el_salario_ordinario_no_baja_del_minimo() {
        assert!(validar_condiciones(&ordinario("1500000"), &catalogo()).is_ok());
        let e = error(validar_condiciones(&ordinario("1499999.99"), &catalogo()));
        assert!(
            e.contains("salario mínimo") && !e.contains("1499999"),
            "{e}"
        );
    }

    #[test]
    fn la_jornada_parcial_paga_el_minimo_proporcional() {
        let parcial = |salario: &str, horas: &str| CondicionesContrato {
            jornada: Jornada::Parcial,
            horas_semanales: Some(dec(horas)),
            ..ordinario(salario)
        };
        // 21 de 42 horas: medio salario mínimo.
        assert!(validar_condiciones(&parcial("750000", "21"), &catalogo()).is_ok());
        assert!(validar_condiciones(&parcial("749999", "21"), &catalogo()).is_err());
        assert!(validar_condiciones(&parcial("1500000", "42"), &catalogo()).is_err());
        let completa_con_horas = CondicionesContrato {
            horas_semanales: Some(dec("40")),
            ..ordinario("1500000")
        };
        assert!(validar_condiciones(&completa_con_horas, &catalogo()).is_err());
    }

    #[test]
    fn el_salario_integral_exige_su_minimo_y_no_lleva_cesantias() {
        let integral = |salario: &str, cesantias: bool| CondicionesContrato {
            tipo_salario: TipoSalario::Integral,
            afiliaciones: Afiliaciones {
                cesantias,
                ..todas()
            },
            ..ordinario(salario)
        };
        assert!(validar_condiciones(&integral("19500000", false), &catalogo()).is_ok());
        assert!(validar_condiciones(&integral("19499999", false), &catalogo()).is_err());
        assert!(validar_condiciones(&integral("19500000", true), &catalogo()).is_err());
    }

    #[test]
    fn el_aprendiz_recibe_el_apoyo_de_su_etapa() {
        use EtapaAprendizaje::{Lectiva, Productiva};
        assert!(validar_condiciones(&aprendiz(Lectiva, "1125000"), &catalogo()).is_ok());
        assert!(validar_condiciones(&aprendiz(Lectiva, "1124999"), &catalogo()).is_err());
        assert!(validar_condiciones(&aprendiz(Productiva, "1500000"), &catalogo()).is_ok());
        assert!(validar_condiciones(&aprendiz(Productiva, "1125000"), &catalogo()).is_err());
    }

    #[test]
    fn el_aprendizaje_y_el_apoyo_van_juntos() {
        let apoyo_sin_aprendizaje = CondicionesContrato {
            tipo_salario: TipoSalario::ApoyoSostenimiento,
            etapa_aprendizaje: Some(EtapaAprendizaje::Productiva),
            ..ordinario("1500000")
        };
        assert!(validar_condiciones(&apoyo_sin_aprendizaje, &catalogo()).is_err());
        let aprendiz_con_salario = CondicionesContrato {
            tipo_salario: TipoSalario::Ordinario,
            ..aprendiz(EtapaAprendizaje::Productiva, "1500000")
        };
        assert!(validar_condiciones(&aprendiz_con_salario, &catalogo()).is_err());
    }

    #[test]
    fn las_afiliaciones_dependen_del_contrato() {
        let sin = |f: fn(&mut Afiliaciones)| {
            let mut c = ordinario("1500000");
            f(&mut c.afiliaciones);
            c
        };
        assert!(error(validar_condiciones(&sin(|a| a.eps = false), &catalogo())).contains("EPS"));
        assert!(validar_condiciones(&sin(|a| a.afp = false), &catalogo()).is_err());
        assert!(validar_condiciones(&sin(|a| a.ccf = false), &catalogo()).is_err());
        assert!(validar_condiciones(&sin(|a| a.arl = false), &catalogo()).is_err());
        assert!(validar_condiciones(&sin(|a| a.cesantias = false), &catalogo()).is_err());
        let pensionado = CondicionesContrato {
            pensionado: true,
            ..sin(|a| a.afp = false)
        };
        assert!(validar_condiciones(&pensionado, &catalogo()).is_ok());
        let mut lectiva_con_arl = aprendiz(EtapaAprendizaje::Lectiva, "1125000");
        lectiva_con_arl.afiliaciones.arl = true;
        assert!(validar_condiciones(&lectiva_con_arl, &catalogo()).is_err());
        let mut productiva_sin_arl = aprendiz(EtapaAprendizaje::Productiva, "1500000");
        productiva_sin_arl.afiliaciones.arl = false;
        assert!(validar_condiciones(&productiva_sin_arl, &catalogo()).is_err());
    }

    #[test]
    fn sin_catalogo_no_se_valida_el_contrato() {
        let vacio = CatalogoLaboral::vigente_en(&[], fecha(2026, 8, 1));
        assert!(error(validar_condiciones(&ordinario("1500000"), &vacio)).contains("catálogo"));
    }

    #[test]
    fn el_codigo_del_empleado_tiene_formato() {
        assert!(CodigoEmpleado::new("EMP-001").is_ok());
        assert!(CodigoEmpleado::new("").is_err());
        assert!(CodigoEmpleado::new("con espacio").is_err());
        assert_eq!(
            validar_codigo_administradora(" eps037 "),
            Ok("EPS037".into())
        );
        assert!(validar_codigo_administradora("x").is_err());
    }
}
