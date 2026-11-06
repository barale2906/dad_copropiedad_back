//! Rutas globales de la plataforma: `/api/v1/plataforma/…`.
//!
//! Operaciones que no pertenecen a una copropiedad (alta de copropiedades, organizaciones,
//! moderación de publicidad). Solo para roles de plataforma (superusuario y administrativos,
//! plan 02). Aún no hay endpoints.

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// Router de la plataforma (se monta en `/api/v1/plataforma`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}
