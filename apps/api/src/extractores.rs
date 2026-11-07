//! Extractores de Axum que devuelven el error uniforme de la API.
//!
//! Los extractores estándar (`axum::Json`, `Query`, `Path`) responden en texto plano cuando la
//! petición es inválida. Estos envoltorios convierten ese rechazo en [`ApiError`]
//! (`peticion_invalida`, 400), para que **toda** respuesta de error tenga el mismo formato.
//! Los handlers deben usar estos tipos en lugar de los de `axum`.

use axum::{
    extract::{FromRequest, FromRequestParts},
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::error::ApiError;

/// Cuerpo JSON de entrada o de salida.
#[derive(Debug, Clone, Copy, Default, FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct Json<T>(pub T);

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

/// Parámetros de la cadena de consulta (`?limite=50&despues=…`).
#[derive(Debug, Clone, Copy, Default, FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(ApiError))]
pub struct Query<T>(pub T);

/// Parámetros de la ruta (`/copropiedades/{copropiedad_id}`).
#[derive(Debug, Clone, Copy, FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub struct Path<T>(pub T);
