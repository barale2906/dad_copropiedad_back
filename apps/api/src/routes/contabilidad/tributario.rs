//! Tributario de una entidad (plan 15): catálogo vigente, perfil tributario, resoluciones de
//! numeración, preparación de retención, IVA e ICA, exógena y beneficiarios finales. Más el
//! catálogo tributario de la plataforma (IVA, ICA y formatos de exógena).

use application::{
    casos_uso::contabilidad::{
        declaraciones::{self as uc_decl, NuevoBeneficiario},
        tributario::{
            self as uc, NuevaResolucion, NuevaTarifaIca, NuevaTarifaIva, NuevoFormatoExogena,
        },
    },
    puertos::RelojSistema,
};
use axum::{extract::State, http::StatusCode};
use chrono::NaiveDate;
use domain::{
    ArchivoId, BeneficiarioFinalId, FormatoExogenaId, PerfilTributarioId, PersonaId,
    ResolucionNumeracionId,
};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto_tributario::*;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::{ETIQUETA_PLATAFORMA, ETIQUETA_TRIBUTARIO},
    sesion::{EnEntidad, OperadorPlataforma},
    state::AppState,
};

/// Rutas tributarias de una entidad.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(catalogo))
        .routes(routes!(listar_perfiles, crear_perfil))
        .routes(routes!(confirmar_perfil))
        .routes(routes!(listar_resoluciones, crear_resolucion))
        .routes(routes!(activar_resolucion))
        .routes(routes!(preparar_retencion))
        .routes(routes!(preparar_iva))
        .routes(routes!(preparar_ica))
        .routes(routes!(listar_exogena, generar_exogena))
        .routes(routes!(listar_beneficiarios, crear_beneficiario))
        .routes(routes!(terminar_beneficiario))
}

/// Rutas del catálogo tributario de la plataforma (se montan en `/api/v1/plataforma`).
pub fn rutas_plataforma() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(catalogo_plataforma))
        .routes(routes!(crear_tarifa_iva))
        .routes(routes!(crear_tarifa_ica))
        .routes(routes!(crear_formato_exogena))
}

// ---------------------------------------------------------------------------------------------
// Catálogo
// ---------------------------------------------------------------------------------------------

/// Fecha de vigencia de un catálogo.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FechaParametros {
    /// Vigentes en esta fecha (sin ella, todas las versiones).
    pub fecha: Option<NaiveDate>,
}

/// Catálogo tributario de la plataforma (tarifas de IVA e ICA y formatos de exógena).
#[utoipa::path(
    get,
    path = "/tributario/catalogo",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FechaParametros),
    responses(
        (status = OK, description = "Catálogo", body = CatalogoTributarioRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn catalogo(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<FechaParametros>,
) -> ApiResult<Json<CatalogoTributarioRespuesta>> {
    let c = uc::catalogo_tributario(&state.unidad, &RelojSistema, &en.ctx, q.fecha).await?;
    Ok(Json(c.into()))
}

// ---------------------------------------------------------------------------------------------
// Perfil
// ---------------------------------------------------------------------------------------------

/// Versiones del perfil tributario (la más reciente primero).
#[utoipa::path(
    get,
    path = "/tributario/perfiles",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Versiones", body = Vec<PerfilTributarioRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_perfiles(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<PerfilTributarioRespuesta>>> {
    let v = uc::perfiles_tributarios(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea una versión del perfil en borrador (no aplica nada hasta que el contador la confirme).
#[utoipa::path(
    post,
    path = "/tributario/perfiles",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevoPerfilPeticion,
    responses(
        (status = CREATED, description = "Versión en borrador", body = PerfilTributarioRespuesta),
        (status = FORBIDDEN, description = "Sin `tributario.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Perfil incoherente con el RUT (`validacion`, `dato_invalido`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_perfil(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevoPerfilPeticion>,
) -> ApiResult<(StatusCode, Json<PerfilTributarioRespuesta>)> {
    let perfil = uc::crear_perfil_tributario(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        p.vigente_desde,
        p.parametros.into(),
        p.soporte_archivo_id.map(ArchivoId::from_uuid),
        p.observaciones.as_deref(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(perfil.into())))
}

/// Confirma una versión del perfil: desde su vigencia aplican IVA, retenciones y facturación.
/// Una versión confirmada no se modifica.
#[utoipa::path(
    post,
    path = "/tributario/perfiles/{perfil_id}/confirmacion",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("perfil_id" = Uuid, Path, description = "Versión del perfil")),
    responses(
        (status = OK, description = "Versión confirmada", body = PerfilTributarioRespuesta),
        (status = FORBIDDEN, description = "Sin `tributario.perfil.confirmar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya estaba confirmada (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn confirmar_perfil(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<PerfilTributarioRespuesta>> {
    let p = uc::confirmar_perfil_tributario(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PerfilTributarioId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(p.into()))
}

// ---------------------------------------------------------------------------------------------
// Resoluciones de numeración
// ---------------------------------------------------------------------------------------------

/// Resoluciones de numeración (sin la clave técnica).
#[utoipa::path(
    get,
    path = "/tributario/resoluciones",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Resoluciones", body = Vec<ResolucionRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_resoluciones(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<ResolucionRespuesta>>> {
    let v = uc::resoluciones(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Registra una resolución de numeración de la DIAN.
#[utoipa::path(
    post,
    path = "/tributario/resoluciones",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevaResolucionPeticion,
    responses(
        (status = CREATED, description = "Resolución", body = ResolucionRespuesta),
        (status = FORBIDDEN, description = "Sin `tributario.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya existe ese rango (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Rango, prefijo o vigencia inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_resolucion(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevaResolucionPeticion>,
) -> ApiResult<(StatusCode, Json<ResolucionRespuesta>)> {
    let r = uc::crear_resolucion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevaResolucion {
            tipo: p.tipo,
            prefijo: p.prefijo,
            desde: p.desde,
            hasta: p.hasta,
            numero_resolucion: p.numero_resolucion,
            fecha_resolucion: p.fecha_resolucion,
            vigente_desde: p.vigente_desde,
            vigente_hasta: p.vigente_hasta,
            clave_tecnica: p.clave_tecnica,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Activa o desactiva una resolución (solo una activa y vigente numera).
#[utoipa::path(
    put,
    path = "/tributario/resoluciones/{resolucion_id}/activa",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("resolucion_id" = Uuid, Path, description = "Resolución")),
    request_body = ActivaPeticion,
    responses(
        (status = NO_CONTENT, description = "Actualizada"),
        (status = FORBIDDEN, description = "Sin `tributario.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Resolución inexistente", body = ErrorRespuesta),
    )
)]
pub async fn activar_resolucion(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ActivaPeticion>,
) -> ApiResult<StatusCode> {
    uc::activar_resolucion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ResolucionNumeracionId::from_uuid(ruta.1),
        p.activa,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Preparación de declaraciones
// ---------------------------------------------------------------------------------------------

/// Periodo a preparar.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PeriodoParametros {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
}

/// Prepara la retención en la fuente del periodo con su conciliación contra el mayor (no se
/// presenta desde aquí).
#[utoipa::path(
    get,
    path = "/tributario/retencion",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), PeriodoParametros),
    responses(
        (status = OK, description = "Preparación", body = PreparacionRetencionRespuesta),
        (status = FORBIDDEN, description = "Sin `tributario.preparar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Periodo inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn preparar_retencion(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<PeriodoParametros>,
) -> ApiResult<Json<PreparacionRetencionRespuesta>> {
    let p = uc_decl::preparar_retencion(&state.unidad, &RelojSistema, &en.ctx, q.desde, q.hasta)
        .await?;
    Ok(Json(p.into()))
}

/// Prepara el IVA del periodo: generado y descontable por tarifa.
#[utoipa::path(
    get,
    path = "/tributario/iva",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), PeriodoParametros),
    responses(
        (status = OK, description = "Preparación", body = PreparacionIvaRespuesta),
        (status = FORBIDDEN, description = "Sin `tributario.preparar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Periodo inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn preparar_iva(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<PeriodoParametros>,
) -> ApiResult<Json<PreparacionIvaRespuesta>> {
    let p = uc_decl::preparar_iva(&state.unidad, &RelojSistema, &en.ctx, q.desde, q.hasta).await?;
    Ok(Json(p.into()))
}

/// Prepara el ICA de los municipios y actividades del perfil confirmado y la ReteICA.
#[utoipa::path(
    get,
    path = "/tributario/ica",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), PeriodoParametros),
    responses(
        (status = OK, description = "Preparación", body = PreparacionIcaRespuesta),
        (status = FORBIDDEN, description = "Sin `tributario.preparar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin perfil confirmado o periodo inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn preparar_ica(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<PeriodoParametros>,
) -> ApiResult<Json<PreparacionIcaRespuesta>> {
    let p = uc_decl::preparar_ica(&state.unidad, &RelojSistema, &en.ctx, q.desde, q.hasta).await?;
    Ok(Json(p.into()))
}

// ---------------------------------------------------------------------------------------------
// Exógena
// ---------------------------------------------------------------------------------------------

/// Año de las generaciones.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AnioParametros {
    pub anio: Option<i32>,
}

/// Generaciones de exógena (las más recientes primero).
#[utoipa::path(
    get,
    path = "/tributario/exogena",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), AnioParametros),
    responses(
        (status = OK, description = "Generaciones", body = Vec<GeneracionExogenaRespuesta>),
        (status = FORBIDDEN, description = "Sin `tributario.preparar` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_exogena(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<AnioParametros>,
) -> ApiResult<Json<Vec<GeneracionExogenaRespuesta>>> {
    let v = uc_decl::generaciones_exogena(&state.unidad, &RelojSistema, &en.ctx, q.anio).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Genera un formato de exógena con la versión de reglas del catálogo (cuantías menores en el
/// NIT 222222222) y guarda el archivo con su huella.
#[utoipa::path(
    post,
    path = "/tributario/exogena",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = GenerarExogenaPeticion,
    responses(
        (status = CREATED, description = "Generación", body = ExogenaGeneradaRespuesta),
        (status = FORBIDDEN, description = "Sin `tributario.preparar` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Formato inexistente", body = ErrorRespuesta),
    )
)]
pub async fn generar_exogena(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<GenerarExogenaPeticion>,
) -> ApiResult<(StatusCode, Json<ExogenaGeneradaRespuesta>)> {
    let g = uc_decl::generar_exogena(
        &state.unidad,
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        FormatoExogenaId::from_uuid(p.formato_id),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(g.into())))
}

// ---------------------------------------------------------------------------------------------
// RUB
// ---------------------------------------------------------------------------------------------

/// Beneficiarios finales (los vigentes primero).
#[utoipa::path(
    get,
    path = "/tributario/beneficiarios",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Beneficiarios", body = Vec<BeneficiarioRespuesta>),
        (status = FORBIDDEN, description = "Sin `tributario.preparar` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_beneficiarios(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<BeneficiarioRespuesta>>> {
    let v = uc_decl::beneficiarios_finales(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Registra un beneficiario final.
#[utoipa::path(
    post,
    path = "/tributario/beneficiarios",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevoBeneficiarioPeticion,
    responses(
        (status = CREATED, description = "Beneficiario", body = BeneficiarioRespuesta),
        (status = FORBIDDEN, description = "Sin `tributario.preparar` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Persona inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Participación que no corresponde al criterio (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_beneficiario(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevoBeneficiarioPeticion>,
) -> ApiResult<(StatusCode, Json<BeneficiarioRespuesta>)> {
    let b = uc_decl::registrar_beneficiario(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoBeneficiario {
            persona: PersonaId::from_uuid(p.persona_id),
            criterio: p.criterio,
            porcentaje: p.porcentaje,
            soporte_archivo_id: p.soporte_archivo_id.map(ArchivoId::from_uuid),
            vigente_desde: p.vigente_desde,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(b.into())))
}

/// Termina la vigencia de un beneficiario final.
#[utoipa::path(
    post,
    path = "/tributario/beneficiarios/{beneficiario_id}/terminacion",
    tag = ETIQUETA_TRIBUTARIO,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("beneficiario_id" = Uuid, Path, description = "Beneficiario")),
    request_body = TerminarPeticion,
    responses(
        (status = NO_CONTENT, description = "Vigencia terminada"),
        (status = FORBIDDEN, description = "Sin `tributario.preparar` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe o ya terminó", body = ErrorRespuesta),
    )
)]
pub async fn terminar_beneficiario(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<TerminarPeticion>,
) -> ApiResult<StatusCode> {
    uc_decl::terminar_beneficiario(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BeneficiarioFinalId::from_uuid(ruta.1),
        p.hasta,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Plataforma
// ---------------------------------------------------------------------------------------------

/// Catálogo tributario completo (todas las versiones).
#[utoipa::path(
    get,
    path = "/tributario/catalogo",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Catálogo", body = CatalogoTributarioRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar`", body = ErrorRespuesta),
    )
)]
pub async fn catalogo_plataforma(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
) -> ApiResult<Json<CatalogoTributarioRespuesta>> {
    let c = uc::catalogo_tributario_plataforma(&state.plataforma, &op.ctx).await?;
    Ok(Json(c.into()))
}

/// Agrega una versión de una tarifa de IVA (la anterior del mismo código queda vigente hasta el
/// día antes).
#[utoipa::path(
    post,
    path = "/tributario/tarifas-iva",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevaTarifaIvaPeticion,
    responses(
        (status = CREATED, description = "Versión creada", body = TarifaIvaRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar`", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya hay una versión con esa fecha (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Código, tratamiento o tarifa inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_tarifa_iva(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevaTarifaIvaPeticion>,
) -> ApiResult<(StatusCode, Json<TarifaIvaRespuesta>)> {
    let t = uc::crear_tarifa_iva(
        &state.plataforma,
        &op.ctx,
        NuevaTarifaIva {
            codigo: p.codigo,
            nombre: p.nombre,
            tratamiento: p.tratamiento,
            tarifa: p.tarifa,
            fuente: p.fuente,
            vigente_desde: p.vigente_desde,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(t.into())))
}

/// Agrega una versión de una tarifa de ICA por municipio y actividad.
#[utoipa::path(
    post,
    path = "/tributario/tarifas-ica",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevaTarifaIcaPeticion,
    responses(
        (status = CREATED, description = "Versión creada", body = TarifaIcaRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar`", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya hay una versión con esa fecha (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Municipio, actividad o tarifa inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_tarifa_ica(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevaTarifaIcaPeticion>,
) -> ApiResult<(StatusCode, Json<TarifaIcaRespuesta>)> {
    let t = uc::crear_tarifa_ica(
        &state.plataforma,
        &op.ctx,
        NuevaTarifaIca {
            municipio_dane: p.municipio_dane,
            actividad: p.actividad,
            nombre: p.nombre,
            tarifa_por_mil: p.tarifa_por_mil,
            fuente: p.fuente,
            vigente_desde: p.vigente_desde,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(t.into())))
}

/// Agrega un formato de exógena (versión siguiente del año y código) con su mapeo de cuentas.
#[utoipa::path(
    post,
    path = "/tributario/formatos-exogena",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevoFormatoExogenaPeticion,
    responses(
        (status = CREATED, description = "Formato creado", body = FormatoExogenaRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar`", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Código, concepto o cuentas inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_formato_exogena(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevoFormatoExogenaPeticion>,
) -> ApiResult<(StatusCode, Json<FormatoExogenaRespuesta>)> {
    let f = uc::crear_formato_exogena(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        NuevoFormatoExogena {
            anio: p.anio,
            codigo: p.codigo,
            nombre: p.nombre,
            resolucion: p.resolucion,
            cuantia_menor: p.cuantia_menor,
            campos: p.campos.into_iter().map(Into::into).collect(),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(f.into())))
}
