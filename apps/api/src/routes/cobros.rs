//! Cobro de la plataforma (plan 19, ADR 0024): `/api/v1/plataforma/cobros/…` (catálogo,
//! tarifas, suscripciones y cortes) y `/api/v1/entidades/{entidad_id}/suscripcion` (lo que ve la
//! entidad: su plan, su consumo y sus facturas).

use application::{
    casos_uso::cobros::{
        self as uc, CambiosSuscripcion, DatosProducto, DetalleSuscripcion, NuevaSuscripcion,
        cortes as uc_cortes, portal,
    },
    puertos::{
        RelojSistema,
        cobros::{
            Corte, FiltroCortes, LineaSuscripcion, OrigenCorte, Pagador, Producto, Suscripcion,
            TarifaProducto,
        },
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    CorteFacturacionId, DireccionCorreo, ItemFacturacionId, LineaSuscripcionId, LiquidacionId,
    Modulo, OrganizacionId, ProductoPlataformaId, SuscripcionId, TenantId,
    cobro_plataforma::{Ciclo, EstadoCorte, LineaCorte, Metrica, ModeloTarifa, Tarifa},
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_PLATAFORMA,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::{EnEntidad, OperadorPlataforma},
    state::AppState,
};

/// Rutas de la plataforma (se montan en `/api/v1/plataforma`).
pub fn rutas_plataforma() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(catalogo, crear_producto))
        .routes(routes!(editar_producto))
        .routes(routes!(fijar_tarifa))
        .routes(routes!(listar_suscripciones, crear_suscripcion))
        .routes(routes!(ver_suscripcion, editar_suscripcion))
        .routes(routes!(agregar_linea))
        .routes(routes!(terminar_linea))
        .routes(routes!(listar_cortes, calcular_corte))
        .routes(routes!(ver_corte))
        .routes(routes!(aprobar_corte))
        .routes(routes!(descartar_corte))
        .routes(routes!(facturar_corte))
        .routes(routes!(facturado_externo))
        .routes(routes!(pagar_corte))
        .routes(routes!(cobrar_liquidacion))
}

/// Rutas de la entidad (se montan en `/api/v1/entidades/{entidad_id}`).
pub fn rutas_entidad() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(mi_suscripcion))
}

fn invalido(campo: &str) -> ApiError {
    ApiError::App(application::AppError::Validacion(format!(
        "`{campo}` inválido"
    )))
}

const PROHIBIDO_COMERCIAL: &str = "Sin `plataforma.comercial.gestionar` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)";
const PROHIBIDO_APROBAR: &str = "Sin `plataforma.cobros.aprobar` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)";
const PROHIBIDO_FACTURAR: &str = "Sin `plataforma.cobros.facturar` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)";
const PROHIBIDO_LEER: &str = "Sin `plataforma.comercial.gestionar` ni `plataforma.cobros.aprobar` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)";
const PROHIBIDO_CORTES: &str = "Sin `plataforma.cobros.aprobar` ni `plataforma.cobros.facturar` (`sin_permiso`) o sin segundo factor (`segundo_factor_requerido`)";

// ---------------------------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------------------------

/// Producto del catálogo comercial.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ProductoPeticion {
    /// Código único (`a-z0-9_`); no cambia al editar.
    #[schema(example = "asambleas")]
    pub codigo: String,
    #[schema(example = "Asambleas y votaciones")]
    pub nombre: String,
    /// Texto de la línea de la factura: `{producto}`, `{periodo}`, `{cantidad}`, `{incluidas}`.
    #[schema(example = "{producto} — {periodo}")]
    pub descripcion_factura: String,
    /// Módulo que cobra (para avisar de los módulos habilitados sin cobro).
    #[schema(value_type = Option<crate::esquemas::Modulo>)]
    pub modulo: Option<String>,
    /// Ítem de facturación de la entidad de la plataforma (cuenta e IVA salen de él).
    pub item_facturacion_id: Option<Uuid>,
    #[serde(default = "verdadero")]
    pub activo: bool,
}

fn verdadero() -> bool {
    true
}

impl TryFrom<ProductoPeticion> for DatosProducto {
    type Error = ApiError;

    fn try_from(p: ProductoPeticion) -> Result<Self, ApiError> {
        Ok(Self {
            codigo: p.codigo,
            nombre: p.nombre,
            descripcion_factura: p.descripcion_factura,
            modulo: p.modulo.as_deref().map(Modulo::try_from).transpose()?,
            item_facturacion_id: p.item_facturacion_id.map(ItemFacturacionId::from_uuid),
            activo: p.activo,
        })
    }
}

/// Tarifa de un producto desde una fecha.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = TarifaCobroDto)]
pub struct TarifaDto {
    #[schema(value_type = crate::esquemas::ModeloTarifa)]
    pub modelo: String,
    #[schema(value_type = String, example = "90000")]
    pub valor: Decimal,
    /// Unidades incluidas (solo `por_paquete`).
    pub incluidas: Option<u32>,
    /// Valor de cada unidad de excedente (solo `por_paquete`).
    #[schema(value_type = Option<String>)]
    pub valor_excedente: Option<Decimal>,
    /// `ph.bienes_privados`, `facturacion.documentos`, `whatsapp.mensajes` o `usuarios.cuentas`
    /// (obligatoria en `por_unidad`, `por_paquete` y `por_consumo`).
    pub metrica: Option<String>,
    pub vigente_desde: NaiveDate,
}

impl From<TarifaProducto> for TarifaDto {
    fn from(t: TarifaProducto) -> Self {
        Self {
            modelo: t.tarifa.modelo.as_str().to_owned(),
            valor: t.tarifa.valor,
            incluidas: t.tarifa.incluidas,
            valor_excedente: t.tarifa.valor_excedente,
            metrica: t.tarifa.metrica.map(String::from),
            vigente_desde: t.vigente_desde,
        }
    }
}

/// Producto con sus tarifas.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProductoRespuesta {
    pub id: Uuid,
    pub codigo: String,
    pub nombre: String,
    pub descripcion_factura: String,
    #[schema(value_type = Option<crate::esquemas::Modulo>)]
    pub modulo: Option<String>,
    pub item_facturacion_id: Option<Uuid>,
    pub activo: bool,
    /// La más reciente primero.
    pub tarifas: Vec<TarifaDto>,
}

fn producto_respuesta(p: Producto, tarifas: Vec<TarifaProducto>) -> ProductoRespuesta {
    ProductoRespuesta {
        id: p.id.as_uuid(),
        codigo: p.codigo,
        nombre: p.nombre,
        descripcion_factura: p.descripcion_factura,
        modulo: p.modulo.map(|m| m.as_str().to_owned()),
        item_facturacion_id: p.item_facturacion_id.map(|i| i.as_uuid()),
        activo: p.activo,
        tarifas: tarifas.into_iter().map(Into::into).collect(),
    }
}

/// Suscripción nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SuscripcionPeticion {
    pub entidad_id: Uuid,
    /// Otra entidad que paga (o `pagador_organizacion_id`, o ninguno: paga la misma entidad).
    pub pagador_entidad_id: Option<Uuid>,
    pub pagador_organizacion_id: Option<Uuid>,
    /// Cada cuántos meses se corta (1 a 12); `null` = por evento (se corta a pedido).
    pub ciclo_meses: Option<u8>,
    pub inicio: NaiveDate,
    /// Prueba gratuita de 0 a 30 días.
    #[serde(default)]
    pub dias_prueba: u16,
    /// Días para pagar cada factura (15 si se omite).
    pub plazo_pago_dias: Option<u16>,
    #[schema(example = "facturacion@conjunto.co")]
    pub correo_facturacion: String,
}

/// Línea (producto contratado).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = LineaSuscripcionRespuesta)]
pub struct LineaRespuesta {
    pub id: Uuid,
    pub producto_id: Uuid,
    pub producto: String,
    pub nombre: String,
    #[schema(value_type = Option<String>)]
    pub precio_pactado: Option<Decimal>,
    pub motivo_precio: Option<String>,
    pub desde: NaiveDate,
    pub hasta: Option<NaiveDate>,
}

fn linea_respuesta(l: LineaSuscripcion, p: &Producto) -> LineaRespuesta {
    LineaRespuesta {
        id: l.id.as_uuid(),
        producto_id: l.producto_id.as_uuid(),
        producto: p.codigo.clone(),
        nombre: p.nombre.clone(),
        precio_pactado: l.precio_pactado,
        motivo_precio: l.motivo_precio,
        desde: l.desde,
        hasta: l.hasta,
    }
}

/// Suscripción.
#[derive(Debug, Serialize, ToSchema)]
pub struct SuscripcionRespuesta {
    pub id: Uuid,
    pub entidad_id: Uuid,
    pub pagador_entidad_id: Option<Uuid>,
    pub pagador_organizacion_id: Option<Uuid>,
    /// `null` = por evento.
    pub ciclo_meses: Option<u8>,
    pub inicio: NaiveDate,
    pub dias_prueba: u16,
    pub plazo_pago_dias: u16,
    pub correo_facturacion: String,
    #[schema(value_type = crate::esquemas::EstadoSuscripcion)]
    pub estado: String,
    /// La entidad está suspendida por mora (se reactiva al pagar).
    pub suspendida_por_mora: bool,
    pub terminada_en: Option<NaiveDate>,
}

impl From<Suscripcion> for SuscripcionRespuesta {
    fn from(s: Suscripcion) -> Self {
        let (e, o) = match s.pagador {
            Pagador::MismaEntidad => (None, None),
            Pagador::Entidad(e) => (Some(e.as_uuid()), None),
            Pagador::Organizacion(o) => (None, Some(o.as_uuid())),
        };
        Self {
            id: s.id.as_uuid(),
            entidad_id: s.tenant_id.as_uuid(),
            pagador_entidad_id: e,
            pagador_organizacion_id: o,
            ciclo_meses: match s.ciclo {
                Ciclo::Meses(m) => Some(m),
                Ciclo::PorEvento => None,
            },
            inicio: s.inicio,
            dias_prueba: s.dias_prueba,
            plazo_pago_dias: s.plazo_pago_dias,
            correo_facturacion: s.correo_facturacion.as_str().to_owned(),
            estado: s.estado.as_str().to_owned(),
            suspendida_por_mora: s.suspendida_por_mora,
            terminada_en: s.terminada_en,
        }
    }
}

/// Suscripción con sus líneas.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleSuscripcionRespuesta {
    pub suscripcion: SuscripcionRespuesta,
    pub lineas: Vec<LineaRespuesta>,
    /// Módulos habilitados en la entidad sin un producto que los cobre.
    #[schema(value_type = Vec<crate::esquemas::Modulo>)]
    pub modulos_sin_cobro: Vec<String>,
}

impl From<DetalleSuscripcion> for DetalleSuscripcionRespuesta {
    fn from(d: DetalleSuscripcion) -> Self {
        Self {
            suscripcion: d.suscripcion.into(),
            lineas: d
                .lineas
                .into_iter()
                .map(|(l, p)| linea_respuesta(l, &p))
                .collect(),
            modulos_sin_cobro: d.modulos_sin_cobro,
        }
    }
}

/// Cambios de una suscripción.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CambiosSuscripcionPeticion {
    pub correo_facturacion: Option<String>,
    pub plazo_pago_dias: Option<u16>,
    /// Termina la suscripción en esa fecha.
    pub terminar_en: Option<NaiveDate>,
    pub motivo: Option<String>,
}

/// Línea nueva.
#[derive(Debug, Deserialize, ToSchema)]
#[schema(as = LineaSuscripcionPeticion)]
pub struct LineaPeticion {
    pub producto_id: Uuid,
    pub desde: NaiveDate,
    /// Precio pactado que reemplaza la tarifa (exige `motivo_precio`).
    #[schema(value_type = Option<String>)]
    pub precio_pactado: Option<Decimal>,
    pub motivo_precio: Option<String>,
}

/// Fin de una línea.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FinLineaPeticion {
    /// Deja de cobrarse desde este día (excluido).
    pub hasta: NaiveDate,
}

/// Línea de un corte.
#[derive(Debug, Serialize, ToSchema)]
pub struct LineaCorteDto {
    pub producto: String,
    pub descripcion: String,
    #[schema(value_type = String)]
    pub cantidad: Decimal,
    #[schema(value_type = String)]
    pub valor_unitario: Decimal,
    #[schema(value_type = String)]
    pub valor: Decimal,
}

impl From<LineaCorte> for LineaCorteDto {
    fn from(l: LineaCorte) -> Self {
        Self {
            producto: l.producto,
            descripcion: l.descripcion,
            cantidad: l.cantidad,
            valor_unitario: l.valor_unitario,
            valor: l.valor,
        }
    }
}

/// Corte de cobro.
#[derive(Debug, Serialize, ToSchema)]
pub struct CorteRespuesta {
    pub id: Uuid,
    pub suscripcion_id: Option<Uuid>,
    /// Corte de una liquidación de publicidad (un anunciante).
    pub liquidacion_id: Option<Uuid>,
    pub periodo_desde: NaiveDate,
    /// Excluido.
    pub periodo_hasta: NaiveDate,
    pub lineas: Vec<LineaCorteDto>,
    /// Antes de impuestos (el IVA lo pone el ítem de facturación).
    #[schema(value_type = String)]
    pub total: Decimal,
    pub huella: String,
    #[schema(value_type = crate::esquemas::EstadoCorte)]
    pub estado: String,
    pub motivo: Option<String>,
    /// `null` con estado `aprobado` = se aprobó solo (igual al anterior).
    pub aprobado_por: Option<Uuid>,
    pub aprobado_en: Option<DateTime<Utc>>,
    pub factura_venta_id: Option<Uuid>,
    pub factura_externa: Option<String>,
    pub facturado_en: Option<NaiveDate>,
    pub vencimiento: Option<NaiveDate>,
    pub pagado_en: Option<NaiveDate>,
}

impl From<Corte> for CorteRespuesta {
    fn from(c: Corte) -> Self {
        let (s, l) = match c.origen {
            OrigenCorte::Suscripcion(s) => (Some(s.as_uuid()), None),
            OrigenCorte::Liquidacion(l) => (None, Some(l.as_uuid())),
        };
        Self {
            id: c.id.as_uuid(),
            suscripcion_id: s,
            liquidacion_id: l,
            periodo_desde: c.periodo.desde,
            periodo_hasta: c.periodo.hasta,
            lineas: c.lineas.into_iter().map(Into::into).collect(),
            total: c.total,
            huella: c.huella,
            estado: c.estado.as_str().to_owned(),
            motivo: c.motivo,
            aprobado_por: c.aprobado_por.map(|x| x.as_uuid()),
            aprobado_en: c.aprobado_en,
            factura_venta_id: c.factura_venta_id.map(|x| x.as_uuid()),
            factura_externa: c.factura_externa,
            facturado_en: c.facturado_en,
            vencimiento: c.vencimiento,
            pagado_en: c.pagado_en,
        }
    }
}

/// Filtro de cortes.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroCortesParametros {
    #[param(value_type = Option<crate::esquemas::EstadoCorte>)]
    pub estado: Option<String>,
    pub suscripcion_id: Option<Uuid>,
}

/// Periodo a calcular.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CalcularPeticion {
    pub suscripcion_id: Uuid,
    /// Primer día del periodo.
    pub desde: NaiveDate,
    /// Fin (excluido), solo en suscripciones por evento.
    pub hasta: Option<NaiveDate>,
}

/// Motivo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MotivoCortePeticion {
    pub motivo: String,
}

/// Factura emitida por fuera del sistema.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FacturaExternaPeticion {
    #[schema(example = "FE-1203")]
    pub numero: String,
    pub fecha: NaiveDate,
}

/// Fecha del pago.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PagoCortePeticion {
    pub fecha: NaiveDate,
}

/// Consumo medido en el periodo en curso.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConsumoRespuesta {
    pub metrica: String,
    pub cantidad: u64,
}

/// Plan, consumo y facturas de la entidad.
#[derive(Debug, Serialize, ToSchema)]
pub struct MiSuscripcionRespuesta {
    pub suscripcion: SuscripcionRespuesta,
    pub lineas: Vec<LineaRespuesta>,
    pub consumo: Vec<ConsumoRespuesta>,
    /// Aprobados, facturados y pagados, los más recientes primero.
    pub cortes: Vec<CorteRespuesta>,
}

// ---------------------------------------------------------------------------------------------
// Catálogo
// ---------------------------------------------------------------------------------------------

/// Catálogo comercial con las tarifas de cada producto.
#[utoipa::path(
    get,
    path = "/cobros/productos",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Productos", body = Vec<ProductoRespuesta>),
        (status = FORBIDDEN, description = PROHIBIDO_LEER, body = ErrorRespuesta),
    )
)]
pub async fn catalogo(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
) -> ApiResult<Json<Vec<ProductoRespuesta>>> {
    let v = uc::catalogo(&state.plataforma, &op.ctx).await?;
    Ok(Json(
        v.into_iter()
            .map(|(p, t)| producto_respuesta(p, t))
            .collect(),
    ))
}

/// Crea un producto del catálogo comercial.
#[utoipa::path(
    post,
    path = "/cobros/productos",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = ProductoPeticion,
    responses(
        (status = CREATED, description = "Producto creado", body = ProductoRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_COMERCIAL, body = ErrorRespuesta),
        (status = CONFLICT, description = "Código repetido (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_producto(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<ProductoPeticion>,
) -> ApiResult<(StatusCode, Json<ProductoRespuesta>)> {
    let x = uc::crear_producto(&state.plataforma, &RelojSistema, &op.ctx, p.try_into()?).await?;
    Ok((StatusCode::CREATED, Json(producto_respuesta(x, Vec::new()))))
}

/// Edita un producto (el código no cambia).
#[utoipa::path(
    put,
    path = "/cobros/productos/{producto_id}",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("producto_id" = Uuid, Path, description = "Producto")),
    request_body = ProductoPeticion,
    responses(
        (status = OK, description = "Producto editado", body = ProductoRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_COMERCIAL, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn editar_producto(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<ProductoPeticion>,
) -> ApiResult<Json<ProductoRespuesta>> {
    let x = uc::editar_producto(
        &state.plataforma,
        &op.ctx,
        ProductoPlataformaId::from_uuid(id),
        p.try_into()?,
    )
    .await?;
    Ok(Json(producto_respuesta(x, Vec::new())))
}

/// Fija la tarifa del producto desde una fecha (versionada: nunca se edita).
#[utoipa::path(
    post,
    path = "/cobros/productos/{producto_id}/tarifas",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("producto_id" = Uuid, Path, description = "Producto")),
    request_body = TarifaDto,
    responses(
        (status = CREATED, description = "Tarifa fijada", body = TarifaDto),
        (status = FORBIDDEN, description = PROHIBIDO_COMERCIAL, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya hay una tarifa desde ese día (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Tarifa incoherente con su modelo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn fijar_tarifa(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<TarifaDto>,
) -> ApiResult<(StatusCode, Json<TarifaDto>)> {
    let tarifa = Tarifa {
        modelo: ModeloTarifa::try_from(p.modelo.as_str())?,
        valor: p.valor,
        incluidas: p.incluidas,
        valor_excedente: p.valor_excedente,
        metrica: p.metrica.as_deref().map(Metrica::new).transpose()?,
    };
    let t = uc::fijar_tarifa(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        ProductoPlataformaId::from_uuid(id),
        tarifa,
        p.vigente_desde,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(t.into())))
}

// ---------------------------------------------------------------------------------------------
// Suscripciones
// ---------------------------------------------------------------------------------------------

/// Suscripciones vigentes.
#[utoipa::path(
    get,
    path = "/cobros/suscripciones",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Suscripciones", body = Vec<SuscripcionRespuesta>),
        (status = FORBIDDEN, description = PROHIBIDO_LEER, body = ErrorRespuesta),
    )
)]
pub async fn listar_suscripciones(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
) -> ApiResult<Json<Vec<SuscripcionRespuesta>>> {
    let v = uc::suscripciones(&state.plataforma, &op.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea la suscripción de una entidad (una vigente por entidad).
#[utoipa::path(
    post,
    path = "/cobros/suscripciones",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = SuscripcionPeticion,
    responses(
        (status = CREATED, description = "Suscripción creada", body = SuscripcionRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_COMERCIAL, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La entidad o el pagador no existen", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya tiene una suscripción vigente (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Ciclo, prueba, plazo o pagador inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_suscripcion(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<SuscripcionPeticion>,
) -> ApiResult<(StatusCode, Json<SuscripcionRespuesta>)> {
    let pagador = match (p.pagador_entidad_id, p.pagador_organizacion_id) {
        (None, None) => Pagador::MismaEntidad,
        (Some(e), None) => Pagador::Entidad(TenantId::from_uuid(e)),
        (None, Some(o)) => Pagador::Organizacion(OrganizacionId::from_uuid(o)),
        (Some(_), Some(_)) => return Err(invalido("pagador")),
    };
    let s = uc::crear_suscripcion(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        NuevaSuscripcion {
            tenant_id: TenantId::from_uuid(p.entidad_id),
            pagador,
            ciclo: p.ciclo_meses.map_or(Ciclo::PorEvento, Ciclo::Meses),
            inicio: p.inicio,
            dias_prueba: p.dias_prueba,
            plazo_pago_dias: p.plazo_pago_dias,
            correo_facturacion: DireccionCorreo::new(&p.correo_facturacion)?,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(s.into())))
}

/// Suscripción con sus líneas y los módulos habilitados sin cobro.
#[utoipa::path(
    get,
    path = "/cobros/suscripciones/{suscripcion_id}",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("suscripcion_id" = Uuid, Path, description = "Suscripción")),
    responses(
        (status = OK, description = "Suscripción", body = DetalleSuscripcionRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_LEER, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver_suscripcion(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DetalleSuscripcionRespuesta>> {
    let d = uc::ver_suscripcion(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        SuscripcionId::from_uuid(id),
    )
    .await?;
    Ok(Json(d.into()))
}

/// Cambia el correo o el plazo de pago, o termina la suscripción.
#[utoipa::path(
    patch,
    path = "/cobros/suscripciones/{suscripcion_id}",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("suscripcion_id" = Uuid, Path, description = "Suscripción")),
    request_body = CambiosSuscripcionPeticion,
    responses(
        (status = OK, description = "Suscripción", body = SuscripcionRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_COMERCIAL, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Terminada o datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn editar_suscripcion(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<CambiosSuscripcionPeticion>,
) -> ApiResult<Json<SuscripcionRespuesta>> {
    let s = uc::editar_suscripcion(
        &state.plataforma,
        &op.ctx,
        SuscripcionId::from_uuid(id),
        CambiosSuscripcion {
            correo_facturacion: p
                .correo_facturacion
                .as_deref()
                .map(DireccionCorreo::new)
                .transpose()?,
            plazo_pago_dias: p.plazo_pago_dias,
            terminar_en: p.terminar_en,
            motivo: p.motivo,
        },
    )
    .await?;
    Ok(Json(s.into()))
}

/// Agrega un producto a la suscripción.
#[utoipa::path(
    post,
    path = "/cobros/suscripciones/{suscripcion_id}/lineas",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("suscripcion_id" = Uuid, Path, description = "Suscripción")),
    request_body = LineaPeticion,
    responses(
        (status = CREATED, description = "Línea agregada", body = LineaRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_COMERCIAL, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Suscripción o producto inexistentes", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Producto inactivo o sin tarifa, suscripción terminada o precio sin motivo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn agregar_linea(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<LineaPeticion>,
) -> ApiResult<(StatusCode, Json<LineaRespuesta>)> {
    let precio = match (p.precio_pactado, p.motivo_precio) {
        (Some(v), Some(m)) => Some((v, m)),
        (None, None) => None,
        _ => return Err(invalido("motivo_precio")),
    };
    let (l, producto) = uc::agregar_linea(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        SuscripcionId::from_uuid(id),
        ProductoPlataformaId::from_uuid(p.producto_id),
        p.desde,
        precio,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(linea_respuesta(l, &producto))))
}

/// Termina una línea (deja de cobrarse desde `hasta`).
#[utoipa::path(
    post,
    path = "/cobros/lineas/{linea_id}/fin",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("linea_id" = Uuid, Path, description = "Línea")),
    request_body = FinLineaPeticion,
    responses(
        (status = NO_CONTENT, description = "Terminada"),
        (status = FORBIDDEN, description = PROHIBIDO_COMERCIAL, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Termina antes de empezar (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn terminar_linea(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<FinLineaPeticion>,
) -> ApiResult<StatusCode> {
    uc::terminar_linea(
        &state.plataforma,
        &op.ctx,
        LineaSuscripcionId::from_uuid(id),
        p.hasta,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Cortes
// ---------------------------------------------------------------------------------------------

/// Cortes (los más recientes primero), con filtro por estado o suscripción.
#[utoipa::path(
    get,
    path = "/cobros/cortes",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(FiltroCortesParametros, ParametrosPagina),
    responses(
        (status = OK, description = "Cortes", body = PaginaRespuesta<CorteRespuesta>),
        (status = FORBIDDEN, description = PROHIBIDO_CORTES, body = ErrorRespuesta),
    )
)]
pub async fn listar_cortes(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Query(f): Query<FiltroCortesParametros>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<CorteRespuesta>>> {
    let filtro = FiltroCortes {
        estado: f.estado.as_deref().map(EstadoCorte::try_from).transpose()?,
        suscripcion: f.suscripcion_id.map(SuscripcionId::from_uuid),
    };
    let v = uc_cortes::listar(&state.plataforma, &op.ctx, &filtro, p.validar()?).await?;
    Ok(Json(v.mapear(Into::into).into()))
}

/// Calcula (o recalcula, si sigue en borrador) el corte de un periodo. Si cobra lo mismo que el
/// anterior queda aprobado solo.
#[utoipa::path(
    post,
    path = "/cobros/cortes",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    request_body = CalcularPeticion,
    responses(
        (status = OK, description = "Corte", body = CorteRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_APROBAR, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La suscripción no existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Falta `hasta` en un ciclo por evento, producto sin tarifa o métrica desconocida (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn calcular_corte(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Json(p): Json<CalcularPeticion>,
) -> ApiResult<Json<CorteRespuesta>> {
    let c = uc_cortes::calcular(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        SuscripcionId::from_uuid(p.suscripcion_id),
        p.desde,
        p.hasta,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Un corte.
#[utoipa::path(
    get,
    path = "/cobros/cortes/{corte_id}",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("corte_id" = Uuid, Path, description = "Corte")),
    responses(
        (status = OK, description = "Corte", body = CorteRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_CORTES, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver_corte(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<CorteRespuesta>> {
    let c = uc_cortes::ver(
        &state.plataforma,
        &op.ctx,
        CorteFacturacionId::from_uuid(id),
    )
    .await?;
    Ok(Json(c.into()))
}

/// Aprueba un corte en borrador (queda «por facturar»).
#[utoipa::path(
    post,
    path = "/cobros/cortes/{corte_id}/aprobar",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("corte_id" = Uuid, Path, description = "Corte")),
    responses(
        (status = OK, description = "Aprobado", body = CorteRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_APROBAR, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está en borrador (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn aprobar_corte(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<CorteRespuesta>> {
    let c = uc_cortes::aprobar(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        CorteFacturacionId::from_uuid(id),
    )
    .await?;
    Ok(Json(c.into()))
}

/// Descarta un corte en borrador o aprobado (con motivo).
#[utoipa::path(
    post,
    path = "/cobros/cortes/{corte_id}/descartar",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("corte_id" = Uuid, Path, description = "Corte")),
    request_body = MotivoCortePeticion,
    responses(
        (status = OK, description = "Descartado", body = CorteRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_APROBAR, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya facturado (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn descartar_corte(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<MotivoCortePeticion>,
) -> ApiResult<Json<CorteRespuesta>> {
    let c = uc_cortes::descartar(
        &state.plataforma,
        &op.ctx,
        CorteFacturacionId::from_uuid(id),
        &p.motivo,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Factura un corte aprobado en la entidad de la plataforma (factura de venta emitida; el
/// documento electrónico se transmite desde su facturación) y avisa al correo de facturación.
#[utoipa::path(
    post,
    path = "/cobros/cortes/{corte_id}/facturar",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("corte_id" = Uuid, Path, description = "Corte")),
    responses(
        (status = OK, description = "Facturado", body = CorteRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_FACTURAR, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está aprobado (`conflicto_estado`) o ya tiene factura (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin entidad de la plataforma, pagador sin NIT o corte sin valor (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn facturar_corte(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<CorteRespuesta>> {
    let c = uc_cortes::facturar(
        &state.plataforma,
        &state.unidad,
        &RelojSistema,
        &op.ctx,
        CorteFacturacionId::from_uuid(id),
    )
    .await?;
    Ok(Json(c.into()))
}

/// Marca un corte aprobado como facturado por fuera del sistema (mientras la plataforma no
/// factura en él).
#[utoipa::path(
    post,
    path = "/cobros/cortes/{corte_id}/factura-externa",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("corte_id" = Uuid, Path, description = "Corte")),
    request_body = FacturaExternaPeticion,
    responses(
        (status = OK, description = "Facturado", body = CorteRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_FACTURAR, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está aprobado (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn facturado_externo(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<FacturaExternaPeticion>,
) -> ApiResult<Json<CorteRespuesta>> {
    let c = uc_cortes::marcar_facturado_externo(
        &state.plataforma,
        &op.ctx,
        CorteFacturacionId::from_uuid(id),
        &p.numero,
        p.fecha,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Registra el pago de un corte facturado; si la entidad estaba suspendida por mora y no le
/// queda otro vencido, se reactiva.
#[utoipa::path(
    post,
    path = "/cobros/cortes/{corte_id}/pago",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("corte_id" = Uuid, Path, description = "Corte")),
    request_body = PagoCortePeticion,
    responses(
        (status = OK, description = "Pagado", body = CorteRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_FACTURAR, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está facturado (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn pagar_corte(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
    Json(p): Json<PagoCortePeticion>,
) -> ApiResult<Json<CorteRespuesta>> {
    let c = uc_cortes::marcar_pagado(
        &state.plataforma,
        &state.unidad,
        &RelojSistema,
        &op.ctx,
        CorteFacturacionId::from_uuid(id),
        p.fecha,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Crea el corte (en borrador) de una liquidación de publicidad de un anunciante.
#[utoipa::path(
    post,
    path = "/cobros/liquidaciones/{liquidacion_id}/corte",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("liquidacion_id" = Uuid, Path, description = "Liquidación de publicidad")),
    responses(
        (status = OK, description = "Corte", body = CorteRespuesta),
        (status = FORBIDDEN, description = PROHIBIDO_APROBAR, body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin valor (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn cobrar_liquidacion(
    State(state): State<AppState>,
    OperadorPlataforma(op): OperadorPlataforma,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<CorteRespuesta>> {
    let c = uc_cortes::cobrar_liquidacion(
        &state.plataforma,
        &RelojSistema,
        &op.ctx,
        LiquidacionId::from_uuid(id),
    )
    .await?;
    Ok(Json(c.into()))
}

// ---------------------------------------------------------------------------------------------
// Entidad
// ---------------------------------------------------------------------------------------------

/// El plan de la entidad con la plataforma, su consumo del periodo en curso y sus facturas.
#[utoipa::path(
    get,
    path = "/suscripcion",
    tag = ETIQUETA_PLATAFORMA,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Suscripción", body = MiSuscripcionRespuesta),
        (status = NOT_FOUND, description = "La entidad no tiene suscripción (`no_encontrado`)", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `suscripcion.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn mi_suscripcion(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<MiSuscripcionRespuesta>> {
    let m = portal::mi_suscripcion(&state.plataforma, &RelojSistema, &en.ctx)
        .await?
        .ok_or(ApiError::App(application::AppError::NoEncontrado(
            "suscripción",
        )))?;
    Ok(Json(MiSuscripcionRespuesta {
        suscripcion: m.detalle.suscripcion.into(),
        lineas: m
            .detalle
            .lineas
            .into_iter()
            .map(|(l, p)| linea_respuesta(l, &p))
            .collect(),
        consumo: m
            .consumo
            .into_iter()
            .map(|(metrica, cantidad)| ConsumoRespuesta {
                metrica: metrica.as_str().to_owned(),
                cantidad,
            })
            .collect(),
        cortes: m.cortes.into_iter().map(Into::into).collect(),
    }))
}
