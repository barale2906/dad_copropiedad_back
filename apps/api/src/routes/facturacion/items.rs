//! Catálogo de productos y servicios: `/entidades/{entidad_id}/facturacion/items`.

use application::{
    casos_uso::facturacion::items::{self as uc, DatosItem},
    puertos::{
        RelojSistema,
        facturacion::{FiltroItems, ItemFacturacion},
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{ItemFacturacionId, contabilidad::Monto, tributario::ClasificacionTributaria};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::routes::contabilidad::dto::codigo;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_FACTURACION,
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas del catálogo.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_items, crear_item))
        .routes(routes!(ver_item, actualizar_item))
}

/// Datos editables de un producto o servicio.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DatosItemPeticion {
    #[schema(example = "Asesoría por hora")]
    pub descripcion: String,
    /// Código de unidad de la DIAN (`94` = unidad, `HUR` = hora); `94` si no viene.
    #[schema(example = "HUR")]
    pub unidad: Option<String>,
    /// Precio sugerido.
    #[schema(value_type = Option<String>, example = "80000")]
    pub precio: Option<Monto>,
    /// Porcentaje de IVA (0 si no aplica); solo `venta` o `alquiler_servicio` llevan IVA.
    #[serde(default)]
    #[schema(value_type = String, example = "0")]
    pub tarifa_iva: Decimal,
    #[schema(value_type = crate::esquemas::ClasificacionTributaria)]
    pub clasificacion: ClasificacionTributaria,
    /// Código de la cuenta de ingreso, solo si la entidad lleva contabilidad.
    pub cuenta: Option<String>,
    #[serde(default = "verdadero")]
    pub activo: bool,
}

fn verdadero() -> bool {
    true
}

/// Producto o servicio nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoItemPeticion {
    /// Único en la entidad: 1 a 30 caracteres `A-Z a-z 0-9 . _ -`.
    #[schema(example = "ASE-H")]
    pub codigo: String,
    #[serde(flatten)]
    pub datos: DatosItemPeticion,
}

fn datos(p: DatosItemPeticion) -> ApiResult<DatosItem> {
    Ok(DatosItem {
        descripcion: p.descripcion,
        unidad: p.unidad,
        precio: p.precio,
        tarifa_iva: p.tarifa_iva,
        clasificacion: p.clasificacion,
        cuenta: p.cuenta.as_deref().map(codigo).transpose()?,
        activo: p.activo,
    })
}

/// Producto o servicio del catálogo.
#[derive(Debug, Serialize, ToSchema)]
pub struct ItemRespuesta {
    pub id: Uuid,
    pub codigo: String,
    pub descripcion: String,
    pub unidad: String,
    #[schema(value_type = Option<String>)]
    pub precio: Option<Decimal>,
    #[schema(value_type = String)]
    pub tarifa_iva: Decimal,
    #[schema(value_type = crate::esquemas::ClasificacionTributaria)]
    pub clasificacion: ClasificacionTributaria,
    /// Cuenta de ingreso (solo con contabilidad).
    pub cuenta_id: Option<Uuid>,
    pub activo: bool,
    pub actualizado_en: DateTime<Utc>,
}

impl From<ItemFacturacion> for ItemRespuesta {
    fn from(i: ItemFacturacion) -> Self {
        Self {
            id: i.id.as_uuid(),
            codigo: i.codigo,
            descripcion: i.descripcion,
            unidad: i.unidad,
            precio: i.precio.map(Monto::valor),
            tarifa_iva: i.tarifa_iva,
            clasificacion: i.clasificacion,
            cuenta_id: i.cuenta_id.map(|c| c.as_uuid()),
            activo: i.activo,
            actualizado_en: i.actualizado_en,
        }
    }
}

/// Filtro del catálogo.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroItemsParametros {
    /// Busca en el código y la descripción, sin tildes ni mayúsculas.
    pub texto: Option<String>,
    /// Incluir los inactivos.
    #[serde(default)]
    pub con_inactivos: bool,
}

/// Catálogo por descripción (a lo sumo 500).
#[utoipa::path(
    get,
    path = "/items",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), FiltroItemsParametros),
    responses(
        (status = OK, description = "Ítems", body = Vec<ItemRespuesta>),
        (status = FORBIDDEN, description = "Sin `facturacion.ver` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn listar_items(
    State(state): State<AppState>,
    en: EnEntidad,
    Query(q): Query<FiltroItemsParametros>,
) -> ApiResult<Json<Vec<ItemRespuesta>>> {
    let v = uc::items(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FiltroItems {
            texto: q.texto,
            con_inactivos: q.con_inactivos,
        },
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea un producto o servicio.
#[utoipa::path(
    post,
    path = "/items",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = NuevoItemPeticion,
    responses(
        (status = CREATED, description = "Ítem creado", body = ItemRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.items.gestionar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "El código ya existe (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Dato inválido o cuenta sin contabilidad (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_item(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<NuevoItemPeticion>,
) -> ApiResult<(StatusCode, Json<ItemRespuesta>)> {
    let i = uc::crear_item(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        &p.codigo,
        datos(p.datos)?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(i.into())))
}

/// Producto o servicio por id.
#[utoipa::path(
    get,
    path = "/items/{item_id}",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("item_id" = Uuid, Path, description = "Ítem")),
    responses(
        (status = OK, description = "Ítem", body = ItemRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.ver` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Ítem inexistente", body = ErrorRespuesta),
    )
)]
pub async fn ver_item(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ItemRespuesta>> {
    let i = uc::item(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ItemFacturacionId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(i.into()))
}

/// Reemplaza los datos de un ítem (el código no cambia). Desactivarlo lo saca de las facturas
/// nuevas.
#[utoipa::path(
    put,
    path = "/items/{item_id}",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad"), ("item_id" = Uuid, Path, description = "Ítem")),
    request_body = DatosItemPeticion,
    responses(
        (status = OK, description = "Ítem actualizado", body = ItemRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.items.gestionar` o sin el módulo", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Ítem inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Dato inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn actualizar_item(
    State(state): State<AppState>,
    en: EnEntidad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<DatosItemPeticion>,
) -> ApiResult<Json<ItemRespuesta>> {
    let i = uc::actualizar_item(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ItemFacturacionId::from_uuid(ruta.1),
        datos(p)?,
    )
    .await?;
    Ok(Json(i.into()))
}
