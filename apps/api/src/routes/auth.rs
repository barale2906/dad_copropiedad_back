//! Autenticación: `/api/v1/auth/…` (registro, inicio y cierre de sesión, renovación,
//! recuperación de la contraseña). Son las únicas rutas de la API, junto con la descarga
//! firmada, que no exigen sesión.

use application::{
    ContextoPlataforma,
    casos_uso::autenticacion::{
        self as uc, Registro, ResultadoInicio, SolicitudInicio, TokensSesion,
    },
    puertos::RelojSistema,
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, header},
};
use chrono::{DateTime, Utc};
use domain::{ContrasenaNueva, CuentaId, DireccionCorreo, Identificador, NombreUsuario};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::Json,
    openapi::ETIQUETA_AUTH,
    sesion::CuentaAutenticada,
    state::AppState,
};

/// Router de autenticación (se monta en `/api/v1/auth`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(registrar))
        .routes(routes!(iniciar_sesion, cerrar_sesion))
        .routes(routes!(completar_segundo_factor))
        .routes(routes!(renovar))
        .routes(routes!(solicitar_recuperacion))
        .routes(routes!(restablecer_contrasena))
}

fn request_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Datos para crear una cuenta. Se inicia sesión luego con el usuario o el correo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegistroPeticion {
    #[schema(example = "Ana María Pérez")]
    pub nombre: String,
    /// 3 a 30 caracteres `a-z`, `0-9`, `.`, `_`, `-`; opcional si hay correo.
    #[schema(example = "ana.perez")]
    pub usuario: Option<String>,
    /// Opcional si hay usuario; necesario para recuperar la contraseña.
    #[schema(example = "ana@correo.co")]
    pub correo: Option<String>,
    /// De 10 a 128 caracteres.
    pub contrasena: String,
}

/// Cuenta creada.
#[derive(Debug, Serialize, ToSchema)]
pub struct CuentaCreada {
    pub id: Uuid,
}

/// Crea una cuenta. Si trae correo, envía un código para verificarlo.
#[utoipa::path(
    post,
    path = "/registro",
    tag = ETIQUETA_AUTH,
    request_body = RegistroPeticion,
    responses(
        (status = CREATED, description = "Cuenta creada", body = CuentaCreada),
        (status = CONFLICT, description = "Usuario o correo ya registrados (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(p): Json<RegistroPeticion>,
) -> ApiResult<(StatusCode, Json<CuentaCreada>)> {
    let registro = Registro {
        nombre_mostrar: p.nombre,
        usuario: p.usuario.as_deref().map(NombreUsuario::new).transpose()?,
        correo: p.correo.as_deref().map(DireccionCorreo::new).transpose()?,
        contrasena: ContrasenaNueva::new(&p.contrasena)?,
    };
    let id: CuentaId = uc::registrar_cuenta(
        &state.plataforma,
        &state.contrasenas,
        &RelojSistema,
        &ContextoPlataforma::anonimo(request_id(&headers)),
        registro,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(CuentaCreada { id: id.as_uuid() })))
}

/// Inicio de sesión.
#[derive(Debug, Deserialize, ToSchema)]
pub struct InicioPeticion {
    /// Nombre de usuario o correo.
    #[schema(example = "ana.perez")]
    pub identificador: String,
    pub contrasena: String,
    /// Equipo compartido (portería): la sesión dura un turno (12 h).
    #[serde(default)]
    pub equipo_compartido: bool,
}

/// Tokens de una sesión. Se muestran **una sola vez**: el servidor solo guarda su hash.
#[derive(Debug, Serialize, ToSchema)]
pub struct TokensRespuesta {
    pub sesion_id: Uuid,
    /// Va en `Authorization: Bearer …`; dura 30 minutos.
    pub token_acceso: String,
    pub acceso_expira_en: DateTime<Utc>,
    /// Se cambia por tokens nuevos en `POST /auth/sesion/renovar`; cada uso lo reemplaza.
    pub token_renovacion: String,
    /// Después de esto hay que volver a iniciar sesión (30 días; 12 h en equipo compartido).
    pub expira_maximo_en: DateTime<Utc>,
}

impl From<TokensSesion> for TokensRespuesta {
    fn from(t: TokensSesion) -> Self {
        Self {
            sesion_id: t.sesion_id.as_uuid(),
            token_acceso: t.acceso,
            acceso_expira_en: t.acceso_expira_en,
            token_renovacion: t.refresh,
            expira_maximo_en: t.expira_maximo_en,
        }
    }
}

/// Resultado del inicio de sesión: los tokens, o un desafío si la cuenta tiene segundo factor.
#[derive(Debug, Serialize, ToSchema)]
pub struct InicioRespuesta {
    /// `sesion` o `segundo_factor`.
    #[schema(example = "sesion")]
    pub resultado: &'static str,
    pub tokens: Option<TokensRespuesta>,
    /// Con `segundo_factor`: enviarlo con el código TOTP a `POST /auth/sesion/segundo-factor`.
    pub desafio: Option<String>,
    pub desafio_expira_en: Option<DateTime<Utc>>,
}

/// Inicia sesión con usuario o correo y contraseña. Tras 5 fallos seguidos la cuenta se bloquea
/// 15 minutos.
#[utoipa::path(
    post,
    path = "/sesion",
    tag = ETIQUETA_AUTH,
    request_body = InicioPeticion,
    responses(
        (status = OK, description = "Sesión iniciada o segundo factor pendiente", body = InicioRespuesta),
        (status = UNAUTHORIZED, description = "Credenciales incorrectas (`credenciales_invalidas`)", body = ErrorRespuesta),
        (status = TOO_MANY_REQUESTS, description = "Cuenta bloqueada temporalmente (`demasiados_intentos`)", body = ErrorRespuesta),
    )
)]
pub async fn iniciar_sesion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(p): Json<InicioPeticion>,
) -> ApiResult<Json<InicioRespuesta>> {
    let Ok(identificador) = Identificador::parse(&p.identificador) else {
        return Err(application::AppError::CredencialesInvalidas.into());
    };
    let agente = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let resultado = uc::iniciar_sesion(
        &state.plataforma,
        &state.contrasenas,
        &RelojSistema,
        &ContextoPlataforma::anonimo(request_id(&headers)),
        SolicitudInicio {
            identificador,
            contrasena: p.contrasena,
            equipo_compartido: p.equipo_compartido,
            agente,
        },
    )
    .await?;
    Ok(Json(match resultado {
        ResultadoInicio::Sesion(t) => InicioRespuesta {
            resultado: "sesion",
            tokens: Some(t.into()),
            desafio: None,
            desafio_expira_en: None,
        },
        ResultadoInicio::SegundoFactor { desafio, expira_en } => InicioRespuesta {
            resultado: "segundo_factor",
            tokens: None,
            desafio: Some(desafio),
            desafio_expira_en: Some(expira_en),
        },
    }))
}

/// Código TOTP para completar el inicio de sesión.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SegundoFactorPeticion {
    pub desafio: String,
    #[schema(example = "123456")]
    pub codigo: String,
}

/// Completa el inicio de sesión con el código de la app autenticadora.
#[utoipa::path(
    post,
    path = "/sesion/segundo-factor",
    tag = ETIQUETA_AUTH,
    request_body = SegundoFactorPeticion,
    responses(
        (status = OK, description = "Sesión iniciada", body = TokensRespuesta),
        (status = UNAUTHORIZED, description = "Desafío vencido o código incorrecto", body = ErrorRespuesta),
    )
)]
pub async fn completar_segundo_factor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(p): Json<SegundoFactorPeticion>,
) -> ApiResult<Json<TokensRespuesta>> {
    let tokens = uc::completar_segundo_factor(
        &state.plataforma,
        state.totp.as_ref(),
        &RelojSistema,
        &ContextoPlataforma::anonimo(request_id(&headers)),
        &p.desafio,
        &p.codigo,
    )
    .await?;
    Ok(Json(tokens.into()))
}

/// Token de renovación.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RenovarPeticion {
    pub token_renovacion: String,
}

/// Cambia el token de renovación por tokens nuevos. Si llega uno ya usado, se cierra la sesión
/// (alguien lo copió).
#[utoipa::path(
    post,
    path = "/sesion/renovar",
    tag = ETIQUETA_AUTH,
    request_body = RenovarPeticion,
    responses(
        (status = OK, description = "Tokens nuevos", body = TokensRespuesta),
        (status = UNAUTHORIZED, description = "Token inválido, vencido o reutilizado (`no_autenticado`)", body = ErrorRespuesta),
    )
)]
pub async fn renovar(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(p): Json<RenovarPeticion>,
) -> ApiResult<Json<TokensRespuesta>> {
    let tokens = uc::renovar_sesion(
        &state.plataforma,
        &RelojSistema,
        &ContextoPlataforma::anonimo(request_id(&headers)),
        &p.token_renovacion,
    )
    .await?;
    Ok(Json(tokens.into()))
}

/// Cierra la sesión actual.
#[utoipa::path(
    delete,
    path = "/sesion",
    tag = ETIQUETA_AUTH,
    security(("bearer" = [])),
    responses(
        (status = NO_CONTENT, description = "Sesión cerrada"),
        (status = UNAUTHORIZED, description = "Sin sesión (`no_autenticado`)", body = ErrorRespuesta),
    )
)]
pub async fn cerrar_sesion(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
) -> ApiResult<StatusCode> {
    uc::cerrar_sesion(
        &state.plataforma,
        &RelojSistema,
        &cuenta.ctx,
        cuenta.sesion.sesion_id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Correo de la cuenta que olvidó la contraseña.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RecuperarPeticion {
    #[schema(example = "ana@correo.co")]
    pub correo: String,
}

/// Envía un código para restablecer la contraseña. Responde igual exista o no la cuenta.
#[utoipa::path(
    post,
    path = "/contrasena/recuperar",
    tag = ETIQUETA_AUTH,
    request_body = RecuperarPeticion,
    responses((status = ACCEPTED, description = "Si la cuenta existe, se envió el código"))
)]
pub async fn solicitar_recuperacion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(p): Json<RecuperarPeticion>,
) -> ApiResult<StatusCode> {
    if let Ok(correo) = DireccionCorreo::new(&p.correo) {
        uc::solicitar_recuperacion(
            &state.plataforma,
            &RelojSistema,
            &ContextoPlataforma::anonimo(request_id(&headers)),
            &correo,
        )
        .await?;
    }
    Ok(StatusCode::ACCEPTED)
}

/// Código recibido y contraseña nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RestablecerPeticion {
    pub correo: String,
    #[schema(example = "123456")]
    pub codigo: String,
    pub contrasena_nueva: String,
}

/// Restablece la contraseña con el código del correo y cierra todas las sesiones.
#[utoipa::path(
    post,
    path = "/contrasena/restablecer",
    tag = ETIQUETA_AUTH,
    request_body = RestablecerPeticion,
    responses(
        (status = NO_CONTENT, description = "Contraseña cambiada"),
        (status = UNPROCESSABLE_ENTITY, description = "Código inválido o contraseña débil (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn restablecer_contrasena(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(p): Json<RestablecerPeticion>,
) -> ApiResult<StatusCode> {
    let correo = DireccionCorreo::new(&p.correo)?;
    let nueva = ContrasenaNueva::new(&p.contrasena_nueva)?;
    uc::restablecer_contrasena(
        &state.plataforma,
        &state.contrasenas,
        &RelojSistema,
        &ContextoPlataforma::anonimo(request_id(&headers)),
        &correo,
        &p.codigo,
        &nueva,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
