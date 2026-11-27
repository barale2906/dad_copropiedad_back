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

/// Tipo de dispositivo desde el que se inicia sesión, deducido del `User-Agent`: navegador (o la
/// app) y sistema operativo, **sin versiones** (una actualización del navegador no es un
/// dispositivo nuevo). Sirve para avisar al titular de un inicio de sesión desde un dispositivo
/// que no había usado; no identifica un equipo físico.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TipoDispositivo {
    /// `Chrome`, `Firefox`, `Edge`, `Safari`, `Opera`, `App DAD` o `Otro navegador`.
    pub navegador: &'static str,
    /// `Windows`, `Android`, `iPhone`, `iPad`, `macOS`, `Linux` u `Otro sistema`.
    pub sistema: &'static str,
}

impl TipoDispositivo {
    /// Deduce el tipo desde el `User-Agent`; `None` si no llegó.
    pub fn desde_agente(agente: Option<&str>) -> Option<Self> {
        let a = agente?.trim();
        if a.is_empty() {
            return None;
        }
        let navegador = if a.contains("Dart/") || a.contains("dad-app") {
            "App DAD"
        } else if a.contains("Edg/") || a.contains("EdgA/") || a.contains("EdgiOS/") {
            "Edge"
        } else if a.contains("OPR/") || a.contains("Opera") {
            "Opera"
        } else if a.contains("Firefox/") || a.contains("FxiOS/") {
            "Firefox"
        } else if a.contains("Chrome/") || a.contains("CriOS/") {
            "Chrome"
        } else if a.contains("Safari/") {
            "Safari"
        } else {
            "Otro navegador"
        };
        // El orden importa: Android se anuncia como Linux y el iPhone como «like Mac OS X».
        let sistema = if a.contains("Android") {
            "Android"
        } else if a.contains("iPhone") {
            "iPhone"
        } else if a.contains("iPad") {
            "iPad"
        } else if a.contains("Windows") {
            "Windows"
        } else if a.contains("Mac OS X") || a.contains("Macintosh") {
            "macOS"
        } else if a.contains("Linux") || a.contains("X11") {
            "Linux"
        } else {
            "Otro sistema"
        };
        Some(Self { navegador, sistema })
    }
}

impl fmt::Display for TipoDispositivo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} en {}", self.navegador, self.sistema)
    }
}

/// ¿Se avisa al titular de este inicio de sesión? Sí cuando el tipo de dispositivo no aparece
/// entre los de sus sesiones anteriores. No se avisa en el primer inicio de sesión de la cuenta
/// (no hay contra qué comparar) ni cuando no se sabe desde dónde se entró.
pub fn avisar_dispositivo_nuevo(
    anteriores: &[Option<TipoDispositivo>],
    actual: Option<&TipoDispositivo>,
) -> bool {
    match actual {
        None => false,
        Some(actual) => !anteriores.is_empty() && !anteriores.iter().flatten().any(|d| d == actual),
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

    const CHROME_WINDOWS: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                                  (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
    const SAFARI_IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) \
                                 AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 \
                                 Mobile/15E148 Safari/604.1";
    const CHROME_ANDROID: &str = "Mozilla/5.0 (Linux; Android 14; SM-A145M) AppleWebKit/537.36 \
                                  (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36";
    const EDGE_WINDOWS: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                                (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0";

    fn tipo(a: &str) -> TipoDispositivo {
        TipoDispositivo::desde_agente(Some(a)).expect("agente")
    }

    #[test]
    fn el_tipo_de_dispositivo_sale_del_agente_sin_versiones() {
        assert_eq!(tipo(CHROME_WINDOWS).to_string(), "Chrome en Windows");
        assert_eq!(tipo(SAFARI_IPHONE).to_string(), "Safari en iPhone");
        assert_eq!(tipo(CHROME_ANDROID).to_string(), "Chrome en Android");
        assert_eq!(tipo(EDGE_WINDOWS).to_string(), "Edge en Windows");
        assert_eq!(TipoDispositivo::desde_agente(Some("  ")), None);
        assert_eq!(TipoDispositivo::desde_agente(None), None);
    }

    #[test]
    fn otra_version_del_mismo_navegador_no_es_un_dispositivo_nuevo() {
        let nuevo = CHROME_WINDOWS.replace("131.0.0.0", "132.0.6834.83");
        assert_eq!(tipo(&nuevo), tipo(CHROME_WINDOWS));
    }

    #[test]
    fn se_avisa_solo_si_el_dispositivo_no_se_habia_usado() {
        let anteriores = vec![Some(tipo(CHROME_WINDOWS)), None];

        assert!(avisar_dispositivo_nuevo(
            &anteriores,
            Some(&tipo(SAFARI_IPHONE))
        ));
        assert!(!avisar_dispositivo_nuevo(
            &anteriores,
            Some(&tipo(CHROME_WINDOWS))
        ));
    }

    #[test]
    fn el_primer_inicio_o_un_agente_desconocido_no_avisan() {
        assert!(!avisar_dispositivo_nuevo(&[], Some(&tipo(CHROME_WINDOWS))));
        assert!(!avisar_dispositivo_nuevo(
            &[Some(tipo(CHROME_WINDOWS))],
            None
        ));
    }
}
