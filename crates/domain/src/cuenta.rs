//! Cuentas de la plataforma: nombre de usuario, contraseña e identificador de inicio de sesión.
//!
//! Se inicia sesión con **usuario o correo** y contraseña. El correo se verifica con un código;
//! la contraseña se guarda solo como hash Argon2id.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{DireccionCorreo, DominioError};

/// Estado de una cuenta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoCuenta {
    Activa,
    /// Bloqueada por la plataforma: no puede iniciar sesión.
    Bloqueada,
    /// Eliminada a pedido del titular (supresión): se conserva solo lo que exige la ley.
    Eliminada,
}

enum_texto!(EstadoCuenta {
    Activa => "activa",
    Bloqueada => "bloqueada",
    Eliminada => "eliminada",
});

/// Nombre de usuario: 3 a 30 caracteres `a-z`, `0-9`, `.`, `_` o `-`, empieza por letra. Se
/// guarda en minúsculas. No puede contener `@` (así nunca se confunde con un correo).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NombreUsuario(String);

impl NombreUsuario {
    /// # Errors
    ///
    /// [`DominioError::NombreUsuarioInvalido`] si no cumple el formato.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let valor = valor.trim().to_lowercase();
        let valido = (3..=30).contains(&valor.len())
            && valor.starts_with(|c: char| c.is_ascii_lowercase())
            && valor
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c));
        if valido {
            Ok(Self(valor))
        } else {
            Err(DominioError::NombreUsuarioInvalido)
        }
    }

    /// Nombre de usuario normalizado en minúsculas.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NombreUsuario {
    type Error = DominioError;

    fn try_from(valor: String) -> Result<Self, Self::Error> {
        Self::new(&valor)
    }
}

impl From<NombreUsuario> for String {
    fn from(n: NombreUsuario) -> Self {
        n.0
    }
}

/// Con qué se identifica alguien al iniciar sesión: su correo o su nombre de usuario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identificador {
    Correo(DireccionCorreo),
    Usuario(NombreUsuario),
}

impl Identificador {
    /// Si el texto tiene `@` es un correo; si no, un nombre de usuario.
    ///
    /// # Errors
    ///
    /// El error de validación del tipo que corresponda.
    pub fn parse(valor: &str) -> Result<Self, DominioError> {
        if valor.contains('@') {
            DireccionCorreo::new(valor).map(Self::Correo)
        } else {
            NombreUsuario::new(valor).map(Self::Usuario)
        }
    }
}

/// Contraseña nueva que cumple la política: de 10 a 128 caracteres y no puede ser un solo
/// carácter repetido. (NIST SP 800-63B: largo antes que reglas de composición.)
///
/// Nunca se muestra: `Debug` la oculta y no implementa `Serialize`.
#[derive(Clone, PartialEq, Eq)]
pub struct ContrasenaNueva(String);

impl ContrasenaNueva {
    /// Caracteres mínimos de una contraseña.
    pub const LONGITUD_MINIMA: usize = 10;
    /// Caracteres máximos de una contraseña.
    pub const LONGITUD_MAXIMA: usize = 128;

    /// # Errors
    ///
    /// [`DominioError::ContrasenaDebil`] si no cumple la política.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let largo = valor.chars().count();
        let mut caracteres = valor.chars();
        let primero = caracteres.next();
        let repetida = primero.is_some_and(|p| caracteres.all(|c| c == p));
        if (Self::LONGITUD_MINIMA..=Self::LONGITUD_MAXIMA).contains(&largo) && !repetida {
            Ok(Self(valor.to_owned()))
        } else {
            Err(DominioError::ContrasenaDebil)
        }
    }

    /// Texto de la contraseña, solo para calcular su hash. Nunca se loguea ni se guarda.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ContrasenaNueva {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ContrasenaNueva(<oculta>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombre_de_usuario_valido_se_normaliza() {
        assert_eq!(
            NombreUsuario::new(" Admin.Alameda_2 ")
                .expect("válido")
                .as_str(),
            "admin.alameda_2"
        );
        for invalido in [
            "ab",
            "2admin",
            "con espacio",
            "a@b",
            "ñandu",
            &"a".repeat(31),
        ] {
            assert_eq!(
                NombreUsuario::new(invalido),
                Err(DominioError::NombreUsuarioInvalido),
                "{invalido:?}"
            );
        }
    }

    #[test]
    fn identificador_distingue_correo_de_usuario() {
        assert!(matches!(
            Identificador::parse("Admin@Conjunto.co"),
            Ok(Identificador::Correo(c)) if c.as_str() == "admin@conjunto.co"
        ));
        assert!(matches!(
            Identificador::parse("porteria1"),
            Ok(Identificador::Usuario(u)) if u.as_str() == "porteria1"
        ));
        assert!(Identificador::parse("x@").is_err());
    }

    #[test]
    fn politica_de_contrasena() {
        assert!(ContrasenaNueva::new("una frase larga").is_ok());
        for debil in ["corta", "aaaaaaaaaaaa", &"x".repeat(129)] {
            assert_eq!(
                ContrasenaNueva::new(debil),
                Err(DominioError::ContrasenaDebil)
            );
        }
        let c = ContrasenaNueva::new("secreto-muy-largo").expect("válida");
        assert!(!format!("{c:?}").contains("secreto"));
    }
}
