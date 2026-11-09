//! DTOs de las asambleas (no se exponen entidades de dominio directamente).

use application::{
    casos_uso::asambleas::{
        quorum::{PreguntaEnVivo, TableroEnVivo},
        registro::{FilaMesa, ResultadoRegistro},
        votaciones::ResultadoVoto,
    },
    puertos::asambleas::{
        Asistente, BienDeAsistente, CoincidenciaMesa, ConfiguracionReunion, DocumentoReunion,
        EnvioConvocatoria, Poder, PreguntaGuardada, PuntoOrden, Reunion,
    },
};
use chrono::{DateTime, Utc};
use domain::{
    BaseComputo, BaseMayoria, CalidadRepresentacion, CargoEleccion, Convocatoria, EfectoMora,
    EstadoActa, EstadoPoder, EstadoPregunta, EstadoPunto, EstadoReunion, MetodoRegistro, Modalidad,
    QuorumReunion, ReglaMayoria, TipoDocumentoReunion, TipoPregunta, TipoReunion, TipoVoto,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

// ---------------------------------------------------------------------------------------------
// Reuniones
// ---------------------------------------------------------------------------------------------

/// Configuración que la copropiedad fija para la asamblea.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ConfiguracionAsambleaDto {
    /// `coeficiente` (por defecto, lo que exige la ley) o `unidades` (se advierte en el acta).
    #[schema(value_type = String, example = "coeficiente")]
    #[serde(default)]
    pub base_computo: BaseComputo,
    /// Máximo de bienes que representa un apoderado (`null` = sin límite).
    #[schema(example = 2)]
    pub max_bienes_apoderado: Option<u16>,
    /// Cargos (rol plantilla) que no pueden ser apoderados.
    #[schema(example = json!(["administrador", "revisor_fiscal"]))]
    #[serde(default = "prohibidos_por_defecto")]
    pub apoderados_prohibidos: Vec<String>,
    #[serde(default)]
    pub exigir_soporte_poder: bool,
    /// `informativo`, `sin_voto` o `sin_voto_ni_quorum` (validar con asesor).
    #[schema(value_type = String, example = "informativo")]
    #[serde(default)]
    pub efecto_mora: EfectoMora,
    /// Temporizador por defecto de las preguntas, en segundos.
    #[schema(example = 120)]
    pub temporizador_segundos: Option<u32>,
}

fn prohibidos_por_defecto() -> Vec<String> {
    ConfiguracionReunion::default().apoderados_prohibidos
}

impl Default for ConfiguracionAsambleaDto {
    fn default() -> Self {
        ConfiguracionReunion::default().into()
    }
}

impl From<ConfiguracionReunion> for ConfiguracionAsambleaDto {
    fn from(c: ConfiguracionReunion) -> Self {
        Self {
            base_computo: c.base_computo,
            max_bienes_apoderado: c.max_bienes_apoderado,
            apoderados_prohibidos: c.apoderados_prohibidos,
            exigir_soporte_poder: c.exigir_soporte_poder,
            efecto_mora: c.efecto_mora,
            temporizador_segundos: c.temporizador_segundos,
        }
    }
}

impl From<ConfiguracionAsambleaDto> for ConfiguracionReunion {
    fn from(c: ConfiguracionAsambleaDto) -> Self {
        Self {
            base_computo: c.base_computo,
            max_bienes_apoderado: c.max_bienes_apoderado,
            apoderados_prohibidos: c.apoderados_prohibidos,
            exigir_soporte_poder: c.exigir_soporte_poder,
            efecto_mora: c.efecto_mora,
            temporizador_segundos: c.temporizador_segundos,
        }
    }
}

/// Reunión de la asamblea (o consulta por escrito).
#[derive(Debug, Serialize, ToSchema)]
pub struct ReunionRespuesta {
    pub id: Uuid,
    pub titulo: String,
    #[schema(value_type = String, example = "ordinaria")]
    pub tipo: TipoReunion,
    /// `primera`, `segunda` o `derecho_propio`.
    #[schema(value_type = String, example = "primera")]
    pub convocatoria: Convocatoria,
    /// `presencial`, `virtual`, `mixta` o `escrita`.
    #[schema(value_type = String, example = "presencial")]
    pub modalidad: Modalidad,
    /// `borrador`, `convocada`, `en_registro`, `instalada`, `suspendida`, `cerrada`, `cancelada`.
    #[schema(value_type = String, example = "convocada")]
    pub estado: EstadoReunion,
    pub fecha_programada: DateTime<Utc>,
    pub lugar: Option<String>,
    /// Enlace de la videollamada (p. ej. Google Meet).
    pub enlace_video: Option<String>,
    pub plazo_respuesta: Option<DateTime<Utc>>,
    pub reunion_origen_id: Option<Uuid>,
    pub configuracion: ConfiguracionAsambleaDto,
    pub convocada_en: Option<DateTime<Utc>>,
    pub instalada_en: Option<DateTime<Utc>>,
    pub cerrada_en: Option<DateTime<Utc>>,
    pub motivo_cancelacion: Option<String>,
    /// `pendiente`, `generada`, `aprobada` o `publicada`.
    #[schema(value_type = String, example = "pendiente")]
    pub acta_estado: EstadoActa,
    pub acta_archivo_id: Option<Uuid>,
    pub acta_firmada_archivo_id: Option<Uuid>,
}

impl From<Reunion> for ReunionRespuesta {
    fn from(r: Reunion) -> Self {
        Self {
            id: r.id.as_uuid(),
            titulo: r.titulo,
            tipo: r.tipo,
            convocatoria: r.convocatoria,
            modalidad: r.modalidad,
            estado: r.estado,
            fecha_programada: r.fecha_programada,
            lugar: r.lugar,
            enlace_video: r.enlace_video,
            plazo_respuesta: r.plazo_respuesta,
            reunion_origen_id: r.reunion_origen_id.map(domain::ReunionId::as_uuid),
            configuracion: r.configuracion.into(),
            convocada_en: r.convocada_en,
            instalada_en: r.instalada_en,
            cerrada_en: r.cerrada_en,
            motivo_cancelacion: r.motivo_cancelacion,
            acta_estado: r.acta_estado,
            acta_archivo_id: r.acta_archivo_id.map(domain::ArchivoId::as_uuid),
            acta_firmada_archivo_id: r.acta_firmada_archivo_id.map(domain::ArchivoId::as_uuid),
        }
    }
}

/// Punto del orden del día.
#[derive(Debug, Serialize, ToSchema)]
pub struct PuntoRespuesta {
    pub id: Uuid,
    pub orden: u16,
    pub titulo: String,
    pub descripcion: Option<String>,
    #[schema(value_type = String, example = "pendiente")]
    pub estado: EstadoPunto,
    pub iniciado_en: Option<DateTime<Utc>>,
    pub terminado_en: Option<DateTime<Utc>>,
}

impl From<PuntoOrden> for PuntoRespuesta {
    fn from(p: PuntoOrden) -> Self {
        Self {
            id: p.id.as_uuid(),
            orden: p.orden,
            titulo: p.titulo,
            descripcion: p.descripcion,
            estado: p.estado,
            iniciado_en: p.iniciado_en,
            terminado_en: p.terminado_en,
        }
    }
}

/// Documento anexo a la convocatoria.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct DocumentoDto {
    pub archivo_id: Uuid,
    /// `estados_financieros`, `presupuesto`, `informe`, `reglamento` u `otro`.
    #[schema(value_type = String, example = "presupuesto")]
    pub tipo: TipoDocumentoReunion,
    pub titulo: String,
}

impl From<DocumentoReunion> for DocumentoDto {
    fn from(d: DocumentoReunion) -> Self {
        Self {
            archivo_id: d.archivo_id.as_uuid(),
            tipo: d.tipo,
            titulo: d.titulo,
        }
    }
}

/// Reunión con su orden del día y anexos.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleReunionRespuesta {
    pub reunion: ReunionRespuesta,
    pub puntos: Vec<PuntoRespuesta>,
    pub documentos: Vec<DocumentoDto>,
}

/// Constancia de envío de la convocatoria.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConstanciaRespuesta {
    pub persona_id: Uuid,
    pub nombre: String,
    /// `whatsapp` o `correo`.
    pub canal: &'static str,
    /// `pendiente`, `enviado` o `fallido`.
    pub estado_envio: String,
    /// Último acuse de WhatsApp: `sent`, `delivered`, `read` o `failed`.
    pub acuse: Option<String>,
    pub enviado_en: Option<DateTime<Utc>>,
}

impl From<EnvioConvocatoria> for ConstanciaRespuesta {
    fn from(e: EnvioConvocatoria) -> Self {
        Self {
            persona_id: e.persona_id.as_uuid(),
            nombre: e.nombre,
            canal: e.canal.as_str(),
            estado_envio: e.estado_envio,
            acuse: e.acuse,
            enviado_en: e.enviado_en,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Quórum y tablero
// ---------------------------------------------------------------------------------------------

/// Quórum por coeficiente y por unidades.
#[derive(Debug, Serialize, ToSchema)]
pub struct QuorumRespuesta {
    #[schema(value_type = String, example = "0.55")]
    pub coeficiente_presente: Decimal,
    #[schema(value_type = String, example = "1")]
    pub coeficiente_total: Decimal,
    /// Porcentaje del coeficiente (4 decimales).
    #[schema(value_type = String, example = "55.0000")]
    pub porcentaje_coeficiente: Decimal,
    pub unidades_presentes: u32,
    pub unidades_total: u32,
    pub asistentes: u32,
}

impl From<QuorumReunion> for QuorumRespuesta {
    fn from(q: QuorumReunion) -> Self {
        use rust_decimal::prelude::ToPrimitive;
        Self {
            coeficiente_presente: q.coeficientes.representados,
            coeficiente_total: q.coeficientes.total,
            porcentaje_coeficiente: (q.coeficientes.fraccion() * Decimal::ONE_HUNDRED).round_dp(4),
            unidades_presentes: q.unidades.representados.to_u32().unwrap_or(0),
            unidades_total: q.unidades.total.to_u32().unwrap_or(0),
            asistentes: q.asistentes,
        }
    }
}

/// Pregunta con su escrutinio en vivo.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreguntaEnVivoRespuesta {
    pub pregunta: PreguntaRespuesta,
    /// Escrutinio por coeficiente y por unidades (ver `ResultadosRespuesta`).
    #[schema(value_type = Object)]
    pub escrutinio: serde_json::Value,
}

impl From<PreguntaEnVivo> for PreguntaEnVivoRespuesta {
    fn from(p: PreguntaEnVivo) -> Self {
        Self {
            pregunta: p.pregunta.into(),
            escrutinio: serde_json::to_value(p.escrutinio).unwrap_or_default(),
        }
    }
}

/// Lo que muestran las pantallas en vivo.
#[derive(Debug, Serialize, ToSchema)]
pub struct TableroRespuesta {
    pub reunion: ReunionRespuesta,
    pub quorum: QuorumRespuesta,
    pub hay_quorum: bool,
    pub punto_en_curso: Option<String>,
    pub preguntas_abiertas: Vec<PreguntaEnVivoRespuesta>,
    pub ultima_cerrada: Option<PreguntaEnVivoRespuesta>,
    pub generado_en: DateTime<Utc>,
}

impl From<TableroEnVivo> for TableroRespuesta {
    fn from(t: TableroEnVivo) -> Self {
        Self {
            reunion: t.reunion.into(),
            quorum: t.quorum.into(),
            hay_quorum: t.hay_quorum,
            punto_en_curso: t.punto_en_curso,
            preguntas_abiertas: t.preguntas_abiertas.into_iter().map(Into::into).collect(),
            ultima_cerrada: t.ultima_cerrada.map(Into::into),
            generado_en: t.generado_en,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Registro
// ---------------------------------------------------------------------------------------------

/// Bien que representa un asistente.
#[derive(Debug, Serialize, ToSchema)]
pub struct BienAsistenteRespuesta {
    pub bien_privado_id: Uuid,
    #[schema(example = "T3-506")]
    pub codigo: String,
    #[schema(value_type = String, example = "0.01250000")]
    pub coeficiente: Decimal,
    /// `propietario` o `apoderado`.
    #[schema(value_type = String, example = "propietario")]
    pub calidad: CalidadRepresentacion,
    pub poder_id: Option<Uuid>,
    pub moroso: bool,
}

impl From<BienDeAsistente> for BienAsistenteRespuesta {
    fn from(b: BienDeAsistente) -> Self {
        Self {
            bien_privado_id: b.bien_privado_id.as_uuid(),
            codigo: b.codigo,
            coeficiente: b.coeficiente.valor(),
            calidad: b.calidad,
            poder_id: b.poder_id.map(domain::PoderId::as_uuid),
            moroso: b.moroso,
        }
    }
}

/// Asistente de la reunión.
#[derive(Debug, Serialize, ToSchema)]
pub struct AsistenteRespuesta {
    pub id: Uuid,
    pub persona_id: Uuid,
    pub nombre: String,
    /// `mesa`, `prerregistro`, `enlace` o `consulta`.
    #[schema(value_type = String, example = "mesa")]
    pub metodo: MetodoRegistro,
    /// Teléfono de WhatsApp (dato personal; solo para la mesa).
    pub telefono: Option<String>,
    pub whatsapp_confirmado: bool,
    pub presente: bool,
    pub rotulo: Option<String>,
    pub bienes: Vec<BienAsistenteRespuesta>,
    /// Último acuse de la plantilla de confirmación (`failed` = «no llegó: entregue rótulo»).
    pub entrega_confirmacion: Option<String>,
}

impl AsistenteRespuesta {
    pub fn con_entrega(a: Asistente, entrega: Option<String>) -> Self {
        Self {
            id: a.id.as_uuid(),
            persona_id: a.persona_id.as_uuid(),
            nombre: a.nombre,
            metodo: a.metodo,
            telefono: a.telefono,
            whatsapp_confirmado: a.whatsapp_confirmado_en.is_some(),
            presente: a.presente,
            rotulo: a.rotulo.map(String::from),
            bienes: a.bienes.into_iter().map(Into::into).collect(),
            entrega_confirmacion: entrega,
        }
    }
}

impl From<Asistente> for AsistenteRespuesta {
    fn from(a: Asistente) -> Self {
        Self::con_entrega(a, None)
    }
}

impl From<FilaMesa> for AsistenteRespuesta {
    fn from(f: FilaMesa) -> Self {
        Self::con_entrega(f.asistente, f.entrega_confirmacion)
    }
}

/// Resultado del registro en la mesa.
#[derive(Debug, Serialize, ToSchema)]
pub struct RegistroRespuesta {
    pub asistente: AsistenteRespuesta,
    /// Token de la papeleta virtual (enlace o QR): se entrega **una sola vez**.
    pub token_papeleta: Option<String>,
    /// Avisos para la mesa (morosos, poderes pendientes…).
    pub avisos: Vec<String>,
}

impl From<ResultadoRegistro> for RegistroRespuesta {
    fn from(r: ResultadoRegistro) -> Self {
        Self {
            asistente: r.asistente.into(),
            token_papeleta: r.token_papeleta,
            avisos: r.avisos,
        }
    }
}

/// Coincidencia de la búsqueda de la mesa.
#[derive(Debug, Serialize, ToSchema)]
pub struct CoincidenciaRespuesta {
    pub bien_privado_id: Uuid,
    pub codigo: String,
    #[schema(value_type = String, example = "0.01250000")]
    pub coeficiente: Decimal,
    pub moroso: bool,
    pub representante_persona_id: Option<Uuid>,
    pub representante_nombre: Option<String>,
    /// Teléfono del censo (dato personal; solo para la mesa).
    pub representante_telefono: Option<String>,
    /// Otros bienes del mismo representante (se agregan con un toque).
    pub otros_bienes: Vec<Uuid>,
    /// Asistente que ya representa el bien.
    pub asistente_actual: Option<Uuid>,
}

impl From<CoincidenciaMesa> for CoincidenciaRespuesta {
    fn from(c: CoincidenciaMesa) -> Self {
        Self {
            bien_privado_id: c.bien.bien_privado_id.as_uuid(),
            codigo: c.bien.codigo,
            coeficiente: c.bien.coeficiente.valor(),
            moroso: c.bien.moroso,
            representante_persona_id: c.bien.representante.map(domain::PersonaId::as_uuid),
            representante_nombre: c.representante_nombre,
            representante_telefono: c.representante_telefono,
            otros_bienes: c
                .otros_bienes
                .into_iter()
                .map(domain::BienPrivadoId::as_uuid)
                .collect(),
            asistente_actual: c.asistente_actual.map(domain::AsistenteId::as_uuid),
        }
    }
}

/// Poder.
#[derive(Debug, Serialize, ToSchema)]
pub struct PoderRespuesta {
    pub id: Uuid,
    pub bien_privado_id: Uuid,
    pub poderdante_persona_id: Option<Uuid>,
    pub apoderado_persona_id: Uuid,
    pub soporte_archivo_id: Option<Uuid>,
    /// `pendiente`, `verificado`, `rechazado` o `revocado`.
    #[schema(value_type = String, example = "pendiente")]
    pub estado: EstadoPoder,
    pub motivo: Option<String>,
    pub registrado_en: DateTime<Utc>,
    pub revisado_en: Option<DateTime<Utc>>,
}

impl From<Poder> for PoderRespuesta {
    fn from(p: Poder) -> Self {
        Self {
            id: p.id.as_uuid(),
            bien_privado_id: p.bien_privado_id.as_uuid(),
            poderdante_persona_id: p.poderdante.map(domain::PersonaId::as_uuid),
            apoderado_persona_id: p.apoderado.as_uuid(),
            soporte_archivo_id: p.soporte_archivo_id.map(domain::ArchivoId::as_uuid),
            estado: p.estado,
            motivo: p.motivo,
            registrado_en: p.registrado_en,
            revisado_en: p.revisado_en,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Preguntas y votos
// ---------------------------------------------------------------------------------------------

/// Regla de mayoría.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ReglaDto {
    /// `simple` (art. 45), `calificada_70` (art. 46) o `personalizada`.
    #[schema(example = "simple")]
    pub regla: String,
    /// Solo en `personalizada`: porcentaje (0–100].
    #[schema(value_type = Option<String>, example = "60")]
    pub porcentaje: Option<Decimal>,
    /// Solo en `personalizada`: `presentes` o `total`.
    #[schema(value_type = Option<String>, example = "presentes")]
    pub sobre: Option<BaseMayoria>,
}

impl ReglaDto {
    /// # Errors
    ///
    /// [`ApiError::App`] con `validacion` si la regla no es coherente.
    pub fn validar(self) -> ApiResult<ReglaMayoria> {
        Ok(ReglaMayoria::desde_columnas(
            &self.regla,
            self.porcentaje,
            self.sobre.map(BaseMayoria::as_str),
        )?)
    }
}

impl From<ReglaMayoria> for ReglaDto {
    fn from(r: ReglaMayoria) -> Self {
        let (porcentaje, sobre) = r.umbral();
        Self {
            regla: r.codigo().into(),
            porcentaje,
            sobre,
        }
    }
}

/// Opción de una pregunta.
#[derive(Debug, Serialize, ToSchema)]
pub struct OpcionRespuesta {
    pub id: Uuid,
    pub orden: u16,
    pub texto: String,
    pub en_blanco: bool,
    /// Integrantes de la plancha (elecciones).
    pub candidatos: Vec<Uuid>,
}

/// Pregunta.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreguntaRespuesta {
    pub id: Uuid,
    pub orden: u16,
    pub punto_id: Option<Uuid>,
    pub enunciado: String,
    /// `si_no`, `seleccion_unica`, `seleccion_multiple` o `eleccion`.
    #[schema(value_type = String, example = "si_no")]
    pub tipo: TipoPregunta,
    /// `nominal` o `reservado`.
    #[schema(value_type = String, example = "nominal")]
    pub tipo_voto: TipoVoto,
    #[schema(value_type = String, example = "coeficiente")]
    pub base_computo: BaseComputo,
    pub regla_mayoria: ReglaDto,
    pub max_selecciones: u16,
    pub cupos: Option<u16>,
    #[schema(value_type = Option<String>, example = "consejero")]
    pub cargo: Option<CargoEleccion>,
    /// `borrador`, `abierta`, `cerrada` o `anulada`.
    #[schema(value_type = String, example = "abierta")]
    pub estado: EstadoPregunta,
    pub temporizador_segundos: Option<u32>,
    pub cierra_en: Option<DateTime<Utc>>,
    pub abierta_en: Option<DateTime<Utc>>,
    pub cerrada_en: Option<DateTime<Utc>>,
    pub opciones: Vec<OpcionRespuesta>,
    pub roles_aplicados_en: Option<DateTime<Utc>>,
}

impl From<PreguntaGuardada> for PreguntaRespuesta {
    fn from(p: PreguntaGuardada) -> Self {
        let opciones = p
            .pregunta
            .opciones
            .iter()
            .map(|o| OpcionRespuesta {
                id: o.id.as_uuid(),
                orden: o.orden,
                texto: o.texto.clone(),
                en_blanco: o.en_blanco,
                candidatos: p.candidatos_de(o.id).iter().map(|c| c.as_uuid()).collect(),
            })
            .collect();
        let q = p.pregunta;
        Self {
            id: q.id.as_uuid(),
            orden: p.orden,
            punto_id: p.punto_id.map(domain::PuntoOrdenId::as_uuid),
            enunciado: q.enunciado,
            tipo: q.tipo,
            tipo_voto: q.tipo_voto,
            base_computo: q.base_computo,
            regla_mayoria: q.regla_mayoria.into(),
            max_selecciones: q.max_selecciones,
            cupos: q.cupos,
            cargo: q.cargo,
            estado: q.estado,
            temporizador_segundos: p.temporizador_segundos,
            cierra_en: p.cierra_en,
            abierta_en: p.abierta_en,
            cerrada_en: p.cerrada_en,
            opciones,
            roles_aplicados_en: p.roles_aplicados_en,
        }
    }
}

/// Resultado de una emisión de voto (sin la opción elegida).
#[derive(Debug, Serialize, ToSchema)]
pub struct VotoRespuesta {
    /// `false` = reintento de una emisión ya registrada.
    pub registrado: bool,
    pub asistente_id: Uuid,
    pub nombre: String,
    /// Bienes por los que votó.
    pub bienes: Vec<String>,
    /// Bienes que no votaron esta vez (ya habían votado o no estaban habilitados).
    pub omitidos: Vec<String>,
}

impl From<ResultadoVoto> for VotoRespuesta {
    fn from(r: ResultadoVoto) -> Self {
        Self {
            registrado: r.registrado,
            asistente_id: r.asistente_id.as_uuid(),
            nombre: r.nombre,
            bienes: r.bienes,
            omitidos: r.omitidos,
        }
    }
}

/// Rechaza un cuerpo vacío donde se espera algo.
///
/// # Errors
///
/// [`ApiError::PeticionInvalida`].
pub fn exigir(cond: bool, mensaje: &str) -> ApiResult<()> {
    if cond {
        Ok(())
    } else {
        Err(ApiError::PeticionInvalida(mensaje.into()))
    }
}
