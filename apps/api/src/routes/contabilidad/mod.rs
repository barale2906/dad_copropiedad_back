//! Rutas del núcleo contable (plan 12, ADR 0017): `/api/v1/entidades/{entidad_id}/contabilidad/…`
//! con el extractor [`crate::sesion::EnEntidad`] (sirven a cualquier entidad; en una PH el id es
//! el de la copropiedad). Más las de plataforma (alta de entidades, PUC y plantillas estándar) y
//! las de las firmas contables (sus plantillas).

mod comprobantes;
mod configuracion;
mod dto;
mod plataforma;
mod reportes;

use utoipa_axum::router::OpenApiRouter;

pub use plataforma::{rutas_organizacion, rutas_plataforma};

use crate::state::AppState;

/// Rutas contables de una entidad (se montan en `/entidades/{entidad_id}/contabilidad`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(configuracion::rutas())
        .merge(comprobantes::rutas())
        .merge(reportes::rutas())
}
