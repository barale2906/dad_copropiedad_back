//! Control de acceso, visitas y correspondencia (plan 05, módulo `control_acceso`), dentro de
//! `/api/v1/copropiedades/{copropiedad_id}`:
//!
//! - `/visitas`: anunciar, pre-autorizar (QR), decidir, llamar, ingreso y salida ([`visitas`]).
//! - `/correspondencia`: recibir, entregar con firma o foto, devolver ([`correspondencia`]).
//! - `/acceso/configuracion`, `/acceso/alertas`, `/acceso/costo-avisos`, `/minuta`,
//!   `/eventos-acceso` ([`porteria`]).
//! - `/puntos-acceso`, `/puentes`, `/dispositivos-acceso` ([`infraestructura`]).
//! - `/credenciales`, `/autorizaciones-acceso`, `/personas/{persona_id}/consentimiento-biometrico`
//!   ([`credenciales`]).
//!
//! Fuera de la copropiedad: la API de los puentes en `/api/v1/puentes` (token de máquina,
//! [`puentes`]) y los dispositivos de la app en `/api/v1/yo/dispositivos-push` ([`push`]).
//!
//! Los archivos (firma, fotos, adjuntos) viajan en Base64 dentro del JSON: la portería los toma
//! con la cámara de la tablet y la cuenta del vigilante no necesita `archivos.subir`.

mod correspondencia;
mod credenciales;
mod dto;
mod infraestructura;
mod porteria;
pub mod puentes;
pub mod push;
mod visitas;

use axum::extract::DefaultBodyLimit;
use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// Cuerpo máximo de las rutas que reciben fotos en Base64 (≈ 3 fotos de 5 MB).
pub const LIMITE_CUERPO: usize = 24 * 1024 * 1024;

/// Rutas de control de acceso de una copropiedad.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(visitas::rutas())
        .merge(correspondencia::rutas())
        .merge(porteria::rutas())
        .merge(infraestructura::rutas())
        .merge(credenciales::rutas())
        .layer(DefaultBodyLimit::max(LIMITE_CUERPO))
}
