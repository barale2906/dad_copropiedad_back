//! Identificadores tipados: impiden pasar un `BienPrivadoId` donde se espera un `PreguntaId`.
//!
//! Todas las entidades usan **UUID v7** (ordenables por tiempo), excepto [`VotoId`], que usa
//! **UUID v4** para no filtrar la hora del voto (secreto del voto).

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_entidad {
    ($(#[$meta:meta])* $nombre:ident, $generador:path) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $nombre(Uuid);

        impl $nombre {
            /// Genera un identificador nuevo.
            pub fn nuevo() -> Self {
                Self($generador())
            }

            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl From<Uuid> for $nombre {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl From<$nombre> for Uuid {
            fn from(id: $nombre) -> Self {
                id.0
            }
        }

        impl fmt::Display for $nombre {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

id_entidad!(
    /// Copropiedad (PH). Es la unidad de aislamiento multi-tenant (`tenant_id` + RLS).
    TenantId,
    Uuid::now_v7
);
id_entidad!(
    /// Reunión de asamblea (ordinaria o extraordinaria).
    ReunionId,
    Uuid::now_v7
);
id_entidad!(PreguntaId, Uuid::now_v7);
id_entidad!(OpcionId, Uuid::now_v7);
id_entidad!(
    /// Bien privado o de dominio particular (Ley 675, art. 3): unidad con coeficiente de copropiedad.
    BienPrivadoId,
    Uuid::now_v7
);
id_entidad!(
    /// Persona natural: propietario o apoderado.
    PersonaId,
    Uuid::now_v7
);
id_entidad!(
    /// Asistente de una reunión: la persona que representa uno o varios bienes privados.
    AsistenteId,
    Uuid::now_v7
);
id_entidad!(
    /// Poder con el que un apoderado representa un bien privado en una reunión.
    PoderId,
    Uuid::now_v7
);
id_entidad!(
    /// Punto del orden del día de una reunión.
    PuntoOrdenId,
    Uuid::now_v7
);
id_entidad!(
    /// Periodo de presencia de un asistente (entrada y salida).
    PresenciaId,
    Uuid::now_v7
);
id_entidad!(
    /// Vínculo vigente entre un asistente y un bien privado que representa.
    AsistenteBienId,
    Uuid::now_v7
);
id_entidad!(
    /// Asignación de un rótulo preimpreso a un asistente.
    RotuloId,
    Uuid::now_v7
);
id_entidad!(
    /// Foto del quórum en un momento de la reunión.
    FotoQuorumId,
    Uuid::now_v7
);
id_entidad!(ParticipacionId, Uuid::now_v7);
id_entidad!(
    /// Archivo subido a una copropiedad (`archivos`).
    ArchivoId,
    Uuid::now_v7
);
id_entidad!(
    /// Versión de la política de tratamiento de datos de una copropiedad.
    PoliticaTratamientoId,
    Uuid::now_v7
);
id_entidad!(
    /// Autorización de tratamiento de datos de un titular para una finalidad.
    AutorizacionId,
    Uuid::now_v7
);
id_entidad!(
    /// Consulta o reclamo de un titular sobre sus datos (Ley 1581, arts. 14 y 15).
    SolicitudTitularId,
    Uuid::now_v7
);
id_entidad!(
    /// Cuenta global de una persona en la plataforma (inicia sesión una vez, opera en varias PH).
    CuentaId,
    Uuid::now_v7
);
id_entidad!(
    /// Sesión de una cuenta en un dispositivo.
    SesionId,
    Uuid::now_v7
);
id_entidad!(
    /// Rol (plantilla del sistema o propio de una copropiedad).
    RolId,
    Uuid::now_v7
);
id_entidad!(
    /// Asignación de un rol a una cuenta, con vigencia.
    AsignacionRolId,
    Uuid::now_v7
);
id_entidad!(
    /// Organización (empresa o persona natural con RUT) compartida entre copropiedades.
    OrganizacionId,
    Uuid::now_v7
);
id_entidad!(
    /// Contrato de servicio entre una copropiedad y una organización.
    ContratoServicioId,
    Uuid::now_v7
);
id_entidad!(
    /// Solicitud para reclamar la ficha de una organización.
    ReclamacionId,
    Uuid::now_v7
);
id_entidad!(
    /// Agrupación de bienes privados (edificio, torre, etapa, bloque, manzana…).
    AgrupacionId,
    Uuid::now_v7
);
id_entidad!(
    /// Versión del cuadro de coeficientes de copropiedad.
    CuadroCoeficientesId,
    Uuid::now_v7
);
id_entidad!(
    /// Titularidad de una persona sobre un bien privado, con vigencia.
    TitularidadId,
    Uuid::now_v7
);
id_entidad!(
    /// Asignación de un bien común de uso exclusivo a un bien privado (Ley 675, art. 22).
    UsoExclusivoId,
    Uuid::now_v7
);
id_entidad!(
    /// Punto de atención de la copropiedad (oficina de administración, portería…).
    PuntoAtencionId,
    Uuid::now_v7
);
id_entidad!(
    /// Importación masiva desde un archivo CSV/XLSX.
    ImportacionId,
    Uuid::now_v7
);
id_entidad!(
    /// Residencia de una persona en un bien privado (censo).
    ResidenciaId,
    Uuid::now_v7
);
id_entidad!(
    /// Vehículo registrado en el censo.
    VehiculoId,
    Uuid::now_v7
);
id_entidad!(
    /// Mascota registrada en el censo.
    MascotaId,
    Uuid::now_v7
);
id_entidad!(
    /// Propuesta de cambio del censo (autoservicio).
    CambioCensoId,
    Uuid::now_v7
);
id_entidad!(
    /// Recurso reservable (bien común, parqueadero, agenda de la administración, herramienta).
    RecursoId,
    Uuid::now_v7
);
id_entidad!(
    /// Reserva (o cita) de un recurso.
    ReservaId,
    Uuid::now_v7
);
id_entidad!(
    /// Asignación de un parqueadero a un bien privado.
    AsignacionParqueaderoId,
    Uuid::now_v7
);
id_entidad!(
    /// Sorteo de recursos.
    SorteoId,
    Uuid::now_v7
);
id_entidad!(
    /// Restricción de reservas de un bien privado (mora o sanción).
    RestriccionId,
    Uuid::now_v7
);
id_entidad!(
    /// Cargo pendiente para el futuro módulo de contabilidad.
    CargoId,
    Uuid::now_v7
);
id_entidad!(
    /// Ticket (PQRS o solicitud).
    TicketId,
    Uuid::now_v7
);
id_entidad!(
    /// Mensaje de un ticket (público o nota interna).
    MensajeTicketId,
    Uuid::now_v7
);
id_entidad!(
    /// Categoría de tickets configurable por copropiedad.
    CategoriaTicketId,
    Uuid::now_v7
);
id_entidad!(
    /// Conversación de WhatsApp escalada a la administración.
    ConversacionId,
    Uuid::now_v7
);
id_entidad!(
    /// Pregunta frecuente que responde el chat.
    PreguntaFrecuenteId,
    Uuid::now_v7
);
id_entidad!(
    /// Activo del inventario (equipo, herramienta, mobiliario, dotación).
    ActivoId,
    Uuid::now_v7
);
id_entidad!(
    /// Movimiento de inventario de un activo.
    MovimientoInventarioId,
    Uuid::now_v7
);
id_entidad!(
    /// Plan de mantenimiento preventivo.
    PlanMantenimientoId,
    Uuid::now_v7
);
id_entidad!(
    /// Orden de trabajo de mantenimiento.
    OrdenTrabajoId,
    Uuid::now_v7
);
id_entidad!(
    /// Obligación periódica (certificaciones, renovaciones, inspecciones).
    ObligacionId,
    Uuid::now_v7
);
id_entidad!(
    /// Visita anunciada o pre-autorizada en la portería (plan 05).
    VisitaId,
    Uuid::now_v7
);
id_entidad!(
    /// Paquete, sobre o recibo recibido en la portería.
    CorrespondenciaId,
    Uuid::now_v7
);
id_entidad!(
    /// Novedad de la minuta digital de portería.
    NovedadMinutaId,
    Uuid::now_v7
);
id_entidad!(
    /// Dispositivo de la app móvil registrado para notificaciones push.
    DispositivoPushId,
    Uuid::now_v7
);
id_entidad!(
    /// Punto de acceso (portería peatonal, vehicular, gimnasio…).
    PuntoAccesoId,
    Uuid::now_v7
);
id_entidad!(
    /// Puente de acceso instalado en la copropiedad (equipo en sitio).
    PuenteId,
    Uuid::now_v7
);
id_entidad!(
    /// Dispositivo de acceso conectado a un puente (torniquete, lector, cámara…).
    DispositivoAccesoId,
    Uuid::now_v7
);
id_entidad!(
    /// Credencial de acceso (tarjeta, PIN, QR, placa, referencia biométrica).
    CredencialId,
    Uuid::now_v7
);
id_entidad!(
    /// Autorización de acceso (quién, dónde y cuándo).
    AutorizacionAccesoId,
    Uuid::now_v7
);
id_entidad!(
    /// Evento de acceso reportado por la portería o un puente.
    EventoAccesoId,
    Uuid::now_v7
);
id_entidad!(
    /// Consentimiento biométrico de una persona (Ley 1581 de 2012).
    ConsentimientoBiometricoId,
    Uuid::now_v7
);
id_entidad!(
    /// Comunicado de la administración (cartelera digital).
    ComunicadoId,
    Uuid::now_v7
);
id_entidad!(
    /// Identificador de un voto. **Siempre UUID v4**: un v7 revelaría la hora de emisión.
    VotoId,
    Uuid::new_v4
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voto_usa_uuid_v4_y_entidades_v7() {
        assert_eq!(VotoId::nuevo().as_uuid().get_version_num(), 4);
        assert_eq!(PreguntaId::nuevo().as_uuid().get_version_num(), 7);
        assert_eq!(TenantId::nuevo().as_uuid().get_version_num(), 7);
    }
}
