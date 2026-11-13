//! Configuración contable de la entidad: configuración, PUC, plan de cuentas, centros de costo,
//! tipos de comprobante, reglas, rubros, notas, terceros y plantillas.

use application::{
    casos_uso::contabilidad::{
        configuracion as uc,
        plantillas::{self as uc_plantillas, DestinoPlantilla},
    },
    puertos::{RelojSistema, contabilidad::AmbitoPlantilla},
};
use axum::{extract::State, http::StatusCode};
use domain::{
    ArchivoId, CentroCostoId, CuentaContableId, Documento, OrganizacionId, PersonaId,
    PlantillaContableId, TipoComprobanteId,
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
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas de configuración.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(ver_configuracion, guardar_configuracion))
        .routes(routes!(catalogo_puc))
        .routes(routes!(listar_cuentas, crear_cuenta))
        .routes(routes!(editar_cuenta))
        .routes(routes!(listar_centros, crear_centro))
        .routes(routes!(editar_centro))
        .routes(routes!(listar_tipos, crear_tipo))
        .routes(routes!(editar_tipo))
        .routes(routes!(listar_reglas, crear_regla))
        .routes(routes!(ver_rubros, reemplazar_rubros))
        .routes(routes!(listar_notas, guardar_nota))
        .routes(routes!(listar_terceros, crear_tercero))
        .routes(routes!(guardar_datos_fiscales))
        .routes(routes!(plantillas_disponibles))
        .routes(routes!(aplicar_plantilla))
        .routes(routes!(guardar_como_plantilla))
}

/// Configuración contable vigente (`null` si aún no la tiene).
#[utoipa::path(
    get,
    path = "/configuracion",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Configuración", body = Option<ConfiguracionRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo `contabilidad`", body = ErrorRespuesta),
    )
)]
pub async fn ver_configuracion(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Option<ConfiguracionRespuesta>>> {
    Ok(Json(
        uc::configuracion(&state.unidad, &RelojSistema, &en.ctx)
            .await?
            .map(Into::into),
    ))
}

/// Guarda una versión nueva de la configuración contable.
#[utoipa::path(
    put,
    path = "/configuracion",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = ConfiguracionPeticion,
    responses(
        (status = OK, description = "Configuración guardada", body = ConfiguracionRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Cuenta de resultado inválida (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn guardar_configuracion(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<ConfiguracionPeticion>,
) -> ApiResult<Json<ConfiguracionRespuesta>> {
    let nueva = uc::NuevaConfiguracion {
        marco_tecnico: p.marco_tecnico,
        cuenta_resultado: p.cuenta_resultado.as_deref().map(codigo).transpose()?,
        soporte_archivo_id: p.soporte_archivo_id.map(ArchivoId::from_uuid),
        vigente_desde: p.vigente_desde,
    };
    Ok(Json(
        uc::guardar_configuracion(&state.unidad, &RelojSistema, &en.ctx, nueva)
            .await?
            .into(),
    ))
}

/// Filtro del catálogo PUC.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroPuc {
    /// Solo las cuentas cuyo código empieza así.
    #[param(example = "11")]
    pub prefijo: Option<String>,
}

/// Catálogo PUC de la plataforma (solo lectura).
#[utoipa::path(
    get,
    path = "/puc",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroPuc),
    responses(
        (status = OK, description = "Cuentas del PUC", body = Vec<CuentaPucDto>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn catalogo_puc(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroPuc>,
) -> ApiResult<Json<Vec<CuentaPucDto>>> {
    let cuentas =
        uc::catalogo_puc(&state.unidad, &RelojSistema, &en.ctx, f.prefijo.as_deref()).await?;
    Ok(Json(cuentas.into_iter().map(Into::into).collect()))
}

/// Plan de cuentas de la entidad, por código.
#[utoipa::path(
    get,
    path = "/cuentas",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Cuentas", body = Vec<CuentaRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_cuentas(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<CuentaRespuesta>>> {
    let cuentas = uc::cuentas(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(cuentas.into_iter().map(Into::into).collect()))
}

/// Crea una cuenta (su padre debe existir, salvo que sea una clase).
#[utoipa::path(
    post,
    path = "/cuentas",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevaCuentaPeticion,
    responses(
        (status = CREATED, description = "Cuenta creada", body = CuentaRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "El código ya existe (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Código inválido, falta el padre o naturaleza (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_cuenta(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevaCuentaPeticion>,
) -> ApiResult<(StatusCode, Json<CuentaRespuesta>)> {
    let nueva = uc::NuevaCuenta {
        codigo: codigo(&p.codigo)?,
        nombre: p.nombre,
        naturaleza: p.naturaleza,
        acepta_movimiento: p.acepta_movimiento,
        exige_tercero: p.exige_tercero,
        exige_centro_costo: p.exige_centro_costo,
    };
    let c = uc::crear_cuenta(&state.unidad, &RelojSistema, &en.ctx, nueva).await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Edita una cuenta (el código y la naturaleza no cambian).
#[utoipa::path(
    put,
    path = "/cuentas/{cuenta_id}",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("cuenta_id" = Uuid, Path, description = "Cuenta")),
    request_body = CambioCuentaPeticion,
    responses(
        (status = OK, description = "Cuenta editada", body = CuentaRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La cuenta no existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "Tiene movimientos y se le quitan (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn editar_cuenta(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<CambioCuentaPeticion>,
) -> ApiResult<Json<CuentaRespuesta>> {
    let cuenta = ruta.1;
    let cambio = uc::CambioCuenta {
        nombre: p.nombre,
        acepta_movimiento: p.acepta_movimiento,
        exige_tercero: p.exige_tercero,
        exige_centro_costo: p.exige_centro_costo,
        activa: p.activa,
    };
    let c = uc::editar_cuenta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CuentaContableId::from_uuid(cuenta),
        cambio,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Centros de costo.
#[utoipa::path(
    get,
    path = "/centros-costo",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Centros de costo", body = Vec<CentroCostoRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_centros(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<CentroCostoRespuesta>>> {
    let v = uc::centros_costo(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea un centro de costo.
#[utoipa::path(
    post,
    path = "/centros-costo",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = CentroCostoPeticion,
    responses(
        (status = CREATED, description = "Centro creado", body = CentroCostoRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "El código ya existe (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_centro(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<CentroCostoPeticion>,
) -> ApiResult<(StatusCode, Json<CentroCostoRespuesta>)> {
    let c = uc::guardar_centro_costo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        None,
        &p.codigo,
        &p.nombre,
        p.activo,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Edita un centro de costo (nombre y estado).
#[utoipa::path(
    put,
    path = "/centros-costo/{centro_id}",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("centro_id" = Uuid, Path, description = "Centro de costo")),
    request_body = CentroCostoPeticion,
    responses(
        (status = OK, description = "Centro editado", body = CentroCostoRespuesta),
        (status = NOT_FOUND, description = "El centro no existe", body = ErrorRespuesta),
    )
)]
pub async fn editar_centro(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<CentroCostoPeticion>,
) -> ApiResult<Json<CentroCostoRespuesta>> {
    let centro = ruta.1;
    let c = uc::guardar_centro_costo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        Some(CentroCostoId::from_uuid(centro)),
        &p.codigo,
        &p.nombre,
        p.activo,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Tipos de comprobante.
#[utoipa::path(
    get,
    path = "/tipos-comprobante",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Tipos de comprobante", body = Vec<TipoComprobanteRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_tipos(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<TipoComprobanteRespuesta>>> {
    let v = uc::tipos_comprobante(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

fn datos_tipo(p: TipoComprobantePeticion) -> uc::DatosTipoComprobante {
    uc::DatosTipoComprobante {
        codigo: p.codigo,
        nombre: p.nombre,
        manual: p.manual,
        activo: p.activo,
    }
}

/// Crea un tipo de comprobante (su consecutivo es por año).
#[utoipa::path(
    post,
    path = "/tipos-comprobante",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = TipoComprobantePeticion,
    responses(
        (status = CREATED, description = "Tipo creado", body = TipoComprobanteRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "El código ya existe (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Código inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_tipo(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<TipoComprobantePeticion>,
) -> ApiResult<(StatusCode, Json<TipoComprobanteRespuesta>)> {
    let t =
        uc::guardar_tipo_comprobante(&state.unidad, &RelojSistema, &en.ctx, None, datos_tipo(p))
            .await?;
    Ok((StatusCode::CREATED, Json(t.into())))
}

/// Edita un tipo de comprobante (nombre, uso manual y estado).
#[utoipa::path(
    put,
    path = "/tipos-comprobante/{tipo_id}",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("tipo_id" = Uuid, Path, description = "Tipo de comprobante")),
    request_body = TipoComprobantePeticion,
    responses(
        (status = OK, description = "Tipo editado", body = TipoComprobanteRespuesta),
        (status = NOT_FOUND, description = "El tipo no existe", body = ErrorRespuesta),
    )
)]
pub async fn editar_tipo(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<TipoComprobantePeticion>,
) -> ApiResult<Json<TipoComprobanteRespuesta>> {
    let tipo = ruta.1;
    let t = uc::guardar_tipo_comprobante(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        Some(TipoComprobanteId::from_uuid(tipo)),
        datos_tipo(p),
    )
    .await?;
    Ok(Json(t.into()))
}

/// Reglas de contabilización (última versión de cada evento).
#[utoipa::path(
    get,
    path = "/reglas",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Reglas", body = Vec<ReglaRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_reglas(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<ReglaRespuesta>>> {
    let v = uc::reglas(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea una versión nueva de la regla de un evento.
#[utoipa::path(
    post,
    path = "/reglas",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevaReglaPeticion,
    responses(
        (status = CREATED, description = "Versión creada", body = ReglaRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Cuenta, tipo o centro inexistente (`validacion`) o regla mal formada (`regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_regla(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevaReglaPeticion>,
) -> ApiResult<(StatusCode, Json<ReglaRespuesta>)> {
    let lineas = p
        .lineas
        .into_iter()
        .map(|l| {
            Ok(uc::NuevaLineaRegla {
                concepto: l.concepto,
                cara: l.cara,
                cuenta: codigo(&l.cuenta)?,
                tercero: l.tercero,
                centro: l.centro,
                centro_costo: l.centro_costo,
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let nueva = uc::NuevaRegla {
        evento: p.evento,
        tipo_comprobante: p.tipo_comprobante,
        descripcion: p.descripcion,
        vigente_desde: p.vigente_desde,
        lineas,
    };
    let r = uc::crear_regla(&state.unidad, &RelojSistema, &en.ctx, nueva).await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Rubros de los estados financieros.
#[utoipa::path(
    get,
    path = "/rubros",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Rubros", body = Vec<RubroDto>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn ver_rubros(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<RubroDto>>> {
    let v = uc::rubros(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Reemplaza los rubros de los estados financieros.
#[utoipa::path(
    put,
    path = "/rubros",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = Vec<RubroDto>,
    responses(
        (status = OK, description = "Rubros guardados", body = Vec<RubroDto>),
        (status = CONFLICT, description = "Se quita un rubro con notas (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Rubro mal formado (`regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn reemplazar_rubros(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<Vec<RubroDto>>,
) -> ApiResult<Json<Vec<RubroDto>>> {
    let v = uc::reemplazar_rubros(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        p.into_iter().map(Into::into).collect(),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Año de las notas.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroNotas {
    pub anio: i32,
}

/// Notas a los estados financieros del año (última versión de cada una).
#[utoipa::path(
    get,
    path = "/notas",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroNotas),
    responses(
        (status = OK, description = "Notas", body = Vec<NotaRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_notas(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroNotas>,
) -> ApiResult<Json<Vec<NotaRespuesta>>> {
    let v = uc::notas(&state.unidad, &RelojSistema, &en.ctx, f.anio).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Guarda una versión nueva de una nota.
#[utoipa::path(
    post,
    path = "/notas",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NotaPeticion,
    responses(
        (status = CREATED, description = "Nota guardada", body = NotaRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Falta título o texto, o el rubro no existe (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn guardar_nota(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NotaPeticion>,
) -> ApiResult<(StatusCode, Json<NotaRespuesta>)> {
    let nota = uc::NuevaNota {
        anio: p.anio,
        numero: p.numero,
        titulo: p.titulo,
        texto: p.texto,
        rubro: p.rubro,
    };
    let n = uc::guardar_nota(&state.unidad, &RelojSistema, &en.ctx, nota).await?;
    Ok((StatusCode::CREATED, Json(n.into())))
}

/// Búsqueda de terceros.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroTerceros {
    /// Fragmento del nombre o del documento.
    pub texto: Option<String>,
}

/// Terceros de la entidad (sus personas) con datos fiscales.
#[utoipa::path(
    get,
    path = "/terceros",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroTerceros, ParametrosPagina),
    responses(
        (status = OK, description = "Terceros", body = PaginaRespuesta<TerceroRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_terceros(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(f): Query<FiltroTerceros>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<TerceroRespuesta>>> {
    let pagina = uc::terceros(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        f.texto.as_deref(),
        p.validar()?,
    )
    .await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Crea un tercero (reutiliza la persona si el documento ya existe).
#[utoipa::path(
    post,
    path = "/terceros",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevoTerceroPeticion,
    responses(
        (status = CREATED, description = "Tercero creado o completado", body = TerceroRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.terceros.gestionar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Dato inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_tercero(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevoTerceroPeticion>,
) -> ApiResult<(StatusCode, Json<TerceroRespuesta>)> {
    let nuevo = uc::NuevoTercero {
        tipo: p.tipo,
        nombre: p.nombre,
        tipo_documento: p.tipo_documento,
        documento: Documento::new(&p.documento).map_err(|e| ApiError::App(e.into()))?,
        fiscal: p.fiscal.into(),
    };
    let t = uc::crear_tercero(&state.unidad, &RelojSistema, &en.ctx, nuevo).await?;
    Ok((StatusCode::CREATED, Json(t.into())))
}

/// Reemplaza los datos fiscales de una persona de la entidad.
#[utoipa::path(
    put,
    path = "/terceros/{persona_id}/datos-fiscales",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("persona_id" = Uuid, Path, description = "Persona")),
    request_body = DatosFiscalesDto,
    responses(
        (status = OK, description = "Datos guardados", body = DatosFiscalesDto),
        (status = NOT_FOUND, description = "La persona no existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Dato inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn guardar_datos_fiscales(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<DatosFiscalesDto>,
) -> ApiResult<Json<DatosFiscalesDto>> {
    let persona = ruta.1;
    let d = uc::guardar_datos_fiscales(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PersonaId::from_uuid(persona),
        p.into(),
    )
    .await?;
    Ok(Json(d.into()))
}

/// Plantillas que la cuenta puede aplicar (estándar y de sus firmas contables).
#[utoipa::path(
    get,
    path = "/plantillas",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Plantillas", body = Vec<PlantillaResumenRespuesta>),
        (status = FORBIDDEN, description = "Sin `contabilidad.configurar` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn plantillas_disponibles(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<PlantillaResumenRespuesta>>> {
    let v = uc::plantillas_disponibles(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Aplica una plantilla (o muestra qué haría con `previsualizar`).
#[utoipa::path(
    post,
    path = "/plantillas/aplicar",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = AplicarPlantillaPeticion,
    responses(
        (status = OK, description = "Resumen de lo aplicado", body = ResumenAplicacionRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La plantilla no existe, no está publicada o no es de su firma", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es coherente con el plan de la entidad (`regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn aplicar_plantilla(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<AplicarPlantillaPeticion>,
) -> ApiResult<Json<ResumenAplicacionRespuesta>> {
    let r = uc::aplicar_plantilla(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PlantillaContableId::from_uuid(p.plantilla_id),
        p.previsualizar,
    )
    .await?;
    Ok(Json(ResumenAplicacionRespuesta::de(r, p.previsualizar)))
}

/// Guarda la configuración de la entidad como plantilla en borrador (de su firma contable o,
/// con el permiso de plataforma, estándar).
#[utoipa::path(
    post,
    path = "/plantillas/guardar",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = GuardarPlantillaPeticion,
    responses(
        (status = CREATED, description = "Plantilla guardada en borrador", body = PlantillaRespuesta),
        (status = FORBIDDEN, description = "Sin `contabilidad.configurar`, sin el módulo o sin el permiso de plataforma", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No es dueño o gestor de la organización", body = ErrorRespuesta),
    )
)]
pub async fn guardar_como_plantilla(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<GuardarPlantillaPeticion>,
) -> ApiResult<(StatusCode, Json<PlantillaRespuesta>)> {
    let ambito = p.organizacion_id.map_or(AmbitoPlantilla::Plataforma, |o| {
        AmbitoPlantilla::Organizacion(OrganizacionId::from_uuid(o))
    });
    let ctx_plataforma = application::ContextoPlataforma::de_cuenta(
        en.sesion.cuenta_id,
        en.sesion.permisos_plataforma.clone(),
        en.ctx.request_id.clone(),
    );
    let destino = DestinoPlantilla {
        ambito,
        nombre: p.nombre,
        tipo_empresa: p.tipo_empresa,
    };
    let g = uc_plantillas::guardar_como_plantilla(
        &state.unidad,
        &state.plataforma,
        &RelojSistema,
        &en.ctx,
        &ctx_plataforma,
        destino,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(g.into())))
}
