//! Entidades desde la consola de la plataforma (plan 20): `/api/v1/plataforma/entidades/…`.
//!
//! - Listado de PH, empresas y la plataforma con tipo, estado y módulos; cambio de estado.
//! - Gestión de usuarios y roles de **cualquier** entidad con `plataforma.entidades.usuarios`
//!   (mismos esquemas que `/entidades/{entidad_id}/roles`, `…/asignaciones` y `…/usuarios`).
//! - Acceso de soporte de solo lectura (`…/soporte`, 2 h); después la cuenta usa las rutas
//!   normales `/entidades/{entidad_id}/…` y `capacidades.modo_soporte` vale `true`.

use application::{
    Contexto,
    casos_uso::{
        plataforma::{self as uc, contexto_gestion_usuarios},
        roles::{self as uc_roles, SolicitudAsignacion},
        usuarios::{self as uc_usuarios, NuevoUsuario},
    },
    puertos::{
        RelojSistema,
        identidad::{EntidadResumen, FiltroEntidades as FiltroEntidadesPlataforma},
        usuarios::FiltroUsuarios,
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{
    AsignacionRolId, CuentaId, DireccionCorreo, Documento, EstadoCopropiedad, NombreUsuario,
    OrigenAsignacion, RolId, Telefono, TenantId, TipoEntidad,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_PLATAFORMA,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    routes::{
        plataforma::{EstadoCopropiedadPeticion, EstadoCopropiedadRespuesta},
        roles::{AsignacionPeticion, AsignacionRespuesta, FiltroAsignaciones, RolRespuesta},
        usuarios::{
            FiltroUsuariosParametros, NuevoUsuarioPeticion, UsuarioCreadoRespuesta,
            UsuarioRespuesta,
        },
    },
    sesion::OperadorPlataforma,
    state::AppState,
};

/// Rutas de entidades de la plataforma (se montan en `/api/v1/plataforma`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_entidades))
        .routes(routes!(ver_entidad))
        .routes(routes!(cambiar_estado))
        .routes(routes!(abrir_soporte))
        .routes(routes!(roles))
        .routes(routes!(asignaciones, asignar))
        .routes(routes!(revocar))
        .routes(routes!(usuarios, crear_usuario))
        .routes(routes!(reenviar_codigo))
}

const PARAM_ENTIDAD: &str = "Entidad (PH, empresa o la plataforma)";

/// Entidad vista desde la consola.
#[derive(Debug, Serialize, ToSchema)]
pub struct EntidadPlataformaRespuesta {
    pub id: Uuid,
    pub nombre: String,
    pub nit: Option<String>,
    #[schema(value_type = crate::esquemas::TipoEntidad)]
    pub tipo: TipoEntidad,
    #[schema(value_type = crate::esquemas::EstadoCopropiedad)]
    pub estado: EstadoCopropiedad,
    /// Módulos vigentes.
    #[schema(value_type = Vec<crate::esquemas::Modulo>)]
    pub modulos: Vec<String>,
}

impl From<EntidadResumen> for EntidadPlataformaRespuesta {
    fn from(e: EntidadResumen) -> Self {
        Self {
            id: e.id.as_uuid(),
            nombre: e.nombre,
            nit: e.nit,
            tipo: e.tipo,
            estado: e.estado,
            modulos: e.modulos,
        }
    }
}

/// Búsqueda de entidades.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroEntidades {
    /// Fragmento del nombre (sin tildes ni mayúsculas) o del NIT.
    pub q: Option<String>,
    /// Solo las de este tipo.
    #[param(value_type = Option<crate::esquemas::TipoEntidad>)]
    pub tipo: Option<TipoEntidad>,
    /// Solo las que están en este estado.
    #[param(value_type = Option<crate::esquemas::EstadoCopropiedad>)]
    pub estado: Option<EstadoCopropiedad>,
}

/// Motivo de un acceso de soporte.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SoportePeticion {
    #[schema(example = "Ticket 4521: el administrador no ve la cartera")]
    pub motivo: String,
}

/// Acceso de soporte abierto.
#[derive(Debug, Serialize, ToSchema)]
pub struct SoporteRespuesta {
    pub id: Uuid,
    pub entidad_id: Uuid,
    /// Hasta cuándo puede entrar en solo lectura.
    pub hasta: DateTime<Utc>,
}

async fn gestion(
    state: &AppState,
    op: &crate::sesion::CuentaAutenticada,
    id: Uuid,
) -> ApiResult<Contexto> {
    Ok(contexto_gestion_usuarios(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        TenantId::from_uuid(id),
    )
    .await?)
}

/// PH, empresas y la plataforma en una sola tabla, con tipo, estado y módulos; los filtros se
/// aplican en todas las páginas.
#[utoipa::path(
    get,
    path = "/entidades",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(FiltroEntidades, ParametrosPagina),
    responses(
        (status = OK, description = "Entidades", body = PaginaRespuesta<EntidadPlataformaRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.copropiedades.ver` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_entidades(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Query(f): Query<FiltroEntidades>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<EntidadPlataformaRespuesta>>> {
    let filtro = FiltroEntidadesPlataforma {
        texto: f.q.filter(|q| !q.trim().is_empty()),
        tipo: f.tipo,
        estado: f.estado,
        id: None,
    };
    let pagina = uc::listar_entidades(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        &filtro,
        p.validar()?,
    )
    .await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Una entidad por su id, con tipo, estado y módulos (la ficha del cliente en la consola).
#[utoipa::path(
    get,
    path = "/entidades/{entidad_id}",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = PARAM_ENTIDAD)),
    responses(
        (status = OK, description = "Entidad", body = EntidadPlataformaRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.copropiedades.ver` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver_entidad(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<EntidadPlataformaRespuesta>> {
    let e = uc::ver_entidad(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        TenantId::from_uuid(id),
    )
    .await?;
    Ok(Json(e.into()))
}

/// Activa, suspende, reactiva o retira una entidad (PH o empresa).
#[utoipa::path(
    post,
    path = "/entidades/{entidad_id}/estado",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = PARAM_ENTIDAD)),
    request_body = EstadoCopropiedadPeticion,
    responses(
        (status = OK, description = "Estado cambiado", body = EstadoCopropiedadRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.copropiedades.crear` (PH) o `plataforma.entidades.crear` (empresa) (`sin_permiso`), o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "Transición no permitida (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn cambiar_estado(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<EstadoCopropiedadPeticion>,
) -> ApiResult<Json<EstadoCopropiedadRespuesta>> {
    let estado = uc::cambiar_estado_entidad(
        &state.plataforma,
        &state.unidad,
        &RelojSistema,
        &op.ctx,
        TenantId::from_uuid(id),
        p.estado,
    )
    .await?;
    Ok(Json(EstadoCopropiedadRespuesta { estado }))
}

/// Abre un acceso de soporte de solo lectura a la entidad por 2 horas. Después la cuenta entra
/// por las rutas normales de la entidad: ve lo que se lee (nunca datos sensibles, votos
/// reservados ni convivencia) y toda escritura responde 403.
#[utoipa::path(
    post,
    path = "/entidades/{entidad_id}/soporte",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = PARAM_ENTIDAD)),
    request_body = SoportePeticion,
    responses(
        (status = CREATED, description = "Acceso abierto", body = SoporteRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.soporte.ver` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La entidad no existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin motivo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn abrir_soporte(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<SoportePeticion>,
) -> ApiResult<(StatusCode, Json<SoporteRespuesta>)> {
    let acceso = uc::abrir_soporte(
        &state.plataforma,
        &state.unidad,
        &RelojSistema,
        &op.ctx,
        TenantId::from_uuid(id),
        &p.motivo,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(SoporteRespuesta {
            id: acceso.id,
            entidad_id: acceso.tenant_id.as_uuid(),
            hasta: acceso.hasta,
        }),
    ))
}

/// Roles de la entidad (plantillas y propios).
#[utoipa::path(
    get,
    path = "/entidades/{entidad_id}/roles",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = PARAM_ENTIDAD)),
    responses(
        (status = OK, description = "Roles", body = Vec<RolRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.entidades.usuarios` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La entidad no existe", body = ErrorRespuesta),
    )
)]
pub async fn roles(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<RolRespuesta>>> {
    let ctx = gestion(&state, &op, id).await?;
    let roles = uc_roles::listar_roles(&state.unidad, &ctx).await?;
    Ok(Json(roles.into_iter().map(Into::into).collect()))
}

/// Historial de asignaciones de la entidad.
#[utoipa::path(
    get,
    path = "/entidades/{entidad_id}/asignaciones",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = PARAM_ENTIDAD), FiltroAsignaciones),
    responses(
        (status = OK, description = "Asignaciones", body = Vec<AsignacionRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.entidades.usuarios` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La entidad no existe", body = ErrorRespuesta),
    )
)]
pub async fn asignaciones(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Query(f): Query<FiltroAsignaciones>,
) -> ApiResult<Json<Vec<AsignacionRespuesta>>> {
    let ctx = gestion(&state, &op, id).await?;
    let lista =
        uc_roles::listar_asignaciones(&state.unidad, &ctx, f.cuenta_id.map(CuentaId::from_uuid))
            .await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Asigna un rol de la entidad a una cuenta existente.
#[utoipa::path(
    operation_id = "asignar_en_entidad",
    post,
    path = "/entidades/{entidad_id}/asignaciones",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = PARAM_ENTIDAD)),
    request_body = AsignacionPeticion,
    responses(
        (status = CREATED, description = "Asignado", body = AsignacionRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.entidades.usuarios` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Entidad, rol o cuenta inexistentes", body = ErrorRespuesta),
    )
)]
pub async fn asignar(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<AsignacionPeticion>,
) -> ApiResult<(StatusCode, Json<AsignacionRespuesta>)> {
    let ctx = gestion(&state, &op, id).await?;
    let a = uc_roles::asignar_rol(
        &state.unidad,
        &RelojSistema,
        &ctx,
        SolicitudAsignacion {
            cuenta_id: CuentaId::from_uuid(p.cuenta_id),
            rol_id: RolId::from_uuid(p.rol_id),
            vigente_desde: p.vigente_desde,
            vigente_hasta: p.vigente_hasta,
            origen: OrigenAsignacion::Manual,
            origen_id: None,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(a.into())))
}

/// Termina una asignación (la entidad no se queda sin administrador).
#[utoipa::path(
    operation_id = "revocar_en_entidad",
    delete,
    path = "/entidades/{entidad_id}/asignaciones/{asignacion_id}",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(
        ("entidad_id" = Uuid, Path, description = PARAM_ENTIDAD),
        ("asignacion_id" = Uuid, Path, description = "Asignación"),
    ),
    responses(
        (status = NO_CONTENT, description = "Terminada"),
        (status = FORBIDDEN, description = "Sin `plataforma.entidades.usuarios` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Es el último administrador (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn revocar(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let ctx = gestion(&state, &op, ruta.0).await?;
    uc_roles::revocar_asignacion(
        &state.unidad,
        &RelojSistema,
        &ctx,
        AsignacionRolId::from_uuid(ruta.1),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Usuarios de la entidad con sus roles.
#[utoipa::path(
    get,
    path = "/entidades/{entidad_id}/usuarios",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = PARAM_ENTIDAD), FiltroUsuariosParametros, ParametrosPagina),
    responses(
        (status = OK, description = "Usuarios", body = PaginaRespuesta<UsuarioRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.entidades.usuarios` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La entidad no existe", body = ErrorRespuesta),
    )
)]
pub async fn usuarios(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Query(f): Query<FiltroUsuariosParametros>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<UsuarioRespuesta>>> {
    let ctx = gestion(&state, &op, id).await?;
    let filtro = FiltroUsuarios {
        texto: f.q.filter(|q| !q.trim().is_empty()),
    };
    let pagina =
        uc_usuarios::listar_usuarios(&state.unidad, &RelojSistema, &ctx, &filtro, p.validar()?)
            .await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Crea un usuario en la entidad con su rol (o asigna el rol a la cuenta de ese correo).
#[utoipa::path(
    post,
    path = "/entidades/{entidad_id}/usuarios",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = PARAM_ENTIDAD)),
    request_body = NuevoUsuarioPeticion,
    responses(
        (status = CREATED, description = "Usuario creado o activado", body = UsuarioCreadoRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.entidades.usuarios` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Entidad o rol inexistentes", body = ErrorRespuesta),
        (status = CONFLICT, description = "El `usuario` es de otra cuenta o la cuenta del correo está bloqueada (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_usuario(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<NuevoUsuarioPeticion>,
) -> ApiResult<(StatusCode, Json<UsuarioCreadoRespuesta>)> {
    let ctx = gestion(&state, &op, id).await?;
    let nuevo = NuevoUsuario {
        nombre: p.nombre,
        correo: DireccionCorreo::new(&p.correo)?,
        usuario: p.usuario.as_deref().map(NombreUsuario::new).transpose()?,
        telefono: p.telefono.as_deref().map(Telefono::new).transpose()?,
        documento: p.documento.as_deref().map(Documento::new).transpose()?,
        rol_id: RolId::from_uuid(p.rol_id),
    };
    let creado = uc_usuarios::crear_usuario(&state.unidad, &RelojSistema, &ctx, nuevo).await?;
    Ok((StatusCode::CREATED, Json(creado.into())))
}

/// Reenvía el código para definir la contraseña a un usuario de la entidad.
#[utoipa::path(
    post,
    path = "/entidades/{entidad_id}/usuarios/{cuenta_id}/codigo-contrasena",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(
        ("entidad_id" = Uuid, Path, description = PARAM_ENTIDAD),
        ("cuenta_id" = Uuid, Path, description = "Cuenta del usuario"),
    ),
    responses(
        (status = NO_CONTENT, description = "Código enviado"),
        (status = FORBIDDEN, description = "Sin `plataforma.entidades.usuarios` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No es usuario de la entidad", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Ya definió su contraseña (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn reenviar_codigo(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let ctx = gestion(&state, &op, ruta.0).await?;
    uc_usuarios::reenviar_codigo(
        &state.unidad,
        &RelojSistema,
        &ctx,
        CuentaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
