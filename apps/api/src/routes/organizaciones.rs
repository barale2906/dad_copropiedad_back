//! Organizaciones compartidas y contratos de servicio.
//!
//! - En una copropiedad: `/api/v1/copropiedades/{copropiedad_id}/organizaciones` (buscar por
//!   NIT, crear) y `/contratos-servicio` (vincular, terminar, asignar roles al personal).
//! - Para la cuenta: `/api/v1/organizaciones/{organizacion_id}/…` (editar la ficha global,
//!   miembros, reclamar).

use application::{
    casos_uso::organizaciones::{self as uc, NuevoContrato},
    puertos::{
        RelojSistema,
        identidad::{CambiosFicha, ContratoServicio, FichaOrganizacion, NuevaOrganizacion},
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{
    ContratoServicioId, CuentaId, NaturalezaOrganizacion, Nit, OrganizacionId, RolId,
    RolOrganizacion, TipoOrganizacion,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_ORGANIZACIONES,
    routes::roles::AsignacionRespuesta,
    sesion::{CuentaAutenticada, EnCopropiedad},
    state::AppState,
};

/// Rutas dentro de una copropiedad.
pub fn rutas_copropiedad() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(buscar_por_nit, crear))
        .routes(routes!(listar_contratos, vincular))
        .routes(routes!(terminar_contrato))
        .routes(routes!(asignar_por_contrato))
}

/// Rutas de la cuenta sobre una organización (se montan en `/api/v1/organizaciones`).
pub fn rutas_cuenta() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(editar_ficha))
        .routes(routes!(guardar_miembro))
        .routes(routes!(reclamar))
}

fn texto_valor<T: Copy>(v: &[T], f: fn(T) -> &'static str) -> Vec<String> {
    v.iter().map(|x| f(*x).to_owned()).collect()
}

/// Ficha pública de una organización.
#[derive(Debug, Serialize, ToSchema)]
pub struct FichaOrganizacionRespuesta {
    pub id: Uuid,
    #[schema(example = "Seguridad Andina S.A.S.")]
    pub razon_social: String,
    /// `juridica` o `natural_con_rut`.
    #[schema(value_type = crate::esquemas::NaturalezaOrganizacion)]
    pub naturaleza: String,
    #[schema(example = json!(["seguridad"]))]
    pub tipos: Vec<String>,
    /// `sin_reclamar`, `reclamada` o `suspendida`.
    #[schema(value_type = crate::esquemas::EstadoOrganizacion)]
    pub estado: String,
}

impl From<FichaOrganizacion> for FichaOrganizacionRespuesta {
    fn from(f: FichaOrganizacion) -> Self {
        Self {
            id: f.id.as_uuid(),
            razon_social: f.razon_social,
            naturaleza: f.naturaleza.as_str().into(),
            tipos: texto_valor(&f.tipos, TipoOrganizacion::as_str),
            estado: f.estado.as_str().into(),
        }
    }
}

/// NIT exacto a buscar.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BusquedaNit {
    /// Con o sin puntos y dígito de verificación (`900.123.456-8`).
    #[param(example = "900123456-8")]
    pub nit: String,
}

/// Busca una organización por NIT exacto, antes de crearla (para reutilizarla).
#[utoipa::path(
    get,
    path = "/organizaciones",
    tag = ETIQUETA_ORGANIZACIONES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), BusquedaNit),
    responses(
        (status = OK, description = "Ficha pública", body = FichaOrganizacionRespuesta),
        (status = NOT_FOUND, description = "No existe: se puede crear", body = ErrorRespuesta),
    )
)]
pub async fn buscar_por_nit(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(b): Query<BusquedaNit>,
) -> ApiResult<Json<FichaOrganizacionRespuesta>> {
    let nit = Nit::new(&b.nit)?;
    let ficha = uc::buscar_por_nit(&state.unidad, &en.ctx, &nit)
        .await?
        .ok_or(ApiError::App(application::AppError::NoEncontrado(
            "organización",
        )))?;
    Ok(Json(ficha.into()))
}

/// Organización nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaOrganizacionPeticion {
    #[schema(example = "900.123.456-8")]
    pub nit: String,
    pub razon_social: String,
    /// `juridica` o `natural_con_rut`.
    #[schema(value_type = crate::esquemas::NaturalezaOrganizacion)]
    pub naturaleza: String,
    #[schema(value_type = Vec<crate::esquemas::TipoOrganizacion>, example = json!(["seguridad"]))]
    pub tipos: Vec<String>,
    pub correo_contacto: Option<String>,
    pub telefono_contacto: Option<String>,
}

impl TryFrom<NuevaOrganizacionPeticion> for NuevaOrganizacion {
    type Error = ApiError;

    fn try_from(p: NuevaOrganizacionPeticion) -> Result<Self, ApiError> {
        Ok(Self {
            nit: Nit::new(&p.nit)?,
            razon_social: p.razon_social,
            naturaleza: NaturalezaOrganizacion::try_from(p.naturaleza.as_str())?,
            tipos: p
                .tipos
                .iter()
                .map(|t| TipoOrganizacion::try_from(t.as_str()))
                .collect::<Result<_, _>>()?,
            correo_contacto: p.correo_contacto,
            telefono_contacto: p.telefono_contacto,
        })
    }
}

/// Organización creada o reutilizada.
#[derive(Debug, Serialize, ToSchema)]
pub struct OrganizacionCreada {
    pub id: Uuid,
    /// `false` si ya existía con ese NIT (se reutiliza).
    pub creada: bool,
}

/// Crea la organización (o devuelve la existente con ese NIT).
#[utoipa::path(
    post,
    path = "/organizaciones",
    tag = ETIQUETA_ORGANIZACIONES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevaOrganizacionPeticion,
    responses(
        (status = OK, description = "Creada o reutilizada", body = OrganizacionCreada),
        (status = UNPROCESSABLE_ENTITY, description = "NIT o datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevaOrganizacionPeticion>,
) -> ApiResult<Json<OrganizacionCreada>> {
    let (id, creada) = uc::crear_organizacion(&state.unidad, &en.ctx, p.try_into()?).await?;
    Ok(Json(OrganizacionCreada {
        id: id.as_uuid(),
        creada,
    }))
}

/// Contrato de servicio.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContratoRespuesta {
    pub id: Uuid,
    pub organizacion_id: Uuid,
    #[schema(value_type = crate::esquemas::TipoOrganizacion)]
    pub tipo_servicio: String,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    /// `vigente` o `terminado`.
    #[schema(value_type = crate::esquemas::EstadoContrato)]
    pub estado: String,
    #[schema(value_type = Option<String>, example = "2500000.00")]
    pub valor: Option<Decimal>,
}

impl From<ContratoServicio> for ContratoRespuesta {
    fn from(c: ContratoServicio) -> Self {
        Self {
            id: c.id.as_uuid(),
            organizacion_id: c.organizacion_id.as_uuid(),
            tipo_servicio: c.tipo_servicio.as_str().into(),
            vigente_desde: c.vigente_desde,
            vigente_hasta: c.vigente_hasta,
            estado: c.estado.as_str().into(),
            valor: c.valor,
        }
    }
}

/// Contratos de la copropiedad.
#[utoipa::path(
    get,
    path = "/contratos-servicio",
    tag = ETIQUETA_ORGANIZACIONES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses((status = OK, description = "Contratos", body = Vec<ContratoRespuesta>))
)]
pub async fn listar_contratos(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<ContratoRespuesta>>> {
    let lista = uc::listar_contratos(&state.unidad, &en.ctx).await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Contrato nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ContratoPeticion {
    pub organizacion_id: Uuid,
    #[schema(value_type = crate::esquemas::TipoOrganizacion)]
    pub tipo_servicio: String,
    pub vigente_desde: Option<DateTime<Utc>>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    #[schema(value_type = Option<String>, example = "2500000.00")]
    pub valor: Option<Decimal>,
}

/// Vincula una organización con la copropiedad mediante un contrato de servicio.
#[utoipa::path(
    post,
    path = "/contratos-servicio",
    tag = ETIQUETA_ORGANIZACIONES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = ContratoPeticion,
    responses(
        (status = CREATED, description = "Contrato creado", body = ContratoRespuesta),
        (status = NOT_FOUND, description = "La organización no existe", body = ErrorRespuesta),
    )
)]
pub async fn vincular(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<ContratoPeticion>,
) -> ApiResult<(StatusCode, Json<ContratoRespuesta>)> {
    let c = uc::vincular_organizacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoContrato {
            organizacion_id: OrganizacionId::from_uuid(p.organizacion_id),
            tipo_servicio: TipoOrganizacion::try_from(p.tipo_servicio.as_str())?,
            vigente_desde: p.vigente_desde,
            vigente_hasta: p.vigente_hasta,
            valor: p.valor,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Roles que vencieron al terminar el contrato.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContratoTerminado {
    pub roles_vencidos: u64,
}

/// Termina el contrato: vencen los roles del personal derivados de él.
#[utoipa::path(
    post,
    path = "/contratos-servicio/{contrato_id}/terminar",
    tag = ETIQUETA_ORGANIZACIONES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("contrato_id" = Uuid, Path, description = "Contrato"),
    ),
    responses((status = OK, description = "Terminado", body = ContratoTerminado))
)]
pub async fn terminar_contrato(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ContratoTerminado>> {
    let roles_vencidos = uc::terminar_contrato(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ContratoServicioId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(ContratoTerminado { roles_vencidos }))
}

/// Rol para una persona del personal de la organización.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AsignacionContratoPeticion {
    pub cuenta_id: Uuid,
    pub rol_id: Uuid,
}

/// Asigna un rol al personal de la organización, derivado del contrato (vence con él).
#[utoipa::path(
    post,
    path = "/contratos-servicio/{contrato_id}/asignaciones",
    tag = ETIQUETA_ORGANIZACIONES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("contrato_id" = Uuid, Path, description = "Contrato"),
    ),
    request_body = AsignacionContratoPeticion,
    responses(
        (status = CREATED, description = "Asignado", body = AsignacionRespuesta),
        (status = CONFLICT, description = "Contrato no vigente (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn asignar_por_contrato(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AsignacionContratoPeticion>,
) -> ApiResult<(StatusCode, Json<AsignacionRespuesta>)> {
    let a = uc::asignar_rol_por_contrato(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ContratoServicioId::from_uuid(ruta.1),
        CuentaId::from_uuid(p.cuenta_id),
        RolId::from_uuid(p.rol_id),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(a.into())))
}

/// Cambios a la ficha global (lo que no venga no cambia).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CambiosFichaPeticion {
    pub razon_social: Option<String>,
    #[schema(value_type = Option<Vec<crate::esquemas::TipoOrganizacion>>)]
    pub tipos: Option<Vec<String>>,
    pub correo_contacto: Option<String>,
    pub telefono_contacto: Option<String>,
}

/// Edita la ficha global: soporte; sin reclamar, quien la creó; reclamada, dueños y gestores.
#[utoipa::path(
    patch,
    path = "/{organizacion_id}",
    tag = ETIQUETA_ORGANIZACIONES,
    security(("bearer" = [])),
    params(("organizacion_id" = Uuid, Path, description = "Organización")),
    request_body = CambiosFichaPeticion,
    responses(
        (status = NO_CONTENT, description = "Ficha actualizada"),
        (status = FORBIDDEN, description = "No puede editarla (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn editar_ficha(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
    Path(id): Path<Uuid>,
    Json(p): Json<CambiosFichaPeticion>,
) -> ApiResult<StatusCode> {
    let tipos = p
        .tipos
        .map(|t| {
            t.iter()
                .map(|x| TipoOrganizacion::try_from(x.as_str()))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    uc::editar_ficha(
        &state.plataforma,
        &RelojSistema,
        &cuenta.ctx,
        OrganizacionId::from_uuid(id),
        CambiosFicha {
            razon_social: p.razon_social,
            tipos,
            correo_contacto: p.correo_contacto,
            telefono_contacto: p.telefono_contacto,
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Miembro de la organización.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MiembroPeticion {
    pub cuenta_id: Uuid,
    /// `dueno`, `gestor` u `operario`.
    #[schema(value_type = crate::esquemas::RolOrganizacion)]
    pub rol: String,
}

/// Agrega una cuenta a la organización o cambia su papel.
#[utoipa::path(
    post,
    path = "/{organizacion_id}/miembros",
    tag = ETIQUETA_ORGANIZACIONES,
    security(("bearer" = [])),
    params(("organizacion_id" = Uuid, Path, description = "Organización")),
    request_body = MiembroPeticion,
    responses(
        (status = NO_CONTENT, description = "Guardado"),
        (status = FORBIDDEN, description = "Sin permiso (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn guardar_miembro(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
    Path(id): Path<Uuid>,
    Json(p): Json<MiembroPeticion>,
) -> ApiResult<StatusCode> {
    uc::guardar_miembro(
        &state.plataforma,
        &cuenta.ctx,
        OrganizacionId::from_uuid(id),
        CuentaId::from_uuid(p.cuenta_id),
        RolOrganizacion::try_from(p.rol.as_str())?,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Solicitud para manejar la ficha de una organización.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReclamacionPeticion {
    /// Por qué representa a la organización (cargo, soporte que enviará…).
    pub mensaje: String,
}

/// Reclamación creada.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReclamacionRespuesta {
    pub id: Uuid,
    pub organizacion_id: Uuid,
    pub cuenta_id: Uuid,
    pub mensaje: String,
    /// `pendiente`, `aprobada` o `rechazada`.
    #[schema(value_type = crate::esquemas::EstadoReclamacion)]
    pub estado: String,
    pub creada_en: DateTime<Utc>,
}

impl From<application::puertos::identidad::Reclamacion> for ReclamacionRespuesta {
    fn from(r: application::puertos::identidad::Reclamacion) -> Self {
        Self {
            id: r.id.as_uuid(),
            organizacion_id: r.organizacion_id.as_uuid(),
            cuenta_id: r.cuenta_id.as_uuid(),
            mensaje: r.mensaje,
            estado: r.estado.as_str().into(),
            creada_en: r.creada_en,
        }
    }
}

/// Pide manejar la ficha de la organización; la revisa soporte.
#[utoipa::path(
    post,
    path = "/{organizacion_id}/reclamaciones",
    tag = ETIQUETA_ORGANIZACIONES,
    security(("bearer" = [])),
    params(("organizacion_id" = Uuid, Path, description = "Organización")),
    request_body = ReclamacionPeticion,
    responses(
        (status = CREATED, description = "Reclamación pendiente de revisión", body = ReclamacionRespuesta),
        (status = CONFLICT, description = "Ya reclamada o con una pendiente (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn reclamar(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
    Path(id): Path<Uuid>,
    Json(p): Json<ReclamacionPeticion>,
) -> ApiResult<(StatusCode, Json<ReclamacionRespuesta>)> {
    let r = uc::reclamar_organizacion(
        &state.plataforma,
        &RelojSistema,
        &cuenta.ctx,
        OrganizacionId::from_uuid(id),
        &p.mensaje,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}
