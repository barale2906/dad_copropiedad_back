//! Resumen de las muestras por operación: peticiones, errores, latencias (p50, p95, p99,
//! máxima) y peticiones por segundo. Aritmética entera: los percentiles son del método del
//! rango más cercano.

use std::{collections::BTreeMap, time::Duration};

use serde::Serialize;

use crate::cliente::Muestra;

/// Resumen de una operación (o del total).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resumen {
    /// Operación (`total` para todas juntas).
    pub operacion: String,
    /// Peticiones hechas.
    pub peticiones: u64,
    /// Respuestas que no fueron 2xx (incluye las que no respondieron).
    pub errores: u64,
    /// Códigos de error con su cantidad (`0` = sin respuesta).
    pub codigos_error: BTreeMap<u16, u64>,
    /// Percentil 50 en milisegundos.
    pub p50_ms: u64,
    /// Percentil 95 en milisegundos.
    pub p95_ms: u64,
    /// Percentil 99 en milisegundos.
    pub p99_ms: u64,
    /// Máxima en milisegundos.
    pub max_ms: u64,
    /// Peticiones por segundo, en décimas (`125` = 12,5 por segundo).
    pub decimas_por_segundo: u64,
}

/// Percentil `por_mil` (p. ej. 950 = p95) de latencias **ordenadas**: el menor valor que deja a
/// su izquierda al menos esa fracción. `0` si no hay valores.
pub fn percentil(ordenadas: &[u64], por_mil: u64) -> u64 {
    if ordenadas.is_empty() {
        return 0;
    }
    let n = ordenadas.len() as u64;
    // Rango más cercano: ceil(n · p / 1000), en base 1.
    let rango = (n * por_mil).div_ceil(1000).max(1);
    let indice = usize::try_from(rango - 1).unwrap_or(usize::MAX);
    ordenadas[indice.min(ordenadas.len() - 1)]
}

fn resumir_grupo(operacion: &str, muestras: &[&Muestra], duracion: Duration) -> Resumen {
    let mut micros: Vec<u64> = muestras.iter().map(|m| m.micros).collect();
    micros.sort_unstable();
    let mut codigos_error = BTreeMap::new();
    for m in muestras.iter().filter(|m| !m.exitosa()) {
        *codigos_error.entry(m.estado).or_insert(0) += 1;
    }
    let ms = |por_mil| percentil(&micros, por_mil) / 1000;
    let peticiones = muestras.len() as u64;
    let duracion_ms = u64::try_from(duracion.as_millis())
        .unwrap_or(u64::MAX)
        .max(1);
    Resumen {
        operacion: operacion.to_owned(),
        peticiones,
        errores: codigos_error.values().sum(),
        codigos_error,
        p50_ms: ms(500),
        p95_ms: ms(950),
        p99_ms: ms(990),
        max_ms: micros.last().copied().unwrap_or(0) / 1000,
        decimas_por_segundo: peticiones * 10_000 / duracion_ms,
    }
}

/// Un resumen por operación (en orden alfabético) más el `total`.
pub fn resumir(muestras: &[Muestra], duracion: Duration) -> Vec<Resumen> {
    let mut grupos: BTreeMap<&str, Vec<&Muestra>> = BTreeMap::new();
    for m in muestras {
        grupos.entry(m.operacion).or_default().push(m);
    }
    let mut resumenes: Vec<Resumen> = grupos
        .iter()
        .map(|(op, ms)| resumir_grupo(op, ms, duracion))
        .collect();
    let todas: Vec<&Muestra> = muestras.iter().collect();
    resumenes.push(resumir_grupo("total", &todas, duracion));
    resumenes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn muestra(operacion: &'static str, ms: u64, estado: u16) -> Muestra {
        Muestra {
            operacion,
            micros: ms * 1000,
            estado,
        }
    }

    #[test]
    fn percentil_por_rango_mas_cercano() {
        let v: Vec<u64> = (1..=100).collect();
        assert_eq!(percentil(&v, 500), 50);
        assert_eq!(percentil(&v, 950), 95);
        assert_eq!(percentil(&v, 990), 99);
        assert_eq!(percentil(&v, 1000), 100);
        assert_eq!(percentil(&[7], 990), 7);
        assert_eq!(percentil(&[], 500), 0);
    }

    #[test]
    fn resume_por_operacion_cuenta_errores_y_agrega_el_total() {
        let muestras = [
            muestra("votar", 10, 201),
            muestra("votar", 30, 409),
            muestra("tablero", 5, 200),
            muestra("tablero", 7, 0),
        ];

        let r = resumir(&muestras, Duration::from_secs(2));

        let por_op: BTreeMap<&str, &Resumen> =
            r.iter().map(|x| (x.operacion.as_str(), x)).collect();
        assert_eq!(por_op["votar"].errores, 1);
        assert_eq!(por_op["votar"].max_ms, 30);
        assert_eq!(por_op["tablero"].codigos_error, BTreeMap::from([(0, 1)]));
        assert_eq!(por_op["total"].peticiones, 4);
        assert_eq!(por_op["total"].errores, 2);
        // 4 peticiones en 2 s = 2,0 por segundo.
        assert_eq!(por_op["total"].decimas_por_segundo, 20);
        assert_eq!(r.last().map(|x| x.operacion.as_str()), Some("total"));
    }
}
