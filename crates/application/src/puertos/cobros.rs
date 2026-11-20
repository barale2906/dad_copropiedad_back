//! Cobro de la plataforma a sus clientes (plan 19, ADR 0024): catálogo comercial, tarifas,
//! suscripciones, mediciones de consumo y cortes. Todo vive en tablas de **plataforma** (lo
//! implementa la transacción de plataforma); la medición entre entidades va por una función
//! `SECURITY DEFINER` que solo devuelve números.

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    CorteFacturacionId, CuentaId, DireccionCorreo, FacturaVentaId, ItemFacturacionId,
    LineaSuscripcionId, LiquidacionId, Modulo, OrganizacionId, ProductoPlataformaId, SuscripcionId,
    TenantId,
    cobro_plataforma::{
        Ciclo, EstadoCorte, EstadoSuscripcion, LineaCorte, Metrica, Periodo, Tarifa,
    },
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{AppResult, Pagina, SolicitudPagina};

/// Producto del catálogo comercial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Producto {
    pub id: ProductoPlataformaId,
    pub codigo: String,
    pub nombre: String,
    /// Plantilla del texto de la factura (`{producto}`, `{periodo}`, `{cantidad}`,
    /// `{incluidas}`).
    pub descripcion_factura: String,
    /// Módulo que cobra, si cobra uno (implementación o capacitación no).
    pub modulo: Option<Modulo>,
    /// Ítem de facturación del tenant de la plataforma (cuenta e IVA salen de él).
    pub item_facturacion_id: Option<ItemFacturacionId>,
    pub activo: bool,
    pub creado_en: DateTime<Utc>,
}

/// Tarifa de un producto desde una fecha (versionada: nunca se edita).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarifaProducto {
    pub id: Uuid,
    pub producto_id: ProductoPlataformaId,
    pub tarifa: Tarifa,
    pub vigente_desde: NaiveDate,
    pub creada_por: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
}

/// Quién paga la suscripción.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pagador {
    /// La misma entidad.
    MismaEntidad,
    /// Otra entidad del sistema.
    Entidad(TenantId),
    /// Una organización (p. ej. la administradora que paga por varias PH).
    Organizacion(OrganizacionId),
}

/// Suscripción de una entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suscripcion {
    pub id: SuscripcionId,
    pub tenant_id: TenantId,
    pub pagador: Pagador,
    pub ciclo: Ciclo,
    pub inicio: NaiveDate,
    /// Días de prueba gratuita (0 a 30): no se cobran.
    pub dias_prueba: u16,
    /// Días entre la factura y su vencimiento.
    pub plazo_pago_dias: u16,
    pub correo_facturacion: DireccionCorreo,
    pub estado: EstadoSuscripcion,
    /// La suspendió la plataforma por mora (al pagar se reactiva la entidad).
    pub suspendida_por_mora: bool,
    pub terminada_en: Option<NaiveDate>,
    pub creada_por: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
}

impl Suscripcion {
    /// Primer día que se cobra (después de la prueba).
    pub fn cobra_desde(&self) -> NaiveDate {
        self.inicio + chrono::Duration::days(i64::from(self.dias_prueba))
    }
}

/// Producto contratado en una suscripción.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineaSuscripcion {
    pub id: LineaSuscripcionId,
    pub suscripcion_id: SuscripcionId,
    pub producto_id: ProductoPlataformaId,
    /// Precio pactado que reemplaza la tarifa (con su motivo).
    pub precio_pactado: Option<Decimal>,
    pub motivo_precio: Option<String>,
    pub desde: NaiveDate,
    pub hasta: Option<NaiveDate>,
    pub creada_en: DateTime<Utc>,
}

/// De dónde sale un corte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrigenCorte {
    Suscripcion(SuscripcionId),
    /// La liquidación mensual de publicidad de un anunciante.
    Liquidacion(LiquidacionId),
}

/// Corte de cobro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Corte {
    pub id: CorteFacturacionId,
    pub origen: OrigenCorte,
    pub periodo: Periodo,
    pub lineas: Vec<LineaCorte>,
    pub total: Decimal,
    /// SHA-256 de las mediciones y las líneas.
    pub huella: String,
    pub estado: EstadoCorte,
    pub motivo: Option<String>,
    pub aprobado_por: Option<CuentaId>,
    pub aprobado_en: Option<DateTime<Utc>>,
    pub factura_venta_id: Option<FacturaVentaId>,
    /// Número de la factura emitida por fuera (mientras la plataforma no factura en el sistema).
    pub factura_externa: Option<String>,
    pub facturado_en: Option<NaiveDate>,
    pub vencimiento: Option<NaiveDate>,
    pub pagado_en: Option<NaiveDate>,
    pub ultimo_aviso_en: Option<NaiveDate>,
    pub creado_en: DateTime<Utc>,
}

/// Datos fiscales de quien recibe la factura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClienteFacturacion {
    pub nombre: String,
    /// NIT sin dígito de verificación ni separadores; `None` si no lo tiene registrado.
    pub nit: Option<String>,
    pub correo: Option<DireccionCorreo>,
}

/// Liquidación de publicidad con los datos del anunciante para cobrarla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquidacionACobrar {
    pub id: LiquidacionId,
    pub organizacion_id: OrganizacionId,
    pub nombre_comercial: String,
    /// Primer día del mes liquidado.
    pub periodo: NaiveDate,
    pub valor: Decimal,
    pub estado: domain::publicidad::EstadoLiquidacion,
}

/// Filtro del listado de cortes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroCortes {
    pub estado: Option<EstadoCorte>,
    pub suscripcion: Option<SuscripcionId>,
}

/// Cobros de la plataforma (transacción de plataforma).
pub trait RepositorioCobros: Send {
    /// Productos del catálogo (activos e inactivos), por código.
    fn productos(&mut self) -> impl Future<Output = AppResult<Vec<Producto>>> + Send;

    /// Producto por su id.
    fn producto(
        &mut self,
        id: ProductoPlataformaId,
    ) -> impl Future<Output = AppResult<Option<Producto>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el código ya existe.
    fn insertar_producto(&mut self, p: &Producto) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda los cambios del producto (nombre, texto, módulo, ítem, activo).
    fn actualizar_producto(&mut self, p: &Producto) -> impl Future<Output = AppResult<()>> + Send;

    /// Tarifas del producto, la más reciente primero.
    fn tarifas(
        &mut self,
        producto: ProductoPlataformaId,
    ) -> impl Future<Output = AppResult<Vec<TarifaProducto>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay una tarifa del producto desde ese día.
    fn insertar_tarifa(&mut self, t: &TarifaProducto)
    -> impl Future<Output = AppResult<()>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si la entidad ya tiene una suscripción vigente.
    fn insertar_suscripcion(
        &mut self,
        s: &Suscripcion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda los cambios de la suscripción.
    fn actualizar_suscripcion(
        &mut self,
        s: &Suscripcion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Suscripción por su id.
    fn suscripcion(
        &mut self,
        id: SuscripcionId,
    ) -> impl Future<Output = AppResult<Option<Suscripcion>>> + Send;

    /// Suscripción vigente (no terminada) de la entidad.
    fn suscripcion_de_entidad(
        &mut self,
        tenant: TenantId,
    ) -> impl Future<Output = AppResult<Option<Suscripcion>>> + Send;

    /// Suscripciones no terminadas, por id.
    fn suscripciones_vigentes(
        &mut self,
    ) -> impl Future<Output = AppResult<Vec<Suscripcion>>> + Send;

    /// Líneas de la suscripción, por fecha de inicio.
    fn lineas(
        &mut self,
        suscripcion: SuscripcionId,
    ) -> impl Future<Output = AppResult<Vec<LineaSuscripcion>>> + Send;

    /// Línea por su id.
    fn linea(
        &mut self,
        id: LineaSuscripcionId,
    ) -> impl Future<Output = AppResult<Option<LineaSuscripcion>>> + Send;

    /// Guarda una línea nueva.
    fn insertar_linea(
        &mut self,
        l: &LineaSuscripcion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Fija la fecha de fin de la línea.
    fn terminar_linea(
        &mut self,
        id: LineaSuscripcionId,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Mide la métrica de la entidad en el periodo (función `SECURITY DEFINER`, solo números).
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Validacion`] si la métrica no existe.
    fn medir(
        &mut self,
        tenant: TenantId,
        metrica: &Metrica,
        periodo: &Periodo,
    ) -> impl Future<Output = AppResult<u64>> + Send;

    /// Medición guardada de la métrica en el periodo, si ya se tomó.
    fn medicion(
        &mut self,
        tenant: TenantId,
        metrica: &Metrica,
        periodo: &Periodo,
    ) -> impl Future<Output = AppResult<Option<u64>>> + Send;

    /// Guarda la medición (una sola por entidad, métrica y periodo).
    fn guardar_medicion(
        &mut self,
        tenant: TenantId,
        metrica: &Metrica,
        periodo: &Periodo,
        cantidad: u64,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Cortes por id descendente (los más recientes primero), con cursor.
    fn cortes(
        &mut self,
        filtro: &FiltroCortes,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Corte>>> + Send;

    /// Corte por su id; `bloquear` = `FOR UPDATE`.
    fn corte(
        &mut self,
        id: CorteFacturacionId,
        bloquear: bool,
    ) -> impl Future<Output = AppResult<Option<Corte>>> + Send;

    /// Corte no descartado del origen que empieza en `desde` (de una liquidación, el suyo).
    fn corte_vigente(
        &mut self,
        origen: OrigenCorte,
        desde: NaiveDate,
    ) -> impl Future<Output = AppResult<Option<Corte>>> + Send;

    /// Último corte no descartado de la suscripción (el de periodo más reciente).
    fn ultimo_corte(
        &mut self,
        suscripcion: SuscripcionId,
    ) -> impl Future<Output = AppResult<Option<Corte>>> + Send;

    /// Último corte aprobado, facturado o pagado de la suscripción antes de `desde` (para la
    /// aprobación automática).
    fn corte_anterior_cobrado(
        &mut self,
        suscripcion: SuscripcionId,
        desde: NaiveDate,
    ) -> impl Future<Output = AppResult<Option<Corte>>> + Send;

    /// Cortes facturados sin pagar (los que siguen el calendario de mora).
    fn cortes_por_cobrar(&mut self) -> impl Future<Output = AppResult<Vec<Corte>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay un corte vigente del mismo origen y periodo.
    fn insertar_corte(&mut self, c: &Corte) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda los cambios del corte.
    fn actualizar_corte(&mut self, c: &Corte) -> impl Future<Output = AppResult<()>> + Send;

    /// Datos de facturación de una entidad (de su ficha).
    fn cliente_entidad(
        &mut self,
        tenant: TenantId,
    ) -> impl Future<Output = AppResult<Option<ClienteFacturacion>>> + Send;

    /// Datos de facturación de una organización.
    fn cliente_organizacion(
        &mut self,
        organizacion: OrganizacionId,
    ) -> impl Future<Output = AppResult<Option<ClienteFacturacion>>> + Send;

    /// Pasa la liquidación de publicidad de `desde` a `hacia` (no hace nada si ya no está en
    /// `desde`).
    fn avanzar_liquidacion(
        &mut self,
        id: LiquidacionId,
        desde: domain::publicidad::EstadoLiquidacion,
        hacia: domain::publicidad::EstadoLiquidacion,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Liquidación de publicidad con el anunciante.
    fn liquidacion_a_cobrar(
        &mut self,
        id: LiquidacionId,
    ) -> impl Future<Output = AppResult<Option<LiquidacionACobrar>>> + Send;
}
