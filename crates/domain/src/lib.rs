//! Dominio de DAD_COPROPIEDAD (propiedad horizontal, Ley 675 de 2001).
//!
//! Núcleo de la arquitectura hexagonal: **Rust puro, sin IO** (ni base de datos, ni HTTP, ni
//! Tokio). Todo lo que aquí vive se prueba con tests unitarios rápidos.
//!
//! Invariantes críticos (ver `AGENTS.md`):
//! - Un bien privado vota **una sola vez** por pregunta (unicidad en BD + validación aquí).
//! - Reintentos con la misma clave de idempotencia (o el mismo `wamid`) **no** generan votos.
//! - Solo una [`PreguntaAbierta`] puede emitir votos: el compilador impide votar en otra.
//! - El asistente vota una vez por todos los bienes que representa: una participación por bien.
//! - En votaciones reservadas el vínculo bien → opción solo lo consulta quien tiene permiso
//!   (ADR 0010: no existe el secreto absoluto).
//! - Resultados y quórum se calculan por **coeficientes** (`Decimal`) y, a la vez, por unidades;
//!   decide la base de cómputo de la pregunta.

#[macro_use]
mod macros;

pub mod acceso;
pub mod archivo;
pub mod asistente;
pub mod bien_privado;
pub mod calendario;
pub mod censo;
pub mod coeficiente;
pub mod comunicado;
pub mod copropiedad;
pub mod correo;
pub mod cuadro_coeficientes;
pub mod cuenta;
pub mod eleccion;
pub mod error;
pub mod horario;
pub mod ids;
pub mod importacion;
pub mod mantenimiento;
pub mod mayoria;
pub mod modulo;
pub mod nit;
pub mod organizacion;
pub mod parqueadero;
pub mod permisos;
pub mod pregunta;
pub mod proteccion_datos;
pub mod quorum;
pub mod reserva;
pub mod resultados;
pub mod reunion;
pub mod sorteo;
pub mod ticket;
pub mod titularidad;
pub mod voto;

pub use acceso::{
    CanalAviso, CanalRespuestaVisita, CargaQr, ConfiguracionAcceso, EstadoCorrespondencia,
    EstadoCredencial, EstadoPuente, EstadoVisita, HorarioSilencio, NivelAviso,
    OrigenAutorizacionAcceso, OrigenVisita, PasoVisita, PlataformaPush, QuienRecoge,
    ResidenteAviso, ResultadoAcceso, ResultadoLlamada, SentidoAcceso, SoporteEntrega,
    TipoCorrespondencia, TipoCredencial, TipoDispositivoAcceso, TipoEventoVisita,
    TipoNovedadMinuta, TipoPuntoAcceso, TipoVisita, TitularAviso, VentanaAcceso,
};
pub use archivo::{Almacen, ClasificacionArchivo, ClaveArchivo, NombreArchivo, TipoArchivo};
pub use asistente::{
    CalidadRepresentacion, CodigoRotulo, EstadoPoder, MetodoRegistro, MomentoQuorum, MotivoSalida,
};
pub use bien_privado::{
    CodigoAgrupacion, CodigoBienPrivado, NumeroBienPrivado, PlantillaNomenclatura, TipoAgrupacion,
    TipoBienPrivado, TipoUsoExclusivo, UsoBienPrivado,
};
pub use censo::{
    AlertaMascota, CalidadResidencia, EspecieMascota, EstadoCambioCenso, EstadoMascota, Placa,
    RespuestaCampanaCenso, TipoCambioCenso, TipoDocumento, TipoVehiculo,
};
pub use coeficiente::Coeficiente;
pub use comunicado::{Audiencia, CanalesComunicado, EstadoComunicado};
pub use copropiedad::{
    AdvertenciaFicha, CodigoDane, EstadoCopropiedad, EstadoLegal, FormaCopropiedad, Telefono,
    TipoCopropiedad, TipoPuntoAtencion,
};
pub use correo::DireccionCorreo;
pub use cuadro_coeficientes::{EstadoCuadro, RevisionCuadro, TipoCuadro};
pub use cuenta::{ContrasenaNueva, EstadoCuenta, Identificador, NombreUsuario};
pub use error::DominioError;
pub use horario::{ExcepcionHorario, FranjaSemanal, Horario};
pub use ids::{
    ActivoId, AgrupacionId, ArchivoId, AsignacionParqueaderoId, AsignacionRolId, AsistenteBienId,
    AsistenteId, AutorizacionAccesoId, AutorizacionId, BienPrivadoId, CambioCensoId, CargoId,
    CategoriaTicketId, ComunicadoId, ConsentimientoBiometricoId, ContratoServicioId,
    ConversacionId, CorrespondenciaId, CredencialId, CuadroCoeficientesId, CuentaId,
    DispositivoAccesoId, DispositivoPushId, EventoAccesoId, FotoQuorumId, ImportacionId, MascotaId,
    MensajeTicketId, MovimientoInventarioId, NovedadMinutaId, ObligacionId, OpcionId,
    OrdenTrabajoId, OrganizacionId, ParticipacionId, PersonaId, PlanMantenimientoId, PoderId,
    PoliticaTratamientoId, PreguntaFrecuenteId, PreguntaId, PresenciaId, PuenteId, PuntoAccesoId,
    PuntoAtencionId, PuntoOrdenId, ReclamacionId, RecursoId, ReservaId, ResidenciaId,
    RestriccionId, ReunionId, RolId, RotuloId, SesionId, SolicitudTitularId, SorteoId, TenantId,
    TicketId, TitularidadId, UsoExclusivoId, VehiculoId, VisitaId, VotoId,
};
pub use importacion::{Documento, EstadoImportacion, TipoImportacion};
pub use mantenimiento::{
    ClaseActivo, CodigoActivo, EstadoActivo, EstadoOrden, Frecuencia, MomentoEvidencia,
    NumeroOrden, OrigenOrden, PuntoChecklist, Recurrencia, TipoArchivoActivo, TipoMovimiento,
    TipoOrden,
};
pub use mayoria::{BaseComputo, BaseMayoria, ReglaMayoria};
pub use modulo::Modulo;
pub use nit::Nit;
pub use organizacion::{
    EstadoContrato, EstadoOrganizacion, EstadoReclamacion, NaturalezaOrganizacion, RolOrganizacion,
    TipoOrganizacion,
};
pub use parqueadero::{
    EstadoSorteo, NaturalezaParqueadero, OrigenAsignacionParqueadero, TipoParqueadero,
};
pub use permisos::{Ambito, ConjuntoPermisos, OrigenAsignacion, Permiso, RolPlantilla};
pub use pregunta::{
    CargoEleccion, EstadoPregunta, OpcionPregunta, Pregunta, PreguntaAbierta, TEXTOS_SI_NO,
    TipoPregunta, TipoVoto,
};
pub use proteccion_datos::{
    EstadoSolicitudTitular, FinalidadTratamiento, MedioAutorizacion, TipoSolicitudTitular,
};
pub use quorum::{Convocatoria, Quorum, QuorumReunion};
pub use reserva::{
    CanalReserva, CategoriaBienComun, CobroPor, ConceptoCargo, EstadoPago, EstadoRecurso,
    EstadoReserva, Franja, MetodoEntrega, ModalidadCita, ModoAsignacion, PoliticaReserva,
    QuienReserva, TipoEntrega, TipoRecurso, VinculoSolicitante,
};
pub use resultados::{Conteo, Decision, Escrutinio, ResultadoOpcion, VotosOpcion};
pub use reunion::{
    EfectoMora, EstadoActa, EstadoPunto, EstadoReunion, Modalidad, TipoDocumentoReunion,
    TipoReunion,
};
pub use ticket::{
    Calificacion, CanalTicket, EstadoTicket, JornadaHabil, NumeroTicket, Plazo, PlazosTicket,
    PrioridadTicket, TipoTicket, UnidadPlazo,
};
pub use titularidad::{ConjuntoTitulares, Porcentaje, TipoPersona, TipoTitularidad, TitularNuevo};
pub use voto::{
    BienRepresentado, Canal, ClaveIdempotencia, Papeleta, ParticipacionNueva, VotoEmitido,
    VotoNuevo,
};
