//! Extractos y conciliación bancaria sobre PostgreSQL (plan 13 fase 3). Las conciliaciones y
//! sus partidas son append-only: una conciliación vigente es la que ningún registro deshizo.

use application::{
    AppError, AppResult,
    puertos::tesoreria::{
        AccionConciliacion, Conciliacion, ExtractoBancario, LineaBanco, MovimientoExtracto,
        RepositorioConciliacion,
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, ComprobanteId, ConciliacionId, CuentaContableId, CuentaId, CuentaTesoreriaId,
    ExtractoBancarioId, MovimientoExtractoId,
    contabilidad::tesoreria::{EstadoMovimientoExtracto, FormatoExtracto, ModoConciliacion},
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{
    TransaccionPg,
    contabilidad::{error_contable, u32_de},
};

struct FilaMovimiento {
    id: Uuid,
    extracto_id: Uuid,
    cuenta_tesoreria_id: Uuid,
    orden: i32,
    fecha: NaiveDate,
    descripcion: String,
    referencia: Option<String>,
    valor: Decimal,
    estado: String,
}

impl TryFrom<FilaMovimiento> for MovimientoExtracto {
    type Error = AppError;

    fn try_from(f: FilaMovimiento) -> AppResult<Self> {
        Ok(Self {
            id: MovimientoExtractoId::from_uuid(f.id),
            extracto_id: ExtractoBancarioId::from_uuid(f.extracto_id),
            cuenta_tesoreria_id: CuentaTesoreriaId::from_uuid(f.cuenta_tesoreria_id),
            orden: u32_de(f.orden, "orden")?,
            fecha: f.fecha,
            descripcion: f.descripcion,
            referencia: f.referencia,
            valor: f.valor,
            estado: EstadoMovimientoExtracto::try_from(f.estado.as_str())?,
        })
    }
}

struct FilaConciliacion {
    id: Uuid,
    cuenta_tesoreria_id: Uuid,
    accion: String,
    modo: String,
    deshace_a: Option<Uuid>,
    por: Option<Uuid>,
    en: DateTime<Utc>,
    deshecha: bool,
}

impl TransaccionPg {
    async fn con_partidas(&mut self, filas: Vec<FilaConciliacion>) -> AppResult<Vec<Conciliacion>> {
        let ids: Vec<Uuid> = filas.iter().map(|f| f.id).collect();
        let partidas = sqlx::query!(
            "SELECT conciliacion_id, movimiento_id, comprobante_id, linea_orden
             FROM conciliacion_partidas WHERE conciliacion_id = ANY($1)",
            &ids
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        filas
            .into_iter()
            .map(|f| {
                let propias = partidas.iter().filter(|p| p.conciliacion_id == f.id);
                let movimientos = propias
                    .clone()
                    .filter_map(|p| p.movimiento_id.map(MovimientoExtractoId::from_uuid))
                    .collect();
                let lineas = propias
                    .filter_map(|p| p.comprobante_id.zip(p.linea_orden))
                    .map(|(c, o)| {
                        u16::try_from(o)
                            .map(|o| (ComprobanteId::from_uuid(c), o))
                            .map_err(|_| AppError::Validacion("orden de línea inválido".into()))
                    })
                    .collect::<AppResult<Vec<_>>>()?;
                let accion = match (f.accion.as_str(), f.deshace_a) {
                    ("deshacer", Some(d)) => {
                        AccionConciliacion::Deshacer(ConciliacionId::from_uuid(d))
                    }
                    _ => AccionConciliacion::Conciliar,
                };
                Ok(Conciliacion {
                    id: ConciliacionId::from_uuid(f.id),
                    cuenta_tesoreria_id: CuentaTesoreriaId::from_uuid(f.cuenta_tesoreria_id),
                    accion,
                    modo: ModoConciliacion::try_from(f.modo.as_str())?,
                    por: f.por.map(CuentaId::from_uuid),
                    en: f.en,
                    movimientos,
                    lineas,
                    deshecha: f.deshecha,
                })
            })
            .collect()
    }
}

struct FilaExtracto {
    id: Uuid,
    cuenta_tesoreria_id: Uuid,
    desde: NaiveDate,
    hasta: NaiveDate,
    formato: String,
    version_lector: String,
    archivo_id: Uuid,
    sha256: String,
    saldo_inicial: Option<Decimal>,
    saldo_final: Option<Decimal>,
    cargado_por: Option<Uuid>,
    cargado_en: DateTime<Utc>,
    movimientos: i64,
}

impl TryFrom<FilaExtracto> for ExtractoBancario {
    type Error = AppError;

    fn try_from(f: FilaExtracto) -> AppResult<Self> {
        Ok(Self {
            id: ExtractoBancarioId::from_uuid(f.id),
            cuenta_tesoreria_id: CuentaTesoreriaId::from_uuid(f.cuenta_tesoreria_id),
            desde: f.desde,
            hasta: f.hasta,
            formato: FormatoExtracto::try_from(f.formato.as_str())?,
            version_lector: f.version_lector,
            archivo_id: ArchivoId::from_uuid(f.archivo_id),
            sha256: f.sha256,
            saldo_inicial: f.saldo_inicial,
            saldo_final: f.saldo_final,
            cargado_por: f.cargado_por.map(CuentaId::from_uuid),
            cargado_en: f.cargado_en,
            movimientos: u32::try_from(f.movimientos).unwrap_or(u32::MAX),
        })
    }
}

impl RepositorioConciliacion for TransaccionPg {
    async fn extracto_por_hash(
        &mut self,
        cuenta: CuentaTesoreriaId,
        sha256: &str,
    ) -> AppResult<Option<ExtractoBancarioId>> {
        Ok(sqlx::query_scalar!(
            "SELECT id FROM extractos_bancarios WHERE cuenta_tesoreria_id = $1 AND sha256 = $2",
            cuenta.as_uuid(),
            sha256
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(ExtractoBancarioId::from_uuid))
    }

    async fn insertar_extracto(
        &mut self,
        e: &ExtractoBancario,
        movimientos: &[MovimientoExtracto],
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO extractos_bancarios (id, tenant_id, cuenta_tesoreria_id, desde, hasta,
                 formato, version_lector, archivo_id, sha256, saldo_inicial, saldo_final,
                 cargado_por, cargado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            e.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            e.cuenta_tesoreria_id.as_uuid(),
            e.desde,
            e.hasta,
            e.formato.as_str(),
            e.version_lector,
            e.archivo_id.as_uuid(),
            e.sha256,
            e.saldo_inicial,
            e.saldo_final,
            e.cargado_por.map(CuentaId::as_uuid),
            e.cargado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|x| {
            error_contable(
                x,
                "ese extracto ya se importó en la cuenta",
                "cuenta o archivo",
            )
        })?;
        for m in movimientos {
            sqlx::query!(
                "INSERT INTO movimientos_extracto (id, tenant_id, extracto_id, cuenta_tesoreria_id,
                     orden, fecha, descripcion, referencia, valor, estado)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                m.id.as_uuid(),
                self.ctx.tenant_id.as_uuid(),
                m.extracto_id.as_uuid(),
                m.cuenta_tesoreria_id.as_uuid(),
                i32::try_from(m.orden).unwrap_or(i32::MAX),
                m.fecha,
                m.descripcion,
                m.referencia,
                m.valor,
                m.estado.as_str(),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|x| error_contable(x, "movimiento repetido", "extracto"))?;
        }
        Ok(())
    }

    async fn extractos(&mut self, cuenta: CuentaTesoreriaId) -> AppResult<Vec<ExtractoBancario>> {
        sqlx::query_as!(
            FilaExtracto,
            "SELECT e.id, e.cuenta_tesoreria_id, e.desde, e.hasta, e.formato, e.version_lector,
                    e.archivo_id, e.sha256, e.saldo_inicial, e.saldo_final, e.cargado_por,
                    e.cargado_en,
                    (SELECT count(*) FROM movimientos_extracto m
                     WHERE m.tenant_id = e.tenant_id AND m.extracto_id = e.id) AS \"movimientos!\"
             FROM extractos_bancarios e WHERE e.cuenta_tesoreria_id = $1
             ORDER BY e.hasta DESC, e.cargado_en DESC",
            cuenta.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn movimientos_extracto(
        &mut self,
        cuenta: CuentaTesoreriaId,
        desde: NaiveDate,
        hasta: NaiveDate,
        estado: Option<EstadoMovimientoExtracto>,
    ) -> AppResult<Vec<MovimientoExtracto>> {
        sqlx::query_as!(
            FilaMovimiento,
            "SELECT id, extracto_id, cuenta_tesoreria_id, orden, fecha, descripcion, referencia,
                    valor, estado
             FROM movimientos_extracto
             WHERE cuenta_tesoreria_id = $1 AND fecha BETWEEN $2 AND $3
               AND ($4::text IS NULL OR estado = $4)
             ORDER BY fecha, extracto_id, orden",
            cuenta.as_uuid(),
            desde,
            hasta,
            estado.map(EstadoMovimientoExtracto::as_str),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn movimientos_por_id(
        &mut self,
        ids: &[MovimientoExtractoId],
    ) -> AppResult<Vec<MovimientoExtracto>> {
        let ids: Vec<Uuid> = ids.iter().map(|i| i.as_uuid()).collect();
        sqlx::query_as!(
            FilaMovimiento,
            "SELECT id, extracto_id, cuenta_tesoreria_id, orden, fecha, descripcion, referencia,
                    valor, estado
             FROM movimientos_extracto WHERE id = ANY($1) ORDER BY fecha, orden FOR UPDATE",
            &ids
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn cambiar_estado_movimientos(
        &mut self,
        ids: &[MovimientoExtractoId],
        estado: EstadoMovimientoExtracto,
    ) -> AppResult<()> {
        let ids: Vec<Uuid> = ids.iter().map(|i| i.as_uuid()).collect();
        sqlx::query!(
            "UPDATE movimientos_extracto SET estado = $2 WHERE id = ANY($1)",
            &ids,
            estado.as_str()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn lineas_banco(
        &mut self,
        cuenta: CuentaContableId,
        desde: Option<NaiveDate>,
        hasta: NaiveDate,
        solo_pendientes: bool,
    ) -> AppResult<Vec<LineaBanco>> {
        let filas = sqlx::query!(
            "SELECT l.comprobante_id, l.orden, t.codigo AS tipo_codigo, c.numero AS \"numero!\",
                    c.fecha, c.descripcion, (l.debito - l.credito) AS \"valor!\",
                    (SELECT k.id FROM conciliacion_partidas p
                     JOIN conciliaciones k ON k.tenant_id = p.tenant_id AND k.id = p.conciliacion_id
                     WHERE p.tenant_id = l.tenant_id AND p.comprobante_id = l.comprobante_id
                       AND p.linea_orden = l.orden AND k.accion = 'conciliar'
                       AND NOT EXISTS (SELECT 1 FROM conciliaciones d
                                       WHERE d.tenant_id = k.tenant_id AND d.deshace_a = k.id)
                     LIMIT 1) AS conciliacion_id
             FROM lineas_comprobante l
             JOIN comprobantes c ON c.tenant_id = l.tenant_id AND c.id = l.comprobante_id
             JOIN tipos_comprobante t ON t.tenant_id = c.tenant_id AND t.id = c.tipo_comprobante_id
             WHERE l.cuenta_id = $1 AND c.estado = 'contabilizado'
               AND ($2::date IS NULL OR c.fecha >= $2) AND c.fecha <= $3
             ORDER BY c.fecha, c.numero, l.orden",
            cuenta.as_uuid(),
            desde,
            hasta,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        filas
            .into_iter()
            .filter(|f| !solo_pendientes || f.conciliacion_id.is_none())
            .map(|f| {
                Ok(LineaBanco {
                    comprobante_id: ComprobanteId::from_uuid(f.comprobante_id),
                    orden: u16::try_from(f.orden)
                        .map_err(|_| AppError::Validacion("orden de línea inválido".into()))?,
                    tipo_codigo: f.tipo_codigo,
                    numero: u32_de(f.numero, "numero")?,
                    fecha: f.fecha,
                    descripcion: f.descripcion,
                    valor: f.valor,
                    conciliacion_id: f.conciliacion_id.map(ConciliacionId::from_uuid),
                })
            })
            .collect()
    }

    async fn saldo_libros(
        &mut self,
        cuenta: CuentaContableId,
        hasta: NaiveDate,
    ) -> AppResult<Decimal> {
        sqlx::query_scalar!(
            "SELECT coalesce(sum(l.debito - l.credito), 0) AS \"saldo!\"
             FROM lineas_comprobante l
             JOIN comprobantes c ON c.tenant_id = l.tenant_id AND c.id = l.comprobante_id
             WHERE l.cuenta_id = $1 AND c.estado = 'contabilizado' AND c.fecha <= $2",
            cuenta.as_uuid(),
            hasta
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn insertar_conciliacion(&mut self, c: &Conciliacion) -> AppResult<()> {
        let (accion, deshace_a) = match c.accion {
            AccionConciliacion::Conciliar => ("conciliar", None),
            AccionConciliacion::Deshacer(id) => ("deshacer", Some(id.as_uuid())),
        };
        sqlx::query!(
            "INSERT INTO conciliaciones (id, tenant_id, cuenta_tesoreria_id, accion, modo, deshace_a,
                 por, en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.cuenta_tesoreria_id.as_uuid(),
            accion,
            c.modo.as_str(),
            deshace_a,
            c.por.map(CuentaId::as_uuid),
            c.en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|x| error_contable(x, "la conciliación ya se deshizo", "cuenta o conciliación"))?;
        for m in &c.movimientos {
            sqlx::query!(
                "INSERT INTO conciliacion_partidas (tenant_id, conciliacion_id, movimiento_id)
                 VALUES ($1, $2, $3)",
                self.ctx.tenant_id.as_uuid(),
                c.id.as_uuid(),
                m.as_uuid(),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|x| error_contable(x, "partida repetida", "movimiento del extracto"))?;
        }
        for (comprobante, orden) in &c.lineas {
            sqlx::query!(
                "INSERT INTO conciliacion_partidas (tenant_id, conciliacion_id, comprobante_id,
                     linea_orden)
                 VALUES ($1, $2, $3, $4)",
                self.ctx.tenant_id.as_uuid(),
                c.id.as_uuid(),
                comprobante.as_uuid(),
                i16::try_from(*orden).unwrap_or(i16::MAX),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|x| error_contable(x, "partida repetida", "línea contable"))?;
        }
        Ok(())
    }

    async fn conciliacion(&mut self, id: ConciliacionId) -> AppResult<Option<Conciliacion>> {
        let filas = sqlx::query_as!(
            FilaConciliacion,
            "SELECT k.id, k.cuenta_tesoreria_id, k.accion, k.modo, k.deshace_a, k.por, k.en,
                    EXISTS (SELECT 1 FROM conciliaciones d
                            WHERE d.tenant_id = k.tenant_id AND d.deshace_a = k.id) AS \"deshecha!\"
             FROM conciliaciones k WHERE k.id = $1",
            id.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(self.con_partidas(filas).await?.pop())
    }

    async fn conciliaciones(
        &mut self,
        cuenta: CuentaTesoreriaId,
        limite: u32,
    ) -> AppResult<Vec<Conciliacion>> {
        let filas = sqlx::query_as!(
            FilaConciliacion,
            "SELECT k.id, k.cuenta_tesoreria_id, k.accion, k.modo, k.deshace_a, k.por, k.en,
                    EXISTS (SELECT 1 FROM conciliaciones d
                            WHERE d.tenant_id = k.tenant_id AND d.deshace_a = k.id) AS \"deshecha!\"
             FROM conciliaciones k WHERE k.cuenta_tesoreria_id = $1
             ORDER BY k.en DESC, k.id DESC LIMIT $2",
            cuenta.as_uuid(),
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        self.con_partidas(filas).await
    }
}
