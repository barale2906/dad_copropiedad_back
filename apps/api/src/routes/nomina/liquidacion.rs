//! Liquidación de la nómina (plan 16 fase 2): configuración, novedades, periodos, liquidación,
//! aprobación y pago en `/entidades/{entidad_id}/nomina/…`, y las tablas por tramos del
//! catálogo en `/plataforma/nomina/tablas`.

use application::{
    casos_uso::nomina::{
        catalogo::{self as uc_catalogo, NuevaTablaLaboral},
        liquidacion::{self as uc, PeriodoLiquidado},
        novedades::{self as uc_novedades, NuevaNovedad},
        prestaciones::{self as uc_prestaciones, CesantiasContrato},
    },
    puertos::{
        RelojSistema,
        nomina::{
            ConfiguracionNomina, LiquidacionContrato, NovedadNomina, PeriodoNomina,
            VersionTablaLaboral,
        },
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, ContratoLaboralId, NovedadNominaId, PeriodoNominaId,
    nomina::{
        PeriodicidadPago,
        liquidacion::{ClaseLinea, ConceptoNomina, Medida, TablaLaboral, TipoNovedad, Tramo},
        periodo::EstadoPeriodoNomina,
    },
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::{ETIQUETA_NOMINA, ETIQUETA_PLATAFORMA},
    sesion::{EnEntidad, OperadorPlataforma},
    state::AppState,
};

/// Rutas de la liquidación (se fusionan en `/entidades/{entidad_id}/nomina`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(ver_configuracion, guardar_configuracion))
        .routes(routes!(registrar_novedad))
        .routes(routes!(listar_novedades))
        .routes(routes!(anular_novedad))
        .routes(routes!(listar_periodos, crear_periodo))
        .routes(routes!(ver_periodo))
        .routes(routes!(liquidar_periodo))
        .routes(routes!(aprobar_periodo))
        .routes(routes!(pagar_periodo))
        .routes(routes!(cesantias_del_ano))
}

// ---------------------------------------------------------------------------------------------
// Tablas por tramos (plataforma)
// ---------------------------------------------------------------------------------------------

/// Tramo `(desde, hasta]` de una tabla (el primero incluye su `desde`).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TramoDto {
    /// En UVT (retención) o en salarios mínimos (fondo de solidaridad).
    #[schema(value_type = String, example = "95")]
    pub desde: Decimal,
    /// `null` en el último tramo.
    #[schema(value_type = Option<String>, example = "150")]
    pub hasta: Option<Decimal>,
    /// Porcentaje.
    #[schema(value_type = String, example = "19")]
    pub tarifa: Decimal,
    /// UVT que se suman (retención); cero en el fondo de solidaridad.
    #[serde(default)]
    #[schema(value_type = String, example = "0")]
    pub adicional: Decimal,
}

/// Versión de una tabla por tramos.
#[derive(Debug, Serialize, ToSchema)]
pub struct TablaLaboralRespuesta {
    #[schema(value_type = crate::esquemas::TablaLaboral)]
    pub tabla: TablaLaboral,
    pub vigente_desde: NaiveDate,
    pub fuente: String,
    pub tramos: Vec<TramoDto>,
}

impl From<VersionTablaLaboral> for TablaLaboralRespuesta {
    fn from(v: VersionTablaLaboral) -> Self {
        Self {
            tabla: v.tabla,
            vigente_desde: v.vigente_desde,
            fuente: v.fuente,
            tramos: v
                .tramos
                .tramos()
                .iter()
                .map(|t| TramoDto {
                    desde: t.desde.normalize(),
                    hasta: t.hasta.map(|h| h.normalize()),
                    tarifa: t.tarifa.normalize(),
                    adicional: t.adicional.normalize(),
                })
                .collect(),
        }
    }
}

/// Versión nueva de una tabla por tramos.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaTablaLaboralPeticion {
    #[schema(value_type = crate::esquemas::TablaLaboral)]
    pub tabla: TablaLaboral,
    pub vigente_desde: NaiveDate,
    #[schema(example = "Estatuto Tributario, art. 383")]
    pub fuente: String,
    /// Todos los tramos: empiezan en cero, contiguos y el último abierto.
    pub tramos: Vec<TramoDto>,
}

/// Agrega una versión completa de una tabla por tramos (retención por salarios o fondo de
/// solidaridad). Nunca se edita: un error se corrige con otra versión.
#[utoipa::path(
    post,
    path = "/nomina/tablas",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevaTablaLaboralPeticion,
    responses(
        (status = CREATED, description = "Versión creada", body = TablaLaboralRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya hay una versión con esa fecha (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Tramos que no empiezan en cero, con huecos o sin el último abierto (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_tabla_laboral(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevaTablaLaboralPeticion>,
) -> ApiResult<(StatusCode, Json<TablaLaboralRespuesta>)> {
    let t = uc_catalogo::crear_tabla_laboral(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        NuevaTablaLaboral {
            tabla: p.tabla,
            vigente_desde: p.vigente_desde,
            fuente: p.fuente,
            tramos: p
                .tramos
                .into_iter()
                .map(|t| Tramo {
                    desde: t.desde,
                    hasta: t.hasta,
                    tarifa: t.tarifa,
                    adicional: t.adicional,
                })
                .collect(),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(t.into())))
}

// ---------------------------------------------------------------------------------------------
// Configuración
// ---------------------------------------------------------------------------------------------

/// Configuración de la nómina de la entidad.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConfiguracionNominaRespuesta {
    /// Aplica la exoneración del art. 114-1 del ET (salud del empleador, ICBF y SENA por debajo
    /// del tope del catálogo).
    pub exonerada_aportes: bool,
    pub actualizada_en: DateTime<Utc>,
}

impl From<ConfiguracionNomina> for ConfiguracionNominaRespuesta {
    fn from(c: ConfiguracionNomina) -> Self {
        Self {
            exonerada_aportes: c.exonerada_aportes,
            actualizada_en: c.actualizada_en,
        }
    }
}

/// Cambios de la configuración.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfiguracionNominaPeticion {
    /// La deciden el contador y el revisor fiscal según la situación tributaria de la entidad.
    pub exonerada_aportes: bool,
}

/// Configuración de la nómina (la predeterminada, sin exoneración, si nunca se guardó).
#[utoipa::path(
    get,
    path = "/configuracion",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Configuración", body = ConfiguracionNominaRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn ver_configuracion(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<ConfiguracionNominaRespuesta>> {
    let c = uc::configuracion(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(c.into()))
}

/// Guarda la configuración de la nómina.
#[utoipa::path(
    put,
    path = "/configuracion",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = ConfiguracionNominaPeticion,
    responses(
        (status = OK, description = "Configuración guardada", body = ConfiguracionNominaRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.configurar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn guardar_configuracion(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<ConfiguracionNominaPeticion>,
) -> ApiResult<Json<ConfiguracionNominaRespuesta>> {
    let c = uc::guardar_configuracion(&state.unidad, &RelojSistema, &en.ctx, p.exonerada_aportes)
        .await?;
    Ok(Json(c.into()))
}

// ---------------------------------------------------------------------------------------------
// Novedades
// ---------------------------------------------------------------------------------------------

/// Novedad nueva de un contrato.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaNovedadPeticion {
    #[schema(value_type = crate::esquemas::TipoNovedad)]
    pub tipo: TipoNovedad,
    /// Fecha (horas y valores) o primer día (días).
    pub desde: NaiveDate,
    /// Último día: solo en las de días (incapacidades, licencias, ausencias).
    pub hasta: Option<NaiveDate>,
    /// Horas (de 0 a 24) o valor en pesos; no va en las de días.
    #[schema(value_type = Option<String>, example = "4")]
    pub cantidad: Option<Decimal>,
    pub descripcion: Option<String>,
    /// Soporte ya subido a `/archivos` (incapacidad, licencia…).
    pub soporte_archivo_id: Option<Uuid>,
}

/// Novedad registrada.
#[derive(Debug, Serialize, ToSchema)]
pub struct NovedadNominaRespuesta {
    pub id: Uuid,
    pub contrato_id: Uuid,
    #[schema(value_type = crate::esquemas::TipoNovedad)]
    pub tipo: TipoNovedad,
    #[schema(value_type = crate::esquemas::MedidaNovedad)]
    pub medida: Medida,
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    /// Horas o valor; los valores en `0` sin `nomina.salarios.ver`.
    #[schema(value_type = String)]
    pub cantidad: Decimal,
    pub descripcion: Option<String>,
    pub soporte_archivo_id: Option<Uuid>,
    pub creado_en: DateTime<Utc>,
    pub anulada: bool,
    pub motivo_anulacion: Option<String>,
}

impl From<NovedadNomina> for NovedadNominaRespuesta {
    fn from(n: NovedadNomina) -> Self {
        Self {
            id: n.id.as_uuid(),
            contrato_id: n.contrato_id.as_uuid(),
            tipo: n.novedad.tipo,
            medida: n.novedad.tipo.medida(),
            desde: n.novedad.desde,
            hasta: n.novedad.hasta,
            cantidad: n.novedad.cantidad,
            descripcion: n.descripcion,
            soporte_archivo_id: n.soporte_archivo_id.map(ArchivoId::as_uuid),
            creado_en: n.creado_en,
            anulada: n.anulacion.is_some(),
            motivo_anulacion: n.anulacion.map(|a| a.motivo),
        }
    }
}

/// Registra una novedad del contrato (dentro de sus fechas y fuera de periodos aprobados).
#[utoipa::path(
    post,
    path = "/contratos/{contrato_id}/novedades",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("contrato_id" = Uuid, Path, description = "Contrato laboral")),
    request_body = NuevaNovedadPeticion,
    responses(
        (status = CREATED, description = "Novedad registrada", body = NovedadNominaRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.novedades.registrar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Contrato o soporte inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Medida o fechas que no corresponden, dominical que no cae en domingo o festivo, fuera del contrato o en un periodo aprobado (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_novedad(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<NuevaNovedadPeticion>,
) -> ApiResult<(StatusCode, Json<NovedadNominaRespuesta>)> {
    let n = uc_novedades::registrar_novedad(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ContratoLaboralId::from_uuid(ruta.1),
        NuevaNovedad {
            tipo: p.tipo,
            desde: p.desde,
            hasta: p.hasta,
            cantidad: p.cantidad,
            descripcion: p.descripcion,
            soporte_archivo_id: p.soporte_archivo_id.map(ArchivoId::from_uuid),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(n.into())))
}

/// Rango de las novedades.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroNovedadesParametros {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    /// Solo las de este contrato.
    pub contrato_id: Option<Uuid>,
}

/// Novedades que tocan el rango (con las anuladas marcadas). Las incapacidades y licencias de
/// maternidad o paternidad solo con `personas.datos_sensibles.ver`.
#[utoipa::path(
    get,
    path = "/novedades",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroNovedadesParametros),
    responses(
        (status = OK, description = "Novedades", body = Vec<NovedadNominaRespuesta>),
        (status = FORBIDDEN, description = "Sin `nomina.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "`hasta` antes de `desde` (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_novedades(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<FiltroNovedadesParametros>,
) -> ApiResult<Json<Vec<NovedadNominaRespuesta>>> {
    let v = uc_novedades::novedades(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        q.contrato_id.map(ContratoLaboralId::from_uuid),
        q.desde,
        q.hasta,
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Motivo de la anulación.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AnulacionNovedadPeticion {
    #[schema(example = "Registrada en el contrato equivocado")]
    pub motivo: String,
}

/// Anula una novedad (deja de entrar a la liquidación).
#[utoipa::path(
    post,
    path = "/novedades/{novedad_id}/anulacion",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("novedad_id" = Uuid, Path, description = "Novedad")),
    request_body = AnulacionNovedadPeticion,
    responses(
        (status = NO_CONTENT, description = "Anulada"),
        (status = FORBIDDEN, description = "Sin `nomina.novedades.registrar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Novedad inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya estaba anulada (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Cae en un periodo aprobado o falta el motivo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn anular_novedad(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AnulacionNovedadPeticion>,
) -> ApiResult<StatusCode> {
    uc_novedades::anular_novedad(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NovedadNominaId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Periodos y liquidación
// ---------------------------------------------------------------------------------------------

/// Periodo de nómina.
#[derive(Debug, Serialize, ToSchema)]
pub struct PeriodoNominaRespuesta {
    pub id: Uuid,
    #[schema(value_type = crate::esquemas::PeriodicidadPago)]
    pub periodicidad: PeriodicidadPago,
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    #[schema(value_type = crate::esquemas::EstadoPeriodoNomina)]
    pub estado: EstadoPeriodoNomina,
    pub liquidado_por: Option<Uuid>,
    pub liquidado_en: Option<DateTime<Utc>>,
    pub aprobado_por: Option<Uuid>,
    pub aprobado_en: Option<DateTime<Utc>>,
    pub fecha_pago: Option<NaiveDate>,
}

impl From<PeriodoNomina> for PeriodoNominaRespuesta {
    fn from(p: PeriodoNomina) -> Self {
        Self {
            id: p.id.as_uuid(),
            periodicidad: p.periodicidad,
            desde: p.desde,
            hasta: p.hasta,
            estado: p.estado,
            liquidado_por: p.liquidado_por.map(|c| c.as_uuid()),
            liquidado_en: p.liquidado_en,
            aprobado_por: p.aprobado_por.map(|c| c.as_uuid()),
            aprobado_en: p.aprobado_en,
            fecha_pago: p.fecha_pago,
        }
    }
}

/// Línea de una liquidación.
#[derive(Debug, Serialize, ToSchema)]
pub struct LineaLiquidacionNominaRespuesta {
    #[schema(value_type = crate::esquemas::ConceptoNomina)]
    pub concepto: ConceptoNomina,
    #[schema(value_type = crate::esquemas::ClaseLineaNomina)]
    pub clase: ClaseLinea,
    /// Días, horas o el porcentaje del aporte.
    #[schema(value_type = String)]
    pub cantidad: Decimal,
    #[schema(value_type = String)]
    pub valor: Decimal,
}

/// Liquidación de un contrato en el periodo.
#[derive(Debug, Serialize, ToSchema)]
pub struct LiquidacionNominaRespuesta {
    pub id: Uuid,
    pub contrato_id: Uuid,
    pub empleado_id: Uuid,
    pub dias: u32,
    #[schema(value_type = String)]
    pub ibc: Decimal,
    #[schema(value_type = String)]
    pub devengos: Decimal,
    #[schema(value_type = String)]
    pub deducciones: Decimal,
    #[schema(value_type = String)]
    pub neto: Decimal,
    #[schema(value_type = String)]
    pub aportes_empleador: Decimal,
    pub lineas: Vec<LineaLiquidacionNominaRespuesta>,
}

impl From<LiquidacionContrato> for LiquidacionNominaRespuesta {
    fn from(l: LiquidacionContrato) -> Self {
        let q = l.liquidacion;
        Self {
            id: l.id.as_uuid(),
            contrato_id: l.contrato_id.as_uuid(),
            empleado_id: l.empleado_id.as_uuid(),
            dias: q.dias,
            ibc: q.ibc.valor(),
            devengos: q.devengos.valor(),
            deducciones: q.deducciones.valor(),
            neto: q.neto.valor(),
            aportes_empleador: q.aportes_empleador.valor(),
            lineas: q
                .lineas
                .into_iter()
                .map(|x| LineaLiquidacionNominaRespuesta {
                    concepto: x.concepto,
                    clase: x.concepto.clase(),
                    cantidad: x.cantidad.normalize(),
                    valor: x.valor.valor(),
                })
                .collect(),
        }
    }
}

/// Periodo con sus liquidaciones.
#[derive(Debug, Serialize, ToSchema)]
pub struct PeriodoLiquidadoRespuesta {
    pub periodo: PeriodoNominaRespuesta,
    pub liquidaciones: Vec<LiquidacionNominaRespuesta>,
}

impl From<PeriodoLiquidado> for PeriodoLiquidadoRespuesta {
    fn from(p: PeriodoLiquidado) -> Self {
        Self {
            periodo: p.periodo.into(),
            liquidaciones: p.liquidaciones.into_iter().map(Into::into).collect(),
        }
    }
}

/// Periodo nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoPeriodoNominaPeticion {
    #[schema(value_type = crate::esquemas::PeriodicidadPago)]
    pub periodicidad: PeriodicidadPago,
    /// El día 1 (o el 16 en la quincenal).
    pub desde: NaiveDate,
}

/// Periodos de la entidad, el más reciente primero (a lo sumo 120).
#[utoipa::path(
    get,
    path = "/periodos",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Periodos", body = Vec<PeriodoNominaRespuesta>),
        (status = FORBIDDEN, description = "Sin `nomina.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_periodos(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<PeriodoNominaRespuesta>>> {
    let v = uc::periodos(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Abre un periodo (el mes completo, o la quincena del 1 al 15 o del 16 al fin de mes).
#[utoipa::path(
    post,
    path = "/periodos",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevoPeriodoNominaPeticion,
    responses(
        (status = CREATED, description = "Periodo abierto", body = PeriodoNominaRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.liquidar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "El periodo ya existe (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "`desde` no inicia un periodo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_periodo(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevoPeriodoNominaPeticion>,
) -> ApiResult<(StatusCode, Json<PeriodoNominaRespuesta>)> {
    let v = uc::crear_periodo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        p.periodicidad,
        p.desde,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(v.into())))
}

/// Periodo con sus liquidaciones (datos personales).
#[utoipa::path(
    get,
    path = "/periodos/{periodo_id}",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("periodo_id" = Uuid, Path, description = "Periodo de nómina")),
    responses(
        (status = OK, description = "Periodo", body = PeriodoLiquidadoRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.salarios.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Periodo inexistente", body = ErrorRespuesta),
    )
)]
pub async fn ver_periodo(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<PeriodoLiquidadoRespuesta>> {
    let p = uc::periodo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PeriodoNominaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(p.into()))
}

/// Liquida (o vuelve a liquidar) todos los contratos del periodo con su periodicidad.
#[utoipa::path(
    post,
    path = "/periodos/{periodo_id}/liquidacion",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("periodo_id" = Uuid, Path, description = "Periodo de nómina")),
    responses(
        (status = OK, description = "Liquidado", body = PeriodoLiquidadoRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.liquidar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Periodo inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya fue aprobado (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Falta un parámetro, una tabla o la UVT vigentes, o un contrato no se puede liquidar (`validacion`; el mensaje dice el código del empleado)", body = ErrorRespuesta),
    )
)]
pub async fn liquidar_periodo(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<PeriodoLiquidadoRespuesta>> {
    let p = uc::liquidar_periodo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PeriodoNominaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(p.into()))
}

/// Aprueba la liquidación: otra cuenta distinta de la que liquidó y sin novedades posteriores.
#[utoipa::path(
    post,
    path = "/periodos/{periodo_id}/aprobacion",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("periodo_id" = Uuid, Path, description = "Periodo de nómina")),
    responses(
        (status = OK, description = "Aprobado", body = PeriodoNominaRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.aprobar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Periodo inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está liquidado (`conflicto_estado`), lo aprueba quien liquidó o hay novedades posteriores (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn aprobar_periodo(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<PeriodoNominaRespuesta>> {
    let p = uc::aprobar_periodo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PeriodoNominaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(p.into()))
}

/// Fecha en que se pagó la nómina.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PagoPeriodoNominaPeticion {
    pub fecha_pago: NaiveDate,
}

/// Marca pagado un periodo aprobado.
#[utoipa::path(
    post,
    path = "/periodos/{periodo_id}/pago",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("periodo_id" = Uuid, Path, description = "Periodo de nómina")),
    request_body = PagoPeriodoNominaPeticion,
    responses(
        (status = OK, description = "Pagado", body = PeriodoNominaRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.liquidar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Periodo inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está aprobado (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn pagar_periodo(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PagoPeriodoNominaPeticion>,
) -> ApiResult<Json<PeriodoNominaRespuesta>> {
    let v = uc::marcar_pagado(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PeriodoNominaId::from_uuid(ruta.1),
        p.fecha_pago,
    )
    .await?;
    Ok(Json(v.into()))
}

// ---------------------------------------------------------------------------------------------
// Cesantías por consignar
// ---------------------------------------------------------------------------------------------

/// Año de las cesantías.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AnioCesantiasParametros {
    #[param(example = 2026)]
    pub anio: i32,
}

/// Cesantías e intereses de un contrato en el año.
#[derive(Debug, Serialize, ToSchema)]
pub struct CesantiasContratoRespuesta {
    pub empleado_id: Uuid,
    pub contrato_id: Uuid,
    pub dias: u32,
    /// Base mensual (último salario o promedio, más auxilio y lo variable).
    #[schema(value_type = String)]
    pub base: Decimal,
    /// Por consignar al fondo antes del 14 de febrero.
    #[schema(value_type = String)]
    pub cesantias: Decimal,
    /// Por pagar al empleado antes del 31 de enero (salen en el primer periodo de enero).
    #[schema(value_type = String)]
    pub intereses: Decimal,
    pub fondo_id: Option<Uuid>,
}

impl From<CesantiasContrato> for CesantiasContratoRespuesta {
    fn from(c: CesantiasContrato) -> Self {
        Self {
            empleado_id: c.empleado_id.as_uuid(),
            contrato_id: c.contrato_id.as_uuid(),
            dias: c.dias,
            base: c.base,
            cesantias: c.cesantias,
            intereses: c.intereses,
            fondo_id: c.fondo_id.map(|f| f.as_uuid()),
        }
    }
}

/// Cesantías del año por consignar (contratos con salario ordinario vigentes al 31 de diciembre).
#[utoipa::path(
    get,
    path = "/cesantias",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), AnioCesantiasParametros),
    responses(
        (status = OK, description = "Cesantías por contrato", body = Vec<CesantiasContratoRespuesta>),
        (status = FORBIDDEN, description = "Sin `nomina.salarios.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Falta un parámetro del catálogo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn cesantias_del_ano(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<AnioCesantiasParametros>,
) -> ApiResult<Json<Vec<CesantiasContratoRespuesta>>> {
    let v =
        uc_prestaciones::cesantias_del_ano(&state.unidad, &RelojSistema, &en.ctx, q.anio).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}
