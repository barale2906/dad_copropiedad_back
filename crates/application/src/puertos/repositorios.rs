//! Puertos de persistencia. Implementación: `crates/infra-postgres`.

use std::future::Future;

use domain::{
    Coeficiente, EstadoReunion, InmuebleId, ParticipacionId, PersonaId, Pregunta, PreguntaId,
    ReunionId, RolAsistente, TenantId, VotoEmitido,
};

use crate::{AppResult, Contexto, outbox::EventoSaliente};

pub trait RepositorioPreguntas: Send + Sync {
    fn obtener(
        &self,
        ctx: &Contexto,
        id: PreguntaId,
    ) -> impl Future<Output = AppResult<Option<Pregunta>>> + Send;
}

/// Inscripción vigente de un inmueble en una reunión, con lo necesario para votar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InscripcionVigente {
    pub inmueble_id: InmuebleId,
    pub persona_id: PersonaId,
    pub rol: RolAsistente,
    /// Coeficiente vigente del inmueble (se copia en la participación).
    pub coeficiente: Coeficiente,
}

pub trait RepositorioInscripciones: Send + Sync {
    /// Devuelve la inscripción si `persona_id` representa a `inmueble_id` en la reunión.
    fn vigente(
        &self,
        ctx: &Contexto,
        reunion_id: ReunionId,
        inmueble_id: InmuebleId,
        persona_id: PersonaId,
    ) -> impl Future<Output = AppResult<Option<InscripcionVigente>>> + Send;
}

/// Resultado de registrar un voto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultadoRegistro {
    /// Voto nuevo grabado.
    Registrado(ParticipacionId),
    /// Reintento (misma `idempotency_key` o mismo `wamid`): se devuelve el original, sin filas
    /// nuevas.
    Reintento(ParticipacionId),
}

pub trait RepositorioVotos: Send + Sync {
    /// En **una sola transacción**: fija el tenant, verifica que la pregunta siga `abierta`
    /// (`FOR SHARE`), inserta participación y voto (`ON CONFLICT DO NOTHING`), encola la
    /// confirmación en `outbox` y audita.
    ///
    /// Errores esperados: [`crate::AppError::InmuebleYaVoto`] si el inmueble ya votó con otra
    /// clave; `Dominio(PreguntaNoAbierta)` si la pregunta se cerró entre la lectura y la
    /// escritura.
    fn registrar(
        &self,
        ctx: &Contexto,
        voto: VotoEmitido,
        confirmacion: Option<EventoSaliente>,
    ) -> impl Future<Output = AppResult<ResultadoRegistro>> + Send;
}

/// Reunión activa en la que está inscrito un teléfono (vista mínima, cruza tenants).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReunionActiva {
    pub tenant_id: TenantId,
    pub reunion_id: ReunionId,
    pub titulo: String,
    pub estado: EstadoReunion,
}

/// Consulta **entre tenants** usada solo por el gateway para enrutar un mensaje entrante antes
/// de saber a qué copropiedad pertenece. En PostgreSQL es una función `SECURITY DEFINER` que
/// devuelve únicamente estos campos.
pub trait DirectorioReuniones: Send + Sync {
    fn activas_por_telefono(
        &self,
        linea: &str,
        telefono: &str,
    ) -> impl Future<Output = AppResult<Vec<ReunionActiva>>> + Send;
}
