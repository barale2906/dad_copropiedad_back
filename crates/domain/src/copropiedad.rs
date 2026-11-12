//! La copropiedad (persona jurídica que nace de la propiedad horizontal, Ley 675 art. 4) como
//! tenant: tipo, forma, estado y las advertencias legales de su ficha.

use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Destinación de la propiedad horizontal (Ley 675, art. 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoCopropiedad {
    Residencial,
    Comercial,
    Mixta,
}

enum_texto!(TipoCopropiedad {
    Residencial => "residencial",
    Comercial => "comercial",
    Mixta => "mixta",
});

/// Forma física del inmueble sometido al régimen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormaCopropiedad {
    Edificio,
    Conjunto,
    Condominio,
    CentroComercial,
    Parcelacion,
    Otro,
}

enum_texto!(FormaCopropiedad {
    Edificio => "edificio",
    Conjunto => "conjunto",
    Condominio => "condominio",
    CentroComercial => "centro_comercial",
    Parcelacion => "parcelacion",
    Otro => "otro",
});

/// Estado de la copropiedad en la plataforma.
///
/// ```text
/// EnConfiguracion ──▶ Activa ◀──▶ Suspendida
///        │              │             │
///        └──────────────┴─────────────┴──▶ Retirada
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoCopropiedad {
    /// Recién dada de alta: se cargan agrupaciones, bienes privados y coeficientes.
    EnConfiguracion,
    Activa,
    /// Servicio suspendido (p. ej. falta de pago): se conserva todo.
    Suspendida,
    /// Dejó la plataforma. Estado final.
    Retirada,
}

enum_texto!(EstadoCopropiedad {
    EnConfiguracion => "en_configuracion",
    Activa => "activa",
    Suspendida => "suspendida",
    Retirada => "retirada",
});

impl EstadoCopropiedad {
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si la transición no está en el diagrama.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoCopropiedad::{Activa, EnConfiguracion, Retirada, Suspendida};
        match (self, hacia) {
            (EnConfiguracion, Activa)
            | (Activa, Suspendida)
            | (Suspendida, Activa)
            | (EnConfiguracion | Activa | Suspendida, Retirada) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "copropiedad",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }
}

/// Clase de punto de atención de la copropiedad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoPuntoAtencion {
    OficinaAdministracion,
    Porteria,
    Recepcion,
    Otro,
}

enum_texto!(TipoPuntoAtencion {
    OficinaAdministracion => "oficina_administracion",
    Porteria => "porteria",
    Recepcion => "recepcion",
    Otro => "otro",
});

/// Advertencia legal que se muestra en la ficha de la copropiedad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvertenciaFicha {
    /// Art. 56: la PH comercial o mixta debe tener revisor fiscal.
    FaltaRevisorFiscal,
    /// Art. 53: la PH comercial o mixta con más de 30 bienes privados (sin contar parqueaderos
    /// ni depósitos) debe tener consejo de administración.
    FaltaConsejoAdministracion,
    /// No hay un cuadro de coeficientes vigente: no se puede votar ni liquidar expensas.
    SinCuadroVigente,
}

enum_texto!(AdvertenciaFicha {
    FaltaRevisorFiscal => "falta_revisor_fiscal",
    FaltaConsejoAdministracion => "falta_consejo_administracion",
    SinCuadroVigente => "sin_cuadro_vigente",
});

/// Lo que se necesita saber de la copropiedad para calcular las advertencias de su ficha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstadoLegal {
    pub tipo: TipoCopropiedad,
    pub tiene_revisor_fiscal: bool,
    pub tiene_consejo: bool,
    /// Bienes privados activos sin contar parqueaderos ni depósitos.
    pub bienes_para_consejo: u64,
    pub tiene_cuadro_vigente: bool,
}

impl EstadoLegal {
    /// Umbral del art. 53 (más de 30 bienes privados).
    pub const UMBRAL_CONSEJO: u64 = 30;

    /// Advertencias en orden estable.
    pub fn advertencias(&self) -> Vec<AdvertenciaFicha> {
        let no_residencial = self.tipo != TipoCopropiedad::Residencial;
        let mut v = Vec::new();
        if no_residencial && !self.tiene_revisor_fiscal {
            v.push(AdvertenciaFicha::FaltaRevisorFiscal);
        }
        if no_residencial && self.bienes_para_consejo > Self::UMBRAL_CONSEJO && !self.tiene_consejo
        {
            v.push(AdvertenciaFicha::FaltaConsejoAdministracion);
        }
        if !self.tiene_cuadro_vigente {
            v.push(AdvertenciaFicha::SinCuadroVigente);
        }
        v
    }
}

/// Código DANE de municipio (5 dígitos, p. ej. `11001` Bogotá D.C.).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CodigoDane(String);

impl CodigoDane {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no son 5 dígitos.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let v = valor.trim();
        if v.len() == 5 && v.chars().all(|c| c.is_ascii_digit()) {
            Ok(Self(v.to_owned()))
        } else {
            Err(DominioError::DatoInvalido {
                campo: "municipio_dane",
                regla: "deben ser los 5 dígitos del código DANE del municipio",
            })
        }
    }

    /// Código DANE de cinco dígitos.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CodigoDane {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<CodigoDane> for String {
    fn from(c: CodigoDane) -> Self {
        c.0
    }
}

/// Teléfono en formato E.164 sin `+` (8 a 15 dígitos, sin empezar por 0). Dato personal cuando
/// es de una persona: `Debug` lo oculta.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Telefono(String);

impl Telefono {
    /// Acepta `+57 300 123 4567`, `573001234567`… (quita `+`, espacios, guiones y paréntesis).
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no quedan de 8 a 15 dígitos.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let digitos: String = valor
            .chars()
            .filter(|c| !matches!(c, '+' | ' ' | '-' | '(' | ')'))
            .collect();
        let valido = (8..=15).contains(&digitos.len())
            && digitos.chars().all(|c| c.is_ascii_digit())
            && !digitos.starts_with('0');
        if valido {
            Ok(Self(digitos))
        } else {
            Err(DominioError::DatoInvalido {
                campo: "telefono",
                regla: "de 8 a 15 dígitos con indicativo de país, p. ej. 573001234567",
            })
        }
    }

    /// Teléfono en E.164 sin `+`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Telefono {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Telefono(<oculto>)")
    }
}

impl TryFrom<String> for Telefono {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<Telefono> for String {
    fn from(t: Telefono) -> Self {
        t.0
    }
}

/// Valida un texto libre opcional: recorta espacios, `None` si queda vacío.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si supera `maximo` caracteres o tiene caracteres de control.
pub fn texto_opcional(
    campo: &'static str,
    valor: Option<&str>,
    maximo: usize,
) -> Result<Option<String>, DominioError> {
    let Some(v) = valor.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if v.chars().count() > maximo || v.chars().any(char::is_control) {
        return Err(DominioError::DatoInvalido {
            campo,
            regla: "texto demasiado largo o con caracteres no permitidos",
        });
    }
    Ok(Some(v.to_owned()))
}

/// Como [`texto_opcional`] pero obligatorio.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si queda vacío o no cumple [`texto_opcional`].
pub fn texto_obligatorio(
    campo: &'static str,
    valor: &str,
    maximo: usize,
) -> Result<String, DominioError> {
    texto_opcional(campo, Some(valor), maximo)?.ok_or(DominioError::DatoInvalido {
        campo,
        regla: "es obligatorio",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maquina_de_estados_de_la_copropiedad() {
        use EstadoCopropiedad::*;
        for (d, h) in [
            (EnConfiguracion, Activa),
            (Activa, Suspendida),
            (Suspendida, Activa),
            (Activa, Retirada),
            (EnConfiguracion, Retirada),
        ] {
            assert_eq!(d.transicionar(h), Ok(h), "{d} -> {h}");
        }
        for (d, h) in [
            (Retirada, Activa),
            (Activa, EnConfiguracion),
            (EnConfiguracion, Suspendida),
            (Activa, Activa),
        ] {
            assert!(d.transicionar(h).is_err(), "{d} -> {h}");
        }
    }

    fn legal(tipo: TipoCopropiedad, bienes: u64) -> EstadoLegal {
        EstadoLegal {
            tipo,
            tiene_revisor_fiscal: false,
            tiene_consejo: false,
            bienes_para_consejo: bienes,
            tiene_cuadro_vigente: true,
        }
    }

    #[test]
    fn residencial_no_exige_revisor_ni_consejo() {
        assert!(
            legal(TipoCopropiedad::Residencial, 500)
                .advertencias()
                .is_empty()
        );
    }

    #[test]
    fn comercial_exige_revisor_y_consejo_si_supera_30() {
        assert_eq!(
            legal(TipoCopropiedad::Comercial, 30).advertencias(),
            vec![AdvertenciaFicha::FaltaRevisorFiscal]
        );
        assert_eq!(
            legal(TipoCopropiedad::Mixta, 31).advertencias(),
            vec![
                AdvertenciaFicha::FaltaRevisorFiscal,
                AdvertenciaFicha::FaltaConsejoAdministracion
            ]
        );
        let completa = EstadoLegal {
            tiene_revisor_fiscal: true,
            tiene_consejo: true,
            ..legal(TipoCopropiedad::Mixta, 31)
        };
        assert!(completa.advertencias().is_empty());
    }

    #[test]
    fn sin_cuadro_vigente_se_advierte() {
        let l = EstadoLegal {
            tiene_cuadro_vigente: false,
            ..legal(TipoCopropiedad::Residencial, 1)
        };
        assert_eq!(l.advertencias(), vec![AdvertenciaFicha::SinCuadroVigente]);
    }

    #[test]
    fn codigo_dane_y_telefono() {
        assert_eq!(
            CodigoDane::new(" 11001 ").expect("válido").as_str(),
            "11001"
        );
        assert!(CodigoDane::new("1100").is_err());
        assert_eq!(
            Telefono::new("+57 300-123 4567").expect("válido").as_str(),
            "573001234567"
        );
        assert!(Telefono::new("0123456").is_err());
        assert!(Telefono::new("12ab5678").is_err());
        assert!(Telefono::new("1234567").is_err());
        assert!(!format!("{:?}", Telefono::new("573001234567").expect("válido")).contains("573"));
    }

    #[test]
    fn textos_libres() {
        assert_eq!(texto_opcional("x", Some("  "), 5), Ok(None));
        assert_eq!(texto_opcional("x", Some(" ab "), 5), Ok(Some("ab".into())));
        assert!(texto_opcional("x", Some("abcdef"), 5).is_err());
        assert!(texto_obligatorio("x", " ", 5).is_err());
    }
}
