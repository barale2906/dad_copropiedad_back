//! Facturas de proveedores y de venta sobre PostgreSQL (plan 13, ADR 0018). La base de datos
//! rechaza la factura de proveedor repetida sin excepción (`ux_factura_proveedor`).

use application::{
    AppError, AppResult,
    puertos::{
        contabilidad::Origen,
        tesoreria::{
            DatosEntidad, FacturaProveedor, FacturaVenta, FilaCertificado, FiltroFacturas,
            LineaFacturaProveedor, LineaFacturaVenta, RepositorioFacturas, RetencionLinea,
        },
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, CentroCostoId, ComprobanteId, ConceptoRetencionId, CuentaContableId, CuentaId,
    FacturaProveedorId, FacturaVentaId, PersonaId,
    contabilidad::{
        Monto,
        tesoreria::{EstadoFacturaProveedor, EstadoFacturaVenta, ImpuestoRetencion},
        tributario::ClasificacionTributaria,
    },
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{
    TransaccionPg,
    contabilidad::{error_contable, i16_de, u32_de},
};

fn monto(v: Decimal) -> AppResult<Monto> {
    Ok(Monto::new(v)?)
}

fn origen(tipo: Option<String>, id: Option<Uuid>) -> Option<Origen> {
    tipo.zip(id).map(|(tipo, id)| Origen { tipo, id })
}

struct FilaFacturaProveedor {
    id: Uuid,
    persona_id: Uuid,
    prefijo: String,
    numero: String,
    fecha: NaiveDate,
    vencimiento: NaiveDate,
    descripcion: String,
    origen_tipo: Option<String>,
    origen_id: Option<Uuid>,
    cufe: Option<String>,
    archivo_id: Option<Uuid>,
    subtotal: Decimal,
    iva: Decimal,
    retenciones: Decimal,
    neto_pagar: Decimal,
    pagado: Decimal,
    estado: String,
    excepcion_duplicado: Option<String>,
    excepcion_autorizada_por: Option<Uuid>,
    comprobante_id: Option<Uuid>,
    motivo_anulacion: Option<String>,
    registrada_por: Option<Uuid>,
    registrada_en: DateTime<Utc>,
}

impl FilaFacturaProveedor {
    fn a_factura(self, lineas: Vec<LineaFacturaProveedor>) -> AppResult<FacturaProveedor> {
        Ok(FacturaProveedor {
            id: FacturaProveedorId::from_uuid(self.id),
            persona_id: PersonaId::from_uuid(self.persona_id),
            prefijo: self.prefijo,
            numero: self.numero,
            fecha: self.fecha,
            vencimiento: self.vencimiento,
            descripcion: self.descripcion,
            origen: origen(self.origen_tipo, self.origen_id),
            cufe: self.cufe,
            archivo_id: self.archivo_id.map(ArchivoId::from_uuid),
            subtotal: monto(self.subtotal)?,
            iva: monto(self.iva)?,
            retenciones: monto(self.retenciones)?,
            neto_pagar: monto(self.neto_pagar)?,
            pagado: monto(self.pagado)?,
            estado: EstadoFacturaProveedor::try_from(self.estado.as_str())?,
            excepcion_duplicado: self.excepcion_duplicado,
            excepcion_autorizada_por: self.excepcion_autorizada_por.map(CuentaId::from_uuid),
            comprobante_id: self.comprobante_id.map(ComprobanteId::from_uuid),
            motivo_anulacion: self.motivo_anulacion,
            registrada_por: self.registrada_por.map(CuentaId::from_uuid),
            registrada_en: self.registrada_en,
            lineas,
        })
    }
}

struct FilaFacturaVenta {
    id: Uuid,
    persona_id: Uuid,
    numero: Option<i32>,
    fecha: NaiveDate,
    vencimiento: NaiveDate,
    descripcion: String,
    origen_tipo: Option<String>,
    origen_id: Option<Uuid>,
    subtotal: Decimal,
    iva: Decimal,
    total: Decimal,
    pagado: Decimal,
    estado: String,
    comprobante_id: Option<Uuid>,
    motivo_anulacion: Option<String>,
    creada_por: Option<Uuid>,
    creada_en: DateTime<Utc>,
    emitida_en: Option<DateTime<Utc>>,
}

impl FilaFacturaVenta {
    fn a_factura(self, lineas: Vec<LineaFacturaVenta>) -> AppResult<FacturaVenta> {
        Ok(FacturaVenta {
            id: FacturaVentaId::from_uuid(self.id),
            persona_id: PersonaId::from_uuid(self.persona_id),
            numero: self.numero.map(|n| u32_de(n, "numero")).transpose()?,
            fecha: self.fecha,
            vencimiento: self.vencimiento,
            descripcion: self.descripcion,
            origen: origen(self.origen_tipo, self.origen_id),
            subtotal: monto(self.subtotal)?,
            iva: monto(self.iva)?,
            total: monto(self.total)?,
            pagado: monto(self.pagado)?,
            estado: EstadoFacturaVenta::try_from(self.estado.as_str())?,
            comprobante_id: self.comprobante_id.map(ComprobanteId::from_uuid),
            motivo_anulacion: self.motivo_anulacion,
            creada_por: self.creada_por.map(CuentaId::from_uuid),
            creada_en: self.creada_en,
            emitida_en: self.emitida_en,
            lineas,
        })
    }
}

impl TransaccionPg {
    async fn lineas_proveedor(&mut self, factura: Uuid) -> AppResult<Vec<LineaFacturaProveedor>> {
        let retenciones = sqlx::query!(
            "SELECT linea_orden, concepto_retencion_id, impuesto, base, tarifa, valor
             FROM retenciones_factura_proveedor WHERE factura_id = $1 ORDER BY linea_orden, impuesto",
            factura
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let lineas = sqlx::query!(
            "SELECT orden, descripcion, cuenta_id, centro_costo_id, base, iva, iva_descontable
             FROM lineas_factura_proveedor WHERE factura_id = $1 ORDER BY orden",
            factura
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        lineas
            .into_iter()
            .map(|l| {
                let propias = retenciones
                    .iter()
                    .filter(|r| r.linea_orden == l.orden)
                    .map(|r| {
                        Ok(RetencionLinea {
                            concepto_id: ConceptoRetencionId::from_uuid(r.concepto_retencion_id),
                            impuesto: ImpuestoRetencion::try_from(r.impuesto.as_str())?,
                            base: monto(r.base)?,
                            tarifa: r.tarifa,
                            valor: monto(r.valor)?,
                        })
                    })
                    .collect::<AppResult<Vec<_>>>()?;
                Ok(LineaFacturaProveedor {
                    descripcion: l.descripcion,
                    cuenta_id: CuentaContableId::from_uuid(l.cuenta_id),
                    centro_costo_id: l.centro_costo_id.map(CentroCostoId::from_uuid),
                    base: monto(l.base)?,
                    iva: monto(l.iva)?,
                    iva_descontable: l.iva_descontable,
                    retenciones: propias,
                })
            })
            .collect()
    }

    async fn lineas_venta(&mut self, factura: Uuid) -> AppResult<Vec<LineaFacturaVenta>> {
        sqlx::query!(
            "SELECT descripcion, cuenta_id, centro_costo_id, cantidad, valor_unitario, tarifa_iva,
                    subtotal, iva, clasificacion
             FROM lineas_factura_venta WHERE factura_id = $1 ORDER BY orden",
            factura
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|l| {
            Ok(LineaFacturaVenta {
                descripcion: l.descripcion,
                cuenta_id: CuentaContableId::from_uuid(l.cuenta_id),
                centro_costo_id: l.centro_costo_id.map(CentroCostoId::from_uuid),
                cantidad: l.cantidad,
                valor_unitario: monto(l.valor_unitario)?,
                tarifa_iva: l.tarifa_iva,
                subtotal: monto(l.subtotal)?,
                iva: monto(l.iva)?,
                clasificacion: ClasificacionTributaria::try_from(l.clasificacion.as_str())?,
            })
        })
        .collect()
    }
}

impl RepositorioFacturas for TransaccionPg {
    async fn datos_entidad(&mut self) -> AppResult<DatosEntidad> {
        let r = sqlx::query!(
            "SELECT nombre, nit FROM entidades WHERE id = $1",
            self.ctx.tenant_id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .ok_or(AppError::NoEncontrado("entidad"))?;
        Ok(DatosEntidad {
            nombre: r.nombre,
            nit: r.nit,
        })
    }

    async fn insertar_factura_proveedor(&mut self, f: &FacturaProveedor) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO facturas_proveedor (id, tenant_id, persona_id, prefijo, numero, fecha,
                 vencimiento, descripcion, origen_tipo, origen_id, cufe, archivo_id, subtotal, iva,
                 retenciones, neto_pagar, pagado, estado, excepcion_duplicado,
                 excepcion_autorizada_por, registrada_por, registrada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18, $19, $20, $21, $22)",
            f.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            f.persona_id.as_uuid(),
            f.prefijo,
            f.numero,
            f.fecha,
            f.vencimiento,
            f.descripcion,
            f.origen.as_ref().map(|o| o.tipo.as_str()),
            f.origen.as_ref().map(|o| o.id),
            f.cufe,
            f.archivo_id.map(ArchivoId::as_uuid),
            f.subtotal.valor(),
            f.iva.valor(),
            f.retenciones.valor(),
            f.neto_pagar.valor(),
            f.pagado.valor(),
            f.estado.as_str(),
            f.excepcion_duplicado,
            f.excepcion_autorizada_por.map(CuentaId::as_uuid),
            f.registrada_por.map(CuentaId::as_uuid),
            f.registrada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "el proveedor ya tiene registrada esa factura (prefijo y número); si es otra, \
                 regístrela con la excepción autorizada",
                "tercero o archivo",
            )
        })?;
        for (i, l) in f.lineas.iter().enumerate() {
            let orden = i16_de(i + 1, "orden")?;
            sqlx::query!(
                "INSERT INTO lineas_factura_proveedor (tenant_id, factura_id, orden, descripcion,
                     cuenta_id, centro_costo_id, base, iva, iva_descontable)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                self.ctx.tenant_id.as_uuid(),
                f.id.as_uuid(),
                orden,
                l.descripcion,
                l.cuenta_id.as_uuid(),
                l.centro_costo_id.map(CentroCostoId::as_uuid),
                l.base.valor(),
                l.iva.valor(),
                l.iva_descontable,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_contable(e, "línea repetida", "cuenta o centro de costo"))?;
            for r in &l.retenciones {
                sqlx::query!(
                    "INSERT INTO retenciones_factura_proveedor (tenant_id, factura_id, linea_orden,
                         concepto_retencion_id, impuesto, base, tarifa, valor)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                    self.ctx.tenant_id.as_uuid(),
                    f.id.as_uuid(),
                    orden,
                    r.concepto_id.as_uuid(),
                    r.impuesto.as_str(),
                    r.base.valor(),
                    r.tarifa,
                    r.valor.valor(),
                )
                .execute(&mut *self.tx)
                .await
                .map_err(|e| error_contable(e, "retención repetida", "concepto de retención"))?;
            }
        }
        Ok(())
    }

    async fn factura_proveedor(
        &mut self,
        id: FacturaProveedorId,
    ) -> AppResult<Option<FacturaProveedor>> {
        let fila = sqlx::query_as!(
            FilaFacturaProveedor,
            "SELECT id, persona_id, prefijo, numero, fecha, vencimiento, descripcion, origen_tipo,
                    origen_id, cufe, archivo_id, subtotal, iva, retenciones, neto_pagar, pagado,
                    estado, excepcion_duplicado, excepcion_autorizada_por, comprobante_id,
                    motivo_anulacion, registrada_por, registrada_en
             FROM facturas_proveedor WHERE id = $1 FOR UPDATE",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        match fila {
            None => Ok(None),
            Some(f) => {
                let lineas = self.lineas_proveedor(f.id).await?;
                Ok(Some(f.a_factura(lineas)?))
            }
        }
    }

    async fn facturas_proveedor(
        &mut self,
        filtro: &FiltroFacturas,
        limite: u32,
    ) -> AppResult<Vec<FacturaProveedor>> {
        sqlx::query_as!(
            FilaFacturaProveedor,
            "SELECT id, persona_id, prefijo, numero, fecha, vencimiento, descripcion, origen_tipo,
                    origen_id, cufe, archivo_id, subtotal, iva, retenciones, neto_pagar, pagado,
                    estado, excepcion_duplicado, excepcion_autorizada_por, comprobante_id,
                    motivo_anulacion, registrada_por, registrada_en
             FROM facturas_proveedor
             WHERE ($1::uuid IS NULL OR persona_id = $1) AND ($2::text IS NULL OR estado = $2)
               AND ($3::date IS NULL OR fecha >= $3) AND ($4::date IS NULL OR fecha <= $4)
             ORDER BY vencimiento, id LIMIT $5",
            filtro.persona_id.map(PersonaId::as_uuid),
            filtro.estado,
            filtro.desde,
            filtro.hasta,
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| f.a_factura(Vec::new()))
        .collect()
    }

    async fn actualizar_factura_proveedor(&mut self, f: &FacturaProveedor) -> AppResult<()> {
        sqlx::query!(
            "UPDATE facturas_proveedor SET estado = $2, pagado = $3, comprobante_id = $4,
                 motivo_anulacion = $5
             WHERE id = $1",
            f.id.as_uuid(),
            f.estado.as_str(),
            f.pagado.valor(),
            f.comprobante_id.map(ComprobanteId::as_uuid),
            f.motivo_anulacion,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "factura", "comprobante"))?;
        Ok(())
    }

    async fn retenciones_de_tercero(
        &mut self,
        persona: PersonaId,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> AppResult<Vec<FilaCertificado>> {
        sqlx::query!(
            "SELECT r.impuesto, c.nombre AS concepto, r.tarifa,
                    sum(r.base) AS \"base!\", sum(r.valor) AS \"valor!\"
             FROM retenciones_factura_proveedor r
             JOIN facturas_proveedor f ON f.tenant_id = r.tenant_id AND f.id = r.factura_id
             JOIN conceptos_retencion c ON c.id = r.concepto_retencion_id
             WHERE f.persona_id = $1 AND f.fecha BETWEEN $2 AND $3
               AND f.estado NOT IN ('registrada', 'anulada')
             GROUP BY r.impuesto, c.nombre, r.tarifa
             ORDER BY r.impuesto, c.nombre, r.tarifa",
            persona.as_uuid(),
            desde,
            hasta,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| {
            Ok(FilaCertificado {
                impuesto: ImpuestoRetencion::try_from(r.impuesto.as_str())?,
                concepto: r.concepto,
                tarifa: r.tarifa,
                base: r.base,
                valor: r.valor,
            })
        })
        .collect()
    }

    async fn insertar_factura_venta(&mut self, f: &FacturaVenta) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO facturas_venta (id, tenant_id, persona_id, fecha, vencimiento, descripcion,
                 origen_tipo, origen_id, subtotal, iva, total, pagado, estado, creada_por, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
            f.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            f.persona_id.as_uuid(),
            f.fecha,
            f.vencimiento,
            f.descripcion,
            f.origen.as_ref().map(|o| o.tipo.as_str()),
            f.origen.as_ref().map(|o| o.id),
            f.subtotal.valor(),
            f.iva.valor(),
            f.total.valor(),
            f.pagado.valor(),
            f.estado.as_str(),
            f.creada_por.map(CuentaId::as_uuid),
            f.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "el documento origen ya tiene factura", "tercero"))?;
        for (i, l) in f.lineas.iter().enumerate() {
            sqlx::query!(
                "INSERT INTO lineas_factura_venta (tenant_id, factura_id, orden, descripcion,
                     cuenta_id, centro_costo_id, cantidad, valor_unitario, tarifa_iva, subtotal, iva,
                     clasificacion)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                self.ctx.tenant_id.as_uuid(),
                f.id.as_uuid(),
                i16_de(i + 1, "orden")?,
                l.descripcion,
                l.cuenta_id.as_uuid(),
                l.centro_costo_id.map(CentroCostoId::as_uuid),
                l.cantidad,
                l.valor_unitario.valor(),
                l.tarifa_iva,
                l.subtotal.valor(),
                l.iva.valor(),
                l.clasificacion.as_str(),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_contable(e, "línea repetida", "cuenta o centro de costo"))?;
        }
        Ok(())
    }

    async fn factura_venta(&mut self, id: FacturaVentaId) -> AppResult<Option<FacturaVenta>> {
        let fila = sqlx::query_as!(
            FilaFacturaVenta,
            "SELECT id, persona_id, numero, fecha, vencimiento, descripcion, origen_tipo, origen_id,
                    subtotal, iva, total, pagado, estado, comprobante_id, motivo_anulacion,
                    creada_por, creada_en, emitida_en
             FROM facturas_venta WHERE id = $1 FOR UPDATE",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        match fila {
            None => Ok(None),
            Some(f) => {
                let lineas = self.lineas_venta(f.id).await?;
                Ok(Some(f.a_factura(lineas)?))
            }
        }
    }

    async fn facturas_venta(
        &mut self,
        filtro: &FiltroFacturas,
        limite: u32,
    ) -> AppResult<Vec<FacturaVenta>> {
        sqlx::query_as!(
            FilaFacturaVenta,
            "SELECT id, persona_id, numero, fecha, vencimiento, descripcion, origen_tipo, origen_id,
                    subtotal, iva, total, pagado, estado, comprobante_id, motivo_anulacion,
                    creada_por, creada_en, emitida_en
             FROM facturas_venta
             WHERE ($1::uuid IS NULL OR persona_id = $1) AND ($2::text IS NULL OR estado = $2)
               AND ($3::date IS NULL OR fecha >= $3) AND ($4::date IS NULL OR fecha <= $4)
             ORDER BY vencimiento, id LIMIT $5",
            filtro.persona_id.map(PersonaId::as_uuid),
            filtro.estado,
            filtro.desde,
            filtro.hasta,
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| f.a_factura(Vec::new()))
        .collect()
    }

    async fn actualizar_factura_venta(&mut self, f: &FacturaVenta) -> AppResult<()> {
        sqlx::query!(
            "UPDATE facturas_venta SET estado = $2, numero = $3, pagado = $4, comprobante_id = $5,
                 motivo_anulacion = $6, emitida_en = $7
             WHERE id = $1",
            f.id.as_uuid(),
            f.estado.as_str(),
            f.numero
                .map(i64::from)
                .map(|n| i32::try_from(n).unwrap_or(i32::MAX)),
            f.pagado.valor(),
            f.comprobante_id.map(ComprobanteId::as_uuid),
            f.motivo_anulacion,
            f.emitida_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "factura", "comprobante"))?;
        Ok(())
    }

    async fn eliminar_factura_venta(&mut self, id: FacturaVentaId) -> AppResult<()> {
        sqlx::query!(
            "DELETE FROM facturas_venta WHERE id = $1 AND estado = 'borrador'",
            id.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn facturas_venta_con_saldo(&mut self, corte: NaiveDate) -> AppResult<Vec<FacturaVenta>> {
        sqlx::query_as!(
            FilaFacturaVenta,
            "SELECT id, persona_id, numero, fecha, vencimiento, descripcion, origen_tipo, origen_id,
                    subtotal, iva, total, pagado, estado, comprobante_id, motivo_anulacion,
                    creada_por, creada_en, emitida_en
             FROM facturas_venta
             WHERE estado IN ('emitida', 'pagada_parcial') AND fecha <= $1
             ORDER BY persona_id, vencimiento",
            corte
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| f.a_factura(Vec::new()))
        .collect()
    }
}
