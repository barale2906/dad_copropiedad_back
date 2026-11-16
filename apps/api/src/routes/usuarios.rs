//! Usuarios (plan 20): `/api/v1/entidades/{entidad_id}/usuarios/…` (los crea el administrador
//! de la entidad) y `/api/v1/plataforma/cuentas/…` (usuarios de la plataforma, consulta y
//! desbloqueo). No hay registro abierto: la persona recibe un código en su correo y define su
//! contraseña en `POST /auth/contrasena/restablecer`.

use application::casos_uso::{
    plataforma::{self as uc_plataforma, AdministradorAlta, CuentaConsultada},
    usuarios::{
        self as uc, FilaUsuarioPrevia, NuevoUsuario, NuevoUsuarioPlataforma, UsuarioCreado,
        VistaPreviaUsuarios,
    },
};
use application::puertos::{
    RelojSistema,
    usuarios::{FiltroUsuarios, UsuarioEntidad},
};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use domain::{
    CuentaId, DireccionCorreo, Documento, EstadoCuenta, Identificador, ImportacionId,
    NombreArchivo, NombreUsuario, RolId, Telefono, TipoArchivo,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::{ETIQUETA_PLATAFORMA, ETIQUETA_ROLES},
    paginacion::{PaginaRespuesta, ParametrosPagina},
    routes::importaciones::{DatosPlanilla, ImportacionRespuesta},
    sesion::{EnEntidad, OperadorPlataforma},
    state::AppState,
};

/// Plantilla CSV de usuarios (separada por `;`, como la guarda Excel en español).
pub const PLANTILLA_USUARIOS: &str = "nombre;correo;rol;usuario;telefono;documento\n\
Ana Pérez;ana@correo.co;contador;ana.perez;573001234567;52123456\n\
Luis Gómez;luis@correo.co;vigilante;;;\n";

/// Rutas de usuarios de una entidad (se montan en `/api/v1/entidades/{entidad_id}`).
pub fn rutas_entidad() -> OpenApiRouter<AppState> {
    let limite = usize::try_from(TipoArchivo::Xlsx.tamano_maximo()).unwrap_or(usize::MAX);
    OpenApiRouter::new()
        .routes(routes!(importar))
        .layer(DefaultBodyLimit::max(limite))
        .routes(routes!(listar, crear))
        .routes(routes!(reenviar_codigo))
        .routes(routes!(plantilla))
        .routes(routes!(confirmar_importacion))
}

/// Rutas de cuentas de la plataforma (se montan en `/api/v1/plataforma`).
pub fn rutas_plataforma() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(consultar_cuenta, crear_usuario_plataforma))
        .routes(routes!(desbloquear_cuenta))
}

// ---------------------------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------------------------

/// Administrador nuevo de una entidad que se da de alta (puede no tener cuenta todavía).
#[derive(Debug, Deserialize, ToSchema)]
pub struct AdministradorNuevoPeticion {
    #[schema(example = "Ana Pérez")]
    pub nombre: String,
    /// Obligatorio: por él llega el código para definir la contraseña. Si ya tiene cuenta, se
    /// usa esa.
    #[schema(example = "ana@correo.co")]
    pub correo: String,
    /// Opcional (3 a 30 caracteres `a-z 0-9 . _ -`).
    #[schema(example = "ana.perez")]
    pub usuario: Option<String>,
}

/// El primer administrador de un alta: una cuenta existente **o** los datos de uno nuevo.
///
/// # Errors
///
/// [`ApiError`] de validación si vienen los dos o ninguno, o si el correo o el usuario no son
/// válidos.
pub fn administrador_alta(
    cuenta_id: Option<Uuid>,
    nuevo: Option<AdministradorNuevoPeticion>,
) -> ApiResult<AdministradorAlta> {
    match (cuenta_id, nuevo) {
        (Some(id), None) => Ok(AdministradorAlta::Existente(CuentaId::from_uuid(id))),
        (None, Some(n)) => Ok(AdministradorAlta::Nuevo {
            nombre: n.nombre,
            correo: DireccionCorreo::new(&n.correo)?,
            usuario: n.usuario.as_deref().map(NombreUsuario::new).transpose()?,
        }),
        _ => Err(ApiError::App(application::AppError::Validacion(
            "indique `administrador_cuenta_id` o `administrador`, uno de los dos".into(),
        ))),
    }
}

/// Usuario nuevo de la entidad.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoUsuarioPeticion {
    #[schema(example = "Ana Pérez")]
    pub nombre: String,
    /// Obligatorio. Si ya tiene cuenta (trabaja en otra PH o empresa), se le asigna el rol.
    #[schema(example = "ana@correo.co")]
    pub correo: String,
    /// Rol en la entidad (`GET …/roles`).
    pub rol_id: Uuid,
    #[schema(example = "ana.perez")]
    pub usuario: Option<String>,
    #[schema(example = "573001234567")]
    pub telefono: Option<String>,
    /// Si coincide con una persona del censo, se vincula esa persona.
    #[schema(example = "52123456")]
    pub documento: Option<String>,
}

/// Usuario creado o activado en la entidad.
#[derive(Debug, Serialize, ToSchema)]
pub struct UsuarioCreadoRespuesta {
    pub cuenta_id: Uuid,
    /// `false` = el correo ya tenía cuenta: se le asignó el rol y se le avisó por correo.
    pub cuenta_nueva: bool,
    /// Persona de la entidad vinculada a la cuenta.
    pub persona_id: Uuid,
    /// `null` si ya tenía ese rol vigente.
    pub asignacion_id: Option<Uuid>,
}

impl From<UsuarioCreado> for UsuarioCreadoRespuesta {
    fn from(u: UsuarioCreado) -> Self {
        Self {
            cuenta_id: u.cuenta_id.as_uuid(),
            cuenta_nueva: u.cuenta_nueva,
            persona_id: u.persona_id.as_uuid(),
            asignacion_id: u.asignacion.map(|a| a.id.as_uuid()),
        }
    }
}

/// Usuario de la entidad (cuenta con algún rol vigente allí).
#[derive(Debug, Serialize, ToSchema)]
pub struct UsuarioRespuesta {
    pub cuenta_id: Uuid,
    pub nombre: String,
    pub usuario: Option<String>,
    pub correo: Option<String>,
    #[schema(value_type = crate::esquemas::EstadoCuenta)]
    pub estado: EstadoCuenta,
    /// Aún no define su contraseña (se le puede reenviar el código).
    pub contrasena_pendiente: bool,
    pub ultimo_acceso_en: Option<DateTime<Utc>>,
    /// Códigos de sus roles vigentes en la entidad.
    pub roles: Vec<String>,
}

impl From<UsuarioEntidad> for UsuarioRespuesta {
    fn from(u: UsuarioEntidad) -> Self {
        Self {
            cuenta_id: u.id.as_uuid(),
            nombre: u.nombre_mostrar,
            usuario: u.usuario.map(|x| x.as_str().to_owned()),
            correo: u.correo.map(|c| c.as_str().to_owned()),
            estado: u.estado,
            contrasena_pendiente: u.contrasena_pendiente,
            ultimo_acceso_en: u.ultimo_acceso_en,
            roles: u.roles,
        }
    }
}

/// Filtro del listado de usuarios.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroUsuariosParametros {
    /// Fragmento del nombre (sin tildes ni mayúsculas), del correo o del usuario.
    pub q: Option<String>,
}

/// Fila válida de la vista previa (sin datos personales).
#[derive(Debug, Serialize, ToSchema)]
pub struct FilaUsuarioPreviaRespuesta {
    pub fila: u32,
    #[schema(example = "contador")]
    pub rol: String,
}

impl From<FilaUsuarioPrevia> for FilaUsuarioPreviaRespuesta {
    fn from(f: FilaUsuarioPrevia) -> Self {
        Self {
            fila: f.fila,
            rol: f.rol,
        }
    }
}

/// Importación de usuarios con su vista previa.
#[derive(Debug, Serialize, ToSchema)]
pub struct VistaPreviaUsuariosRespuesta {
    pub importacion: ImportacionRespuesta,
    pub filas: Vec<FilaUsuarioPreviaRespuesta>,
}

impl From<VistaPreviaUsuarios> for VistaPreviaUsuariosRespuesta {
    fn from(v: VistaPreviaUsuarios) -> Self {
        Self {
            importacion: v.importacion.into(),
            filas: v.filas.into_iter().map(Into::into).collect(),
        }
    }
}

/// Usuario nuevo de la plataforma.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoUsuarioPlataformaPeticion {
    #[schema(example = "Laura Soporte")]
    pub nombre: String,
    #[schema(example = "laura@plataforma.co")]
    pub correo: String,
    pub usuario: Option<String>,
    /// Rol de plataforma (`GET /plataforma/roles`) que se le asigna en el mismo paso.
    pub rol_plataforma_id: Option<Uuid>,
}

/// Usuario de la plataforma creado o reconocido.
#[derive(Debug, Serialize, ToSchema)]
pub struct UsuarioPlataformaRespuesta {
    pub cuenta_id: Uuid,
    /// `false` = el correo ya tenía cuenta.
    pub cuenta_nueva: bool,
    pub asignacion_id: Option<Uuid>,
}

/// Búsqueda exacta de una cuenta.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BusquedaCuentaPlataforma {
    /// Usuario o correo exactos.
    pub identificador: String,
}

/// Cuenta vista por la plataforma.
#[derive(Debug, Serialize, ToSchema)]
pub struct CuentaPlataformaRespuesta {
    pub id: Uuid,
    pub nombre: String,
    pub usuario: Option<String>,
    pub correo: Option<String>,
    pub correo_verificado: bool,
    /// `bloqueada` = no puede iniciar sesión hasta que se desbloquee.
    #[schema(value_type = crate::esquemas::EstadoCuenta)]
    pub estado: EstadoCuenta,
    pub contrasena_pendiente: bool,
    pub segundo_factor_activo: bool,
    /// Bloqueo temporal por contraseñas fallidas (se levanta solo o al desbloquear).
    pub bloqueo_temporal_hasta: Option<DateTime<Utc>>,
}

impl From<CuentaConsultada> for CuentaPlataformaRespuesta {
    fn from(c: CuentaConsultada) -> Self {
        Self {
            id: c.id.as_uuid(),
            nombre: c.nombre_mostrar,
            usuario: c.usuario.map(|u| u.as_str().to_owned()),
            correo: c.correo.map(|x| x.as_str().to_owned()),
            correo_verificado: c.correo_verificado,
            estado: c.estado,
            contrasena_pendiente: c.contrasena_pendiente,
            segundo_factor_activo: c.segundo_factor_activo,
            bloqueo_temporal_hasta: c.bloqueo_temporal_hasta,
        }
    }
}

/// Motivo de un desbloqueo (queda en la auditoría).
#[derive(Debug, Deserialize, ToSchema)]
pub struct MotivoPeticion {
    #[schema(example = "La titular confirmó su identidad por teléfono")]
    pub motivo: String,
}

// ---------------------------------------------------------------------------------------------
// Entidad
// ---------------------------------------------------------------------------------------------

/// Usuarios de la entidad con sus roles y si la contraseña está pendiente.
#[utoipa::path(
    get,
    path = "/usuarios",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad (en una PH, el id de la copropiedad)"), FiltroUsuariosParametros, ParametrosPagina),
    responses(
        (status = OK, description = "Usuarios", body = PaginaRespuesta<UsuarioRespuesta>),
        (status = FORBIDDEN, description = "Sin `roles.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroUsuariosParametros>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<UsuarioRespuesta>>> {
    let filtro = FiltroUsuarios {
        texto: f.q.filter(|q| !q.trim().is_empty()),
    };
    let pagina =
        uc::listar_usuarios(&state.unidad, &RelojSistema, &en.ctx, &filtro, p.validar()?).await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Crea un usuario de la entidad con su rol (o asigna el rol a la cuenta que ya tiene ese
/// correo) y le envía el código para definir su contraseña (o un aviso del acceso nuevo).
#[utoipa::path(
    post,
    path = "/usuarios",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad (en una PH, el id de la copropiedad)")),
    request_body = NuevoUsuarioPeticion,
    responses(
        (status = CREATED, description = "Usuario creado o activado", body = UsuarioCreadoRespuesta),
        (status = FORBIDDEN, description = "Sin `roles.asignar` o con un rol con permisos que usted no tiene (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "El rol no existe en la entidad", body = ErrorRespuesta),
        (status = CONFLICT, description = "El `usuario` es de otra cuenta, la cuenta del correo está bloqueada o el documento es de una persona con otra cuenta (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevoUsuarioPeticion>,
) -> ApiResult<(StatusCode, Json<UsuarioCreadoRespuesta>)> {
    let nuevo = NuevoUsuario {
        nombre: p.nombre,
        correo: DireccionCorreo::new(&p.correo)?,
        usuario: p.usuario.as_deref().map(NombreUsuario::new).transpose()?,
        telefono: p.telefono.as_deref().map(Telefono::new).transpose()?,
        documento: p.documento.as_deref().map(Documento::new).transpose()?,
        rol_id: RolId::from_uuid(p.rol_id),
    };
    let creado = uc::crear_usuario(&state.unidad, &RelojSistema, &en.ctx, nuevo).await?;
    Ok((StatusCode::CREATED, Json(creado.into())))
}

/// Reenvía el código para definir la contraseña a un usuario que aún no la define.
#[utoipa::path(
    post,
    path = "/usuarios/{cuenta_id}/codigo-contrasena",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(
        ("entidad_id" = Uuid, Path, description = "Entidad (en una PH, el id de la copropiedad)"),
        ("cuenta_id" = Uuid, Path, description = "Cuenta del usuario"),
    ),
    responses(
        (status = NO_CONTENT, description = "Código enviado"),
        (status = FORBIDDEN, description = "Sin `roles.asignar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La cuenta no es usuario de la entidad", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Ya definió su contraseña (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn reenviar_codigo(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    uc::reenviar_codigo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CuentaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sube y valida una planilla de usuarios (CSV o XLSX): columnas `nombre`, `correo` y `rol`
/// (código), y opcionales `usuario`, `telefono` y `documento`. No crea nada: devuelve la vista
/// previa con los errores por fila; se aplica con `…/usuarios/importaciones/{id}/confirmar`.
#[utoipa::path(
    post,
    path = "/usuarios/importaciones",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad (en una PH, el id de la copropiedad)"), DatosPlanilla),
    request_body(content = Vec<u8>, content_type = "application/octet-stream",
                 description = "Contenido del CSV o XLSX"),
    responses(
        (status = CREATED, description = "Importación registrada (revise `estado` y `errores`)", body = VistaPreviaUsuariosRespuesta),
        (status = FORBIDDEN, description = "Sin `importaciones.ejecutar` o sin `roles.asignar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es CSV/XLSX o no se puede leer (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn importar(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(d): Query<DatosPlanilla>,
    cuerpo: Bytes,
) -> ApiResult<(StatusCode, Json<VistaPreviaUsuariosRespuesta>)> {
    let vista = uc::importar_usuarios(
        &state.unidad,
        state.archivos.as_ref(),
        &state.planillas,
        &RelojSistema,
        &en.ctx,
        NombreArchivo::new(&d.nombre)?,
        cuerpo.to_vec(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(vista.into())))
}

/// Plantilla CSV de usuarios con ejemplos.
#[utoipa::path(
    get,
    path = "/usuarios/importaciones/plantilla",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad (en una PH, el id de la copropiedad)")),
    responses(
        (status = OK, description = "Plantilla CSV", content_type = "text/csv", body = String),
        (status = FORBIDDEN, description = "No exige permisos: basta un rol en la entidad", body = ErrorRespuesta),
    )
)]
pub async fn plantilla(_en: EnEntidad) -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/csv; charset=utf-8"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"usuarios.csv\""),
            ),
        ],
        PLANTILLA_USUARIOS,
    )
        .into_response()
}

/// Crea los usuarios de una importación validada en una sola transacción (si una fila choca con
/// otra cuenta, no se crea ninguno y el mensaje dice la fila).
#[utoipa::path(
    post,
    path = "/usuarios/importaciones/{importacion_id}/confirmar",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(
        ("entidad_id" = Uuid, Path, description = "Entidad (en una PH, el id de la copropiedad)"),
        ("importacion_id" = Uuid, Path, description = "Importación de usuarios"),
    ),
    responses(
        (status = OK, description = "Aplicada", body = ImportacionRespuesta),
        (status = CONFLICT, description = "Ya aplicada, con errores, los datos cambiaron o una fila choca con otra cuenta (`conflicto`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe o no es de usuarios", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `importaciones.ejecutar` o sin `roles.asignar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn confirmar_importacion(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ImportacionRespuesta>> {
    let i = uc::confirmar_importacion_usuarios(
        &state.unidad,
        state.archivos.as_ref(),
        &state.planillas,
        &RelojSistema,
        &en.ctx,
        ImportacionId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(i.into()))
}

// ---------------------------------------------------------------------------------------------
// Plataforma
// ---------------------------------------------------------------------------------------------

/// Crea un usuario de la plataforma (o reconoce la cuenta de ese correo) y, si se pide, le
/// asigna un rol de plataforma. Recibe por correo el código para definir su contraseña.
#[utoipa::path(
    post,
    path = "/cuentas",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevoUsuarioPlataformaPeticion,
    responses(
        (status = CREATED, description = "Usuario creado o reconocido", body = UsuarioPlataformaRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.cuentas.crear`; con rol, sin `plataforma.roles.asignar` o con un rol con permisos que usted no tiene (`sin_permiso`); o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "El rol no existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "El `usuario` es de otra cuenta o la cuenta está bloqueada (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_usuario_plataforma(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevoUsuarioPlataformaPeticion>,
) -> ApiResult<(StatusCode, Json<UsuarioPlataformaRespuesta>)> {
    let creado = uc::crear_usuario_plataforma(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        NuevoUsuarioPlataforma {
            nombre: p.nombre,
            correo: DireccionCorreo::new(&p.correo)?,
            usuario: p.usuario.as_deref().map(NombreUsuario::new).transpose()?,
            rol_id: p.rol_plataforma_id.map(RolId::from_uuid),
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(UsuarioPlataformaRespuesta {
            cuenta_id: creado.cuenta_id.as_uuid(),
            cuenta_nueva: creado.cuenta_nueva,
            asignacion_id: creado.asignacion.map(|a| a.id.as_uuid()),
        }),
    ))
}

/// Busca una cuenta por usuario o correo exactos: estado, contraseña pendiente y segundo factor.
#[utoipa::path(
    get,
    path = "/cuentas",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(BusquedaCuentaPlataforma),
    responses(
        (status = OK, description = "Cuenta", body = CuentaPlataformaRespuesta),
        (status = NOT_FOUND, description = "No hay una cuenta con ese identificador", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.cuentas.ver` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
    )
)]
pub async fn consultar_cuenta(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Query(b): Query<BusquedaCuentaPlataforma>,
) -> ApiResult<Json<CuentaPlataformaRespuesta>> {
    let identificador = Identificador::parse(&b.identificador)?;
    let cuenta =
        uc_plataforma::consultar_cuenta(&state.plataforma, &RelojSistema, &op.ctx, &identificador)
            .await?
            .ok_or(ApiError::App(application::AppError::NoEncontrado("cuenta")))?;
    Ok(Json(cuenta.into()))
}

/// Desbloquea una cuenta bloqueada por la plataforma (y su bloqueo temporal por intentos).
#[utoipa::path(
    post,
    path = "/cuentas/{cuenta_id}/desbloquear",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("cuenta_id" = Uuid, Path, description = "Cuenta")),
    request_body = MotivoPeticion,
    responses(
        (status = NO_CONTENT, description = "Desbloqueada"),
        (status = FORBIDDEN, description = "Sin `plataforma.cuentas.bloquear` (`sin_permiso`; un superusuario solo lo desbloquea otro) o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La cuenta no existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin motivo o la cuenta no está bloqueada (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn desbloquear_cuenta(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<MotivoPeticion>,
) -> ApiResult<StatusCode> {
    uc_plataforma::desbloquear_cuenta(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        CuentaId::from_uuid(id),
        &p.motivo,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
