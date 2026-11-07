//! Adaptador de PostgreSQL. Implementa los puertos de persistencia de `application`.
//!
//! Reglas de este crate:
//! - Toda operación de negocio corre dentro de [`transaccion_tenant`]: fija `app.tenant_id`
//!   con alcance de transacción y las políticas RLS filtran el resto.
//! - La aplicación se conecta con el rol `dad_app` (sin `BYPASSRLS`, no dueño de las tablas);
//!   [`verificar_rol_restringido`] impide arrancar con un rol privilegiado.
//! - Los cambios de negocio y su auditoría se agrupan en una [`TransaccionPg`] abierta con
//!   [`UnidadDeTrabajoPg`] (puerto `UnidadDeTrabajo`).
//! - [`TransaccionPg`] también implementa los repositorios de archivos, protección de datos,
//!   copropiedad, bienes privados, coeficientes, titulares e importaciones.
//! - Cuentas, sesiones, roles de plataforma y organizaciones van en transacciones de plataforma
//!   ([`UnidadDeTrabajoPlataformaPg`], `app.plataforma = on`), que no ven datos de ninguna PH.
//! - El worker de envíos consume el outbox con [`ColaSalidaPg`] (funciones `SECURITY DEFINER`).
//! - Las migraciones se ejecutan con otra conexión (dueño del esquema): [`migrar`].
//! - Preferir `sqlx::query!`/`query_as!` en los repositorios y versionar `.sqlx/`.

mod archivos;
mod bienes_privados;
mod coeficientes;
mod cola_salida;
mod copropiedades;
mod error;
mod modulos;
mod plataforma;
mod pool;
mod proteccion_datos;
mod roles;
mod tenant;
mod titulares;
mod unidad_trabajo;

pub use cola_salida::{ColaSalidaPg, LoteSalidaPg};
pub use error::InfraPgError;
pub use plataforma::{TransaccionPlataformaPg, UnidadDeTrabajoPlataformaPg};
pub use pool::{conectar, migrar, ping, verificar_rol_restringido};
pub use tenant::transaccion_tenant;
pub use unidad_trabajo::{TransaccionPg, UnidadDeTrabajoPg};

/// Re-exportado para que los binarios no dependan de SQLx solo por el tipo del pool.
pub use sqlx::PgPool;
