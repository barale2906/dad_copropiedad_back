//! Recursos reservables y reservas (plan 04): bienes comunes, parqueaderos de visitantes y la
//! agenda de la administración comparten un solo motor.
//!
//! - La [`PoliticaReserva`] de cada recurso dice cómo se asigna, cuánto dura, con cuánta
//!   anticipación, quién reserva, cuánto cuesta y qué penalidad tiene cancelar tarde.
//! - Los horarios ([`Horario`]) se evalúan en la **hora local de la PH** (zona IANA).
//! - [`disponibilidad`] calcula las franjas libres: horario − excepciones − ocupadas −
//!   alistamiento.
//! - Montos en `Decimal` (COP), nunca `f64`.

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Timelike, Utc};
/// Zona horaria IANA (re-exportada para que las capas externas no dependan de `chrono-tz`).
pub use chrono_tz::Tz;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

use crate::{CalidadResidencia, DominioError, Horario, calendario};

/// Clase de recurso reservable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoRecurso {
    BienComun,
    Parqueadero,
    /// Citas con la administración.
    AgendaAdministracion,
    /// Préstamo de herramientas o equipos.
    Herramienta,
}

enum_texto!(TipoRecurso {
    BienComun => "bien_comun",
    Parqueadero => "parqueadero",
    AgendaAdministracion => "agenda_administracion",
    Herramienta => "herramienta",
});

/// Estado operativo de un recurso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoRecurso {
    Disponible,
    EnMantenimiento,
    FueraDeServicio,
    Inactivo,
}

enum_texto!(EstadoRecurso {
    Disponible => "disponible",
    EnMantenimiento => "en_mantenimiento",
    FueraDeServicio => "fuera_de_servicio",
    Inactivo => "inactivo",
});

/// Categoría de bien común.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoriaBienComun {
    SalonSocial,
    SalonComunal,
    Bbq,
    Piscina,
    Gimnasio,
    Cancha,
    Sauna,
    Turco,
    Jacuzzi,
    ParqueInfantil,
    Coworking,
    SalaJuegos,
    Terraza,
    Otro,
}

enum_texto!(CategoriaBienComun {
    SalonSocial => "salon_social",
    SalonComunal => "salon_comunal",
    Bbq => "bbq",
    Piscina => "piscina",
    Gimnasio => "gimnasio",
    Cancha => "cancha",
    Sauna => "sauna",
    Turco => "turco",
    Jacuzzi => "jacuzzi",
    ParqueInfantil => "parque_infantil",
    Coworking => "coworking",
    SalaJuegos => "sala_juegos",
    Terraza => "terraza",
    Otro => "otro",
});

/// Zona horaria IANA de la copropiedad (`America/Bogota`).
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si no es una zona conocida.
pub fn zona_horaria(nombre: &str) -> Result<Tz, DominioError> {
    nombre
        .trim()
        .parse()
        .map_err(|_| DominioError::DatoInvalido {
            campo: "zona_horaria",
            regla: "debe ser una zona IANA, p. ej. America/Bogota",
        })
}

/// Intervalo `[inicio, fin)` en UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Franja {
    inicio: DateTime<Utc>,
    fin: DateTime<Utc>,
}

impl Franja {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si `fin` no es posterior a `inicio`.
    pub fn new(inicio: DateTime<Utc>, fin: DateTime<Utc>) -> Result<Self, DominioError> {
        if fin <= inicio {
            return Err(DominioError::DatoInvalido {
                campo: "franja",
                regla: "el fin debe ser posterior al inicio",
            });
        }
        Ok(Self { inicio, fin })
    }

    pub const fn inicio(&self) -> DateTime<Utc> {
        self.inicio
    }

    pub const fn fin(&self) -> DateTime<Utc> {
        self.fin
    }

    pub fn duracion(&self) -> Duration {
        self.fin - self.inicio
    }

    pub fn se_solapa(&self, otra: &Self) -> bool {
        self.inicio < otra.fin && otra.inicio < self.fin
    }

    /// La misma franja ampliada `margen` por cada lado (alistamiento).
    fn ampliada(&self, margen: Duration) -> Self {
        Self {
            inicio: self.inicio - margen,
            fin: self.fin + margen,
        }
    }
}

/// Cómo se asigna un recurso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModoAsignacion {
    /// Sin reserva: solo horario y aforo.
    Libre,
    /// Se confirma al solicitarla.
    FranjaInmediata,
    /// La aprueba la administración.
    FranjaConAprobacion,
    /// Alta demanda: se adjudica por sorteo.
    Sorteo,
    /// Por turnos que define la administración.
    TurnoRotativo,
}

enum_texto!(ModoAsignacion {
    Libre => "libre",
    FranjaInmediata => "franja_inmediata",
    FranjaConAprobacion => "franja_con_aprobacion",
    Sorteo => "sorteo",
    TurnoRotativo => "turno_rotativo",
});

/// Quién puede reservar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuienReserva {
    Propietario,
    Residente,
    Ambos,
}

enum_texto!(QuienReserva {
    Propietario => "propietario",
    Residente => "residente",
    Ambos => "ambos",
});

/// Base del cobro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CobroPor {
    Reserva,
    Hora,
}

enum_texto!(CobroPor {
    Reserva => "reserva",
    Hora => "hora",
});

/// Cómo se entrega el recurso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetodoEntrega {
    SinEntrega,
    EntregaLlaves,
    /// Checklist de entrega y recibo con fotos.
    EntregaConInventario,
    /// Credencial temporal (plan 05).
    AccesoAutomatico,
}

enum_texto!(MetodoEntrega {
    SinEntrega => "sin_entrega",
    EntregaLlaves => "entrega_llaves",
    EntregaConInventario => "entrega_con_inventario",
    AccesoAutomatico => "acceso_automatico",
});

/// Vínculo del solicitante con el bien privado por el que reserva.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VinculoSolicitante {
    pub es_titular: bool,
    /// Residencia vigente: calidad y si el propietario autorizó sus reservas.
    pub residencia: Option<(CalidadResidencia, bool)>,
}

/// Política de reserva de un recurso. Solo se construye válida ([`PoliticaReserva::validar`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoliticaReserva {
    pub modo: ModoAsignacion,
    pub duracion_minima_min: u32,
    pub duracion_maxima_min: u32,
    /// Las franjas empiezan y duran múltiplos de este bloque.
    pub granularidad_min: u32,
    pub anticipacion_minima_horas: u32,
    pub anticipacion_maxima_dias: u32,
    /// Reservas por bien privado en el mes calendario (`None` = sin límite).
    pub max_reservas_mes: Option<u32>,
    /// Reservas futuras activas a la vez por bien privado.
    pub max_simultaneas: Option<u32>,
    /// Tiempo libre exigido antes y después de cada reserva.
    pub alistamiento_min: u32,
    pub quien: QuienReserva,
    /// Los tenedores (arrendatarios) necesitan que el propietario autorice sus reservas.
    pub tenedor_requiere_autorizacion: bool,
    /// Cupo de personas por franja; `None` = uso exclusivo (una reserva a la vez).
    pub aforo: Option<u32>,
    pub tarifa: Decimal,
    pub cobro_por: CobroPor,
    /// Tarifa de domingos y festivos (si es distinta).
    pub tarifa_festivo: Option<Decimal>,
    pub deposito: Decimal,
    pub cancelacion_sin_penalidad_horas: u32,
    /// Porcentaje del valor que se cobra si se cancela tarde.
    pub penalidad_porcentaje: Decimal,
    pub max_invitados: u32,
    pub metodo_entrega: MetodoEntrega,
}

impl Default for PoliticaReserva {
    /// Franjas de 1 a 4 horas con aprobación, hasta 60 días antes, sin costo.
    fn default() -> Self {
        Self {
            modo: ModoAsignacion::FranjaConAprobacion,
            duracion_minima_min: 60,
            duracion_maxima_min: 240,
            granularidad_min: 60,
            anticipacion_minima_horas: 0,
            anticipacion_maxima_dias: 60,
            max_reservas_mes: None,
            max_simultaneas: None,
            alistamiento_min: 0,
            quien: QuienReserva::Ambos,
            tenedor_requiere_autorizacion: false,
            aforo: None,
            tarifa: Decimal::ZERO,
            cobro_por: CobroPor::Reserva,
            tarifa_festivo: None,
            deposito: Decimal::ZERO,
            cancelacion_sin_penalidad_horas: 0,
            penalidad_porcentaje: Decimal::ZERO,
            max_invitados: 0,
            metodo_entrega: MetodoEntrega::SinEntrega,
        }
    }
}

fn no_permitida(motivo: &'static str) -> DominioError {
    DominioError::ReservaNoPermitida(motivo)
}

fn redondear(v: Decimal) -> Decimal {
    v.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

impl PoliticaReserva {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si la política es incoherente.
    pub fn validar(self) -> Result<Self, DominioError> {
        let invalida = |regla| DominioError::DatoInvalido {
            campo: "politica",
            regla,
        };
        if self.granularidad_min == 0 || self.duracion_minima_min == 0 {
            return Err(invalida(
                "la granularidad y la duración mínima deben ser positivas",
            ));
        }
        if self.duracion_minima_min > self.duracion_maxima_min {
            return Err(invalida("la duración mínima supera la máxima"));
        }
        if self.anticipacion_maxima_dias == 0 {
            return Err(invalida(
                "la anticipación máxima debe ser de al menos un día",
            ));
        }
        if self.aforo == Some(0) {
            return Err(invalida("el aforo debe ser positivo"));
        }
        let montos = [Some(self.tarifa), self.tarifa_festivo, Some(self.deposito)];
        if montos.iter().flatten().any(|m| m.is_sign_negative()) {
            return Err(invalida(
                "las tarifas y el depósito no pueden ser negativos",
            ));
        }
        if self.penalidad_porcentaje.is_sign_negative()
            || self.penalidad_porcentaje > Decimal::ONE_HUNDRED
        {
            return Err(invalida("la penalidad va de 0 a 100 %"));
        }
        Ok(self)
    }

    /// ¿Se reserva por franjas (y no por sorteo, turnos o libre acceso)?
    pub const fn se_reserva_por_franjas(&self) -> bool {
        matches!(
            self.modo,
            ModoAsignacion::FranjaInmediata | ModoAsignacion::FranjaConAprobacion
        )
    }

    /// Valida la franja contra la política y el horario (en la hora local `tz`).
    ///
    /// # Errors
    ///
    /// [`DominioError::ReservaNoPermitida`] con la regla que incumple.
    pub fn validar_franja(
        &self,
        franja: &Franja,
        ahora: DateTime<Utc>,
        horario: &Horario,
        tz: Tz,
    ) -> Result<(), DominioError> {
        if !self.se_reserva_por_franjas() {
            return Err(no_permitida(
                "este recurso no se reserva por franjas (libre, sorteo o turnos)",
            ));
        }
        let minutos = franja.duracion().num_minutes();
        if minutos < i64::from(self.duracion_minima_min)
            || minutos > i64::from(self.duracion_maxima_min)
        {
            return Err(no_permitida("la duración está fuera de lo permitido"));
        }
        let inicio = franja.inicio.with_timezone(&tz);
        let fin = franja.fin.with_timezone(&tz);
        let bloque = i64::from(self.granularidad_min);
        let minuto_del_dia = i64::from(inicio.time().num_seconds_from_midnight() / 60);
        if minutos % bloque != 0 || minuto_del_dia % bloque != 0 || inicio.time().second() != 0 {
            return Err(no_permitida(
                "la franja debe empezar y durar en bloques de la granularidad",
            ));
        }
        if franja.inicio < ahora + Duration::hours(i64::from(self.anticipacion_minima_horas)) {
            return Err(no_permitida("no cumple la anticipación mínima"));
        }
        if franja.inicio > ahora + Duration::days(i64::from(self.anticipacion_maxima_dias)) {
            return Err(no_permitida("supera la anticipación máxima"));
        }
        let dentro = inicio.date_naive() == fin.date_naive()
            && horario
                .franjas_en(inicio.date_naive())
                .iter()
                .any(|(i, f)| *i <= inicio.time() && fin.time() <= *f);
        if !dentro {
            return Err(no_permitida("la franja está fuera del horario del recurso"));
        }
        Ok(())
    }

    /// Valida los límites por bien privado.
    ///
    /// # Errors
    ///
    /// [`DominioError::ReservaNoPermitida`] si supera un límite.
    pub fn validar_limites(
        &self,
        reservas_del_mes: u32,
        simultaneas: u32,
        invitados: u32,
    ) -> Result<(), DominioError> {
        if self.max_reservas_mes.is_some_and(|m| reservas_del_mes >= m) {
            return Err(no_permitida("el bien privado alcanzó las reservas del mes"));
        }
        if self.max_simultaneas.is_some_and(|m| simultaneas >= m) {
            return Err(no_permitida(
                "el bien privado tiene demasiadas reservas activas",
            ));
        }
        if invitados > self.max_invitados {
            return Err(no_permitida("supera el máximo de invitados"));
        }
        Ok(())
    }

    /// ¿Puede reservar quien tiene este vínculo con el bien privado?
    ///
    /// # Errors
    ///
    /// [`DominioError::ReservaNoPermitida`] si la política no lo admite.
    pub fn admite(&self, v: &VinculoSolicitante) -> Result<(), DominioError> {
        let residente = match v.residencia {
            Some((CalidadResidencia::Tenedor, autorizado)) => {
                if self.tenedor_requiere_autorizacion && !autorizado {
                    return Err(no_permitida(
                        "el tenedor necesita la autorización del propietario para reservar",
                    ));
                }
                true
            }
            Some((calidad, _)) => calidad.recibe_rol_residente(),
            None => false,
        };
        let admitido = match self.quien {
            QuienReserva::Propietario => v.es_titular,
            QuienReserva::Residente => residente,
            QuienReserva::Ambos => v.es_titular || residente,
        };
        if admitido {
            Ok(())
        } else {
            Err(no_permitida(
                "su vínculo con el bien privado no le permite reservar",
            ))
        }
    }

    /// Valor de la reserva (COP). Domingos y festivos usan la tarifa de festivo si existe.
    pub fn liquidar(&self, franja: &Franja, tz: Tz) -> Decimal {
        let dia = franja.inicio.with_timezone(&tz).date_naive();
        let festivo = calendario::es_festivo(dia) || dia.weekday() == chrono::Weekday::Sun;
        let base = if festivo {
            self.tarifa_festivo.unwrap_or(self.tarifa)
        } else {
            self.tarifa
        };
        let valor = match self.cobro_por {
            CobroPor::Reserva => base,
            CobroPor::Hora => {
                base * Decimal::from(franja.duracion().num_minutes()) / Decimal::from(60)
            }
        };
        redondear(valor)
    }

    /// Penalidad por cancelar en `ahora` una reserva de `valor` que empieza en `inicio`.
    pub fn penalidad(
        &self,
        valor: Decimal,
        inicio: DateTime<Utc>,
        ahora: DateTime<Utc>,
    ) -> Decimal {
        let plazo = Duration::hours(i64::from(self.cancelacion_sin_penalidad_horas));
        if inicio - ahora >= plazo {
            Decimal::ZERO
        } else {
            redondear(valor * self.penalidad_porcentaje / Decimal::ONE_HUNDRED)
        }
    }
}

/// Estado de una reserva.
///
/// ```text
/// Solicitada ──▶ Aprobada ──▶ EnCurso ──▶ Finalizada | ConNovedad
///     │  └──▶ Rechazada   │  └──▶ NoAsistio
///     └──────────┬────────┘
///                ▼
///            Cancelada
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoReserva {
    Solicitada,
    Aprobada,
    Rechazada,
    EnCurso,
    Finalizada,
    ConNovedad,
    Cancelada,
    NoAsistio,
}

enum_texto!(EstadoReserva {
    Solicitada => "solicitada",
    Aprobada => "aprobada",
    Rechazada => "rechazada",
    EnCurso => "en_curso",
    Finalizada => "finalizada",
    ConNovedad => "con_novedad",
    Cancelada => "cancelada",
    NoAsistio => "no_asistio",
});

impl EstadoReserva {
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está en el diagrama.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoReserva::*;
        match (self, hacia) {
            (Solicitada, Aprobada | Rechazada | Cancelada)
            | (Aprobada, EnCurso | Cancelada | NoAsistio)
            | (EnCurso, Finalizada | ConNovedad) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "reserva",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }

    /// Ocupa la franja (cuenta para solapes, cupos y límites).
    pub const fn activa(self) -> bool {
        matches!(self, Self::Solicitada | Self::Aprobada | Self::EnCurso)
    }
}

/// Por dónde llegó la reserva.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanalReserva {
    Web,
    Whatsapp,
    Presencial,
}

enum_texto!(CanalReserva {
    Web => "web",
    Whatsapp => "whatsapp",
    Presencial => "presencial",
});

/// Estado del pago del valor de la reserva. Hoy se registra a mano; el pago en línea llegará
/// con una pasarela (decisión pendiente) o con el módulo de contabilidad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoPago {
    NoAplica,
    Pendiente,
    Pagado,
    Exonerado,
}

enum_texto!(EstadoPago {
    NoAplica => "no_aplica",
    Pendiente => "pendiente",
    Pagado => "pagado",
    Exonerado => "exonerado",
});

/// Modalidad de una cita con la administración.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModalidadCita {
    Presencial,
    Llamada,
    Videollamada,
}

enum_texto!(ModalidadCita {
    Presencial => "presencial",
    Llamada => "llamada",
    Videollamada => "videollamada",
});

/// Momento de la entrega del recurso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoEntrega {
    Entrega,
    Recibo,
}

enum_texto!(TipoEntrega {
    Entrega => "entrega",
    Recibo => "recibo",
});

/// Concepto de un cargo pendiente (para el futuro módulo de contabilidad).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConceptoCargo {
    Danos,
    PenalidadCancelacion,
}

enum_texto!(ConceptoCargo {
    Danos => "danos",
    PenalidadCancelacion => "penalidad_cancelacion",
});

/// Franjas en que las reservas activas (franja y personas) llenan el `aforo`: lo que la
/// disponibilidad debe descontar en un recurso con cupo.
pub fn franjas_llenas(reservas: &[(Franja, u32)], aforo: u32) -> Vec<Franja> {
    let mut puntos: Vec<DateTime<Utc>> = reservas
        .iter()
        .flat_map(|(f, _)| [f.inicio, f.fin])
        .collect();
    puntos.sort();
    puntos.dedup();
    let mut llenas: Vec<Franja> = Vec::new();
    for par in puntos.windows(2) {
        let tramo = Franja {
            inicio: par[0],
            fin: par[1],
        };
        let ocupacion: u32 = reservas
            .iter()
            .filter(|(f, _)| f.se_solapa(&tramo))
            .map(|(_, p)| *p)
            .sum();
        if ocupacion >= aforo {
            match llenas.last_mut() {
                Some(ultima) if ultima.fin == tramo.inicio => ultima.fin = tramo.fin,
                _ => llenas.push(tramo),
            }
        }
    }
    llenas
}

/// Máximo de días por consulta de disponibilidad.
pub const DIAS_MAXIMOS_DISPONIBILIDAD: i64 = 62;

/// Franjas libres entre `desde` y `hasta` (fechas locales, inclusive): horario del recurso
/// menos las franjas ocupadas ampliadas por el alistamiento. Para recursos con aforo se pasan
/// solo las franjas que ya llenaron el cupo.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si el rango está invertido o supera
/// [`DIAS_MAXIMOS_DISPONIBILIDAD`] días.
pub fn disponibilidad(
    horario: &Horario,
    tz: Tz,
    desde: NaiveDate,
    hasta: NaiveDate,
    ocupadas: &[Franja],
    alistamiento: Duration,
) -> Result<Vec<Franja>, DominioError> {
    let dias = (hasta - desde).num_days();
    if !(0..DIAS_MAXIMOS_DISPONIBILIDAD).contains(&dias) {
        return Err(DominioError::DatoInvalido {
            campo: "rango",
            regla: "`hasta` debe ser posterior a `desde` y el rango de máximo 62 días",
        });
    }
    let bloqueos: Vec<Franja> = ocupadas.iter().map(|f| f.ampliada(alistamiento)).collect();
    let mut libres = Vec::new();
    let mut dia = desde;
    while dia <= hasta {
        for (i, f) in horario.franjas_en(dia) {
            let (Some(inicio), Some(fin)) = (
                tz.from_local_datetime(&dia.and_time(i)).earliest(),
                tz.from_local_datetime(&dia.and_time(f)).earliest(),
            ) else {
                continue;
            };
            let mut pendientes = vec![(inicio.with_timezone(&Utc), fin.with_timezone(&Utc))];
            for b in &bloqueos {
                pendientes = pendientes
                    .into_iter()
                    .flat_map(|(a, z)| {
                        if b.fin <= a || z <= b.inicio {
                            vec![(a, z)]
                        } else {
                            [(a, b.inicio), (b.fin, z)]
                                .into_iter()
                                .filter(|(x, y)| x < y)
                                .collect()
                        }
                    })
                    .collect();
            }
            libres.extend(
                pendientes
                    .into_iter()
                    .filter_map(|(a, z)| Franja::new(a, z).ok()),
            );
        }
        let Some(siguiente) = dia.succ_opt() else {
            break;
        };
        dia = siguiente;
    }
    Ok(libres)
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveTime, Weekday};
    use rust_decimal::dec;

    use super::*;
    use crate::FranjaSemanal;

    const BOGOTA: Tz = chrono_tz::America::Bogota;

    fn local(a: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        BOGOTA
            .with_ymd_and_hms(a, m, d, h, 0, 0)
            .single()
            .expect("hora válida")
            .with_timezone(&Utc)
    }

    fn horario_semana() -> Horario {
        let dias = [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ];
        Horario::new(
            dias.iter()
                .map(|d| FranjaSemanal {
                    dia: *d,
                    inicio: NaiveTime::from_hms_opt(8, 0, 0).expect("hora"),
                    fin: NaiveTime::from_hms_opt(22, 0, 0).expect("hora"),
                    aplica_festivos: true,
                })
                .collect(),
            vec![],
        )
        .expect("válido")
    }

    fn franja(d: u32, h1: u32, h2: u32) -> Franja {
        Franja::new(local(2026, 12, d, h1), local(2026, 12, d, h2)).expect("válida")
    }

    fn ahora() -> DateTime<Utc> {
        local(2026, 11, 7, 10)
    }

    #[test]
    fn franja_valida_dentro_del_horario() {
        let p = PoliticaReserva::default();
        assert!(
            p.validar_franja(&franja(5, 14, 18), ahora(), &horario_semana(), BOGOTA)
                .is_ok()
        );
    }

    #[test]
    fn reglas_de_la_franja() {
        let p = PoliticaReserva::default();
        let h = horario_semana();
        let casos = [
            franja(5, 14, 19), // 5 h > máximo
            franja(5, 20, 23), // fuera del horario
            Franja::new(local(2027, 3, 1, 10), local(2027, 3, 1, 12)).expect("válida"), // > 60 días
            Franja::new(
                local(2026, 12, 5, 14) + Duration::minutes(30),
                local(2026, 12, 5, 16) + Duration::minutes(30),
            )
            .expect("válida"), // no empieza en bloque
        ];
        for f in casos {
            assert!(
                matches!(
                    p.validar_franja(&f, ahora(), &h, BOGOTA),
                    Err(DominioError::ReservaNoPermitida(_))
                ),
                "{f:?}"
            );
        }
        let sorteo = PoliticaReserva {
            modo: ModoAsignacion::Sorteo,
            ..PoliticaReserva::default()
        };
        assert!(
            sorteo
                .validar_franja(&franja(5, 14, 16), ahora(), &h, BOGOTA)
                .is_err()
        );
    }

    #[test]
    fn limites_por_bien_privado() {
        let p = PoliticaReserva {
            max_reservas_mes: Some(2),
            max_simultaneas: Some(1),
            max_invitados: 30,
            ..PoliticaReserva::default()
        };
        assert!(p.validar_limites(1, 0, 30).is_ok());
        assert!(p.validar_limites(2, 0, 0).is_err());
        assert!(p.validar_limites(0, 1, 0).is_err());
        assert!(p.validar_limites(0, 0, 31).is_err());
    }

    #[test]
    fn quien_puede_reservar_y_tenedores() {
        let titular = VinculoSolicitante {
            es_titular: true,
            residencia: None,
        };
        let tenedor = |autorizado| VinculoSolicitante {
            es_titular: false,
            residencia: Some((CalidadResidencia::Tenedor, autorizado)),
        };
        let solo_propietario = PoliticaReserva {
            quien: QuienReserva::Propietario,
            ..PoliticaReserva::default()
        };
        assert!(solo_propietario.admite(&titular).is_ok());
        assert!(solo_propietario.admite(&tenedor(true)).is_err());

        let con_autorizacion = PoliticaReserva {
            tenedor_requiere_autorizacion: true,
            ..PoliticaReserva::default()
        };
        assert!(con_autorizacion.admite(&tenedor(false)).is_err());
        assert!(con_autorizacion.admite(&tenedor(true)).is_ok());
        assert!(PoliticaReserva::default().admite(&tenedor(false)).is_ok());
        let empleado = VinculoSolicitante {
            es_titular: false,
            residencia: Some((CalidadResidencia::EmpleadoDomestico, false)),
        };
        assert!(PoliticaReserva::default().admite(&empleado).is_err());
    }

    #[test]
    fn liquidacion_por_hora_y_festivo() {
        let p = PoliticaReserva {
            tarifa: dec!(20000),
            tarifa_festivo: Some(dec!(30000)),
            cobro_por: CobroPor::Hora,
            ..PoliticaReserva::default()
        };
        // 5 de diciembre de 2026: sábado ordinario. 8 de diciembre: festivo (Inmaculada).
        assert_eq!(p.liquidar(&franja(5, 14, 17), BOGOTA), dec!(60000));
        assert_eq!(p.liquidar(&franja(8, 14, 16), BOGOTA), dec!(60000));
        assert_eq!(
            p.liquidar(&franja(6, 14, 15), BOGOTA),
            dec!(30000),
            "domingo"
        );
    }

    #[test]
    fn penalidad_solo_si_cancela_tarde() {
        let p = PoliticaReserva {
            cancelacion_sin_penalidad_horas: 48,
            penalidad_porcentaje: dec!(50),
            ..PoliticaReserva::default()
        };
        let inicio = local(2026, 12, 5, 14);
        assert_eq!(
            p.penalidad(dec!(100000), inicio, inicio - Duration::hours(72)),
            Decimal::ZERO
        );
        assert_eq!(
            p.penalidad(dec!(100000), inicio, inicio - Duration::hours(24)),
            dec!(50000)
        );
    }

    #[test]
    fn politicas_incoherentes() {
        for p in [
            PoliticaReserva {
                duracion_minima_min: 300,
                ..PoliticaReserva::default()
            },
            PoliticaReserva {
                aforo: Some(0),
                ..PoliticaReserva::default()
            },
            PoliticaReserva {
                penalidad_porcentaje: dec!(120),
                ..PoliticaReserva::default()
            },
            PoliticaReserva {
                tarifa: dec!(-1),
                ..PoliticaReserva::default()
            },
        ] {
            assert!(p.validar().is_err());
        }
    }

    #[test]
    fn disponibilidad_resta_ocupadas_y_alistamiento() {
        let h = horario_semana();
        let dia = NaiveDate::from_ymd_opt(2026, 12, 5).expect("fecha");
        let libres = disponibilidad(
            &h,
            BOGOTA,
            dia,
            dia,
            &[franja(5, 12, 14)],
            Duration::hours(1),
        )
        .expect("calcula");
        assert_eq!(libres, vec![franja(5, 8, 11), franja(5, 15, 22)]);
        let dos_dias = disponibilidad(
            &h,
            BOGOTA,
            dia,
            dia.succ_opt().expect("fecha"),
            &[],
            Duration::zero(),
        )
        .expect("calcula");
        assert_eq!(dos_dias.len(), 2);
        assert!(
            disponibilidad(
                &h,
                BOGOTA,
                dia,
                dia - Duration::days(1),
                &[],
                Duration::zero()
            )
            .is_err()
        );
    }

    #[test]
    fn maquina_de_estados() {
        use EstadoReserva::*;
        assert!(Solicitada.transicionar(Aprobada).is_ok());
        assert!(Aprobada.transicionar(EnCurso).is_ok());
        assert!(EnCurso.transicionar(ConNovedad).is_ok());
        for (d, h) in [
            (Finalizada, Cancelada),
            (Rechazada, Aprobada),
            (EnCurso, Cancelada),
        ] {
            assert!(d.transicionar(h).is_err(), "{d} -> {h}");
        }
        assert!(Aprobada.activa() && !Cancelada.activa());
    }

    #[test]
    fn franjas_llenas_suma_personas() {
        let r = [
            (franja(5, 8, 12), 6),
            (franja(5, 10, 14), 5),
            (franja(5, 16, 18), 10),
        ];
        assert_eq!(
            franjas_llenas(&r, 10),
            vec![franja(5, 10, 12), franja(5, 16, 18)]
        );
        assert!(franjas_llenas(&r, 20).is_empty());
    }

    #[test]
    fn zona_horaria_iana() {
        assert!(zona_horaria("America/Bogota").is_ok());
        assert!(zona_horaria("Bogota").is_err());
    }
}
