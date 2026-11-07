//! Criptografía de cuentas: implementa los puertos [`HashContrasenas`] (Argon2id) y
//! [`SegundoFactor`] (TOTP, RFC 6238, compatible con Google Authenticator, Microsoft
//! Authenticator, Authy, 1Password…).

mod totp;

use application::{
    AppError, AppResult,
    puertos::{HashContrasenas, SegundoFactor},
};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use chrono::{DateTime, Utc};
use domain::ContrasenaNueva;

pub use totp::{base32, codigo_totp};

/// Argon2id con los parámetros por defecto de la librería (19 MiB, 2 iteraciones: la
/// recomendación de OWASP). El trabajo corre en `spawn_blocking` para no frenar a Tokio.
#[derive(Debug, Clone, Copy, Default)]
pub struct Argon2Contrasenas;

/// Hash real de una contraseña que nadie tiene, calculado una sola vez: verificar contra él
/// cuesta lo mismo que contra una cuenta existente.
fn hash_ficticio() -> &'static str {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| {
        Argon2::default()
            .hash_password(b"cuenta-inexistente")
            .map(|h| h.to_string())
            .unwrap_or_default()
    })
}

#[derive(Debug, thiserror::Error)]
#[error("no se pudo calcular el hash de la contraseña: {0}")]
struct ErrorHash(String);

impl HashContrasenas for Argon2Contrasenas {
    async fn hash(&self, contrasena: &ContrasenaNueva) -> AppResult<String> {
        let texto = contrasena.expose().to_owned();
        tokio::task::spawn_blocking(move || {
            Argon2::default()
                .hash_password(texto.as_bytes())
                .map(|h| h.to_string())
                .map_err(|e| AppError::infra(ErrorHash(e.to_string())))
        })
        .await
        .map_err(AppError::infra)?
    }

    async fn verificar(&self, contrasena: &str, hash: &str) -> bool {
        let (texto, hash) = (contrasena.to_owned(), hash.to_owned());
        tokio::task::spawn_blocking(move || {
            PasswordHash::new(&hash).is_ok_and(|h| {
                Argon2::default()
                    .verify_password(texto.as_bytes(), &h)
                    .is_ok()
            })
        })
        .await
        .unwrap_or(false)
    }

    async fn igualar_tiempo(&self, contrasena: &str) {
        let hash = tokio::task::spawn_blocking(|| hash_ficticio().to_owned())
            .await
            .unwrap_or_default();
        let _ = self.verificar(contrasena, &hash).await;
    }
}

/// TOTP de 6 dígitos cada 30 s con HMAC-SHA1 (lo que esperan las apps autenticadoras).
#[derive(Debug, Clone)]
pub struct Totp {
    emisor: String,
}

impl Totp {
    /// `emisor` es el nombre que muestra la app autenticadora (p. ej. "DAD Copropiedad").
    pub fn new(emisor: &str) -> Self {
        Self {
            emisor: emisor.to_owned(),
        }
    }
}

impl SegundoFactor for Totp {
    fn generar_secreto(&self) -> String {
        let mut bytes = [0u8; 20];
        // Sin aleatoriedad del sistema no hay seguridad posible: se detiene el proceso.
        getrandom::fill(&mut bytes).expect("el sistema operativo debe entregar aleatoriedad");
        base32::codificar(&bytes)
    }

    fn uri(&self, secreto: &str, cuenta: &str) -> String {
        let emisor = codificar_uri(&self.emisor);
        format!(
            "otpauth://totp/{emisor}:{}?secret={secreto}&issuer={emisor}&algorithm=SHA1&digits=6&period=30",
            codificar_uri(cuenta)
        )
    }

    fn verificar(&self, secreto: &str, codigo: &str, ahora: DateTime<Utc>) -> bool {
        let Some(clave) = base32::decodificar(secreto) else {
            return false;
        };
        let paso = ahora.timestamp().div_euclid(30);
        let codigo = codigo.trim();
        // ±1 paso: tolera relojes desfasados hasta 30 s.
        (-1..=1).any(|d| codigo_totp(&clave, paso + d).is_some_and(|c| c == codigo))
    }
}

fn codificar_uri(texto: &str) -> String {
    texto
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'@') {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[tokio::test]
    async fn argon2id_verifica_la_correcta_y_rechaza_las_demas() {
        let h = Argon2Contrasenas;
        let c = ContrasenaNueva::new("una frase bien larga").expect("válida");

        let hash = h.hash(&c).await.expect("hash");

        assert!(hash.starts_with("$argon2id$"));
        assert!(h.verificar("una frase bien larga", &hash).await);
        assert!(!h.verificar("otra frase bien larga", &hash).await);
        assert!(!h.verificar("una frase bien larga", "no-es-un-hash").await);
        h.igualar_tiempo("cualquiera").await;
        assert!(
            hash_ficticio().starts_with("$argon2id$"),
            "el ficticio debe ser un hash real"
        );
    }

    #[test]
    fn totp_acepta_el_codigo_actual_y_los_vecinos() {
        let totp = Totp::new("DAD Copropiedad");
        let secreto = totp.generar_secreto();
        let ahora = Utc
            .with_ymd_and_hms(2026, 11, 6, 12, 0, 0)
            .single()
            .expect("fecha");
        let clave = base32::decodificar(&secreto).expect("base32");
        let paso = ahora.timestamp() / 30;
        let actual = codigo_totp(&clave, paso).expect("código");
        let viejo = codigo_totp(&clave, paso - 2).expect("código");

        assert!(totp.verificar(&secreto, &actual, ahora));
        assert!(totp.verificar(&secreto, &actual, ahora + chrono::Duration::seconds(30)));
        assert!(!totp.verificar(&secreto, &viejo, ahora));
        assert!(!totp.verificar("no-base32!", &actual, ahora));
        assert!(
            totp.uri(&secreto, "ana@correo.co")
                .starts_with("otpauth://totp/DAD%20Copropiedad:ana@correo.co?secret=")
        );
    }
}
