//! Identificadores tipados: impiden pasar un `BienPrivadoId` donde se espera un `PreguntaId`.
//!
//! Todas las entidades usan **UUID v7** (ordenables por tiempo), excepto [`VotoId`], que usa
//! **UUID v4** para no filtrar la hora del voto (secreto del voto).

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_entidad {
    ($(#[$meta:meta])* $nombre:ident, $generador:path) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $nombre(Uuid);

        impl $nombre {
            /// Genera un identificador nuevo.
            pub fn nuevo() -> Self {
                Self($generador())
            }

            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl From<Uuid> for $nombre {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl From<$nombre> for Uuid {
            fn from(id: $nombre) -> Self {
                id.0
            }
        }

        impl fmt::Display for $nombre {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

id_entidad!(
    /// Copropiedad (PH). Es la unidad de aislamiento multi-tenant (`tenant_id` + RLS).
    TenantId,
    Uuid::now_v7
);
id_entidad!(
    /// Reunión de asamblea (ordinaria o extraordinaria).
    ReunionId,
    Uuid::now_v7
);
id_entidad!(PreguntaId, Uuid::now_v7);
id_entidad!(OpcionId, Uuid::now_v7);
id_entidad!(
    /// Bien privado o de dominio particular (Ley 675, art. 3): unidad con coeficiente de copropiedad.
    BienPrivadoId,
    Uuid::now_v7
);
id_entidad!(
    /// Persona natural: propietario o apoderado.
    PersonaId,
    Uuid::now_v7
);
id_entidad!(
    /// Inscripción de un bien privado a una reunión (enlace, prerregistro o QR en puerta).
    InscripcionId,
    Uuid::now_v7
);
id_entidad!(ParticipacionId, Uuid::now_v7);
id_entidad!(
    /// Archivo subido a una copropiedad (`archivos`).
    ArchivoId,
    Uuid::now_v7
);
id_entidad!(
    /// Versión de la política de tratamiento de datos de una copropiedad.
    PoliticaTratamientoId,
    Uuid::now_v7
);
id_entidad!(
    /// Autorización de tratamiento de datos de un titular para una finalidad.
    AutorizacionId,
    Uuid::now_v7
);
id_entidad!(
    /// Consulta o reclamo de un titular sobre sus datos (Ley 1581, arts. 14 y 15).
    SolicitudTitularId,
    Uuid::now_v7
);
id_entidad!(
    /// Cuenta global de una persona en la plataforma (inicia sesión una vez, opera en varias PH).
    CuentaId,
    Uuid::now_v7
);
id_entidad!(
    /// Sesión de una cuenta en un dispositivo.
    SesionId,
    Uuid::now_v7
);
id_entidad!(
    /// Rol (plantilla del sistema o propio de una copropiedad).
    RolId,
    Uuid::now_v7
);
id_entidad!(
    /// Asignación de un rol a una cuenta, con vigencia.
    AsignacionRolId,
    Uuid::now_v7
);
id_entidad!(
    /// Organización (empresa o persona natural con RUT) compartida entre copropiedades.
    OrganizacionId,
    Uuid::now_v7
);
id_entidad!(
    /// Contrato de servicio entre una copropiedad y una organización.
    ContratoServicioId,
    Uuid::now_v7
);
id_entidad!(
    /// Solicitud para reclamar la ficha de una organización.
    ReclamacionId,
    Uuid::now_v7
);
id_entidad!(
    /// Identificador de un voto. **Siempre UUID v4**: un v7 revelaría la hora de emisión.
    VotoId,
    Uuid::new_v4
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voto_usa_uuid_v4_y_entidades_v7() {
        assert_eq!(VotoId::nuevo().as_uuid().get_version_num(), 4);
        assert_eq!(PreguntaId::nuevo().as_uuid().get_version_num(), 7);
        assert_eq!(TenantId::nuevo().as_uuid().get_version_num(), 7);
    }
}
