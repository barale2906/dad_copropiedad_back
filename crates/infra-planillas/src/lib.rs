//! Adaptador de planillas: implementa [`LectorPlanillas`] para importaciones masivas.
//!
//! - **CSV** (`csv`): separador `;` o `,` (el que más aparezca en la primera línea; Excel en
//!   español guarda con `;`), UTF-8 con o sin BOM, comillas estándar.
//! - **XLSX** (`calamine`, Rust puro): primera hoja del libro. Los números se convierten a texto
//!   con su representación más corta (`0.00523`, `506`), así el dominio los valida igual que si
//!   vinieran de un CSV.
//!
//! Solo lectura y sin IO: recibe los bytes ya validados por tipo (`TipoArchivo::validar_planilla`).
//! Los errores no repiten el contenido del archivo (puede tener datos personales).

use std::io::Cursor;

use application::{AppError, AppResult, puertos::LectorPlanillas};
use calamine::{Data, Reader, Xlsx};
use domain::TipoArchivo;

/// Lector de CSV y XLSX.
#[derive(Debug, Clone, Copy, Default)]
pub struct LectorCsvXlsx;

fn ilegible() -> AppError {
    AppError::Validacion("no se pudo leer la planilla: guárdela como CSV o XLSX".into())
}

fn separador(texto: &str) -> u8 {
    let primera = texto.lines().next().unwrap_or_default();
    if primera.matches(';').count() >= primera.matches(',').count() && primera.contains(';') {
        b';'
    } else {
        b','
    }
}

fn leer_csv(contenido: &[u8]) -> AppResult<Vec<Vec<String>>> {
    let texto = std::str::from_utf8(contenido).map_err(|_| ilegible())?;
    let texto = texto.trim_start_matches('\u{feff}');
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(separador(texto))
        .from_reader(texto.as_bytes())
        .records()
        .map(|r| {
            r.map(|fila| fila.iter().map(str::to_owned).collect())
                .map_err(|_| ilegible())
        })
        .collect()
}

fn celda(d: &Data) -> String {
    match d {
        Data::Empty | Data::Error(_) => String::new(),
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => f.to_string(),
        Data::Bool(b) => b.to_string(),
        otro => otro.to_string(),
    }
}

fn leer_xlsx(contenido: &[u8]) -> AppResult<Vec<Vec<String>>> {
    let mut libro: Xlsx<_> =
        calamine::open_workbook_from_rs(Cursor::new(contenido)).map_err(|_| ilegible())?;
    let hoja = libro
        .worksheet_range_at(0)
        .ok_or_else(ilegible)?
        .map_err(|_| ilegible())?;
    Ok(hoja
        .rows()
        .map(|fila| fila.iter().map(celda).collect())
        .collect())
}

impl LectorPlanillas for LectorCsvXlsx {
    fn leer(&self, tipo: TipoArchivo, contenido: &[u8]) -> AppResult<Vec<Vec<String>>> {
        match tipo {
            TipoArchivo::Csv => leer_csv(contenido),
            TipoArchivo::Xlsx => leer_xlsx(contenido),
            _ => Err(ilegible()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_con_punto_y_coma_bom_y_comillas() {
        let texto = "\u{feff}numero;coeficiente;propietario\n506;0,5230 %;\"Pérez; Ana\"\n";
        let filas = LectorCsvXlsx
            .leer(TipoArchivo::Csv, texto.as_bytes())
            .expect("legible");
        assert_eq!(filas[0], ["numero", "coeficiente", "propietario"]);
        assert_eq!(filas[1], ["506", "0,5230 %", "Pérez; Ana"]);
    }

    #[test]
    fn csv_con_comas_y_filas_de_distinto_largo() {
        let filas = LectorCsvXlsx
            .leer(TipoArchivo::Csv, b"numero,tipo\n101\n102,casa,extra\n")
            .expect("legible");
        assert_eq!(filas.len(), 3);
        assert_eq!(filas[1], ["101"]);
    }

    #[test]
    fn xlsx_real_de_la_plantilla() {
        let contenido = include_bytes!("../tests/fixtures/bienes_privados.xlsx");
        let filas = LectorCsvXlsx
            .leer(TipoArchivo::Xlsx, contenido)
            .expect("legible");
        assert_eq!(filas[0][0], "agrupacion");
        assert_eq!(filas[1][1], "506");
        assert_eq!(filas[1][3], "0.00523");
    }

    #[test]
    fn archivo_corrupto_es_error_de_validacion() {
        assert!(matches!(
            LectorCsvXlsx.leer(TipoArchivo::Xlsx, b"PK\x03\x04xl/ roto"),
            Err(AppError::Validacion(_))
        ));
        assert!(LectorCsvXlsx.leer(TipoArchivo::Pdf, b"%PDF").is_err());
    }
}
