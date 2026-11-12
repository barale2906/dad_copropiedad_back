//! Importación masiva desde una planilla (CSV o XLSX): validación **fila por fila** sin IO.
//!
//! El adaptador de planillas entrega filas de texto; aquí se reconocen los encabezados y cada
//! fila se convierte en tipos de dominio o en una lista de errores por columna. Los mensajes de
//! error **nunca repiten el valor de la celda** (puede ser un documento, un teléfono o un
//! correo).
//!
//! Plantilla de bienes privados (encabezados sin importar mayúsculas, tildes ni espacios):
//!
//! | Columna | Obligatoria | Ejemplo |
//! |---|---|---|
//! | `agrupacion` | no | `T3` (código corto; se crea si no existe) |
//! | `numero` | sí, salvo que venga `codigo` | `506` |
//! | `codigo` | no (se calcula con la plantilla de la PH) | `3506` |
//! | `tipo` | no (`apartamento`) | `parqueadero` |
//! | `area_m2` | no | `72,5` |
//! | `coeficiente` | no | `0,5230 %` o `0.005230` |
//! | `propietario` | no | `Ana Pérez` |
//! | `documento` | si viene `propietario` | `52123456` |
//! | `telefono` | no | `573001234567` |
//! | `correo` | no | `ana@correo.co` |
//! | `porcentaje` | no (100) | `50` |
//!
//! Varias filas del mismo bien privado registran a sus copropietarios: la primera trae los
//! datos del bien y su propietario es el representante; las siguientes solo aportan titulares.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    Coeficiente, DireccionCorreo, DominioError,
    bien_privado::{CodigoAgrupacion, CodigoBienPrivado, NumeroBienPrivado, TipoBienPrivado},
    copropiedad::Telefono,
    titularidad::Porcentaje,
};

/// Qué se importa. El texto es contrato con `importaciones.tipo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoImportacion {
    BienesPrivados,
}

enum_texto!(TipoImportacion {
    BienesPrivados => "bienes_privados",
});

/// Estado de una importación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoImportacion {
    /// Sin errores: se puede confirmar.
    Validada,
    /// Hay errores: se corrige el archivo y se sube otra vez.
    ConErrores,
    /// Ya se aplicó (estado final).
    Aplicada,
}

enum_texto!(EstadoImportacion {
    Validada => "validada",
    ConErrores => "con_errores",
    Aplicada => "aplicada",
});

/// Error de una fila de la planilla. `fila` cuenta desde 1 e incluye el encabezado (es la
/// fila que ve la persona en su hoja de cálculo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorFila {
    pub fila: u32,
    pub columna: Option<String>,
    pub mensaje: String,
}

impl ErrorFila {
    /// Error de validación de una fila (y de una columna, si aplica). El mensaje no repite el valor
    /// de la celda.
    pub fn new(fila: u32, columna: Option<&str>, mensaje: impl Into<String>) -> Self {
        Self {
            fila,
            columna: columna.map(str::to_owned),
            mensaje: mensaje.into(),
        }
    }
}

/// Columnas reconocidas de la plantilla de bienes privados.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Columna {
    Agrupacion,
    Numero,
    Codigo,
    Tipo,
    AreaM2,
    Coeficiente,
    Propietario,
    Documento,
    Telefono,
    Correo,
    Porcentaje,
}

impl Columna {
    /// Columnas de la plantilla de importación, en orden.
    pub const TODAS: [Self; 11] = [
        Self::Agrupacion,
        Self::Numero,
        Self::Codigo,
        Self::Tipo,
        Self::AreaM2,
        Self::Coeficiente,
        Self::Propietario,
        Self::Documento,
        Self::Telefono,
        Self::Correo,
        Self::Porcentaje,
    ];

    /// Nombre de la columna en la plantilla.
    pub const fn nombre(self) -> &'static str {
        match self {
            Self::Agrupacion => "agrupacion",
            Self::Numero => "numero",
            Self::Codigo => "codigo",
            Self::Tipo => "tipo",
            Self::AreaM2 => "area_m2",
            Self::Coeficiente => "coeficiente",
            Self::Propietario => "propietario",
            Self::Documento => "documento",
            Self::Telefono => "telefono",
            Self::Correo => "correo",
            Self::Porcentaje => "porcentaje",
        }
    }
}

/// Normaliza un encabezado: minúsculas, sin tildes, espacios y guiones como `_`.
fn normalizar_encabezado(texto: &str) -> String {
    texto
        .trim()
        .trim_start_matches('\u{feff}')
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            ' ' | '-' => '_',
            otro => otro,
        })
        .collect()
}

/// Posición de cada columna reconocida en la planilla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encabezados(Vec<(Columna, usize)>);

impl Encabezados {
    /// Reconoce los encabezados de la primera fila. Las columnas desconocidas se ignoran.
    ///
    /// # Errors
    ///
    /// Errores de la fila 1 si una columna se repite o faltan `numero` y `codigo` a la vez.
    pub fn detectar(fila: &[String]) -> Result<Self, Vec<ErrorFila>> {
        let mut columnas = Vec::new();
        let mut errores = Vec::new();
        for (i, celda) in fila.iter().enumerate() {
            let nombre = normalizar_encabezado(celda);
            if let Some(c) = Columna::TODAS.iter().find(|c| c.nombre() == nombre) {
                if columnas.iter().any(|(x, _)| x == c) {
                    errores.push(ErrorFila::new(1, Some(c.nombre()), "columna repetida"));
                } else {
                    columnas.push((*c, i));
                }
            }
        }
        let tiene = |c: Columna| columnas.iter().any(|(x, _)| *x == c);
        if !tiene(Columna::Numero) && !tiene(Columna::Codigo) {
            errores.push(ErrorFila::new(
                1,
                None,
                "la planilla debe tener la columna `numero` o la columna `codigo`",
            ));
        }
        if errores.is_empty() {
            Ok(Self(columnas))
        } else {
            Err(errores)
        }
    }

    fn celda<'a>(&self, fila: &'a [String], columna: Columna) -> Option<&'a str> {
        self.0
            .iter()
            .find(|(c, _)| *c == columna)
            .and_then(|(_, i)| fila.get(*i))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
    }

    /// Convierte una fila de datos (`numero_fila` como la ve la persona).
    ///
    /// # Errors
    ///
    /// Un [`ErrorFila`] por cada celda inválida.
    pub fn fila(
        &self,
        numero_fila: u32,
        celdas: &[String],
    ) -> Result<FilaBienPrivado, Vec<ErrorFila>> {
        let mut errores = Vec::new();
        let leer = |columna: Columna| self.celda(celdas, columna);
        let mut error = |columna: Columna, mensaje: &str| {
            errores.push(ErrorFila::new(numero_fila, Some(columna.nombre()), mensaje));
        };

        let agrupacion = leer(Columna::Agrupacion).and_then(|v| {
            CodigoAgrupacion::new(v)
                .map_err(|_| error(Columna::Agrupacion, "de 1 a 10 letras o dígitos, p. ej. T3"))
                .ok()
        });
        let numero = leer(Columna::Numero).and_then(|v| {
            NumeroBienPrivado::new(v)
                .map_err(|_| error(Columna::Numero, "de 1 a 20 letras, dígitos o guiones"))
                .ok()
        });
        let codigo = leer(Columna::Codigo).and_then(|v| {
            CodigoBienPrivado::new(v)
                .map_err(|_| error(Columna::Codigo, "de 1 a 40 caracteres"))
                .ok()
        });
        let tipo = match leer(Columna::Tipo) {
            None => Some(TipoBienPrivado::Apartamento),
            Some(v) => TipoBienPrivado::try_from(normalizar_encabezado(v).as_str())
                .map_err(|_| error(Columna::Tipo, "tipo de bien privado desconocido"))
                .ok(),
        };
        let area_m2 = leer(Columna::AreaM2).and_then(|v| {
            v.replace(',', ".")
                .parse::<Decimal>()
                .ok()
                .filter(|a| *a > Decimal::ZERO && a.normalize().scale() <= 2)
                .or_else(|| {
                    error(Columna::AreaM2, "área positiva con máximo 2 decimales");
                    None
                })
        });
        let coeficiente = leer(Columna::Coeficiente).and_then(|v| {
            Coeficiente::desde_texto(v)
                .map_err(|_| {
                    error(
                        Columna::Coeficiente,
                        "escriba un número como 0,5230 % o 0.005230",
                    )
                })
                .ok()
        });
        let titular = self.titular(celdas, &mut error);
        if numero.is_none()
            && codigo.is_none()
            && leer(Columna::Numero).is_none()
            && leer(Columna::Codigo).is_none()
        {
            error(
                Columna::Numero,
                "falta el número (o el código) del bien privado",
            );
        }

        if errores.is_empty() {
            Ok(FilaBienPrivado {
                fila: numero_fila,
                agrupacion,
                numero,
                codigo,
                tipo: tipo.unwrap_or(TipoBienPrivado::Apartamento),
                area_m2,
                coeficiente,
                titular,
            })
        } else {
            Err(errores)
        }
    }

    fn titular(
        &self,
        celdas: &[String],
        error: &mut impl FnMut(Columna, &str),
    ) -> Option<TitularImportado> {
        let nombre = self.celda(celdas, Columna::Propietario);
        let documento = self.celda(celdas, Columna::Documento);
        let telefono = self.celda(celdas, Columna::Telefono);
        let correo = self.celda(celdas, Columna::Correo);
        let porcentaje = self.celda(celdas, Columna::Porcentaje);
        if nombre.is_none() && documento.is_none() && telefono.is_none() && correo.is_none() {
            if porcentaje.is_some() {
                error(Columna::Porcentaje, "el porcentaje exige un propietario");
            }
            return None;
        }
        let nombre = nombre.map(str::to_owned).or_else(|| {
            error(Columna::Propietario, "falta el nombre del propietario");
            None
        });
        let documento = match documento {
            Some(d) if Documento::new(d).is_ok() => Documento::new(d).ok(),
            Some(_) => {
                error(Columna::Documento, "de 3 a 20 letras o dígitos, sin puntos");
                None
            }
            None => {
                error(Columna::Documento, "falta el documento del propietario");
                None
            }
        };
        let telefono = telefono.and_then(|t| {
            Telefono::new(t)
                .map_err(|_| {
                    error(
                        Columna::Telefono,
                        "de 8 a 15 dígitos con indicativo de país",
                    )
                })
                .ok()
        });
        let correo = correo.and_then(|c| {
            DireccionCorreo::new(c)
                .map_err(|_| error(Columna::Correo, "correo electrónico inválido"))
                .ok()
        });
        let porcentaje = match porcentaje {
            None => Some(Porcentaje::CIEN),
            Some(p) => p
                .trim_end_matches('%')
                .trim()
                .replace(',', ".")
                .parse::<Decimal>()
                .ok()
                .and_then(|d| Porcentaje::new(d).ok())
                .or_else(|| {
                    error(Columna::Porcentaje, "entre 0 y 100 con máximo 4 decimales");
                    None
                }),
        };
        Some(TitularImportado {
            nombre: nombre?,
            documento: documento?,
            telefono,
            correo,
            porcentaje: porcentaje?,
        })
    }
}

/// Documento de identidad (cédula, NIT sin DV, pasaporte): 3 a 20 letras o dígitos. Dato
/// personal: `Debug` lo oculta.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Documento(String);

impl Documento {
    /// Quita puntos, espacios y guiones de separación.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no quedan de 3 a 20 letras o dígitos.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let limpio: String = valor
            .chars()
            .filter(|c| !matches!(c, '.' | ' ' | '-'))
            .collect::<String>()
            .to_uppercase();
        if (3..=20).contains(&limpio.len()) && limpio.chars().all(|c| c.is_ascii_alphanumeric()) {
            Ok(Self(limpio))
        } else {
            Err(DominioError::DatoInvalido {
                campo: "documento",
                regla: "de 3 a 20 letras o dígitos",
            })
        }
    }

    /// Documento normalizado.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Documento {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Documento(<oculto>)")
    }
}

impl TryFrom<String> for Documento {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<Documento> for String {
    fn from(d: Documento) -> Self {
        d.0
    }
}

/// Propietario leído de una fila.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitularImportado {
    pub nombre: String,
    pub documento: Documento,
    pub telefono: Option<Telefono>,
    pub correo: Option<DireccionCorreo>,
    pub porcentaje: Porcentaje,
}

/// Fila válida de la plantilla de bienes privados.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaBienPrivado {
    pub fila: u32,
    pub agrupacion: Option<CodigoAgrupacion>,
    pub numero: Option<NumeroBienPrivado>,
    pub codigo: Option<CodigoBienPrivado>,
    pub tipo: TipoBienPrivado,
    pub area_m2: Option<Decimal>,
    pub coeficiente: Option<Coeficiente>,
    pub titular: Option<TitularImportado>,
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    fn fila(celdas: &[&str]) -> Vec<String> {
        celdas.iter().map(|c| (*c).to_owned()).collect()
    }

    fn encabezados() -> Encabezados {
        Encabezados::detectar(&fila(&[
            "\u{feff}Agrupación",
            "Número",
            "Tipo",
            "Área m2",
            "Coeficiente",
            "Propietario",
            "Documento",
            "Teléfono",
            "Correo",
            "Porcentaje",
            "Observaciones",
        ]))
        .expect("encabezados válidos")
    }

    #[test]
    fn reconoce_encabezados_sin_tildes_ni_mayusculas() {
        assert_eq!(encabezados().0.len(), 10);
        let errores =
            Encabezados::detectar(&fila(&["agrupacion", "tipo"])).expect_err("falta numero");
        assert_eq!(errores[0].fila, 1);
        let errores = Encabezados::detectar(&fila(&["numero", "Número"])).expect_err("repetida");
        assert_eq!(errores[0].mensaje, "columna repetida");
    }

    #[test]
    fn convierte_una_fila_completa() {
        let f = encabezados()
            .fila(
                2,
                &fila(&[
                    "t3",
                    "506",
                    "Apartamento",
                    "72,5",
                    "0,5230 %",
                    "Ana Pérez",
                    "52.123.456",
                    "+57 300 123 4567",
                    "Ana@Correo.co",
                    "",
                ]),
            )
            .expect("válida");
        assert_eq!(f.agrupacion.expect("agrupación").as_str(), "T3");
        assert_eq!(f.numero.expect("número").as_str(), "506");
        assert_eq!(f.area_m2, Some(dec!(72.5)));
        assert_eq!(f.coeficiente.expect("coeficiente").valor(), dec!(0.00523));
        let t = f.titular.expect("titular");
        assert_eq!(t.documento.as_str(), "52123456");
        assert_eq!(t.porcentaje, Porcentaje::CIEN);
    }

    #[test]
    fn fila_sin_propietario_es_valida() {
        let f = encabezados()
            .fila(3, &fila(&["", "P-01", "parqueadero"]))
            .expect("válida");
        assert_eq!(f.tipo, TipoBienPrivado::Parqueadero);
        assert!(f.titular.is_none() && f.agrupacion.is_none());
    }

    #[test]
    fn errores_por_columna_sin_repetir_el_valor() {
        let errores = encabezados()
            .fila(
                4,
                &fila(&[
                    "Torre 3",
                    "",
                    "castillo",
                    "-1",
                    "2",
                    "Ana",
                    "",
                    "12",
                    "no-es-correo",
                    "120",
                ]),
            )
            .expect_err("inválida");
        let columnas: Vec<_> = errores
            .iter()
            .filter_map(|e| e.columna.as_deref())
            .collect();
        for c in [
            "agrupacion",
            "tipo",
            "area_m2",
            "coeficiente",
            "documento",
            "telefono",
            "correo",
            "porcentaje",
            "numero",
        ] {
            assert!(columnas.contains(&c), "falta el error de {c}: {columnas:?}");
        }
        assert!(errores.iter().all(|e| e.fila == 4));
        assert!(!errores.iter().any(|e| e.mensaje.contains("no-es-correo")));
    }

    #[test]
    fn documento_quita_separadores_y_se_oculta() {
        assert_eq!(
            Documento::new("52.123.456").expect("válido").as_str(),
            "52123456"
        );
        assert!(Documento::new("1").is_err());
        assert!(!format!("{:?}", Documento::new("52123456").expect("válido")).contains("521"));
    }
}
