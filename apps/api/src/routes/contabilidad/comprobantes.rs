//! Periodos, comprobantes, documentos origen, saldos iniciales y cierre anual.

use application::{
    casos_uso::contabilidad::{comprobantes as uc, periodos as uc_periodos},
    puertos::{
        RelojSistema,
        contabilidad::{FiltroComprobantes, Origen},
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::NaiveDate;
use domain::{
    CentroCostoId, ComprobanteId, PeriodoContableId, PersonaId, ReaperturaPeriodoId,
    TipoComprobanteId,
    contabilidad::{EstadoComprobante, ImporteDocumento, PeriodoContable},
};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::*;
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_CONTABILIDAD,
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas de periodos y comprobantes.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_periodos, abrir_periodo))
        .routes(routes!(cambiar_estado_periodo))
        .routes(routes!(listar_reaperturas, solicitar_reapertura))
        .routes(routes!(decidir_reapertura))
        .routes(routes!(listar_comprobantes, crear_borrador))
        .routes(routes!(ver_comprobante, editar_borrador, eliminar_borrador))
        .routes(routes!(previsualizar))
        .routes(routes!(contabilizar))
        .routes(routes!(revertir))
        .routes(routes!(previsualizar_documento))
        .routes(routes!(saldos_iniciales))
        .routes(routes!(cierre_anual))
}

/// Año de los periodos.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroPeriodos {
    pub anio: Option<i32>,
}

/// Periodos contables (del año o todos).
#[utoipa::path(
    get,
    path = "/periodos",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroPeriodos),
    responses(
        (status = OK, description = "Periodos", body = Vec<PeriodoRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_periodos(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroPeriodos>,
) -> ApiResult<Json<Vec<PeriodoRespuesta>>> {
    let v = uc_periodos::periodos(&state.unidad, &RelojSistema, &en.ctx, f.anio).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Abre un periodo (si ya existe, lo devuelve como está).
#[utoipa::path(
    post,
    path = "/periodos",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = AbrirPeriodoPeticion,
    responses(
        (status = OK, description = "Periodo", body = PeriodoRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.periodos.cerrar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Año o mes inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn abrir_periodo(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<AbrirPeriodoPeticion>,
) -> ApiResult<Json<PeriodoRespuesta>> {
    let periodo = PeriodoContable::new(p.anio, p.mes).map_err(|e| ApiError::App(e.into()))?;
    Ok(Json(
        uc_periodos::abrir_periodo(&state.unidad, &RelojSistema, &en.ctx, periodo)
            .await?
            .into(),
    ))
}

/// Cambia el estado del periodo (revisión, cierre, o vuelta a abierto desde revisión).
#[utoipa::path(
    put,
    path = "/periodos/{periodo_id}/estado",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("periodo_id" = Uuid, Path, description = "Periodo")),
    request_body = EstadoPeriodoPeticion,
    responses(
        (status = OK, description = "Periodo", body = PeriodoRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.periodos.cerrar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Transición no permitida o reabrir un cerrado sin reapertura (`conflicto_estado`, `conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn cambiar_estado_periodo(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<EstadoPeriodoPeticion>,
) -> ApiResult<Json<PeriodoRespuesta>> {
    let periodo = ruta.1;
    let r = uc_periodos::cambiar_estado(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PeriodoContableId::from_uuid(periodo),
        p.estado,
    )
    .await?;
    Ok(Json(r.into()))
}

/// Reaperturas de un periodo.
#[utoipa::path(
    get,
    path = "/periodos/{periodo_id}/reaperturas",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("periodo_id" = Uuid, Path, description = "Periodo")),
    responses(
        (status = OK, description = "Reaperturas", body = Vec<ReaperturaRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_reaperturas(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<ReaperturaRespuesta>>> {
    let periodo = ruta.1;
    let v = uc_periodos::reaperturas(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PeriodoContableId::from_uuid(periodo),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Solicita reabrir un periodo cerrado (la aprueba otra cuenta).
#[utoipa::path(
    post,
    path = "/periodos/{periodo_id}/reaperturas",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("periodo_id" = Uuid, Path, description = "Periodo")),
    request_body = SolicitudReaperturaPeticion,
    responses(
        (status = CREATED, description = "Solicitud registrada", body = ReaperturaRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.periodos.reabrir` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está cerrado o ya hay una pendiente (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn solicitar_reapertura(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<SolicitudReaperturaPeticion>,
) -> ApiResult<(StatusCode, Json<ReaperturaRespuesta>)> {
    let periodo = ruta.1;
    let r = uc_periodos::solicitar_reapertura(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PeriodoContableId::from_uuid(periodo),
        &p.motivo,
        p.alcance.as_deref(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Aprueba o rechaza una reapertura (quien la solicitó no puede).
#[utoipa::path(
    put,
    path = "/reaperturas/{reapertura_id}",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("reapertura_id" = Uuid, Path, description = "Reapertura")),
    request_body = DecisionReaperturaPeticion,
    responses(
        (status = OK, description = "Decisión registrada", body = ReaperturaRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.periodos.aprobar_reapertura` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya se decidió o es su propia solicitud (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn decidir_reapertura(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<DecisionReaperturaPeticion>,
) -> ApiResult<Json<ReaperturaRespuesta>> {
    let id = ruta.1;
    let r = uc_periodos::decidir_reapertura(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReaperturaPeriodoId::from_uuid(id),
        p.aprobar,
    )
    .await?;
    Ok(Json(r.into()))
}

/// Filtro de comprobantes.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroComprobantesQuery {
    pub desde: Option<NaiveDate>,
    pub hasta: Option<NaiveDate>,
    pub tipo_comprobante_id: Option<Uuid>,
    #[param(value_type = Option<crate::esquemas::EstadoComprobante>)]
    pub estado: Option<EstadoComprobante>,
}

/// Comprobantes por fecha, más recientes primero (sin líneas; a lo sumo 500).
#[utoipa::path(
    get,
    path = "/comprobantes",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroComprobantesQuery),
    responses(
        (status = OK, description = "Comprobantes", body = Vec<ComprobanteRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_comprobantes(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroComprobantesQuery>,
) -> ApiResult<Json<Vec<ComprobanteRespuesta>>> {
    let filtro = FiltroComprobantes {
        desde: f.desde,
        hasta: f.hasta,
        tipo_comprobante_id: f.tipo_comprobante_id.map(TipoComprobanteId::from_uuid),
        estado: f.estado,
    };
    let v = uc::comprobantes(&state.unidad, &RelojSistema, &en.ctx, filtro).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

fn borrador(p: BorradorPeticion) -> ApiResult<uc::BorradorComprobante> {
    Ok(uc::BorradorComprobante {
        tipo: p.tipo,
        fecha: p.fecha,
        descripcion: p.descripcion,
        lineas: lineas(p.lineas)?,
    })
}

/// Guarda un comprobante manual en borrador (se valida al contabilizar).
#[utoipa::path(
    post,
    path = "/comprobantes",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = BorradorPeticion,
    responses(
        (status = CREATED, description = "Borrador guardado", body = ComprobanteRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.comprobantes.crear` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Tipo no manual, cuenta o centro inexistente (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_borrador(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<BorradorPeticion>,
) -> ApiResult<(StatusCode, Json<ComprobanteRespuesta>)> {
    let c = uc::crear_borrador(&state.unidad, &RelojSistema, &en.ctx, borrador(p)?).await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Comprobante con sus líneas.
#[utoipa::path(
    get,
    path = "/comprobantes/{comprobante_id}",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("comprobante_id" = Uuid, Path, description = "Comprobante")),
    responses(
        (status = OK, description = "Comprobante", body = ComprobanteRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver_comprobante(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ComprobanteRespuesta>> {
    let id = ruta.1;
    Ok(Json(
        uc::comprobante(
            &state.unidad,
            &RelojSistema,
            &en.ctx,
            ComprobanteId::from_uuid(id),
        )
        .await?
        .into(),
    ))
}

/// Reemplaza un borrador.
#[utoipa::path(
    put,
    path = "/comprobantes/{comprobante_id}",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("comprobante_id" = Uuid, Path, description = "Comprobante")),
    request_body = BorradorPeticion,
    responses(
        (status = OK, description = "Borrador guardado", body = ComprobanteRespuesta),
        (status = CONFLICT, description = "Ya está contabilizado: se revierte (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn editar_borrador(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<BorradorPeticion>,
) -> ApiResult<Json<ComprobanteRespuesta>> {
    let id = ruta.1;
    let c = uc::editar_borrador(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ComprobanteId::from_uuid(id),
        borrador(p)?,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Borra un borrador.
#[utoipa::path(
    delete,
    path = "/comprobantes/{comprobante_id}",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("comprobante_id" = Uuid, Path, description = "Comprobante")),
    responses(
        (status = NO_CONTENT, description = "Borrado"),
        (status = CONFLICT, description = "Ya está contabilizado (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn eliminar_borrador(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let id = ruta.1;
    uc::eliminar_borrador(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ComprobanteId::from_uuid(id),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Valida un comprobante sin guardarlo y devuelve el asiento.
#[utoipa::path(
    post,
    path = "/comprobantes/previsualizar",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = BorradorPeticion,
    responses(
        (status = OK, description = "Asiento válido", body = AsientoRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No cuadra o una cuenta no admite la línea (`regla_contable`, `validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn previsualizar(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<BorradorPeticion>,
) -> ApiResult<Json<AsientoRespuesta>> {
    Ok(Json(
        uc::previsualizar(&state.unidad, &RelojSistema, &en.ctx, borrador(p)?)
            .await?
            .into(),
    ))
}

/// Contabiliza un borrador (partida doble, cuentas y periodo abierto; asigna número).
#[utoipa::path(
    post,
    path = "/comprobantes/{comprobante_id}/contabilizar",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("comprobante_id" = Uuid, Path, description = "Comprobante")),
    responses(
        (status = OK, description = "Contabilizado", body = ComprobanteRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.comprobantes.contabilizar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya está contabilizado (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No cuadra, cuenta inválida o periodo no abierto (`regla_contable`, `validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn contabilizar(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ComprobanteRespuesta>> {
    let id = ruta.1;
    Ok(Json(
        uc::contabilizar(
            &state.unidad,
            &RelojSistema,
            &en.ctx,
            ComprobanteId::from_uuid(id),
        )
        .await?
        .into(),
    ))
}

/// Revierte un comprobante contabilizado con otro de caras invertidas.
#[utoipa::path(
    post,
    path = "/comprobantes/{comprobante_id}/revertir",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("comprobante_id" = Uuid, Path, description = "Comprobante")),
    request_body = RevertirPeticion,
    responses(
        (status = CREATED, description = "Reverso contabilizado", body = ComprobanteRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.comprobantes.revertir` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está contabilizado, ya se revirtió o es un reverso (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn revertir(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<RevertirPeticion>,
) -> ApiResult<(StatusCode, Json<ComprobanteRespuesta>)> {
    let id = ruta.1;
    let c = uc::revertir(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ComprobanteId::from_uuid(id),
        &p.motivo,
        p.fecha,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Muestra el asiento que produciría un documento origen con la regla vigente (sin guardarlo).
#[utoipa::path(
    post,
    path = "/documentos/previsualizar",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = DocumentoPeticion,
    responses(
        (status = OK, description = "Asiento", body = AsientoRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin regla vigente, concepto desconocido o no cuadra", body = ErrorRespuesta),
    )
)]
pub async fn previsualizar_documento(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<DocumentoPeticion>,
) -> ApiResult<Json<AsientoRespuesta>> {
    let doc = uc::DocumentoOrigen {
        evento: p.evento,
        origen: Origen {
            tipo: "previsualizacion".into(),
            id: Uuid::nil(),
        },
        fecha: p.fecha,
        descripcion: p.descripcion,
        importes: p
            .importes
            .into_iter()
            .map(|i| ImporteDocumento {
                concepto: i.concepto,
                monto: i.monto,
                persona: i.persona_id.map(PersonaId::from_uuid),
                centro_costo: i.centro_costo_id.map(CentroCostoId::from_uuid),
                descripcion: i.descripcion,
                cuenta: None,
                base: None,
            })
            .collect(),
    };
    Ok(Json(
        uc::previsualizar_documento(&state.unidad, &RelojSistema, &en.ctx, doc)
            .await?
            .into(),
    ))
}

/// Contabiliza los saldos iniciales en un comprobante de apertura.
#[utoipa::path(
    post,
    path = "/saldos-iniciales",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = SaldosInicialesPeticion,
    responses(
        (status = CREATED, description = "Apertura contabilizada", body = ComprobanteRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.comprobantes.contabilizar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No cuadra, cuenta inválida o periodo no abierto", body = ErrorRespuesta),
    )
)]
pub async fn saldos_iniciales(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<SaldosInicialesPeticion>,
) -> ApiResult<(StatusCode, Json<ComprobanteRespuesta>)> {
    let s = uc::SaldosIniciales {
        fecha: p.fecha,
        tipo: p.tipo.unwrap_or_else(|| "AP".into()),
        descripcion: p.descripcion.unwrap_or_else(|| "Saldos iniciales".into()),
        lineas: lineas(p.lineas)?,
    };
    let c = uc::registrar_saldos_iniciales(&state.unidad, &RelojSistema, &en.ctx, s).await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Cierre anual: lleva a cero las cuentas de resultado contra la cuenta de excedente o déficit.
#[utoipa::path(
    post,
    path = "/cierre-anual",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = CierreAnualPeticion,
    responses(
        (status = CREATED, description = "Cierre contabilizado", body = ComprobanteRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.comprobantes.contabilizar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Meses sin cerrar o ya hay un cierre vigente (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin cuenta de resultado o sin saldos que cerrar", body = ErrorRespuesta),
    )
)]
pub async fn cierre_anual(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<CierreAnualPeticion>,
) -> ApiResult<(StatusCode, Json<ComprobanteRespuesta>)> {
    let tipo = p.tipo.unwrap_or_else(|| "CI".into());
    let c = uc::cierre_anual(&state.unidad, &RelojSistema, &en.ctx, p.anio, &tipo).await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}
