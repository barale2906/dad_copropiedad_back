//! Error HTTP unificado: único lugar donde los errores de aplicación y dominio se traducen a
//! códigos HTTP (AGENTS.md §5).
//!
//! Toda respuesta de error tiene la forma [`ErrorRespuesta`]:
//! `{ "error": { "codigo": "...", "mensaje": "..." } }`. El `codigo` es estable y forma parte
//! del contrato con los clientes (web y apps móviles); el `mensaje` es legible y puede cambiar.

use application::AppError;
use axum::{
    Json,
    extract::rejection::{JsonRejection, PathRejection, QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use domain::DominioError;
use serde::Serialize;
use utoipa::ToSchema;

/// Catálogo de códigos de error de la API. **Contrato**: no renombrar ni reutilizar un código
/// con otro significado; agregar uno nuevo es un cambio compatible.
///
/// | Código | HTTP | Significado |
/// |---|---|---|
/// | `peticion_invalida` | 400 | Petición mal formada (JSON inválido, parámetro con tipo incorrecto). |
/// | `no_autenticado` | 401 | Falta la sesión o venció (`Authorization: Bearer …`). |
/// | `credenciales_invalidas` | 401 | Usuario/correo o contraseña (o código) incorrectos. |
/// | `sin_permiso` | 403 | La cuenta no tiene el permiso necesario en esta copropiedad o en la plataforma. |
/// | `segundo_factor_requerido` | 403 | La operación de plataforma exige una sesión con segundo factor. |
/// | `conflicto` | 409 | Ya existe (usuario, correo, código de rol) o rompe una regla (último administrador). |
/// | `demasiados_intentos` | 429 | Demasiados intentos fallidos; esperar unos minutos. |
/// | `demasiadas_peticiones` | 429 | Superó el límite de peticiones por minuto de la ruta; reintentar tras `Retry-After`. |
/// | `no_encontrado` | 404 | El recurso o la ruta no existen (o no pertenecen a la copropiedad). |
/// | `conflicto_estado` | 409 | La operación no es válida en el estado actual del recurso. |
/// | `ya_voto` | 409 | El bien privado ya votó en esta pregunta. |
/// | `no_inscrito` | 403 | La persona no representa al bien privado en la reunión. |
/// | `enlace_invalido` | 403 | Enlace de descarga con firma inválida o vencido. |
/// | `modulo_no_habilitado` | 403 | La copropiedad no tiene contratado el módulo. |
/// | `validacion` | 422 | Datos bien formados que violan una regla de negocio. |
/// | `idempotencia_reusada` | 422 | La `Idempotency-Key` ya se usó con otra petición (otro método, ruta o cuerpo). |
/// | `coeficientes_no_suman_uno` | 422 | El cuadro de coeficientes no suma exactamente 1 (el mensaje trae la diferencia). |
/// | `regla_contable` | 422 | Incumple una regla contable: partida doble, periodo no abierto, cuenta que no admite la línea, plantilla inválida (el mensaje dice cuál). |
/// | `interno` | 500 | Error inesperado; el detalle queda en logs con el `x-request-id`. |
/// | `no_disponible` | 503 | Una dependencia (p. ej. la base de datos) no está disponible. |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodigoError {
    /// 400 · La petición está mal formada (JSON inválido, parámetro con tipo incorrecto).
    PeticionInvalida,
    /// 401 · Falta la sesión o venció.
    NoAutenticado,
    /// 401 · Usuario/correo o contraseña (o código) incorrectos.
    CredencialesInvalidas,
    /// 403 · La cuenta no tiene el permiso necesario.
    SinPermiso,
    /// 403 · La operación de plataforma exige una sesión con segundo factor.
    SegundoFactorRequerido,
    /// 409 · El recurso ya existe o la operación rompe una regla del estado actual.
    Conflicto,
    /// 429 · Demasiados intentos fallidos.
    DemasiadosIntentos,
    /// 429 · Demasiadas peticiones por minuto desde la misma IP (cabecera `Retry-After`).
    DemasiadasPeticiones,
    /// 404 · El recurso o la ruta no existen (o no pertenecen a la copropiedad).
    NoEncontrado,
    /// 409 · La operación no es válida en el estado actual del recurso.
    ConflictoEstado,
    /// 409 · El bien privado ya votó en esta pregunta.
    YaVoto,
    /// 403 · La persona no está inscrita como representante del bien privado en la reunión.
    NoInscrito,
    /// 403 · El enlace de descarga tiene una firma inválida o ya venció: hay que pedir otro.
    EnlaceInvalido,
    /// 403 · La copropiedad no tiene contratado el módulo de esta ruta.
    ModuloNoHabilitado,
    /// 422 · Los datos son sintácticamente correctos pero violan una regla de negocio.
    Validacion,
    /// 422 · La `Idempotency-Key` ya se usó con otra petición.
    IdempotenciaReusada,
    /// 422 · El cuadro de coeficientes no suma exactamente 1; el mensaje indica la diferencia.
    CoeficientesNoSumanUno,
    /// 422 · Incumple una regla contable (partida doble, periodo, cuentas, plantilla).
    ReglaContable,
    /// 500 · Error inesperado; el detalle queda en los logs con el `x-request-id`.
    Interno,
    /// 503 · Una dependencia (p. ej. la base de datos) no está disponible.
    NoDisponible,
}

/// Cuerpo de toda respuesta de error.
#[derive(Debug, Serialize, ToSchema)]
#[schema(example = json!({ "error": { "codigo": "no_encontrado", "mensaje": "ruta no encontrada" } }))]
pub struct ErrorRespuesta {
    pub error: DetalleError,
}

/// Código estable y mensaje legible del error.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleError {
    pub codigo: CodigoError,
    /// Mensaje para mostrar o registrar; nunca incluye detalles internos.
    pub mensaje: String,
}

/// Errores que un handler puede devolver; se convierten en [`ErrorRespuesta`].
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    App(#[from] AppError),

    /// Petición mal formada: cuerpo, parámetros de ruta o de consulta inválidos.
    #[error("petición inválida: {0}")]
    PeticionInvalida(String),

    #[error("ruta no encontrada")]
    RutaNoEncontrada,

    /// Mismo mensaje para firma inválida y enlace vencido: no da pistas a quien lo manipula.
    #[error("el enlace de descarga no es válido o ya venció; solicite uno nuevo")]
    EnlaceInvalido,

    #[error("servicio no disponible: {0}")]
    NoDisponible(&'static str),

    /// Límite de peticiones por minuto superado; `segundos` va en `Retry-After`.
    #[error("demasiadas peticiones; intente de nuevo en {segundos} s")]
    DemasiadasPeticiones { segundos: u64 },

    /// La `Idempotency-Key` ya se usó con otra petición (plan 10, fase 6).
    #[error("la clave `Idempotency-Key` ya se usó con otra petición; use una clave nueva")]
    IdempotenciaReusada,

    /// Errores inesperados. El detalle se registra en logs y **no** se expone al cliente.
    #[error(transparent)]
    Interno(#[from] anyhow::Error),
}

impl From<DominioError> for ApiError {
    fn from(e: DominioError) -> Self {
        Self::App(AppError::Dominio(e))
    }
}

impl From<JsonRejection> for ApiError {
    fn from(e: JsonRejection) -> Self {
        Self::PeticionInvalida(e.body_text())
    }
}

impl From<QueryRejection> for ApiError {
    fn from(e: QueryRejection) -> Self {
        Self::PeticionInvalida(e.body_text())
    }
}

impl From<PathRejection> for ApiError {
    fn from(e: PathRejection) -> Self {
        Self::PeticionInvalida(e.body_text())
    }
}

impl ApiError {
    /// Estado HTTP y código estable de cada error.
    pub fn status_y_codigo(&self) -> (StatusCode, CodigoError) {
        use CodigoError as C;
        match self {
            Self::IdempotenciaReusada => (StatusCode::UNPROCESSABLE_ENTITY, C::IdempotenciaReusada),
            Self::App(AppError::Dominio(d)) => match d {
                DominioError::TransicionInvalida { .. }
                | DominioError::PreguntaNoAbierta(_)
                | DominioError::ReunionNoInstalada(_) => (StatusCode::CONFLICT, C::ConflictoEstado),
                DominioError::OpcionInvalida(_)
                | DominioError::CoeficienteInvalido(_)
                | DominioError::ClaveIdempotenciaInvalida(_)
                | DominioError::CorreoInvalido
                | DominioError::ArchivoVacio
                | DominioError::TipoArchivoNoAdmitido
                | DominioError::ArchivoDemasiadoGrande { .. }
                | DominioError::NombreArchivoInvalido
                | DominioError::ClaveArchivoInvalida
                | DominioError::NombreUsuarioInvalido
                | DominioError::ContrasenaDebil
                | DominioError::NitInvalido
                | DominioError::PermisoDeOtroAmbito(_)
                | DominioError::CuadroIncompleto { .. }
                | DominioError::TitularesInvalidos(_)
                | DominioError::PlantillaInvalida(_)
                | DominioError::DatoInvalido { .. }
                | DominioError::ReservaNoPermitida(_)
                | DominioError::VotoInvalido(_)
                | DominioError::ValorDesconocido { .. } => {
                    (StatusCode::UNPROCESSABLE_ENTITY, C::Validacion)
                }
                DominioError::CoeficientesNoSumanUno { .. } => {
                    (StatusCode::UNPROCESSABLE_ENTITY, C::CoeficientesNoSumanUno)
                }
                DominioError::ReglaContable(_) => {
                    (StatusCode::UNPROCESSABLE_ENTITY, C::ReglaContable)
                }
            },
            Self::App(AppError::Validacion(_)) => (StatusCode::UNPROCESSABLE_ENTITY, C::Validacion),
            Self::App(AppError::NoEncontrado(_)) | Self::RutaNoEncontrada => {
                (StatusCode::NOT_FOUND, C::NoEncontrado)
            }
            Self::App(AppError::BienPrivadoYaVoto) => (StatusCode::CONFLICT, C::YaVoto),
            Self::App(AppError::NoInscrito) => (StatusCode::FORBIDDEN, C::NoInscrito),
            Self::EnlaceInvalido => (StatusCode::FORBIDDEN, C::EnlaceInvalido),
            Self::App(AppError::NoAutenticado) => (StatusCode::UNAUTHORIZED, C::NoAutenticado),
            Self::App(AppError::CredencialesInvalidas) => {
                (StatusCode::UNAUTHORIZED, C::CredencialesInvalidas)
            }
            Self::App(AppError::SinPermiso(_)) => (StatusCode::FORBIDDEN, C::SinPermiso),
            Self::App(AppError::SegundoFactorRequerido) => {
                (StatusCode::FORBIDDEN, C::SegundoFactorRequerido)
            }
            Self::App(AppError::Conflicto(_)) => (StatusCode::CONFLICT, C::Conflicto),
            Self::App(AppError::DemasiadosIntentos) => {
                (StatusCode::TOO_MANY_REQUESTS, C::DemasiadosIntentos)
            }
            Self::App(AppError::ModuloNoHabilitado(_)) => {
                (StatusCode::FORBIDDEN, C::ModuloNoHabilitado)
            }
            Self::PeticionInvalida(_) => (StatusCode::BAD_REQUEST, C::PeticionInvalida),
            Self::App(AppError::Infraestructura(_)) | Self::Interno(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, C::Interno)
            }
            Self::NoDisponible(_) => (StatusCode::SERVICE_UNAVAILABLE, C::NoDisponible),
            Self::DemasiadasPeticiones { .. } => {
                (StatusCode::TOO_MANY_REQUESTS, C::DemasiadasPeticiones)
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, codigo) = self.status_y_codigo();
        let reintento = match &self {
            Self::DemasiadasPeticiones { segundos } => Some(*segundos),
            _ => None,
        };
        let mensaje = if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!(error = ?self, "error interno");
            "error interno del servidor".to_owned()
        } else {
            self.to_string()
        };
        let cuerpo = ErrorRespuesta {
            error: DetalleError { codigo, mensaje },
        };
        let mut respuesta = (status, Json(cuerpo)).into_response();
        if let Some(s) = reintento
            && let Ok(v) = axum::http::HeaderValue::from_str(&s.to_string())
        {
            respuesta
                .headers_mut()
                .insert(axum::http::header::RETRY_AFTER, v);
        }
        if status == StatusCode::UNAUTHORIZED {
            respuesta.headers_mut().insert(
                axum::http::header::WWW_AUTHENTICATE,
                axum::http::HeaderValue::from_static("Bearer"),
            );
        }
        respuesta
    }
}

/// Resultado de los handlers.
pub type ApiResult<T> = Result<T, ApiError>;

/// Respuesta para rutas inexistentes: mismo formato que cualquier otro error.
pub async fn ruta_no_encontrada() -> ApiError {
    ApiError::RutaNoEncontrada
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codigos_se_serializan_en_snake_case() {
        let json = serde_json::to_value(CodigoError::PeticionInvalida).expect("serializa");
        assert_eq!(json, "peticion_invalida");
    }

    #[tokio::test]
    async fn error_interno_no_expone_detalle() {
        let res = ApiError::Interno(anyhow::anyhow!("contraseña=secreta")).into_response();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let cuerpo = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("cuerpo legible");
        let texto = String::from_utf8_lossy(&cuerpo);
        assert!(!texto.contains("secreta"), "{texto}");
        assert!(texto.contains("\"codigo\":\"interno\""), "{texto}");
    }

    #[test]
    fn validacion_de_aplicacion_es_422() {
        let (status, codigo) = ApiError::App(AppError::Validacion("x".into())).status_y_codigo();
        assert_eq!(
            (status, codigo),
            (StatusCode::UNPROCESSABLE_ENTITY, CodigoError::Validacion)
        );
    }

    #[test]
    fn errores_de_archivos_y_modulos_tienen_su_codigo() {
        let casos = [
            (
                ApiError::EnlaceInvalido,
                (StatusCode::FORBIDDEN, CodigoError::EnlaceInvalido),
            ),
            (
                ApiError::App(AppError::ModuloNoHabilitado(domain::Modulo::Asambleas)),
                (StatusCode::FORBIDDEN, CodigoError::ModuloNoHabilitado),
            ),
            (
                ApiError::from(DominioError::TipoArchivoNoAdmitido),
                (StatusCode::UNPROCESSABLE_ENTITY, CodigoError::Validacion),
            ),
            (
                ApiError::from(DominioError::CoeficientesNoSumanUno {
                    suma: rust_decimal::Decimal::ONE,
                    diferencia: rust_decimal::Decimal::ZERO,
                }),
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    CodigoError::CoeficientesNoSumanUno,
                ),
            ),
        ];
        for (error, esperado) in casos {
            assert_eq!(error.status_y_codigo(), esperado, "{error}");
        }
    }
}
