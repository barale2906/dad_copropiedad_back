//! Lectura de la auditoría de una copropiedad (la escritura es [`super::RegistroAuditoria`]).
//! Se usa dentro de la transacción del tenant: RLS limita la consulta a esa copropiedad.

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::CuentaId;
use uuid::Uuid;

use crate::AppResult;

/// Evento registrado en la auditoría.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistroAuditado {
    /// Id numérico (orden de registro); el cursor de la paginación.
    pub id: i64,
    pub ocurrido_en: DateTime<Utc>,
    /// `None` = el sistema (workers, tareas internas).
    pub actor_id: Option<CuentaId>,
    pub accion: String,
    pub entidad: String,
    pub entidad_id: Option<Uuid>,
    /// `ok`, `rechazado` o `error`.
    pub resultado: String,
    /// Sin datos personales (lo garantiza `DetalleAuditoria` al registrar).
    pub detalle: serde_json::Value,
    pub request_id: Option<String>,
}

/// Filtros de la consulta.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroAuditoria {
    pub entidad: Option<String>,
    pub entidad_id: Option<Uuid>,
    /// Acción exacta o prefijo terminado en punto (`acceso.` = todo el módulo).
    pub accion: Option<String>,
    pub actor_id: Option<CuentaId>,
    pub desde: Option<DateTime<Utc>>,
    pub hasta: Option<DateTime<Utc>>,
}

pub trait LecturaAuditoria: Send {
    /// Eventos más recientes primero con `id < antes_de`, a lo sumo `limite`.
    fn consultar_auditoria(
        &mut self,
        filtro: &FiltroAuditoria,
        antes_de: Option<i64>,
        limite: i64,
    ) -> impl Future<Output = AppResult<Vec<RegistroAuditado>>> + Send;
}
