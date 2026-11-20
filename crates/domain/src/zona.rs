//! Zona horaria IANA de cada entidad (`copropiedades.zona_horaria`) sobre la base de datos de
//! zonas de `jiff`, embebida en el binario (reproducible y sin leer el sistema).
//!
//! [`Tz`] implementa [`chrono::TimeZone`], así el resto del código sigue usando `DateTime<Tz>` y
//! `with_timezone(&tz)` mientras la migración a `jiff` avanza (plan 21). Es `Copy`: cada zona se
//! carga una vez y queda en un registro estático.

use std::{
    collections::BTreeMap,
    fmt,
    str::FromStr,
    sync::{Mutex, OnceLock},
};

use chrono::{FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeZone};
use jiff::tz::{AmbiguousOffset, TimeZone as ZonaJiff};

/// Zona horaria IANA (p. ej. `America/Bogota`).
#[derive(Clone, Copy)]
pub struct Tz {
    nombre: &'static str,
    zona: &'static ZonaJiff,
}

fn registro() -> &'static Mutex<BTreeMap<String, (&'static str, &'static ZonaJiff)>> {
    static REGISTRO: OnceLock<Mutex<BTreeMap<String, (&'static str, &'static ZonaJiff)>>> =
        OnceLock::new();
    REGISTRO.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Zona (o desplazamiento fijo) del registro, cargándola la primera vez. El conjunto de zonas es
/// finito (la base IANA), así que guardarlas para siempre es acotado.
fn cargar(nombre: &str, crear: impl FnOnce() -> Option<ZonaJiff>) -> Option<Tz> {
    let mut r = registro().lock().ok()?;
    if let Some((n, z)) = r.get(nombre) {
        return Some(Tz { nombre: n, zona: z });
    }
    let zona: &'static ZonaJiff = Box::leak(Box::new(crear()?));
    let n: &'static str = Box::leak(nombre.to_owned().into_boxed_str());
    r.insert(nombre.to_owned(), (n, zona));
    Some(Tz { nombre: n, zona })
}

impl Tz {
    /// La zona de Colombia (la de casi todas las entidades).
    pub fn bogota() -> Self {
        // La base IANA embebida siempre trae America/Bogota.
        Self::from_str("America/Bogota").unwrap_or_else(|_| Self::fija(-5 * 3600))
    }

    /// Nombre IANA (o `UTC±hh:mm` en un desplazamiento fijo).
    pub fn nombre(&self) -> &'static str {
        self.nombre
    }

    fn fija(segundos: i32) -> Self {
        let nombre = format!(
            "UTC{:+03}:{:02}",
            segundos / 3600,
            (segundos.abs() % 3600) / 60
        );
        let crear = || {
            jiff::tz::Offset::from_seconds(segundos)
                .ok()
                .map(ZonaJiff::fixed)
        };
        cargar(&nombre, crear).unwrap_or(Tz {
            nombre: "UTC",
            zona: Box::leak(Box::new(ZonaJiff::UTC)),
        })
    }

    fn desplazamiento(o: jiff::tz::Offset) -> FixedOffset {
        // Los desplazamientos de jiff (±25:59:59) caben en los de chrono (±23:59:59 salvo
        // extremos que no existen en la base IANA).
        FixedOffset::east_opt(o.seconds()).unwrap_or(CERO)
    }
}

/// Desplazamiento cero (se evalúa al compilar: si fallara, no compilaría).
const CERO: FixedOffset = match FixedOffset::east_opt(0) {
    Some(o) => o,
    None => panic!("el desplazamiento cero siempre es válido"),
};

impl FromStr for Tz {
    type Err = String;

    fn from_str(nombre: &str) -> Result<Self, Self::Err> {
        let nombre = nombre.trim();
        cargar(nombre, || jiff::tz::db().get(nombre).ok())
            .ok_or_else(|| format!("zona horaria desconocida: {nombre}"))
    }
}

impl PartialEq for Tz {
    fn eq(&self, otra: &Self) -> bool {
        self.nombre == otra.nombre
    }
}

impl Eq for Tz {}

impl fmt::Debug for Tz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.nombre)
    }
}

impl fmt::Display for Tz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.nombre)
    }
}

fn civil(n: &NaiveDateTime) -> Option<jiff::civil::DateTime> {
    use chrono::{Datelike, Timelike};
    jiff::civil::DateTime::new(
        i16::try_from(n.year()).ok()?,
        i8::try_from(n.month()).ok()?,
        i8::try_from(n.day()).ok()?,
        i8::try_from(n.hour()).ok()?,
        i8::try_from(n.minute()).ok()?,
        i8::try_from(n.second()).ok()?,
        i32::try_from(n.nanosecond().min(999_999_999)).ok()?,
    )
    .ok()
}

impl TimeZone for Tz {
    type Offset = FixedOffset;

    fn from_offset(o: &FixedOffset) -> Self {
        Self::fija(o.local_minus_utc())
    }

    fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<FixedOffset> {
        match local.and_hms_opt(0, 0, 0) {
            Some(d) => self.offset_from_local_datetime(&d),
            None => LocalResult::None,
        }
    }

    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
        let Some(dt) = civil(local) else {
            return LocalResult::None;
        };
        match self.zona.to_ambiguous_timestamp(dt).offset() {
            AmbiguousOffset::Unambiguous { offset } => {
                LocalResult::Single(Self::desplazamiento(offset))
            }
            AmbiguousOffset::Gap { .. } => LocalResult::None,
            AmbiguousOffset::Fold { before, after } => {
                LocalResult::Ambiguous(Self::desplazamiento(before), Self::desplazamiento(after))
            }
        }
    }

    fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
        utc.and_hms_opt(0, 0, 0).map_or_else(
            || Self::desplazamiento(jiff::tz::Offset::UTC),
            |d| self.offset_from_utc_datetime(&d),
        )
    }

    fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
        let ts = jiff::Timestamp::from_second(utc.and_utc().timestamp())
            .unwrap_or(jiff::Timestamp::UNIX_EPOCH);
        Self::desplazamiento(self.zona.to_offset(ts))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Timelike, Utc};

    use super::*;

    #[test]
    fn bogota_esta_cinco_horas_atras_de_utc() {
        let t = Utc
            .with_ymd_and_hms(2026, 3, 1, 17, 0, 0)
            .single()
            .expect("hora");
        let local = t.with_timezone(&Tz::bogota());
        assert_eq!(local.hour(), 12);
        assert_eq!(Tz::bogota().nombre(), "America/Bogota");
        assert_eq!(Tz::bogota(), "America/Bogota".parse().expect("zona"));
    }

    #[test]
    fn una_zona_con_cambio_de_hora_respeta_el_salto() {
        let nueva_york: Tz = "America/New_York".parse().expect("zona");
        // 8 de marzo de 2026, 02:30 no existe en Nueva York (salta de 2:00 a 3:00).
        let hueco = NaiveDate::from_ymd_opt(2026, 3, 8)
            .and_then(|d| d.and_hms_opt(2, 30, 0))
            .expect("fecha");
        assert_eq!(nueva_york.from_local_datetime(&hueco), LocalResult::None);
        let invierno = Utc
            .with_ymd_and_hms(2026, 1, 15, 17, 0, 0)
            .single()
            .expect("hora")
            .with_timezone(&nueva_york);
        assert_eq!(invierno.hour(), 12);
    }

    #[test]
    fn un_nombre_que_no_es_iana_se_rechaza() {
        assert!("Bogota".parse::<Tz>().is_err());
    }
}
