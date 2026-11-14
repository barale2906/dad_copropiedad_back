//! Rutas del núcleo contable (plan 12, ADR 0017): `/api/v1/entidades/{entidad_id}/contabilidad/…`
//! con el extractor [`crate::sesion::EnEntidad`] (sirven a cualquier entidad; en una PH el id es
//! el de la copropiedad). Más las de plataforma (alta de entidades, PUC y plantillas estándar) y
//! las de las firmas contables (sus plantillas). Tesorería, cuentas por pagar y por cobrar,
//! conciliación (plan 13) y las declaraciones tributarias (plan 15) cuelgan de la misma ruta. Las
//! facturas y los documentos electrónicos están en [`crate::routes::facturacion`]; el perfil
//! tributario y los terceros, en las rutas comunes de la entidad (ADR 0022).

mod comprobantes;
mod conciliacion;
mod configuracion;
mod cuentas_por_cobrar;
pub(crate) mod cuentas_por_pagar;
pub(crate) mod dto;
pub(crate) mod dto_tesoreria;
pub(crate) mod dto_tributario;
mod plataforma;
pub(crate) mod reportes;
mod tesoreria;
pub(crate) mod tributario;

use utoipa_axum::router::OpenApiRouter;

pub use configuracion::rutas_terceros;
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
        .merge(cuentas_por_cobrar::rutas())
        .merge(conciliacion::rutas())
        .merge(tributario::rutas())
}
