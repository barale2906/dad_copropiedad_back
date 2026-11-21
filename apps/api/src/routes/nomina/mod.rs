//! Rutas de la nómina (plan 16, ADR 0025): `/api/v1/entidades/{entidad_id}/nomina/…` con el
//! extractor [`EnEntidad`] (catálogo vigente, empleados, contratos e importación) y el catálogo
//! laboral de la plataforma en `/api/v1/plataforma/nomina/…`. Los handlers solo traducen HTTP ↔
//! casos de uso.

use application::{
    AppError,
    casos_uso::nomina::{
        catalogo::{
            self as uc_catalogo, CatalogoLaboralRespuesta as Catalogo, NuevaAdministradora,
            NuevoParametro,
        },
        empleados::{
            self as uc, CondicionesVersion, ContratoConVersiones, FichaEmpleado,
            NuevaPersonaEmpleado, NuevoContrato, NuevoEmpleado, PersonaEmpleado,
        },
        importacion::{self as uc_importacion, FilaEmpleadoPrevia, VistaPreviaEmpleados},
    },
    puertos::{
        RelojSistema,
        nomina::{
            Administradora, ContratoLaboral, CuentaNomina, FiltroEmpleados, ResumenEmpleado,
            VersionContrato, VersionParametroLaboral,
        },
    },
};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AdministradoraId, ContratoLaboralId, DireccionCorreo, Documento, EmpleadoId, ImportacionId,
    Nit, NombreArchivo, PersonaId, Telefono, TipoArchivo, TipoDocumento,
    contabilidad::{Monto, tesoreria::TipoCuentaBancaria},
    importacion::empleados::encabezados_empleados,
    nomina::{
        ClaseRiesgo, CodigoEmpleado, EstadoEmpleado, EtapaAprendizaje, Jornada, MedioPago,
        MotivoTerminacion, ParametroLaboral, PeriodicidadPago, TipoAdministradora,
        TipoContratoLaboral, TipoSalario, Unidad,
    },
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

mod complementos;
mod desprendibles;
mod liquidacion;
mod reportes;

pub(crate) use desprendibles::rutas_yo;

pub(crate) use liquidacion::TablaLaboralRespuesta;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::{ETIQUETA_NOMINA, ETIQUETA_PLATAFORMA},
    paginacion::{PaginaRespuesta, ParametrosPagina},
    routes::importaciones::{DatosPlanilla, ImportacionRespuesta},
    sesion::{EnEntidad, OperadorPlataforma},
    state::AppState,
};

/// Rutas de la nómina de una entidad (se montan en `/entidades/{entidad_id}/nomina`).
pub fn rutas() -> OpenApiRouter<AppState> {
    let limite = usize::try_from(TipoArchivo::Xlsx.tamano_maximo()).unwrap_or(usize::MAX);
    OpenApiRouter::new()
        .routes(routes!(importar))
        .layer(DefaultBodyLimit::max(limite))
        .routes(routes!(plantilla))
        .routes(routes!(confirmar_importacion))
        .routes(routes!(catalogo))
        .routes(routes!(listar_empleados, crear_empleado))
        .routes(routes!(ver_empleado))
        .routes(routes!(contratos_por_vencer))
        .routes(routes!(crear_contrato))
        .routes(routes!(modificar_contrato))
        .routes(routes!(prorrogar_contrato))
        .routes(routes!(terminar_contrato))
        .merge(liquidacion::rutas())
        .merge(desprendibles::rutas())
        .merge(reportes::rutas())
        .merge(complementos::rutas())
}

/// Rutas del catálogo laboral de la plataforma (se montan en `/api/v1/plataforma`).
pub fn rutas_plataforma() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(catalogo_laboral_plataforma))
        .routes(routes!(crear_parametro_laboral))
        .routes(routes!(crear_administradora))
        .routes(routes!(actualizar_administradora))
        .routes(routes!(liquidacion::crear_tabla_laboral))
}

// ---------------------------------------------------------------------------------------------
// Catálogo
// ---------------------------------------------------------------------------------------------

/// Versión de un parámetro laboral.
#[derive(Debug, Serialize, ToSchema)]
pub struct ParametroLaboralRespuesta {
    pub id: Uuid,
    #[schema(value_type = crate::esquemas::ParametroLaboral)]
    pub parametro: ParametroLaboral,
    /// Unidad del valor (pesos, porcentaje, salarios mínimos, horas a la semana, hora del día o
    /// días).
    #[schema(value_type = crate::esquemas::UnidadParametroLaboral)]
    pub unidad: Unidad,
    #[schema(value_type = String, example = "1750905")]
    pub valor: Decimal,
    #[schema(example = "Decreto 1572 de 2025")]
    pub fuente: String,
    pub vigente_desde: NaiveDate,
    pub creado_en: DateTime<Utc>,
}

impl From<VersionParametroLaboral> for ParametroLaboralRespuesta {
    fn from(v: VersionParametroLaboral) -> Self {
        Self {
            id: v.id.as_uuid(),
            unidad: v.parametro.unidad(),
            parametro: v.parametro,
            valor: v.valor.normalize(),
            fuente: v.fuente,
            vigente_desde: v.vigente_desde,
            creado_en: v.creado_en,
        }
    }
}

/// Administradora de la seguridad social.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdministradoraRespuesta {
    pub id: Uuid,
    #[schema(value_type = crate::esquemas::TipoAdministradora)]
    pub tipo: TipoAdministradora,
    /// Código en la PILA.
    #[schema(example = "EPS037")]
    pub codigo: String,
    #[schema(example = "900156264-2")]
    pub nit: String,
    #[schema(example = "Nueva EPS")]
    pub nombre: String,
    pub activa: bool,
}

impl From<Administradora> for AdministradoraRespuesta {
    fn from(a: Administradora) -> Self {
        Self {
            id: a.id.as_uuid(),
            tipo: a.tipo,
            codigo: a.codigo,
            nit: a.nit.to_string(),
            nombre: a.nombre,
            activa: a.activa,
        }
    }
}

/// Catálogo laboral: versiones de los parámetros y administradoras.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogoLaboralRespuesta {
    /// En la entidad, la versión vigente de cada parámetro en la fecha; en la plataforma, todas.
    pub parametros: Vec<ParametroLaboralRespuesta>,
    /// En la entidad, las activas; en la plataforma, todas.
    pub administradoras: Vec<AdministradoraRespuesta>,
    /// Tablas por tramos: en la entidad, la vigente de cada una; en la plataforma, todas.
    pub tablas: Vec<TablaLaboralRespuesta>,
}

impl From<Catalogo> for CatalogoLaboralRespuesta {
    fn from(c: Catalogo) -> Self {
        Self {
            parametros: c.parametros.into_iter().map(Into::into).collect(),
            administradoras: c.administradoras.into_iter().map(Into::into).collect(),
            tablas: c.tablas.into_iter().map(Into::into).collect(),
        }
    }
}

/// Fecha del catálogo vigente.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FechaCatalogoParametros {
    /// Vigente en esta fecha (hoy si se omite).
    pub fecha: Option<NaiveDate>,
}

/// Catálogo laboral vigente en la fecha y administradoras activas (lo que usan los contratos).
#[utoipa::path(
    get,
    path = "/catalogo",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FechaCatalogoParametros),
    responses(
        (status = OK, description = "Catálogo vigente", body = CatalogoLaboralRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn catalogo(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<FechaCatalogoParametros>,
) -> ApiResult<Json<CatalogoLaboralRespuesta>> {
    let c = uc_catalogo::catalogo_vigente(&state.unidad, &RelojSistema, &en.ctx, q.fecha).await?;
    Ok(Json(c.into()))
}

/// Catálogo laboral completo de la plataforma (todas las versiones y administradoras).
#[utoipa::path(
    get,
    path = "/nomina/catalogo",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Catálogo", body = CatalogoLaboralRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn catalogo_laboral_plataforma(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
) -> ApiResult<Json<CatalogoLaboralRespuesta>> {
    let c = uc_catalogo::catalogo_plataforma(&state.plataforma, &op.ctx).await?;
    Ok(Json(c.into()))
}

/// Versión nueva de un parámetro laboral.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoParametroLaboralPeticion {
    #[schema(value_type = crate::esquemas::ParametroLaboral)]
    pub parametro: ParametroLaboral,
    /// En la unidad del parámetro (porcentaje de 0 a 100, pesos, hora del día…).
    #[schema(value_type = String, example = "1750905")]
    pub valor: Decimal,
    #[schema(example = "Decreto 1572 de 2025")]
    pub fuente: String,
    pub vigente_desde: NaiveDate,
}

/// Agrega una versión de un parámetro laboral. El catálogo es de solo inserción: un error se
/// corrige con otra versión.
#[utoipa::path(
    post,
    path = "/nomina/parametros",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevoParametroLaboralPeticion,
    responses(
        (status = CREATED, description = "Versión creada", body = ParametroLaboralRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya hay una versión con esa fecha (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "El valor no cabe en la unidad del parámetro (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_parametro_laboral(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevoParametroLaboralPeticion>,
) -> ApiResult<(StatusCode, Json<ParametroLaboralRespuesta>)> {
    let v = uc_catalogo::crear_parametro(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        NuevoParametro {
            parametro: p.parametro,
            valor: p.valor,
            fuente: p.fuente,
            vigente_desde: p.vigente_desde,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(v.into())))
}

/// Administradora nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaAdministradoraPeticion {
    #[schema(value_type = crate::esquemas::TipoAdministradora)]
    pub tipo: TipoAdministradora,
    /// Código en la PILA (único por tipo).
    #[schema(example = "EPS037")]
    pub codigo: String,
    #[schema(example = "900156264-2")]
    pub nit: String,
    #[schema(example = "Nueva EPS")]
    pub nombre: String,
}

/// Agrega una administradora de la seguridad social al catálogo.
#[utoipa::path(
    post,
    path = "/nomina/administradoras",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevaAdministradoraPeticion,
    responses(
        (status = CREATED, description = "Administradora creada", body = AdministradoraRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya existe una del mismo tipo con ese código (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Código, NIT o nombre inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_administradora(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevaAdministradoraPeticion>,
) -> ApiResult<(StatusCode, Json<AdministradoraRespuesta>)> {
    let a = uc_catalogo::crear_administradora(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        NuevaAdministradora {
            tipo: p.tipo,
            codigo: p.codigo,
            nit: Nit::new(&p.nit)?,
            nombre: p.nombre,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(a.into())))
}

/// Cambios de una administradora (el tipo, el código y el NIT no cambian).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CambiosAdministradoraPeticion {
    pub nombre: String,
    /// Una inactiva no se usa en contratos ni versiones nuevas.
    pub activa: bool,
}

/// Cambia el nombre y el estado de una administradora.
#[utoipa::path(
    put,
    path = "/nomina/administradoras/{administradora_id}",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("administradora_id" = Uuid, Path, description = "Administradora")),
    request_body = CambiosAdministradoraPeticion,
    responses(
        (status = OK, description = "Administradora actualizada", body = AdministradoraRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Administradora inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Nombre inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn actualizar_administradora(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<CambiosAdministradoraPeticion>,
) -> ApiResult<Json<AdministradoraRespuesta>> {
    let a = uc_catalogo::actualizar_administradora(
        &state.plataforma,
        &op.ctx,
        AdministradoraId::from_uuid(id),
        &p.nombre,
        p.activa,
    )
    .await?;
    Ok(Json(a.into()))
}

// ---------------------------------------------------------------------------------------------
// Empleados y contratos
// ---------------------------------------------------------------------------------------------

/// Cuenta bancaria de la nómina (dato personal).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CuentaNominaDto {
    #[schema(example = "Bancolombia")]
    pub banco: String,
    #[schema(value_type = crate::esquemas::TipoCuentaBancaria)]
    pub tipo: TipoCuentaBancaria,
    /// Solo dígitos (4 a 20).
    #[schema(example = "0012345678")]
    pub numero: String,
}

/// Condiciones de una versión del contrato.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CondicionesContratoPeticion {
    #[schema(example = "Todero")]
    pub cargo: String,
    /// Salario mensual (o apoyo de sostenimiento del aprendiz).
    #[schema(value_type = String, example = "1750905")]
    pub salario: Monto,
    #[schema(value_type = crate::esquemas::TipoSalario)]
    pub tipo_salario: TipoSalario,
    /// Solo con el apoyo de sostenimiento.
    #[schema(value_type = Option<crate::esquemas::EtapaAprendizaje>)]
    pub etapa_aprendizaje: Option<EtapaAprendizaje>,
    #[schema(value_type = crate::esquemas::Jornada)]
    pub jornada: Jornada,
    /// Solo en jornada parcial.
    #[schema(value_type = Option<String>, example = "21")]
    pub horas_semanales: Option<Decimal>,
    #[schema(value_type = crate::esquemas::PeriodicidadPago)]
    pub periodicidad: PeriodicidadPago,
    pub eps_id: Uuid,
    /// Salvo pensionados y aprendices.
    pub afp_id: Option<Uuid>,
    /// Solo con salario ordinario.
    pub cesantias_id: Option<Uuid>,
    /// Salvo aprendices.
    pub ccf_id: Option<Uuid>,
    /// Salvo el aprendiz en etapa lectiva; va con `clase_riesgo`.
    pub arl_id: Option<Uuid>,
    #[schema(value_type = Option<crate::esquemas::ClaseRiesgo>)]
    pub clase_riesgo: Option<ClaseRiesgo>,
    #[serde(default)]
    pub pensionado: bool,
    /// Centro de costo del gasto (solo si la entidad lleva contabilidad).
    pub centro_costo_id: Option<Uuid>,
    #[schema(value_type = crate::esquemas::MedioPago)]
    pub medio_pago: MedioPago,
    /// Solo con transferencia.
    pub cuenta: Option<CuentaNominaDto>,
}

impl From<CondicionesContratoPeticion> for CondicionesVersion {
    fn from(p: CondicionesContratoPeticion) -> Self {
        let adm = |u: Option<Uuid>| u.map(AdministradoraId::from_uuid);
        Self {
            cargo: p.cargo,
            salario: p.salario,
            tipo_salario: p.tipo_salario,
            etapa_aprendizaje: p.etapa_aprendizaje,
            jornada: p.jornada,
            horas_semanales: p.horas_semanales,
            periodicidad: p.periodicidad,
            eps_id: AdministradoraId::from_uuid(p.eps_id),
            afp_id: adm(p.afp_id),
            cesantias_id: adm(p.cesantias_id),
            ccf_id: adm(p.ccf_id),
            arl_id: adm(p.arl_id),
            clase_riesgo: p.clase_riesgo,
            pensionado: p.pensionado,
            centro_costo_id: p.centro_costo_id.map(domain::CentroCostoId::from_uuid),
            medio_pago: p.medio_pago,
            cuenta: p.cuenta.map(|c| CuentaNomina {
                banco: c.banco,
                tipo: c.tipo,
                numero: c.numero,
            }),
        }
    }
}

/// Contrato nuevo con sus condiciones iniciales.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoContratoLaboralPeticion {
    #[schema(value_type = crate::esquemas::TipoContratoLaboral)]
    pub tipo: TipoContratoLaboral,
    pub fecha_inicio: NaiveDate,
    /// Término fijo y aprendizaje.
    pub fecha_fin: Option<NaiveDate>,
    /// Obra o labor.
    pub descripcion_obra: Option<String>,
    pub condiciones: CondicionesContratoPeticion,
}

impl From<NuevoContratoLaboralPeticion> for NuevoContrato {
    fn from(p: NuevoContratoLaboralPeticion) -> Self {
        Self {
            tipo: p.tipo,
            fecha_inicio: p.fecha_inicio,
            fecha_fin: p.fecha_fin,
            descripcion_obra: p.descripcion_obra,
            condiciones: p.condiciones.into(),
        }
    }
}

/// Persona nueva del empleado (si el documento ya está en la entidad, se usa esa persona).
#[derive(Debug, Deserialize, ToSchema)]
pub struct PersonaEmpleadoPeticion {
    #[schema(example = "Ana Pérez")]
    pub nombre: String,
    #[schema(value_type = crate::esquemas::TipoDocumento)]
    pub tipo_documento: TipoDocumento,
    #[schema(example = "52123456")]
    pub documento: String,
    pub correo: Option<String>,
    pub telefono: Option<String>,
}

/// Empleado nuevo con su primer contrato: `persona_id` (una persona de la entidad) **o**
/// `persona` (sus datos).
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoEmpleadoPeticion {
    /// Código interno: 1 a 30 caracteres `A-Z a-z 0-9 . _ -`.
    #[schema(example = "E-001")]
    pub codigo: String,
    pub persona_id: Option<Uuid>,
    pub persona: Option<PersonaEmpleadoPeticion>,
    pub contrato: NuevoContratoLaboralPeticion,
}

fn alta(p: NuevoEmpleadoPeticion) -> ApiResult<NuevoEmpleado> {
    let persona = match (p.persona_id, p.persona) {
        (Some(id), None) => PersonaEmpleado::Existente(PersonaId::from_uuid(id)),
        (None, Some(n)) => PersonaEmpleado::Nueva(NuevaPersonaEmpleado {
            nombre: n.nombre,
            tipo_documento: n.tipo_documento,
            documento: Documento::new(&n.documento)?,
            correo: n
                .correo
                .as_deref()
                .filter(|c| !c.trim().is_empty())
                .map(DireccionCorreo::new)
                .transpose()?,
            telefono: n
                .telefono
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .map(Telefono::new)
                .transpose()?,
        }),
        _ => {
            return Err(ApiError::from(AppError::Validacion(
                "envíe `persona_id` o `persona` (uno de los dos)".into(),
            )));
        }
    };
    Ok(NuevoEmpleado {
        codigo: CodigoEmpleado::new(&p.codigo)?,
        persona,
        contrato: p.contrato.into(),
    })
}

/// Contrato laboral (sin sus condiciones).
#[derive(Debug, Serialize, ToSchema)]
pub struct ContratoLaboralRespuesta {
    pub id: Uuid,
    pub empleado_id: Uuid,
    #[schema(value_type = crate::esquemas::TipoContratoLaboral)]
    pub tipo: TipoContratoLaboral,
    pub fecha_inicio: NaiveDate,
    pub fecha_fin: Option<NaiveDate>,
    pub descripcion_obra: Option<String>,
    pub fecha_terminacion: Option<NaiveDate>,
    #[schema(value_type = Option<crate::esquemas::MotivoTerminacion>)]
    pub motivo_terminacion: Option<MotivoTerminacion>,
    pub creado_en: DateTime<Utc>,
}

impl From<ContratoLaboral> for ContratoLaboralRespuesta {
    fn from(c: ContratoLaboral) -> Self {
        Self {
            id: c.id.as_uuid(),
            empleado_id: c.empleado_id.as_uuid(),
            tipo: c.tipo,
            fecha_inicio: c.fecha_inicio,
            fecha_fin: c.fecha_fin,
            descripcion_obra: c.descripcion_obra,
            fecha_terminacion: c.fecha_terminacion,
            motivo_terminacion: c.motivo_terminacion,
            creado_en: c.creado_en,
        }
    }
}

/// Condiciones de un contrato desde una fecha.
#[derive(Debug, Serialize, ToSchema)]
pub struct VersionContratoRespuesta {
    pub id: Uuid,
    pub contrato_id: Uuid,
    pub vigente_desde: NaiveDate,
    pub cargo: String,
    /// `null` sin `nomina.salarios.ver`.
    #[schema(value_type = Option<String>)]
    pub salario: Option<Decimal>,
    #[schema(value_type = crate::esquemas::TipoSalario)]
    pub tipo_salario: TipoSalario,
    #[schema(value_type = Option<crate::esquemas::EtapaAprendizaje>)]
    pub etapa_aprendizaje: Option<EtapaAprendizaje>,
    #[schema(value_type = crate::esquemas::Jornada)]
    pub jornada: Jornada,
    #[schema(value_type = Option<String>)]
    pub horas_semanales: Option<Decimal>,
    #[schema(value_type = crate::esquemas::PeriodicidadPago)]
    pub periodicidad: PeriodicidadPago,
    pub eps_id: Uuid,
    pub afp_id: Option<Uuid>,
    pub cesantias_id: Option<Uuid>,
    pub ccf_id: Option<Uuid>,
    pub arl_id: Option<Uuid>,
    #[schema(value_type = Option<crate::esquemas::ClaseRiesgo>)]
    pub clase_riesgo: Option<ClaseRiesgo>,
    pub pensionado: bool,
    pub centro_costo_id: Option<Uuid>,
    #[schema(value_type = crate::esquemas::MedioPago)]
    pub medio_pago: MedioPago,
    /// `null` sin transferencia o sin `nomina.salarios.ver`.
    pub cuenta: Option<CuentaNominaDto>,
    pub creada_en: DateTime<Utc>,
}

impl From<VersionContrato> for VersionContratoRespuesta {
    fn from(v: VersionContrato) -> Self {
        let id = |a: Option<AdministradoraId>| a.map(AdministradoraId::as_uuid);
        Self {
            id: v.id.as_uuid(),
            contrato_id: v.contrato_id.as_uuid(),
            vigente_desde: v.vigente_desde,
            cargo: v.cargo,
            salario: v.salario.map(Monto::valor),
            tipo_salario: v.tipo_salario,
            etapa_aprendizaje: v.etapa_aprendizaje,
            jornada: v.jornada,
            horas_semanales: v.horas_semanales,
            periodicidad: v.periodicidad,
            eps_id: v.eps_id.as_uuid(),
            afp_id: id(v.afp_id),
            cesantias_id: id(v.cesantias_id),
            ccf_id: id(v.ccf_id),
            arl_id: id(v.arl_id),
            clase_riesgo: v.clase_riesgo,
            pensionado: v.pensionado,
            centro_costo_id: v.centro_costo_id.map(|c| c.as_uuid()),
            medio_pago: v.medio_pago,
            cuenta: v.cuenta.map(|c| CuentaNominaDto {
                banco: c.banco,
                tipo: c.tipo,
                numero: c.numero,
            }),
            creada_en: v.creada_en,
        }
    }
}

/// Contrato con sus versiones (de la más antigua a la más reciente).
#[derive(Debug, Serialize, ToSchema)]
pub struct ContratoConVersionesRespuesta {
    pub contrato: ContratoLaboralRespuesta,
    pub versiones: Vec<VersionContratoRespuesta>,
}

impl From<ContratoConVersiones> for ContratoConVersionesRespuesta {
    fn from(c: ContratoConVersiones) -> Self {
        Self {
            contrato: c.contrato.into(),
            versiones: c.versiones.into_iter().map(Into::into).collect(),
        }
    }
}

/// Empleado con su persona y su historia de contratos.
#[derive(Debug, Serialize, ToSchema)]
pub struct FichaEmpleadoRespuesta {
    pub id: Uuid,
    pub codigo: String,
    pub persona_id: Uuid,
    pub nombre: String,
    #[schema(value_type = Option<crate::esquemas::TipoDocumento>)]
    pub tipo_documento: Option<TipoDocumento>,
    pub documento: Option<String>,
    pub contratos: Vec<ContratoConVersionesRespuesta>,
}

impl From<FichaEmpleado> for FichaEmpleadoRespuesta {
    fn from(f: FichaEmpleado) -> Self {
        Self {
            id: f.empleado.id.as_uuid(),
            codigo: f.empleado.codigo.as_str().to_owned(),
            persona_id: f.empleado.persona_id.as_uuid(),
            nombre: f.nombre,
            tipo_documento: f.tipo_documento,
            documento: f.documento.map(|d| d.as_str().to_owned()),
            contratos: f.contratos.into_iter().map(Into::into).collect(),
        }
    }
}

/// Empleado en el listado.
#[derive(Debug, Serialize, ToSchema)]
pub struct EmpleadoResumenRespuesta {
    pub id: Uuid,
    pub codigo: String,
    pub persona_id: Uuid,
    pub nombre: String,
    pub documento: Option<String>,
    #[schema(value_type = crate::esquemas::EstadoEmpleado)]
    pub estado: EstadoEmpleado,
    /// Cargo de la versión vigente (o de la última, si ya se retiró).
    pub cargo: Option<String>,
    /// Inicio del primer contrato.
    pub fecha_ingreso: Option<NaiveDate>,
}

impl From<ResumenEmpleado> for EmpleadoResumenRespuesta {
    fn from(e: ResumenEmpleado) -> Self {
        Self {
            id: e.id.as_uuid(),
            codigo: e.codigo.as_str().to_owned(),
            persona_id: e.persona_id.as_uuid(),
            nombre: e.nombre,
            documento: e.documento.map(|d| d.as_str().to_owned()),
            estado: e.estado,
            cargo: e.cargo,
            fecha_ingreso: e.fecha_ingreso,
        }
    }
}

/// Filtro de empleados.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroEmpleadosParametros {
    /// Fragmento del nombre, el documento o el código (sin tildes ni mayúsculas).
    pub q: Option<String>,
    #[param(value_type = Option<crate::esquemas::EstadoEmpleado>)]
    pub estado: Option<EstadoEmpleado>,
}

/// Empleados de la entidad con su estado de hoy.
#[utoipa::path(
    get,
    path = "/empleados",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroEmpleadosParametros, ParametrosPagina),
    responses(
        (status = OK, description = "Empleados", body = PaginaRespuesta<EmpleadoResumenRespuesta>),
        (status = FORBIDDEN, description = "Sin `nomina.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_empleados(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroEmpleadosParametros>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<EmpleadoResumenRespuesta>>> {
    let pagina = uc::empleados(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FiltroEmpleados {
            texto: f.q.filter(|q| !q.trim().is_empty()),
            estado: f.estado,
        },
        p.validar()?,
    )
    .await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Contrato a término fijo por vencer.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContratoPorVencerRespuesta {
    pub contrato_id: Uuid,
    pub empleado_id: Uuid,
    pub codigo: String,
    pub nombre: String,
    pub fecha_fin: NaiveDate,
    /// Último día para entregar el preaviso de no renovación (30 días antes del fin, CST art. 46);
    /// si ya pasó, el contrato se prorroga solo por un periodo igual.
    pub fecha_limite_preaviso: NaiveDate,
}

impl From<uc::ContratoPorVencer> for ContratoPorVencerRespuesta {
    fn from(c: uc::ContratoPorVencer) -> Self {
        Self {
            contrato_id: c.contrato_id.as_uuid(),
            empleado_id: c.empleado_id.as_uuid(),
            codigo: c.codigo.as_str().to_owned(),
            nombre: c.nombre,
            fecha_fin: c.fecha_fin,
            fecha_limite_preaviso: c.fecha_limite_preaviso,
        }
    }
}

/// Plazo de la consulta de contratos por vencer.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PorVencerParametros {
    /// Días hacia adelante (por omisión 45, el plazo del aviso por correo; máximo 120).
    pub dias: Option<u64>,
}

/// Contratos a término fijo sin terminar que vencen en los próximos días (los mismos del aviso por
/// correo), del más próximo al más lejano.
#[utoipa::path(
    get,
    path = "/contratos/por-vencer",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), PorVencerParametros),
    responses(
        (status = OK, description = "Contratos por vencer", body = [ContratoPorVencerRespuesta]),
        (status = UNPROCESSABLE_ENTITY, description = "`dias` mayor que 120 (`validacion`)", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn contratos_por_vencer(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<PorVencerParametros>,
) -> ApiResult<Json<Vec<ContratoPorVencerRespuesta>>> {
    let lista = uc::contratos_por_vencer(&state.unidad, &RelojSistema, &en.ctx, q.dias).await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Da de alta un empleado con su primer contrato (crea la persona si no está en la entidad).
#[utoipa::path(
    post,
    path = "/empleados",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevoEmpleadoPeticion,
    responses(
        (status = CREATED, description = "Empleado creado", body = FichaEmpleadoRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.empleados.gestionar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Persona inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Código repetido o la persona ya es empleada (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Contrato inválido: fechas, salario, afiliaciones o catálogo sin el parámetro vigente (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_empleado(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevoEmpleadoPeticion>,
) -> ApiResult<(StatusCode, Json<FichaEmpleadoRespuesta>)> {
    let f = uc::crear_empleado(&state.unidad, &RelojSistema, &en.ctx, alta(p)?).await?;
    Ok((StatusCode::CREATED, Json(f.into())))
}

/// Ficha del empleado con todos sus contratos y versiones (salarios y cuentas solo con
/// `nomina.salarios.ver`).
#[utoipa::path(
    get,
    path = "/empleados/{empleado_id}",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("empleado_id" = Uuid, Path, description = "Empleado")),
    responses(
        (status = OK, description = "Ficha", body = FichaEmpleadoRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Empleado inexistente", body = ErrorRespuesta),
    )
)]
pub async fn ver_empleado(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<FichaEmpleadoRespuesta>> {
    let f = uc::ficha_empleado(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        EmpleadoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(f.into()))
}

/// Agrega un contrato a un empleado (no se cruza en fechas con sus otros contratos).
#[utoipa::path(
    post,
    path = "/empleados/{empleado_id}/contratos",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("empleado_id" = Uuid, Path, description = "Empleado")),
    request_body = NuevoContratoLaboralPeticion,
    responses(
        (status = CREATED, description = "Contrato creado", body = ContratoConVersionesRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.empleados.gestionar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Empleado inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Se cruza con otro contrato del empleado (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Contrato inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_contrato(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<NuevoContratoLaboralPeticion>,
) -> ApiResult<(StatusCode, Json<ContratoConVersionesRespuesta>)> {
    let c = uc::crear_contrato(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        EmpleadoId::from_uuid(ruta.1),
        p.into(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Modificación del contrato desde una fecha.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ModificacionContratoPeticion {
    /// Posterior a la última versión y dentro de las fechas del contrato.
    pub vigente_desde: NaiveDate,
    pub condiciones: CondicionesContratoPeticion,
}

/// Modifica las condiciones del contrato desde una fecha (una versión nueva; las anteriores no
/// cambian).
#[utoipa::path(
    post,
    path = "/contratos/{contrato_id}/versiones",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("contrato_id" = Uuid, Path, description = "Contrato laboral")),
    request_body = ModificacionContratoPeticion,
    responses(
        (status = CREATED, description = "Versión creada", body = VersionContratoRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.empleados.gestionar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Contrato inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "El contrato ya terminó (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Fecha o condiciones inválidas (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn modificar_contrato(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ModificacionContratoPeticion>,
) -> ApiResult<(StatusCode, Json<VersionContratoRespuesta>)> {
    let v = uc::modificar_contrato(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ContratoLaboralId::from_uuid(ruta.1),
        p.vigente_desde,
        p.condiciones.into(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(v.into())))
}

/// Prórroga de un contrato a término fijo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ProrrogaContratoPeticion {
    /// Posterior a la fecha final actual.
    pub fecha_fin: NaiveDate,
}

/// Prorroga un contrato a término fijo.
#[utoipa::path(
    post,
    path = "/contratos/{contrato_id}/prorroga",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("contrato_id" = Uuid, Path, description = "Contrato laboral")),
    request_body = ProrrogaContratoPeticion,
    responses(
        (status = OK, description = "Contrato prorrogado", body = ContratoLaboralRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.empleados.gestionar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Contrato inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya terminó o la prórroga lo cruza con otro contrato (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es a término fijo o la fecha no es posterior (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn prorrogar_contrato(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ProrrogaContratoPeticion>,
) -> ApiResult<Json<ContratoLaboralRespuesta>> {
    let c = uc::prorrogar_contrato(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ContratoLaboralId::from_uuid(ruta.1),
        p.fecha_fin,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Terminación de un contrato.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TerminacionContratoPeticion {
    /// Dentro del contrato y no antes de su última versión.
    pub fecha: NaiveDate,
    #[schema(value_type = crate::esquemas::MotivoTerminacion)]
    pub motivo: MotivoTerminacion,
}

/// Termina un contrato con su motivo (la liquidación llega en la fase 4).
#[utoipa::path(
    post,
    path = "/contratos/{contrato_id}/terminacion",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("contrato_id" = Uuid, Path, description = "Contrato laboral")),
    request_body = TerminacionContratoPeticion,
    responses(
        (status = OK, description = "Contrato terminado", body = ContratoLaboralRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.empleados.gestionar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Contrato inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya terminó (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Fecha fuera del contrato o antes de su última versión, o motivo que no aplica al tipo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn terminar_contrato(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<TerminacionContratoPeticion>,
) -> ApiResult<Json<ContratoLaboralRespuesta>> {
    let c = uc::terminar_contrato(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ContratoLaboralId::from_uuid(ruta.1),
        p.fecha,
        p.motivo,
    )
    .await?;
    Ok(Json(c.into()))
}

// ---------------------------------------------------------------------------------------------
// Importación masiva
// ---------------------------------------------------------------------------------------------

/// Fila válida de la vista previa (sin datos personales).
#[derive(Debug, Serialize, ToSchema)]
pub struct FilaEmpleadoPreviaRespuesta {
    pub fila: u32,
    pub codigo: String,
    #[schema(value_type = crate::esquemas::TipoContratoLaboral)]
    pub tipo_contrato: TipoContratoLaboral,
}

impl From<FilaEmpleadoPrevia> for FilaEmpleadoPreviaRespuesta {
    fn from(f: FilaEmpleadoPrevia) -> Self {
        Self {
            fila: f.fila,
            codigo: f.codigo,
            tipo_contrato: f.tipo_contrato,
        }
    }
}

/// Importación de empleados con su vista previa.
#[derive(Debug, Serialize, ToSchema)]
pub struct VistaPreviaEmpleadosRespuesta {
    pub importacion: ImportacionRespuesta,
    pub filas: Vec<FilaEmpleadoPreviaRespuesta>,
}

impl From<VistaPreviaEmpleados> for VistaPreviaEmpleadosRespuesta {
    fn from(v: VistaPreviaEmpleados) -> Self {
        Self {
            importacion: v.importacion.into(),
            filas: v.filas.into_iter().map(Into::into).collect(),
        }
    }
}

/// Sube y valida una planilla de empleados (CSV o XLSX; columnas en la plantilla). No crea
/// nada: devuelve la vista previa con los errores por fila; se aplica con
/// `…/nomina/importaciones/{id}/confirmar`.
#[utoipa::path(
    post,
    path = "/importaciones",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), DatosPlanilla),
    request_body(content = Vec<u8>, content_type = "application/octet-stream",
                 description = "Contenido del CSV o XLSX"),
    responses(
        (status = CREATED, description = "Importación registrada (revise `estado` y `errores`)", body = VistaPreviaEmpleadosRespuesta),
        (status = FORBIDDEN, description = "Sin `importaciones.ejecutar` o `nomina.empleados.gestionar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es CSV/XLSX o no se puede leer (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn importar(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(d): Query<DatosPlanilla>,
    cuerpo: Bytes,
) -> ApiResult<(StatusCode, Json<VistaPreviaEmpleadosRespuesta>)> {
    let vista = uc_importacion::importar_empleados(
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

/// Plantilla CSV de empleados (encabezados y una fila de ejemplo, separada por `;`).
#[utoipa::path(
    get,
    path = "/importaciones/plantilla",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Plantilla CSV", content_type = "text/csv", body = String),
        (status = FORBIDDEN, description = "No exige permisos: basta un rol en la entidad", body = ErrorRespuesta),
    )
)]
pub async fn plantilla(_en: EnEntidad) -> Response {
    let ejemplo = [
        "E-001",
        "Ana Pérez",
        "CC",
        "52123456",
        "ana@correo.co",
        "573001234567",
        "termino_fijo",
        "2026-02-01",
        "2027-01-31",
        "",
        "Todero",
        "1750905",
        "ordinario",
        "",
        "completa",
        "",
        "mensual",
        "EPS037",
        "230301",
        "230301",
        "CCF22",
        "14-23",
        "1",
        "no",
        "transferencia",
        "Bancolombia",
        "ahorros",
        "0012345678",
    ];
    let csv = format!(
        "{}\n{}\n",
        encabezados_empleados().join(";"),
        ejemplo.join(";")
    );
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/csv; charset=utf-8"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"empleados.csv\""),
            ),
        ],
        csv,
    )
        .into_response()
}

/// Crea los empleados de una importación validada en una sola transacción (si una fila choca,
/// no se crea ninguno y el mensaje dice la fila).
#[utoipa::path(
    post,
    path = "/importaciones/{importacion_id}/confirmar",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(
        ("entidad_id" = Uuid, Path, description = "Entidad"),
        ("importacion_id" = Uuid, Path, description = "Importación de empleados"),
    ),
    responses(
        (status = OK, description = "Aplicada", body = ImportacionRespuesta),
        (status = CONFLICT, description = "Ya aplicada, con errores, los datos cambiaron o una fila choca con lo existente (`conflicto`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe o no es de empleados", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `importaciones.ejecutar` o `nomina.empleados.gestionar` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn confirmar_importacion(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ImportacionRespuesta>> {
    let i = uc_importacion::confirmar_importacion_empleados(
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
