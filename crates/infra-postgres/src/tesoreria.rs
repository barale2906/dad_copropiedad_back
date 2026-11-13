//! Tesorería sobre PostgreSQL (plan 13, ADR 0018): cuentas de tesorería, recibos con sus
//! aplicaciones, egresos, eventos de la pasarela y el catálogo de retenciones (lectura en
//! cualquier transacción; escritura en la de plataforma).

use application::{
    AppError, AppResult,
    puertos::tesoreria::{
        AplicacionPago, CatalogoRetenciones, CatalogoRetencionesPlataforma, ConceptoRetencion,
        CuentaTesoreria, Egreso, EventoPasarela, FiltroRecibos, GastoEgreso, ReciboCaja,
        RepositorioTesoreria, ValorUvt,
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AplicacionPagoId, ArchivoId, CentroCostoId, ComprobanteId, ConceptoRetencionId,
    CuentaContableId, CuentaId, CuentaTesoreriaId, EgresoId, EventoPasarelaId, FacturaProveedorId,
    FacturaVentaId, ObligacionCobroId, PersonaId, ReciboCajaId,
    contabilidad::{
        CodigoCuenta, Monto,
        tesoreria::{
            EstadoEgreso, EstadoEventoPasarela, EstadoRecibo, ImpuestoRetencion, MedioEgreso,
            MedioRecibo, TipoCuentaBancaria, TipoCuentaTesoreria,
        },
    },
};
use rust_decimal::Decimal;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{
    TransaccionPg,
    contabilidad::{error_contable, i16_de},
    plataforma::TransaccionPlataformaPg,
};

fn monto(v: Decimal) -> AppResult<Monto> {
    Ok(Monto::new(v)?)
}

// ---------------------------------------------------------------------------------------------
// Cuentas de tesorería
// ---------------------------------------------------------------------------------------------

struct FilaCuentaTesoreria {
    id: Uuid,
    tipo: String,
    nombre: String,
    banco: Option<String>,
    numero_enmascarado: Option<String>,
    tipo_cuenta: Option<String>,
    cuenta_contable_id: Uuid,
    destinacion: Option<String>,
    activa: bool,
    creada_en: DateTime<Utc>,
}

impl TryFrom<FilaCuentaTesoreria> for CuentaTesoreria {
    type Error = AppError;

    fn try_from(f: FilaCuentaTesoreria) -> AppResult<Self> {
        Ok(Self {
            id: CuentaTesoreriaId::from_uuid(f.id),
            tipo: TipoCuentaTesoreria::try_from(f.tipo.as_str())?,
            nombre: f.nombre,
            banco: f.banco,
            numero_enmascarado: f.numero_enmascarado,
            tipo_cuenta: f
                .tipo_cuenta
                .as_deref()
                .map(TipoCuentaBancaria::try_from)
                .transpose()?,
            cuenta_contable_id: CuentaContableId::from_uuid(f.cuenta_contable_id),
            destinacion: f.destinacion,
            activa: f.activa,
            creada_en: f.creada_en,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Recibos y aplicaciones
// ---------------------------------------------------------------------------------------------

struct FilaRecibo {
    id: Uuid,
    cuenta_tesoreria_id: Uuid,
    persona_id: Option<Uuid>,
    fecha: NaiveDate,
    valor: Decimal,
    medio: String,
    referencia: Option<String>,
    descripcion: String,
    soporte_archivo_id: Option<Uuid>,
    estado: String,
    comprobante_id: Uuid,
    motivo_anulacion: Option<String>,
    registrado_por: Option<Uuid>,
    registrado_en: DateTime<Utc>,
}

struct FilaAplicacion {
    id: Uuid,
    recibo_id: Uuid,
    factura_venta_id: Option<Uuid>,
    obligacion_id: Option<Uuid>,
    valor: Decimal,
    aplicada_en: DateTime<Utc>,
    comprobante_id: Option<Uuid>,
    anulada_en: Option<DateTime<Utc>>,
}

impl TryFrom<FilaAplicacion> for AplicacionPago {
    type Error = AppError;

    fn try_from(f: FilaAplicacion) -> AppResult<Self> {
        Ok(Self {
            id: AplicacionPagoId::from_uuid(f.id),
            recibo_id: ReciboCajaId::from_uuid(f.recibo_id),
            factura_venta_id: f.factura_venta_id.map(FacturaVentaId::from_uuid),
            obligacion_id: f.obligacion_id.map(ObligacionCobroId::from_uuid),
            valor: monto(f.valor)?,
            aplicada_en: f.aplicada_en,
            comprobante_id: f.comprobante_id.map(ComprobanteId::from_uuid),
            anulada_en: f.anulada_en,
        })
    }
}

impl FilaRecibo {
    fn a_recibo(self, aplicaciones: Vec<AplicacionPago>) -> AppResult<ReciboCaja> {
        Ok(ReciboCaja {
            id: ReciboCajaId::from_uuid(self.id),
            cuenta_tesoreria_id: CuentaTesoreriaId::from_uuid(self.cuenta_tesoreria_id),
            persona_id: self.persona_id.map(PersonaId::from_uuid),
            fecha: self.fecha,
            valor: monto(self.valor)?,
            medio: MedioRecibo::try_from(self.medio.as_str())?,
            referencia: self.referencia,
            descripcion: self.descripcion,
            soporte_archivo_id: self.soporte_archivo_id.map(ArchivoId::from_uuid),
            estado: EstadoRecibo::try_from(self.estado.as_str())?,
            comprobante_id: ComprobanteId::from_uuid(self.comprobante_id),
            motivo_anulacion: self.motivo_anulacion,
            registrado_por: self.registrado_por.map(CuentaId::from_uuid),
            registrado_en: self.registrado_en,
            aplicaciones,
        })
    }
}

impl TransaccionPg {
    async fn con_aplicaciones(&mut self, filas: Vec<FilaRecibo>) -> AppResult<Vec<ReciboCaja>> {
        let ids: Vec<Uuid> = filas.iter().map(|f| f.id).collect();
        let aplicaciones = sqlx::query_as!(
            FilaAplicacion,
            "SELECT id, recibo_id, factura_venta_id, obligacion_id, valor, aplicada_en,
                    comprobante_id, anulada_en
             FROM aplicaciones_pago WHERE recibo_id = ANY($1) ORDER BY aplicada_en, id",
            &ids
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let mut todas: Vec<AplicacionPago> = aplicaciones
            .into_iter()
            .map(TryInto::try_into)
            .collect::<AppResult<_>>()?;
        filas
            .into_iter()
            .map(|f| {
                let id = ReciboCajaId::from_uuid(f.id);
                let (propias, resto): (Vec<_>, Vec<_>) =
                    todas.drain(..).partition(|a| a.recibo_id == id);
                todas = resto;
                f.a_recibo(propias)
            })
            .collect()
    }

    async fn insertar_aplicacion_en(&mut self, a: &AplicacionPago) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO aplicaciones_pago (id, tenant_id, recibo_id, factura_venta_id, valor,
                 aplicada_en, comprobante_id, anulada_en, obligacion_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            a.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            a.recibo_id.as_uuid(),
            a.factura_venta_id.map(FacturaVentaId::as_uuid),
            a.valor.valor(),
            a.aplicada_en,
            a.comprobante_id.map(ComprobanteId::as_uuid),
            a.anulada_en,
            a.obligacion_id.map(ObligacionCobroId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "aplicación repetida", "factura de venta u obligación"))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Egresos
// ---------------------------------------------------------------------------------------------

struct FilaEgreso {
    id: Uuid,
    cuenta_tesoreria_id: Uuid,
    persona_id: Uuid,
    fecha: NaiveDate,
    valor: Decimal,
    medio: String,
    referencia: Option<String>,
    descripcion: String,
    soporte_archivo_id: Option<Uuid>,
    estado: String,
    registrado_por: Uuid,
    registrado_en: DateTime<Utc>,
    autorizado_por: Option<Uuid>,
    autorizado_en: Option<DateTime<Utc>>,
    pagado_en: Option<DateTime<Utc>>,
    comprobante_id: Option<Uuid>,
    motivo_anulacion: Option<String>,
}

impl TransaccionPg {
    async fn con_detalle_egreso(&mut self, f: FilaEgreso) -> AppResult<Egreso> {
        let facturas = sqlx::query!(
            "SELECT factura_proveedor_id, valor FROM egreso_facturas
             WHERE egreso_id = $1 ORDER BY factura_proveedor_id",
            f.id
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| {
            Ok((
                FacturaProveedorId::from_uuid(r.factura_proveedor_id),
                monto(r.valor)?,
            ))
        })
        .collect::<AppResult<Vec<_>>>()?;
        let gastos = sqlx::query!(
            "SELECT cuenta_id, persona_id, centro_costo_id, valor, descripcion
             FROM egreso_gastos WHERE egreso_id = $1 ORDER BY orden",
            f.id
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| {
            Ok(GastoEgreso {
                cuenta_id: CuentaContableId::from_uuid(r.cuenta_id),
                persona_id: r.persona_id.map(PersonaId::from_uuid),
                centro_costo_id: r.centro_costo_id.map(CentroCostoId::from_uuid),
                valor: monto(r.valor)?,
                descripcion: r.descripcion,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
        Ok(Egreso {
            id: EgresoId::from_uuid(f.id),
            cuenta_tesoreria_id: CuentaTesoreriaId::from_uuid(f.cuenta_tesoreria_id),
            persona_id: PersonaId::from_uuid(f.persona_id),
            fecha: f.fecha,
            valor: monto(f.valor)?,
            medio: MedioEgreso::try_from(f.medio.as_str())?,
            referencia: f.referencia,
            descripcion: f.descripcion,
            soporte_archivo_id: f.soporte_archivo_id.map(ArchivoId::from_uuid),
            estado: EstadoEgreso::try_from(f.estado.as_str())?,
            registrado_por: CuentaId::from_uuid(f.registrado_por),
            registrado_en: f.registrado_en,
            autorizado_por: f.autorizado_por.map(CuentaId::from_uuid),
            autorizado_en: f.autorizado_en,
            pagado_en: f.pagado_en,
            comprobante_id: f.comprobante_id.map(ComprobanteId::from_uuid),
            motivo_anulacion: f.motivo_anulacion,
            facturas,
            gastos,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Eventos de la pasarela
// ---------------------------------------------------------------------------------------------

struct FilaEvento {
    id: Uuid,
    proveedor: String,
    evento_externo: String,
    referencia: String,
    estado: String,
    valor: Decimal,
    comision: Decimal,
    retenciones: Decimal,
    cuenta_tesoreria_id: Uuid,
    persona_id: Option<Uuid>,
    recibo_id: Option<Uuid>,
    ocurrido_en: DateTime<Utc>,
    recibido_en: DateTime<Utc>,
}

impl TryFrom<FilaEvento> for EventoPasarela {
    type Error = AppError;

    fn try_from(f: FilaEvento) -> AppResult<Self> {
        Ok(Self {
            id: EventoPasarelaId::from_uuid(f.id),
            proveedor: f.proveedor,
            evento_externo: f.evento_externo,
            referencia: f.referencia,
            estado: EstadoEventoPasarela::try_from(f.estado.as_str())?,
            valor: monto(f.valor)?,
            comision: monto(f.comision)?,
            retenciones: monto(f.retenciones)?,
            cuenta_tesoreria_id: CuentaTesoreriaId::from_uuid(f.cuenta_tesoreria_id),
            persona_id: f.persona_id.map(PersonaId::from_uuid),
            recibo_id: f.recibo_id.map(ReciboCajaId::from_uuid),
            ocurrido_en: f.ocurrido_en,
            recibido_en: f.recibido_en,
        })
    }
}

impl RepositorioTesoreria for TransaccionPg {
    async fn cuentas_tesoreria(&mut self) -> AppResult<Vec<CuentaTesoreria>> {
        sqlx::query_as!(
            FilaCuentaTesoreria,
            "SELECT id, tipo, nombre, banco, numero_enmascarado, tipo_cuenta, cuenta_contable_id,
                    destinacion, activa, creada_en
             FROM cuentas_tesoreria ORDER BY nombre"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn cuenta_tesoreria(
        &mut self,
        id: CuentaTesoreriaId,
    ) -> AppResult<Option<CuentaTesoreria>> {
        sqlx::query_as!(
            FilaCuentaTesoreria,
            "SELECT id, tipo, nombre, banco, numero_enmascarado, tipo_cuenta, cuenta_contable_id,
                    destinacion, activa, creada_en
             FROM cuentas_tesoreria WHERE id = $1",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn insertar_cuenta_tesoreria(&mut self, c: &CuentaTesoreria) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO cuentas_tesoreria (id, tenant_id, tipo, nombre, banco, numero_enmascarado,
                 tipo_cuenta, cuenta_contable_id, destinacion, activa, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.tipo.as_str(),
            c.nombre,
            c.banco,
            c.numero_enmascarado,
            c.tipo_cuenta.map(TipoCuentaBancaria::as_str),
            c.cuenta_contable_id.as_uuid(),
            c.destinacion,
            c.activa,
            c.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "ya hay una cuenta de tesorería con ese nombre o esa cuenta contable",
                "cuenta contable",
            )
        })?;
        Ok(())
    }

    async fn actualizar_cuenta_tesoreria(&mut self, c: &CuentaTesoreria) -> AppResult<()> {
        sqlx::query!(
            "UPDATE cuentas_tesoreria SET nombre = $2, destinacion = $3, activa = $4 WHERE id = $1",
            c.id.as_uuid(),
            c.nombre,
            c.destinacion,
            c.activa,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(e, "ya hay una cuenta de tesorería con ese nombre", "cuenta")
        })?;
        Ok(())
    }

    async fn insertar_recibo(&mut self, r: &ReciboCaja) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO recibos_caja (id, tenant_id, cuenta_tesoreria_id, persona_id, fecha, valor,
                 medio, referencia, descripcion, soporte_archivo_id, estado, comprobante_id,
                 registrado_por, registrado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            r.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            r.cuenta_tesoreria_id.as_uuid(),
            r.persona_id.map(PersonaId::as_uuid),
            r.fecha,
            r.valor.valor(),
            r.medio.as_str(),
            r.referencia,
            r.descripcion,
            r.soporte_archivo_id.map(ArchivoId::as_uuid),
            r.estado.as_str(),
            r.comprobante_id.as_uuid(),
            r.registrado_por.map(CuentaId::as_uuid),
            r.registrado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "recibo repetido", "cuenta, tercero o soporte"))?;
        for a in &r.aplicaciones {
            self.insertar_aplicacion_en(a).await?;
        }
        Ok(())
    }

    async fn recibo(&mut self, id: ReciboCajaId) -> AppResult<Option<ReciboCaja>> {
        let filas = sqlx::query_as!(
            FilaRecibo,
            "SELECT id, cuenta_tesoreria_id, persona_id, fecha, valor, medio, referencia,
                    descripcion, soporte_archivo_id, estado, comprobante_id, motivo_anulacion,
                    registrado_por, registrado_en
             FROM recibos_caja WHERE id = $1 FOR UPDATE",
            id.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(self.con_aplicaciones(filas).await?.pop())
    }

    async fn recibos(&mut self, filtro: &FiltroRecibos, limite: u32) -> AppResult<Vec<ReciboCaja>> {
        let filas = sqlx::query_as!(
            FilaRecibo,
            "SELECT id, cuenta_tesoreria_id, persona_id, fecha, valor, medio, referencia,
                    descripcion, soporte_archivo_id, estado, comprobante_id, motivo_anulacion,
                    registrado_por, registrado_en
             FROM recibos_caja
             WHERE ($1::date IS NULL OR fecha >= $1) AND ($2::date IS NULL OR fecha <= $2)
               AND ($3::uuid IS NULL OR persona_id = $3)
               AND ($4::uuid IS NULL OR cuenta_tesoreria_id = $4)
               AND (NOT $5 OR persona_id IS NULL)
             ORDER BY fecha DESC, id DESC LIMIT $6",
            filtro.desde,
            filtro.hasta,
            filtro.persona_id.map(PersonaId::as_uuid),
            filtro.cuenta_tesoreria_id.map(CuentaTesoreriaId::as_uuid),
            filtro.sin_identificar,
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        self.con_aplicaciones(filas).await
    }

    async fn anular_recibo(&mut self, id: ReciboCajaId, motivo: &str) -> AppResult<()> {
        sqlx::query!(
            "UPDATE recibos_caja SET estado = 'anulado', motivo_anulacion = $2 WHERE id = $1",
            id.as_uuid(),
            motivo
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn identificar_recibo(&mut self, id: ReciboCajaId, persona: PersonaId) -> AppResult<()> {
        sqlx::query!(
            "UPDATE recibos_caja SET persona_id = $2 WHERE id = $1 AND persona_id IS NULL",
            id.as_uuid(),
            persona.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "recibo", "tercero"))?;
        Ok(())
    }

    async fn insertar_aplicacion(&mut self, a: &AplicacionPago) -> AppResult<()> {
        self.insertar_aplicacion_en(a).await
    }

    async fn anular_aplicaciones(
        &mut self,
        recibo: ReciboCajaId,
        en: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE aplicaciones_pago SET anulada_en = $2 WHERE recibo_id = $1 AND anulada_en IS NULL",
            recibo.as_uuid(),
            en
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn insertar_egreso(&mut self, e: &Egreso) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO egresos (id, tenant_id, cuenta_tesoreria_id, persona_id, fecha, valor, medio,
                 referencia, descripcion, soporte_archivo_id, estado, registrado_por, registrado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            e.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            e.cuenta_tesoreria_id.as_uuid(),
            e.persona_id.as_uuid(),
            e.fecha,
            e.valor.valor(),
            e.medio.as_str(),
            e.referencia,
            e.descripcion,
            e.soporte_archivo_id.map(ArchivoId::as_uuid),
            e.estado.as_str(),
            e.registrado_por.as_uuid(),
            e.registrado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|x| error_contable(x, "egreso repetido", "cuenta, tercero o soporte"))?;
        for (factura, valor) in &e.facturas {
            sqlx::query!(
                "INSERT INTO egreso_facturas (tenant_id, egreso_id, factura_proveedor_id, valor)
                 VALUES ($1, $2, $3, $4)",
                self.ctx.tenant_id.as_uuid(),
                e.id.as_uuid(),
                factura.as_uuid(),
                valor.valor(),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|x| {
                error_contable(x, "factura repetida en el egreso", "factura de proveedor")
            })?;
        }
        for (i, g) in e.gastos.iter().enumerate() {
            sqlx::query!(
                "INSERT INTO egreso_gastos (tenant_id, egreso_id, orden, cuenta_id, persona_id,
                     centro_costo_id, valor, descripcion)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                self.ctx.tenant_id.as_uuid(),
                e.id.as_uuid(),
                i16_de(i + 1, "orden")?,
                g.cuenta_id.as_uuid(),
                g.persona_id.map(PersonaId::as_uuid),
                g.centro_costo_id.map(CentroCostoId::as_uuid),
                g.valor.valor(),
                g.descripcion,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|x| {
                error_contable(x, "gasto repetido", "cuenta, tercero o centro de costo")
            })?;
        }
        Ok(())
    }

    async fn egreso(&mut self, id: EgresoId) -> AppResult<Option<Egreso>> {
        let fila = sqlx::query_as!(
            FilaEgreso,
            "SELECT id, cuenta_tesoreria_id, persona_id, fecha, valor, medio, referencia,
                    descripcion, soporte_archivo_id, estado, registrado_por, registrado_en,
                    autorizado_por, autorizado_en, pagado_en, comprobante_id, motivo_anulacion
             FROM egresos WHERE id = $1 FOR UPDATE",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        match fila {
            Some(f) => Ok(Some(self.con_detalle_egreso(f).await?)),
            None => Ok(None),
        }
    }

    async fn egresos(
        &mut self,
        estado: Option<EstadoEgreso>,
        limite: u32,
    ) -> AppResult<Vec<Egreso>> {
        let filas = sqlx::query_as!(
            FilaEgreso,
            "SELECT id, cuenta_tesoreria_id, persona_id, fecha, valor, medio, referencia,
                    descripcion, soporte_archivo_id, estado, registrado_por, registrado_en,
                    autorizado_por, autorizado_en, pagado_en, comprobante_id, motivo_anulacion
             FROM egresos WHERE ($1::text IS NULL OR estado = $1)
             ORDER BY fecha DESC, id DESC LIMIT $2",
            estado.map(EstadoEgreso::as_str),
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let mut v = Vec::with_capacity(filas.len());
        for f in filas {
            v.push(self.con_detalle_egreso(f).await?);
        }
        Ok(v)
    }

    async fn actualizar_egreso(&mut self, e: &Egreso) -> AppResult<()> {
        sqlx::query!(
            "UPDATE egresos SET estado = $2, fecha = $3, autorizado_por = $4, autorizado_en = $5,
                 pagado_en = $6, comprobante_id = $7, motivo_anulacion = $8
             WHERE id = $1",
            e.id.as_uuid(),
            e.estado.as_str(),
            e.fecha,
            e.autorizado_por.map(CuentaId::as_uuid),
            e.autorizado_en,
            e.pagado_en,
            e.comprobante_id.map(ComprobanteId::as_uuid),
            e.motivo_anulacion,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|x| error_contable(x, "egreso", "comprobante"))?;
        Ok(())
    }

    async fn evento_pasarela(
        &mut self,
        proveedor: &str,
        evento_externo: &str,
    ) -> AppResult<Option<EventoPasarela>> {
        sqlx::query_as!(
            FilaEvento,
            "SELECT id, proveedor, evento_externo, referencia, estado, valor, comision, retenciones,
                    cuenta_tesoreria_id, persona_id, recibo_id, ocurrido_en, recibido_en
             FROM eventos_pasarela WHERE proveedor = $1 AND evento_externo = $2",
            proveedor,
            evento_externo
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn recibo_de_referencia(
        &mut self,
        proveedor: &str,
        referencia: &str,
    ) -> AppResult<Option<ReciboCajaId>> {
        Ok(sqlx::query_scalar!(
            "SELECT recibo_id AS \"recibo_id!\" FROM eventos_pasarela
             WHERE proveedor = $1 AND referencia = $2 AND recibo_id IS NOT NULL
             ORDER BY ocurrido_en LIMIT 1",
            proveedor,
            referencia
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(ReciboCajaId::from_uuid))
    }

    async fn insertar_evento_pasarela(&mut self, e: &EventoPasarela) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO eventos_pasarela (id, tenant_id, proveedor, evento_externo, referencia,
                 estado, valor, comision, retenciones, cuenta_tesoreria_id, persona_id, recibo_id,
                 ocurrido_en, recibido_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            e.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            e.proveedor,
            e.evento_externo,
            e.referencia,
            e.estado.as_str(),
            e.valor.valor(),
            e.comision.valor(),
            e.retenciones.valor(),
            e.cuenta_tesoreria_id.as_uuid(),
            e.persona_id.map(PersonaId::as_uuid),
            e.recibo_id.map(ReciboCajaId::as_uuid),
            e.ocurrido_en,
            e.recibido_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|x| {
            error_contable(
                x,
                "el evento de la pasarela ya se registró",
                "cuenta o tercero",
            )
        })?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Catálogo de retenciones
// ---------------------------------------------------------------------------------------------

struct FilaConcepto {
    id: Uuid,
    codigo: String,
    impuesto: String,
    nombre: String,
    base_minima_uvt: Decimal,
    tarifa: Decimal,
    cuenta_sugerida: Option<String>,
    municipio_dane: Option<String>,
    fuente: String,
    vigente_desde: NaiveDate,
    vigente_hasta: Option<NaiveDate>,
}

impl TryFrom<FilaConcepto> for ConceptoRetencion {
    type Error = AppError;

    fn try_from(f: FilaConcepto) -> AppResult<Self> {
        Ok(Self {
            id: ConceptoRetencionId::from_uuid(f.id),
            codigo: f.codigo,
            impuesto: ImpuestoRetencion::try_from(f.impuesto.as_str())?,
            nombre: f.nombre,
            base_minima_uvt: f.base_minima_uvt,
            tarifa: f.tarifa,
            cuenta_sugerida: f
                .cuenta_sugerida
                .as_deref()
                .map(CodigoCuenta::new)
                .transpose()?,
            municipio_dane: f.municipio_dane,
            fuente: f.fuente,
            vigente_desde: f.vigente_desde,
            vigente_hasta: f.vigente_hasta,
        })
    }
}

async fn concepto_vigente(
    c: &mut PgConnection,
    codigo: &str,
    fecha: NaiveDate,
) -> AppResult<Option<ConceptoRetencion>> {
    sqlx::query_as!(
        FilaConcepto,
        "SELECT id, codigo, impuesto, nombre, base_minima_uvt, tarifa, cuenta_sugerida,
                municipio_dane, fuente, vigente_desde, vigente_hasta
         FROM conceptos_retencion
         WHERE codigo = $1 AND vigente_desde <= $2 AND (vigente_hasta IS NULL OR vigente_hasta >= $2)
         ORDER BY vigente_desde DESC LIMIT 1",
        codigo,
        fecha
    )
    .fetch_optional(c)
    .await
    .map_err(AppError::infra)?
    .map(TryInto::try_into)
    .transpose()
}

async fn concepto_por_id(
    c: &mut PgConnection,
    id: ConceptoRetencionId,
) -> AppResult<Option<ConceptoRetencion>> {
    sqlx::query_as!(
        FilaConcepto,
        "SELECT id, codigo, impuesto, nombre, base_minima_uvt, tarifa, cuenta_sugerida,
                municipio_dane, fuente, vigente_desde, vigente_hasta
         FROM conceptos_retencion WHERE id = $1",
        id.as_uuid()
    )
    .fetch_optional(c)
    .await
    .map_err(AppError::infra)?
    .map(TryInto::try_into)
    .transpose()
}

async fn conceptos(
    c: &mut PgConnection,
    vigentes_en: Option<NaiveDate>,
) -> AppResult<Vec<ConceptoRetencion>> {
    sqlx::query_as!(
        FilaConcepto,
        "SELECT id, codigo, impuesto, nombre, base_minima_uvt, tarifa, cuenta_sugerida,
                municipio_dane, fuente, vigente_desde, vigente_hasta
         FROM conceptos_retencion
         WHERE $1::date IS NULL
            OR (vigente_desde <= $1 AND (vigente_hasta IS NULL OR vigente_hasta >= $1))
         ORDER BY impuesto, codigo, vigente_desde DESC",
        vigentes_en
    )
    .fetch_all(c)
    .await
    .map_err(AppError::infra)?
    .into_iter()
    .map(TryInto::try_into)
    .collect()
}

async fn uvt_de(c: &mut PgConnection, anio: i32) -> AppResult<Option<Decimal>> {
    sqlx::query_scalar!(
        "SELECT valor FROM uvt WHERE anio = $1",
        i16_de(anio, "anio")?
    )
    .fetch_optional(c)
    .await
    .map_err(AppError::infra)
}

async fn valores_uvt_de(c: &mut PgConnection) -> AppResult<Vec<ValorUvt>> {
    Ok(
        sqlx::query!("SELECT anio, valor, fuente FROM uvt ORDER BY anio DESC")
            .fetch_all(c)
            .await
            .map_err(AppError::infra)?
            .into_iter()
            .map(|r| ValorUvt {
                anio: i32::from(r.anio),
                valor: r.valor,
                fuente: r.fuente,
            })
            .collect(),
    )
}

/// Implementa la lectura del catálogo sobre la conexión de la transacción.
macro_rules! catalogo_retenciones {
    ($tipo:ty) => {
        impl CatalogoRetenciones for $tipo {
            async fn concepto_retencion_vigente(
                &mut self,
                codigo: &str,
                fecha: NaiveDate,
            ) -> AppResult<Option<ConceptoRetencion>> {
                concepto_vigente(&mut self.tx, codigo, fecha).await
            }

            async fn conceptos_retencion(
                &mut self,
                vigentes_en: Option<NaiveDate>,
            ) -> AppResult<Vec<ConceptoRetencion>> {
                conceptos(&mut self.tx, vigentes_en).await
            }

            async fn concepto_retencion(
                &mut self,
                id: ConceptoRetencionId,
            ) -> AppResult<Option<ConceptoRetencion>> {
                concepto_por_id(&mut self.tx, id).await
            }

            async fn uvt(&mut self, anio: i32) -> AppResult<Option<Decimal>> {
                uvt_de(&mut self.tx, anio).await
            }

            async fn valores_uvt(&mut self) -> AppResult<Vec<ValorUvt>> {
                valores_uvt_de(&mut self.tx).await
            }
        }
    };
}

catalogo_retenciones!(TransaccionPg);
catalogo_retenciones!(TransaccionPlataformaPg);

impl CatalogoRetencionesPlataforma for TransaccionPlataformaPg {
    async fn insertar_concepto_retencion(&mut self, c: &ConceptoRetencion) -> AppResult<()> {
        sqlx::query!(
            "UPDATE conceptos_retencion SET vigente_hasta = $2::date - 1
             WHERE codigo = $1 AND vigente_desde < $2
               AND (vigente_hasta IS NULL OR vigente_hasta >= $2)",
            c.codigo,
            c.vigente_desde,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query!(
            "INSERT INTO conceptos_retencion (id, codigo, impuesto, nombre, base_minima_uvt, tarifa,
                 cuenta_sugerida, municipio_dane, fuente, vigente_desde, vigente_hasta)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            c.id.as_uuid(),
            c.codigo,
            c.impuesto.as_str(),
            c.nombre,
            c.base_minima_uvt,
            c.tarifa,
            c.cuenta_sugerida.as_ref().map(CodigoCuenta::as_str),
            c.municipio_dane,
            c.fuente,
            c.vigente_desde,
            c.vigente_hasta,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "ya hay una versión del concepto con esa fecha",
                "concepto",
            )
        })?;
        Ok(())
    }

    async fn guardar_uvt(&mut self, uvt: &ValorUvt) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO uvt (anio, valor, fuente) VALUES ($1, $2, $3)
             ON CONFLICT (anio) DO UPDATE SET valor = EXCLUDED.valor, fuente = EXCLUDED.fuente",
            i16_de(uvt.anio, "anio")?,
            uvt.valor,
            uvt.fuente,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}
