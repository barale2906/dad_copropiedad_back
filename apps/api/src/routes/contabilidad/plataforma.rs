//! Plataforma (con segundo factor): alta de entidades sin vertical, carga del PUC y plantillas
//! contables estándar. Firmas contables (cuenta autenticada, dueña o gestora): sus plantillas.

use application::{
    casos_uso::{
        contabilidad::{
            cuentas_por_pagar::{self as uc_cxp, NuevoConceptoRetencion},
            plantillas as uc,
        },
        plataforma as uc_plataforma,
    },
    puertos::{
        RelojSistema,
        contabilidad::{AmbitoPlantilla, CuentaPuc},
        tesoreria::ValorUvt,
    },
};
use axum::{extract::State, http::StatusCode};
use domain::{CuentaId, Nit, OrganizacionId, PlantillaContableId, contabilidad::PlantillaContable};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::*;
use super::dto_tesoreria::{
    ConceptoRetencionRespuesta, NuevoConceptoRetencionPeticion, UvtPeticion, UvtRespuesta,
};
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path},
    openapi::{ETIQUETA_CONTABILIDAD, ETIQUETA_PLATAFORMA},
    sesion::{CuentaAutenticada, OperadorPlataforma},
    state::AppState,
};

/// Rutas de plataforma (se montan en `/api/v1/plataforma`).
pub fn rutas_plataforma() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(crear_entidad))
        .routes(routes!(cargar_puc))
        .routes(routes!(catalogo_retenciones, crear_concepto_retencion))
        .routes(routes!(guardar_uvt))
        .routes(routes!(listar_plantillas, crear_plantilla))
        .routes(routes!(ver_plantilla))
        .routes(routes!(actualizar_plantilla))
        .routes(routes!(estado_plantilla))
}

/// Rutas de una firma contable (se montan en `/api/v1/organizaciones`).
pub fn rutas_organizacion() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(plantillas_organizacion))
        .routes(routes!(estado_plantilla_organizacion))
}

fn contenido(json: serde_json::Value) -> ApiResult<PlantillaContable> {
    serde_json::from_value(json).map_err(|e| {
        ApiError::App(application::AppError::Validacion(format!(
            "contenido de plantilla inválido: {e}"
        )))
    })
}

/// Entidad sin vertical por crear.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaEntidadPeticion {
    #[schema(example = "Gimnasio Fuerza S.A.S.")]
    pub nombre: String,
    /// NIT con o sin dígito de verificación.
    #[schema(value_type = Option<String>, example = "900123456-8")]
    pub nit: Option<Nit>,
    /// Cuenta que será su primer administrador.
    pub administrador_cuenta_id: Uuid,
}

/// Entidad creada.
#[derive(Debug, Serialize, ToSchema)]
pub struct EntidadRespuesta {
    pub id: Uuid,
    pub nombre: String,
}

/// Da de alta una entidad sin vertical (cualquier empresa de la contabilidad base); sus módulos
/// se habilitan después, uno a uno.
#[utoipa::path(
    post,
    path = "/entidades",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevaEntidadPeticion,
    responses(
        (status = CREATED, description = "Entidad creada", body = EntidadRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.entidades.crear`", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La cuenta del administrador no existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "Otra entidad tiene ese NIT (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_entidad(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevaEntidadPeticion>,
) -> ApiResult<(StatusCode, Json<EntidadRespuesta>)> {
    let id = uc_plataforma::crear_entidad(
        &state.plataforma,
        &state.unidad,
        &RelojSistema,
        &op.ctx,
        &p.nombre,
        p.nit,
        CuentaId::from_uuid(p.administrador_cuenta_id),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(EntidadRespuesta {
            id: id.as_uuid(),
            nombre: p.nombre.trim().to_owned(),
        }),
    ))
}

/// Carga (inserta o actualiza) cuentas del catálogo PUC.
#[utoipa::path(
    put,
    path = "/puc",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = CargaPucPeticion,
    responses(
        (status = OK, description = "Cuentas cargadas", body = CargaPucRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar`", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Auxiliares, códigos repetidos o inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn cargar_puc(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<CargaPucPeticion>,
) -> ApiResult<Json<CargaPucRespuesta>> {
    let cuentas = p
        .cuentas
        .into_iter()
        .map(|c| {
            Ok(CuentaPuc {
                codigo: codigo(&c.codigo)?,
                nombre: c.nombre,
                naturaleza: c.naturaleza,
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let n = uc::cargar_puc(&state.plataforma, &op.ctx, cuentas).await?;
    Ok(Json(CargaPucRespuesta { cuentas: n }))
}

/// Catálogo de retenciones de la plataforma: todas las versiones de los conceptos y la UVT.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogoRetencionesRespuesta {
    pub conceptos: Vec<ConceptoRetencionRespuesta>,
    pub uvt: Vec<UvtRespuesta>,
}

/// Catálogo de retenciones (todas las versiones) y valores de la UVT.
#[utoipa::path(
    get,
    path = "/retenciones",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Catálogo", body = CatalogoRetencionesRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar`", body = ErrorRespuesta),
    )
)]
pub async fn catalogo_retenciones(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
) -> ApiResult<Json<CatalogoRetencionesRespuesta>> {
    let (conceptos, uvt) =
        uc_cxp::catalogo_retenciones_plataforma(&state.plataforma, &op.ctx).await?;
    Ok(Json(CatalogoRetencionesRespuesta {
        conceptos: conceptos.into_iter().map(Into::into).collect(),
        uvt: uvt.into_iter().map(Into::into).collect(),
    }))
}

/// Agrega una versión de un concepto de retención (la anterior del mismo código queda vigente
/// hasta el día antes de `vigente_desde`).
#[utoipa::path(
    post,
    path = "/retenciones",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevoConceptoRetencionPeticion,
    responses(
        (status = CREATED, description = "Versión creada", body = ConceptoRetencionRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar`", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya hay una versión con esa fecha (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Código, tarifa o municipio inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_concepto_retencion(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevoConceptoRetencionPeticion>,
) -> ApiResult<(StatusCode, Json<ConceptoRetencionRespuesta>)> {
    let c = uc_cxp::crear_concepto_retencion(
        &state.plataforma,
        &op.ctx,
        NuevoConceptoRetencion {
            codigo: p.codigo,
            impuesto: p.impuesto,
            nombre: p.nombre,
            base_minima_uvt: p.base_minima_uvt,
            tarifa: p.tarifa,
            cuenta_sugerida: p.cuenta_sugerida.as_deref().map(codigo).transpose()?,
            municipio_dane: p.municipio_dane,
            fuente: p.fuente,
            vigente_desde: p.vigente_desde,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Crea o corrige el valor de la UVT del año.
#[utoipa::path(
    put,
    path = "/uvt/{anio}",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("anio" = i32, Path, description = "Año")),
    request_body = UvtPeticion,
    responses(
        (status = OK, description = "UVT", body = UvtRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.normativa.gestionar`", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Año o valor inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn guardar_uvt(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(anio): Path<i32>,
    Json(p): Json<UvtPeticion>,
) -> ApiResult<Json<UvtRespuesta>> {
    let u = uc_cxp::guardar_uvt(
        &state.plataforma,
        &op.ctx,
        ValorUvt {
            anio,
            valor: p.valor,
            fuente: p.fuente,
        },
    )
    .await?;
    Ok(Json(u.into()))
}

/// Plantillas estándar de la plataforma (todas sus versiones y estados).
#[utoipa::path(
    get,
    path = "/plantillas-contables",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Plantillas", body = Vec<PlantillaResumenRespuesta>),
        (status = FORBIDDEN, description = "Sin `plataforma.plantillas_contables.publicar`", body = ErrorRespuesta),
    )
)]
pub async fn listar_plantillas(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
) -> ApiResult<Json<Vec<PlantillaResumenRespuesta>>> {
    let v = uc::plantillas(&state.plataforma, &op.ctx, AmbitoPlantilla::Plataforma).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea una plantilla estándar en borrador (p. ej. la de propiedad horizontal).
#[utoipa::path(
    post,
    path = "/plantillas-contables",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = NuevaPlantillaPeticion,
    responses(
        (status = CREATED, description = "Plantilla creada", body = PlantillaRespuesta),
        (status = FORBIDDEN, description = "Sin `plataforma.plantillas_contables.publicar`", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Contenido incoherente (`validacion`, `regla_contable`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_plantilla(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<NuevaPlantillaPeticion>,
) -> ApiResult<(StatusCode, Json<PlantillaRespuesta>)> {
    let g = uc::crear_plantilla(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        AmbitoPlantilla::Plataforma,
        &p.nombre,
        &p.tipo_empresa,
        contenido(p.contenido)?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(g.into())))
}

/// Plantilla con su contenido.
#[utoipa::path(
    get,
    path = "/plantillas-contables/{plantilla_id}",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("plantilla_id" = Uuid, Path, description = "Plantilla")),
    responses(
        (status = OK, description = "Plantilla", body = PlantillaRespuesta),
        (status = NOT_FOUND, description = "No existe o no la puede gestionar", body = ErrorRespuesta),
    )
)]
pub async fn ver_plantilla(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PlantillaRespuesta>> {
    Ok(Json(
        uc::plantilla(
            &state.plataforma,
            &op.ctx,
            PlantillaContableId::from_uuid(id),
        )
        .await?
        .into(),
    ))
}

/// Reemplaza el contenido de una plantilla en borrador.
#[utoipa::path(
    put,
    path = "/plantillas-contables/{plantilla_id}/contenido",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("plantilla_id" = Uuid, Path, description = "Plantilla")),
    request_body = ContenidoPlantillaPeticion,
    responses(
        (status = NO_CONTENT, description = "Contenido guardado"),
        (status = CONFLICT, description = "Ya está publicada: cree una versión nueva (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Contenido incoherente", body = ErrorRespuesta),
    )
)]
pub async fn actualizar_plantilla(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<ContenidoPlantillaPeticion>,
) -> ApiResult<StatusCode> {
    uc::actualizar_plantilla(
        &state.plataforma,
        &op.ctx,
        PlantillaContableId::from_uuid(id),
        contenido(p.contenido)?,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Publica o retira una plantilla estándar.
#[utoipa::path(
    put,
    path = "/plantillas-contables/{plantilla_id}/estado",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("plantilla_id" = Uuid, Path, description = "Plantilla")),
    request_body = EstadoPlantillaPeticion,
    responses(
        (status = NO_CONTENT, description = "Estado cambiado"),
        (status = CONFLICT, description = "Transición no permitida (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn estado_plantilla(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<EstadoPlantillaPeticion>,
) -> ApiResult<StatusCode> {
    uc::cambiar_estado_plantilla(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        PlantillaContableId::from_uuid(id),
        p.estado,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Plantillas de la firma contable (la cuenta debe ser dueña o gestora).
#[utoipa::path(
    get,
    path = "/{organizacion_id}/plantillas-contables",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("organizacion_id" = Uuid, Path, description = "Firma contable")),
    responses(
        (status = OK, description = "Plantillas", body = Vec<PlantillaResumenRespuesta>),
        (status = NOT_FOUND, description = "No es dueña o gestora de la organización", body = ErrorRespuesta),
    )
)]
pub async fn plantillas_organizacion(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
    Path(org): Path<Uuid>,
) -> ApiResult<Json<Vec<PlantillaResumenRespuesta>>> {
    let ambito = AmbitoPlantilla::Organizacion(OrganizacionId::from_uuid(org));
    let v = uc::plantillas(&state.plataforma, &cuenta.ctx, ambito).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Publica o retira una plantilla de la firma contable.
#[utoipa::path(
    put,
    path = "/{organizacion_id}/plantillas-contables/{plantilla_id}/estado",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(
        ("organizacion_id" = Uuid, Path, description = "Firma contable"),
        ("plantilla_id" = Uuid, Path, description = "Plantilla"),
    ),
    request_body = EstadoPlantillaPeticion,
    responses(
        (status = NO_CONTENT, description = "Estado cambiado"),
        (status = NOT_FOUND, description = "No existe o no la puede gestionar", body = ErrorRespuesta),
        (status = CONFLICT, description = "Transición no permitida (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn estado_plantilla_organizacion(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<EstadoPlantillaPeticion>,
) -> ApiResult<StatusCode> {
    let id = ruta.1;
    uc::cambiar_estado_plantilla(
        &state.plataforma,
        &RelojSistema,
        &cuenta.ctx,
        PlantillaContableId::from_uuid(id),
        p.estado,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
