//! Organizaciones compartidas entre copropiedades (administradoras, contables, seguridad, aseo,
//! mantenimiento, anunciantes…) y sus contratos de servicio con cada copropiedad.
//!
//! La ficha global (identificación y contacto público) la crea el administrador de cualquier PH
//! y se reutiliza buscando por NIT. Mientras está sin reclamar la edita quien la creó y soporte;
//! una vez reclamada (revisión de soporte), solo sus miembros dueño o gestor.

use serde::{Deserialize, Serialize};

/// A qué se dedica la organización (puede tener varios tipos).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoOrganizacion {
    Administradora,
    Contable,
    RevisoriaFiscal,
    Seguridad,
    Aseo,
    Mantenimiento,
    Anunciante,
    Otro,
}

enum_texto!(TipoOrganizacion {
    Administradora => "administradora",
    Contable => "contable",
    RevisoriaFiscal => "revisoria_fiscal",
    Seguridad => "seguridad",
    Aseo => "aseo",
    Mantenimiento => "mantenimiento",
    Anunciante => "anunciante",
    Otro => "otro",
});

/// Persona jurídica o persona natural con RUT (su NIT es su cédula: dato personal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NaturalezaOrganizacion {
    Juridica,
    NaturalConRut,
}

enum_texto!(NaturalezaOrganizacion {
    Juridica => "juridica",
    NaturalConRut => "natural_con_rut",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoOrganizacion {
    /// Creada por una copropiedad; la empresa aún no la administra.
    SinReclamar,
    /// La empresa la reclamó y soporte lo aprobó: solo sus dueños y gestores la editan.
    Reclamada,
    Suspendida,
}

enum_texto!(EstadoOrganizacion {
    SinReclamar => "sin_reclamar",
    Reclamada => "reclamada",
    Suspendida => "suspendida",
});

/// Papel de una cuenta dentro de una organización.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolOrganizacion {
    Dueno,
    Gestor,
    Operario,
}

enum_texto!(RolOrganizacion {
    Dueno => "dueno",
    Gestor => "gestor",
    Operario => "operario",
});

impl RolOrganizacion {
    /// ¿Puede editar la ficha global de una organización reclamada?
    pub const fn edita_ficha(self) -> bool {
        matches!(self, Self::Dueno | Self::Gestor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoReclamacion {
    Pendiente,
    Aprobada,
    Rechazada,
}

enum_texto!(EstadoReclamacion {
    Pendiente => "pendiente",
    Aprobada => "aprobada",
    Rechazada => "rechazada",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoContrato {
    Vigente,
    /// Terminado antes o al llegar su fecha: vencen los roles derivados del contrato.
    Terminado,
}

enum_texto!(EstadoContrato {
    Vigente => "vigente",
    Terminado => "terminado",
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_dueno_y_gestor_editan_la_ficha() {
        assert!(RolOrganizacion::Dueno.edita_ficha());
        assert!(RolOrganizacion::Gestor.edita_ficha());
        assert!(!RolOrganizacion::Operario.edita_ficha());
        assert_eq!(
            TipoOrganizacion::RevisoriaFiscal.as_str(),
            "revisoria_fiscal"
        );
    }
}
