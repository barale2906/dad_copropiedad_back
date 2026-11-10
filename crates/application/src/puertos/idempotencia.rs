//! Registro de peticiones idempotentes de la API (plan 10, fase 6, cabecera `Idempotency-Key`).
//!
//! La primera petición con una clave la **reserva**; al terminar se guarda su respuesta (o se
//! libera si falló el servidor). Un reintento con la misma clave y la misma petición recibe la
//! respuesta guardada; con otra petición, [`Reserva::OtraPeticion`]. Vigencia: 24 h.
//!
//! Implementación: `infra_postgres::IdempotenciaPg` (tabla de plataforma
//! `peticiones_idempotentes`).

use std::future::Future;

use chrono::{DateTime, Duration, Utc};
use domain::{ClaveIdempotencia, CuentaId};

use crate::AppResult;

/// Cuánto vale una clave.
pub const VIGENCIA_IDEMPOTENCIA: Duration = Duration::hours(24);

/// Respuesta guardada de la primera petición.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RespuestaGuardada {
    /// Código HTTP (nunca 5xx: esas se liberan para reintentar).
    pub estado: u16,
    pub tipo: Option<String>,
    pub cuerpo: Vec<u8>,
}

/// Resultado de reservar una clave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reserva {
    /// Primera vez (o la anterior venció): la petición se ejecuta.
    Nueva,
    /// La primera petición con esta clave aún no termina.
    EnCurso,
    /// Reintento: se devuelve la respuesta original.
    Repetida(RespuestaGuardada),
    /// La clave ya se usó con otra petición.
    OtraPeticion,
}

pub trait RegistroIdempotencia: Send + Sync {
    /// Reserva `clave` para la petición con `huella` (borra antes una reserva vencida).
    fn reservar(
        &self,
        cuenta: CuentaId,
        clave: &ClaveIdempotencia,
        huella: &str,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Reserva>> + Send;

    /// Guarda la respuesta de la petición reservada.
    fn guardar(
        &self,
        cuenta: CuentaId,
        clave: &ClaveIdempotencia,
        respuesta: &RespuestaGuardada,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Libera la reserva (la petición falló en el servidor y puede reintentarse).
    fn liberar(
        &self,
        cuenta: CuentaId,
        clave: &ClaveIdempotencia,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Borra las claves vencidas. Devuelve cuántas.
    fn purgar(&self, ahora: DateTime<Utc>) -> impl Future<Output = AppResult<u64>> + Send;
}
