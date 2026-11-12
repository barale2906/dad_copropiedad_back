//! Búsqueda de proveedores (plan 08, fase 2; ADR 0014).
//!
//! El residente escribe lo que necesita («busco costurera», «¿alguien conoce un plomero?») en
//! el chat de su PH o en la app. Reglas puras de este módulo:
//! - [`consulta_busqueda`]: quita las palabras de intención y cortesía y deja lo que se busca.
//! - [`resolver_categoria`]: la categoría del catálogo que nombra la consulta, por nombre o
//!   sinónimo («modista» → costura), tolerando plurales.
//! - [`comparar_sugerencias`]: orden de las fichas sugeridas (destacada solo si hay pauta,
//!   calificación, cercanía).
//! - [`enlace_whatsapp`]: el enlace `wa.me` que se fija en los grupos de WhatsApp para llevar la
//!   pregunta al chat de la PH.
//!
//! Nada de esto usa atributos de personas: la consulta solo se compara con el catálogo.

use std::cmp::Ordering;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{DominioError, directorio::CodigoCategoria};

/// Fichas que se sugieren por búsqueda (las que caben en un mensaje de WhatsApp).
pub const MAXIMO_SUGERENCIAS: usize = 3;
/// Días que se conserva el texto de una búsqueda (dato personal); después se borra.
pub const RETENCION_SOLICITUDES_DIAS: u32 = 90;
/// Largo mínimo de lo que se busca, ya sin palabras de intención.
pub const LARGO_MINIMO_CONSULTA: usize = 3;
/// Largo máximo de lo que se busca.
pub const LARGO_MAXIMO_CONSULTA: usize = 100;

/// Canal por el que llegó una búsqueda de proveedor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanalSolicitud {
    Whatsapp,
    App,
}

enum_texto!(CanalSolicitud {
    Whatsapp => "whatsapp",
    App => "app",
});

/// Palabras que abren una búsqueda en el chat: un texto que empieza así busca un proveedor sin
/// pasar por el menú (es lo que trae el enlace de los grupos: «Busco …»).
const VERBOS_BUSQUEDA: &[&str] = &["busco", "buscar", "buscando", "necesito", "requiero"];

/// Palabras de intención y cortesía que se quitan al comienzo de la consulta.
const PALABRAS_INICIALES: &[&str] = &[
    "hola",
    "buenas",
    "buenos",
    "dias",
    "tardes",
    "noches",
    "vecinos",
    "vecino",
    "vecina",
    "busco",
    "buscar",
    "buscando",
    "estoy",
    "necesito",
    "requiero",
    "quiero",
    "quisiera",
    "alguien",
    "algun",
    "alguna",
    "conoce",
    "conocen",
    "sabe",
    "saben",
    "recomienda",
    "recomiendan",
    "me",
    "nos",
    "que",
    "quien",
    "donde",
    "venden",
    "vendan",
    "consigo",
    "hay",
    "haga",
    "hace",
    "arregle",
    "arregla",
    "repare",
    "un",
    "una",
    "unos",
    "unas",
    "el",
    "la",
    "los",
    "las",
    "por",
    "para",
    "a",
    "de",
    "del",
];

/// Palabras de cortesía que se quitan al final.
const PALABRAS_FINALES: &[&str] = &[
    "por", "favor", "porfa", "gracias", "urgente", "ya", "hoy", "aqui", "cerca",
];

/// Minúsculas, sin tildes (también la ñ, como `unaccent`), solo letras y dígitos separados por
/// un espacio.
pub fn normalizar_busqueda(texto: &str) -> String {
    let mut s = String::with_capacity(texto.len());
    for c in texto.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        };
        s.push(c);
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// ¿El texto abre una búsqueda de proveedor («busco …», «necesito …»)?
pub fn es_busqueda(texto: &str) -> bool {
    normalizar_busqueda(texto)
        .split(' ')
        .next()
        .is_some_and(|p| VERBOS_BUSQUEDA.contains(&p))
}

/// Lo que se busca, normalizado y sin palabras de intención ni cortesía:
/// «¿Alguien conoce una costurera, por favor?» → `costurera`.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si lo que queda tiene menos de [`LARGO_MINIMO_CONSULTA`] o
/// más de [`LARGO_MAXIMO_CONSULTA`] caracteres.
pub fn consulta_busqueda(texto: &str) -> Result<String, DominioError> {
    let normal = normalizar_busqueda(texto);
    let mut palabras: Vec<&str> = normal.split(' ').filter(|p| !p.is_empty()).collect();
    let inicio = palabras
        .iter()
        .position(|p| !PALABRAS_INICIALES.contains(p))
        .unwrap_or(palabras.len());
    palabras.drain(..inicio);
    while palabras
        .last()
        .is_some_and(|p| PALABRAS_FINALES.contains(p))
    {
        palabras.pop();
    }
    let consulta = palabras.join(" ");
    let largo = consulta.chars().count();
    if largo < LARGO_MINIMO_CONSULTA {
        return Err(DominioError::DatoInvalido {
            campo: "texto",
            regla: "escriba qué busca (por ejemplo «plomero» o «costurera»)",
        });
    }
    if largo > LARGO_MAXIMO_CONSULTA {
        return Err(DominioError::DatoInvalido {
            campo: "texto",
            regla: "la búsqueda admite hasta 100 caracteres",
        });
    }
    Ok(consulta)
}

/// Categoría del catálogo con lo necesario para reconocerla en una consulta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoriaBuscable {
    pub codigo: CodigoCategoria,
    pub nombre: String,
    pub sinonimos: Vec<String>,
    /// `true` = rama del árbol (agrupa subcategorías).
    pub es_rama: bool,
}

/// Raíz sencilla para tolerar plurales: «lavadoras» → «lavadora», «televisores» → «televisor».
/// Se aplica igual a la consulta y al catálogo, así que basta con que sea consistente.
fn raiz(palabra: &str) -> &str {
    let largo = palabra.chars().count();
    if largo > 4 && palabra.ends_with("es") {
        &palabra[..palabra.len() - 2]
    } else if largo > 3 && palabra.ends_with('s') {
        &palabra[..palabra.len() - 1]
    } else {
        palabra
    }
}

/// Palabras normalizadas y reducidas a su raíz, entre espacios (para comparar palabras
/// completas con `contains`).
fn en_palabras(texto: &str) -> String {
    let normal = normalizar_busqueda(texto);
    let raices: Vec<&str> = normal.split(' ').map(raiz).collect();
    format!(" {} ", raices.join(" "))
}

/// Cómo nombra la consulta a una categoría.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Coincidencia {
    /// La consulta es parte de un nombre o sinónimo («lavadora» en «técnico de lavadoras»).
    Parcial,
    /// La consulta contiene el nombre o sinónimo completo («busco costurera» → «costurera»).
    Completa,
}

/// La categoría que nombra la consulta (por nombre o sinónimo, palabras completas y tolerando
/// plurales). Gana la coincidencia completa más larga y, a igualdad, la subcategoría sobre la
/// rama. `None` si nada coincide o si solo hay coincidencias parciales con varias categorías
/// (ambiguo: entonces se busca por texto).
pub fn resolver_categoria(
    consulta: &str,
    categorias: &[CategoriaBuscable],
) -> Option<CodigoCategoria> {
    let q = en_palabras(consulta);
    if q.trim().is_empty() {
        return None;
    }
    let mut mejor: Option<((Coincidencia, usize, bool), &CategoriaBuscable)> = None;
    let mut parciales = 0usize;
    for c in categorias {
        let puntaje = std::iter::once(&c.nombre)
            .chain(&c.sinonimos)
            .filter_map(|termino| {
                let t = en_palabras(termino);
                if t.trim().is_empty() {
                    None
                } else if q.contains(&t) {
                    Some((Coincidencia::Completa, t.len(), !c.es_rama))
                } else if t.contains(&q) {
                    Some((Coincidencia::Parcial, t.len(), !c.es_rama))
                } else {
                    None
                }
            })
            .max();
        let Some(p) = puntaje else { continue };
        if p.0 == Coincidencia::Parcial {
            parciales += 1;
        }
        if mejor.as_ref().is_none_or(|(m, _)| p > *m) {
            mejor = Some((p, c));
        }
    }
    match mejor {
        Some(((Coincidencia::Parcial, ..), _)) if parciales > 1 => None,
        Some((_, c)) => Some(c.codigo.clone()),
        None => None,
    }
}

/// Lo que decide el orden de una ficha sugerida.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidato {
    /// Destacada **con pauta**: solo `true` donde la PH tiene el módulo `publicidad`.
    pub destacada: bool,
    pub promedio: Option<Decimal>,
    pub resenas: u32,
    pub distancia_km: Option<Decimal>,
}

/// Orden de las sugerencias: primero las destacadas (pauta), luego la mejor calificada, luego
/// la más cercana y, al final, la de más reseñas. Las que no tienen calificación o distancia
/// van después de las que sí.
pub fn comparar_sugerencias(a: &Candidato, b: &Candidato) -> Ordering {
    fn desc_opcional(a: Option<Decimal>, b: Option<Decimal>) -> Ordering {
        match (a, b) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    }
    fn asc_opcional(a: Option<Decimal>, b: Option<Decimal>) -> Ordering {
        match (a, b) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    }
    b.destacada
        .cmp(&a.destacada)
        .then_with(|| desc_opcional(a.promedio, b.promedio))
        .then_with(|| asc_opcional(a.distancia_km, b.distancia_km))
        .then_with(|| b.resenas.cmp(&a.resenas))
}

/// Texto con que el enlace de los grupos abre el chat de la PH.
pub const TEXTO_ENLACE_GRUPOS: &str = "Busco ";

/// Enlace `https://wa.me/<número>?text=<texto>` para abrir el chat de una línea con un texto
/// escrito. `None` si el número visible no tiene de 8 a 15 dígitos.
pub fn enlace_whatsapp(numero_visible: &str, texto: &str) -> Option<String> {
    let digitos: String = numero_visible
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    if !(8..=15).contains(&digitos.len()) {
        return None;
    }
    let mut codificado = String::with_capacity(texto.len() * 3);
    for b in texto.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            codificado.push(char::from(b));
        } else {
            codificado.push_str(&format!("%{b:02X}"));
        }
    }
    Some(format!("https://wa.me/{digitos}?text={codificado}"))
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    fn cat(codigo: &str, nombre: &str, sinonimos: &[&str], es_rama: bool) -> CategoriaBuscable {
        CategoriaBuscable {
            codigo: CodigoCategoria::new(codigo).expect("válido"),
            nombre: nombre.into(),
            sinonimos: sinonimos.iter().map(|s| (*s).to_owned()).collect(),
            es_rama,
        }
    }

    fn catalogo() -> Vec<CategoriaBuscable> {
        vec![
            cat(
                "servicios_tecnicos",
                "Servicios técnicos",
                &["técnico", "reparación"],
                true,
            ),
            cat(
                "tecnico_lavadoras",
                "Técnico de lavadoras",
                &["lavadora", "secadora"],
                false,
            ),
            cat(
                "tecnico_televisores",
                "Técnico de televisores",
                &["televisor", "tv"],
                false,
            ),
            cat(
                "costura",
                "Costura y modistería",
                &["costurera", "modista", "arreglos de ropa"],
                false,
            ),
            cat("plomeria", "Plomería", &["plomero", "fuga de agua"], false),
        ]
    }

    fn resuelve(texto: &str) -> Option<String> {
        let q = consulta_busqueda(texto).expect("consulta");
        resolver_categoria(&q, &catalogo()).map(|c| c.as_str().to_owned())
    }

    #[test]
    fn normaliza_sin_tildes_ni_signos() {
        assert_eq!(
            normalizar_busqueda("  ¿Alguien conoce un TÉCNICO de Neveras?? "),
            "alguien conoce un tecnico de neveras"
        );
        assert_eq!(normalizar_busqueda("Señora, ¡pan!"), "senora pan");
    }

    #[test]
    fn reconoce_los_textos_que_abren_una_busqueda() {
        assert!(es_busqueda("Busco costurera"));
        assert!(es_busqueda("necesito un plomero"));
        assert!(!es_busqueda("hola"));
        assert!(!es_busqueda("quiero reservar el salón"));
    }

    #[test]
    fn consulta_quita_intencion_y_cortesia() {
        assert_eq!(
            consulta_busqueda("¿Alguien conoce una costurera, por favor?"),
            Ok("costurera".into())
        );
        assert_eq!(
            consulta_busqueda("Buenas vecinos, dónde venden arroz"),
            Ok("arroz".into())
        );
        assert_eq!(
            consulta_busqueda("busco quien haga arreglos de ropa"),
            Ok("arreglos de ropa".into())
        );
    }

    #[test]
    fn consulta_vacia_o_muy_larga_se_rechaza() {
        assert!(consulta_busqueda("busco").is_err());
        assert!(consulta_busqueda("necesito un ?").is_err());
        assert!(consulta_busqueda(&"a".repeat(101)).is_err());
    }

    #[test]
    fn resuelve_la_categoria_por_sinonimo() {
        assert_eq!(resuelve("busco costurera"), Some("costura".into()));
        assert_eq!(
            resuelve("alguien conoce una modista"),
            Some("costura".into())
        );
        assert_eq!(resuelve("tengo una fuga de agua"), Some("plomeria".into()));
    }

    #[test]
    fn resuelve_tolerando_plurales() {
        assert_eq!(
            resuelve("arreglo de lavadoras"),
            Some("tecnico_lavadoras".into())
        );
        assert_eq!(resuelve("televisores"), Some("tecnico_televisores".into()));
    }

    #[test]
    fn la_subcategoria_gana_a_la_rama() {
        assert_eq!(
            resuelve("técnico de lavadoras"),
            Some("tecnico_lavadoras".into())
        );
        assert_eq!(resuelve("un técnico"), Some("servicios_tecnicos".into()));
    }

    #[test]
    fn sin_categoria_o_ambigua_no_resuelve() {
        assert_eq!(resuelve("arroz"), None);
        // «arreglo» es parte de dos categorías y no nombra ninguna completa: ambiguo.
        let catalogo = [
            cat("ropa", "Arreglo de ropa", &[], false),
            cat("calzado", "Arreglo de calzado", &[], false),
        ];
        assert_eq!(resolver_categoria("arreglo", &catalogo), None);
        assert_eq!(
            resolver_categoria("arreglo de calzado", &catalogo).map(|c| c.as_str().to_owned()),
            Some("calzado".into())
        );
    }

    #[test]
    fn destacada_calificacion_y_cercania_ordenan() {
        let c = |destacada, promedio, distancia_km| Candidato {
            destacada,
            promedio,
            resenas: 1,
            distancia_km,
        };
        let mut v = [
            c(false, None, Some(dec!(0.5))),
            c(false, Some(dec!(4.0)), Some(dec!(3))),
            c(false, Some(dec!(4.0)), Some(dec!(1))),
            c(true, None, None),
            c(false, Some(dec!(4.8)), None),
        ];
        v.sort_by(comparar_sugerencias);
        assert_eq!(
            v,
            [
                c(true, None, None),
                c(false, Some(dec!(4.8)), None),
                c(false, Some(dec!(4.0)), Some(dec!(1))),
                c(false, Some(dec!(4.0)), Some(dec!(3))),
                c(false, None, Some(dec!(0.5))),
            ]
        );
    }

    #[test]
    fn enlace_de_whatsapp_codifica_el_texto() {
        assert_eq!(
            enlace_whatsapp("+57 300 123 4567", TEXTO_ENLACE_GRUPOS),
            Some("https://wa.me/573001234567?text=Busco%20".into())
        );
        assert_eq!(
            enlace_whatsapp("573001234567", "¿Ñ?"),
            Some("https://wa.me/573001234567?text=%C2%BF%C3%91%3F".into())
        );
        assert_eq!(enlace_whatsapp("123", "x"), None);
    }

    #[test]
    fn canal_de_la_solicitud_es_contrato_con_la_bd() {
        assert_eq!(CanalSolicitud::Whatsapp.as_str(), "whatsapp");
        assert_eq!(CanalSolicitud::try_from("app"), Ok(CanalSolicitud::App));
        assert!(CanalSolicitud::try_from("sms").is_err());
    }
}
