//! Puertos (traits) que implementan los adaptadores `infra-*`.
//!
//! Los métodos asíncronos devuelven `impl Future + Send` para poder usarse desde Tokio sin
//! `async-trait`. En la implementación basta con escribir `async fn`.

pub mod acceso;
pub mod actas;
pub mod archivos;
pub mod asambleas;
pub mod auditoria;
pub mod censo;
pub mod chat;
pub mod cola_salida;
pub mod comunicados;
pub mod contabilidad;
pub mod copropiedades;
pub mod correo;
pub mod directorio;
pub mod idempotencia;
pub mod identidad;
pub mod mantenimiento;
pub mod mensajeria;
pub mod modulos;
pub mod personas;
pub mod planillas;
pub mod proteccion_datos;
pub mod publicidad;
pub mod reloj;
pub mod reservas;
pub mod sesiones;
pub mod tesoreria;
pub mod tickets;
pub mod unidad_trabajo;

pub use acceso::{
    DirectorioAcceso, EnvioPush, FirmaQr, GeneradorImagenQr, RepositorioAcceso,
    RepositorioDispositivosPush,
};
pub use actas::GeneradorActas;
pub use archivos::{AlmacenArchivos, RepositorioArchivos};
pub use asambleas::{DirectorioAsambleas, RepositorioAsambleas, RepositorioLineasWhatsapp};
pub use auditoria::LecturaAuditoria;
pub use censo::{RepositorioCampanasCenso, RepositorioCenso};
pub use chat::{DirectorioTelefonos, RepositorioChat};
pub use cola_salida::{ColaSalida, LoteSalida, MensajePendiente};
pub use comunicados::RepositorioComunicados;
pub use copropiedades::{
    RepositorioBienesPrivados, RepositorioCoeficientes, RepositorioCopropiedad,
    RepositorioImportaciones, RepositorioPersonas, RepositorioTitulares,
};
pub use correo::EnvioCorreo;
pub use directorio::{
    PurgaDirectorio, RepositorioDirectorio, RepositorioDirectorioPlataforma,
    RepositorioSolicitudesProveedor,
};
pub use idempotencia::RegistroIdempotencia;
pub use identidad::{
    HashContrasenas, RepositorioContratos, RepositorioCuentas, RepositorioOrganizaciones,
    RepositorioPlataforma, RepositorioRoles, RepositorioSesiones, SegundoFactor,
    UnidadDeTrabajoPlataforma,
};
pub use mantenimiento::RepositorioMantenimiento;
pub use mensajeria::Mensajeria;
pub use modulos::RepositorioModulos;
pub use planillas::LectorPlanillas;
pub use proteccion_datos::RepositorioProteccionDatos;
pub use publicidad::{
    DirectorioPublicidad, RepositorioPublicidad, RepositorioPublicidadPlataforma,
};
pub use reloj::{Reloj, RelojSistema};
pub use reservas::{DirectorioRecordatorios, RepositorioReservas};
pub use sesiones::SesionesConversacion;
pub use tickets::{DirectorioGestion, RepositorioTickets};
pub use unidad_trabajo::{RegistroAuditoria, RegistroOutbox, Transaccion, UnidadDeTrabajo};
