//! Rutas de la cartera de la PH (plan 14, ADR 0019; módulo `contabilidad_ph`):
//! `/api/v1/copropiedades/{copropiedad_id}/cartera/…` con el extractor
//! [`crate::sesion::EnCopropiedad`]. Los recaudos son recibos de caja de la tesorería
//! (`/entidades/{id}/contabilidad/tesoreria/…`) aplicados a las obligaciones de cada bien.

mod configuracion;
mod dto;
mod liquidacion;
mod mora;
mod recaudo;

use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// Rutas de la cartera (se montan dentro de `/copropiedades/{copropiedad_id}`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(configuracion::rutas())
        .merge(liquidacion::rutas())
        .merge(recaudo::rutas())
        .merge(mora::rutas())
}

/// Rutas de plataforma de la cartera (tasas de referencia; se montan en `/plataforma`).
pub fn rutas_plataforma() -> OpenApiRouter<AppState> {
    mora::rutas_plataforma()
}
