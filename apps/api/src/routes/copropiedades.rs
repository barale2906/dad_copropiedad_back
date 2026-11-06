//! Rutas de una copropiedad: `/api/v1/copropiedades/{copropiedad_id}/…`.
//!
//! La copropiedad va **explícita en la ruta** (decisión D2 de la hoja de ruta): el cliente
//! elige en qué copropiedad trabaja y puede cambiar sin volver a autenticarse. Cada módulo
//! (bienes privados, bienes comunes, reservas, asambleas…) agrega aquí su router.
//!
//! Cuando exista autenticación (plan 02), un extractor validará que la cuenta tenga un rol
//! vigente en `copropiedad_id` antes de abrir `infra_postgres::transaccion_tenant`. Aún no hay
//! endpoints.

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// Nombre del parámetro de ruta con el id de la copropiedad (= `tenant_id`).
pub const PARAMETRO_COPROPIEDAD: &str = "copropiedad_id";

/// Router de una copropiedad (se monta en `/api/v1/copropiedades/{copropiedad_id}`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}
