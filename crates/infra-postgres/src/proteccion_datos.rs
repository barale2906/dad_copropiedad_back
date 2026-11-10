//! Repositorio de protección de datos (`politicas_tratamiento`, `autorizaciones_tratamiento`,
//! `solicitudes_titular`) sobre la transacción del tenant.

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::{
        RepositorioProteccionDatos,
        proteccion_datos::{
            Autorizacion, AutorizacionRegistrada, FiltroSolicitudes, PoliticaTratamiento,
            SolicitudTitular,
        },
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, AutorizacionId, CuentaId, EstadoSolicitudTitular, FinalidadTratamiento,
    MedioAutorizacion, PersonaId, PoliticaTratamientoId, SolicitudTitularId, TipoSolicitudTitular,
};
use uuid::Uuid;

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
            "INSERT INTO solicitudes_titular (id, tenant_id, persona_id, tipo, radicada_en, vence_el,
                                              estado, detalle, radicada_por)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            s.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            s.persona_id.as_uuid(),
            s.tipo.as_str(),
            s.radicada_en,
            s.vence_el,
            s.estado.as_str(),
            s.detalle,
            s.radicada_por.map(CuentaId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn historial_politicas(&mut self) -> AppResult<Vec<PoliticaTratamiento>> {
        sqlx::query!(
            "SELECT id, version, texto, archivo_id, vigente_desde FROM politicas_tratamiento
             ORDER BY version DESC"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(PoliticaTratamiento {
                id: PoliticaTratamientoId::from_uuid(f.id),
                version: a_u32(f.version)?,
                texto: f.texto,
                archivo_id: f.archivo_id.map(ArchivoId::from_uuid),
                vigente_desde: f.vigente_desde,
            })
        })
        .collect()
    }

    async fn autorizaciones_de_persona(
        &mut self,
        persona_id: PersonaId,
    ) -> AppResult<Vec<AutorizacionRegistrada>> {
        sqlx::query!(
            "SELECT a.id, a.politica_id, a.finalidad, a.medio, a.evidencia_archivo_id, a.otorgada_en,
                    a.revocada_en, p.version
             FROM autorizaciones_tratamiento a
             JOIN politicas_tratamiento p ON p.tenant_id = a.tenant_id AND p.id = a.politica_id
             WHERE a.persona_id = $1 ORDER BY a.otorgada_en DESC, a.id",
            persona_id.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(AutorizacionRegistrada {
                autorizacion: Autorizacion {
                    id: AutorizacionId::from_uuid(f.id),
                    persona_id,
                    politica_id: PoliticaTratamientoId::from_uuid(f.politica_id),
                    finalidad: FinalidadTratamiento::try_from(f.finalidad.as_str())?,
                    medio: MedioAutorizacion::try_from(f.medio.as_str())?,
                    evidencia_archivo_id: f.evidencia_archivo_id.map(ArchivoId::from_uuid),
                    otorgada_en: f.otorgada_en,
                },
                politica_version: a_u32(f.version)?,
                revocada_en: f.revocada_en,
            })
        })
        .collect()
    }

    async fn solicitudes(
        &mut self,
        f: &FiltroSolicitudes,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<SolicitudTitular>> {
        let filas = sqlx::query_as!(
            FilaSolicitud,
            "SELECT id, persona_id, tipo, radicada_en, vence_el, estado, detalle, radicada_por,
                    prorrogada_hasta, motivo_prorroga, respuesta, respuesta_archivo_id,
                    respondida_en, respondida_por
             FROM solicitudes_titular
             WHERE ($1::text IS NULL OR estado = $1)
               AND ($2::uuid IS NULL OR persona_id = $2)
               AND ($3::date IS NULL OR (estado <> 'respondida'
                                         AND coalesce(prorrogada_hasta, vence_el) <= $3))
               AND ($4::uuid IS NULL OR id < $4)
             ORDER BY id DESC LIMIT $5",
            f.estado.map(EstadoSolicitudTitular::as_str),
            f.persona_id.map(PersonaId::as_uuid),
            f.vencen_hasta,
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(SolicitudTitular::try_from)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(v, p, |s| s.id.as_uuid()))
    }

    async fn solicitud_para_actualizar(
        &mut self,
        id: SolicitudTitularId,
    ) -> AppResult<Option<SolicitudTitular>> {
        sqlx::query_as!(
            FilaSolicitud,
            "SELECT id, persona_id, tipo, radicada_en, vence_el, estado, detalle, radicada_por,
                    prorrogada_hasta, motivo_prorroga, respuesta, respuesta_archivo_id,
                    respondida_en, respondida_por
             FROM solicitudes_titular WHERE id = $1 FOR UPDATE",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(SolicitudTitular::try_from)
        .transpose()
    }

    async fn actualizar_solicitud(&mut self, s: &SolicitudTitular) -> AppResult<()> {
        sqlx::query!(
            "UPDATE solicitudes_titular SET estado = $2, prorrogada_hasta = $3, motivo_prorroga = $4,
                 respuesta = $5, respuesta_archivo_id = $6, respondida_en = $7, respondida_por = $8
             WHERE id = $1",
            s.id.as_uuid(),
            s.estado.as_str(),
            s.prorrogada_hasta,
            s.motivo_prorroga,
            s.respuesta,
            s.respuesta_archivo_id.map(ArchivoId::as_uuid),
            s.respondida_en,
            s.respondida_por.map(CuentaId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

struct FilaSolicitud {
    id: Uuid,
    persona_id: Uuid,
    tipo: String,
    radicada_en: DateTime<Utc>,
    vence_el: NaiveDate,
    estado: String,
    detalle: Option<String>,
    radicada_por: Option<Uuid>,
    prorrogada_hasta: Option<NaiveDate>,
    motivo_prorroga: Option<String>,
    respuesta: Option<String>,
    respuesta_archivo_id: Option<Uuid>,
    respondida_en: Option<DateTime<Utc>>,
    respondida_por: Option<Uuid>,
}

impl TryFrom<FilaSolicitud> for SolicitudTitular {
    type Error = AppError;

    fn try_from(f: FilaSolicitud) -> AppResult<Self> {
        Ok(Self {
            id: SolicitudTitularId::from_uuid(f.id),
            persona_id: PersonaId::from_uuid(f.persona_id),
            tipo: TipoSolicitudTitular::try_from(f.tipo.as_str())?,
            radicada_en: f.radicada_en,
            vence_el: f.vence_el,
            estado: EstadoSolicitudTitular::try_from(f.estado.as_str())?,
            detalle: f.detalle,
            radicada_por: f.radicada_por.map(CuentaId::from_uuid),
            prorrogada_hasta: f.prorrogada_hasta,
            motivo_prorroga: f.motivo_prorroga,
            respuesta: f.respuesta,
            respuesta_archivo_id: f.respuesta_archivo_id.map(ArchivoId::from_uuid),
            respondida_en: f.respondida_en,
            respondida_por: f.respondida_por.map(CuentaId::from_uuid),
        })
    }
}
