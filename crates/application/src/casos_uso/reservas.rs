//! Recursos reservables (bienes comunes, agenda de la administración, herramientas) y su motor
//! de reservas (plan 04, módulo vendible `bienes_comunes`).
//!
//! Invariantes:
//! 1. Dos reservas activas de un recurso exclusivo nunca se solapan (y la BD lo repite).
//! 2. En recursos con aforo nunca se supera el cupo por franja: el recurso se bloquea mientras
//!    se cuentan las personas.
//! 3. Una reserva solo se crea dentro del horario y respetando la política.
//! 4. Solicitar es idempotente por `idempotency_key`: un reintento devuelve la reserva original.
//!
//! El valor se liquida y se registra (estado del pago manual); el pago en línea llegará con una
//! pasarela o con el módulo de contabilidad. Las restricciones por mora o sanción no aplican a
//! los bienes comunes esenciales (art. 59).

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use domain::{
    ArchivoId, BienPrivadoId, CanalReserva, CargoId, ConceptoCargo, EstadoPago, EstadoRecurso,
    EstadoReserva, Franja, Horario, ModalidadCita, Modulo, Permiso, PersonaId, Placa,
    PoliticaReserva, RecursoId, ReservaId, ResidenciaId, RestriccionId, TipoEntrega, TipoRecurso,
    copropiedad::{texto_obligatorio, texto_opcional},
    reserva::{Tz, disponibilidad as calcular_disponibilidad, franjas_llenas, zona_horaria},
};
use rust_decimal::Decimal;

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::modulos::exigir_modulo_en,
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        RegistroAuditoria, RegistroOutbox, Reloj, RepositorioCenso, RepositorioCopropiedad,
        RepositorioModulos, RepositorioPersonas, RepositorioReservas, RepositorioTitulares,
        Transaccion, UnidadDeTrabajo,
        correo::CorreoSaliente,
        reservas::{
            CargoPendiente, DetalleBienComun, ElementoChecklist, Entrega, FiltroReservas, Recurso,
            Reserva, Restriccion,
        },
    },
};

pub(crate) const ACCION_CREAR_RECURSO: AccionAuditoria =
    AccionAuditoria::new("bienes_comunes.recurso.crear");
pub(crate) const ACCION_EDITAR_RECURSO: AccionAuditoria =
    AccionAuditoria::new("bienes_comunes.recurso.editar");
pub(crate) const ACCION_ESTADO_RECURSO: AccionAuditoria =
    AccionAuditoria::new("bienes_comunes.recurso.cambiar_estado");
pub(crate) const ACCION_SOLICITAR: AccionAuditoria =
    AccionAuditoria::new("reservas.reserva.solicitar");
pub(crate) const ACCION_APROBAR: AccionAuditoria = AccionAuditoria::new("reservas.reserva.aprobar");
pub(crate) const ACCION_RECHAZAR: AccionAuditoria =
    AccionAuditoria::new("reservas.reserva.rechazar");
pub(crate) const ACCION_CANCELAR: AccionAuditoria =
    AccionAuditoria::new("reservas.reserva.cancelar");
pub(crate) const ACCION_ENTREGA: AccionAuditoria =
    AccionAuditoria::new("reservas.reserva.entregar");
pub(crate) const ACCION_RECIBO: AccionAuditoria = AccionAuditoria::new("reservas.reserva.recibir");
pub(crate) const ACCION_NO_ASISTIO: AccionAuditoria =
    AccionAuditoria::new("reservas.reserva.no_asistio");
pub(crate) const ACCION_PAGO: AccionAuditoria =
    AccionAuditoria::new("reservas.reserva.registrar_pago");
pub(crate) const ACCION_RESTRINGIR: AccionAuditoria =
    AccionAuditoria::new("reservas.restriccion.crear");
pub(crate) const ACCION_LEVANTAR: AccionAuditoria =
    AccionAuditoria::new("reservas.restriccion.levantar");
pub(crate) const ACCION_AUTORIZAR_TENEDOR: AccionAuditoria =
    AccionAuditoria::new("reservas.tenedor.autorizar");

/// Anticipación de los recordatorios.
pub const HORAS_RECORDATORIO: i64 = 24;

pub(crate) async fn zona<T: RepositorioCopropiedad>(tx: &mut T) -> AppResult<Tz> {
    let ficha = tx
        .ficha()
        .await?
        .ok_or(AppError::NoEncontrado("copropiedad"))?;
    Ok(zona_horaria(&ficha.zona_horaria)?)
}

/// Encola un correo para la persona si tiene correo registrado.
pub(crate) async fn avisar<T>(
    tx: &mut T,
    persona: Option<PersonaId>,
    prioridad: PrioridadSalida,
    asunto: String,
    texto: String,
) -> AppResult<()>
where
    T: RepositorioPersonas + RegistroOutbox,
{
    let Some(p) = persona else { return Ok(()) };
    let Some(correo) = tx.persona(p).await?.and_then(|p| p.correo) else {
        return Ok(());
    };
    tx.encolar(EventoSaliente {
        prioridad,
        mensaje: MensajeOutbox::Correo(CorreoSaliente {
            para: correo,
            asunto,
            texto,
            html: None,
        }),
    })
    .await
}

fn texto_franja(r: &Reserva, recurso: &str, tz: Tz) -> String {
    let i = r.franja.inicio().with_timezone(&tz);
    let f = r.franja.fin().with_timezone(&tz);
    format!(
        "{recurso}: {} de {} a {}",
        i.format("%Y-%m-%d"),
        i.format("%H:%M"),
        f.format("%H:%M")
    )
}

// ---------------------------------------------------------------------------------------------
// Catálogo de recursos
// ---------------------------------------------------------------------------------------------

/// Datos de un recurso nuevo (bien común, agenda o herramienta; los parqueaderos van por
/// [`crate::casos_uso::parqueaderos`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoRecurso {
    pub tipo: TipoRecurso,
    pub nombre: String,
    pub ubicacion: Option<String>,
    pub descripcion: Option<String>,
    pub politica: PoliticaReserva,
    pub horario: Horario,
    /// Obligatorio para los bienes comunes.
    pub bien_comun: Option<DetalleBienComun>,
}

pub(crate) fn validar_textos_recurso(r: &mut Recurso) -> AppResult<()> {
    r.nombre = texto_obligatorio("nombre", &r.nombre, 80)?;
    r.ubicacion = texto_opcional("ubicacion", r.ubicacion.as_deref(), 200)?;
    r.descripcion = texto_opcional("descripcion", r.descripcion.as_deref(), 2000)?;
    if let Some(b) = &mut r.bien_comun {
        b.reglas = texto_opcional("reglas", b.reglas.as_deref(), 10_000)?;
        if b.area_m2.is_some_and(|a| a <= Decimal::ZERO) {
            return Err(AppError::Validacion("`area_m2` debe ser positiva".into()));
        }
    }
    Ok(())
}

/// Crea un bien común, la agenda de la administración o una herramienta prestable.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_comunes.editar`; [`AppError::ModuloNoHabilitado`];
/// [`AppError::Validacion`] / [`AppError::Dominio`] si los datos o la política son inválidos.
pub async fn crear_recurso<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoRecurso,
) -> AppResult<Recurso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    ctx.exigir(Permiso::BienesComunesEditar)?;
    if nuevo.tipo == TipoRecurso::Parqueadero {
        return Err(AppError::Validacion(
            "los parqueaderos se crean en /parqueaderos".into(),
        ));
    }
    if (nuevo.tipo == TipoRecurso::BienComun) != nuevo.bien_comun.is_some() {
        return Err(AppError::Validacion(
            "un bien común lleva su detalle (`bien_comun`) y los demás recursos no".into(),
        ));
    }
    let mut recurso = Recurso {
        id: RecursoId::nuevo(),
        tipo: nuevo.tipo,
        nombre: nuevo.nombre,
        estado: EstadoRecurso::Disponible,
        ubicacion: nuevo.ubicacion,
        descripcion: nuevo.descripcion,
        politica: nuevo.politica.validar()?,
        horario: nuevo.horario,
        bien_comun: nuevo.bien_comun,
        parqueadero: None,
    };
    validar_textos_recurso(&mut recurso)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    tx.insertar_recurso(&recurso).await?;
    let detalle = DetalleAuditoria::vacio().con("tipo", recurso.tipo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR_RECURSO, "recurso", recurso.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(recurso)
}

/// Recursos de la copropiedad (de un tipo o todos).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_comunes.ver`; [`AppError::ModuloNoHabilitado`].
pub async fn listar_recursos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    tipo: Option<TipoRecurso>,
) -> AppResult<Vec<Recurso>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos,
{
    ctx.exigir(Permiso::BienesComunesVer)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    tx.recursos(tipo).await
}

/// Un recurso.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_comunes.ver`; [`AppError::NoEncontrado`].
pub async fn ver_recurso<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: RecursoId,
) -> AppResult<Recurso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos,
{
    ctx.exigir(Permiso::BienesComunesVer)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    tx.recurso(id)
        .await?
        .ok_or(AppError::NoEncontrado("recurso"))
}

/// Cambios a un recurso. `None` = sin cambio.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CambiosRecurso {
    pub nombre: Option<String>,
    pub ubicacion: Option<String>,
    pub descripcion: Option<String>,
    pub politica: Option<PoliticaReserva>,
    pub horario: Option<Horario>,
    pub bien_comun: Option<DetalleBienComun>,
}

/// Edita un recurso (datos, política, horario o detalle). Las reservas ya hechas conservan
/// el valor y la exclusividad con que se crearon.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_comunes.editar` (o `parqueaderos.asignar` si es un
/// parqueadero); [`AppError::NoEncontrado`]; errores de validación.
pub async fn editar_recurso<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: RecursoId,
    c: CambiosRecurso,
) -> AppResult<Recurso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let mut r = tx
        .recurso(id)
        .await?
        .ok_or(AppError::NoEncontrado("recurso"))?;
    exigir_edicion(ctx, &r)?;
    if c.bien_comun.is_some() && r.bien_comun.is_none() {
        return Err(AppError::Validacion(
            "el recurso no es un bien común".into(),
        ));
    }
    if let Some(v) = c.nombre {
        r.nombre = v;
    }
    if let Some(v) = c.ubicacion {
        r.ubicacion = Some(v);
    }
    if let Some(v) = c.descripcion {
        r.descripcion = Some(v);
    }
    if let Some(p) = c.politica {
        r.politica = p.validar()?;
    }
    if let Some(h) = c.horario {
        r.horario = h;
    }
    if let Some(b) = c.bien_comun {
        r.bien_comun = Some(b);
    }
    validar_textos_recurso(&mut r)?;
    tx.actualizar_recurso(&r).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_EDITAR_RECURSO,
        "recurso",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(r)
}

fn exigir_edicion(ctx: &Contexto, r: &Recurso) -> AppResult<()> {
    if r.tipo == TipoRecurso::Parqueadero {
        ctx.exigir(Permiso::ParqueaderosAsignar)
    } else {
        ctx.exigir(Permiso::BienesComunesEditar)
    }
}

/// Cambia el estado de un recurso. Al dejarlo no disponible (mantenimiento, fuera de servicio,
/// inactivo) cancela sin penalidad las reservas activas que aún no empiezan y avisa a los
/// solicitantes. Devuelve cuántas canceló.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_comunes.editar`; [`AppError::NoEncontrado`].
pub async fn cambiar_estado_recurso<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: RecursoId,
    estado: EstadoRecurso,
    motivo: Option<String>,
) -> AppResult<u32>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas
        + RepositorioModulos
        + RepositorioCopropiedad
        + RepositorioPersonas
        + RegistroOutbox
        + RegistroAuditoria,
{
    let ahora = reloj.ahora();
    let motivo = texto_opcional("motivo", motivo.as_deref(), 500)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let mut r = tx
        .recurso_para_reservar(id)
        .await?
        .ok_or(AppError::NoEncontrado("recurso"))?;
    exigir_edicion(ctx, &r)?;
    r.estado = estado;
    tx.actualizar_recurso(&r).await?;
    let mut canceladas = 0;
    if estado != EstadoRecurso::Disponible {
        let tz = zona(&mut tx).await?;
        let filtro = FiltroReservas {
            recurso_id: Some(id),
            ..FiltroReservas::default()
        };
        for mut reserva in tx.reservas(&filtro).await? {
            if !reserva.estado.activa() || reserva.franja.inicio() <= ahora {
                continue;
            }
            reserva.estado = reserva.estado.transicionar(EstadoReserva::Cancelada)?;
            reserva.observacion = Some(
                motivo
                    .clone()
                    .unwrap_or_else(|| format!("recurso {}", estado.as_str())),
            );
            tx.actualizar_reserva(&reserva).await?;
            avisar(
                &mut tx,
                reserva.solicitante_persona_id,
                PrioridadSalida::Evento,
                "Reserva cancelada".into(),
                format!(
                    "Su reserva ({}) se canceló porque el recurso quedó {}.",
                    texto_franja(&reserva, &r.nombre, tz),
                    estado.as_str().replace('_', " ")
                ),
            )
            .await?;
            canceladas += 1;
        }
    }
    let detalle = DetalleAuditoria::vacio()
        .con("estado", estado.as_str())?
        .con("reservas_canceladas", canceladas)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ESTADO_RECURSO, "recurso", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(canceladas)
}

/// Franjas libres del recurso entre dos fechas locales (inclusive).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_comunes.ver`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si el rango es inválido.
pub async fn disponibilidad<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: RecursoId,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<Vec<Franja>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RepositorioCopropiedad,
{
    ctx.exigir(Permiso::BienesComunesVer)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let r = tx
        .recurso(id)
        .await?
        .ok_or(AppError::NoEncontrado("recurso"))?;
    if r.estado != EstadoRecurso::Disponible {
        return Ok(Vec::new());
    }
    let tz = zona(&mut tx).await?;
    let a_utc = |d: NaiveDate| {
        tz.from_local_datetime(&d.and_time(chrono::NaiveTime::MIN))
            .earliest()
            .map(|x| x.with_timezone(&Utc))
            .ok_or(AppError::Validacion(
                "fecha inválida en la zona de la PH".into(),
            ))
    };
    let rango = Franja::new(a_utc(desde)?, a_utc(hasta + Duration::days(1))?)?;
    let activas: Vec<Reserva> = tx
        .reservas(&FiltroReservas {
            recurso_id: Some(id),
            entre: Some(rango),
            ..FiltroReservas::default()
        })
        .await?
        .into_iter()
        .filter(|r| r.estado.activa())
        .collect();
    let ocupadas: Vec<Franja> = match r.politica.aforo {
        None => activas.iter().map(|r| r.franja).collect(),
        Some(aforo) => franjas_llenas(
            &activas
                .iter()
                .map(|r| (r.franja, r.invitados + 1))
                .collect::<Vec<_>>(),
            aforo,
        ),
    };
    Ok(calcular_disponibilidad(
        &r.horario,
        tz,
        desde,
        hasta,
        &ocupadas,
        Duration::minutes(i64::from(r.politica.alistamiento_min)),
    )?)
}

// ---------------------------------------------------------------------------------------------
// Reservas
// ---------------------------------------------------------------------------------------------

/// Solicitud de una reserva (o cita, o parqueadero de visitantes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolicitudReserva {
    pub recurso_id: RecursoId,
    pub bien_privado_id: BienPrivadoId,
    pub franja: Franja,
    pub invitados: u32,
    /// La genera el cliente; un reintento con la misma clave devuelve la reserva original.
    pub idempotency_key: String,
    pub canal: CanalReserva,
    pub modalidad: Option<ModalidadCita>,
    pub motivo: Option<String>,
    /// Obligatoria en los parqueaderos de visitantes.
    pub placa_visitante: Option<Placa>,
}

/// Solicita una reserva. Con confirmación inmediata queda aprobada; si no, solicitada.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `reservas.crear`; [`AppError::ModuloNoHabilitado`].
/// - [`AppError::Dominio`] ([`domain::DominioError::ReservaNoPermitida`]) si incumple la
///   política, el horario, los límites, el cupo, el vínculo con el bien o hay restricción.
/// - [`AppError::Conflicto`] si la franja ya está ocupada.
pub async fn solicitar_reserva<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    s: SolicitudReserva,
) -> AppResult<Reserva>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas
        + RepositorioModulos
        + RepositorioCopropiedad
        + RepositorioPersonas
        + RegistroOutbox
        + RegistroAuditoria,
{
    ctx.exigir(Permiso::ReservasCrear)?;
    let clave = s.idempotency_key.trim().to_owned();
    if clave.is_empty() || clave.len() > 200 {
        return Err(AppError::Validacion(
            "`idempotency_key` debe tener de 1 a 200 caracteres".into(),
        ));
    }
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    if let Some(existente) = tx.reserva_por_clave(&clave).await? {
        return Ok(existente);
    }
    let recurso = tx
        .recurso_para_reservar(s.recurso_id)
        .await?
        .ok_or(AppError::NoEncontrado("recurso"))?;
    let no_permitida = |m| AppError::Dominio(domain::DominioError::ReservaNoPermitida(m));
    if recurso.estado != EstadoRecurso::Disponible {
        return Err(no_permitida("el recurso no está disponible"));
    }
    let es_visitantes = recurso
        .parqueadero
        .as_ref()
        .is_some_and(|p| p.naturaleza == domain::NaturalezaParqueadero::Visitantes);
    if es_visitantes && s.placa_visitante.is_none() {
        return Err(no_permitida("indique la placa del visitante"));
    }
    let tz = zona(&mut tx).await?;
    recurso
        .politica
        .validar_franja(&s.franja, ahora, &recurso.horario, tz)?;

    // Quien reserva por un bien privado debe tener vínculo con él, salvo la administración.
    let gestiona = ctx.exigir(Permiso::ReservasAprobar).is_ok();
    let solicitante = match ctx.actor_id {
        Some(cuenta) => tx.vinculo_solicitante(cuenta, s.bien_privado_id).await?,
        None => None,
    };
    match (&solicitante, gestiona) {
        (Some((_, vinculo)), false) => recurso.politica.admite(vinculo)?,
        (None, false) => return Err(no_permitida("no tiene vínculo con ese bien privado")),
        _ => {}
    }
    let esencial = recurso.bien_comun.as_ref().is_some_and(|b| b.es_esencial);
    let restringido = tx
        .restricciones(Some(s.bien_privado_id))
        .await?
        .iter()
        .any(|r| r.desde <= ahora && r.hasta.is_none_or(|h| ahora < h));
    if restringido && !esencial {
        return Err(no_permitida(
            "el bien privado tiene restringidas las reservas (mora o sanción)",
        ));
    }

    // Límites por bien privado: mes calendario local de la franja y reservas futuras activas.
    let local = s.franja.inicio().with_timezone(&tz);
    let mes = NaiveDate::from_ymd_opt(local.year(), local.month(), 1)
        .ok_or(AppError::Validacion("fecha inválida".into()))?;
    let siguiente = mes
        .checked_add_months(chrono::Months::new(1))
        .ok_or(AppError::Validacion("fecha inválida".into()))?;
    let a_utc = |d: NaiveDate| -> AppResult<DateTime<Utc>> {
        tz.from_local_datetime(&d.and_time(chrono::NaiveTime::MIN))
            .earliest()
            .map(|x| x.with_timezone(&Utc))
            .ok_or(AppError::Validacion("fecha inválida".into()))
    };
    let del_mes = tx
        .contar_reservas_bien(
            s.recurso_id,
            s.bien_privado_id,
            a_utc(mes)?,
            a_utc(siguiente)?,
        )
        .await?;
    let futuras = tx
        .contar_reservas_bien(
            s.recurso_id,
            s.bien_privado_id,
            ahora,
            ahora + Duration::days(3650),
        )
        .await?;
    recurso
        .politica
        .validar_limites(del_mes, futuras, s.invitados)?;

    if let Some(aforo) = recurso.politica.aforo {
        let ocupadas: u32 = tx
            .reservas(&FiltroReservas {
                recurso_id: Some(s.recurso_id),
                entre: Some(s.franja),
                ..FiltroReservas::default()
            })
            .await?
            .iter()
            .filter(|r| r.estado.activa())
            .map(|r| r.invitados + 1)
            .sum();
        if ocupadas + s.invitados + 1 > aforo {
            return Err(no_permitida("no hay cupo en esa franja"));
        }
    }

    let valor = recurso.politica.liquidar(&s.franja, tz);
    let estado = if recurso.politica.modo == domain::ModoAsignacion::FranjaInmediata {
        EstadoReserva::Aprobada
    } else {
        EstadoReserva::Solicitada
    };
    let reserva = Reserva {
        id: ReservaId::nuevo(),
        recurso_id: s.recurso_id,
        bien_privado_id: s.bien_privado_id,
        solicitante_persona_id: solicitante.map(|(p, _)| p),
        solicitada_por: ctx.actor_id,
        franja: s.franja,
        exclusiva: recurso.politica.aforo.is_none(),
        invitados: s.invitados,
        estado,
        valor,
        deposito: recurso.politica.deposito,
        penalidad: Decimal::ZERO,
        estado_pago: if valor.is_zero() && recurso.politica.deposito.is_zero() {
            EstadoPago::NoAplica
        } else {
            EstadoPago::Pendiente
        },
        idempotency_key: clave,
        canal: s.canal,
        modalidad: s.modalidad,
        motivo: texto_opcional("motivo", s.motivo.as_deref(), 500)?,
        placa_visitante: s.placa_visitante,
        observacion: None,
        creada_en: ahora,
    };
    tx.insertar_reserva(&reserva).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("recurso_id", reserva.recurso_id.to_string())?
        .con("bien_privado_id", reserva.bien_privado_id.to_string())?
        .con("estado", reserva.estado.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_SOLICITAR, "reserva", reserva.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    if estado == EstadoReserva::Aprobada {
        avisar(
            &mut tx,
            reserva.solicitante_persona_id,
            PrioridadSalida::Confirmacion,
            "Reserva confirmada".into(),
            format!(
                "Su reserva quedó confirmada: {}.",
                texto_franja(&reserva, &recurso.nombre, tz)
            ),
        )
        .await?;
    }
    tx.confirmar().await?;
    Ok(reserva)
}

async fn cargar<T: RepositorioReservas>(
    tx: &mut T,
    id: ReservaId,
) -> AppResult<(Reserva, Recurso)> {
    let r = tx
        .reserva(id)
        .await?
        .ok_or(AppError::NoEncontrado("reserva"))?;
    let recurso = tx
        .recurso(r.recurso_id)
        .await?
        .ok_or(AppError::NoEncontrado("recurso"))?;
    Ok((r, recurso))
}

/// Aprueba (`aprobar = true`) o rechaza una reserva solicitada y avisa al solicitante.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `reservas.aprobar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si no está solicitada.
pub async fn resolver_reserva<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReservaId,
    aprobar: bool,
    motivo: Option<String>,
) -> AppResult<Reserva>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas
        + RepositorioModulos
        + RepositorioCopropiedad
        + RepositorioPersonas
        + RegistroOutbox
        + RegistroAuditoria,
{
    ctx.exigir(Permiso::ReservasAprobar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let (mut r, recurso) = cargar(&mut tx, id).await?;
    let hacia = if aprobar {
        EstadoReserva::Aprobada
    } else {
        EstadoReserva::Rechazada
    };
    r.estado = r.estado.transicionar(hacia)?;
    r.observacion = texto_opcional("motivo", motivo.as_deref(), 500)?;
    tx.actualizar_reserva(&r).await?;
    let tz = zona(&mut tx).await?;
    let (asunto, texto) = if aprobar {
        (
            "Reserva aprobada",
            format!(
                "Su reserva fue aprobada: {}.",
                texto_franja(&r, &recurso.nombre, tz)
            ),
        )
    } else {
        (
            "Reserva rechazada",
            format!(
                "Su reserva ({}) fue rechazada. {}",
                texto_franja(&r, &recurso.nombre, tz),
                r.observacion.clone().unwrap_or_default()
            ),
        )
    };
    avisar(
        &mut tx,
        r.solicitante_persona_id,
        PrioridadSalida::Evento,
        asunto.into(),
        texto,
    )
    .await?;
    let accion = if aprobar {
        ACCION_APROBAR
    } else {
        ACCION_RECHAZAR
    };
    tx.registrar(EventoAuditoria::ok(accion, "reserva", id.as_uuid()))
        .await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Cancela una reserva. Quien la solicitó paga la penalidad si cancela tarde (queda como
/// cargo pendiente); la administración (`reservas.cancelar_cualquiera`) cancela sin penalidad.
///
/// # Errors
///
/// [`AppError::SinPermiso`] si no es suya ni tiene `reservas.cancelar_cualquiera`;
/// [`AppError::NoEncontrado`]; [`AppError::Dominio`] si ya empezó o terminó.
pub async fn cancelar_reserva<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReservaId,
    motivo: Option<String>,
) -> AppResult<Reserva>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let (mut r, recurso) = cargar(&mut tx, id).await?;
    let propia = ctx.actor_id.is_some() && r.solicitada_por == ctx.actor_id;
    let administra = ctx.exigir(Permiso::ReservasCancelarCualquiera).is_ok();
    if !propia && !administra {
        return Err(AppError::SinPermiso(Permiso::ReservasCancelarCualquiera));
    }
    r.estado = r.estado.transicionar(EstadoReserva::Cancelada)?;
    r.observacion = texto_opcional("motivo", motivo.as_deref(), 500)?;
    if propia && !administra {
        r.penalidad = recurso
            .politica
            .penalidad(r.valor, r.franja.inicio(), ahora);
    }
    tx.actualizar_reserva(&r).await?;
    if r.penalidad > Decimal::ZERO {
        tx.insertar_cargo(&CargoPendiente {
            id: CargoId::nuevo(),
            bien_privado_id: r.bien_privado_id,
            concepto: ConceptoCargo::PenalidadCancelacion,
            descripcion: Some(format!("Cancelación tardía: {}", recurso.nombre)),
            valor: r.penalidad,
            reserva_id: Some(r.id),
            creado_en: ahora,
        })
        .await?;
    }
    let detalle = DetalleAuditoria::vacio().con("penalidad", r.penalidad.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CANCELAR, "reserva", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Datos de una entrega o un recibo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatosEntrega {
    pub checklist: Vec<ElementoChecklist>,
    pub fotos: Vec<ArchivoId>,
    pub novedad: Option<String>,
    /// Solo en el recibo: valor estimado de los daños (genera un cargo pendiente).
    pub valor_danos: Option<Decimal>,
}

/// Registra la entrega (aprobada → en curso) o el recibo (en curso → finalizada, o con novedad
/// si hay novedad o daños; los daños quedan como cargo pendiente del bien privado).
///
/// Un recibo con novedad radica además un ticket de mantenimiento (plan 10, fase 3) si la PH
/// tiene la gestión administrativa: idempotente por reserva, con las fotos del recibo como
/// adjuntos. Si no se puede radicar, el recibo igual queda registrado.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `reservas.entregas.registrar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si el estado no lo permite; [`AppError::Validacion`] si el valor de
/// los daños no es positivo o viene en una entrega.
pub async fn registrar_entrega<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReservaId,
    tipo: TipoEntrega,
    datos: DatosEntrega,
) -> AppResult<Reserva>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + crate::casos_uso::tickets::TxTickets,
{
    ctx.exigir(Permiso::ReservasEntregasRegistrar)?;
    if datos.valor_danos.is_some_and(|v| v <= Decimal::ZERO)
        || (tipo == TipoEntrega::Entrega && datos.valor_danos.is_some())
    {
        return Err(AppError::Validacion(
            "el valor de los daños es positivo y solo va en el recibo".into(),
        ));
    }
    let ahora = reloj.ahora();
    let novedad = texto_opcional("novedad", datos.novedad.as_deref(), 2000)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let (mut r, recurso) = cargar(&mut tx, id).await?;
    let hacia = match tipo {
        TipoEntrega::Entrega => EstadoReserva::EnCurso,
        TipoEntrega::Recibo if novedad.is_some() || datos.valor_danos.is_some() => {
            EstadoReserva::ConNovedad
        }
        TipoEntrega::Recibo => EstadoReserva::Finalizada,
    };
    r.estado = r.estado.transicionar(hacia)?;
    tx.actualizar_reserva(&r).await?;
    tx.insertar_entrega(&Entrega {
        reserva_id: id,
        tipo,
        checklist: datos.checklist,
        fotos: datos.fotos.clone(),
        novedad: novedad.clone(),
        valor_danos: datos.valor_danos,
        registrado_por: ctx.actor_id,
        registrado_en: ahora,
    })
    .await?;
    if let Some(valor) = datos.valor_danos {
        tx.insertar_cargo(&CargoPendiente {
            id: CargoId::nuevo(),
            bien_privado_id: r.bien_privado_id,
            concepto: ConceptoCargo::Danos,
            descripcion: Some(format!("Daños en {}", recurso.nombre)),
            valor,
            reserva_id: Some(id),
            creado_en: ahora,
        })
        .await?;
    }
    let accion = match tipo {
        TipoEntrega::Entrega => ACCION_ENTREGA,
        TipoEntrega::Recibo => ACCION_RECIBO,
    };
    let detalle = DetalleAuditoria::vacio().con("estado", r.estado.as_str())?;
    tx.registrar(EventoAuditoria::ok(accion, "reserva", id.as_uuid()).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    if r.estado == EstadoReserva::ConNovedad {
        let n = ticket_por_novedad(
            &r,
            &recurso.nombre,
            novedad.as_deref(),
            datos.valor_danos,
            datos.fotos,
        );
        if let Err(e) = radicar_por_novedad(unidad, reloj, ctx, n).await {
            tracing::warn!(tenant_id = %ctx.tenant_id, reserva_id = %id, error = %e, "no se radicó el ticket de la novedad");
        }
    }
    Ok(r)
}

/// Ticket de mantenimiento para la novedad del recibo de una reserva.
fn ticket_por_novedad(
    r: &Reserva,
    recurso: &str,
    novedad: Option<&str>,
    valor_danos: Option<Decimal>,
    fotos: Vec<ArchivoId>,
) -> crate::casos_uso::tickets::NuevoTicket {
    let mut descripcion = format!("Novedad al recibir «{recurso}» tras la reserva {}.", r.id);
    if let Some(n) = novedad {
        descripcion.push_str(&format!(" Novedad: {}", n.replace(char::is_control, " ")));
    }
    if let Some(v) = valor_danos {
        descripcion.push_str(&format!(
            " Daños estimados: {v} (cargo pendiente del bien privado)."
        ));
    }
    crate::casos_uso::tickets::NuevoTicket {
        tipo: domain::TipoTicket::SolicitudMantenimiento,
        categoria_id: None,
        prioridad: None,
        canal: domain::CanalTicket::Presencial,
        asunto: format!(
            "Novedad en {}",
            recurso.chars().take(100).collect::<String>()
        ),
        descripcion,
        bien_privado_id: Some(r.bien_privado_id),
        ubicacion: None,
        recurso_id: Some(r.recurso_id),
        activo_id: None,
        adjuntos: fotos
            .into_iter()
            .take(crate::casos_uso::tickets::MAX_ADJUNTOS)
            .collect(),
        idempotency_key: format!("reserva-novedad:{}", r.id),
        solicitante: None,
    }
}

/// Radica el ticket a nombre de quien registró el recibo. Sin la gestión administrativa no se
/// radica (no es un error).
async fn radicar_por_novedad<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: crate::casos_uso::tickets::NuevoTicket,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: crate::casos_uso::tickets::TxTickets,
{
    if ctx.actor_id.is_none() {
        return Ok(());
    }
    // Quien registra el recibo radica el ticket aunque su rol no tenga `tickets.crear`: es una
    // consecuencia del recibo, no una solicitud suya.
    let interno = Contexto {
        permisos: domain::ConjuntoPermisos::todos(domain::Ambito::Copropiedad),
        ..ctx.clone()
    };
    match crate::casos_uso::tickets::radicar_ticket(unidad, reloj, &interno, n).await {
        Ok(_) | Err(AppError::ModuloNoHabilitado(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Marca que el solicitante no asistió a una reserva aprobada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `reservas.entregas.registrar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si no está aprobada.
pub async fn marcar_no_asistio<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReservaId,
) -> AppResult<Reserva>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    ctx.exigir(Permiso::ReservasEntregasRegistrar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let (mut r, _) = cargar(&mut tx, id).await?;
    r.estado = r.estado.transicionar(EstadoReserva::NoAsistio)?;
    tx.actualizar_reserva(&r).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_NO_ASISTIO,
        "reserva",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Registra a mano el estado del pago (pagado o exonerado).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `reservas.aprobar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si la reserva no tiene valor que pagar.
pub async fn registrar_pago<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReservaId,
    estado: EstadoPago,
) -> AppResult<Reserva>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    ctx.exigir(Permiso::ReservasAprobar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let (mut r, _) = cargar(&mut tx, id).await?;
    if r.estado_pago == EstadoPago::NoAplica || estado == EstadoPago::NoAplica {
        return Err(AppError::Validacion(
            "la reserva no tiene valor que pagar".into(),
        ));
    }
    r.estado_pago = estado;
    tx.actualizar_reserva(&r).await?;
    let detalle = DetalleAuditoria::vacio().con("estado_pago", estado.as_str())?;
    tx.registrar(EventoAuditoria::ok(ACCION_PAGO, "reserva", id.as_uuid()).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Reservas: todas (con `reservas.ver_todas`) o las que pidió la cuenta (`solo_mias`).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `reservas.ver_todas` para ver las de todos;
/// [`AppError::NoAutenticado`] sin cuenta para `solo_mias`.
pub async fn listar_reservas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    mut filtro: FiltroReservas,
    solo_mias: bool,
) -> AppResult<Vec<Reserva>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos,
{
    if solo_mias {
        filtro.solicitada_por = Some(ctx.actor_id.ok_or(AppError::NoAutenticado)?);
    } else {
        ctx.exigir(Permiso::ReservasVerTodas)?;
    }
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    tx.reservas(&filtro).await
}

/// Una reserva con su entrega y su recibo (de la cuenta o con `reservas.ver_todas`).
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si no existe o no es visible para la cuenta.
pub async fn ver_reserva<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReservaId,
) -> AppResult<(Reserva, Vec<Entrega>)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos,
{
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let r = tx
        .reserva(id)
        .await?
        .ok_or(AppError::NoEncontrado("reserva"))?;
    let visible = ctx.exigir(Permiso::ReservasVerTodas).is_ok()
        || ctx.exigir(Permiso::ReservasEntregasRegistrar).is_ok()
        || (ctx.actor_id.is_some() && r.solicitada_por == ctx.actor_id);
    if !visible {
        return Err(AppError::NoEncontrado("reserva"));
    }
    let entregas = tx.entregas(id).await?;
    Ok((r, entregas))
}

// ---------------------------------------------------------------------------------------------
// Restricciones y autorización de tenedores
// ---------------------------------------------------------------------------------------------

/// Restringe las reservas de un bien privado (mora o sanción) desde ahora hasta `hasta` (o
/// hasta levantarla). No aplica a los bienes comunes esenciales.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `reservas.restringir`; errores de validación.
pub async fn restringir_bien<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    motivo: &str,
    soporte: Option<ArchivoId>,
    hasta: Option<DateTime<Utc>>,
) -> AppResult<Restriccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    ctx.exigir(Permiso::ReservasRestringir)?;
    let ahora = reloj.ahora();
    if hasta.is_some_and(|h| h <= ahora) {
        return Err(AppError::Validacion("`hasta` debe ser futura".into()));
    }
    let r = Restriccion {
        id: RestriccionId::nuevo(),
        bien_privado_id: bien,
        motivo: texto_obligatorio("motivo", motivo, 500)?,
        soporte_archivo_id: soporte,
        desde: ahora,
        hasta,
        creada_por: ctx.actor_id,
    };
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    tx.insertar_restriccion(&r).await?;
    let detalle = DetalleAuditoria::vacio().con("bien_privado_id", bien.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RESTRINGIR, "restriccion_reserva", r.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Levanta una restricción desde ahora.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `reservas.restringir`; [`AppError::NoEncontrado`].
pub async fn levantar_restriccion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: RestriccionId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos + RegistroAuditoria,
{
    ctx.exigir(Permiso::ReservasRestringir)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    if !tx.levantar_restriccion(id, reloj.ahora()).await? {
        return Err(AppError::NoEncontrado("restricción vigente"));
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_LEVANTAR,
        "restriccion_reserva",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Restricciones (de un bien o todas).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `reservas.restringir`.
pub async fn listar_restricciones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: Option<BienPrivadoId>,
) -> AppResult<Vec<Restriccion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioReservas + RepositorioModulos,
{
    ctx.exigir(Permiso::ReservasRestringir)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    tx.restricciones(bien).await
}

/// El propietario (titular vigente del bien) o la administración (`censo.editar`) autoriza —o
/// retira la autorización— para que un tenedor reserve, cuando la política lo exige.
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si la residencia no existe o no es de un tenedor vigente;
/// [`AppError::SinPermiso`] si no es titular del bien ni tiene `censo.editar`.
pub async fn autorizar_reservas_tenedor<U>(
    unidad: &U,
    ctx: &Contexto,
    residencia: ResidenciaId,
    autorizadas: bool,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion:
        RepositorioReservas + RepositorioCenso + RepositorioTitulares + RegistroAuditoria,
{
    let mut tx = unidad.iniciar(ctx).await?;
    let r = tx
        .residencia(residencia)
        .await?
        .filter(|r| r.vigente_hasta.is_none() && r.calidad == domain::CalidadResidencia::Tenedor)
        .ok_or(AppError::NoEncontrado("residencia de tenedor vigente"))?;
    let es_titular = match ctx.actor_id {
        Some(c) => tx.bienes_de_cuenta(c).await?.contains(&r.bien_privado_id),
        None => false,
    };
    if !es_titular {
        ctx.exigir(Permiso::CensoEditar)?;
    }
    tx.autorizar_reservas(residencia, autorizadas).await?;
    let detalle = DetalleAuditoria::vacio().con("autorizadas", autorizadas)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_AUTORIZAR_TENEDOR, "residencia", residencia.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await
}

// ---------------------------------------------------------------------------------------------
// Recordatorios
// ---------------------------------------------------------------------------------------------

/// Encola el recordatorio (24 h antes) de las reservas aprobadas de la copropiedad del contexto
/// que aún no lo tienen. Lo llama el worker del gateway. Devuelve cuántas recordó.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn encolar_recordatorios<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<u32>
where
    U: UnidadDeTrabajo,
    U::Transaccion:
        RepositorioReservas + RepositorioCopropiedad + RepositorioPersonas + RegistroOutbox,
{
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    let tz = zona(&mut tx).await?;
    let pendientes = tx
        .reservas_por_recordar(ahora, ahora + Duration::hours(HORAS_RECORDATORIO))
        .await?;
    let mut n = 0;
    for r in pendientes {
        let nombre = tx
            .recurso(r.recurso_id)
            .await?
            .map_or_else(|| "Reserva".to_owned(), |x| x.nombre);
        avisar(
            &mut tx,
            r.solicitante_persona_id,
            PrioridadSalida::Recordatorio,
            "Recordatorio de su reserva".into(),
            format!(
                "Le recordamos su reserva: {}.",
                texto_franja(&r, &nombre, tz)
            ),
        )
        .await?;
        tx.marcar_recordada(r.id, ahora).await?;
        n += 1;
    }
    tx.confirmar().await?;
    Ok(n)
}

/// Recorre las copropiedades con recordatorios pendientes y los encola en cada una. Un fallo en
/// una copropiedad no detiene a las demás (se registra y sigue). Devuelve cuántos encoló.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si no se puede consultar el directorio.
pub async fn recordar_reservas<D, U>(
    directorio: &D,
    unidad: &U,
    reloj: &impl Reloj,
) -> AppResult<u32>
where
    D: crate::puertos::DirectorioRecordatorios,
    U: UnidadDeTrabajo,
    U::Transaccion:
        RepositorioReservas + RepositorioCopropiedad + RepositorioPersonas + RegistroOutbox,
{
    let hasta = reloj.ahora() + Duration::hours(HORAS_RECORDATORIO);
    let mut total = 0;
    for tenant in directorio.copropiedades_con_recordatorios(hasta).await? {
        match encolar_recordatorios(unidad, reloj, &Contexto::new(tenant)).await {
            Ok(n) => total += n,
            Err(e) => {
                tracing::error!(tenant_id = %tenant, error = %e, "no se pudieron encolar recordatorios")
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
pub(crate) mod tests {
    use chrono::{NaiveTime, Weekday};
    use domain::{
        CalidadResidencia, CategoriaBienComun, CuentaId, DireccionCorreo, Documento, DominioError,
        FranjaSemanal, Porcentaje, TenantId, TipoPersona, TipoTitularidad,
    };
    use rust_decimal::dec;

    use super::*;
    use crate::{
        casos_uso::{
            bienes_privados::{crear_bien_privado, tests::nuevo_bien},
            censo::{NuevaResidencia, registrar_residencia},
            titulares::{DatosPersona, RefPersona, SolicitudTitular, registrar_titulares},
        },
        puertos::{reloj::RelojFijo, unidad_trabajo::memoria::UnidadEnMemoria},
    };

    const BOGOTA: Tz = Tz::America__Bogota;

    pub(crate) fn local(d: u32, h: u32) -> DateTime<Utc> {
        BOGOTA
            .with_ymd_and_hms(2026, 11, d, h, 0, 0)
            .single()
            .expect("hora válida")
            .with_timezone(&Utc)
    }

    pub(crate) fn reloj() -> RelojFijo {
        RelojFijo(local(7, 10))
    }

    pub(crate) fn todos_los_dias() -> Horario {
        Horario::new(
            [
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
                Weekday::Sat,
                Weekday::Sun,
            ]
            .iter()
            .map(|d| FranjaSemanal {
                dia: *d,
                inicio: NaiveTime::from_hms_opt(8, 0, 0).expect("hora"),
                fin: NaiveTime::from_hms_opt(22, 0, 0).expect("hora"),
                aplica_festivos: true,
            })
            .collect(),
            vec![],
        )
        .expect("válido")
    }

    pub(crate) struct Entorno {
        pub unidad: UnidadEnMemoria,
        pub ctx: Contexto,
        pub bien: BienPrivadoId,
        pub dueno: Contexto,
    }

    pub(crate) async fn entorno() -> Entorno {
        let unidad = UnidadEnMemoria::default();
        unidad.habilitar(Modulo::BienesComunes);
        let ctx = Contexto::new(TenantId::nuevo());
        let bien = crear_bien_privado(&unidad, &ctx, nuevo_bien("101", None))
            .await
            .expect("bien")
            .id;
        let cuenta = CuentaId::nuevo();
        registrar_titulares(
            &unidad,
            &reloj(),
            &ctx,
            bien,
            vec![SolicitudTitular {
                persona: RefPersona::Nueva(DatosPersona {
                    tipo: TipoPersona::Natural,
                    nombre: "Ana".into(),
                    documento: Documento::new("111").expect("válido"),
                    telefono: None,
                    correo: Some(DireccionCorreo::new("ana@correo.co").expect("válido")),
                    cuenta_id: Some(cuenta),
                    tipo_documento: None,
                }),
                tipo: TipoTitularidad::Propietario,
                porcentaje: Porcentaje::new(dec!(100)).expect("válido"),
                es_representante: true,
            }],
        )
        .await
        .expect("titular");
        let dueno = Contexto::de_cuenta(
            ctx.tenant_id,
            cuenta,
            [Permiso::ReservasCrear, Permiso::BienesComunesVer]
                .into_iter()
                .collect(),
        );
        Entorno {
            unidad,
            ctx,
            bien,
            dueno,
        }
    }

    pub(crate) async fn salon(e: &Entorno, politica: PoliticaReserva, esencial: bool) -> Recurso {
        crear_recurso(
            &e.unidad,
            &reloj(),
            &e.ctx,
            NuevoRecurso {
                tipo: TipoRecurso::BienComun,
                nombre: "Salón social".into(),
                ubicacion: None,
                descripcion: None,
                politica,
                horario: todos_los_dias(),
                bien_comun: Some(DetalleBienComun {
                    categoria: CategoriaBienComun::SalonSocial,
                    area_m2: None,
                    es_esencial: esencial,
                    reglas: None,
                    reglas_archivo_id: None,
                }),
            },
        )
        .await
        .expect("recurso")
    }

    pub(crate) fn solicitud(
        e: &Entorno,
        r: &Recurso,
        d: u32,
        h1: u32,
        h2: u32,
        clave: &str,
    ) -> SolicitudReserva {
        SolicitudReserva {
            recurso_id: r.id,
            bien_privado_id: e.bien,
            franja: Franja::new(local(d, h1), local(d, h2)).expect("válida"),
            invitados: 0,
            idempotency_key: clave.into(),
            canal: CanalReserva::Web,
            modalidad: None,
            motivo: None,
            placa_visitante: None,
        }
    }

    fn no_permitida<T: std::fmt::Debug>(r: AppResult<T>) -> bool {
        matches!(
            r,
            Err(AppError::Dominio(DominioError::ReservaNoPermitida(_)))
        )
    }

    #[tokio::test]
    async fn sin_modulo_no_hay_reservas() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        assert!(matches!(
            listar_recursos(&unidad, &reloj(), &ctx, None).await,
            Err(AppError::ModuloNoHabilitado(Modulo::BienesComunes))
        ));
    }

    #[tokio::test]
    async fn solicitar_es_idempotente_sin_solapes_y_se_aprueba_con_aviso() {
        let e = entorno().await;
        let r = salon(&e, PoliticaReserva::default(), false).await;
        let a = solicitar_reserva(
            &e.unidad,
            &reloj(),
            &e.dueno,
            solicitud(&e, &r, 20, 14, 16, "k1"),
        )
        .await
        .expect("solicita");
        assert_eq!(a.estado, EstadoReserva::Solicitada);
        let otra_vez = solicitar_reserva(
            &e.unidad,
            &reloj(),
            &e.dueno,
            solicitud(&e, &r, 20, 14, 16, "k1"),
        )
        .await
        .expect("reintento");
        assert_eq!(otra_vez.id, a.id);
        assert!(matches!(
            solicitar_reserva(
                &e.unidad,
                &reloj(),
                &e.ctx,
                solicitud(&e, &r, 20, 15, 17, "k2")
            )
            .await,
            Err(AppError::Conflicto(_))
        ));
        resolver_reserva(&e.unidad, &reloj(), &e.ctx, a.id, true, None)
            .await
            .expect("aprueba");
        let avisos = e.unidad.outbox();
        assert_eq!(avisos.len(), 1);
        assert_eq!(avisos[0].1.mensaje.destino(), "ana@correo.co");
    }

    #[tokio::test]
    async fn limites_restricciones_y_bienes_esenciales() {
        let e = entorno().await;
        let r = salon(
            &e,
            PoliticaReserva {
                max_reservas_mes: Some(1),
                ..PoliticaReserva::default()
            },
            false,
        )
        .await;
        solicitar_reserva(
            &e.unidad,
            &reloj(),
            &e.dueno,
            solicitud(&e, &r, 20, 14, 16, "a"),
        )
        .await
        .expect("primera");
        assert!(no_permitida(
            solicitar_reserva(
                &e.unidad,
                &reloj(),
                &e.dueno,
                solicitud(&e, &r, 21, 14, 16, "b")
            )
            .await
        ));

        restringir_bien(
            &e.unidad,
            &reloj(),
            &e.ctx,
            e.bien,
            "Mora de 3 meses",
            None,
            None,
        )
        .await
        .expect("restringe");
        let otro = salon(&e, PoliticaReserva::default(), false).await;
        assert!(no_permitida(
            solicitar_reserva(
                &e.unidad,
                &reloj(),
                &e.dueno,
                solicitud(&e, &otro, 22, 14, 16, "c")
            )
            .await
        ));
        let esencial = salon(&e, PoliticaReserva::default(), true).await;
        solicitar_reserva(
            &e.unidad,
            &reloj(),
            &e.dueno,
            solicitud(&e, &esencial, 22, 14, 16, "d"),
        )
        .await
        .expect("art. 59: lo esencial no se restringe");
    }

    #[tokio::test]
    async fn el_aforo_limita_las_personas_por_franja() {
        let e = entorno().await;
        let gimnasio = salon(
            &e,
            PoliticaReserva {
                modo: domain::ModoAsignacion::FranjaInmediata,
                aforo: Some(3),
                max_invitados: 5,
                ..PoliticaReserva::default()
            },
            false,
        )
        .await;
        let r = solicitar_reserva(
            &e.unidad,
            &reloj(),
            &e.ctx,
            SolicitudReserva {
                invitados: 1,
                ..solicitud(&e, &gimnasio, 20, 8, 9, "g1")
            },
        )
        .await
        .expect("2 personas");
        assert_eq!(r.estado, EstadoReserva::Aprobada);
        assert!(no_permitida(
            solicitar_reserva(
                &e.unidad,
                &reloj(),
                &e.ctx,
                SolicitudReserva {
                    invitados: 1,
                    ..solicitud(&e, &gimnasio, 20, 8, 9, "g2")
                }
            )
            .await
        ));
        solicitar_reserva(
            &e.unidad,
            &reloj(),
            &e.ctx,
            solicitud(&e, &gimnasio, 20, 8, 9, "g3"),
        )
        .await
        .expect("cabe una persona más");
    }

    /// Reserva aprobada y entregada del salón; devuelve el contexto de la administración (con
    /// cuenta) y la reserva.
    async fn reserva_en_curso(e: &Entorno) -> (Contexto, Reserva) {
        let r = salon(e, PoliticaReserva::default(), false).await;
        let b = solicitar_reserva(
            &e.unidad,
            &reloj(),
            &e.dueno,
            solicitud(e, &r, 20, 14, 16, "nov"),
        )
        .await
        .expect("solicita");
        let admin = Contexto::de_cuenta(
            e.ctx.tenant_id,
            domain::CuentaId::nuevo(),
            domain::ConjuntoPermisos::todos(domain::Ambito::Copropiedad),
        );
        if b.estado == EstadoReserva::Solicitada {
            resolver_reserva(&e.unidad, &reloj(), &admin, b.id, true, None)
                .await
                .expect("aprueba");
        }
        let b = registrar_entrega(
            &e.unidad,
            &reloj(),
            &admin,
            b.id,
            TipoEntrega::Entrega,
            DatosEntrega::default(),
        )
        .await
        .expect("entrega");
        (admin, b)
    }

    fn con_novedad() -> DatosEntrega {
        DatosEntrega {
            novedad: Some("Mesa rota".into()),
            valor_danos: Some(dec!(80000)),
            ..DatosEntrega::default()
        }
    }

    #[tokio::test]
    async fn la_novedad_del_recibo_radica_un_ticket_de_mantenimiento() {
        let e = entorno().await;
        e.unidad.habilitar(Modulo::GestionAdministrativa);
        let (admin, b) = reserva_en_curso(&e).await;

        registrar_entrega(
            &e.unidad,
            &reloj(),
            &admin,
            b.id,
            TipoEntrega::Recibo,
            con_novedad(),
        )
        .await
        .expect("recibo");

        let tickets = e.unidad.datos().gestion.tickets;
        assert_eq!(tickets.len(), 1);
        let t = &tickets[0];
        assert_eq!(t.tipo, domain::TipoTicket::SolicitudMantenimiento);
        assert_eq!(t.recurso_id, Some(b.recurso_id));
        assert_eq!(t.bien_privado_id, Some(b.bien_privado_id));
        assert_eq!(t.solicitante_cuenta_id, admin.actor_id);
        assert!(t.descripcion.contains("Mesa rota") && t.descripcion.contains("80000"));
        assert_eq!(t.idempotency_key, format!("reserva-novedad:{}", b.id));
    }

    #[tokio::test]
    async fn sin_gestion_administrativa_el_recibo_queda_sin_ticket() {
        let e = entorno().await;
        let (admin, b) = reserva_en_curso(&e).await;

        let r = registrar_entrega(
            &e.unidad,
            &reloj(),
            &admin,
            b.id,
            TipoEntrega::Recibo,
            con_novedad(),
        )
        .await
        .expect("el recibo no falla");

        assert_eq!(r.estado, EstadoReserva::ConNovedad);
        assert!(e.unidad.datos().gestion.tickets.is_empty());
    }

    #[tokio::test]
    async fn cancelar_tarde_genera_penalidad_y_los_danos_un_cargo() {
        let e = entorno().await;
        let r = salon(
            &e,
            PoliticaReserva {
                tarifa: dec!(100000),
                cancelacion_sin_penalidad_horas: 72,
                penalidad_porcentaje: dec!(50),
                ..PoliticaReserva::default()
            },
            false,
        )
        .await;
        let a = solicitar_reserva(
            &e.unidad,
            &reloj(),
            &e.dueno,
            solicitud(&e, &r, 8, 14, 16, "p"),
        )
        .await
        .expect("solicita");
        assert_eq!(
            (a.valor, a.estado_pago),
            (dec!(100000), EstadoPago::Pendiente)
        );
        let c = cancelar_reserva(&e.unidad, &reloj(), &e.dueno, a.id, None)
            .await
            .expect("cancela");
        assert_eq!(c.penalidad, dec!(50000));

        let b = solicitar_reserva(
            &e.unidad,
            &reloj(),
            &e.dueno,
            solicitud(&e, &r, 20, 14, 16, "q"),
        )
        .await
        .expect("solicita");
        resolver_reserva(&e.unidad, &reloj(), &e.ctx, b.id, true, None)
            .await
            .expect("aprueba");
        registrar_entrega(
            &e.unidad,
            &reloj(),
            &e.ctx,
            b.id,
            TipoEntrega::Entrega,
            DatosEntrega::default(),
        )
        .await
        .expect("entrega");
        let recibida = registrar_entrega(
            &e.unidad,
            &reloj(),
            &e.ctx,
            b.id,
            TipoEntrega::Recibo,
            DatosEntrega {
                novedad: Some("Mesa rota".into()),
                valor_danos: Some(dec!(80000)),
                ..DatosEntrega::default()
            },
        )
        .await
        .expect("recibo");
        assert_eq!(recibida.estado, EstadoReserva::ConNovedad);
        let cargos = e.unidad.datos().cargos;
        assert_eq!(
            cargos
                .iter()
                .map(|c| (c.concepto, c.valor))
                .collect::<Vec<_>>(),
            vec![
                (ConceptoCargo::PenalidadCancelacion, dec!(50000)),
                (ConceptoCargo::Danos, dec!(80000))
            ]
        );
    }

    #[tokio::test]
    async fn mantenimiento_cancela_las_reservas_futuras() {
        let e = entorno().await;
        let r = salon(&e, PoliticaReserva::default(), false).await;
        solicitar_reserva(
            &e.unidad,
            &reloj(),
            &e.dueno,
            solicitud(&e, &r, 20, 14, 16, "m"),
        )
        .await
        .expect("solicita");
        let n = cambiar_estado_recurso(
            &e.unidad,
            &reloj(),
            &e.ctx,
            r.id,
            EstadoRecurso::EnMantenimiento,
            Some("Pintura".into()),
        )
        .await
        .expect("cierra");
        assert_eq!(n, 1);
        assert!(no_permitida(
            solicitar_reserva(
                &e.unidad,
                &reloj(),
                &e.dueno,
                solicitud(&e, &r, 21, 14, 16, "n")
            )
            .await
        ));
        assert!(
            disponibilidad(
                &e.unidad,
                &reloj(),
                &e.ctx,
                r.id,
                local(21, 0).date_naive(),
                local(21, 0).date_naive()
            )
            .await
            .expect("consulta")
            .is_empty()
        );
    }

    #[tokio::test]
    async fn el_tenedor_necesita_autorizacion_si_la_politica_lo_exige() {
        let e = entorno().await;
        let r = salon(
            &e,
            PoliticaReserva {
                tenedor_requiere_autorizacion: true,
                ..PoliticaReserva::default()
            },
            false,
        )
        .await;
        let dueno = e
            .unidad
            .datos()
            .titularidades
            .first()
            .map(|t| t.persona_id)
            .expect("titular");
        let cuenta = CuentaId::nuevo();
        let residencia = registrar_residencia(
            &e.unidad,
            &reloj(),
            &e.ctx,
            e.bien,
            NuevaResidencia {
                persona: RefPersona::Nueva(DatosPersona {
                    tipo: TipoPersona::Natural,
                    nombre: "Luis".into(),
                    documento: Documento::new("222").expect("válido"),
                    telefono: None,
                    correo: None,
                    cuenta_id: Some(cuenta),
                    tipo_documento: None,
                }),
                calidad: CalidadResidencia::Tenedor,
                es_responsable: false,
                autorizado_por: Some(dueno),
                contrato_archivo_id: None,
            },
        )
        .await
        .expect("tenedor");
        let tenedor = Contexto::de_cuenta(
            e.ctx.tenant_id,
            cuenta,
            [Permiso::ReservasCrear].into_iter().collect(),
        );
        assert!(no_permitida(
            solicitar_reserva(
                &e.unidad,
                &reloj(),
                &tenedor,
                solicitud(&e, &r, 20, 14, 16, "t1")
            )
            .await
        ));
        autorizar_reservas_tenedor(&e.unidad, &e.dueno, residencia.id, true)
            .await
            .expect("el propietario autoriza");
        solicitar_reserva(
            &e.unidad,
            &reloj(),
            &tenedor,
            solicitud(&e, &r, 20, 14, 16, "t2"),
        )
        .await
        .expect("autorizado");
    }

    #[tokio::test]
    async fn recordatorio_una_sola_vez_el_dia_antes() {
        let e = entorno().await;
        let r = salon(
            &e,
            PoliticaReserva {
                modo: domain::ModoAsignacion::FranjaInmediata,
                ..PoliticaReserva::default()
            },
            false,
        )
        .await;
        solicitar_reserva(
            &e.unidad,
            &reloj(),
            &e.dueno,
            solicitud(&e, &r, 8, 9, 10, "r1"),
        )
        .await
        .expect("mañana");
        solicitar_reserva(
            &e.unidad,
            &reloj(),
            &e.dueno,
            solicitud(&e, &r, 20, 9, 10, "r2"),
        )
        .await
        .expect("en dos semanas");
        let antes = e.unidad.outbox().len();
        assert_eq!(
            encolar_recordatorios(&e.unidad, &reloj(), &e.ctx)
                .await
                .expect("encola"),
            1
        );
        assert_eq!(e.unidad.outbox().len(), antes + 1);
        assert_eq!(
            encolar_recordatorios(&e.unidad, &reloj(), &e.ctx)
                .await
                .expect("encola"),
            0
        );
    }
}
