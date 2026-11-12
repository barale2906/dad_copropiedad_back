//! Sorteos verificables (parqueaderos comunales, salones en temporada alta).
//!
//! Esquema **compromiso–revelación**: al crear el sorteo se publica el SHA-256 de una semilla
//! secreta ([`Semilla::compromiso`]); al ejecutarlo se revela la semilla y cualquiera puede
//! recalcular el resultado con [`ejecutar`] y comprobar que coincide con el compromiso. Nadie
//! (ni la administración) puede escoger la semilla después de ver a los inscritos.
//!
//! El orden: primero el **puntaje** (prioridades de [`CriteriosSorteo`]; después se podrán
//! sumar o restar factores, como rotación o llamados de atención), y a igual puntaje un valor
//! pseudoaleatorio derivado de la semilla y del bien privado.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{BienPrivadoId, DominioError};

/// Semilla de 32 bytes en hexadecimal (64 caracteres en minúscula).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Semilla(String);

impl Semilla {
    /// Semilla a partir de 32 bytes aleatorios (los genera la aplicación con el SO).
    pub fn desde_bytes(bytes: [u8; 32]) -> Self {
        Self(hex::encode(bytes))
    }

    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si no son 64 caracteres hexadecimales.
    pub fn new(texto: &str) -> Result<Self, DominioError> {
        let t = texto.trim().to_lowercase();
        if t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(Self(t))
        } else {
            Err(DominioError::DatoInvalido {
                campo: "semilla",
                regla: "64 caracteres hexadecimales",
            })
        }
    }

    /// Semilla en hexadecimal.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// SHA-256 de la semilla (hexadecimal): se publica antes de ejecutar el sorteo.
    pub fn compromiso(&self) -> String {
        hex::encode(Sha256::digest(self.0.as_bytes()))
    }
}

// La semilla no debe salir en logs antes de tiempo.
impl std::fmt::Debug for Semilla {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Semilla(<oculta>)")
    }
}

impl TryFrom<String> for Semilla {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<Semilla> for String {
    fn from(s: Semilla) -> Self {
        s.0
    }
}

/// Puntos de cada prioridad. Se guardan con el sorteo para poder reproducirlo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriteriosSorteo {
    /// Para quien inscribe a una persona con discapacidad.
    pub puntos_discapacidad: u32,
    /// Para quien no salió favorecido en el sorteo anterior del mismo tipo.
    pub puntos_no_favorecido: u32,
}

impl Default for CriteriosSorteo {
    fn default() -> Self {
        Self {
            puntos_discapacidad: 100,
            puntos_no_favorecido: 10,
        }
    }
}

/// Situación de un inscrito, de la que sale su puntaje.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inscrito {
    pub bien_privado_id: BienPrivadoId,
    pub discapacidad: bool,
    pub no_favorecido_anterior: bool,
}

impl CriteriosSorteo {
    /// Puntos de prioridad del inscrito según los criterios (discapacidad, no favorecido antes).
    pub fn puntaje(&self, i: &Inscrito) -> u32 {
        let mut p = 0;
        if i.discapacidad {
            p += self.puntos_discapacidad;
        }
        if i.no_favorecido_anterior {
            p += self.puntos_no_favorecido;
        }
        p
    }
}

/// Inscritos en el orden del sorteo (el primero es el primer favorecido). Determinista: con la
/// misma semilla, criterios e inscritos, siempre el mismo orden.
pub fn ejecutar(
    semilla: &Semilla,
    criterios: &CriteriosSorteo,
    inscritos: &[Inscrito],
) -> Vec<BienPrivadoId> {
    let mut orden: Vec<(u32, [u8; 32], BienPrivadoId)> = inscritos
        .iter()
        .map(|i| {
            let mut h = Sha256::new();
            h.update(semilla.as_str().as_bytes());
            h.update(i.bien_privado_id.as_uuid().as_bytes());
            (criterios.puntaje(i), h.finalize().into(), i.bien_privado_id)
        })
        .collect();
    orden.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    orden.into_iter().map(|(_, _, id)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inscrito(n: u128, discapacidad: bool, no_favorecido: bool) -> Inscrito {
        Inscrito {
            bien_privado_id: BienPrivadoId::from_uuid(uuid::Uuid::from_u128(n)),
            discapacidad,
            no_favorecido_anterior: no_favorecido,
        }
    }

    #[test]
    fn el_compromiso_verifica_la_semilla() {
        let s = Semilla::desde_bytes([7; 32]);
        assert_eq!(s.compromiso().len(), 64);
        assert_eq!(
            Semilla::new(s.as_str()).expect("válida").compromiso(),
            s.compromiso()
        );
        assert!(Semilla::new("xyz").is_err());
        assert!(!format!("{s:?}").contains("0707"));
    }

    #[test]
    fn es_reproducible_y_depende_de_la_semilla() {
        let inscritos: Vec<_> = (1..=20).map(|n| inscrito(n, false, false)).collect();
        let c = CriteriosSorteo::default();
        let a = ejecutar(&Semilla::desde_bytes([1; 32]), &c, &inscritos);
        assert_eq!(a, ejecutar(&Semilla::desde_bytes([1; 32]), &c, &inscritos));
        assert_ne!(a, ejecutar(&Semilla::desde_bytes([2; 32]), &c, &inscritos));
        let mut ordenados = a.clone();
        ordenados.sort();
        assert_eq!(ordenados.len(), 20);
    }

    #[test]
    fn las_prioridades_van_primero() {
        let inscritos = vec![
            inscrito(1, false, false),
            inscrito(2, false, true),
            inscrito(3, true, false),
        ];
        let orden = ejecutar(
            &Semilla::desde_bytes([9; 32]),
            &CriteriosSorteo::default(),
            &inscritos,
        );
        assert_eq!(orden[0], inscritos[2].bien_privado_id);
        assert_eq!(orden[1], inscritos[1].bien_privado_id);
    }
}
