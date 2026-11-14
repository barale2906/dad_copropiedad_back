//! Facturación sobre la transacción de la entidad (plan 18): catálogo de productos y servicios
//! (`items_facturacion`) y número interno de las facturas de venta (`consecutivos`, serie
//! `factura_venta`).

use application::{
    AppError, AppResult,
    puertos::facturacion::{
        CertificadoFirma, ConfiguracionFacturacion, EnvioSetPruebas, FiltroItems, ItemFacturacion,
        RepositorioFacturacion,
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    CertificadoFirmaId, CuentaContableId, CuentaId, EnvioSetPruebasId, FacturaVentaId,
    ItemFacturacionId,
    contabilidad::Monto,
    facturacion::{EstadoEnvioSet, EstadoHabilitacion},
    tributario::{ClasificacionTributaria, TipoDocumentoElectronico},
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{TransaccionPg, contabilidad::error_contable};

struct FilaItem {
    id: Uuid,
    codigo: String,
    descripcion: String,
    unidad: String,
    precio: Option<Decimal>,
    tarifa_iva: Decimal,
    clasificacion: String,
    cuenta_id: Option<Uuid>,
    activo: bool,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
    actualizado_en: DateTime<Utc>,
}

impl TryFrom<FilaItem> for ItemFacturacion {
    type Error = AppError;

    fn try_from(f: FilaItem) -> AppResult<Self> {
        Ok(Self {
            id: ItemFacturacionId::from_uuid(f.id),
            codigo: f.codigo,
            descripcion: f.descripcion,
            unidad: f.unidad,
            precio: f.precio.map(Monto::new).transpose()?,
            tarifa_iva: f.tarifa_iva,
            clasificacion: ClasificacionTributaria::try_from(f.clasificacion.as_str())?,
            cuenta_id: f.cuenta_id.map(CuentaContableId::from_uuid),
            activo: f.activo,
            creado_por: f.creado_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
            actualizado_en: f.actualizado_en,
        })
    }
}

struct FilaCertificado {
    id: Uuid,
    p12_cifrado: Vec<u8>,
    contrasena_cifrada: Vec<u8>,
    sujeto: String,
    emisor: String,
    serial: String,
    vigente_desde: DateTime<Utc>,
    vigente_hasta: DateTime<Utc>,
    activo: bool,
    cargado_por: Option<Uuid>,
    cargado_en: DateTime<Utc>,
}

impl From<FilaCertificado> for CertificadoFirma {
    fn from(f: FilaCertificado) -> Self {
        Self {
            id: CertificadoFirmaId::from_uuid(f.id),
            p12_cifrado: f.p12_cifrado,
            contrasena_cifrada: f.contrasena_cifrada,
            sujeto: f.sujeto,
            emisor: f.emisor,
            serial: f.serial,
            vigente_desde: f.vigente_desde,
            vigente_hasta: f.vigente_hasta,
            activo: f.activo,
            cargado_por: f.cargado_por.map(CuentaId::from_uuid),
            cargado_en: f.cargado_en,
        }
    }
}

struct FilaEnvio {
    id: Uuid,
    tipo: String,
    prefijo: String,
    numero: i64,
    codigo: String,
    clave: Option<String>,
    estado: String,
    respuesta: Option<String>,
    enviado_en: DateTime<Utc>,
    consultado_en: Option<DateTime<Utc>>,
}

impl TryFrom<FilaEnvio> for EnvioSetPruebas {
    type Error = AppError;

    fn try_from(f: FilaEnvio) -> AppResult<Self> {
        Ok(Self {
            id: EnvioSetPruebasId::from_uuid(f.id),
            tipo: TipoDocumentoElectronico::try_from(f.tipo.as_str())?,
            prefijo: f.prefijo,
            numero: u64::try_from(f.numero).unwrap_or_default(),
            codigo: f.codigo,
            clave: f.clave,
            estado: EstadoEnvioSet::try_from(f.estado.as_str())?,
            respuesta: f.respuesta,
            enviado_en: f.enviado_en,
            consultado_en: f.consultado_en,
        })
    }
}

fn i64_de(n: u64) -> AppResult<i64> {
    i64::try_from(n).map_err(|_| AppError::Validacion("número fuera de rango".into()))
}

impl RepositorioFacturacion for TransaccionPg {
    async fn siguiente_numero_factura(&mut self, anio: i32) -> AppResult<u32> {
        self.siguiente_en_serie("factura_venta", anio).await
    }

    async fn siguiente_consecutivo_archivo(&mut self, anio: i32) -> AppResult<u32> {
        self.siguiente_en_serie("archivo_dian", anio).await
    }

    async fn configuracion_facturacion(&mut self) -> AppResult<Option<ConfiguracionFacturacion>> {
        sqlx::query!(
            "SELECT software_id, pin_cifrado, test_set_id, estado, certificado_id,
                    actualizado_por, actualizado_en
             FROM configuraciones_facturacion WHERE tenant_id = $1",
            self.ctx.tenant_id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| {
            Ok(ConfiguracionFacturacion {
                software_id: f.software_id,
                pin_cifrado: f.pin_cifrado,
                test_set_id: f.test_set_id,
                estado: EstadoHabilitacion::try_from(f.estado.as_str())?,
                certificado_id: f.certificado_id.map(CertificadoFirmaId::from_uuid),
                actualizado_por: f.actualizado_por.map(CuentaId::from_uuid),
                actualizado_en: f.actualizado_en,
            })
        })
        .transpose()
    }

    async fn guardar_configuracion_facturacion(
        &mut self,
        c: &ConfiguracionFacturacion,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO configuraciones_facturacion (tenant_id, software_id, pin_cifrado,
                 test_set_id, ambiente, estado, certificado_id, actualizado_por, actualizado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (tenant_id) DO UPDATE SET software_id = EXCLUDED.software_id,
                 pin_cifrado = EXCLUDED.pin_cifrado, test_set_id = EXCLUDED.test_set_id,
                 ambiente = EXCLUDED.ambiente, estado = EXCLUDED.estado,
                 certificado_id = EXCLUDED.certificado_id,
                 actualizado_por = EXCLUDED.actualizado_por,
                 actualizado_en = EXCLUDED.actualizado_en",
            self.ctx.tenant_id.as_uuid(),
            c.software_id,
            c.pin_cifrado,
            c.test_set_id,
            c.estado.ambiente().as_str(),
            c.estado.as_str(),
            c.certificado_id.map(CertificadoFirmaId::as_uuid),
            c.actualizado_por.map(CuentaId::as_uuid),
            c.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "configuración repetida", "certificado"))?;
        Ok(())
    }

    async fn insertar_certificado(&mut self, c: &CertificadoFirma) -> AppResult<()> {
        sqlx::query!("UPDATE certificados_firma SET activo = FALSE WHERE activo",)
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        sqlx::query!(
            "INSERT INTO certificados_firma (id, tenant_id, p12_cifrado, contrasena_cifrada, sujeto,
                 emisor, serial, vigente_desde, vigente_hasta, activo, cargado_por, cargado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.p12_cifrado,
            c.contrasena_cifrada,
            c.sujeto,
            c.emisor,
            c.serial,
            c.vigente_desde,
            c.vigente_hasta,
            c.activo,
            c.cargado_por.map(CuentaId::as_uuid),
            c.cargado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "certificado repetido", "entidad"))?;
        Ok(())
    }

    async fn certificado_activo(&mut self) -> AppResult<Option<CertificadoFirma>> {
        Ok(sqlx::query_as!(
            FilaCertificado,
            "SELECT id, p12_cifrado, contrasena_cifrada, sujeto, emisor, serial, vigente_desde,
                    vigente_hasta, activo, cargado_por, cargado_en
             FROM certificados_firma WHERE activo",
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Into::into))
    }

    async fn insertar_envio_set(&mut self, e: &EnvioSetPruebas) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO envios_set_pruebas (id, tenant_id, tipo, prefijo, numero, codigo, clave,
                 estado, respuesta, enviado_en, consultado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            e.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            e.tipo.as_str(),
            e.prefijo,
            i64_de(e.numero)?,
            e.codigo,
            e.clave,
            e.estado.as_str(),
            e.respuesta,
            e.enviado_en,
            e.consultado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|x| error_contable(x, "ese número del set ya se envió", "entidad"))?;
        Ok(())
    }

    async fn actualizar_envio_set(&mut self, e: &EnvioSetPruebas) -> AppResult<()> {
        sqlx::query!(
            "UPDATE envios_set_pruebas SET estado = $2, respuesta = $3, consultado_en = $4
             WHERE id = $1",
            e.id.as_uuid(),
            e.estado.as_str(),
            e.respuesta,
            e.consultado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn envios_set(&mut self) -> AppResult<Vec<EnvioSetPruebas>> {
        sqlx::query_as!(
            FilaEnvio,
            "SELECT id, tipo, prefijo, numero, codigo, clave, estado, respuesta, enviado_en,
                    consultado_en
             FROM envios_set_pruebas ORDER BY enviado_en DESC, numero DESC",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn insertar_item(&mut self, i: &ItemFacturacion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO items_facturacion (id, tenant_id, codigo, descripcion, unidad, precio,
                 tarifa_iva, clasificacion, cuenta_id, activo, creado_por, creado_en,
                 actualizado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            i.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            i.codigo,
            i.descripcion,
            i.unidad,
            i.precio.map(Monto::valor),
            i.tarifa_iva,
            i.clasificacion.as_str(),
            i.cuenta_id.map(CuentaContableId::as_uuid),
            i.activo,
            i.creado_por.map(CuentaId::as_uuid),
            i.creado_en,
            i.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "ya existe un ítem con ese código", "cuenta contable"))?;
        Ok(())
    }

    async fn actualizar_item(&mut self, i: &ItemFacturacion) -> AppResult<()> {
        sqlx::query!(
            "UPDATE items_facturacion SET descripcion = $2, unidad = $3, precio = $4,
                 tarifa_iva = $5, clasificacion = $6, cuenta_id = $7, activo = $8,
                 actualizado_en = $9
             WHERE id = $1",
            i.id.as_uuid(),
            i.descripcion,
            i.unidad,
            i.precio.map(Monto::valor),
            i.tarifa_iva,
            i.clasificacion.as_str(),
            i.cuenta_id.map(CuentaContableId::as_uuid),
            i.activo,
            i.actualizado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "ítem repetido", "cuenta contable"))?;
        Ok(())
    }

    async fn facturas_sin_contabilizar(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
        limite: u32,
    ) -> AppResult<Vec<FacturaVentaId>> {
        Ok(sqlx::query_scalar!(
            "SELECT id FROM facturas_venta
             WHERE comprobante_id IS NULL AND estado NOT IN ('borrador', 'anulada')
               AND fecha BETWEEN $1 AND $2
             ORDER BY fecha, numero, id
             LIMIT $3",
            desde,
            hasta,
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(FacturaVentaId::from_uuid)
        .collect())
    }

    async fn item(&mut self, id: ItemFacturacionId) -> AppResult<Option<ItemFacturacion>> {
        sqlx::query_as!(
            FilaItem,
            "SELECT id, codigo, descripcion, unidad, precio, tarifa_iva, clasificacion, cuenta_id,
                    activo, creado_por, creado_en, actualizado_en
             FROM items_facturacion WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn items(
        &mut self,
        filtro: &FiltroItems,
        limite: u32,
    ) -> AppResult<Vec<ItemFacturacion>> {
        sqlx::query_as!(
            FilaItem,
            "SELECT id, codigo, descripcion, unidad, precio, tarifa_iva, clasificacion, cuenta_id,
                    activo, creado_por, creado_en, actualizado_en
             FROM items_facturacion
             WHERE ($1 OR activo)
               AND ($2::text IS NULL
                    OR normalizar(descripcion) LIKE '%' || normalizar($2) || '%'
                    OR normalizar(codigo) LIKE '%' || normalizar($2) || '%')
             ORDER BY normalizar(descripcion), id
             LIMIT $3",
            filtro.con_inactivos,
            filtro.texto.as_deref(),
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }
}
