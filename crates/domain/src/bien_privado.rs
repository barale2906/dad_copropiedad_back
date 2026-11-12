//! Bienes privados (Ley 675, art. 3), agrupaciones y nomenclatura.
//!
//! - Un bien privado tiene `numero` ("506") y, opcionalmente, una agrupación ("Torre 3").
//! - Su `codigo` legible y **único por copropiedad** ("T3-506" o "3506") se calcula con la
//!   [`PlantillaNomenclatura`] de la PH, salvo que la PH escriba su código propio.
//! - Las agrupaciones forman un árbol de hasta [`PROFUNDIDAD_MAXIMA_AGRUPACIONES`] niveles
//!   (etapa → torre → interior).

use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Niveles máximos del árbol de agrupaciones.
pub const PROFUNDIDAD_MAXIMA_AGRUPACIONES: usize = 3;

/// Clase de bien privado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoBienPrivado {
    Apartamento,
    Casa,
    Local,
    Oficina,
    Consultorio,
    /// Con folio de matrícula propio. Si está englobado con el apartamento no se registra:
    /// su coeficiente ya va en el del apartamento. Si es común asignado, es un uso exclusivo.
    Parqueadero,
    Deposito,
    Bodega,
    Lote,
    Finca,
    Otro,
}

enum_texto!(TipoBienPrivado {
    Apartamento => "apartamento",
    Casa => "casa",
    Local => "local",
    Oficina => "oficina",
    Consultorio => "consultorio",
    Parqueadero => "parqueadero",
    Deposito => "deposito",
    Bodega => "bodega",
    Lote => "lote",
    Finca => "finca",
    Otro => "otro",
});

impl TipoBienPrivado {
    /// ¿Cuenta para el umbral de 30 bienes del consejo de administración (art. 53)? Los
    /// parqueaderos y depósitos no.
    pub const fn cuenta_para_consejo(self) -> bool {
        !matches!(self, Self::Parqueadero | Self::Deposito)
    }

    /// ¿Puede vincularse a otro bien privado como anexo (parqueadero o depósito con folio
    /// propio)?
    pub const fn puede_ser_anexo(self) -> bool {
        matches!(self, Self::Parqueadero | Self::Deposito | Self::Bodega)
    }
}

/// Destinación del bien privado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsoBienPrivado {
    Residencial,
    Comercial,
    Mixto,
    Industrial,
    Institucional,
    Otro,
}

enum_texto!(UsoBienPrivado {
    Residencial => "residencial",
    Comercial => "comercial",
    Mixto => "mixto",
    Industrial => "industrial",
    Institucional => "institucional",
    Otro => "otro",
});

/// Clase de agrupación. La PH elige la etiqueta que muestra ("Torre", "Bloque") en el nombre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoAgrupacion {
    /// Edificio o torre.
    Edificio,
    Etapa,
    Bloque,
    Manzana,
    Interior,
    Sector,
}

enum_texto!(TipoAgrupacion {
    Edificio => "edificio",
    Etapa => "etapa",
    Bloque => "bloque",
    Manzana => "manzana",
    Interior => "interior",
    Sector => "sector",
});

/// Bien común asignado en uso exclusivo a un bien privado (art. 22).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoUsoExclusivo {
    Parqueadero,
    Deposito,
    Terraza,
    Patio,
    Jardin,
    Cubierta,
    Otro,
}

enum_texto!(TipoUsoExclusivo {
    Parqueadero => "parqueadero",
    Deposito => "deposito",
    Terraza => "terraza",
    Patio => "patio",
    Jardin => "jardin",
    Cubierta => "cubierta",
    Otro => "otro",
});

fn alfanumerico(valor: &str, maximo: usize, guion: bool) -> bool {
    !valor.is_empty()
        && valor.len() <= maximo
        && valor
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || (guion && c == '-'))
}

/// Código corto de una agrupación para la nomenclatura: 1 a 10 letras o dígitos ("T3", "3").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CodigoAgrupacion(String);

impl CodigoAgrupacion {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no son de 1 a 10 caracteres alfanuméricos.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let v = valor.trim();
        if alfanumerico(v, 10, false) {
            Ok(Self(v.to_uppercase()))
        } else {
            Err(DominioError::DatoInvalido {
                campo: "codigo",
                regla: "de 1 a 10 letras o dígitos, p. ej. T3",
            })
        }
    }

    /// Código normalizado.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CodigoAgrupacion {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<CodigoAgrupacion> for String {
    fn from(c: CodigoAgrupacion) -> Self {
        c.0
    }
}

/// Número del bien privado dentro de su agrupación: 1 a 20 letras, dígitos o guiones ("506",
/// "P-12").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NumeroBienPrivado(String);

impl NumeroBienPrivado {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no cumple el formato.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let v = valor.trim();
        if alfanumerico(v, 20, true) {
            Ok(Self(v.to_uppercase()))
        } else {
            Err(DominioError::DatoInvalido {
                campo: "numero",
                regla: "de 1 a 20 letras, dígitos o guiones, p. ej. 506",
            })
        }
    }

    /// Número normalizado.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NumeroBienPrivado {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<NumeroBienPrivado> for String {
    fn from(n: NumeroBienPrivado) -> Self {
        n.0
    }
}

/// Código legible y único por copropiedad de un bien privado ("T3-506", "3506", "Casa 12"):
/// 1 a 40 caracteres sin caracteres de control. Se guarda tal cual (sin cambiar mayúsculas)
/// salvo los espacios de los extremos.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CodigoBienPrivado(String);

impl CodigoBienPrivado {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si queda vacío, pasa de 40 caracteres o tiene caracteres
    /// de control.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let v = valor.trim();
        if v.is_empty() || v.chars().count() > 40 || v.chars().any(char::is_control) {
            return Err(DominioError::DatoInvalido {
                campo: "codigo",
                regla: "de 1 a 40 caracteres",
            });
        }
        Ok(Self(v.to_owned()))
    }

    /// Código normalizado.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CodigoBienPrivado {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<CodigoBienPrivado> for String {
    fn from(c: CodigoBienPrivado) -> Self {
        c.0
    }
}

/// Plantilla con que la PH arma el código de sus bienes privados.
///
/// Marcadores:
/// - `{numero}` (obligatorio): el número del bien ("506").
/// - `{agrupacion}`: código de la agrupación directa ("T3"); vacío si no tiene.
/// - `{ruta}`: códigos de toda la cadena de agrupaciones, de la raíz a la hoja, separados por
///   `-` ("E2-T3").
///
/// El resto del texto se copia tal cual (letras, dígitos, espacio, `-`, `_`, `.`, `/`). Si el
/// bien no tiene agrupación, los separadores que quedarían colgando se quitan.
///
/// ```
/// use domain::bien_privado::{CodigoAgrupacion, NumeroBienPrivado, PlantillaNomenclatura};
///
/// let numero = NumeroBienPrivado::new("506").expect("válido");
/// let torre = [CodigoAgrupacion::new("3").expect("válido")];
///
/// let pegado = PlantillaNomenclatura::new("{agrupacion}{numero}").expect("válida");
/// assert_eq!(pegado.codigo(&torre, &numero).expect("válido").as_str(), "3506");
///
/// let con_guion = PlantillaNomenclatura::new("T{agrupacion}-{numero}").expect("válida");
/// assert_eq!(con_guion.codigo(&torre, &numero).expect("válido").as_str(), "T3-506");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PlantillaNomenclatura(String);

impl PlantillaNomenclatura {
    /// Plantilla por defecto de toda PH nueva.
    pub const POR_DEFECTO: &'static str = "{agrupacion}-{numero}";
    const MARCADORES: [&'static str; 3] = ["{numero}", "{agrupacion}", "{ruta}"];

    /// # Errors
    ///
    /// [`DominioError::PlantillaInvalida`] si no tiene `{numero}`, tiene llaves sueltas o
    /// marcadores desconocidos, caracteres no permitidos o más de 60 caracteres.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let v = valor.trim();
        if v.chars().count() > 60 {
            return Err(DominioError::PlantillaInvalida("máximo 60 caracteres"));
        }
        if v.matches("{numero}").count() != 1 {
            return Err(DominioError::PlantillaInvalida(
                "debe contener {numero} exactamente una vez",
            ));
        }
        let mut resto = v.to_owned();
        for m in Self::MARCADORES {
            resto = resto.replace(m, "");
        }
        if resto
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '/')))
        {
            return Err(DominioError::PlantillaInvalida(
                "solo {numero}, {agrupacion}, {ruta}, letras, dígitos, espacio y - _ . /",
            ));
        }
        Ok(Self(v.to_owned()))
    }

    /// Plantilla tal como se configuró.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Código del bien privado con número `numero` en la cadena de agrupaciones `ruta` (de la
    /// raíz a la agrupación directa; vacía si no tiene).
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si el resultado no es un [`CodigoBienPrivado`] válido.
    pub fn codigo(
        &self,
        ruta: &[CodigoAgrupacion],
        numero: &NumeroBienPrivado,
    ) -> Result<CodigoBienPrivado, DominioError> {
        let directa = ruta.last().map_or("", CodigoAgrupacion::as_str);
        let completa = ruta
            .iter()
            .map(CodigoAgrupacion::as_str)
            .collect::<Vec<_>>()
            .join("-");
        let texto = self
            .0
            .replace("{agrupacion}", directa)
            .replace("{ruta}", &completa)
            .replace("{numero}", numero.as_str());
        let separadores: &[char] = &[' ', '-', '_', '.', '/'];
        CodigoBienPrivado::new(texto.trim_matches(separadores))
    }
}

impl Default for PlantillaNomenclatura {
    fn default() -> Self {
        Self(Self::POR_DEFECTO.to_owned())
    }
}

impl TryFrom<String> for PlantillaNomenclatura {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<PlantillaNomenclatura> for String {
    fn from(p: PlantillaNomenclatura) -> Self {
        p.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ruta(codigos: &[&str]) -> Vec<CodigoAgrupacion> {
        codigos
            .iter()
            .map(|c| CodigoAgrupacion::new(c).expect("válido"))
            .collect()
    }

    fn numero(n: &str) -> NumeroBienPrivado {
        NumeroBienPrivado::new(n).expect("válido")
    }

    #[test]
    fn plantilla_por_defecto_da_torre_guion_numero() {
        let p = PlantillaNomenclatura::default();
        assert_eq!(
            p.codigo(&ruta(&["t3"]), &numero("506"))
                .expect("válido")
                .as_str(),
            "T3-506"
        );
    }

    #[test]
    fn codigo_personalizado_sin_separador() {
        let p = PlantillaNomenclatura::new("{agrupacion}{numero}").expect("válida");
        assert_eq!(
            p.codigo(&ruta(&["3"]), &numero("506"))
                .expect("válido")
                .as_str(),
            "3506"
        );
    }

    #[test]
    fn ruta_completa_y_sin_agrupacion() {
        let p = PlantillaNomenclatura::new("{ruta}-{numero}").expect("válida");
        assert_eq!(
            p.codigo(&ruta(&["E2", "T3"]), &numero("101"))
                .expect("válido")
                .as_str(),
            "E2-T3-101"
        );
        // Una casa sin agrupación no queda con el guion colgando.
        assert_eq!(
            PlantillaNomenclatura::default()
                .codigo(&[], &numero("12"))
                .expect("válido")
                .as_str(),
            "12"
        );
    }

    #[test]
    fn plantillas_invalidas() {
        for p in [
            "{agrupacion}",
            "{numero}{numero}",
            "{torre}-{numero}",
            "{numero}#",
            "{numero",
        ] {
            assert!(PlantillaNomenclatura::new(p).is_err(), "{p}");
        }
        assert!(PlantillaNomenclatura::new("Casa {numero}").is_ok());
    }

    #[test]
    fn formatos_de_codigos_y_numeros() {
        assert_eq!(
            CodigoAgrupacion::new(" t3 ").expect("válido").as_str(),
            "T3"
        );
        assert!(CodigoAgrupacion::new("Torre 3").is_err());
        assert_eq!(numero("p-12").as_str(), "P-12");
        assert!(NumeroBienPrivado::new("").is_err());
        assert!(CodigoBienPrivado::new("  ").is_err());
        assert_eq!(
            CodigoBienPrivado::new(" Casa 12 ")
                .expect("válido")
                .as_str(),
            "Casa 12"
        );
    }

    #[test]
    fn parqueaderos_y_depositos_no_cuentan_para_el_consejo() {
        assert!(TipoBienPrivado::Apartamento.cuenta_para_consejo());
        assert!(!TipoBienPrivado::Parqueadero.cuenta_para_consejo());
        assert!(!TipoBienPrivado::Deposito.cuenta_para_consejo());
        assert!(TipoBienPrivado::Parqueadero.puede_ser_anexo());
        assert!(!TipoBienPrivado::Apartamento.puede_ser_anexo());
    }
}
