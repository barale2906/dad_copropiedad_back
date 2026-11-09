//! Control de acceso, visitas y correspondencia (plan 05): reglas puras de la portería.
//!
//! - **Visitas**: máquina de estados ([`EstadoVisita::transicionar`]) y calendario de
//!   escalamiento: aviso al primer respondiente, a los [`ESPERA_SEGUNDO_AVISO`] a todos los
//!   residentes y a los [`ESPERA_LLAMADA`] la portería debe llamar ([`paso_pendiente_visita`]).
//! - **Correspondencia**: aviso al recibir y cada 24 h hasta la entrega, respetando el horario
//!   de silencio de la PH y un tope de días ([`proximo_aviso_correspondencia`]).
//! - **Destinatarios** ([`destinatarios_aviso`]): el primer aviso va al responsable del bien
//!   privado; los siguientes, a los residentes mayores de edad. Los propietarios que no viven
//!   allí solo se avisan si en el bien no vive ningún residente de 18 a 59 años: todos los
//!   adultos son personas mayores (decisión del 2026-11-09) o no hay residentes adultos.
//! - **Canal** ([`elegir_canal_aviso`]): la app (push) siempre que haya un dispositivo activo;
//!   WhatsApp solo como respaldo, para ahorrar costos.
//! - **Credenciales y autorizaciones** (fases 2–5): estados, normalización de identificadores,
//!   QR temporales firmados ([`CargaQr`]) y evaluación de ventanas ([`VentanaAcceso`]).

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeDelta, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    ArchivoId, DominioError, Horario, PersonaId, Placa, PuntoAccesoId, TenantId, VisitaId,
    importacion::Documento, reserva::Tz,
};

// ---------------------------------------------------------------------------------------------
// Enumeraciones persistidas
// ---------------------------------------------------------------------------------------------

/// Tipo de visita. Todas siguen el mismo flujo de autorización (incluidos los domicilios,
/// decisión del 2026-11-09).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoVisita {
    Visita,
    Domicilio,
    Proveedor,
    Trasteo,
    ServicioDomestico,
}

enum_texto!(TipoVisita {
    Visita => "visita",
    Domicilio => "domicilio",
    Proveedor => "proveedor",
    Trasteo => "trasteo",
    ServicioDomestico => "servicio_domestico",
});

/// Cómo nació la visita.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrigenVisita {
    /// La portería la anuncia y espera la respuesta de un residente.
    AnuncioPorteria,
    /// Un residente la autorizó antes (app, web o chat) y recibe un QR temporal.
    Preautorizada,
}

enum_texto!(OrigenVisita {
    AnuncioPorteria => "anuncio_porteria",
    Preautorizada => "preautorizada",
});

/// Estado de una visita.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoVisita {
    /// Anunciada; se espera la respuesta de un residente.
    Anunciada,
    /// Nadie respondió en [`ESPERA_LLAMADA`]: la portería debe llamar.
    RequiereLlamada,
    Autorizada,
    Rechazada,
    /// La portería llamó y nadie contestó.
    SinRespuesta,
    Ingresada,
    /// Salió de la copropiedad.
    Finalizada,
    /// Autorizada pero no llegó dentro de su ventana.
    Vencida,
    Cancelada,
}

enum_texto!(EstadoVisita {
    Anunciada => "anunciada",
    RequiereLlamada => "requiere_llamada",
    Autorizada => "autorizada",
    Rechazada => "rechazada",
    SinRespuesta => "sin_respuesta",
    Ingresada => "ingresada",
    Finalizada => "finalizada",
    Vencida => "vencida",
    Cancelada => "cancelada",
});

impl EstadoVisita {
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está en el diagrama.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoVisita::*;
        match (self, hacia) {
            (Anunciada, Autorizada | Rechazada | RequiereLlamada | Cancelada)
            | (RequiereLlamada, Autorizada | Rechazada | SinRespuesta | Cancelada)
            | (Autorizada, Ingresada | Vencida | Cancelada)
            // Una pre-autorización con usos pendientes vuelve a quedar autorizada al salir.
            | (Ingresada, Finalizada | Autorizada) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "visita",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }

    /// ¿Aún espera la decisión de un residente (o de la llamada)?
    pub const fn espera_decision(self) -> bool {
        matches!(self, Self::Anunciada | Self::RequiereLlamada)
    }

    /// ¿Está dentro de la copropiedad?
    pub const fn dentro(self) -> bool {
        matches!(self, Self::Ingresada)
    }
}

/// Hecho del historial de una visita (append-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoEventoVisita {
    Anunciada,
    Preautorizada,
    AvisoEnviado,
    Autorizada,
    Rechazada,
    EscaladaLlamada,
    Llamada,
    Ingreso,
    Salida,
    Vencida,
    Cancelada,
}

enum_texto!(TipoEventoVisita {
    Anunciada => "anunciada",
    Preautorizada => "preautorizada",
    AvisoEnviado => "aviso_enviado",
    Autorizada => "autorizada",
    Rechazada => "rechazada",
    EscaladaLlamada => "escalada_llamada",
    Llamada => "llamada",
    Ingreso => "ingreso",
    Salida => "salida",
    Vencida => "vencida",
    Cancelada => "cancelada",
});

/// Por dónde llegó la decisión sobre una visita.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanalRespuestaVisita {
    App,
    Web,
    Whatsapp,
    /// La registró el vigilante tras llamar.
    Llamada,
}

enum_texto!(CanalRespuestaVisita {
    App => "app",
    Web => "web",
    Whatsapp => "whatsapp",
    Llamada => "llamada",
});

/// Resultado de la llamada de la portería.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultadoLlamada {
    Autorizo,
    Rechazo,
    NoContesto,
}

enum_texto!(ResultadoLlamada {
    Autorizo => "autorizo",
    Rechazo => "rechazo",
    NoContesto => "no_contesto",
});

impl ResultadoLlamada {
    /// Estado al que lleva la visita.
    pub const fn estado(self) -> EstadoVisita {
        match self {
            Self::Autorizo => EstadoVisita::Autorizada,
            Self::Rechazo => EstadoVisita::Rechazada,
            Self::NoContesto => EstadoVisita::SinRespuesta,
        }
    }
}

/// Tipo de correspondencia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoCorrespondencia {
    Paquete,
    Sobre,
    Recibo,
    Otro,
}

enum_texto!(TipoCorrespondencia {
    Paquete => "paquete",
    Sobre => "sobre",
    Recibo => "recibo",
    Otro => "otro",
});

/// Estado de la correspondencia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoCorrespondencia {
    Recibida,
    Entregada,
    /// Devuelta al remitente o a la transportadora.
    Devuelta,
}

enum_texto!(EstadoCorrespondencia {
    Recibida => "recibida",
    Entregada => "entregada",
    Devuelta => "devuelta",
});

impl EstadoCorrespondencia {
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si ya se entregó o devolvió.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        match (self, hacia) {
            (Self::Recibida, Self::Entregada | Self::Devuelta) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "correspondencia",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }
}

/// Tipo de novedad de la minuta de portería.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoNovedadMinuta {
    InicioTurno,
    EntregaTurno,
    Ronda,
    Novedad,
    Incidente,
    Otro,
}

enum_texto!(TipoNovedadMinuta {
    InicioTurno => "inicio_turno",
    EntregaTurno => "entrega_turno",
    Ronda => "ronda",
    Novedad => "novedad",
    Incidente => "incidente",
    Otro => "otro",
});

/// Plataforma del dispositivo móvil (apps Flutter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlataformaPush {
    Android,
    Ios,
}

enum_texto!(PlataformaPush {
    Android => "android",
    Ios => "ios",
});

/// Canal de un aviso a un residente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanalAviso {
    Push,
    Whatsapp,
}

enum_texto!(CanalAviso {
    Push => "push",
    Whatsapp => "whatsapp",
});

/// Elige el canal de un aviso: **siempre la app** si la persona tiene un dispositivo push
/// activo; si no, WhatsApp (plantilla con costo); `None` si no tiene ninguno.
pub const fn elegir_canal_aviso(tiene_push: bool, tiene_whatsapp: bool) -> Option<CanalAviso> {
    if tiene_push {
        Some(CanalAviso::Push)
    } else if tiene_whatsapp {
        Some(CanalAviso::Whatsapp)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// Destinatarios de los avisos
// ---------------------------------------------------------------------------------------------

/// Mayoría de edad en Colombia (Código Civil, art. 34; Ley 27 de 1977).
pub const EDAD_MAYORIA: u32 = 18;
/// Persona mayor (Ley 1276 de 2009, art. 7: 60 años o más).
pub const EDAD_PERSONA_MAYOR: u32 = 60;

/// Edad cumplida a `hoy`.
pub fn edad(nacimiento: NaiveDate, hoy: NaiveDate) -> u32 {
    let mut anios = hoy.year() - nacimiento.year();
    if (hoy.month(), hoy.day()) < (nacimiento.month(), nacimiento.day()) {
        anios -= 1;
    }
    u32::try_from(anios).unwrap_or(0)
}

/// Residente vigente de un bien privado, con lo necesario para decidir a quién avisar. La fecha
/// de nacimiento es dato sensible: solo se usa aquí para decidir, nunca se muestra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidenteAviso {
    pub persona: PersonaId,
    /// Responsable del bien ante la copropiedad = **primer respondiente**.
    pub es_responsable: bool,
    pub fecha_nacimiento: Option<NaiveDate>,
}

impl ResidenteAviso {
    /// Sin fecha de nacimiento se presume mayor de edad (el censo la pide solo para menores).
    fn adulto(&self, hoy: NaiveDate) -> bool {
        self.fecha_nacimiento
            .is_none_or(|f| edad(f, hoy) >= EDAD_MAYORIA)
    }

    fn persona_mayor(&self, hoy: NaiveDate) -> bool {
        self.fecha_nacimiento
            .is_some_and(|f| edad(f, hoy) >= EDAD_PERSONA_MAYOR)
    }
}

/// Titular vigente (propietario) del bien privado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitularAviso {
    pub persona: PersonaId,
    pub es_representante: bool,
}

/// Nivel del aviso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NivelAviso {
    /// Primer aviso: solo al primer respondiente.
    PrimerRespondiente,
    /// Repeticiones: a todos los residentes adultos.
    Todos,
}

impl NivelAviso {
    /// Nivel según cuántos avisos se enviaron antes.
    pub const fn segun_avisos_previos(previos: u32) -> Self {
        if previos == 0 {
            Self::PrimerRespondiente
        } else {
            Self::Todos
        }
    }
}

fn agregar(v: &mut Vec<PersonaId>, p: PersonaId) {
    if !v.contains(&p) {
        v.push(p);
    }
}

/// A quién se avisa de una visita o correspondencia (decisiones del 2026-11-09):
///
/// - [`NivelAviso::PrimerRespondiente`]: el responsable del bien si es adulto; si no, el primer
///   residente adulto.
/// - [`NivelAviso::Todos`]: todos los residentes adultos (nunca menores de edad). Los
///   propietarios que no viven allí se agregan si ningún residente tiene de 18 a 59 años
///   (todos los adultos son personas mayores; los menores no cuentan). Sin fecha de
///   nacimiento se presume adulto menor de 60.
/// - Sin residentes adultos: el representante de los titulares (primer aviso) o todos los
///   titulares (repeticiones).
pub fn destinatarios_aviso(
    nivel: NivelAviso,
    residentes: &[ResidenteAviso],
    titulares: &[TitularAviso],
    hoy: NaiveDate,
) -> Vec<PersonaId> {
    let adultos: Vec<&ResidenteAviso> = residentes.iter().filter(|r| r.adulto(hoy)).collect();
    let mut v = Vec::new();
    if adultos.is_empty() {
        match nivel {
            NivelAviso::PrimerRespondiente => {
                if let Some(t) = titulares
                    .iter()
                    .find(|t| t.es_representante)
                    .or_else(|| titulares.first())
                {
                    v.push(t.persona);
                }
            }
            NivelAviso::Todos => titulares.iter().for_each(|t| agregar(&mut v, t.persona)),
        }
        return v;
    }
    match nivel {
        NivelAviso::PrimerRespondiente => {
            let r = adultos
                .iter()
                .find(|r| r.es_responsable)
                .unwrap_or(&adultos[0]);
            v.push(r.persona);
        }
        NivelAviso::Todos => {
            adultos.iter().for_each(|r| agregar(&mut v, r.persona));
            if adultos.iter().all(|r| r.persona_mayor(hoy)) {
                titulares.iter().for_each(|t| agregar(&mut v, t.persona));
            }
        }
    }
    v
}

/// ¿Puede esta persona decidir sobre una visita del bien? Quien recibiría el aviso de nivel
/// [`NivelAviso::Todos`].
pub fn puede_decidir_visita(
    persona: PersonaId,
    residentes: &[ResidenteAviso],
    titulares: &[TitularAviso],
    hoy: NaiveDate,
) -> bool {
    destinatarios_aviso(NivelAviso::Todos, residentes, titulares, hoy).contains(&persona)
}

// ---------------------------------------------------------------------------------------------
// Escalamiento de visitas
// ---------------------------------------------------------------------------------------------

/// Sin respuesta en este tiempo, se avisa a todos los residentes.
pub const ESPERA_SEGUNDO_AVISO: TimeDelta = TimeDelta::minutes(2);
/// Sin respuesta en este tiempo, la portería debe llamar.
pub const ESPERA_LLAMADA: TimeDelta = TimeDelta::minutes(5);

/// Siguiente paso del escalamiento de una visita anunciada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasoVisita {
    /// Aviso a todos los residentes.
    SegundoAviso,
    /// Alerta a la portería para que llame.
    PedirLlamada,
}

/// Paso pendiente de una visita: ninguno si ya se decidió o aún no es hora. Si el worker se
/// atrasó más allá de [`ESPERA_LLAMADA`], se pide la llamada directamente.
pub fn paso_pendiente_visita(
    estado: EstadoVisita,
    anunciada_en: DateTime<Utc>,
    avisos_enviados: u32,
    ahora: DateTime<Utc>,
) -> Option<PasoVisita> {
    if estado != EstadoVisita::Anunciada {
        return None;
    }
    let espera = ahora - anunciada_en;
    if espera >= ESPERA_LLAMADA {
        Some(PasoVisita::PedirLlamada)
    } else if espera >= ESPERA_SEGUNDO_AVISO && avisos_enviados < 2 {
        Some(PasoVisita::SegundoAviso)
    } else {
        None
    }
}

/// Momento en que corresponde el siguiente paso (para programar el worker).
pub fn momento_siguiente_paso(anunciada_en: DateTime<Utc>, avisos_enviados: u32) -> DateTime<Utc> {
    if avisos_enviados < 2 {
        anunciada_en + ESPERA_SEGUNDO_AVISO
    } else {
        anunciada_en + ESPERA_LLAMADA
    }
}

// ---------------------------------------------------------------------------------------------
// Configuración por PH y avisos de correspondencia
// ---------------------------------------------------------------------------------------------

/// Horario de silencio (hora local de la PH) en el que no salen avisos de correspondencia. Las
/// visitas siempre avisan de inmediato. Puede cruzar la medianoche (21:00–07:00).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HorarioSilencio {
    pub inicio: NaiveTime,
    pub fin: NaiveTime,
}

impl HorarioSilencio {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si inicio y fin coinciden.
    pub fn new(inicio: NaiveTime, fin: NaiveTime) -> Result<Self, DominioError> {
        if inicio == fin {
            return Err(DominioError::DatoInvalido {
                campo: "horario_silencio",
                regla: "el inicio y el fin deben ser distintos",
            });
        }
        Ok(Self { inicio, fin })
    }

    /// ¿La hora local cae dentro del silencio? `[inicio, fin)`.
    pub fn contiene(&self, hora: NaiveTime) -> bool {
        if self.inicio < self.fin {
            hora >= self.inicio && hora < self.fin
        } else {
            hora >= self.inicio || hora < self.fin
        }
    }
}

/// Configuración del control de acceso de una PH (editable por la administración).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguracionAcceso {
    /// Días que se guardan la firma y la foto de quien recoge correspondencia (por defecto 365).
    pub retencion_soportes_dias: u32,
    /// Días durante los que se repite el aviso de correspondencia; después se alerta a la
    /// administración (por defecto 7).
    pub dias_avisos_correspondencia: u32,
    /// Silencio de los avisos de correspondencia (por defecto 21:00–07:00).
    pub silencio: Option<HorarioSilencio>,
    /// Días que se guardan los eventos de acceso (por defecto 730 = 2 años).
    pub retencion_eventos_dias: u32,
    /// Días que se guardan las fotos de placas (por defecto 30, guía de la SIC).
    pub retencion_fotos_placas_dias: u32,
}

impl Default for ConfiguracionAcceso {
    fn default() -> Self {
        Self {
            retencion_soportes_dias: 365,
            dias_avisos_correspondencia: 7,
            silencio: Some(HorarioSilencio {
                inicio: NaiveTime::from_hms_opt(21, 0, 0).unwrap_or(NaiveTime::MIN),
                fin: NaiveTime::from_hms_opt(7, 0, 0).unwrap_or(NaiveTime::MIN),
            }),
            retencion_eventos_dias: 730,
            retencion_fotos_placas_dias: 30,
        }
    }
}

fn rango(
    campo: &'static str,
    v: u32,
    min: u32,
    max: u32,
    regla: &'static str,
) -> Result<(), DominioError> {
    if (min..=max).contains(&v) {
        Ok(())
    } else {
        Err(DominioError::DatoInvalido { campo, regla })
    }
}

impl ConfiguracionAcceso {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si un plazo está fuera de rango.
    pub fn validar(self) -> Result<Self, DominioError> {
        rango(
            "retencion_soportes_dias",
            self.retencion_soportes_dias,
            30,
            3650,
            "entre 30 y 3650 días",
        )?;
        rango(
            "dias_avisos_correspondencia",
            self.dias_avisos_correspondencia,
            1,
            60,
            "entre 1 y 60 días",
        )?;
        rango(
            "retencion_eventos_dias",
            self.retencion_eventos_dias,
            365,
            730,
            "entre 365 y 730 días (1 a 2 años)",
        )?;
        rango(
            "retencion_fotos_placas_dias",
            self.retencion_fotos_placas_dias,
            1,
            90,
            "entre 1 y 90 días",
        )?;
        Ok(self)
    }
}

/// Intervalo entre avisos de correspondencia pendiente.
pub const INTERVALO_AVISO_CORRESPONDENCIA: TimeDelta = TimeDelta::hours(24);

fn a_utc(tz: Tz, fecha: NaiveDate, hora: NaiveTime) -> DateTime<Utc> {
    let local = fecha.and_time(hora);
    tz.from_local_datetime(&local)
        .earliest()
        .map_or_else(|| local.and_utc(), |d| d.with_timezone(&Utc))
}

/// Primer momento `>= t` fuera del silencio (hora local de la PH).
pub fn fuera_de_silencio(
    t: DateTime<Utc>,
    tz: Tz,
    silencio: Option<HorarioSilencio>,
) -> DateTime<Utc> {
    let Some(s) = silencio else { return t };
    let local = t.with_timezone(&tz);
    let hora = local.time();
    if !s.contiene(hora) {
        return t;
    }
    let fecha = local.date_naive();
    let fecha_fin = if s.inicio > s.fin && hora >= s.inicio {
        fecha.succ_opt().unwrap_or(fecha)
    } else {
        fecha
    };
    a_utc(tz, fecha_fin, s.fin)
}

/// Momento del siguiente aviso de una correspondencia pendiente, o `None` si ya se agotaron los
/// días de aviso (entonces se alerta a la administración una sola vez).
pub fn proximo_aviso_correspondencia(
    recibida_en: DateTime<Utc>,
    ultimo_aviso: DateTime<Utc>,
    config: &ConfiguracionAcceso,
    tz: Tz,
) -> Option<DateTime<Utc>> {
    let siguiente = fuera_de_silencio(
        ultimo_aviso + INTERVALO_AVISO_CORRESPONDENCIA,
        tz,
        config.silencio,
    );
    let limite = recibida_en + TimeDelta::days(i64::from(config.dias_avisos_correspondencia));
    (siguiente <= limite).then_some(siguiente)
}

// ---------------------------------------------------------------------------------------------
// Entrega de correspondencia
// ---------------------------------------------------------------------------------------------

/// Quién recoge la correspondencia.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuienRecoge {
    /// Persona registrada en el bien (se elige del listado de residentes).
    Registrada(PersonaId),
    /// Tercero no registrado: nombre y documento (datos personales).
    Tercero {
        nombre: String,
        documento: Documento,
    },
}

/// Soporte de la entrega: firma en pantalla y/o foto de quien recoge (al menos uno).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoporteEntrega {
    firma: Option<ArchivoId>,
    foto: Option<ArchivoId>,
}

impl SoporteEntrega {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no trae ni firma ni foto.
    pub fn new(firma: Option<ArchivoId>, foto: Option<ArchivoId>) -> Result<Self, DominioError> {
        if firma.is_none() && foto.is_none() {
            return Err(DominioError::DatoInvalido {
                campo: "soporte",
                regla: "se exige la firma o la foto de quien recoge",
            });
        }
        Ok(Self { firma, foto })
    }

    pub const fn firma(&self) -> Option<ArchivoId> {
        self.firma
    }

    pub const fn foto(&self) -> Option<ArchivoId> {
        self.foto
    }
}

// ---------------------------------------------------------------------------------------------
// Credenciales, puntos y dispositivos (fases 2–5)
// ---------------------------------------------------------------------------------------------

/// Tipo de credencial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoCredencial {
    TarjetaRfid,
    Pin,
    QrPermanente,
    QrTemporal,
    Placa,
    /// Id de enrolamiento en el equipo biométrico; la plantilla **nunca** sale del equipo.
    BiometriaRef,
    AppMovil,
}

enum_texto!(TipoCredencial {
    TarjetaRfid => "tarjeta_rfid",
    Pin => "pin",
    QrPermanente => "qr_permanente",
    QrTemporal => "qr_temporal",
    Placa => "placa",
    BiometriaRef => "biometria_ref",
    AppMovil => "app_movil",
});

impl TipoCredencial {
    /// Normaliza el identificador antes de calcular su huella (nunca se guarda en claro).
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no tiene el formato del tipo.
    pub fn normalizar(self, valor: &str) -> Result<String, DominioError> {
        let v = valor.trim();
        let invalido = |regla| DominioError::DatoInvalido {
            campo: "identificador",
            regla,
        };
        match self {
            Self::Pin => {
                if (4..=8).contains(&v.len()) && v.chars().all(|c| c.is_ascii_digit()) {
                    Ok(v.to_owned())
                } else {
                    Err(invalido("el PIN tiene de 4 a 8 dígitos"))
                }
            }
            Self::TarjetaRfid => {
                let t = v.replace([' ', ':', '-'], "").to_uppercase();
                if (4..=32).contains(&t.len()) && t.chars().all(|c| c.is_ascii_hexdigit()) {
                    Ok(t)
                } else {
                    Err(invalido(
                        "número de tarjeta de 4 a 32 dígitos hexadecimales",
                    ))
                }
            }
            Self::Placa => Ok(Placa::new(v)?.as_str().to_owned()),
            Self::QrPermanente | Self::QrTemporal | Self::BiometriaRef | Self::AppMovil => {
                if (1..=200).contains(&v.len()) && !v.chars().any(char::is_control) {
                    Ok(v.to_owned())
                } else {
                    Err(invalido("de 1 a 200 caracteres"))
                }
            }
        }
    }
}

/// Huella de un identificador de credencial: SHA-256 de `tenant:tipo:normalizado` en hex. El
/// puente la calcula igual al leer una tarjeta, un PIN o una placa, así el identificador nunca
/// viaja ni se guarda en claro (invariante 6).
pub fn huella_credencial(tenant: TenantId, tipo: TipoCredencial, normalizado: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(tenant.as_uuid().as_bytes());
    h.update(b":");
    h.update(tipo.as_str().as_bytes());
    h.update(b":");
    h.update(normalizado.as_bytes());
    hex::encode(h.finalize())
}

/// Estado de una credencial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoCredencial {
    Activa,
    Suspendida,
    Revocada,
    Perdida,
}

enum_texto!(EstadoCredencial {
    Activa => "activa",
    Suspendida => "suspendida",
    Revocada => "revocada",
    Perdida => "perdida",
});

impl EstadoCredencial {
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si la credencial ya es definitiva (revocada o
    /// perdida) o la transición no existe.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoCredencial::*;
        match (self, hacia) {
            (Activa, Suspendida | Revocada | Perdida)
            | (Suspendida, Activa | Revocada | Perdida) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "credencial",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }
}

/// Tipo de punto de acceso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoPuntoAcceso {
    Peatonal,
    Vehicular,
    Mixto,
    /// Dentro de la copropiedad (gimnasio, piscina…).
    Interno,
}

enum_texto!(TipoPuntoAcceso {
    Peatonal => "peatonal",
    Vehicular => "vehicular",
    Mixto => "mixto",
    Interno => "interno",
});

/// Sentido de un punto o de un evento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SentidoAcceso {
    Entrada,
    Salida,
    Ambos,
}

enum_texto!(SentidoAcceso {
    Entrada => "entrada",
    Salida => "salida",
    Ambos => "ambos",
});

/// Tipo de dispositivo conectado a un puente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoDispositivoAcceso {
    Torniquete,
    Talanquera,
    Puerta,
    LectorQr,
    LectorRfid,
    TecladoPin,
    BiometricoHuella,
    BiometricoFacial,
    CamaraLpr,
}

enum_texto!(TipoDispositivoAcceso {
    Torniquete => "torniquete",
    Talanquera => "talanquera",
    Puerta => "puerta",
    LectorQr => "lector_qr",
    LectorRfid => "lector_rfid",
    TecladoPin => "teclado_pin",
    BiometricoHuella => "biometrico_huella",
    BiometricoFacial => "biometrico_facial",
    CamaraLpr => "camara_lpr",
});

impl TipoDispositivoAcceso {
    pub const fn es_biometrico(self) -> bool {
        matches!(self, Self::BiometricoHuella | Self::BiometricoFacial)
    }
}

/// Estado de un puente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoPuente {
    Activo,
    /// Credencial de máquina revocada: ya no sincroniza ni reporta.
    Retirado,
}

enum_texto!(EstadoPuente {
    Activo => "activo",
    Retirado => "retirado",
});

/// Resultado de un intento de acceso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultadoAcceso {
    Permitido,
    Denegado,
    /// Apertura sin credencial (puerta forzada o abierta a la fuerza).
    Forzado,
    /// Apertura manual del vigilante.
    Manual,
}

enum_texto!(ResultadoAcceso {
    Permitido => "permitido",
    Denegado => "denegado",
    Forzado => "forzado",
    Manual => "manual",
});

/// De dónde sale una autorización de acceso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrigenAutorizacionAcceso {
    Residencia,
    Propiedad,
    Contrato,
    Reserva,
    Visita,
    Manual,
}

enum_texto!(OrigenAutorizacionAcceso {
    Residencia => "residencia",
    Propiedad => "propiedad",
    Contrato => "contrato",
    Reserva => "reserva",
    Visita => "visita",
    Manual => "manual",
});

impl OrigenAutorizacionAcceso {
    /// Las derivadas se crean y revocan solas (invariante 8); solo las manuales se editan.
    pub const fn es_derivada(self) -> bool {
        !matches!(self, Self::Manual)
    }
}

/// Cuándo y dónde vale una autorización.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VentanaAcceso {
    /// Puntos permitidos; vacío = todos.
    pub puntos: Vec<PuntoAccesoId>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    /// Franjas semanales (hora local de la PH); `None` = a toda hora.
    pub horario: Option<Horario>,
}

impl VentanaAcceso {
    /// ¿Permite pasar por `punto` a la hora `ahora`?
    pub fn permite(&self, punto: PuntoAccesoId, ahora: DateTime<Utc>, tz: Tz) -> bool {
        if ahora < self.vigente_desde || self.vigente_hasta.is_some_and(|h| ahora >= h) {
            return false;
        }
        if !self.puntos.is_empty() && !self.puntos.contains(&punto) {
            return false;
        }
        self.horario.as_ref().is_none_or(|h| {
            let local = ahora.with_timezone(&tz);
            h.abierto(local.date_naive(), local.time())
        })
    }
}

// ---------------------------------------------------------------------------------------------
// QR temporal firmado
// ---------------------------------------------------------------------------------------------

/// Prefijo y versión del texto de un QR de visita.
pub const PREFIJO_QR: &str = "DQ1";
const LARGO_CARGA: usize = 1 + 16 + 16 + 8 + 8 + 2;
/// Largo de una firma Ed25519.
pub const LARGO_FIRMA: usize = 64;

/// Contenido firmado de un QR temporal: el puente lo valida sin internet con la clave pública
/// de la PH (Ed25519). No lleva datos personales.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CargaQr {
    pub tenant: TenantId,
    pub visita: VisitaId,
    pub desde: DateTime<Utc>,
    pub hasta: DateTime<Utc>,
    pub usos: u16,
}

impl CargaQr {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si la ventana está vacía o no hay usos.
    pub fn new(
        tenant: TenantId,
        visita: VisitaId,
        desde: DateTime<Utc>,
        hasta: DateTime<Utc>,
        usos: u16,
    ) -> Result<Self, DominioError> {
        if hasta <= desde {
            return Err(DominioError::DatoInvalido {
                campo: "ventana",
                regla: "el fin debe ser posterior al inicio",
            });
        }
        if usos == 0 {
            return Err(DominioError::DatoInvalido {
                campo: "usos",
                regla: "al menos un uso",
            });
        }
        Ok(Self {
            tenant,
            visita,
            desde,
            hasta,
            usos,
        })
    }

    /// Bytes que se firman (versión, tenant, visita, ventana en segundos Unix, usos).
    pub fn bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(LARGO_CARGA);
        b.push(1);
        b.extend_from_slice(self.tenant.as_uuid().as_bytes());
        b.extend_from_slice(self.visita.as_uuid().as_bytes());
        b.extend_from_slice(&self.desde.timestamp().to_be_bytes());
        b.extend_from_slice(&self.hasta.timestamp().to_be_bytes());
        b.extend_from_slice(&self.usos.to_be_bytes());
        b
    }

    /// Inverso de [`CargaQr::bytes`].
    pub fn desde_bytes(b: &[u8]) -> Option<Self> {
        if b.len() != LARGO_CARGA || b[0] != 1 {
            return None;
        }
        let uuid = |r: std::ops::Range<usize>| uuid::Uuid::from_slice(&b[r]).ok();
        let i64_en = |i: usize| b.get(i..i + 8)?.try_into().ok().map(i64::from_be_bytes);
        Some(Self {
            tenant: TenantId::from_uuid(uuid(1..17)?),
            visita: VisitaId::from_uuid(uuid(17..33)?),
            desde: DateTime::from_timestamp(i64_en(33)?, 0)?,
            hasta: DateTime::from_timestamp(i64_en(41)?, 0)?,
            usos: u16::from_be_bytes(b.get(49..51)?.try_into().ok()?),
        })
    }

    /// Texto del QR: `DQ1.<carga hex>.<firma hex>`.
    pub fn texto(&self, firma: &[u8]) -> String {
        format!(
            "{PREFIJO_QR}.{}.{}",
            hex::encode(self.bytes()),
            hex::encode(firma)
        )
    }

    /// Separa un texto de QR en (carga, firma) sin verificar la firma.
    pub fn leer_texto(texto: &str) -> Option<(Self, Vec<u8>)> {
        let mut partes = texto.trim().split('.');
        if partes.next()? != PREFIJO_QR {
            return None;
        }
        let carga = hex::decode(partes.next()?).ok()?;
        let firma = hex::decode(partes.next()?).ok()?;
        if partes.next().is_some() || firma.len() != LARGO_FIRMA {
            return None;
        }
        Some((Self::desde_bytes(&carga)?, firma))
    }

    /// ¿Se puede usar ahora, llevando `usados` usos? (invariante 7).
    pub fn admite(&self, ahora: DateTime<Utc>, usados: u32) -> bool {
        ahora >= self.desde && ahora < self.hasta && usados < u32::from(self.usos)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn p(n: u128) -> PersonaId {
        PersonaId::from_uuid(uuid::Uuid::from_u128(n))
    }

    fn hoy() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 9).expect("fecha")
    }

    fn residente(
        n: u128,
        responsable: bool,
        nacimiento: Option<(i32, u32, u32)>,
    ) -> ResidenteAviso {
        ResidenteAviso {
            persona: p(n),
            es_responsable: responsable,
            fecha_nacimiento: nacimiento.and_then(|(a, m, d)| NaiveDate::from_ymd_opt(a, m, d)),
        }
    }

    fn titular(n: u128, representante: bool) -> TitularAviso {
        TitularAviso {
            persona: p(n),
            es_representante: representante,
        }
    }

    #[test]
    fn maquina_de_estados_de_la_visita() {
        use EstadoVisita::*;
        for (d, h) in [
            (Anunciada, Autorizada),
            (Anunciada, RequiereLlamada),
            (RequiereLlamada, SinRespuesta),
            (Autorizada, Ingresada),
            (Ingresada, Finalizada),
            (Autorizada, Vencida),
        ] {
            assert_eq!(d.transicionar(h), Ok(h));
        }
        for (d, h) in [
            (Rechazada, Autorizada),
            (Finalizada, Ingresada),
            (Anunciada, Ingresada),
            (SinRespuesta, Autorizada),
        ] {
            assert!(d.transicionar(h).is_err(), "{d} -> {h}");
        }
    }

    #[test]
    fn correspondencia_entregada_no_vuelve_atras() {
        use EstadoCorrespondencia::*;
        assert_eq!(Recibida.transicionar(Entregada), Ok(Entregada));
        assert!(Entregada.transicionar(Devuelta).is_err());
        assert!(Devuelta.transicionar(Entregada).is_err());
    }

    #[test]
    fn la_app_tiene_prioridad_sobre_whatsapp() {
        assert_eq!(elegir_canal_aviso(true, true), Some(CanalAviso::Push));
        assert_eq!(elegir_canal_aviso(false, true), Some(CanalAviso::Whatsapp));
        assert_eq!(elegir_canal_aviso(false, false), None);
    }

    #[test]
    fn primer_aviso_va_al_responsable() {
        let r = [residente(1, false, None), residente(2, true, None)];
        assert_eq!(
            destinatarios_aviso(
                NivelAviso::PrimerRespondiente,
                &r,
                &[titular(9, true)],
                hoy()
            ),
            vec![p(2)]
        );
    }

    #[test]
    fn primer_aviso_sin_responsable_adulto_va_al_primer_adulto() {
        let r = [
            residente(1, true, Some((2015, 1, 1))),
            residente(2, false, None),
        ];
        assert_eq!(
            destinatarios_aviso(NivelAviso::PrimerRespondiente, &r, &[], hoy()),
            vec![p(2)]
        );
    }

    #[test]
    fn repeticiones_van_a_los_adultos_y_nunca_a_menores() {
        let r = [
            residente(1, true, None),
            residente(2, false, Some((2012, 5, 5))),
            residente(3, false, Some((1990, 1, 1))),
        ];
        assert_eq!(
            destinatarios_aviso(NivelAviso::Todos, &r, &[titular(9, true)], hoy()),
            vec![p(1), p(3)]
        );
    }

    #[test]
    fn propietarios_no_residentes_si_no_vive_nadie_de_18_a_59() {
        let t = [titular(9, true), titular(8, false)];
        let mayor = [residente(1, true, Some((1950, 3, 3)))];
        assert_eq!(
            destinatarios_aviso(NivelAviso::Todos, &mayor, &t, hoy()),
            vec![p(1), p(9), p(8)]
        );
        let joven = [residente(1, true, Some((1990, 3, 3)))];
        assert_eq!(
            destinatarios_aviso(NivelAviso::Todos, &joven, &t, hoy()),
            vec![p(1)]
        );
        let dos_mayores = [
            residente(1, true, Some((1950, 3, 3))),
            residente(2, false, Some((1940, 1, 1))),
        ];
        assert_eq!(
            destinatarios_aviso(NivelAviso::Todos, &dos_mayores, &t, hoy()),
            vec![p(1), p(2), p(9), p(8)]
        );
        let mayor_y_menor = [
            residente(1, true, Some((1950, 3, 3))),
            residente(2, false, Some((2015, 1, 1))),
        ];
        assert_eq!(
            destinatarios_aviso(NivelAviso::Todos, &mayor_y_menor, &t, hoy()),
            vec![p(1), p(9), p(8)],
            "los menores no cuentan"
        );
        let mayor_y_adulto = [
            residente(1, true, Some((1950, 3, 3))),
            residente(2, false, Some((1990, 1, 1))),
        ];
        assert_eq!(
            destinatarios_aviso(NivelAviso::Todos, &mayor_y_adulto, &t, hoy()),
            vec![p(1), p(2)]
        );
        let sin_fecha = [
            residente(1, true, Some((1950, 3, 3))),
            residente(2, false, None),
        ];
        assert_eq!(
            destinatarios_aviso(NivelAviso::Todos, &sin_fecha, &t, hoy()),
            vec![p(1), p(2)],
            "sin fecha se presume adulto menor de 60"
        );
        // El primer aviso sigue siendo solo para la persona mayor.
        assert_eq!(
            destinatarios_aviso(NivelAviso::PrimerRespondiente, &mayor, &t, hoy()),
            vec![p(1)]
        );
    }

    #[test]
    fn sin_residentes_adultos_se_avisa_a_los_titulares() {
        let t = [titular(8, false), titular(9, true)];
        let menores = [residente(1, true, Some((2016, 1, 1)))];
        assert_eq!(
            destinatarios_aviso(NivelAviso::PrimerRespondiente, &menores, &t, hoy()),
            vec![p(9)]
        );
        assert_eq!(
            destinatarios_aviso(NivelAviso::Todos, &[], &t, hoy()),
            vec![p(8), p(9)]
        );
    }

    #[test]
    fn solo_decide_quien_recibiria_el_aviso() {
        let r = [
            residente(1, true, None),
            residente(2, false, Some((2014, 1, 1))),
        ];
        assert!(puede_decidir_visita(p(1), &r, &[titular(9, true)], hoy()));
        assert!(!puede_decidir_visita(p(2), &r, &[], hoy()), "menor de edad");
        assert!(
            !puede_decidir_visita(p(9), &r, &[titular(9, true)], hoy()),
            "propietario no residente"
        );
    }

    #[test]
    fn edad_cumplida() {
        let n = NaiveDate::from_ymd_opt(2008, 11, 10).expect("fecha");
        assert_eq!(edad(n, hoy()), 17);
        assert_eq!(
            edad(n, NaiveDate::from_ymd_opt(2026, 11, 10).expect("fecha")),
            18
        );
    }

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 9, 15, 0, 0)
            .single()
            .expect("hora")
    }

    #[test]
    fn escalamiento_de_la_visita_a_los_2_y_5_minutos() {
        let e = EstadoVisita::Anunciada;
        assert_eq!(
            paso_pendiente_visita(e, t0(), 1, t0() + TimeDelta::seconds(119)),
            None
        );
        assert_eq!(
            paso_pendiente_visita(e, t0(), 1, t0() + TimeDelta::minutes(2)),
            Some(PasoVisita::SegundoAviso)
        );
        assert_eq!(
            paso_pendiente_visita(e, t0(), 2, t0() + TimeDelta::minutes(3)),
            None
        );
        assert_eq!(
            paso_pendiente_visita(e, t0(), 2, t0() + TimeDelta::minutes(5)),
            Some(PasoVisita::PedirLlamada)
        );
        assert_eq!(
            paso_pendiente_visita(e, t0(), 1, t0() + TimeDelta::minutes(9)),
            Some(PasoVisita::PedirLlamada)
        );
        assert_eq!(
            paso_pendiente_visita(
                EstadoVisita::Autorizada,
                t0(),
                1,
                t0() + TimeDelta::minutes(9)
            ),
            None
        );
        assert_eq!(momento_siguiente_paso(t0(), 1), t0() + ESPERA_SEGUNDO_AVISO);
        assert_eq!(momento_siguiente_paso(t0(), 2), t0() + ESPERA_LLAMADA);
    }

    fn bogota() -> Tz {
        chrono_tz::America::Bogota
    }

    fn local(d: u32, h: u32, m: u32) -> DateTime<Utc> {
        bogota()
            .with_ymd_and_hms(2026, 11, d, h, m, 0)
            .single()
            .expect("hora")
            .with_timezone(&Utc)
    }

    #[test]
    fn el_silencio_corre_el_aviso_a_la_manana() {
        let c = ConfiguracionAcceso::default();
        assert_eq!(
            fuera_de_silencio(local(9, 23, 0), bogota(), c.silencio),
            local(10, 7, 0)
        );
        assert_eq!(
            fuera_de_silencio(local(10, 5, 30), bogota(), c.silencio),
            local(10, 7, 0)
        );
        assert_eq!(
            fuera_de_silencio(local(10, 12, 0), bogota(), c.silencio),
            local(10, 12, 0)
        );
        assert_eq!(
            fuera_de_silencio(local(10, 23, 0), bogota(), None),
            local(10, 23, 0)
        );
    }

    #[test]
    fn silencio_dentro_del_mismo_dia() {
        let s = HorarioSilencio::new(
            NaiveTime::from_hms_opt(12, 0, 0).expect("hora"),
            NaiveTime::from_hms_opt(14, 0, 0).expect("hora"),
        )
        .expect("silencio");
        assert!(s.contiene(NaiveTime::from_hms_opt(13, 0, 0).expect("hora")));
        assert!(!s.contiene(NaiveTime::from_hms_opt(14, 0, 0).expect("hora")));
        assert_eq!(
            fuera_de_silencio(local(10, 13, 0), bogota(), Some(s)),
            local(10, 14, 0)
        );
        assert!(HorarioSilencio::new(s.inicio, s.inicio).is_err());
    }

    #[test]
    fn avisos_de_correspondencia_cada_24_horas_hasta_el_tope() {
        let c = ConfiguracionAcceso {
            dias_avisos_correspondencia: 2,
            ..ConfiguracionAcceso::default()
        };
        let recibida = local(9, 10, 0);
        assert_eq!(
            proximo_aviso_correspondencia(recibida, recibida, &c, bogota()),
            Some(local(10, 10, 0))
        );
        assert_eq!(
            proximo_aviso_correspondencia(recibida, local(10, 10, 0), &c, bogota()),
            Some(local(11, 10, 0))
        );
        assert_eq!(
            proximo_aviso_correspondencia(recibida, local(11, 10, 0), &c, bogota()),
            None
        );
    }

    #[test]
    fn aviso_de_correspondencia_que_cae_en_silencio_se_aplaza() {
        let c = ConfiguracionAcceso::default();
        let recibida = local(9, 22, 30);
        assert_eq!(
            proximo_aviso_correspondencia(recibida, local(10, 7, 0), &c, bogota()),
            Some(local(11, 7, 0))
        );
        assert_eq!(
            fuera_de_silencio(recibida, bogota(), c.silencio),
            local(10, 7, 0)
        );
    }

    #[test]
    fn configuracion_fuera_de_rango() {
        assert!(ConfiguracionAcceso::default().validar().is_ok());
        let mala = ConfiguracionAcceso {
            retencion_eventos_dias: 100,
            ..ConfiguracionAcceso::default()
        };
        assert!(mala.validar().is_err());
        let mala = ConfiguracionAcceso {
            retencion_soportes_dias: 10,
            ..ConfiguracionAcceso::default()
        };
        assert!(mala.validar().is_err());
    }

    #[test]
    fn entrega_exige_firma_o_foto() {
        let a = ArchivoId::nuevo();
        assert!(SoporteEntrega::new(None, None).is_err());
        assert_eq!(
            SoporteEntrega::new(Some(a), None).map(|s| s.firma()),
            Ok(Some(a))
        );
        assert!(SoporteEntrega::new(None, Some(a)).is_ok());
    }

    #[test]
    fn identificadores_de_credencial_normalizados() {
        assert_eq!(TipoCredencial::Pin.normalizar(" 1234 "), Ok("1234".into()));
        assert!(TipoCredencial::Pin.normalizar("12a4").is_err());
        assert_eq!(
            TipoCredencial::TarjetaRfid.normalizar("0a:1b:2c:3d"),
            Ok("0A1B2C3D".into())
        );
        assert!(TipoCredencial::TarjetaRfid.normalizar("xyz").is_err());
        assert_eq!(
            TipoCredencial::Placa.normalizar("abc-123"),
            Ok("ABC123".into())
        );
    }

    #[test]
    fn huella_depende_del_tenant_y_del_tipo() {
        let t = TenantId::from_uuid(uuid::Uuid::from_u128(1));
        let a = huella_credencial(t, TipoCredencial::Pin, "1234");
        assert_eq!(a.len(), 64);
        assert_eq!(a, huella_credencial(t, TipoCredencial::Pin, "1234"));
        assert_ne!(
            a,
            huella_credencial(TenantId::nuevo(), TipoCredencial::Pin, "1234")
        );
        assert_ne!(a, huella_credencial(t, TipoCredencial::TarjetaRfid, "1234"));
        assert!(!a.contains("1234"));
    }

    #[test]
    fn credencial_revocada_es_definitiva() {
        use EstadoCredencial::*;
        assert_eq!(Activa.transicionar(Suspendida), Ok(Suspendida));
        assert_eq!(Suspendida.transicionar(Activa), Ok(Activa));
        assert!(Revocada.transicionar(Activa).is_err());
        assert!(Perdida.transicionar(Activa).is_err());
    }

    #[test]
    fn ventana_de_acceso_por_punto_vigencia_y_horario() {
        use chrono::Weekday;
        let punto = PuntoAccesoId::nuevo();
        let otro = PuntoAccesoId::nuevo();
        let horario = Horario::new(
            vec![crate::FranjaSemanal {
                dia: Weekday::Mon,
                inicio: NaiveTime::from_hms_opt(8, 0, 0).expect("hora"),
                fin: NaiveTime::from_hms_opt(18, 0, 0).expect("hora"),
                aplica_festivos: true,
            }],
            vec![],
        )
        .expect("horario");
        let v = VentanaAcceso {
            puntos: vec![punto],
            vigente_desde: local(1, 0, 0),
            vigente_hasta: Some(local(30, 0, 0)),
            horario: Some(horario),
        };
        // 9 de noviembre de 2026 es lunes.
        assert!(v.permite(punto, local(9, 10, 0), bogota()));
        assert!(
            !v.permite(punto, local(9, 19, 0), bogota()),
            "fuera del horario"
        );
        assert!(!v.permite(otro, local(9, 10, 0), bogota()), "otro punto");
        assert!(!v.permite(punto, local(10, 10, 0), bogota()), "martes");
        let sin_limites = VentanaAcceso {
            puntos: vec![],
            vigente_desde: local(1, 0, 0),
            vigente_hasta: None,
            horario: None,
        };
        assert!(sin_limites.permite(otro, local(20, 3, 0), bogota()));
        assert!(!sin_limites.permite(otro, local(1, 0, 0) - TimeDelta::seconds(1), bogota()));
    }

    #[test]
    fn carga_qr_ida_y_vuelta_y_usos() {
        let c = CargaQr::new(
            TenantId::nuevo(),
            VisitaId::nuevo(),
            t0(),
            t0() + TimeDelta::hours(4),
            2,
        )
        .expect("carga");
        let texto = c.texto(&[7; LARGO_FIRMA]);
        let (leida, firma) = CargaQr::leer_texto(&texto).expect("texto");
        assert_eq!(leida, c);
        assert_eq!(firma, vec![7; LARGO_FIRMA]);
        assert!(c.admite(t0(), 1));
        assert!(!c.admite(t0(), 2), "sin usos");
        assert!(
            !c.admite(t0() + TimeDelta::hours(4), 0),
            "fuera de la ventana"
        );
        assert!(CargaQr::leer_texto("DQ1.zz.00").is_none());
        assert!(CargaQr::leer_texto(&texto.replace("DQ1", "XX9")).is_none());
        assert!(CargaQr::new(c.tenant, c.visita, t0(), t0(), 1).is_err());
        assert!(CargaQr::new(c.tenant, c.visita, t0(), t0() + TimeDelta::hours(1), 0).is_err());
    }
}
