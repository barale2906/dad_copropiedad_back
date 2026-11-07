//! Puertos de archivos: dónde se guardan los bytes ([`AlmacenArchivos`]) y su registro en la
//! base de datos ([`RepositorioArchivos`]).
//!
//! **Metodología para cambiar de almacén** (disco del VPS → S3, ADR 0003):
//! 1. Ningún módulo toca rutas: todo pasa por [`AlmacenArchivos`] con una [`ClaveArchivo`].
//! 2. La fila de `archivos` guarda la clave lógica y el [`Almacen`] donde están los bytes.
//! 3. Un adaptador por almacén (`infra-archivos`: local; S3 cuando haga falta), elegido con
//!    `ARCHIVOS_ALMACEN`.
//! 4. Migrar = [`crate::casos_uso::archivos::migrar_lote`]: copia, verifica el SHA-256 y cambia
//!    la columna `almacen`, archivo por archivo.
//! 5. Descargas solo por URL temporal firmada que genera el adaptador ([`AlmacenArchivos::url_temporal`]).

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::{
    Almacen, ArchivoId, ClasificacionArchivo, ClaveArchivo, CuentaId, NombreArchivo, TipoArchivo,
};

use crate::AppResult;

/// Archivo registrado en una copropiedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archivo {
    pub id: ArchivoId,
    pub clave: ClaveArchivo,
    /// Almacén donde están hoy los bytes.
    pub almacen: Almacen,
    pub nombre_original: NombreArchivo,
    /// Detectado por el contenido al subirlo.
    pub tipo: TipoArchivo,
    pub tamano_bytes: u64,
    /// SHA-256 del contenido en hexadecimal (64 caracteres en minúscula). Verifica copias y
    /// migraciones.
    pub sha256: String,
    pub clasificacion: ClasificacionArchivo,
    /// `None` = lo subió el sistema (p. ej. un acta generada).
    pub subido_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
}

/// Lo que necesita un adaptador para firmar un enlace de descarga.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescargaArchivo {
    pub clave: ClaveArchivo,
    pub tipo: TipoArchivo,
    /// Nombre con el que se descarga.
    pub nombre: NombreArchivo,
    /// Después de este instante el enlace ya no sirve.
    pub vence_en: DateTime<Utc>,
}

/// Almacén de los bytes de los archivos (disco local, S3…).
pub trait AlmacenArchivos: Send + Sync {
    /// Qué almacén es (se guarda en la fila de cada archivo).
    fn almacen(&self) -> Almacen;

    /// Guarda el contenido bajo `clave`. Escritura atómica: o queda completo o no queda.
    fn guardar(
        &self,
        clave: &ClaveArchivo,
        contenido: &[u8],
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Lee el contenido completo.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::NoEncontrado`] si no hay nada bajo `clave`.
    fn obtener(&self, clave: &ClaveArchivo) -> impl Future<Output = AppResult<Vec<u8>>> + Send;

    /// Borra el contenido. Borrar algo que no existe no es error.
    fn eliminar(&self, clave: &ClaveArchivo) -> impl Future<Output = AppResult<()>> + Send;

    /// URL de vida corta para descargar sin más credenciales (local: endpoint propio con firma
    /// HMAC; S3: URL prefirmada). El permiso se verifica **antes** de generarla.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Infraestructura`] si no se puede firmar.
    fn url_temporal(&self, descarga: &DescargaArchivo) -> AppResult<String>;
}

/// Registro de archivos dentro de una transacción del tenant.
pub trait RepositorioArchivos: Send {
    /// Registra un archivo nuevo.
    fn insertar(&mut self, archivo: &Archivo) -> impl Future<Output = AppResult<()>> + Send;

    /// El archivo `id` de la copropiedad de la transacción, si existe.
    fn buscar(&mut self, id: ArchivoId) -> impl Future<Output = AppResult<Option<Archivo>>> + Send;

    /// Hasta `limite` archivos guardados en `almacen`, bloqueados para la transacción (los
    /// usa la migración entre almacenes; dos migraciones simultáneas no toman el mismo).
    fn en_almacen(
        &mut self,
        almacen: Almacen,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<Archivo>>> + Send;

    /// Anota que los bytes del archivo ahora están en `almacen`.
    fn cambiar_almacen(
        &mut self,
        id: ArchivoId,
        almacen: Almacen,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

#[cfg(test)]
pub(crate) mod memoria {
    //! Almacén en memoria para las pruebas de los casos de uso.

    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::AppError;

    #[derive(Clone)]
    pub struct AlmacenEnMemoria {
        pub tipo: Almacen,
        pub contenido: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        /// Si es `true`, `guardar` falla (simula disco lleno o S3 caído).
        pub falla_al_guardar: bool,
    }

    impl AlmacenEnMemoria {
        pub fn new(tipo: Almacen) -> Self {
            Self {
                tipo,
                contenido: Arc::default(),
                falla_al_guardar: false,
            }
        }

        pub fn claves(&self) -> Vec<String> {
            let mut claves: Vec<_> = self
                .contenido
                .lock()
                .expect("mutex sano")
                .keys()
                .cloned()
                .collect();
            claves.sort();
            claves
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("almacén en memoria sin espacio")]
    struct SinEspacio;

    impl AlmacenArchivos for AlmacenEnMemoria {
        fn almacen(&self) -> Almacen {
            self.tipo
        }

        async fn guardar(&self, clave: &ClaveArchivo, contenido: &[u8]) -> AppResult<()> {
            if self.falla_al_guardar {
                return Err(AppError::infra(SinEspacio));
            }
            self.contenido
                .lock()
                .expect("mutex sano")
                .insert(clave.as_str().to_owned(), contenido.to_vec());
            Ok(())
        }

        async fn obtener(&self, clave: &ClaveArchivo) -> AppResult<Vec<u8>> {
            self.contenido
                .lock()
                .expect("mutex sano")
                .get(clave.as_str())
                .cloned()
                .ok_or(AppError::NoEncontrado("archivo"))
        }

        async fn eliminar(&self, clave: &ClaveArchivo) -> AppResult<()> {
            self.contenido
                .lock()
                .expect("mutex sano")
                .remove(clave.as_str());
            Ok(())
        }

        fn url_temporal(&self, d: &DescargaArchivo) -> AppResult<String> {
            Ok(format!(
                "memoria://{}?vence={}",
                d.clave,
                d.vence_en.timestamp()
            ))
        }
    }
}
