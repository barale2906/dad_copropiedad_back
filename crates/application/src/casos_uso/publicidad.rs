//! Publicidad de la plataforma (plan 08, fases 3 a 6; ADR 0015).
//!
//! La publicidad es un negocio **de la plataforma**: la venden y la cobran sus dueños. Las
//! entidades (PH y empresas) no venden, no cobran ni configuran nada: son el canal para llegar a
//! las personas que **autorizaron** recibir publicidad frente a la plataforma (finalidad
//! `publicidad`, con la política de la plataforma). Sin esa autorización no se muestra ninguna
//! pieza en la app ni se envía ningún mensaje.
//!
//! - [`gestion`]: el personal de la plataforma carga campañas y piezas de cualquier comercio, ve
//!   métricas y contactos autorizados, administra el enlace de cupones del comercio y excluye
//!   entidades de la publicidad (plan 20: los comercios no usan la plataforma).
//! - [`moderacion`]: verificación, aprobación de creativos y campañas (siempre y por **otra
//!   cuenta** que la que los cargó; el conflicto de interés queda marcado), reportes y pantallas
//!   del lobby.
//! - [`entrega`]: piezas para el residente autorizado, eventos agregados con topes de
//!   presupuesto, cupones, reportes, autorización y revocación, pantalla del lobby y envíos
//!   masivos con tope de frecuencia (worker).
//! - [`liquidacion`]: tarifas, política de publicidad y liquidación mensual (sin reparto a las
//!   copropiedades).

use crate::{
    casos_uso::tickets::TxTickets,
    puertos::{
        RegistroAuditoria, RepositorioArchivosPlataforma, RepositorioDirectorio,
        RepositorioDirectorioPlataforma, RepositorioProteccionDatos, RepositorioPublicidad,
        RepositorioPublicidadPlataforma, identidad::RepositorioOrganizaciones,
    },
};

pub mod entrega;
pub mod gestion;
pub mod liquidacion;
pub mod moderacion;

/// Puertos de la transacción de plataforma que usa la publicidad.
pub trait TxPublicidadPlataforma:
    RepositorioPublicidadPlataforma
    + RepositorioDirectorioPlataforma
    + RepositorioOrganizaciones
    + RepositorioArchivosPlataforma
    + RegistroAuditoria
{
}

impl<T> TxPublicidadPlataforma for T where
    T: RepositorioPublicidadPlataforma
        + RepositorioDirectorioPlataforma
        + RepositorioOrganizaciones
        + RepositorioArchivosPlataforma
        + RegistroAuditoria
{
}

/// Puertos de la transacción de copropiedad que usa la publicidad.
pub trait TxPublicidad:
    TxTickets + RepositorioPublicidad + RepositorioProteccionDatos + RepositorioDirectorio
{
}

impl<T> TxPublicidad for T where
    T: TxTickets + RepositorioPublicidad + RepositorioProteccionDatos + RepositorioDirectorio
{
}

/// Motivo obligatorio (rechazos, retiros), hasta 500 caracteres.
fn motivo(valor: &str) -> crate::AppResult<String> {
    domain::copropiedad::texto_opcional("motivo", Some(valor), 500)?
        .ok_or_else(|| crate::AppError::Validacion("indique el motivo".into()))
}

#[cfg(test)]
mod tests;
