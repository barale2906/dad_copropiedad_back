//! Nómina (plan 16, ADR 0025): módulo vendible [`Modulo::Nomina`] que sirve a cualquier
//! entidad. Usa la contabilidad si la entidad la tiene (fases siguientes); **la contabilidad no
//! importa la nómina** (prueba de fronteras).
//!
//! - [`catalogo`]: catálogo laboral de la plataforma (parámetros versionados y administradoras
//!   de la seguridad social) y su lectura desde la entidad.
//! - [`empleados`]: empleados, contratos laborales y sus versiones (modificación, prórroga y
//!   terminación).
//! - [`importacion`]: carga masiva de empleados con vista previa.

pub mod catalogo;
pub mod empleados;
pub mod importacion;

use domain::{Modulo, Permiso};

use crate::{
    AppResult, Contexto,
    casos_uso::modulos::exigir_modulo_en,
    puertos::{
        RegistroAuditoria, Reloj, RepositorioModulos, Transaccion, UnidadDeTrabajo,
        nomina::{RepositorioCatalogoLaboral, RepositorioNomina},
        personas::RepositorioPersonas,
    },
};

/// Lo que usa una transacción de la nómina.
pub trait TxNomina:
    Transaccion
    + RepositorioNomina
    + RepositorioCatalogoLaboral
    + RepositorioPersonas
    + RepositorioModulos
    + RegistroAuditoria
{
}

impl<T> TxNomina for T where
    T: Transaccion
        + RepositorioNomina
        + RepositorioCatalogoLaboral
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
