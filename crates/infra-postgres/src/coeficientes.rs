//! Cuadros de coeficientes sobre la transacción del tenant. Las reglas de activación (suma
//! exacta, bienes completos, sin reunión instalada) y la copia a `bienes_privados.coeficiente`
//! las repiten los triggers de la migración `copropiedades_y_bienes_privados`.

use std::collections::BTreeMap;

use application::{
    AppError, AppResult,
    puertos::{RepositorioCoeficientes, copropiedades::CuadroCoeficientes},
};
use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, BienPrivadoId, Coeficiente, CuadroCoeficientesId, CuentaId, EstadoCuadro, TipoCuadro,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{TransaccionPg, copropiedades::error_escritura};

struct FilaCuadro {
    id: Uuid,
    version: i32,
    tipo: String,
    estado: String,
    descripcion: Option<String>,
    soporte_archivo_id: Option<Uuid>,
    vigente_desde: Option<DateTime<Utc>>,
    aprobado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
}

impl TryFrom<FilaCuadro> for CuadroCoeficientes {
    type Error = AppError;

    fn try_from(f: FilaCuadro) -> AppResult<Self> {
        Ok(Self {
            id: CuadroCoeficientesId::from_uuid(f.id),
            version: u32::try_from(f.version).unwrap_or(0),
            tipo: TipoCuadro::try_from(f.tipo.as_str())?,
            estado: EstadoCuadro::try_from(f.estado.as_str())?,
            descripcion: f.descripcion,
            soporte_archivo_id: f.soporte_archivo_id.map(ArchivoId::from_uuid),
            vigente_desde: f.vigente_desde,
            aprobado_por: f.aprobado_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
        })
    }
}

impl TransaccionPg {
    async fn cuadros_donde(
        &mut self,
        id: Option<Uuid>,
        solo_vigente: bool,
    ) -> AppResult<Vec<CuadroCoeficientes>> {
        sqlx::query_as!(
            FilaCuadro,
            "SELECT id, version, tipo, estado, descripcion, soporte_archivo_id, vigente_desde,
                    aprobado_por, creado_en
             FROM cuadros_coeficientes
             WHERE ($1::uuid IS NULL OR id = $1)
               AND (NOT $2 OR (estado = 'vigente' AND tipo = 'copropiedad'))
             ORDER BY version DESC",
            id,
            solo_vigente,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(CuadroCoeficientes::try_from)
        .collect()
    }
}

impl RepositorioCoeficientes for TransaccionPg {
    async fn cuadros(&mut self) -> AppResult<Vec<CuadroCoeficientes>> {
        self.cuadros_donde(None, false).await
    }

    async fn cuadro(&mut self, id: CuadroCoeficientesId) -> AppResult<Option<CuadroCoeficientes>> {
        Ok(self.cuadros_donde(Some(id.as_uuid()), false).await?.pop())
    }

    async fn cuadro_vigente(&mut self) -> AppResult<Option<CuadroCoeficientes>> {
        Ok(self.cuadros_donde(None, true).await?.pop())
    }

    async fn ultima_version(&mut self) -> AppResult<u32> {
        let v = sqlx::query_scalar!(
            r#"SELECT coalesce(max(version), 0) AS "v!" FROM cuadros_coeficientes
               WHERE tipo = 'copropiedad'"#
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(u32::try_from(v).unwrap_or(0))
    }

    async fn insertar_cuadro(&mut self, c: &CuadroCoeficientes) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO cuadros_coeficientes
                 (id, tenant_id, version, tipo, estado, descripcion, soporte_archivo_id, creado_en)
             VALUES ($1, $2, $3, $4, 'borrador', $5, $6, $7)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            i32::try_from(c.version).unwrap_or(i32::MAX),
            c.tipo.as_str(),
            c.descripcion,
            c.soporte_archivo_id.map(ArchivoId::as_uuid),
            c.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "ya existe un cuadro con esa versión", "archivo"))?;
        Ok(())
    }

    async fn coeficientes(
        &mut self,
        cuadro: CuadroCoeficientesId,
    ) -> AppResult<BTreeMap<BienPrivadoId, Coeficiente>> {
        sqlx::query!(
            "SELECT bien_privado_id, coeficiente FROM coeficientes_bien_privado
             WHERE cuadro_id = $1",
            cuadro.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok((
                BienPrivadoId::from_uuid(f.bien_privado_id),
                Coeficiente::new(f.coeficiente)?,
            ))
        })
        .collect()
    }

    async fn reemplazar_coeficientes(
        &mut self,
        cuadro: CuadroCoeficientesId,
        coeficientes: &BTreeMap<BienPrivadoId, Coeficiente>,
    ) -> AppResult<()> {
        sqlx::query!(
            "DELETE FROM coeficientes_bien_privado WHERE cuadro_id = $1",
            cuadro.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "", "cuadro de coeficientes"))?;
        let bienes: Vec<Uuid> = coeficientes.keys().map(|b| b.as_uuid()).collect();
        let valores: Vec<Decimal> = coeficientes.values().map(|c| c.valor()).collect();
        sqlx::query!(
            "INSERT INTO coeficientes_bien_privado (tenant_id, cuadro_id, bien_privado_id, coeficiente)
             SELECT $1, $2, b, c FROM unnest($3::uuid[], $4::numeric[]) AS t (b, c)",
            self.ctx.tenant_id.as_uuid(),
            cuadro.as_uuid(),
            &bienes,
            &valores,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "bien privado repetido en el cuadro", "bien privado"))?;
        Ok(())
    }

    async fn activar(
        &mut self,
        cuadro: CuadroCoeficientesId,
        anterior: Option<CuadroCoeficientesId>,
        ahora: DateTime<Utc>,
        aprobado_por: Option<CuentaId>,
    ) -> AppResult<()> {
        if let Some(a) = anterior {
            sqlx::query!(
                "UPDATE cuadros_coeficientes SET estado = 'historico' WHERE id = $1",
                a.as_uuid()
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_escritura(e, "", "cuadro de coeficientes"))?;
        }
        sqlx::query!(
            "UPDATE cuadros_coeficientes
             SET estado = 'vigente', vigente_desde = $2, aprobado_por = $3
             WHERE id = $1",
            cuadro.as_uuid(),
            ahora,
            aprobado_por.map(CuentaId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "ya hay otro cuadro vigente", "cuadro de coeficientes"))?;
        Ok(())
    }

    async fn reuniones_instaladas(&mut self) -> AppResult<u64> {
        let n = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM reuniones WHERE estado = 'instalada'"#
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}
