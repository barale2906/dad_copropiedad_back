//! Puertos del control de acceso, visitas y correspondencia (plan 05).
//!
//! - [`RepositorioAcceso`]: lo que vive en la copropiedad (visitas, correspondencia, minuta,
//!   configuración, puntos, puentes, dispositivos, credenciales, autorizaciones, eventos y
//!   consentimientos biométricos). Se usa dentro de la transacción del tenant.
//! - [`DirectorioAcceso`]: lo que cruza copropiedades (worker de escalamiento y autenticación
//!   de los puentes), solo mediante funciones `SECURITY DEFINER` que devuelven ids.
//! - [`RepositorioDispositivosPush`]: los dispositivos de la app móvil (Flutter) de una cuenta,
//!   en el ámbito de plataforma.
//! - [`EnvioPush`]: envío de notificaciones (Firebase Cloud Messaging en `infra-push`).
//! - [`FirmaQr`]: firma Ed25519 de los QR temporales con la clave de cada PH.

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, AutorizacionAccesoId, BienPrivadoId, CanalRespuestaVisita, ConfiguracionAcceso,
    ConsentimientoBiometricoId, CorrespondenciaId, CredencialId, CuentaId, DispositivoAccesoId,
    DispositivoPushId, Documento, EstadoCorrespondencia, EstadoCredencial, EstadoPuente,
    EstadoVisita, EventoAccesoId, Horario, NovedadMinutaId, OrigenAutorizacionAcceso, OrigenVisita,
    PersonaId, Placa, PlataformaPush, PuenteId, PuntoAccesoId, RecursoId, ResultadoAcceso,
    SentidoAcceso, TenantId, TipoCorrespondencia, TipoCredencial, TipoDispositivoAcceso,
    TipoEventoVisita, TipoNovedadMinuta, TipoPuntoAcceso, TipoVisita, VehiculoId, VisitaId,
};
use serde::{Deserialize, Serialize};

use crate::{AppResult, Pagina, SolicitudPagina, outbox::EventoSaliente};

/// Avisos de control de acceso de un periodo, por canal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConteoAvisos {
    pub push_enviados: u64,
    /// Plantillas de WhatsApp enviadas (tienen costo).
    pub whatsapp_enviados: u64,
    pub fallidos: u64,
}

// ---------------------------------------------------------------------------------------------
// Visitas
// ---------------------------------------------------------------------------------------------

/// Visita anunciada por la portería o pre-autorizada por un residente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visita {
    pub id: VisitaId,
    pub bien_privado_id: BienPrivadoId,
    pub origen: OrigenVisita,
    pub tipo: TipoVisita,
    pub estado: EstadoVisita,
    /// Dato personal.
    pub visitante_nombre: String,
    /// Dato personal.
    pub visitante_documento: Option<Documento>,
    pub placa: Option<Placa>,
    pub observacion: Option<String>,
    /// Persona que autorizó o rechazó (residente) o a quien se llamó.
    pub decidida_por: Option<PersonaId>,
    pub canal_decision: Option<CanalRespuestaVisita>,
    /// Ventana de una pre-autorización (y del QR temporal).
    pub ventana_desde: Option<DateTime<Utc>>,
    pub ventana_hasta: Option<DateTime<Utc>>,
    pub usos_permitidos: u16,
    pub usos: u16,
    /// Avisos enviados (niveles de escalamiento).
    pub avisos_enviados: u32,
    /// Cuándo le toca al worker revisar la visita (`None` = nada pendiente).
    pub siguiente_paso_en: Option<DateTime<Utc>>,
    pub registrada_por: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
    pub decidida_en: Option<DateTime<Utc>>,
    pub ingreso_en: Option<DateTime<Utc>>,
    pub salida_en: Option<DateTime<Utc>>,
}

/// Hecho del historial de una visita (append-only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventoVisita {
    pub visita_id: VisitaId,
    pub tipo: TipoEventoVisita,
    pub canal: Option<CanalRespuestaVisita>,
    pub persona_id: Option<PersonaId>,
    pub cuenta_id: Option<CuentaId>,
    /// Nivel del aviso (1 = primer respondiente, 2 = todos) en `aviso_enviado`.
    pub nivel: Option<u32>,
    /// Cuántos destinatarios recibieron el aviso.
    pub destinatarios: Option<u32>,
    pub ocurrido_en: DateTime<Utc>,
}

/// Filtro del listado de visitas.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroVisitas {
    pub estado: Option<EstadoVisita>,
    pub bien_privado_id: Option<BienPrivadoId>,
    /// Solo las de estos bienes (lo que ve un residente).
    pub bienes: Option<Vec<BienPrivadoId>>,
    pub desde: Option<DateTime<Utc>>,
    /// Solo las que están dentro (visitantes activos).
    pub dentro: bool,
}

// ---------------------------------------------------------------------------------------------
// Correspondencia
// ---------------------------------------------------------------------------------------------

/// Paquete, sobre o recibo recibido en la portería.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correspondencia {
    pub id: CorrespondenciaId,
    pub bien_privado_id: BienPrivadoId,
    pub tipo: TipoCorrespondencia,
    pub estado: EstadoCorrespondencia,
    pub remitente: Option<String>,
    pub guia: Option<String>,
    pub ubicacion: Option<String>,
    pub foto_archivo_id: Option<ArchivoId>,
    pub recibida_por: Option<CuentaId>,
    pub recibida_en: DateTime<Utc>,
    pub avisos_enviados: u32,
    pub ultimo_aviso_en: Option<DateTime<Utc>>,
    /// Cuándo sale el siguiente aviso (`None` = no hay más).
    pub siguiente_aviso_en: Option<DateTime<Utc>>,
    /// Alerta a la administración al agotar los días de aviso (una sola vez).
    pub alerta_administracion_en: Option<DateTime<Utc>>,
    pub cerrada_en: Option<DateTime<Utc>>,
    pub motivo_devolucion: Option<String>,
}

/// Entrega de una correspondencia (una por correspondencia, append-only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntregaCorrespondencia {
    pub correspondencia_id: CorrespondenciaId,
    /// Persona registrada del bien que recogió.
    pub persona_id: Option<PersonaId>,
    /// Tercero no registrado (datos personales).
    pub tercero_nombre: Option<String>,
    pub tercero_documento: Option<Documento>,
    pub firma_archivo_id: Option<ArchivoId>,
    pub foto_archivo_id: Option<ArchivoId>,
    pub entregada_por: Option<CuentaId>,
    pub entregada_en: DateTime<Utc>,
    /// Hasta cuándo se guardan la firma y la foto (configuración de la PH).
    pub soportes_hasta: DateTime<Utc>,
}

/// Filtro del listado de correspondencia.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroCorrespondencia {
    pub estado: Option<EstadoCorrespondencia>,
    pub bien_privado_id: Option<BienPrivadoId>,
    pub bienes: Option<Vec<BienPrivadoId>>,
}

/// Soportes de entregas cuyo plazo de retención venció (para borrar los bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoportesVencidos {
    pub correspondencia_id: CorrespondenciaId,
    pub archivos: Vec<ArchivoId>,
}

// ---------------------------------------------------------------------------------------------
// Minuta
// ---------------------------------------------------------------------------------------------

/// Novedad de la minuta digital de portería (append-only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NovedadMinuta {
    pub id: NovedadMinutaId,
    pub tipo: TipoNovedadMinuta,
    pub texto: String,
    pub punto_acceso_id: Option<PuntoAccesoId>,
    pub adjuntos: Vec<ArchivoId>,
    pub registrada_por: Option<CuentaId>,
    pub ocurrida_en: DateTime<Utc>,
}

/// Filtro de la minuta.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroMinuta {
    pub tipo: Option<TipoNovedadMinuta>,
    pub desde: Option<DateTime<Utc>>,
    pub hasta: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------------------------
// Infraestructura de acceso: puntos, puentes, dispositivos
// ---------------------------------------------------------------------------------------------

/// Puerta, portería o talanquera donde se controla el paso.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PuntoAcceso {
    pub id: PuntoAccesoId,
    pub nombre: String,
    pub tipo: TipoPuntoAcceso,
    pub sentido: SentidoAcceso,
    /// Bien común al que da acceso (las reservas con acceso automático lo habilitan).
    pub recurso_id: Option<RecursoId>,
    pub activo: bool,
}

/// Equipo en sitio que habla con los dispositivos y decide sin internet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Puente {
    pub id: PuenteId,
    pub nombre: String,
    pub estado: EstadoPuente,
    /// SHA-256 del token de máquina (el token solo se muestra al registrarlo).
    pub credencial_hash: String,
    pub version: Option<String>,
    pub ultimo_latido_en: Option<DateTime<Utc>>,
    pub creado_en: DateTime<Utc>,
}

/// Lector o actuador (lector de QR, de placas, cerradura) conectado a un puente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispositivoAcceso {
    pub id: DispositivoAccesoId,
    pub puente_id: PuenteId,
    pub punto_acceso_id: PuntoAccesoId,
    pub tipo: TipoDispositivoAcceso,
    pub fabricante: String,
    pub modelo: Option<String>,
    /// Configuración del adaptador (sin secretos).
    pub config: serde_json::Value,
    pub activo: bool,
}

// ---------------------------------------------------------------------------------------------
// Credenciales y autorizaciones
// ---------------------------------------------------------------------------------------------

/// A quién pertenece una credencial o una autorización.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "tipo", content = "id", rename_all = "snake_case")]
pub enum TitularAcceso {
    Persona(PersonaId),
    Vehiculo(VehiculoId),
}

impl TitularAcceso {
    /// Columnas `(persona_id, vehiculo_id)`.
    pub const fn columnas(self) -> (Option<PersonaId>, Option<VehiculoId>) {
        match self {
            Self::Persona(p) => (Some(p), None),
            Self::Vehiculo(v) => (None, Some(v)),
        }
    }

    /// Titular a partir de las columnas `persona_id` / `vehiculo_id` de la BD (la persona primero).
    pub fn desde_columnas(p: Option<PersonaId>, v: Option<VehiculoId>) -> Option<Self> {
        p.map(Self::Persona).or_else(|| v.map(Self::Vehiculo))
    }
}

/// Credencial de acceso. El identificador **nunca** se guarda en claro: solo su huella.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credencial {
    pub id: CredencialId,
    pub titular: TitularAcceso,
    pub tipo: TipoCredencial,
    /// SHA-256 del identificador normalizado con el tenant ([`domain::acceso`]).
    pub huella: String,
    pub estado: EstadoCredencial,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    /// Biometría: el puente confirmó que borró la plantilla del equipo.
    pub borrado_confirmado_en: Option<DateTime<Utc>>,
    pub creada_en: DateTime<Utc>,
}

/// Autorización de acceso (quién, dónde, cuándo). Las derivadas del censo y la titularidad no
/// se guardan: se calculan al sincronizar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutorizacionAcceso {
    pub id: AutorizacionAccesoId,
    pub titular: TitularAcceso,
    pub origen: OrigenAutorizacionAcceso,
    /// Puntos permitidos; vacío = todos.
    pub puntos: Vec<PuntoAccesoId>,
    pub horario: Option<Horario>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub motivo: Option<String>,
    pub revocada_en: Option<DateTime<Utc>>,
}

/// Credencial tal como la recibe el puente en la sincronización: huella, estado y ventanas en
/// las que vale (autorizaciones manuales + derivadas).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredencialSincronizada {
    pub id: CredencialId,
    pub tipo: TipoCredencial,
    pub huella: String,
    /// `false` si está suspendida, revocada, perdida o vencida: el puente la borra de su cache.
    pub activa: bool,
    pub vigente_hasta: Option<DateTime<Utc>>,
    /// Ventanas que la habilitan; vacía = no pasa por ningún punto.
    pub ventanas: Vec<VentanaSincronizada>,
    /// Biometría revocada: el puente debe borrar la plantilla y confirmarlo.
    pub borrar_plantilla: bool,
    pub version: i64,
}

/// Ventana serializable para el puente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VentanaSincronizada {
    pub puntos: Vec<PuntoAccesoId>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub horario: Option<Horario>,
}

/// Visita con QR temporal vigente (para que el puente sepa si se canceló y cuántos usos lleva).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisitaSincronizada {
    pub id: VisitaId,
    pub activa: bool,
    pub usos: u16,
    pub version: i64,
}

/// Lote de cambios para un puente desde su cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CambiosAcceso {
    pub credenciales: Vec<CredencialSincronizada>,
    pub visitas: Vec<VisitaSincronizada>,
    /// Cursor que el puente debe enviar la próxima vez.
    pub cursor: i64,
}

/// Evento de acceso (append-only). Lo registra la portería o lo reporta un puente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventoAcceso {
    pub id: EventoAccesoId,
    pub punto_acceso_id: Option<PuntoAccesoId>,
    pub dispositivo_id: Option<DispositivoAccesoId>,
    pub puente_id: Option<PuenteId>,
    /// Id del evento en el puente (idempotencia de los reportes).
    pub id_local: Option<String>,
    pub credencial_id: Option<CredencialId>,
    pub visita_id: Option<VisitaId>,
    pub sentido: SentidoAcceso,
    pub resultado: ResultadoAcceso,
    pub motivo: Option<String>,
    /// Placa leída por la cámara LPR.
    pub placa: Option<Placa>,
    pub registrado_por: Option<CuentaId>,
    pub ocurrido_en: DateTime<Utc>,
}

/// Filtro de eventos de acceso.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroEventosAcceso {
    pub desde: Option<DateTime<Utc>>,
    pub hasta: Option<DateTime<Utc>>,
    pub punto_acceso_id: Option<PuntoAccesoId>,
    pub resultado: Option<ResultadoAcceso>,
}

/// Consentimiento biométrico (Ley 1581 de 2012: explícito y facultativo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentimientoBiometrico {
    pub id: ConsentimientoBiometricoId,
    pub persona_id: PersonaId,
    pub version_texto: String,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub otorgado_en: DateTime<Utc>,
    pub revocado_en: Option<DateTime<Utc>>,
    pub registrado_por: Option<CuentaId>,
}

/// Alerta operativa de la portería.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertaAcceso {
    /// El puente no reporta latido hace más de lo tolerado.
    PuenteSinLatido {
        puente: PuenteId,
        nombre: String,
        ultimo: Option<DateTime<Utc>>,
    },
    /// Apertura forzada en un punto.
    PuertaForzada {
        punto: Option<PuntoAccesoId>,
        ocurrido_en: DateTime<Utc>,
    },
    /// Varios intentos denegados seguidos en un punto.
    DenegadosRepetidos {
        punto: Option<PuntoAccesoId>,
        intentos: u32,
    },
}

// ---------------------------------------------------------------------------------------------
// Repositorio de la copropiedad
// ---------------------------------------------------------------------------------------------

/// Persistencia del control de acceso de la copropiedad.
pub trait RepositorioAcceso: Send {
    /// Configuración de la PH o la predeterminada.
    fn configuracion_acceso(
        &mut self,
    ) -> impl Future<Output = AppResult<ConfiguracionAcceso>> + Send;

    /// Crea o reemplaza la configuración de acceso de la copropiedad.
    fn guardar_configuracion_acceso(
        &mut self,
        c: &ConfiguracionAcceso,
    ) -> impl Future<Output = AppResult<()>> + Send;

    // --- Visitas ---

    /// Guarda una visita nueva.
    fn insertar_visita(&mut self, v: &Visita) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda estado, decisión, avisos, siguiente paso, usos e ingreso/salida.
    fn actualizar_visita(&mut self, v: &Visita) -> impl Future<Output = AppResult<()>> + Send;

    /// Visita bloqueada para actualizar (`FOR UPDATE`): dos respuestas simultáneas no deciden
    /// dos veces.
    fn visita_para_actualizar(
        &mut self,
        id: VisitaId,
    ) -> impl Future<Output = AppResult<Option<Visita>>> + Send;

    /// Visita por su id.
    fn visita(&mut self, id: VisitaId) -> impl Future<Output = AppResult<Option<Visita>>> + Send;

    /// Más recientes primero (cursor por `id` descendente).
    fn visitas(
        &mut self,
        filtro: &FiltroVisitas,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Visita>>> + Send;

    /// Visitas cuyo `siguiente_paso_en` ya llegó.
    fn visitas_con_paso_pendiente(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<Visita>>> + Send;

    /// Agrega un evento al historial de la visita.
    fn insertar_evento_visita(
        &mut self,
        e: &EventoVisita,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Historial de la visita, en orden.
    fn eventos_visita(
        &mut self,
        id: VisitaId,
    ) -> impl Future<Output = AppResult<Vec<EventoVisita>>> + Send;

    // --- Correspondencia ---

    /// Guarda una correspondencia recibida en portería.
    fn insertar_correspondencia(
        &mut self,
        c: &Correspondencia,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda los cambios de una correspondencia (estado, avisos, entrega o devolución).
    fn actualizar_correspondencia(
        &mut self,
        c: &Correspondencia,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Bloqueada para actualizar (`FOR UPDATE`).
    fn correspondencia_para_actualizar(
        &mut self,
        id: CorrespondenciaId,
    ) -> impl Future<Output = AppResult<Option<Correspondencia>>> + Send;

    /// Correspondencia por su id.
    fn correspondencia(
        &mut self,
        id: CorrespondenciaId,
    ) -> impl Future<Output = AppResult<Option<Correspondencia>>> + Send;

    /// Correspondencias que cumplen el filtro, paginadas por cursor.
    fn correspondencias(
        &mut self,
        filtro: &FiltroCorrespondencia,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Correspondencia>>> + Send;

    /// Recibidas cuyo `siguiente_aviso_en` ya llegó, o que agotaron los días de aviso sin
    /// alerta a la administración.
    fn correspondencia_con_aviso_pendiente(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<Correspondencia>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si la correspondencia ya tiene entrega.
    fn insertar_entrega(
        &mut self,
        e: &EntregaCorrespondencia,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Entrega registrada de una correspondencia, si ya se entregó.
    fn entrega(
        &mut self,
        id: CorrespondenciaId,
    ) -> impl Future<Output = AppResult<Option<EntregaCorrespondencia>>> + Send;

    /// Entregas cuyo plazo de retención de soportes venció y aún no se purgaron.
    fn soportes_vencidos(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<SoportesVencidos>>> + Send;

    /// Marca los soportes de la entrega como purgados (los bytes ya se borraron).
    fn marcar_soportes_purgados(
        &mut self,
        id: CorrespondenciaId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    // --- Minuta ---

    /// Agrega una novedad a la minuta de portería.
    fn insertar_novedad(&mut self, n: &NovedadMinuta)
    -> impl Future<Output = AppResult<()>> + Send;

    /// Novedades de la minuta que cumplen el filtro, paginadas por cursor.
    fn novedades(
        &mut self,
        filtro: &FiltroMinuta,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<NovedadMinuta>>> + Send;

    // --- Contactos para avisos ---

    /// Tokens push activos de las cuentas (tabla de plataforma, función `SECURITY DEFINER`).
    fn tokens_push(
        &mut self,
        cuentas: &[CuentaId],
    ) -> impl Future<Output = AppResult<Vec<(CuentaId, String)>>> + Send;

    /// Encola un aviso de control de acceso en el outbox marcado con el concepto
    /// `control_acceso` (para llevar el costo de WhatsApp por PH).
    fn encolar_aviso_acceso(
        &mut self,
        evento: &EventoSaliente,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Avisos de control de acceso enviados en `[desde, hasta)`.
    fn mensajes_avisos_acceso(
        &mut self,
        desde: DateTime<Utc>,
        hasta: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<ConteoAvisos>> + Send;

    // --- Puntos, puentes y dispositivos ---

    /// Crea (`nuevo`) o actualiza un punto de acceso.
    fn guardar_punto_acceso(
        &mut self,
        p: &PuntoAcceso,
        nuevo: bool,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Puntos de acceso de la copropiedad.
    fn puntos_acceso(&mut self) -> impl Future<Output = AppResult<Vec<PuntoAcceso>>> + Send;

    /// Registra un puente de acceso.
    fn insertar_puente(&mut self, p: &Puente) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda estado, versión, latido y huella de la credencial de máquina.
    fn actualizar_puente(&mut self, p: &Puente) -> impl Future<Output = AppResult<()>> + Send;

    /// Puente por su id.
    fn puente(&mut self, id: PuenteId) -> impl Future<Output = AppResult<Option<Puente>>> + Send;

    /// Puentes de la copropiedad.
    fn puentes(&mut self) -> impl Future<Output = AppResult<Vec<Puente>>> + Send;

    /// Crea (`nuevo`) o actualiza un dispositivo.
    fn guardar_dispositivo(
        &mut self,
        d: &DispositivoAcceso,
        nuevo: bool,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Dispositivos de la copropiedad, o solo los de un puente.
    fn dispositivos(
        &mut self,
        puente: Option<PuenteId>,
    ) -> impl Future<Output = AppResult<Vec<DispositivoAcceso>>> + Send;

    // --- Credenciales y autorizaciones ---

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay una credencial vigente con la misma huella.
    fn insertar_credencial(&mut self, c: &Credencial)
    -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda estado, vigencia y confirmación de borrado (sube la versión de sincronización).
    fn actualizar_credencial(
        &mut self,
        c: &Credencial,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Credencial por su id.
    fn credencial(
        &mut self,
        id: CredencialId,
    ) -> impl Future<Output = AppResult<Option<Credencial>>> + Send;

    /// Credencial activa con esa huella (verificación en la portería digital).
    fn credencial_por_huella(
        &mut self,
        huella: &str,
    ) -> impl Future<Output = AppResult<Option<Credencial>>> + Send;

    /// Credenciales del titular.
    fn credenciales_de(
        &mut self,
        titular: TitularAcceso,
    ) -> impl Future<Output = AppResult<Vec<Credencial>>> + Send;

    /// Guarda una autorización de acceso.
    fn insertar_autorizacion_acceso(
        &mut self,
        a: &AutorizacionAcceso,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Revoca la autorización; `false` si no existía o ya estaba revocada.
    fn revocar_autorizacion_acceso(
        &mut self,
        id: AutorizacionAccesoId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Autorizaciones del titular (las revocadas, solo si se piden).
    fn autorizaciones_de(
        &mut self,
        titular: TitularAcceso,
        incluir_revocadas: bool,
    ) -> impl Future<Output = AppResult<Vec<AutorizacionAcceso>>> + Send;

    /// Ventanas vigentes del titular: autorizaciones manuales y derivadas (residencia o
    /// titularidad vigente en cualquier bien de la copropiedad; vehículo de un bien con
    /// residentes o titulares).
    fn ventanas_de(
        &mut self,
        titular: TitularAcceso,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<VentanaSincronizada>>> + Send;

    /// Cambios para un puente desde `cursor` (credenciales y visitas con QR), a lo sumo
    /// `limite` de cada clase.
    fn cambios_desde(
        &mut self,
        cursor: i64,
        limite: u32,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<CambiosAcceso>> + Send;

    // --- Eventos ---

    /// Inserta el evento; `false` si ya existía (mismo puente e `id_local`): reportar es
    /// idempotente.
    fn insertar_evento_acceso(
        &mut self,
        e: &EventoAcceso,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Asocia la foto de un evento (placa o rostro) con la fecha en que se purga.
    fn adjuntar_foto_evento(
        &mut self,
        evento: EventoAccesoId,
        archivo: ArchivoId,
        purgar_en: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Eventos de acceso que cumplen el filtro, paginados por cursor.
    fn eventos_acceso(
        &mut self,
        filtro: &FiltroEventosAcceso,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<EventoAcceso>>> + Send;

    /// Fotos de placas vencidas sin purgar.
    fn fotos_eventos_vencidas(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<(EventoAccesoId, ArchivoId)>>> + Send;

    /// Marca como purgada la foto del evento (el archivo ya se borró).
    fn marcar_foto_evento_purgada(
        &mut self,
        evento: EventoAccesoId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Borra los eventos más viejos que `dias` (≥ 365, función `SECURITY DEFINER`). Devuelve
    /// cuántos borró.
    fn purgar_eventos_acceso(&mut self, dias: u32) -> impl Future<Output = AppResult<u64>> + Send;

    /// Alertas operativas a la hora `ahora`.
    fn alertas_acceso(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<AlertaAcceso>>> + Send;

    // --- Biometría ---

    /// Guarda un consentimiento biométrico.
    fn insertar_consentimiento(
        &mut self,
        c: &ConsentimientoBiometrico,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Consentimiento biométrico vigente de la persona.
    fn consentimiento_vigente(
        &mut self,
        persona: PersonaId,
    ) -> impl Future<Output = AppResult<Option<ConsentimientoBiometrico>>> + Send;

    /// Revoca el consentimiento biométrico.
    fn revocar_consentimiento(
        &mut self,
        id: ConsentimientoBiometricoId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Cruce de copropiedades
// ---------------------------------------------------------------------------------------------

/// Lo que el worker y la API de puentes necesitan fuera de una copropiedad.
pub trait DirectorioAcceso: Send + Sync {
    /// Copropiedades con visitas o correspondencia por escalar, o soportes por purgar.
    fn copropiedades_con_acceso_pendiente(
        &self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<TenantId>>> + Send;

    /// Copropiedades con el módulo de control de acceso (tareas diarias de retención).
    fn copropiedades_con_control_acceso(
        &self,
    ) -> impl Future<Output = AppResult<Vec<TenantId>>> + Send;

    /// Puente activo con esa huella de credencial de máquina.
    fn puente_por_credencial(
        &self,
        huella: &str,
    ) -> impl Future<Output = AppResult<Option<(TenantId, PuenteId)>>> + Send;

    /// Crea las particiones mensuales de `eventos_acceso` de los próximos meses.
    fn preparar_particiones(&self, meses: u32) -> impl Future<Output = AppResult<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Dispositivos push (plataforma)
// ---------------------------------------------------------------------------------------------

/// Dispositivo de la app móvil registrado para notificaciones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispositivoPush {
    pub id: DispositivoPushId,
    pub cuenta_id: CuentaId,
    pub plataforma: PlataformaPush,
    /// Token de FCM (dato personal: identifica el teléfono).
    pub token: String,
    pub nombre: Option<String>,
    pub activo: bool,
    pub registrado_en: DateTime<Utc>,
}

/// Dispositivos push de las cuentas (transacción de plataforma).
pub trait RepositorioDispositivosPush: Send {
    /// Registra o reactiva el token (si ya existía, pasa a esta cuenta).
    fn registrar_dispositivo_push(
        &mut self,
        d: &DispositivoPush,
    ) -> impl Future<Output = AppResult<DispositivoPush>> + Send;

    /// Dispositivos push activos de la cuenta.
    fn dispositivos_push(
        &mut self,
        cuenta: CuentaId,
    ) -> impl Future<Output = AppResult<Vec<DispositivoPush>>> + Send;

    /// Desactiva el dispositivo de la cuenta; `false` si no existe.
    fn retirar_dispositivo_push(
        &mut self,
        cuenta: CuentaId,
        id: DispositivoPushId,
    ) -> impl Future<Output = AppResult<bool>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Envío push y firma de QR
// ---------------------------------------------------------------------------------------------

/// Notificación push encolada en el outbox. Si el token resulta inválido, el worker desvía el
/// `respaldo` (WhatsApp) en lugar de reintentar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificacionPush {
    pub token: String,
    pub titulo: String,
    pub cuerpo: String,
    /// Datos para la app (tipo de aviso, ids, acciones). Sin datos personales.
    #[serde(default)]
    pub datos: std::collections::BTreeMap<String, String>,
    /// Mensaje de WhatsApp a enviar si el push no se puede entregar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respaldo: Option<crate::puertos::mensajeria::MensajeSaliente>,
}

/// Error de un envío push.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorPush {
    /// El token ya no existe (app desinstalada): se desactiva y se usa el respaldo.
    #[error("token de notificaciones inválido")]
    TokenInvalido,
    /// Falla transitoria: se reintenta.
    #[error("envío push fallido: {0}")]
    Temporal(String),
}

/// Envío de notificaciones a las apps (FCM sirve para Android e iOS).
pub trait EnvioPush: Send + Sync {
    /// Devuelve el id del mensaje asignado por el proveedor.
    fn enviar(
        &self,
        n: &NotificacionPush,
    ) -> impl Future<Output = Result<String, ErrorPush>> + Send;
}

/// Firma y verificación de los QR temporales con la clave Ed25519 de cada PH.
pub trait FirmaQr: Send + Sync {
    /// Firma la carga del QR con la clave de la copropiedad.
    fn firmar(&self, tenant: TenantId, carga: &[u8]) -> Vec<u8>;

    /// Si la firma corresponde a la carga con la clave de la copropiedad.
    fn verificar(&self, tenant: TenantId, carga: &[u8], firma: &[u8]) -> bool;

    /// Clave pública de la PH (la recibe el puente para validar sin internet).
    fn clave_publica(&self, tenant: TenantId) -> [u8; 32];
}

/// Imagen PNG de un código QR (el de una visita, para enviarlo por WhatsApp).
pub trait GeneradorImagenQr: Send + Sync {
    /// # Errors
    ///
    /// [`crate::AppError::Validacion`] si el texto no cabe en un QR.
    fn png(&self, texto: &str) -> AppResult<Vec<u8>>;
}
