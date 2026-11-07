//! Enlaces de descarga firmados para el almacén local: HMAC-SHA256 sobre la clave, el tipo, el
//! nombre y el vencimiento. Quien tenga el enlace puede descargar hasta que venza, sin otra
//! credencial (igual que una URL prefirmada de S3); por eso la vigencia es corta y el permiso se
//! verifica antes de generarlo.

use std::fmt::Write as _;

use application::puertos::archivos::DescargaArchivo;
use chrono::{DateTime, Utc};
use domain::{ClaveArchivo, NombreArchivo, TipoArchivo};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

/// Ruta del endpoint de la API que sirve los enlaces del almacén local.
pub const RUTA_DESCARGA: &str = "/api/v1/archivos/descarga";

/// Longitud mínima del secreto de firma (256 bits).
pub const LONGITUD_MINIMA_SECRETO: usize = 32;

/// Versión del formato firmado: permite cambiarlo sin aceptar enlaces viejos por error.
const VERSION: &str = "v1";

/// Errores al crear el firmante.
#[derive(Debug, thiserror::Error)]
pub enum ErrorFirmante {
    #[error("el secreto de firma debe tener al menos {LONGITUD_MINIMA_SECRETO} bytes")]
    SecretoCorto,
    #[error("la URL pública debe empezar por http:// o https://")]
    UrlBase,
}

/// Por qué se rechaza un enlace. El cliente recibe siempre la misma respuesta; la variante
/// queda para las pruebas y los logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ErrorEnlace {
    #[error("la firma del enlace no es válida")]
    FirmaInvalida,
    #[error("el enlace venció")]
    Vencido,
}

/// Parámetros de un enlace de descarga, tal como llegan en la consulta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParametrosDescarga {
    pub clave: String,
    pub tipo: String,
    pub nombre: String,
    /// Vencimiento en segundos Unix.
    pub vence: i64,
    /// HMAC en hexadecimal.
    pub firma: String,
}

/// Enlace verificado: qué servir y con qué cabeceras.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescargaVerificada {
    pub clave: ClaveArchivo,
    pub tipo: TipoArchivo,
    pub nombre: NombreArchivo,
}

/// Firma y verifica enlaces de descarga.
#[derive(Clone)]
pub struct FirmanteEnlaces {
    secreto: Vec<u8>,
    url_base: String,
}

impl std::fmt::Debug for FirmanteEnlaces {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FirmanteEnlaces")
            .field("secreto", &"<oculto>")
            .field("url_base", &self.url_base)
            .finish()
    }
}

impl FirmanteEnlaces {
    /// `url_base` es la URL pública de la API (p. ej. `https://api.dominio.co`).
    ///
    /// # Errors
    ///
    /// [`ErrorFirmante`] si el secreto es corto o la URL no es http(s).
    pub fn new(secreto: &[u8], url_base: &str) -> Result<Self, ErrorFirmante> {
        if secreto.len() < LONGITUD_MINIMA_SECRETO {
            return Err(ErrorFirmante::SecretoCorto);
        }
        if !(url_base.starts_with("http://") || url_base.starts_with("https://")) {
            return Err(ErrorFirmante::UrlBase);
        }
        Ok(Self {
            secreto: secreto.to_vec(),
            url_base: url_base.trim_end_matches('/').to_owned(),
        })
    }

    /// URL completa del enlace.
    pub fn firmar(&self, d: &DescargaArchivo) -> String {
        let vence = d.vence_en.timestamp();
        let firma =
            hex::encode(self.mac(d.clave.as_str(), d.tipo.as_str(), d.nombre.as_str(), vence));
        format!(
            "{}{RUTA_DESCARGA}?clave={}&tipo={}&nombre={}&vence={vence}&firma={firma}",
            self.url_base,
            codificar_porcentual(d.clave.as_str()),
            codificar_porcentual(d.tipo.as_str()),
            codificar_porcentual(d.nombre.as_str()),
        )
    }

    /// Verifica la firma (en tiempo constante) y el vencimiento.
    ///
    /// # Errors
    ///
    /// [`ErrorEnlace::FirmaInvalida`] si algún parámetro fue alterado o no es válido;
    /// [`ErrorEnlace::Vencido`] si `ahora` ya pasó el vencimiento.
    pub fn verificar(
        &self,
        p: &ParametrosDescarga,
        ahora: DateTime<Utc>,
    ) -> Result<DescargaVerificada, ErrorEnlace> {
        let firma = hex::decode(&p.firma).map_err(|_| ErrorEnlace::FirmaInvalida)?;
        self.calcular(&p.clave, &p.tipo, &p.nombre, p.vence)
            .verify_slice(&firma)
            .map_err(|_| ErrorEnlace::FirmaInvalida)?;
        if ahora.timestamp() >= p.vence {
            return Err(ErrorEnlace::Vencido);
        }
        // Firmado por nosotros: si no parsea es un enlace de otra versión o un bug.
        Ok(DescargaVerificada {
            clave: ClaveArchivo::parse(&p.clave).map_err(|_| ErrorEnlace::FirmaInvalida)?,
            tipo: TipoArchivo::try_from(p.tipo.as_str()).map_err(|_| ErrorEnlace::FirmaInvalida)?,
            nombre: NombreArchivo::new(&p.nombre).map_err(|_| ErrorEnlace::FirmaInvalida)?,
        })
    }

    fn mac(&self, clave: &str, tipo: &str, nombre: &str, vence: i64) -> Vec<u8> {
        self.calcular(clave, tipo, nombre, vence)
            .finalize()
            .into_bytes()
            .to_vec()
    }

    fn calcular(&self, clave: &str, tipo: &str, nombre: &str, vence: i64) -> Hmac<Sha256> {
        // Separador `\n`: ni la clave, ni el tipo, ni un `NombreArchivo` válido lo contienen,
        // así que no se pueden correr campos de uno a otro.
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secreto)
            .expect("HMAC acepta claves de cualquier longitud");
        mac.update(format!("{VERSION}\n{clave}\n{tipo}\n{vence}\n{nombre}").as_bytes());
        mac
    }
}

/// Codificación porcentual de un componente de URL o de `filename*` (RFC 3986: solo pasan los
/// caracteres no reservados).
pub fn codificar_porcentual(texto: &str) -> String {
    let mut salida = String::with_capacity(texto.len());
    for byte in texto.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            salida.push(char::from(byte));
        } else {
            let _ = write!(salida, "%{byte:02X}");
        }
    }
    salida
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use domain::{ArchivoId, TenantId};

    use super::*;

    const SECRETO: &[u8] = b"secreto-de-pruebas-de-32-bytes!!";

    fn ahora() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 6, 15, 0, 0)
            .single()
            .expect("fecha válida")
    }

    fn descarga() -> DescargaArchivo {
        DescargaArchivo {
            clave: ClaveArchivo::new(TenantId::nuevo(), ArchivoId::nuevo()),
            tipo: TipoArchivo::Pdf,
            nombre: NombreArchivo::new("Acta año 2026 & anexos.pdf").expect("válido"),
            vence_en: ahora() + chrono::Duration::minutes(5),
        }
    }

    fn firmante() -> FirmanteEnlaces {
        FirmanteEnlaces::new(SECRETO, "https://api.dad.co/").expect("válido")
    }

    /// Lee los parámetros de una URL firmada (decodificación porcentual mínima para pruebas).
    fn parametros(url: &str) -> ParametrosDescarga {
        let consulta = url.split_once('?').expect("tiene consulta").1;
        let valor = |nombre: &str| {
            let crudo = consulta
                .split('&')
                .find_map(|par| par.strip_prefix(&format!("{nombre}=")))
                .expect("parámetro presente");
            let mut bytes = Vec::new();
            let mut i = 0;
            let b = crudo.as_bytes();
            while i < b.len() {
                if b[i] == b'%' {
                    bytes.push(u8::from_str_radix(&crudo[i + 1..i + 3], 16).expect("hex"));
                    i += 3;
                } else {
                    bytes.push(b[i]);
                    i += 1;
                }
            }
            String::from_utf8(bytes).expect("utf-8")
        };
        ParametrosDescarga {
            clave: valor("clave"),
            tipo: valor("tipo"),
            nombre: valor("nombre"),
            vence: valor("vence").parse().expect("número"),
            firma: valor("firma"),
        }
    }

    #[test]
    fn enlace_firmado_se_verifica_y_devuelve_lo_que_hay_que_servir() {
        let d = descarga();
        let url = firmante().firmar(&d);

        assert!(url.starts_with("https://api.dad.co/api/v1/archivos/descarga?clave="));
        assert!(
            url.contains("nombre=Acta%20a%C3%B1o%202026%20%26%20anexos.pdf"),
            "{url}"
        );
        let verificada = firmante()
            .verificar(&parametros(&url), ahora())
            .expect("válido");
        assert_eq!(
            verificada,
            DescargaVerificada {
                clave: d.clave,
                tipo: d.tipo,
                nombre: d.nombre,
            }
        );
    }

    #[test]
    fn cualquier_parametro_alterado_invalida_la_firma() {
        let url = firmante().firmar(&descarga());
        let original = parametros(&url);
        let otra_clave = ClaveArchivo::new(TenantId::nuevo(), ArchivoId::nuevo());
        let alteraciones = [
            ParametrosDescarga {
                clave: otra_clave.as_str().to_owned(),
                ..original.clone()
            },
            ParametrosDescarga {
                tipo: "image/png".into(),
                ..original.clone()
            },
            ParametrosDescarga {
                nombre: "otro.pdf".into(),
                ..original.clone()
            },
            ParametrosDescarga {
                vence: original.vence + 3600,
                ..original.clone()
            },
            ParametrosDescarga {
                firma: "no-es-hex".into(),
                ..original.clone()
            },
        ];
        for alterado in alteraciones {
            assert_eq!(
                firmante().verificar(&alterado, ahora()),
                Err(ErrorEnlace::FirmaInvalida),
                "{alterado:?}"
            );
        }
        let otro_secreto =
            FirmanteEnlaces::new(b"otro-secreto-de-pruebas-32-bytes", "https://api.dad.co")
                .expect("válido");
        assert_eq!(
            otro_secreto.verificar(&original, ahora()),
            Err(ErrorEnlace::FirmaInvalida)
        );
    }

    #[test]
    fn enlace_vencido_se_rechaza() {
        let p = parametros(&firmante().firmar(&descarga()));

        assert!(
            firmante()
                .verificar(&p, ahora() + chrono::Duration::minutes(4))
                .is_ok()
        );
        assert_eq!(
            firmante().verificar(&p, ahora() + chrono::Duration::minutes(5)),
            Err(ErrorEnlace::Vencido)
        );
    }

    #[test]
    fn configuracion_insegura_se_rechaza() {
        assert!(matches!(
            FirmanteEnlaces::new(b"corto", "https://api.dad.co"),
            Err(ErrorFirmante::SecretoCorto)
        ));
        assert!(matches!(
            FirmanteEnlaces::new(SECRETO, "api.dad.co"),
            Err(ErrorFirmante::UrlBase)
        ));
        assert!(!format!("{:?}", firmante()).contains("secreto-de-pruebas"));
    }
}
