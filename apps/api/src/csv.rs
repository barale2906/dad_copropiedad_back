//! Respuestas CSV de los reportes y libros (censo, asambleas).

use axum::{
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};

/// CSV con BOM y `;` (Excel en español lo abre directo). Los campos se citan siempre.
/// `nombre` es la cabecera `Content-Disposition` completa.
pub fn csv(nombre: &'static str, encabezado: &[&str], filas: Vec<Vec<String>>) -> Response {
    let citar = |c: &str| format!("\"{}\"", c.replace('"', "\"\""));
    let mut texto = String::from("\u{feff}");
    texto.push_str(
        &encabezado
            .iter()
            .map(|c| citar(c))
            .collect::<Vec<_>>()
            .join(";"),
    );
    texto.push_str("\r\n");
    for f in filas {
        texto.push_str(&f.iter().map(|c| citar(c)).collect::<Vec<_>>().join(";"));
        texto.push_str("\r\n");
    }
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/csv; charset=utf-8"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static(nombre),
            ),
        ],
        texto,
    )
        .into_response()
}
