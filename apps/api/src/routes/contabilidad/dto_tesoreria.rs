//! DTO de tesorería, cuentas por pagar, facturas de venta y conciliación (plan 13). Los montos
//! viajan como texto decimal (`"1500.50"`), nunca como número de punto flotante.

use application::{
    casos_uso::contabilidad::{
        conciliacion::InformeConciliacion, cuentas_por_pagar::CertificadoRetenciones,
        ventas::FilaCartera,
    },
    puertos::tesoreria::{
        AccionConciliacion, AplicacionPago, ConceptoRetencion, Conciliacion, CuentaTesoreria,
        Egreso, ExtractoBancario, FacturaProveedor, FacturaVenta, FilaCertificado, GastoEgreso,
        LineaBanco, LineaFacturaProveedor, LineaFacturaVenta, MovimientoExtracto, ReciboCaja,
        RetencionLinea, ValorUvt,
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::contabilidad::{
    Monto,
    extracto::{ErrorLectura, ExtractoLeido, MapeoColumnas, MovimientoLeido, SeparadorDecimal},
    tesoreria::{
        EstadoEgreso, EstadoFacturaProveedor, EstadoFacturaVenta, EstadoMovimientoExtracto,
        EstadoRecibo, FormatoExtracto, ImpuestoRetencion, MedioEgreso, MedioRecibo,
        ModoConciliacion, TipoCuentaBancaria, TipoCuentaTesoreria, TramoEdad,
    },
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// ---------------------------------------------------------------------------------------------
// Cuentas de tesorería
// ---------------------------------------------------------------------------------------------

/// Cuenta de tesorería nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaCuentaTesoreriaPeticion {
    #[schema(value_type = crate::esquemas::TipoCuentaTesoreria)]
    pub tipo: TipoCuentaTesoreria,
    #[schema(example = "Cuenta corriente principal")]
    pub nombre: String,
    /// Obligatorio en las de tipo `banco`.
    #[schema(example = "Banco de Bogotá")]
    pub banco: Option<String>,
    /// Número completo: solo se guardan los cuatro últimos dígitos.
    #[schema(example = "123-456789-01")]
    pub numero_cuenta: Option<String>,
    #[schema(value_type = Option<crate::esquemas::TipoCuentaBancaria>)]
    pub tipo_cuenta: Option<TipoCuentaBancaria>,
    /// Código de la cuenta contable de movimiento (naturaleza débito) que la respalda.
    #[schema(example = "111005")]
    pub cuenta_contable: String,
    pub destinacion: Option<String>,
}

/// Cambios de una cuenta de tesorería.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CambioCuentaTesoreriaPeticion {
    pub nombre: String,
    pub destinacion: Option<String>,
    pub activa: bool,
}

/// Cuenta de tesorería.
#[derive(Debug, Serialize, ToSchema)]
pub struct CuentaTesoreriaRespuesta {
    pub id: Uuid,
    #[schema(value_type = crate::esquemas::TipoCuentaTesoreria)]
    pub tipo: TipoCuentaTesoreria,
    pub nombre: String,
    pub banco: Option<String>,
    #[schema(example = "****8901")]
    pub numero_enmascarado: Option<String>,
    #[schema(value_type = Option<crate::esquemas::TipoCuentaBancaria>)]
    pub tipo_cuenta: Option<TipoCuentaBancaria>,
    pub cuenta_contable_id: Uuid,
    pub destinacion: Option<String>,
    pub activa: bool,
    pub creada_en: DateTime<Utc>,
}

impl From<CuentaTesoreria> for CuentaTesoreriaRespuesta {
    fn from(c: CuentaTesoreria) -> Self {
        Self {
            id: c.id.as_uuid(),
            tipo: c.tipo,
            nombre: c.nombre,
            banco: c.banco,
            numero_enmascarado: c.numero_enmascarado,
            tipo_cuenta: c.tipo_cuenta,
            cuenta_contable_id: c.cuenta_contable_id.as_uuid(),
            destinacion: c.destinacion,
            activa: c.activa,
            creada_en: c.creada_en,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Recibos
// ---------------------------------------------------------------------------------------------

/// Aplicación de un recibo a una factura de venta.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AplicacionPeticion {
    pub factura_venta_id: Uuid,
    #[schema(value_type = String, example = "250000")]
    pub valor: Monto,
}

/// Recibo de caja nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoReciboPeticion {
    pub cuenta_tesoreria_id: Uuid,
    /// Pagador; sin él queda como no identificado.
    pub persona_id: Option<Uuid>,
    pub fecha: NaiveDate,
    #[schema(value_type = String, example = "250000")]
    pub valor: Monto,
    #[schema(value_type = crate::esquemas::MedioRecibo)]
    pub medio: MedioRecibo,
    pub referencia: Option<String>,
    #[schema(example = "Pago del alquiler del salón")]
    pub descripcion: String,
    pub soporte_archivo_id: Option<Uuid>,
    /// Lo que no se aplica queda sin aplicar (anticipo o por identificar).
    #[serde(default)]
    pub aplicaciones: Vec<AplicacionPeticion>,
}

/// Aplicación posterior de un recibo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AplicarReciboPeticion {
    pub factura_venta_id: Uuid,
    #[schema(value_type = String, example = "250000")]
    pub valor: Monto,
    /// Fecha contable (por defecto, hoy).
    pub fecha: Option<NaiveDate>,
}

/// Anulación de un documento (se revierte su comprobante).
#[derive(Debug, Deserialize, ToSchema)]
pub struct AnulacionPeticion {
    #[schema(example = "Consignación registrada dos veces")]
    pub motivo: String,
}

/// Aplicación de un recibo: a una factura de venta o a una obligación de la cartera de la PH.
#[derive(Debug, Serialize, ToSchema)]
pub struct AplicacionRespuesta {
    pub id: Uuid,
    pub factura_venta_id: Option<Uuid>,
    /// Obligación de la cartera de la PH (`/copropiedades/{id}/cartera`).
    pub obligacion_id: Option<Uuid>,
    #[schema(value_type = String, example = "250000")]
    pub valor: Decimal,
    pub aplicada_en: DateTime<Utc>,
    pub comprobante_id: Option<Uuid>,
    pub anulada_en: Option<DateTime<Utc>>,
}

impl From<AplicacionPago> for AplicacionRespuesta {
    fn from(a: AplicacionPago) -> Self {
        Self {
            id: a.id.as_uuid(),
            factura_venta_id: a.factura_venta_id.map(|f| f.as_uuid()),
            obligacion_id: a.obligacion_id.map(|o| o.as_uuid()),
            valor: a.valor.valor(),
            aplicada_en: a.aplicada_en,
            comprobante_id: a.comprobante_id.map(|c| c.as_uuid()),
            anulada_en: a.anulada_en,
        }
    }
}

/// Recibo de caja.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReciboRespuesta {
    pub id: Uuid,
    pub cuenta_tesoreria_id: Uuid,
    pub persona_id: Option<Uuid>,
    pub fecha: NaiveDate,
    #[schema(value_type = String, example = "250000")]
    pub valor: Decimal,
    /// Lo que falta por aplicar.
    #[schema(value_type = String, example = "0")]
    pub sin_aplicar: Decimal,
    #[schema(value_type = crate::esquemas::MedioRecibo)]
    pub medio: MedioRecibo,
    pub referencia: Option<String>,
    pub descripcion: String,
    pub soporte_archivo_id: Option<Uuid>,
    #[schema(value_type = crate::esquemas::EstadoRecibo)]
    pub estado: EstadoRecibo,
    pub comprobante_id: Uuid,
    pub motivo_anulacion: Option<String>,
    pub registrado_en: DateTime<Utc>,
    pub aplicaciones: Vec<AplicacionRespuesta>,
}

impl From<ReciboCaja> for ReciboRespuesta {
    fn from(r: ReciboCaja) -> Self {
        Self {
            sin_aplicar: r.sin_aplicar(),
            id: r.id.as_uuid(),
            cuenta_tesoreria_id: r.cuenta_tesoreria_id.as_uuid(),
            persona_id: r.persona_id.map(|p| p.as_uuid()),
            fecha: r.fecha,
            valor: r.valor.valor(),
            medio: r.medio,
            referencia: r.referencia,
            descripcion: r.descripcion,
            soporte_archivo_id: r.soporte_archivo_id.map(|a| a.as_uuid()),
            estado: r.estado,
            comprobante_id: r.comprobante_id.as_uuid(),
            motivo_anulacion: r.motivo_anulacion,
            registrado_en: r.registrado_en,
            aplicaciones: r.aplicaciones.into_iter().map(Into::into).collect(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Egresos
// ---------------------------------------------------------------------------------------------

/// Factura que paga un egreso.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PagoFacturaDto {
    pub factura_proveedor_id: Uuid,
    #[schema(value_type = String, example = "1121500")]
    pub valor: Monto,
}

/// Gasto directo de un egreso (caja menor, gastos bancarios, anticipos).
#[derive(Debug, Deserialize, ToSchema)]
pub struct GastoPeticion {
    #[schema(example = "530505")]
    pub cuenta: String,
    pub persona_id: Option<Uuid>,
    /// Código del centro de costo.
    pub centro_costo: Option<String>,
    #[schema(value_type = String, example = "15000")]
    pub valor: Monto,
    pub descripcion: String,
}

/// Egreso nuevo (queda por autorizar).
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoEgresoPeticion {
    pub cuenta_tesoreria_id: Uuid,
    /// Beneficiario.
    pub persona_id: Uuid,
    pub fecha: NaiveDate,
    #[schema(value_type = crate::esquemas::MedioEgreso)]
    pub medio: MedioEgreso,
    /// Obligatoria con cheque (su número).
    pub referencia: Option<String>,
    pub descripcion: String,
    pub soporte_archivo_id: Option<Uuid>,
    #[serde(default)]
    pub facturas: Vec<PagoFacturaDto>,
    #[serde(default)]
    pub gastos: Vec<GastoPeticion>,
}

/// Pago de un egreso autorizado.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct PagoEgresoPeticion {
    /// Fecha del pago (por defecto, la del egreso).
    pub fecha: Option<NaiveDate>,
}

/// Gasto directo de un egreso.
#[derive(Debug, Serialize, ToSchema)]
pub struct GastoRespuesta {
    pub cuenta_id: Uuid,
    pub persona_id: Option<Uuid>,
    pub centro_costo_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub valor: Decimal,
    pub descripcion: String,
}

impl From<GastoEgreso> for GastoRespuesta {
    fn from(g: GastoEgreso) -> Self {
        Self {
            cuenta_id: g.cuenta_id.as_uuid(),
            persona_id: g.persona_id.map(|p| p.as_uuid()),
            centro_costo_id: g.centro_costo_id.map(|c| c.as_uuid()),
            valor: g.valor.valor(),
            descripcion: g.descripcion,
        }
    }
}

/// Egreso.
#[derive(Debug, Serialize, ToSchema)]
pub struct EgresoRespuesta {
    pub id: Uuid,
    pub cuenta_tesoreria_id: Uuid,
    pub persona_id: Uuid,
    pub fecha: NaiveDate,
    #[schema(value_type = String, example = "1126500")]
    pub valor: Decimal,
    #[schema(value_type = crate::esquemas::MedioEgreso)]
    pub medio: MedioEgreso,
    pub referencia: Option<String>,
    pub descripcion: String,
    pub soporte_archivo_id: Option<Uuid>,
    #[schema(value_type = crate::esquemas::EstadoEgreso)]
    pub estado: EstadoEgreso,
    pub registrado_por: Uuid,
    pub registrado_en: DateTime<Utc>,
    pub autorizado_por: Option<Uuid>,
    pub autorizado_en: Option<DateTime<Utc>>,
    pub pagado_en: Option<DateTime<Utc>>,
    pub comprobante_id: Option<Uuid>,
    pub motivo_anulacion: Option<String>,
    pub facturas: Vec<PagoFacturaDto>,
    pub gastos: Vec<GastoRespuesta>,
}

impl From<Egreso> for EgresoRespuesta {
    fn from(e: Egreso) -> Self {
        Self {
            id: e.id.as_uuid(),
            cuenta_tesoreria_id: e.cuenta_tesoreria_id.as_uuid(),
            persona_id: e.persona_id.as_uuid(),
            fecha: e.fecha,
            valor: e.valor.valor(),
            medio: e.medio,
            referencia: e.referencia,
            descripcion: e.descripcion,
            soporte_archivo_id: e.soporte_archivo_id.map(|a| a.as_uuid()),
            estado: e.estado,
            registrado_por: e.registrado_por.as_uuid(),
            registrado_en: e.registrado_en,
            autorizado_por: e.autorizado_por.map(|c| c.as_uuid()),
            autorizado_en: e.autorizado_en,
            pagado_en: e.pagado_en,
            comprobante_id: e.comprobante_id.map(|c| c.as_uuid()),
            motivo_anulacion: e.motivo_anulacion,
            facturas: e
                .facturas
                .into_iter()
                .map(|(f, v)| PagoFacturaDto {
                    factura_proveedor_id: f.as_uuid(),
                    valor: v,
                })
                .collect(),
            gastos: e.gastos.into_iter().map(Into::into).collect(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Catálogo de retenciones
// ---------------------------------------------------------------------------------------------

/// Versión de un concepto de retención.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConceptoRetencionRespuesta {
    pub id: Uuid,
    #[schema(example = "servicios_generales")]
    pub codigo: String,
    #[schema(value_type = crate::esquemas::ImpuestoRetencion)]
    pub impuesto: ImpuestoRetencion,
    pub nombre: String,
    #[schema(value_type = String, example = "4")]
    pub base_minima_uvt: Decimal,
    /// Porcentaje.
    #[schema(value_type = String, example = "4")]
    pub tarifa: Decimal,
    pub cuenta_sugerida: Option<String>,
    pub municipio_dane: Option<String>,
    pub fuente: String,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: Option<NaiveDate>,
}

impl From<ConceptoRetencion> for ConceptoRetencionRespuesta {
    fn from(c: ConceptoRetencion) -> Self {
        Self {
            id: c.id.as_uuid(),
            codigo: c.codigo,
            impuesto: c.impuesto,
            nombre: c.nombre,
            base_minima_uvt: c.base_minima_uvt,
            tarifa: c.tarifa,
            cuenta_sugerida: c.cuenta_sugerida.map(|c| c.as_str().to_owned()),
            municipio_dane: c.municipio_dane,
            fuente: c.fuente,
            vigente_desde: c.vigente_desde,
            vigente_hasta: c.vigente_hasta,
        }
    }
}

/// Versión nueva de un concepto de retención (la anterior del código queda vigente hasta el día
/// antes).
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoConceptoRetencionPeticion {
    #[schema(example = "servicios_generales")]
    pub codigo: String,
    #[schema(value_type = crate::esquemas::ImpuestoRetencion)]
    pub impuesto: ImpuestoRetencion,
    pub nombre: String,
    #[schema(value_type = String, example = "4")]
    pub base_minima_uvt: Decimal,
    /// Porcentaje.
    #[schema(value_type = String, example = "4")]
    pub tarifa: Decimal,
    #[schema(example = "236525")]
    pub cuenta_sugerida: Option<String>,
    /// Solo ReteICA.
    pub municipio_dane: Option<String>,
    /// Norma de donde sale la tarifa.
    pub fuente: String,
    pub vigente_desde: NaiveDate,
}

/// Valor de la UVT de un año.
#[derive(Debug, Serialize, ToSchema)]
pub struct UvtRespuesta {
    pub anio: i32,
    #[schema(value_type = String, example = "52374")]
    pub valor: Decimal,
    pub fuente: String,
}

impl From<ValorUvt> for UvtRespuesta {
    fn from(u: ValorUvt) -> Self {
        Self {
            anio: u.anio,
            valor: u.valor,
            fuente: u.fuente,
        }
    }
}

/// UVT de un año.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UvtPeticion {
    #[schema(value_type = String, example = "52374")]
    pub valor: Decimal,
    #[schema(example = "Resolución DIAN 000238 de 2025")]
    pub fuente: String,
}

// ---------------------------------------------------------------------------------------------
// Facturas de proveedores
// ---------------------------------------------------------------------------------------------

/// Línea de una factura de proveedor nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct LineaFacturaProveedorPeticion {
    pub descripcion: String,
    /// Código de la cuenta de gasto, costo o activo.
    #[schema(example = "513525")]
    pub cuenta: String,
    pub centro_costo: Option<String>,
    #[schema(value_type = String, example = "1000000")]
    pub base: Monto,
    #[schema(value_type = String, example = "190000")]
    pub iva: Monto,
    /// `false` = el IVA es mayor valor del gasto.
    #[serde(default = "verdadero")]
    pub iva_descontable: bool,
    /// Códigos de los conceptos de retención que aplican (uno por impuesto como máximo).
    #[serde(default)]
    pub retenciones: Vec<String>,
}

fn verdadero() -> bool {
    true
}

/// Documento que originó una factura en otro módulo.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct OrigenDto {
    #[schema(example = "mantenimiento.orden_trabajo")]
    pub tipo: String,
    pub id: Uuid,
}

/// Factura de proveedor nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaFacturaProveedorPeticion {
    pub persona_id: Uuid,
    #[serde(default)]
    #[schema(example = "FE")]
    pub prefijo: String,
    #[schema(example = "1024")]
    pub numero: String,
    pub fecha: NaiveDate,
    pub vencimiento: NaiveDate,
    pub descripcion: String,
    pub origen: Option<OrigenDto>,
    /// CUFE si es electrónica (96 caracteres hexadecimales).
    pub cufe: Option<String>,
    pub archivo_id: Option<Uuid>,
    pub lineas: Vec<LineaFacturaProveedorPeticion>,
    /// Motivo para aceptar un número que el proveedor ya usó (exige
    /// `tesoreria.egresos.autorizar`).
    pub excepcion_duplicado: Option<String>,
}

/// Retención calculada.
#[derive(Debug, Serialize, ToSchema)]
pub struct RetencionRespuesta {
    pub concepto_id: Uuid,
    #[schema(value_type = crate::esquemas::ImpuestoRetencion)]
    pub impuesto: ImpuestoRetencion,
    #[schema(value_type = String)]
    pub base: Decimal,
    #[schema(value_type = String)]
    pub tarifa: Decimal,
    #[schema(value_type = String)]
    pub valor: Decimal,
}

impl From<RetencionLinea> for RetencionRespuesta {
    fn from(r: RetencionLinea) -> Self {
        Self {
            concepto_id: r.concepto_id.as_uuid(),
            impuesto: r.impuesto,
            base: r.base.valor(),
            tarifa: r.tarifa,
            valor: r.valor.valor(),
        }
    }
}

/// Línea de una factura de proveedor.
#[derive(Debug, Serialize, ToSchema)]
pub struct LineaFacturaProveedorRespuesta {
    pub descripcion: String,
    pub cuenta_id: Uuid,
    pub centro_costo_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub base: Decimal,
    #[schema(value_type = String)]
    pub iva: Decimal,
    pub iva_descontable: bool,
    pub retenciones: Vec<RetencionRespuesta>,
}

impl From<LineaFacturaProveedor> for LineaFacturaProveedorRespuesta {
    fn from(l: LineaFacturaProveedor) -> Self {
        Self {
            descripcion: l.descripcion,
            cuenta_id: l.cuenta_id.as_uuid(),
            centro_costo_id: l.centro_costo_id.map(|c| c.as_uuid()),
            base: l.base.valor(),
            iva: l.iva.valor(),
            iva_descontable: l.iva_descontable,
            retenciones: l.retenciones.into_iter().map(Into::into).collect(),
        }
    }
}

/// Factura de proveedor (las líneas solo en el detalle).
#[derive(Debug, Serialize, ToSchema)]
pub struct FacturaProveedorRespuesta {
    pub id: Uuid,
    pub persona_id: Uuid,
    pub prefijo: String,
    pub numero: String,
    pub fecha: NaiveDate,
    pub vencimiento: NaiveDate,
    pub descripcion: String,
    pub origen: Option<OrigenDto>,
    pub cufe: Option<String>,
    pub archivo_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub subtotal: Decimal,
    #[schema(value_type = String)]
    pub iva: Decimal,
    #[schema(value_type = String)]
    pub retenciones: Decimal,
    #[schema(value_type = String)]
    pub neto_pagar: Decimal,
    #[schema(value_type = String)]
    pub pagado: Decimal,
    #[schema(value_type = crate::esquemas::EstadoFacturaProveedor)]
    pub estado: EstadoFacturaProveedor,
    pub excepcion_duplicado: Option<String>,
    pub comprobante_id: Option<Uuid>,
    pub motivo_anulacion: Option<String>,
    pub registrada_en: DateTime<Utc>,
    pub lineas: Vec<LineaFacturaProveedorRespuesta>,
}

impl From<FacturaProveedor> for FacturaProveedorRespuesta {
    fn from(f: FacturaProveedor) -> Self {
        Self {
            id: f.id.as_uuid(),
            persona_id: f.persona_id.as_uuid(),
            prefijo: f.prefijo,
            numero: f.numero,
            fecha: f.fecha,
            vencimiento: f.vencimiento,
            descripcion: f.descripcion,
            origen: f.origen.map(|o| OrigenDto {
                tipo: o.tipo,
                id: o.id,
            }),
            cufe: f.cufe,
            archivo_id: f.archivo_id.map(|a| a.as_uuid()),
            subtotal: f.subtotal.valor(),
            iva: f.iva.valor(),
            retenciones: f.retenciones.valor(),
            neto_pagar: f.neto_pagar.valor(),
            pagado: f.pagado.valor(),
            estado: f.estado,
            excepcion_duplicado: f.excepcion_duplicado,
            comprobante_id: f.comprobante_id.map(|c| c.as_uuid()),
            motivo_anulacion: f.motivo_anulacion,
            registrada_en: f.registrada_en,
            lineas: f.lineas.into_iter().map(Into::into).collect(),
        }
    }
}

/// Fila de un certificado de retenciones.
#[derive(Debug, Serialize, ToSchema)]
pub struct FilaCertificadoRespuesta {
    #[schema(value_type = crate::esquemas::ImpuestoRetencion)]
    pub impuesto: ImpuestoRetencion,
    pub concepto: String,
    #[schema(value_type = String)]
    pub tarifa: Decimal,
    #[schema(value_type = String)]
    pub base: Decimal,
    #[schema(value_type = String)]
    pub valor: Decimal,
}

impl From<FilaCertificado> for FilaCertificadoRespuesta {
    fn from(f: FilaCertificado) -> Self {
        Self {
            impuesto: f.impuesto,
            concepto: f.concepto,
            tarifa: f.tarifa,
            base: f.base,
            valor: f.valor,
        }
    }
}

/// Certificado de retenciones de un tercero.
#[derive(Debug, Serialize, ToSchema)]
pub struct CertificadoRespuesta {
    pub entidad: String,
    pub entidad_nit: Option<String>,
    pub tercero: String,
    /// Dato personal si el tercero es persona natural.
    pub tercero_documento: Option<String>,
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    pub filas: Vec<FilaCertificadoRespuesta>,
    #[schema(value_type = String)]
    pub total_base: Decimal,
    #[schema(value_type = String)]
    pub total_retenido: Decimal,
}

impl From<CertificadoRetenciones> for CertificadoRespuesta {
    fn from(c: CertificadoRetenciones) -> Self {
        Self {
            entidad: c.entidad,
            entidad_nit: c.entidad_nit,
            tercero: c.tercero,
            tercero_documento: c.tercero_documento,
            desde: c.desde,
            hasta: c.hasta,
            filas: c.filas.into_iter().map(Into::into).collect(),
            total_base: c.total_base,
            total_retenido: c.total_retenido,
        }
    }
}

/// A dónde se envió el certificado.
#[derive(Debug, Serialize, ToSchema)]
pub struct EnvioCertificadoRespuesta {
    /// Correo del tercero (dato personal).
    pub para: String,
}

// ---------------------------------------------------------------------------------------------
// Facturas de venta
// ---------------------------------------------------------------------------------------------

/// Línea de una factura de venta nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct LineaVentaPeticion {
    pub descripcion: String,
    /// Código de la cuenta de ingreso.
    #[schema(example = "415505")]
    pub cuenta: String,
    pub centro_costo: Option<String>,
    #[schema(value_type = String, example = "1")]
    pub cantidad: Decimal,
    #[schema(value_type = String, example = "350000")]
    pub valor_unitario: Monto,
    /// Porcentaje de IVA que elige quien factura (0 si no aplica).
    #[serde(default)]
    #[schema(value_type = String, example = "0")]
    pub tarifa_iva: Decimal,
}

/// Factura de venta nueva (queda en borrador).
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaFacturaVentaPeticion {
    pub persona_id: Uuid,
    pub fecha: NaiveDate,
    pub vencimiento: NaiveDate,
    pub descripcion: String,
    pub origen: Option<OrigenDto>,
    pub lineas: Vec<LineaVentaPeticion>,
}

/// Línea de una factura de venta.
#[derive(Debug, Serialize, ToSchema)]
pub struct LineaVentaRespuesta {
    pub descripcion: String,
    pub cuenta_id: Uuid,
    pub centro_costo_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub cantidad: Decimal,
    #[schema(value_type = String)]
    pub valor_unitario: Decimal,
    #[schema(value_type = String)]
    pub tarifa_iva: Decimal,
    #[schema(value_type = String)]
    pub subtotal: Decimal,
    #[schema(value_type = String)]
    pub iva: Decimal,
}

impl From<LineaFacturaVenta> for LineaVentaRespuesta {
    fn from(l: LineaFacturaVenta) -> Self {
        Self {
            descripcion: l.descripcion,
            cuenta_id: l.cuenta_id.as_uuid(),
            centro_costo_id: l.centro_costo_id.map(|c| c.as_uuid()),
            cantidad: l.cantidad,
            valor_unitario: l.valor_unitario.valor(),
            tarifa_iva: l.tarifa_iva,
            subtotal: l.subtotal.valor(),
            iva: l.iva.valor(),
        }
    }
}

/// Factura de venta (las líneas solo en el detalle).
#[derive(Debug, Serialize, ToSchema)]
pub struct FacturaVentaRespuesta {
    pub id: Uuid,
    pub persona_id: Uuid,
    /// Número de su comprobante `FV` (al emitir).
    pub numero: Option<u32>,
    pub fecha: NaiveDate,
    pub vencimiento: NaiveDate,
    pub descripcion: String,
    pub origen: Option<OrigenDto>,
    #[schema(value_type = String)]
    pub subtotal: Decimal,
    #[schema(value_type = String)]
    pub iva: Decimal,
    #[schema(value_type = String)]
    pub total: Decimal,
    #[schema(value_type = String)]
    pub pagado: Decimal,
    #[schema(value_type = crate::esquemas::EstadoFacturaVenta)]
    pub estado: EstadoFacturaVenta,
    pub comprobante_id: Option<Uuid>,
    pub motivo_anulacion: Option<String>,
    pub creada_en: DateTime<Utc>,
    pub emitida_en: Option<DateTime<Utc>>,
    pub lineas: Vec<LineaVentaRespuesta>,
}

impl From<FacturaVenta> for FacturaVentaRespuesta {
    fn from(f: FacturaVenta) -> Self {
        Self {
            id: f.id.as_uuid(),
            persona_id: f.persona_id.as_uuid(),
            numero: f.numero,
            fecha: f.fecha,
            vencimiento: f.vencimiento,
            descripcion: f.descripcion,
            origen: f.origen.map(|o| OrigenDto {
                tipo: o.tipo,
                id: o.id,
            }),
            subtotal: f.subtotal.valor(),
            iva: f.iva.valor(),
            total: f.total.valor(),
            pagado: f.pagado.valor(),
            estado: f.estado,
            comprobante_id: f.comprobante_id.map(|c| c.as_uuid()),
            motivo_anulacion: f.motivo_anulacion,
            creada_en: f.creada_en,
            emitida_en: f.emitida_en,
            lineas: f.lineas.into_iter().map(Into::into).collect(),
        }
    }
}

/// Saldo de un tramo de edad.
#[derive(Debug, Serialize, ToSchema)]
pub struct TramoRespuesta {
    #[schema(value_type = crate::esquemas::TramoEdad)]
    pub tramo: TramoEdad,
    #[schema(value_type = String)]
    pub saldo: Decimal,
}

/// Cartera de un cliente por edades.
#[derive(Debug, Serialize, ToSchema)]
pub struct FilaCarteraRespuesta {
    pub persona_id: Uuid,
    pub nombre: String,
    pub tramos: Vec<TramoRespuesta>,
    #[schema(value_type = String)]
    pub total: Decimal,
}

impl From<FilaCartera> for FilaCarteraRespuesta {
    fn from(f: FilaCartera) -> Self {
        Self {
            persona_id: f.persona_id.as_uuid(),
            nombre: f.nombre,
            tramos: f
                .tramos
                .into_iter()
                .map(|(tramo, saldo)| TramoRespuesta { tramo, saldo })
                .collect(),
            total: f.total,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Extractos y conciliación
// ---------------------------------------------------------------------------------------------

/// Cómo leer la planilla de un banco.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MapeoColumnasDto {
    /// Fila (desde 1) con los encabezados.
    #[serde(default = "uno")]
    pub fila_encabezado: usize,
    #[schema(example = "Fecha")]
    pub fecha: String,
    #[schema(example = "%d/%m/%Y")]
    pub formato_fecha: String,
    #[schema(example = "Descripción")]
    pub descripcion: String,
    pub referencia: Option<String>,
    /// Columna con signo (positivo = entra dinero).
    pub valor: Option<String>,
    /// Columna de salidas (sin signo).
    pub debito: Option<String>,
    /// Columna de entradas (sin signo).
    pub credito: Option<String>,
    #[serde(default)]
    #[schema(value_type = crate::esquemas::SeparadorDecimal)]
    pub separador_decimal: SeparadorDecimal,
}

fn uno() -> usize {
    1
}

impl From<MapeoColumnasDto> for MapeoColumnas {
    fn from(m: MapeoColumnasDto) -> Self {
        Self {
            fila_encabezado: m.fila_encabezado,
            fecha: m.fecha,
            formato_fecha: m.formato_fecha,
            descripcion: m.descripcion,
            referencia: m.referencia,
            valor: m.valor,
            debito: m.debito,
            credito: m.credito,
            separador_decimal: m.separador_decimal,
        }
    }
}

/// Extracto a leer o importar. El archivo va en Base64.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ExtractoPeticion {
    #[schema(example = "extracto-marzo.csv")]
    pub nombre_archivo: String,
    /// Contenido del archivo en Base64 (CSV, XLSX o el texto Asobancaria).
    pub archivo_base64: String,
    #[schema(value_type = crate::esquemas::FormatoExtracto)]
    pub formato: FormatoExtracto,
    /// Obligatorio con `planilla`.
    pub mapeo: Option<MapeoColumnasDto>,
    #[schema(value_type = Option<String>)]
    pub saldo_inicial: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub saldo_final: Option<Decimal>,
}

/// Movimiento leído (vista previa).
#[derive(Debug, Serialize, ToSchema)]
pub struct MovimientoLeidoRespuesta {
    pub fecha: NaiveDate,
    pub descripcion: String,
    pub referencia: Option<String>,
    /// Positivo = entra dinero.
    #[schema(value_type = String)]
    pub valor: Decimal,
}

impl From<MovimientoLeido> for MovimientoLeidoRespuesta {
    fn from(m: MovimientoLeido) -> Self {
        Self {
            fecha: m.fecha,
            descripcion: m.descripcion,
            referencia: m.referencia,
            valor: m.valor,
        }
    }
}

/// Problema de una fila del extracto.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorLecturaRespuesta {
    pub fila: usize,
    pub mensaje: String,
}

impl From<ErrorLectura> for ErrorLecturaRespuesta {
    fn from(e: ErrorLectura) -> Self {
        Self {
            fila: e.fila,
            mensaje: e.mensaje,
        }
    }
}

/// Vista previa del extracto.
#[derive(Debug, Serialize, ToSchema)]
pub struct VistaPreviaExtractoRespuesta {
    pub movimientos: Vec<MovimientoLeidoRespuesta>,
    pub errores: Vec<ErrorLecturaRespuesta>,
}

impl From<ExtractoLeido> for VistaPreviaExtractoRespuesta {
    fn from(e: ExtractoLeido) -> Self {
        Self {
            movimientos: e.movimientos.into_iter().map(Into::into).collect(),
            errores: e.errores.into_iter().map(Into::into).collect(),
        }
    }
}

/// Extracto importado.
#[derive(Debug, Serialize, ToSchema)]
pub struct ExtractoRespuesta {
    pub id: Uuid,
    pub cuenta_tesoreria_id: Uuid,
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    #[schema(value_type = crate::esquemas::FormatoExtracto)]
    pub formato: FormatoExtracto,
    #[schema(example = "planilla-1")]
    pub version_lector: String,
    pub archivo_id: Uuid,
    #[schema(value_type = Option<String>)]
    pub saldo_inicial: Option<Decimal>,
    #[schema(value_type = Option<String>)]
    pub saldo_final: Option<Decimal>,
    pub cargado_en: DateTime<Utc>,
    pub movimientos: u32,
}

impl From<ExtractoBancario> for ExtractoRespuesta {
    fn from(e: ExtractoBancario) -> Self {
        Self {
            id: e.id.as_uuid(),
            cuenta_tesoreria_id: e.cuenta_tesoreria_id.as_uuid(),
            desde: e.desde,
            hasta: e.hasta,
            formato: e.formato,
            version_lector: e.version_lector,
            archivo_id: e.archivo_id.as_uuid(),
            saldo_inicial: e.saldo_inicial,
            saldo_final: e.saldo_final,
            cargado_en: e.cargado_en,
            movimientos: e.movimientos,
        }
    }
}

/// Movimiento del extracto.
#[derive(Debug, Serialize, ToSchema)]
pub struct MovimientoExtractoRespuesta {
    pub id: Uuid,
    pub extracto_id: Uuid,
    pub orden: u32,
    pub fecha: NaiveDate,
    pub descripcion: String,
    pub referencia: Option<String>,
    /// Positivo = entra dinero.
    #[schema(value_type = String)]
    pub valor: Decimal,
    #[schema(value_type = crate::esquemas::EstadoMovimientoExtracto)]
    pub estado: EstadoMovimientoExtracto,
}

impl From<MovimientoExtracto> for MovimientoExtractoRespuesta {
    fn from(m: MovimientoExtracto) -> Self {
        Self {
            id: m.id.as_uuid(),
            extracto_id: m.extracto_id.as_uuid(),
            orden: m.orden,
            fecha: m.fecha,
            descripcion: m.descripcion,
            referencia: m.referencia,
            valor: m.valor,
            estado: m.estado,
        }
    }
}

/// Línea de libros de la cuenta bancaria.
#[derive(Debug, Serialize, ToSchema)]
pub struct LineaLibrosRespuesta {
    pub comprobante_id: Uuid,
    pub orden: u16,
    pub tipo: String,
    pub numero: u32,
    pub fecha: NaiveDate,
    pub descripcion: String,
    /// Débito − crédito (positivo = entra dinero).
    #[schema(value_type = String)]
    pub valor: Decimal,
    pub conciliacion_id: Option<Uuid>,
}

impl From<LineaBanco> for LineaLibrosRespuesta {
    fn from(l: LineaBanco) -> Self {
        Self {
            comprobante_id: l.comprobante_id.as_uuid(),
            orden: l.orden,
            tipo: l.tipo_codigo,
            numero: l.numero,
            fecha: l.fecha,
            descripcion: l.descripcion,
            valor: l.valor,
            conciliacion_id: l.conciliacion_id.map(|c| c.as_uuid()),
        }
    }
}

/// Línea de un comprobante (comprobante + orden).
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct LineaRefDto {
    pub comprobante_id: Uuid,
    pub orden: u16,
}

/// Conciliación manual.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ConciliacionManualPeticion {
    pub movimientos: Vec<Uuid>,
    pub lineas: Vec<LineaRefDto>,
}

/// Conciliación automática.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ConciliacionAutomaticaPeticion {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    /// Días de diferencia admitidos entre el extracto y los libros (máximo 10).
    #[serde(default = "tres")]
    pub tolerancia_dias: u32,
}

fn tres() -> u32 {
    3
}

/// Qué hace un registro de conciliación.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccionConciliacionDto {
    Conciliar,
    Deshacer,
}

/// Registro de conciliación.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConciliacionRespuesta {
    pub id: Uuid,
    pub cuenta_tesoreria_id: Uuid,
    pub accion: AccionConciliacionDto,
    /// Conciliación que deshace (solo en `deshacer`).
    pub deshace_a: Option<Uuid>,
    #[schema(value_type = crate::esquemas::ModoConciliacion)]
    pub modo: ModoConciliacion,
    pub por: Option<Uuid>,
    pub en: DateTime<Utc>,
    pub movimientos: Vec<Uuid>,
    pub lineas: Vec<LineaRefDto>,
    pub deshecha: bool,
}

impl From<Conciliacion> for ConciliacionRespuesta {
    fn from(c: Conciliacion) -> Self {
        let (accion, deshace_a) = match c.accion {
            AccionConciliacion::Conciliar => (AccionConciliacionDto::Conciliar, None),
            AccionConciliacion::Deshacer(id) => {
                (AccionConciliacionDto::Deshacer, Some(id.as_uuid()))
            }
        };
        Self {
            id: c.id.as_uuid(),
            cuenta_tesoreria_id: c.cuenta_tesoreria_id.as_uuid(),
            accion,
            deshace_a,
            modo: c.modo,
            por: c.por.map(|p| p.as_uuid()),
            en: c.en,
            movimientos: c.movimientos.into_iter().map(|m| m.as_uuid()).collect(),
            lineas: c
                .lineas
                .into_iter()
                .map(|(comprobante, orden)| LineaRefDto {
                    comprobante_id: comprobante.as_uuid(),
                    orden,
                })
                .collect(),
            deshecha: c.deshecha,
        }
    }
}

/// Marcar o desmarcar una partida conciliatoria.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PartidaConciliatoriaPeticion {
    pub partida: bool,
}

/// Recibo no identificado desde un crédito del extracto.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReciboNoIdentificadoPeticion {
    #[schema(example = "Consignación por identificar")]
    pub descripcion: String,
}

/// Informe de conciliación.
#[derive(Debug, Serialize, ToSchema)]
pub struct InformeConciliacionRespuesta {
    pub corte: NaiveDate,
    #[schema(value_type = Option<String>)]
    pub saldo_extracto: Option<Decimal>,
    #[schema(value_type = String)]
    pub saldo_libros: Decimal,
    /// En libros y no en el extracto (cheques y consignaciones en tránsito).
    pub libros_pendientes: Vec<LineaLibrosRespuesta>,
    /// En el extracto y no en libros.
    pub extracto_pendientes: Vec<MovimientoExtractoRespuesta>,
    #[schema(value_type = String)]
    pub saldo_conciliado: Decimal,
    /// Saldo del extracto − saldo conciliado (cero si cuadra).
    #[schema(value_type = Option<String>)]
    pub diferencia: Option<Decimal>,
}

impl From<InformeConciliacion> for InformeConciliacionRespuesta {
    fn from(i: InformeConciliacion) -> Self {
        Self {
            corte: i.corte,
            saldo_extracto: i.saldo_extracto,
            saldo_libros: i.saldo_libros,
            libros_pendientes: i.libros_pendientes.into_iter().map(Into::into).collect(),
            extracto_pendientes: i.extracto_pendientes.into_iter().map(Into::into).collect(),
            saldo_conciliado: i.saldo_conciliado,
            diferencia: i.diferencia,
        }
    }
}
