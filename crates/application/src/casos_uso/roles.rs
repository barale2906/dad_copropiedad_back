//! Roles y asignaciones dentro de una copropiedad.
//!
//! Reglas:
//! - **No escalamiento**: nadie crea, edita, asigna ni revoca un rol con permisos que no tiene.
//! - Los roles de copropiedad solo llevan permisos de copropiedad; las plantillas no se editan.
//! - Siempre queda al menos un administrador vigente (invariante 3 del plan 02).
//! - Todo cambio queda auditado.

use chrono::{DateTime, Utc};
use domain::{
    Ambito, AsignacionRolId, ConjuntoPermisos, CuentaId, Identificador, OrigenAsignacion, RolId,
    TenantId, permisos::CODIGO_ADMINISTRADOR,
};
use uuid::Uuid;

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioRoles, Transaccion, UnidadDeTrabajo,
        identidad::{Asignacion, CuentaResumen, Rol},
    },
};

use domain::Permiso;

pub const ACCION_CREAR_ROL: AccionAuditoria = AccionAuditoria::new("roles.rol.crear");
pub const ACCION_EDITAR_PERMISOS: AccionAuditoria =
    AccionAuditoria::new("roles.rol.editar_permisos");
pub const ACCION_ASIGNAR: AccionAuditoria = AccionAuditoria::new("roles.asignacion.crear");
pub const ACCION_REVOCAR: AccionAuditoria = AccionAuditoria::new("roles.asignacion.terminar");

/// Valida el código de un rol propio: `^[a-z][a-z0-9_]{1,59}$`.
///
/// # Errors
///
/// [`AppError::Validacion`] si no cumple el formato.
pub fn validar_codigo_rol(codigo: &str) -> AppResult<()> {
    let valido = (2..=60).contains(&codigo.len())
        && codigo.starts_with(|c: char| c.is_ascii_lowercase())
        && codigo
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if valido {
        Ok(())
    } else {
        Err(AppError::Validacion(
            "el código del rol debe ser de 2 a 60 caracteres a-z, 0-9 o _, empezando por letra"
                .into(),
        ))
    }
}

/// Datos de un rol propio.
#[derive(Debug, Clone)]
pub struct NuevoRol {
    pub codigo: String,
    pub nombre: String,
    pub descripcion: Option<String>,
    pub permisos: ConjuntoPermisos,
}

/// Exige que el actor tenga todos los permisos de `permisos`.
fn sin_escalamiento(ctx_permisos: &ConjuntoPermisos, permisos: &ConjuntoPermisos) -> AppResult<()> {
    match permisos.iter().find(|p| !ctx_permisos.contiene(*p)) {
        Some(p) => Err(AppError::SinPermiso(p)),
        None => Ok(()),
    }
}

/// Valida los datos comunes de un rol nuevo (código, nombre, ámbito y no escalamiento).
///
/// # Errors
///
/// [`AppError::Validacion`], [`AppError::Dominio`] o [`AppError::SinPermiso`].
pub fn validar_nuevo_rol(
    nuevo: &NuevoRol,
    ambito: Ambito,
    permisos_actor: &ConjuntoPermisos,
) -> AppResult<()> {
    validar_codigo_rol(&nuevo.codigo)?;
    let nombre = nuevo.nombre.trim();
    if nombre.is_empty() || nombre.chars().count() > 80 {
        return Err(AppError::Validacion(
            "el nombre del rol debe tener entre 1 y 80 caracteres".into(),
        ));
    }
    nuevo.permisos.exigir_ambito(ambito)?;
    sin_escalamiento(permisos_actor, &nuevo.permisos)
}

/// Roles disponibles en la copropiedad: plantillas y propios.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `roles.ver`.
pub async fn listar_roles<U>(unidad: &U, ctx: &Contexto) -> AppResult<Vec<Rol>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioRoles,
{
    ctx.exigir(Permiso::RolesVer)?;
    unidad.iniciar(ctx).await?.roles().await
}

/// Crea un rol propio de la copropiedad.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `roles.gestionar` o si el rol tendría permisos que el actor no
///   tiene.
/// - [`AppError::Dominio`] si incluye permisos de plataforma.
/// - [`AppError::Conflicto`] si el código ya existe.
pub async fn crear_rol<U>(unidad: &U, ctx: &Contexto, nuevo: NuevoRol) -> AppResult<Rol>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioRoles + RegistroAuditoria,
{
    ctx.exigir(Permiso::RolesGestionar)?;
    validar_nuevo_rol(&nuevo, Ambito::Copropiedad, &ctx.permisos)?;
    let rol = Rol {
        id: RolId::nuevo(),
        tenant_id: Some(ctx.tenant_id),
        codigo: nuevo.codigo,
        nombre: nuevo.nombre.trim().to_owned(),
        descripcion: nuevo.descripcion,
        ambito: Ambito::Copropiedad,
        es_plantilla: false,
        permisos: nuevo.permisos,
    };
    let mut tx = unidad.iniciar(ctx).await?;
    tx.insertar_rol(&rol).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR_ROL, "rol", rol.id.as_uuid())
            .con_detalle(detalle_permisos(&rol.permisos)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(rol)
}

fn detalle_permisos(permisos: &ConjuntoPermisos) -> AppResult<DetalleAuditoria> {
    let lista: Vec<&str> = permisos.iter().map(Permiso::as_str).collect();
    DetalleAuditoria::vacio().con("permisos", lista)
}

/// Reemplaza los permisos de un rol propio.
///
/// # Errors
///
/// - [`AppError::NoEncontrado`] si el rol no existe en la copropiedad.
/// - [`AppError::Conflicto`] si es una plantilla.
/// - [`AppError::SinPermiso`] si el actor no tiene alguno de los permisos actuales o nuevos.
pub async fn editar_permisos_rol<U>(
    unidad: &U,
    ctx: &Contexto,
    rol_id: RolId,
    permisos: ConjuntoPermisos,
) -> AppResult<Rol>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioRoles + RegistroAuditoria,
{
    ctx.exigir(Permiso::RolesGestionar)?;
    permisos.exigir_ambito(Ambito::Copropiedad)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut rol = tx.rol(rol_id).await?.ok_or(AppError::NoEncontrado("rol"))?;
    if rol.es_plantilla || rol.tenant_id != Some(ctx.tenant_id) {
        return Err(AppError::Conflicto(
            "los roles plantilla no se modifican; cree un rol propio".into(),
        ));
    }
    sin_escalamiento(&ctx.permisos, &rol.permisos)?;
    sin_escalamiento(&ctx.permisos, &permisos)?;
    tx.reemplazar_permisos(rol.id, &permisos).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EDITAR_PERMISOS, "rol", rol.id.as_uuid())
            .con_detalle(detalle_permisos(&permisos)?),
    )
    .await?;
    tx.confirmar().await?;
    rol.permisos = permisos;
    Ok(rol)
}

/// Solicitud de asignación de un rol.
#[derive(Debug, Clone)]
pub struct SolicitudAsignacion {
    pub cuenta_id: CuentaId,
    pub rol_id: RolId,
    /// `None` = desde ahora.
    pub vigente_desde: Option<DateTime<Utc>>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub origen: OrigenAsignacion,
    pub origen_id: Option<Uuid>,
}

impl SolicitudAsignacion {
    /// Asignación manual desde ahora y sin fecha de fin.
    pub fn manual(cuenta_id: CuentaId, rol_id: RolId) -> Self {
        Self {
            cuenta_id,
            rol_id,
            vigente_desde: None,
            vigente_hasta: None,
            origen: OrigenAsignacion::Manual,
            origen_id: None,
        }
    }
}

/// Asigna un rol a una cuenta en la copropiedad.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `roles.asignar` o si el rol tiene permisos que el actor no
///   tiene.
/// - [`AppError::NoEncontrado`] si el rol no existe en la copropiedad.
/// - [`AppError::Validacion`] si la vigencia está invertida o el origen no cuadra.
pub async fn asignar_rol<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    solicitud: SolicitudAsignacion,
) -> AppResult<Asignacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioRoles + RegistroAuditoria,
{
    ctx.exigir(Permiso::RolesAsignar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let asignacion = asignar_en(&mut tx, reloj, ctx, solicitud).await?;
    tx.confirmar().await?;
    Ok(asignacion)
}

/// Asigna dentro de una transacción ya abierta (la usan otros casos de uso: alta de una
/// copropiedad, contratos de servicio).
pub(crate) async fn asignar_en<T>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    s: SolicitudAsignacion,
) -> AppResult<Asignacion>
where
    T: RepositorioRoles + RegistroAuditoria,
{
    if (s.origen == OrigenAsignacion::Manual) != s.origen_id.is_none() {
        return Err(AppError::Validacion(
            "solo las asignaciones derivadas llevan origen_id".into(),
        ));
    }
    let rol = tx
        .rol(s.rol_id)
        .await?
        .ok_or(AppError::NoEncontrado("rol"))?;
    if rol.ambito != Ambito::Copropiedad {
        return Err(AppError::NoEncontrado("rol"));
    }
    sin_escalamiento(&ctx.permisos, &rol.permisos)?;
    let desde = s.vigente_desde.unwrap_or_else(|| reloj.ahora());
    if s.vigente_hasta.is_some_and(|h| h < desde) {
        return Err(AppError::Validacion(
            "la vigencia termina antes de empezar".into(),
        ));
    }
    let asignacion = Asignacion {
        id: AsignacionRolId::nuevo(),
        cuenta_id: s.cuenta_id,
        rol_id: rol.id,
        vigente_desde: desde,
        vigente_hasta: s.vigente_hasta,
        origen: s.origen,
        origen_id: s.origen_id,
        asignado_por: ctx.actor_id,
    };
    tx.insertar_asignacion(&asignacion).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("cuenta_id", asignacion.cuenta_id.to_string())?
        .con("rol", rol.codigo.as_str())?
        .con("origen", asignacion.origen.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ASIGNAR, "asignacion_rol", asignacion.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    Ok(asignacion)
}

/// Termina una asignación (desde ahora).
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `roles.asignar` o si el rol tiene permisos que el actor no
///   tiene.
/// - [`AppError::Conflicto`] si es el último administrador vigente.
pub async fn revocar_asignacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: AsignacionRolId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioRoles + RegistroAuditoria,
{
    ctx.exigir(Permiso::RolesAsignar)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    let asignacion = tx
        .asignacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("asignación"))?;
    let rol = tx
        .rol(asignacion.rol_id)
        .await?
        .ok_or(AppError::NoEncontrado("rol"))?;
    sin_escalamiento(&ctx.permisos, &rol.permisos)?;
    if !asignacion.vigente_en(ahora) && asignacion.vigente_desde <= ahora {
        return Ok(());
    }
    if rol.es_plantilla
        && rol.codigo == CODIGO_ADMINISTRADOR
        && asignacion.vigente_en(ahora)
        && tx.administradores_vigentes(ahora).await? <= 1
    {
        return Err(AppError::Conflicto(
            "la copropiedad debe conservar al menos un administrador vigente; asigne otro antes"
                .into(),
        ));
    }
    tx.terminar_asignacion(id, ahora).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("cuenta_id", asignacion.cuenta_id.to_string())?
        .con("rol", rol.codigo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_REVOCAR, "asignacion_rol", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await
}

/// Historial de asignaciones (de una cuenta o de todas).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `roles.ver`.
pub async fn listar_asignaciones<U>(
    unidad: &U,
    ctx: &Contexto,
    cuenta: Option<CuentaId>,
) -> AppResult<Vec<Asignacion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioRoles,
{
    ctx.exigir(Permiso::RolesVer)?;
    unidad.iniciar(ctx).await?.asignaciones(cuenta).await
}

/// Busca una cuenta por usuario o correo exactos, para asignarle un rol.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `roles.asignar`.
pub async fn buscar_cuenta<U>(
    unidad: &U,
    ctx: &Contexto,
    identificador: &Identificador,
) -> AppResult<Option<CuentaResumen>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioRoles,
{
    ctx.exigir(Permiso::RolesAsignar)?;
    unidad
        .iniciar(ctx)
        .await?
        .buscar_cuenta(identificador)
        .await
}

/// Permisos efectivos de una cuenta en una copropiedad: todos si es superusuario; si no, la
/// unión de sus roles vigentes. Vacío = la cuenta no tiene nada que ver con esa copropiedad.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn permisos_en_copropiedad<U>(
    unidad: &U,
    reloj: &impl Reloj,
    tenant: TenantId,
    cuenta: CuentaId,
    es_superusuario: bool,
) -> AppResult<ConjuntoPermisos>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioRoles,
{
    if es_superusuario {
        return Ok(ConjuntoPermisos::todos(Ambito::Copropiedad));
    }
    unidad
        .iniciar(&Contexto::de_cuenta(
            tenant,
            cuenta,
            ConjuntoPermisos::vacio(),
        ))
        .await?
        .permisos_de_cuenta(cuenta, reloj.ahora())
        .await
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::puertos::unidad_trabajo::memoria::UnidadEnMemoria;

    struct RelojFijo;

    impl Reloj for RelojFijo {
        fn ahora(&self) -> DateTime<Utc> {
            Utc.with_ymd_and_hms(2026, 11, 6, 12, 0, 0)
                .single()
                .expect("fecha válida")
        }
    }

    fn ctx_con(tenant: TenantId, permisos: &[Permiso]) -> Contexto {
        Contexto::de_cuenta(
            tenant,
            CuentaId::nuevo(),
            permisos.iter().copied().collect(),
        )
    }

    async fn rol_plantilla(unidad: &UnidadEnMemoria, ctx: &Contexto, codigo: &str) -> Rol {
        unidad
            .iniciar(ctx)
            .await
            .expect("inicia")
            .rol_por_codigo(codigo)
            .await
            .expect("consulta")
            .expect("plantilla")
    }

    #[tokio::test]
    async fn no_se_crea_un_rol_con_permisos_que_el_actor_no_tiene() {
        let unidad = UnidadEnMemoria::default();
        let ctx = ctx_con(
            TenantId::nuevo(),
            &[Permiso::RolesGestionar, Permiso::TicketsCrear],
        );
        let rol = |permisos: &[Permiso]| NuevoRol {
            codigo: "auxiliar".into(),
            nombre: "Auxiliar".into(),
            descripcion: None,
            permisos: permisos.iter().copied().collect(),
        };

        let escalado = crear_rol(&unidad, &ctx, rol(&[Permiso::RolesAsignar])).await;
        let de_plataforma = crear_rol(&unidad, &ctx, rol(&[Permiso::PlataformaCuentasVer])).await;
        let valido = crear_rol(&unidad, &ctx, rol(&[Permiso::TicketsCrear])).await;

        assert!(matches!(
            escalado,
            Err(AppError::SinPermiso(Permiso::RolesAsignar))
        ));
        assert!(matches!(de_plataforma, Err(AppError::Dominio(_))));
        assert_eq!(valido.expect("crea").tenant_id, Some(ctx.tenant_id));
    }

    #[tokio::test]
    async fn no_se_asigna_un_rol_con_mas_permisos_que_los_propios() {
        let unidad = UnidadEnMemoria::default();
        let tenant = TenantId::nuevo();
        let asistente = ctx_con(tenant, &[Permiso::RolesAsignar, Permiso::RolesVer]);
        let admin = rol_plantilla(&unidad, &asistente, CODIGO_ADMINISTRADOR).await;

        let res = asignar_rol(
            &unidad,
            &RelojFijo,
            &asistente,
            SolicitudAsignacion::manual(CuentaId::nuevo(), admin.id),
        )
        .await;

        assert!(matches!(res, Err(AppError::SinPermiso(_))));
        assert!(unidad.confirmados().is_empty());
    }

    #[tokio::test]
    async fn la_copropiedad_conserva_al_menos_un_administrador() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let admin = rol_plantilla(&unidad, &ctx, CODIGO_ADMINISTRADOR).await;
        let primero = asignar_rol(
            &unidad,
            &RelojFijo,
            &ctx,
            SolicitudAsignacion::manual(CuentaId::nuevo(), admin.id),
        )
        .await
        .expect("asigna");

        let ultimo = revocar_asignacion(&unidad, &RelojFijo, &ctx, primero.id).await;
        assert!(matches!(ultimo, Err(AppError::Conflicto(_))));

        asignar_rol(
            &unidad,
            &RelojFijo,
            &ctx,
            SolicitudAsignacion::manual(CuentaId::nuevo(), admin.id),
        )
        .await
        .expect("asigna otro");
        revocar_asignacion(&unidad, &RelojFijo, &ctx, primero.id)
            .await
            .expect("ahora sí");
        let acciones: Vec<_> = unidad.confirmados().iter().map(|e| e.1.accion).collect();
        assert_eq!(
            acciones,
            vec![ACCION_ASIGNAR, ACCION_ASIGNAR, ACCION_REVOCAR]
        );
    }

    #[tokio::test]
    async fn permisos_efectivos_son_la_union_de_roles_vigentes() {
        let unidad = UnidadEnMemoria::default();
        let tenant = TenantId::nuevo();
        let ctx = Contexto::new(tenant);
        let cuenta = CuentaId::nuevo();
        let propietario = rol_plantilla(&unidad, &ctx, "propietario").await;
        let contador = rol_plantilla(&unidad, &ctx, "contador").await;
        asignar_rol(
            &unidad,
            &RelojFijo,
            &ctx,
            SolicitudAsignacion::manual(cuenta, propietario.id),
        )
        .await
        .expect("asigna");
        // Vencida: no cuenta.
        asignar_rol(
            &unidad,
            &RelojFijo,
            &ctx,
            SolicitudAsignacion {
                vigente_desde: Some(RelojFijo.ahora() - chrono::Duration::days(10)),
                vigente_hasta: Some(RelojFijo.ahora() - chrono::Duration::days(1)),
                ..SolicitudAsignacion::manual(cuenta, contador.id)
            },
        )
        .await
        .expect("asigna");

        let permisos = permisos_en_copropiedad(&unidad, &RelojFijo, tenant, cuenta, false)
            .await
            .expect("permisos");
        let ajena = permisos_en_copropiedad(&unidad, &RelojFijo, TenantId::nuevo(), cuenta, false)
            .await
            .expect("permisos");
        let superusuario =
            permisos_en_copropiedad(&unidad, &RelojFijo, TenantId::nuevo(), cuenta, true)
                .await
                .expect("permisos");

        assert!(permisos.contiene(Permiso::ReservasCrear));
        assert!(!permisos.contiene(Permiso::ContabilidadVer), "rol vencido");
        assert!(ajena.es_vacio(), "nunca se mezclan permisos de dos PH");
        assert_eq!(superusuario, ConjuntoPermisos::todos(Ambito::Copropiedad));
    }

    #[test]
    fn codigo_de_rol() {
        assert!(validar_codigo_rol("auxiliar_cartera").is_ok());
        for invalido in ["", "a", "Mayus", "1rol", "con-guion"] {
            assert!(validar_codigo_rol(invalido).is_err(), "{invalido}");
        }
    }
}
