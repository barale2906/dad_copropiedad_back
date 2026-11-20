//! Reportes de la nómina (plan 16 fases 5 y 7): aportes del mes por administradora (insumo de la
//! PILA) e ingresos y retenciones del año por empleado (insumo del certificado). Solo con lo
//! aprobado o pagado.

use application::{
    casos_uso::nomina::reportes::{
        self as uc, AportesEmpleado, IngresosEmpleado, ResumenAportes, TotalAdministradora,
    },
    puertos::RelojSistema,
};
use axum::extract::State;
use chrono::NaiveDate;
use domain::nomina::{ClaseRiesgo, reportes::GrupoCertificado};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Query},
    openapi::ETIQUETA_NOMINA,
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas de los reportes (se fusionan en `/entidades/{entidad_id}/nomina`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(aportes_del_mes))
        .routes(routes!(ingresos_y_retenciones))
}

/// Mes del resumen de aportes.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MesAportesParametros {
    #[param(example = 2026)]
    pub anio: i32,
    /// 1 a 12.
    #[param(example = 3)]
    pub mes: u32,
}

/// Aportes del mes de un empleado.
#[derive(Debug, Serialize, ToSchema)]
pub struct AportesEmpleadoRespuesta {
    pub empleado_id: Uuid,
    pub contrato_id: Uuid,
    pub codigo: String,
    pub documento: Option<String>,
    /// Tipo de cotizante de la PILA (`01` dependiente, `12` aprendiz lectivo, `19` aprendiz
    /// productivo).
    pub tipo_cotizante: String,
    pub dias: u32,
    #[schema(value_type = String)]
    pub ibc: Decimal,
    /// Salud del empleado y del empleador.
    #[schema(value_type = String)]
    pub salud: Decimal,
    /// Pensión del empleado y del empleador.
    #[schema(value_type = String)]
    pub pension: Decimal,
    #[schema(value_type = String)]
    pub fondo_solidaridad: Decimal,
    #[schema(value_type = String)]
    pub arl: Decimal,
    #[schema(value_type = String)]
    pub caja: Decimal,
    #[schema(value_type = String)]
    pub icbf: Decimal,
    #[schema(value_type = String)]
    pub sena: Decimal,
    pub eps_id: Option<Uuid>,
    pub afp_id: Option<Uuid>,
    pub arl_id: Option<Uuid>,
    pub ccf_id: Option<Uuid>,
    #[schema(value_type = Option<crate::esquemas::ClaseRiesgo>)]
    pub clase_riesgo: Option<ClaseRiesgo>,
}

impl From<AportesEmpleado> for AportesEmpleadoRespuesta {
    fn from(a: AportesEmpleado) -> Self {
        let id = |x: Option<domain::AdministradoraId>| x.map(|i| i.as_uuid());
        Self {
            empleado_id: a.empleado_id.as_uuid(),
            contrato_id: a.contrato_id.as_uuid(),
            codigo: a.codigo,
            documento: a.documento.map(|d| d.as_str().to_owned()),
            tipo_cotizante: a.tipo_cotizante.to_owned(),
            dias: a.dias,
            ibc: a.ibc,
            salud: a.salud,
            pension: a.pension,
            fondo_solidaridad: a.fondo_solidaridad,
            arl: a.arl,
            caja: a.caja,
            icbf: a.icbf,
            sena: a.sena,
            eps_id: id(a.eps_id),
            afp_id: id(a.afp_id),
            arl_id: id(a.arl_id),
            ccf_id: id(a.ccf_id),
            clase_riesgo: a.clase_riesgo,
        }
    }
}

/// Total del mes a una administradora.
#[derive(Debug, Serialize, ToSchema)]
pub struct TotalAdministradoraRespuesta {
    pub administradora_id: Uuid,
    #[schema(value_type = String)]
    pub valor: Decimal,
}

impl From<TotalAdministradora> for TotalAdministradoraRespuesta {
    fn from(t: TotalAdministradora) -> Self {
        Self {
            administradora_id: t.administradora_id.as_uuid(),
            valor: t.valor,
        }
    }
}

/// Aportes del mes.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResumenAportesRespuesta {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    pub empleados: Vec<AportesEmpleadoRespuesta>,
    /// Totales por EPS, fondo de pensiones (con el de solidaridad), ARL y caja.
    pub administradoras: Vec<TotalAdministradoraRespuesta>,
    #[schema(value_type = String)]
    pub icbf: Decimal,
    #[schema(value_type = String)]
    pub sena: Decimal,
}

impl From<ResumenAportes> for ResumenAportesRespuesta {
    fn from(r: ResumenAportes) -> Self {
        Self {
            desde: r.desde,
            hasta: r.hasta,
            empleados: r.empleados.into_iter().map(Into::into).collect(),
            administradoras: r.administradoras.into_iter().map(Into::into).collect(),
            icbf: r.icbf,
            sena: r.sena,
        }
    }
}

/// Aportes a la seguridad social y parafiscales del mes, por empleado y por administradora
/// (insumo para diligenciar la PILA en el operador).
#[utoipa::path(
    get,
    path = "/aportes",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), MesAportesParametros),
    responses(
        (status = OK, description = "Aportes del mes", body = ResumenAportesRespuesta),
        (status = FORBIDDEN, description = "Sin `nomina.salarios.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Mes inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn aportes_del_mes(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<MesAportesParametros>,
) -> ApiResult<Json<ResumenAportesRespuesta>> {
    let r = uc::resumen_aportes(&state.unidad, &RelojSistema, &en.ctx, q.anio, q.mes).await?;
    Ok(Json(r.into()))
}

/// Año del certificado.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AnioIngresosParametros {
    #[param(example = 2026)]
    pub anio: i32,
}

/// Total de un grupo del certificado.
#[derive(Debug, Serialize, ToSchema)]
pub struct TotalGrupoCertificadoRespuesta {
    #[schema(value_type = crate::esquemas::GrupoCertificado)]
    pub grupo: GrupoCertificado,
    #[schema(value_type = String)]
    pub valor: Decimal,
}

/// Ingresos y retenciones del año de un empleado.
#[derive(Debug, Serialize, ToSchema)]
pub struct IngresosEmpleadoRespuesta {
    pub empleado_id: Uuid,
    pub codigo: String,
    pub nombre: String,
    pub documento: Option<String>,
    pub totales: Vec<TotalGrupoCertificadoRespuesta>,
}

impl From<IngresosEmpleado> for IngresosEmpleadoRespuesta {
    fn from(i: IngresosEmpleado) -> Self {
        Self {
            empleado_id: i.empleado_id.as_uuid(),
            codigo: i.codigo,
            nombre: i.nombre,
            documento: i.documento.map(|d| d.as_str().to_owned()),
            totales: i
                .totales
                .into_iter()
                .map(|(grupo, valor)| TotalGrupoCertificadoRespuesta { grupo, valor })
                .collect(),
        }
    }
}

/// Ingresos y retenciones del año por empleado, en los grupos del certificado de ingresos y
/// retenciones por rentas de trabajo (insumo del formulario 220).
#[utoipa::path(
    get,
    path = "/ingresos-retenciones",
    tag = ETIQUETA_NOMINA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), AnioIngresosParametros),
    responses(
        (status = OK, description = "Totales por empleado", body = Vec<IngresosEmpleadoRespuesta>),
        (status = FORBIDDEN, description = "Sin `nomina.salarios.ver` (`sin_permiso`) o sin el módulo `nomina` (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Año inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn ingresos_y_retenciones(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<AnioIngresosParametros>,
) -> ApiResult<Json<Vec<IngresosEmpleadoRespuesta>>> {
    let v = uc::ingresos_y_retenciones(&state.unidad, &RelojSistema, &en.ctx, q.anio).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}
