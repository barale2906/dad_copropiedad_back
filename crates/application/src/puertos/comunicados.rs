//! Puerto de los comunicados de la administración (plan 10, fase 4).
//!
//! Implementación: `infra_postgres::comunicados` (tablas `comunicados`, `comunicado_adjuntos`,
//! `comunicado_destinatarios`; la audiencia se resuelve con la función
//! `destinatarios_comunicado`).

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, Audiencia, CanalesComunicado, ComunicadoId, CuentaId, EstadoComunicado,
    FinalidadTratamiento, PersonaId,
};

use crate::{
    AppResult,
    outbox::EventoSaliente,
    paginacion::{Pagina, SolicitudPagina},
};

/// Comunicado guardado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comunicado {
    pub id: ComunicadoId,
    pub titulo: String,
    /// Admite saltos de línea.
    pub cuerpo: String,
    pub finalidad: FinalidadTratamiento,
    pub audiencia: Audiencia,
    /// Sale también por WhatsApp a quien no tiene la app.
    pub importante: bool,
    /// Pide el botón «Enterado».
    pub requiere_confirmacion: bool,
    pub estado: EstadoComunicado,
    pub programado_para: Option<DateTime<Utc>>,
    pub enviado_en: Option<DateTime<Utc>>,
    /// PDF o imágenes, en orden.
    pub adjuntos: Vec<ArchivoId>,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
    pub actualizado_en: DateTime<Utc>,
}

/// Persona de la audiencia con sus medios de contacto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinatarioComunicado {
    pub persona: PersonaId,
    pub cuenta: Option<CuentaId>,
    pub correo: Option<String>,
    pub telefono: Option<String>,
    /// Tokens push activos de su cuenta.
    pub tokens: Vec<String>,
}

/// Por qué canales salió el comunicado a una persona.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntregaComunicado {
    pub persona: PersonaId,
    pub canales: CanalesComunicado,
}

/// Contadores de entrega y lectura de un comunicado (tablero de la administración).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumenLectura {
    pub destinatarios: u32,
    pub por_push: u32,
    pub por_correo: u32,
    pub por_whatsapp: u32,
    /// Personas de la audiencia sin ningún medio de contacto (solo lo verán en la cartelera).
    pub sin_canal: u32,
    pub leidos: u32,
    pub enterados: u32,
}

/// Comunicado en el listado de la administración.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComunicadoConResumen {
    pub comunicado: Comunicado,
    pub resumen: ResumenLectura,
}

/// Comunicado recibido por una persona (su cartelera).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComunicadoRecibido {
    pub comunicado: Comunicado,
    pub leido_en: Option<DateTime<Utc>>,
    pub enterado_en: Option<DateTime<Utc>>,
}

/// Persistencia de los comunicados dentro de la transacción de la copropiedad.
pub trait RepositorioComunicados: Send {
    /// Inserta el comunicado y sus adjuntos.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Validacion`] si un adjunto no es un archivo de la copropiedad.
    fn insertar_comunicado(&mut self, c: &Comunicado)
    -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda contenido, estado y fechas, y reemplaza los adjuntos.
    ///
    /// # Errors
    ///
    /// Como [`RepositorioComunicados::insertar_comunicado`].
    fn actualizar_comunicado(
        &mut self,
        c: &Comunicado,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn comunicado(
        &mut self,
        id: ComunicadoId,
    ) -> impl Future<Output = AppResult<Option<Comunicado>>> + Send;

    /// Como [`RepositorioComunicados::comunicado`] pero bloquea la fila hasta el fin de la
    /// transacción (publicar, anular, enviar).
    fn bloquear_comunicado(
        &mut self,
        id: ComunicadoId,
    ) -> impl Future<Output = AppResult<Option<Comunicado>>> + Send;

    /// Comunicados del más reciente al más antiguo, con sus contadores.
    fn comunicados(
        &mut self,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<ComunicadoConResumen>>> + Send;

    fn resumen_lectura(
        &mut self,
        id: ComunicadoId,
    ) -> impl Future<Output = AppResult<ResumenLectura>> + Send;

    /// Programados cuya hora ya llegó.
    fn programados_vencidos(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<ComunicadoId>>> + Send;

    /// Personas de la audiencia: adultas (o de edad desconocida) y, con finalidad
    /// `comunicaciones`, solo las que la autorizaron.
    fn destinatarios_comunicado(
        &mut self,
        audiencia: &Audiencia,
        finalidad: FinalidadTratamiento,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<DestinatarioComunicado>>> + Send;

    fn insertar_entregas(
        &mut self,
        id: ComunicadoId,
        entregas: &[EntregaComunicado],
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Encola un envío en el outbox con el concepto `comunicados` (costo de WhatsApp por PH).
    fn encolar_comunicado(
        &mut self,
        evento: &EventoSaliente,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Comunicados enviados a la persona, del más reciente al más antiguo.
    fn recibidos(
        &mut self,
        persona: PersonaId,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<ComunicadoRecibido>>> + Send;

    fn recibido(
        &mut self,
        persona: PersonaId,
        id: ComunicadoId,
    ) -> impl Future<Output = AppResult<Option<ComunicadoRecibido>>> + Send;

    /// Fija la primera lectura (no cambia una ya registrada).
    fn marcar_leido(
        &mut self,
        persona: PersonaId,
        id: ComunicadoId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Fija el «Enterado» (y la lectura si faltaba); no cambia uno ya registrado.
    fn marcar_enterado(
        &mut self,
        persona: PersonaId,
        id: ComunicadoId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;
}
