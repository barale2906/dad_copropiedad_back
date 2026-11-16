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
    casos_uso::{
        roles::{NuevoRol, SolicitudAsignacion, asignar_en, validar_nuevo_rol},
        usuarios::{NuevoUsuario, TxUsuarios, crear_en},
    },
    puertos::{
        RegistroAuditoria, Reloj, RepositorioCoeficientes as _, RepositorioCuentas,
        RepositorioEntidad, RepositorioPlataforma, RepositorioRoles, RepositorioSesiones,
        Transaccion, UnidadDeTrabajo, UnidadDeTrabajoPlataforma,
        copropiedades::AltaCopropiedad,
        identidad::{AccesoSoporte, AsignacionPlataforma, CopropiedadResumen, EntidadResumen, Rol},
    },
};

pub(crate) const ACCION_CREAR_COPROPIEDAD: AccionAuditoria =
    AccionAuditoria::new("plataforma.copropiedad.crear");
pub(crate) const ACCION_CREAR_ENTIDAD: AccionAuditoria =
    AccionAuditoria::new("plataforma.entidad.crear");
pub(crate) const ACCION_DESIGNAR_PLATAFORMA: AccionAuditoria =
    AccionAuditoria::new("plataforma.entidad.designar");
pub(crate) const ACCION_CREAR_ROL: AccionAuditoria = AccionAuditoria::new("plataforma.rol.crear");
pub(crate) const ACCION_EDITAR_ROL: AccionAuditoria =
    AccionAuditoria::new("plataforma.rol.editar_permisos");
pub(crate) const ACCION_ASIGNAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.asignacion.crear");
pub(crate) const ACCION_REVOCAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.asignacion.terminar");
pub(crate) const ACCION_BLOQUEAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.cuenta.bloquear");
pub(crate) const ACCION_DESBLOQUEAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.cuenta.desbloquear");

pub(crate) const ACCION_CAMBIAR_ESTADO: AccionAuditoria =
    AccionAuditoria::new("plataforma.entidad.cambiar_estado");
pub(crate) const ACCION_GESTIONAR_USUARIOS: AccionAuditoria =
    AccionAuditoria::new("plataforma.entidad.gestionar_usuarios");
pub(crate) const ACCION_ABRIR_SOPORTE: AccionAuditoria =
    AccionAuditoria::new("plataforma.soporte.abrir");

/// Da de alta una copropiedad (en configuración): la entidad y su extensión de la vertical PH
/// (ADR 0016), sin módulos vendibles; le asigna su primer administrador.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `plataforma.copropiedades.crear`.
/// - [`AppError::Validacion`] si el nombre está vacío.
/// - [`AppError::NoEncontrado`] si la cuenta del administrador no existe.
/// - [`AppError::Conflicto`] si el NIT ya lo usa otra copropiedad, o con un administrador
///   nuevo, los de [`crate::casos_uso::usuarios::crear_usuario`].
pub async fn crear_copropiedad<UP, U>(
    plataforma: &UP,
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    alta: AltaCopropiedad,
    administrador: AdministradorAlta,
) -> AppResult<TenantId>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas,
    U: UnidadDeTrabajo,
    U::Transaccion: TxUsuarios,
{
    ctx.exigir(Permiso::PlataformaCopropiedadesCrear)?;
    let alta = AltaCopropiedad {
        nombre: domain::copropiedad::texto_obligatorio("nombre", &alta.nombre, 200)?,
        ..alta
    };
    dar_de_alta(
        plataforma,
        unidad,
        reloj,
        ctx,
        (alta.nombre.clone(), alta.nit.clone()),
        Some(&alta),
        administrador,
    )
    .await
}

/// Da de alta una entidad sin vertical (cualquier empresa que contrata la contabilidad base,
/// ADR 0016) y le asigna su primer administrador. Sus módulos se habilitan después, uno a uno.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `plataforma.entidades.crear`.
/// - [`AppError::Validacion`] si el nombre está vacío.
/// - [`AppError::NoEncontrado`] si la cuenta del administrador no existe.
/// - [`AppError::Conflicto`] si el NIT ya lo usa otra entidad, o con un administrador nuevo,
///   los de [`crate::casos_uso::usuarios::crear_usuario`].
pub async fn crear_entidad<UP, U>(
    plataforma: &UP,
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    nombre: &str,
    nit: Option<domain::Nit>,
    administrador: AdministradorAlta,
) -> AppResult<TenantId>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas,
    U: UnidadDeTrabajo,
    U::Transaccion: TxUsuarios,
{
    ctx.exigir(Permiso::PlataformaEntidadesCrear)?;
    let nombre = domain::copropiedad::texto_obligatorio("nombre", nombre, 200)?;
    dar_de_alta(
        plataforma,
        unidad,
        reloj,
        ctx,
        (nombre, nit),
        None,
        administrador,
    )
    .await
}

/// Primer administrador de una entidad que se da de alta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdministradorAlta {
    /// Una cuenta activa que ya existe.
    Existente(CuentaId),
    /// Una persona que puede no tener cuenta: se crea (o se reconoce por el correo) como en
    /// [`crate::casos_uso::usuarios::crear_usuario`] y define su contraseña con un código.
    Nuevo {
        nombre: String,
        correo: domain::DireccionCorreo,
        usuario: Option<domain::NombreUsuario>,
    },
}

/// Crea la entidad (y, si viene, su extensión de copropiedad) y asigna el administrador, todo en
/// una transacción de la entidad.
async fn dar_de_alta<UP, U>(
    plataforma: &UP,
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    (nombre, nit): (String, Option<domain::Nit>),
    copropiedad: Option<&AltaCopropiedad>,
    administrador: AdministradorAlta,
) -> AppResult<TenantId>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas,
    U: UnidadDeTrabajo,
    U::Transaccion: TxUsuarios,
{
    if let AdministradorAlta::Existente(cuenta) = &administrador {
        plataforma
            .iniciar_plataforma(ctx)
            .await?
            .cuenta_por_id(*cuenta)
            .await?
            .filter(|c| c.estado == EstadoCuenta::Activa)
            .ok_or(AppError::NoEncontrado("cuenta"))?;
    }

    let tenant = TenantId::nuevo();
    // Quien da de alta actúa en la entidad nueva con todos los permisos, solo para dejarla con
    // su administrador; queda auditado a su nombre.
    let ctx_ph = en_entidad(ctx, tenant);
    let mut tx = unidad.iniciar(&ctx_ph).await?;
    tx.insertar_entidad(&nombre, nit.as_ref()).await?;
    let (accion, entidad) = match copropiedad {
        Some(alta) => {
            tx.insertar_copropiedad(alta).await?;
            (ACCION_CREAR_COPROPIEDAD, "copropiedad")
        }
        None => (ACCION_CREAR_ENTIDAD, "entidad"),
    };
    let rol = tx
        .rol_por_codigo(CODIGO_ADMINISTRADOR)
        .await?
        .ok_or(AppError::NoEncontrado("rol administrador"))?;
    tx.registrar(EventoAuditoria::ok(accion, entidad, tenant.as_uuid()))
        .await?;
    match administrador {
        AdministradorAlta::Existente(cuenta) => {
            asignar_en(
                &mut tx,
                reloj,
                &ctx_ph,
                SolicitudAsignacion::manual(cuenta, rol.id),
            )
            .await?;
        }
        AdministradorAlta::Nuevo {
            nombre,
            correo,
            usuario,
        } => {
            crear_en(
                &mut tx,
                reloj,
                &ctx_ph,
                &NuevoUsuario {
                    nombre,
                    correo,
                    usuario,
                    telefono: None,
                    documento: None,
                    rol_id: rol.id,
                },
            )
            .await?;
        }
    }
    tx.confirmar().await?;
    Ok(tenant)
}

/// Contexto de **sistema** en una entidad a nombre de una cuenta de la plataforma: para dejar
/// una entidad con su administrador, cambiar su estado o gestionar sus usuarios (todo queda
/// auditado a su nombre en la entidad).
fn en_entidad(ctx: &ContextoPlataforma, tenant: TenantId) -> Contexto {
    Contexto {
        tenant_id: tenant,
        actor_id: ctx.actor_id,
        request_id: ctx.request_id.clone(),
        permisos: ConjuntoPermisos::todos(Ambito::Copropiedad),
    }
}

/// Entidad por su id vista desde la plataforma.
async fn entidad<T: RepositorioPlataforma>(
    tx: &mut T,
    tenant: TenantId,
    ahora: chrono::DateTime<chrono::Utc>,
) -> AppResult<EntidadResumen> {
    tx.entidades(
        None,
        Some(tenant),
        crate::SolicitudPagina::new(Some(1), None)?,
        ahora,
    )
    .await?
    .datos
    .into_iter()
    .next()
    .ok_or(AppError::NoEncontrado("entidad"))
}

/// Cambia el estado de una entidad —PH o empresa— (activar, suspender, reactivar o retirar).
/// La plataforma decide el ciclo de vida del servicio; la máquina de estados es
/// [`EstadoCopropiedad::transicionar`].
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `plataforma.copropiedades.crear` (PH) o
///   `plataforma.entidades.crear` (empresa o la plataforma).
/// - [`AppError::NoEncontrado`] si la entidad no existe.
/// - [`AppError::Dominio`] si la transición no está permitida.
/// - [`AppError::Conflicto`] si se suspende con una asamblea instalada (nunca en plena asamblea).
pub async fn cambiar_estado_entidad<UP, U>(
    plataforma: &UP,
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    tenant: TenantId,
    hacia: EstadoCopropiedad,
) -> AppResult<EstadoCopropiedad>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma,
    U: UnidadDeTrabajo,
    U::Transaccion:
        RepositorioEntidad + RegistroAuditoria + crate::puertos::RepositorioCoeficientes,
{
    let resumen = entidad(
        &mut plataforma.iniciar_plataforma(ctx).await?,
        tenant,
        reloj.ahora(),
    )
    .await?;
    ctx.exigir(match resumen.tipo {
        domain::TipoEntidad::PropiedadHorizontal => Permiso::PlataformaCopropiedadesCrear,
        domain::TipoEntidad::Empresa | domain::TipoEntidad::Plataforma => {
            Permiso::PlataformaEntidadesCrear
        }
    })?;
    let mut tx = unidad.iniciar(&en_entidad(ctx, tenant)).await?;
    let ficha = tx
        .ficha_entidad()
        .await?
        .ok_or(AppError::NoEncontrado("entidad"))?;
    let desde = ficha.estado;
    let nuevo = desde.transicionar(hacia)?;
    if nuevo == EstadoCopropiedad::Suspendida && tx.reuniones_instaladas().await? > 0 {
        return Err(AppError::Conflicto(
            "hay una asamblea instalada: suspenda la entidad cuando se cierre".into(),
        ));
    }
    tx.guardar_estado_entidad(nuevo).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("desde", desde.as_str())?
        .con("hacia", hacia.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CAMBIAR_ESTADO, "entidad", tenant.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(nuevo)
}

/// Entidades de la plataforma (PH, empresas y la plataforma) con su tipo, estado y módulos.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.copropiedades.ver`.
pub async fn listar_entidades<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    texto: Option<&str>,
    pagina: crate::SolicitudPagina,
) -> AppResult<crate::Pagina<EntidadResumen>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma,
{
    ctx.exigir(Permiso::PlataformaCopropiedadesVer)?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .entidades(texto, None, pagina, reloj.ahora())
        .await
}

/// Contexto para gestionar los usuarios y roles de una entidad desde la plataforma (U3): todos
/// los permisos de la entidad a nombre de la cuenta, que debe tener
/// `plataforma.entidades.usuarios`. La intervención queda en la auditoría de la plataforma y
/// cada cambio, en la de la entidad.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.entidades.usuarios`; [`AppError::NoEncontrado`] si
/// la entidad no existe.
pub async fn contexto_gestion_usuarios<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    tenant: TenantId,
) -> AppResult<Contexto>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaEntidadesUsuarios)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    entidad(&mut tx, tenant, reloj.ahora()).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_GESTIONAR_USUARIOS,
        "entidad",
        tenant.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(en_entidad(ctx, tenant))
}

/// Vigencia de un acceso de soporte.
pub const VIDA_SOPORTE: chrono::Duration = chrono::Duration::hours(2);

/// Abre un acceso de soporte de solo lectura a una entidad cliente ([`VIDA_SOPORTE`]), con
/// motivo. Queda en la auditoría de la plataforma y en la de la entidad.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.soporte.ver`; [`AppError::NoAutenticado`] sin
/// cuenta; [`AppError::NoEncontrado`] si la entidad no existe; [`AppError::Validacion`] sin
/// motivo.
pub async fn abrir_soporte<UP, U>(
    plataforma: &UP,
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    tenant: TenantId,
    motivo: &str,
) -> AppResult<AccesoSoporte>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma + RegistroAuditoria,
    U: UnidadDeTrabajo,
    U::Transaccion: RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaSoporteVer)?;
    let cuenta = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    let motivo = domain::copropiedad::texto_obligatorio("motivo", motivo, 500)?;
    let ahora = reloj.ahora();
    let acceso = AccesoSoporte {
        id: uuid::Uuid::now_v7(),
        cuenta_id: cuenta,
        tenant_id: tenant,
        motivo,
        desde: ahora,
        hasta: ahora + VIDA_SOPORTE,
    };
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    entidad(&mut tx, tenant, ahora).await?;
    tx.insertar_acceso_soporte(&acceso).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("entidad_id", tenant.to_string())?
        .con("motivo", acceso.motivo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ABRIR_SOPORTE, "acceso_soporte", acceso.id).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    let mut tx = unidad.iniciar(&en_entidad(ctx, tenant)).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("motivo", acceso.motivo.as_str())?
        .con("hasta", acceso.hasta.to_rfc3339())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ABRIR_SOPORTE, "acceso_soporte", acceso.id).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(acceso)
}

/// Permisos de soporte de la cuenta en la entidad: los de solo lectura
/// ([`domain::permisos::permisos_soporte`]) si tiene `plataforma.soporte.ver` y un acceso
/// vigente; `None` si no.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn permisos_de_soporte<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    tenant: TenantId,
) -> AppResult<Option<ConjuntoPermisos>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma,
{
    let Some(cuenta) = ctx.actor_id else {
        return Ok(None);
    };
    if !ctx.permisos.contiene(Permiso::PlataformaSoporteVer) {
        return Ok(None);
    }
    let vigente = unidad
        .iniciar_plataforma(ctx)
        .await?
        .acceso_soporte_vigente(cuenta, tenant, reloj.ahora())
        .await?;
    Ok(vigente.map(|_| domain::permisos::permisos_soporte()))
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
    let asignacion = asignar_plataforma_en(&mut tx, reloj, ctx, cuenta, rol_id).await?;
    tx.confirmar().await?;
    Ok(asignacion)
}

/// Asigna un rol de plataforma dentro de una transacción ya abierta (también al crear un
/// usuario de la plataforma). Exige que el actor tenga todos los permisos del rol.
pub(crate) async fn asignar_plataforma_en<T>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    cuenta: CuentaId,
    rol_id: RolId,
) -> AppResult<AsignacionPlataforma>
where
    T: RepositorioPlataforma + RepositorioCuentas + RegistroAuditoria,
{
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
    Ok(asignacion)
}

/// Designa la entidad que opera la plataforma (la empresa de GISLA SAS): desde entonces aparece
/// como `plataforma` a quien tiene roles de plataforma al iniciar sesión, y es la que factura los
/// servicios de la plataforma. Solo puede haber una; designar otra le quita la marca a la
/// anterior. Una copropiedad no puede ser la plataforma.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.entidad_propia.designar`; [`AppError::NoEncontrado`]
/// si la entidad no existe o está retirada; [`AppError::Conflicto`] si es una copropiedad.
pub async fn designar_entidad_plataforma<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    entidad: TenantId,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaEntidadPropiaDesignar)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    if !tx.designar_entidad_plataforma(entidad).await? {
        return Err(AppError::NoEncontrado("entidad"));
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_DESIGNAR_PLATAFORMA,
        "entidad",
        entidad.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Documentos electrónicos transmitidos por entidad y tipo en el periodo: la base para cobrar
/// el módulo de facturación por paquetes de documentos (plan 19). Solo conteos.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.facturacion.ver`; [`AppError::Validacion`] si el
/// rango está invertido.
pub async fn consumo_documentos<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    desde: chrono::NaiveDate,
    hasta: chrono::NaiveDate,
) -> AppResult<Vec<crate::puertos::identidad::ConsumoDocumentos>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma,
{
    ctx.exigir(Permiso::PlataformaFacturacionVer)?;
    if hasta < desde {
        return Err(AppError::Validacion(
            "`hasta` no puede ser anterior a `desde`".into(),
        ));
    }
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .consumo_documentos(desde, hasta)
        .await
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

/// Desbloquea una cuenta bloqueada por la plataforma (y quita un bloqueo temporal por intentos).
/// Un superusuario solo lo desbloquea otro. El motivo queda en la auditoría.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.cuentas.bloquear`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si el motivo está vacío o la cuenta no está bloqueada.
pub async fn desbloquear_cuenta<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    cuenta: CuentaId,
    motivo: &str,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma + RepositorioCuentas + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaCuentasBloquear)?;
    let motivo = domain::copropiedad::texto_obligatorio("motivo", motivo, 500)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let actual = tx
        .cuenta_por_id(cuenta)
        .await?
        .ok_or(AppError::NoEncontrado("cuenta"))?;
    if actual.estado != EstadoCuenta::Bloqueada {
        return Err(AppError::Validacion("la cuenta no está bloqueada".into()));
    }
    let (_, objetivo_es_super) = tx.permisos_plataforma(cuenta, ahora).await?;
    if objetivo_es_super
        && !ctx
            .permisos
            .incluye(&ConjuntoPermisos::todos(Ambito::Plataforma))
    {
        return Err(AppError::SinPermiso(Permiso::PlataformaCuentasBloquear));
    }
    tx.cambiar_estado(cuenta, EstadoCuenta::Activa).await?;
    tx.registrar_fallo(cuenta, 0, None).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_DESBLOQUEAR, "cuenta", cuenta.as_uuid())
            .con_detalle(DetalleAuditoria::vacio().con("motivo", motivo.as_str())?),
    )
    .await?;
    tx.confirmar().await
}

/// Cuenta vista por la plataforma (soporte): estado, contraseña pendiente y segundo factor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuentaConsultada {
    pub id: CuentaId,
    pub nombre_mostrar: String,
    pub usuario: Option<domain::NombreUsuario>,
    pub correo: Option<domain::DireccionCorreo>,
    pub correo_verificado: bool,
    pub estado: EstadoCuenta,
    pub contrasena_pendiente: bool,
    pub segundo_factor_activo: bool,
    /// Bloqueo temporal por contraseñas fallidas, si sigue vigente.
    pub bloqueo_temporal_hasta: Option<chrono::DateTime<chrono::Utc>>,
}

/// Busca una cuenta por usuario o correo exactos (nunca listados).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.cuentas.ver`.
pub async fn consultar_cuenta<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    identificador: &domain::Identificador,
) -> AppResult<Option<CuentaConsultada>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas,
{
    ctx.exigir(Permiso::PlataformaCuentasVer)?;
    let ahora = reloj.ahora();
    Ok(unidad
        .iniciar_plataforma(ctx)
        .await?
        .cuenta_por_identificador(identificador)
        .await?
        .map(|c| CuentaConsultada {
            id: c.id,
            nombre_mostrar: c.nombre_mostrar,
            usuario: c.usuario,
            correo: c.correo,
            correo_verificado: c.correo_verificado,
            estado: c.estado,
            contrasena_pendiente: c.hash_contrasena.is_none(),
            segundo_factor_activo: c.totp_activo,
            bloqueo_temporal_hasta: c.bloqueo_hasta.filter(|b| *b > ahora),
        }))
}
