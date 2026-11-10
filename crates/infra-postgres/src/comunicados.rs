//! Comunicados de la administración sobre la transacción del tenant (plan 10, fase 4).
//!
//! La audiencia se resuelve con la función `destinatarios_comunicado` (RLS del tenant) y los
//! tokens push con `tokens_push_de_cuentas` (tabla de plataforma, `SECURITY DEFINER`).

use std::collections::BTreeMap;

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    outbox::EventoSaliente,
    puertos::{
        RepositorioComunicados,
        comunicados::{
            Comunicado, ComunicadoConResumen, ComunicadoRecibido, DestinatarioComunicado,
            EntregaComunicado, ResumenLectura,
        },
    },
};
use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, Audiencia, ComunicadoId, CuentaId, EstadoComunicado, FinalidadTratamiento, PersonaId,
};
use uuid::Uuid;

use crate::{TransaccionPg, copropiedades::error_escritura};

struct FilaComunicado {
    id: Uuid,
    titulo: String,
    cuerpo: String,
    finalidad: String,
    audiencia: serde_json::Value,
    importante: bool,
    requiere_confirmacion: bool,
    estado: String,
    programado_para: Option<DateTime<Utc>>,
    enviado_en: Option<DateTime<Utc>>,
    adjuntos: Vec<Uuid>,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
    actualizado_en: DateTime<Utc>,
}

impl TryFrom<FilaComunicado> for Comunicado {
    type Error = AppError;

    fn try_from(f: FilaComunicado) -> AppResult<Self> {
        Ok(Self {
            id: ComunicadoId::from_uuid(f.id),
            titulo: f.titulo,
            cuerpo: f.cuerpo,
            finalidad: FinalidadTratamiento::try_from(f.finalidad.as_str())?,
            audiencia: serde_json::from_value::<Audiencia>(f.audiencia).map_err(AppError::infra)?,
            importante: f.importante,
            requiere_confirmacion: f.requiere_confirmacion,
            estado: EstadoComunicado::try_from(f.estado.as_str())?,
            programado_para: f.programado_para,
            enviado_en: f.enviado_en,
            adjuntos: f.adjuntos.into_iter().map(ArchivoId::from_uuid).collect(),
            creado_por: f.creado_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
            actualizado_en: f.actualizado_en,
        })
    }
}

fn contador(v: Option<i64>) -> u32 {
    u32::try_from(v.unwrap_or(0)).unwrap_or(u32::MAX)
}

impl TransaccionPg {
    async fn guardar_adjuntos(&mut self, c: &Comunicado) -> AppResult<()> {
        sqlx::query!(
            "DELETE FROM comunicado_adjuntos WHERE comunicado_id = $1",
            c.id.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let ids: Vec<Uuid> = c.adjuntos.iter().map(|a| a.as_uuid()).collect();
        sqlx::query!(
            "INSERT INTO comunicado_adjuntos (tenant_id, comunicado_id, archivo_id, orden)
             SELECT $1, $2, a, o::smallint FROM unnest($3::uuid[]) WITH ORDINALITY AS t (a, o)",
            self.ctx.tenant_id.as_uuid(),
            c.id.as_uuid(),
            &ids,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "adjunto repetido", "archivo"))?;
        Ok(())
    }

    async fn cargar(&mut self, id: ComunicadoId, bloquear: bool) -> AppResult<Option<Comunicado>> {
        if bloquear {
            sqlx::query!(
                "SELECT id FROM comunicados WHERE id = $1 FOR UPDATE",
                id.as_uuid()
            )
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        }
        sqlx::query_as!(
            FilaComunicado,
            r#"SELECT c.id, c.titulo, c.cuerpo, c.finalidad, c.audiencia, c.importante,
                      c.requiere_confirmacion, c.estado, c.programado_para, c.enviado_en,
                      coalesce((SELECT array_agg(a.archivo_id ORDER BY a.orden)
                                FROM comunicado_adjuntos a WHERE a.comunicado_id = c.id),
                               '{}') AS "adjuntos!",
                      c.creado_por, c.creado_en, c.actualizado_en
               FROM comunicados c WHERE c.id = $1"#,
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Comunicado::try_from)
        .transpose()
    }
}

impl RepositorioComunicados for TransaccionPg {
    async fn insertar_comunicado(&mut self, c: &Comunicado) -> AppResult<()> {
        let audiencia = serde_json::to_value(&c.audiencia).map_err(AppError::infra)?;
        sqlx::query!(
            "INSERT INTO comunicados (id, tenant_id, titulo, cuerpo, finalidad, audiencia,
                 importante, requiere_confirmacion, estado, programado_para, enviado_en,
                 creado_por, creado_en, actualizado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.titulo,
            c.cuerpo,
            c.finalidad.as_str(),
            audiencia,
            c.importante,
            c.requiere_confirmacion,
            c.estado.as_str(),
            c.programado_para,
            c.enviado_en,
            c.creado_por.map(CuentaId::as_uuid),
            c.creado_en,
            c.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        self.guardar_adjuntos(c).await
    }

    async fn actualizar_comunicado(&mut self, c: &Comunicado) -> AppResult<()> {
        let audiencia = serde_json::to_value(&c.audiencia).map_err(AppError::infra)?;
        sqlx::query!(
            "UPDATE comunicados SET titulo = $2, cuerpo = $3, finalidad = $4, audiencia = $5,
                 importante = $6, requiere_confirmacion = $7, estado = $8, programado_para = $9,
                 enviado_en = $10, actualizado_en = $11
             WHERE id = $1",
            c.id.as_uuid(),
            c.titulo,
            c.cuerpo,
            c.finalidad.as_str(),
            audiencia,
            c.importante,
            c.requiere_confirmacion,
            c.estado.as_str(),
            c.programado_para,
            c.enviado_en,
            c.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        self.guardar_adjuntos(c).await
    }

    async fn comunicado(&mut self, id: ComunicadoId) -> AppResult<Option<Comunicado>> {
        self.cargar(id, false).await
    }

    async fn bloquear_comunicado(&mut self, id: ComunicadoId) -> AppResult<Option<Comunicado>> {
        self.cargar(id, true).await
    }

    async fn comunicados(&mut self, p: SolicitudPagina) -> AppResult<Pagina<ComunicadoConResumen>> {
        let ids: Vec<Uuid> = sqlx::query_scalar!(
            "SELECT id FROM comunicados WHERE ($1::uuid IS NULL OR id < $1)
             ORDER BY id DESC LIMIT $2",
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let mut v = Vec::with_capacity(ids.len());
        for id in ids {
            let id = ComunicadoId::from_uuid(id);
            if let Some(comunicado) = self.cargar(id, false).await? {
                let resumen = self.resumen_lectura(id).await?;
                v.push(ComunicadoConResumen {
                    comunicado,
                    resumen,
                });
            }
        }
        Ok(Pagina::desde_filas(v, p, |c| c.comunicado.id.as_uuid()))
    }

    async fn resumen_lectura(&mut self, id: ComunicadoId) -> AppResult<ResumenLectura> {
        let f = sqlx::query!(
            r#"SELECT count(*) AS destinatarios,
                      count(*) FILTER (WHERE por_push) AS por_push,
                      count(*) FILTER (WHERE por_correo) AS por_correo,
                      count(*) FILTER (WHERE por_whatsapp) AS por_whatsapp,
                      count(*) FILTER (WHERE NOT (por_push OR por_correo OR por_whatsapp)) AS sin_canal,
                      count(leido_en) AS leidos,
                      count(enterado_en) AS enterados
               FROM comunicado_destinatarios WHERE comunicado_id = $1"#,
            id.as_uuid(),
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(ResumenLectura {
            destinatarios: contador(f.destinatarios),
            por_push: contador(f.por_push),
            por_correo: contador(f.por_correo),
            por_whatsapp: contador(f.por_whatsapp),
            sin_canal: contador(f.sin_canal),
            leidos: contador(f.leidos),
            enterados: contador(f.enterados),
        })
    }

    async fn programados_vencidos(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<ComunicadoId>> {
        Ok(sqlx::query_scalar!(
            "SELECT id FROM comunicados WHERE estado = 'programado' AND programado_para <= $1
             ORDER BY programado_para",
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(ComunicadoId::from_uuid)
        .collect())
    }

    async fn destinatarios_comunicado(
        &mut self,
        audiencia: &Audiencia,
        finalidad: FinalidadTratamiento,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<DestinatarioComunicado>> {
        let audiencia = serde_json::to_value(audiencia).map_err(AppError::infra)?;
        let filas = sqlx::query!(
            r#"SELECT persona_id AS "persona_id!", cuenta_id, correo, telefono
               FROM destinatarios_comunicado($1, $2, $3)"#,
            audiencia,
            finalidad.as_str(),
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let cuentas: Vec<Uuid> = filas.iter().filter_map(|f| f.cuenta_id).collect();
        let mut tokens: BTreeMap<Uuid, Vec<String>> = BTreeMap::new();
        if !cuentas.is_empty() {
            for t in sqlx::query!(
                r#"SELECT cuenta_id AS "cuenta_id!", token AS "token!"
                   FROM tokens_push_de_cuentas($1)"#,
                &cuentas,
            )
            .fetch_all(&mut *self.tx)
            .await
            .map_err(AppError::infra)?
            {
                tokens.entry(t.cuenta_id).or_default().push(t.token);
            }
        }
        Ok(filas
            .into_iter()
            .map(|f| DestinatarioComunicado {
                persona: PersonaId::from_uuid(f.persona_id),
                cuenta: f.cuenta_id.map(CuentaId::from_uuid),
                correo: f.correo,
                telefono: f.telefono,
                tokens: f
                    .cuenta_id
                    .and_then(|c| tokens.remove(&c))
                    .unwrap_or_default(),
            })
            .collect())
    }

    async fn insertar_entregas(
        &mut self,
        id: ComunicadoId,
        entregas: &[EntregaComunicado],
    ) -> AppResult<()> {
        let personas: Vec<Uuid> = entregas.iter().map(|e| e.persona.as_uuid()).collect();
        let push: Vec<bool> = entregas.iter().map(|e| e.canales.push).collect();
        let correo: Vec<bool> = entregas.iter().map(|e| e.canales.correo).collect();
        let whatsapp: Vec<bool> = entregas.iter().map(|e| e.canales.whatsapp).collect();
        sqlx::query!(
            "INSERT INTO comunicado_destinatarios
                 (tenant_id, comunicado_id, persona_id, por_push, por_correo, por_whatsapp)
             SELECT $1, $2, p, pu, co, wa
             FROM unnest($3::uuid[], $4::bool[], $5::bool[], $6::bool[]) AS t (p, pu, co, wa)",
            self.ctx.tenant_id.as_uuid(),
            id.as_uuid(),
            &personas,
            &push,
            &correo,
            &whatsapp,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn encolar_comunicado(&mut self, evento: &EventoSaliente) -> AppResult<()> {
        let payload = serde_json::to_value(&evento.mensaje).map_err(AppError::infra)?;
        sqlx::query!(
            "INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload, concepto)
             VALUES ($1, $2, $3, $4, $5, $6, 'comunicados')",
            self.ctx.tenant_id.as_uuid(),
            evento.mensaje.canal().as_str(),
            evento.prioridad.as_i16(),
            evento.mensaje.linea(),
            evento.mensaje.destino(),
            payload,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn recibidos(
        &mut self,
        persona: PersonaId,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<ComunicadoRecibido>> {
        let filas = sqlx::query!(
            "SELECT comunicado_id, leido_en, enterado_en FROM comunicado_destinatarios
             WHERE persona_id = $1 AND ($2::uuid IS NULL OR comunicado_id < $2)
             ORDER BY comunicado_id DESC LIMIT $3",
            persona.as_uuid(),
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let mut v = Vec::with_capacity(filas.len());
        for f in filas {
            if let Some(comunicado) = self
                .cargar(ComunicadoId::from_uuid(f.comunicado_id), false)
                .await?
            {
                v.push(ComunicadoRecibido {
                    comunicado,
                    leido_en: f.leido_en,
                    enterado_en: f.enterado_en,
                });
            }
        }
        Ok(Pagina::desde_filas(v, p, |r| r.comunicado.id.as_uuid()))
    }

    async fn recibido(
        &mut self,
        persona: PersonaId,
        id: ComunicadoId,
    ) -> AppResult<Option<ComunicadoRecibido>> {
        let Some(f) = sqlx::query!(
            "SELECT leido_en, enterado_en FROM comunicado_destinatarios
             WHERE persona_id = $1 AND comunicado_id = $2",
            persona.as_uuid(),
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        else {
            return Ok(None);
        };
        Ok(self
            .cargar(id, false)
            .await?
            .map(|comunicado| ComunicadoRecibido {
                comunicado,
                leido_en: f.leido_en,
                enterado_en: f.enterado_en,
            }))
    }

    async fn marcar_leido(
        &mut self,
        persona: PersonaId,
        id: ComunicadoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE comunicado_destinatarios SET leido_en = $3
             WHERE persona_id = $1 AND comunicado_id = $2 AND leido_en IS NULL",
            persona.as_uuid(),
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn marcar_enterado(
        &mut self,
        persona: PersonaId,
        id: ComunicadoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE comunicado_destinatarios
             SET leido_en = coalesce(leido_en, $3), enterado_en = coalesce(enterado_en, $3)
             WHERE persona_id = $1 AND comunicado_id = $2",
            persona.as_uuid(),
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}
