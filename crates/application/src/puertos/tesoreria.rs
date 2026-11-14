//! Puertos de tesorería, cuentas por pagar, facturas de venta y conciliación (plan 13,
//! ADR 0018). Genéricos: sirven a cualquier entidad (la prueba `tests/fronteras.rs` impide que
//! importen la vertical PH).

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AplicacionPagoId, ArchivoId, CentroCostoId, ComprobanteId, ConceptoRetencionId, ConciliacionId,
    CuentaContableId, CuentaId, CuentaTesoreriaId, EgresoId, EventoPasarelaId, ExtractoBancarioId,
    FacturaProveedorId, FacturaVentaId, MovimientoExtractoId, ObligacionCobroId, PersonaId,
    ReciboCajaId,
    contabilidad::{
        CodigoCuenta, Monto,
        tesoreria::{
            EstadoEgreso, EstadoEventoPasarela, EstadoFacturaProveedor, EstadoFacturaVenta,
            EstadoMovimientoExtracto, EstadoRecibo, FormatoExtracto, ImpuestoRetencion,
            MedioEgreso, MedioRecibo, ModoConciliacion, TipoCuentaBancaria, TipoCuentaTesoreria,
        },
        tributario::ClasificacionTributaria,
    },
};
use rust_decimal::Decimal;

use super::contabilidad::Origen;
use crate::AppResult;

// ---------------------------------------------------------------------------------------------
// Cuentas de tesorería, recibos, egresos y eventos de la pasarela
// ---------------------------------------------------------------------------------------------

/// Cuenta de tesorería con la cuenta contable que la respalda.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuentaTesoreria {
    pub id: CuentaTesoreriaId,
    pub tipo: TipoCuentaTesoreria,
    pub nombre: String,
    pub banco: Option<String>,
    /// `****1234`: el número completo no se guarda.
    pub numero_enmascarado: Option<String>,
    pub tipo_cuenta: Option<TipoCuentaBancaria>,
    pub cuenta_contable_id: CuentaContableId,
    pub destinacion: Option<String>,
    pub activa: bool,
    pub creada_en: DateTime<Utc>,
}

/// Aplicación de un recibo a una factura de venta o a una obligación de cobro de una vertical
/// (las de la PH): exactamente uno de los dos destinos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AplicacionPago {
    pub id: AplicacionPagoId,
    pub recibo_id: ReciboCajaId,
    pub factura_venta_id: Option<FacturaVentaId>,
    /// Obligación de la vertical; su saldo lo reconstruye la vertical sumando aplicaciones.
    pub obligacion_id: Option<ObligacionCobroId>,
    pub valor: Monto,
    pub aplicada_en: DateTime<Utc>,
    /// Solo si se aplicó después del recibo (reclasificación del anticipo).
    pub comprobante_id: Option<ComprobanteId>,
    pub anulada_en: Option<DateTime<Utc>>,
}

/// Recibo de caja con sus aplicaciones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReciboCaja {
    pub id: ReciboCajaId,
    pub cuenta_tesoreria_id: CuentaTesoreriaId,
    /// `None` = pagador no identificado.
    pub persona_id: Option<PersonaId>,
    pub fecha: NaiveDate,
    pub valor: Monto,
    pub medio: MedioRecibo,
    pub referencia: Option<String>,
    pub descripcion: String,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub estado: EstadoRecibo,
    pub comprobante_id: ComprobanteId,
    pub motivo_anulacion: Option<String>,
    pub registrado_por: Option<CuentaId>,
    pub registrado_en: DateTime<Utc>,
    pub aplicaciones: Vec<AplicacionPago>,
}

impl ReciboCaja {
    /// Lo que falta por aplicar (aplicaciones vigentes).
    pub fn sin_aplicar(&self) -> Decimal {
        self.valor.valor()
            - self
                .aplicaciones
                .iter()
                .filter(|a| a.anulada_en.is_none())
                .map(|a| a.valor.valor())
                .sum::<Decimal>()
    }
}

/// Filtro de recibos.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroRecibos {
    pub desde: Option<NaiveDate>,
    pub hasta: Option<NaiveDate>,
    pub persona_id: Option<PersonaId>,
    pub cuenta_tesoreria_id: Option<CuentaTesoreriaId>,
    /// Solo los no identificados.
    pub sin_identificar: bool,
}

/// Gasto pagado directamente por un egreso (caja menor, gastos bancarios, anticipos).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GastoEgreso {
    pub cuenta_id: CuentaContableId,
    pub persona_id: Option<PersonaId>,
    pub centro_costo_id: Option<CentroCostoId>,
    pub valor: Monto,
    pub descripcion: String,
}

/// Egreso con lo que paga.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Egreso {
    pub id: EgresoId,
    pub cuenta_tesoreria_id: CuentaTesoreriaId,
    /// Beneficiario.
    pub persona_id: PersonaId,
    pub fecha: NaiveDate,
    pub valor: Monto,
    pub medio: MedioEgreso,
    /// Número del cheque, de la transferencia o del PSE.
    pub referencia: Option<String>,
    pub descripcion: String,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub estado: EstadoEgreso,
    pub registrado_por: CuentaId,
    pub registrado_en: DateTime<Utc>,
    pub autorizado_por: Option<CuentaId>,
    pub autorizado_en: Option<DateTime<Utc>>,
    pub pagado_en: Option<DateTime<Utc>>,
    pub comprobante_id: Option<ComprobanteId>,
    pub motivo_anulacion: Option<String>,
    /// Facturas de proveedor que paga y cuánto de cada una.
    pub facturas: Vec<(FacturaProveedorId, Monto)>,
    pub gastos: Vec<GastoEgreso>,
}

/// Evento de la pasarela de pagos (append-only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventoPasarela {
    pub id: EventoPasarelaId,
    /// Código de la pasarela (`wompi`, `payu`…).
    pub proveedor: String,
    /// Id del evento en la pasarela: único por proveedor.
    pub evento_externo: String,
    /// Referencia de la intención de pago (la misma en todos sus eventos).
    pub referencia: String,
    pub estado: EstadoEventoPasarela,
    pub valor: Monto,
    pub comision: Monto,
    pub retenciones: Monto,
    pub cuenta_tesoreria_id: CuentaTesoreriaId,
    pub persona_id: Option<PersonaId>,
    pub recibo_id: Option<ReciboCajaId>,
    pub ocurrido_en: DateTime<Utc>,
    pub recibido_en: DateTime<Utc>,
}

/// Cuentas de tesorería, recibos, egresos y eventos de la pasarela.
pub trait RepositorioTesoreria: Send {
    /// Cuentas de tesorería por nombre.
    fn cuentas_tesoreria(&mut self)
    -> impl Future<Output = AppResult<Vec<CuentaTesoreria>>> + Send;

    /// Cuenta de tesorería por id.
    fn cuenta_tesoreria(
        &mut self,
        id: CuentaTesoreriaId,
    ) -> impl Future<Output = AppResult<Option<CuentaTesoreria>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el nombre o la cuenta contable ya se usan.
    fn insertar_cuenta_tesoreria(
        &mut self,
        c: &CuentaTesoreria,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Cambia nombre, destinación y si está activa.
    fn actualizar_cuenta_tesoreria(
        &mut self,
        c: &CuentaTesoreria,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda el recibo con sus aplicaciones.
    fn insertar_recibo(&mut self, r: &ReciboCaja) -> impl Future<Output = AppResult<()>> + Send;

    /// Recibo por id, con sus aplicaciones.
    fn recibo(
        &mut self,
        id: ReciboCajaId,
    ) -> impl Future<Output = AppResult<Option<ReciboCaja>>> + Send;

    /// Recibos por fecha descendente, con sus aplicaciones, a lo sumo `limite`.
    fn recibos(
        &mut self,
        filtro: &FiltroRecibos,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<ReciboCaja>>> + Send;

    /// Marca el recibo anulado con el motivo.
    fn anular_recibo(
        &mut self,
        id: ReciboCajaId,
        motivo: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Asigna el pagador a un recibo que no lo tenía.
    fn identificar_recibo(
        &mut self,
        id: ReciboCajaId,
        persona: PersonaId,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda una aplicación posterior al recibo.
    fn insertar_aplicacion(
        &mut self,
        a: &AplicacionPago,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Marca anuladas las aplicaciones vigentes del recibo.
    fn anular_aplicaciones(
        &mut self,
        recibo: ReciboCajaId,
        en: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda el egreso con sus facturas y gastos.
    fn insertar_egreso(&mut self, e: &Egreso) -> impl Future<Output = AppResult<()>> + Send;

    /// Egreso por id, bloqueado para actualizar (`FOR UPDATE`).
    fn egreso(&mut self, id: EgresoId) -> impl Future<Output = AppResult<Option<Egreso>>> + Send;

    /// Egresos (del estado, si se da) por fecha descendente, a lo sumo `limite`.
    fn egresos(
        &mut self,
        estado: Option<EstadoEgreso>,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<Egreso>>> + Send;

    /// Guarda estado, autorización, pago, comprobante y motivo.
    fn actualizar_egreso(&mut self, e: &Egreso) -> impl Future<Output = AppResult<()>> + Send;

    /// Evento ya recibido (idempotencia).
    fn evento_pasarela(
        &mut self,
        proveedor: &str,
        evento_externo: &str,
    ) -> impl Future<Output = AppResult<Option<EventoPasarela>>> + Send;

    /// Recibo creado por un evento aprobado de la referencia.
    fn recibo_de_referencia(
        &mut self,
        proveedor: &str,
        referencia: &str,
    ) -> impl Future<Output = AppResult<Option<ReciboCajaId>>> + Send;

    /// Guarda el evento.
    fn insertar_evento_pasarela(
        &mut self,
        e: &EventoPasarela,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Facturas de proveedores y de venta
// ---------------------------------------------------------------------------------------------

/// Retención calculada de una línea, con la versión del concepto usada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetencionLinea {
    pub concepto_id: ConceptoRetencionId,
    pub impuesto: ImpuestoRetencion,
    pub base: Monto,
    /// Porcentaje.
    pub tarifa: Decimal,
    pub valor: Monto,
}

/// Línea de una factura de proveedor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineaFacturaProveedor {
    pub descripcion: String,
    /// Cuenta de gasto, costo o activo.
    pub cuenta_id: CuentaContableId,
    pub centro_costo_id: Option<CentroCostoId>,
    pub base: Monto,
    pub iva: Monto,
    /// `false` = el IVA es mayor valor del gasto.
    pub iva_descontable: bool,
    pub retenciones: Vec<RetencionLinea>,
}

/// Factura de proveedor con sus líneas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacturaProveedor {
    pub id: FacturaProveedorId,
    pub persona_id: PersonaId,
    pub prefijo: String,
    pub numero: String,
    pub fecha: NaiveDate,
    pub vencimiento: NaiveDate,
    pub descripcion: String,
    /// Documento que la originó en otro módulo.
    pub origen: Option<Origen>,
    /// CUFE si es electrónica.
    pub cufe: Option<String>,
    pub archivo_id: Option<ArchivoId>,
    pub subtotal: Monto,
    pub iva: Monto,
    pub retenciones: Monto,
    /// Subtotal + IVA − retenciones.
    pub neto_pagar: Monto,
    pub pagado: Monto,
    pub estado: EstadoFacturaProveedor,
    /// Motivo por el que se aceptó un número repetido.
    pub excepcion_duplicado: Option<String>,
    pub excepcion_autorizada_por: Option<CuentaId>,
    pub comprobante_id: Option<ComprobanteId>,
    pub motivo_anulacion: Option<String>,
    pub registrada_por: Option<CuentaId>,
    pub registrada_en: DateTime<Utc>,
    /// Vacío en los listados.
    pub lineas: Vec<LineaFacturaProveedor>,
}

/// Filtro de facturas (de proveedor o de venta).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroFacturas {
    pub persona_id: Option<PersonaId>,
    /// Texto del estado (`causada`, `emitida`…).
    pub estado: Option<String>,
    pub desde: Option<NaiveDate>,
    pub hasta: Option<NaiveDate>,
}

/// Línea de una factura de venta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineaFacturaVenta {
    pub descripcion: String,
    /// Cuenta de ingreso.
    pub cuenta_id: CuentaContableId,
    pub centro_costo_id: Option<CentroCostoId>,
    pub cantidad: Decimal,
    pub valor_unitario: Monto,
    /// Porcentaje de IVA elegido por quien factura.
    pub tarifa_iva: Decimal,
    /// Clasificación tributaria (solo alquiler o servicio y venta llevan IVA).
    pub clasificacion: ClasificacionTributaria,
    pub subtotal: Monto,
    pub iva: Monto,
}

/// Factura de venta con sus líneas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacturaVenta {
    pub id: FacturaVentaId,
    pub persona_id: PersonaId,
    /// Número del comprobante `FV` (al emitir).
    pub numero: Option<u32>,
    pub fecha: NaiveDate,
    pub vencimiento: NaiveDate,
    pub descripcion: String,
    pub origen: Option<Origen>,
    pub subtotal: Monto,
    pub iva: Monto,
    pub total: Monto,
    pub pagado: Monto,
    pub estado: EstadoFacturaVenta,
    pub comprobante_id: Option<ComprobanteId>,
    pub motivo_anulacion: Option<String>,
    pub creada_por: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
    pub emitida_en: Option<DateTime<Utc>>,
    /// Vacío en los listados.
    pub lineas: Vec<LineaFacturaVenta>,
}

/// Retenciones practicadas a un tercero, agrupadas por concepto (certificado).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaCertificado {
    pub impuesto: ImpuestoRetencion,
    pub concepto: String,
    pub tarifa: Decimal,
    pub base: Decimal,
    pub valor: Decimal,
}

/// Nombre y NIT de la entidad (encabezado de certificados y documentos).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosEntidad {
    pub nombre: String,
    pub nit: Option<String>,
}

/// Facturas de proveedores y de venta.
pub trait RepositorioFacturas: Send {
    /// Nombre y NIT de la entidad de la transacción.
    fn datos_entidad(&mut self) -> impl Future<Output = AppResult<DatosEntidad>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el proveedor ya tiene esa factura (sin excepción).
    fn insertar_factura_proveedor(
        &mut self,
        f: &FacturaProveedor,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Factura de proveedor por id, con sus líneas, bloqueada para actualizar.
    fn factura_proveedor(
        &mut self,
        id: FacturaProveedorId,
    ) -> impl Future<Output = AppResult<Option<FacturaProveedor>>> + Send;

    /// Facturas de proveedor (sin líneas) por vencimiento, a lo sumo `limite`.
    fn facturas_proveedor(
        &mut self,
        filtro: &FiltroFacturas,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<FacturaProveedor>>> + Send;

    /// Guarda estado, lo pagado, comprobante y motivo.
    fn actualizar_factura_proveedor(
        &mut self,
        f: &FacturaProveedor,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Retenciones practicadas al tercero en facturas no anuladas entre dos fechas.
    fn retenciones_de_tercero(
        &mut self,
        persona: PersonaId,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<Vec<FilaCertificado>>> + Send;

    /// Guarda la factura de venta en borrador con sus líneas.
    fn insertar_factura_venta(
        &mut self,
        f: &FacturaVenta,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Factura de venta por id, con sus líneas, bloqueada para actualizar.
    fn factura_venta(
        &mut self,
        id: FacturaVentaId,
    ) -> impl Future<Output = AppResult<Option<FacturaVenta>>> + Send;

    /// Facturas de venta (sin líneas) por vencimiento, a lo sumo `limite`.
    fn facturas_venta(
        &mut self,
        filtro: &FiltroFacturas,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<FacturaVenta>>> + Send;

    /// Guarda estado, número, lo pagado, comprobante y motivo.
    fn actualizar_factura_venta(
        &mut self,
        f: &FacturaVenta,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Borra un borrador.
    fn eliminar_factura_venta(
        &mut self,
        id: FacturaVentaId,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Facturas de venta con saldo (emitidas o pagadas en parte) con fecha hasta el corte.
    fn facturas_venta_con_saldo(
        &mut self,
        corte: NaiveDate,
    ) -> impl Future<Output = AppResult<Vec<FacturaVenta>>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Catálogo de retenciones (plataforma)
// ---------------------------------------------------------------------------------------------

/// Versión de un concepto de retención.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptoRetencion {
    pub id: ConceptoRetencionId,
    pub codigo: String,
    pub impuesto: ImpuestoRetencion,
    pub nombre: String,
    pub base_minima_uvt: Decimal,
    /// Porcentaje.
    pub tarifa: Decimal,
    /// Código PUC sugerido para la cuenta de la retención.
    pub cuenta_sugerida: Option<CodigoCuenta>,
    /// Solo ReteICA.
    pub municipio_dane: Option<String>,
    /// Norma de donde sale la tarifa.
    pub fuente: String,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: Option<NaiveDate>,
}

/// Valor de la UVT de un año.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValorUvt {
    pub anio: i32,
    pub valor: Decimal,
    pub fuente: String,
}

/// Lectura del catálogo de retenciones (lo ve cualquier transacción).
pub trait CatalogoRetenciones: Send {
    /// Versión del concepto vigente en la fecha.
    fn concepto_retencion_vigente(
        &mut self,
        codigo: &str,
        fecha: NaiveDate,
    ) -> impl Future<Output = AppResult<Option<ConceptoRetencion>>> + Send;

    /// Conceptos (vigentes en la fecha, o todas las versiones) por impuesto y código.
    fn conceptos_retencion(
        &mut self,
        vigentes_en: Option<NaiveDate>,
    ) -> impl Future<Output = AppResult<Vec<ConceptoRetencion>>> + Send;

    /// Versión de un concepto por id.
    fn concepto_retencion(
        &mut self,
        id: ConceptoRetencionId,
    ) -> impl Future<Output = AppResult<Option<ConceptoRetencion>>> + Send;

    /// UVT del año.
    fn uvt(&mut self, anio: i32) -> impl Future<Output = AppResult<Option<Decimal>>> + Send;

    /// UVT de todos los años.
    fn valores_uvt(&mut self) -> impl Future<Output = AppResult<Vec<ValorUvt>>> + Send;
}

/// Mantenimiento del catálogo (transacción de plataforma).
pub trait CatalogoRetencionesPlataforma: CatalogoRetenciones {
    /// Guarda una versión nueva y cierra la vigencia de la anterior del mismo código el día
    /// antes.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay una versión del código con esa fecha.
    fn insertar_concepto_retencion(
        &mut self,
        c: &ConceptoRetencion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Crea o corrige la UVT del año.
    fn guardar_uvt(&mut self, uvt: &ValorUvt) -> impl Future<Output = AppResult<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Extractos y conciliación
// ---------------------------------------------------------------------------------------------

/// Extracto importado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractoBancario {
    pub id: ExtractoBancarioId,
    pub cuenta_tesoreria_id: CuentaTesoreriaId,
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    pub formato: FormatoExtracto,
    pub version_lector: String,
    pub archivo_id: ArchivoId,
    pub sha256: String,
    pub saldo_inicial: Option<Decimal>,
    pub saldo_final: Option<Decimal>,
    pub cargado_por: Option<CuentaId>,
    pub cargado_en: DateTime<Utc>,
    pub movimientos: u32,
}

/// Movimiento del extracto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovimientoExtracto {
    pub id: MovimientoExtractoId,
    pub extracto_id: ExtractoBancarioId,
    pub cuenta_tesoreria_id: CuentaTesoreriaId,
    pub orden: u32,
    pub fecha: NaiveDate,
    pub descripcion: String,
    pub referencia: Option<String>,
    /// Positivo = entra dinero.
    pub valor: Decimal,
    pub estado: EstadoMovimientoExtracto,
}

/// Línea contabilizada de la cuenta contable del banco.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineaBanco {
    pub comprobante_id: ComprobanteId,
    pub orden: u16,
    pub tipo_codigo: String,
    pub numero: u32,
    pub fecha: NaiveDate,
    pub descripcion: String,
    /// Débito − crédito (positivo = entra dinero).
    pub valor: Decimal,
    /// Conciliación vigente que la incluye.
    pub conciliacion_id: Option<ConciliacionId>,
}

/// Qué hace un registro de conciliación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccionConciliacion {
    Conciliar,
    /// Deshace la conciliación indicada.
    Deshacer(ConciliacionId),
}

/// Registro de conciliación (append-only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conciliacion {
    pub id: ConciliacionId,
    pub cuenta_tesoreria_id: CuentaTesoreriaId,
    pub accion: AccionConciliacion,
    pub modo: ModoConciliacion,
    pub por: Option<CuentaId>,
    pub en: DateTime<Utc>,
    pub movimientos: Vec<MovimientoExtractoId>,
    /// Comprobante y orden de la línea.
    pub lineas: Vec<(ComprobanteId, u16)>,
    /// ¿La deshizo un registro posterior?
    pub deshecha: bool,
}

/// Extractos, movimientos y conciliaciones.
pub trait RepositorioConciliacion: Send {
    /// Extracto con el mismo archivo en la cuenta.
    fn extracto_por_hash(
        &mut self,
        cuenta: CuentaTesoreriaId,
        sha256: &str,
    ) -> impl Future<Output = AppResult<Option<ExtractoBancarioId>>> + Send;

    /// Guarda el extracto y sus movimientos.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el archivo ya se importó en la cuenta.
    fn insertar_extracto(
        &mut self,
        e: &ExtractoBancario,
        movimientos: &[MovimientoExtracto],
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Extractos de la cuenta, más recientes primero.
    fn extractos(
        &mut self,
        cuenta: CuentaTesoreriaId,
    ) -> impl Future<Output = AppResult<Vec<ExtractoBancario>>> + Send;

    /// Movimientos de la cuenta entre dos fechas (del estado, si se da), por fecha y orden.
    fn movimientos_extracto(
        &mut self,
        cuenta: CuentaTesoreriaId,
        desde: NaiveDate,
        hasta: NaiveDate,
        estado: Option<EstadoMovimientoExtracto>,
    ) -> impl Future<Output = AppResult<Vec<MovimientoExtracto>>> + Send;

    /// Movimientos por id (bloqueados para actualizar).
    fn movimientos_por_id(
        &mut self,
        ids: &[MovimientoExtractoId],
    ) -> impl Future<Output = AppResult<Vec<MovimientoExtracto>>> + Send;

    /// Cambia el estado de los movimientos.
    fn cambiar_estado_movimientos(
        &mut self,
        ids: &[MovimientoExtractoId],
        estado: EstadoMovimientoExtracto,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Líneas contabilizadas de la cuenta contable entre dos fechas (o hasta `hasta` si
    /// `desde` es `None`); `solo_pendientes` deja fuera las conciliadas.
    fn lineas_banco(
        &mut self,
        cuenta: CuentaContableId,
        desde: Option<NaiveDate>,
        hasta: NaiveDate,
        solo_pendientes: bool,
    ) -> impl Future<Output = AppResult<Vec<LineaBanco>>> + Send;

    /// Saldo contable (débitos − créditos) de la cuenta hasta la fecha.
    fn saldo_libros(
        &mut self,
        cuenta: CuentaContableId,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<Decimal>> + Send;

    /// Guarda la conciliación con sus partidas.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si la conciliación que deshace ya se deshizo.
    fn insertar_conciliacion(
        &mut self,
        c: &Conciliacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Conciliación por id.
    fn conciliacion(
        &mut self,
        id: ConciliacionId,
    ) -> impl Future<Output = AppResult<Option<Conciliacion>>> + Send;

    /// Conciliaciones de la cuenta, más recientes primero, a lo sumo `limite`.
    fn conciliaciones(
        &mut self,
        cuenta: CuentaTesoreriaId,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<Conciliacion>>> + Send;
}
