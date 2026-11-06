//! Capa de aplicación de DAD_COPROPIEDAD (arquitectura hexagonal).
//!
//! - [`puertos`]: traits que la aplicación **necesita** del mundo exterior (repositorios,
//!   mensajería, reloj, sesiones…). Los implementan los crates `infra-*`.
//! - [`paginacion`]: paginación por cursor ([`Pagina`], [`SolicitudPagina`]) común a los listados.
//! - [`casos_uso`]: orquestan el dominio a través de los puertos. Son genéricos sobre los
//!   puertos, así que se prueban con dobles en memoria sin base de datos ni red.
//!
//! Esta capa no conoce SQL, HTTP ni WhatsApp: solo tipos de `domain` y de este crate.

pub mod casos_uso;
pub mod contexto;
pub mod error;
pub mod outbox;
pub mod paginacion;
pub mod puertos;

pub use contexto::Contexto;
pub use error::{AppError, AppResult};
pub use paginacion::{Pagina, SolicitudPagina};
