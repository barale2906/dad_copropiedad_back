//! Configuración de la cartera y presupuesto (plan 14 fases 1 y 2): módulos de contribución,
//! conceptos de cobro, políticas, datos de pago de los bienes y presupuestos.

use application::{
    casos_uso::contabilidad_ph::{
        configuracion::{self as cfg, CambioConcepto, NuevoConcepto},
        presupuesto::{self as pres, NuevoRubro},
    },
    puertos::RelojSistema,
};
use axum::{extract::State, http::StatusCode};
use domain::{
    BienPrivadoId, CentroCostoId, ConceptoCobroId, CuentaContableId, ModuloContribucionId,
    PersonaId, PreguntaId, PresupuestoId, contabilidad_ph::ParametrosPolitica,
};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::*;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_CARTERA,
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de configuración y presupuesto.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_modulos, crear_modulo))
        .routes(routes!(actualizar_modulo))
        .routes(routes!(listar_conceptos, crear_concepto))
        .routes(routes!(actualizar_concepto))
        .routes(routes!(listar_politicas, crear_politica))
        .routes(routes!(listar_bienes))
        .routes(routes!(guardar_datos_pago))
        .routes(routes!(listar_presupuestos, crear_presupuesto))
        .routes(routes!(ver_presupuesto))
        .routes(routes!(reemplazar_rubros))
        .routes(routes!(aprobar_presupuesto))
        .routes(routes!(ejecucion_presupuestal))
}

fn tarifas(v: Vec<TarifaDto>) -> Vec<(domain::TipoBienPrivado, domain::contabilidad::Monto)> {
    v.into_iter().map(|t| (t.tipo_bien, t.valor)).collect()
}

fn rubros(v: Vec<RubroPeticion>) -> Vec<NuevoRubro> {
    v.into_iter()
        .map(|r| NuevoRubro {
            concepto: ConceptoCobroId::from_uuid(r.concepto_cobro_id),
            cuenta: CuentaContableId::from_uuid(r.cuenta_contable_id),
            centro_costo: r.centro_costo_id.map(CentroCostoId::from_uuid),
            nombre: r.nombre,
            valores_mensuales: r.valores_mensuales,
        })
        .collect()
}

/// Módulos de contribución por nombre.
#[utoipa::path(
    get,
    path = "/cartera/modulos-contribucion",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Módulos", body = Vec<ModuloContribucionRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn listar_modulos(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<ModuloContribucionRespuesta>>> {
    let v = cfg::modulos_contribucion(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea un módulo de contribución (conjuntos mixtos, art. 29). Sus coeficientes se cargan como
/// un cuadro de coeficientes con `modulo_id`.
#[utoipa::path(
    post,
    path = "/cartera/modulos-contribucion",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = ModuloContribucionPeticion,
    responses(
        (status = CREATED, description = "Módulo creado", body = ModuloContribucionRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.configurar` o sin los módulos", body = ErrorRespuesta),
        (status = CONFLICT, description = "El nombre ya existe (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Nombre inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_modulo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<ModuloContribucionPeticion>,
) -> ApiResult<(StatusCode, Json<ModuloContribucionRespuesta>)> {
    let m = cfg::guardar_modulo_contribucion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        None,
        &p.nombre,
        p.descripcion.as_deref(),
        p.activo,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(m.into())))
}

/// Actualiza un módulo de contribución.
#[utoipa::path(
    put,
    path = "/cartera/modulos-contribucion/{modulo_id}",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("modulo_id" = Uuid, Path, description = "Módulo")),
    request_body = ModuloContribucionPeticion,
    responses(
        (status = OK, description = "Módulo actualizado", body = ModuloContribucionRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.configurar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Módulo inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "El nombre ya existe (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn actualizar_modulo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ModuloContribucionPeticion>,
) -> ApiResult<Json<ModuloContribucionRespuesta>> {
    let m = cfg::guardar_modulo_contribucion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        Some(ModuloContribucionId::from_uuid(ruta.1)),
        &p.nombre,
        p.descripcion.as_deref(),
        p.activo,
    )
    .await?;
    Ok(Json(m.into()))
}

/// Conceptos de cobro por código.
#[utoipa::path(
    get,
    path = "/cartera/conceptos",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Conceptos", body = Vec<ConceptoCobroRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn listar_conceptos(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<ConceptoCobroRespuesta>>> {
    let v = cfg::conceptos_cobro(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea un concepto de cobro (su `concepto_contable` es el de las reglas `ph.*`).
#[utoipa::path(
    post,
    path = "/cartera/conceptos",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoConceptoPeticion,
    responses(
        (status = CREATED, description = "Concepto creado", body = ConceptoCobroRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.configurar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Módulo inexistente o inactivo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Código repetido o ya hay un fondo activo (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Incoherente con su criterio o tipo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_concepto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoConceptoPeticion>,
) -> ApiResult<(StatusCode, Json<ConceptoCobroRespuesta>)> {
    let c = cfg::crear_concepto_cobro(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoConcepto {
            codigo: p.codigo,
            nombre: p.nombre,
            tipo: p.tipo,
            criterio: p.criterio,
            modulo: p.modulo_id.map(ModuloContribucionId::from_uuid),
            valor_fijo: p.valor_fijo,
            tarifas: tarifas(p.tarifas),
            genera_interes: p.genera_interes,
            concepto_contable: p.concepto_contable,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Actualiza un concepto de cobro (código, tipo y criterio no cambian).
#[utoipa::path(
    put,
    path = "/cartera/conceptos/{concepto_id}",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("concepto_id" = Uuid, Path, description = "Concepto")),
    request_body = CambioConceptoPeticion,
    responses(
        (status = OK, description = "Concepto actualizado", body = ConceptoCobroRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.configurar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Concepto inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Incoherente con su criterio o tipo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn actualizar_concepto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<CambioConceptoPeticion>,
) -> ApiResult<Json<ConceptoCobroRespuesta>> {
    let c = cfg::actualizar_concepto_cobro(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ConceptoCobroId::from_uuid(ruta.1),
        CambioConcepto {
            nombre: p.nombre,
            valor_fijo: p.valor_fijo,
            tarifas: tarifas(p.tarifas),
            genera_interes: p.genera_interes,
            concepto_contable: p.concepto_contable,
            activo: p.activo,
        },
    )
    .await?;
    Ok(Json(c.into()))
}

/// Versiones de la política de cartera, la más reciente primero.
#[utoipa::path(
    get,
    path = "/cartera/politicas",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Versiones", body = Vec<PoliticaCarteraRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn listar_politicas(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<PoliticaCarteraRespuesta>>> {
    let v = cfg::politicas_cartera(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea una versión de la política de cartera (vencimiento, redondeo, pronto pago, mora,
/// imputación y fondo de imprevistos).
#[utoipa::path(
    post,
    path = "/cartera/politicas",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevaPoliticaPeticion,
    responses(
        (status = CREATED, description = "Versión creada", body = PoliticaCarteraRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.configurar` o sin los módulos", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya hay una versión con esa fecha (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Incumple un tope legal (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_politica(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevaPoliticaPeticion>,
) -> ApiResult<(StatusCode, Json<PoliticaCarteraRespuesta>)> {
    let pronto_pago = p.pronto_pago_porcentaje.zip(p.pronto_pago_dia);
    let politica = cfg::crear_politica_cartera(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        p.vigente_desde,
        ParametrosPolitica {
            dia_vencimiento: p.dia_vencimiento,
            redondeo: p.redondeo,
            pronto_pago,
            factor_mora: p.factor_mora,
            tasa_mora_mensual: p.tasa_mora_mensual,
            imputacion: p.imputacion,
            permite_elegir_obligacion: p.permite_elegir_obligacion,
            porcentaje_fondo: p.porcentaje_fondo,
            fondo_suspendido: p.fondo_suspendido,
        },
        p.pregunta_id.map(PreguntaId::from_uuid),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(politica.into())))
}

/// Bienes privados con su referencia de pago, pagador y deudor.
#[utoipa::path(
    get,
    path = "/cartera/bienes",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Bienes", body = Vec<BienCarteraRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn listar_bienes(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<BienCarteraRespuesta>>> {
    let v = cfg::bienes_cartera(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Fija la referencia de pago y el pagador (tenedor) de un bien.
#[utoipa::path(
    put,
    path = "/cartera/bienes/{bien_privado_id}/datos-pago",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("bien_privado_id" = Uuid, Path, description = "Bien privado")),
    request_body = DatosPagoPeticion,
    responses(
        (status = OK, description = "Bien actualizado", body = BienCarteraRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.configurar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Bien o persona inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Otro bien usa la referencia (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Referencia no numérica (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn guardar_datos_pago(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<DatosPagoPeticion>,
) -> ApiResult<Json<BienCarteraRespuesta>> {
    let b = cfg::guardar_datos_pago(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
        p.referencia_pago.as_deref(),
        p.pagador_persona_id.map(PersonaId::from_uuid),
    )
    .await?;
    Ok(Json(b.into()))
}

/// Filtro de presupuestos.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroPresupuestosParametros {
    pub anio: Option<i32>,
}

/// Presupuestos (del año, si se da), del más reciente al más antiguo.
#[utoipa::path(
    get,
    path = "/cartera/presupuestos",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroPresupuestosParametros),
    responses(
        (status = OK, description = "Presupuestos", body = Vec<PresupuestoRespuesta>),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
    )
)]
pub async fn listar_presupuestos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroPresupuestosParametros>,
) -> ApiResult<Json<Vec<PresupuestoRespuesta>>> {
    let v = pres::presupuestos(&state.unidad, &RelojSistema, &en.ctx, f.anio).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea un presupuesto en borrador (siguiente versión del año).
#[utoipa::path(
    post,
    path = "/cartera/presupuestos",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoPresupuestoPeticion,
    responses(
        (status = CREATED, description = "Borrador", body = PresupuestoRespuesta),
        (status = FORBIDDEN, description = "Sin `presupuesto.gestionar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Concepto o cuenta inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Rubro inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_presupuesto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoPresupuestoPeticion>,
) -> ApiResult<(StatusCode, Json<PresupuestoRespuesta>)> {
    let v = pres::crear_presupuesto(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        p.anio,
        p.descripcion.as_deref(),
        rubros(p.rubros),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(v.into())))
}

/// Presupuesto con sus rubros.
#[utoipa::path(
    get,
    path = "/cartera/presupuestos/{presupuesto_id}",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("presupuesto_id" = Uuid, Path, description = "Presupuesto")),
    responses(
        (status = OK, description = "Presupuesto", body = PresupuestoRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Presupuesto inexistente", body = ErrorRespuesta),
    )
)]
pub async fn ver_presupuesto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<PresupuestoRespuesta>> {
    let p = pres::presupuesto(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PresupuestoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(p.into()))
}

/// Reemplaza los rubros de un borrador.
#[utoipa::path(
    put,
    path = "/cartera/presupuestos/{presupuesto_id}/rubros",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("presupuesto_id" = Uuid, Path, description = "Presupuesto")),
    request_body = RubrosPeticion,
    responses(
        (status = OK, description = "Borrador actualizado", body = PresupuestoRespuesta),
        (status = FORBIDDEN, description = "Sin `presupuesto.gestionar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Presupuesto, concepto o cuenta inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya fue aprobado (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Rubro inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn reemplazar_rubros(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<RubrosPeticion>,
) -> ApiResult<Json<PresupuestoRespuesta>> {
    let v = pres::reemplazar_rubros(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PresupuestoId::from_uuid(ruta.1),
        rubros(p.rubros),
    )
    .await?;
    Ok(Json(v.into()))
}

/// Aprueba el borrador (asamblea): pasa a vigente y reemplaza la versión vigente del año.
#[utoipa::path(
    post,
    path = "/cartera/presupuestos/{presupuesto_id}/aprobacion",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("presupuesto_id" = Uuid, Path, description = "Presupuesto")),
    request_body = AprobacionPresupuestoPeticion,
    responses(
        (status = OK, description = "Presupuesto vigente", body = PresupuestoRespuesta),
        (status = FORBIDDEN, description = "Sin `presupuesto.gestionar` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Presupuesto inexistente", body = ErrorRespuesta),
        (status = CONFLICT, description = "No es un borrador (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin rubros o fecha de otro año (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn aprobar_presupuesto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AprobacionPresupuestoPeticion>,
) -> ApiResult<Json<PresupuestoRespuesta>> {
    let v = pres::aprobar_presupuesto(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PresupuestoId::from_uuid(ruta.1),
        p.vigente_desde,
        p.pregunta_id.map(PreguntaId::from_uuid),
    )
    .await?;
    Ok(Json(v.into()))
}

/// Mes de corte de la ejecución.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EjecucionParametros {
    /// Mes del corte (1 a 12).
    pub hasta_mes: u32,
}

/// Ejecución presupuestal: lo presupuestado contra el mayor, de enero al mes del corte.
#[utoipa::path(
    get,
    path = "/cartera/presupuestos/{presupuesto_id}/ejecucion",
    tag = ETIQUETA_CARTERA,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("presupuesto_id" = Uuid, Path, description = "Presupuesto"), EjecucionParametros),
    responses(
        (status = OK, description = "Ejecución", body = EjecucionPresupuestalRespuesta),
        (status = FORBIDDEN, description = "Sin `cartera.ver` o sin los módulos", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Presupuesto inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Mes inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn ejecucion_presupuestal(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(q): Query<EjecucionParametros>,
) -> ApiResult<Json<EjecucionPresupuestalRespuesta>> {
    let e = pres::ejecucion_presupuestal(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PresupuestoId::from_uuid(ruta.1),
        q.hasta_mes,
    )
    .await?;
    Ok(Json(e.into()))
}
