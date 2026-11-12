//! Tabla legible del [`Informe`].

use std::fmt::Write as _;

use crate::escenario::Informe;

/// Errores por cada mil peticiones del total.
pub fn errores_por_mil(informe: &Informe) -> u64 {
    informe
        .resumenes
        .iter()
        .find(|r| r.operacion == "total")
        .map_or(0, |t| t.errores * 1000 / t.peticiones.max(1))
}

/// El informe como texto: encabezado, una fila por operación y el total.
pub fn texto(informe: &Informe) -> String {
    let segundos = |ms: u64| format!("{}.{}", ms / 1000, ms % 1000 / 100);
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{} copropiedades · {} s · votos {}/{} en {} s",
        informe.copropiedades,
        segundos(informe.duracion_ms),
        informe.votos_registrados,
        informe.votos_esperados,
        segundos(informe.votacion_ms),
    );
    let _ = writeln!(
        s,
        "{:<14} {:>9} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9}",
        "operación", "peticiones", "errores", "p50 ms", "p95 ms", "p99 ms", "máx ms", "por seg"
    );
    for r in &informe.resumenes {
        let _ = writeln!(
            s,
            "{:<14} {:>9} {:>8} {:>8} {:>8} {:>8} {:>8} {:>7}.{}",
            r.operacion,
            r.peticiones,
            r.errores,
            r.p50_ms,
            r.p95_ms,
            r.p99_ms,
            r.max_ms,
            r.decimas_por_segundo / 10,
            r.decimas_por_segundo % 10,
        );
        if !r.codigos_error.is_empty() {
            let codigos: Vec<String> = r
                .codigos_error
                .iter()
                .map(|(c, n)| {
                    format!(
                        "{}: {n}",
                        if *c == 0 {
                            "sin respuesta".into()
                        } else {
                            c.to_string()
                        }
                    )
                })
                .collect();
            let _ = writeln!(s, "{:<14} └ {}", "", codigos.join(", "));
        }
    }
    s
}
