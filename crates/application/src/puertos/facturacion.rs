//! Puerto de la facturación (plan 18, ADR 0022): catálogo de productos y servicios, numeración
//! interna de las facturas de venta y habilitación de la entidad ante la DIAN (configuración,
//! certificado de firma y set de pruebas). Las facturas en sí siguen en [`super::tesoreria`] (las
//! comparte la contabilidad, que lleva su recaudo).

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    CertificadoFirmaId, CuentaContableId, CuentaId, EnvioSetPruebasId, FacturaVentaId,
    ItemFacturacionId,
    contabilidad::Monto,
    facturacion::{EstadoEnvioSet, EstadoHabilitacion},
    tributario::{ClasificacionTributaria, TipoDocumentoElectronico},
};
use rust_decimal::Decimal;

use crate::AppResult;

/// Producto o servicio que se factura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemFacturacion {
    pub id: ItemFacturacionId,
    /// Único en la entidad.
    pub codigo: String,
    pub descripcion: String,
    /// Código de unidad de la DIAN (`94` = unidad).
    pub unidad: String,
    /// Precio sugerido; cada línea lleva el suyo.
    pub precio: Option<Monto>,
    pub tarifa_iva: Decimal,
    pub clasificacion: ClasificacionTributaria,
    /// Cuenta de ingreso: solo con el módulo de contabilidad.
    pub cuenta_id: Option<CuentaContableId>,
    pub activo: bool,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
    pub actualizado_en: DateTime<Utc>,
}

/// Filtro del catálogo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroItems {
    /// Busca en el código y la descripción, sin tildes ni mayúsculas.
    pub texto: Option<String>,
    /// Incluir los inactivos.
    pub con_inactivos: bool,
}

/// Configuración de la entidad para facturar con el motor propio.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfiguracionFacturacion {
    /// SoftwareID que asignó la DIAN.
    pub software_id: String,
    /// PIN del software, cifrado con el [`super::cofre::Cofre`].
    pub pin_cifrado: Vec<u8>,
    pub test_set_id: Option<String>,
    pub estado: EstadoHabilitacion,
    pub certificado_id: Option<CertificadoFirmaId>,
    pub actualizado_por: Option<CuentaId>,
    pub actualizado_en: DateTime<Utc>,
}

impl std::fmt::Debug for ConfiguracionFacturacion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfiguracionFacturacion")
            .field("software_id", &self.software_id)
            .field("pin_cifrado", &"<oculto>")
            .field("estado", &self.estado)
            .finish()
    }
}

/// Certificado de firma cargado por la entidad (el `.p12` y su contraseña, cifrados).
#[derive(Clone, PartialEq, Eq)]
pub struct CertificadoFirma {
    pub id: CertificadoFirmaId,
    /// El `.p12`, cifrado con el [`super::cofre::Cofre`].
    pub p12_cifrado: Vec<u8>,
    /// Contraseña del `.p12`, cifrada con el [`super::cofre::Cofre`].
    pub contrasena_cifrada: Vec<u8>,
    pub sujeto: String,
    pub emisor: String,
    pub serial: String,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: DateTime<Utc>,
    pub activo: bool,
    pub cargado_por: Option<CuentaId>,
    pub cargado_en: DateTime<Utc>,
}

impl std::fmt::Debug for CertificadoFirma {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertificadoFirma")
            .field("id", &self.id)
            .field("sujeto", &self.sujeto)
            .field("contrasena_cifrada", &"<oculta>")
            .finish()
    }
}

/// Documento del set de pruebas enviado a la DIAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvioSetPruebas {
    pub id: EnvioSetPruebasId,
    pub tipo: TipoDocumentoElectronico,
    pub prefijo: String,
    pub numero: u64,
    /// CUFE o CUDE del documento.
    pub codigo: String,
    /// Clave del envío (`zipKey`) para consultarlo.
    pub clave: Option<String>,
    pub estado: EstadoEnvioSet,
    pub respuesta: Option<String>,
    pub enviado_en: DateTime<Utc>,
    pub consultado_en: Option<DateTime<Utc>>,
}

/// Persistencia propia de la facturación sobre la transacción de la entidad.
pub trait RepositorioFacturacion: Send {
    /// Siguiente número interno de factura de venta en el año (atómico en la transacción).
    fn siguiente_numero_factura(
        &mut self,
        anio: i32,
    ) -> impl Future<Output = AppResult<u32>> + Send;

    /// Siguiente número del archivo enviado a la DIAN en el año (nombre del XML y del ZIP).
    fn siguiente_consecutivo_archivo(
        &mut self,
        anio: i32,
    ) -> impl Future<Output = AppResult<u32>> + Send;

    /// Siguiente número de una nota (crédito, débito o de ajuste) en el año (atómico en la
    /// transacción; una serie por tipo).
    fn siguiente_numero_nota(
        &mut self,
        tipo: domain::tributario::TipoDocumentoElectronico,
        anio: i32,
    ) -> impl Future<Output = AppResult<u32>> + Send;

    /// Correo de recepción que el NIT registró ante la DIAN, si está en la lista sincronizada.
    fn correo_recepcion_dian(
        &mut self,
        nit: &str,
    ) -> impl Future<Output = AppResult<Option<String>>> + Send;

    /// Siguiente número de evento RADIAN en el año (atómico en la transacción).
    fn siguiente_numero_evento(&mut self, anio: i32)
    -> impl Future<Output = AppResult<u32>> + Send;

    /// Configuración de la habilitación, si ya se registró.
    fn configuracion_facturacion(
        &mut self,
    ) -> impl Future<Output = AppResult<Option<ConfiguracionFacturacion>>> + Send;

    /// Crea o reemplaza la configuración.
    fn guardar_configuracion_facturacion(
        &mut self,
        c: &ConfiguracionFacturacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda un certificado nuevo como el activo (desactiva el anterior).
    fn insertar_certificado(
        &mut self,
        c: &CertificadoFirma,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// El certificado activo, si hay.
    fn certificado_activo(
        &mut self,
    ) -> impl Future<Output = AppResult<Option<CertificadoFirma>>> + Send;

    /// Registra un documento del set de pruebas enviado.
    fn insertar_envio_set(
        &mut self,
        e: &EnvioSetPruebas,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Actualiza el resultado de un envío del set.
    fn actualizar_envio_set(
        &mut self,
        e: &EnvioSetPruebas,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Envíos del set de pruebas, del más reciente al más antiguo.
    fn envios_set(&mut self) -> impl Future<Output = AppResult<Vec<EnvioSetPruebas>>> + Send;

    /// Guarda un ítem nuevo.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el código ya existe en la entidad.
    fn insertar_item(
        &mut self,
        item: &ItemFacturacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Reemplaza los datos editables (todo menos el código y la creación).
    fn actualizar_item(
        &mut self,
        item: &ItemFacturacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Ítem por id.
    fn item(
        &mut self,
        id: ItemFacturacionId,
    ) -> impl Future<Output = AppResult<Option<ItemFacturacion>>> + Send;

    /// Facturas de venta emitidas (no anuladas) sin contabilizar con fecha en el rango, por
    /// fecha (a lo sumo `limite`).
    fn facturas_sin_contabilizar(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<FacturaVentaId>>> + Send;

    /// Catálogo ordenado por descripción (a lo sumo `limite`).
    fn items(
        &mut self,
        filtro: &FiltroItems,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<ItemFacturacion>>> + Send;
}
