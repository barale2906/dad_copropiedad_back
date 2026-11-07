//! DTOs HTTP de la paginación por cursor ([`application::Pagina`]).
//!
//! Petición: `?limite=50&despues=<id>`. Respuesta: `{ "datos": [...], "siguiente": "<id>" | null }`.
//! Para pedir la página siguiente se envía `despues=<siguiente>`; `siguiente: null` indica que
//! no hay más.

use application::{AppResult, Pagina, SolicitudPagina};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Parámetros de consulta de un listado paginado.
#[derive(Debug, Clone, Copy, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ParametrosPagina {
    /// Elementos por página (1–200; 50 si se omite).
    #[param(minimum = 1, maximum = 200, example = 50)]
    pub limite: Option<u32>,
    /// Cursor: el valor `siguiente` de la página anterior. Se omite para la primera página.
    pub despues: Option<Uuid>,
}

impl ParametrosPagina {
    /// Valida los parámetros y los convierte al tipo de la capa de aplicación.
    ///
    /// # Errors
    ///
    /// [`application::AppError::Validacion`] si `limite` está fuera de rango.
    pub fn validar(self) -> AppResult<SolicitudPagina> {
        SolicitudPagina::new(self.limite, self.despues)
    }
}

/// Página de resultados de un listado.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaginaRespuesta<T: ToSchema> {
    /// Elementos de esta página.
    pub datos: Vec<T>,
    /// Cursor para pedir la página siguiente (`despues=<siguiente>`); `null` si es la última.
    pub siguiente: Option<Uuid>,
}

impl<T: ToSchema> From<Pagina<T>> for PaginaRespuesta<T> {
    fn from(p: Pagina<T>) -> Self {
        Self {
            datos: p.datos,
            siguiente: p.siguiente,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parametros_vacios_piden_la_primera_pagina() {
        let s = ParametrosPagina::default().validar().expect("válidos");
        assert_eq!(s, SolicitudPagina::primera());
    }

    #[test]
    fn limite_fuera_de_rango_es_error_de_validacion() {
        let p = ParametrosPagina {
            limite: Some(0),
            despues: None,
        };
        assert!(p.validar().is_err());
    }
}
