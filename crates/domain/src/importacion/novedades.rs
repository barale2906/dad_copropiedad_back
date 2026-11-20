//! Planilla de novedades de nómina (plan 16): una fila por novedad de un empleado. Sin IO; el
//! empleado (por su código), su contrato en la fecha y los periodos aprobados los verifica el caso
//! de uso.
//!
//! | Columna | Obligatoria | Ejemplo |
//! |---|---|---|
//! | `codigo` | sí (código del empleado) | `E-001` |
//! | `tipo` | sí (tipo de novedad) | `hora_extra_diurna` |
//! | `desde` | sí (fecha, o primer día) | `2026-03-04` |
//! | `hasta` | en las de días | `2026-03-06` |
//! | `cantidad` | en las de horas o valores | `4` |
//! | `descripcion` | no | `Inventario de fin de mes` |
//!
//! Fechas en `AAAA-MM-DD` o `DD/MM/AAAA`; números con punto o coma decimal. Los mensajes nunca
//! repiten el valor de la celda.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::{ErrorFila, normalizar_encabezado};
use crate::nomina::{
    CodigoEmpleado,
    liquidacion::{Novedad, TipoNovedad},
};

/// Filas de datos máximas de una planilla de novedades.
pub const MAXIMO_FILAS_NOVEDADES: usize = 5_000;

const COLUMNAS: [&str; 6] = [
    "codigo",
    "tipo",
    "desde",
    "hasta",
    "cantidad",
    "descripcion",
];

/// Encabezados de la plantilla de novedades.
pub fn encabezados_novedades() -> Vec<&'static str> {
    COLUMNAS.to_vec()
}

/// Novedad leída de una fila válida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NovedadImportada {
    /// Número de la fila en la planilla (la 1 son los encabezados).
    pub fila: u32,
    pub codigo: CodigoEmpleado,
    pub novedad: Novedad,
    pub descripcion: Option<String>,
}

/// Resultado de leer la planilla.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanillaNovedades {
    pub novedades: Vec<NovedadImportada>,
    pub errores: Vec<ErrorFila>,
    /// Filas de datos no vacías.
    pub total_filas: u32,
}

fn fecha(texto: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(texto, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(texto, "%d/%m/%Y"))
        .ok()
}

/// Lee la planilla (la primera fila son los encabezados, sin importar mayúsculas ni tildes).
pub fn leer_planilla_novedades(filas: &[Vec<String>]) -> PlanillaNovedades {
    let mut p = PlanillaNovedades::default();
    let Some((encabezado, datos)) = filas.split_first() else {
        p.errores
            .push(ErrorFila::new(1, None, "la planilla está vacía"));
        return p;
    };
    let indice: Vec<Option<usize>> = COLUMNAS
        .iter()
        .map(|c| {
            encabezado
                .iter()
                .position(|e| normalizar_encabezado(e) == *c)
        })
        .collect();
    for (c, i) in COLUMNAS.iter().zip(&indice).take(3) {
        if i.is_none() {
            p.errores
                .push(ErrorFila::new(1, Some(c), "falta la columna obligatoria"));
        }
    }
    if !p.errores.is_empty() {
        return p;
    }
    let datos: Vec<(u32, &Vec<String>)> = datos
        .iter()
        .enumerate()
        .filter(|(_, f)| f.iter().any(|c| !c.trim().is_empty()))
        .map(|(i, f)| (u32::try_from(i + 2).unwrap_or(u32::MAX), f))
        .collect();
    p.total_filas = u32::try_from(datos.len()).unwrap_or(u32::MAX);
    if datos.len() > MAXIMO_FILAS_NOVEDADES {
        p.errores.push(ErrorFila::new(
            1,
            None,
            format!("máximo {MAXIMO_FILAS_NOVEDADES} filas por archivo"),
        ));
        return p;
    }
    for (n, celdas) in datos {
        let celda = |k: usize| {
            indice[k]
                .and_then(|i| celdas.get(i))
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
        };
        let error = |col: &str, msj: &str| ErrorFila::new(n, Some(col), msj);
        let mut errores = Vec::new();
        let codigo = match celda(0).map(CodigoEmpleado::new) {
            Some(Ok(c)) => Some(c),
            Some(Err(_)) => {
                errores.push(error("codigo", "código inválido"));
                None
            }
            None => {
                errores.push(error("codigo", "falta el dato"));
                None
            }
        };
        let tipo = match celda(1).map(|v| TipoNovedad::try_from(normalizar_encabezado(v).as_str()))
        {
            Some(Ok(t)) => Some(t),
            Some(Err(_)) => {
                errores.push(error("tipo", "tipo de novedad desconocido"));
                None
            }
            None => {
                errores.push(error("tipo", "falta el dato"));
                None
            }
        };
        let desde = match celda(2).map(fecha) {
            Some(Some(d)) => Some(d),
            Some(None) => {
                errores.push(error("desde", "fecha inválida"));
                None
            }
            None => {
                errores.push(error("desde", "falta el dato"));
                None
            }
        };
        let hasta = match celda(3).map(fecha) {
            Some(Some(d)) => Some(d),
            Some(None) => {
                errores.push(error("hasta", "fecha inválida"));
                None
            }
            None => None,
        };
        let cantidad = match celda(4).map(|v| v.replace(',', ".").parse::<Decimal>()) {
            Some(Ok(c)) => Some(c),
            Some(Err(_)) => {
                errores.push(error("cantidad", "número inválido"));
                None
            }
            None => None,
        };
        let descripcion = celda(5).map(str::to_owned);
        if descripcion
            .as_ref()
            .is_some_and(|d| d.chars().count() > 300)
        {
            errores.push(error("descripcion", "la descripción supera 300 caracteres"));
        }
        match (codigo, tipo, desde) {
            (Some(codigo), Some(tipo), Some(desde)) if errores.is_empty() => {
                match Novedad::new(tipo, desde, hasta, cantidad) {
                    Ok(novedad) => p.novedades.push(NovedadImportada {
                        fila: n,
                        codigo,
                        novedad,
                        descripcion,
                    }),
                    Err(e) => p.errores.push(ErrorFila::new(n, None, e.to_string())),
                }
            }
            _ => p.errores.extend(errores),
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planilla(filas: &[&[&str]]) -> Vec<Vec<String>> {
        filas
            .iter()
            .map(|f| f.iter().map(|c| (*c).to_owned()).collect())
            .collect()
    }

    #[test]
    fn lee_novedades_de_horas_dias_y_valores() {
        let p = leer_planilla_novedades(&planilla(&[
            &["Código", "Tipo", "Desde", "Hasta", "Cantidad"],
            &["E-1", "hora_extra_diurna", "2026-03-04", "", "4,5"],
            &["E-2", "Incapacidad común", "10/03/2026", "12/03/2026", ""],
            &["E-1", "bonificacion_salarial", "2026-03-20", "", "300000"],
        ]));
        assert!(p.errores.is_empty(), "{:?}", p.errores);
        assert_eq!(p.novedades.len(), 3);
        assert_eq!(p.novedades[0].novedad.cantidad.to_string(), "4.5");
        assert_eq!(p.novedades[1].novedad.tipo, TipoNovedad::IncapacidadComun);
    }

    #[test]
    fn los_errores_van_por_fila_y_columna_sin_el_valor() {
        let p = leer_planilla_novedades(&planilla(&[
            &["codigo", "tipo", "desde", "cantidad"],
            &["E 1", "propina", "31/31/2026", "x"],
            &["E-1", "hora_extra_diurna", "2026-03-04", "30"],
        ]));
        assert!(p.novedades.is_empty());
        let columnas: Vec<_> = p.errores.iter().filter_map(|e| e.columna.clone()).collect();
        assert_eq!(columnas, vec!["codigo", "tipo", "desde", "cantidad"]);
        assert_eq!(
            p.errores.last().map(|e| e.fila),
            Some(3),
            "30 horas en un día"
        );
        assert!(p.errores.iter().all(|e| !e.mensaje.contains("propina")));
    }

    #[test]
    fn faltan_columnas_obligatorias() {
        let p = leer_planilla_novedades(&planilla(&[&["codigo", "cantidad"]]));
        assert_eq!(p.errores.len(), 2);
        assert_eq!(encabezados_novedades().len(), 6);
    }
}
