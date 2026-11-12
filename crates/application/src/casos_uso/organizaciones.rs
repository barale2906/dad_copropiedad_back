//! Organizaciones compartidas y contratos de servicio.
//!
//! - Desde una copropiedad: buscar por NIT, crear (si no existe), vincular con un contrato,
//!   asignar roles al personal **derivados del contrato** y terminar el contrato (vencen esos
//!   roles).
//! - Desde la plataforma: editar la ficha global, agregar miembros, reclamar la organización y
//!   revisar reclamaciones (siempre soporte).

use chrono::{DateTime, Utc};
use domain::{
    ContratoServicioId, CuentaId, EstadoContrato, EstadoOrganizacion, EstadoReclamacion, Nit,
    OrganizacionId, OrigenAsignacion, Permiso, ReclamacionId, RolId, RolOrganizacion,
    TipoOrganizacion,
};
use rust_decimal::Decimal;

use crate::{
    AppError, AppResult, Contexto, ContextoPlataforma,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::roles::{SolicitudAsignacion, asignar_en},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioContratos, RepositorioOrganizaciones,
        RepositorioRoles, Transaccion, UnidadDeTrabajo, UnidadDeTrabajoPlataforma,
        identidad::{
            Asignacion, CambiosFicha, ContratoServicio, FichaOrganizacion, NuevaOrganizacion,
            Reclamacion,
        },
    },
};

pub(crate) const ACCION_CREAR: AccionAuditoria =
    AccionAuditoria::new("organizaciones.organizacion.crear");
pub(crate) const ACCION_VINCULAR: AccionAuditoria =
    AccionAuditoria::new("organizaciones.contrato.crear");
pub(crate) const ACCION_TERMINAR: AccionAuditoria =
    AccionAuditoria::new("organizaciones.contrato.terminar");
pub(crate) const ACCION_EDITAR_FICHA: AccionAuditoria =
    AccionAuditoria::new("organizaciones.ficha.editar");
pub(crate) const ACCION_MIEMBRO: AccionAuditoria =
    AccionAuditoria::new("organizaciones.miembro.guardar");
pub(crate) const ACCION_RECLAMAR: AccionAuditoria =
    AccionAuditoria::new("organizaciones.reclamacion.crear");
pub(crate) const ACCION_RESOLVER: AccionAuditoria =
    AccionAuditoria::new("organizaciones.reclamacion.resolver");

/// Busca la ficha pública por NIT exacto.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `organizaciones.vincular` ni `organizaciones.crear`.
pub async fn buscar_por_nit<U>(
    unidad: &U,
    ctx: &Contexto,
    nit: &Nit,
) -> AppResult<Option<FichaOrganizacion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioContratos,
{
    ctx.exigir(Permiso::OrganizacionesVincular)
        .or_else(|_| ctx.exigir(Permiso::OrganizacionesCrear))?;
    unidad
        .iniciar(ctx)
        .await?
        .buscar_organizacion_por_nit(nit)
        .await
}

pub(crate) fn validar_nueva(nueva: &NuevaOrganizacion) -> AppResult<()> {
    let razon = nueva.razon_social.trim();
    if razon.is_empty() || razon.chars().count() > 200 {
        return Err(AppError::Validacion(
            "la razón social debe tener entre 1 y 200 caracteres".into(),
        ));
    }
    if nueva.tipos.is_empty() {
        return Err(AppError::Validacion("indique al menos un tipo".into()));
    }
    Ok(())
}

/// Crea la organización o devuelve la que ya existe con ese NIT (`bool` = la creó). Nunca hay
/// dos con el mismo NIT, aunque dos copropiedades la creen a la vez.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `organizaciones.crear`; [`AppError::Validacion`].
pub async fn crear_organizacion<U>(
    unidad: &U,
    ctx: &Contexto,
    nueva: NuevaOrganizacion,
) -> AppResult<(OrganizacionId, bool)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioContratos + RegistroAuditoria,
{
    ctx.exigir(Permiso::OrganizacionesCrear)?;
    validar_nueva(&nueva)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let (id, creada) = tx
        .crear_organizacion(OrganizacionId::nuevo(), &nueva, ctx.actor_id)
        .await?;
    if creada {
        tx.registrar(EventoAuditoria::ok(
            ACCION_CREAR,
            "organizacion",
            id.as_uuid(),
        ))
        .await?;
    }
    tx.confirmar().await?;
    Ok((id, creada))
}

/// Datos de un contrato nuevo.
#[derive(Debug, Clone)]
pub struct NuevoContrato {
    pub organizacion_id: OrganizacionId,
    pub tipo_servicio: TipoOrganizacion,
    pub vigente_desde: Option<DateTime<Utc>>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub valor: Option<Decimal>,
}

/// Vincula una organización con la copropiedad mediante un contrato de servicio.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `organizaciones.vincular`; [`AppError::Validacion`].
pub async fn vincular_organizacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoContrato,
) -> AppResult<ContratoServicio>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioContratos + RegistroAuditoria,
{
    ctx.exigir(Permiso::OrganizacionesVincular)?;
    let desde = nuevo.vigente_desde.unwrap_or_else(|| reloj.ahora());
    if nuevo.vigente_hasta.is_some_and(|h| h < desde) {
        return Err(AppError::Validacion(
            "el contrato termina antes de empezar".into(),
        ));
    }
    if nuevo.valor.is_some_and(|v| v.is_sign_negative()) {
        return Err(AppError::Validacion(
            "el valor no puede ser negativo".into(),
        ));
    }
    let contrato = ContratoServicio {
        id: ContratoServicioId::nuevo(),
        organizacion_id: nuevo.organizacion_id,
        tipo_servicio: nuevo.tipo_servicio,
        vigente_desde: desde,
        vigente_hasta: nuevo.vigente_hasta,
        estado: EstadoContrato::Vigente,
        valor: nuevo.valor,
    };
    let mut tx = unidad.iniciar(ctx).await?;
    tx.insertar_contrato(&contrato).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("organizacion_id", contrato.organizacion_id.to_string())?
        .con("tipo_servicio", contrato.tipo_servicio.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_VINCULAR, "contrato_servicio", contrato.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(contrato)
}

/// Contratos de la copropiedad.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `organizaciones.vincular` ni `contratos.gestionar`.
pub async fn listar_contratos<U>(unidad: &U, ctx: &Contexto) -> AppResult<Vec<ContratoServicio>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioContratos,
{
    ctx.exigir(Permiso::ContratosGestionar)
        .or_else(|_| ctx.exigir(Permiso::OrganizacionesVincular))?;
    unidad.iniciar(ctx).await?.contratos().await
}

/// Asigna un rol al personal de la organización, **derivado del contrato**: vence con él.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::Conflicto`] si el contrato no está vigente.
pub async fn asignar_rol_por_contrato<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    contrato_id: ContratoServicioId,
    cuenta: CuentaId,
    rol: RolId,
) -> AppResult<Asignacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioContratos + RepositorioRoles + RegistroAuditoria,
{
    ctx.exigir(Permiso::RolesAsignar)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    let contrato = tx
        .contrato(contrato_id)
        .await?
        .ok_or(AppError::NoEncontrado("contrato"))?;
    if contrato.estado != EstadoContrato::Vigente
        || contrato.vigente_hasta.is_some_and(|h| h <= ahora)
    {
        return Err(AppError::Conflicto("el contrato no está vigente".into()));
    }
    let asignacion = asignar_en(
        &mut tx,
        reloj,
        ctx,
        SolicitudAsignacion {
            cuenta_id: cuenta,
            rol_id: rol,
            vigente_desde: None,
            vigente_hasta: contrato.vigente_hasta,
            origen: OrigenAsignacion::ContratoServicio,
            origen_id: Some(contrato.id.as_uuid()),
        },
    )
    .await?;
    tx.confirmar().await?;
    Ok(asignacion)
}

/// Termina el contrato y **vence los roles derivados** de él. Devuelve cuántas asignaciones
/// terminó.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contratos.gestionar`; [`AppError::NoEncontrado`].
pub async fn terminar_contrato<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ContratoServicioId,
) -> AppResult<u64>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioContratos + RepositorioRoles + RegistroAuditoria,
{
    ctx.exigir(Permiso::ContratosGestionar)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    let contrato = tx
        .contrato(id)
        .await?
        .ok_or(AppError::NoEncontrado("contrato"))?;
    if contrato.estado == EstadoContrato::Terminado {
        return Ok(0);
    }
    tx.terminar_contrato(id, ahora).await?;
    let vencidas = tx
        .terminar_por_origen(OrigenAsignacion::ContratoServicio, id.as_uuid(), ahora)
        .await?;
    let detalle = DetalleAuditoria::vacio().con("roles_vencidos", vencidas)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_TERMINAR, "contrato_servicio", id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(vencidas)
}

/// ¿Puede el actor editar la ficha global? Soporte siempre; sin reclamar, quien la creó;
/// reclamada, sus dueños y gestores.
async fn puede_editar_ficha<T: RepositorioOrganizaciones>(
    tx: &mut T,
    ctx: &ContextoPlataforma,
    id: OrganizacionId,
) -> AppResult<bool> {
    if ctx.exigir(Permiso::PlataformaOrganizacionesRevisar).is_ok() {
        return Ok(true);
    }
    let cuenta = ctx.cuenta()?;
    let org = tx
        .organizacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("organización"))?;
    Ok(match org.ficha.estado {
        EstadoOrganizacion::SinReclamar => org.creada_por_cuenta == Some(cuenta),
        EstadoOrganizacion::Reclamada => tx
            .rol_miembro(id, cuenta)
            .await?
            .is_some_and(RolOrganizacion::edita_ficha),
        EstadoOrganizacion::Suspendida => false,
    })
}

/// Edita la ficha global (identificación y contacto público).
///
/// # Errors
///
/// [`AppError::SinPermiso`] si el actor no puede editarla; [`AppError::NoEncontrado`].
pub async fn editar_ficha<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: OrganizacionId,
    cambios: CambiosFicha,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioOrganizaciones + RegistroAuditoria,
{
    if cambios.tipos.as_ref().is_some_and(Vec::is_empty) {
        return Err(AppError::Validacion("indique al menos un tipo".into()));
    }
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    if !puede_editar_ficha(&mut tx, ctx, id).await? {
        return Err(AppError::SinPermiso(
            Permiso::PlataformaOrganizacionesRevisar,
        ));
    }
    tx.actualizar_ficha(id, &cambios, reloj.ahora()).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_EDITAR_FICHA,
        "organizacion",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Agrega una cuenta a la organización (o cambia su papel). Solo dueños o soporte agregan
/// dueños; los gestores agregan gestores y operarios.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`].
pub async fn guardar_miembro<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    organizacion: OrganizacionId,
    cuenta: CuentaId,
    rol: RolOrganizacion,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioOrganizaciones + RegistroAuditoria,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.organizacion(organizacion)
        .await?
        .ok_or(AppError::NoEncontrado("organización"))?;
    let es_soporte = ctx.exigir(Permiso::PlataformaOrganizacionesRevisar).is_ok();
    let mio = tx.rol_miembro(organizacion, ctx.cuenta()?).await?;
    let permitido = es_soporte
        || match rol {
            RolOrganizacion::Dueno => mio == Some(RolOrganizacion::Dueno),
            RolOrganizacion::Gestor | RolOrganizacion::Operario => {
                mio.is_some_and(RolOrganizacion::edita_ficha)
            }
        };
    if !permitido {
        return Err(AppError::SinPermiso(
            Permiso::PlataformaOrganizacionesRevisar,
        ));
    }
    tx.guardar_miembro(organizacion, cuenta, rol).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("cuenta_id", cuenta.to_string())?
        .con("rol", rol.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_MIEMBRO, "organizacion", organizacion.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await
}

/// La cuenta autenticada pide manejar la ficha de la organización. La revisa soporte.
///
/// # Errors
///
/// [`AppError::Validacion`] si el mensaje está vacío; [`AppError::Conflicto`] si ya tiene una
/// pendiente o la organización ya fue reclamada.
pub async fn reclamar_organizacion<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    organizacion: OrganizacionId,
    mensaje: &str,
) -> AppResult<Reclamacion>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioOrganizaciones + RegistroAuditoria,
{
    let mensaje = mensaje.trim();
    if mensaje.is_empty() || mensaje.chars().count() > 2000 {
        return Err(AppError::Validacion(
            "explique en hasta 2000 caracteres por qué representa a la organización".into(),
        ));
    }
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let org = tx
        .organizacion(organizacion)
        .await?
        .ok_or(AppError::NoEncontrado("organización"))?;
    if org.ficha.estado != EstadoOrganizacion::SinReclamar {
        return Err(AppError::Conflicto(
            "la organización ya fue reclamada o está suspendida".into(),
        ));
    }
    let reclamacion = Reclamacion {
        id: ReclamacionId::nuevo(),
        organizacion_id: organizacion,
        cuenta_id: ctx.cuenta()?,
        mensaje: mensaje.to_owned(),
        estado: EstadoReclamacion::Pendiente,
        creada_en: reloj.ahora(),
    };
    tx.insertar_reclamacion(&reclamacion).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_RECLAMAR,
        "reclamacion_organizacion",
        reclamacion.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(reclamacion)
}

/// Reclamaciones pendientes de revisión.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.organizaciones.revisar`.
pub async fn reclamaciones_pendientes<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<Reclamacion>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioOrganizaciones,
{
    ctx.exigir(Permiso::PlataformaOrganizacionesRevisar)?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .reclamaciones_pendientes()
        .await
}

/// Aprueba (la organización pasa a `reclamada` y quien reclamó queda como dueño) o rechaza una
/// reclamación.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::Conflicto`] si ya estaba resuelta.
pub async fn resolver_reclamacion<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: ReclamacionId,
    aprobar: bool,
    motivo: Option<&str>,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioOrganizaciones + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaOrganizacionesRevisar)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let reclamacion = tx
        .reclamacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("reclamación"))?;
    if reclamacion.estado != EstadoReclamacion::Pendiente {
        return Err(AppError::Conflicto("la reclamación ya fue resuelta".into()));
    }
    let estado = if aprobar {
        EstadoReclamacion::Aprobada
    } else {
        EstadoReclamacion::Rechazada
    };
    tx.resolver_reclamacion(id, estado, ctx.actor_id, motivo, reloj.ahora())
        .await?;
    if aprobar {
        tx.cambiar_estado_organizacion(reclamacion.organizacion_id, EstadoOrganizacion::Reclamada)
            .await?;
        tx.guardar_miembro(
            reclamacion.organizacion_id,
            reclamacion.cuenta_id,
            RolOrganizacion::Dueno,
        )
        .await?;
    }
    let detalle = DetalleAuditoria::vacio().con("estado", estado.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RESOLVER, "reclamacion_organizacion", id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await
}
