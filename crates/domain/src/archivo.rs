//! Archivos y adjuntos: tipo verificado por contenido, clasificación, nombre y clave lógica.
//!
//! Lo usan fotos de tickets, reglamento de propiedad horizontal, actas, contratos, creativos de
//! publicidad… Ningún módulo toca rutas de disco: todo pasa por el puerto `AlmacenArchivos` con
//! una [`ClaveArchivo`], que es igual en disco local y en S3. Así migrar de almacén es copiar
//! claves, no reescribir rutas.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ArchivoId, DominioError, TenantId};

/// Tipos de archivo admitidos (lista blanca). Se detecta por los primeros bytes del contenido
/// (*magic bytes*), **nunca** por la extensión ni por el `Content-Type` que envía el cliente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TipoArchivo {
    Pdf,
    Jpeg,
    Png,
    Webp,
    /// Fotos de iPhone.
    Heic,
}

enum_texto!(TipoArchivo {
    Pdf => "application/pdf",
    Jpeg => "image/jpeg",
    Png => "image/png",
    Webp => "image/webp",
    Heic => "image/heic",
});

const MB: u64 = 1024 * 1024;

impl TipoArchivo {
    /// Tamaño máximo de cualquier archivo admitido (sirve de límite del cuerpo HTTP).
    pub const TAMANO_MAXIMO_GLOBAL: u64 = 25 * MB;

    /// Detecta el tipo por el contenido. `None` si no es un tipo admitido.
    pub fn detectar(contenido: &[u8]) -> Option<Self> {
        const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        // Marcas ISO-BMFF de HEIC/HEIF de imágenes (no se admite `mif1` a secas: también es AVIF).
        const MARCAS_HEIC: [&[u8]; 6] = [b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx"];

        if contenido.starts_with(b"%PDF-") {
            Some(Self::Pdf)
        } else if contenido.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if contenido.starts_with(PNG) {
            Some(Self::Png)
        } else if contenido.len() >= 12
            && &contenido[..4] == b"RIFF"
            && &contenido[8..12] == b"WEBP"
        {
            Some(Self::Webp)
        } else if contenido.len() >= 12
            && &contenido[4..8] == b"ftyp"
            && MARCAS_HEIC.contains(&&contenido[8..12])
        {
            Some(Self::Heic)
        } else {
            None
        }
    }

    /// Tamaño máximo en bytes para este tipo.
    pub const fn tamano_maximo(self) -> u64 {
        match self {
            Self::Pdf => Self::TAMANO_MAXIMO_GLOBAL,
            Self::Jpeg | Self::Png | Self::Webp | Self::Heic => 10 * MB,
        }
    }

    /// Extensión habitual (para el nombre de descarga si hiciera falta).
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Heic => "heic",
        }
    }

    /// Valida un contenido subido: no vacío, de un tipo admitido y dentro del tamaño máximo.
    ///
    /// # Errors
    ///
    /// [`DominioError::ArchivoVacio`], [`DominioError::TipoArchivoNoAdmitido`] o
    /// [`DominioError::ArchivoDemasiadoGrande`].
    pub fn validar(contenido: &[u8]) -> Result<Self, DominioError> {
        if contenido.is_empty() {
            return Err(DominioError::ArchivoVacio);
        }
        let tipo = Self::detectar(contenido).ok_or(DominioError::TipoArchivoNoAdmitido)?;
        if contenido.len() as u64 > tipo.tamano_maximo() {
            return Err(DominioError::ArchivoDemasiadoGrande {
                maximo_bytes: tipo.tamano_maximo(),
            });
        }
        Ok(tipo)
    }
}

/// Sensibilidad del archivo (Ley 1581): define quién puede verlo y cuánto se protege.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClasificacionArchivo {
    /// Cualquiera con acceso a la copropiedad (reglamento, circulares).
    Publico,
    /// Solo la administración y los órganos de la copropiedad (contratos, cotizaciones).
    Interno,
    /// Contiene datos personales (fotos de un ticket, carné de vacunas de una mascota).
    Personal,
    /// Datos sensibles (art. 5 Ley 1581): salud, biometría, menores de edad.
    Sensible,
}

enum_texto!(ClasificacionArchivo {
    Publico => "publico",
    Interno => "interno",
    Personal => "personal",
    Sensible => "sensible",
});

/// Dónde están físicamente los bytes de un archivo. El texto es contrato con `archivos.almacen`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Almacen {
    /// Disco del VPS (`ARCHIVOS_DIR`).
    Local,
    /// Almacenamiento compatible con S3 (previsto).
    S3,
}

enum_texto!(Almacen {
    Local => "local",
    S3 => "s3",
});

/// Nombre original del archivo, para mostrarlo y para la descarga: sin separadores de ruta ni
/// caracteres de control, de 1 a 255 caracteres.
///
/// Puede contener datos personales ("cedula_juan_perez.pdf"): `Debug` lo oculta y no se audita.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NombreArchivo(String);

impl NombreArchivo {
    pub const LONGITUD_MAXIMA: usize = 255;

    /// # Errors
    ///
    /// [`DominioError::NombreArchivoInvalido`] si queda vacío, es demasiado largo o contiene
    /// `/`, `\` o caracteres de control.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let valor = valor.trim();
        let valido = !valor.is_empty()
            && valor.chars().count() <= Self::LONGITUD_MAXIMA
            && !valor
                .chars()
                .any(|c| c.is_control() || c == '/' || c == '\\');
        if valido {
            Ok(Self(valor.to_owned()))
        } else {
            Err(DominioError::NombreArchivoInvalido)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for NombreArchivo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NombreArchivo(<oculto>)")
    }
}

impl TryFrom<String> for NombreArchivo {
    type Error = DominioError;

    fn try_from(valor: String) -> Result<Self, Self::Error> {
        Self::new(&valor)
    }
}

impl From<NombreArchivo> for String {
    fn from(n: NombreArchivo) -> Self {
        n.0
    }
}

/// Clave lógica de un archivo en cualquier almacén: `{tenant}/{aa}/{bb}/{archivo}`, donde `aa` y
/// `bb` son los cuatro últimos dígitos hexadecimales del id (parte aleatoria del UUID v7) y
/// reparten los archivos en carpetas pequeñas.
///
/// Solo se construye a partir de ids o validando el formato exacto: nunca contiene `..`, rutas
/// absolutas ni texto del usuario, así que es segura como ruta de disco y como clave de S3.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClaveArchivo(String);

impl ClaveArchivo {
    /// Clave del archivo `archivo` de la copropiedad `tenant`.
    pub fn new(tenant: TenantId, archivo: ArchivoId) -> Self {
        let id = archivo.as_uuid().simple().to_string();
        Self(format!(
            "{}/{}/{}/{}",
            tenant.as_uuid().hyphenated(),
            &id[28..30],
            &id[30..32],
            archivo.as_uuid().hyphenated()
        ))
    }

    /// Valida una clave leída de la base de datos o de una URL firmada.
    ///
    /// # Errors
    ///
    /// [`DominioError::ClaveArchivoInvalida`] si no tiene exactamente el formato de
    /// [`ClaveArchivo::new`].
    pub fn parse(valor: &str) -> Result<Self, DominioError> {
        let mut partes = valor.split('/');
        let (Some(tenant), Some(_), Some(_), Some(archivo), None) = (
            partes.next(),
            partes.next(),
            partes.next(),
            partes.next(),
            partes.next(),
        ) else {
            return Err(DominioError::ClaveArchivoInvalida);
        };
        let tenant = Uuid::try_parse(tenant).map_err(|_| DominioError::ClaveArchivoInvalida)?;
        let archivo = Uuid::try_parse(archivo).map_err(|_| DominioError::ClaveArchivoInvalida)?;
        let clave = Self::new(TenantId::from_uuid(tenant), ArchivoId::from_uuid(archivo));
        // Compara con la forma canónica: descarta mayúsculas, llaves y carpetas que no cuadren.
        if clave.0 == valor {
            Ok(clave)
        } else {
            Err(DominioError::ClaveArchivoInvalida)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClaveArchivo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn con_relleno(cabecera: &[u8]) -> Vec<u8> {
        let mut v = cabecera.to_vec();
        v.extend_from_slice(&[0u8; 32]);
        v
    }

    #[test]
    fn detecta_cada_tipo_admitido_por_su_contenido() {
        let casos: [(&[u8], TipoArchivo); 5] = [
            (b"%PDF-1.7\n", TipoArchivo::Pdf),
            (&[0xFF, 0xD8, 0xFF, 0xE0], TipoArchivo::Jpeg),
            (
                &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
                TipoArchivo::Png,
            ),
            (b"RIFF\x24\0\0\0WEBPVP8 ", TipoArchivo::Webp),
            (b"\0\0\0\x18ftypheic\0\0\0\0", TipoArchivo::Heic),
        ];
        for (cabecera, tipo) in casos {
            assert_eq!(TipoArchivo::detectar(&con_relleno(cabecera)), Some(tipo));
        }
    }

    #[test]
    fn rechaza_tipos_no_admitidos_aunque_se_llamen_pdf() {
        for contenido in [
            &b"MZ\x90\0ejecutable de windows"[..],
            b"<html><script>alert(1)</script>",
            b"PK\x03\x04 un zip o un docx",
            b"\0\0\0\x18ftypavif\0\0\0\0", // AVIF: no está en la lista blanca
            b"GIF89a",
        ] {
            assert_eq!(TipoArchivo::detectar(contenido), None);
            assert_eq!(
                TipoArchivo::validar(contenido),
                Err(DominioError::TipoArchivoNoAdmitido)
            );
        }
    }

    #[test]
    fn valida_vacio_y_tamano_maximo_por_tipo() {
        assert_eq!(TipoArchivo::validar(&[]), Err(DominioError::ArchivoVacio));

        let mut png = con_relleno(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        png.resize(10 * 1024 * 1024 + 1, 0);
        assert_eq!(
            TipoArchivo::validar(&png),
            Err(DominioError::ArchivoDemasiadoGrande {
                maximo_bytes: 10 * 1024 * 1024
            })
        );

        // El mismo tamaño en PDF sí cabe.
        let mut pdf = b"%PDF-1.7\n".to_vec();
        pdf.resize(png.len(), 0);
        assert_eq!(TipoArchivo::validar(&pdf), Ok(TipoArchivo::Pdf));
    }

    #[test]
    fn texto_persistido_es_el_tipo_mime() {
        assert_eq!(TipoArchivo::Pdf.as_str(), "application/pdf");
        assert_eq!(TipoArchivo::try_from("image/heic"), Ok(TipoArchivo::Heic));
        assert!(TipoArchivo::try_from("text/html").is_err());
    }

    #[test]
    fn clave_tiene_tenant_carpetas_e_id() {
        let tenant = TenantId::from_uuid(
            Uuid::try_parse("0190c1a2-0000-7000-8000-000000000001").expect("uuid"),
        );
        let archivo = ArchivoId::from_uuid(
            Uuid::try_parse("0190c1a2-1111-7000-8000-00000000abcd").expect("uuid"),
        );

        let clave = ClaveArchivo::new(tenant, archivo);

        assert_eq!(
            clave.as_str(),
            "0190c1a2-0000-7000-8000-000000000001/ab/cd/0190c1a2-1111-7000-8000-00000000abcd"
        );
        assert_eq!(ClaveArchivo::parse(clave.as_str()), Ok(clave));
    }

    #[test]
    fn clave_rechaza_cualquier_otra_forma() {
        let valida = ClaveArchivo::new(TenantId::nuevo(), ArchivoId::nuevo());
        let mayusculas = valida.as_str().to_uppercase();
        let sin_carpetas = valida.as_str().replacen('/', "/00/00/", 1);
        for invalida in [
            "",
            "../../etc/passwd",
            "/0190c1a2-0000-7000-8000-000000000001/ab/cd/x",
            "0190c1a2-0000-7000-8000-000000000001/00/00/0190c1a2-1111-7000-8000-00000000abcd",
            mayusculas.as_str(),
            sin_carpetas.as_str(),
        ] {
            assert_eq!(
                ClaveArchivo::parse(invalida),
                Err(DominioError::ClaveArchivoInvalida),
                "{invalida:?}"
            );
        }
    }

    #[test]
    fn nombre_sin_rutas_ni_control_y_oculto_en_debug() {
        let nombre = NombreArchivo::new("  Acta asamblea 2026.pdf ").expect("válido");
        assert_eq!(nombre.as_str(), "Acta asamblea 2026.pdf");
        assert!(!format!("{nombre:?}").contains("Acta"));

        for invalido in ["", "   ", "../secreto.pdf", "a\\b.pdf", "linea\nnueva.pdf"] {
            assert_eq!(
                NombreArchivo::new(invalido),
                Err(DominioError::NombreArchivoInvalido),
                "{invalido:?}"
            );
        }
        assert!(NombreArchivo::new(&"ñ".repeat(255)).is_ok());
        assert!(NombreArchivo::new(&"ñ".repeat(256)).is_err());
    }
}
