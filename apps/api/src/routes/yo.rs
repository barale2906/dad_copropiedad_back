//! Rutas de la cuenta autenticada: `/api/v1/yo/…`.
//!
//! Datos propios de quien inició sesión, sin depender de una copropiedad: perfil, sesiones y
//! la lista de copropiedades donde tiene roles, para elegir en cuál trabajar (plan 02).
//! Aún no hay endpoints.

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// Router de la cuenta autenticada (se monta en `/api/v1/yo`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
}
