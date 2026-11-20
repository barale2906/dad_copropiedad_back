//! Nómina (plan 16, ADR 0025): módulo vendible [`Modulo::Nomina`] que sirve a cualquier
//! entidad. Usa la contabilidad si la entidad la tiene (fases siguientes); **la contabilidad no
//! importa la nómina** (prueba de fronteras).
//!
//! - [`catalogo`]: catálogo laboral de la plataforma (parámetros versionados y administradoras
//!   de la seguridad social) y su lectura desde la entidad.
//! - [`empleados`]: empleados, contratos laborales y sus versiones (modificación, prórroga y
//!   terminación).
//! - [`importacion`]: carga masiva de empleados con vista previa.
//! - [`importacion_novedades`]: carga masiva de novedades con vista previa.
//! - [`novedades`]: novedades de los contratos (horas, días y valores) y su anulación.
//! - [`contabilizacion`]: documentos origen al contabilizador al aprobar y al pagar (solo con
//!   contabilidad).
//! - [`deducciones`]: deducciones de la retención declaradas por el empleado.
//! - [`desprendibles`]: PDF de cada liquidación aprobada, envío por correo y portal del
//!   empleado.
//! - [`prestaciones`]: prima, cesantías, intereses, vacaciones e indemnización en la
//!   liquidación y cesantías por consignar.
//! - [`reportes`]: aportes del mes por administradora (insumo de la PILA) e ingresos y
//!   retenciones del año (insumo del certificado).
//! - [`liquidacion`]: configuración de la entidad, periodos, liquidación, aprobación por otra
//!   cuenta y pago.

pub mod catalogo;
pub mod contabilizacion;
pub mod deducciones;
pub mod desprendibles;
pub mod empleados;
pub mod importacion;
pub mod importacion_novedades;
pub mod liquidacion;
pub mod novedades;
pub mod prestaciones;
pub mod reportes;

use domain::{Modulo, Permiso};

use crate::{
    AppResult, Contexto,
    casos_uso::modulos::exigir_modulo_en,
    puertos::{
        RegistroAuditoria, Reloj, RepositorioModulos, Transaccion, UnidadDeTrabajo,
        nomina::{RepositorioCatalogoLaboral, RepositorioLiquidacionNomina, RepositorioNomina},
        personas::RepositorioPersonas,
        tesoreria::CatalogoRetenciones,
    },
};

/// Lo que usa una transacción de la nómina.
pub trait TxNomina:
    Transaccion
    + RepositorioNomina
    + RepositorioLiquidacionNomina
    + RepositorioCatalogoLaboral
    + CatalogoRetenciones
    + RepositorioPersonas
    + RepositorioModulos
    + RegistroAuditoria
{
}

impl<T> TxNomina for T where
    T: Transaccion
        + RepositorioNomina
        + RepositorioLiquidacionNomina
        + RepositorioCatalogoLaboral
        + CatalogoRetenciones
        + RepositorioPersonas
        + RepositorioModulos
        + RegistroAuditoria
{
}

/// Abre la transacción de la entidad exigiendo el permiso y el módulo de nómina.
pub(crate) async fn abrir_nomina<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    permiso: Permiso,
) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    ctx.exigir(permiso)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::Nomina).await?;
    Ok(tx)
}
