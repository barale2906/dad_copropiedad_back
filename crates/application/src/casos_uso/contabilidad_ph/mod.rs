//! Vertical contable de la propiedad horizontal (plan 14 fases 1–4, ADR 0019; módulo
//! [`Modulo::ContabilidadPh`], que exige además [`Modulo::Contabilidad`]).
//!
//! - [`configuracion`]: módulos de contribución, conceptos de cobro, políticas de cartera y
//!   datos de pago de cada bien privado.
//! - [`presupuesto`]: presupuesto anual por versiones, aprobación con la pregunta de la asamblea
//!   y ejecución presupuestal contra el mayor.
//! - [`liquidacion`]: cuotas extraordinarias, liquidación mensual de expensas (vista previa,
//!   contabilización, anulación), cuentas de cobro en PDF y su envío.
//! - [`recaudo`]: recaudo por bien con imputación y pronto pago, recaudo desde el extracto por
//!   referencia, anticipos, cobros manuales, notas crédito, cartera inicial, estado de cuenta,
//!   cartera por bien y cuadre contra el mayor.
//! - [`mora`]: tasas de referencia, causación de intereses de mora, acuerdos de pago, cartera
//!   por edades y certificados (paz y salvo y deuda) verificables.
//! - [`portal`]: lo que ve el propietario o pagador de sus bienes (estado de cuenta, cuentas de
//!   cobro y su paz y salvo).
//! - [`integraciones`]: morosos de las asambleas y restricción de reservas por mora desde la
//!   cartera.
//! - [`informes`]: estado del fondo de imprevistos, flujo de caja, proyecto de presupuesto y el
//!   paquete en PDF para la asamblea.
//!
//! Se monta sobre el núcleo contable y la tesorería (ADR 0017, 0018): los documentos se
//! entregan al contabilizador con los eventos de [`eventos`] (el contador define sus reglas) y
//! el recaudo es un recibo de caja con aplicaciones a obligaciones. El núcleo no conoce esta
//! vertical.

pub mod configuracion;
pub mod informes;
pub mod integraciones;
pub mod liquidacion;
pub mod mora;
pub mod portal;
pub mod presupuesto;
pub mod recaudo;

use domain::{Modulo, Permiso, contabilidad::Monto};
use rust_decimal::Decimal;

use super::contabilidad::TxTesoreria;
use crate::{
    AppError, AppResult, Contexto,
    casos_uso::modulos::exigir_modulo_en,
    puertos::{
        RegistroOutbox, Reloj, RepositorioCartera, RepositorioTickets, RepositorioTitulares,
        UnidadDeTrabajo, cartera::CatalogoTasas,
    },
};

/// Eventos de las reglas de contabilización de la vertical (y tipo de origen del comprobante).
/// En cada uno, los conceptos son el `concepto_contable` de los conceptos de cobro.
pub mod eventos {
    /// Liquidación de expensas: cada concepto debita la cartera y acredita el ingreso (o el
    /// pasivo de una cuota con destinación específica).
    pub const LIQUIDACION: &str = "ph.liquidacion";
    /// Cobro manual (multa, uso de un bien común): las mismas cuentas de la liquidación.
    pub const COBRO: &str = "ph.cobro";
    /// Recaudo: `recibido` (banco), un concepto por lo aplicado (crédito a la cartera) y
    /// `sin_aplicar` (anticipo).
    pub const RECAUDO: &str = "ph.recaudo";
    /// Aplicación posterior de un anticipo: `sin_aplicar` (débito) y el concepto (crédito).
    pub const APLICACION: &str = "ph.aplicacion";
    /// Nota crédito: reduce la cartera del concepto.
    pub const NOTA_CREDITO: &str = "ph.nota_credito";
    /// Descuento por pronto pago: reduce la cartera del concepto.
    pub const PRONTO_PAGO: &str = "ph.pronto_pago";
    /// Causación de intereses de mora: el concepto de tipo `interes_mora` debita la cartera de
    /// intereses y acredita el ingreso.
    pub const INTERESES: &str = "ph.intereses";
}

/// Serie del consecutivo de las cuentas de cobro (por año).
pub const SERIE_CUENTA_COBRO: &str = "CB";

/// Lo que usa una transacción de la cartera: la de tesorería, la cartera y los avisos.
pub trait TxCartera:
    TxTesoreria
    + RepositorioCartera
    + CatalogoTasas
    + RepositorioTickets
    + RepositorioTitulares
    + RegistroOutbox
{
}

impl<T> TxCartera for T where
    T: TxTesoreria
        + RepositorioCartera
        + CatalogoTasas
        + RepositorioTickets
        + RepositorioTitulares
        + RegistroOutbox
{
}

/// Abre la transacción exigiendo el permiso y los dos módulos.
pub(crate) async fn abrir<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    permiso: Permiso,
) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    ctx.exigir(permiso)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::Contabilidad).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::ContabilidadPh).await?;
    Ok(tx)
}

pub(crate) fn validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

/// Monto desde un decimal ya calculado (no negativo, dos decimales).
pub(crate) fn monto(valor: Decimal) -> AppResult<Monto> {
    Ok(Monto::new(valor.round_dp(2))?)
}
