//! Inventario y mantenimiento (plan 07c, módulo `gestion_administrativa`): `/activos`,
//! `/planes-mantenimiento`, `/ordenes-trabajo`, `/obligaciones` y `/mantenimiento/…`, dentro de
//! `/api/v1/copropiedades/{copropiedad_id}`.

use application::{
    casos_uso::mantenimiento::{
        self as uc, AlertasMantenimiento, AvanceOrden, CambiosActivo, ConsultaOrdenes,
        DatosObligacion, DatosPlan, DetalleOrden, Ejecucion, HojaDeVida, NuevaOrden, NuevoActivo,
        NuevoMovimiento,
    },
    puertos::{
        RelojSistema,
        mantenimiento::{
            Activo, ArchivoActivo, Cumplimiento, FiltroActivos, Movimiento, Obligacion,
            OrdenTrabajo, PlanMantenimiento, TableroMantenimiento,
        },
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ActivoId, AgrupacionId, ArchivoId, ClaseActivo, CodigoActivo, CuentaId, EstadoActivo,
    EstadoOrden, Frecuencia, MomentoEvidencia, ObligacionId, OrdenTrabajoId, OrganizacionId,
    OrigenOrden, PlanMantenimientoId, PuntoChecklist, Recurrencia, RecursoId, ReservaId, TicketId,
    TipoArchivoActivo, TipoMovimiento, TipoOrden, mantenimiento::ObligacionSugerida,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_MANTENIMIENTO,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    routes::tickets::ResponsableDto,
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de inventario y mantenimiento.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_activos, registrar_activo))
        .routes(routes!(hoja_de_vida, editar_activo))
        .routes(routes!(adjuntar_archivo))
        .routes(routes!(registrar_movimiento))
        .routes(routes!(listar_planes, crear_plan))
        .routes(routes!(editar_plan))
        .routes(routes!(listar_ordenes, crear_orden))
        .routes(routes!(ver_orden))
        .routes(routes!(avanzar_orden))
        .routes(routes!(agregar_evidencia))
        .routes(routes!(obligaciones_sugeridas))
        .routes(routes!(listar_obligaciones, crear_obligacion))
        .routes(routes!(editar_obligacion))
        .routes(routes!(listar_cumplimientos, registrar_cumplimiento))
        .routes(routes!(alertas))
        .routes(routes!(tablero))
}

// ---------------------------------------------------------------------------------------------
// Activos
// ---------------------------------------------------------------------------------------------

/// Activo del inventario.
#[derive(Debug, Serialize, ToSchema)]
pub struct ActivoRespuesta {
    pub id: Uuid,
    #[schema(example = "BOMBA-01")]
    pub codigo: String,
    pub nombre: String,
    #[schema(value_type = crate::esquemas::ClaseActivo)]
    pub clase: ClaseActivo,
    pub marca: Option<String>,
    pub modelo: Option<String>,
    pub serie: Option<String>,
    pub ubicacion: Option<String>,
    pub agrupacion_id: Option<Uuid>,
    pub recurso_id: Option<Uuid>,
    #[schema(value_type = crate::esquemas::EstadoActivo)]
    pub estado: EstadoActivo,
    pub fecha_compra: Option<NaiveDate>,
    #[schema(value_type = Option<String>, example = "12500000.00")]
    pub valor: Option<Decimal>,
    pub garantia_vence: Option<NaiveDate>,
    pub proveedor_organizacion_id: Option<Uuid>,
    pub creado_en: DateTime<Utc>,
}

impl From<Activo> for ActivoRespuesta {
    fn from(a: Activo) -> Self {
        Self {
            id: a.id.as_uuid(),
            codigo: a.codigo.as_str().to_owned(),
            nombre: a.nombre,
            clase: a.clase,
            marca: a.marca,
            modelo: a.modelo,
            serie: a.serie,
            ubicacion: a.ubicacion,
            agrupacion_id: a.agrupacion_id.map(AgrupacionId::as_uuid),
            recurso_id: a.recurso_id.map(RecursoId::as_uuid),
            estado: a.estado,
            fecha_compra: a.fecha_compra,
            valor: a.valor,
            garantia_vence: a.garantia_vence,
            proveedor_organizacion_id: a.proveedor_organizacion_id.map(OrganizacionId::as_uuid),
            creado_en: a.creado_en,
        }
    }
}

/// Filtro del inventario.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroActivosQuery {
    #[param(value_type = Option<crate::esquemas::ClaseActivo>)]
    pub clase: Option<ClaseActivo>,
    #[param(value_type = Option<crate::esquemas::EstadoActivo>)]
    pub estado: Option<EstadoActivo>,
    /// Fragmento del nombre o del código.
    pub q: Option<String>,
}

/// Inventario de activos (paginado).
#[utoipa::path(
    get,
    path = "/activos",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroActivosQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Activos", body = PaginaRespuesta<ActivoRespuesta>),
        (status = FORBIDDEN, description = "Sin `inventario.ver` (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_activos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroActivosQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<ActivoRespuesta>>> {
    let pagina = uc::listar_activos(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FiltroActivos {
            clase: f.clase,
            estado: f.estado,
            texto: f.q,
        },
        p.validar()?,
    )
    .await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Activo nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ActivoPeticion {
    #[schema(value_type = String, example = "BOMBA-01")]
    pub codigo: CodigoActivo,
    #[schema(example = "Bomba de agua potable 1")]
    pub nombre: String,
    #[schema(value_type = crate::esquemas::ClaseActivo)]
    pub clase: ClaseActivo,
    pub marca: Option<String>,
    pub modelo: Option<String>,
    pub serie: Option<String>,
    pub ubicacion: Option<String>,
    pub agrupacion_id: Option<Uuid>,
    /// Bien común donde está o herramienta prestable (recurso del motor de reservas).
    pub recurso_id: Option<Uuid>,
    pub fecha_compra: Option<NaiveDate>,
    #[schema(value_type = Option<String>, example = "12500000")]
    pub valor: Option<Decimal>,
    pub garantia_vence: Option<NaiveDate>,
    pub proveedor_organizacion_id: Option<Uuid>,
}

/// Registra un activo (queda `operativo`, con un movimiento de entrada).
#[utoipa::path(
    post,
    path = "/activos",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = ActivoPeticion,
    responses(
        (status = CREATED, description = "Activo registrado", body = ActivoRespuesta),
        (status = CONFLICT, description = "Código repetido (`conflicto`)", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `inventario.editar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_activo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<ActivoPeticion>,
) -> ApiResult<(StatusCode, Json<ActivoRespuesta>)> {
    let a = uc::registrar_activo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoActivo {
            codigo: p.codigo,
            nombre: p.nombre,
            clase: p.clase,
            marca: p.marca,
            modelo: p.modelo,
            serie: p.serie,
            ubicacion: p.ubicacion,
            agrupacion_id: p.agrupacion_id.map(AgrupacionId::from_uuid),
            recurso_id: p.recurso_id.map(RecursoId::from_uuid),
            fecha_compra: p.fecha_compra,
            valor: p.valor,
            garantia_vence: p.garantia_vence,
            proveedor_organizacion_id: p.proveedor_organizacion_id.map(OrganizacionId::from_uuid),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(a.into())))
}

/// Archivo de la hoja de vida.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ArchivoActivoDto {
    pub archivo_id: Uuid,
    /// `manual`, `foto`, `factura`, `garantia` u `otro`.
    #[schema(value_type = crate::esquemas::TipoArchivoActivo)]
    pub tipo: TipoArchivoActivo,
}

impl From<ArchivoActivo> for ArchivoActivoDto {
    fn from(a: ArchivoActivo) -> Self {
        Self {
            archivo_id: a.archivo_id.as_uuid(),
            tipo: a.tipo,
        }
    }
}

/// Movimiento de inventario.
#[derive(Debug, Serialize, ToSchema)]
pub struct MovimientoRespuesta {
    pub id: Uuid,
    #[schema(value_type = crate::esquemas::TipoMovimiento)]
    pub tipo: TipoMovimiento,
    pub ubicacion_destino: Option<String>,
    pub reserva_id: Option<Uuid>,
    pub soporte_archivo_id: Option<Uuid>,
    pub observacion: Option<String>,
    pub registrado_por: Option<Uuid>,
    pub ocurrido_en: DateTime<Utc>,
}

impl From<Movimiento> for MovimientoRespuesta {
    fn from(m: Movimiento) -> Self {
        Self {
            id: m.id.as_uuid(),
            tipo: m.tipo,
            ubicacion_destino: m.ubicacion_destino,
            reserva_id: m.reserva_id.map(ReservaId::as_uuid),
            soporte_archivo_id: m.soporte_archivo_id.map(ArchivoId::as_uuid),
            observacion: m.observacion,
            registrado_por: m.registrado_por.map(CuentaId::as_uuid),
            ocurrido_en: m.ocurrido_en,
        }
    }
}

/// Hoja de vida de un activo.
#[derive(Debug, Serialize, ToSchema)]
pub struct HojaDeVidaRespuesta {
    pub activo: ActivoRespuesta,
    pub archivos: Vec<ArchivoActivoDto>,
    pub movimientos: Vec<MovimientoRespuesta>,
    pub ordenes: Vec<OrdenRespuesta>,
    /// Costo acumulado de las órdenes.
    #[schema(value_type = String, example = "350000.00")]
    pub costo_total: Decimal,
}

impl From<HojaDeVida> for HojaDeVidaRespuesta {
    fn from(h: HojaDeVida) -> Self {
        Self {
            activo: h.activo.into(),
            archivos: h.archivos.into_iter().map(Into::into).collect(),
            movimientos: h.movimientos.into_iter().map(Into::into).collect(),
            ordenes: h.ordenes.into_iter().map(Into::into).collect(),
            costo_total: h.costo_total,
        }
    }
}

fn id_activo(ruta: (Uuid, Uuid)) -> ActivoId {
    ActivoId::from_uuid(ruta.1)
}

/// Hoja de vida: el activo con sus archivos, movimientos y órdenes de trabajo.
#[utoipa::path(
    get,
    path = "/activos/{activo_id}",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("activo_id" = Uuid, Path, description = "Activo"),
    ),
    responses(
        (status = OK, description = "Hoja de vida", body = HojaDeVidaRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn hoja_de_vida(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<HojaDeVidaRespuesta>> {
    let h = uc::hoja_de_vida(&state.unidad, &RelojSistema, &en.ctx, id_activo(ruta)).await?;
    Ok(Json(h.into()))
}

/// Cambios a un activo (los omitidos no cambian; la ubicación cambia con un traslado).
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct CambiosActivoPeticion {
    pub nombre: Option<String>,
    pub marca: Option<String>,
    pub modelo: Option<String>,
    pub serie: Option<String>,
    pub recurso_id: Option<Uuid>,
    #[schema(value_type = Option<String>)]
    pub valor: Option<Decimal>,
    pub garantia_vence: Option<NaiveDate>,
    pub proveedor_organizacion_id: Option<Uuid>,
    /// `operativo`, `con_falla`, `en_reparacion` o `fuera_de_servicio` (la baja va por movimiento).
    #[schema(value_type = Option<crate::esquemas::EstadoActivo>)]
    pub estado: Option<EstadoActivo>,
}

/// Edita los datos o el estado operativo de un activo.
#[utoipa::path(
    patch,
    path = "/activos/{activo_id}",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("activo_id" = Uuid, Path, description = "Activo"),
    ),
    request_body = CambiosActivoPeticion,
    responses(
        (status = OK, description = "Activo", body = ActivoRespuesta),
        (status = CONFLICT, description = "Dado de baja (`conflicto`) o estado no permitido (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn editar_activo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<CambiosActivoPeticion>,
) -> ApiResult<Json<ActivoRespuesta>> {
    let a = uc::editar_activo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_activo(ruta),
        CambiosActivo {
            nombre: p.nombre,
            marca: p.marca,
            modelo: p.modelo,
            serie: p.serie,
            recurso_id: p.recurso_id.map(RecursoId::from_uuid),
            valor: p.valor,
            garantia_vence: p.garantia_vence,
            proveedor_organizacion_id: p.proveedor_organizacion_id.map(OrganizacionId::from_uuid),
            estado: p.estado,
        },
    )
    .await?;
    Ok(Json(a.into()))
}

/// Agrega un archivo (manual, foto, factura, garantía) a la hoja de vida.
#[utoipa::path(
    post,
    path = "/activos/{activo_id}/archivos",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("activo_id" = Uuid, Path, description = "Activo"),
    ),
    request_body = ArchivoActivoDto,
    responses(
        (status = NO_CONTENT, description = "Archivo agregado"),
        (status = NOT_FOUND, description = "Activo o archivo inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya estaba (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn adjuntar_archivo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ArchivoActivoDto>,
) -> ApiResult<StatusCode> {
    uc::adjuntar_archivo_activo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_activo(ruta),
        ArchivoActivo {
            archivo_id: ArchivoId::from_uuid(p.archivo_id),
            tipo: p.tipo,
        },
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Movimiento nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MovimientoPeticion {
    /// `entrada`, `salida`, `traslado`, `prestamo`, `devolucion` o `baja`.
    #[schema(value_type = crate::esquemas::TipoMovimiento)]
    pub tipo: TipoMovimiento,
    /// Obligatoria en los traslados.
    pub ubicacion_destino: Option<String>,
    /// Reserva de la herramienta prestada.
    pub reserva_id: Option<Uuid>,
    /// Obligatorio en las bajas (acta o documento).
    pub soporte_archivo_id: Option<Uuid>,
    pub observacion: Option<String>,
}

/// Registra un movimiento (el traslado cambia la ubicación; la baja es definitiva).
#[utoipa::path(
    post,
    path = "/activos/{activo_id}/movimientos",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("activo_id" = Uuid, Path, description = "Activo"),
    ),
    request_body = MovimientoPeticion,
    responses(
        (status = CREATED, description = "Movimiento", body = MovimientoRespuesta),
        (status = CONFLICT, description = "Activo dado de baja (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Falta el destino o el soporte (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_movimiento(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<MovimientoPeticion>,
) -> ApiResult<(StatusCode, Json<MovimientoRespuesta>)> {
    let m = uc::registrar_movimiento(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_activo(ruta),
        NuevoMovimiento {
            tipo: p.tipo,
            ubicacion_destino: p.ubicacion_destino,
            reserva_id: p.reserva_id.map(ReservaId::from_uuid),
            soporte_archivo_id: p.soporte_archivo_id.map(ArchivoId::from_uuid),
            observacion: p.observacion,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(m.into())))
}

// ---------------------------------------------------------------------------------------------
// Planes preventivos
// ---------------------------------------------------------------------------------------------

/// Recurrencia: «cada `intervalo` `frecuencia` desde `inicio`».
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RecurrenciaDto {
    /// `dias`, `semanas` o `meses` (anual = 12 meses).
    #[schema(value_type = crate::esquemas::Frecuencia)]
    pub frecuencia: Frecuencia,
    #[schema(example = 1)]
    pub intervalo: u32,
    pub inicio: NaiveDate,
}

/// Plan preventivo.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlanRespuesta {
    pub id: Uuid,
    pub activo_id: Option<Uuid>,
    pub ubicacion: Option<String>,
    pub actividad: String,
    pub recurrencia: RecurrenciaDto,
    pub anticipacion_dias: u32,
    pub responsable: Option<ResponsableDto>,
    pub duracion_estimada_min: Option<u32>,
    pub checklist: Vec<String>,
    pub activo: bool,
}

impl From<PlanMantenimiento> for PlanRespuesta {
    fn from(p: PlanMantenimiento) -> Self {
        Self {
            id: p.id.as_uuid(),
            activo_id: p.activo_id.map(ActivoId::as_uuid),
            ubicacion: p.ubicacion,
            actividad: p.actividad,
            recurrencia: RecurrenciaDto {
                frecuencia: p.recurrencia.frecuencia,
                intervalo: p.recurrencia.intervalo,
                inicio: p.recurrencia.inicio,
            },
            anticipacion_dias: p.anticipacion_dias,
            responsable: p.responsable.map(Into::into),
            duracion_estimada_min: p.duracion_estimada_min,
            checklist: p.checklist,
            activo: p.activo,
        }
    }
}

/// Datos de un plan preventivo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PlanPeticion {
    pub activo_id: Option<Uuid>,
    pub ubicacion: Option<String>,
    #[schema(example = "Mantenimiento preventivo de la bomba")]
    pub actividad: String,
    pub recurrencia: RecurrenciaDto,
    /// Días antes de la fecha en que se genera la orden (7 si se omite).
    #[serde(default = "siete")]
    pub anticipacion_dias: u32,
    pub responsable: Option<ResponsableDto>,
    pub duracion_estimada_min: Option<u32>,
    #[serde(default)]
    pub checklist: Vec<String>,
    #[serde(default = "verdadero")]
    pub activo: bool,
}

const fn siete() -> u32 {
    7
}

const fn verdadero() -> bool {
    true
}

impl PlanPeticion {
    fn datos(self) -> ApiResult<DatosPlan> {
        Ok(DatosPlan {
            activo_id: self.activo_id.map(ActivoId::from_uuid),
            ubicacion: self.ubicacion,
            actividad: self.actividad,
            recurrencia: Recurrencia::new(
                self.recurrencia.frecuencia,
                self.recurrencia.intervalo,
                self.recurrencia.inicio,
            )?,
            anticipacion_dias: self.anticipacion_dias,
            responsable: ResponsableDto::opcional(self.responsable)?,
            duracion_estimada_min: self.duracion_estimada_min,
            checklist: self.checklist,
            activo: self.activo,
        })
    }
}

/// Filtro de planes.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroPlanes {
    /// Solo los activos.
    #[serde(default)]
    pub activos: bool,
}

/// Planes preventivos.
#[utoipa::path(
    get,
    path = "/planes-mantenimiento",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroPlanes),
    responses(
        (status = OK, description = "Planes", body = Vec<PlanRespuesta>),
        (status = FORBIDDEN, description = "Sin `mantenimiento.planificar` ni `inventario.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_planes(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroPlanes>,
) -> ApiResult<Json<Vec<PlanRespuesta>>> {
    let v = uc::listar_planes(&state.unidad, &RelojSistema, &en.ctx, f.activos).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea un plan preventivo; el worker genera sus órdenes con la anticipación indicada.
#[utoipa::path(
    post,
    path = "/planes-mantenimiento",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = PlanPeticion,
    responses(
        (status = CREATED, description = "Plan", body = PlanRespuesta),
        (status = FORBIDDEN, description = "Sin `mantenimiento.planificar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_plan(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<PlanPeticion>,
) -> ApiResult<(StatusCode, Json<PlanRespuesta>)> {
    let plan = uc::guardar_plan(&state.unidad, &RelojSistema, &en.ctx, None, p.datos()?).await?;
    Ok((StatusCode::CREATED, Json(plan.into())))
}

/// Reemplaza un plan preventivo (las órdenes ya generadas no cambian).
#[utoipa::path(
    put,
    path = "/planes-mantenimiento/{plan_id}",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("plan_id" = Uuid, Path, description = "Plan"),
    ),
    request_body = PlanPeticion,
    responses(
        (status = OK, description = "Plan", body = PlanRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn editar_plan(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PlanPeticion>,
) -> ApiResult<Json<PlanRespuesta>> {
    let plan = uc::guardar_plan(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        Some(PlanMantenimientoId::from_uuid(ruta.1)),
        p.datos()?,
    )
    .await?;
    Ok(Json(plan.into()))
}

// ---------------------------------------------------------------------------------------------
// Órdenes de trabajo
// ---------------------------------------------------------------------------------------------

/// Punto del checklist.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PuntoChecklistDto {
    pub descripcion: String,
    pub cumplido: bool,
    pub observacion: Option<String>,
}

impl From<PuntoChecklist> for PuntoChecklistDto {
    fn from(p: PuntoChecklist) -> Self {
        Self {
            descripcion: p.descripcion,
            cumplido: p.cumplido,
            observacion: p.observacion,
        }
    }
}

/// Orden de trabajo.
#[derive(Debug, Serialize, ToSchema)]
pub struct OrdenRespuesta {
    pub id: Uuid,
    #[schema(example = "OT-2026-00007")]
    pub numero: String,
    #[schema(value_type = crate::esquemas::OrigenOrden)]
    pub origen: OrigenOrden,
    #[schema(value_type = crate::esquemas::TipoOrden)]
    pub tipo: TipoOrden,
    pub plan_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    pub activo_id: Option<Uuid>,
    pub ubicacion: Option<String>,
    pub descripcion: String,
    pub asignado: Option<ResponsableDto>,
    pub programada_para: NaiveDate,
    #[schema(value_type = crate::esquemas::EstadoOrden)]
    pub estado: EstadoOrden,
    pub checklist: Vec<PuntoChecklistDto>,
    #[schema(value_type = String, example = "0.00")]
    pub costo_mano_obra: Decimal,
    #[schema(value_type = String, example = "0.00")]
    pub costo_repuestos: Decimal,
    pub observaciones: Option<String>,
    pub recibido_por: Option<String>,
    pub firma_archivo_id: Option<Uuid>,
    pub iniciada_en: Option<DateTime<Utc>>,
    pub ejecutada_en: Option<DateTime<Utc>>,
    pub verificada_en: Option<DateTime<Utc>>,
    pub verificada_por: Option<Uuid>,
    pub creado_en: DateTime<Utc>,
}

impl From<OrdenTrabajo> for OrdenRespuesta {
    fn from(o: OrdenTrabajo) -> Self {
        Self {
            id: o.id.as_uuid(),
            numero: o.numero.to_string(),
            origen: o.origen,
            tipo: o.tipo,
            plan_id: o.plan_id.map(PlanMantenimientoId::as_uuid),
            ticket_id: o.ticket_id.map(TicketId::as_uuid),
            activo_id: o.activo_id.map(ActivoId::as_uuid),
            ubicacion: o.ubicacion,
            descripcion: o.descripcion,
            asignado: o.asignado.map(Into::into),
            programada_para: o.programada_para,
            estado: o.estado,
            checklist: o.checklist.into_iter().map(Into::into).collect(),
            costo_mano_obra: o.costo_mano_obra,
            costo_repuestos: o.costo_repuestos,
            observaciones: o.observaciones,
            recibido_por: o.recibido_por,
            firma_archivo_id: o.firma_archivo_id.map(ArchivoId::as_uuid),
            iniciada_en: o.iniciada_en,
            ejecutada_en: o.ejecutada_en,
            verificada_en: o.verificada_en,
            verificada_por: o.verificada_por.map(CuentaId::as_uuid),
            creado_en: o.creado_en,
        }
    }
}

/// Filtro de órdenes.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroOrdenesQuery {
    #[param(value_type = Option<crate::esquemas::EstadoOrden>)]
    pub estado: Option<EstadoOrden>,
    pub activo_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    /// Solo las pendientes con fecha ya pasada.
    #[serde(default)]
    pub vencidas: bool,
}

/// Órdenes de trabajo, más recientes primero (quien solo ejecuta ve las asignadas).
#[utoipa::path(
    get,
    path = "/ordenes-trabajo",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroOrdenesQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Órdenes", body = PaginaRespuesta<OrdenRespuesta>),
        (status = FORBIDDEN, description = "Sin permisos de mantenimiento (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_ordenes(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroOrdenesQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<OrdenRespuesta>>> {
    let pagina = uc::listar_ordenes(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ConsultaOrdenes {
            estado: f.estado,
            activo_id: f.activo_id.map(ActivoId::from_uuid),
            ticket_id: f.ticket_id.map(TicketId::from_uuid),
            solo_vencidas: f.vencidas,
        },
        p.validar()?,
    )
    .await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Orden correctiva, urgente o de inspección.
#[derive(Debug, Deserialize, ToSchema)]
pub struct OrdenPeticion {
    /// `correctivo` o `urgente` (las preventivas las generan los planes; `preventivo` vale para
    /// una inspección).
    #[schema(value_type = crate::esquemas::TipoOrden)]
    pub tipo: TipoOrden,
    /// Ticket que la origina.
    pub ticket_id: Option<Uuid>,
    pub activo_id: Option<Uuid>,
    pub ubicacion: Option<String>,
    pub descripcion: String,
    pub asignado: Option<ResponsableDto>,
    pub programada_para: NaiveDate,
    #[serde(default)]
    pub checklist: Vec<String>,
}

/// Crea una orden; si nace de un ticket, el ticket avanza y el solicitante recibe un mensaje.
#[utoipa::path(
    post,
    path = "/ordenes-trabajo",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = OrdenPeticion,
    responses(
        (status = CREATED, description = "Orden", body = OrdenRespuesta),
        (status = FORBIDDEN, description = "Sin `mantenimiento.planificar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Activo dado de baja o ticket terminado (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_orden(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<OrdenPeticion>,
) -> ApiResult<(StatusCode, Json<OrdenRespuesta>)> {
    let o = uc::crear_orden(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevaOrden {
            tipo: p.tipo,
            ticket_id: p.ticket_id.map(TicketId::from_uuid),
            activo_id: p.activo_id.map(ActivoId::from_uuid),
            ubicacion: p.ubicacion,
            descripcion: p.descripcion,
            asignado: ResponsableDto::opcional(p.asignado)?,
            programada_para: p.programada_para,
            checklist: p.checklist,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(o.into())))
}

/// Evidencia de una orden.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct EvidenciaDto {
    pub archivo_id: Uuid,
    /// `antes`, `despues` o `soporte`.
    #[schema(value_type = crate::esquemas::MomentoEvidencia)]
    pub momento: MomentoEvidencia,
}

/// Orden con sus evidencias.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleOrdenRespuesta {
    pub orden: OrdenRespuesta,
    pub evidencias: Vec<EvidenciaDto>,
}

impl From<DetalleOrden> for DetalleOrdenRespuesta {
    fn from(d: DetalleOrden) -> Self {
        Self {
            orden: d.orden.into(),
            evidencias: d
                .evidencias
                .into_iter()
                .map(|e| EvidenciaDto {
                    archivo_id: e.archivo_id.as_uuid(),
                    momento: e.momento,
                })
                .collect(),
        }
    }
}

fn id_orden(ruta: (Uuid, Uuid)) -> OrdenTrabajoId {
    OrdenTrabajoId::from_uuid(ruta.1)
}

/// Una orden de trabajo con sus evidencias.
#[utoipa::path(
    get,
    path = "/ordenes-trabajo/{orden_id}",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("orden_id" = Uuid, Path, description = "Orden"),
    ),
    responses(
        (status = OK, description = "Orden", body = DetalleOrdenRespuesta),
        (status = NOT_FOUND, description = "No existe o no es visible", body = ErrorRespuesta),
    )
)]
pub async fn ver_orden(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DetalleOrdenRespuesta>> {
    let d = uc::ver_orden(&state.unidad, &RelojSistema, &en.ctx, id_orden(ruta)).await?;
    Ok(Json(d.into()))
}

/// Avance de una orden.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "accion", rename_all = "snake_case")]
pub enum AvancePeticion {
    /// `programada → en_ejecucion`.
    Iniciar,
    /// `en_ejecucion → ejecutada`.
    Ejecutar {
        #[serde(default)]
        checklist: Vec<PuntoChecklistDto>,
        #[serde(default)]
        #[schema(value_type = String, example = "150000")]
        costo_mano_obra: Decimal,
        #[serde(default)]
        #[schema(value_type = String, example = "80000")]
        costo_repuestos: Decimal,
        observaciones: Option<String>,
        /// Nombre de quien recibe el trabajo.
        recibido_por: Option<String>,
        firma_archivo_id: Option<Uuid>,
    },
    /// `ejecutada → verificada` (o devuelta a `en_ejecucion` si `aprobada` es falso).
    Verificar {
        aprobada: bool,
        observacion: Option<String>,
        /// Resolver el ticket que originó la orden.
        #[serde(default)]
        resolver_ticket: bool,
    },
    /// `programada | en_ejecucion → cancelada`.
    Cancelar { motivo: String },
}

/// Avanza una orden: iniciar y ejecutar (quien la tiene asignada o planifica), verificar
/// (`mantenimiento.verificar`) o cancelar (`mantenimiento.planificar`).
#[utoipa::path(
    post,
    path = "/ordenes-trabajo/{orden_id}/avance",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("orden_id" = Uuid, Path, description = "Orden"),
    ),
    request_body = AvancePeticion,
    responses(
        (status = OK, description = "Orden", body = OrdenRespuesta),
        (status = FORBIDDEN, description = "Sin el permiso de esa acción (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Transición no permitida (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Checklist incompleto o costos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn avanzar_orden(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AvancePeticion>,
) -> ApiResult<Json<OrdenRespuesta>> {
    let avance = match p {
        AvancePeticion::Iniciar => AvanceOrden::Iniciar,
        AvancePeticion::Ejecutar {
            checklist,
            costo_mano_obra,
            costo_repuestos,
            observaciones,
            recibido_por,
            firma_archivo_id,
        } => AvanceOrden::Ejecutar(Ejecucion {
            checklist: checklist
                .into_iter()
                .map(|p| PuntoChecklist {
                    descripcion: p.descripcion,
                    cumplido: p.cumplido,
                    observacion: p.observacion,
                })
                .collect(),
            costo_mano_obra,
            costo_repuestos,
            observaciones,
            recibido_por,
            firma_archivo_id: firma_archivo_id.map(ArchivoId::from_uuid),
        }),
        AvancePeticion::Verificar {
            aprobada,
            observacion,
            resolver_ticket,
        } => AvanceOrden::Verificar {
            aprobada,
            observacion,
            resolver_ticket,
        },
        AvancePeticion::Cancelar { motivo } => AvanceOrden::Cancelar { motivo },
    };
    let o = uc::avanzar_orden(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_orden(ruta),
        avance,
    )
    .await?;
    Ok(Json(o.into()))
}

/// Agrega una evidencia (foto antes/después o soporte).
#[utoipa::path(
    post,
    path = "/ordenes-trabajo/{orden_id}/evidencias",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("orden_id" = Uuid, Path, description = "Orden"),
    ),
    request_body = EvidenciaDto,
    responses(
        (status = NO_CONTENT, description = "Evidencia agregada"),
        (status = CONFLICT, description = "La orden terminó (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn agregar_evidencia(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<EvidenciaDto>,
) -> ApiResult<StatusCode> {
    uc::agregar_evidencia(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_orden(ruta),
        ArchivoId::from_uuid(p.archivo_id),
        p.momento,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Obligaciones periódicas
// ---------------------------------------------------------------------------------------------

/// Obligación del catálogo sugerido.
#[derive(Debug, Serialize, ToSchema)]
pub struct SugeridaRespuesta {
    #[schema(example = "lavado_tanques")]
    pub codigo: &'static str,
    pub nombre: &'static str,
    pub descripcion: &'static str,
    pub periodicidad_meses: u32,
}

impl From<&ObligacionSugerida> for SugeridaRespuesta {
    fn from(s: &ObligacionSugerida) -> Self {
        Self {
            codigo: s.codigo,
            nombre: s.nombre,
            descripcion: s.descripcion,
            periodicidad_meses: s.periodicidad_meses,
        }
    }
}

/// Catálogo sugerido de obligaciones periódicas (editable al activarlas: la periodicidad
/// exacta depende de la norma local).
#[utoipa::path(
    get,
    path = "/obligaciones/sugeridas",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses((status = OK, description = "Catálogo", body = Vec<SugeridaRespuesta>))
)]
pub async fn obligaciones_sugeridas(_en: EnCopropiedad) -> Json<Vec<SugeridaRespuesta>> {
    Json(
        uc::obligaciones_sugeridas()
            .iter()
            .map(Into::into)
            .collect(),
    )
}

/// Obligación periódica.
#[derive(Debug, Serialize, ToSchema)]
pub struct ObligacionRespuesta {
    pub id: Uuid,
    pub codigo_sugerido: Option<String>,
    pub nombre: String,
    pub descripcion: Option<String>,
    pub periodicidad_meses: u32,
    pub proxima: NaiveDate,
    pub dias_alerta: u32,
    pub activo_id: Option<Uuid>,
    pub responsable_organizacion_id: Option<Uuid>,
    pub activa: bool,
}

impl From<Obligacion> for ObligacionRespuesta {
    fn from(o: Obligacion) -> Self {
        Self {
            id: o.id.as_uuid(),
            codigo_sugerido: o.codigo_sugerido,
            nombre: o.nombre,
            descripcion: o.descripcion,
            periodicidad_meses: o.periodicidad_meses,
            proxima: o.proxima,
            dias_alerta: o.dias_alerta,
            activo_id: o.activo_id.map(ActivoId::as_uuid),
            responsable_organizacion_id: o.responsable_organizacion_id.map(OrganizacionId::as_uuid),
            activa: o.activa,
        }
    }
}

/// Datos de una obligación. Con `codigo_sugerido` los campos omitidos salen del catálogo.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ObligacionPeticion {
    #[schema(example = "lavado_tanques")]
    pub codigo_sugerido: Option<String>,
    pub nombre: Option<String>,
    pub descripcion: Option<String>,
    pub periodicidad_meses: Option<u32>,
    pub proxima: Option<NaiveDate>,
    pub dias_alerta: Option<u32>,
    pub activo_id: Option<Uuid>,
    pub responsable_organizacion_id: Option<Uuid>,
    pub activa: Option<bool>,
}

impl From<ObligacionPeticion> for DatosObligacion {
    fn from(p: ObligacionPeticion) -> Self {
        Self {
            codigo_sugerido: p.codigo_sugerido,
            nombre: p.nombre,
            descripcion: p.descripcion,
            periodicidad_meses: p.periodicidad_meses,
            proxima: p.proxima,
            dias_alerta: p.dias_alerta,
            activo_id: p.activo_id.map(ActivoId::from_uuid),
            responsable_organizacion_id: p
                .responsable_organizacion_id
                .map(OrganizacionId::from_uuid),
            activa: p.activa,
        }
    }
}

/// Filtro de obligaciones.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroObligaciones {
    /// Solo las activas.
    #[serde(default)]
    pub activas: bool,
}

/// Obligaciones periódicas, por próxima fecha.
#[utoipa::path(
    get,
    path = "/obligaciones",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroObligaciones),
    responses(
        (status = OK, description = "Obligaciones", body = Vec<ObligacionRespuesta>),
        (status = FORBIDDEN, description = "Sin `mantenimiento.planificar` ni `inventario.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_obligaciones(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroObligaciones>,
) -> ApiResult<Json<Vec<ObligacionRespuesta>>> {
    let v = uc::listar_obligaciones(&state.unidad, &RelojSistema, &en.ctx, f.activas).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea una obligación (propia o del catálogo sugerido).
#[utoipa::path(
    post,
    path = "/obligaciones",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = ObligacionPeticion,
    responses(
        (status = CREATED, description = "Obligación", body = ObligacionRespuesta),
        (status = CONFLICT, description = "La sugerida ya está activa (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Faltan datos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_obligacion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<ObligacionPeticion>,
) -> ApiResult<(StatusCode, Json<ObligacionRespuesta>)> {
    let o = uc::guardar_obligacion(&state.unidad, &RelojSistema, &en.ctx, None, p.into()).await?;
    Ok((StatusCode::CREATED, Json(o.into())))
}

fn id_obligacion(ruta: (Uuid, Uuid)) -> ObligacionId {
    ObligacionId::from_uuid(ruta.1)
}

/// Edita una obligación (los campos omitidos no cambian).
#[utoipa::path(
    patch,
    path = "/obligaciones/{obligacion_id}",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("obligacion_id" = Uuid, Path, description = "Obligación"),
    ),
    request_body = ObligacionPeticion,
    responses(
        (status = OK, description = "Obligación", body = ObligacionRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn editar_obligacion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ObligacionPeticion>,
) -> ApiResult<Json<ObligacionRespuesta>> {
    let o = uc::guardar_obligacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        Some(id_obligacion(ruta)),
        p.into(),
    )
    .await?;
    Ok(Json(o.into()))
}

/// Cumplimiento registrado.
#[derive(Debug, Serialize, ToSchema)]
pub struct CumplimientoRespuesta {
    pub realizada_el: NaiveDate,
    pub soporte_archivo_id: Option<Uuid>,
    pub observacion: Option<String>,
    pub registrado_por: Option<Uuid>,
    pub registrado_en: DateTime<Utc>,
}

impl From<Cumplimiento> for CumplimientoRespuesta {
    fn from(c: Cumplimiento) -> Self {
        Self {
            realizada_el: c.realizada_el,
            soporte_archivo_id: c.soporte_archivo_id.map(ArchivoId::as_uuid),
            observacion: c.observacion,
            registrado_por: c.registrado_por.map(CuentaId::as_uuid),
            registrado_en: c.registrado_en,
        }
    }
}

/// Historial de cumplimientos, más recientes primero.
#[utoipa::path(
    get,
    path = "/obligaciones/{obligacion_id}/cumplimientos",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("obligacion_id" = Uuid, Path, description = "Obligación"),
    ),
    responses(
        (status = OK, description = "Cumplimientos", body = Vec<CumplimientoRespuesta>),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn listar_cumplimientos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<CumplimientoRespuesta>>> {
    let v =
        uc::cumplimientos_obligacion(&state.unidad, &RelojSistema, &en.ctx, id_obligacion(ruta))
            .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Cumplimiento nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CumplimientoPeticion {
    pub realizada_el: NaiveDate,
    /// Certificado o informe.
    pub soporte_archivo_id: Option<Uuid>,
    pub observacion: Option<String>,
}

/// Registra que se cumplió; la próxima fecha se calcula desde la realización.
#[utoipa::path(
    post,
    path = "/obligaciones/{obligacion_id}/cumplimientos",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("obligacion_id" = Uuid, Path, description = "Obligación"),
    ),
    request_body = CumplimientoPeticion,
    responses(
        (status = CREATED, description = "Obligación con la próxima fecha", body = ObligacionRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Fecha futura (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_cumplimiento(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<CumplimientoPeticion>,
) -> ApiResult<(StatusCode, Json<ObligacionRespuesta>)> {
    let o = uc::registrar_cumplimiento(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_obligacion(ruta),
        p.realizada_el,
        p.soporte_archivo_id.map(ArchivoId::from_uuid),
        p.observacion,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(o.into())))
}

// ---------------------------------------------------------------------------------------------
// Alertas y tablero
// ---------------------------------------------------------------------------------------------

/// Alertas de mantenimiento a hoy (hora local de la copropiedad).
#[derive(Debug, Serialize, ToSchema)]
pub struct AlertasMantenimientoRespuesta {
    /// Obligaciones vencidas o dentro de su ventana de alerta.
    pub obligaciones: Vec<ObligacionRespuesta>,
    /// Garantías vencidas hace poco o por vencer (30 días).
    pub garantias: Vec<ActivoRespuesta>,
    pub ordenes_vencidas: Vec<OrdenRespuesta>,
}

impl From<AlertasMantenimiento> for AlertasMantenimientoRespuesta {
    fn from(a: AlertasMantenimiento) -> Self {
        Self {
            obligaciones: a.obligaciones.into_iter().map(Into::into).collect(),
            garantias: a.garantias.into_iter().map(Into::into).collect(),
            ordenes_vencidas: a.ordenes_vencidas.into_iter().map(Into::into).collect(),
        }
    }
}

/// Alertas: obligaciones por vencer, garantías y órdenes vencidas.
#[utoipa::path(
    get,
    path = "/mantenimiento/alertas",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Alertas", body = AlertasMantenimientoRespuesta),
        (status = FORBIDDEN, description = "Sin `mantenimiento.planificar` ni `inventario.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn alertas(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<AlertasMantenimientoRespuesta>> {
    let a = uc::alertas_mantenimiento(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(a.into()))
}

/// Costo acumulado de un activo.
#[derive(Debug, Serialize, ToSchema)]
pub struct CostoActivo {
    pub activo_id: Uuid,
    pub nombre: String,
    #[schema(value_type = String, example = "350000.00")]
    pub costo: Decimal,
}

/// Indicadores del tablero de mantenimiento.
#[derive(Debug, Serialize, ToSchema)]
pub struct TableroMantenimientoRespuesta {
    /// Preventivas programadas en los últimos 12 meses.
    pub preventivas_programadas: u64,
    pub preventivas_cumplidas: u64,
    /// Porcentaje de cumplimiento del plan preventivo (`null` si no hubo programadas).
    #[schema(value_type = Option<String>, example = "87.50")]
    pub cumplimiento_porcentaje: Option<Decimal>,
    pub ordenes_vencidas: u64,
    pub costo_por_activo: Vec<CostoActivo>,
}

impl From<TableroMantenimiento> for TableroMantenimientoRespuesta {
    fn from(t: TableroMantenimiento) -> Self {
        Self {
            cumplimiento_porcentaje: t.cumplimiento_porcentaje(),
            preventivas_programadas: t.preventivas_programadas,
            preventivas_cumplidas: t.preventivas_cumplidas,
            ordenes_vencidas: t.ordenes_vencidas,
            costo_por_activo: t
                .costo_por_activo
                .into_iter()
                .map(|(id, nombre, costo)| CostoActivo {
                    activo_id: id.as_uuid(),
                    nombre,
                    costo,
                })
                .collect(),
        }
    }
}

/// Tablero: cumplimiento del plan preventivo, órdenes vencidas y costo por activo.
#[utoipa::path(
    get,
    path = "/mantenimiento/tablero",
    tag = ETIQUETA_MANTENIMIENTO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Indicadores", body = TableroMantenimientoRespuesta),
        (status = FORBIDDEN, description = "Sin `mantenimiento.planificar` ni `inventario.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn tablero(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<TableroMantenimientoRespuesta>> {
    let t = uc::tablero_mantenimiento(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(t.into()))
}
