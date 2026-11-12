//! Portería: `/acceso/configuracion`, `/acceso/alertas`, `/acceso/costo-avisos`, `/minuta` y
//! `/eventos-acceso` (bitácora).

use application::{
    casos_uso::acceso::porteria::{self as uc, NuevaNovedad, NuevoEventoManual},
    puertos::{
        RelojSistema,
        acceso::{AlertaAcceso, FiltroEventosAcceso, FiltroMinuta, NovedadMinuta},
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, NaiveTime, Utc};
use domain::{
    ConfiguracionAcceso, HorarioSilencio, PuntoAccesoId, ResultadoAcceso, SentidoAcceso,
    TipoNovedadMinuta,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::{EventoAccesoRespuesta, bytes_base64};
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Query},
    openapi::ETIQUETA_ACCESO,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::EnCopropiedad,
    state::AppState,
};

pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(ver_configuracion, guardar_configuracion))
        .routes(routes!(alertas))
        .routes(routes!(costo_avisos))
        .routes(routes!(consultar_minuta, registrar_novedad))
        .routes(routes!(listar_eventos, registrar_evento))
}

/// Configuración del control de acceso de la PH.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ConfiguracionAccesoDto {
    /// Días que se guardan la firma y la foto de quien recoge (30–3650; 365 por defecto).
    #[schema(example = 365)]
    pub retencion_soportes_dias: u32,
    /// Días durante los que se repite el aviso de correspondencia (1–60; 7 por defecto).
    #[schema(example = 7)]
    pub dias_avisos_correspondencia: u32,
    /// Inicio del horario de silencio de los avisos de correspondencia (hora local); `null` = sin
    /// silencio. Las visitas siempre avisan de inmediato.
    #[schema(value_type = Option<String>, example = "21:00:00")]
    pub silencio_inicio: Option<NaiveTime>,
    #[schema(value_type = Option<String>, example = "07:00:00")]
    pub silencio_fin: Option<NaiveTime>,
    /// Días que se guardan los eventos de acceso (365–730).
    #[schema(example = 730)]
    pub retencion_eventos_dias: u32,
    /// Días que se guardan las fotos de placas (1–90; 30 por defecto).
    #[schema(example = 30)]
    pub retencion_fotos_placas_dias: u32,
}

impl From<ConfiguracionAcceso> for ConfiguracionAccesoDto {
    fn from(c: ConfiguracionAcceso) -> Self {
        Self {
            retencion_soportes_dias: c.retencion_soportes_dias,
            dias_avisos_correspondencia: c.dias_avisos_correspondencia,
            silencio_inicio: c.silencio.map(|s| s.inicio),
            silencio_fin: c.silencio.map(|s| s.fin),
            retencion_eventos_dias: c.retencion_eventos_dias,
            retencion_fotos_placas_dias: c.retencion_fotos_placas_dias,
        }
    }
}

/// Configuración vigente (o la predeterminada).
#[utoipa::path(
    get,
    path = "/acceso/configuracion",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Configuración", body = ConfiguracionAccesoDto),
        (status = FORBIDDEN, description = "Sin permiso (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn ver_configuracion(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<ConfiguracionAccesoDto>> {
    Ok(Json(
        uc::ver_configuracion(&state.unidad, &RelojSistema, &en.ctx)
            .await?
            .into(),
    ))
}

/// Reemplaza la configuración (cada PH edita sus plazos de retención y aviso).
#[utoipa::path(
    put,
    path = "/acceso/configuracion",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = ConfiguracionAccesoDto,
    responses(
        (status = OK, description = "Configuración guardada", body = ConfiguracionAccesoDto),
        (status = FORBIDDEN, description = "Sin `acceso.configurar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Plazos fuera de rango (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn guardar_configuracion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<ConfiguracionAccesoDto>,
) -> ApiResult<Json<ConfiguracionAccesoDto>> {
    let silencio = match (p.silencio_inicio, p.silencio_fin) {
        (Some(i), Some(f)) => Some(HorarioSilencio::new(i, f)?),
        (None, None) => None,
        _ => {
            return Err(crate::error::ApiError::PeticionInvalida(
                "indique `silencio_inicio` y `silencio_fin`, o ninguno".into(),
            ));
        }
    };
    let c = ConfiguracionAcceso {
        retencion_soportes_dias: p.retencion_soportes_dias,
        dias_avisos_correspondencia: p.dias_avisos_correspondencia,
        silencio,
        retencion_eventos_dias: p.retencion_eventos_dias,
        retencion_fotos_placas_dias: p.retencion_fotos_placas_dias,
    };
    Ok(Json(
        uc::guardar_configuracion(&state.unidad, &RelojSistema, &en.ctx, c)
            .await?
            .into(),
    ))
}

/// Alerta operativa.
#[derive(Debug, Serialize, ToSchema)]
pub struct AlertaRespuesta {
    /// `puente_sin_latido`, `puerta_forzada` o `denegados_repetidos`.
    #[schema(example = "puente_sin_latido")]
    pub tipo: String,
    pub puente_id: Option<Uuid>,
    pub nombre: Option<String>,
    pub punto_acceso_id: Option<Uuid>,
    pub momento: Option<DateTime<Utc>>,
    pub intentos: Option<u32>,
}

impl From<AlertaAcceso> for AlertaRespuesta {
    fn from(a: AlertaAcceso) -> Self {
        let base = Self {
            tipo: String::new(),
            puente_id: None,
            nombre: None,
            punto_acceso_id: None,
            momento: None,
            intentos: None,
        };
        match a {
            AlertaAcceso::PuenteSinLatido {
                puente,
                nombre,
                ultimo,
            } => Self {
                tipo: "puente_sin_latido".into(),
                puente_id: Some(puente.as_uuid()),
                nombre: Some(nombre),
                momento: ultimo,
                ..base
            },
            AlertaAcceso::PuertaForzada { punto, ocurrido_en } => Self {
                tipo: "puerta_forzada".into(),
                punto_acceso_id: punto.map(PuntoAccesoId::as_uuid),
                momento: Some(ocurrido_en),
                ..base
            },
            AlertaAcceso::DenegadosRepetidos { punto, intentos } => Self {
                tipo: "denegados_repetidos".into(),
                punto_acceso_id: punto.map(PuntoAccesoId::as_uuid),
                intentos: Some(intentos),
                ..base
            },
        }
    }
}

/// Puentes sin latido, puertas forzadas (24 h) e intentos denegados repetidos (10 min).
#[utoipa::path(
    get,
    path = "/acceso/alertas",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Alertas", body = Vec<AlertaRespuesta>),
        (status = FORBIDDEN, description = "Sin `acceso.bitacora.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn alertas(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<AlertaRespuesta>>> {
    let v = uc::alertas(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Periodo del costo.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PeriodoQuery {
    pub desde: DateTime<Utc>,
    pub hasta: DateTime<Utc>,
    /// `COP` si se omite.
    pub moneda: Option<String>,
}

/// Costo de los avisos del periodo.
#[derive(Debug, Serialize, ToSchema)]
pub struct CostoAvisosRespuesta {
    pub moneda: String,
    pub push_enviados: u64,
    /// Plantillas de WhatsApp enviadas (tienen costo).
    pub whatsapp_enviados: u64,
    pub fallidos: u64,
    #[schema(value_type = String, example = "40.50")]
    pub tarifa_utilidad: Decimal,
    #[schema(value_type = String, example = "4050.00")]
    pub valor_estimado: Decimal,
}

/// Avisos de portería del periodo y su costo estimado de WhatsApp (el push es gratuito).
#[utoipa::path(
    get,
    path = "/acceso/costo-avisos",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), PeriodoQuery),
    responses(
        (status = OK, description = "Costo", body = CostoAvisosRespuesta),
        (status = FORBIDDEN, description = "Sin `acceso.configurar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Periodo invertido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn costo_avisos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(q): Query<PeriodoQuery>,
) -> ApiResult<Json<CostoAvisosRespuesta>> {
    let moneda = q.moneda.unwrap_or_else(|| "COP".into());
    let c = uc::costo_avisos(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        q.desde,
        q.hasta,
        &moneda,
    )
    .await?;
    Ok(Json(CostoAvisosRespuesta {
        moneda: c.moneda,
        push_enviados: c.conteo.push_enviados,
        whatsapp_enviados: c.conteo.whatsapp_enviados,
        fallidos: c.conteo.fallidos,
        tarifa_utilidad: c.tarifa_utilidad,
        valor_estimado: c.valor_estimado,
    }))
}

/// Novedad de la minuta.
#[derive(Debug, Serialize, ToSchema)]
pub struct NovedadRespuesta {
    pub id: Uuid,
    /// `inicio_turno`, `entrega_turno`, `ronda`, `novedad`, `incidente` u `otro`.
    #[schema(value_type = crate::esquemas::TipoNovedadMinuta)]
    pub tipo: String,
    pub texto: String,
    pub punto_acceso_id: Option<Uuid>,
    pub adjuntos: Vec<Uuid>,
    pub registrada_por: Option<Uuid>,
    pub ocurrida_en: DateTime<Utc>,
}

impl From<NovedadMinuta> for NovedadRespuesta {
    fn from(n: NovedadMinuta) -> Self {
        Self {
            id: n.id.as_uuid(),
            tipo: n.tipo.as_str().into(),
            texto: n.texto,
            punto_acceso_id: n.punto_acceso_id.map(PuntoAccesoId::as_uuid),
            adjuntos: n.adjuntos.into_iter().map(|a| a.as_uuid()).collect(),
            registrada_por: n.registrada_por.map(|c| c.as_uuid()),
            ocurrida_en: n.ocurrida_en,
        }
    }
}

/// Novedad nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NovedadPeticion {
    #[schema(value_type = crate::esquemas::TipoNovedadMinuta)]
    pub tipo: TipoNovedadMinuta,
    #[schema(example = "Ronda perimetral sin novedad")]
    pub texto: String,
    pub punto_acceso_id: Option<Uuid>,
    /// Fotos o documentos en Base64 (a lo sumo 5).
    #[serde(default)]
    pub adjuntos_base64: Vec<String>,
}

/// Filtros de la minuta.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroMinutaQuery {
    #[param(value_type = Option<crate::esquemas::TipoNovedadMinuta>)]
    pub tipo: Option<TipoNovedadMinuta>,
    pub desde: Option<DateTime<Utc>>,
    pub hasta: Option<DateTime<Utc>>,
}

/// Minuta de portería, más reciente primero.
#[utoipa::path(
    get,
    path = "/minuta",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroMinutaQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Minuta", body = PaginaRespuesta<NovedadRespuesta>),
        (status = FORBIDDEN, description = "Sin `acceso.minuta.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn consultar_minuta(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroMinutaQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<NovedadRespuesta>>> {
    let filtro = FiltroMinuta {
        tipo: f.tipo,
        desde: f.desde,
        hasta: f.hasta,
    };
    let pagina =
        uc::consultar_minuta(&state.unidad, &RelojSistema, &en.ctx, filtro, p.validar()?).await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Registra una novedad (append-only) con sus adjuntos.
#[utoipa::path(
    post,
    path = "/minuta",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NovedadPeticion,
    responses(
        (status = CREATED, description = "Novedad registrada", body = NovedadRespuesta),
        (status = FORBIDDEN, description = "Sin `acceso.minuta.registrar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Texto o adjuntos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_novedad(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NovedadPeticion>,
) -> ApiResult<(StatusCode, Json<NovedadRespuesta>)> {
    let adjuntos = p
        .adjuntos_base64
        .iter()
        .map(|a| bytes_base64("adjuntos_base64", Some(a)).map(Option::unwrap_or_default))
        .collect::<ApiResult<Vec<_>>>()?;
    let n = uc::registrar_novedad(
        &state.unidad,
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        NuevaNovedad {
            tipo: p.tipo,
            texto: p.texto,
            punto_acceso_id: p.punto_acceso_id.map(PuntoAccesoId::from_uuid),
            adjuntos,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(n.into())))
}

/// Filtros de la bitácora.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroEventosQuery {
    pub desde: Option<DateTime<Utc>>,
    pub hasta: Option<DateTime<Utc>>,
    pub punto_acceso_id: Option<Uuid>,
    #[param(value_type = Option<crate::esquemas::ResultadoAcceso>)]
    pub resultado: Option<ResultadoAcceso>,
}

/// Bitácora de eventos de acceso (portería y puentes), más recientes primero.
#[utoipa::path(
    get,
    path = "/eventos-acceso",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroEventosQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Eventos", body = PaginaRespuesta<EventoAccesoRespuesta>),
        (status = FORBIDDEN, description = "Sin `acceso.bitacora.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_eventos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroEventosQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<EventoAccesoRespuesta>>> {
    let filtro = FiltroEventosAcceso {
        desde: f.desde,
        hasta: f.hasta,
        punto_acceso_id: f.punto_acceso_id.map(PuntoAccesoId::from_uuid),
        resultado: f.resultado,
    };
    let pagina =
        uc::listar_eventos(&state.unidad, &RelojSistema, &en.ctx, filtro, p.validar()?).await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Evento manual de la portería.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EventoManualPeticion {
    pub punto_acceso_id: Option<Uuid>,
    #[schema(value_type = crate::esquemas::SentidoAcceso)]
    pub sentido: SentidoAcceso,
    /// `manual` (apertura del vigilante), `denegado` o `forzado`.
    #[schema(value_type = crate::esquemas::ResultadoAcceso)]
    pub resultado: ResultadoAcceso,
    pub motivo: Option<String>,
    pub placa: Option<String>,
}

/// Registra un evento manual en la bitácora (apertura del vigilante, novedad en un punto).
#[utoipa::path(
    post,
    path = "/eventos-acceso",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = EventoManualPeticion,
    responses(
        (status = CREATED, description = "Evento registrado", body = EventoAccesoRespuesta),
        (status = FORBIDDEN, description = "Sin `acceso.registrar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_evento(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<EventoManualPeticion>,
) -> ApiResult<(StatusCode, Json<EventoAccesoRespuesta>)> {
    let e = uc::registrar_evento_manual(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoEventoManual {
            punto_acceso_id: p.punto_acceso_id.map(PuntoAccesoId::from_uuid),
            sentido: p.sentido,
            resultado: p.resultado,
            motivo: p.motivo,
            placa: p.placa,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(e.into())))
}
