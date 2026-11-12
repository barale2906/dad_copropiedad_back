//! Puertos de asambleas y votaciones (plan 06, ADR 0010).
//!
//! - [`RepositorioAsambleas`] (transacción de una copropiedad): reuniones, orden del día,
//!   padrón, morosos, asistentes con sus bienes, poderes, presencias, rótulos, preguntas,
//!   votos, fotos del quórum, constancias de la convocatoria y mensajes de la asamblea.
//! - [`DirectorioAsambleas`] (cruza copropiedades, solo el gateway): asistencias de un teléfono,
//!   copropiedades con preguntas por cerrar y acuses de entrega de WhatsApp.
//! - [`RepositorioLineasWhatsapp`] (transacción de plataforma): líneas y tarifas de Meta.

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, AsistenteId, BaseComputo, BienPrivadoId, CalidadRepresentacion, Canal, CodigoRotulo,
    Coeficiente, Conteo, Convocatoria, CuentaId, EfectoMora, Escrutinio, EstadoActa, EstadoPoder,
    EstadoPunto, EstadoReunion, MetodoRegistro, Modalidad, MomentoQuorum, MotivoSalida, OpcionId,
    PersonaId, PoderId, PreguntaId, PresenciaId, PuntoOrdenId, QuorumReunion, ReunionId, TenantId,
    TipoDocumentoReunion, TipoReunion, VotoEmitido, VotosOpcion,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{AppResult, Pagina, SolicitudPagina, outbox::EventoSaliente};

// ---------------------------------------------------------------------------------------------
// Reuniones
// ---------------------------------------------------------------------------------------------

/// Configuración que cada copropiedad fija al crear la asamblea.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguracionReunion {
    /// Base por defecto de las preguntas y del quórum.
    pub base_computo: BaseComputo,
    /// Máximo de bienes que puede representar un apoderado (`None` = sin límite).
    pub max_bienes_apoderado: Option<u16>,
    /// Cargos (códigos de rol plantilla) que no pueden ser apoderados.
    pub apoderados_prohibidos: Vec<String>,
    /// Si es `true`, un poder sin soporte no se acepta.
    pub exigir_soporte_poder: bool,
    pub efecto_mora: EfectoMora,
    /// Temporizador por defecto de las preguntas.
    pub temporizador_segundos: Option<u32>,
}

impl Default for ConfiguracionReunion {
    fn default() -> Self {
        Self {
            base_computo: BaseComputo::Coeficiente,
            max_bienes_apoderado: None,
            apoderados_prohibidos: vec!["administrador".into(), "revisor_fiscal".into()],
            exigir_soporte_poder: false,
            efecto_mora: EfectoMora::Informativo,
            temporizador_segundos: None,
        }
    }
}

/// Reunión de la asamblea (o consulta por escrito).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reunion {
    pub id: ReunionId,
    pub titulo: String,
    pub tipo: TipoReunion,
    pub convocatoria: Convocatoria,
    pub modalidad: Modalidad,
    pub estado: EstadoReunion,
    pub fecha_programada: DateTime<Utc>,
    pub lugar: Option<String>,
    pub enlace_video: Option<String>,
    /// Solo en consultas por escrito.
    pub plazo_respuesta: Option<DateTime<Utc>>,
    /// Reunión de primera convocatoria que originó esta (segunda convocatoria).
    pub reunion_origen_id: Option<ReunionId>,
    pub configuracion: ConfiguracionReunion,
    pub convocada_en: Option<DateTime<Utc>>,
    pub registro_abierto_en: Option<DateTime<Utc>>,
    pub instalada_en: Option<DateTime<Utc>>,
    pub cerrada_en: Option<DateTime<Utc>>,
    pub cancelada_en: Option<DateTime<Utc>>,
    pub motivo_cancelacion: Option<String>,
    pub acta_estado: EstadoActa,
    pub acta_archivo_id: Option<ArchivoId>,
    pub acta_firmada_archivo_id: Option<ArchivoId>,
    pub acta_generada_en: Option<DateTime<Utc>>,
    pub acta_aprobada_en: Option<DateTime<Utc>>,
    pub acta_publicada_en: Option<DateTime<Utc>>,
    pub creada_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
}

/// Punto del orden del día.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PuntoOrden {
    pub id: PuntoOrdenId,
    pub orden: u16,
    pub titulo: String,
    pub descripcion: Option<String>,
    pub estado: EstadoPunto,
    pub iniciado_en: Option<DateTime<Utc>>,
    pub terminado_en: Option<DateTime<Utc>>,
}

/// Documento anexo a la convocatoria.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentoReunion {
    pub archivo_id: ArchivoId,
    pub tipo: TipoDocumentoReunion,
    pub titulo: String,
}

// ---------------------------------------------------------------------------------------------
// Padrón, asistentes, poderes, presencias y rótulos
// ---------------------------------------------------------------------------------------------

/// Bien privado del padrón de la reunión.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BienPadron {
    pub bien_privado_id: BienPrivadoId,
    pub codigo: String,
    pub coeficiente: Coeficiente,
    /// Representante ante la copropiedad al convocar.
    pub representante: Option<PersonaId>,
    pub moroso: bool,
}

/// Bien que se incluye al tomar el padrón.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BienParaPadron {
    pub bien_privado_id: BienPrivadoId,
    pub coeficiente: Coeficiente,
    pub representante: Option<PersonaId>,
}

/// Coincidencia de la búsqueda de la mesa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoincidenciaMesa {
    pub bien: BienPadron,
    pub representante_nombre: Option<String>,
    /// Teléfono del censo del representante (dato personal: solo para la mesa).
    pub representante_telefono: Option<String>,
    /// Otros bienes del padrón con el mismo representante (sugeridos con un toque).
    pub otros_bienes: Vec<BienPrivadoId>,
    /// Asistente que ya representa el bien, si lo hay.
    pub asistente_actual: Option<AsistenteId>,
}

/// Bien que representa un asistente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BienDeAsistente {
    pub bien_privado_id: BienPrivadoId,
    pub codigo: String,
    pub coeficiente: Coeficiente,
    pub calidad: CalidadRepresentacion,
    pub poder_id: Option<PoderId>,
    pub moroso: bool,
}

/// Asistente de la reunión con sus medios y sus bienes vigentes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asistente {
    pub id: AsistenteId,
    pub reunion_id: ReunionId,
    pub persona_id: PersonaId,
    pub nombre: String,
    pub metodo: MetodoRegistro,
    /// Teléfono de WhatsApp (dato personal).
    pub telefono: Option<String>,
    /// Línea (`phone_number_id`) por la que se le escribe en esta asamblea.
    pub linea_whatsapp: Option<String>,
    pub whatsapp_confirmado_en: Option<DateTime<Utc>>,
    pub confirmacion_outbox_id: Option<i64>,
    pub registrado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
    /// Tiene una presencia abierta.
    pub presente: bool,
    pub rotulo: Option<CodigoRotulo>,
    pub bienes: Vec<BienDeAsistente>,
}

/// Asistente nuevo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsistenteNuevo {
    pub id: AsistenteId,
    pub reunion_id: ReunionId,
    pub persona_id: PersonaId,
    pub metodo: MetodoRegistro,
    pub telefono: Option<String>,
    pub registrado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
}

/// Poder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Poder {
    pub id: PoderId,
    pub reunion_id: ReunionId,
    pub bien_privado_id: BienPrivadoId,
    pub poderdante: Option<PersonaId>,
    pub apoderado: PersonaId,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub estado: EstadoPoder,
    pub motivo: Option<String>,
    pub registrado_por: Option<CuentaId>,
    pub registrado_en: DateTime<Utc>,
    pub revisado_por: Option<CuentaId>,
    pub revisado_en: Option<DateTime<Utc>>,
}

/// Periodo de presencia de un asistente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presencia {
    pub id: PresenciaId,
    pub asistente_id: AsistenteId,
    pub entrada_en: DateTime<Utc>,
    pub salida_en: Option<DateTime<Utc>>,
    pub motivo_salida: Option<MotivoSalida>,
    pub detalle_salida: Option<String>,
    pub registrada_por: Option<CuentaId>,
}

/// Bien privado de un asistente presente (base del quórum y de las preguntas que se abren).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BienPresente {
    pub bien_privado_id: BienPrivadoId,
    pub coeficiente: Coeficiente,
    pub asistente_id: AsistenteId,
    pub moroso: bool,
}

/// Foto del quórum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FotoQuorum {
    pub pregunta_id: Option<PreguntaId>,
    pub momento: MomentoQuorum,
    pub quorum: QuorumReunion,
    pub tomada_en: DateTime<Utc>,
}

// ---------------------------------------------------------------------------------------------
// Preguntas y votos
// ---------------------------------------------------------------------------------------------

/// Pregunta con lo que se guarda además de su definición de dominio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreguntaGuardada {
    pub pregunta: domain::Pregunta,
    pub orden: u16,
    pub punto_id: Option<PuntoOrdenId>,
    pub temporizador_segundos: Option<u32>,
    pub cierra_en: Option<DateTime<Utc>>,
    pub abierta_en: Option<DateTime<Utc>>,
    pub cerrada_en: Option<DateTime<Utc>>,
    pub resultado: Option<Escrutinio>,
    /// Integrantes de cada plancha (elecciones), en orden.
    pub candidatos: Vec<(OpcionId, Vec<PersonaId>)>,
    pub roles_aplicados_en: Option<DateTime<Utc>>,
}

impl PreguntaGuardada {
    /// Candidatos de una opción de elección (vacío si la opción no es de candidatos).
    pub fn candidatos_de(&self, opcion: OpcionId) -> &[PersonaId] {
        self.candidatos
            .iter()
            .find(|(o, _)| *o == opcion)
            .map_or(&[], |(_, c)| c.as_slice())
    }
}

/// Totales de votación de una pregunta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConteoVotos {
    /// Bienes que votaron.
    pub votantes: Conteo,
    pub por_opcion: Vec<VotosOpcion>,
}

/// Una fila del detalle de votos (bien → opción). En `reservado` solo con permiso.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetalleVoto {
    pub bien_privado_id: BienPrivadoId,
    pub codigo: String,
    pub coeficiente: Coeficiente,
    pub opcion_id: OpcionId,
    pub canal: Canal,
    pub asistente: String,
    pub apoderado: bool,
    pub registrado_en: DateTime<Utc>,
}

/// Participación (quién votó por cada bien) para el acta: sin la opción elegida.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipacionActa {
    pub bien_privado_id: BienPrivadoId,
    pub codigo: String,
    pub coeficiente: Coeficiente,
    pub canal: Canal,
    pub asistente: String,
    pub apoderado: bool,
    pub registrado_en: DateTime<Utc>,
}

// ---------------------------------------------------------------------------------------------
// Constancias, entregas y costos
// ---------------------------------------------------------------------------------------------

/// Canal de una constancia de convocatoria.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanalConvocatoria {
    Whatsapp,
    Correo,
}

domain::enum_texto!(CanalConvocatoria {
    Whatsapp => "whatsapp",
    Correo => "correo",
});

/// Constancia de envío de la convocatoria a una persona.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvioConvocatoria {
    pub persona_id: PersonaId,
    pub nombre: String,
    pub canal: CanalConvocatoria,
    /// Estado en el outbox (`pendiente`, `enviado`, `fallido`).
    pub estado_envio: String,
    /// Último acuse de WhatsApp (`sent`, `delivered`, `read`, `failed`).
    pub acuse: Option<String>,
    pub enviado_en: Option<DateTime<Utc>>,
}

/// Mensajes de WhatsApp de una reunión, por línea y tipo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MensajesLinea {
    pub linea: Option<String>,
    pub es_plantilla: bool,
    pub enviados: u64,
    pub fallidos: u64,
}

// ---------------------------------------------------------------------------------------------
// Repositorio
// ---------------------------------------------------------------------------------------------

/// Persistencia de asambleas dentro de la transacción de una copropiedad.
pub trait RepositorioAsambleas: Send {
    // --- Reuniones ---
    /// Guarda una reunión nueva en borrador.
    fn insertar_reunion(&mut self, r: &Reunion) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda los cambios de una reunión existente (datos, estado y configuración).
    fn actualizar_reunion(&mut self, r: &Reunion) -> impl Future<Output = AppResult<()>> + Send;

    /// Reunión sin bloqueo (lecturas y votos).
    fn reunion(&mut self, id: ReunionId)
    -> impl Future<Output = AppResult<Option<Reunion>>> + Send;

    /// Reunión con bloqueo de fila (`FOR UPDATE`): las transiciones se serializan.
    fn bloquear_reunion(
        &mut self,
        id: ReunionId,
    ) -> impl Future<Output = AppResult<Option<Reunion>>> + Send;

    /// Más recientes primero (cursor por `id` descendente).
    fn reuniones(
        &mut self,
        estado: Option<EstadoReunion>,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Reunion>>> + Send;

    /// Reuniones cerradas con acta publicada (libro de actas).
    fn actas_publicadas(&mut self) -> impl Future<Output = AppResult<Vec<Reunion>>> + Send;

    // --- Orden del día y documentos ---
    /// Orden del día de la reunión, en su orden.
    fn puntos(
        &mut self,
        reunion: ReunionId,
    ) -> impl Future<Output = AppResult<Vec<PuntoOrden>>> + Send;

    /// Crea (`nuevo`) o actualiza un punto del orden del día.
    fn guardar_punto(
        &mut self,
        reunion: ReunionId,
        p: &PuntoOrden,
        nuevo: bool,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Elimina un punto del orden del día; `false` si no existía.
    fn eliminar_punto(&mut self, id: PuntoOrdenId) -> impl Future<Output = AppResult<bool>> + Send;

    /// Documentos adjuntos a la reunión (convocatoria, informes, soportes).
    fn documentos(
        &mut self,
        reunion: ReunionId,
    ) -> impl Future<Output = AppResult<Vec<DocumentoReunion>>> + Send;

    /// Adjunta un archivo ya subido a la reunión.
    fn agregar_documento(
        &mut self,
        reunion: ReunionId,
        d: &DocumentoReunion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Quita un documento de la reunión; `false` si no estaba adjunto.
    fn quitar_documento(
        &mut self,
        reunion: ReunionId,
        archivo: ArchivoId,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    // --- Padrón y morosos ---
    /// Bienes activos con coeficiente del cuadro vigente y su representante vigente.
    fn bienes_para_padron(&mut self)
    -> impl Future<Output = AppResult<Vec<BienParaPadron>>> + Send;

    /// Fija el padrón al convocar: cada bien con su coeficiente vigente y su representante. No se
    /// modifica después.
    fn insertar_padron(
        &mut self,
        reunion: ReunionId,
        bienes: &[BienParaPadron],
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Padrón fijado de la reunión.
    fn padron(
        &mut self,
        reunion: ReunionId,
    ) -> impl Future<Output = AppResult<Vec<BienPadron>>> + Send;

    /// Bienes del padrón cuyo código contiene `texto` o cuyo representante tiene ese documento
    /// (sin tildes ni mayúsculas).
    fn buscar_en_padron(
        &mut self,
        reunion: ReunionId,
        texto: &str,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<CoincidenciaMesa>>> + Send;

    /// Reemplaza el listado de morosos (bienes del padrón).
    fn reemplazar_morosos(
        &mut self,
        reunion: ReunionId,
        bienes: &[BienPrivadoId],
    ) -> impl Future<Output = AppResult<()>> + Send;

    // --- Asistentes ---
    /// Registra un asistente con los bienes que representa.
    fn insertar_asistente(
        &mut self,
        a: &AsistenteNuevo,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Asistente por su id.
    fn asistente(
        &mut self,
        id: AsistenteId,
    ) -> impl Future<Output = AppResult<Option<Asistente>>> + Send;

    /// Asistente vigente de una persona en la reunión, si ya se registró.
    fn asistente_de_persona(
        &mut self,
        reunion: ReunionId,
        persona: PersonaId,
    ) -> impl Future<Output = AppResult<Option<Asistente>>> + Send;

    /// Asistentes de la reunión.
    fn asistentes(
        &mut self,
        reunion: ReunionId,
    ) -> impl Future<Output = AppResult<Vec<Asistente>>> + Send;

    /// Asistente dueño de la papeleta virtual, por el hash de su token.
    fn asistente_por_token(
        &mut self,
        token_hash: &[u8],
    ) -> impl Future<Output = AppResult<Option<Asistente>>> + Send;

    /// Asistente que tiene asignado el rótulo en la reunión.
    fn asistente_por_rotulo(
        &mut self,
        reunion: ReunionId,
        codigo: &CodigoRotulo,
    ) -> impl Future<Output = AppResult<Option<Asistente>>> + Send;

    /// Asistentes (de cualquier reunión de la copropiedad) con ese teléfono de WhatsApp.
    fn asistentes_por_telefono(
        &mut self,
        telefono: &str,
    ) -> impl Future<Output = AppResult<Vec<Asistente>>> + Send;

    /// Teléfono, línea y confirmación de WhatsApp del asistente.
    fn actualizar_medios(
        &mut self,
        id: AsistenteId,
        telefono: Option<&str>,
        linea: Option<&str>,
        confirmado_en: Option<DateTime<Utc>>,
        confirmacion_outbox_id: Option<i64>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda el hash del token de la papeleta virtual del asistente (reemplaza el anterior).
    fn fijar_token(
        &mut self,
        id: AsistenteId,
        token_hash: &[u8],
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Vincula un bien al asistente desde `desde`. Falla con conflicto si otro asistente ya
    /// lo representa (índice único parcial).
    fn vincular_bien(
        &mut self,
        reunion: ReunionId,
        asistente: AsistenteId,
        bien: BienPrivadoId,
        calidad: CalidadRepresentacion,
        poder: Option<PoderId>,
        desde: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Termina la representación vigente del bien. Devuelve el asistente que lo tenía.
    fn desvincular_bien(
        &mut self,
        reunion: ReunionId,
        bien: BienPrivadoId,
        hasta: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<AsistenteId>>> + Send;

    // --- Poderes ---
    /// Registra un poder.
    fn insertar_poder(&mut self, p: &Poder) -> impl Future<Output = AppResult<()>> + Send;

    /// Poder por su id.
    fn poder(&mut self, id: PoderId) -> impl Future<Output = AppResult<Option<Poder>>> + Send;

    /// Guarda los cambios de un poder (revisión o revocación).
    fn actualizar_poder(&mut self, p: &Poder) -> impl Future<Output = AppResult<()>> + Send;

    /// Poderes de la reunión.
    fn poderes(&mut self, reunion: ReunionId)
    -> impl Future<Output = AppResult<Vec<Poder>>> + Send;

    // --- Presencias y rótulos ---
    /// Abre una presencia del asistente (entrada o reingreso).
    fn abrir_presencia(
        &mut self,
        reunion: ReunionId,
        p: &Presencia,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Cierra la presencia abierta del asistente. `false` si no tenía.
    fn cerrar_presencia(
        &mut self,
        asistente: AsistenteId,
        salida: DateTime<Utc>,
        motivo: MotivoSalida,
        detalle: Option<&str>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Presencias de la reunión, abiertas y cerradas.
    fn presencias(
        &mut self,
        reunion: ReunionId,
    ) -> impl Future<Output = AppResult<Vec<Presencia>>> + Send;

    /// Bienes de los asistentes presentes ahora: base del quórum y de las preguntas que se abren.
    fn bienes_presentes(
        &mut self,
        reunion: ReunionId,
    ) -> impl Future<Output = AppResult<Vec<BienPresente>>> + Send;

    /// Liga el rótulo al asistente. Conflicto si el código ya está en uso en la reunión.
    fn asignar_rotulo(
        &mut self,
        reunion: ReunionId,
        codigo: &CodigoRotulo,
        asistente: AsistenteId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Libera el rótulo asignado al asistente; `false` si no tenía uno.
    fn liberar_rotulo(
        &mut self,
        asistente: AsistenteId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    // --- Quórum ---
    /// Guarda una foto del quórum (append-only).
    fn insertar_foto(
        &mut self,
        reunion: ReunionId,
        f: &FotoQuorum,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Fotos del quórum de la reunión, en orden.
    fn fotos(
        &mut self,
        reunion: ReunionId,
    ) -> impl Future<Output = AppResult<Vec<FotoQuorum>>> + Send;

    // --- Preguntas ---
    /// Inserta (`nueva`) o reemplaza una pregunta en borrador con sus opciones y candidatos.
    fn guardar_pregunta(
        &mut self,
        p: &PreguntaGuardada,
        nueva: bool,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Elimina una pregunta en borrador con sus opciones y candidatos; `false` si no existía o ya
    /// no estaba en borrador.
    fn eliminar_pregunta(&mut self, id: PreguntaId)
    -> impl Future<Output = AppResult<bool>> + Send;

    /// Pregunta sin bloqueo (votos: el trigger la vuelve a verificar con `FOR SHARE`).
    fn pregunta(
        &mut self,
        id: PreguntaId,
    ) -> impl Future<Output = AppResult<Option<PreguntaGuardada>>> + Send;

    /// Pregunta con bloqueo de fila (`FOR UPDATE`) para abrirla, cerrarla o editarla.
    fn bloquear_pregunta(
        &mut self,
        id: PreguntaId,
    ) -> impl Future<Output = AppResult<Option<PreguntaGuardada>>> + Send;

    /// Preguntas de la reunión con sus opciones, en orden.
    fn preguntas(
        &mut self,
        reunion: ReunionId,
    ) -> impl Future<Output = AppResult<Vec<PreguntaGuardada>>> + Send;

    /// Preguntas abiertas cuyo temporizador venció.
    fn preguntas_vencidas(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<PreguntaId>>> + Send;

    /// Consultas por escrito con el plazo cumplido.
    fn consultas_vencidas(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<ReunionId>>> + Send;

    /// Estado, horas, resultado y roles aplicados.
    fn actualizar_pregunta(
        &mut self,
        p: &PreguntaGuardada,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Agrega bienes habilitados (ignora los que ya estaban).
    fn habilitar(
        &mut self,
        pregunta: PreguntaId,
        bienes: &[(BienPrivadoId, Coeficiente)],
        desde: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Coeficiente y unidades de los bienes habilitados para votar la pregunta.
    fn conteo_habilitados(
        &mut self,
        pregunta: PreguntaId,
    ) -> impl Future<Output = AppResult<Conteo>> + Send;

    /// De `bienes`, los habilitados en la pregunta que aún no votaron, con el coeficiente con
    /// que quedaron habilitados.
    fn pendientes_de_voto(
        &mut self,
        pregunta: PreguntaId,
        bienes: &[BienPrivadoId],
    ) -> impl Future<Output = AppResult<Vec<(BienPrivadoId, Coeficiente)>>> + Send;

    /// Participaciones de una emisión anterior con la misma clave (reintento).
    fn participaciones_con_clave(
        &mut self,
        pregunta: PreguntaId,
        asistente: AsistenteId,
        clave: &str,
    ) -> impl Future<Output = AppResult<u32>> + Send;

    /// Inserta participaciones y votos. Una participación que choca con otra del mismo bien
    /// (`ON CONFLICT DO NOTHING`) no inserta sus votos. Devuelve cuántas se insertaron.
    ///
    /// Errores: `Dominio(PreguntaNoAbierta)` si la pregunta se cerró entre la lectura y la
    /// escritura; [`crate::AppError::Conflicto`] si un bien no está habilitado.
    fn registrar_voto(&mut self, v: &VotoEmitido) -> impl Future<Output = AppResult<u32>> + Send;

    /// Totales de la pregunta por opción (coeficiente y unidades).
    fn conteo_votos(
        &mut self,
        pregunta: PreguntaId,
    ) -> impl Future<Output = AppResult<ConteoVotos>> + Send;

    /// Detalle bien → opción. En votaciones reservadas solo se consulta con permiso y se audita.
    fn detalle_votos(
        &mut self,
        pregunta: PreguntaId,
    ) -> impl Future<Output = AppResult<Vec<DetalleVoto>>> + Send;

    /// Participaciones de la pregunta con los datos que exige el acta (hora, canal, apoderado).
    fn participaciones(
        &mut self,
        pregunta: PreguntaId,
    ) -> impl Future<Output = AppResult<Vec<ParticipacionActa>>> + Send;

    /// ¿El asistente ya votó la pregunta (por alguno de sus bienes)?
    fn asistente_voto(
        &mut self,
        pregunta: PreguntaId,
        asistente: AsistenteId,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    // --- Tiempo real, outbox y constancias ---
    /// Avisa a las pantallas en vivo de la reunión (se entrega al confirmar la transacción).
    fn avisar_en_vivo(&mut self, reunion: ReunionId) -> impl Future<Output = AppResult<()>> + Send;

    /// Encola un mensaje atribuido a la reunión (costo por asamblea) y devuelve su id.
    fn encolar_de_reunion(
        &mut self,
        reunion: ReunionId,
        evento: &EventoSaliente,
    ) -> impl Future<Output = AppResult<i64>> + Send;

    /// Constancia de la convocatoria enviada a una persona por un canal (append-only).
    fn registrar_envio_convocatoria(
        &mut self,
        reunion: ReunionId,
        persona: PersonaId,
        canal: CanalConvocatoria,
        outbox_id: i64,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Constancias de envío de la convocatoria con su acuse.
    fn envios_convocatoria(
        &mut self,
        reunion: ReunionId,
    ) -> impl Future<Output = AppResult<Vec<EnvioConvocatoria>>> + Send;

    /// Último acuse de WhatsApp de cada mensaje del outbox.
    fn estados_entrega(
        &mut self,
        outbox: &[i64],
    ) -> impl Future<Output = AppResult<Vec<(i64, Option<String>)>>> + Send;

    /// Mensajes de WhatsApp de la reunión por línea y tipo, para su costo.
    fn mensajes_whatsapp(
        &mut self,
        reunion: ReunionId,
    ) -> impl Future<Output = AppResult<Vec<MensajesLinea>>> + Send;

    /// Tarifas vigentes (categoría → valor) en el país y la moneda.
    fn tarifas_whatsapp(
        &mut self,
        pais: &str,
        moneda: &str,
        fecha: NaiveDate,
    ) -> impl Future<Output = AppResult<Vec<(String, Decimal)>>> + Send;

    /// Línea activa con más franquicia libre (para iniciar una conversación con plantilla).
    fn elegir_linea(&mut self) -> impl Future<Output = AppResult<Option<String>>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Directorio (gateway, cruza copropiedades)
// ---------------------------------------------------------------------------------------------

/// Asistencia activa de un teléfono en alguna copropiedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsistenciaTelefono {
    pub tenant_id: TenantId,
    pub copropiedad: String,
    pub reunion_id: ReunionId,
    pub titulo: String,
    pub asistente_id: AsistenteId,
    pub linea: Option<String>,
    pub confirmado: bool,
}

/// Acuse de entrega de un mensaje de WhatsApp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcuseEntrega {
    pub wamid: String,
    pub linea: String,
    /// `sent`, `delivered`, `read` o `failed`.
    pub estado: String,
    pub categoria: Option<String>,
    pub facturable: Option<bool>,
    pub error_codigo: Option<i32>,
    pub ocurrido_en: DateTime<Utc>,
}

/// Consultas entre copropiedades que usa el gateway (funciones `SECURITY DEFINER`).
pub trait DirectorioAsambleas: Send + Sync {
    /// Asistencias vigentes ligadas a un teléfono en todas las copropiedades (SECURITY DEFINER).
    fn asistencias_por_telefono(
        &self,
        telefono: &str,
    ) -> impl Future<Output = AppResult<Vec<AsistenciaTelefono>>> + Send;

    /// Copropiedades con preguntas cuyo temporizador venció o consultas por escrito con el plazo
    /// cumplido: el worker las cierra. Solo devuelve ids de tenant (SECURITY DEFINER).
    fn copropiedades_con_asambleas_pendientes(
        &self,
    ) -> impl Future<Output = AppResult<Vec<TenantId>>> + Send;

    /// Guarda el acuse de entrega de un mensaje (`statuses` del webhook de Meta).
    fn registrar_acuse(&self, acuse: &AcuseEntrega) -> impl Future<Output = AppResult<()>> + Send;

    /// Calidad de una línea según Meta (por su número visible).
    fn actualizar_calidad_linea(
        &self,
        numero: &str,
        degradada: bool,
        calidad: Option<&str>,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Líneas y tarifas (plataforma)
// ---------------------------------------------------------------------------------------------

/// Estado operativo de una línea de WhatsApp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoLinea {
    /// Recibe conversaciones nuevas.
    Activa,
    /// Meta bajó su calidad: solo atiende las conversaciones ya abiertas.
    Degradada,
    /// Fuera de uso por decisión de la plataforma.
    Pausada,
}

domain::enum_texto!(EstadoLinea {
    Activa => "activa",
    Degradada => "degradada",
    Pausada => "pausada",
});

/// Línea de WhatsApp de la plataforma con su uso del mes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineaWhatsapp {
    pub id: uuid::Uuid,
    pub phone_number_id: String,
    pub numero_visible: String,
    pub nombre: String,
    pub limite_mps: u32,
    pub estado: EstadoLinea,
    pub calidad: Option<String>,
    pub franquicia_servicio_mes: u32,
    /// Mensajes de servicio de este mes (consumen la franquicia gratuita).
    pub servicio_mes: u64,
    /// Plantillas de este mes (se pagan aparte; no consumen la franquicia).
    pub plantillas_mes: u64,
}

impl LineaWhatsapp {
    /// Mensajes de servicio gratuitos que le quedan este mes.
    pub fn franquicia_disponible(&self) -> u64 {
        u64::from(self.franquicia_servicio_mes).saturating_sub(self.servicio_mes)
    }
}

/// Tarifa de Meta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarifaWhatsapp {
    pub pais: String,
    pub categoria: String,
    pub moneda: String,
    pub valor: Decimal,
    pub vigente_desde: NaiveDate,
}

/// Líneas y tarifas en una transacción de plataforma.
pub trait RepositorioLineasWhatsapp: Send {
    /// Líneas de WhatsApp de la plataforma con su uso del mes.
    fn lineas(&mut self) -> impl Future<Output = AppResult<Vec<LineaWhatsapp>>> + Send;

    /// Registra una línea de WhatsApp.
    fn insertar_linea(&mut self, l: &LineaWhatsapp) -> impl Future<Output = AppResult<()>> + Send;

    /// Cambia el estado, el límite de mensajes por segundo y la franquicia de una línea; `false` si
    /// no existe.
    fn actualizar_linea(
        &mut self,
        id: uuid::Uuid,
        estado: EstadoLinea,
        limite_mps: u32,
        franquicia: u32,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Tarifas de Meta por tipo de conversación.
    fn tarifas(&mut self) -> impl Future<Output = AppResult<Vec<TarifaWhatsapp>>> + Send;

    /// Crea o actualiza una tarifa de Meta.
    fn guardar_tarifa(&mut self, t: &TarifaWhatsapp) -> impl Future<Output = AppResult<()>> + Send;
}
