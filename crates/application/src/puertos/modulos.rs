//! Puerto de los módulos habilitados por copropiedad (`modulos_habilitados`).

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::{Modulo, TipoEntidad};

use crate::AppResult;

/// Periodo durante el que una copropiedad tiene un módulo contratado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodoModulo {
    pub modulo: Modulo,
    pub desde: DateTime<Utc>,
    /// `None` = sin fecha de fin.
    pub hasta: Option<DateTime<Utc>>,
}

impl PeriodoModulo {
    /// ¿El periodo cubre el instante `ahora`? (`desde` incluido, `hasta` excluido).
    pub fn cubre(&self, ahora: DateTime<Utc>) -> bool {
        self.desde <= ahora && self.hasta.is_none_or(|hasta| ahora < hasta)
    }
}

/// Módulos de la entidad de la transacción.
pub trait RepositorioModulos: Send {
    /// Qué es la entidad de la transacción: propiedad horizontal, empresa o la plataforma.
    fn tipo_entidad(&mut self) -> impl Future<Output = AppResult<TipoEntidad>> + Send;

    /// Zona horaria IANA de la entidad de la transacción (las fechas se muestran en ella).
    fn zona_horaria_entidad(&mut self) -> impl Future<Output = AppResult<String>> + Send;

    /// ¿Algún periodo del módulo cubre `ahora`?
    fn modulo_habilitado(
        &mut self,
        modulo: Modulo,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Módulos con algún periodo que cubre `ahora`, sin repetir y en cualquier orden.
    fn modulos_habilitados(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<Modulo>>> + Send;

    /// Registra un periodo nuevo.
    fn insertar_periodo(
        &mut self,
        periodo: &PeriodoModulo,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Termina en `hasta` los periodos del módulo que sigan abiertos después de esa fecha.
    /// Devuelve cuántos terminó.
    fn terminar_periodos(
        &mut self,
        modulo: Modulo,
        hasta: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<u64>> + Send;
}
