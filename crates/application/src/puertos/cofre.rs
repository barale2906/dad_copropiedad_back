//! Cofre de secretos (plan 18 fase B1): cifra con autenticación lo que la plataforma guarda en
//! nombre de una entidad y debe poder volver a usar (el certificado `.p12`, su contraseña y el PIN
//! del software de facturación). La clave maestra vive solo en la configuración del servidor.

use crate::AppResult;

/// Cifrado autenticado con la clave maestra de la plataforma.
pub trait Cofre: Send + Sync {
    /// Cifra `datos` atados a `contexto` (p. ej. `certificado:{tenant}`): descifrarlos con otro
    /// contexto falla, así un secreto no se puede mover a otra entidad ni a otro uso.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Validacion`] si el servidor no tiene clave maestra.
    fn cifrar(&self, contexto: &str, datos: &[u8]) -> AppResult<Vec<u8>>;

    /// Descifra lo que [`Cofre::cifrar`] produjo con el mismo contexto.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Validacion`] sin clave maestra; [`crate::AppError::Infraestructura`] si
    /// el contenido fue alterado o se cifró con otra clave u otro contexto.
    fn descifrar(&self, contexto: &str, cifrado: &[u8]) -> AppResult<Vec<u8>>;
}

#[cfg(test)]
pub(crate) mod memoria {
    //! Cofre de pruebas: "cifra" anteponiendo el contexto (descifrar con otro contexto falla).

    use super::Cofre;
    use crate::{AppError, AppResult};

    /// Cofre sin criptografía para las pruebas de los casos de uso.
    pub struct CofrePrueba;

    impl Cofre for CofrePrueba {
        fn cifrar(&self, contexto: &str, datos: &[u8]) -> AppResult<Vec<u8>> {
            Ok([contexto.as_bytes(), b"|", datos].concat())
        }

        fn descifrar(&self, contexto: &str, cifrado: &[u8]) -> AppResult<Vec<u8>> {
            let prefijo = [contexto.as_bytes(), b"|"].concat();
            cifrado
                .strip_prefix(prefijo.as_slice())
                .map(<[u8]>::to_vec)
                .ok_or_else(|| AppError::infra(std::io::Error::other("otro contexto")))
        }
    }
}
