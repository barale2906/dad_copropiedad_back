//! Consumo del outbox por el worker de envíos (gateway).
//!
//! El worker reclama un lote de mensajes pendientes de los canales que sabe enviar; las filas
//! quedan bloqueadas para otros workers hasta que el lote se confirma. Por cada mensaje marca
//! el resultado (enviado, o fallido con reintento) y al final confirma el lote.
//!
//! Implementación: `infra_postgres::ColaSalidaPg` (funciones `outbox_reclamar`,
//! `outbox_marcar_enviado`, `outbox_marcar_fallido`).

use std::{future::Future, time::Duration};

use domain::TenantId;

use crate::{
    AppResult,
    outbox::{Canal, MensajeOutbox},
};

/// Mensaje reclamado del outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MensajePendiente {
    /// Id de la fila en `outbox`.
    pub id: i64,
    pub tenant_id: TenantId,
    /// Intentos fallidos previos.
    pub intentos: u32,
    /// Contenido; `Err` si el payload guardado no se puede interpretar (no tiene sentido
    /// reintentarlo: se marca fallido de inmediato).
    pub mensaje: Result<MensajeOutbox, String>,
}

/// Fuente de lotes de mensajes pendientes.
pub trait ColaSalida: Send + Sync {
    type Lote: LoteSalida;

    /// Reclama hasta `limite` mensajes pendientes de `canales`, por prioridad y antigüedad.
    fn reclamar(
        &self,
        canales: &[Canal],
        limite: u32,
    ) -> impl Future<Output = AppResult<Self::Lote>> + Send;
}

/// Lote reclamado. Si se suelta sin confirmar, los mensajes vuelven a quedar pendientes.
pub trait LoteSalida: Send {
    /// Entrega los mensajes del lote (una sola vez).
    fn tomar_mensajes(&mut self) -> Vec<MensajePendiente>;

    /// Marca el mensaje como enviado, con la referencia del proveedor si la hay.
    fn marcar_enviado(
        &mut self,
        id: i64,
        referencia: Option<String>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Registra un intento fallido: se reintenta tras `espera`, salvo que con este intento se
    /// alcance `max_intentos`, en cuyo caso queda `fallido`.
    fn marcar_fallido(
        &mut self,
        id: i64,
        error: String,
        espera: Duration,
        max_intentos: u32,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Confirma las marcas del lote y libera las filas.
    fn confirmar(self) -> impl Future<Output = AppResult<()>> + Send;
}
