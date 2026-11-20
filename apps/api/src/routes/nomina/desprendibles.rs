//! Desprendibles de nómina (plan 16 fase 3): el PDF y el envío por correo en
//! `/entidades/{entidad_id}/nomina/…`, y el portal del empleado en `/yo/nomina/…` (no exige roles
//! en la entidad: basta ser la persona de la liquidación).

use application::{
    casos_uso::nomina::{
        certificados as uc_certificados,
        desprendibles::{self as uc, EnvioDesprendibles},
    },
    puertos::{RelojSistema, nomina::DesprendibleDeCuenta},
};
use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use domain::{
    LiquidacionNominaId, PeriodoNominaId, TenantId,
    nomina::{PeriodicidadPago, periodo::EstadoPeriodoNomina},
};
use rust_decimal::Decimal;
use serde::Serialize;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::{ETIQUETA_CUENTA, ETIQUETA_NOMINA},
    sesion::{CuentaAutenticada, EnEntidad},
    state::AppState,
};

/// Rutas de la entidad (se fusionan en `/entidades/{entidad_id}/nomina`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(desprendible))
        .routes(routes!(enviar_desprendibles))
        .routes(routes!(certificado_laboral))
}

/// Rutas del portal del empleado (se montan en `/api/v1/yo`).
pub fn rutas_yo() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(mis_desprendibles))
        .routes(routes!(mi_desprendible))
        .routes(routes!(mi_certificado_laboral))
}

fn pdf(contenido: Vec<u8>, nombre: &str) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/pdf".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{nombre}\""),
            ),
        ],
        contenido,
    )
        .into_response()
}

/// PDF del desprendible de una liquidación (periodo aprobado o pagado). Las incapacidades y
/// licencias salen como «Ausencia remunerada» y no lleva la cuenta bancaria.
#[utoipa::path(
    get,
    path = "/liquidaciones/{liquidacion_id}/desprendible",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("liquidacion_id" = Uuid, Path, description = "Liquidación")),
    responses(
        (status = OK, description = "PDF del desprendible", content_type = "application/pdf", body = Vec<u8>),
        (status = FORBIDDEN, description = "Sin `nomina.salarios.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Liquidación inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "El periodo no está aprobado (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn desprendible(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    let contenido = uc::desprendible(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        &en.ctx,
        LiquidacionNominaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(pdf(contenido, "desprendible.pdf"))
}

/// Resultado del envío de los desprendibles.
#[derive(Debug, Serialize, ToSchema)]
pub struct EnvioDesprendiblesRespuesta {
    pub enviados: u32,
    /// Empleados sin correo: entrégueles el PDF a mano o por el portal.
    pub sin_correo: u32,
}

impl From<EnvioDesprendibles> for EnvioDesprendiblesRespuesta {
    fn from(e: EnvioDesprendibles) -> Self {
        Self {
            enviados: e.enviados,
            sin_correo: e.sin_correo,
        }
    }
}

/// Envía por correo el desprendible de cada empleado del periodo aprobado.
#[utoipa::path(
    post,
    path = "/periodos/{periodo_id}/desprendibles",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("periodo_id" = Uuid, Path, description = "Periodo de nómina")),
    responses(
        (status = OK, description = "Desprendibles encolados", body = EnvioDesprendiblesRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.liquidar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Periodo inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "El periodo no está aprobado (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn enviar_desprendibles(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<EnvioDesprendiblesRespuesta>> {
    let e = uc::enviar_desprendibles(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        &en.ctx,
        PeriodoNominaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(e.into()))
}

/// Desprendible de la cuenta en una de sus entidades.
#[derive(Debug, Serialize, ToSchema)]
pub struct DesprendibleDeCuentaRespuesta {
    pub entidad_id: Uuid,
    pub entidad: String,
    pub liquidacion_id: Uuid,
    #[schema(value_type = crate::esquemas::PeriodicidadPago)]
    pub periodicidad: PeriodicidadPago,
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    #[schema(value_type = crate::esquemas::EstadoPeriodoNomina)]
    pub estado: EstadoPeriodoNomina,
    pub fecha_pago: Option<NaiveDate>,
    #[schema(value_type = String)]
    pub devengos: Decimal,
    #[schema(value_type = String)]
    pub deducciones: Decimal,
    #[schema(value_type = String)]
    pub neto: Decimal,
}

impl From<DesprendibleDeCuenta> for DesprendibleDeCuentaRespuesta {
    fn from(d: DesprendibleDeCuenta) -> Self {
        Self {
            entidad_id: d.tenant_id.as_uuid(),
            entidad: d.entidad,
            liquidacion_id: d.liquidacion_id.as_uuid(),
            periodicidad: d.periodicidad,
            desde: d.desde,
            hasta: d.hasta,
            estado: d.estado,
            fecha_pago: d.fecha_pago,
            devengos: d.devengos.valor(),
            deducciones: d.deducciones.valor(),
            neto: d.neto.valor(),
        }
    }
}

/// Mis desprendibles en todas las entidades donde trabajo o trabajé (periodos aprobados).
#[utoipa::path(
    get,
    path = "/nomina/desprendibles",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Desprendibles de la cuenta", body = Vec<DesprendibleDeCuentaRespuesta>),
        (status = UNAUTHORIZED, description = "Sin sesión (`no_autenticado`)", body = ErrorRespuesta),
    )
)]
pub async fn mis_desprendibles(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
) -> ApiResult<Json<Vec<DesprendibleDeCuentaRespuesta>>> {
    let v = uc::mis_desprendibles(&state.plataforma, &cuenta.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// PDF de uno de mis desprendibles.
#[utoipa::path(
    get,
    path = "/nomina/{entidad_id}/desprendibles/{liquidacion_id}",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad empleadora"), ("liquidacion_id" = Uuid, Path, description = "Liquidación")),
    responses(
        (status = OK, description = "PDF del desprendible", content_type = "application/pdf", body = Vec<u8>),
        (status = UNAUTHORIZED, description = "Sin sesión (`no_autenticado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe o no es de la persona vinculada a la cuenta", body = ErrorRespuesta),
        (status = CONFLICT, description = "El periodo no está aprobado (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn mi_desprendible(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    let contenido = uc::mi_desprendible(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        cuenta.sesion.cuenta_id,
        TenantId::from_uuid(ruta.0),
        LiquidacionNominaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(pdf(contenido, "desprendible.pdf"))
}

/// Opciones del certificado laboral.
#[derive(Debug, serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CertificadoLaboralParametros {
    /// Incluir el salario actual (exige `nomina.salarios.ver`).
    #[serde(default)]
    pub con_salario: bool,
}

/// Certificado laboral del empleado en PDF (cargo, tipo de contrato y fechas; el salario solo si
/// se pide).
#[utoipa::path(
    get,
    path = "/empleados/{empleado_id}/certificado-laboral",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("empleado_id" = Uuid, Path, description = "Empleado"), CertificadoLaboralParametros),
    responses(
        (status = OK, description = "PDF del certificado", content_type = "application/pdf", body = Vec<u8>),
        (status = FORBIDDEN, description = "Sin `nomina.ver` (o sin `nomina.salarios.ver` con salario) (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Empleado inexistente o sin contratos", body = ErrorRespuesta),
    )
)]
pub async fn certificado_laboral(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(q): Query<CertificadoLaboralParametros>,
) -> ApiResult<Response> {
    let contenido = uc_certificados::certificado_laboral(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        &en.ctx,
        domain::EmpleadoId::from_uuid(ruta.1),
        q.con_salario,
    )
    .await?;
    Ok(pdf(contenido, "certificado-laboral.pdf"))
}

/// Mi certificado laboral en una entidad donde trabajo o trabajé (con mi salario actual).
#[utoipa::path(
    get,
    path = "/nomina/{entidad_id}/certificado-laboral",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad empleadora")),
    responses(
        (status = OK, description = "PDF del certificado", content_type = "application/pdf", body = Vec<u8>),
        (status = UNAUTHORIZED, description = "Sin sesión (`no_autenticado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La cuenta no es empleada de la entidad", body = ErrorRespuesta),
    )
)]
pub async fn mi_certificado_laboral(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
    Path(entidad): Path<Uuid>,
) -> ApiResult<Response> {
    let contenido = uc_certificados::mi_certificado_laboral(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        cuenta.sesion.cuenta_id,
        TenantId::from_uuid(entidad),
    )
    .await?;
    Ok(pdf(contenido, "certificado-laboral.pdf"))
}
