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
//! - Asambleas: [`TransaccionPg`] implementa `RepositorioAsambleas`; [`DirectorioAsambleasPg`]
//!   cruza copropiedades para el gateway; los cambios en vivo se avisan con `pg_notify` en
//!   [`CANAL_EN_VIVO`].
//! - Control de acceso: [`TransaccionPg`] implementa `RepositorioAcceso`; [`DirectorioAccesoPg`]
//!   cruza copropiedades para el worker y autentica los puentes; los dispositivos push van en la
//!   transacción de plataforma.
//! - Las migraciones se ejecutan con otra conexión (dueño del esquema): [`migrar`].
//! - Preferir `sqlx::query!`/`query_as!` en los repositorios y versionar `.sqlx/`.

mod acceso;
mod archivos;
mod asambleas;
mod bienes_privados;
mod censo;
mod chat;
mod coeficientes;
mod cola_salida;
mod comunicados;
mod copropiedades;
mod error;
mod mantenimiento;
mod modulos;
mod personas;
mod plataforma;
mod pool;
mod proteccion_datos;
mod reservas;
mod roles;
mod tenant;
mod tickets;
mod titulares;
mod unidad_trabajo;

pub use acceso::DirectorioAccesoPg;
pub use asambleas::{CANAL_EN_VIVO, DirectorioAsambleasPg, OyenteEnVivo};
pub use chat::{DirectorioGestionPg, DirectorioTelefonosPg, SesionesChatPg};
pub use cola_salida::{ColaSalidaPg, LoteSalidaPg};
pub use error::InfraPgError;
pub use plataforma::{TransaccionPlataformaPg, UnidadDeTrabajoPlataformaPg};
pub use pool::{conectar, migrar, ping, verificar_rol_restringido};
pub use reservas::DirectorioRecordatoriosPg;
pub use tenant::transaccion_tenant;
pub use unidad_trabajo::{TransaccionPg, UnidadDeTrabajoPg};

/// Re-exportado para que los binarios no dependan de SQLx solo por el tipo del pool.
pub use sqlx::PgPool;
