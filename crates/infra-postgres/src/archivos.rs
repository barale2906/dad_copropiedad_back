//! Repositorio de archivos (`archivos`) sobre la transacción del tenant, y de los archivos de la
//! plataforma (`archivos_plataforma`) sobre la de la plataforma.

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::{
        RepositorioArchivos, RepositorioArchivosPlataforma,
        archivos::{Archivo, FiltroArchivos},
    },
};
use chrono::{DateTime, Utc};
use domain::{
    Almacen, ArchivoId, ClasificacionArchivo, ClaveArchivo, CuentaId, NombreArchivo, TipoArchivo,
};
use uuid::Uuid;

use crate::{TransaccionPg, TransaccionPlataformaPg};

struct FilaArchivo {
    id: Uuid,
    clave: String,
    almacen: String,
    nombre_original: String,
    tipo_mime: String,
    tamano_bytes: i64,
    sha256: String,
    clasificacion: String,
    subido_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
}

impl TryFrom<FilaArchivo> for Archivo {
    type Error = AppError;

    fn try_from(f: FilaArchivo) -> AppResult<Self> {
        Ok(Self {
            id: ArchivoId::from_uuid(f.id),
            clave: ClaveArchivo::parse(&f.clave)?,
            almacen: Almacen::try_from(f.almacen.as_str())?,
            nombre_original: NombreArchivo::new(&f.nombre_original)?,
            tipo: TipoArchivo::try_from(f.tipo_mime.as_str())?,
            tamano_bytes: u64::try_from(f.tamano_bytes).map_err(AppError::infra)?,
            sha256: f.sha256,
            clasificacion: ClasificacionArchivo::try_from(f.clasificacion.as_str())?,
            subido_por: f.subido_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
        })
    }
}

impl RepositorioArchivos for TransaccionPg {
    async fn insertar(&mut self, a: &Archivo) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO archivos (id, tenant_id, clave, almacen, nombre_original, tipo_mime,
                                   tamano_bytes, sha256, clasificacion, subido_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            a.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            a.clave.as_str(),
            a.almacen.as_str(),
            a.nombre_original.as_str(),
            a.tipo.as_str(),
            i64::try_from(a.tamano_bytes).map_err(AppError::infra)?,
            a.sha256,
            a.clasificacion.as_str(),
            a.subido_por.map(CuentaId::as_uuid),
            a.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn buscar(&mut self, id: ArchivoId) -> AppResult<Option<Archivo>> {
        // RLS limita la búsqueda a la copropiedad de la transacción.
        sqlx::query_as!(
            FilaArchivo,
            "SELECT id, clave, almacen, nombre_original, tipo_mime, tamano_bytes, sha256,
                    clasificacion, subido_por, creado_en
             FROM archivos WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Archivo::try_from)
        .transpose()
    }

    async fn listar(
        &mut self,
        f: &FiltroArchivos,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<Archivo>> {
        let filas = sqlx::query_as!(
            FilaArchivo,
            "SELECT id, clave, almacen, nombre_original, tipo_mime, tamano_bytes, sha256,
                    clasificacion, subido_por, creado_en
             FROM archivos
             WHERE tenant_id = $1
               AND ($2::text IS NULL OR clasificacion = $2)
               AND ($3 OR clasificacion NOT IN ('personal', 'sensible'))
               AND ($4::text IS NULL
                    OR normalizar(nombre_original) LIKE '%' || normalizar($4) || '%')
               AND ($5::timestamptz IS NULL OR creado_en >= $5)
               AND ($6::timestamptz IS NULL OR creado_en < $6)
               AND ($7::uuid IS NULL OR id < $7)
             ORDER BY id DESC
             LIMIT $8",
            self.ctx.tenant_id.as_uuid(),
            f.clasificacion.map(ClasificacionArchivo::as_str),
            f.incluir_personales,
            f.texto.as_deref(),
            f.desde,
            f.hasta,
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(Archivo::try_from)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(v, p, |a| a.id.as_uuid()))
    }

    async fn en_almacen(&mut self, almacen: Almacen, limite: u32) -> AppResult<Vec<Archivo>> {
        sqlx::query_as!(
            FilaArchivo,
            "SELECT id, clave, almacen, nombre_original, tipo_mime, tamano_bytes, sha256,
                    clasificacion, subido_por, creado_en
             FROM archivos
             WHERE tenant_id = $1 AND almacen = $2
             ORDER BY id
             LIMIT $3
             FOR UPDATE SKIP LOCKED",
            self.ctx.tenant_id.as_uuid(),
            almacen.as_str(),
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Archivo::try_from)
        .collect()
    }

    async fn cambiar_almacen(&mut self, id: ArchivoId, almacen: Almacen) -> AppResult<()> {
        let filas = sqlx::query!(
            "UPDATE archivos SET almacen = $2 WHERE id = $1",
            id.as_uuid(),
            almacen.as_str(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        if filas == 0 {
            return Err(AppError::NoEncontrado("archivo"));
        }
        Ok(())
    }
}

impl RepositorioArchivosPlataforma for TransaccionPlataformaPg {
    async fn insertar_archivo_plataforma(&mut self, a: &Archivo) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO archivos_plataforma (id, clave, almacen, nombre_original, tipo_mime,
                                              tamano_bytes, sha256, subido_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            a.id.as_uuid(),
            a.clave.as_str(),
            a.almacen.as_str(),
            a.nombre_original.as_str(),
            a.tipo.as_str(),
            i64::try_from(a.tamano_bytes).map_err(AppError::infra)?,
            a.sha256,
            a.subido_por.map(CuentaId::as_uuid),
            a.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn archivo_plataforma(&mut self, id: ArchivoId) -> AppResult<Option<Archivo>> {
        // Los archivos de la plataforma son públicos (imágenes de la publicidad).
        sqlx::query_as!(
            FilaArchivo,
            r#"SELECT id, clave, almacen, nombre_original, tipo_mime, tamano_bytes, sha256,
                      'publico' AS "clasificacion!", subido_por, creado_en
               FROM archivos_plataforma WHERE id = $1"#,
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Archivo::try_from)
        .transpose()
    }
}
