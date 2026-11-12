//! Roles y asignaciones de una copropiedad: `/api/v1/copropiedades/{copropiedad_id}/roles`,
//! `/asignaciones` y `/cuentas` (búsqueda para asignar). También los DTOs de roles que usa la
//! plataforma.

use application::{
    casos_uso::roles::{self as uc, NuevoRol, SolicitudAsignacion},
    puertos::{
        RelojSistema,
        identidad::{Asignacion, Rol},
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{
    AsignacionRolId, ConjuntoPermisos, CuentaId, Identificador, OrigenAsignacion, Permiso, RolId,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_ROLES,
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de roles dentro de una copropiedad.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_roles, crear_rol))
        .routes(routes!(editar_permisos))
        .routes(routes!(listar_asignaciones, asignar))
        .routes(routes!(revocar))
        .routes(routes!(buscar_cuenta))
}

/// Catálogo de permisos (`/api/v1/permisos`), para que el cliente arme roles propios.
pub fn rutas_catalogo() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(catalogo))
}

/// Permiso del catálogo.
#[derive(Debug, Serialize, ToSchema)]
pub struct PermisoRespuesta {
    #[schema(example = "roles.asignar")]
    pub codigo: &'static str,
    /// `plataforma` o `copropiedad`.
    #[schema(value_type = crate::esquemas::Ambito)]
    pub ambito: &'static str,
}

/// Catálogo completo de permisos. Requiere sesión.
#[utoipa::path(
    get,
    path = "",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    responses((status = OK, description = "Permisos", body = Vec<PermisoRespuesta>))
)]
pub async fn catalogo(_cuenta: crate::sesion::CuentaAutenticada) -> Json<Vec<PermisoRespuesta>> {
    Json(
        Permiso::TODOS
            .iter()
            .map(|p| PermisoRespuesta {
                codigo: p.as_str(),
                ambito: p.ambito().as_str(),
            })
            .collect(),
    )
}

/// Rol con sus permisos.
#[derive(Debug, Serialize, ToSchema)]
pub struct RolRespuesta {
    pub id: Uuid,
    #[schema(example = "administrador")]
    pub codigo: String,
    #[schema(example = "Administrador")]
    pub nombre: String,
    pub descripcion: Option<String>,
    /// `plataforma` o `copropiedad`.
    #[schema(value_type = crate::esquemas::Ambito)]
    pub ambito: String,
    /// Rol del sistema: no se edita.
    pub es_plantilla: bool,
    #[schema(value_type = Vec<crate::esquemas::Permiso>, example = json!(["roles.ver", "roles.asignar"]))]
    pub permisos: Vec<Permiso>,
}

impl From<Rol> for RolRespuesta {
    fn from(r: Rol) -> Self {
        Self {
            id: r.id.as_uuid(),
            codigo: r.codigo,
            nombre: r.nombre,
            descripcion: r.descripcion,
            ambito: r.ambito.as_str().into(),
            es_plantilla: r.es_plantilla,
            permisos: r.permisos.iter().collect(),
        }
    }
}

/// Rol propio nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoRolPeticion {
    /// 2 a 60 caracteres `a-z`, `0-9`, `_`.
    #[schema(example = "auxiliar_cartera")]
    pub codigo: String,
    #[schema(example = "Auxiliar de cartera")]
    pub nombre: String,
    pub descripcion: Option<String>,
    #[schema(value_type = Vec<crate::esquemas::Permiso>, example = json!(["contabilidad.ver"]))]
    pub permisos: Vec<Permiso>,
}

impl From<NuevoRolPeticion> for NuevoRol {
    fn from(p: NuevoRolPeticion) -> Self {
        Self {
            codigo: p.codigo,
            nombre: p.nombre,
            descripcion: p.descripcion,
            permisos: p.permisos.into_iter().collect(),
        }
    }
}

/// Permisos nuevos de un rol (reemplazan a los anteriores).
#[derive(Debug, Deserialize, ToSchema)]
pub struct PermisosPeticion {
    #[schema(value_type = Vec<crate::esquemas::Permiso>)]
    pub permisos: Vec<Permiso>,
}

impl PermisosPeticion {
    pub fn conjunto(self) -> ConjuntoPermisos {
        self.permisos.into_iter().collect()
    }
}

/// Roles de la copropiedad: plantillas del sistema y propios.
#[utoipa::path(
    get,
    path = "/roles",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Roles", body = Vec<RolRespuesta>),
        (status = FORBIDDEN, description = "Sin `roles.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_roles(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<RolRespuesta>>> {
    let roles = uc::listar_roles(&state.unidad, &en.ctx).await?;
    Ok(Json(roles.into_iter().map(Into::into).collect()))
}

/// Crea un rol propio. Nunca con permisos que quien lo crea no tiene.
#[utoipa::path(
    post,
    path = "/roles",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoRolPeticion,
    responses(
        (status = CREATED, description = "Rol creado", body = RolRespuesta),
        (status = FORBIDDEN, description = "Sin `roles.gestionar` o permisos que no tiene (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Código repetido (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_rol(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoRolPeticion>,
) -> ApiResult<(StatusCode, Json<RolRespuesta>)> {
    let rol = uc::crear_rol(&state.unidad, &en.ctx, p.into()).await?;
    Ok((StatusCode::CREATED, Json(rol.into())))
}

/// Reemplaza los permisos de un rol propio (las plantillas no se editan).
#[utoipa::path(
    put,
    path = "/roles/{rol_id}/permisos",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("rol_id" = Uuid, Path, description = "Rol"),
    ),
    request_body = PermisosPeticion,
    responses(
        (status = OK, description = "Rol actualizado", body = RolRespuesta),
        (status = CONFLICT, description = "Es una plantilla (`conflicto`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe en la copropiedad", body = ErrorRespuesta),
    )
)]
pub async fn editar_permisos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PermisosPeticion>,
) -> ApiResult<Json<RolRespuesta>> {
    let rol = uc::editar_permisos_rol(
        &state.unidad,
        &en.ctx,
        RolId::from_uuid(ruta.1),
        p.conjunto(),
    )
    .await?;
    Ok(Json(rol.into()))
}

/// Asignación de un rol con su vigencia.
#[derive(Debug, Serialize, ToSchema)]
pub struct AsignacionRespuesta {
    pub id: Uuid,
    pub cuenta_id: Uuid,
    pub rol_id: Uuid,
    pub vigente_desde: DateTime<Utc>,
    /// `null` = sin fecha de fin.
    pub vigente_hasta: Option<DateTime<Utc>>,
    /// `manual`, `contrato_servicio`, `propiedad`, `residencia` o `asamblea`.
    #[schema(value_type = crate::esquemas::OrigenAsignacion)]
    pub origen: String,
    pub origen_id: Option<Uuid>,
    pub asignado_por: Option<Uuid>,
}

impl From<Asignacion> for AsignacionRespuesta {
    fn from(a: Asignacion) -> Self {
        Self {
            id: a.id.as_uuid(),
            cuenta_id: a.cuenta_id.as_uuid(),
            rol_id: a.rol_id.as_uuid(),
            vigente_desde: a.vigente_desde,
            vigente_hasta: a.vigente_hasta,
            origen: a.origen.as_str().into(),
            origen_id: a.origen_id,
            asignado_por: a.asignado_por.map(CuentaId::as_uuid),
        }
    }
}

/// Filtro del historial de asignaciones.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroAsignaciones {
    /// Solo las de esta cuenta.
    pub cuenta_id: Option<Uuid>,
}

/// Historial de asignaciones (quién tuvo qué rol y cuándo), más recientes primero.
#[utoipa::path(
    get,
    path = "/asignaciones",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroAsignaciones),
    responses((status = OK, description = "Asignaciones", body = Vec<AsignacionRespuesta>))
)]
pub async fn listar_asignaciones(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroAsignaciones>,
) -> ApiResult<Json<Vec<AsignacionRespuesta>>> {
    let lista =
        uc::listar_asignaciones(&state.unidad, &en.ctx, f.cuenta_id.map(CuentaId::from_uuid))
            .await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Asignación manual de un rol.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AsignacionPeticion {
    pub cuenta_id: Uuid,
    pub rol_id: Uuid,
    /// Por defecto, desde ahora.
    pub vigente_desde: Option<DateTime<Utc>>,
    /// Por defecto, sin fecha de fin.
    pub vigente_hasta: Option<DateTime<Utc>>,
}

/// Asigna un rol a una cuenta. Nunca un rol con permisos que quien asigna no tiene.
#[utoipa::path(
    post,
    path = "/asignaciones",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = AsignacionPeticion,
    responses(
        (status = CREATED, description = "Asignado", body = AsignacionRespuesta),
        (status = FORBIDDEN, description = "Sin `roles.asignar` o rol con más permisos (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Rol o cuenta inexistentes", body = ErrorRespuesta),
    )
)]
pub async fn asignar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<AsignacionPeticion>,
) -> ApiResult<(StatusCode, Json<AsignacionRespuesta>)> {
    let a = uc::asignar_rol(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
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

/// Termina una asignación desde ahora. La copropiedad no se queda sin administrador.
#[utoipa::path(
    delete,
    path = "/asignaciones/{asignacion_id}",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("asignacion_id" = Uuid, Path, description = "Asignación"),
    ),
    responses(
        (status = NO_CONTENT, description = "Terminada"),
        (status = CONFLICT, description = "Es el último administrador (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn revocar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    uc::revocar_asignacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        AsignacionRolId::from_uuid(ruta.1),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Búsqueda exacta de una cuenta.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BusquedaCuenta {
    /// Usuario o correo exactos.
    pub identificador: String,
}

/// Cuenta encontrada.
#[derive(Debug, Serialize, ToSchema)]
pub struct CuentaEncontrada {
    pub id: Uuid,
    pub nombre: String,
}

/// Busca una cuenta por usuario o correo exactos para asignarle un rol (nunca listados).
#[utoipa::path(
    get,
    path = "/cuentas",
    tag = ETIQUETA_ROLES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), BusquedaCuenta),
    responses(
        (status = OK, description = "Cuenta", body = CuentaEncontrada),
        (status = NOT_FOUND, description = "No hay una cuenta activa con ese identificador", body = ErrorRespuesta),
    )
)]
pub async fn buscar_cuenta(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(b): Query<BusquedaCuenta>,
) -> ApiResult<Json<CuentaEncontrada>> {
    let identificador = Identificador::parse(&b.identificador)?;
    let cuenta = uc::buscar_cuenta(&state.unidad, &en.ctx, &identificador)
        .await?
        .ok_or(ApiError::App(application::AppError::NoEncontrado("cuenta")))?;
    Ok(Json(CuentaEncontrada {
        id: cuenta.id.as_uuid(),
        nombre: cuenta.nombre_mostrar,
    }))
}
