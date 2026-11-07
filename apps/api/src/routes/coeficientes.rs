//! Cuadros de coeficientes: `/api/v1/copropiedades/{copropiedad_id}/cuadros-coeficientes/…`.

use application::{
    casos_uso::coeficientes::{self as uc, DetalleCuadro, NuevoCuadro, RefBienPrivado},
    puertos::{RelojSistema, copropiedades::CuadroCoeficientes},
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, BienPrivadoId, CodigoBienPrivado, Coeficiente, CuadroCoeficientesId, CuentaId,
    EstadoCuadro, RevisionCuadro,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path},
    openapi::ETIQUETA_COEFICIENTES,
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de cuadros de coeficientes.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar, crear))
        .routes(routes!(ver))
        .routes(routes!(cargar))
        .routes(routes!(validar))
        .routes(routes!(activar))
}

/// Cuadro de coeficientes.
#[derive(Debug, Serialize, ToSchema)]
pub struct CuadroRespuesta {
    pub id: Uuid,
    pub version: u32,
    /// `borrador`, `vigente` o `historico`.
    #[schema(value_type = String, example = "borrador")]
    pub estado: EstadoCuadro,
    pub descripcion: Option<String>,
    pub soporte_archivo_id: Option<Uuid>,
    pub vigente_desde: Option<DateTime<Utc>>,
    /// Cuenta que lo activó.
    pub aprobado_por: Option<Uuid>,
    pub creado_en: DateTime<Utc>,
}

impl From<CuadroCoeficientes> for CuadroRespuesta {
    fn from(c: CuadroCoeficientes) -> Self {
        Self {
            id: c.id.as_uuid(),
            version: c.version,
            estado: c.estado,
            descripcion: c.descripcion,
            soporte_archivo_id: c.soporte_archivo_id.map(ArchivoId::as_uuid),
            vigente_desde: c.vigente_desde,
            aprobado_por: c.aprobado_por.map(CuentaId::as_uuid),
            creado_en: c.creado_en,
        }
    }
}

/// Revisión del cuadro contra los bienes privados activos (herramienta de ajuste de redondeo).
#[derive(Debug, Serialize, ToSchema)]
pub struct RevisionRespuesta {
    #[schema(value_type = String, example = "0.99999999")]
    pub suma: Decimal,
    /// `1 - suma`: positiva si falta, negativa si sobra.
    #[schema(value_type = String, example = "0.00000001")]
    pub diferencia: Decimal,
    /// Bienes privados activos sin coeficiente.
    pub faltantes: Vec<Uuid>,
    /// Bienes del cuadro que no están activos.
    pub sobrantes: Vec<Uuid>,
    /// Sugerencia (decisión humana): el bien con mayor coeficiente.
    pub sugerido_para_ajuste: Option<Uuid>,
    /// ¿Se puede activar?
    pub valido: bool,
}

impl From<RevisionCuadro> for RevisionRespuesta {
    fn from(r: RevisionCuadro) -> Self {
        Self {
            valido: r.es_valido(),
            suma: r.suma,
            diferencia: r.diferencia,
            faltantes: r
                .faltantes
                .into_iter()
                .map(BienPrivadoId::as_uuid)
                .collect(),
            sobrantes: r
                .sobrantes
                .into_iter()
                .map(BienPrivadoId::as_uuid)
                .collect(),
            sugerido_para_ajuste: r.sugerido_para_ajuste.map(BienPrivadoId::as_uuid),
        }
    }
}

/// Coeficiente de un bien privado en un cuadro.
#[derive(Debug, Serialize, ToSchema)]
pub struct CoeficienteRespuesta {
    pub bien_privado_id: Uuid,
    /// Fracción (0, 1].
    #[schema(value_type = String, example = "0.00523000")]
    pub coeficiente: Decimal,
}

/// Cuadro con sus coeficientes y su revisión.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleCuadroRespuesta {
    pub cuadro: CuadroRespuesta,
    pub coeficientes: Vec<CoeficienteRespuesta>,
    pub revision: RevisionRespuesta,
}

impl From<DetalleCuadro> for DetalleCuadroRespuesta {
    fn from(d: DetalleCuadro) -> Self {
        Self {
            cuadro: d.cuadro.into(),
            coeficientes: d
                .coeficientes
                .into_iter()
                .map(|(b, c)| CoeficienteRespuesta {
                    bien_privado_id: b.as_uuid(),
                    coeficiente: c.valor(),
                })
                .collect(),
            revision: d.revision.into(),
        }
    }
}

/// Cuadros de coeficientes, del más reciente al más antiguo.
#[utoipa::path(
    get,
    path = "/cuadros-coeficientes",
    tag = ETIQUETA_COEFICIENTES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Cuadros", body = Vec<CuadroRespuesta>),
        (status = FORBIDDEN, description = "Sin `bienes_privados.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<CuadroRespuesta>>> {
    let cuadros = uc::listar_cuadros(&state.unidad, &en.ctx).await?;
    Ok(Json(cuadros.into_iter().map(Into::into).collect()))
}

/// Borrador nuevo.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct NuevoCuadroPeticion {
    /// Partir de los coeficientes del cuadro vigente.
    #[serde(default)]
    pub copiar_vigente: bool,
    #[schema(example = "Reforma del reglamento, escritura 1234 de la Notaría 5")]
    pub descripcion: Option<String>,
    /// Escritura o acta de la reforma (opcional).
    pub soporte_archivo_id: Option<Uuid>,
}

/// Crea un cuadro borrador (siguiente versión), opcionalmente copiando el vigente.
#[utoipa::path(
    post,
    path = "/cuadros-coeficientes",
    tag = ETIQUETA_COEFICIENTES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoCuadroPeticion,
    responses(
        (status = CREATED, description = "Borrador creado", body = CuadroRespuesta),
        (status = FORBIDDEN, description = "Sin `bienes_privados.coeficientes.editar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn crear(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoCuadroPeticion>,
) -> ApiResult<(StatusCode, Json<CuadroRespuesta>)> {
    let c = uc::crear_cuadro(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoCuadro {
            copiar_vigente: p.copiar_vigente,
            descripcion: p.descripcion,
            soporte_archivo_id: p.soporte_archivo_id.map(ArchivoId::from_uuid),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

fn id_cuadro(ruta: (Uuid, Uuid)) -> CuadroCoeficientesId {
    CuadroCoeficientesId::from_uuid(ruta.1)
}

/// Cuadro con sus coeficientes y su revisión.
#[utoipa::path(
    get,
    path = "/cuadros-coeficientes/{cuadro_id}",
    tag = ETIQUETA_COEFICIENTES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("cuadro_id" = Uuid, Path, description = "Cuadro"),
    ),
    responses(
        (status = OK, description = "Cuadro", body = DetalleCuadroRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DetalleCuadroRespuesta>> {
    Ok(Json(
        uc::validar_cuadro(&state.unidad, &en.ctx, id_cuadro(ruta))
            .await?
            .into(),
    ))
}

/// Coeficiente de un bien privado (por id o por código).
#[derive(Debug, Deserialize, ToSchema)]
pub struct FilaCoeficientePeticion {
    pub bien_privado_id: Option<Uuid>,
    #[schema(value_type = Option<String>, example = "T3-506")]
    pub codigo: Option<CodigoBienPrivado>,
    /// Porcentaje (`"0,5230 %"`) o fracción (`"0.005230"`).
    #[schema(example = "0,5230 %")]
    pub coeficiente: String,
}

/// Carga completa del borrador (reemplaza las filas anteriores).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CoeficientesPeticion {
    pub coeficientes: Vec<FilaCoeficientePeticion>,
}

/// Reemplaza los coeficientes de un borrador. Devuelve la revisión (suma y diferencia).
#[utoipa::path(
    put,
    path = "/cuadros-coeficientes/{cuadro_id}/coeficientes",
    tag = ETIQUETA_COEFICIENTES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("cuadro_id" = Uuid, Path, description = "Cuadro borrador"),
    ),
    request_body = CoeficientesPeticion,
    responses(
        (status = OK, description = "Coeficientes cargados", body = RevisionRespuesta),
        (status = CONFLICT, description = "El cuadro no es borrador (`conflicto_estado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Cuadro o bien privado inexistente", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Coeficiente inválido o repetido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn cargar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<CoeficientesPeticion>,
) -> ApiResult<Json<RevisionRespuesta>> {
    let filas = p
        .coeficientes
        .into_iter()
        .map(|f| {
            let referencia = match (f.bien_privado_id, f.codigo) {
                (Some(id), None) => RefBienPrivado::Id(BienPrivadoId::from_uuid(id)),
                (None, Some(c)) => RefBienPrivado::Codigo(c),
                _ => {
                    return Err(ApiError::PeticionInvalida(
                        "cada fila lleva `bien_privado_id` o `codigo`, no ambos".into(),
                    ));
                }
            };
            Ok((referencia, Coeficiente::desde_texto(&f.coeficiente)?))
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let revision = uc::cargar_coeficientes(&state.unidad, &en.ctx, id_cuadro(ruta), filas).await?;
    Ok(Json(revision.into()))
}

/// Revisa el cuadro: suma, diferencia frente a 1, faltantes y sobrantes.
#[utoipa::path(
    post,
    path = "/cuadros-coeficientes/{cuadro_id}/validar",
    tag = ETIQUETA_COEFICIENTES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("cuadro_id" = Uuid, Path, description = "Cuadro"),
    ),
    responses(
        (status = OK, description = "Revisión", body = RevisionRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn validar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<RevisionRespuesta>> {
    let d = uc::validar_cuadro(&state.unidad, &en.ctx, id_cuadro(ruta)).await?;
    Ok(Json(d.revision.into()))
}

/// Activa el borrador: el vigente pasa a histórico y cada bien privado toma su coeficiente.
/// Exige suma exactamente 1, todos los bienes activos y ninguna reunión instalada.
#[utoipa::path(
    post,
    path = "/cuadros-coeficientes/{cuadro_id}/activar",
    tag = ETIQUETA_COEFICIENTES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("cuadro_id" = Uuid, Path, description = "Cuadro borrador"),
    ),
    responses(
        (status = OK, description = "Cuadro vigente", body = CuadroRespuesta),
        (status = FORBIDDEN, description = "Sin `bienes_privados.coeficientes.activar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "No es borrador (`conflicto_estado`) o hay una reunión instalada (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No suma exactamente 1 (`coeficientes_no_suman_uno`) o faltan bienes (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn activar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<CuadroRespuesta>> {
    let c = uc::activar_cuadro(&state.unidad, &RelojSistema, &en.ctx, id_cuadro(ruta)).await?;
    Ok(Json(c.into()))
}
