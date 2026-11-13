//! Esquemas OpenAPI de los enums del dominio.
//!
//! `domain` y `application` no dependen de utoipa (§2), y la regla del huérfano impide
//! implementar `ToSchema` aquí para sus tipos. Cada enum persistido (`enum_texto!`) tiene en
//! este módulo un **tipo espejo** sin datos que publica un esquema con nombre propio y sus
//! valores, tomados de `TODOS` / `as_str`: el contrato no puede desalinearse del código.
//!
//! Uso en un DTO: `#[schema(value_type = esquemas::EstadoTicket)]` (o `Option<…>`, `Vec<…>`);
//! en parámetros: `#[param(value_type = Option<esquemas::EstadoTicket>)]`. El cliente del
//! frontend recibe un tipo con nombre (`'radicado' | 'en_tramite' | …`) en lugar de `string`.
//! Un enum nuevo expuesto en la API se declara aquí (la descripción repite su `///`).

use std::borrow::Cow;

use utoipa::openapi::{
    RefOr, Schema,
    schema::{ObjectBuilder, Type},
};

/// Declara el tipo espejo de un enum: `Nombre = ruta::del::Enum, "descripción";`.
macro_rules! esquema_enum {
    ($($nombre:ident = $origen:path, $descripcion:literal;)+) => {
        $(
            #[doc = $descripcion]
            ///
            #[doc = concat!("Esquema OpenAPI de [`", stringify!($origen), "`].")]
            #[derive(Debug, Clone, Copy)]
            pub struct $nombre;

            impl utoipa::PartialSchema for $nombre {
                fn schema() -> RefOr<Schema> {
                    ObjectBuilder::new()
                        .schema_type(Type::String)
                        .enum_values(Some(<$origen>::TODOS.iter().map(|v| v.as_str())))
                        .description(Some($descripcion))
                        .into()
                }
            }

            impl utoipa::ToSchema for $nombre {
                fn name() -> Cow<'static, str> {
                    Cow::Borrowed(stringify!($nombre))
                }
            }
        )+
    };
}

esquema_enum! {
    AdvertenciaFicha = domain::copropiedad::AdvertenciaFicha,
        "Advertencia legal que se muestra en la ficha de la copropiedad.";
    AlertaMascota = domain::censo::AlertaMascota,
        "Alerta del censo.";
    Ambito = domain::permisos::Ambito,
        "Ámbito de un permiso o de un rol.";
    BaseComputo = domain::mayoria::BaseComputo,
        "Con qué se pesan los votos y el quórum.";
    BaseMayoria = domain::mayoria::BaseMayoria,
        "Sobre qué total se mide un umbral personalizado.";
    CalidadRepresentacion = domain::asistente::CalidadRepresentacion,
        "En qué calidad el asistente representa un bien privado.";
    CalidadResidencia = domain::censo::CalidadResidencia,
        "Calidad en que una persona reside en un bien privado.";
    CanalConvocatoria = application::puertos::asambleas::CanalConvocatoria,
        "Canal de una constancia de convocatoria.";
    CanalPublicidad = domain::publicidad::CanalPublicidad,
        "Canal por el que se entrega una pieza.";
    CanalReserva = domain::reserva::CanalReserva,
        "Por dónde llegó la reserva.";
    CanalRespuestaVisita = domain::acceso::CanalRespuestaVisita,
        "Por dónde llegó la decisión sobre una visita.";
    CanalTicket = domain::ticket::CanalTicket,
        "Por dónde llegó el ticket.";
    CanalVoto = domain::voto::Canal,
        "Canal por el que llegó el voto (queda en el acta).";
    Cara = domain::contabilidad::Cara,
        "Cara de una línea de regla contable.";
    EstadoComprobante = domain::contabilidad::EstadoComprobante,
        "Estado de un comprobante contable.";
    EstadoPeriodo = domain::contabilidad::EstadoPeriodo,
        "Estado de un periodo contable.";
    EstadoPlantilla = application::puertos::contabilidad::EstadoPlantilla,
        "Estado de una plantilla contable.";
    EstadoReapertura = domain::contabilidad::EstadoReapertura,
        "Estado de una solicitud de reapertura de periodo.";
    MarcoTecnico = domain::contabilidad::MarcoTecnico,
        "Marco técnico contable (grupo NIIF).";
    Naturaleza = domain::contabilidad::Naturaleza,
        "Naturaleza de una cuenta contable.";
    NivelCuenta = domain::contabilidad::NivelCuenta,
        "Nivel de una cuenta según el largo de su código.";
    OrigenCentro = domain::contabilidad::OrigenCentro,
        "De dónde sale el centro de costo de una línea de regla.";
    OrigenTercero = domain::contabilidad::OrigenTercero,
        "De dónde sale el tercero de una línea de regla.";
    RegimenIva = domain::contabilidad::RegimenIva,
        "Régimen de IVA de un tercero.";
    ResponsabilidadFiscal = domain::contabilidad::ResponsabilidadFiscal,
        "Responsabilidad fiscal del RUT.";
    CargoEleccion = domain::pregunta::CargoEleccion,
        "Cargo o designación que se elige con una pregunta de elección.";
    CategoriaBienComun = domain::reserva::CategoriaBienComun,
        "Categoría de bien común.";
    ClaseActivo = domain::mantenimiento::ClaseActivo,
        "Clase de activo.";
    ClasificacionArchivo = domain::archivo::ClasificacionArchivo,
        "Sensibilidad del archivo (Ley 1581): define quién puede verlo y cuánto se protege.";
    CobroPor = domain::reserva::CobroPor,
        "Base del cobro.";
    Convocatoria = domain::quorum::Convocatoria,
        "Convocatoria en la que se reúne la asamblea.";
    EfectoMora = domain::reunion::EfectoMora,
        "Qué pasa con los bienes privados del listado de morosos que entrega la copropiedad. **Validar con asesor**: la jurisprudencia ha limitado restringir el voto del moroso; la elección queda auditada y en el acta.";
    EspecieMascota = domain::censo::EspecieMascota,
        "Especie de la mascota.";
    EstadoActa = domain::reunion::EstadoActa,
        "Estado del acta de la reunión.";
    EstadoActivo = domain::mantenimiento::EstadoActivo,
        "Estado operativo de un activo.";
    EstadoCambioCenso = domain::censo::EstadoCambioCenso,
        "Estado de una propuesta de cambio del censo.";
    EstadoCampana = domain::publicidad::EstadoCampana,
        "Estado de una campaña.";
    EstadoComunicado = domain::comunicado::EstadoComunicado,
        "Estado de un comunicado.";
    EstadoContrato = domain::organizacion::EstadoContrato,
        "Estado de un contrato de servicio con una copropiedad.";
    EstadoConversacion = application::puertos::chat::EstadoConversacion,
        "Estado de una conversación de la bandeja.";
    EstadoCopropiedad = domain::copropiedad::EstadoCopropiedad,
        "Estado de la copropiedad en la plataforma.";
    EstadoCorrespondencia = domain::acceso::EstadoCorrespondencia,
        "Estado de la correspondencia.";
    EstadoCreativo = domain::publicidad::EstadoCreativo,
        "Estado de moderación de un creativo.";
    EstadoCredencial = domain::acceso::EstadoCredencial,
        "Estado de una credencial.";
    EstadoCuadro = domain::cuadro_coeficientes::EstadoCuadro,
        "Estado de un cuadro de coeficientes.";
    EstadoImportacion = domain::importacion::EstadoImportacion,
        "Estado de una importación.";
    EstadoLinea = application::puertos::asambleas::EstadoLinea,
        "Estado operativo de una línea de WhatsApp.";
    EstadoLiquidacion = domain::publicidad::EstadoLiquidacion,
        "Estado de una liquidación de pauta (sin reparto a las copropiedades).";
    EstadoMascota = domain::censo::EstadoMascota,
        "Estado de la mascota en el censo.";
    EstadoOrden = domain::mantenimiento::EstadoOrden,
        "Estado de una orden de trabajo. `programada → en_ejecucion → ejecutada → verificada`; `programada | en_ejecucion → cancelada`; la verificación puede devolverla (`ejecutada → en_ejecucion`).";
    EstadoOrganizacion = domain::organizacion::EstadoOrganizacion,
        "Estado de una organización en la plataforma.";
    EstadoPago = domain::reserva::EstadoPago,
        "Estado del pago del valor de la reserva. Hoy se registra a mano; el pago en línea llegará con una pasarela (decisión pendiente) o con el módulo de contabilidad.";
    EstadoPoder = domain::asistente::EstadoPoder,
        "Estado de un poder.";
    EstadoPregunta = domain::pregunta::EstadoPregunta,
        "Ciclo de vida de una pregunta sometida a votación.";
    EstadoPuente = domain::acceso::EstadoPuente,
        "Estado de un puente.";
    EstadoPunto = domain::reunion::EstadoPunto,
        "Estado de un punto del orden del día.";
    EstadoReclamacion = domain::organizacion::EstadoReclamacion,
        "Estado de la reclamación de una organización por su empresa.";
    EstadoRecurso = domain::reserva::EstadoRecurso,
        "Estado operativo de un recurso.";
    EstadoReserva = domain::reserva::EstadoReserva,
        "Estado de una reserva.";
    EstadoReunion = domain::reunion::EstadoReunion,
        "Ciclo de vida de una reunión. Una consulta por escrito (art. 43) pasa de `Convocada` a `Cerrada` al vencer el plazo.";
    EstadoSolicitudTitular = domain::proteccion_datos::EstadoSolicitudTitular,
        "Estado de una solicitud del titular.";
    EstadoSorteo = domain::parqueadero::EstadoSorteo,
        "Estado de un sorteo.";
    EstadoTicket = domain::ticket::EstadoTicket,
        "Estado de un ticket.";
    EstadoVerificacion = domain::directorio::EstadoVerificacion,
        "Estado de verificación de un anunciante. El texto es contrato con la BD.";
    EstadoVisita = domain::acceso::EstadoVisita,
        "Estado de una visita.";
    FinalidadTratamiento = domain::proteccion_datos::FinalidadTratamiento,
        "Para qué se tratan los datos. La autorización es por finalidad: aceptar una no implica aceptar las demás (por ejemplo, la publicidad siempre es aparte).";
    FormaCopropiedad = domain::copropiedad::FormaCopropiedad,
        "Forma física del inmueble sometido al régimen.";
    FormatoCreativo = domain::publicidad::FormatoCreativo,
        "Formato de un creativo.";
    Frecuencia = domain::mantenimiento::Frecuencia,
        "Unidad de la recurrencia de un plan preventivo.";
    MedioAutorizacion = domain::proteccion_datos::MedioAutorizacion,
        "Medio por el que el titular otorgó la autorización. Debe poder probarse (Decreto 1377, art. 7): el medio, la fecha y, si existe, la evidencia (archivo firmado).";
    MetodoEntrega = domain::reserva::MetodoEntrega,
        "Cómo se entrega el recurso.";
    MetodoRegistro = domain::asistente::MetodoRegistro,
        "Cómo quedó registrado el asistente.";
    Modalidad = domain::reunion::Modalidad,
        "Cómo se reúne la asamblea.";
    ModalidadCita = domain::reserva::ModalidadCita,
        "Modalidad de una cita con la administración.";
    ModeloCobro = domain::publicidad::ModeloCobro,
        "Modelo de cobro de una campaña.";
    ModoAsignacion = domain::reserva::ModoAsignacion,
        "Cómo se asigna un recurso.";
    Modulo = domain::modulo::Modulo,
        "Módulo contratable por la copropiedad.";
    MomentoEvidencia = domain::mantenimiento::MomentoEvidencia,
        "Momento de una evidencia de una orden de trabajo.";
    MotivoSalida = domain::asistente::MotivoSalida,
        "Motivo de salida de un asistente: sus bienes dejan el quórum y las preguntas que se abran después; sus votos ya emitidos se conservan.";
    NaturalezaOrganizacion = domain::organizacion::NaturalezaOrganizacion,
        "Persona jurídica o persona natural con RUT (su NIT es su cédula: dato personal).";
    NaturalezaParqueadero = domain::parqueadero::NaturalezaParqueadero,
        "Naturaleza jurídica y de uso del parqueadero.";
    ObjetivoCampana = domain::publicidad::ObjetivoCampana,
        "Objetivo de una campaña.";
    OrigenAsignacion = domain::permisos::OrigenAsignacion,
        "Origen de una asignación de rol. Las derivadas no se asignan a mano: nacen y vencen con el hecho que las origina (vender el bien privado retira el acceso del vendedor).";
    OrigenAsignacionParqueadero = domain::parqueadero::OrigenAsignacionParqueadero,
        "De dónde sale la asignación de un parqueadero a un bien privado.";
    OrigenAutorizacionAcceso = domain::acceso::OrigenAutorizacionAcceso,
        "De dónde sale una autorización de acceso.";
    OrigenOrden = domain::mantenimiento::OrigenOrden,
        "De dónde nace una orden de trabajo.";
    OrigenVisita = domain::acceso::OrigenVisita,
        "Cómo nació la visita.";
    Permiso = domain::permisos::Permiso,
        "Permiso del catálogo. El texto (`modulo.recurso.accion`) es contrato con la BD y con los clientes: no se renombra sin migración.";
    PlataformaPush = domain::acceso::PlataformaPush,
        "Plataforma del dispositivo móvil (apps Flutter).";
    PrioridadTicket = domain::ticket::PrioridadTicket,
        "Prioridad de atención.";
    QuienReserva = domain::reserva::QuienReserva,
        "Quién puede reservar.";
    RespuestaCampanaCenso = domain::censo::RespuestaCampanaCenso,
        "Respuesta a una campaña de actualización del censo (plan 10, fase 5).";
    ResultadoAcceso = domain::acceso::ResultadoAcceso,
        "Resultado de un intento de acceso.";
    ResultadoLlamada = domain::acceso::ResultadoLlamada,
        "Resultado de la llamada de la portería.";
    RolOrganizacion = domain::organizacion::RolOrganizacion,
        "Papel de una cuenta dentro de una organización.";
    SentidoAcceso = domain::acceso::SentidoAcceso,
        "Sentido de un punto o de un evento.";
    TipoAgrupacion = domain::bien_privado::TipoAgrupacion,
        "Clase de agrupación. La PH elige la etiqueta que muestra (\"Torre\", \"Bloque\") en el nombre.";
    TipoArchivo = domain::archivo::TipoArchivo,
        "Tipos de archivo admitidos (lista blanca). Se detecta por los primeros bytes del contenido (*magic bytes*), **nunca** por la extensión ni por el `Content-Type` que envía el cliente.";
    TipoArchivoActivo = domain::mantenimiento::TipoArchivoActivo,
        "Clase de archivo de la hoja de vida de un activo.";
    TipoBienPrivado = domain::bien_privado::TipoBienPrivado,
        "Clase de bien privado.";
    TipoCambioCenso = domain::censo::TipoCambioCenso,
        "Qué se propone cambiar en el censo.";
    TipoCobertura = domain::directorio::TipoCobertura,
        "Tipo de cobertura persistido (`fichas_directorio.cobertura`).";
    TipoCopropiedad = domain::copropiedad::TipoCopropiedad,
        "Destinación de la propiedad horizontal (Ley 675, art. 3).";
    TipoCorrespondencia = domain::acceso::TipoCorrespondencia,
        "Tipo de correspondencia.";
    TipoCredencial = domain::acceso::TipoCredencial,
        "Tipo de credencial.";
    TipoDispositivoAcceso = domain::acceso::TipoDispositivoAcceso,
        "Tipo de dispositivo conectado a un puente.";
    TipoDocumento = domain::censo::TipoDocumento,
        "Tipo de documento de identidad.";
    TipoDocumentoReunion = domain::reunion::TipoDocumentoReunion,
        "Tipo de documento anexo a la convocatoria.";
    TipoEntrega = domain::reserva::TipoEntrega,
        "Momento de la entrega del recurso.";
    TipoEventoVisita = domain::acceso::TipoEventoVisita,
        "Hecho del historial de una visita (append-only).";
    TipoMovimiento = domain::mantenimiento::TipoMovimiento,
        "Movimiento de inventario (historial append-only).";
    TipoNovedadMinuta = domain::acceso::TipoNovedadMinuta,
        "Tipo de novedad de la minuta de portería.";
    TipoOrden = domain::mantenimiento::TipoOrden,
        "Tipo de mantenimiento.";
    TipoOrganizacion = domain::organizacion::TipoOrganizacion,
        "A qué se dedica la organización (puede tener varios tipos).";
    TipoParqueadero = domain::parqueadero::TipoParqueadero,
        "Vehículo para el que está destinado el parqueadero.";
    TipoPersona = domain::titularidad::TipoPersona,
        "Persona natural o jurídica.";
    TipoPregunta = domain::pregunta::TipoPregunta,
        "Qué se pregunta.";
    TipoPuntoAcceso = domain::acceso::TipoPuntoAcceso,
        "Tipo de punto de acceso.";
    TipoPuntoAtencion = domain::copropiedad::TipoPuntoAtencion,
        "Clase de punto de atención de la copropiedad.";
    TipoRecurso = domain::reserva::TipoRecurso,
        "Clase de recurso reservable.";
    TipoReunion = domain::reunion::TipoReunion,
        "Tipo de reunión según la Ley 675 (art. 39).";
    TipoSolicitudTitular = domain::proteccion_datos::TipoSolicitudTitular,
        "Solicitud del titular sobre sus datos.";
    TipoTicket = domain::ticket::TipoTicket,
        "Clase de ticket. El texto es contrato con el `CHECK` de `tickets.tipo`.";
    TipoTitularidad = domain::titularidad::TipoTitularidad,
        "Calidad en que una persona es titular.";
    TipoUsoExclusivo = domain::bien_privado::TipoUsoExclusivo,
        "Bien común asignado en uso exclusivo a un bien privado (art. 22).";
    TipoVehiculo = domain::censo::TipoVehiculo,
        "Clase de vehículo.";
    TipoVisita = domain::acceso::TipoVisita,
        "Tipo de visita. Todas siguen el mismo flujo de autorización (incluidos los domicilios, decisión del 2026-11-09).";
    TipoVoto = domain::pregunta::TipoVoto,
        "Visibilidad del voto (ADR 0010: no existe el secreto absoluto).";
    UnidadPlazo = domain::ticket::UnidadPlazo,
        "Unidad de un plazo.";
    UsoBienPrivado = domain::bien_privado::UsoBienPrivado,
        "Destinación del bien privado.";
}
