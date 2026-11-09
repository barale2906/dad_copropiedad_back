//! Rutas de una copropiedad: `/api/v1/copropiedades/{copropiedad_id}/…`.
//!
//! La copropiedad va **explícita en la ruta** (decisión D2 de la hoja de ruta): el cliente
//! elige en qué copropiedad trabaja y puede cambiar sin volver a autenticarse. Cada módulo
//! (bienes privados, bienes comunes, reservas, asambleas…) agrega aquí su router.
//!
//! Cada handler usa el extractor [`crate::sesion::EnCopropiedad`]: exige sesión, carga los
//! permisos de la cuenta en `copropiedad_id` y responde 404 si no tiene ningún rol allí.

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// Nombre del parámetro de ruta con el id de la copropiedad (= `tenant_id`).
pub const PARAMETRO_COPROPIEDAD: &str = "copropiedad_id";

/// Router de una copropiedad (se monta en `/api/v1/copropiedades/{copropiedad_id}`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(super::ficha::rutas())
        .merge(super::bienes_privados::rutas())
        .merge(super::coeficientes::rutas())
        .merge(super::censo::rutas())
        .merge(super::reservas::rutas())
        .merge(super::parqueaderos::rutas())
        .merge(super::tickets::rutas())
        .merge(super::conversaciones::rutas())
        .merge(super::mantenimiento::rutas())
        .merge(super::asambleas::rutas())
        .merge(super::acceso::rutas())
        .merge(super::importaciones::rutas())
        .merge(super::roles::rutas())
        .merge(super::organizaciones::rutas_copropiedad())
        .merge(super::archivos::rutas_copropiedad())
}
