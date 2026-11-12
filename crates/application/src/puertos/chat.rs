//! Puertos del chat de WhatsApp (plan 07b):
//!
//! - [`DirectorioTelefonos`] (plataforma): a qué copropiedades y bienes privados está vinculado
//!   un teléfono. Cualquier línea atiende cualquier PH: la PH se resuelve por el teléfono.
//! - [`RepositorioChat`] (transacción de una copropiedad): preguntas frecuentes y la bandeja
//!   de conversaciones escaladas a la administración.

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, BienPrivadoId, ConversacionId, CuentaId, PersonaId, PreguntaFrecuenteId, TenantId,
};
use serde::{Deserialize, Serialize};

use crate::{AppResult, Pagina, SolicitudPagina};

/// Vínculo de un teléfono con un bien privado de una copropiedad.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VinculoTelefono {
    pub tenant_id: TenantId,
    pub copropiedad: String,
    pub persona_id: PersonaId,
    /// Cuenta de la plataforma de la persona (necesaria para reservar desde el chat).
    pub cuenta_id: Option<CuentaId>,
    pub bien_privado_id: BienPrivadoId,
    /// Código visible del bien privado (p. ej. `T1-506`).
    pub codigo: String,
    /// `true` = propietario; `false` = residente.
    pub es_propietario: bool,
}

/// Resuelve a qué copropiedades y bienes pertenece un teléfono (cruza tenants: SECURITY DEFINER).
pub trait DirectorioTelefonos: Send + Sync {
    /// Vínculos vigentes del teléfono en copropiedades activas con la gestión administrativa
    /// contratada (función `vinculos_por_telefono`).
    fn vinculos_por_telefono(
        &self,
        telefono: &str,
    ) -> impl Future<Output = AppResult<Vec<VinculoTelefono>>> + Send;
}

/// Pregunta frecuente que responde el chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreguntaFrecuente {
    pub id: PreguntaFrecuenteId,
    /// Título corto (se muestra en la lista de WhatsApp).
    pub pregunta: String,
    pub respuesta: String,
    pub orden: i16,
    pub activa: bool,
    /// Documento que se puede enlazar (reglamento, manual de convivencia).
    pub archivo_id: Option<ArchivoId>,
}

/// Estado de una conversación de la bandeja.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoConversacion {
    Abierta,
    Cerrada,
}

domain::enum_texto!(EstadoConversacion {
    Abierta => "abierta",
    Cerrada => "cerrada",
});

/// Conversación escalada a la administración.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversacion {
    pub id: ConversacionId,
    pub linea: String,
    /// Teléfono del residente (dato personal).
    pub telefono: String,
    pub persona_id: Option<PersonaId>,
    pub estado: EstadoConversacion,
    pub atendida_por: Option<CuentaId>,
    pub abierta_en: DateTime<Utc>,
    pub ultimo_entrante_en: DateTime<Utc>,
    pub cerrada_en: Option<DateTime<Utc>>,
}

impl Conversacion {
    /// Ventana de Meta para responder con texto libre.
    pub const VENTANA_HORAS: i64 = 24;

    /// ¿Se puede responder con texto libre?
    pub fn ventana_abierta(&self, ahora: DateTime<Utc>) -> bool {
        ahora < self.ultimo_entrante_en + chrono::Duration::hours(Self::VENTANA_HORAS)
    }
}

/// Mensaje de una conversación (append-only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MensajeConversacion {
    pub conversacion_id: ConversacionId,
    /// `true` = lo escribió el residente.
    pub entrante: bool,
    pub texto: String,
    pub autor_cuenta_id: Option<CuentaId>,
    /// `wamid` del mensaje entrante (deduplica reintentos de Meta).
    pub wamid: Option<String>,
    pub creado_en: DateTime<Utc>,
}

/// Persistencia del chat: preguntas frecuentes y bandeja de conversaciones escaladas.
pub trait RepositorioChat: Send {
    /// Preguntas frecuentes (solo las activas si se pide).
    fn preguntas_frecuentes(
        &mut self,
        solo_activas: bool,
    ) -> impl Future<Output = AppResult<Vec<PreguntaFrecuente>>> + Send;

    /// Pregunta frecuente por su id.
    fn pregunta_frecuente(
        &mut self,
        id: PreguntaFrecuenteId,
    ) -> impl Future<Output = AppResult<Option<PreguntaFrecuente>>> + Send;

    /// Inserta (`nueva`) o reemplaza la pregunta.
    fn guardar_pregunta_frecuente(
        &mut self,
        p: &PreguntaFrecuente,
        nueva: bool,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Conversación abierta de ese teléfono en esa línea, si la hay.
    fn conversacion_abierta(
        &mut self,
        linea: &str,
        telefono: &str,
    ) -> impl Future<Output = AppResult<Option<Conversacion>>> + Send;

    /// Conversación por su id.
    fn conversacion(
        &mut self,
        id: ConversacionId,
    ) -> impl Future<Output = AppResult<Option<Conversacion>>> + Send;

    /// Guarda una conversación nueva.
    fn insertar_conversacion(
        &mut self,
        c: &Conversacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda los cambios de una conversación (atención, último mensaje, cierre).
    fn actualizar_conversacion(
        &mut self,
        c: &Conversacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Conversaciones más recientes primero (cursor por `id` descendente).
    fn conversaciones(
        &mut self,
        estado: Option<EstadoConversacion>,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Conversacion>>> + Send;

    /// Agrega el mensaje; devuelve `false` si el `wamid` ya estaba (reintento).
    fn insertar_mensaje_conversacion(
        &mut self,
        m: &MensajeConversacion,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Mensajes de la conversación, en orden.
    fn mensajes_conversacion(
        &mut self,
        id: ConversacionId,
    ) -> impl Future<Output = AppResult<Vec<MensajeConversacion>>> + Send;
}
