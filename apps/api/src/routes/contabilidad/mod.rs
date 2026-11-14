//! Rutas del núcleo contable (plan 12, ADR 0017): `/api/v1/entidades/{entidad_id}/contabilidad/…`
//! con el extractor [`crate::sesion::EnEntidad`] (sirven a cualquier entidad; en una PH el id es
//! el de la copropiedad). Más las de plataforma (alta de entidades, PUC y plantillas estándar) y
//! las de las firmas contables (sus plantillas). Tesorería, cuentas por pagar, facturas de venta
//! y conciliación (plan 13), y lo tributario con los documentos electrónicos (plan 15) cuelgan de
//! la misma ruta.

mod comprobantes;
mod conciliacion;
mod configuracion;
mod cuentas_por_pagar;
mod dto;
pub(crate) mod dto_tesoreria;
mod dto_tributario;
mod facturacion;
mod plataforma;
mod reportes;
mod tesoreria;
mod tributario;
mod ventas;

use utoipa_axum::router::OpenApiRouter;

pub use plataforma::{rutas_organizacion, rutas_plataforma};

use crate::state::AppState;

/// Rutas contables de una entidad (se montan en `/entidades/{entidad_id}/contabilidad`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(configuracion::rutas())
        .merge(comprobantes::rutas())
        .merge(reportes::rutas())
        .merge(tesoreria::rutas())
        .merge(cuentas_por_pagar::rutas())
        .merge(ventas::rutas())
        .merge(conciliacion::rutas())
        .merge(tributario::rutas())
        .merge(facturacion::rutas())
}
