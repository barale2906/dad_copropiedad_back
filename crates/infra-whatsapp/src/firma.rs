//! Verificación de la firma que Meta envía en cada webhook.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

/// Nombre de la cabecera HTTP con la firma.
pub const CABECERA_FIRMA: &str = "x-hub-signature-256";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorFirma {
    #[error("falta la cabecera de firma")]
    Ausente,
    #[error("formato de firma inválido")]
    Formato,
    #[error("la firma no coincide")]
    NoCoincide,
}

/// Verifica `cabecera` (`sha256=<hex>`) contra el HMAC-SHA256 de `cuerpo` **crudo** (antes de
/// deserializar). La comparación es de tiempo constante.
pub fn verificar_firma(
    app_secret: &[u8],
    cuerpo: &[u8],
    cabecera: Option<&str>,
) -> Result<(), ErrorFirma> {
    let cabecera = cabecera.ok_or(ErrorFirma::Ausente)?;
    let hex_firma = cabecera
        .strip_prefix("sha256=")
        .ok_or(ErrorFirma::Formato)?;
    let esperada = hex::decode(hex_firma).map_err(|_| ErrorFirma::Formato)?;

    let mut mac = Hmac::<Sha256>::new_from_slice(app_secret).map_err(|_| ErrorFirma::Formato)?;
    mac.update(cuerpo);
    mac.verify_slice(&esperada)
        .map_err(|_| ErrorFirma::NoCoincide)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn firmar(secreto: &[u8], cuerpo: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secreto).unwrap();
        mac.update(cuerpo);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn acepta_firma_valida_y_rechaza_las_demas() {
        let cuerpo = br#"{"object":"whatsapp_business_account"}"#;
        let firma = firmar(b"secreto", cuerpo);

        assert_eq!(verificar_firma(b"secreto", cuerpo, Some(&firma)), Ok(()));
        assert_eq!(
            verificar_firma(b"otro", cuerpo, Some(&firma)),
            Err(ErrorFirma::NoCoincide)
        );
        assert_eq!(
            verificar_firma(b"secreto", b"{}", Some(&firma)),
            Err(ErrorFirma::NoCoincide)
        );
        assert_eq!(
            verificar_firma(b"secreto", cuerpo, None),
            Err(ErrorFirma::Ausente)
        );
        assert_eq!(
            verificar_firma(b"secreto", cuerpo, Some("sha1=abc")),
            Err(ErrorFirma::Formato)
        );
    }
}
