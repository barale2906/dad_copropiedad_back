//! Módulos que se contratan por copropiedad (venta por suscripción y por módulo).
//!
//! Los módulos base (identidad, copropiedad y bienes privados, censo) siempre están activos y
//! no aparecen aquí.

use serde::{Deserialize, Serialize};

/// Módulo contratable. El texto es contrato con el `CHECK` de `modulos_habilitados.modulo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modulo {
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
    BienesComunes => "bienes_comunes",
    ControlAcceso => "control_acceso",
    Asambleas => "asambleas",
    GestionAdministrativa => "gestion_administrativa",
    Publicidad => "publicidad",
});

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
}
