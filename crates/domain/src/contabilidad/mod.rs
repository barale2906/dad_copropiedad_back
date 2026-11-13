//! Núcleo contable genérico (plan 12, ADR 0017): sirve a cualquier entidad y **no conoce a la
//! PH** (la prueba `tests/fronteras.rs` lo hace cumplir).
//!
//! - [`Monto`]: dinero en la moneda funcional, nunca negativo, con dos decimales.
//! - [`CodigoCuenta`], [`Naturaleza`], [`NivelCuenta`]: el plan de cuentas (PUC y auxiliares).
//! - [`PeriodoContable`] y [`EstadoPeriodo`]: meses y cierre anual (mes 13).
//! - [`AsientoBalanceado`]: la partida doble; solo se contabiliza lo que se pudo construir así.
//! - [`ReglaContable`]: convierte los importes de un documento origen en un asiento.
//! - [`PlantillaContable`]: lo que el contador elige y reutiliza por tipo de empresa (P1).
//! - [`estados`]: estados financieros y cierre anual a partir de saldos.

mod asiento;
mod cuenta;
pub mod estados;
mod monto;
mod periodo;
mod plantilla;
mod regla;

pub use asiento::{AsientoBalanceado, LineaAsiento, ReglasCuenta};
pub use cuenta::{CodigoCuenta, Naturaleza, NivelCuenta};
pub use monto::Monto;
pub use periodo::{EstadoPeriodo, EstadoReapertura, MarcoTecnico, PeriodoContable};
pub use plantilla::{
    CuentaPlantilla, LineaReglaPlantilla, PlantillaContable, ReglaPlantilla, RubroPlantilla,
    TipoComprobantePlantilla,
};
pub use regla::{Cara, ImporteDocumento, LineaRegla, OrigenCentro, OrigenTercero, ReglaContable};

/// Mes del cierre anual (periodo 13).
pub use periodo::MES_CIERRE;

/// Estado de un comprobante: el borrador se edita; el contabilizado es inmutable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoComprobante {
    Borrador,
    Contabilizado,
}

enum_texto!(EstadoComprobante {
    Borrador => "borrador",
    Contabilizado => "contabilizado",
});

/// Régimen de IVA de un tercero (dato del RUT).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegimenIva {
    Responsable,
    NoResponsable,
    Simple,
}

enum_texto!(RegimenIva {
    Responsable => "responsable",
    NoResponsable => "no_responsable",
    Simple => "simple",
});

/// Responsabilidad fiscal del RUT que el sistema usa (gran contribuyente, autorretenedor,
/// agente de retención de IVA, régimen simple, no responsable). Contrato con el `CHECK` de
/// `personas.responsabilidades_fiscales`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum ResponsabilidadFiscal {
    #[serde(rename = "O-13")]
    GranContribuyente,
    #[serde(rename = "O-15")]
    Autorretenedor,
    #[serde(rename = "O-23")]
    AgenteRetencionIva,
    #[serde(rename = "O-47")]
    RegimenSimple,
    #[serde(rename = "R-99-PN")]
    NoResponsable,
}

enum_texto!(ResponsabilidadFiscal {
    GranContribuyente => "O-13",
    Autorretenedor => "O-15",
    AgenteRetencionIva => "O-23",
    RegimenSimple => "O-47",
    NoResponsable => "R-99-PN",
});

/// Código de un tipo de comprobante (`RC`, `CE`, `CC`…): de 2 a 5 caracteres, mayúsculas y
/// dígitos, empezando por letra. Es la serie de su consecutivo (`comprobante:<código>`).
///
/// # Errors
///
/// [`crate::DominioError::DatoInvalido`] si no cumple el formato.
pub fn validar_codigo_tipo_comprobante(codigo: &str) -> Result<(), crate::DominioError> {
    let mut letras = codigo.chars();
    let valido = (2..=5).contains(&codigo.len())
        && letras.next().is_some_and(|c| c.is_ascii_uppercase())
        && letras.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if valido {
        Ok(())
    } else {
        Err(crate::DominioError::DatoInvalido {
            campo: "codigo",
            regla: "de 2 a 5 mayúsculas o dígitos, empezando por letra",
        })
    }
}

/// Código de un evento o concepto contable: minúsculas, dígitos, `_` y `.` (eventos), empezando
/// por letra. `largo` es el máximo permitido.
pub(crate) fn es_identificador(texto: &str, con_punto: bool, largo: usize) -> bool {
    let mut c = texto.chars();
    (2..=largo).contains(&texto.len())
        && c.next().is_some_and(|x| x.is_ascii_lowercase())
        && c.all(|x| {
            x.is_ascii_lowercase() || x.is_ascii_digit() || x == '_' || (con_punto && x == '.')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El texto de serde (API) y el de la base de datos (`as_str`) son el mismo contrato.
    fn serde_igual_a_texto<T: serde::Serialize + Copy>(todos: &[T], texto: fn(T) -> &'static str) {
        for v in todos {
            assert_eq!(
                serde_json::to_value(v).expect("json"),
                serde_json::json!(texto(*v))
            );
        }
    }

    #[test]
    fn los_enums_contables_viajan_con_su_texto_persistido() {
        serde_igual_a_texto(MarcoTecnico::TODOS, MarcoTecnico::as_str);
        serde_igual_a_texto(EstadoPeriodo::TODOS, EstadoPeriodo::as_str);
        serde_igual_a_texto(EstadoReapertura::TODOS, EstadoReapertura::as_str);
        serde_igual_a_texto(EstadoComprobante::TODOS, EstadoComprobante::as_str);
        serde_igual_a_texto(Naturaleza::TODOS, Naturaleza::as_str);
        serde_igual_a_texto(NivelCuenta::TODOS, NivelCuenta::as_str);
        serde_igual_a_texto(Cara::TODOS, Cara::as_str);
        serde_igual_a_texto(OrigenTercero::TODOS, OrigenTercero::as_str);
        serde_igual_a_texto(OrigenCentro::TODOS, OrigenCentro::as_str);
        serde_igual_a_texto(RegimenIva::TODOS, RegimenIva::as_str);
        serde_igual_a_texto(ResponsabilidadFiscal::TODOS, ResponsabilidadFiscal::as_str);
    }

    #[test]
    fn codigos_de_tipo_de_comprobante() {
        for ok in ["RC", "CE", "CC", "AP", "CI", "FV2"] {
            assert!(validar_codigo_tipo_comprobante(ok).is_ok(), "{ok}");
        }
        for malo in ["R", "rc", "1C", "RECIBO", "R-C"] {
            assert!(validar_codigo_tipo_comprobante(malo).is_err(), "{malo}");
        }
    }

    #[test]
    fn identificadores_de_eventos_y_conceptos() {
        assert!(es_identificador("expensa_ordinaria", false, 60));
        assert!(es_identificador("cartera.liquidacion", true, 80));
        assert!(!es_identificador("cartera.liquidacion", false, 80));
        assert!(!es_identificador("Expensa", false, 60));
        assert!(!es_identificador("x", false, 60));
    }
}
