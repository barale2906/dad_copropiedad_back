//! Liquidación de la nómina de un contrato en un periodo (plan 16 fase 2): reglas puras, sin IO.
//!
//! La aplicación arma la [`EntradaLiquidacion`] (los tramos del contrato con su versión vigente,
//! las novedades del periodo, el catálogo laboral, las tablas y la UVT) y [`liquidar`] devuelve
//! las líneas de devengos, deducciones y aportes del empleador, el IBC y el neto.
//!
//! Convenciones (marcadas para la revisión del especialista laboral):
//! - **Días comerciales**: todo mes tiene 30 días ([`dias_comerciales`]); el día 31 no cuenta y
//!   febrero se completa a 30.
//! - **Valor de la hora** = salario ÷ (jornada semanal ÷ 6 × 30) (210 horas con 42 a la semana).
//!   Recargos y extras salen del catálogo; los dominicales y festivos se validan con
//!   [`crate::calendario`].
//! - **Incapacidad de origen común**: el porcentaje del catálogo sobre el salario diario, nunca
//!   por debajo del salario mínimo diario; la laboral y las licencias remuneradas, al 100 %.
//! - **Auxilio de transporte**: con salario ordinario hasta el tope en salarios mínimos, por los
//!   días efectivamente laborados.
//! - **IBC**: devengos salariales (70 % en el salario integral) más el exceso de los pagos no
//!   salariales sobre el límite (Ley 1393 de 2010), entre un salario mínimo proporcional (jornada
//!   completa) y el tope; redondeado al peso.
//! - **Aportes**: salud y pensión del empleado y del empleador, fondo de solidaridad por tramos,
//!   ARL por clase, caja, ICBF y SENA; con la exoneración del art. 114-1 del ET por debajo de su
//!   tope. El aprendiz cotiza solo salud (todo a cargo del empleador) y ARL en etapa productiva.
//! - **Retención en la fuente** (procedimiento 1, art. 383 del ET) sobre el mes: ingresos
//!   laborales − aportes obligatorios − renta exenta (con su tope y el límite global) → tabla en
//!   UVT, aproximada al múltiplo de mil más cercano.
//!
//! Todo en [`Decimal`]; cada línea se redondea al peso.

use std::collections::BTreeMap;

use chrono::{Datelike, Days, NaiveDate, Weekday};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

use super::{
    CatalogoLaboral, ClaseRiesgo, EtapaAprendizaje, Jornada, ParametroLaboral as P, TipoSalario,
};
use crate::{DominioError, calendario::es_festivo, contabilidad::Monto};

fn regla(texto: impl Into<String>) -> DominioError {
    DominioError::ReglaLaboral(texto.into())
}

fn cien() -> Decimal {
    Decimal::ONE_HUNDRED
}

fn treinta() -> Decimal {
    Decimal::from(30)
}

/// Redondea al peso (mitad hacia arriba).
fn al_peso(v: Decimal) -> Decimal {
    v.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
}

// ---------------------------------------------------------------------------------------------
// Días comerciales
// ---------------------------------------------------------------------------------------------

fn ultimo_del_mes(d: NaiveDate) -> bool {
    d.checked_add_days(Days::new(1))
        .is_none_or(|siguiente| siguiente.month() != d.month())
}

/// Días comerciales entre dos fechas (inclusive): cada mes vale 30 días, el día 31 no cuenta y
/// el último día de febrero completa hasta 30.
pub fn dias_comerciales(desde: NaiveDate, hasta: NaiveDate) -> u32 {
    let mut dias = 0;
    let mut d = desde;
    while d <= hasta {
        if d.day() <= 30 {
            dias += 1;
        }
        if ultimo_del_mes(d) && d.day() < 30 {
            dias += 30 - d.day();
        }
        match d.checked_add_days(Days::new(1)) {
            Some(s) => d = s,
            None => break,
        }
    }
    dias
}

fn interseccion(
    a: (NaiveDate, NaiveDate),
    b: (NaiveDate, NaiveDate),
) -> Option<(NaiveDate, NaiveDate)> {
    let desde = a.0.max(b.0);
    let hasta = a.1.min(b.1);
    (desde <= hasta).then_some((desde, hasta))
}

// ---------------------------------------------------------------------------------------------
// Novedades
// ---------------------------------------------------------------------------------------------

/// Cómo se mide una novedad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Medida {
    /// Horas en una fecha.
    Horas,
    /// Un rango de días (`desde`–`hasta`).
    Dias,
    /// Un valor en pesos en una fecha.
    Valor,
}

enum_texto!(Medida {
    Horas => "horas",
    Dias => "dias",
    Valor => "valor",
});

/// Tipo de novedad de nómina. El texto es contrato con `novedades_nomina.tipo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoNovedad {
    HoraExtraDiurna,
    HoraExtraNocturna,
    /// Hora extra diurna en domingo o festivo.
    HoraExtraDominicalDiurna,
    /// Hora extra nocturna en domingo o festivo.
    HoraExtraDominicalNocturna,
    /// Hora ordinaria nocturna (solo el recargo).
    RecargoNocturno,
    /// Hora ordinaria en domingo o festivo (solo el recargo).
    RecargoDominical,
    /// Hora ordinaria nocturna en domingo o festivo (solo los recargos).
    RecargoNocturnoDominical,
    /// Incapacidad de origen común (dato sensible: nunca el diagnóstico).
    IncapacidadComun,
    /// Incapacidad de origen laboral (la reconoce la ARL).
    IncapacidadLaboral,
    LicenciaMaternidad,
    LicenciaPaternidad,
    /// Licencia remunerada (luto, calamidad, otras).
    LicenciaRemunerada,
    /// Vacaciones disfrutadas (los días calendario del descanso; se pagan al salario diario).
    Vacaciones,
    LicenciaNoRemunerada,
    /// Ausencia injustificada (no se paga).
    Ausencia,
    /// Suspensión disciplinaria (no se paga).
    Suspension,
    BonificacionSalarial,
    /// Bonificación pactada como no constitutiva de salario.
    BonificacionNoSalarial,
    Comision,
    /// Auxilio pactado como no constitutivo de salario.
    AuxilioNoSalarial,
    Prestamo,
    Libranza,
    Embargo,
    OtraDeduccion,
}

enum_texto!(TipoNovedad {
    HoraExtraDiurna => "hora_extra_diurna",
    HoraExtraNocturna => "hora_extra_nocturna",
    HoraExtraDominicalDiurna => "hora_extra_dominical_diurna",
    HoraExtraDominicalNocturna => "hora_extra_dominical_nocturna",
    RecargoNocturno => "recargo_nocturno",
    RecargoDominical => "recargo_dominical",
    RecargoNocturnoDominical => "recargo_nocturno_dominical",
    IncapacidadComun => "incapacidad_comun",
    IncapacidadLaboral => "incapacidad_laboral",
    LicenciaMaternidad => "licencia_maternidad",
    LicenciaPaternidad => "licencia_paternidad",
    LicenciaRemunerada => "licencia_remunerada",
    Vacaciones => "vacaciones",
    LicenciaNoRemunerada => "licencia_no_remunerada",
    Ausencia => "ausencia",
    Suspension => "suspension",
    BonificacionSalarial => "bonificacion_salarial",
    BonificacionNoSalarial => "bonificacion_no_salarial",
    Comision => "comision",
    AuxilioNoSalarial => "auxilio_no_salarial",
    Prestamo => "prestamo",
    Libranza => "libranza",
    Embargo => "embargo",
    OtraDeduccion => "otra_deduccion",
});

impl TipoNovedad {
    /// Cómo se mide.
    pub const fn medida(self) -> Medida {
        use TipoNovedad as T;
        match self {
            T::HoraExtraDiurna
            | T::HoraExtraNocturna
            | T::HoraExtraDominicalDiurna
            | T::HoraExtraDominicalNocturna
            | T::RecargoNocturno
            | T::RecargoDominical
            | T::RecargoNocturnoDominical => Medida::Horas,
            T::IncapacidadComun
            | T::IncapacidadLaboral
            | T::LicenciaMaternidad
            | T::LicenciaPaternidad
            | T::LicenciaRemunerada
            | T::Vacaciones
            | T::LicenciaNoRemunerada
            | T::Ausencia
            | T::Suspension => Medida::Dias,
            T::BonificacionSalarial
            | T::BonificacionNoSalarial
            | T::Comision
            | T::AuxilioNoSalarial
            | T::Prestamo
            | T::Libranza
            | T::Embargo
            | T::OtraDeduccion => Medida::Valor,
        }
    }

    /// ¿Es de domingo o festivo? (su fecha debe serlo).
    pub const fn es_dominical(self) -> bool {
        matches!(
            self,
            Self::HoraExtraDominicalDiurna
                | Self::HoraExtraDominicalNocturna
                | Self::RecargoDominical
                | Self::RecargoNocturnoDominical
        )
    }

    /// ¿Revela la salud del empleado? (incapacidades y licencias de maternidad o paternidad).
    pub const fn es_sensible(self) -> bool {
        matches!(
            self,
            Self::IncapacidadComun
                | Self::IncapacidadLaboral
                | Self::LicenciaMaternidad
                | Self::LicenciaPaternidad
        )
    }
}

/// Novedad del periodo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Novedad {
    pub tipo: TipoNovedad,
    /// Fecha (horas y valores) o primer día (días).
    pub desde: NaiveDate,
    /// Último día (días); igual a `desde` en las demás.
    pub hasta: NaiveDate,
    /// Horas o valor en pesos; en las de días no se usa (cero).
    pub cantidad: Decimal,
}

impl Novedad {
    /// Valida y normaliza una novedad.
    ///
    /// # Errors
    ///
    /// [`DominioError::ReglaLaboral`] si las fechas o la cantidad no corresponden a la medida
    /// (horas de 0 a 24 en un día, valores positivos en pesos, días con `hasta` ≥ `desde`) o una
    /// dominical no cae en domingo o festivo.
    pub fn new(
        tipo: TipoNovedad,
        desde: NaiveDate,
        hasta: Option<NaiveDate>,
        cantidad: Option<Decimal>,
    ) -> Result<Self, DominioError> {
        let (hasta, cantidad) = match (tipo.medida(), hasta, cantidad) {
            (Medida::Dias, Some(h), None) if h >= desde => (h, Decimal::ZERO),
            (Medida::Dias, _, _) => {
                return Err(regla(format!(
                    "la novedad `{tipo}` lleva el último día (no antes del primero) y no lleva cantidad"
                )));
            }
            (Medida::Horas, None, Some(h)) if h > Decimal::ZERO && h <= Decimal::from(24) => {
                (desde, h)
            }
            (Medida::Valor, None, Some(v)) if v > Decimal::ZERO && v.normalize().scale() <= 2 => {
                (desde, v)
            }
            _ => {
                return Err(regla(format!(
                    "la novedad `{tipo}` lleva una sola fecha y su cantidad (horas de 0 a 24 o un valor en pesos)"
                )));
            }
        };
        if tipo.es_dominical() && desde.weekday() != Weekday::Sun && !es_festivo(desde) {
            return Err(regla(format!(
                "la novedad `{tipo}` es de un domingo o festivo"
            )));
        }
        Ok(Self {
            tipo,
            desde,
            hasta,
            cantidad,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Conceptos y líneas
// ---------------------------------------------------------------------------------------------

/// Clase de una línea de la liquidación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaseLinea {
    Devengo,
    Deduccion,
    /// Lo que paga el empleador además del neto (no se descuenta al empleado).
    AporteEmpleador,
    /// Provisión de prestaciones del periodo (informativa: no se paga ni se descuenta).
    Provision,
}

enum_texto!(ClaseLinea {
    Devengo => "devengo",
    Deduccion => "deduccion",
    AporteEmpleador => "aporte_empleador",
    Provision => "provision",
});

/// Concepto de una línea. El texto es contrato con `lineas_liquidacion_nomina.concepto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConceptoNomina {
    Basico,
    ApoyoSostenimiento,
    AuxilioTransporte,
    HoraExtraDiurna,
    HoraExtraNocturna,
    HoraExtraDominicalDiurna,
    HoraExtraDominicalNocturna,
    RecargoNocturno,
    RecargoDominical,
    RecargoNocturnoDominical,
    IncapacidadComun,
    IncapacidadLaboral,
    LicenciaMaternidad,
    LicenciaPaternidad,
    LicenciaRemunerada,
    Vacaciones,
    BonificacionSalarial,
    BonificacionNoSalarial,
    Comision,
    AuxilioNoSalarial,
    /// Prima de servicios (semestral o proporcional al retiro).
    Prima,
    /// Cesantías pagadas al retiro.
    Cesantias,
    InteresesCesantias,
    /// Vacaciones pendientes pagadas en dinero al retiro.
    VacacionesCompensadas,
    /// Indemnización por terminación sin justa causa (CST art. 64).
    Indemnizacion,
    Salud,
    Pension,
    FondoSolidaridad,
    RetencionFuente,
    Prestamo,
    Libranza,
    Embargo,
    OtraDeduccion,
    SaludEmpleador,
    PensionEmpleador,
    Arl,
    CajaCompensacion,
    Icbf,
    Sena,
    ProvisionCesantias,
    ProvisionIntereses,
    ProvisionPrima,
    ProvisionVacaciones,
}

enum_texto!(ConceptoNomina {
    Basico => "basico",
    ApoyoSostenimiento => "apoyo_sostenimiento",
    AuxilioTransporte => "auxilio_transporte",
    HoraExtraDiurna => "hora_extra_diurna",
    HoraExtraNocturna => "hora_extra_nocturna",
    HoraExtraDominicalDiurna => "hora_extra_dominical_diurna",
    HoraExtraDominicalNocturna => "hora_extra_dominical_nocturna",
    RecargoNocturno => "recargo_nocturno",
    RecargoDominical => "recargo_dominical",
    RecargoNocturnoDominical => "recargo_nocturno_dominical",
    IncapacidadComun => "incapacidad_comun",
    IncapacidadLaboral => "incapacidad_laboral",
    LicenciaMaternidad => "licencia_maternidad",
    LicenciaPaternidad => "licencia_paternidad",
    LicenciaRemunerada => "licencia_remunerada",
    Vacaciones => "vacaciones",
    BonificacionSalarial => "bonificacion_salarial",
    BonificacionNoSalarial => "bonificacion_no_salarial",
    Comision => "comision",
    AuxilioNoSalarial => "auxilio_no_salarial",
    Prima => "prima",
    Cesantias => "cesantias",
    InteresesCesantias => "intereses_cesantias",
    VacacionesCompensadas => "vacaciones_compensadas",
    Indemnizacion => "indemnizacion",
    Salud => "salud",
    Pension => "pension",
    FondoSolidaridad => "fondo_solidaridad",
    RetencionFuente => "retencion_fuente",
    Prestamo => "prestamo",
    Libranza => "libranza",
    Embargo => "embargo",
    OtraDeduccion => "otra_deduccion",
    SaludEmpleador => "salud_empleador",
    PensionEmpleador => "pension_empleador",
    Arl => "arl",
    CajaCompensacion => "caja_compensacion",
    Icbf => "icbf",
    Sena => "sena",
    ProvisionCesantias => "provision_cesantias",
    ProvisionIntereses => "provision_intereses",
    ProvisionPrima => "provision_prima",
    ProvisionVacaciones => "provision_vacaciones",
});

impl ConceptoNomina {
    /// Clase de la línea.
    pub const fn clase(self) -> ClaseLinea {
        use ConceptoNomina as C;
        match self {
            C::Salud
            | C::Pension
            | C::FondoSolidaridad
            | C::RetencionFuente
            | C::Prestamo
            | C::Libranza
            | C::Embargo
            | C::OtraDeduccion => ClaseLinea::Deduccion,
            C::SaludEmpleador
            | C::PensionEmpleador
            | C::Arl
            | C::CajaCompensacion
            | C::Icbf
            | C::Sena => ClaseLinea::AporteEmpleador,
            C::ProvisionCesantias
            | C::ProvisionIntereses
            | C::ProvisionPrima
            | C::ProvisionVacaciones => ClaseLinea::Provision,
            _ => ClaseLinea::Devengo,
        }
    }

    /// ¿Es un devengo constitutivo de salario (base del IBC)?
    pub const fn salarial(self) -> bool {
        self.clase() as u8 == ClaseLinea::Devengo as u8
            && !self.es_prestacion()
            && !matches!(
                self,
                Self::AuxilioTransporte | Self::BonificacionNoSalarial | Self::AuxilioNoSalarial
            )
    }

    /// ¿Es una prestación social o la indemnización? (no son salario, no entran al IBC ni a la
    /// regla del 40 %).
    pub const fn es_prestacion(self) -> bool {
        matches!(
            self,
            Self::Prima
                | Self::Cesantias
                | Self::InteresesCesantias
                | Self::VacacionesCompensadas
                | Self::Indemnizacion
        )
    }

    /// ¿Entra a la base de la retención en la fuente? Todo devengo, salvo las cesantías y sus
    /// intereses (exentos, art. 206 num. 4 del ET) y la indemnización (retención propia, art.
    /// 401-3), pendientes de revisión con el especialista.
    pub const fn gravable(self) -> bool {
        self.clase() as u8 == ClaseLinea::Devengo as u8
            && !matches!(
                self,
                Self::Cesantias | Self::InteresesCesantias | Self::Indemnizacion
            )
    }

    /// Nombre legible del concepto.
    pub const fn etiqueta(self) -> &'static str {
        use ConceptoNomina as C;
        match self {
            C::Basico => "Salario básico",
            C::ApoyoSostenimiento => "Apoyo de sostenimiento",
            C::AuxilioTransporte => "Auxilio de transporte",
            C::HoraExtraDiurna => "Horas extra diurnas",
            C::HoraExtraNocturna => "Horas extra nocturnas",
            C::HoraExtraDominicalDiurna => "Horas extra dominicales diurnas",
            C::HoraExtraDominicalNocturna => "Horas extra dominicales nocturnas",
            C::RecargoNocturno => "Recargo nocturno",
            C::RecargoDominical => "Recargo dominical o festivo",
            C::RecargoNocturnoDominical => "Recargo nocturno dominical o festivo",
            C::IncapacidadComun => "Incapacidad de origen común",
            C::IncapacidadLaboral => "Incapacidad de origen laboral",
            C::LicenciaMaternidad => "Licencia de maternidad",
            C::LicenciaPaternidad => "Licencia de paternidad",
            C::LicenciaRemunerada => "Licencia remunerada",
            C::Vacaciones => "Vacaciones disfrutadas",
            C::Prima => "Prima de servicios",
            C::Cesantias => "Cesantías",
            C::InteresesCesantias => "Intereses sobre cesantías",
            C::VacacionesCompensadas => "Vacaciones compensadas",
            C::Indemnizacion => "Indemnización",
            C::ProvisionCesantias => "Provisión de cesantías",
            C::ProvisionIntereses => "Provisión de intereses sobre cesantías",
            C::ProvisionPrima => "Provisión de prima",
            C::ProvisionVacaciones => "Provisión de vacaciones",
            C::BonificacionSalarial => "Bonificación",
            C::BonificacionNoSalarial => "Bonificación no salarial",
            C::Comision => "Comisiones",
            C::AuxilioNoSalarial => "Auxilio no salarial",
            C::Salud => "Aporte a salud",
            C::Pension => "Aporte a pensión",
            C::FondoSolidaridad => "Fondo de solidaridad pensional",
            C::RetencionFuente => "Retención en la fuente",
            C::Prestamo => "Préstamo",
            C::Libranza => "Libranza",
            C::Embargo => "Embargo",
            C::OtraDeduccion => "Otra deducción",
            C::SaludEmpleador => "Salud (empleador)",
            C::PensionEmpleador => "Pensión (empleador)",
            C::Arl => "ARL",
            C::CajaCompensacion => "Caja de compensación",
            C::Icbf => "ICBF",
            C::Sena => "SENA",
        }
    }

    /// Nombre en el desprendible: las incapacidades y las licencias de maternidad o paternidad
    /// salen como «Ausencia remunerada» (el desprendible circula; el motivo es dato sensible).
    pub const fn etiqueta_desprendible(self) -> &'static str {
        match self {
            Self::IncapacidadComun
            | Self::IncapacidadLaboral
            | Self::LicenciaMaternidad
            | Self::LicenciaPaternidad => "Ausencia remunerada",
            otro => otro.etiqueta(),
        }
    }

    /// Concepto de una novedad.
    pub const fn de_novedad(tipo: TipoNovedad) -> Option<Self> {
        use {ConceptoNomina as C, TipoNovedad as T};
        Some(match tipo {
            T::HoraExtraDiurna => C::HoraExtraDiurna,
            T::HoraExtraNocturna => C::HoraExtraNocturna,
            T::HoraExtraDominicalDiurna => C::HoraExtraDominicalDiurna,
            T::HoraExtraDominicalNocturna => C::HoraExtraDominicalNocturna,
            T::RecargoNocturno => C::RecargoNocturno,
            T::RecargoDominical => C::RecargoDominical,
            T::RecargoNocturnoDominical => C::RecargoNocturnoDominical,
            T::IncapacidadComun => C::IncapacidadComun,
            T::IncapacidadLaboral => C::IncapacidadLaboral,
            T::LicenciaMaternidad => C::LicenciaMaternidad,
            T::LicenciaPaternidad => C::LicenciaPaternidad,
            T::LicenciaRemunerada => C::LicenciaRemunerada,
            T::Vacaciones => C::Vacaciones,
            T::LicenciaNoRemunerada | T::Ausencia | T::Suspension => return None,
            T::BonificacionSalarial => C::BonificacionSalarial,
            T::BonificacionNoSalarial => C::BonificacionNoSalarial,
            T::Comision => C::Comision,
            T::AuxilioNoSalarial => C::AuxilioNoSalarial,
            T::Prestamo => C::Prestamo,
            T::Libranza => C::Libranza,
            T::Embargo => C::Embargo,
            T::OtraDeduccion => C::OtraDeduccion,
        })
    }
}

/// Línea de la liquidación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineaLiquidacion {
    pub concepto: ConceptoNomina,
    /// Días, horas o, en los aportes, el porcentaje aplicado; cero si no aplica.
    pub cantidad: Decimal,
    pub valor: Monto,
}

// ---------------------------------------------------------------------------------------------
// Tablas por tramos (retención en la fuente y fondo de solidaridad)
// ---------------------------------------------------------------------------------------------

/// Tabla laboral por tramos del catálogo de plataforma.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TablaLaboral {
    /// Retención en la fuente por pagos laborales (art. 383 del ET): tramos en UVT, tarifa
    /// marginal en % y UVT adicionales.
    RetencionSalarios,
    /// Fondo de solidaridad pensional: tramos de IBC en salarios mínimos y porcentaje.
    FondoSolidaridad,
}

enum_texto!(TablaLaboral {
    RetencionSalarios => "retencion_salarios",
    FondoSolidaridad => "fondo_solidaridad",
});

/// Tramo `(desde, hasta]` de una tabla (el primero incluye su `desde`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tramo {
    pub desde: Decimal,
    /// `None` en el último.
    pub hasta: Option<Decimal>,
    /// Porcentaje.
    pub tarifa: Decimal,
    /// Valor fijo que se suma (UVT en la retención; cero en el fondo de solidaridad).
    pub adicional: Decimal,
}

/// Tabla por tramos validada: empieza en cero, los tramos son contiguos y el último es abierto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TablaTramos(Vec<Tramo>);

impl TablaTramos {
    /// # Errors
    ///
    /// [`DominioError::ReglaLaboral`] si la tabla no empieza en cero, tiene huecos o solapes, la
    /// tarifa no está entre 0 y 100 o el último tramo no es abierto.
    pub fn new(mut tramos: Vec<Tramo>) -> Result<Self, DominioError> {
        tramos.sort_by_key(|a| a.desde);
        let error = || regla("los tramos empiezan en cero, son contiguos y el último es abierto");
        if tramos.first().is_none_or(|t| !t.desde.is_zero()) {
            return Err(error());
        }
        for (i, t) in tramos.iter().enumerate() {
            let ultimo = i + 1 == tramos.len();
            if t.tarifa < Decimal::ZERO || t.tarifa > cien() || t.adicional < Decimal::ZERO {
                return Err(regla(
                    "la tarifa va de 0 a 100 y el adicional no es negativo",
                ));
            }
            match (t.hasta, tramos.get(i + 1)) {
                (None, None) if ultimo => {}
                (Some(h), Some(s)) if h > t.desde && s.desde == h => {}
                _ => return Err(error()),
            }
        }
        Ok(Self(tramos))
    }

    /// Tramos de la tabla, del primero al último.
    pub fn tramos(&self) -> &[Tramo] {
        &self.0
    }

    /// Tramo donde cae `x`.
    pub fn tramo_de(&self, x: Decimal) -> Option<&Tramo> {
        self.0.iter().find(|t| t.hasta.is_none_or(|h| x <= h))
    }

    /// `(x − desde) × tarifa + adicional` del tramo donde cae `x` (cero si `x` no es positivo).
    pub fn aplicar(&self, x: Decimal) -> Decimal {
        if x <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        self.tramo_de(x).map_or(Decimal::ZERO, |t| {
            (x - t.desde) * t.tarifa / cien() + t.adicional
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Entrada y resultado
// ---------------------------------------------------------------------------------------------

/// Parte del periodo en que rige una versión del contrato.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TramoContrato {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    pub salario: Monto,
    pub tipo_salario: TipoSalario,
    pub etapa_aprendizaje: Option<EtapaAprendizaje>,
    pub jornada: Jornada,
    pub horas_semanales: Option<Decimal>,
    pub pensionado: bool,
    /// `None` sin ARL.
    pub clase_riesgo: Option<ClaseRiesgo>,
}

/// Lo ya liquidado en el mes (la primera quincena) que entra a la retención de la segunda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AcumuladoMes {
    /// Pagos laborales.
    pub ingresos: Decimal,
    /// Aportes obligatorios del empleado (salud, pensión y fondo de solidaridad).
    pub aportes: Decimal,
    /// Retención ya practicada.
    pub retenido: Decimal,
}

/// Cuándo se calcula la retención en la fuente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retencion {
    /// No en este periodo (primera quincena).
    NoAplica,
    /// Cierra el mes: suma lo acumulado (cero en la nómina mensual).
    CierreMes(AcumuladoMes),
}

/// Tablas y valores que la liquidación lee del catálogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normativa {
    pub catalogo: CatalogoLaboral,
    pub retencion: TablaTramos,
    pub fondo_solidaridad: TablaTramos,
    /// UVT del año del periodo.
    pub uvt: Decimal,
}

/// Todo lo que necesita la liquidación de un contrato en un periodo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntradaLiquidacion {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    /// Tramos del contrato dentro del periodo, ordenados y sin cruces.
    pub tramos: Vec<TramoContrato>,
    pub novedades: Vec<Novedad>,
    /// La entidad aplica la exoneración del art. 114-1 del ET.
    pub exonerada: bool,
    pub retencion: Retencion,
    /// Prestaciones que se pagan en este periodo (prima, intereses, liquidación del contrato),
    /// ya calculadas con [`super::prestaciones`].
    pub prestaciones: Vec<LineaLiquidacion>,
}

/// Resultado de la liquidación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Liquidacion {
    /// Días comerciales con contrato en el periodo.
    pub dias: u32,
    pub ibc: Monto,
    pub lineas: Vec<LineaLiquidacion>,
    pub devengos: Monto,
    pub deducciones: Monto,
    pub neto: Monto,
    pub aportes_empleador: Monto,
    /// Para la retención de la segunda quincena: pagos laborales y aportes del empleado.
    pub acumulado: AcumuladoMes,
}

#[derive(Default)]
struct Acumulador(BTreeMap<ConceptoNomina, (Decimal, Decimal)>);

impl Acumulador {
    fn sumar(&mut self, concepto: ConceptoNomina, cantidad: Decimal, valor: Decimal) {
        let e = self.0.entry(concepto).or_default();
        e.0 += cantidad;
        e.1 += valor;
    }

    fn total(&self, f: impl Fn(ConceptoNomina) -> bool) -> Decimal {
        self.0
            .iter()
            .filter(|(c, _)| f(**c))
            .map(|(_, (_, v))| al_peso(*v))
            .sum()
    }
}

fn monto(v: Decimal) -> Result<Monto, DominioError> {
    Monto::new(al_peso(v))
}

/// Factor del valor de la hora de cada novedad de horas (1 = la hora completa).
fn factor_hora(tipo: TipoNovedad, c: &CatalogoLaboral) -> Result<Decimal, DominioError> {
    use TipoNovedad as T;
    let p = |x| c.valor(x).map(|v| v / cien());
    Ok(match tipo {
        T::HoraExtraDiurna => Decimal::ONE + p(P::HoraExtraDiurna)?,
        T::HoraExtraNocturna => Decimal::ONE + p(P::HoraExtraNocturna)?,
        T::HoraExtraDominicalDiurna => {
            Decimal::ONE + p(P::HoraExtraDiurna)? + p(P::RecargoDominicalFestivo)?
        }
        T::HoraExtraDominicalNocturna => {
            Decimal::ONE + p(P::HoraExtraNocturna)? + p(P::RecargoDominicalFestivo)?
        }
        T::RecargoNocturno => p(P::RecargoNocturno)?,
        T::RecargoDominical => p(P::RecargoDominicalFestivo)?,
        T::RecargoNocturnoDominical => p(P::RecargoNocturno)? + p(P::RecargoDominicalFestivo)?,
        _ => Decimal::ZERO,
    })
}

/// Liquida un contrato en un periodo.
///
/// # Errors
///
/// [`DominioError::ReglaLaboral`] si no hay tramos, falta un parámetro del catálogo, una novedad
/// de horas cae fuera del contrato o las deducciones superan los devengos.
pub fn liquidar(e: &EntradaLiquidacion, n: &Normativa) -> Result<Liquidacion, DominioError> {
    let c = &n.catalogo;
    let ultimo = *e
        .tramos
        .last()
        .ok_or_else(|| regla("el contrato no tiene días en el periodo"))?;
    let aprendiz = ultimo.tipo_salario == TipoSalario::ApoyoSostenimiento;
    let smmlv = c.valor(P::SalarioMinimo)?;
    let mut a = Acumulador::default();
    let mut dias_contrato = 0;
    let mut ibc_salarial = Decimal::ZERO;
    let mut algun_completo = false;

    for t in &e.tramos {
        let dias_tramo = dias_comerciales(t.desde, t.hasta);
        dias_contrato += dias_tramo;
        algun_completo |= t.jornada == Jornada::Completa;
        let diario = t.salario.valor() / treinta();
        let mut dias_novedad = 0;
        let mut salarial_tramo = Decimal::ZERO;
        for nov in &e.novedades {
            let Some((d, h)) = interseccion((nov.desde, nov.hasta), (t.desde, t.hasta)) else {
                continue;
            };
            let concepto = ConceptoNomina::de_novedad(nov.tipo);
            match nov.tipo.medida() {
                Medida::Dias => {
                    let dias = dias_comerciales(d, h);
                    dias_novedad += dias;
                    let por_dia = match nov.tipo {
                        TipoNovedad::IncapacidadComun => {
                            (diario * c.valor(P::IncapacidadEps)? / cien()).max(smmlv / treinta())
                        }
                        TipoNovedad::IncapacidadLaboral
                        | TipoNovedad::LicenciaMaternidad
                        | TipoNovedad::LicenciaPaternidad
                        | TipoNovedad::LicenciaRemunerada
                        | TipoNovedad::Vacaciones => diario,
                        _ => Decimal::ZERO,
                    };
                    if let Some(concepto) = concepto {
                        let valor = por_dia * Decimal::from(dias);
                        a.sumar(concepto, Decimal::from(dias), valor);
                        salarial_tramo += valor;
                    }
                }
                Medida::Horas => {
                    let jornada = t
                        .horas_semanales
                        .map_or_else(|| c.valor(P::JornadaMaximaSemanal), Ok)?;
                    let horas_mes = jornada / Decimal::from(6) * treinta();
                    let valor =
                        t.salario.valor() / horas_mes * factor_hora(nov.tipo, c)? * nov.cantidad;
                    if let Some(concepto) = concepto {
                        a.sumar(concepto, nov.cantidad, valor);
                        salarial_tramo += valor;
                    }
                }
                Medida::Valor => {
                    if let Some(concepto) = concepto {
                        a.sumar(concepto, Decimal::ONE, nov.cantidad);
                        if concepto.salarial() {
                            salarial_tramo += nov.cantidad;
                        }
                    }
                }
            }
        }
        let laborados = dias_tramo.saturating_sub(dias_novedad);
        let basico = diario * Decimal::from(laborados);
        let concepto_basico = if t.tipo_salario == TipoSalario::ApoyoSostenimiento {
            ConceptoNomina::ApoyoSostenimiento
        } else {
            ConceptoNomina::Basico
        };
        a.sumar(concepto_basico, Decimal::from(laborados), basico);
        salarial_tramo += basico;
        let tope_auxilio = c.salarios_minimos(c.valor(P::TopeAuxilioTransporte)?)?;
        let mut auxilio = Decimal::ZERO;
        if t.tipo_salario == TipoSalario::Ordinario && t.salario.valor() <= tope_auxilio {
            auxilio = c.valor(P::AuxilioTransporte)? / treinta() * Decimal::from(laborados);
            a.sumar(
                ConceptoNomina::AuxilioTransporte,
                Decimal::from(laborados),
                auxilio,
            );
        }
        // Provisiones del tramo: cesantías, intereses y prima sobre lo salarial más el auxilio
        // (solo salario ordinario); vacaciones sobre el salario ordinario de los días (también en
        // el integral). El aprendiz no las causa.
        let dias = Decimal::from(dias_tramo);
        let ano = Decimal::from(360);
        if t.tipo_salario == TipoSalario::Ordinario {
            let base = salarial_tramo + auxilio;
            let cesantias = base * c.valor(P::DiasCesantias)? / ano;
            a.sumar(ConceptoNomina::ProvisionCesantias, dias, cesantias);
            a.sumar(
                ConceptoNomina::ProvisionIntereses,
                dias,
                cesantias * c.valor(P::InteresesCesantias)? / cien() * dias / ano,
            );
            a.sumar(
                ConceptoNomina::ProvisionPrima,
                dias,
                base * c.valor(P::DiasPrima)? / ano,
            );
        }
        if t.tipo_salario != TipoSalario::ApoyoSostenimiento {
            a.sumar(
                ConceptoNomina::ProvisionVacaciones,
                dias,
                diario * dias * c.valor(P::DiasVacaciones)? / ano,
            );
        }
        ibc_salarial += if t.tipo_salario == TipoSalario::Integral {
            salarial_tramo * c.valor(P::FactorSalarioIntegral)? / cien()
        } else {
            salarial_tramo
        };
    }
    for nov in &e.novedades {
        if nov.tipo.medida() != Medida::Dias
            && !e
                .tramos
                .iter()
                .any(|t| nov.desde >= t.desde && nov.desde <= t.hasta)
        {
            return Err(regla(format!(
                "la novedad `{}` cae fuera del contrato en el periodo",
                nov.tipo
            )));
        }
    }

    for l in &e.prestaciones {
        a.sumar(l.concepto, l.cantidad, l.valor.valor());
    }

    // IBC.
    let salariales = a.total(ConceptoNomina::salarial);
    let no_salariales = a.total(|x| {
        matches!(
            x,
            ConceptoNomina::BonificacionNoSalarial | ConceptoNomina::AuxilioNoSalarial
        )
    });
    let limite = (salariales + no_salariales) * c.valor(P::LimitePagosNoSalariales)? / cien();
    let exceso = (no_salariales - limite).max(Decimal::ZERO);
    let dias = Decimal::from(dias_contrato);
    let mut ibc = ibc_salarial + exceso;
    if algun_completo {
        ibc = ibc.max(smmlv / treinta() * dias);
    }
    ibc = al_peso(ibc.min(c.salarios_minimos(c.valor(P::TopeIbc)?)? / treinta() * dias));
    let ibc_mensual = if dias_contrato == 0 {
        Decimal::ZERO
    } else {
        ibc * treinta() / dias
    };

    // Aportes.
    let mut aporte = |concepto, pct: Decimal| a.sumar(concepto, pct, ibc * pct / cien());
    if aprendiz {
        aporte(
            ConceptoNomina::SaludEmpleador,
            c.valor(P::SaludEmpleador)? + c.valor(P::SaludEmpleado)?,
        );
        if ultimo.etapa_aprendizaje == Some(EtapaAprendizaje::Productiva)
            && let Some(clase) = ultimo.clase_riesgo
        {
            aporte(ConceptoNomina::Arl, c.valor(P::arl(clase))?);
        }
    } else {
        let exonerada =
            e.exonerada && ibc_mensual < c.salarios_minimos(c.valor(P::TopeExoneracionAportes)?)?;
        aporte(ConceptoNomina::Salud, c.valor(P::SaludEmpleado)?);
        if !exonerada {
            aporte(ConceptoNomina::SaludEmpleador, c.valor(P::SaludEmpleador)?);
            aporte(ConceptoNomina::Icbf, c.valor(P::Icbf)?);
            aporte(ConceptoNomina::Sena, c.valor(P::Sena)?);
        }
        if !ultimo.pensionado {
            aporte(ConceptoNomina::Pension, c.valor(P::PensionEmpleado)?);
            aporte(
                ConceptoNomina::PensionEmpleador,
                c.valor(P::PensionEmpleador)?,
            );
            let fondo = n
                .fondo_solidaridad
                .tramo_de(ibc_mensual / smmlv)
                .map_or(Decimal::ZERO, |t| t.tarifa);
            if fondo > Decimal::ZERO {
                aporte(ConceptoNomina::FondoSolidaridad, fondo);
            }
        }
        if let Some(clase) = ultimo.clase_riesgo {
            aporte(ConceptoNomina::Arl, c.valor(P::arl(clase))?);
        }
        aporte(
            ConceptoNomina::CajaCompensacion,
            c.valor(P::CajaCompensacion)?,
        );
    }

    // Retención en la fuente (procedimiento 1).
    let ingresos = a.total(ConceptoNomina::gravable);
    let aportes_empleado = a.total(|x| {
        matches!(
            x,
            ConceptoNomina::Salud | ConceptoNomina::Pension | ConceptoNomina::FondoSolidaridad
        )
    });
    if let Retencion::CierreMes(previo) = e.retencion {
        let depurado = previo.ingresos + ingresos - previo.aportes - aportes_empleado;
        let doce = Decimal::from(12);
        let exenta = (depurado * c.valor(P::RentaExentaLaboral)? / cien())
            .min(c.valor(P::TopeRentaExentaLaboral)? / doce * n.uvt);
        let limite = (depurado * c.valor(P::LimiteBeneficiosRetencion)? / cien())
            .min(c.valor(P::TopeBeneficiosRetencion)? / doce * n.uvt);
        let base = (depurado - exenta.min(limite)).max(Decimal::ZERO);
        let en_uvt = n.retencion.aplicar(base / n.uvt);
        let mes = (en_uvt * n.uvt / Decimal::from(1000))
            .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
            * Decimal::from(1000);
        let retencion = (mes - previo.retenido).max(Decimal::ZERO);
        if retencion > Decimal::ZERO {
            a.sumar(ConceptoNomina::RetencionFuente, Decimal::ZERO, retencion);
        }
    }

    let lineas =
        a.0.iter()
            .filter(|(_, (_, v))| !al_peso(*v).is_zero())
            .map(|(concepto, (cantidad, valor))| {
                Ok(LineaLiquidacion {
                    concepto: *concepto,
                    cantidad: *cantidad,
                    valor: monto(*valor)?,
                })
            })
            .collect::<Result<Vec<_>, DominioError>>()?;
    let suma = |clase| -> Decimal {
        lineas
            .iter()
            .filter(|l| l.concepto.clase() == clase)
            .map(|l| l.valor.valor())
            .sum()
    };
    let devengos = suma(ClaseLinea::Devengo);
    let deducciones = suma(ClaseLinea::Deduccion);
    if deducciones > devengos {
        return Err(regla("las deducciones superan los devengos del periodo"));
    }
    let retenido = lineas
        .iter()
        .find(|l| l.concepto == ConceptoNomina::RetencionFuente)
        .map_or(Decimal::ZERO, |l| l.valor.valor());
    Ok(Liquidacion {
        dias: dias_contrato,
        ibc: Monto::new(ibc)?,
        devengos: Monto::new(devengos)?,
        deducciones: Monto::new(deducciones)?,
        neto: Monto::new(devengos - deducciones)?,
        aportes_empleador: Monto::new(suma(ClaseLinea::AporteEmpleador))?,
        acumulado: AcumuladoMes {
            ingresos,
            aportes: aportes_empleado,
            retenido,
        },
        lineas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nomina::VersionParametro;

    fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, d).expect("fecha válida")
    }

    fn dec(t: &str) -> Decimal {
        t.parse().expect("decimal")
    }

    fn monto_(t: &str) -> Monto {
        Monto::new(dec(t)).expect("monto")
    }

    /// Catálogo de referencia (valores ilustrativos para las pruebas, no oficiales).
    fn normativa() -> Normativa {
        let d = fecha(2026, 1, 1);
        let v = |p, x: &str| VersionParametro {
            parametro: p,
            valor: dec(x),
            vigente_desde: d,
        };
        let catalogo = CatalogoLaboral::vigente_en(
            &[
                v(P::SalarioMinimo, "1500000"),
                v(P::AuxilioTransporte, "200000"),
                v(P::TopeAuxilioTransporte, "2"),
                v(P::JornadaMaximaSemanal, "42"),
                v(P::RecargoNocturno, "35"),
                v(P::HoraExtraDiurna, "25"),
                v(P::HoraExtraNocturna, "75"),
                v(P::RecargoDominicalFestivo, "90"),
                v(P::SaludEmpleador, "8.5"),
                v(P::SaludEmpleado, "4"),
                v(P::PensionEmpleador, "12"),
                v(P::PensionEmpleado, "4"),
                v(P::CajaCompensacion, "4"),
                v(P::Icbf, "3"),
                v(P::Sena, "2"),
                v(P::ArlClase1, "0.522"),
                v(P::FactorSalarioIntegral, "70"),
                v(P::LimitePagosNoSalariales, "40"),
                v(P::IncapacidadEps, "66.67"),
                v(P::TopeIbc, "25"),
                v(P::TopeExoneracionAportes, "10"),
                v(P::RentaExentaLaboral, "25"),
                v(P::TopeRentaExentaLaboral, "790"),
                v(P::LimiteBeneficiosRetencion, "40"),
                v(P::TopeBeneficiosRetencion, "1340"),
                v(P::DiasCesantias, "30"),
                v(P::DiasPrima, "30"),
                v(P::InteresesCesantias, "12"),
                v(P::DiasVacaciones, "15"),
            ],
            fecha(2026, 3, 31),
        );
        let t = |desde: &str, hasta: Option<&str>, tarifa: &str, adicional: &str| Tramo {
            desde: dec(desde),
            hasta: hasta.map(dec),
            tarifa: dec(tarifa),
            adicional: dec(adicional),
        };
        Normativa {
            catalogo,
            retencion: TablaTramos::new(vec![
                t("0", Some("95"), "0", "0"),
                t("95", Some("150"), "19", "0"),
                t("150", Some("360"), "28", "10"),
                t("360", Some("640"), "33", "69"),
                t("640", Some("945"), "35", "162"),
                t("945", Some("2300"), "37", "268"),
                t("2300", None, "39", "770"),
            ])
            .expect("tabla de retención"),
            fondo_solidaridad: TablaTramos::new(vec![
                t("0", Some("4"), "0", "0"),
                t("4", Some("16"), "1", "0"),
                t("16", Some("17"), "1.2", "0"),
                t("17", Some("18"), "1.4", "0"),
                t("18", Some("19"), "1.6", "0"),
                t("19", Some("20"), "1.8", "0"),
                t("20", None, "2", "0"),
            ])
            .expect("tabla del fondo"),
            uvt: dec("50000"),
        }
    }

    fn tramo(salario: &str, desde: NaiveDate, hasta: NaiveDate) -> TramoContrato {
        TramoContrato {
            desde,
            hasta,
            salario: monto_(salario),
            tipo_salario: TipoSalario::Ordinario,
            etapa_aprendizaje: None,
            jornada: Jornada::Completa,
            horas_semanales: None,
            pensionado: false,
            clase_riesgo: Some(ClaseRiesgo::I),
        }
    }

    fn marzo(tramos: Vec<TramoContrato>, novedades: Vec<Novedad>) -> EntradaLiquidacion {
        EntradaLiquidacion {
            desde: fecha(2026, 3, 1),
            hasta: fecha(2026, 3, 31),
            tramos,
            novedades,
            exonerada: false,
            retencion: Retencion::CierreMes(AcumuladoMes::default()),
            prestaciones: vec![],
        }
    }

    fn valor(l: &Liquidacion, c: ConceptoNomina) -> Option<Decimal> {
        l.lineas
            .iter()
            .find(|x| x.concepto == c)
            .map(|x| x.valor.valor())
    }

    #[test]
    fn los_dias_comerciales_valen_treinta_por_mes() {
        assert_eq!(dias_comerciales(fecha(2026, 3, 1), fecha(2026, 3, 31)), 30);
        assert_eq!(dias_comerciales(fecha(2026, 2, 1), fecha(2026, 2, 28)), 30);
        assert_eq!(dias_comerciales(fecha(2026, 2, 16), fecha(2026, 2, 28)), 15);
        assert_eq!(dias_comerciales(fecha(2026, 3, 16), fecha(2026, 3, 31)), 15);
        assert_eq!(dias_comerciales(fecha(2026, 3, 1), fecha(2026, 3, 15)), 15);
        assert_eq!(dias_comerciales(fecha(2026, 3, 10), fecha(2026, 3, 14)), 5);
        assert_eq!(dias_comerciales(fecha(2026, 1, 20), fecha(2026, 3, 10)), 51);
    }

    #[test]
    fn la_tabla_de_tramos_es_contigua_y_empieza_en_cero() {
        let t = |d: &str, h: Option<&str>| Tramo {
            desde: dec(d),
            hasta: h.map(dec),
            tarifa: dec("1"),
            adicional: Decimal::ZERO,
        };
        assert!(TablaTramos::new(vec![t("0", Some("4")), t("4", None)]).is_ok());
        assert!(TablaTramos::new(vec![t("1", Some("4")), t("4", None)]).is_err());
        assert!(TablaTramos::new(vec![t("0", Some("4")), t("5", None)]).is_err());
        assert!(TablaTramos::new(vec![t("0", Some("4")), t("4", Some("9"))]).is_err());
        let r = &normativa().retencion;
        assert_eq!(r.aplicar(dec("95")), Decimal::ZERO);
        // 100 UVT: (100 − 95) × 19 % = 0,95 UVT.
        assert_eq!(r.aplicar(dec("100")), dec("0.95"));
    }

    /// Caso de referencia 1 (para revisión del especialista): salario mínimo con auxilio de
    /// transporte, mes completo.
    #[test]
    fn salario_minimo_con_auxilio_de_transporte() {
        let e = marzo(
            vec![tramo("1500000", fecha(2026, 3, 1), fecha(2026, 3, 31))],
            vec![],
        );
        let l = liquidar(&e, &normativa()).expect("liquida");

        assert_eq!(l.dias, 30);
        assert_eq!(valor(&l, ConceptoNomina::Basico), Some(dec("1500000")));
        assert_eq!(
            valor(&l, ConceptoNomina::AuxilioTransporte),
            Some(dec("200000"))
        );
        assert_eq!(l.ibc.valor(), dec("1500000"));
        assert_eq!(valor(&l, ConceptoNomina::Salud), Some(dec("60000")));
        assert_eq!(valor(&l, ConceptoNomina::Pension), Some(dec("60000")));
        assert_eq!(valor(&l, ConceptoNomina::FondoSolidaridad), None);
        assert_eq!(valor(&l, ConceptoNomina::RetencionFuente), None);
        // 1.700.000 − 120.000.
        assert_eq!(l.neto.valor(), dec("1580000"));
        // Salud 127.500 + pensión 180.000 + ARL 7.830 + caja 60.000 + ICBF 45.000 + SENA 30.000.
        assert_eq!(l.aportes_empleador.valor(), dec("450330"));
        // Provisiones sobre 1.700.000 (con auxilio): cesantías y prima 141.667, intereses 1.417;
        // vacaciones sobre 1.500.000: 62.500. No cambian el neto.
        assert_eq!(
            valor(&l, ConceptoNomina::ProvisionCesantias),
            Some(dec("141667"))
        );
        assert_eq!(
            valor(&l, ConceptoNomina::ProvisionPrima),
            Some(dec("141667"))
        );
        assert_eq!(
            valor(&l, ConceptoNomina::ProvisionIntereses),
            Some(dec("1417"))
        );
        assert_eq!(
            valor(&l, ConceptoNomina::ProvisionVacaciones),
            Some(dec("62500"))
        );
    }

    #[test]
    fn la_exoneracion_quita_salud_icbf_y_sena_del_empleador() {
        let mut e = marzo(
            vec![tramo("1500000", fecha(2026, 3, 1), fecha(2026, 3, 31))],
            vec![],
        );
        e.exonerada = true;
        let l = liquidar(&e, &normativa()).expect("liquida");
        assert_eq!(valor(&l, ConceptoNomina::SaludEmpleador), None);
        assert_eq!(valor(&l, ConceptoNomina::Icbf), None);
        assert_eq!(valor(&l, ConceptoNomina::Sena), None);
        assert_eq!(valor(&l, ConceptoNomina::Salud), Some(dec("60000")));
    }

    /// Caso de referencia 2: salario integral de 26.000.000 (IBC sobre el 70 %, fondo de
    /// solidaridad y retención con depuración).
    #[test]
    fn salario_integral_con_fondo_de_solidaridad_y_retencion() {
        let mut t = tramo("26000000", fecha(2026, 3, 1), fecha(2026, 3, 31));
        t.tipo_salario = TipoSalario::Integral;
        let l = liquidar(&marzo(vec![t], vec![]), &normativa()).expect("liquida");

        // IBC = 26.000.000 × 70 % = 18.200.000 (12,13 salarios mínimos: fondo del 1 %).
        assert_eq!(l.ibc.valor(), dec("18200000"));
        assert_eq!(valor(&l, ConceptoNomina::AuxilioTransporte), None);
        assert_eq!(valor(&l, ConceptoNomina::Salud), Some(dec("728000")));
        assert_eq!(valor(&l, ConceptoNomina::Pension), Some(dec("728000")));
        assert_eq!(
            valor(&l, ConceptoNomina::FondoSolidaridad),
            Some(dec("182000"))
        );
        // Depurado = 26.000.000 − 1.638.000 = 24.362.000. Exenta 25 % = 6.090.500, tope
        // 790/12 × 50.000 = 3.291.666,67; límite 40 % = 9.744.800 y 1340/12 × 50.000 =
        // 5.583.333,33 → beneficio 3.291.666,67. Base 21.070.333,33 = 421,41 UVT →
        // (421,41 − 360) × 33 % + 69 = 89,26 UVT = 4.462.512 → 4.463.000.
        assert_eq!(
            valor(&l, ConceptoNomina::RetencionFuente),
            Some(dec("4463000"))
        );
    }

    /// Caso de referencia 3: horas extra y recargos con dominical.
    #[test]
    fn horas_extra_y_dominicales() {
        // 2026-03-08 es domingo.
        let n = vec![
            Novedad::new(
                TipoNovedad::HoraExtraDiurna,
                fecha(2026, 3, 4),
                None,
                Some(dec("10")),
            )
            .expect("novedad"),
            Novedad::new(
                TipoNovedad::HoraExtraDominicalDiurna,
                fecha(2026, 3, 8),
                None,
                Some(dec("4")),
            )
            .expect("novedad"),
            Novedad::new(
                TipoNovedad::RecargoNocturno,
                fecha(2026, 3, 5),
                None,
                Some(dec("8")),
            )
            .expect("novedad"),
        ];
        let e = marzo(
            vec![tramo("2100000", fecha(2026, 3, 1), fecha(2026, 3, 31))],
            n,
        );
        let l = liquidar(&e, &normativa()).expect("liquida");

        // Valor hora = 2.100.000 / 210 = 10.000.
        assert_eq!(
            valor(&l, ConceptoNomina::HoraExtraDiurna),
            Some(dec("125000"))
        );
        // 4 × 10.000 × (1 + 25 % + 90 %).
        assert_eq!(
            valor(&l, ConceptoNomina::HoraExtraDominicalDiurna),
            Some(dec("86000"))
        );
        assert_eq!(
            valor(&l, ConceptoNomina::RecargoNocturno),
            Some(dec("28000"))
        );
        assert_eq!(l.ibc.valor(), dec("2339000"));
        assert!(
            Novedad::new(
                TipoNovedad::RecargoDominical,
                fecha(2026, 3, 9),
                None,
                Some(dec("2"))
            )
            .is_err(),
            "el 9 de marzo es lunes"
        );
    }

    /// Caso de referencia 4: incapacidad de origen común de 5 días.
    #[test]
    fn incapacidad_comun_de_cinco_dias() {
        let n = vec![
            Novedad::new(
                TipoNovedad::IncapacidadComun,
                fecha(2026, 3, 10),
                Some(fecha(2026, 3, 14)),
                None,
            )
            .expect("novedad"),
        ];
        let e = marzo(
            vec![tramo("3000000", fecha(2026, 3, 1), fecha(2026, 3, 31))],
            n,
        );
        let l = liquidar(&e, &normativa()).expect("liquida");

        // 25 días laborados × 100.000.
        assert_eq!(valor(&l, ConceptoNomina::Basico), Some(dec("2500000")));
        // 5 × 100.000 × 66,67 % = 333.350 (sobre el mínimo diario de 50.000).
        assert_eq!(
            valor(&l, ConceptoNomina::IncapacidadComun),
            Some(dec("333350"))
        );
        // Hasta dos mínimos (inclusive) hay auxilio: 25 días laborados × 6.666,67.
        assert_eq!(
            valor(&l, ConceptoNomina::AuxilioTransporte),
            Some(dec("166667"))
        );
        assert_eq!(l.ibc.valor(), dec("2833350"));
    }

    #[test]
    fn la_incapacidad_no_baja_del_minimo_diario() {
        let n = vec![
            Novedad::new(
                TipoNovedad::IncapacidadComun,
                fecha(2026, 3, 10),
                Some(fecha(2026, 3, 11)),
                None,
            )
            .expect("novedad"),
        ];
        let e = marzo(
            vec![tramo("1500000", fecha(2026, 3, 1), fecha(2026, 3, 31))],
            n,
        );
        let l = liquidar(&e, &normativa()).expect("liquida");
        // 2 × máx(50.000 × 66,67 %, 50.000).
        assert_eq!(
            valor(&l, ConceptoNomina::IncapacidadComun),
            Some(dec("100000"))
        );
        assert_eq!(
            valor(&l, ConceptoNomina::AuxilioTransporte),
            Some(dec("186667"))
        );
    }

    #[test]
    fn el_exceso_de_pagos_no_salariales_entra_al_ibc() {
        let n = vec![
            Novedad::new(
                TipoNovedad::BonificacionNoSalarial,
                fecha(2026, 3, 20),
                None,
                Some(dec("2000000")),
            )
            .expect("novedad"),
        ];
        let e = marzo(
            vec![tramo("2000000", fecha(2026, 3, 1), fecha(2026, 3, 31))],
            n,
        );
        let l = liquidar(&e, &normativa()).expect("liquida");
        // Total 4.000.000; límite 40 % = 1.600.000; exceso 400.000.
        assert_eq!(l.ibc.valor(), dec("2400000"));
    }

    #[test]
    fn ingreso_a_mitad_de_mes_y_aumento_en_el_periodo() {
        let e = marzo(
            vec![
                tramo("1500000", fecha(2026, 3, 16), fecha(2026, 3, 20)),
                tramo("3000000", fecha(2026, 3, 21), fecha(2026, 3, 31)),
            ],
            vec![],
        );
        let l = liquidar(&e, &normativa()).expect("liquida");
        // 5 × 50.000 + 10 × 100.000.
        assert_eq!(l.dias, 15);
        assert_eq!(valor(&l, ConceptoNomina::Basico), Some(dec("1250000")));
        // Los dos salarios llegan hasta dos mínimos: auxilio por los 15 días.
        assert_eq!(
            valor(&l, ConceptoNomina::AuxilioTransporte),
            Some(dec("100000"))
        );
    }

    #[test]
    fn el_aprendiz_lectivo_solo_cotiza_salud_a_cargo_del_empleador() {
        let mut t = tramo("1125000", fecha(2026, 3, 1), fecha(2026, 3, 31));
        t.tipo_salario = TipoSalario::ApoyoSostenimiento;
        t.etapa_aprendizaje = Some(EtapaAprendizaje::Lectiva);
        t.clase_riesgo = None;
        let l = liquidar(&marzo(vec![t], vec![]), &normativa()).expect("liquida");
        assert_eq!(
            valor(&l, ConceptoNomina::ApoyoSostenimiento),
            Some(dec("1125000"))
        );
        assert_eq!(l.deducciones.valor(), Decimal::ZERO);
        // IBC con el mínimo: 1.500.000 × 12,5 %.
        assert_eq!(
            valor(&l, ConceptoNomina::SaludEmpleador),
            Some(dec("187500"))
        );
        assert_eq!(l.lineas.len(), 2, "el aprendiz no causa prestaciones");
    }

    #[test]
    fn la_segunda_quincena_retiene_sobre_el_mes() {
        let n = normativa();
        let mut t = tramo("10000000", fecha(2026, 3, 1), fecha(2026, 3, 15));
        let primera = EntradaLiquidacion {
            desde: fecha(2026, 3, 1),
            hasta: fecha(2026, 3, 15),
            tramos: vec![t],
            novedades: vec![],
            exonerada: false,
            retencion: Retencion::NoAplica,
            prestaciones: vec![],
        };
        let q1 = liquidar(&primera, &n).expect("primera");
        assert_eq!(valor(&q1, ConceptoNomina::RetencionFuente), None);
        t.desde = fecha(2026, 3, 16);
        t.hasta = fecha(2026, 3, 31);
        let segunda = EntradaLiquidacion {
            desde: fecha(2026, 3, 16),
            hasta: fecha(2026, 3, 31),
            tramos: vec![t],
            novedades: vec![],
            exonerada: false,
            retencion: Retencion::CierreMes(q1.acumulado),
            prestaciones: vec![],
        };
        let q2 = liquidar(&segunda, &n).expect("segunda");
        let mes = liquidar(
            &marzo(
                vec![tramo("10000000", fecha(2026, 3, 1), fecha(2026, 3, 31))],
                vec![],
            ),
            &n,
        )
        .expect("mes");
        assert_eq!(
            valor(&q2, ConceptoNomina::RetencionFuente),
            valor(&mes, ConceptoNomina::RetencionFuente)
        );
    }

    #[test]
    fn las_deducciones_no_superan_los_devengos() {
        let n = vec![
            Novedad::new(
                TipoNovedad::Prestamo,
                fecha(2026, 3, 5),
                None,
                Some(dec("5000000")),
            )
            .expect("novedad"),
        ];
        let e = marzo(
            vec![tramo("1500000", fecha(2026, 3, 1), fecha(2026, 3, 31))],
            n,
        );
        assert!(liquidar(&e, &normativa()).is_err());
    }

    #[test]
    fn la_novedad_valida_su_medida() {
        let d = fecha(2026, 3, 4);
        assert!(Novedad::new(TipoNovedad::HoraExtraDiurna, d, None, Some(dec("25"))).is_err());
        assert!(Novedad::new(TipoNovedad::HoraExtraDiurna, d, Some(d), Some(dec("2"))).is_err());
        assert!(Novedad::new(TipoNovedad::Ausencia, d, None, None).is_err());
        assert!(Novedad::new(TipoNovedad::Ausencia, d, Some(fecha(2026, 3, 3)), None).is_err());
        assert!(Novedad::new(TipoNovedad::Libranza, d, None, Some(dec("-1"))).is_err());
        assert!(TipoNovedad::IncapacidadComun.es_sensible());
        for t in TipoNovedad::TODOS {
            assert_eq!(TipoNovedad::try_from(t.as_str()), Ok(*t));
        }
        for c in ConceptoNomina::TODOS {
            assert_eq!(ConceptoNomina::try_from(c.as_str()), Ok(*c));
        }
        assert_eq!(
            ConceptoNomina::IncapacidadComun.etiqueta_desprendible(),
            "Ausencia remunerada"
        );
        assert_eq!(
            ConceptoNomina::Basico.etiqueta_desprendible(),
            "Salario básico"
        );
        assert!(ConceptoNomina::Basico.salarial());
        assert!(!ConceptoNomina::AuxilioTransporte.salarial());
        assert!(!ConceptoNomina::Salud.salarial());
    }
}
