//! Reglas de los reportes de nómina (plan 16 fases 5 y 7): el tipo de cotizante de la PILA y los
//! grupos del certificado de ingresos y retenciones por rentas de trabajo.
//!
//! El sistema **prepara** los insumos: el archivo plano de la PILA y el formulario 220 tienen
//! formatos prescritos que se generan cuando se tenga su especificación vigente.

use serde::{Deserialize, Serialize};

use super::{EtapaAprendizaje, TipoContratoLaboral, liquidacion::ConceptoNomina};

/// Tipo de cotizante de la PILA según el contrato.
pub const fn tipo_cotizante(
    tipo: TipoContratoLaboral,
    etapa: Option<EtapaAprendizaje>,
) -> &'static str {
    match (tipo, etapa) {
        (TipoContratoLaboral::Aprendizaje, Some(EtapaAprendizaje::Productiva)) => "19",
        (TipoContratoLaboral::Aprendizaje, _) => "12",
        _ => "01",
    }
}

/// Grupo del certificado de ingresos y retenciones donde suma un concepto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrupoCertificado {
    /// Pagos por salarios (básico, horas extra, recargos, incapacidades, licencias, vacaciones
    /// disfrutadas, bonificaciones salariales, apoyo de sostenimiento).
    Salarios,
    Comisiones,
    /// Prestaciones sociales (prima y vacaciones compensadas).
    Prestaciones,
    /// Cesantías e intereses efectivamente pagados.
    Cesantias,
    /// Otros pagos (auxilio de transporte, pagos no salariales, indemnización).
    OtrosPagos,
    AportesSalud,
    /// Aportes obligatorios a pensión y al fondo de solidaridad.
    AportesPension,
    RetencionFuente,
}

enum_texto!(GrupoCertificado {
    Salarios => "salarios",
    Comisiones => "comisiones",
    Prestaciones => "prestaciones",
    Cesantias => "cesantias",
    OtrosPagos => "otros_pagos",
    AportesSalud => "aportes_salud",
    AportesPension => "aportes_pension",
    RetencionFuente => "retencion_fuente",
});

impl GrupoCertificado {
    /// Grupo del concepto (`None` en lo que no va al certificado: préstamos, libranzas,
    /// embargos, aportes del empleador y provisiones).
    pub const fn de(concepto: ConceptoNomina) -> Option<Self> {
        use ConceptoNomina as C;
        Some(match concepto {
            C::Basico
            | C::ApoyoSostenimiento
            | C::HoraExtraDiurna
            | C::HoraExtraNocturna
            | C::HoraExtraDominicalDiurna
            | C::HoraExtraDominicalNocturna
            | C::RecargoNocturno
            | C::RecargoDominical
            | C::RecargoNocturnoDominical
            | C::IncapacidadComun
            | C::IncapacidadLaboral
            | C::LicenciaMaternidad
            | C::LicenciaPaternidad
            | C::LicenciaRemunerada
            | C::Vacaciones
            | C::BonificacionSalarial => Self::Salarios,
            C::Comision => Self::Comisiones,
            C::Prima | C::VacacionesCompensadas => Self::Prestaciones,
            C::Cesantias | C::InteresesCesantias => Self::Cesantias,
            C::AuxilioTransporte
            | C::BonificacionNoSalarial
            | C::AuxilioNoSalarial
            | C::Indemnizacion => Self::OtrosPagos,
            C::Salud => Self::AportesSalud,
            C::Pension | C::FondoSolidaridad => Self::AportesPension,
            C::RetencionFuente => Self::RetencionFuente,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_tipo_de_cotizante_sale_del_contrato() {
        assert_eq!(tipo_cotizante(TipoContratoLaboral::TerminoFijo, None), "01");
        assert_eq!(
            tipo_cotizante(
                TipoContratoLaboral::Aprendizaje,
                Some(EtapaAprendizaje::Lectiva)
            ),
            "12"
        );
        assert_eq!(
            tipo_cotizante(
                TipoContratoLaboral::Aprendizaje,
                Some(EtapaAprendizaje::Productiva)
            ),
            "19"
        );
    }

    #[test]
    fn cada_devengo_y_deduccion_del_empleado_tiene_grupo() {
        use crate::nomina::liquidacion::ClaseLinea;
        for c in ConceptoNomina::TODOS {
            let empleado = matches!(c.clase(), ClaseLinea::Devengo)
                || matches!(
                    c,
                    ConceptoNomina::Salud
                        | ConceptoNomina::Pension
                        | ConceptoNomina::FondoSolidaridad
                        | ConceptoNomina::RetencionFuente
                );
            assert_eq!(GrupoCertificado::de(*c).is_some(), empleado, "{c:?}");
        }
    }
}
