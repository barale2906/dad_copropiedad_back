//! Búsqueda de proveedores, contactos autorizados y enlace público del directorio (plan 08,
//! fase 2; ADR 0014).
//!
//! - [`RepositorioSolicitudesProveedor`] sobre la transacción del tenant (RLS):
//!   `solicitudes_proveedor`, `contactos_proveedor` (append-only) y `enlaces_directorio`. Los
//!   mensajes van al outbox con `concepto = 'directorio'`.
//! - [`PurgaDirectorioPg`]: `purgar_solicitudes_proveedor` (SECURITY DEFINER) borra el texto de
//!   las búsquedas vencidas en todas las copropiedades.

use application::{
    AppError, AppResult,
    outbox::EventoSaliente,
    puertos::{
        PurgaDirectorio, RepositorioSolicitudesProveedor,
        directorio::{EnlaceDirectorio, SolicitudProveedor},
    },
};
use chrono::{DateTime, Utc};
use domain::{
    AnuncianteId, CanalSolicitud, CodigoCategoria, CuentaId, PersonaId, SolicitudProveedorId,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::TransaccionPg;

struct FilaSolicitud {
    id: Uuid,
    persona_id: Uuid,
    canal: String,
    clave_idempotencia: String,
    texto: Option<String>,
    categoria: Option<String>,
    sugeridos: Vec<Uuid>,
    autorizado_en: Option<DateTime<Utc>>,
    creada_en: DateTime<Utc>,
}

impl TryFrom<FilaSolicitud> for SolicitudProveedor {
    type Error = AppError;

    fn try_from(f: FilaSolicitud) -> AppResult<Self> {
        Ok(Self {
            id: SolicitudProveedorId::from_uuid(f.id),
            persona_id: PersonaId::from_uuid(f.persona_id),
            canal: CanalSolicitud::try_from(f.canal.as_str())?,
            clave_idempotencia: f.clave_idempotencia,
            texto: f.texto,
            categoria: f
                .categoria
                .as_deref()
                .map(CodigoCategoria::new)
                .transpose()?,
            sugeridos: f
                .sugeridos
                .into_iter()
                .map(AnuncianteId::from_uuid)
                .collect(),
            autorizado_en: f.autorizado_en,
            creada_en: f.creada_en,
        })
    }
}

impl RepositorioSolicitudesProveedor for TransaccionPg {
    async fn insertar_solicitud_proveedor(
        &mut self,
        s: &SolicitudProveedor,
    ) -> AppResult<SolicitudProveedor> {
        let sugeridos: Vec<Uuid> = s.sugeridos.iter().map(|a| a.as_uuid()).collect();
        let insertada = sqlx::query_scalar!(
            "INSERT INTO solicitudes_proveedor (id, tenant_id, persona_id, canal,
                 clave_idempotencia, texto, categoria, sugeridos, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (tenant_id, clave_idempotencia) DO NOTHING
             RETURNING id",
            s.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            s.persona_id.as_uuid(),
            s.canal.as_str(),
            s.clave_idempotencia,
            s.texto,
            s.categoria.as_ref().map(CodigoCategoria::as_str),
            &sugeridos,
            s.creada_en,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        if insertada.is_some() {
            return Ok(s.clone());
        }
        let fila = sqlx::query_as!(
            FilaSolicitud,
            r#"SELECT id, persona_id, canal, clave_idempotencia, texto, categoria,
                      sugeridos AS "sugeridos!", autorizado_en, creada_en
               FROM solicitudes_proveedor WHERE clave_idempotencia = $1"#,
            s.clave_idempotencia,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        SolicitudProveedor::try_from(fila)
    }

    async fn solicitud_proveedor(
        &mut self,
        id: SolicitudProveedorId,
    ) -> AppResult<Option<SolicitudProveedor>> {
        let fila = sqlx::query_as!(
            FilaSolicitud,
            r#"SELECT id, persona_id, canal, clave_idempotencia, texto, categoria,
                      sugeridos AS "sugeridos!", autorizado_en, creada_en
               FROM solicitudes_proveedor WHERE id = $1 FOR UPDATE"#,
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        fila.map(SolicitudProveedor::try_from).transpose()
    }

    async fn autorizar_contacto(
        &mut self,
        id: SolicitudProveedorId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE solicitudes_proveedor SET autorizado_en = $2
             WHERE id = $1 AND autorizado_en IS NULL",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn insertar_contacto_proveedor(
        &mut self,
        solicitud: SolicitudProveedorId,
        anunciante: AnuncianteId,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        let n = sqlx::query!(
            "INSERT INTO contactos_proveedor (tenant_id, solicitud_id, anunciante_id, creado_en)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT DO NOTHING",
            self.ctx.tenant_id.as_uuid(),
            solicitud.as_uuid(),
            anunciante.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        Ok(n == 1)
    }

    async fn contactos_de_solicitud(
        &mut self,
        solicitud: SolicitudProveedorId,
    ) -> AppResult<Vec<AnuncianteId>> {
        Ok(sqlx::query_scalar!(
            "SELECT anunciante_id FROM contactos_proveedor
             WHERE solicitud_id = $1 ORDER BY creado_en, anunciante_id",
            solicitud.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(AnuncianteId::from_uuid)
        .collect())
    }

    async fn encolar_directorio(
        &mut self,
        evento: &EventoSaliente,
        disponible_en: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        let payload = serde_json::to_value(&evento.mensaje).map_err(AppError::infra)?;
        sqlx::query!(
            "INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload, concepto,
                                 disponible_en)
             VALUES ($1, $2, $3, $4, $5, $6, 'directorio', coalesce($7, now()))",
            self.ctx.tenant_id.as_uuid(),
            evento.mensaje.canal().as_str(),
            evento.prioridad.as_i16(),
            evento.mensaje.linea(),
            evento.mensaje.destino(),
            payload,
            disponible_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn enlace_directorio(&mut self) -> AppResult<Option<EnlaceDirectorio>> {
        Ok(sqlx::query!(
            "SELECT token_hash, creado_por, creado_en FROM enlaces_directorio
             WHERE tenant_id = $1",
            self.ctx.tenant_id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| EnlaceDirectorio {
            token_hash: f.token_hash,
            creado_por: f.creado_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
        }))
    }

    async fn guardar_enlace_directorio(&mut self, e: &EnlaceDirectorio) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO enlaces_directorio (tenant_id, token_hash, creado_por, creado_en)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (tenant_id) DO UPDATE
                 SET token_hash = EXCLUDED.token_hash, creado_por = EXCLUDED.creado_por,
                     creado_en = EXCLUDED.creado_en",
            self.ctx.tenant_id.as_uuid(),
            e.token_hash,
            e.creado_por.map(|c| c.as_uuid()),
            e.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn borrar_enlace_directorio(&mut self) -> AppResult<bool> {
        let n = sqlx::query!(
            "DELETE FROM enlaces_directorio WHERE tenant_id = $1",
            self.ctx.tenant_id.as_uuid(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        Ok(n > 0)
    }

    async fn numero_whatsapp_publico(&mut self) -> AppResult<Option<String>> {
        sqlx::query_scalar!("SELECT numero_whatsapp_publico()")
            .fetch_one(&mut *self.tx)
            .await
            .map_err(AppError::infra)
    }
}

/// Purga de las búsquedas de proveedores (worker de gestión del gateway).
#[derive(Debug, Clone)]
pub struct PurgaDirectorioPg {
    pool: PgPool,
}

impl PurgaDirectorioPg {
    /// Purga sobre el pool de la app (`dad_app`).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl PurgaDirectorio for PurgaDirectorioPg {
    async fn purgar_solicitudes_proveedor(&self, dias: u32) -> AppResult<u64> {
        let dias = i32::try_from(dias).unwrap_or(i32::MAX);
        let n = sqlx::query_scalar!(r#"SELECT purgar_solicitudes_proveedor($1) AS "n!""#, dias)
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::infra)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}
