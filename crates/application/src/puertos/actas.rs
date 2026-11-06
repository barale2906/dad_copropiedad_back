//! Generación de actas. Implementación prevista: `crates/infra-pdf` (Typst).

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::{ReunionId, TenantId};

use crate::AppResult;

/// Datos ya calculados para el acta. El generador solo da formato: no decide nada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosActa {
    pub tenant_id: TenantId,
    pub reunion_id: ReunionId,
    pub titulo: String,
    pub instalada_en: DateTime<Utc>,
    pub cerrada_en: DateTime<Utc>,
}

pub trait GeneradorActas: Send + Sync {
    /// Devuelve el PDF del acta.
    fn generar(&self, datos: &DatosActa) -> impl Future<Output = AppResult<Vec<u8>>> + Send;
}
