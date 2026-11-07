//! Calendario colombiano: días festivos y días hábiles.
//!
//! Lo usan los plazos legales y operativos: segunda convocatoria de la asamblea ("tercer día
//! hábil siguiente", Ley 675 art. 41), plazos de PQRS y de solicitudes de datos personales,
//! SLA de tickets y horarios de bienes comunes en festivos.
//!
//! Festivos nacionales (18 celebraciones):
//! - **Fijos**: 1 de enero, 1 de mayo, 20 de julio, 7 de agosto, 8 y 25 de diciembre.
//! - **Trasladables al lunes siguiente** (Ley 51 de 1983, "Ley Emiliani"): Reyes (6 ene),
//!   San José (19 mar), San Pedro y San Pablo (29 jun), Asunción (15 ago), Día de la Raza
//!   (12 oct), Todos los Santos (1 nov) e Independencia de Cartagena (11 nov).
//! - **Dependientes de la Pascua**: Jueves y Viernes Santo (no se trasladan); Ascensión,
//!   Corpus Christi y Sagrado Corazón (trasladados al lunes: Pascua + 43, + 64 y + 71 días).
//!
//! Dos celebraciones pueden caer el mismo lunes (p. ej. en 2025, San Pedro y Sagrado Corazón el
//! 30 de junio); en ese caso el día cuenta una vez. Las reglas aplican desde 1984; los festivos
//! extraordinarios que decrete el Gobierno no se incluyen.
//!
//! Día hábil: lunes a viernes que no sea festivo.

use chrono::{Datelike, Days, NaiveDate, Weekday};

/// Un día festivo con el nombre de la celebración.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Festivo {
    pub fecha: NaiveDate,
    pub nombre: &'static str,
}

/// Festivos nacionales de Colombia de un año, ordenados por fecha. Si dos celebraciones caen
/// el mismo día, aparecen ambas.
pub fn festivos(anio: i32) -> Vec<Festivo> {
    let mut lista = Vec::with_capacity(18);

    for (mes, dia, nombre) in [
        (1, 1, "Año Nuevo"),
        (5, 1, "Día del Trabajo"),
        (7, 20, "Día de la Independencia"),
        (8, 7, "Batalla de Boyacá"),
        (12, 8, "Inmaculada Concepción"),
        (12, 25, "Navidad"),
    ] {
        lista.extend(fecha(anio, mes, dia).map(|fecha| Festivo { fecha, nombre }));
    }

    for (mes, dia, nombre) in [
        (1, 6, "Día de los Reyes Magos"),
        (3, 19, "Día de San José"),
        (6, 29, "San Pedro y San Pablo"),
        (8, 15, "La Asunción de la Virgen"),
        (10, 12, "Día de la Raza"),
        (11, 1, "Día de Todos los Santos"),
        (11, 11, "Independencia de Cartagena"),
    ] {
        lista.extend(fecha(anio, mes, dia).map(|f| Festivo {
            fecha: lunes_siguiente(f),
            nombre,
        }));
    }

    if let Some(pascua) = domingo_de_pascua(anio) {
        for (desplazamiento, nombre) in [
            (-3, "Jueves Santo"),
            (-2, "Viernes Santo"),
            (43, "Ascensión del Señor"),
            (64, "Corpus Christi"),
            (71, "Sagrado Corazón de Jesús"),
        ] {
            lista.extend(desplazar(pascua, desplazamiento).map(|fecha| Festivo { fecha, nombre }));
        }
    }

    lista.sort();
    lista
}

/// ¿La fecha es festivo nacional?
pub fn es_festivo(dia: NaiveDate) -> bool {
    festivos(dia.year()).iter().any(|f| f.fecha == dia)
}

/// ¿La fecha es día hábil (lunes a viernes, no festivo)?
pub fn es_dia_habil(dia: NaiveDate) -> bool {
    !matches!(dia.weekday(), Weekday::Sat | Weekday::Sun) && !es_festivo(dia)
}

/// El `n`-ésimo día hábil **posterior** a `desde` (sin contar `desde`). Con `n = 0` devuelve
/// `desde`.
///
/// ```
/// use chrono::NaiveDate;
/// use domain::calendario::sumar_dias_habiles;
///
/// // Desde el Jueves Santo de 2026 (2 de abril), el tercer día hábil siguiente es el
/// // miércoles 8: no cuentan el Viernes Santo, el sábado ni el domingo.
/// let desde = NaiveDate::from_ymd_opt(2026, 4, 2).expect("fecha válida");
/// assert_eq!(sumar_dias_habiles(desde, 3), NaiveDate::from_ymd_opt(2026, 4, 8));
/// ```
///
/// Devuelve `None` solo si el resultado se sale del rango de fechas de `chrono`.
pub fn sumar_dias_habiles(desde: NaiveDate, n: u32) -> Option<NaiveDate> {
    let mut dia = desde;
    let mut contados = 0;
    while contados < n {
        dia = dia.succ_opt()?;
        if es_dia_habil(dia) {
            contados += 1;
        }
    }
    Some(dia)
}

/// Domingo de Pascua (calendario gregoriano, algoritmo de Meeus/Jones/Butcher).
pub fn domingo_de_pascua(anio: i32) -> Option<NaiveDate> {
    let a = anio % 19;
    let b = anio / 100;
    let c = anio % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let mes = (h + l - 7 * m + 114) / 31;
    let dia = (h + l - 7 * m + 114) % 31 + 1;
    fecha(anio, u32::try_from(mes).ok()?, u32::try_from(dia).ok()?)
}

fn fecha(anio: i32, mes: u32, dia: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(anio, mes, dia)
}

/// La misma fecha si es lunes; si no, el lunes siguiente.
fn lunes_siguiente(dia: NaiveDate) -> NaiveDate {
    let faltan = (7 - dia.weekday().num_days_from_monday()) % 7;
    dia + Days::new(u64::from(faltan))
}

fn desplazar(dia: NaiveDate, dias: i64) -> Option<NaiveDate> {
    let delta = Days::new(dias.unsigned_abs());
    if dias >= 0 {
        dia.checked_add_days(delta)
    } else {
        dia.checked_sub_days(delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(anio: i32, mes: u32, dia: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(anio, mes, dia).expect("fecha válida")
    }

    fn fechas(anio: i32) -> Vec<NaiveDate> {
        let mut v: Vec<_> = festivos(anio).into_iter().map(|f| f.fecha).collect();
        v.dedup();
        v
    }

    #[test]
    fn pascua_de_anios_conocidos() {
        assert_eq!(domingo_de_pascua(2024), Some(d(2024, 3, 31)));
        assert_eq!(domingo_de_pascua(2025), Some(d(2025, 4, 20)));
        assert_eq!(domingo_de_pascua(2026), Some(d(2026, 4, 5)));
        assert_eq!(domingo_de_pascua(2027), Some(d(2027, 3, 28)));
    }

    #[test]
    fn festivos_2026() {
        let esperados = [
            d(2026, 1, 1),
            d(2026, 1, 12),
            d(2026, 3, 23),
            d(2026, 4, 2),
            d(2026, 4, 3),
            d(2026, 5, 1),
            d(2026, 5, 18),
            d(2026, 6, 8),
            d(2026, 6, 15),
            d(2026, 6, 29),
            d(2026, 7, 20),
            d(2026, 8, 7),
            d(2026, 8, 17),
            d(2026, 10, 12),
            d(2026, 11, 2),
            d(2026, 11, 16),
            d(2026, 12, 8),
            d(2026, 12, 25),
        ];
        assert_eq!(fechas(2026), esperados);
    }

    #[test]
    fn festivos_2025_con_dos_celebraciones_el_mismo_lunes() {
        let esperados = [
            d(2025, 1, 1),
            d(2025, 1, 6),
            d(2025, 3, 24),
            d(2025, 4, 17),
            d(2025, 4, 18),
            d(2025, 5, 1),
            d(2025, 6, 2),
            d(2025, 6, 23),
            d(2025, 6, 30),
            d(2025, 7, 20),
            d(2025, 8, 7),
            d(2025, 8, 18),
            d(2025, 10, 13),
            d(2025, 11, 3),
            d(2025, 11, 17),
            d(2025, 12, 8),
            d(2025, 12, 25),
        ];
        assert_eq!(fechas(2025), esperados);
        assert_eq!(festivos(2025).len(), 18, "18 celebraciones en 17 días");
    }

    #[test]
    fn festivos_2027() {
        let esperados = [
            d(2027, 1, 1),
            d(2027, 1, 11),
            d(2027, 3, 22),
            d(2027, 3, 25),
            d(2027, 3, 26),
            d(2027, 5, 1),
            d(2027, 5, 10),
            d(2027, 5, 31),
            d(2027, 6, 7),
            d(2027, 7, 5),
            d(2027, 7, 20),
            d(2027, 8, 7),
            d(2027, 8, 16),
            d(2027, 10, 18),
            d(2027, 11, 1),
            d(2027, 11, 15),
            d(2027, 12, 8),
            d(2027, 12, 25),
        ];
        assert_eq!(fechas(2027), esperados);
    }

    #[test]
    fn festivo_que_cae_en_lunes_no_se_mueve() {
        // 12 de octubre de 2026 es lunes.
        assert!(es_festivo(d(2026, 10, 12)));
        assert!(!es_festivo(d(2026, 10, 19)));
    }

    #[test]
    fn fines_de_semana_y_festivos_no_son_habiles() {
        assert!(es_dia_habil(d(2026, 11, 6))); // viernes
        assert!(!es_dia_habil(d(2026, 11, 7))); // sábado
        assert!(!es_dia_habil(d(2026, 11, 8))); // domingo
        assert!(!es_dia_habil(d(2026, 11, 16))); // lunes festivo
    }

    #[test]
    fn suma_de_dias_habiles_salta_fin_de_semana_y_festivo() {
        // Jueves 12 nov 2026 + 3 hábiles: vie 13, (sáb, dom, lun 16 festivo), mar 17, mié 18.
        assert_eq!(
            sumar_dias_habiles(d(2026, 11, 12), 3),
            Some(d(2026, 11, 18))
        );
        assert_eq!(
            sumar_dias_habiles(d(2026, 11, 12), 0),
            Some(d(2026, 11, 12))
        );
    }
}
