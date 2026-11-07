//! TOTP (RFC 6238) sobre HOTP (RFC 4226) y Base32 (RFC 4648) para los secretos.

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

/// Código HOTP de 6 dígitos para el contador `paso` (paso = segundos Unix / 30).
pub fn codigo_totp(clave: &[u8], paso: i64) -> Option<String> {
    let contador = u64::try_from(paso).ok()?;
    let mut mac = Hmac::<Sha1>::new_from_slice(clave).ok()?;
    mac.update(&contador.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let desfase = usize::from(h[h.len() - 1] & 0x0f);
    let binario = (u32::from(h[desfase]) & 0x7f) << 24
        | u32::from(h[desfase + 1]) << 16
        | u32::from(h[desfase + 2]) << 8
        | u32::from(h[desfase + 3]);
    Some(format!("{:06}", binario % 1_000_000))
}

/// Base32 sin relleno (alfabeto RFC 4648), el formato de los secretos TOTP.
pub mod base32 {
    const ALFABETO: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

    pub fn codificar(datos: &[u8]) -> String {
        let mut salida = String::new();
        let (mut buffer, mut bits) = (0u32, 0u32);
        for &b in datos {
            buffer = (buffer << 8) | u32::from(b);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                salida.push(char::from(ALFABETO[((buffer >> bits) & 31) as usize]));
            }
        }
        if bits > 0 {
            salida.push(char::from(ALFABETO[((buffer << (5 - bits)) & 31) as usize]));
        }
        salida
    }

    pub fn decodificar(texto: &str) -> Option<Vec<u8>> {
        let mut salida = Vec::new();
        let (mut buffer, mut bits) = (0u32, 0u32);
        for c in texto.trim_end_matches('=').bytes() {
            let c = c.to_ascii_uppercase();
            let valor = ALFABETO.iter().position(|&a| a == c)?;
            buffer = (buffer << 5) | u32::try_from(valor).ok()?;
            bits += 5;
            if bits >= 8 {
                bits -= 8;
                salida.push(u8::try_from((buffer >> bits) & 0xff).ok()?);
            }
        }
        Some(salida)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectores_de_prueba_del_rfc_6238() {
        // Apéndice B del RFC 6238 (SHA-1, clave "12345678901234567890"): 8 dígitos; aquí se
        // comparan los 6 últimos.
        let clave = b"12345678901234567890";
        for (segundos, esperado) in [
            (59, "287082"),
            (1_111_111_109, "081804"),
            (2_000_000_000, "279037"),
        ] {
            assert_eq!(
                codigo_totp(clave, segundos / 30).as_deref(),
                Some(esperado),
                "{segundos}"
            );
        }
    }

    #[test]
    fn base32_ida_y_vuelta() {
        assert_eq!(base32::codificar(b"foobar"), "MZXW6YTBOI");
        assert_eq!(
            base32::decodificar("MZXW6YTBOI").as_deref(),
            Some(&b"foobar"[..])
        );
        assert_eq!(
            base32::decodificar("mzxw6ytboi======").as_deref(),
            Some(&b"foobar"[..])
        );
        assert!(base32::decodificar("1!").is_none());
    }
}
