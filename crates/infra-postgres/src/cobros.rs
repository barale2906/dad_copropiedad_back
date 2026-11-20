//! Cobros de la plataforma (plan 19) sobre la transacción de plataforma: catálogo, tarifas,
//! suscripciones, mediciones y cortes. La medición entre entidades usa `medir_consumo`
//! (`SECURITY DEFINER`, solo números).

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::{
        RepositorioCobros,
        cobros::{
            ClienteFacturacion, Corte, FiltroCortes, LineaSuscripcion, LiquidacionACobrar,
            OrigenCorte, Pagador, Producto, Suscripcion, TarifaProducto,
        },
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    CorteFacturacionId, CuentaId, DireccionCorreo, FacturaVentaId, ItemFacturacionId,
    LineaSuscripcionId, LiquidacionId, Modulo, OrganizacionId, ProductoPlataformaId, SuscripcionId,
    TenantId,
    cobro_plataforma::{
        Ciclo, EstadoCorte, EstadoSuscripcion, LineaCorte, Metrica, ModeloTarifa, Periodo, Tarifa,
    },
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{TransaccionPlataformaPg, plataforma::conflicto_si_unico};

fn u16_de(v: i16) -> u16 {
    u16::try_from(v).unwrap_or(0)
}

/// NIT sin dígito de verificación (lo que se guarda como documento del tercero).
fn nit_base(nit: Option<String>) -> Option<String> {
    nit.and_then(|n| n.split('-').next().map(str::to_owned))
}

fn correo(c: Option<String>) -> AppResult<Option<DireccionCorreo>> {
    c.as_deref()
        .map(DireccionCorreo::new)
        .transpose()
        .map_err(AppError::from)
}

struct FilaProducto {
    id: Uuid,
    codigo: String,
    nombre: String,
    descripcion_factura: String,
    modulo: Option<String>,
    item_facturacion_id: Option<Uuid>,
    activo: bool,
    creado_en: DateTime<Utc>,
}

impl TryFrom<FilaProducto> for Producto {
    type Error = AppError;

    fn try_from(f: FilaProducto) -> AppResult<Self> {
        Ok(Self {
            id: ProductoPlataformaId::from_uuid(f.id),
            codigo: f.codigo,
            nombre: f.nombre,
            descripcion_factura: f.descripcion_factura,
            modulo: f.modulo.as_deref().map(Modulo::try_from).transpose()?,
            item_facturacion_id: f.item_facturacion_id.map(ItemFacturacionId::from_uuid),
            activo: f.activo,
            creado_en: f.creado_en,
        })
    }
}

struct FilaSuscripcion {
    id: Uuid,
    tenant_id: Uuid,
    pagador_entidad_id: Option<Uuid>,
    pagador_organizacion_id: Option<Uuid>,
    ciclo_meses: Option<i16>,
    inicio: NaiveDate,
    dias_prueba: i16,
    plazo_pago_dias: i16,
    correo_facturacion: String,
    estado: String,
    suspendida_por_mora: bool,
    terminada_en: Option<NaiveDate>,
    creada_por: Option<Uuid>,
    creada_en: DateTime<Utc>,
}

impl TryFrom<FilaSuscripcion> for Suscripcion {
    type Error = AppError;

    fn try_from(f: FilaSuscripcion) -> AppResult<Self> {
        let pagador = match (f.pagador_entidad_id, f.pagador_organizacion_id) {
            (Some(e), _) => Pagador::Entidad(TenantId::from_uuid(e)),
            (None, Some(o)) => Pagador::Organizacion(OrganizacionId::from_uuid(o)),
            (None, None) => Pagador::MismaEntidad,
        };
        Ok(Self {
            id: SuscripcionId::from_uuid(f.id),
            tenant_id: TenantId::from_uuid(f.tenant_id),
            pagador,
            ciclo: f.ciclo_meses.map_or(Ciclo::PorEvento, |m| {
                Ciclo::Meses(u8::try_from(m).unwrap_or(1))
            }),
            inicio: f.inicio,
            dias_prueba: u16_de(f.dias_prueba),
            plazo_pago_dias: u16_de(f.plazo_pago_dias),
            correo_facturacion: DireccionCorreo::new(&f.correo_facturacion)?,
            estado: EstadoSuscripcion::try_from(f.estado.as_str())?,
            suspendida_por_mora: f.suspendida_por_mora,
            terminada_en: f.terminada_en,
            creada_por: f.creada_por.map(CuentaId::from_uuid),
            creada_en: f.creada_en,
        })
    }
}

struct FilaCorte {
    id: Uuid,
    suscripcion_id: Option<Uuid>,
    liquidacion_id: Option<Uuid>,
    periodo_desde: NaiveDate,
    periodo_hasta: NaiveDate,
    lineas: serde_json::Value,
    total: Decimal,
    huella: String,
    estado: String,
    motivo: Option<String>,
    aprobado_por: Option<Uuid>,
    aprobado_en: Option<DateTime<Utc>>,
    factura_venta_id: Option<Uuid>,
    factura_externa: Option<String>,
    facturado_en: Option<NaiveDate>,
    vencimiento: Option<NaiveDate>,
    pagado_en: Option<NaiveDate>,
    ultimo_aviso_en: Option<NaiveDate>,
    creado_en: DateTime<Utc>,
}

impl TryFrom<FilaCorte> for Corte {
    type Error = AppError;

    fn try_from(f: FilaCorte) -> AppResult<Self> {
        let origen = match (f.suscripcion_id, f.liquidacion_id) {
            (Some(s), _) => OrigenCorte::Suscripcion(SuscripcionId::from_uuid(s)),
            (None, Some(l)) => OrigenCorte::Liquidacion(LiquidacionId::from_uuid(l)),
            (None, None) => {
                return Err(AppError::Infraestructura("corte sin origen".into()));
            }
        };
        Ok(Self {
            id: CorteFacturacionId::from_uuid(f.id),
            origen,
            periodo: Periodo::new(f.periodo_desde, f.periodo_hasta)?,
            lineas: serde_json::from_value::<Vec<LineaCorte>>(f.lineas).map_err(AppError::infra)?,
            total: f.total,
            huella: f.huella,
            estado: EstadoCorte::try_from(f.estado.as_str())?,
            motivo: f.motivo,
            aprobado_por: f.aprobado_por.map(CuentaId::from_uuid),
            aprobado_en: f.aprobado_en,
            factura_venta_id: f.factura_venta_id.map(FacturaVentaId::from_uuid),
            factura_externa: f.factura_externa,
            facturado_en: f.facturado_en,
            vencimiento: f.vencimiento,
            pagado_en: f.pagado_en,
            ultimo_aviso_en: f.ultimo_aviso_en,
            creado_en: f.creado_en,
        })
    }
}

fn origen_ids(o: OrigenCorte) -> (Option<Uuid>, Option<Uuid>) {
    match o {
        OrigenCorte::Suscripcion(s) => (Some(s.as_uuid()), None),
        OrigenCorte::Liquidacion(l) => (None, Some(l.as_uuid())),
    }
}

impl TransaccionPlataformaPg {
    async fn cortes_donde(
        &mut self,
        id: Option<Uuid>,
        estado: Option<&str>,
        suscripcion: Option<Uuid>,
        antes_de: Option<Uuid>,
        limite: i64,
    ) -> AppResult<Vec<Corte>> {
        sqlx::query_as!(
            FilaCorte,
            "SELECT id, suscripcion_id, liquidacion_id, periodo_desde, periodo_hasta, lineas,
                    total, huella, estado, motivo, aprobado_por, aprobado_en, factura_venta_id,
                    factura_externa, facturado_en, vencimiento, pagado_en, ultimo_aviso_en,
                    creado_en
             FROM cortes_facturacion
             WHERE ($1::uuid IS NULL OR id = $1) AND ($2::text IS NULL OR estado = $2)
               AND ($3::uuid IS NULL OR suscripcion_id = $3)
               AND ($4::uuid IS NULL OR id < $4)
             ORDER BY id DESC LIMIT $5",
            id,
            estado,
            suscripcion,
            antes_de,
            limite,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Corte::try_from)
        .collect()
    }

    async fn suscripciones_donde(
        &mut self,
        id: Option<Uuid>,
        tenant: Option<Uuid>,
    ) -> AppResult<Vec<Suscripcion>> {
        sqlx::query_as!(
            FilaSuscripcion,
            "SELECT id, tenant_id, pagador_entidad_id, pagador_organizacion_id, ciclo_meses, inicio,
                    dias_prueba, plazo_pago_dias, correo_facturacion, estado, suspendida_por_mora,
                    terminada_en, creada_por, creada_en
             FROM suscripciones
             WHERE ($1::uuid IS NULL OR id = $1)
               AND ($2::uuid IS NULL OR (tenant_id = $2 AND estado <> 'terminada'))
               AND ($1::uuid IS NOT NULL OR estado <> 'terminada')
             ORDER BY id",
            id,
            tenant,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Suscripcion::try_from)
        .collect()
    }
}

impl RepositorioCobros for TransaccionPlataformaPg {
    async fn productos(&mut self) -> AppResult<Vec<Producto>> {
        sqlx::query_as!(
            FilaProducto,
            "SELECT id, codigo, nombre, descripcion_factura, modulo, item_facturacion_id, activo,
                    creado_en
             FROM productos_plataforma ORDER BY codigo"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Producto::try_from)
        .collect()
    }

    async fn producto(&mut self, id: ProductoPlataformaId) -> AppResult<Option<Producto>> {
        sqlx::query_as!(
            FilaProducto,
            "SELECT id, codigo, nombre, descripcion_factura, modulo, item_facturacion_id, activo,
                    creado_en
             FROM productos_plataforma WHERE id = $1",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Producto::try_from)
        .transpose()
    }

    async fn insertar_producto(&mut self, p: &Producto) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO productos_plataforma (id, codigo, nombre, descripcion_factura, modulo,
                                               item_facturacion_id, activo, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            p.id.as_uuid(),
            p.codigo,
            p.nombre,
            p.descripcion_factura,
            p.modulo.map(Modulo::as_str),
            p.item_facturacion_id.map(ItemFacturacionId::as_uuid),
            p.activo,
            p.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "ya hay un producto con ese código"))?;
        Ok(())
    }

    async fn actualizar_producto(&mut self, p: &Producto) -> AppResult<()> {
        sqlx::query!(
            "UPDATE productos_plataforma SET nombre = $2, descripcion_factura = $3, modulo = $4,
                 item_facturacion_id = $5, activo = $6
             WHERE id = $1",
            p.id.as_uuid(),
            p.nombre,
            p.descripcion_factura,
            p.modulo.map(Modulo::as_str),
            p.item_facturacion_id.map(ItemFacturacionId::as_uuid),
            p.activo,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn tarifas(&mut self, producto: ProductoPlataformaId) -> AppResult<Vec<TarifaProducto>> {
        sqlx::query!(
            "SELECT id, producto_id, modelo, valor, incluidas, valor_excedente, metrica,
                    vigente_desde, creada_por, creada_en
             FROM tarifas_producto WHERE producto_id = $1 ORDER BY vigente_desde DESC",
            producto.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(TarifaProducto {
                id: f.id,
                producto_id: ProductoPlataformaId::from_uuid(f.producto_id),
                tarifa: Tarifa {
                    modelo: ModeloTarifa::try_from(f.modelo.as_str())?,
                    valor: f.valor,
                    incluidas: f.incluidas.and_then(|i| u32::try_from(i).ok()),
                    valor_excedente: f.valor_excedente,
                    metrica: f.metrica.as_deref().map(Metrica::new).transpose()?,
                },
                vigente_desde: f.vigente_desde,
                creada_por: f.creada_por.map(CuentaId::from_uuid),
                creada_en: f.creada_en,
            })
        })
        .collect()
    }

    async fn insertar_tarifa(&mut self, t: &TarifaProducto) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO tarifas_producto (id, producto_id, modelo, valor, incluidas,
                                           valor_excedente, metrica, vigente_desde, creada_por,
                                           creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            t.id,
            t.producto_id.as_uuid(),
            t.tarifa.modelo.as_str(),
            t.tarifa.valor,
            t.tarifa
                .incluidas
                .map(|i| i32::try_from(i).unwrap_or(i32::MAX)),
            t.tarifa.valor_excedente,
            t.tarifa.metrica.as_ref().map(Metrica::as_str),
            t.vigente_desde,
            t.creada_por.map(CuentaId::as_uuid),
            t.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "ya hay una tarifa del producto desde ese día"))?;
        Ok(())
    }

    async fn insertar_suscripcion(&mut self, s: &Suscripcion) -> AppResult<()> {
        let (entidad, organizacion) = match s.pagador {
            Pagador::MismaEntidad => (None, None),
            Pagador::Entidad(e) => (Some(e.as_uuid()), None),
            Pagador::Organizacion(o) => (None, Some(o.as_uuid())),
        };
        sqlx::query!(
            "INSERT INTO suscripciones (id, tenant_id, pagador_entidad_id, pagador_organizacion_id,
                 ciclo_meses, inicio, dias_prueba, plazo_pago_dias, correo_facturacion, estado,
                 suspendida_por_mora, terminada_en, creada_por, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            s.id.as_uuid(),
            s.tenant_id.as_uuid(),
            entidad,
            organizacion,
            match s.ciclo {
                Ciclo::Meses(m) => Some(i16::from(m)),
                Ciclo::PorEvento => None,
            },
            s.inicio,
            i16::try_from(s.dias_prueba).unwrap_or(i16::MAX),
            i16::try_from(s.plazo_pago_dias).unwrap_or(i16::MAX),
            s.correo_facturacion.as_str(),
            s.estado.as_str(),
            s.suspendida_por_mora,
            s.terminada_en,
            s.creada_por.map(CuentaId::as_uuid),
            s.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "la entidad ya tiene una suscripción vigente"))?;
        Ok(())
    }

    async fn actualizar_suscripcion(&mut self, s: &Suscripcion) -> AppResult<()> {
        sqlx::query!(
            "UPDATE suscripciones SET plazo_pago_dias = $2, correo_facturacion = $3, estado = $4,
                 suspendida_por_mora = $5, terminada_en = $6
             WHERE id = $1",
            s.id.as_uuid(),
            i16::try_from(s.plazo_pago_dias).unwrap_or(i16::MAX),
            s.correo_facturacion.as_str(),
            s.estado.as_str(),
            s.suspendida_por_mora,
            s.terminada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn suscripcion(&mut self, id: SuscripcionId) -> AppResult<Option<Suscripcion>> {
        Ok(self
            .suscripciones_donde(Some(id.as_uuid()), None)
            .await?
            .into_iter()
            .next())
    }

    async fn suscripcion_de_entidad(&mut self, tenant: TenantId) -> AppResult<Option<Suscripcion>> {
        Ok(self
            .suscripciones_donde(None, Some(tenant.as_uuid()))
            .await?
            .into_iter()
            .next())
    }

    async fn suscripciones_vigentes(&mut self) -> AppResult<Vec<Suscripcion>> {
        self.suscripciones_donde(None, None).await
    }

    async fn lineas(&mut self, suscripcion: SuscripcionId) -> AppResult<Vec<LineaSuscripcion>> {
        Ok(sqlx::query!(
            "SELECT id, suscripcion_id, producto_id, precio_pactado, motivo_precio, desde, hasta,
                    creada_en
             FROM lineas_suscripcion WHERE suscripcion_id = $1 ORDER BY desde, id",
            suscripcion.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| LineaSuscripcion {
            id: LineaSuscripcionId::from_uuid(f.id),
            suscripcion_id: SuscripcionId::from_uuid(f.suscripcion_id),
            producto_id: ProductoPlataformaId::from_uuid(f.producto_id),
            precio_pactado: f.precio_pactado,
            motivo_precio: f.motivo_precio,
            desde: f.desde,
            hasta: f.hasta,
            creada_en: f.creada_en,
        })
        .collect())
    }

    async fn linea(&mut self, id: LineaSuscripcionId) -> AppResult<Option<LineaSuscripcion>> {
        Ok(sqlx::query!(
            "SELECT id, suscripcion_id, producto_id, precio_pactado, motivo_precio, desde, hasta,
                    creada_en
             FROM lineas_suscripcion WHERE id = $1",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| LineaSuscripcion {
            id: LineaSuscripcionId::from_uuid(f.id),
            suscripcion_id: SuscripcionId::from_uuid(f.suscripcion_id),
            producto_id: ProductoPlataformaId::from_uuid(f.producto_id),
            precio_pactado: f.precio_pactado,
            motivo_precio: f.motivo_precio,
            desde: f.desde,
            hasta: f.hasta,
            creada_en: f.creada_en,
        }))
    }

    async fn insertar_linea(&mut self, l: &LineaSuscripcion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO lineas_suscripcion (id, suscripcion_id, producto_id, precio_pactado,
                                             motivo_precio, desde, hasta, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            l.id.as_uuid(),
            l.suscripcion_id.as_uuid(),
            l.producto_id.as_uuid(),
            l.precio_pactado,
            l.motivo_precio,
            l.desde,
            l.hasta,
            l.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn terminar_linea(&mut self, id: LineaSuscripcionId, hasta: NaiveDate) -> AppResult<()> {
        sqlx::query!(
            "UPDATE lineas_suscripcion SET hasta = $2 WHERE id = $1",
            id.as_uuid(),
            hasta
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn medir(
        &mut self,
        tenant: TenantId,
        metrica: &Metrica,
        periodo: &Periodo,
    ) -> AppResult<u64> {
        let n: Option<i64> = sqlx::query_scalar!(
            "SELECT medir_consumo($1, $2, $3, $4)",
            tenant.as_uuid(),
            metrica.as_str(),
            periodo.desde,
            periodo.hasta,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        n.and_then(|n| u64::try_from(n).ok()).ok_or_else(|| {
            AppError::Validacion(format!("la métrica {} no existe", metrica.as_str()))
        })
    }

    async fn medicion(
        &mut self,
        tenant: TenantId,
        metrica: &Metrica,
        periodo: &Periodo,
    ) -> AppResult<Option<u64>> {
        Ok(sqlx::query_scalar!(
            "SELECT cantidad FROM mediciones_consumo
             WHERE tenant_id = $1 AND metrica = $2 AND periodo_desde = $3 AND periodo_hasta = $4",
            tenant.as_uuid(),
            metrica.as_str(),
            periodo.desde,
            periodo.hasta,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .and_then(|n| u64::try_from(n).ok()))
    }

    async fn guardar_medicion(
        &mut self,
        tenant: TenantId,
        metrica: &Metrica,
        periodo: &Periodo,
        cantidad: u64,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO mediciones_consumo (id, tenant_id, metrica, periodo_desde, periodo_hasta,
                                             cantidad, medida_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (tenant_id, metrica, periodo_desde, periodo_hasta) DO NOTHING",
            Uuid::now_v7(),
            tenant.as_uuid(),
            metrica.as_str(),
            periodo.desde,
            periodo.hasta,
            i64::try_from(cantidad).unwrap_or(i64::MAX),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn cortes(&mut self, f: &FiltroCortes, p: SolicitudPagina) -> AppResult<Pagina<Corte>> {
        let filas = self
            .cortes_donde(
                None,
                f.estado.map(EstadoCorte::as_str),
                f.suscripcion.map(SuscripcionId::as_uuid),
                p.despues(),
                p.filas_a_consultar(),
            )
            .await?;
        Ok(Pagina::desde_filas(filas, p, |c| c.id.as_uuid()))
    }

    async fn corte(&mut self, id: CorteFacturacionId, bloquear: bool) -> AppResult<Option<Corte>> {
        if bloquear {
            sqlx::query!(
                "SELECT 1 AS x FROM cortes_facturacion WHERE id = $1 FOR UPDATE",
                id.as_uuid()
            )
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        }
        Ok(self
            .cortes_donde(Some(id.as_uuid()), None, None, None, 1)
            .await?
            .into_iter()
            .next())
    }

    async fn corte_vigente(
        &mut self,
        origen: OrigenCorte,
        desde: NaiveDate,
    ) -> AppResult<Option<Corte>> {
        let (s, l) = origen_ids(origen);
        let id = sqlx::query_scalar!(
            "SELECT id FROM cortes_facturacion
             WHERE estado <> 'descartado'
               AND (($1::uuid IS NOT NULL AND suscripcion_id = $1 AND periodo_desde = $3)
                    OR ($2::uuid IS NOT NULL AND liquidacion_id = $2))
             FOR UPDATE",
            s,
            l,
            desde,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        match id {
            Some(id) => self.corte(CorteFacturacionId::from_uuid(id), false).await,
            None => Ok(None),
        }
    }

    async fn ultimo_corte(&mut self, suscripcion: SuscripcionId) -> AppResult<Option<Corte>> {
        let id = sqlx::query_scalar!(
            "SELECT id FROM cortes_facturacion
             WHERE suscripcion_id = $1 AND estado <> 'descartado'
             ORDER BY periodo_desde DESC LIMIT 1",
            suscripcion.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        match id {
            Some(id) => self.corte(CorteFacturacionId::from_uuid(id), false).await,
            None => Ok(None),
        }
    }

    async fn corte_anterior_cobrado(
        &mut self,
        suscripcion: SuscripcionId,
        desde: NaiveDate,
    ) -> AppResult<Option<Corte>> {
        let id = sqlx::query_scalar!(
            "SELECT id FROM cortes_facturacion
             WHERE suscripcion_id = $1 AND periodo_desde < $2
               AND estado IN ('aprobado', 'facturado', 'pagado')
             ORDER BY periodo_desde DESC LIMIT 1",
            suscripcion.as_uuid(),
            desde,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        match id {
            Some(id) => self.corte(CorteFacturacionId::from_uuid(id), false).await,
            None => Ok(None),
        }
    }

    async fn cortes_por_cobrar(&mut self) -> AppResult<Vec<Corte>> {
        self.cortes_donde(None, Some("facturado"), None, None, 10_000)
            .await
    }

    async fn insertar_corte(&mut self, c: &Corte) -> AppResult<()> {
        let (s, l) = origen_ids(c.origen);
        sqlx::query!(
            "INSERT INTO cortes_facturacion (id, suscripcion_id, liquidacion_id, periodo_desde,
                 periodo_hasta, lineas, total, huella, estado, motivo, aprobado_por, aprobado_en,
                 factura_venta_id, factura_externa, facturado_en, vencimiento, pagado_en,
                 ultimo_aviso_en, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18, $19)",
            c.id.as_uuid(),
            s,
            l,
            c.periodo.desde,
            c.periodo.hasta,
            serde_json::to_value(&c.lineas).map_err(AppError::infra)?,
            c.total,
            c.huella,
            c.estado.as_str(),
            c.motivo,
            c.aprobado_por.map(CuentaId::as_uuid),
            c.aprobado_en,
            c.factura_venta_id.map(FacturaVentaId::as_uuid),
            c.factura_externa,
            c.facturado_en,
            c.vencimiento,
            c.pagado_en,
            c.ultimo_aviso_en,
            c.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "ya hay un corte vigente de ese periodo"))?;
        Ok(())
    }

    async fn actualizar_corte(&mut self, c: &Corte) -> AppResult<()> {
        sqlx::query!(
            "UPDATE cortes_facturacion SET lineas = $2, total = $3, huella = $4, estado = $5,
                 motivo = $6, aprobado_por = $7, aprobado_en = $8, factura_venta_id = $9,
                 factura_externa = $10, facturado_en = $11, vencimiento = $12, pagado_en = $13,
                 ultimo_aviso_en = $14
             WHERE id = $1",
            c.id.as_uuid(),
            serde_json::to_value(&c.lineas).map_err(AppError::infra)?,
            c.total,
            c.huella,
            c.estado.as_str(),
            c.motivo,
            c.aprobado_por.map(CuentaId::as_uuid),
            c.aprobado_en,
            c.factura_venta_id.map(FacturaVentaId::as_uuid),
            c.factura_externa,
            c.facturado_en,
            c.vencimiento,
            c.pagado_en,
            c.ultimo_aviso_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn cliente_entidad(&mut self, tenant: TenantId) -> AppResult<Option<ClienteFacturacion>> {
        let Some(f) = sqlx::query!(
            "SELECT nombre, razon_social, nit, correo FROM entidades WHERE id = $1",
            tenant.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        else {
            return Ok(None);
        };
        Ok(Some(ClienteFacturacion {
            nombre: f.razon_social.unwrap_or(f.nombre),
            nit: nit_base(f.nit),
            correo: correo(f.correo)?,
        }))
    }

    async fn cliente_organizacion(
        &mut self,
        organizacion: OrganizacionId,
    ) -> AppResult<Option<ClienteFacturacion>> {
        let Some(f) = sqlx::query!(
            "SELECT razon_social, nit, correo_contacto FROM organizaciones WHERE id = $1",
            organizacion.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        else {
            return Ok(None);
        };
        Ok(Some(ClienteFacturacion {
            nombre: f.razon_social,
            nit: nit_base(Some(f.nit)),
            correo: correo(f.correo_contacto)?,
        }))
    }

    async fn liquidacion_a_cobrar(
        &mut self,
        id: LiquidacionId,
    ) -> AppResult<Option<LiquidacionACobrar>> {
        sqlx::query!(
            "SELECT l.id, l.periodo, l.valor, l.estado, a.organizacion_id, a.nombre_comercial
             FROM liquidaciones l JOIN anunciantes a ON a.id = l.anunciante_id
             WHERE l.id = $1",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| {
            Ok::<_, AppError>(LiquidacionACobrar {
                id: LiquidacionId::from_uuid(f.id),
                organizacion_id: OrganizacionId::from_uuid(f.organizacion_id),
                nombre_comercial: f.nombre_comercial,
                periodo: f.periodo,
                valor: f.valor,
                estado: domain::publicidad::EstadoLiquidacion::try_from(f.estado.as_str())?,
            })
        })
        .transpose()
    }

    async fn avanzar_liquidacion(
        &mut self,
        id: LiquidacionId,
        desde: domain::publicidad::EstadoLiquidacion,
        hacia: domain::publicidad::EstadoLiquidacion,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE liquidaciones SET estado = $3, actualizada_en = $4
             WHERE id = $1 AND estado = $2",
            id.as_uuid(),
            desde.as_str(),
            hacia.as_str(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}
