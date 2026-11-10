//! API de los puentes de acceso (`/api/v1/puentes`, sin sesión de persona): el puente se
//! autentica con su **token de máquina** en `Authorization: Bearer`, sincroniza credenciales y
//! visitas desde su cursor, reporta eventos (idempotente por `id_local`) y envía su latido.

use application::{
    casos_uso::acceso::puentes::{self as uc, EventoReportado, PuenteAutenticado},
    puertos::{
        RelojSistema,
        acceso::{CredencialSincronizada, VisitaSincronizada},
    },
};
use axum::{
    extract::{DefaultBodyLimit, FromRequestParts, State},
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};
use chrono::{DateTime, Utc};
use domain::{
    CredencialId, DispositivoAccesoId, PuntoAccesoId, ResultadoAcceso, SentidoAcceso, VisitaId,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::{
    LIMITE_CUERPO,
    dto::bytes_base64,
    infraestructura::{DispositivoDto, PuntoDto},
};
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Query},
    limite::LimitePublico,
    openapi::ETIQUETA_ACCESO,
    state::AppState,
};

/// Rutas de `/api/v1/puentes`.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(sincronizacion))
        .routes(routes!(eventos))
        .routes(routes!(latido))
        .layer(DefaultBodyLimit::max(LIMITE_CUERPO))
}

/// Puente autenticado por su token de máquina.
pub struct PuenteMaquina(PuenteAutenticado);

impl FromRequestParts<AppState> for PuenteMaquina {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(ApiError::App(application::AppError::NoAutenticado))?;
        let request_id = parts
            .headers
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        Ok(Self(
            uc::autenticar_puente(&state.acceso, token.trim(), request_id).await?,
        ))
    }
}

/// Cursor de la sincronización.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SincronizacionQuery {
    /// `cursor` de la respuesta anterior (0 la primera vez).
    #[serde(default)]
    pub cursor: i64,
    /// Versión del binario del puente.
    pub version: Option<String>,
}

/// Ventana en la que vale una credencial.
#[derive(Debug, Serialize, ToSchema)]
pub struct VentanaDto {
    /// Puntos permitidos; vacío = todos.
    pub puntos: Vec<Uuid>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    /// Franjas semanales (hora local de la PH); `null` = a toda hora.
    #[schema(value_type = Option<Object>)]
    pub horario: Option<domain::Horario>,
}

/// Credencial para la cache del puente.
#[derive(Debug, Serialize, ToSchema)]
pub struct CredencialSincronizadaDto {
    pub id: Uuid,
    #[schema(example = "tarjeta_rfid")]
    pub tipo: String,
    /// SHA-256 de `tenant:tipo:identificador normalizado` (el puente la calcula igual al leer).
    pub huella: String,
    /// `false` = borrarla de la cache.
    pub activa: bool,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub ventanas: Vec<VentanaDto>,
    /// Biometría revocada: borrar la plantilla del equipo y confirmarlo en `/puentes/eventos`.
    pub borrar_plantilla: bool,
}

impl From<CredencialSincronizada> for CredencialSincronizadaDto {
    fn from(c: CredencialSincronizada) -> Self {
        Self {
            id: c.id.as_uuid(),
            tipo: c.tipo.as_str().into(),
            huella: c.huella,
            activa: c.activa,
            vigente_hasta: c.vigente_hasta,
            ventanas: c
                .ventanas
                .into_iter()
                .map(|v| VentanaDto {
                    puntos: v.puntos.into_iter().map(PuntoAccesoId::as_uuid).collect(),
                    vigente_desde: v.vigente_desde,
                    vigente_hasta: v.vigente_hasta,
                    horario: v.horario,
                })
                .collect(),
            borrar_plantilla: c.borrar_plantilla,
        }
    }
}

/// Visita pre-autorizada (para los QR temporales).
#[derive(Debug, Serialize, ToSchema)]
pub struct VisitaSincronizadaDto {
    pub id: Uuid,
    /// `false` = cancelada, vencida o terminada: rechazar su QR.
    pub activa: bool,
    pub usos: u16,
}

impl From<VisitaSincronizada> for VisitaSincronizadaDto {
    fn from(v: VisitaSincronizada) -> Self {
        Self {
            id: v.id.as_uuid(),
            activa: v.activa,
            usos: v.usos,
        }
    }
}

/// Cambios para el puente.
#[derive(Debug, Serialize, ToSchema)]
pub struct SincronizacionRespuesta {
    pub copropiedad_id: Uuid,
    pub credenciales: Vec<CredencialSincronizadaDto>,
    pub visitas: Vec<VisitaSincronizadaDto>,
    /// Cursor para la siguiente llamada. Si llegó igual al enviado, no hay más cambios.
    pub cursor: i64,
    /// Clave pública Ed25519 de la PH (hex) para validar los QR temporales sin internet.
    pub clave_publica_qr: String,
    pub puntos: Vec<PuntoDto>,
    pub dispositivos: Vec<DispositivoDto>,
    /// Hora del servidor (para corregir el reloj del puente).
    pub ahora: DateTime<Utc>,
}

/// Cambios desde el cursor del puente (cuenta como latido).
#[utoipa::path(
    get,
    path = "/sincronizacion",
    tag = ETIQUETA_ACCESO,
    security(("puente" = [])),
    params(SincronizacionQuery),
    responses(
        (status = OK, description = "Cambios", body = SincronizacionRespuesta),
        (status = UNAUTHORIZED, description = "Token de máquina inválido o puente retirado (`no_autenticado`)", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "La PH no tiene el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn sincronizacion(
    _limite: LimitePublico,
    State(state): State<AppState>,
    PuenteMaquina(p): PuenteMaquina,
    Query(q): Query<SincronizacionQuery>,
) -> ApiResult<Json<SincronizacionRespuesta>> {
    let s = uc::sincronizar(
        &state.unidad,
        &RelojSistema,
        state.firma_qr.as_ref(),
        &p,
        q.cursor,
        q.version.as_deref(),
    )
    .await?;
    Ok(Json(SincronizacionRespuesta {
        copropiedad_id: p.ctx.tenant_id.as_uuid(),
        credenciales: s.cambios.credenciales.into_iter().map(Into::into).collect(),
        visitas: s.cambios.visitas.into_iter().map(Into::into).collect(),
        cursor: s.cambios.cursor,
        clave_publica_qr: s.clave_publica.iter().map(|b| format!("{b:02x}")).collect(),
        puntos: s.puntos.into_iter().map(Into::into).collect(),
        dispositivos: s.dispositivos.into_iter().map(Into::into).collect(),
        ahora: s.ahora,
    }))
}

/// Evento que reporta el puente.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EventoReportadoDto {
    /// Id del evento en el puente: un reintento con el mismo id no se duplica.
    pub id_local: String,
    pub dispositivo_id: Option<Uuid>,
    pub punto_acceso_id: Option<Uuid>,
    pub credencial_id: Option<Uuid>,
    /// Visita del QR temporal usado (cuenta el uso).
    pub visita_id: Option<Uuid>,
    #[schema(value_type = String, example = "entrada")]
    pub sentido: SentidoAcceso,
    #[schema(value_type = String, example = "permitido")]
    pub resultado: ResultadoAcceso,
    pub motivo: Option<String>,
    /// Placa leída por la cámara LPR.
    pub placa: Option<String>,
    /// Foto de la placa en Base64 (se borra a los días configurados).
    pub foto_base64: Option<String>,
    pub ocurrido_en: DateTime<Utc>,
}

/// Lote de eventos.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReportePeticion {
    #[serde(default)]
    pub eventos: Vec<EventoReportadoDto>,
    /// Credenciales biométricas cuya plantilla ya se borró del equipo.
    #[serde(default)]
    pub borrados_biometricos: Vec<Uuid>,
}

/// Resultado del reporte.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReporteRespuesta {
    pub nuevos: u32,
    pub repetidos: u32,
    pub borrados_confirmados: u32,
}

/// Recibe un lote de eventos (máx. 500) y confirmaciones de borrado biométrico.
#[utoipa::path(
    post,
    path = "/eventos",
    tag = ETIQUETA_ACCESO,
    security(("puente" = [])),
    request_body = ReportePeticion,
    responses(
        (status = OK, description = "Reporte procesado", body = ReporteRespuesta),
        (status = UNAUTHORIZED, description = "Token de máquina inválido (`no_autenticado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Lote demasiado grande o evento inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn eventos(
    _limite: LimitePublico,
    State(state): State<AppState>,
    PuenteMaquina(p): PuenteMaquina,
    Json(r): Json<ReportePeticion>,
) -> ApiResult<Json<ReporteRespuesta>> {
    let eventos = r
        .eventos
        .into_iter()
        .map(|e| {
            Ok(EventoReportado {
                id_local: e.id_local,
                dispositivo_id: e.dispositivo_id.map(DispositivoAccesoId::from_uuid),
                punto_acceso_id: e.punto_acceso_id.map(PuntoAccesoId::from_uuid),
                credencial_id: e.credencial_id.map(CredencialId::from_uuid),
                visita_id: e.visita_id.map(VisitaId::from_uuid),
                sentido: e.sentido,
                resultado: e.resultado,
                motivo: e.motivo,
                placa: e.placa,
                foto: bytes_base64("foto_base64", e.foto_base64.as_deref())?,
                ocurrido_en: e.ocurrido_en,
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let borrados = r
        .borrados_biometricos
        .into_iter()
        .map(CredencialId::from_uuid)
        .collect();
    let x = uc::reportar_eventos(
        &state.unidad,
        state.archivos.as_ref(),
        &RelojSistema,
        &p,
        eventos,
        borrados,
    )
    .await?;
    Ok(Json(ReporteRespuesta {
        nuevos: x.nuevos,
        repetidos: x.repetidos,
        borrados_confirmados: x.borrados_confirmados,
    }))
}

/// Latido.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct LatidoPeticion {
    #[schema(example = "0.1.0")]
    pub version: Option<String>,
}

/// Latido del puente (sin él por más de 5 min aparece en `/acceso/alertas`).
#[utoipa::path(
    post,
    path = "/latido",
    tag = ETIQUETA_ACCESO,
    security(("puente" = [])),
    request_body = LatidoPeticion,
    responses(
        (status = NO_CONTENT, description = "Latido registrado"),
        (status = UNAUTHORIZED, description = "Token de máquina inválido (`no_autenticado`)", body = ErrorRespuesta),
    )
)]
pub async fn latido(
    _limite: LimitePublico,
    State(state): State<AppState>,
    PuenteMaquina(p): PuenteMaquina,
    Json(l): Json<LatidoPeticion>,
) -> ApiResult<StatusCode> {
    uc::latido(&state.unidad, &RelojSistema, &p, l.version.as_deref()).await?;
    Ok(StatusCode::NO_CONTENT)
}
