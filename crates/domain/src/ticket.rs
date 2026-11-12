//! Tickets (plan 07b): peticiones, quejas, reclamos, sugerencias y solicitudes que la
//! administración atiende por web, WhatsApp, correo o en persona.
//!
//! Reglas de este módulo:
//! - Los estados solo cambian por [`EstadoTicket::transicionar`]; el historial es append-only.
//! - Cada ticket tiene un número legible por copropiedad y año ([`NumeroTicket`],
//!   `PQR-2026-00042`).
//! - El plazo de respuesta se cuenta en horas corridas, horas hábiles o días hábiles con los
//!   festivos de Colombia ([`vencimiento`]). Los valores por defecto siguen la Ley 1755 de 2015
//!   (15 días hábiles para peticiones, 10 para documentos) y cada copropiedad puede cambiarlos.
//! - Mientras el ticket espera al solicitante el plazo no corre: al reanudarse se cuenta de nuevo
//!   (Ley 1755, art. 17).
//! - El reporte de convivencia es confidencial: la identidad del denunciante solo la ve quien
//!   tiene `tickets.convivencia.ver` (comité de convivencia, art. 58 de la Ley 675).

use std::fmt;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::{DominioError, calendario, reserva::Tz};

/// Clase de ticket. El texto es contrato con el `CHECK` de `tickets.tipo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoTicket {
    Peticion,
    Queja,
    Reclamo,
    Sugerencia,
    Felicitacion,
    /// Daño o necesidad de mantenimiento (puede originar una orden de trabajo).
    SolicitudMantenimiento,
    SolicitudAdministrativa,
    /// Copias, certificados, paz y salvo (Ley 1755: 10 días hábiles).
    SolicitudDocumentos,
    /// Confidencial: puede escalar al comité de convivencia (art. 58).
    ReporteConvivencia,
}

enum_texto!(TipoTicket {
    Peticion => "peticion",
    Queja => "queja",
    Reclamo => "reclamo",
    Sugerencia => "sugerencia",
    Felicitacion => "felicitacion",
    SolicitudMantenimiento => "solicitud_mantenimiento",
    SolicitudAdministrativa => "solicitud_administrativa",
    SolicitudDocumentos => "solicitud_documentos",
    ReporteConvivencia => "reporte_convivencia",
});

impl TipoTicket {
    /// ¿Protege la identidad del solicitante?
    pub const fn es_confidencial(self) -> bool {
        matches!(self, Self::ReporteConvivencia)
    }
}

/// Prioridad de atención.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrioridadTicket {
    Urgente,
    Alta,
    Media,
    Baja,
}

enum_texto!(PrioridadTicket {
    Urgente => "urgente",
    Alta => "alta",
    Media => "media",
    Baja => "baja",
});

/// Por dónde llegó el ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanalTicket {
    Web,
    Whatsapp,
    Presencial,
    Correo,
}

enum_texto!(CanalTicket {
    Web => "web",
    Whatsapp => "whatsapp",
    Presencial => "presencial",
    Correo => "correo",
});

/// Estado de un ticket.
///
/// ```text
/// abierto ─┬─> en_revision ─┬─> asignado ─> en_progreso ⇄ en_espera_tercero
///          │                │                    ⇅        en_espera_solicitante
///          └────────────────┴─> rechazado | duplicado     │
///                                     resuelto <──────────┘
///                                      ├─> cerrado
///                                      └─> reabierto ─> asignado | en_progreso | resuelto
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoTicket {
    Abierto,
    EnRevision,
    Asignado,
    EnProgreso,
    EnEsperaTercero,
    EnEsperaSolicitante,
    Resuelto,
    Cerrado,
    Reabierto,
    Rechazado,
    Duplicado,
}

enum_texto!(EstadoTicket {
    Abierto => "abierto",
    EnRevision => "en_revision",
    Asignado => "asignado",
    EnProgreso => "en_progreso",
    EnEsperaTercero => "en_espera_tercero",
    EnEsperaSolicitante => "en_espera_solicitante",
    Resuelto => "resuelto",
    Cerrado => "cerrado",
    Reabierto => "reabierto",
    Rechazado => "rechazado",
    Duplicado => "duplicado",
});

impl EstadoTicket {
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está en el diagrama.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoTicket::*;
        match (self, hacia) {
            (Abierto, EnRevision | Asignado | Rechazado | Duplicado)
            | (EnRevision, Asignado | Resuelto | Rechazado | Duplicado)
            | (Asignado, EnProgreso | Resuelto)
            | (EnProgreso, EnEsperaTercero | EnEsperaSolicitante | Resuelto)
            | (EnEsperaTercero | EnEsperaSolicitante, EnProgreso)
            | (Resuelto, Cerrado | Reabierto)
            | (Reabierto, Asignado | EnProgreso | Resuelto) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "ticket",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }

    /// Ya no admite trabajo ni mensajes del solicitante.
    pub const fn es_final(self) -> bool {
        matches!(self, Self::Cerrado | Self::Rechazado | Self::Duplicado)
    }

    /// Pendiente de atención: cuenta para vencimientos y para el tablero de abiertos.
    pub const fn pendiente(self) -> bool {
        !matches!(
            self,
            Self::Resuelto | Self::Cerrado | Self::Rechazado | Self::Duplicado
        )
    }

    /// ¿Corre el plazo de respuesta? No mientras se espera al solicitante (Ley 1755, art. 17).
    pub const fn corre_plazo(self) -> bool {
        self.pendiente() && !matches!(self, Self::EnEsperaSolicitante)
    }

    /// Estados en los que se puede cambiar el responsable.
    pub const fn admite_asignacion(self) -> bool {
        matches!(
            self,
            Self::Abierto
                | Self::EnRevision
                | Self::Asignado
                | Self::EnProgreso
                | Self::EnEsperaTercero
                | Self::EnEsperaSolicitante
                | Self::Reabierto
        )
    }
}

/// Número legible de un ticket: consecutivo por copropiedad que reinicia cada año.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NumeroTicket {
    pub anio: i32,
    pub consecutivo: u32,
}

impl NumeroTicket {
    /// Prefijo común a todos los tipos (decisión del 2026-11-08: una sola serie por PH y año).
    pub const PREFIJO: &'static str = "PQR";
}

impl fmt::Display for NumeroTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}-{:05}", Self::PREFIJO, self.anio, self.consecutivo)
    }
}

/// Unidad de un plazo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnidadPlazo {
    /// Horas corridas (incluye noches y festivos).
    Horas,
    /// Horas dentro de la jornada hábil de la copropiedad.
    HorasHabiles,
    /// Días hábiles: vence al final de la jornada del último día (se cuenta desde el día
    /// siguiente a la radicación, como en la Ley 1755).
    DiasHabiles,
}

enum_texto!(UnidadPlazo {
    Horas => "horas",
    HorasHabiles => "horas_habiles",
    DiasHabiles => "dias_habiles",
});

/// Plazo de respuesta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Plazo {
    pub cantidad: u32,
    pub unidad: UnidadPlazo,
}

impl Plazo {
    /// Máximo razonable (un año en horas): evita configuraciones que no vencen nunca.
    pub const MAXIMO: u32 = 8_760;

    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si la cantidad es 0 o supera [`Plazo::MAXIMO`].
    pub fn new(cantidad: u32, unidad: UnidadPlazo) -> Result<Self, DominioError> {
        if cantidad == 0 || cantidad > Self::MAXIMO {
            return Err(DominioError::DatoInvalido {
                campo: "plazo",
                regla: "la cantidad debe estar entre 1 y 8760",
            });
        }
        Ok(Self { cantidad, unidad })
    }

    const fn dias(cantidad: u32) -> Self {
        Self {
            cantidad,
            unidad: UnidadPlazo::DiasHabiles,
        }
    }
}

/// Jornada hábil de la administración, en la hora local de la copropiedad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JornadaHabil {
    pub inicio: NaiveTime,
    pub fin: NaiveTime,
    /// Los sábados no festivos también son hábiles.
    pub incluye_sabado: bool,
}

impl Default for JornadaHabil {
    /// Lunes a viernes de 8:00 a 17:00.
    fn default() -> Self {
        Self {
            inicio: NaiveTime::from_hms_opt(8, 0, 0).unwrap_or(NaiveTime::MIN),
            fin: NaiveTime::from_hms_opt(17, 0, 0).unwrap_or(NaiveTime::MIN),
            incluye_sabado: false,
        }
    }
}

impl JornadaHabil {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si la jornada no termina después de empezar.
    pub fn validar(self) -> Result<Self, DominioError> {
        if self.fin <= self.inicio {
            return Err(DominioError::DatoInvalido {
                campo: "jornada",
                regla: "el fin debe ser posterior al inicio",
            });
        }
        Ok(self)
    }

    /// ¿La fecha es hábil para esta jornada?
    pub fn es_habil(&self, dia: NaiveDate) -> bool {
        calendario::es_dia_habil(dia)
            || (self.incluye_sabado
                && dia.weekday() == chrono::Weekday::Sat
                && !calendario::es_festivo(dia))
    }

    fn siguiente_habil(&self, desde: NaiveDate) -> Option<NaiveDate> {
        let mut dia = desde.succ_opt()?;
        // Un año sin días hábiles no existe en el calendario colombiano; el tope evita bucles.
        for _ in 0..366 {
            if self.es_habil(dia) {
                return Some(dia);
            }
            dia = dia.succ_opt()?;
        }
        None
    }
}

fn a_utc(tz: Tz, local: NaiveDateTime) -> DateTime<Utc> {
    tz.from_local_datetime(&local)
        .earliest()
        // Una hora local inexistente (cambio de horario) se toma una hora después.
        .or_else(|| {
            tz.from_local_datetime(&(local + Duration::hours(1)))
                .earliest()
        })
        .map_or_else(|| local.and_utc(), |d| d.with_timezone(&Utc))
}

/// Fecha y hora en que vence un plazo que empieza en `desde`.
///
/// ```
/// use chrono::{NaiveDate, TimeZone, Utc};
/// use domain::ticket::{JornadaHabil, Plazo, UnidadPlazo, vencimiento};
///
/// let tz: chrono_tz::Tz = "America/Bogota".parse().expect("zona");
/// // Radicado el jueves 2 de abril de 2026 (Jueves Santo) a las 10:00 locales: 3 días hábiles
/// // vencen el miércoles 8 al final de la jornada (17:00).
/// let desde = tz.with_ymd_and_hms(2026, 4, 2, 10, 0, 0).unwrap().with_timezone(&Utc);
/// let plazo = Plazo::new(3, UnidadPlazo::DiasHabiles).expect("válido");
/// let vence = vencimiento(desde, plazo, &JornadaHabil::default(), tz).expect("calculable");
/// assert_eq!(vence, tz.with_ymd_and_hms(2026, 4, 8, 17, 0, 0).unwrap().with_timezone(&Utc));
/// ```
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si el resultado se sale del rango de fechas.
pub fn vencimiento(
    desde: DateTime<Utc>,
    plazo: Plazo,
    jornada: &JornadaHabil,
    tz: Tz,
) -> Result<DateTime<Utc>, DominioError> {
    let fuera_de_rango = DominioError::DatoInvalido {
        campo: "plazo",
        regla: "el vencimiento se sale del calendario",
    };
    match plazo.unidad {
        UnidadPlazo::Horas => desde
            .checked_add_signed(Duration::hours(i64::from(plazo.cantidad)))
            .ok_or(fuera_de_rango),
        UnidadPlazo::DiasHabiles => {
            let mut dia = desde.with_timezone(&tz).date_naive();
            for _ in 0..plazo.cantidad {
                dia = jornada.siguiente_habil(dia).ok_or(fuera_de_rango.clone())?;
            }
            Ok(a_utc(tz, dia.and_time(jornada.fin)))
        }
        UnidadPlazo::HorasHabiles => {
            let mut restante = Duration::hours(i64::from(plazo.cantidad));
            let local = desde.with_timezone(&tz).naive_local();
            let mut dia = local.date();
            let mut cursor = local.time();
            loop {
                if jornada.es_habil(dia) && cursor < jornada.fin {
                    let inicio = cursor.max(jornada.inicio);
                    let disponible = jornada.fin - inicio;
                    if restante <= disponible {
                        return Ok(a_utc(tz, dia.and_time(inicio) + restante));
                    }
                    restante -= disponible;
                }
                dia = jornada.siguiente_habil(dia).ok_or(fuera_de_rango.clone())?;
                cursor = NaiveTime::MIN;
            }
        }
    }
}

/// Plazos configurados por la copropiedad: por tipo y, opcionalmente, por prioridad.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlazosTicket {
    reglas: Vec<(TipoTicket, Option<PrioridadTicket>, Plazo)>,
}

impl PlazosTicket {
    /// Plazos con las reglas de la copropiedad (las que falten se toman de
    /// [`plazo_predeterminado`]).
    pub fn new(reglas: Vec<(TipoTicket, Option<PrioridadTicket>, Plazo)>) -> Self {
        Self { reglas }
    }

    /// Plazo que aplica: (tipo, prioridad) → (tipo, cualquiera) → valor por defecto.
    /// `None` = sin plazo (felicitaciones).
    pub fn plazo_para(&self, tipo: TipoTicket, prioridad: PrioridadTicket) -> Option<Plazo> {
        self.reglas
            .iter()
            .find(|(t, p, _)| *t == tipo && *p == Some(prioridad))
            .or_else(|| {
                self.reglas
                    .iter()
                    .find(|(t, p, _)| *t == tipo && p.is_none())
            })
            .map(|(_, _, plazo)| *plazo)
            .or_else(|| plazo_predeterminado(tipo, prioridad))
    }

    /// Plazos configurados por tipo y, opcionalmente, por prioridad.
    pub fn reglas(&self) -> &[(TipoTicket, Option<PrioridadTicket>, Plazo)] {
        &self.reglas
    }
}

/// Plazos por defecto (decisión del 2026-11-08): Ley 1755 de 2015 para peticiones (15 días
/// hábiles) y documentos (10); mantenimiento según la prioridad.
pub const fn plazo_predeterminado(tipo: TipoTicket, prioridad: PrioridadTicket) -> Option<Plazo> {
    use PrioridadTicket::*;
    match (tipo, prioridad) {
        (TipoTicket::Felicitacion, _) => None,
        (TipoTicket::SolicitudDocumentos, _) => Some(Plazo::dias(10)),
        (TipoTicket::SolicitudMantenimiento, Urgente) => Some(Plazo {
            cantidad: 4,
            unidad: UnidadPlazo::HorasHabiles,
        }),
        (TipoTicket::SolicitudMantenimiento, Alta) => Some(Plazo::dias(2)),
        (TipoTicket::SolicitudMantenimiento, Media) => Some(Plazo::dias(5)),
        (TipoTicket::SolicitudMantenimiento, Baja) => Some(Plazo::dias(10)),
        _ => Some(Plazo::dias(15)),
    }
}

/// Días hábiles por defecto para reabrir un ticket resuelto; luego se cierra solo.
pub const DIAS_REAPERTURA_PREDETERMINADOS: u32 = 5;

/// Calificación de la encuesta de satisfacción (1 a 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Calificacion(u8);

impl Calificacion {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no está entre 1 y 5.
    pub fn new(valor: u8) -> Result<Self, DominioError> {
        if (1..=5).contains(&valor) {
            Ok(Self(valor))
        } else {
            Err(DominioError::DatoInvalido {
                campo: "calificacion",
                regla: "debe estar entre 1 y 5",
            })
        }
    }

    /// Calificación de 1 a 5.
    pub const fn valor(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Calificacion {
    type Error = DominioError;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}

impl From<Calificacion> for u8 {
    fn from(c: Calificacion) -> Self {
        c.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bogota() -> Tz {
        "America/Bogota".parse().expect("zona")
    }

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        bogota()
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .expect("fecha")
            .with_timezone(&Utc)
    }

    fn plazo(n: u32, u: UnidadPlazo) -> Plazo {
        Plazo::new(n, u).expect("válido")
    }

    #[test]
    fn transiciones_del_diagrama() {
        use EstadoTicket::*;
        let validas = [
            (Abierto, EnRevision),
            (Abierto, Asignado),
            (EnRevision, Resuelto),
            (Asignado, EnProgreso),
            (EnProgreso, EnEsperaSolicitante),
            (EnEsperaSolicitante, EnProgreso),
            (EnProgreso, Resuelto),
            (Resuelto, Reabierto),
            (Reabierto, EnProgreso),
            (Resuelto, Cerrado),
            (EnRevision, Duplicado),
        ];
        for (d, h) in validas {
            assert_eq!(d.transicionar(h), Ok(h), "{d} -> {h}");
        }
        let invalidas = [
            (Cerrado, Reabierto),
            (Rechazado, Abierto),
            (Abierto, Resuelto),
            (Asignado, Rechazado),
            (Resuelto, EnProgreso),
            (EnEsperaTercero, Resuelto),
        ];
        for (d, h) in invalidas {
            assert!(d.transicionar(h).is_err(), "{d} -> {h}");
        }
    }

    #[test]
    fn el_plazo_no_corre_esperando_al_solicitante() {
        assert!(EstadoTicket::EnProgreso.corre_plazo());
        assert!(!EstadoTicket::EnEsperaSolicitante.corre_plazo());
        assert!(!EstadoTicket::Resuelto.corre_plazo());
        assert!(EstadoTicket::EnEsperaSolicitante.pendiente());
        assert!(EstadoTicket::Cerrado.es_final() && !EstadoTicket::Resuelto.es_final());
    }

    #[test]
    fn numero_legible() {
        let n = NumeroTicket {
            anio: 2026,
            consecutivo: 42,
        };
        assert_eq!(n.to_string(), "PQR-2026-00042");
    }

    #[test]
    fn dias_habiles_saltan_fines_de_semana_y_festivos() {
        let j = JornadaHabil::default();
        // Viernes 30 de octubre de 2026; el lunes 2 de noviembre es festivo (Todos los Santos).
        let desde = local(2026, 10, 30, 16, 0);
        let vence = vencimiento(desde, plazo(1, UnidadPlazo::DiasHabiles), &j, bogota());
        assert_eq!(vence, Ok(local(2026, 11, 3, 17, 0)));
        let quince = vencimiento(desde, plazo(15, UnidadPlazo::DiasHabiles), &j, bogota());
        // 3,4,5,6,9,10,11,12,13,(16 festivo),17,18,19,20,23,24 → el 15.º es el 24.
        assert_eq!(quince, Ok(local(2026, 11, 24, 17, 0)));
    }

    #[test]
    fn horas_habiles_continuan_la_jornada_siguiente() {
        let j = JornadaHabil::default();
        // Miércoles 4 de noviembre de 2026 a las 15:00: quedan 2 h hoy y 2 h el jueves.
        let vence = vencimiento(
            local(2026, 11, 4, 15, 0),
            plazo(4, UnidadPlazo::HorasHabiles),
            &j,
            bogota(),
        );
        assert_eq!(vence, Ok(local(2026, 11, 5, 10, 0)));
        // Radicado de noche: empieza a contar al abrir la jornada.
        let noche = vencimiento(
            local(2026, 11, 4, 22, 0),
            plazo(4, UnidadPlazo::HorasHabiles),
            &j,
            bogota(),
        );
        assert_eq!(noche, Ok(local(2026, 11, 5, 12, 0)));
        // Antes de la jornada.
        let madrugada = vencimiento(
            local(2026, 11, 4, 6, 0),
            plazo(1, UnidadPlazo::HorasHabiles),
            &j,
            bogota(),
        );
        assert_eq!(madrugada, Ok(local(2026, 11, 4, 9, 0)));
    }

    #[test]
    fn sabado_habil_si_la_jornada_lo_incluye() {
        let j = JornadaHabil {
            incluye_sabado: true,
            ..JornadaHabil::default()
        };
        // Viernes 6 de noviembre de 2026 a las 16:00 + 2 h hábiles → sábado 7 a las 9:00.
        let vence = vencimiento(
            local(2026, 11, 6, 16, 0),
            plazo(2, UnidadPlazo::HorasHabiles),
            &j,
            bogota(),
        );
        assert_eq!(vence, Ok(local(2026, 11, 7, 9, 0)));
    }

    #[test]
    fn horas_corridas_no_miran_el_calendario() {
        let vence = vencimiento(
            local(2026, 11, 7, 22, 0),
            plazo(4, UnidadPlazo::Horas),
            &JornadaHabil::default(),
            bogota(),
        );
        assert_eq!(vence, Ok(local(2026, 11, 8, 2, 0)));
    }

    #[test]
    fn plazos_por_defecto_y_configurados() {
        use PrioridadTicket::*;
        let defecto = PlazosTicket::default();
        assert_eq!(
            defecto.plazo_para(TipoTicket::Peticion, Media),
            Some(plazo(15, UnidadPlazo::DiasHabiles))
        );
        assert_eq!(
            defecto.plazo_para(TipoTicket::SolicitudDocumentos, Alta),
            Some(plazo(10, UnidadPlazo::DiasHabiles))
        );
        assert_eq!(
            defecto.plazo_para(TipoTicket::SolicitudMantenimiento, Urgente),
            Some(plazo(4, UnidadPlazo::HorasHabiles))
        );
        assert_eq!(defecto.plazo_para(TipoTicket::Felicitacion, Baja), None);

        let ph = PlazosTicket::new(vec![
            (TipoTicket::Queja, None, plazo(8, UnidadPlazo::DiasHabiles)),
            (
                TipoTicket::Queja,
                Some(Urgente),
                plazo(24, UnidadPlazo::Horas),
            ),
        ]);
        assert_eq!(
            ph.plazo_para(TipoTicket::Queja, Urgente),
            Some(plazo(24, UnidadPlazo::Horas))
        );
        assert_eq!(
            ph.plazo_para(TipoTicket::Queja, Baja),
            Some(plazo(8, UnidadPlazo::DiasHabiles))
        );
        assert_eq!(
            ph.plazo_para(TipoTicket::Reclamo, Baja),
            Some(plazo(15, UnidadPlazo::DiasHabiles))
        );
    }

    #[test]
    fn plazo_y_jornada_invalidos() {
        assert!(Plazo::new(0, UnidadPlazo::Horas).is_err());
        assert!(Plazo::new(Plazo::MAXIMO + 1, UnidadPlazo::Horas).is_err());
        let j = JornadaHabil {
            inicio: NaiveTime::from_hms_opt(17, 0, 0).expect("hora"),
            fin: NaiveTime::from_hms_opt(8, 0, 0).expect("hora"),
            incluye_sabado: false,
        };
        assert!(j.validar().is_err());
    }

    #[test]
    fn calificacion_de_uno_a_cinco() {
        assert!(Calificacion::new(0).is_err());
        assert!(Calificacion::new(6).is_err());
        assert_eq!(Calificacion::new(5).map(Calificacion::valor), Ok(5));
        assert!(serde_json::from_str::<Calificacion>("9").is_err());
    }

    #[test]
    fn solo_convivencia_es_confidencial() {
        for t in TipoTicket::TODOS {
            assert_eq!(
                t.es_confidencial(),
                *t == TipoTicket::ReporteConvivencia,
                "{t}"
            );
            assert_eq!(TipoTicket::try_from(t.as_str()), Ok(*t));
        }
    }
}
