//! Descarga de archivos del almacén local: `/api/v1/archivos/descarga`.
//!
//! Es la única ruta fuera de la convención `plataforma` / `yo` / `copropiedades`: no lleva
//! sesión porque **el enlace firmado es la credencial** (equivale a una URL prefirmada de S3).
//! El permiso se verifica al generar el enlace (`casos_uso::archivos::enlace_descarga`), que
//! vence en minutos. Con un almacén S3 los enlaces apuntan al proveedor y esta ruta deja de
//! usarse.

use application::puertos::{AlmacenArchivos, Reloj, RelojSistema};
use axum::{
    extract::State,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use domain::NombreArchivo;
use infra_archivos::ParametrosDescarga;
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::Query,
    openapi::ETIQUETA_ARCHIVOS,
    state::AppState,
};

/// Router de archivos (se monta en `/api/v1/archivos`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(descargar))
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
