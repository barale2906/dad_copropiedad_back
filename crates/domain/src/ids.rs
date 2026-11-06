//! Identificadores tipados: impiden pasar un `InmuebleId` donde se espera un `PreguntaId`.
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
    /// Inmueble / unidad privada con coeficiente de copropiedad.
    InmuebleId,
    Uuid::now_v7
);
id_entidad!(
    /// Persona natural: propietario o apoderado.
    PersonaId,
    Uuid::now_v7
);
id_entidad!(
    /// Inscripción de un inmueble a una reunión (enlace, prerregistro o QR en puerta).
    InscripcionId,
    Uuid::now_v7
);
id_entidad!(ParticipacionId, Uuid::now_v7);
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
