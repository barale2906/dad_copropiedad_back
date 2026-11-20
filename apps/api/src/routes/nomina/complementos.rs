//! Complementos de la nómina (plan 16): contabilizar lo aprobado antes de tener contabilidad,
//! simular la terminación de un contrato, saldo de vacaciones, importación de novedades y
//! deducciones de la retención declaradas por el empleado.

use application::{
    casos_uso::nomina::{
        contabilizacion::{self as uc_contab, InformeContabilizacionNomina},
        deducciones::{self as uc_deducciones, NuevaDeduccion},
        importacion_novedades::{self as uc_importacion, FilaNovedadPrevia, VistaPreviaNovedades},
        prestaciones::{self as uc_prestaciones, SaldoVacaciones},
    },
    puertos::{RelojSistema, nomina::DeduccionRetencion},
};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, ContratoLaboralId, DeduccionRetencionId, EmpleadoId, ImportacionId, NombreArchivo,
    TipoArchivo,
    contabilidad::Monto,
    importacion::novedades::encabezados_novedades,
    nomina::{
        MotivoTerminacion,
        liquidacion::{TipoDeduccion, TipoNovedad},
    },
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::liquidacion::LineaLiquidacionNominaRespuesta;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_NOMINA,
    routes::importaciones::{DatosPlanilla, ImportacionRespuesta},
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas de los complementos (se fusionan en `/entidades/{entidad_id}/nomina`).
pub fn rutas() -> OpenApiRouter<AppState> {
    let limite = usize::try_from(TipoArchivo::Xlsx.tamano_maximo()).unwrap_or(usize::MAX);
    OpenApiRouter::new()
        .routes(routes!(importar_novedades))
        .layer(DefaultBodyLimit::max(limite))
        .routes(routes!(plantilla_novedades))
        .routes(routes!(confirmar_importacion_novedades))
        .routes(routes!(contabilizar_pendientes))
        .routes(routes!(simular_terminacion))
        .routes(routes!(saldo_vacaciones))
        .routes(routes!(listar_deducciones, registrar_deduccion))
        .routes(routes!(terminar_deduccion))
}

// ---------------------------------------------------------------------------------------------
// Contabilizar lo aprobado antes de tener contabilidad
// ---------------------------------------------------------------------------------------------

/// Rango de periodos por contabilizar.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RangoContabilizacionNominaPeticion {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
}

/// Periodo que no se pudo contabilizar.
#[derive(Debug, Serialize, ToSchema)]
pub struct PeriodoFallidoRespuesta {
    pub periodo_id: Uuid,
    pub motivo: String,
}

/// Resultado de contabilizar la nómina pendiente.
#[derive(Debug, Serialize, ToSchema)]
pub struct InformeContabilizacionNominaRespuesta {
    pub contabilizados: Vec<Uuid>,
    pub fallidos: Vec<PeriodoFallidoRespuesta>,
}

impl From<InformeContabilizacionNomina> for InformeContabilizacionNominaRespuesta {
    fn from(i: InformeContabilizacionNomina) -> Self {
        Self {
            contabilizados: i.contabilizados.into_iter().map(|p| p.as_uuid()).collect(),
            fallidos: i
                .fallidos
                .into_iter()
                .map(|(p, motivo)| PeriodoFallidoRespuesta {
                    periodo_id: p.as_uuid(),
                    motivo,
                })
                .collect(),
        }
    }
}

/// Contabiliza los periodos aprobados o pagados entre las fechas (la nómina aprobada antes de
/// habilitar la contabilidad). Es idempotente; cada periodo va aparte.
#[utoipa::path(
    post,
    path = "/contabilizacion",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = RangoContabilizacionNominaPeticion,
    responses(
        (status = OK, description = "Informe", body = InformeContabilizacionNominaRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.comprobantes.contabilizar` (`sin_permiso`) o sin los módulos `nomina` y `contabilidad` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Rango invertido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn contabilizar_pendientes(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<RangoContabilizacionNominaPeticion>,
) -> ApiResult<Json<InformeContabilizacionNominaRespuesta>> {
    let i =
        uc_contab::contabilizar_pendientes(&state.unidad, &RelojSistema, &en.ctx, p.desde, p.hasta)
            .await?;
    Ok(Json(i.into()))
}

// ---------------------------------------------------------------------------------------------
// Simulación de la terminación y saldo de vacaciones
// ---------------------------------------------------------------------------------------------

/// Terminación que se quiere simular.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SimulacionTerminacionPeticion {
    pub fecha: NaiveDate,
    #[schema(value_type = crate::esquemas::MotivoTerminacion)]
    pub motivo: MotivoTerminacion,
}

/// Lo que costaría terminar el contrato.
#[derive(Debug, Serialize, ToSchema)]
pub struct SimulacionTerminacionRespuesta {
    /// Prestaciones, vacaciones pendientes e indemnización.
    pub lineas: Vec<LineaLiquidacionNominaRespuesta>,
    /// Suma de las líneas (sin el salario del periodo ni las deducciones).
    #[schema(value_type = String)]
    pub total: Decimal,
}

/// Simula la terminación del contrato en una fecha y con un motivo: prestaciones, vacaciones
/// pendientes e indemnización, sin guardar nada.
#[utoipa::path(
    post,
    path = "/contratos/{contrato_id}/simulacion-terminacion",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("contrato_id" = Uuid, Path, description = "Contrato laboral")),
    request_body = SimulacionTerminacionPeticion,
    responses(
        (status = OK, description = "Simulación", body = SimulacionTerminacionRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.salarios.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Contrato inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "El contrato ya terminó (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Fecha o motivo que no aplican, o falta un parámetro del catálogo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn simular_terminacion(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<SimulacionTerminacionPeticion>,
) -> ApiResult<Json<SimulacionTerminacionRespuesta>> {
    let lineas = uc_prestaciones::simular_terminacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ContratoLaboralId::from_uuid(ruta.1),
        p.fecha,
        p.motivo,
    )
    .await?;
    let total = lineas.iter().map(|l| l.valor.valor()).sum();
    Ok(Json(SimulacionTerminacionRespuesta {
        lineas: lineas
            .into_iter()
            .map(|x| LineaLiquidacionNominaRespuesta {
                concepto: x.concepto,
                clase: x.concepto.clase(),
                cantidad: x.cantidad.normalize(),
                valor: x.valor.valor(),
            })
            .collect(),
        total,
    }))
}

/// Fecha del saldo.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FechaSaldoParametros {
    /// Hoy si se omite (o la terminación, si el contrato terminó antes).
    pub fecha: Option<NaiveDate>,
}

/// Saldo de vacaciones.
#[derive(Debug, Serialize, ToSchema)]
pub struct SaldoVacacionesRespuesta {
    pub dias_trabajados: u32,
    #[schema(value_type = String)]
    pub causados: Decimal,
    /// Días hábiles disfrutados.
    #[schema(value_type = String)]
    pub disfrutados: Decimal,
    #[schema(value_type = String)]
    pub pendientes: Decimal,
}

impl From<SaldoVacaciones> for SaldoVacacionesRespuesta {
    fn from(s: SaldoVacaciones) -> Self {
        Self {
            dias_trabajados: s.dias_trabajados,
            causados: s.causados.normalize(),
            disfrutados: s.disfrutados.normalize(),
            pendientes: s.pendientes.normalize(),
        }
    }
}

/// Saldo de vacaciones del contrato: días causados, disfrutados y pendientes.
#[utoipa::path(
    get,
    path = "/contratos/{contrato_id}/vacaciones",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("contrato_id" = Uuid, Path, description = "Contrato laboral"), FechaSaldoParametros),
    responses(
        (status = OK, description = "Saldo", body = SaldoVacacionesRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Contrato inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Fecha antes del inicio o falta un parámetro del catálogo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn saldo_vacaciones(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(q): Query<FechaSaldoParametros>,
) -> ApiResult<Json<SaldoVacacionesRespuesta>> {
    let s = uc_prestaciones::saldo_vacaciones(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ContratoLaboralId::from_uuid(ruta.1),
        q.fecha,
    )
    .await?;
    Ok(Json(s.into()))
}

// ---------------------------------------------------------------------------------------------
// Importación de novedades
// ---------------------------------------------------------------------------------------------

/// Fila válida de la vista previa.
#[derive(Debug, Serialize, ToSchema)]
pub struct FilaNovedadPreviaRespuesta {
    pub fila: u32,
    pub codigo: String,
    #[schema(value_type = crate::esquemas::TipoNovedad)]
    pub tipo: TipoNovedad,
}

impl From<FilaNovedadPrevia> for FilaNovedadPreviaRespuesta {
    fn from(f: FilaNovedadPrevia) -> Self {
        Self {
            fila: f.fila,
            codigo: f.codigo,
            tipo: f.tipo,
        }
    }
}

/// Importación de novedades con su vista previa.
#[derive(Debug, Serialize, ToSchema)]
pub struct VistaPreviaNovedadesRespuesta {
    pub importacion: ImportacionRespuesta,
    pub filas: Vec<FilaNovedadPreviaRespuesta>,
}

impl From<VistaPreviaNovedades> for VistaPreviaNovedadesRespuesta {
    fn from(v: VistaPreviaNovedades) -> Self {
        Self {
            importacion: v.importacion.into(),
            filas: v.filas.into_iter().map(Into::into).collect(),
        }
    }
}

/// Sube y valida una planilla de novedades (columnas `codigo` del empleado, `tipo`, `desde`,
/// `hasta`, `cantidad` y `descripcion`). No registra nada: devuelve la vista previa.
#[utoipa::path(
    post,
    path = "/novedades/importaciones",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), DatosPlanilla),
    request_body(content = Vec<u8>, content_type = "application/octet-stream",
                 description = "Contenido del CSV o XLSX"),
    responses(
        (status = CREATED, description = "Importación registrada (revise `estado` y `errores`)", body = VistaPreviaNovedadesRespuesta),
        (status = FORBIDDEN, description = "Sin `importaciones.ejecutar` o `nomina.novedades.registrar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es CSV/XLSX o no se puede leer (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn importar_novedades(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(d): Query<DatosPlanilla>,
    cuerpo: Bytes,
) -> ApiResult<(StatusCode, Json<VistaPreviaNovedadesRespuesta>)> {
    let v = uc_importacion::importar_novedades(
        &state.unidad,
        state.archivos.as_ref(),
        &state.planillas,
        &RelojSistema,
        &en.ctx,
        NombreArchivo::new(&d.nombre)?,
        cuerpo.to_vec(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(v.into())))
}

/// Plantilla CSV de novedades.
#[utoipa::path(
    get,
    path = "/novedades/importaciones/plantilla",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Plantilla CSV", content_type = "text/csv", body = String),
        (status = FORBIDDEN, description = "No exige permisos: basta un rol en la entidad", body = ErrorRespuesta),
    )
)]
pub async fn plantilla_novedades(_en: EnEntidad) -> Response {
    let csv = format!(
        "{}\nE-001;hora_extra_diurna;2026-03-04;;4;Inventario\nE-001;vacaciones;2026-03-09;2026-03-13;;\n",
        encabezados_novedades().join(";")
    );
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/csv; charset=utf-8"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"novedades.csv\""),
            ),
        ],
        csv,
    )
        .into_response()
}

/// Registra las novedades de una importación validada en una sola transacción.
#[utoipa::path(
    post,
    path = "/novedades/importaciones/{importacion_id}/confirmar",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(
        ("entidad_id" = Uuid, Path, description = "Entidad"),
        ("importacion_id" = Uuid, Path, description = "Importación de novedades"),
    ),
    responses(
        (status = OK, description = "Aplicada", body = ImportacionRespuesta),
        (status = CONFLICT, description = "Ya aplicada, con errores, los datos cambiaron o una fila cae en un periodo aprobado (`conflicto`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe o no es de novedades", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `importaciones.ejecutar` o `nomina.novedades.registrar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn confirmar_importacion_novedades(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ImportacionRespuesta>> {
    let i = uc_importacion::confirmar_importacion_novedades(
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
// Deducciones de la retención
// ---------------------------------------------------------------------------------------------

/// Deducción nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaDeduccionPeticion {
    #[schema(value_type = crate::esquemas::TipoDeduccion)]
    pub tipo: TipoDeduccion,
    /// Valor mensual; no va en `dependientes`.
    #[schema(value_type = Option<String>, example = "450000")]
    pub valor_mensual: Option<Monto>,
    pub vigente_desde: NaiveDate,
    pub soporte_archivo_id: Option<Uuid>,
}

/// Deducción declarada.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeduccionRetencionRespuesta {
    pub id: Uuid,
    pub empleado_id: Uuid,
    #[schema(value_type = crate::esquemas::TipoDeduccion)]
    pub tipo: TipoDeduccion,
    #[schema(value_type = String)]
    pub valor_mensual: Decimal,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: Option<NaiveDate>,
    pub soporte_archivo_id: Option<Uuid>,
    pub creado_en: DateTime<Utc>,
}

impl From<DeduccionRetencion> for DeduccionRetencionRespuesta {
    fn from(d: DeduccionRetencion) -> Self {
        Self {
            id: d.id.as_uuid(),
            empleado_id: d.empleado_id.as_uuid(),
            tipo: d.tipo,
            valor_mensual: d.valor_mensual.valor(),
            vigente_desde: d.vigente_desde,
            vigente_hasta: d.vigente_hasta,
            soporte_archivo_id: d.soporte_archivo_id.map(ArchivoId::as_uuid),
            creado_en: d.creado_en,
        }
    }
}

/// Deducciones de la retención del empleado.
#[utoipa::path(
    get,
    path = "/empleados/{empleado_id}/deducciones",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("empleado_id" = Uuid, Path, description = "Empleado")),
    responses(
        (status = OK, description = "Deducciones", body = Vec<DeduccionRetencionRespuesta>),
        (status = FORBIDDEN, description = "Sin `nomina.salarios.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_deducciones(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<DeduccionRetencionRespuesta>>> {
    let v = uc_deducciones::deducciones(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        EmpleadoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Registra una deducción de la retención que declaró el empleado (con su soporte).
#[utoipa::path(
    post,
    path = "/empleados/{empleado_id}/deducciones",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("empleado_id" = Uuid, Path, description = "Empleado")),
    request_body = NuevaDeduccionPeticion,
    responses(
        (status = CREATED, description = "Deducción registrada", body = DeduccionRetencionRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.empleados.gestionar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Empleado o soporte inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Dependientes con valor o los demás sin él (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_deduccion(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<NuevaDeduccionPeticion>,
) -> ApiResult<(StatusCode, Json<DeduccionRetencionRespuesta>)> {
    let d = uc_deducciones::registrar_deduccion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        EmpleadoId::from_uuid(ruta.1),
        NuevaDeduccion {
            tipo: p.tipo,
            valor_mensual: p.valor_mensual,
            vigente_desde: p.vigente_desde,
            soporte_archivo_id: p.soporte_archivo_id.map(ArchivoId::from_uuid),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(d.into())))
}

/// Último día de la deducción.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TerminacionDeduccionPeticion {
    pub hasta: NaiveDate,
}

/// Termina una deducción (desde el día siguiente ya no se aplica).
#[utoipa::path(
    post,
    path = "/deducciones/{deduccion_id}/terminacion",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("deduccion_id" = Uuid, Path, description = "Deducción")),
    request_body = TerminacionDeduccionPeticion,
    responses(
        (status = OK, description = "Deducción terminada", body = DeduccionRetencionRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.empleados.gestionar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Deducción inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Antes de su inicio o ya terminada (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn terminar_deduccion(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<TerminacionDeduccionPeticion>,
) -> ApiResult<Json<DeduccionRetencionRespuesta>> {
    let d = uc_deducciones::terminar_deduccion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        DeduccionRetencionId::from_uuid(ruta.1),
        p.hasta,
    )
    .await?;
    Ok(Json(d.into()))
}
