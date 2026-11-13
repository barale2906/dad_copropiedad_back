//! Lectura de extractos bancarios (plan 13 fase 3): convierte las filas de una planilla (CSV o
//! XLSX, ya leída por `infra-planillas`) o un archivo de recaudo Asobancaria 2001 en
//! movimientos con signo. Es lógica pura: el adaptador solo entrega texto.
//!
//! Cada lector tiene una **versión** que se guarda con el extracto: si un banco cambia su
//! formato, se agrega una versión y los extractos viejos siguen explicando cómo se leyeron.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Versión del lector de planillas.
pub const VERSION_LECTOR_PLANILLA: &str = "planilla-1";
/// Versión del lector Asobancaria 2001.
pub const VERSION_LECTOR_ASOBANCARIA_2001: &str = "asobancaria_2001-1";

/// Separador decimal de los valores de la planilla.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeparadorDecimal {
    /// `1.234.567,89`
    #[default]
    Coma,
    /// `1,234,567.89`
    Punto,
}

/// Cómo leer la planilla de un banco: nombres de las columnas (sin importar mayúsculas ni
/// espacios a los lados) y formatos. El valor llega en una columna con signo o en dos (débitos
/// y créditos).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapeoColumnas {
    /// Fila (desde 1) con los encabezados; las anteriores se ignoran (preámbulo del banco).
    #[serde(default = "fila_uno")]
    pub fila_encabezado: usize,
    pub fecha: String,
    /// Formato de `chrono` (p. ej. `%d/%m/%Y` o `%Y-%m-%d`).
    pub formato_fecha: String,
    pub descripcion: String,
    #[serde(default)]
    pub referencia: Option<String>,
    /// Columna con signo (positivo = entra dinero).
    #[serde(default)]
    pub valor: Option<String>,
    /// Columna de salidas (sin signo).
    #[serde(default)]
    pub debito: Option<String>,
    /// Columna de entradas (sin signo).
    #[serde(default)]
    pub credito: Option<String>,
    #[serde(default)]
    pub separador_decimal: SeparadorDecimal,
}

enum_texto!(SeparadorDecimal {
    Coma => "coma",
    Punto => "punto",
});

fn fila_uno() -> usize {
    1
}

/// Movimiento leído del extracto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovimientoLeido {
    pub fecha: NaiveDate,
    pub descripcion: String,
    pub referencia: Option<String>,
    /// Positivo = entra dinero.
    pub valor: Decimal,
}

/// Problema de una fila (no repite el contenido de la celda).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorLectura {
    /// Fila o línea del archivo, desde 1.
    pub fila: usize,
    pub mensaje: String,
}

/// Resultado de leer un extracto.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtractoLeido {
    pub movimientos: Vec<MovimientoLeido>,
    pub errores: Vec<ErrorLectura>,
    /// Fecha del archivo (Asobancaria) si la trae.
    pub fecha_archivo: Option<NaiveDate>,
}

impl ExtractoLeido {
    /// Primera y última fecha de los movimientos.
    pub fn rango(&self) -> Option<(NaiveDate, NaiveDate)> {
        let min = self.movimientos.iter().map(|m| m.fecha).min()?;
        let max = self.movimientos.iter().map(|m| m.fecha).max()?;
        Some((min, max))
    }
}

fn invalido(regla: &'static str) -> DominioError {
    DominioError::DatoInvalido {
        campo: "mapeo",
        regla,
    }
}

fn normalizar(t: &str) -> String {
    t.trim().to_lowercase()
}

/// Convierte un valor del banco (`$ 1.234,50`, `-20,00`, `(15,00)`) en `Decimal`.
fn leer_valor(texto: &str, sep: SeparadorDecimal) -> Option<Decimal> {
    let t = texto.trim();
    if t.is_empty() {
        return Some(Decimal::ZERO);
    }
    let (negativo, t) = match t.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        Some(x) => (true, x),
        None => (false, t),
    };
    let limpio: String = t
        .chars()
        .filter(|c| !matches!(c, '$' | ' ' | '\u{a0}'))
        .collect();
    let normal = match sep {
        SeparadorDecimal::Coma => limpio.replace('.', "").replace(',', "."),
        SeparadorDecimal::Punto => limpio.replace(',', ""),
    };
    let v: Decimal = normal.parse().ok()?;
    if v.scale() > 2 {
        return None;
    }
    Some(if negativo { -v } else { v })
}

impl MapeoColumnas {
    /// Valida el mapeo: una columna con signo **o** las dos de débito y crédito.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si falta algo o sobra.
    pub fn validar(&self) -> Result<(), DominioError> {
        if self.fila_encabezado == 0 {
            return Err(invalido("la fila de encabezados empieza en 1"));
        }
        if self.fecha.trim().is_empty() || self.descripcion.trim().is_empty() {
            return Err(invalido("indique las columnas de fecha y descripción"));
        }
        if self.formato_fecha.trim().is_empty() || !self.formato_fecha.contains('%') {
            return Err(invalido("indique el formato de la fecha (p. ej. %d/%m/%Y)"));
        }
        match (&self.valor, &self.debito, &self.credito) {
            (Some(_), None, None) | (None, Some(_), Some(_)) => Ok(()),
            _ => Err(invalido(
                "use una columna de valor con signo o las dos de débito y crédito",
            )),
        }
    }

    /// Lee las filas de la planilla (la primera de `filas` es la fila 1 del archivo).
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si el mapeo no es válido o una columna no está en los
    /// encabezados. Los problemas de cada fila van en [`ExtractoLeido::errores`].
    pub fn leer(&self, filas: &[Vec<String>]) -> Result<ExtractoLeido, DominioError> {
        self.validar()?;
        let encabezados = filas
            .get(self.fila_encabezado - 1)
            .ok_or_else(|| invalido("la planilla no tiene la fila de encabezados indicada"))?;
        let indice = |nombre: &str| -> Result<usize, DominioError> {
            encabezados
                .iter()
                .position(|e| normalizar(e) == normalizar(nombre))
                .ok_or_else(|| invalido("una columna del mapeo no está en los encabezados"))
        };
        let opcional = |n: &Option<String>| n.as_deref().map(indice).transpose();
        let (c_fecha, c_desc) = (indice(&self.fecha)?, indice(&self.descripcion)?);
        let (c_ref, c_valor) = (opcional(&self.referencia)?, opcional(&self.valor)?);
        let (c_deb, c_cre) = (opcional(&self.debito)?, opcional(&self.credito)?);

        let mut leido = ExtractoLeido::default();
        for (i, fila) in filas.iter().enumerate().skip(self.fila_encabezado) {
            let n = i + 1;
            if fila.iter().all(|c| c.trim().is_empty()) {
                continue;
            }
            let celda = |c: usize| fila.get(c).map_or("", |s| s.trim());
            let mut error = |mensaje: &str| {
                leido.errores.push(ErrorLectura {
                    fila: n,
                    mensaje: mensaje.to_owned(),
                });
            };
            let Ok(fecha) = NaiveDate::parse_from_str(celda(c_fecha), &self.formato_fecha) else {
                error("la fecha no tiene el formato indicado");
                continue;
            };
            let valor = match (c_valor, c_deb, c_cre) {
                (Some(v), _, _) => leer_valor(celda(v), self.separador_decimal),
                (None, Some(d), Some(c)) => leer_valor(celda(c), self.separador_decimal)
                    .zip(leer_valor(celda(d), self.separador_decimal))
                    .map(|(cr, db)| cr.abs() - db.abs()),
                _ => None,
            };
            let Some(valor) = valor else {
                error("el valor no es un número con a lo sumo dos decimales");
                continue;
            };
            if valor.is_zero() {
                continue;
            }
            let descripcion: String = celda(c_desc).chars().take(300).collect();
            leido.movimientos.push(MovimientoLeido {
                fecha,
                descripcion: if descripcion.is_empty() {
                    "Sin descripción".into()
                } else {
                    descripcion
                },
                referencia: c_ref
                    .map(|c| celda(c).chars().take(80).collect::<String>())
                    .filter(|r| !r.is_empty()),
                valor,
            });
        }
        Ok(leido)
    }
}

/// Campo de ancho fijo (posiciones desde 1, inclusivas); la línea corta se completa con espacios.
fn campo(linea: &str, desde: usize, hasta: usize) -> &str {
    let fin = hasta.min(linea.len());
    linea.get(desde - 1..fin).unwrap_or("").trim()
}

fn fecha_aaaammdd(t: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(t, "%Y%m%d").ok()
}

fn valor_implicito(t: &str) -> Option<Decimal> {
    if t.is_empty() || !t.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let entero: i64 = t.parse().ok()?;
    Some(Decimal::new(entero, 2))
}

/// Lee un archivo de recaudo **Asobancaria 2001** (registros de ancho fijo: `01` encabezado de
/// archivo con la fecha de recaudo, `05` encabezado de lote, `06` detalle con la referencia del
/// usuario y el valor con dos decimales implícitos, `08` control de lote y `09` control de
/// archivo). Cada `06` es un movimiento positivo (recaudo) en la fecha del `01`; el `09` debe
/// cuadrar con la suma de los detalles.
///
/// Posiciones según el instructivo de Asobancaria; antes de usarlo con un banco se verifica con
/// un archivo real suyo (decisión abierta 2 del plan 13).
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si falta el registro `01`. Los problemas de cada línea (y el
/// descuadre del control) van en [`ExtractoLeido::errores`].
pub fn leer_asobancaria_2001(texto: &str) -> Result<ExtractoLeido, DominioError> {
    let mut leido = ExtractoLeido::default();
    let mut fecha_recaudo = None;
    let mut suma = Decimal::ZERO;
    let mut control = None;
    for (i, bruta) in texto.lines().enumerate() {
        let n = i + 1;
        let linea = bruta.trim_end_matches('\r');
        if linea.trim().is_empty() {
            continue;
        }
        let mut error = |mensaje: &str| {
            leido.errores.push(ErrorLectura {
                fila: n,
                mensaje: mensaje.to_owned(),
            });
        };
        match campo(linea, 1, 2) {
            "01" => match fecha_aaaammdd(campo(linea, 13, 20)) {
                Some(f) => {
                    fecha_recaudo = Some(f);
                    leido.fecha_archivo = fecha_aaaammdd(campo(linea, 41, 48)).or(Some(f));
                }
                None => error("el encabezado no trae una fecha de recaudo válida"),
            },
            "06" => {
                let Some(fecha) = fecha_recaudo else {
                    error("hay un detalle antes del encabezado de archivo");
                    continue;
                };
                let Some(valor) = valor_implicito(campo(linea, 51, 64)).filter(|v| !v.is_zero())
                else {
                    error("el valor recaudado no es válido");
                    continue;
                };
                let referencia = campo(linea, 3, 50).trim_start_matches('0').to_owned();
                suma += valor;
                leido.movimientos.push(MovimientoLeido {
                    fecha,
                    descripcion: format!("Recaudo Asobancaria op. {}", campo(linea, 69, 74)),
                    referencia: (!referencia.is_empty()).then_some(referencia),
                    valor,
                });
            }
            "09" => match valor_implicito(campo(linea, 12, 29)) {
                Some(v) => control = Some((n, v)),
                None => error("el control de archivo no trae un valor válido"),
            },
            "05" | "08" => {}
            _ => error("tipo de registro desconocido"),
        }
    }
    if fecha_recaudo.is_none() {
        return Err(DominioError::DatoInvalido {
            campo: "archivo",
            regla: "un archivo Asobancaria empieza con el registro 01",
        });
    }
    match control {
        Some((n, v)) if v != suma => leido.errores.push(ErrorLectura {
            fila: n,
            mensaje: format!("el control suma {v} y los detalles {suma}"),
        }),
        None => leido.errores.push(ErrorLectura {
            fila: texto.lines().count().max(1),
            mensaje: "falta el registro de control 09".into(),
        }),
        _ => {}
    }
    Ok(leido)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filas(t: &[&[&str]]) -> Vec<Vec<String>> {
        t.iter()
            .map(|f| f.iter().map(|c| (*c).to_owned()).collect())
            .collect()
    }

    fn mapeo_con_signo() -> MapeoColumnas {
        MapeoColumnas {
            fila_encabezado: 2,
            fecha: "Fecha".into(),
            formato_fecha: "%d/%m/%Y".into(),
            descripcion: "Descripción".into(),
            referencia: Some("Referencia".into()),
            valor: Some("Valor".into()),
            debito: None,
            credito: None,
            separador_decimal: SeparadorDecimal::Coma,
        }
    }

    #[test]
    fn lee_una_planilla_con_preambulo_y_valor_con_signo() {
        let f = filas(&[
            &["Banco de prueba - extracto", "", "", ""],
            &[" fecha ", "DESCRIPCIÓN", "Referencia", "Valor"],
            &["05/03/2026", "Consignación", "REF-1", "$ 1.234,50"],
            &["06/03/2026", "Comisión", "", "(15,00)"],
            &["", "", "", ""],
            &["07/03/2026", "Informativo", "", "0"],
        ]);

        let leido = mapeo_con_signo().leer(&f).expect("lee");

        assert!(leido.errores.is_empty(), "{:?}", leido.errores);
        assert_eq!(leido.movimientos.len(), 2, "vacías y en cero se omiten");
        assert_eq!(
            leido.movimientos[0].valor,
            "1234.50".parse::<Decimal>().expect("d")
        );
        assert_eq!(leido.movimientos[0].referencia.as_deref(), Some("REF-1"));
        assert_eq!(
            leido.movimientos[1].valor,
            "-15".parse::<Decimal>().expect("d")
        );
        assert_eq!(
            leido.rango().map(|r| r.1),
            NaiveDate::from_ymd_opt(2026, 3, 6)
        );
    }

    #[test]
    fn lee_debitos_y_creditos_con_punto_decimal() {
        let mapeo = MapeoColumnas {
            fila_encabezado: 1,
            valor: None,
            debito: Some("Débitos".into()),
            credito: Some("Créditos".into()),
            separador_decimal: SeparadorDecimal::Punto,
            formato_fecha: "%Y-%m-%d".into(),
            referencia: None,
            ..mapeo_con_signo()
        };
        let f = filas(&[
            &["Fecha", "Descripción", "Débitos", "Créditos"],
            &["2026-03-05", "Pago", "1,000.25", ""],
            &["2026-03-06", "Abono", "", "500"],
        ]);
        let leido = mapeo.leer(&f).expect("lee");
        let valores: Vec<Decimal> = leido.movimientos.iter().map(|m| m.valor).collect();
        assert_eq!(
            valores,
            vec!["-1000.25".parse().expect("d"), "500".parse().expect("d")]
        );
    }

    #[test]
    fn reporta_filas_malas_sin_repetir_la_celda() {
        let f = filas(&[
            &["x"],
            &["Fecha", "Descripción", "Referencia", "Valor"],
            &["2026/03/05", "A", "", "10"],
            &["05/03/2026", "B", "", "10,005"],
        ]);
        let leido = mapeo_con_signo().leer(&f).expect("lee");
        assert_eq!(leido.errores.len(), 2);
        assert_eq!(leido.errores[0].fila, 3);
        assert!(!leido.errores[1].mensaje.contains("10,005"));
    }

    #[test]
    fn mapeos_invalidos() {
        let sin_valor = MapeoColumnas {
            valor: None,
            ..mapeo_con_signo()
        };
        let ambos = MapeoColumnas {
            debito: Some("D".into()),
            credito: Some("C".into()),
            ..mapeo_con_signo()
        };
        assert!(sin_valor.validar().is_err());
        assert!(ambos.validar().is_err());
        let f = filas(&[&["x"], &["Fecha", "Descripción", "Valor"]]);
        assert!(
            mapeo_con_signo().leer(&f).is_err(),
            "falta la columna Referencia"
        );
    }

    fn rellenar(mut s: String, largo: usize) -> String {
        while s.len() < largo {
            s.push(' ');
        }
        s
    }

    /// Arma un archivo Asobancaria 2001 de prueba con las posiciones del lector.
    fn asobancaria(detalles: &[(&str, i64)], control: i64) -> String {
        let mut lineas = vec![rellenar(
            format!(
                "01{:0>10}{}{:0>3}{:0>17}{}",
                "900123456", "20260305", "007", "1234", "20260306"
            ),
            162,
        )];
        lineas.push(rellenar(format!("05{:0>13}{:0>4}", "77", "1"), 162));
        for (i, (referencia, centavos)) in detalles.iter().enumerate() {
            lineas.push(rellenar(
                format!(
                    "06{:0>48}{:0>14}0102{:0>6}{:0>6}",
                    referencia,
                    centavos,
                    i + 1,
                    "1"
                ),
                162,
            ));
        }
        lineas.push(rellenar(
            format!("08{:0>9}{:0>18}{:0>4}", detalles.len(), control, "1"),
            162,
        ));
        lineas.push(format!("09{:0>9}{:0>18}", detalles.len(), control));
        lineas.join("\r\n")
    }

    #[test]
    fn lee_un_recaudo_asobancaria_2001() {
        let texto = asobancaria(&[("1001", 15_000_000), ("2002", 250_050)], 15_250_050);

        let leido = leer_asobancaria_2001(&texto).expect("lee");

        assert!(leido.errores.is_empty(), "{:?}", leido.errores);
        assert_eq!(leido.movimientos.len(), 2);
        assert_eq!(leido.movimientos[0].referencia.as_deref(), Some("1001"));
        assert_eq!(
            leido.movimientos[1].valor,
            "2500.50".parse::<Decimal>().expect("d")
        );
        assert_eq!(
            leido.movimientos[0].fecha,
            NaiveDate::from_ymd_opt(2026, 3, 5).expect("f")
        );
        assert_eq!(leido.fecha_archivo, NaiveDate::from_ymd_opt(2026, 3, 6));
    }

    #[test]
    fn el_control_del_archivo_debe_cuadrar() {
        let texto = asobancaria(&[("1001", 100)], 999);
        let leido = leer_asobancaria_2001(&texto).expect("lee");
        assert_eq!(leido.errores.len(), 1);
        assert!(leido.errores[0].mensaje.contains("control"));
    }

    #[test]
    fn sin_encabezado_no_es_asobancaria() {
        assert!(leer_asobancaria_2001("06000").is_err());
        let leido =
            leer_asobancaria_2001(&asobancaria(&[], 0).replace("\r\n09", "\r\nZZ")).expect("lee");
        assert_eq!(leido.errores.len(), 2, "registro desconocido y falta el 09");
    }
}
