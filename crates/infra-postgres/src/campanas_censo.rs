//! Campañas de actualización del censo sobre la transacción del tenant (plan 10, fase 5).

use application::{
    AppError, AppResult,
    puertos::{
        RepositorioCampanasCenso,
        censo::{
            CampanaCenso, DestinatarioCampanaCenso, RespuestaCenso, RespuestaCensoListada,
            ResumenCampanaCenso,
        },
    },
};
use chrono::{DateTime, Utc};
use domain::{BienPrivadoId, CampanaCensoId, CuentaId, PersonaId, RespuestaCampanaCenso};

use crate::TransaccionPg;

fn contador(v: Option<i64>) -> u32 {
    u32::try_from(v.unwrap_or(0)).unwrap_or(u32::MAX)
}

impl RepositorioCampanasCenso for TransaccionPg {
    async fn insertar_campana_censo(&mut self, c: &CampanaCenso) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO campanas_censo (id, tenant_id, creada_por, creada_en, enviados)
             VALUES ($1, $2, $3, $4, $5)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.creada_por.map(CuentaId::as_uuid),
            c.creada_en,
            i32::try_from(c.enviados).unwrap_or(i32::MAX),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn campana_censo(&mut self, id: CampanaCensoId) -> AppResult<Option<CampanaCenso>> {
        Ok(sqlx::query!(
            "SELECT id, creada_por, creada_en, enviados FROM campanas_censo WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| CampanaCenso {
            id: CampanaCensoId::from_uuid(f.id),
            creada_por: f.creada_por.map(CuentaId::from_uuid),
            creada_en: f.creada_en,
            enviados: u32::try_from(f.enviados).unwrap_or(0),
        }))
    }

    async fn campanas_censo(&mut self) -> AppResult<Vec<ResumenCampanaCenso>> {
        Ok(sqlx::query!(
            r#"SELECT c.id, c.creada_por, c.creada_en, c.enviados,
                      count(r.*) FILTER (WHERE r.respuesta = 'confirmado') AS confirmados,
                      count(r.*) FILTER (WHERE r.respuesta = 'cambios') AS con_cambios,
                      count(r.*) FILTER (WHERE r.respuesta = 'cambios' AND r.atendida_en IS NULL)
                          AS por_atender
               FROM campanas_censo c
               LEFT JOIN respuestas_censo r ON r.tenant_id = c.tenant_id AND r.campana_id = c.id
               GROUP BY c.id ORDER BY c.id DESC LIMIT 100"#,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| ResumenCampanaCenso {
            campana: CampanaCenso {
                id: CampanaCensoId::from_uuid(f.id),
                creada_por: f.creada_por.map(CuentaId::from_uuid),
                creada_en: f.creada_en,
                enviados: u32::try_from(f.enviados).unwrap_or(0),
            },
            confirmados: contador(f.confirmados),
            con_cambios: contador(f.con_cambios),
            por_atender: contador(f.por_atender),
        })
        .collect())
    }

    async fn destinatarios_campana_censo(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<DestinatarioCampanaCenso>> {
        Ok(sqlx::query!(
            r#"SELECT persona_id AS "persona_id!", bien_privado_id AS "bien_privado_id!",
                      codigo AS "codigo!", telefono AS "telefono!"
               FROM destinatarios_campana_censo($1)"#,
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| DestinatarioCampanaCenso {
            persona: PersonaId::from_uuid(f.persona_id),
            bien: BienPrivadoId::from_uuid(f.bien_privado_id),
            codigo: f.codigo,
            telefono: f.telefono,
        })
        .collect())
    }

    async fn guardar_respuesta_censo(&mut self, r: &RespuestaCenso) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO respuestas_censo (tenant_id, campana_id, persona_id, bien_privado_id,
                 respuesta, detalle, respondida_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (tenant_id, campana_id, persona_id, bien_privado_id) DO UPDATE
             SET respuesta = EXCLUDED.respuesta, detalle = EXCLUDED.detalle,
                 respondida_en = EXCLUDED.respondida_en, atendida_en = NULL, atendida_por = NULL",
            self.ctx.tenant_id.as_uuid(),
            r.campana.as_uuid(),
            r.persona.as_uuid(),
            r.bien.as_uuid(),
            r.respuesta.as_str(),
            r.detalle,
            r.respondida_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn respuestas_censo(
        &mut self,
        campana: CampanaCensoId,
    ) -> AppResult<Vec<RespuestaCensoListada>> {
        sqlx::query!(
            "SELECT r.persona_id, r.bien_privado_id, r.respuesta, r.detalle, r.respondida_en,
                    r.atendida_en, r.atendida_por, p.nombre, b.codigo
             FROM respuestas_censo r
             JOIN personas p ON p.tenant_id = r.tenant_id AND p.id = r.persona_id
             JOIN bienes_privados b ON b.tenant_id = r.tenant_id AND b.id = r.bien_privado_id
             WHERE r.campana_id = $1
             ORDER BY (r.respuesta = 'cambios' AND r.atendida_en IS NULL) DESC, b.codigo",
            campana.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(RespuestaCensoListada {
                respuesta: RespuestaCenso {
                    campana,
                    persona: PersonaId::from_uuid(f.persona_id),
                    bien: BienPrivadoId::from_uuid(f.bien_privado_id),
                    respuesta: RespuestaCampanaCenso::try_from(f.respuesta.as_str())?,
                    detalle: f.detalle,
                    respondida_en: f.respondida_en,
                    atendida_en: f.atendida_en,
                    atendida_por: f.atendida_por.map(CuentaId::from_uuid),
                },
                nombre: f.nombre,
                codigo: f.codigo,
            })
        })
        .collect()
    }

    async fn atender_respuesta_censo(
        &mut self,
        campana: CampanaCensoId,
        persona: PersonaId,
        bien: BienPrivadoId,
        por: Option<CuentaId>,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        let n = sqlx::query!(
            "UPDATE respuestas_censo SET atendida_en = $4, atendida_por = $5
             WHERE campana_id = $1 AND persona_id = $2 AND bien_privado_id = $3
               AND respuesta = 'cambios'",
            campana.as_uuid(),
            persona.as_uuid(),
            bien.as_uuid(),
            ahora,
            por.map(CuentaId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        Ok(n > 0)
    }
}
