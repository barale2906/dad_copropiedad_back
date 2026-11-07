//! Adaptador de PostgreSQL. Implementa los puertos de persistencia de `application`.
//!
//! Reglas de este crate:
//! - Toda operación de negocio corre dentro de [`transaccion_tenant`]: fija `app.tenant_id`
//!   con alcance de transacción y las políticas RLS filtran el resto.
//! - La aplicación se conecta con el rol `dad_app` (sin `BYPASSRLS`, no dueño de las tablas);
//!   [`verificar_rol_restringido`] impide arrancar con un rol privilegiado.
//! - Los cambios de negocio y su auditoría se agrupan en una [`TransaccionPg`] abierta con
//!   [`UnidadDeTrabajoPg`] (puerto `UnidadDeTrabajo`).
//! - El worker de envíos consume el outbox con [`ColaSalidaPg`] (funciones `SECURITY DEFINER`).
//! - Las migraciones se ejecutan con otra conexión (dueño del esquema): [`migrar`].
//! - Preferir `sqlx::query!`/`query_as!` en los repositorios y versionar `.sqlx/`.

mod cola_salida;
mod error;
mod pool;
mod tenant;
mod unidad_trabajo;

pub use cola_salida::{ColaSalidaPg, LoteSalidaPg};
pub use error::InfraPgError;
pub use pool::{conectar, migrar, ping, verificar_rol_restringido};
pub use tenant::transaccion_tenant;
pub use unidad_trabajo::{TransaccionPg, UnidadDeTrabajoPg};

/// Re-exportado para que los binarios no dependan de SQLx solo por el tipo del pool.
pub use sqlx::PgPool;
