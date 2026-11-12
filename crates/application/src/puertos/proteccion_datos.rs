//! Puerto de protección de datos personales (Ley 1581): políticas de tratamiento,
//! autorizaciones por finalidad y solicitudes de los titulares. Lo implementa la transacción
//! del tenant (cada copropiedad es responsable de sus datos).

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, AutorizacionId, CuentaId, EstadoSolicitudTitular, FinalidadTratamiento,
    MedioAutorizacion, PersonaId, PoliticaTratamientoId, SolicitudTitularId, TipoSolicitudTitular,
};

use crate::{AppResult, Pagina, SolicitudPagina};

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
    /// Política de la copropiedad; `None` en la finalidad `publicidad`, que se autoriza frente
    /// a la plataforma con su propia política (`politicas_publicidad`, ADR 0015).
    pub politica_id: Option<PoliticaTratamientoId>,
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
    pub estado: EstadoSolicitudTitular,
    /// Lo que pide el titular (dato personal).
    pub detalle: Option<String>,
    /// Cuenta que la radicó (el titular o la administración por él).
    pub radicada_por: Option<CuentaId>,
    /// Nuevo vencimiento si se prorrogó (una sola vez).
    pub prorrogada_hasta: Option<NaiveDate>,
    pub motivo_prorroga: Option<String>,
    pub respuesta: Option<String>,
    pub respuesta_archivo_id: Option<ArchivoId>,
    pub respondida_en: Option<DateTime<Utc>>,
    pub respondida_por: Option<CuentaId>,
}

impl SolicitudTitular {
    /// Último día para responder (con la prórroga, si la hubo).
    pub fn vence(&self) -> NaiveDate {
        self.prorrogada_hasta.unwrap_or(self.vence_el)
    }
}

/// Autorización con su estado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutorizacionRegistrada {
    pub autorizacion: Autorizacion,
    pub politica_version: u32,
    pub revocada_en: Option<DateTime<Utc>>,
}

/// Filtro de solicitudes del titular.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroSolicitudes {
    pub estado: Option<EstadoSolicitudTitular>,
    pub persona_id: Option<PersonaId>,
    /// Solo las sin responder cuyo vencimiento (con prórroga) es anterior o igual a esta fecha.
    pub vencen_hasta: Option<NaiveDate>,
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

    /// Todas las versiones de la política, la más reciente primero.
    fn historial_politicas(
        &mut self,
    ) -> impl Future<Output = AppResult<Vec<PoliticaTratamiento>>> + Send;

    /// Autorizaciones (vigentes y revocadas) de la persona, más recientes primero.
    fn autorizaciones_de_persona(
        &mut self,
        persona_id: PersonaId,
    ) -> impl Future<Output = AppResult<Vec<AutorizacionRegistrada>>> + Send;

    /// Solicitudes, más recientes primero (cursor por `id` descendente).
    fn solicitudes(
        &mut self,
        filtro: &FiltroSolicitudes,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<SolicitudTitular>>> + Send;

    /// Solicitud bloqueada para actualizar (`FOR UPDATE`).
    fn solicitud_para_actualizar(
        &mut self,
        id: SolicitudTitularId,
    ) -> impl Future<Output = AppResult<Option<SolicitudTitular>>> + Send;

    /// Guarda estado, prórroga y respuesta.
    fn actualizar_solicitud(
        &mut self,
        solicitud: &SolicitudTitular,
    ) -> impl Future<Output = AppResult<()>> + Send;
}
