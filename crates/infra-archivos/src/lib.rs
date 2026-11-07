//! Almacenes de archivos. Implementan el puerto [`application::puertos::AlmacenArchivos`].
//!
//! - [`AlmacenLocal`]: disco del VPS bajo `ARCHIVOS_DIR`. Las descargas salen por el endpoint
//!   [`RUTA_DESCARGA`] de la API con enlaces firmados ([`FirmanteEnlaces`]).
//! - S3 (previsto): mismo puerto, URL prefirmada del proveedor. Cambiar de almacén está descrito
//!   en el ADR 0003 y en [`application::casos_uso::archivos::migrar_lote`].
//!
//! Las rutas en disco salen siempre de una [`ClaveArchivo`] validada: nunca de texto del usuario.

mod firma;

use std::{
    io,
    path::{Path, PathBuf},
};

use application::{
    AppError, AppResult,
    puertos::{AlmacenArchivos, archivos::DescargaArchivo},
};
use domain::{Almacen, ClaveArchivo};
use tokio::io::AsyncWriteExt;

pub use firma::{
    DescargaVerificada, ErrorEnlace, ErrorFirmante, FirmanteEnlaces, LONGITUD_MINIMA_SECRETO,
    ParametrosDescarga, RUTA_DESCARGA, codificar_porcentual,
};

/// Archivos en un directorio del servidor.
#[derive(Debug, Clone)]
pub struct AlmacenLocal {
    raiz: PathBuf,
    enlaces: FirmanteEnlaces,
}

impl AlmacenLocal {
    /// Usa `raiz` como directorio base (lo crea si no existe).
    ///
    /// # Errors
    ///
    /// Error de E/S si no se puede crear el directorio.
    pub fn abrir(raiz: impl Into<PathBuf>, enlaces: FirmanteEnlaces) -> io::Result<Self> {
        let raiz = raiz.into();
        std::fs::create_dir_all(&raiz)?;
        Ok(Self { raiz, enlaces })
    }

    /// Firmante de los enlaces (el endpoint de descarga lo usa para verificarlos).
    pub fn enlaces(&self) -> &FirmanteEnlaces {
        &self.enlaces
    }

    fn ruta(&self, clave: &ClaveArchivo) -> PathBuf {
        self.raiz.join(clave.as_str())
    }
}

/// Escribe en un temporal del mismo directorio, sincroniza y renombra: el archivo final
/// aparece completo o no aparece.
async fn escribir_atomico(ruta: &Path, contenido: &[u8]) -> io::Result<()> {
    let directorio = ruta
        .parent()
        .ok_or_else(|| io::Error::other("ruta de archivo sin directorio"))?;
    tokio::fs::create_dir_all(directorio).await?;
    let temporal = directorio.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let resultado = async {
        let mut archivo = tokio::fs::File::create(&temporal).await?;
        archivo.write_all(contenido).await?;
        archivo.sync_all().await?;
        tokio::fs::rename(&temporal, ruta).await
    }
    .await;
    if resultado.is_err() {
        let _ = tokio::fs::remove_file(&temporal).await;
    }
    resultado
}

impl AlmacenArchivos for AlmacenLocal {
    fn almacen(&self) -> Almacen {
        Almacen::Local
    }

    async fn guardar(&self, clave: &ClaveArchivo, contenido: &[u8]) -> AppResult<()> {
        escribir_atomico(&self.ruta(clave), contenido)
            .await
            .map_err(AppError::infra)
    }

    async fn obtener(&self, clave: &ClaveArchivo) -> AppResult<Vec<u8>> {
        match tokio::fs::read(self.ruta(clave)).await {
            Ok(contenido) => Ok(contenido),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(AppError::NoEncontrado("archivo")),
            Err(e) => Err(AppError::infra(e)),
        }
    }

    async fn eliminar(&self, clave: &ClaveArchivo) -> AppResult<()> {
        match tokio::fs::remove_file(self.ruta(clave)).await {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(AppError::infra(e)),
            _ => Ok(()),
        }
    }

    fn url_temporal(&self, descarga: &DescargaArchivo) -> AppResult<String> {
        Ok(self.enlaces.firmar(descarga))
    }
}

#[cfg(test)]
mod tests {
    use domain::{ArchivoId, TenantId};

    use super::*;

    /// Directorio temporal propio de cada prueba; se borra al soltarlo.
    struct DirTemporal(PathBuf);

    impl DirTemporal {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("dad-archivos-{}", uuid::Uuid::new_v4())))
        }
    }

    impl Drop for DirTemporal {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn almacen(dir: &DirTemporal) -> AlmacenLocal {
        let enlaces = FirmanteEnlaces::new(b"secreto-de-pruebas-de-32-bytes!!", "http://localhost")
            .expect("válido");
        AlmacenLocal::abrir(&dir.0, enlaces).expect("abre")
    }

    fn clave() -> ClaveArchivo {
        ClaveArchivo::new(TenantId::nuevo(), ArchivoId::nuevo())
    }

    #[tokio::test]
    async fn guarda_en_la_ruta_de_la_clave_y_lo_lee_igual() {
        let dir = DirTemporal::new();
        let almacen = almacen(&dir);
        let clave = clave();

        almacen
            .guardar(&clave, b"%PDF-1.7 contenido")
            .await
            .expect("guarda");

        assert!(dir.0.join(clave.as_str()).is_file());
        assert_eq!(
            almacen.obtener(&clave).await.expect("lee"),
            b"%PDF-1.7 contenido"
        );
        // Sin temporales sobrantes en la carpeta.
        let carpeta = dir.0.join(clave.as_str());
        let carpeta = carpeta.parent().expect("carpeta");
        assert_eq!(std::fs::read_dir(carpeta).expect("lista").count(), 1);
    }

    #[tokio::test]
    async fn guardar_de_nuevo_reemplaza_el_contenido() {
        let dir = DirTemporal::new();
        let almacen = almacen(&dir);
        let clave = clave();

        almacen.guardar(&clave, b"uno").await.expect("guarda");
        almacen.guardar(&clave, b"dos").await.expect("guarda");

        assert_eq!(almacen.obtener(&clave).await.expect("lee"), b"dos");
    }

    #[tokio::test]
    async fn obtener_inexistente_es_no_encontrado_y_eliminar_es_idempotente() {
        let dir = DirTemporal::new();
        let almacen = almacen(&dir);
        let clave = clave();

        assert!(matches!(
            almacen.obtener(&clave).await,
            Err(AppError::NoEncontrado("archivo"))
        ));
        almacen.guardar(&clave, b"x").await.expect("guarda");
        almacen.eliminar(&clave).await.expect("elimina");
        almacen
            .eliminar(&clave)
            .await
            .expect("eliminar otra vez no falla");
        assert!(!dir.0.join(clave.as_str()).exists());
    }
}
