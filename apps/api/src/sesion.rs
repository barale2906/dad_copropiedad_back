//! Extractores de autenticación y autorización.
//!
//! - [`CuentaAutenticada`]: exige `Authorization: Bearer <token de acceso>` válido. Para
//!   `/api/v1/yo/…` y lo que hace una cuenta por sí misma.
//! - [`OperadorPlataforma`]: además exige segundo factor (si `PLATAFORMA_EXIGE_SEGUNDO_FACTOR`).
//!   Para `/api/v1/plataforma/…`.
//! - [`EnCopropiedad`]: toma `{copropiedad_id}` de la ruta y arma el [`Contexto`] con los
//!   permisos efectivos de la cuenta en esa PH. Si no tiene ningún rol allí responde 404 (no se
//!   revela si la copropiedad existe). El superusuario tiene todos los permisos en todas.

use application::{
    AppError, Contexto, ContextoPlataforma,
    casos_uso::{
        autenticacion::{SesionAutenticada, autenticar},
        roles::permisos_en_copropiedad,
    },
    puertos::RelojSistema,
};
use axum::{
    extract::{FromRequestParts, RawPathParams},
    http::{header, request::Parts},
};
use domain::TenantId;
use uuid::Uuid;

use crate::{error::ApiError, routes::PARAMETRO_COPROPIEDAD, state::AppState};

fn request_id(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn token(parts: &Parts) -> Result<&str, ApiError> {
    parts
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or(ApiError::App(AppError::NoAutenticado))
}

/// Cuenta con sesión válida.
#[derive(Debug, Clone)]
pub struct CuentaAutenticada {
    pub sesion: SesionAutenticada,
    /// Contexto de plataforma de la cuenta (sus permisos de plataforma, si tiene).
    pub ctx: ContextoPlataforma,
}

impl FromRequestParts<AppState> for CuentaAutenticada {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let sesion = autenticar(&state.plataforma, &RelojSistema, token(parts)?).await?;
        let ctx = ContextoPlataforma::de_cuenta(
            sesion.cuenta_id,
            sesion.permisos_plataforma.clone(),
            request_id(parts),
        );
        Ok(Self { sesion, ctx })
    }
}

/// Cuenta que opera en el ámbito de plataforma (con segundo factor si se exige).
#[derive(Debug, Clone)]
pub struct OperadorPlataforma(pub CuentaAutenticada);

impl FromRequestParts<AppState> for OperadorPlataforma {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let cuenta = CuentaAutenticada::from_request_parts(parts, state).await?;
        if state.plataforma_exige_segundo_factor && !cuenta.sesion.segundo_factor_verificado {
            return Err(ApiError::App(AppError::SegundoFactorRequerido));
        }
        Ok(Self(cuenta))
    }
}

/// Cuenta autenticada dentro de una copropiedad, con sus permisos allí.
#[derive(Debug, Clone)]
pub struct EnCopropiedad {
    pub sesion: SesionAutenticada,
    pub ctx: Contexto,
}

impl FromRequestParts<AppState> for EnCopropiedad {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let params = RawPathParams::from_request_parts(parts, state)
            .await
            .map_err(|e| ApiError::PeticionInvalida(e.body_text()))?;
        let tenant = params
            .iter()
            .find(|(k, _)| *k == PARAMETRO_COPROPIEDAD)
            .and_then(|(_, v)| Uuid::try_parse(v).ok())
            .map(TenantId::from_uuid)
            .ok_or_else(|| ApiError::PeticionInvalida("copropiedad_id inválido".into()))?;
        let cuenta = CuentaAutenticada::from_request_parts(parts, state).await?;
        let permisos = permisos_en_copropiedad(
            &state.unidad,
            &RelojSistema,
            tenant,
            cuenta.sesion.cuenta_id,
            cuenta.sesion.es_superusuario,
        )
        .await?;
        if permisos.es_vacio() {
            // Sin vínculo con la copropiedad: para esa cuenta "no existe" (invariante 1).
            return Err(ApiError::App(AppError::NoEncontrado("copropiedad")));
        }
        let ctx = Contexto::de_cuenta(tenant, cuenta.sesion.cuenta_id, permisos)
            .con_request_id(request_id(parts));
        Ok(Self {
            sesion: cuenta.sesion,
            ctx,
        })
    }
}
