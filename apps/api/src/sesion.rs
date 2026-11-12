//! Extractores de autenticación y autorización.
//!
//! - [`CuentaAutenticada`]: exige `Authorization: Bearer <token de acceso>` válido. Para
//!   `/api/v1/yo/…` y lo que hace una cuenta por sí misma.
//! - [`OperadorPlataforma`]: además exige segundo factor (si `PLATAFORMA_EXIGE_SEGUNDO_FACTOR`).
//!   Para `/api/v1/plataforma/…`.
//! - [`EnCopropiedad`]: toma `{copropiedad_id}` de la ruta y arma el [`Contexto`] con los
//!   permisos efectivos de la cuenta en esa PH. Si no tiene ningún rol allí responde 404 (no se
//!   revela si la copropiedad existe). El superusuario tiene todos los permisos en todas.
//! - [`EnCopropiedadWs`]: igual, para abrir un WebSocket desde el navegador (que no puede
//!   enviar `Authorization`): el token va en `Sec-WebSocket-Protocol: dad.v1, <token>`.
//! - [`PapeletaVirtual`]: la papeleta del asistente (cabecera `X-Papeleta`): el token es la
//!   credencial (como el enlace firmado de una descarga); no hay sesión.
//! - [`EnlaceDirectorioPublico`]: el directorio público de la PH para los grupos de WhatsApp
//!   (cabecera `X-Enlace-Directorio`); tampoco hay sesión.
//! - [`PantallaLobby`]: la pantalla de publicidad del lobby de la PH (cabecera `X-Pantalla`).

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

pub(crate) fn token(parts: &Parts) -> Result<&str, ApiError> {
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

async fn tenant_de_ruta(parts: &mut Parts, state: &AppState) -> Result<TenantId, ApiError> {
    let params = RawPathParams::from_request_parts(parts, state)
        .await
        .map_err(|e| ApiError::PeticionInvalida(e.body_text()))?;
    params
        .iter()
        .find(|(k, _)| *k == PARAMETRO_COPROPIEDAD)
        .and_then(|(_, v)| Uuid::try_parse(v).ok())
        .map(TenantId::from_uuid)
        .ok_or_else(|| ApiError::PeticionInvalida("copropiedad_id inválido".into()))
}

impl FromRequestParts<AppState> for EnCopropiedad {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let tenant = tenant_de_ruta(parts, state).await?;
        let cuenta = CuentaAutenticada::from_request_parts(parts, state).await?;
        en_copropiedad(parts, state, tenant, cuenta).await
    }
}

async fn en_copropiedad(
    parts: &Parts,
    state: &AppState,
    tenant: TenantId,
    cuenta: CuentaAutenticada,
) -> Result<EnCopropiedad, ApiError> {
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
    Ok(EnCopropiedad {
        sesion: cuenta.sesion,
        ctx,
    })
}

/// Subprotocolo del WebSocket del tablero en vivo.
pub const SUBPROTOCOLO_WS: &str = "dad.v1";

/// [`EnCopropiedad`] para abrir un WebSocket: acepta el token en `Authorization` o como
/// segundo valor de `Sec-WebSocket-Protocol` (`dad.v1, <token>`), que los navegadores sí
/// permiten enviar. Nunca en la URL (quedaría en los logs).
#[derive(Debug, Clone)]
pub struct EnCopropiedadWs(pub EnCopropiedad);

impl FromRequestParts<AppState> for EnCopropiedadWs {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let tenant = tenant_de_ruta(parts, state).await?;
        let token = match token(parts) {
            Ok(t) => t.to_owned(),
            Err(e) => parts
                .headers
                .get(header::SEC_WEBSOCKET_PROTOCOL)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| {
                    v.split(',')
                        .map(str::trim)
                        .find(|p| !p.is_empty() && *p != SUBPROTOCOLO_WS)
                        .map(str::to_owned)
                })
                .ok_or(e)?,
        };
        let sesion = autenticar(&state.plataforma, &RelojSistema, &token).await?;
        let ctx = ContextoPlataforma::de_cuenta(
            sesion.cuenta_id,
            sesion.permisos_plataforma.clone(),
            request_id(parts),
        );
        let cuenta = CuentaAutenticada { sesion, ctx };
        Ok(Self(en_copropiedad(parts, state, tenant, cuenta).await?))
    }
}

/// Cabecera con el token de la papeleta virtual.
pub const CABECERA_PAPELETA: &str = "x-papeleta";

/// Papeleta virtual de un asistente: la copropiedad de la ruta y el token de su enlace. El
/// contexto es el del sistema de esa copropiedad; la identidad la prueba el token.
#[derive(Debug, Clone)]
pub struct PapeletaVirtual {
    pub ctx: Contexto,
    pub token: String,
}

impl FromRequestParts<AppState> for PapeletaVirtual {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let tenant = tenant_de_ruta(parts, state).await?;
        Ok(Self {
            ctx: Contexto::new(tenant).con_request_id(request_id(parts)),
            token: token_de_cabecera(parts, CABECERA_PAPELETA)?,
        })
    }
}

/// Token no vacío (hasta 200 caracteres) de una cabecera; si falta, `no_autenticado`.
fn token_de_cabecera(parts: &Parts, cabecera: &str) -> Result<String, ApiError> {
    parts
        .headers
        .get(cabecera)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|t| !t.is_empty() && t.len() <= 200)
        .map(str::to_owned)
        .ok_or(ApiError::App(AppError::NoAutenticado))
}

/// Cabecera con el token del enlace público del directorio.
pub const CABECERA_ENLACE_DIRECTORIO: &str = "x-enlace-directorio";

/// Directorio público de una copropiedad: la copropiedad de la ruta y el token de su enlace
/// (el caso de uso lo compara con el vigente).
#[derive(Debug, Clone)]
pub struct EnlaceDirectorioPublico {
    pub tenant: TenantId,
    pub token: String,
}

impl FromRequestParts<AppState> for EnlaceDirectorioPublico {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        Ok(Self {
            tenant: tenant_de_ruta(parts, state).await?,
            token: token_de_cabecera(parts, CABECERA_ENLACE_DIRECTORIO)?,
        })
    }
}

/// Cabecera con el token de la pantalla del lobby.
pub const CABECERA_PANTALLA: &str = "x-pantalla";

/// Pantalla del lobby de una copropiedad: la copropiedad de la ruta y su token.
#[derive(Debug, Clone)]
pub struct PantallaLobby {
    pub tenant: TenantId,
    pub token: String,
}

impl FromRequestParts<AppState> for PantallaLobby {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        Ok(Self {
            tenant: tenant_de_ruta(parts, state).await?,
            token: token_de_cabecera(parts, CABECERA_PANTALLA)?,
        })
    }
}
