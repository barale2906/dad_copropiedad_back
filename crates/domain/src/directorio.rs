//! Directorio de comercios y servicios (plan 08, fase 1).
//!
//! La publicidad es un negocio **de la plataforma**: los anunciantes, sus fichas y el catálogo
//! de categorías son datos de plataforma; cada copropiedad solo ve las fichas cuya
//! [`Cobertura`] la alcanza. La ubicación de la PH (coordenadas o municipio) es lo único que se
//! usa para decidirlo: **nunca** atributos de personas.
//!
//! Reglas de este módulo:
//! - [`EstadoVerificacion::transicionar`]: ciclo de vida del anunciante. Solo los `verificado`
//!   aparecen en el directorio.
//! - [`cubre`]: ¿la ficha alcanza a la copropiedad? (radio con la fórmula de haversine,
//!   municipio o todo el país).
//! - [`validar_categorias`]: de 1 a [`MAXIMO_CATEGORIAS`] categorías, sin repetir y **nunca**
//!   una `restringida` (licores, apuestas, salud con fórmula).
//! - [`busqueda`]: búsqueda de proveedores desde el chat o la app (fase 2).

pub mod busqueda;

use std::collections::BTreeSet;

use rust_decimal::{Decimal, prelude::ToPrimitive};
use serde::{Deserialize, Serialize};

use crate::{CodigoDane, DominioError};

/// Radio medio de la Tierra en kilómetros (el que usa la fórmula de haversine habitual).
const RADIO_TIERRA_KM: f64 = 6371.0;

/// Máximo de categorías de una ficha (la primera es la principal).
pub const MAXIMO_CATEGORIAS: usize = 5;

/// Radio máximo de cobertura de un anunciante local, en kilómetros.
pub const RADIO_MAXIMO_KM: u16 = 100;

/// Estado de verificación de un anunciante. El texto es contrato con la BD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoVerificacion {
    /// Registrado y sin revisar (portal del anunciante, fase 3).
    Pendiente,
    /// Revisado por la plataforma: sus fichas aparecen en el directorio.
    Verificado,
    Rechazado,
    /// Quejas o publicidad engañosa (Ley 1480 de 2011): sale del directorio hasta reactivarlo.
    Suspendido,
}

enum_texto!(EstadoVerificacion {
    Pendiente => "pendiente",
    Verificado => "verificado",
    Rechazado => "rechazado",
    Suspendido => "suspendido",
});

impl EstadoVerificacion {
    /// Transición permitida: `pendiente → verificado | rechazado`, `verificado ⇄ suspendido` y
    /// `rechazado → pendiente` (corrige y vuelve a pedir la revisión).
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está en el diagrama.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoVerificacion::*;
        match (self, hacia) {
            (Pendiente, Verificado | Rechazado)
            | (Verificado, Suspendido)
            | (Suspendido, Verificado)
            | (Rechazado, Pendiente) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "anunciante",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }

    /// ¿Sus fichas pueden aparecer en el directorio?
    pub const fn visible(self) -> bool {
        matches!(self, Self::Verificado)
    }

    /// ¿La transición exige explicar el motivo (rechazo o suspensión)?
    pub const fn exige_motivo(self) -> bool {
        matches!(self, Self::Rechazado | Self::Suspendido)
    }
}

/// Coordenadas geográficas (WGS 84) con 6 decimales, como `NUMERIC(9,6)` en la BD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coordenadas {
    latitud: Decimal,
    longitud: Decimal,
}

impl Coordenadas {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si la latitud no está en [-90, 90] o la longitud en
    /// [-180, 180].
    pub fn new(latitud: Decimal, longitud: Decimal) -> Result<Self, DominioError> {
        if latitud.abs() > Decimal::from(90) {
            return Err(DominioError::DatoInvalido {
                campo: "latitud",
                regla: "debe estar entre -90 y 90",
            });
        }
        if longitud.abs() > Decimal::from(180) {
            return Err(DominioError::DatoInvalido {
                campo: "longitud",
                regla: "debe estar entre -180 y 180",
            });
        }
        Ok(Self {
            latitud: latitud.round_dp(6),
            longitud: longitud.round_dp(6),
        })
    }

    /// Latitud en grados.
    pub const fn latitud(self) -> Decimal {
        self.latitud
    }

    /// Longitud en grados.
    pub const fn longitud(self) -> Decimal {
        self.longitud
    }

    /// Distancia en kilómetros por la fórmula de haversine (esfera de radio medio). Es una
    /// medida geométrica aproximada (error < 0,5 %), no un monto: por eso usa `f64`.
    pub fn distancia_km(self, otra: Self) -> f64 {
        let rad = |d: Decimal| d.to_f64().unwrap_or(0.0).to_radians();
        let (la1, lo1, la2, lo2) = (
            rad(self.latitud),
            rad(self.longitud),
            rad(otra.latitud),
            rad(otra.longitud),
        );
        let h = ((la2 - la1) / 2.0).sin().powi(2)
            + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
        2.0 * RADIO_TIERRA_KM * h.sqrt().min(1.0).asin()
    }
}

/// Tipo de cobertura persistido (`fichas_directorio.cobertura`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoCobertura {
    Nacional,
    Municipio,
    Radio,
}

enum_texto!(TipoCobertura {
    Nacional => "nacional",
    Municipio => "municipio",
    Radio => "radio",
});

/// Hasta dónde llega un anunciante (domicilios, desplazamiento del técnico…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cobertura {
    /// Todo el país (servicios profesionales que se prestan a distancia).
    Nacional,
    /// Las copropiedades de su mismo municipio.
    Municipio,
    /// Las copropiedades a no más de estos kilómetros de su ubicación (1 a
    /// [`RADIO_MAXIMO_KM`]).
    Radio(u16),
}

impl Cobertura {
    /// Construye la cobertura desde su tipo y el radio.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si el radio falta o sobra, o está fuera de rango.
    pub fn new(tipo: TipoCobertura, radio_km: Option<u16>) -> Result<Self, DominioError> {
        match (tipo, radio_km) {
            (TipoCobertura::Nacional, None) => Ok(Self::Nacional),
            (TipoCobertura::Municipio, None) => Ok(Self::Municipio),
            (TipoCobertura::Radio, Some(km)) if (1..=RADIO_MAXIMO_KM).contains(&km) => {
                Ok(Self::Radio(km))
            }
            (TipoCobertura::Radio, _) => Err(DominioError::DatoInvalido {
                campo: "radio_km",
                regla: "la cobertura por radio exige un radio de 1 a 100 km",
            }),
            _ => Err(DominioError::DatoInvalido {
                campo: "radio_km",
                regla: "solo la cobertura por radio lleva radio",
            }),
        }
    }

    /// Tipo de cobertura (nacional, municipio o radio).
    pub const fn tipo(self) -> TipoCobertura {
        match self {
            Self::Nacional => TipoCobertura::Nacional,
            Self::Municipio => TipoCobertura::Municipio,
            Self::Radio(_) => TipoCobertura::Radio,
        }
    }

    /// Radio en kilómetros, solo en la cobertura por radio.
    pub const fn radio_km(self) -> Option<u16> {
        match self {
            Self::Radio(km) => Some(km),
            _ => None,
        }
    }
}

/// Ubicación de un anunciante o de una copropiedad: lo único que decide la cobertura.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ubicacion {
    pub coordenadas: Option<Coordenadas>,
    pub municipio: Option<CodigoDane>,
}

/// Verifica que la ubicación del anunciante permita su cobertura: el radio necesita
/// coordenadas y la cobertura municipal, el municipio.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si falta el dato que la cobertura exige.
pub fn validar_cobertura(cobertura: Cobertura, anunciante: &Ubicacion) -> Result<(), DominioError> {
    match cobertura {
        Cobertura::Radio(_) if anunciante.coordenadas.is_none() => {
            Err(DominioError::DatoInvalido {
                campo: "cobertura",
                regla: "la cobertura por radio exige las coordenadas del anunciante",
            })
        }
        Cobertura::Municipio if anunciante.municipio.is_none() => Err(DominioError::DatoInvalido {
            campo: "cobertura",
            regla: "la cobertura municipal exige el municipio del anunciante",
        }),
        _ => Ok(()),
    }
}

/// ¿La cobertura del anunciante alcanza a la copropiedad?
///
/// - `Nacional`: siempre.
/// - `Municipio`: mismo código DANE.
/// - `Radio`: distancia de haversine ≤ radio; si alguna de las dos no tiene coordenadas, se
///   compara el municipio (una PH sin ubicar no queda sin directorio).
///
/// La función SQL `anunciante_cubre` aplica la misma regla (una prueba de integración compara
/// ambas).
pub fn cubre(cobertura: Cobertura, anunciante: &Ubicacion, copropiedad: &Ubicacion) -> bool {
    let mismo_municipio = || {
        anunciante
            .municipio
            .as_ref()
            .is_some_and(|m| copropiedad.municipio.as_ref() == Some(m))
    };
    match cobertura {
        Cobertura::Nacional => true,
        Cobertura::Municipio => mismo_municipio(),
        Cobertura::Radio(km) => match (anunciante.coordenadas, copropiedad.coordenadas) {
            (Some(a), Some(c)) => a.distancia_km(c) <= f64::from(km),
            _ => mismo_municipio(),
        },
    }
}

/// Código de una categoría del directorio (`tecnico_lavadoras`, `costura`…): estable, es la
/// clave del catálogo y la usan los clientes para filtrar.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CodigoCategoria(String);

impl CodigoCategoria {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no son de 2 a 50 caracteres `a-z`, `0-9` o `_`
    /// empezando por letra.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let v = valor.trim();
        let valido = (2..=50).contains(&v.len())
            && v.starts_with(|c: char| c.is_ascii_lowercase())
            && v.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if valido {
            Ok(Self(v.to_owned()))
        } else {
            Err(DominioError::DatoInvalido {
                campo: "categoria",
                regla: "de 2 a 50 caracteres a-z, 0-9 o guion bajo, empezando por letra",
            })
        }
    }

    /// Código normalizado de la categoría.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CodigoCategoria {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<CodigoCategoria> for String {
    fn from(c: CodigoCategoria) -> Self {
        c.0
    }
}

impl std::fmt::Display for CodigoCategoria {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Lo que la regla de categorías necesita saber de cada una.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoriaElegible {
    pub codigo: CodigoCategoria,
    /// Licores, apuestas, salud con fórmula: no se admiten al inicio (decisión del plan 08).
    pub restringida: bool,
    pub activa: bool,
}

/// Valida las categorías elegidas para una ficha. `catalogo` son las del catálogo con esos
/// códigos (las que no aparezcan no existen).
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si no hay categorías, hay más de [`MAXIMO_CATEGORIAS`], se
/// repiten, alguna no existe o está inactiva, o alguna es `restringida`.
pub fn validar_categorias(
    elegidas: &[CodigoCategoria],
    catalogo: &[CategoriaElegible],
) -> Result<(), DominioError> {
    if elegidas.is_empty() || elegidas.len() > MAXIMO_CATEGORIAS {
        return Err(DominioError::DatoInvalido {
            campo: "categorias",
            regla: "elija de 1 a 5 categorías",
        });
    }
    if elegidas.iter().collect::<BTreeSet<_>>().len() != elegidas.len() {
        return Err(DominioError::DatoInvalido {
            campo: "categorias",
            regla: "no repita categorías",
        });
    }
    for codigo in elegidas {
        let Some(c) = catalogo.iter().find(|c| &c.codigo == codigo) else {
            return Err(DominioError::DatoInvalido {
                campo: "categorias",
                regla: "alguna categoría no existe",
            });
        };
        if c.restringida {
            return Err(DominioError::DatoInvalido {
                campo: "categorias",
                regla: "las categorías restringidas (licores, apuestas, salud con fórmula) no se admiten",
            });
        }
        if !c.activa {
            return Err(DominioError::DatoInvalido {
                campo: "categorias",
                regla: "alguna categoría está inactiva",
            });
        }
    }
    Ok(())
}

/// Estado de una reseña. Se publican de inmediato y la moderación las oculta después (decisión
/// del plan 08, fase 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoResena {
    Publicada,
    /// Ocultada por la moderación de la plataforma (con motivo).
    Oculta,
}

enum_texto!(EstadoResena {
    Publicada => "publicada",
    Oculta => "oculta",
});

/// Texto opcional de una reseña: sin espacios sobrantes y de hasta 1000 caracteres. Vacío
/// equivale a no escribir texto.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si supera 1000 caracteres.
pub fn texto_resena(texto: Option<&str>) -> Result<Option<String>, DominioError> {
    let Some(t) = texto.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if t.chars().count() > 1000 {
        return Err(DominioError::DatoInvalido {
            campo: "texto",
            regla: "la reseña admite hasta 1000 caracteres",
        });
    }
    Ok(Some(t.to_owned()))
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    fn coord(lat: Decimal, lon: Decimal) -> Coordenadas {
        Coordenadas::new(lat, lon).expect("válidas")
    }

    fn en(c: Option<Coordenadas>, municipio: Option<&str>) -> Ubicacion {
        Ubicacion {
            coordenadas: c,
            municipio: municipio.map(|m| CodigoDane::new(m).expect("válido")),
        }
    }

    fn cat(codigo: &str, restringida: bool) -> CategoriaElegible {
        CategoriaElegible {
            codigo: CodigoCategoria::new(codigo).expect("válido"),
            restringida,
            activa: true,
        }
    }

    fn codigos(v: &[&str]) -> Vec<CodigoCategoria> {
        v.iter()
            .map(|c| CodigoCategoria::new(c).expect("válido"))
            .collect()
    }

    #[test]
    fn verificacion_sigue_el_diagrama() {
        use EstadoVerificacion::*;
        assert_eq!(Pendiente.transicionar(Verificado), Ok(Verificado));
        assert_eq!(Verificado.transicionar(Suspendido), Ok(Suspendido));
        assert_eq!(Suspendido.transicionar(Verificado), Ok(Verificado));
        assert_eq!(Rechazado.transicionar(Pendiente), Ok(Pendiente));
        assert!(Rechazado.transicionar(Verificado).is_err());
        assert!(Suspendido.transicionar(Pendiente).is_err());
        assert!(Verificado.transicionar(Verificado).is_err());
    }

    #[test]
    fn solo_el_verificado_es_visible() {
        for e in EstadoVerificacion::TODOS {
            assert_eq!(e.visible(), *e == EstadoVerificacion::Verificado);
        }
        assert!(EstadoVerificacion::Suspendido.exige_motivo());
        assert!(!EstadoVerificacion::Verificado.exige_motivo());
    }

    #[test]
    fn coordenadas_fuera_de_rango_se_rechazan() {
        assert!(Coordenadas::new(dec!(90.1), dec!(0)).is_err());
        assert!(Coordenadas::new(dec!(0), dec!(-180.5)).is_err());
        assert_eq!(
            coord(dec!(4.1234567), dec!(-74)).latitud(),
            dec!(4.123457),
            "se guardan 6 decimales"
        );
    }

    #[test]
    fn haversine_entre_bogota_y_medellin() {
        // Plaza de Bolívar (Bogotá) y Parque de Berrío (Medellín): ~246 km en línea recta.
        let bogota = coord(dec!(4.598056), dec!(-74.075833));
        let medellin = coord(dec!(6.250000), dec!(-75.568000));
        let d = bogota.distancia_km(medellin);
        assert!((d - 246.0).abs() < 5.0, "distancia {d}");
        assert!(bogota.distancia_km(bogota).abs() < 1e-9);
    }

    #[test]
    fn cobertura_exige_radio_solo_cuando_es_por_radio() {
        assert_eq!(
            Cobertura::new(TipoCobertura::Radio, Some(3)),
            Ok(Cobertura::Radio(3))
        );
        assert!(Cobertura::new(TipoCobertura::Radio, None).is_err());
        assert!(Cobertura::new(TipoCobertura::Radio, Some(0)).is_err());
        assert!(Cobertura::new(TipoCobertura::Radio, Some(101)).is_err());
        assert!(Cobertura::new(TipoCobertura::Nacional, Some(5)).is_err());
        assert_eq!(Cobertura::Radio(7).radio_km(), Some(7));
        assert_eq!(Cobertura::Municipio.tipo(), TipoCobertura::Municipio);
    }

    #[test]
    fn cobertura_valida_la_ubicacion_del_anunciante() {
        let sin_nada = Ubicacion::default();
        assert!(validar_cobertura(Cobertura::Radio(2), &sin_nada).is_err());
        assert!(validar_cobertura(Cobertura::Municipio, &sin_nada).is_err());
        assert!(validar_cobertura(Cobertura::Nacional, &sin_nada).is_ok());
        let ubicado = en(Some(coord(dec!(4.6), dec!(-74.07))), Some("11001"));
        assert!(validar_cobertura(Cobertura::Radio(2), &ubicado).is_ok());
        assert!(validar_cobertura(Cobertura::Municipio, &ubicado).is_ok());
    }

    #[test]
    fn radio_alcanza_a_la_ph_cercana_y_no_a_la_lejana() {
        let tienda = en(Some(coord(dec!(4.600000), dec!(-74.070000))), Some("11001"));
        // ~1,1 km al norte y ~11 km al norte.
        let cerca = en(Some(coord(dec!(4.610000), dec!(-74.070000))), Some("11001"));
        let lejos = en(Some(coord(dec!(4.700000), dec!(-74.070000))), Some("11001"));
        assert!(cubre(Cobertura::Radio(2), &tienda, &cerca));
        assert!(!cubre(Cobertura::Radio(2), &tienda, &lejos));
        assert!(cubre(Cobertura::Radio(12), &tienda, &lejos));
    }

    #[test]
    fn radio_sin_coordenadas_compara_el_municipio() {
        let tienda = en(Some(coord(dec!(4.6), dec!(-74.07))), Some("11001"));
        let ph_sin_ubicar = en(None, Some("11001"));
        let ph_otro_municipio = en(None, Some("05001"));
        assert!(cubre(Cobertura::Radio(2), &tienda, &ph_sin_ubicar));
        assert!(!cubre(Cobertura::Radio(2), &tienda, &ph_otro_municipio));
    }

    #[test]
    fn municipio_y_nacional() {
        let contador = en(None, Some("11001"));
        assert!(cubre(
            Cobertura::Municipio,
            &contador,
            &en(None, Some("11001"))
        ));
        assert!(!cubre(
            Cobertura::Municipio,
            &contador,
            &en(None, Some("05001"))
        ));
        assert!(!cubre(
            Cobertura::Municipio,
            &contador,
            &Ubicacion::default()
        ));
        assert!(cubre(Cobertura::Nacional, &contador, &Ubicacion::default()));
    }

    #[test]
    fn codigo_de_categoria() {
        assert!(CodigoCategoria::new("tecnico_lavadoras").is_ok());
        assert!(CodigoCategoria::new("Costura").is_err());
        assert!(CodigoCategoria::new("1costura").is_err());
        assert!(CodigoCategoria::new("c").is_err());
        assert!(CodigoCategoria::new("arreglos de ropa").is_err());
    }

    #[test]
    fn categorias_validas_pasan() {
        let catalogo = [cat("costura", false), cat("belleza", false)];
        assert!(validar_categorias(&codigos(&["costura", "belleza"]), &catalogo).is_ok());
    }

    #[test]
    fn categoria_restringida_se_rechaza() {
        let catalogo = [cat("costura", false), cat("licores", true)];
        assert!(validar_categorias(&codigos(&["costura", "licores"]), &catalogo).is_err());
    }

    #[test]
    fn categorias_vacias_repetidas_inexistentes_o_inactivas_se_rechazan() {
        let mut inactiva = cat("viejo", false);
        inactiva.activa = false;
        let catalogo = [cat("costura", false), inactiva];
        assert!(validar_categorias(&[], &catalogo).is_err());
        assert!(validar_categorias(&codigos(&["costura", "costura"]), &catalogo).is_err());
        assert!(validar_categorias(&codigos(&["plomeria"]), &catalogo).is_err());
        assert!(validar_categorias(&codigos(&["viejo"]), &catalogo).is_err());
        let seis = codigos(&["a1", "a2", "a3", "a4", "a5", "a6"]);
        assert!(validar_categorias(&seis, &catalogo).is_err());
    }

    #[test]
    fn texto_de_resena() {
        assert_eq!(texto_resena(None), Ok(None));
        assert_eq!(texto_resena(Some("   ")), Ok(None));
        assert_eq!(
            texto_resena(Some(" Muy cumplida ")),
            Ok(Some("Muy cumplida".into()))
        );
        assert!(texto_resena(Some(&"a".repeat(1001))).is_err());
    }
}
