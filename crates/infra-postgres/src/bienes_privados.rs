//! Agrupaciones, bienes privados y usos exclusivos sobre la transacción del tenant.

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::{
        RepositorioBienesPrivados,
        copropiedades::{Agrupacion, BienPrivado, FiltroBienes, UsoExclusivo},
    },
};
use chrono::{DateTime, Utc};
use domain::{
    AgrupacionId, BienPrivadoId, CodigoAgrupacion, CodigoBienPrivado, Coeficiente,
    NumeroBienPrivado, TipoAgrupacion, TipoBienPrivado, TipoUsoExclusivo, UsoBienPrivado,
    UsoExclusivoId,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{
    TransaccionPg,
    copropiedades::{error_escritura, opcional},
};

struct FilaBien {
    id: Uuid,
    codigo: String,
    tipo: String,
    agrupacion_id: Option<Uuid>,
    numero: Option<String>,
    piso: Option<i16>,
    area_privada_m2: Option<Decimal>,
    matricula_inmobiliaria: Option<String>,
    cedula_catastral: Option<String>,
    uso: String,
    activo: bool,
    bien_principal_id: Option<Uuid>,
    coeficiente: Option<Decimal>,
}

impl TryFrom<FilaBien> for BienPrivado {
    type Error = AppError;

    fn try_from(f: FilaBien) -> AppResult<Self> {
        Ok(Self {
            id: BienPrivadoId::from_uuid(f.id),
            codigo: CodigoBienPrivado::new(&f.codigo)?,
            tipo: TipoBienPrivado::try_from(f.tipo.as_str())?,
            agrupacion_id: f.agrupacion_id.map(AgrupacionId::from_uuid),
            numero: opcional(f.numero, NumeroBienPrivado::new)?,
            piso: f.piso,
            area_privada_m2: f.area_privada_m2,
            matricula_inmobiliaria: f.matricula_inmobiliaria,
            cedula_catastral: f.cedula_catastral,
            uso: UsoBienPrivado::try_from(f.uso.as_str())?,
            activo: f.activo,
            bien_principal_id: f.bien_principal_id.map(BienPrivadoId::from_uuid),
            coeficiente: f.coeficiente.map(Coeficiente::new).transpose()?,
        })
    }
}

/// Escapa los comodines de `LIKE` del texto que escribe el usuario.
fn patron_like(texto: &str) -> String {
    let escapado = texto
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escapado}%")
}

impl RepositorioBienesPrivados for TransaccionPg {
    async fn agrupaciones(&mut self) -> AppResult<Vec<Agrupacion>> {
        sqlx::query!(
            "SELECT id, tipo, nombre, codigo, padre_id, orden FROM agrupaciones
             ORDER BY orden, normalizar(nombre), id"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(Agrupacion {
                id: AgrupacionId::from_uuid(f.id),
                tipo: TipoAgrupacion::try_from(f.tipo.as_str())?,
                nombre: f.nombre,
                codigo: CodigoAgrupacion::new(&f.codigo)?,
                padre_id: f.padre_id.map(AgrupacionId::from_uuid),
                orden: f.orden,
            })
        })
        .collect()
    }

    async fn insertar_agrupacion(&mut self, a: &Agrupacion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO agrupaciones (id, tenant_id, tipo, nombre, codigo, padre_id, orden)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            a.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            a.tipo.as_str(),
            a.nombre,
            a.codigo.as_str(),
            a.padre_id.map(AgrupacionId::as_uuid),
            a.orden,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "ya hay una agrupación con ese código en ese nivel",
                "agrupación",
            )
        })?;
        Ok(())
    }

    async fn bien(&mut self, id: BienPrivadoId) -> AppResult<Option<BienPrivado>> {
        sqlx::query_as!(
            FilaBien,
            "SELECT id, codigo, tipo, agrupacion_id, numero, piso, area_privada_m2,
                    matricula_inmobiliaria, cedula_catastral, uso, activo, bien_principal_id,
                    coeficiente
             FROM bienes_privados WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(BienPrivado::try_from)
        .transpose()
    }

    async fn todos_los_bienes(&mut self) -> AppResult<Vec<BienPrivado>> {
        sqlx::query_as!(
            FilaBien,
            "SELECT id, codigo, tipo, agrupacion_id, numero, piso, area_privada_m2,
                    matricula_inmobiliaria, cedula_catastral, uso, activo, bien_principal_id,
                    coeficiente
             FROM bienes_privados ORDER BY normalizar(codigo), id",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(BienPrivado::try_from)
        .collect()
    }

    async fn listar_bienes(
        &mut self,
        filtro: &FiltroBienes,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<BienPrivado>> {
        let filas = sqlx::query_as!(
            FilaBien,
            r#"SELECT id, codigo, tipo, agrupacion_id, numero, piso, area_privada_m2,
                      matricula_inmobiliaria, cedula_catastral, uso, activo, bien_principal_id,
                      coeficiente
               FROM bienes_privados
               WHERE ($1::uuid IS NULL OR id > $1)
                 AND ($2 OR activo)
                 AND ($3::text IS NULL OR tipo = $3)
                 AND ($4::uuid IS NULL OR agrupacion_id = $4)
                 AND ($5::text IS NULL OR normalizar(codigo) LIKE normalizar($5) ESCAPE '\')
               ORDER BY id
               LIMIT $6"#,
            pagina.despues(),
            filtro.incluir_inactivos,
            filtro.tipo.map(TipoBienPrivado::as_str),
            filtro.agrupacion_id.map(AgrupacionId::as_uuid),
            filtro.texto.as_deref().map(patron_like),
            pagina.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(BienPrivado::try_from)
        .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(filas, pagina, |b| b.id.as_uuid()))
    }

    async fn insertar_bien(&mut self, b: &BienPrivado) -> AppResult<()> {
        // Sin coeficiente: lo fija la activación de un cuadro (trigger).
        sqlx::query!(
            "INSERT INTO bienes_privados
                 (id, tenant_id, codigo, coeficiente, activo, tipo, agrupacion_id, numero, piso,
                  area_privada_m2, matricula_inmobiliaria, cedula_catastral, uso,
                  bien_principal_id)
             VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            b.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            b.codigo.as_str(),
            b.activo,
            b.tipo.as_str(),
            b.agrupacion_id.map(AgrupacionId::as_uuid),
            b.numero.as_ref().map(NumeroBienPrivado::as_str),
            b.piso,
            b.area_privada_m2,
            b.matricula_inmobiliaria,
            b.cedula_catastral,
            b.uso.as_str(),
            b.bien_principal_id.map(BienPrivadoId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(e, "ya existe un bien privado con ese código", "agrupación")
        })?;
        Ok(())
    }

    async fn actualizar_bien(&mut self, b: &BienPrivado) -> AppResult<()> {
        sqlx::query!(
            "UPDATE bienes_privados SET
                 codigo = $2, activo = $3, tipo = $4, agrupacion_id = $5, numero = $6, piso = $7,
                 area_privada_m2 = $8, matricula_inmobiliaria = $9, cedula_catastral = $10,
                 uso = $11, bien_principal_id = $12, actualizado_en = now()
             WHERE id = $1",
            b.id.as_uuid(),
            b.codigo.as_str(),
            b.activo,
            b.tipo.as_str(),
            b.agrupacion_id.map(AgrupacionId::as_uuid),
            b.numero.as_ref().map(NumeroBienPrivado::as_str),
            b.piso,
            b.area_privada_m2,
            b.matricula_inmobiliaria,
            b.cedula_catastral,
            b.uso.as_str(),
            b.bien_principal_id.map(BienPrivadoId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(e, "ya existe un bien privado con ese código", "agrupación")
        })?;
        Ok(())
    }

    async fn anexos(&mut self, principal: BienPrivadoId) -> AppResult<Vec<BienPrivado>> {
        sqlx::query_as!(
            FilaBien,
            "SELECT id, codigo, tipo, agrupacion_id, numero, piso, area_privada_m2,
                    matricula_inmobiliaria, cedula_catastral, uso, activo, bien_principal_id,
                    coeficiente
             FROM bienes_privados WHERE bien_principal_id = $1 ORDER BY codigo",
            principal.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(BienPrivado::try_from)
        .collect()
    }

    async fn usos_exclusivos(&mut self, bien: BienPrivadoId) -> AppResult<Vec<UsoExclusivo>> {
        sqlx::query!(
            "SELECT id, bien_privado_id, tipo, codigo, descripcion, vigente_desde, vigente_hasta
             FROM usos_exclusivos WHERE bien_privado_id = $1 ORDER BY vigente_desde DESC, id",
            bien.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(UsoExclusivo {
                id: UsoExclusivoId::from_uuid(f.id),
                bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
                tipo: TipoUsoExclusivo::try_from(f.tipo.as_str())?,
                codigo: f.codigo,
                descripcion: f.descripcion,
                vigente_desde: f.vigente_desde,
                vigente_hasta: f.vigente_hasta,
            })
        })
        .collect()
    }

    async fn insertar_uso_exclusivo(&mut self, u: &UsoExclusivo) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO usos_exclusivos (id, tenant_id, bien_privado_id, tipo, codigo, descripcion,
                                          vigente_desde, vigente_hasta)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            u.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            u.bien_privado_id.as_uuid(),
            u.tipo.as_str(),
            u.codigo,
            u.descripcion,
            u.vigente_desde,
            u.vigente_hasta,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "ese bien común ya está asignado en uso exclusivo",
                "bien privado",
            )
        })?;
        Ok(())
    }

    async fn terminar_uso_exclusivo(
        &mut self,
        bien: BienPrivadoId,
        id: UsoExclusivoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        let existe = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM usos_exclusivos WHERE id = $1 AND bien_privado_id = $2)
                   AS "existe!""#,
            id.as_uuid(),
            bien.as_uuid(),
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        if existe {
            sqlx::query!(
                "UPDATE usos_exclusivos SET vigente_hasta = greatest($2, vigente_desde)
                 WHERE id = $1 AND vigente_hasta IS NULL",
                id.as_uuid(),
                ahora,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        }
        Ok(existe)
    }
}

#[cfg(test)]
mod tests {
    use super::patron_like;

    #[test]
    fn escapa_comodines_de_like() {
        assert_eq!(patron_like("T3"), "%T3%");
        assert_eq!(patron_like("50%_\\"), "%50\\%\\_\\\\%");
    }
}
