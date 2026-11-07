//! Dominio de DAD_COPROPIEDAD (propiedad horizontal, Ley 675 de 2001).
//!
//! Núcleo de la arquitectura hexagonal: **Rust puro, sin IO** (ni base de datos, ni HTTP, ni
//! Tokio). Todo lo que aquí vive se prueba con tests unitarios rápidos.
//!
//! Invariantes críticos (ver `AGENTS.md`):
//! - Un bien privado vota **una sola vez** por pregunta (unicidad en BD + validación aquí).
//! - Reintentos con la misma clave de idempotencia (o el mismo `wamid`) **no** generan votos.
//! - Solo una [`PreguntaAbierta`] puede emitir votos: el compilador impide votar en otra.
//! - En votaciones secretas nada permite reconstruir qué opción eligió cada bien privado.
//! - Resultados y quórum se calculan sumando **coeficientes** (`Decimal`), nunca contando votos.

#[macro_use]
mod macros;

pub mod archivo;
pub mod calendario;
pub mod coeficiente;
pub mod correo;
pub mod cuenta;
pub mod error;
pub mod ids;
pub mod mayoria;
pub mod modulo;
pub mod nit;
pub mod organizacion;
pub mod permisos;
pub mod pregunta;
pub mod proteccion_datos;
pub mod quorum;
pub mod registro;
pub mod resultados;
pub mod reunion;
pub mod voto;

pub use archivo::{Almacen, ClasificacionArchivo, ClaveArchivo, NombreArchivo, TipoArchivo};
pub use coeficiente::Coeficiente;
pub use correo::DireccionCorreo;
pub use cuenta::{ContrasenaNueva, EstadoCuenta, Identificador, NombreUsuario};
pub use error::DominioError;
pub use ids::{
    ArchivoId, AsignacionRolId, AutorizacionId, BienPrivadoId, ContratoServicioId, CuentaId,
    InscripcionId, OpcionId, OrganizacionId, ParticipacionId, PersonaId, PoliticaTratamientoId,
    PreguntaId, ReclamacionId, ReunionId, RolId, SesionId, SolicitudTitularId, TenantId, VotoId,
};
pub use mayoria::ReglaMayoria;
pub use modulo::Modulo;
pub use nit::Nit;
pub use organizacion::{
    EstadoContrato, EstadoOrganizacion, EstadoReclamacion, NaturalezaOrganizacion, RolOrganizacion,
    TipoOrganizacion,
};
pub use permisos::{Ambito, ConjuntoPermisos, OrigenAsignacion, Permiso, RolPlantilla};
pub use pregunta::{EstadoPregunta, Pregunta, PreguntaAbierta, TipoVoto};
pub use proteccion_datos::{FinalidadTratamiento, MedioAutorizacion, TipoSolicitudTitular};
pub use quorum::{Convocatoria, Quorum};
pub use registro::{MetodoRegistro, RolAsistente};
pub use resultados::ProyeccionResultados;
pub use reunion::{EstadoReunion, TipoReunion};
pub use voto::{
    Canal, ClaveIdempotencia, Papeleta, ParticipacionNueva, VinculoNominal, VotoEmitido, VotoNuevo,
};
