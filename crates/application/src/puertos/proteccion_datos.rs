//! Puerto de protección de datos personales (Ley 1581): políticas de tratamiento,
//! autorizaciones por finalidad y solicitudes de los titulares. Lo implementa la transacción
//! del tenant (cada copropiedad es responsable de sus datos).

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, AutorizacionId, FinalidadTratamiento, MedioAutorizacion, PersonaId,
    PoliticaTratamientoId, SolicitudTitularId, TipoSolicitudTitular,
};

use crate::AppResult;

/// Versión publicada de la política de tratamiento. Inmutable: un cambio es una versión nueva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoliticaTratamiento {
    pub id: PoliticaTratamientoId,
    /// 1, 2, 3… por copropiedad.
    pub version: u32,
    /// Texto de la política, o `None` si se publicó como archivo (PDF).
    pub texto: Option<String>,
    pub archivo_id: Option<ArchivoId>,
    pub vigente_desde: DateTime<Utc>,
}

/// Autorización de un titular para una finalidad, bajo una versión de la política.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Autorizacion {
    pub id: AutorizacionId,
    pub persona_id: PersonaId,
    pub politica_id: PoliticaTratamientoId,
    pub finalidad: FinalidadTratamiento,
    pub medio: MedioAutorizacion,
    /// Prueba de la autorización (formato firmado escaneado), si la hay.
    pub evidencia_archivo_id: Option<ArchivoId>,
    pub otorgada_en: DateTime<Utc>,
}

/// Consulta o reclamo radicado por un titular.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolicitudTitular {
    pub id: SolicitudTitularId,
    pub persona_id: PersonaId,
    pub tipo: TipoSolicitudTitular,
    pub radicada_en: DateTime<Utc>,
    /// Último día (hora de Colombia) para responder sin prórroga.
    pub vence_el: NaiveDate,
}

/// Persistencia de la protección de datos dentro de una transacción.
pub trait RepositorioProteccionDatos: Send {
    /// Versión más alta publicada (0 si no hay ninguna).
    fn ultima_version_politica(&mut self) -> impl Future<Output = AppResult<u32>> + Send;

    fn insertar_politica(
        &mut self,
        politica: &PoliticaTratamiento,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// La versión vigente en `ahora`: la de mayor versión con `vigente_desde <= ahora`.
    fn politica_vigente(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<PoliticaTratamiento>>> + Send;

    /// Registra la autorización. Devuelve `false` si ya había una vigente para esa persona y
    /// finalidad (no se duplica).
    fn insertar_autorizacion(
        &mut self,
        autorizacion: &Autorizacion,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Revoca las autorizaciones vigentes de la persona para esas finalidades. Devuelve
    /// cuántas revocó.
    fn revocar_autorizaciones(
        &mut self,
        persona_id: PersonaId,
        finalidades: &[FinalidadTratamiento],
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<u64>> + Send;

    /// Finalidades con autorización vigente de la persona.
    fn finalidades_autorizadas(
        &mut self,
        persona_id: PersonaId,
    ) -> impl Future<Output = AppResult<Vec<FinalidadTratamiento>>> + Send;

    fn insertar_solicitud(
        &mut self,
        solicitud: &SolicitudTitular,
    ) -> impl Future<Output = AppResult<()>> + Send;
}
