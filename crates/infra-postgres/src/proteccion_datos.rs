//! Repositorio de protección de datos (`politicas_tratamiento`, `autorizaciones_tratamiento`,
//! `solicitudes_titular`) sobre la transacción del tenant.

use application::{
    AppError, AppResult,
    puertos::{
        RepositorioProteccionDatos,
        proteccion_datos::{Autorizacion, PoliticaTratamiento, SolicitudTitular},
    },
};
use chrono::{DateTime, Utc};
use domain::{ArchivoId, FinalidadTratamiento, PersonaId, PoliticaTratamientoId};

use crate::TransaccionPg;

fn a_u32(v: i32) -> AppResult<u32> {
    u32::try_from(v).map_err(AppError::infra)
}

fn a_i32(v: u32) -> AppResult<i32> {
    i32::try_from(v).map_err(AppError::infra)
}

impl RepositorioProteccionDatos for TransaccionPg {
    async fn ultima_version_politica(&mut self) -> AppResult<u32> {
        let version = sqlx::query_scalar!(
            r#"SELECT COALESCE(MAX(version), 0) AS "version!" FROM politicas_tratamiento"#
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        a_u32(version)
    }

    async fn insertar_politica(&mut self, p: &PoliticaTratamiento) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO politicas_tratamiento (id, tenant_id, version, texto, archivo_id, vigente_desde)
             VALUES ($1, $2, $3, $4, $5, $6)",
            p.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            a_i32(p.version)?,
            p.texto.as_deref(),
            p.archivo_id.map(ArchivoId::as_uuid),
            p.vigente_desde,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn politica_vigente(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Option<PoliticaTratamiento>> {
        let fila = sqlx::query!(
            "SELECT id, version, texto, archivo_id, vigente_desde
             FROM politicas_tratamiento
             WHERE vigente_desde <= $1
             ORDER BY version DESC
             LIMIT 1",
            ahora,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        fila.map(|f| {
            Ok(PoliticaTratamiento {
                id: PoliticaTratamientoId::from_uuid(f.id),
                version: a_u32(f.version)?,
                texto: f.texto,
                archivo_id: f.archivo_id.map(ArchivoId::from_uuid),
                vigente_desde: f.vigente_desde,
            })
        })
        .transpose()
    }

    async fn insertar_autorizacion(&mut self, a: &Autorizacion) -> AppResult<bool> {
        // El índice único parcial impide dos autorizaciones vigentes de la misma finalidad.
        let filas = sqlx::query!(
            "INSERT INTO autorizaciones_tratamiento
                 (id, tenant_id, persona_id, politica_id, finalidad, medio,
                  evidencia_archivo_id, otorgada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (tenant_id, persona_id, finalidad) WHERE revocada_en IS NULL
             DO NOTHING",
            a.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            a.persona_id.as_uuid(),
            a.politica_id.as_uuid(),
            a.finalidad.as_str(),
            a.medio.as_str(),
            a.evidencia_archivo_id.map(ArchivoId::as_uuid),
            a.otorgada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        Ok(filas == 1)
    }

    async fn revocar_autorizaciones(
        &mut self,
        persona_id: PersonaId,
        finalidades: &[FinalidadTratamiento],
        ahora: DateTime<Utc>,
    ) -> AppResult<u64> {
        let finalidades: Vec<String> = finalidades.iter().map(|f| f.as_str().to_owned()).collect();
        let filas = sqlx::query!(
            "UPDATE autorizaciones_tratamiento SET revocada_en = $3
             WHERE persona_id = $1 AND finalidad = ANY($2) AND revocada_en IS NULL",
            persona_id.as_uuid(),
            &finalidades,
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        Ok(filas)
    }

    async fn finalidades_autorizadas(
        &mut self,
        persona_id: PersonaId,
    ) -> AppResult<Vec<FinalidadTratamiento>> {
        sqlx::query_scalar!(
            "SELECT finalidad FROM autorizaciones_tratamiento
             WHERE persona_id = $1 AND revocada_en IS NULL
             ORDER BY finalidad",
            persona_id.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .iter()
        .map(|f| FinalidadTratamiento::try_from(f.as_str()).map_err(AppError::from))
        .collect()
    }

    async fn insertar_solicitud(&mut self, s: &SolicitudTitular) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO solicitudes_titular (id, tenant_id, persona_id, tipo, radicada_en, vence_el)
             VALUES ($1, $2, $3, $4, $5, $6)",
            s.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            s.persona_id.as_uuid(),
            s.tipo.as_str(),
            s.radicada_en,
            s.vence_el,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}
