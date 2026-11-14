//! Cofre de secretos con AES-256-GCM de `ring` (puerto [`Cofre`]): cada secreto lleva su nonce
//! aleatorio de 96 bits y se ata a su contexto como dato adicional autenticado (AAD), así no se
//! puede mover a otra entidad ni a otro uso. Formato: versión (1 byte) ‖ nonce (12) ‖ cifrado y
//! etiqueta (16).

use application::{AppError, AppResult, puertos::cofre::Cofre};
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};

const VERSION: u8 = 1;

#[derive(Debug, thiserror::Error)]
#[error("el secreto no se pudo descifrar (alterado, otra clave u otro contexto)")]
struct SecretoIlegible;

/// AES-256-GCM con la clave maestra de `FACTURACION_CLAVE_MAESTRA`; sin ella, todo responde que
/// el servidor no tiene el cofre configurado.
pub struct CofreAesGcm {
    clave: Option<LessSafeKey>,
    azar: SystemRandom,
}

impl std::fmt::Debug for CofreAesGcm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CofreAesGcm")
            .field("configurado", &self.clave.is_some())
            .finish()
    }
}

impl CofreAesGcm {
    /// Cofre con la clave maestra (o sin ella: deshabilitado).
    pub fn new(clave_maestra: Option<[u8; 32]>) -> Self {
        Self {
            clave: clave_maestra
                .and_then(|k| UnboundKey::new(&AES_256_GCM, &k).ok().map(LessSafeKey::new)),
            azar: SystemRandom::new(),
        }
    }

    fn clave(&self) -> AppResult<&LessSafeKey> {
        self.clave.as_ref().ok_or_else(|| {
            AppError::Validacion(
                "este servidor no tiene configurada la clave maestra de facturación \
                 (FACTURACION_CLAVE_MAESTRA)"
                    .into(),
            )
        })
    }
}

impl Cofre for CofreAesGcm {
    fn cifrar(&self, contexto: &str, datos: &[u8]) -> AppResult<Vec<u8>> {
        let clave = self.clave()?;
        let mut nonce = [0u8; NONCE_LEN];
        self.azar
            .fill(&mut nonce)
            .map_err(|_| AppError::infra(SecretoIlegible))?;
        let mut cuerpo = datos.to_vec();
        clave
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(contexto.as_bytes()),
                &mut cuerpo,
            )
            .map_err(|_| AppError::infra(SecretoIlegible))?;
        let mut salida = Vec::with_capacity(1 + NONCE_LEN + cuerpo.len());
        salida.push(VERSION);
        salida.extend_from_slice(&nonce);
        salida.extend_from_slice(&cuerpo);
        Ok(salida)
    }

    fn descifrar(&self, contexto: &str, cifrado: &[u8]) -> AppResult<Vec<u8>> {
        let clave = self.clave()?;
        let (version, resto) = cifrado
            .split_first()
            .ok_or_else(|| AppError::infra(SecretoIlegible))?;
        if *version != VERSION || resto.len() < NONCE_LEN {
            return Err(AppError::infra(SecretoIlegible));
        }
        let (nonce, cuerpo) = resto.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce
            .try_into()
            .map_err(|_| AppError::infra(SecretoIlegible))?;
        let mut cuerpo = cuerpo.to_vec();
        let claro = clave
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(contexto.as_bytes()),
                &mut cuerpo,
            )
            .map_err(|_| AppError::infra(SecretoIlegible))?;
        Ok(claro.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cofre() -> CofreAesGcm {
        CofreAesGcm::new(Some([7; 32]))
    }

    #[test]
    fn lo_cifrado_se_descifra_con_el_mismo_contexto() {
        let c = cofre();
        let cifrado = c.cifrar("pin:t1", b"12345").expect("cifra");
        assert_ne!(&cifrado[13..], b"12345");
        assert_eq!(c.descifrar("pin:t1", &cifrado).expect("descifra"), b"12345");
    }

    #[test]
    fn otro_contexto_u_otra_clave_no_descifran() {
        let cifrado = cofre().cifrar("pin:t1", b"12345").expect("cifra");
        assert!(cofre().descifrar("pin:t2", &cifrado).is_err());
        assert!(
            CofreAesGcm::new(Some([8; 32]))
                .descifrar("pin:t1", &cifrado)
                .is_err()
        );
    }

    #[test]
    fn un_cambio_en_el_cifrado_se_detecta() {
        let mut cifrado = cofre().cifrar("x", b"secreto").expect("cifra");
        let ultimo = cifrado.len() - 1;
        cifrado[ultimo] ^= 1;
        assert!(cofre().descifrar("x", &cifrado).is_err());
    }

    #[test]
    fn dos_cifrados_del_mismo_secreto_son_distintos() {
        let c = cofre();
        assert_ne!(
            c.cifrar("x", b"s").expect("cifra"),
            c.cifrar("x", b"s").expect("cifra")
        );
    }

    #[test]
    fn sin_clave_maestra_el_cofre_no_opera() {
        let r = CofreAesGcm::new(None).cifrar("x", b"s");
        assert!(matches!(r, Err(AppError::Validacion(_))));
    }
}
