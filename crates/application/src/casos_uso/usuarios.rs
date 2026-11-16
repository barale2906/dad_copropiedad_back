//! Usuarios (plan 20): no hay registro abierto; los usuarios los crea la plataforma o el
//! administrador de cada entidad, y la persona define su contraseña con un código que le llega
//! al correo ([`crate::casos_uso::autenticacion::restablecer_contrasena`]).
//!
//! Reglas:
//! - **Una persona, una cuenta**: si el correo ya tiene cuenta, se le asigna el rol en la entidad
//!   y se le avisa por correo (la persona trabaja en varias PH o empresas). Solo un `usuario`
//!   pedido que ya es de otra cuenta, o una cuenta bloqueada, impiden seguir (409).
//! - **No escalamiento**: se crea el usuario con un rol que quien lo crea podría asignar
//!   ([`crate::casos_uso::roles`]).
//! - Crear el usuario en una entidad deja también su **persona** vinculada a la cuenta (para
//!   «mis datos», la autorización de publicidad y como tercero).
//! - Todo pasa en **una transacción** de la entidad: las cuentas se crean o reconocen con el
//!   puerto [`RepositorioUsuarios`] (funciones `SECURITY DEFINER`), así nunca queda una cuenta
//!   sin rol ni un código enviado para un alta que falló.
//! - Carga masiva: el flujo de las importaciones (planilla → vista previa → confirmar), con la
//!   planilla de [`domain::importacion::usuarios`].

use domain::{
    ClasificacionArchivo, CuentaId, DireccionCorreo, Documento, EstadoCuenta, EstadoImportacion,
    Identificador, ImportacionId, NombreArchivo, NombreUsuario, Permiso, PersonaId, RolId,
    Telefono, TipoArchivo, TipoImportacion, TipoPersona,
    importacion::{
        ErrorFila,
        usuarios::{LARGO_MAXIMO_NOMBRE, UsuarioImportado, leer_planilla_usuarios},
    },
};

use crate::{
    AppError, AppResult, Contexto, ContextoPlataforma, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{
        archivos::{ArchivoSubido, borrar_huerfano, nuevo_archivo, registrar_en},
        autenticacion::{VIDA_CODIGO_DEFINIR, enviar_codigo, texto_definir_contrasena},
        plataforma::asignar_plataforma_en,
        roles::{SolicitudAsignacion, asignar_en},
    },
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        AlmacenArchivos, LectorPlanillas, RegistroAuditoria, RegistroOutbox, Reloj,
        RepositorioArchivos, RepositorioCuentas, RepositorioEntidad, RepositorioImportaciones,
        RepositorioPersonas, RepositorioPlataforma, RepositorioRoles, RepositorioUsuarios,
        Transaccion, UnidadDeTrabajo, UnidadDeTrabajoPlataforma,
        copropiedades::{Importacion, Persona, ResumenImportacion},
        correo::CorreoSaliente,
        identidad::{Asignacion, AsignacionPlataforma, Cuenta, PropositoCodigo},
        usuarios::{CuentaResuelta, CuentaSolicitada, FiltroUsuarios, UsuarioEntidad},
    },
    seguridad::{codigo_numerico, hash_secreto},
};

pub(crate) const ACCION_CREAR: AccionAuditoria = AccionAuditoria::new("usuarios.usuario.crear");
pub(crate) const ACCION_REENVIAR: AccionAuditoria =
    AccionAuditoria::new("usuarios.codigo.reenviar");
pub(crate) const ACCION_IMPORTAR: AccionAuditoria =
    AccionAuditoria::new("importaciones.usuarios.validar");
pub(crate) const ACCION_CONFIRMAR: AccionAuditoria =
    AccionAuditoria::new("importaciones.usuarios.aplicar");
pub(crate) const ACCION_CREAR_PLATAFORMA: AccionAuditoria =
    AccionAuditoria::new("plataforma.cuenta.crear");

/// Lo que necesita la transacción de la entidad para crear usuarios.
pub trait TxUsuarios:
    RepositorioUsuarios
    + RepositorioRoles
    + RepositorioPersonas
    + RepositorioEntidad
    + RegistroAuditoria
    + RegistroOutbox
{
}

impl<T> TxUsuarios for T where
    T: RepositorioUsuarios
        + RepositorioRoles
        + RepositorioPersonas
        + RepositorioEntidad
        + RegistroAuditoria
        + RegistroOutbox
{
}

/// Datos de un usuario nuevo de la entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoUsuario {
    pub nombre: String,
    /// Obligatorio: por él llega el código y por él se reconoce a quien ya tiene cuenta.
    pub correo: DireccionCorreo,
    pub usuario: Option<NombreUsuario>,
    pub telefono: Option<Telefono>,
    pub documento: Option<Documento>,
    pub rol_id: RolId,
}

/// Resultado de crear (o activar) un usuario en la entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsuarioCreado {
    pub cuenta_id: CuentaId,
    /// `false` = el correo ya tenía cuenta: se le asignó el rol y se le avisó.
    pub cuenta_nueva: bool,
    pub persona_id: PersonaId,
    /// `None` si ya tenía ese rol vigente en la entidad (no se repite).
    pub asignacion: Option<Asignacion>,
}

/// Valida el nombre que se mostrará de la cuenta.
fn nombre_valido(nombre: &str) -> AppResult<String> {
    let nombre = nombre.trim();
    if nombre.is_empty() || nombre.chars().count() > LARGO_MAXIMO_NOMBRE {
        return Err(AppError::Validacion(
            "el nombre debe tener entre 1 y 120 caracteres".into(),
        ));
    }
    Ok(nombre.to_owned())
}

fn correo_saliente(para: &DireccionCorreo, (asunto, texto): (&str, String)) -> EventoSaliente {
    EventoSaliente {
        prioridad: PrioridadSalida::Confirmacion,
        mensaje: MensajeOutbox::Correo(CorreoSaliente {
            para: para.clone(),
            asunto: asunto.into(),
            texto,
            html: None,
            adjuntos: Vec::new(),
        }),
    }
}

fn texto_acceso(nombre: &str, donde: &str, rol: &str) -> (&'static str, String) {
    (
        "Tiene un acceso nuevo en DAD Copropiedad",
        format!(
            "Hola, {nombre}.\n\nAhora tiene acceso a {donde} en DAD Copropiedad con el rol \
             «{rol}». Entre con su usuario o correo y su contraseña de siempre. Si no esperaba \
             este mensaje, avise a quien administra esa cuenta."
        ),
    )
}

/// Crea un usuario en la entidad con su rol, o le asigna el rol a la cuenta que ya tiene ese
/// correo. Envía el código para definir la contraseña (cuenta nueva o aún pendiente) o un aviso
/// del acceso nuevo.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `roles.asignar` o si el rol tiene permisos que el actor no.
/// - [`AppError::Validacion`] si el nombre no es válido.
/// - [`AppError::NoEncontrado`] si el rol no existe en la entidad.
/// - [`AppError::Conflicto`] si el `usuario` es de otra cuenta, la cuenta del correo está
///   bloqueada o el documento es de una persona vinculada a otra cuenta.
pub async fn crear_usuario<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoUsuario,
) -> AppResult<UsuarioCreado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxUsuarios,
{
    ctx.exigir(Permiso::RolesAsignar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let creado = crear_en(&mut tx, reloj, ctx, &nuevo).await?;
    tx.confirmar().await?;
    Ok(creado)
}

/// [`crear_usuario`] dentro de una transacción ya abierta (alta de una entidad con su
/// administrador nuevo, carga masiva).
pub(crate) async fn crear_en<T: TxUsuarios>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: &NuevoUsuario,
) -> AppResult<UsuarioCreado> {
    let nombre = nombre_valido(&nuevo.nombre)?;
    let ahora = reloj.ahora();
    let solicitada = CuentaSolicitada {
        id: CuentaId::nuevo(),
        nombre_mostrar: nombre.clone(),
        usuario: nuevo.usuario.clone(),
        correo: nuevo.correo.clone(),
    };
    let (cuenta, cuenta_nueva, pendiente, nombre_cuenta) = match tx
        .resolver_cuenta(&solicitada, ahora)
        .await?
    {
        CuentaResuelta::Creada(id) => (id, true, true, nombre.clone()),
        CuentaResuelta::Existente {
            id,
            nombre_mostrar,
            pendiente,
        } => (id, false, pendiente, nombre_mostrar),
        CuentaResuelta::UsuarioOcupado => {
            return Err(AppError::Conflicto(
                "el nombre de usuario ya lo tiene otra cuenta: elija otro o déjelo vacío".into(),
            ));
        }
        CuentaResuelta::Inactiva => {
            return Err(AppError::Conflicto(
                "la cuenta de ese correo está bloqueada: la plataforma debe desbloquearla".into(),
            ));
        }
    };
    let ya_tiene_el_rol = tx
        .asignaciones(Some(cuenta))
        .await?
        .iter()
        .any(|a| a.rol_id == nuevo.rol_id && a.vigente_en(ahora));
    let asignacion = if ya_tiene_el_rol {
        None
    } else {
        Some(
            asignar_en(
                tx,
                reloj,
                ctx,
                SolicitudAsignacion::manual(cuenta, nuevo.rol_id),
            )
            .await?,
        )
    };
    let persona_id = vincular_persona(tx, cuenta, &nombre, nuevo).await?;
    if pendiente {
        enviar_codigo_definir(tx, cuenta, &nuevo.correo, &nombre_cuenta, ahora).await?;
    } else if asignacion.is_some() {
        let donde = tx
            .ficha_entidad()
            .await?
            .map_or_else(|| "una entidad".to_owned(), |f| f.nombre);
        let rol = tx
            .rol(nuevo.rol_id)
            .await?
            .map_or_else(String::new, |r| r.nombre);
        tx.encolar(correo_saliente(
            &nuevo.correo,
            texto_acceso(&nombre_cuenta, &donde, &rol),
        ))
        .await?;
    }
    let detalle = DetalleAuditoria::vacio()
        .con("cuenta_id", cuenta.to_string())?
        .con("cuenta_nueva", cuenta_nueva)?
        .con("rol_asignado", asignacion.is_some())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR, "cuenta", cuenta.as_uuid()).con_detalle(detalle),
    )
    .await?;
    Ok(UsuarioCreado {
        cuenta_id: cuenta,
        cuenta_nueva,
        persona_id,
        asignacion,
    })
}

/// La persona de la entidad para la cuenta: la ya vinculada, la del mismo documento (se
/// vincula) o una nueva con los datos básicos.
async fn vincular_persona<T: TxUsuarios>(
    tx: &mut T,
    cuenta: CuentaId,
    nombre: &str,
    nuevo: &NuevoUsuario,
) -> AppResult<PersonaId> {
    if let Some(p) = tx.persona_por_cuenta(cuenta).await? {
        return Ok(p.id);
    }
    if let Some(documento) = &nuevo.documento
        && let Some(p) = tx.persona_por_documento(documento).await?
    {
        if p.cuenta_id.is_some() {
            return Err(AppError::Conflicto(
                "la persona con ese documento ya está vinculada a otra cuenta".into(),
            ));
        }
        tx.vincular_cuenta(p.id, cuenta).await?;
        return Ok(p.id);
    }
    let mut persona = Persona::basica(
        PersonaId::nuevo(),
        TipoPersona::Natural,
        nombre.to_owned(),
        nuevo.documento.clone(),
    );
    persona.correo = Some(nuevo.correo.clone());
    persona.telefono = nuevo.telefono.clone();
    persona.cuenta_id = Some(cuenta);
    tx.insertar_persona(&persona).await?;
    Ok(persona.id)
}

/// Emite el código para definir la contraseña y encola el correo. `false` si la cuenta ya no
/// está pendiente (nada que enviar).
async fn enviar_codigo_definir<T: TxUsuarios>(
    tx: &mut T,
    cuenta: CuentaId,
    correo: &DireccionCorreo,
    nombre: &str,
    ahora: chrono::DateTime<chrono::Utc>,
) -> AppResult<bool> {
    let codigo = codigo_numerico()?;
    let emitido = tx
        .emitir_codigo_definir(
            cuenta,
            &hash_secreto(&codigo),
            ahora + VIDA_CODIGO_DEFINIR,
            ahora,
        )
        .await?;
    if emitido {
        tx.encolar(correo_saliente(
            correo,
            texto_definir_contrasena(nombre, &codigo),
        ))
        .await?;
    }
    Ok(emitido)
}

/// Vuelve a enviar el código para definir la contraseña a un usuario de la entidad que aún no
/// la definió (el anterior pudo vencer o perderse).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `roles.asignar`; [`AppError::NoEncontrado`] si la cuenta no es
/// usuario de la entidad; [`AppError::Validacion`] si ya definió su contraseña.
pub async fn reenviar_codigo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    cuenta: CuentaId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxUsuarios,
{
    ctx.exigir(Permiso::RolesAsignar)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    let usuario = tx
        .usuario(cuenta, ahora)
        .await?
        .ok_or(AppError::NoEncontrado("usuario"))?;
    let no_pendiente = || AppError::Validacion("el usuario ya definió su contraseña".into());
    if !usuario.contrasena_pendiente || usuario.estado != EstadoCuenta::Activa {
        return Err(no_pendiente());
    }
    let correo = usuario.correo.ok_or_else(no_pendiente)?;
    if !enviar_codigo_definir(&mut tx, cuenta, &correo, &usuario.nombre_mostrar, ahora).await? {
        return Err(no_pendiente());
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_REENVIAR,
        "cuenta",
        cuenta.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Usuarios de la entidad (cuentas con algún rol vigente), con sus roles y si la contraseña
/// está pendiente.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `roles.ver`.
pub async fn listar_usuarios<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: &FiltroUsuarios,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<UsuarioEntidad>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioUsuarios,
{
    ctx.exigir(Permiso::RolesVer)?;
    unidad
        .iniciar(ctx)
        .await?
        .usuarios(filtro, pagina, reloj.ahora())
        .await
}

// ---------------------------------------------------------------------------------------------
// Carga masiva
// ---------------------------------------------------------------------------------------------

/// Fila válida de la vista previa (sin datos personales: el número de fila y el rol).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaUsuarioPrevia {
    pub fila: u32,
    pub rol: String,
}

/// Importación de usuarios y su vista previa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VistaPreviaUsuarios {
    pub importacion: Importacion,
    pub filas: Vec<FilaUsuarioPrevia>,
}

/// Planilla leída y validada contra los roles de la entidad.
struct AnalisisUsuarios {
    usuarios: Vec<(UsuarioImportado, RolId)>,
    errores: Vec<ErrorFila>,
    total_filas: u32,
}

async fn analizar_usuarios<T: RepositorioRoles>(
    tx: &mut T,
    ctx: &Contexto,
    filas: &[Vec<String>],
) -> AppResult<AnalisisUsuarios> {
    let planilla = leer_planilla_usuarios(filas);
    let roles = tx.roles().await?;
    let mut a = AnalisisUsuarios {
        usuarios: Vec::new(),
        errores: planilla.errores,
        total_filas: planilla.total_filas,
    };
    for u in planilla.usuarios {
        match roles.iter().find(|r| r.codigo == u.rol) {
            None => a.errores.push(ErrorFila::new(
                u.fila,
                Some("rol"),
                "el rol no existe en la entidad",
            )),
            Some(r) if !ctx.permisos.incluye(&r.permisos) => a.errores.push(ErrorFila::new(
                u.fila,
                Some("rol"),
                "no puede asignar ese rol: tiene permisos que usted no tiene",
            )),
            Some(r) => {
                let id = r.id;
                a.usuarios.push((u, id));
            }
        }
    }
    a.errores.sort_by_key(|e| e.fila);
    Ok(a)
}

fn nuevo_de(u: &UsuarioImportado, rol_id: RolId) -> NuevoUsuario {
    NuevoUsuario {
        nombre: u.nombre.clone(),
        correo: u.correo.clone(),
        usuario: u.usuario.clone(),
        telefono: u.telefono.clone(),
        documento: u.documento.clone(),
        rol_id,
    }
}

fn contar(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Sube y valida una planilla de usuarios de la entidad. No crea nada: devuelve la vista previa.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `importaciones.ejecutar` o sin `roles.asignar`.
/// - [`AppError::Dominio`] si el archivo está vacío, no es CSV/XLSX o es muy grande.
/// - [`AppError::Validacion`] si la planilla no se puede leer.
pub async fn importar_usuarios<U, A, L>(
    unidad: &U,
    almacen: &A,
    lector: &L,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nombre: NombreArchivo,
    contenido: Vec<u8>,
) -> AppResult<VistaPreviaUsuarios>
where
    U: UnidadDeTrabajo,
    U::Transaccion:
        RepositorioArchivos + RepositorioImportaciones + RepositorioRoles + RegistroAuditoria,
    A: AlmacenArchivos,
    L: LectorPlanillas,
{
    ctx.exigir(Permiso::ImportacionesEjecutar)?;
    ctx.exigir(Permiso::RolesAsignar)?;
    let tipo = TipoArchivo::validar_planilla(&contenido)?;
    let filas = lector.leer(tipo, &contenido)?;
    let subido = ArchivoSubido {
        nombre_original: nombre,
        clasificacion: ClasificacionArchivo::Personal,
        contenido,
    };
    let archivo = nuevo_archivo(ctx, almacen, reloj, &subido, tipo);
    almacen.guardar(&archivo.clave, &subido.contenido).await?;
    let resultado = async {
        let mut tx = unidad.iniciar(ctx).await?;
        let analisis = analizar_usuarios(&mut tx, ctx, &filas).await?;
        registrar_en(&mut tx, &archivo).await?;
        let importacion = Importacion {
            id: ImportacionId::nuevo(),
            tipo: TipoImportacion::Usuarios,
            archivo_id: archivo.id,
            estado: if analisis.errores.is_empty() {
                EstadoImportacion::Validada
            } else {
                EstadoImportacion::ConErrores
            },
            total_filas: analisis.total_filas,
            filas_validas: contar(analisis.usuarios.len()),
            errores: analisis.errores.clone(),
            resumen: ResumenImportacion {
                usuarios: contar(analisis.usuarios.len()),
                ..ResumenImportacion::default()
            },
            creada_por: ctx.actor_id,
            creada_en: reloj.ahora(),
            aplicada_en: None,
        };
        tx.insertar_importacion(&importacion).await?;
        let detalle = DetalleAuditoria::vacio()
            .con("filas", importacion.total_filas)?
            .con("errores", importacion.errores.len())?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_IMPORTAR, "importacion", importacion.id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
        tx.confirmar().await?;
        let filas = analisis
            .usuarios
            .iter()
            .map(|(u, _)| FilaUsuarioPrevia {
                fila: u.fila,
                rol: u.rol.clone(),
            })
            .collect();
        Ok(VistaPreviaUsuarios { importacion, filas })
    }
    .await;
    if resultado.is_err() {
        borrar_huerfano(almacen, ctx, &archivo).await;
    }
    resultado
}

/// Crea los usuarios de una importación validada, en una sola transacción (si una fila falla,
/// no se crea ninguno).
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `importaciones.ejecutar` o `roles.asignar`.
/// - [`AppError::NoEncontrado`] si no existe o no es de usuarios.
/// - [`AppError::Conflicto`] si ya se aplicó, tenía errores, los datos cambiaron desde la vista
///   previa o una fila choca con otra cuenta (el mensaje dice cuál).
pub async fn confirmar_importacion_usuarios<U, A, L>(
    unidad: &U,
    almacen: &A,
    lector: &L,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ImportacionId,
) -> AppResult<Importacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxUsuarios + RepositorioArchivos + RepositorioImportaciones,
    A: AlmacenArchivos,
    L: LectorPlanillas,
{
    ctx.exigir(Permiso::ImportacionesEjecutar)?;
    ctx.exigir(Permiso::RolesAsignar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut importacion = tx
        .importacion(id)
        .await?
        .filter(|i| i.tipo == TipoImportacion::Usuarios)
        .ok_or(AppError::NoEncontrado("importación"))?;
    match importacion.estado {
        EstadoImportacion::Validada => {}
        EstadoImportacion::Aplicada => {
            return Err(AppError::Conflicto("la importación ya se aplicó".into()));
        }
        EstadoImportacion::ConErrores => {
            return Err(AppError::Conflicto(
                "la importación tiene errores: corrija el archivo e impórtelo de nuevo".into(),
            ));
        }
    }
    let archivo = tx
        .buscar(importacion.archivo_id)
        .await?
        .ok_or(AppError::NoEncontrado("archivo"))?;
    let contenido = almacen.obtener(&archivo.clave).await?;
    let filas = lector.leer(archivo.tipo, &contenido)?;
    let analisis = analizar_usuarios(&mut tx, ctx, &filas).await?;
    if !analisis.errores.is_empty() {
        return Err(AppError::Conflicto(
            "los datos cambiaron desde la vista previa: importe el archivo de nuevo".into(),
        ));
    }
    let mut cuentas_nuevas = 0;
    for (u, rol) in &analisis.usuarios {
        let creado = crear_en(&mut tx, reloj, ctx, &nuevo_de(u, *rol))
            .await
            .map_err(|e| match e {
                AppError::Conflicto(m) | AppError::Validacion(m) => {
                    AppError::Conflicto(format!("fila {}: {m}", u.fila))
                }
                otro => otro,
            })?;
        cuentas_nuevas += u32::from(creado.cuenta_nueva);
    }
    let resumen = ResumenImportacion {
        usuarios: contar(analisis.usuarios.len()),
        cuentas_nuevas,
        ..ResumenImportacion::default()
    };
    let ahora = reloj.ahora();
    tx.marcar_aplicada(id, &resumen, ahora).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("usuarios", resumen.usuarios)?
        .con("cuentas_nuevas", cuentas_nuevas)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CONFIRMAR, "importacion", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    importacion.estado = EstadoImportacion::Aplicada;
    importacion.resumen = resumen;
    importacion.aplicada_en = Some(ahora);
    Ok(importacion)
}

// ---------------------------------------------------------------------------------------------
// Usuarios de la plataforma
// ---------------------------------------------------------------------------------------------

/// Datos de un usuario nuevo de la plataforma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoUsuarioPlataforma {
    pub nombre: String,
    pub correo: DireccionCorreo,
    pub usuario: Option<NombreUsuario>,
    /// Rol de plataforma que se le asigna en el mismo paso.
    pub rol_id: Option<RolId>,
}

/// Resultado de crear (o reconocer) un usuario de la plataforma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsuarioPlataformaCreado {
    pub cuenta_id: CuentaId,
    pub cuenta_nueva: bool,
    pub asignacion: Option<AsignacionPlataforma>,
}

/// Crea un usuario de la plataforma (o reconoce la cuenta de ese correo) y, si se pide, le
/// asigna un rol de plataforma. La persona define su contraseña con el código que le llega.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `plataforma.cuentas.crear`; con rol, sin
///   `plataforma.roles.asignar` o si el rol tiene permisos que el actor no tiene.
/// - [`AppError::Validacion`] si el nombre no es válido; [`AppError::NoEncontrado`] si el rol
///   no existe.
/// - [`AppError::Conflicto`] si el `usuario` es de otra cuenta o la cuenta está bloqueada.
pub async fn crear_usuario_plataforma<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    nuevo: NuevoUsuarioPlataforma,
) -> AppResult<UsuarioPlataformaCreado>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion:
        RepositorioCuentas + RepositorioPlataforma + RegistroAuditoria + RegistroOutbox,
{
    ctx.exigir(Permiso::PlataformaCuentasCrear)?;
    if nuevo.rol_id.is_some() {
        ctx.exigir(Permiso::PlataformaRolesAsignar)?;
    }
    let nombre = nombre_valido(&nuevo.nombre)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let existente = tx
        .cuenta_por_identificador(&Identificador::Correo(nuevo.correo.clone()))
        .await?;
    let (cuenta, cuenta_nueva) = match existente {
        Some(c) if c.estado != EstadoCuenta::Activa => {
            return Err(AppError::Conflicto(
                "la cuenta de ese correo está bloqueada: desbloquéela primero".into(),
            ));
        }
        Some(c) => (c, false),
        None => {
            if let Some(u) = &nuevo.usuario
                && tx
                    .cuenta_por_identificador(&Identificador::Usuario(u.clone()))
                    .await?
                    .is_some()
            {
                return Err(AppError::Conflicto(
                    "el nombre de usuario ya lo tiene otra cuenta: elija otro o déjelo vacío"
                        .into(),
                ));
            }
            let c = Cuenta {
                id: CuentaId::nuevo(),
                nombre_mostrar: nombre,
                usuario: nuevo.usuario.clone(),
                correo: Some(nuevo.correo.clone()),
                correo_verificado: false,
                estado: EstadoCuenta::Activa,
                hash_contrasena: None,
                intentos_fallidos: 0,
                bloqueo_hasta: None,
                totp_secreto: None,
                totp_activo: false,
                creada_en: ahora,
            };
            tx.insertar_cuenta(&c).await?;
            (c, true)
        }
    };
    let asignacion = match nuevo.rol_id {
        Some(rol) => Some(asignar_plataforma_en(&mut tx, reloj, ctx, cuenta.id, rol).await?),
        None => None,
    };
    if cuenta.hash_contrasena.is_none() {
        enviar_codigo(
            &mut tx,
            &cuenta,
            &nuevo.correo,
            PropositoCodigo::DefinirContrasena,
            ahora,
        )
        .await?;
    } else if asignacion.is_some() {
        tx.encolar(correo_saliente(
            &nuevo.correo,
            texto_acceso(
                &cuenta.nombre_mostrar,
                "la consola de la plataforma",
                "plataforma",
            ),
        ))
        .await?;
    }
    let detalle = DetalleAuditoria::vacio()
        .con("cuenta_nueva", cuenta_nueva)?
        .con("rol_asignado", asignacion.is_some())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR_PLATAFORMA, "cuenta", cuenta.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(UsuarioPlataformaCreado {
        cuenta_id: cuenta.id,
        cuenta_nueva,
        asignacion,
    })
}

#[cfg(test)]
mod tests;
