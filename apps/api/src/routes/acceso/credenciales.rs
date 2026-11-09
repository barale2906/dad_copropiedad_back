//! `/credenciales`, `/autorizaciones-acceso` y
//! `/personas/{persona_id}/consentimiento-biometrico`.

use application::{
    casos_uso::acceso::credenciales::{
        self as uc, NuevaAutorizacion, NuevaCredencial, NuevoConsentimiento,
    },
    puertos::{
        RelojSistema,
        acceso::{AutorizacionAcceso, ConsentimientoBiometrico, Credencial},
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{
    AutorizacionAccesoId, CredencialId, EstadoCredencial, PersonaId, PuntoAccesoId,
    ResultadoAcceso, TipoCredencial,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::{TitularDto, bytes_base64};
use crate::routes::ficha::HorarioDto;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_ACCESO,
    sesion::EnCopropiedad,
    state::AppState,
};

pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar, emitir))
        .routes(routes!(verificar))
        .routes(routes!(cambiar_estado))
        .routes(routes!(rotar))
        .routes(routes!(listar_autorizaciones, crear_autorizacion))
        .routes(routes!(revocar_autorizacion))
        .routes(routes!(
            ver_consentimiento,
            registrar_consentimiento,
            revocar_consentimiento
        ))
}

/// Credencial (sin el identificador: solo su huella).
#[derive(Debug, Serialize, ToSchema)]
pub struct CredencialRespuesta {
    pub id: Uuid,
    pub titular: TitularDto,
    /// `tarjeta_rfid`, `pin`, `qr_permanente`, `placa`, `biometria_ref` o `app_movil`.
    #[schema(example = "tarjeta_rfid")]
    pub tipo: String,
    /// `activa`, `suspendida`, `revocada` o `perdida`.
    #[schema(example = "activa")]
    pub estado: String,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    /// Biometría: el puente confirmó que borró la plantilla del equipo.
    pub borrado_confirmado_en: Option<DateTime<Utc>>,
}

impl From<Credencial> for CredencialRespuesta {
    fn from(c: Credencial) -> Self {
        Self {
            id: c.id.as_uuid(),
            titular: c.titular.into(),
            tipo: c.tipo.as_str().into(),
            estado: c.estado.as_str().into(),
            vigente_desde: c.vigente_desde,
            vigente_hasta: c.vigente_hasta,
            borrado_confirmado_en: c.borrado_confirmado_en,
        }
    }
}

/// Credenciales de una persona o vehículo.
#[utoipa::path(
    get,
    path = "/credenciales",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), TitularDto),
    responses(
        (status = OK, description = "Credenciales", body = Vec<CredencialRespuesta>),
        (status = BAD_REQUEST, description = "Falta `persona_id` o `vehiculo_id` (`peticion_invalida`)", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `acceso.credenciales.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(t): Query<TitularDto>,
) -> ApiResult<Json<Vec<CredencialRespuesta>>> {
    let v = uc::credenciales_de(&state.unidad, &RelojSistema, &en.ctx, t.validar()?).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Credencial nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EmitirPeticion {
    pub titular: TitularDto,
    #[schema(value_type = String, example = "tarjeta_rfid")]
    pub tipo: TipoCredencial,
    /// Número de tarjeta, PIN, placa o id de enrolamiento (se guarda solo su huella). Se omite en
    /// `qr_permanente` y `app_movil`: el texto se genera y se devuelve una sola vez.
    pub valor: Option<String>,
    pub vigente_desde: Option<DateTime<Utc>>,
    pub vigente_hasta: Option<DateTime<Utc>>,
}

/// Credencial emitida.
#[derive(Debug, Serialize, ToSchema)]
pub struct EmitidaRespuesta {
    pub credencial: CredencialRespuesta,
    /// Texto del QR permanente o de la app (se muestra **una sola vez**).
    pub secreto: Option<String>,
}

/// Emite una credencial. Las biométricas exigen `acceso.biometria.enrolar` y consentimiento.
#[utoipa::path(
    post,
    path = "/credenciales",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = EmitirPeticion,
    responses(
        (status = CREATED, description = "Credencial emitida", body = EmitidaRespuesta),
        (status = FORBIDDEN, description = "Sin `acceso.credenciales.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya hay otra vigente con ese identificador (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Formato inválido o sin consentimiento biométrico (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn emitir(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<EmitirPeticion>,
) -> ApiResult<(StatusCode, Json<EmitidaRespuesta>)> {
    let e = uc::emitir_credencial(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevaCredencial {
            titular: p.titular.validar()?,
            tipo: p.tipo,
            valor: p.valor,
            vigente_desde: p.vigente_desde,
            vigente_hasta: p.vigente_hasta,
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(EmitidaRespuesta {
            credencial: e.credencial.into(),
            secreto: e.secreto,
        }),
    ))
}

/// Lo leído en la portería.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VerificarPeticion {
    /// Texto del QR, PIN o número de tarjeta.
    pub texto: String,
    /// Tipo de lo leído (`qr_permanente` si se omite; los QR de visita se reconocen solos).
    #[schema(value_type = Option<String>, example = "qr_permanente")]
    pub tipo: Option<TipoCredencial>,
    pub punto_acceso_id: Uuid,
}

/// Resultado de la verificación.
#[derive(Debug, Serialize, ToSchema)]
pub struct VerificacionRespuesta {
    /// `permitido` o `denegado`.
    #[schema(example = "permitido")]
    pub resultado: String,
    pub motivo: String,
    pub titular: Option<TitularDto>,
    /// Visita del QR temporal: registrar el ingreso con `/visitas/ingreso-qr`.
    pub visita_id: Option<Uuid>,
}

/// Verifica lo leído en la portería digital y lo deja en la bitácora.
#[utoipa::path(
    post,
    path = "/credenciales/verificacion",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = VerificarPeticion,
    responses(
        (status = OK, description = "Resultado", body = VerificacionRespuesta),
        (status = FORBIDDEN, description = "Sin `acceso.registrar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn verificar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<VerificarPeticion>,
) -> ApiResult<Json<VerificacionRespuesta>> {
    let v = uc::verificar_en_porteria(
        &state.unidad,
        &RelojSistema,
        state.firma_qr.as_ref(),
        &en.ctx,
        &p.texto,
        p.tipo.unwrap_or(TipoCredencial::QrPermanente),
        PuntoAccesoId::from_uuid(p.punto_acceso_id),
    )
    .await?;
    Ok(Json(VerificacionRespuesta {
        resultado: match v.resultado {
            ResultadoAcceso::Permitido => "permitido",
            _ => "denegado",
        }
        .into(),
        motivo: v.motivo.into(),
        titular: v.titular.map(Into::into),
        visita_id: v.visita.map(|x| x.as_uuid()),
    }))
}

/// Estado nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EstadoPeticion {
    /// `activa` (reactivar), `suspendida`, `revocada` o `perdida`.
    #[schema(value_type = String, example = "suspendida")]
    pub estado: EstadoCredencial,
}

/// Suspende, reactiva, revoca o reporta perdida una credencial (el puente la retira en la
/// siguiente sincronización).
#[utoipa::path(
    post,
    path = "/credenciales/{credencial_id}/estado",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("credencial_id" = Uuid, Path, description = "Credencial")),
    request_body = EstadoPeticion,
    responses(
        (status = OK, description = "Credencial", body = CredencialRespuesta),
        (status = CONFLICT, description = "Transición no permitida: revocada y perdida son definitivas (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn cambiar_estado(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<EstadoPeticion>,
) -> ApiResult<Json<CredencialRespuesta>> {
    let c = uc::cambiar_estado_credencial(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CredencialId::from_uuid(ruta.1),
        p.estado,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Rota un QR permanente o de la app (revoca el actual y emite otro).
#[utoipa::path(
    post,
    path = "/credenciales/{credencial_id}/rotacion",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("credencial_id" = Uuid, Path, description = "Credencial")),
    responses(
        (status = OK, description = "Credencial nueva", body = EmitidaRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es un QR permanente ni de la app (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn rotar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<EmitidaRespuesta>> {
    let e = uc::rotar_qr(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CredencialId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(EmitidaRespuesta {
        credencial: e.credencial.into(),
        secreto: e.secreto,
    }))
}

/// Autorización manual.
#[derive(Debug, Serialize, ToSchema)]
pub struct AutorizacionAccesoRespuesta {
    pub id: Uuid,
    pub titular: TitularDto,
    /// `manual` (las de residentes y propietarios se derivan del censo y no aparecen aquí).
    pub origen: String,
    /// Puntos permitidos; vacío = todos.
    pub puntos: Vec<Uuid>,
    pub horario: Option<HorarioDto>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub motivo: Option<String>,
    pub revocada_en: Option<DateTime<Utc>>,
}

impl From<AutorizacionAcceso> for AutorizacionAccesoRespuesta {
    fn from(a: AutorizacionAcceso) -> Self {
        Self {
            id: a.id.as_uuid(),
            titular: a.titular.into(),
            origen: a.origen.as_str().into(),
            puntos: a.puntos.into_iter().map(PuntoAccesoId::as_uuid).collect(),
            horario: a.horario.as_ref().map(HorarioDto::desde),
            vigente_desde: a.vigente_desde,
            vigente_hasta: a.vigente_hasta,
            motivo: a.motivo,
            revocada_en: a.revocada_en,
        }
    }
}

/// Autorizaciones manuales de una persona o vehículo.
#[utoipa::path(
    get,
    path = "/autorizaciones-acceso",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), TitularDto),
    responses(
        (status = OK, description = "Autorizaciones", body = Vec<AutorizacionAccesoRespuesta>),
        (status = FORBIDDEN, description = "Sin `acceso.credenciales.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_autorizaciones(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(t): Query<TitularDto>,
) -> ApiResult<Json<Vec<AutorizacionAccesoRespuesta>>> {
    let v = uc::autorizaciones_de(&state.unidad, &RelojSistema, &en.ctx, t.validar()?).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Autorización nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AutorizacionAccesoPeticion {
    pub titular: TitularDto,
    /// Puntos permitidos; vacío = todos.
    #[serde(default)]
    pub puntos: Vec<Uuid>,
    /// Franjas semanales en hora local de la PH; sin horario = a toda hora.
    pub horario: Option<HorarioDto>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub motivo: Option<String>,
}

/// Crea una autorización manual (personal de servicio, proveedores frecuentes, horarios
/// especiales).
#[utoipa::path(
    post,
    path = "/autorizaciones-acceso",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = AutorizacionAccesoPeticion,
    responses(
        (status = CREATED, description = "Autorización creada", body = AutorizacionAccesoRespuesta),
        (status = NOT_FOUND, description = "El titular no existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Vigencia invertida o punto inexistente (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_autorizacion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<AutorizacionAccesoPeticion>,
) -> ApiResult<(StatusCode, Json<AutorizacionAccesoRespuesta>)> {
    let a = uc::crear_autorizacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevaAutorizacion {
            titular: p.titular.validar()?,
            puntos: p.puntos.into_iter().map(PuntoAccesoId::from_uuid).collect(),
            horario: p.horario.map(HorarioDto::a_dominio).transpose()?,
            vigente_desde: p.vigente_desde,
            vigente_hasta: p.vigente_hasta,
            motivo: p.motivo,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(a.into())))
}

/// Revoca una autorización manual.
#[utoipa::path(
    post,
    path = "/autorizaciones-acceso/{autorizacion_id}/revocacion",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("autorizacion_id" = Uuid, Path, description = "Autorización")),
    responses(
        (status = NO_CONTENT, description = "Revocada"),
        (status = NOT_FOUND, description = "No existe o ya estaba revocada", body = ErrorRespuesta),
    )
)]
pub async fn revocar_autorizacion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    uc::revocar_autorizacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        AutorizacionAccesoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Consentimiento biométrico.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConsentimientoRespuesta {
    pub id: Uuid,
    pub persona_id: Uuid,
    pub version_texto: String,
    pub soporte_archivo_id: Option<Uuid>,
    pub otorgado_en: DateTime<Utc>,
}

impl From<ConsentimientoBiometrico> for ConsentimientoRespuesta {
    fn from(c: ConsentimientoBiometrico) -> Self {
        Self {
            id: c.id.as_uuid(),
            persona_id: c.persona_id.as_uuid(),
            version_texto: c.version_texto,
            soporte_archivo_id: c.soporte_archivo_id.map(|a| a.as_uuid()),
            otorgado_en: c.otorgado_en,
        }
    }
}

fn persona(ruta: (Uuid, Uuid)) -> PersonaId {
    PersonaId::from_uuid(ruta.1)
}

/// Consentimiento biométrico vigente (o `null`).
#[utoipa::path(
    get,
    path = "/personas/{persona_id}/consentimiento-biometrico",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("persona_id" = Uuid, Path, description = "Persona")),
    responses(
        (status = OK, description = "Consentimiento vigente", body = Option<ConsentimientoRespuesta>),
        (status = FORBIDDEN, description = "Sin `acceso.biometria.enrolar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn ver_consentimiento(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Option<ConsentimientoRespuesta>>> {
    let c = uc::consentimiento_de(&state.unidad, &RelojSistema, &en.ctx, persona(ruta)).await?;
    Ok(Json(c.map(Into::into)))
}

/// Consentimiento nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ConsentimientoPeticion {
    /// Versión del texto de autorización que la persona aceptó.
    #[schema(example = "v1-2026")]
    pub version_texto: String,
    /// Soporte firmado (PDF o foto) en Base64.
    pub soporte_base64: Option<String>,
}

/// Registra el consentimiento biométrico explícito (Ley 1581 de 2012: facultativo, con método
/// alterno siempre disponible).
#[utoipa::path(
    post,
    path = "/personas/{persona_id}/consentimiento-biometrico",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("persona_id" = Uuid, Path, description = "Persona")),
    request_body = ConsentimientoPeticion,
    responses(
        (status = CREATED, description = "Consentimiento registrado", body = ConsentimientoRespuesta),
        (status = FORBIDDEN, description = "Sin `acceso.biometria.enrolar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya tiene uno vigente (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_consentimiento(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ConsentimientoPeticion>,
) -> ApiResult<(StatusCode, Json<ConsentimientoRespuesta>)> {
    let c = uc::registrar_consentimiento(
        &state.unidad,
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        NuevoConsentimiento {
            persona_id: persona(ruta),
            version_texto: p.version_texto,
            soporte: bytes_base64("soporte_base64", p.soporte_base64.as_deref())?,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Credenciales biométricas revocadas.
#[derive(Debug, Serialize, ToSchema)]
pub struct RevocacionRespuesta {
    pub credenciales_revocadas: u32,
}

/// Revoca el consentimiento (la persona misma o quien enrola): las credenciales biométricas se
/// revocan y el puente borra las plantillas del equipo.
#[utoipa::path(
    delete,
    path = "/personas/{persona_id}/consentimiento-biometrico",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("persona_id" = Uuid, Path, description = "Persona")),
    responses(
        (status = OK, description = "Consentimiento revocado", body = RevocacionRespuesta),
        (status = NOT_FOUND, description = "No tiene consentimiento vigente", body = ErrorRespuesta),
    )
)]
pub async fn revocar_consentimiento(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<RevocacionRespuesta>> {
    let n =
        uc::revocar_consentimiento(&state.unidad, &RelojSistema, &en.ctx, persona(ruta)).await?;
    Ok(Json(RevocacionRespuesta {
        credenciales_revocadas: n,
    }))
}
