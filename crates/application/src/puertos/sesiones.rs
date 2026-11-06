//! Sesiones de conversación de WhatsApp (Redis). No son fuente de verdad: si se pierden, el
//! usuario vuelve a elegir la reunión.

use std::{future::Future, time::Duration};

use domain::{ReunionId, TenantId};
use serde::{Deserialize, Serialize};

use crate::AppResult;

/// Estado mínimo de la conversación. Clave en Redis: `wa:sesion:{linea}:{telefono}`
/// (por línea **y** teléfono; la sesión se resuelve contra las reuniones activas).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SesionConversacion {
    pub tenant_id: TenantId,
    pub reunion_id: ReunionId,
}

pub trait SesionesConversacion: Send + Sync {
    fn obtener(
        &self,
        linea: &str,
        telefono: &str,
    ) -> impl Future<Output = AppResult<Option<SesionConversacion>>> + Send;

    fn guardar(
        &self,
        linea: &str,
        telefono: &str,
        sesion: &SesionConversacion,
        ttl: Duration,
    ) -> impl Future<Output = AppResult<()>> + Send;
}
