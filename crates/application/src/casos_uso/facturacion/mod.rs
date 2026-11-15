//! Facturación (plan 15 y plan 18, ADR 0022): facturas de venta, catálogo de productos y
//! servicios y documentos electrónicos (factura, notas, documento soporte, facturas recibidas y
//! eventos RADIAN). **Se vende sola**: funciona con el módulo
//! [`Modulo::FacturacionElectronica`] sin la contabilidad; si la entidad también tiene
//! [`Modulo::Contabilidad`], cada documento se contabiliza en su transacción.
//!
//! - [`items`]: catálogo de productos y servicios.
//! - [`ventas`]: facturas de venta en borrador, emisión, pago manual y anulación.
//! - [`contabilizacion`]: contabilizar lo facturado antes de tener contabilidad.
//! - [`habilitacion`]: habilitación de la entidad ante la DIAN con el motor propio (software,
//!   certificado, set de pruebas, producción y rangos).
//! - [`exportaciones`]: libros de ventas y compras en CSV y paquete ZIP de XML y PDF para el
//!   contador externo.
//! - [`documentos`]: factura electrónica, nota crédito, documento soporte, reintentos,
//!   representación gráfica y facturas recibidas con RADIAN, por el [`ProveedorFacturacion`].
//!
//! La facturación puede usar el núcleo contable; **la contabilidad no importa este módulo**
//! (prueba de fronteras).
//!
//! [`ProveedorFacturacion`]: crate::puertos::tributario::ProveedorFacturacion

pub mod contabilizacion;
pub mod documentos;
pub mod exportaciones;
pub mod habilitacion;
pub mod items;
pub mod radian;
pub mod ventas;

use domain::{Modulo, Permiso};

use crate::{
    AppResult, Contexto,
    casos_uso::{contabilidad::TxTesoreria, modulos::abrir_con_modulos},
    puertos::{Reloj, RepositorioModulos, UnidadDeTrabajo, facturacion::RepositorioFacturacion},
};

/// Lo que usa una transacción de ventas: la de tesorería (facturas, terceros y, si hay
/// contabilidad, el contabilizador) y la propia de la facturación.
pub trait TxVentas: TxTesoreria + RepositorioFacturacion {}

impl<T> TxVentas for T where T: TxTesoreria + RepositorioFacturacion {}

/// Módulos con los que se factura: la facturación electrónica sola o la contabilidad (que emite
/// facturas de venta aunque no sean electrónicas). Son los mismos de lo tributario.
pub(crate) const MODULOS_VENTAS: &[Modulo] = crate::casos_uso::tributario::MODULOS_TRIBUTARIOS;

/// Abre la transacción de ventas: exige el permiso y alguno de [`MODULOS_VENTAS`].
pub(crate) async fn abrir_ventas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    permiso: Permiso,
) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    abrir_con_modulos(unidad, reloj, ctx, &[permiso], MODULOS_VENTAS).await
}

/// ¿La entidad lleva contabilidad ahora? Decide si un documento se contabiliza.
pub(crate) async fn con_contabilidad<T: RepositorioModulos>(
    tx: &mut T,
    reloj: &impl Reloj,
) -> AppResult<bool> {
    tx.modulo_habilitado(Modulo::Contabilidad, reloj.ahora())
        .await
}
