//! Operación de la plataforma: alta de copropiedades, roles de plataforma y bloqueo de cuentas.
//!
//! Solo cuentas con roles de plataforma (y el superusuario, que tiene todos los permisos). Las
//! reglas de no escalamiento y de auditoría son las mismas que en una copropiedad; además nunca
//! se queda la plataforma sin superusuario.

use domain::{
    Ambito, AsignacionRolId, ConjuntoPermisos, CuentaId, EstadoCopropiedad, EstadoCuenta, Permiso,
    RolId, TenantId,
    permisos::{CODIGO_ADMINISTRADOR, CODIGO_SUPERUSUARIO},
};

use crate::{
    AppError, AppResult, Contexto, ContextoPlataforma,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::roles::{NuevoRol, SolicitudAsignacion, asignar_en, validar_nuevo_rol},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioCopropiedad, RepositorioCuentas,
        RepositorioPlataforma, RepositorioRoles, RepositorioSesiones, Transaccion, UnidadDeTrabajo,
        UnidadDeTrabajoPlataforma,
        copropiedades::AltaCopropiedad,
        identidad::{AsignacionPlataforma, CopropiedadResumen, Rol},
    },
};

pub(crate) const ACCION_CREAR_COPROPIEDAD: AccionAuditoria =
    AccionAuditoria::new("plataforma.copropiedad.crear");
pub(crate) const ACCION_CREAR_ROL: AccionAuditoria = AccionAuditoria::new("plataforma.rol.crear");
pub(crate) const ACCION_EDITAR_ROL: AccionAuditoria =
    AccionAuditoria::new("plataforma.rol.editar_permisos");
pub(crate) const ACCION_ASIGNAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.asignacion.crear");
pub(crate) const ACCION_REVOCAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.asignacion.terminar");
pub(crate) const ACCION_BLOQUEAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.cuenta.bloquear");

pub(crate) const ACCION_CAMBIAR_ESTADO: AccionAuditoria =
    AccionAuditoria::new("plataforma.copropiedad.cambiar_estado");

/// Da de alta una copropiedad (en configuración) y le asigna su primer administrador.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `plataforma.copropiedades.crear`.
/// - [`AppError::Validacion`] si el nombre está vacío.
/// - [`AppError::NoEncontrado`] si la cuenta del administrador no existe.
/// - [`AppError::Conflicto`] si el NIT ya lo usa otra copropiedad.
pub async fn crear_copropiedad<UP, U>(
    plataforma: &UP,
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    alta: AltaCopropiedad,
    administrador: CuentaId,
) -> AppResult<TenantId>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas,
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioRoles + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaCopropiedadesCrear)?;
    let alta = AltaCopropiedad {
        nombre: domain::copropiedad::texto_obligatorio("nombre", &alta.nombre, 200)?,
        ..alta
    };
    plataforma
        .iniciar_plataforma(ctx)
        .await?
        .cuenta_por_id(administrador)
        .await?
        .filter(|c| c.estado == EstadoCuenta::Activa)
        .ok_or(AppError::NoEncontrado("cuenta"))?;

    let tenant = TenantId::nuevo();
    // Quien da de alta actúa en la PH nueva con todos los permisos de copropiedad, solo para
    // dejarla con su administrador; queda auditado a su nombre.
    let ctx_ph = Contexto {
        tenant_id: tenant,
        actor_id: ctx.actor_id,
        request_id: ctx.request_id.clone(),
        permisos: ConjuntoPermisos::todos(Ambito::Copropiedad),
    };
    let mut tx = unidad.iniciar(&ctx_ph).await?;
    tx.insertar_copropiedad(&alta).await?;
    let rol = tx
        .rol_por_codigo(CODIGO_ADMINISTRADOR)
        .await?
        .ok_or(AppError::NoEncontrado("rol administrador"))?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_CREAR_COPROPIEDAD,
        "copropiedad",
        tenant.as_uuid(),
    ))
    .await?;
    asignar_en(
        &mut tx,
        reloj,
        &ctx_ph,
        SolicitudAsignacion::manual(administrador, rol.id),
    )
    .await?;
    tx.confirmar().await?;
    Ok(tenant)
}

/// Cambia el estado de una copropiedad (activar, suspender, reactivar o retirar). La
/// plataforma decide el ciclo de vida del servicio; la máquina de estados es
/// [`EstadoCopropiedad::transicionar`].
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `plataforma.copropiedades.crear`.
/// - [`AppError::NoEncontrado`] si la copropiedad no existe.
/// - [`AppError::Dominio`] si la transición no está permitida.
pub async fn cambiar_estado_copropiedad<U>(
    unidad: &U,
    ctx: &ContextoPlataforma,
    tenant: TenantId,
    hacia: EstadoCopropiedad,
) -> AppResult<EstadoCopropiedad>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCopropiedad + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaCopropiedadesCrear)?;
    let ctx_ph = Contexto {
        tenant_id: tenant,
        actor_id: ctx.actor_id,
        request_id: ctx.request_id.clone(),
        permisos: ConjuntoPermisos::todos(Ambito::Copropiedad),
    };
    let mut tx = unidad.iniciar(&ctx_ph).await?;
    let mut ficha = tx
        .ficha()
        .await?
        .ok_or(AppError::NoEncontrado("copropiedad"))?;
    let desde = ficha.estado;
    ficha.estado = desde.transicionar(hacia)?;
    tx.guardar_ficha(&ficha).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("desde", desde.as_str())?
        .con("hacia", hacia.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CAMBIAR_ESTADO, "copropiedad", tenant.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(hacia)
}

/// Todas las copropiedades.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.copropiedades.ver`.
pub async fn listar_copropiedades<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<CopropiedadResumen>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma,
{
    ctx.exigir(Permiso::PlataformaCopropiedadesVer)?;
    unidad.iniciar_plataforma(ctx).await?.copropiedades().await
}

/// Roles de plataforma (plantillas y propios).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.roles.asignar` ni `plataforma.roles.gestionar`.
pub async fn listar_roles_plataforma<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<Rol>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma,
{
    ctx.exigir(Permiso::PlataformaRolesAsignar)
        .or_else(|_| ctx.exigir(Permiso::PlataformaRolesGestionar))?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .roles_plataforma()
        .await
}

/// Crea un rol de plataforma propio.
///
/// # Errors
///
/// [`AppError::SinPermiso`], [`AppError::Dominio`] (permisos de copropiedad) o
/// [`AppError::Conflicto`] (código repetido).
pub async fn crear_rol_plataforma<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    nuevo: NuevoRol,
) -> AppResult<Rol>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaRolesGestionar)?;
    validar_nuevo_rol(&nuevo, Ambito::Plataforma, &ctx.permisos)?;
    let rol = Rol {
        id: RolId::nuevo(),
        tenant_id: None,
        codigo: nuevo.codigo,
        nombre: nuevo.nombre.trim().to_owned(),
        descripcion: nuevo.descripcion,
        ambito: Ambito::Plataforma,
        es_plantilla: false,
        permisos: nuevo.permisos,
    };
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.insertar_rol_plataforma(&rol).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_CREAR_ROL,
        "rol",
        rol.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(rol)
}

/// Reemplaza los permisos de un rol de plataforma propio.
///
/// # Errors
///
/// [`AppError::SinPermiso`], [`AppError::NoEncontrado`] o [`AppError::Conflicto`] (plantilla).
pub async fn editar_permisos_rol_plataforma<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    rol_id: RolId,
    permisos: ConjuntoPermisos,
) -> AppResult<Rol>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaRolesGestionar)?;
    permisos.exigir_ambito(Ambito::Plataforma)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut rol = tx
        .rol_plataforma(rol_id)
        .await?
        .ok_or(AppError::NoEncontrado("rol"))?;
    if rol.es_plantilla {
        return Err(AppError::Conflicto(
            "los roles plantilla no se modifican".into(),
        ));
    }
    if !ctx.permisos.incluye(&rol.permisos) || !ctx.permisos.incluye(&permisos) {
        return Err(AppError::SinPermiso(Permiso::PlataformaRolesGestionar));
    }
    tx.reemplazar_permisos_plataforma(rol.id, &permisos).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_EDITAR_ROL,
        "rol",
        rol.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    rol.permisos = permisos;
    Ok(rol)
}

/// Asigna un rol de plataforma a una cuenta.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.roles.asignar` o si el rol tiene permisos que el
/// actor no tiene; [`AppError::NoEncontrado`] si el rol o la cuenta no existen.
pub async fn asignar_rol_plataforma<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    cuenta: CuentaId,
    rol_id: RolId,
) -> AppResult<AsignacionPlataforma>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma + RepositorioCuentas + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaRolesAsignar)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let rol = tx
        .rol_plataforma(rol_id)
        .await?
        .ok_or(AppError::NoEncontrado("rol"))?;
    if !ctx.permisos.incluye(&rol.permisos) {
        return Err(AppError::SinPermiso(Permiso::PlataformaRolesAsignar));
    }
    tx.cuenta_por_id(cuenta)
        .await?
        .ok_or(AppError::NoEncontrado("cuenta"))?;
    let asignacion = AsignacionPlataforma {
        id: AsignacionRolId::nuevo(),
        cuenta_id: cuenta,
        rol_id: rol.id,
        vigente_desde: reloj.ahora(),
        vigente_hasta: None,
        asignado_por: ctx.actor_id,
    };
    tx.insertar_asignacion_plataforma(&asignacion).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("cuenta_id", cuenta.to_string())?
        .con("rol", rol.codigo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_ASIGNAR,
            "asignacion_plataforma",
            asignacion.id.as_uuid(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(asignacion)
}

/// Termina una asignación de plataforma. Nunca deja la plataforma sin superusuario.
///
/// # Errors
///
/// [`AppError::SinPermiso`], [`AppError::NoEncontrado`] o [`AppError::Conflicto`] (último
/// superusuario).
pub async fn revocar_rol_plataforma<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: AsignacionRolId,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaRolesAsignar)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let asignacion = tx
        .asignacion_plataforma(id)
        .await?
        .ok_or(AppError::NoEncontrado("asignación"))?;
    let rol = tx
        .rol_plataforma(asignacion.rol_id)
        .await?
        .ok_or(AppError::NoEncontrado("rol"))?;
    if !ctx.permisos.incluye(&rol.permisos) {
        return Err(AppError::SinPermiso(Permiso::PlataformaRolesAsignar));
    }
    if rol.es_plantilla
        && rol.codigo == CODIGO_SUPERUSUARIO
        && tx.superusuarios_vigentes(ahora).await? <= 1
    {
        return Err(AppError::Conflicto(
            "la plataforma debe conservar al menos un superusuario".into(),
        ));
    }
    tx.terminar_asignacion_plataforma(id, ahora).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_REVOCAR,
        "asignacion_plataforma",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Bloquea una cuenta y cierra todas sus sesiones. Un superusuario solo lo bloquea otro.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.cuentas.bloquear`; [`AppError::NoEncontrado`].
pub async fn bloquear_cuenta<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    cuenta: CuentaId,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion:
        RepositorioPlataforma + RepositorioCuentas + RepositorioSesiones + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaCuentasBloquear)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.cuenta_por_id(cuenta)
        .await?
        .ok_or(AppError::NoEncontrado("cuenta"))?;
    let (_, objetivo_es_super) = tx.permisos_plataforma(cuenta, ahora).await?;
    if objetivo_es_super
        && !ctx
            .permisos
            .incluye(&ConjuntoPermisos::todos(Ambito::Plataforma))
    {
        return Err(AppError::SinPermiso(Permiso::PlataformaCuentasBloquear));
    }
    tx.cambiar_estado(cuenta, EstadoCuenta::Bloqueada).await?;
    tx.revocar_sesiones_de(cuenta, "cuenta bloqueada", ahora)
        .await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_BLOQUEAR,
        "cuenta",
        cuenta.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}
