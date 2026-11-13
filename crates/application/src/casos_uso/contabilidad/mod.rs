//! Casos de uso del núcleo contable (plan 12, ADR 0017). Sirven a cualquier entidad; la prueba
//! de fronteras impide que importen módulos de la PH.
//!
//! - [`configuracion`]: configuración, plan de cuentas, centros de costo, tipos de comprobante,
//!   reglas, rubros, notas, datos fiscales de terceros y aplicar o exportar plantillas.
//! - [`plantillas`]: PUC y plantillas contables de la plataforma y de las firmas contables.
//! - [`periodos`]: estados de los periodos y reaperturas con segregación de funciones.
//! - [`comprobantes`]: borradores, previsualización, contabilización, reverso, el
//!   contabilizador de documentos origen, saldos iniciales y cierre anual.
//! - [`reportes`]: balance de prueba, libro diario, mayor, auxiliares y estados financieros.
//! - [`tesoreria`]: cuentas de tesorería, recibos de caja, egresos y eventos de la pasarela
//!   (plan 13).
//! - [`cuentas_por_pagar`]: catálogo de retenciones, facturas de proveedores, causación y
//!   certificados de retención.
//! - [`ventas`]: facturas de venta, recaudo con los recibos y cartera por edades.
//! - [`conciliacion`]: extractos bancarios y conciliación automática y manual.
//!
//! Toda operación exige el módulo [`Modulo::Contabilidad`] dentro de su transacción.

pub mod comprobantes;
pub mod conciliacion;
pub mod configuracion;
pub mod cuentas_por_pagar;
pub mod periodos;
pub mod plantillas;
pub mod reportes;
pub mod tesoreria;
pub mod ventas;

use domain::Modulo;

use crate::{
    AppResult,
    casos_uso::modulos::exigir_modulo_en,
    puertos::{
        RegistroAuditoria, Reloj, RepositorioModulos, RepositorioPersonas, Transaccion,
        contabilidad::{
            RepositorioComprobantes, RepositorioConfiguracionContable, RepositorioPlanCuentas,
        },
        tesoreria::{
            CatalogoRetenciones, RepositorioConciliacion, RepositorioFacturas, RepositorioTesoreria,
        },
    },
};

/// Traits de [`TxContable`] en alcance (sus métodos se llaman sobre la transacción).
pub(crate) mod prelude {
    pub(crate) use crate::puertos::{
        RegistroAuditoria as _, RepositorioPersonas as _, Transaccion as _,
        contabilidad::{
            RepositorioComprobantes as _, RepositorioConfiguracionContable as _,
            RepositorioPlanCuentas as _,
        },
        tesoreria::{
            CatalogoRetenciones as _, RepositorioConciliacion as _, RepositorioFacturas as _,
            RepositorioTesoreria as _,
        },
    };
}

/// Lo que usa una transacción contable: los puertos del núcleo, los módulos, las personas
/// (terceros) y la auditoría.
pub trait TxContable:
    RepositorioPlanCuentas
    + RepositorioConfiguracionContable
    + RepositorioComprobantes
    + RepositorioModulos
    + RepositorioPersonas
    + RegistroAuditoria
    + Transaccion
{
}

impl<T> TxContable for T where
    T: RepositorioPlanCuentas
        + RepositorioConfiguracionContable
        + RepositorioComprobantes
        + RepositorioModulos
        + RepositorioPersonas
        + RegistroAuditoria
        + Transaccion
{
}

/// Lo que usa una transacción de tesorería: la contable más tesorería, facturas, conciliación y
/// el catálogo de retenciones.
pub trait TxTesoreria:
    TxContable
    + RepositorioTesoreria
    + RepositorioFacturas
    + RepositorioConciliacion
    + CatalogoRetenciones
{
}

impl<T> TxTesoreria for T where
    T: TxContable
        + RepositorioTesoreria
        + RepositorioFacturas
        + RepositorioConciliacion
        + CatalogoRetenciones
{
}

/// Texto opcional: vacío = `None`; si trae algo, se valida como [`texto`].
pub(crate) fn texto_opcional(
    campo: &'static str,
    valor: Option<&str>,
    maximo: usize,
) -> AppResult<Option<String>> {
    match valor.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => texto(campo, v, maximo).map(Some),
    }
}

/// Exige el módulo de contabilidad en la transacción.
pub(crate) async fn exigir_contabilidad<T: RepositorioModulos>(
    tx: &mut T,
    reloj: &impl Reloj,
) -> AppResult<()> {
    exigir_modulo_en(tx, reloj, Modulo::Contabilidad).await
}

/// ¿Es un código de tipo de empresa válido (`propiedad_horizontal`, `gimnasio`…)?
pub(crate) fn es_tipo_empresa(texto: &str) -> bool {
    let mut c = texto.chars();
    (2..=60).contains(&texto.len())
        && c.next().is_some_and(|x| x.is_ascii_lowercase())
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '_')
}

/// Valida un texto obligatorio con largo máximo.
pub(crate) fn texto(campo: &'static str, valor: &str, maximo: usize) -> AppResult<String> {
    let limpio = valor.trim();
    if limpio.is_empty() || limpio.chars().count() > maximo {
        return Err(crate::AppError::Validacion(format!(
            "`{campo}` es obligatorio y tiene a lo sumo {maximo} caracteres"
        )));
    }
    Ok(limpio.to_owned())
}
