//! Tokens y códigos aleatorios. Se entregan al cliente una sola vez; en la base de datos solo
//! queda su SHA-256 ([`hash_secreto`]).

use sha2::{Digest, Sha256};

use crate::{AppError, AppResult};

/// Token opaco de 256 bits en hexadecimal.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si el sistema operativo no entrega aleatoriedad.
pub fn token_aleatorio() -> AppResult<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| AppError::infra(std::io::Error::other(e.to_string())))?;
    Ok(hex::encode(bytes))
}

/// Código numérico de 6 dígitos (uniforme: descarta valores que sesgarían el módulo).
///
/// # Errors
///
/// [`AppError::Infraestructura`] si el sistema operativo no entrega aleatoriedad.
pub fn codigo_numerico() -> AppResult<String> {
    const LIMITE: u32 = u32::MAX - (u32::MAX % 1_000_000);
    loop {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes)
            .map_err(|e| AppError::infra(std::io::Error::other(e.to_string())))?;
        let n = u32::from_le_bytes(bytes);
        if n < LIMITE {
            return Ok(format!("{:06}", n % 1_000_000));
        }
    }
}

/// SHA-256 en hexadecimal de un token o código (lo único que se guarda).
pub fn hash_secreto(secreto: &str) -> String {
    hex::encode(Sha256::digest(secreto.as_bytes()))
}

/// Compara dos hashes en tiempo constante.
pub fn iguales(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_y_codigos_aleatorios() {
        let a = token_aleatorio().expect("token");
        let b = token_aleatorio().expect("token");
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        let c = codigo_numerico().expect("código");
        assert_eq!(c.len(), 6);
        assert!(c.chars().all(|x| x.is_ascii_digit()));
        assert_eq!(hash_secreto("x").len(), 64);
        assert!(iguales(&hash_secreto("x"), &hash_secreto("x")));
        assert!(!iguales(&hash_secreto("x"), &hash_secreto("y")));
    }
}
