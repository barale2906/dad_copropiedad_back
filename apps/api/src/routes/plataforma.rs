//! Rutas globales de la plataforma: `/api/v1/plataforma/…`.
//!
//! Operaciones que no pertenecen a una copropiedad: alta de copropiedades, roles de plataforma,
//! bloqueo de cuentas y revisión de reclamaciones de organizaciones. Exigen permisos de
//! plataforma y, salvo que se desactive en desarrollo, una sesión con segundo factor.

use application::{
    casos_uso::{organizaciones as org, plataforma as uc},
    puertos::RelojSistema,
};
use axum::{extract::State, http::StatusCode};
use domain::{AsignacionRolId, CuentaId, ReclamacionId, RolId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path},
    openapi::ETIQUETA_PLATAFORMA,
    routes::{
        organizaciones::ReclamacionRespuesta,
        roles::{NuevoRolPeticion, PermisosPeticion, RolRespuesta},
    },
    sesion::OperadorPlataforma,
    state::AppState,
};

/// Router de la plataforma (se monta en `/api/v1/plataforma`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_copropiedades, crear_copropiedad))
        .routes(routes!(listar_roles, crear_rol))
        .routes(routes!(editar_permisos_rol))
        .routes(routes!(asignar))
        .routes(routes!(revocar))
        .routes(routes!(bloquear_cuenta))
        .routes(routes!(reclamaciones_pendientes))
        .routes(routes!(resolver_reclamacion))
}

/// Copropiedad registrada.
#[derive(Debug, Serialize, ToSchema)]
pub struct CopropiedadRespuesta {
    pub id: Uuid,
    pub nombre: String,
}

/// Todas las copropiedades de la plataforma.
#[utoipa::path(
    get,
    path = "/copropiedades",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Copropiedades", body = Vec<CopropiedadRespuesta>),
        (status = FORBIDDEN, description = "Sin permiso o sin segundo factor", body = ErrorRespuesta),
    )
)]
pub async fn listar_copropiedades(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
) -> ApiResult<Json<Vec<CopropiedadRespuesta>>> {
    let lista = uc::listar_copropiedades(&state.plataforma, &op.ctx).await?;
    Ok(Json(
        lista
            .into_iter()
            .map(|c| CopropiedadRespuesta {
                id: c.id.as_uuid(),
                nombre: c.nombre,
            })
            .collect(),
    ))
}

/// Alta de una copropiedad.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaCopropiedadPeticion {
    #[schema(example = "Conjunto Residencial Alameda")]
    pub nombre: String,
    /// Cuenta que será su primer administrador.
    pub administrador_cuenta_id: Uuid,
}

/// Da de alta una copropiedad y le asigna su primer administrador.
#[utoipa::path(
    post,
    path = "/copropiedades",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevaCopropiedadPeticion,
    responses(
        (status = CREATED, description = "Copropiedad creada", body = CopropiedadRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.copropiedades.crear`", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La cuenta del administrador no existe", body = ErrorRespuesta),
    )
)]
pub async fn crear_copropiedad(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevaCopropiedadPeticion>,
) -> ApiResult<(StatusCode, Json<CopropiedadRespuesta>)> {
    let id = uc::crear_copropiedad(
        &state.plataforma,
        &state.unidad,
        &RelojSistema,
        &op.ctx,
        &p.nombre,
        CuentaId::from_uuid(p.administrador_cuenta_id),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(CopropiedadRespuesta {
            id: id.as_uuid(),
            nombre: p.nombre.trim().to_owned(),
        }),
    ))
}

/// Roles de plataforma (plantillas y propios).
#[utoipa::path(
    get,
    path = "/roles",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    responses((status = OK, description = "Roles de plataforma", body = Vec<RolRespuesta>))
)]
pub async fn listar_roles(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
) -> ApiResult<Json<Vec<RolRespuesta>>> {
    let roles = uc::listar_roles_plataforma(&state.plataforma, &op.ctx).await?;
    Ok(Json(roles.into_iter().map(Into::into).collect()))
}

/// Crea un rol de plataforma propio (solo permisos de plataforma).
#[utoipa::path(
    post,
    path = "/roles",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevoRolPeticion,
    responses(
        (status = CREATED, description = "Rol creado", body = RolRespuesta),
        (status = CONFLICT, description = "Código repetido", body = ErrorRespuesta),
    )
)]
pub async fn crear_rol(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevoRolPeticion>,
) -> ApiResult<(StatusCode, Json<RolRespuesta>)> {
    let rol = uc::crear_rol_plataforma(&state.plataforma, &op.ctx, p.into()).await?;
    Ok((StatusCode::CREATED, Json(rol.into())))
}

/// Reemplaza los permisos de un rol de plataforma propio.
#[utoipa::path(
    put,
    path = "/roles/{rol_id}/permisos",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("rol_id" = Uuid, Path, description = "Rol")),
    request_body = PermisosPeticion,
    responses((status = OK, description = "Rol actualizado", body = RolRespuesta))
)]
pub async fn editar_permisos_rol(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(rol_id): Path<Uuid>,
    Json(p): Json<PermisosPeticion>,
) -> ApiResult<Json<RolRespuesta>> {
    let rol = uc::editar_permisos_rol_plataforma(
        &state.plataforma,
        &op.ctx,
        RolId::from_uuid(rol_id),
        p.conjunto(),
    )
    .await?;
    Ok(Json(rol.into()))
}

/// Asignación de un rol de plataforma.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AsignacionPlataformaPeticion {
    pub cuenta_id: Uuid,
    pub rol_id: Uuid,
}

/// Asignación creada.
#[derive(Debug, Serialize, ToSchema)]
pub struct AsignacionPlataformaRespuesta {
    pub id: Uuid,
}

/// Asigna un rol de plataforma (soporte, comercial, superusuario…).
#[utoipa::path(
    post,
    path = "/asignaciones",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = AsignacionPlataformaPeticion,
    responses((status = CREATED, description = "Asignado", body = AsignacionPlataformaRespuesta))
)]
pub async fn asignar(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<AsignacionPlataformaPeticion>,
) -> ApiResult<(StatusCode, Json<AsignacionPlataformaRespuesta>)> {
    let a = uc::asignar_rol_plataforma(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        CuentaId::from_uuid(p.cuenta_id),
        RolId::from_uuid(p.rol_id),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(AsignacionPlataformaRespuesta { id: a.id.as_uuid() }),
    ))
}

/// Termina una asignación de plataforma (nunca el último superusuario).
#[utoipa::path(
    delete,
    path = "/asignaciones/{asignacion_id}",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("asignacion_id" = Uuid, Path, description = "Asignación")),
    responses(
        (status = NO_CONTENT, description = "Terminada"),
        (status = CONFLICT, description = "Último superusuario", body = ErrorRespuesta),
    )
)]
pub async fn revocar(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    uc::revocar_rol_plataforma(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        AsignacionRolId::from_uuid(id),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Bloquea una cuenta y cierra sus sesiones.
#[utoipa::path(
    post,
    path = "/cuentas/{cuenta_id}/bloquear",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("cuenta_id" = Uuid, Path, description = "Cuenta")),
    responses((status = NO_CONTENT, description = "Bloqueada"))
)]
pub async fn bloquear_cuenta(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    uc::bloquear_cuenta(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        CuentaId::from_uuid(id),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Reclamaciones de organizaciones pendientes de revisión.
#[utoipa::path(
    get,
    path = "/reclamaciones",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    responses((status = OK, description = "Pendientes", body = Vec<ReclamacionRespuesta>))
)]
pub async fn reclamaciones_pendientes(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
) -> ApiResult<Json<Vec<ReclamacionRespuesta>>> {
    let lista = org::reclamaciones_pendientes(&state.plataforma, &op.ctx).await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Decisión sobre una reclamación.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResolucionPeticion {
    pub aprobar: bool,
    pub motivo: Option<String>,
}

/// Aprueba (la cuenta queda como dueña de la ficha) o rechaza una reclamación.
#[utoipa::path(
    post,
    path = "/reclamaciones/{reclamacion_id}/resolver",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("reclamacion_id" = Uuid, Path, description = "Reclamación")),
    request_body = ResolucionPeticion,
    responses(
        (status = NO_CONTENT, description = "Resuelta"),
        (status = CONFLICT, description = "Ya estaba resuelta", body = ErrorRespuesta),
    )
)]
pub async fn resolver_reclamacion(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<ResolucionPeticion>,
) -> ApiResult<StatusCode> {
    org::resolver_reclamacion(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        ReclamacionId::from_uuid(id),
        p.aprobar,
        p.motivo.as_deref(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
