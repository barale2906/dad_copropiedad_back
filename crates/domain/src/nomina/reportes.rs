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

/// Obligación legal de la nómina que se recuerda antes de su fecha límite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recordatorio {
    /// Tipo del aviso (`intereses_cesantias`, `consignacion_cesantias`, `prima`).
    pub tipo: &'static str,
    /// Referencia única del aviso (año, o año y semestre).
    pub referencia: String,
    pub asunto: String,
    pub texto: String,
}

/// Recordatorios que tocan en la fecha (dos semanas antes de cada límite legal): intereses de las
/// cesantías (31 de enero), consignación de las cesantías al fondo (14 de febrero) y prima de
/// servicios (30 de junio y 20 de diciembre).
pub fn recordatorios_legales(hoy: chrono::NaiveDate) -> Vec<Recordatorio> {
    use chrono::Datelike;
    let (a, m, d) = (hoy.year(), hoy.month(), hoy.day());
    let mut v = Vec::new();
    if m == 1 && d >= 15 {
        v.push(Recordatorio {
            tipo: "intereses_cesantias",
            referencia: (a - 1).to_string(),
            asunto: "Intereses sobre las cesantías".into(),
            texto: format!(
                "Recuerde pagar a los empleados los intereses sobre las cesantías de {} antes del 31 \
                 de enero. La nómina de enero los incluye: liquídela y apruébela a tiempo.",
                a - 1
            ),
        });
    }
    if m == 2 && d <= 14 {
        v.push(Recordatorio {
            tipo: "consignacion_cesantias",
            referencia: (a - 1).to_string(),
            asunto: "Consignación de las cesantías".into(),
            texto: format!(
                "Las cesantías de {} se consignan en el fondo de cada empleado a más tardar el 14 de \
                 febrero. Consulte el reporte de cesantías por consignar en la nómina.",
                a - 1
            ),
        });
    }
    let prima = match (m, d) {
        (6, 15..=30) => Some(("1", "30 de junio")),
        (12, 5..=20) => Some(("2", "20 de diciembre")),
        _ => None,
    };
    if let Some((semestre, limite)) = prima {
        v.push(Recordatorio {
            tipo: "prima",
            referencia: format!("{a}-{semestre}"),
            asunto: "Prima de servicios".into(),
            texto: format!(
                "La prima de servicios del semestre se paga a más tardar el {limite}. La nómina del \
                 periodo la incluye: liquídela y apruébela a tiempo."
            ),
        });
    }
    v
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

    #[test]
    fn los_recordatorios_salen_dos_semanas_antes_del_limite() {
        let f = |m, d| chrono::NaiveDate::from_ymd_opt(2027, m, d).expect("fecha");
        let tipos = |m, d| {
            recordatorios_legales(f(m, d))
                .into_iter()
                .map(|r| (r.tipo, r.referencia))
                .collect::<Vec<_>>()
        };
        assert_eq!(tipos(1, 14), vec![]);
        assert_eq!(
            tipos(1, 20),
            vec![("intereses_cesantias", "2026".to_owned())]
        );
        assert_eq!(
            tipos(2, 3),
            vec![("consignacion_cesantias", "2026".to_owned())]
        );
        assert_eq!(tipos(2, 15), vec![]);
        assert_eq!(tipos(6, 20), vec![("prima", "2027-1".to_owned())]);
        assert_eq!(tipos(12, 10), vec![("prima", "2027-2".to_owned())]);
        assert_eq!(tipos(12, 21), vec![]);
    }
}
