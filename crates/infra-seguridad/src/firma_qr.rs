//! Firma Ed25519 de los QR temporales de visitas (puerto [`FirmaQr`]).
//!
//! Cada copropiedad tiene su propio par de claves, derivado de un secreto maestro
//! (`ACCESO_QR_SECRETO`) con HMAC-SHA256 sobre el id del tenant: no hay claves privadas que
//! guardar ni rotar por PH, y el puente de cada PH recibe solo su clave pública. Rotar el
//! secreto maestro invalida todos los QR emitidos (se regeneran al consultarlos).

use application::puertos::FirmaQr;
use domain::TenantId;
use hmac::{Hmac, KeyInit, Mac};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use sha2::Sha256;

/// Largo mínimo del secreto maestro (bytes).
pub const LARGO_MINIMO_SECRETO: usize = 32;

/// Firma con Ed25519 (determinista: la misma carga produce la misma firma).
#[derive(Clone)]
pub struct FirmaQrEd25519 {
    secreto: Vec<u8>,
}

impl std::fmt::Debug for FirmaQrEd25519 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FirmaQrEd25519(<secreto oculto>)")
    }
}

/// El secreto maestro es demasiado corto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("el secreto de los QR debe tener al menos {LARGO_MINIMO_SECRETO} bytes")]
pub struct SecretoQrCorto;

impl FirmaQrEd25519 {
    /// # Errors
    ///
    /// [`SecretoQrCorto`] si el secreto tiene menos de [`LARGO_MINIMO_SECRETO`] bytes.
    pub fn new(secreto: &[u8]) -> Result<Self, SecretoQrCorto> {
        if secreto.len() < LARGO_MINIMO_SECRETO {
            return Err(SecretoQrCorto);
        }
        Ok(Self {
            secreto: secreto.to_vec(),
        })
    }

    fn par(&self, tenant: TenantId) -> Ed25519KeyPair {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.secreto)
            .expect("HMAC acepta claves de cualquier largo");
        mac.update(b"dad-copropiedad:qr-visitas:");
        mac.update(tenant.as_uuid().as_bytes());
        let semilla = mac.finalize().into_bytes();
        Ed25519KeyPair::from_seed_unchecked(&semilla)
            .expect("una semilla de 32 bytes siempre es válida")
    }
}

impl FirmaQr for FirmaQrEd25519 {
    fn firmar(&self, tenant: TenantId, carga: &[u8]) -> Vec<u8> {
        self.par(tenant).sign(carga).as_ref().to_vec()
    }

    fn verificar(&self, tenant: TenantId, carga: &[u8], firma: &[u8]) -> bool {
        verificar_con_clave(&self.clave_publica(tenant), carga, firma)
    }

    fn clave_publica(&self, tenant: TenantId) -> [u8; 32] {
        let mut clave = [0u8; 32];
        clave.copy_from_slice(self.par(tenant).public_key().as_ref());
        clave
    }
}

/// Verifica una firma con la clave pública (lo que hace el puente sin internet).
pub fn verificar_con_clave(clave_publica: &[u8; 32], carga: &[u8], firma: &[u8]) -> bool {
    UnparsedPublicKey::new(&ED25519, clave_publica)
        .verify(carga, firma)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn firma() -> FirmaQrEd25519 {
        FirmaQrEd25519::new(&[7; 32]).expect("secreto")
    }

    #[test]
    fn firma_y_verifica_por_copropiedad() {
        let (f, t) = (firma(), TenantId::nuevo());
        let sello = f.firmar(t, b"carga");
        assert_eq!(sello.len(), 64);
        assert!(f.verificar(t, b"carga", &sello));
        assert!(!f.verificar(t, b"otra carga", &sello), "carga alterada");
        assert!(
            !f.verificar(TenantId::nuevo(), b"carga", &sello),
            "otra copropiedad"
        );
        assert!(verificar_con_clave(&f.clave_publica(t), b"carga", &sello));
    }

    #[test]
    fn es_determinista_y_depende_del_secreto() {
        let t = TenantId::nuevo();
        assert_eq!(firma().firmar(t, b"x"), firma().firmar(t, b"x"));
        let otra = FirmaQrEd25519::new(&[8; 32]).expect("secreto");
        assert_ne!(otra.clave_publica(t), firma().clave_publica(t));
    }

    #[test]
    fn rechaza_secretos_cortos() {
        assert_eq!(FirmaQrEd25519::new(&[1; 16]).err(), Some(SecretoQrCorto));
        assert!(format!("{:?}", firma()).contains("oculto"));
    }
}
