//! Descarga de archivos del almacén local: `/api/v1/archivos/descarga`.
//!
//! Es la única ruta fuera de la convención `plataforma` / `yo` / `copropiedades`: no lleva
//! sesión porque **el enlace firmado es la credencial** (equivale a una URL prefirmada de S3).
//! El permiso se verifica al generar el enlace (`casos_uso::archivos::enlace_descarga`), que
//! vence en minutos. Con un almacén S3 los enlaces apuntan al proveedor y esta ruta deja de
//! usarse.

use application::{
    casos_uso::archivos::{self as casos_archivos, ArchivoSubido},
    puertos::{AlmacenArchivos, Reloj, RelojSistema, archivos::Archivo},
};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use domain::{ArchivoId, ClasificacionArchivo, NombreArchivo, TipoArchivo};
use infra_archivos::ParametrosDescarga;
use serde::Deserialize;
use serde::Serialize;
use utoipa::IntoParams;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    limite::LimitePublico,
    openapi::ETIQUETA_ARCHIVOS,
    sesion::EnCopropiedad,
    state::AppState,
};

/// Router de archivos (se monta en `/api/v1/archivos`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(descargar))
}

/// Rutas de archivos dentro de una copropiedad (subir y pedir el enlace de descarga). La subida
/// admite hasta el tamaño máximo de un archivo; el resto de la API, mucho menos.
pub fn rutas_copropiedad() -> OpenApiRouter<AppState> {
    let limite = usize::try_from(TipoArchivo::TAMANO_MAXIMO_GLOBAL).unwrap_or(usize::MAX);
    OpenApiRouter::new()
        .routes(routes!(subir))
        .layer(DefaultBodyLimit::max(limite))
        .routes(routes!(enlace))
}

/// Datos del archivo que se sube (el contenido va en el cuerpo, tal cual).
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DatosSubida {
    /// Nombre original, para mostrarlo y descargarlo.
    #[param(example = "Reglamento de propiedad horizontal.pdf")]
    pub nombre: String,
    /// `publico`, `interno`, `personal` o `sensible`.
    #[param(example = "interno")]
    pub clasificacion: String,
}

/// Archivo guardado.
#[derive(Debug, Serialize, ToSchema)]
pub struct ArchivoRespuesta {
    pub id: Uuid,
    pub nombre: String,
    /// Detectado por el contenido.
    #[schema(example = "application/pdf")]
    pub tipo: String,
    pub tamano_bytes: u64,
    pub sha256: String,
    pub clasificacion: String,
    pub creado_en: DateTime<Utc>,
}

impl From<Archivo> for ArchivoRespuesta {
    fn from(a: Archivo) -> Self {
        Self {
            id: a.id.as_uuid(),
            nombre: a.nombre_original.as_str().to_owned(),
            tipo: a.tipo.as_str().into(),
            tamano_bytes: a.tamano_bytes,
            sha256: a.sha256,
            clasificacion: a.clasificacion.as_str().into(),
            creado_en: a.creado_en,
        }
    }
}

/// Sube un archivo (PDF, JPEG, PNG, WebP o HEIC). El tipo se verifica por el contenido.
#[utoipa::path(
    post,
    path = "/archivos",
    tag = ETIQUETA_ARCHIVOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), DatosSubida),
    request_body(content = Vec<u8>, content_type = "application/octet-stream",
                 description = "Contenido del archivo"),
    responses(
        (status = CREATED, description = "Archivo guardado", body = ArchivoRespuesta),
        (status = FORBIDDEN, description = "Sin `archivos.subir` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Tipo no admitido, vacío o muy grande (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn subir(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(d): Query<DatosSubida>,
    cuerpo: Bytes,
) -> ApiResult<(StatusCode, Json<ArchivoRespuesta>)> {
    let archivo = casos_archivos::subir_archivo(
        &state.unidad,
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        ArchivoSubido {
            nombre_original: NombreArchivo::new(&d.nombre)?,
            clasificacion: ClasificacionArchivo::try_from(d.clasificacion.as_str())?,
            contenido: cuerpo.to_vec(),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(archivo.into())))
}

/// Enlace temporal de descarga.
#[derive(Debug, Serialize, ToSchema)]
pub struct EnlaceRespuesta {
    /// URL firmada; vence en 5 minutos.
    pub url: String,
}

/// Enlace de descarga de un archivo (vence en 5 minutos). Los personales o sensibles exigen
/// además `archivos.personales.ver`.
#[utoipa::path(
    get,
    path = "/archivos/{archivo_id}/enlace",
    tag = ETIQUETA_ARCHIVOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("archivo_id" = Uuid, Path, description = "Archivo"),
    ),
    responses(
        (status = OK, description = "Enlace firmado", body = EnlaceRespuesta),
        (status = FORBIDDEN, description = "Sin permiso (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe en la copropiedad", body = ErrorRespuesta),
    )
)]
pub async fn enlace(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<EnlaceRespuesta>> {
    let url = casos_archivos::enlace_descarga(
        &state.unidad,
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        ArchivoId::from_uuid(ruta.1),
        casos_archivos::VIGENCIA_ENLACE,
    )
    .await?;
    Ok(Json(EnlaceRespuesta { url }))
}

/// Parámetros de un enlace de descarga. Los arma el servidor; el cliente usa la URL tal cual.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ParametrosEnlace {
    /// Clave lógica del archivo.
    pub clave: String,
    /// Tipo MIME del archivo.
    pub tipo: String,
    /// Nombre con el que se descarga.
    pub nombre: String,
    /// Vencimiento del enlace (segundos Unix).
    pub vence: i64,
    /// Firma HMAC-SHA256 del enlace.
    pub firma: String,
}

/// Descarga un archivo con un enlace firmado y vigente.
#[utoipa::path(
    get,
    path = "/descarga",
    tag = ETIQUETA_ARCHIVOS,
    params(ParametrosEnlace),
    responses(
        (status = OK, description = "Contenido del archivo", content(
            ("application/pdf"), ("image/jpeg"), ("image/png"), ("image/webp"), ("image/heic")
        )),
        (status = BAD_REQUEST, description = "Faltan parámetros (`peticion_invalida`)",
         body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Firma inválida o enlace vencido (`enlace_invalido`)",
         body = ErrorRespuesta),
        (status = NOT_FOUND, description = "El archivo ya no está en el almacén (`no_encontrado`)",
         body = ErrorRespuesta),
    )
)]
pub async fn descargar(
    _limite: LimitePublico,
    State(state): State<AppState>,
    Query(p): Query<ParametrosEnlace>,
) -> ApiResult<Response> {
    let parametros = ParametrosDescarga {
        clave: p.clave,
        tipo: p.tipo,
        nombre: p.nombre,
        vence: p.vence,
        firma: p.firma,
    };
    let descarga = state
        .archivos
        .enlaces()
        .verificar(&parametros, RelojSistema.ahora())
        .map_err(|e| {
            tracing::info!(motivo = %e, "enlace de descarga rechazado");
            ApiError::EnlaceInvalido
        })?;
    let contenido = state.archivos.obtener(&descarga.clave).await?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(descarga.tipo.as_str()),
            ),
            (header::CONTENT_DISPOSITION, disposicion(&descarga.nombre)),
            // Ni el navegador adivina otro tipo, ni el contenido ejecuta nada en nuestro origen.
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'none'; sandbox"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-store"),
            ),
        ],
        contenido,
    )
        .into_response())
}

/// `inline` (las apps muestran fotos y PDF) con el nombre original: `filename*` en UTF-8
/// (RFC 6266) y `filename` ASCII de respaldo para clientes antiguos.
fn disposicion(nombre: &NombreArchivo) -> HeaderValue {
    let ascii: String = nombre
        .as_str()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let valor = format!(
        "inline; filename=\"{ascii}\"; filename*=UTF-8''{}",
        infra_archivos::codificar_porcentual(nombre.as_str())
    );
    HeaderValue::from_str(&valor).unwrap_or_else(|_| HeaderValue::from_static("inline"))
}
