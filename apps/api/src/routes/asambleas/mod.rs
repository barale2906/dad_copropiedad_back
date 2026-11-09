//! Asambleas y votaciones (plan 06, módulo `asambleas`) dentro de
//! `/api/v1/copropiedades/{copropiedad_id}`:
//!
//! - `/asambleas`, `/asambleas/{reunion_id}`: reuniones, orden del día, anexos, convocatoria,
//!   transiciones, quórum, tablero y WebSocket en vivo ([`reuniones`]).
//! - `/asambleas/{reunion_id}/mesa`, `/asistentes`, `/poderes`, `/morosos`: registro ([`registro`]).
//! - `/asambleas/{reunion_id}/preguntas`: preguntas, votos del operador, resultados, detalle y
//!   elecciones ([`votaciones`]).
//! - `/asambleas/{reunion_id}/acta`, `/reportes`, `/asambleas/actas`, `/asambleas/rotulos`
//!   ([`acta`]).
//! - `/papeleta`: la papeleta virtual del asistente (sin sesión; cabecera `X-Papeleta`)
//!   ([`papeleta`]).
//!
//! Las líneas de WhatsApp y las tarifas van en `/api/v1/plataforma/whatsapp` ([`lineas`]).

mod acta;
mod dto;
pub mod lineas;
mod papeleta;
mod registro;
mod reuniones;
mod votaciones;

pub use dto::TableroRespuesta;
use utoipa_axum::router::OpenApiRouter;

use crate::state::AppState;

/// Rutas de asambleas de una copropiedad.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(reuniones::rutas())
        .merge(registro::rutas())
        .merge(votaciones::rutas())
        .merge(acta::rutas())
        .merge(papeleta::rutas())
}
