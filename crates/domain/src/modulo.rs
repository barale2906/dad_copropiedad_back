//! Módulos que se contratan por entidad (venta por suscripción y por módulo, ADR 0016).
//!
//! Cada módulo se habilita uno a uno. Lo básico no es un módulo: la identidad y lo transversal
//! están en toda entidad, y lo básico de la PH (copropiedad, bienes privados, censo) viene con
//! la copropiedad (una entidad es PH porque tiene fila en `copropiedades`). Los módulos propios
//! de la PH ([`Modulo::exige_copropiedad`]) solo se habilitan en una copropiedad.

use serde::{Deserialize, Serialize};

/// Módulo contratable. El texto es contrato con el `CHECK` de `modulos_habilitados.modulo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modulo {
    /// Contabilidad base para cualquier entidad (plan 12): plan de cuentas, comprobantes,
    /// periodos, reportes y estados financieros.
    Contabilidad,
    /// Contabilidad de la propiedad horizontal (plan 14): presupuesto, liquidación de expensas,
    /// cartera por bien privado y recaudo. Exige además [`Modulo::Contabilidad`].
    ContabilidadPh,
    /// Facturación electrónica y documento soporte (plan 15): para cualquier entidad que
    /// factura electrónicamente.
    FacturacionElectronica,
    /// Bienes comunes, reservas y parqueaderos (plan 04).
    BienesComunes,
    /// Control de acceso de residentes, visitantes y vehículos (plan 05).
    ControlAcceso,
    /// Asambleas y votaciones (plan 06).
    Asambleas,
    /// Tickets, chat de WhatsApp, mantenimiento e inventario (plan 07).
    GestionAdministrativa,
    /// Publicidad de comercios locales (plan 08).
    Publicidad,
}

enum_texto!(Modulo {
    Contabilidad => "contabilidad",
    ContabilidadPh => "contabilidad_ph",
    FacturacionElectronica => "facturacion_electronica",
    BienesComunes => "bienes_comunes",
    ControlAcceso => "control_acceso",
    Asambleas => "asambleas",
    GestionAdministrativa => "gestion_administrativa",
    Publicidad => "publicidad",
});

impl Modulo {
    /// ¿Es un módulo de la vertical PH? Solo se habilita en una entidad que es copropiedad.
    pub fn exige_copropiedad(self) -> bool {
        match self {
            Modulo::Contabilidad | Modulo::FacturacionElectronica => false,
            Modulo::ContabilidadPh
            | Modulo::BienesComunes
            | Modulo::ControlAcceso
            | Modulo::Asambleas
            | Modulo::GestionAdministrativa
            | Modulo::Publicidad => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texto_persistido_ida_y_vuelta() {
        for m in Modulo::TODOS {
            assert_eq!(Modulo::try_from(m.as_str()), Ok(*m));
        }
        assert!(Modulo::try_from("cartera").is_err());
    }

    #[test]
    fn la_contabilidad_sirve_a_cualquier_entidad_y_lo_demas_es_de_la_ph() {
        assert!(!Modulo::Contabilidad.exige_copropiedad());
        for m in Modulo::TODOS
            .iter()
            .filter(|m| !matches!(m, Modulo::Contabilidad | Modulo::FacturacionElectronica))
        {
            assert!(m.exige_copropiedad(), "{m:?}");
        }
    }
}
