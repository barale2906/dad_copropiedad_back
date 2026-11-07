//! Personas, titularidades e importaciones sobre la transacción del tenant.
//!
//! Las reglas del conjunto de titulares vigentes (suma 100 %, un representante, sin solapes) las
//! repite el trigger diferido `trg_titularidades_validar`, que se evalúa al confirmar.

use application::{
    AppError, AppResult,
    puertos::{
        RepositorioImportaciones, RepositorioTitulares,
        copropiedades::{Importacion, Persona, ResumenImportacion, Titularidad},
    },
};
use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, BienPrivadoId, CuentaId, DireccionCorreo, Documento, EstadoImportacion,
    ImportacionId, PersonaId, Porcentaje, Telefono, TipoImportacion, TipoPersona, TipoTitularidad,
    TitularidadId, importacion::ErrorFila,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{
    TransaccionPg,
    copropiedades::{error_escritura, opcional},
};

struct FilaPersona {
    id: Uuid,
    tipo: String,
    nombre: String,
    documento: Option<String>,
    telefono: Option<String>,
    correo: Option<String>,
    cuenta_id: Option<Uuid>,
}

impl TryFrom<FilaPersona> for Persona {
    type Error = AppError;

    fn try_from(f: FilaPersona) -> AppResult<Self> {
        Ok(Self {
            id: PersonaId::from_uuid(f.id),
            tipo: TipoPersona::try_from(f.tipo.as_str())?,
            nombre: f.nombre,
            documento: opcional(f.documento, Documento::new)?,
            telefono: opcional(f.telefono, Telefono::new)?,
            correo: opcional(f.correo, DireccionCorreo::new)?,
            cuenta_id: f.cuenta_id.map(CuentaId::from_uuid),
        })
    }
}

struct FilaTitularidad {
    id: Uuid,
    bien_privado_id: Uuid,
    persona_id: Uuid,
    tipo: String,
    porcentaje: Decimal,
    es_representante: bool,
    vigente_desde: DateTime<Utc>,
    vigente_hasta: Option<DateTime<Utc>>,
    soporte_archivo_id: Option<Uuid>,
}

impl TryFrom<FilaTitularidad> for Titularidad {
    type Error = AppError;

    fn try_from(f: FilaTitularidad) -> AppResult<Self> {
        Ok(Self {
            id: TitularidadId::from_uuid(f.id),
            bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
            persona_id: PersonaId::from_uuid(f.persona_id),
            tipo: TipoTitularidad::try_from(f.tipo.as_str())?,
            porcentaje: Porcentaje::new(f.porcentaje)?,
            es_representante: f.es_representante,
            vigente_desde: f.vigente_desde,
            vigente_hasta: f.vigente_hasta,
            soporte_archivo_id: f.soporte_archivo_id.map(ArchivoId::from_uuid),
        })
    }
}

impl TransaccionPg {
    async fn personas_donde(
        &mut self,
        id: Option<Uuid>,
        documento: Option<&str>,
    ) -> AppResult<Option<Persona>> {
        sqlx::query_as!(
            FilaPersona,
            "SELECT id, tipo, nombre, documento, telefono, correo, cuenta_id FROM personas
             WHERE ($1::uuid IS NULL OR id = $1) AND ($2::text IS NULL OR documento = $2)
             LIMIT 1",
            id,
            documento,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Persona::try_from)
        .transpose()
    }

    async fn titularidades(
        &mut self,
        bien: BienPrivadoId,
        vigentes: bool,
    ) -> AppResult<Vec<Titularidad>> {
        sqlx::query_as!(
            FilaTitularidad,
            "SELECT id, bien_privado_id, persona_id, tipo, porcentaje, es_representante,
                    vigente_desde, vigente_hasta, soporte_archivo_id
             FROM titularidades
             WHERE bien_privado_id = $1 AND (NOT $2 OR vigente_hasta IS NULL)
             ORDER BY vigente_desde DESC, es_representante DESC, id",
            bien.as_uuid(),
            vigentes,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Titularidad::try_from)
        .collect()
    }

    /// Serializa los cambios de titulares de un mismo bien privado: dos transferencias
    /// simultáneas no pueden crear dos conjuntos vigentes (la segunda espera y su trigger
    /// diferido ve a la primera).
    async fn bloquear_bien(&mut self, bien: BienPrivadoId) -> AppResult<()> {
        sqlx::query!(
            "SELECT id FROM bienes_privados WHERE id = $1 FOR UPDATE",
            bien.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

impl RepositorioTitulares for TransaccionPg {
    async fn persona(&mut self, id: PersonaId) -> AppResult<Option<Persona>> {
        self.personas_donde(Some(id.as_uuid()), None).await
    }

    async fn persona_por_documento(&mut self, documento: &Documento) -> AppResult<Option<Persona>> {
        self.personas_donde(None, Some(documento.as_str())).await
    }

    async fn insertar_persona(&mut self, p: &Persona) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO personas (id, tenant_id, tipo, nombre, documento, telefono, correo, cuenta_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            p.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            p.tipo.as_str(),
            p.nombre,
            p.documento.as_ref().map(Documento::as_str),
            p.telefono.as_ref().map(Telefono::as_str),
            p.correo.as_ref().map(DireccionCorreo::as_str),
            p.cuenta_id.map(CuentaId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(e, "el documento o la cuenta ya están registrados", "cuenta")
        })?;
        Ok(())
    }

    async fn vincular_cuenta(&mut self, persona: PersonaId, cuenta: CuentaId) -> AppResult<()> {
        sqlx::query!(
            "UPDATE personas SET cuenta_id = $2 WHERE id = $1",
            persona.as_uuid(),
            cuenta.as_uuid(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "la cuenta ya está vinculada a otra persona", "cuenta"))?;
        Ok(())
    }

    async fn titulares_vigentes(&mut self, bien: BienPrivadoId) -> AppResult<Vec<Titularidad>> {
        self.titularidades(bien, true).await
    }

    async fn historial_titulares(&mut self, bien: BienPrivadoId) -> AppResult<Vec<Titularidad>> {
        self.titularidades(bien, false).await
    }

    async fn insertar_titularidad(&mut self, t: &Titularidad) -> AppResult<()> {
        self.bloquear_bien(t.bien_privado_id).await?;
        sqlx::query!(
            "INSERT INTO titularidades (id, tenant_id, bien_privado_id, persona_id, tipo, porcentaje,
                                        es_representante, vigente_desde, vigente_hasta,
                                        soporte_archivo_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            t.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            t.bien_privado_id.as_uuid(),
            t.persona_id.as_uuid(),
            t.tipo.as_str(),
            t.porcentaje.valor(),
            t.es_representante,
            t.vigente_desde,
            t.vigente_hasta,
            t.soporte_archivo_id.map(ArchivoId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(e, "la persona ya es titular vigente del bien privado", "bien privado o persona")
        })?;
        Ok(())
    }

    async fn terminar_titulares(
        &mut self,
        bien: BienPrivadoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<TitularidadId>> {
        self.bloquear_bien(bien).await?;
        Ok(sqlx::query_scalar!(
            "UPDATE titularidades SET vigente_hasta = greatest($2, vigente_desde)
             WHERE bien_privado_id = $1 AND vigente_hasta IS NULL
             RETURNING id",
            bien.as_uuid(),
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TitularidadId::from_uuid)
        .collect())
    }

    async fn designar_representante(
        &mut self,
        bien: BienPrivadoId,
        persona: PersonaId,
    ) -> AppResult<()> {
        self.bloquear_bien(bien).await?;
        // Primero se quita al actual: el índice único de representante no es diferible.
        sqlx::query!(
            "UPDATE titularidades SET es_representante = FALSE
             WHERE bien_privado_id = $1 AND vigente_hasta IS NULL AND es_representante",
            bien.as_uuid(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query!(
            "UPDATE titularidades SET es_representante = TRUE
             WHERE id = (SELECT id FROM titularidades
                         WHERE bien_privado_id = $1 AND persona_id = $2 AND vigente_hasta IS NULL
                         ORDER BY id LIMIT 1)",
            bien.as_uuid(),
            persona.as_uuid(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn bienes_de_cuenta(&mut self, cuenta: CuentaId) -> AppResult<Vec<BienPrivadoId>> {
        Ok(sqlx::query_scalar!(
            "SELECT DISTINCT t.bien_privado_id
             FROM titularidades t JOIN personas p ON p.id = t.persona_id
             WHERE p.cuenta_id = $1 AND t.vigente_hasta IS NULL
             ORDER BY 1",
            cuenta.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(BienPrivadoId::from_uuid)
        .collect())
    }
}

impl RepositorioImportaciones for TransaccionPg {
    async fn insertar_importacion(&mut self, i: &Importacion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO importaciones (id, tenant_id, tipo, archivo_id, estado, total_filas,
                                        filas_validas, errores, resumen, creada_por, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            i.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            i.tipo.as_str(),
            i.archivo_id.as_uuid(),
            i.estado.as_str(),
            i32::try_from(i.total_filas).unwrap_or(i32::MAX),
            i32::try_from(i.filas_validas).unwrap_or(i32::MAX),
            serde_json::to_value(&i.errores).map_err(AppError::infra)?,
            serde_json::to_value(&i.resumen).map_err(AppError::infra)?,
            i.creada_por.map(CuentaId::as_uuid),
            i.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn importacion(&mut self, id: ImportacionId) -> AppResult<Option<Importacion>> {
        let Some(f) = sqlx::query!(
            "SELECT id, tipo, archivo_id, estado, total_filas, filas_validas, errores, resumen,
                    creada_por, creada_en, aplicada_en
             FROM importaciones WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        else {
            return Ok(None);
        };
        Ok(Some(Importacion {
            id: ImportacionId::from_uuid(f.id),
            tipo: TipoImportacion::try_from(f.tipo.as_str())?,
            archivo_id: ArchivoId::from_uuid(f.archivo_id),
            estado: EstadoImportacion::try_from(f.estado.as_str())?,
            total_filas: u32::try_from(f.total_filas).unwrap_or(0),
            filas_validas: u32::try_from(f.filas_validas).unwrap_or(0),
            errores: serde_json::from_value::<Vec<ErrorFila>>(f.errores)
                .map_err(AppError::infra)?,
            resumen: serde_json::from_value::<ResumenImportacion>(f.resumen)
                .map_err(AppError::infra)?,
            creada_por: f.creada_por.map(CuentaId::from_uuid),
            creada_en: f.creada_en,
            aplicada_en: f.aplicada_en,
        }))
    }

    async fn marcar_aplicada(
        &mut self,
        id: ImportacionId,
        resumen: &ResumenImportacion,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE importaciones SET estado = 'aplicada', resumen = $2, aplicada_en = $3
             WHERE id = $1",
            id.as_uuid(),
            serde_json::to_value(resumen).map_err(AppError::infra)?,
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}
