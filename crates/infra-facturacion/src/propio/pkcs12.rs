//! Lectura de archivos PKCS#12 (`.p12` / `.pfx`, RFC 7292) con el certificado de firma del
//! facturador (plan 18 fase B2).
//!
//! Solo lee: verifica el MAC con la contraseña, descifra las bolsas y devuelve la llave privada
//! (PKCS#8) y los certificados. La estructura y la derivación de claves son propias (sobre el
//! lector DER del módulo `x509`); los cifrados son de RustCrypto (`aes`, `cbc`, `des`, `rc2`) y
//! PBKDF2 de `ring`: nada de criptografía escrita a mano. Cubre los dos formatos que entregan las
//! entidades de certificación:
//!
//! - moderno (OpenSSL 3): PBES2 con PBKDF2-HMAC-SHA256 y AES-256-CBC, MAC HMAC-SHA256;
//! - heredado (Windows, OpenSSL 1.x): 3DES para la llave, RC2-40 para los certificados, MAC
//!   HMAC-SHA1, con la derivación de claves de PKCS#12 (RFC 7292, apéndice B).
//!
//! Ni la contraseña ni la llave pasan por logs ni por `Debug`.

use std::num::NonZeroU32;

use aes::{Aes128, Aes192, Aes256};
use cbc::Decryptor;
use cbc::cipher::{BlockModeDecrypt, InnerIvInit, KeyIvInit, block_padding::Pkcs7};
use des::TdesEde3;
use hmac::{Hmac, KeyInit, Mac};
use rc2::Rc2;
use sha1::{Digest, Sha1};
use sha2::Sha256;

use super::ErrorMotor;
use super::x509::{ENTERO, Lector, OID, SECUENCIA, oid};

const OCTETOS: u8 = 0x04;
const EXPLICITO_0: u8 = 0xa0;
const IMPLICITO_0: u8 = 0x80;

const DATOS: &str = "1.2.840.113549.1.7.1";
const DATOS_CIFRADOS: &str = "1.2.840.113549.1.7.6";
const BOLSA_LLAVE: &str = "1.2.840.113549.1.12.10.1.1";
const BOLSA_LLAVE_CIFRADA: &str = "1.2.840.113549.1.12.10.1.2";
const BOLSA_CERTIFICADO: &str = "1.2.840.113549.1.12.10.1.3";
const CERTIFICADO_X509: &str = "1.2.840.113549.1.9.22.1";
const PBE_SHA1_3DES: &str = "1.2.840.113549.1.12.1.3";
const PBE_SHA1_RC2_128: &str = "1.2.840.113549.1.12.1.5";
const PBE_SHA1_RC2_40: &str = "1.2.840.113549.1.12.1.6";
const PBES2: &str = "1.2.840.113549.1.5.13";
const PBKDF2: &str = "1.2.840.113549.1.5.12";
const HMAC_SHA1: &str = "1.2.840.113549.2.7";
const HMAC_SHA256: &str = "1.2.840.113549.2.9";
const AES128_CBC: &str = "2.16.840.1.101.3.4.1.2";
const AES192_CBC: &str = "2.16.840.1.101.3.4.1.22";
const AES256_CBC: &str = "2.16.840.1.101.3.4.1.42";
const SHA1: &str = "1.3.14.3.2.26";
const SHA256: &str = "2.16.840.1.101.3.4.2.1";

/// Iteraciones máximas que se aceptan (un archivo malicioso no bloquea el servidor).
const MAXIMO_ITERACIONES: u32 = 1_000_000;

/// Contenido de un PKCS#12.
#[derive(Clone, PartialEq, Eq)]
pub struct ContenidoPkcs12 {
    /// Llave privada en PKCS#8 (DER).
    pub llave_pkcs8: Vec<u8>,
    /// Certificados en DER, en el orden del archivo.
    pub certificados: Vec<Vec<u8>>,
}

impl std::fmt::Debug for ContenidoPkcs12 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContenidoPkcs12")
            .field("llave_pkcs8", &"<oculta>")
            .field("certificados", &self.certificados.len())
            .finish()
    }
}

fn mal_formado() -> ErrorMotor {
    ErrorMotor::Certificado("el archivo no es un PKCS#12 (.p12) legible".into())
}

fn sin_soporte(que: &str) -> ErrorMotor {
    ErrorMotor::Certificado(format!("el .p12 usa un algoritmo no soportado ({que})"))
}

fn contrasena_errada() -> ErrorMotor {
    ErrorMotor::Certificado("la contraseña del certificado no es correcta".into())
}

/// Lee el `.p12` con su contraseña.
///
/// # Errors
///
/// [`ErrorMotor::Certificado`] si el archivo no es legible, usa un algoritmo no soportado, la
/// contraseña no es correcta (MAC inválido) o no trae llave y certificado.
pub fn leer(p12: &[u8], contrasena: &str) -> Result<ContenidoPkcs12, ErrorMotor> {
    let mut pfx = Lector::new(
        Lector::new(p12)
            .esperar(SECUENCIA)
            .ok_or_else(mal_formado)?,
    );
    let version = pfx.esperar(ENTERO).ok_or_else(mal_formado)?;
    if version != [3] {
        return Err(mal_formado());
    }
    let (tipo, contenido) = info_contenido(pfx.esperar(SECUENCIA).ok_or_else(mal_formado)?)?;
    if tipo != DATOS {
        return Err(sin_soporte("contenido firmado con clave pública"));
    }
    let autenticado = octetos(contenido)?;
    match pfx.siguiente() {
        Some((SECUENCIA, mac, _)) => verificar_mac(mac, &autenticado, contrasena)?,
        _ => return Err(mal_formado()),
    }
    let mut salida = ContenidoPkcs12 {
        llave_pkcs8: Vec::new(),
        certificados: Vec::new(),
    };
    let mut seguros = Lector::new(
        Lector::new(&autenticado)
            .esperar(SECUENCIA)
            .ok_or_else(mal_formado)?,
    );
    while !seguros.vacio() {
        let (tipo, contenido) =
            info_contenido(seguros.esperar(SECUENCIA).ok_or_else(mal_formado)?)?;
        let bolsas = match tipo.as_str() {
            DATOS => octetos(contenido)?,
            DATOS_CIFRADOS => datos_cifrados(contenido, contrasena)?,
            otro => return Err(sin_soporte(otro)),
        };
        leer_bolsas(&bolsas, contrasena, &mut salida)?;
    }
    if salida.llave_pkcs8.is_empty() || salida.certificados.is_empty() {
        return Err(ErrorMotor::Certificado(
            "el .p12 no trae la llave privada y su certificado".into(),
        ));
    }
    Ok(salida)
}

/// `ContentInfo ::= SEQUENCE { contentType OID, [0] EXPLICIT contenido }` → (tipo, contenido).
fn info_contenido(valor: &[u8]) -> Result<(String, &[u8]), ErrorMotor> {
    let mut l = Lector::new(valor);
    let tipo = oid(l.esperar(OID).ok_or_else(mal_formado)?).ok_or_else(mal_formado)?;
    let contenido = l.esperar(EXPLICITO_0).ok_or_else(mal_formado)?;
    Ok((tipo, contenido))
}

/// Valor de un OCTET STRING (también en su forma construida, por partes).
fn octetos(tlv: &[u8]) -> Result<Vec<u8>, ErrorMotor> {
    let mut l = Lector::new(tlv);
    match l.siguiente() {
        Some((OCTETOS, v, _)) => Ok(v.to_vec()),
        Some((0x24, v, _)) => unir_partes(v),
        _ => Err(mal_formado()),
    }
}

fn unir_partes(v: &[u8]) -> Result<Vec<u8>, ErrorMotor> {
    let mut l = Lector::new(v);
    let mut todo = Vec::new();
    while !l.vacio() {
        todo.extend_from_slice(l.esperar(OCTETOS).ok_or_else(mal_formado)?);
    }
    Ok(todo)
}

fn entero(v: &[u8]) -> Result<u32, ErrorMotor> {
    if v.is_empty() || v.len() > 5 {
        return Err(mal_formado());
    }
    let mut n: u64 = 0;
    for b in v {
        n = (n << 8) | u64::from(*b);
    }
    u32::try_from(n).map_err(|_| mal_formado())
}

fn iteraciones(v: &[u8]) -> Result<NonZeroU32, ErrorMotor> {
    let n = entero(v)?;
    if n > MAXIMO_ITERACIONES {
        return Err(sin_soporte("demasiadas iteraciones"));
    }
    NonZeroU32::new(n).ok_or_else(mal_formado)
}

/// Contraseña como BMPString (UTF-16 big-endian) con el terminador nulo (RFC 7292 B.1).
fn bmp(contrasena: &str) -> Vec<u8> {
    let mut v: Vec<u8> = contrasena
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    v.extend_from_slice(&[0, 0]);
    v
}

/// Derivación de claves de PKCS#12 (RFC 7292, apéndice B.2). `id`: 1 = clave, 2 = IV, 3 = MAC.
fn kdf_pkcs12<D: Digest + Clone>(
    bloque: usize,
    contrasena: &[u8],
    sal: &[u8],
    iteraciones: u32,
    id: u8,
    largo: usize,
) -> Vec<u8> {
    let repetir = |datos: &[u8]| -> Vec<u8> {
        if datos.is_empty() {
            return Vec::new();
        }
        let n = bloque * datos.len().div_ceil(bloque);
        datos.iter().copied().cycle().take(n).collect()
    };
    let d = vec![id; bloque];
    let mut i = repetir(sal);
    i.extend(repetir(contrasena));
    let mut salida = Vec::with_capacity(largo);
    while salida.len() < largo {
        let mut a = D::new()
            .chain_update(&d)
            .chain_update(&i)
            .finalize()
            .to_vec();
        for _ in 1..iteraciones {
            a = D::digest(&a).to_vec();
        }
        salida.extend_from_slice(&a);
        if salida.len() >= largo {
            break;
        }
        let b: Vec<u8> = a.iter().copied().cycle().take(bloque).collect();
        for trozo in i.chunks_mut(bloque) {
            // Ij = (Ij + B + 1) mod 2^(8·v), como entero big-endian.
            let mut acarreo: u16 = 1;
            for (x, y) in trozo.iter_mut().rev().zip(b.iter().rev()) {
                let suma = u16::from(*x) + u16::from(*y) + acarreo;
                *x = (suma & 0xff) as u8;
                acarreo = suma >> 8;
            }
        }
    }
    salida.truncate(largo);
    salida
}

/// `MacData ::= SEQUENCE { mac DigestInfo, macSalt OCTET STRING, iterations INTEGER DEFAULT 1 }`.
fn verificar_mac(mac: &[u8], datos: &[u8], contrasena: &str) -> Result<(), ErrorMotor> {
    let mut l = Lector::new(mac);
    let mut resumen = Lector::new(l.esperar(SECUENCIA).ok_or_else(mal_formado)?);
    let mut algoritmo = Lector::new(resumen.esperar(SECUENCIA).ok_or_else(mal_formado)?);
    let hash = oid(algoritmo.esperar(OID).ok_or_else(mal_formado)?).ok_or_else(mal_formado)?;
    let esperado = resumen.esperar(OCTETOS).ok_or_else(mal_formado)?;
    let sal = l.esperar(OCTETOS).ok_or_else(mal_formado)?;
    let vueltas = match l.siguiente() {
        Some((ENTERO, v, _)) => iteraciones(v)?.get(),
        None => 1,
        _ => return Err(mal_formado()),
    };
    let clave = bmp(contrasena);
    let valido = match hash.as_str() {
        SHA1 => {
            let k = kdf_pkcs12::<Sha1>(64, &clave, sal, vueltas, 3, 20);
            let mut m = <Hmac<Sha1> as KeyInit>::new_from_slice(&k).map_err(|_| mal_formado())?;
            m.update(datos);
            m.verify_slice(esperado).is_ok()
        }
        SHA256 => {
            let k = kdf_pkcs12::<Sha256>(64, &clave, sal, vueltas, 3, 32);
            let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(&k).map_err(|_| mal_formado())?;
            m.update(datos);
            m.verify_slice(esperado).is_ok()
        }
        otro => return Err(sin_soporte(otro)),
    };
    if valido {
        Ok(())
    } else {
        Err(contrasena_errada())
    }
}

/// `EncryptedData ::= SEQUENCE { version, EncryptedContentInfo }`.
fn datos_cifrados(contenido: &[u8], contrasena: &str) -> Result<Vec<u8>, ErrorMotor> {
    let mut datos = Lector::new(
        Lector::new(contenido)
            .esperar(SECUENCIA)
            .ok_or_else(mal_formado)?,
    );
    datos.esperar(ENTERO).ok_or_else(mal_formado)?;
    let mut info = Lector::new(datos.esperar(SECUENCIA).ok_or_else(mal_formado)?);
    info.esperar(OID).ok_or_else(mal_formado)?;
    let (_, algoritmo, _) = info.siguiente().ok_or_else(mal_formado)?;
    let cifrado = match info.siguiente() {
        Some((IMPLICITO_0, v, _)) => v.to_vec(),
        Some((EXPLICITO_0, v, _)) => unir_partes(v)?,
        _ => return Err(mal_formado()),
    };
    descifrar(algoritmo, &cifrado, contrasena)
}

/// Descifra según el `AlgorithmIdentifier` (su valor, sin la cabecera de la secuencia).
fn descifrar(algoritmo: &[u8], cifrado: &[u8], contrasena: &str) -> Result<Vec<u8>, ErrorMotor> {
    let mut l = Lector::new(algoritmo);
    let id = oid(l.esperar(OID).ok_or_else(mal_formado)?).ok_or_else(mal_formado)?;
    let parametros = l.esperar(SECUENCIA).ok_or_else(mal_formado)?;
    match id.as_str() {
        PBE_SHA1_3DES | PBE_SHA1_RC2_40 | PBE_SHA1_RC2_128 => {
            let mut p = Lector::new(parametros);
            let sal = p.esperar(OCTETOS).ok_or_else(mal_formado)?;
            let vueltas = iteraciones(p.esperar(ENTERO).ok_or_else(mal_formado)?)?.get();
            let clave = bmp(contrasena);
            let largo_clave = match id.as_str() {
                PBE_SHA1_3DES => 24,
                PBE_SHA1_RC2_128 => 16,
                _ => 5,
            };
            let k = kdf_pkcs12::<Sha1>(64, &clave, sal, vueltas, 1, largo_clave);
            let iv = kdf_pkcs12::<Sha1>(64, &clave, sal, vueltas, 2, 8);
            if id == PBE_SHA1_3DES {
                cbc_con::<TdesEde3>(&k, &iv, cifrado)
            } else {
                let rc2 = Rc2::new_with_eff_key_len(&k, largo_clave * 8);
                let iv: [u8; 8] = iv.try_into().map_err(|_| mal_formado())?;
                Decryptor::<Rc2>::inner_iv_init(rc2, &iv.into())
                    .decrypt_padded_vec::<Pkcs7>(cifrado)
                    .map_err(|_| contrasena_errada())
            }
        }
        PBES2 => pbes2(parametros, cifrado, contrasena),
        otro => Err(sin_soporte(otro)),
    }
}

fn cbc_con<C>(clave: &[u8], iv: &[u8], cifrado: &[u8]) -> Result<Vec<u8>, ErrorMotor>
where
    Decryptor<C>: KeyIvInit + BlockModeDecrypt,
    C: cbc::cipher::BlockCipherDecrypt,
{
    Decryptor::<C>::new_from_slices(clave, iv)
        .map_err(|_| mal_formado())?
        .decrypt_padded_vec::<Pkcs7>(cifrado)
        .map_err(|_| contrasena_errada())
}

/// PBES2 (RFC 8018) con PBKDF2 y AES-CBC. OpenSSL toma la contraseña en UTF-8.
fn pbes2(parametros: &[u8], cifrado: &[u8], contrasena: &str) -> Result<Vec<u8>, ErrorMotor> {
    let mut p = Lector::new(parametros);
    let mut kdf = Lector::new(p.esperar(SECUENCIA).ok_or_else(mal_formado)?);
    if oid(kdf.esperar(OID).ok_or_else(mal_formado)?).as_deref() != Some(PBKDF2) {
        return Err(sin_soporte("derivación distinta de PBKDF2"));
    }
    let mut pbkdf2 = Lector::new(kdf.esperar(SECUENCIA).ok_or_else(mal_formado)?);
    let sal = pbkdf2.esperar(OCTETOS).ok_or_else(mal_formado)?;
    let vueltas = iteraciones(pbkdf2.esperar(ENTERO).ok_or_else(mal_formado)?)?;
    let mut prf = HMAC_SHA1.to_owned();
    while !pbkdf2.vacio() {
        match pbkdf2.siguiente().ok_or_else(mal_formado)? {
            (ENTERO, _, _) => {} // largo de la clave: lo fija el cifrado
            (SECUENCIA, v, _) => {
                prf = oid(Lector::new(v).esperar(OID).ok_or_else(mal_formado)?)
                    .ok_or_else(mal_formado)?;
            }
            _ => return Err(mal_formado()),
        }
    }
    let mut esquema = Lector::new(p.esperar(SECUENCIA).ok_or_else(mal_formado)?);
    let cifra = oid(esquema.esperar(OID).ok_or_else(mal_formado)?).ok_or_else(mal_formado)?;
    let iv = esquema.esperar(OCTETOS).ok_or_else(mal_formado)?;
    let largo = match cifra.as_str() {
        AES128_CBC => 16,
        AES192_CBC => 24,
        AES256_CBC => 32,
        otro => return Err(sin_soporte(otro)),
    };
    let algoritmo = match prf.as_str() {
        HMAC_SHA1 => ring::pbkdf2::PBKDF2_HMAC_SHA1,
        HMAC_SHA256 => ring::pbkdf2::PBKDF2_HMAC_SHA256,
        otro => return Err(sin_soporte(otro)),
    };
    let mut clave = vec![0u8; largo];
    ring::pbkdf2::derive(algoritmo, vueltas, sal, contrasena.as_bytes(), &mut clave);
    match largo {
        16 => cbc_con::<Aes128>(&clave, iv, cifrado),
        24 => cbc_con::<Aes192>(&clave, iv, cifrado),
        _ => cbc_con::<Aes256>(&clave, iv, cifrado),
    }
}

/// `SafeContents ::= SEQUENCE OF SafeBag`.
fn leer_bolsas(
    datos: &[u8],
    contrasena: &str,
    salida: &mut ContenidoPkcs12,
) -> Result<(), ErrorMotor> {
    let mut bolsas = Lector::new(
        Lector::new(datos)
            .esperar(SECUENCIA)
            .ok_or_else(mal_formado)?,
    );
    while !bolsas.vacio() {
        let mut bolsa = Lector::new(bolsas.esperar(SECUENCIA).ok_or_else(mal_formado)?);
        let tipo = oid(bolsa.esperar(OID).ok_or_else(mal_formado)?).ok_or_else(mal_formado)?;
        let valor = bolsa.esperar(EXPLICITO_0).ok_or_else(mal_formado)?;
        match tipo.as_str() {
            BOLSA_LLAVE => {
                salida.llave_pkcs8 = Lector::new(valor)
                    .siguiente()
                    .ok_or_else(mal_formado)?
                    .2
                    .to_vec()
            }
            BOLSA_LLAVE_CIFRADA => {
                let mut epki = Lector::new(
                    Lector::new(valor)
                        .esperar(SECUENCIA)
                        .ok_or_else(mal_formado)?,
                );
                let algoritmo = epki.esperar(SECUENCIA).ok_or_else(mal_formado)?;
                let cifrada = epki.esperar(OCTETOS).ok_or_else(mal_formado)?;
                salida.llave_pkcs8 = descifrar(algoritmo, cifrada, contrasena)?;
            }
            BOLSA_CERTIFICADO => {
                let mut c = Lector::new(
                    Lector::new(valor)
                        .esperar(SECUENCIA)
                        .ok_or_else(mal_formado)?,
                );
                let clase = oid(c.esperar(OID).ok_or_else(mal_formado)?).ok_or_else(mal_formado)?;
                if clase == CERTIFICADO_X509 {
                    let der = Lector::new(c.esperar(EXPLICITO_0).ok_or_else(mal_formado)?)
                        .esperar(OCTETOS)
                        .ok_or_else(mal_formado)?;
                    salida.certificados.push(der.to_vec());
                }
            }
            // Otras bolsas (CRL, secretos) no se usan.
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTRASENA: &str = "prueba-123";

    fn fixture(nombre: &str) -> Vec<u8> {
        std::fs::read(format!(
            "{}/tests/fixtures/firma/{nombre}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture")
    }

    fn certificado_esperado() -> Vec<u8> {
        let pem = String::from_utf8(fixture("firmante.crt")).expect("pem");
        let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).expect("base64")
    }

    #[test]
    fn lee_el_formato_moderno_aes_y_sha256() {
        let c = leer(&fixture("firmante_aes.p12"), CONTRASENA).expect("p12");
        assert_eq!(c.certificados.len(), 2, "firmante y CA");
        assert!(c.certificados.contains(&certificado_esperado()));
        assert_eq!(c.llave_pkcs8.first(), Some(&0x30), "PKCS#8 en DER");
    }

    #[test]
    fn lee_el_formato_heredado_3des_rc2_y_sha1() {
        let moderno = leer(&fixture("firmante_aes.p12"), CONTRASENA).expect("p12");
        let heredado = leer(&fixture("firmante_legado.p12"), CONTRASENA).expect("p12");
        assert_eq!(heredado.llave_pkcs8, moderno.llave_pkcs8, "la misma llave");
        assert!(heredado.certificados.contains(&certificado_esperado()));
    }

    #[test]
    fn la_contrasena_errada_se_rechaza_por_el_mac() {
        for archivo in ["firmante_aes.p12", "firmante_legado.p12"] {
            let e = leer(&fixture(archivo), "otra-clave").expect_err("contraseña errada");
            assert!(e.to_string().contains("contraseña"), "{archivo}: {e}");
        }
    }

    #[test]
    fn un_archivo_que_no_es_p12_se_rechaza() {
        assert!(leer(b"no soy un p12", CONTRASENA).is_err());
        assert!(leer(&fixture("firmante.crt"), CONTRASENA).is_err());
    }

    #[test]
    fn el_debug_no_muestra_la_llave() {
        let c = leer(&fixture("firmante_aes.p12"), CONTRASENA).expect("p12");
        assert!(format!("{c:?}").contains("<oculta>"));
    }

    #[test]
    fn el_p12_sirve_para_firmar_con_el_certificado_del_firmante_primero() {
        for archivo in ["firmante_aes.p12", "firmante_legado.p12"] {
            let credencial =
                super::super::firma::CredencialFirma::desde_pkcs12(&fixture(archivo), CONTRASENA)
                    .expect("credencial");
            let resumen = credencial.resumen().expect("resumen");
            assert!(resumen.sujeto.contains("CN="), "{archivo}: {resumen:?}");
            assert_ne!(resumen.sujeto, resumen.emisor, "el firmante, no la CA");
        }
    }

    #[test]
    fn la_contrasena_en_bmp_lleva_terminador() {
        assert_eq!(bmp("ab"), vec![0, b'a', 0, b'b', 0, 0]);
    }
}
