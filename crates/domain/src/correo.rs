//! Dirección de correo electrónico validada.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Dirección de correo con forma válida: `local@dominio.tld`, sin espacios, máximo 254
/// caracteres. Se normaliza a minúsculas (en la práctica los proveedores no distinguen
/// mayúsculas y así se evitan cuentas duplicadas).
///
/// No garantiza que el buzón exista: eso solo lo prueba un correo de verificación.
///
/// Es un **dato personal** (Ley 1581): no se registra en logs ni en la auditoría.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DireccionCorreo(String);

impl DireccionCorreo {
    /// Longitud máxima de una dirección (RFC 5321).
    pub const LONGITUD_MAXIMA: usize = 254;

    /// Valida y normaliza la dirección.
    ///
    /// # Errors
    ///
    /// [`DominioError::CorreoInvalido`] si no tiene la forma `local@dominio.tld`.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let valor = valor.trim().to_lowercase();
        if Self::es_valida(&valor) {
            Ok(Self(valor))
        } else {
            Err(DominioError::CorreoInvalido)
        }
    }

    fn es_valida(valor: &str) -> bool {
        if valor.is_empty()
            || valor.len() > Self::LONGITUD_MAXIMA
            || valor.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return false;
        }
        let Some((local, dominio)) = valor.split_once('@') else {
            return false;
        };
        !local.is_empty()
            && local.len() <= 64
            && !dominio.contains('@')
            && dominio.contains('.')
            && dominio
                .split('.')
                .all(|parte| !parte.is_empty() && !parte.starts_with('-') && !parte.ends_with('-'))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// Debug sin el valor: evita que una dirección termine en logs por un `{:?}`.
impl fmt::Debug for DireccionCorreo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DireccionCorreo(<oculto>)")
    }
}

impl TryFrom<String> for DireccionCorreo {
    type Error = DominioError;

    fn try_from(valor: String) -> Result<Self, Self::Error> {
        Self::new(&valor)
    }
}

impl From<DireccionCorreo> for String {
    fn from(c: DireccionCorreo) -> Self {
        c.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acepta_y_normaliza_direcciones_validas() {
        let c = DireccionCorreo::new("  Admin.Torre3@Conjunto-Alameda.com.co ").expect("válida");
        assert_eq!(c.as_str(), "admin.torre3@conjunto-alameda.com.co");
        assert!(DireccionCorreo::new("a+filtro@b.co").is_ok());
    }

    #[test]
    fn rechaza_direcciones_mal_formadas() {
        for invalida in [
            "",
            "sin-arroba.com",
            "@dominio.com",
            "usuario@",
            "usuario@dominio",
            "dos@@dominio.com",
            "a@b@c.com",
            "con espacio@dominio.com",
            "usuario@.dominio.com",
            "usuario@dominio..com",
            "usuario@-dominio.com",
        ] {
            assert!(DireccionCorreo::new(invalida).is_err(), "{invalida:?}");
        }
        let larga = format!("{}@d.co", "a".repeat(65));
        assert!(DireccionCorreo::new(&larga).is_err());
    }

    #[test]
    fn debug_no_expone_la_direccion() {
        let c = DireccionCorreo::new("persona@correo.co").expect("válida");
        assert!(!format!("{c:?}").contains("persona"));
    }

    #[test]
    fn se_deserializa_validando() {
        let ok: Result<DireccionCorreo, _> = serde_json::from_str("\"x@y.co\"");
        let err: Result<DireccionCorreo, _> = serde_json::from_str("\"no-es-correo\"");
        assert!(ok.is_ok());
        assert!(err.is_err());
    }
}
