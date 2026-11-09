//! Acta (PDF, firmas escaneadas, aprobación, publicación), libro de actas, reportes y la hoja
//! de rótulos.

use application::{
    casos_uso::asambleas::{
        acta::{self as uc, PasoActa},
        reportes,
    },
    puertos::RelojSistema,
};
use axum::{
    extract::State,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use domain::{ArchivoId, PreguntaId, ReunionId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::ReunionRespuesta;
use crate::{
    csv::csv,
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_ASAMBLEAS,
    sesion::EnCopropiedad,
    state::AppState,
};

pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(vista_previa, generar))
        .routes(routes!(avanzar))
        .routes(routes!(libro))
        .routes(routes!(rotulos))
        .routes(routes!(asistencia))
        .routes(routes!(votos))
        .routes(routes!(canales))
        .routes(routes!(costo))
}

fn pdf(nombre: &'static str, bytes: Vec<u8>) -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/pdf"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static(nombre),
            ),
        ],
        bytes,
    )
        .into_response()
}

/// Datos del acta para revisarla antes de generar el PDF (todo ya calculado).
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/acta",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    responses((status = OK, description = "Datos del acta", body = Object))
)]
pub async fn vista_previa(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<serde_json::Value>> {
    let id = ruta.1;
    let d = uc::vista_previa_acta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
    )
    .await?;
    Ok(Json(
        serde_json::to_value(d).map_err(|e| ApiError::Interno(e.into()))?,
    ))
}

/// Genera (o regenera mientras no se apruebe) el acta en PDF con Typst y la guarda como
/// archivo con datos personales (`acta_archivo_id`).
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/acta",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    responses(
        (status = OK, description = "Acta generada", body = ReunionRespuesta),
        (status = CONFLICT, description = "La reunión no está cerrada o el acta ya se aprobó", body = ErrorRespuesta),
    )
)]
pub async fn generar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ReunionRespuesta>> {
    let id = ruta.1;
    let r = uc::generar_acta(
        &state.unidad,
        state.archivos.as_ref(),
        &state.actas,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
    )
    .await?;
    Ok(Json(r.into()))
}

/// Paso del acta.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PasoActaDto {
    /// Liga el escaneo firmado (`archivo_id`, subido con `POST /archivos`).
    Firmada,
    /// Aprobación de la comisión verificadora (exige el escaneo firmado).
    Aprobar,
    /// Publicación en el libro de actas.
    Publicar,
}

/// Avance del acta.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PasoActaPeticion {
    pub paso: PasoActaDto,
    pub archivo_id: Option<Uuid>,
}

/// Sube el escaneo firmado, aprueba o publica el acta.
#[utoipa::path(
    put,
    path = "/asambleas/{reunion_id}/acta/estado",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    request_body = PasoActaPeticion,
    responses(
        (status = OK, description = "Acta actualizada", body = ReunionRespuesta),
        (status = CONFLICT, description = "Transición inválida o falta el escaneo firmado", body = ErrorRespuesta),
    )
)]
pub async fn avanzar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PasoActaPeticion>,
) -> ApiResult<Json<ReunionRespuesta>> {
    let id = ruta.1;
    let paso = match (p.paso, p.archivo_id) {
        (PasoActaDto::Firmada, Some(a)) => PasoActa::Firmada(ArchivoId::from_uuid(a)),
        (PasoActaDto::Firmada, None) => {
            return Err(ApiError::PeticionInvalida("falta `archivo_id`".into()));
        }
        (PasoActaDto::Aprobar, _) => PasoActa::Aprobar,
        (PasoActaDto::Publicar, _) => PasoActa::Publicar,
    };
    let r = uc::avanzar_acta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        paso,
    )
    .await?;
    Ok(Json(r.into()))
}

/// Libro de actas: reuniones con el acta publicada.
#[utoipa::path(
    get,
    path = "/asambleas/actas",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses((status = OK, description = "Libro de actas", body = Vec<ReunionRespuesta>))
)]
pub async fn libro(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<ReunionRespuesta>>> {
    let v = uc::libro_actas(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Paquete de rótulos.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RotulosPeticion {
    #[schema(example = "R-")]
    pub prefijo: String,
    #[schema(example = 1)]
    pub desde: u32,
    #[schema(example = 300)]
    pub cantidad: u32,
}

/// Hoja de rótulos genéricos numerados en Code 128 (PDF para imprimir).
#[utoipa::path(
    post,
    path = "/asambleas/rotulos",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = RotulosPeticion,
    responses(
        (status = OK, description = "PDF", content_type = "application/pdf", body = Vec<u8>),
        (status = UNPROCESSABLE_ENTITY, description = "Cantidad o prefijo inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn rotulos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<RotulosPeticion>,
) -> ApiResult<Response> {
    let bytes = uc::hoja_rotulos(
        &state.unidad,
        &state.actas,
        &RelojSistema,
        &en.ctx,
        &p.prefijo,
        p.desde,
        p.cantidad,
    )
    .await?;
    Ok(pdf("attachment; filename=\"rotulos.pdf\"", bytes))
}

/// Formato de un reporte.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct Formato {
    /// `json` (por defecto) o `csv` (separado por `;`, para Excel).
    #[param(example = "csv")]
    pub formato: Option<String>,
}

impl Formato {
    fn es_csv(&self) -> ApiResult<bool> {
        match self.formato.as_deref() {
            None | Some("json") => Ok(false),
            Some("csv") => Ok(true),
            Some(_) => Err(ApiError::PeticionInvalida(
                "`formato` es `json` o `csv`".into(),
            )),
        }
    }
}

/// Fila del reporte de asistencia.
#[derive(Debug, Serialize, ToSchema)]
pub struct FilaAsistenciaRespuesta {
    pub asistente: String,
    pub bien: String,
    #[schema(value_type = String)]
    pub coeficiente: rust_decimal::Decimal,
    pub calidad: String,
    pub metodo: String,
    pub moroso: bool,
    pub entrada: Option<chrono::DateTime<chrono::Utc>>,
    pub salida: Option<chrono::DateTime<chrono::Utc>>,
    pub motivo_salida: Option<String>,
    pub whatsapp_confirmado: bool,
}

fn fecha(t: Option<chrono::DateTime<chrono::Utc>>) -> String {
    t.map(|t| t.to_rfc3339()).unwrap_or_default()
}

/// Asistencia (un renglón por bien representado), en JSON o CSV.
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/reportes/asistencia",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), Formato),
    responses((status = OK, description = "Asistencia (JSON o `text/csv`)", body = Vec<FilaAsistenciaRespuesta>))
)]
pub async fn asistencia(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(f): Query<Formato>,
) -> ApiResult<Response> {
    let id = ruta.1;
    let filas = reportes::reporte_asistencia(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
    )
    .await?;
    if f.es_csv()? {
        return Ok(csv(
            "attachment; filename=\"asistencia.csv\"",
            &[
                "Asistente",
                "Bien",
                "Coeficiente",
                "Calidad",
                "Método",
                "Moroso",
                "Entrada",
                "Salida",
                "Motivo",
                "WhatsApp",
            ],
            filas
                .into_iter()
                .map(|r| {
                    vec![
                        r.asistente,
                        r.bien,
                        r.coeficiente.to_string(),
                        r.calidad,
                        r.metodo,
                        if r.moroso { "sí" } else { "no" }.into(),
                        fecha(r.entrada),
                        fecha(r.salida),
                        r.motivo_salida.unwrap_or_default(),
                        if r.whatsapp_confirmado { "sí" } else { "no" }.into(),
                    ]
                })
                .collect(),
        ));
    }
    Ok(Json(
        filas
            .into_iter()
            .map(|r| FilaAsistenciaRespuesta {
                asistente: r.asistente,
                bien: r.bien,
                coeficiente: r.coeficiente,
                calidad: r.calidad,
                metodo: r.metodo,
                moroso: r.moroso,
                entrada: r.entrada,
                salida: r.salida,
                motivo_salida: r.motivo_salida,
                whatsapp_confirmado: r.whatsapp_confirmado,
            })
            .collect::<Vec<_>>(),
    )
    .into_response())
}

/// Fila del reporte de votos.
#[derive(Debug, Serialize, ToSchema)]
pub struct FilaVotoRespuesta {
    pub bien: String,
    #[schema(value_type = String)]
    pub coeficiente: rust_decimal::Decimal,
    pub canal: String,
    pub asistente: String,
    pub apoderado: bool,
    pub registrado_en: chrono::DateTime<chrono::Utc>,
    /// Vacío en votaciones reservadas sin permiso de detalle.
    pub opcion: Option<String>,
}

/// Votos de una pregunta (en reservadas, la opción solo con permiso y auditada).
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/reportes/votos/{pregunta_id}",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("pregunta_id" = Uuid, Path, description = "Pregunta"), Formato),
    responses((status = OK, description = "Votos (JSON o `text/csv`)", body = Vec<FilaVotoRespuesta>))
)]
pub async fn votos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    Query(f): Query<Formato>,
) -> ApiResult<Response> {
    let (id, pregunta) = (ruta.1, ruta.2);
    let filas: Vec<FilaVotoRespuesta> = reportes::reporte_votos(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        PreguntaId::from_uuid(pregunta),
    )
    .await?
    .into_iter()
    .map(|f| FilaVotoRespuesta {
        bien: f.participacion.codigo,
        coeficiente: f.participacion.coeficiente.valor(),
        canal: f.participacion.canal.as_str().into(),
        asistente: f.participacion.asistente,
        apoderado: f.participacion.apoderado,
        registrado_en: f.participacion.registrado_en,
        opcion: f.opcion,
    })
    .collect();
    if f.es_csv()? {
        return Ok(csv(
            "attachment; filename=\"votos.csv\"",
            &[
                "Bien",
                "Coeficiente",
                "Canal",
                "Asistente",
                "Apoderado",
                "Hora",
                "Opción",
            ],
            filas
                .into_iter()
                .map(|r| {
                    vec![
                        r.bien,
                        r.coeficiente.to_string(),
                        r.canal,
                        r.asistente,
                        if r.apoderado { "sí" } else { "no" }.into(),
                        r.registrado_en.to_rfc3339(),
                        r.opcion.unwrap_or_default(),
                    ]
                })
                .collect(),
        ));
    }
    Ok(Json(filas).into_response())
}

/// Participación por canal.
#[derive(Debug, Serialize, ToSchema)]
pub struct CanalesRespuesta {
    pub pregunta_id: Uuid,
    pub orden: u16,
    pub enunciado: String,
    /// Canal → bienes que votaron por él.
    pub por_canal: std::collections::BTreeMap<String, u32>,
    pub primero: Option<chrono::DateTime<chrono::Utc>>,
    pub ultimo: Option<chrono::DateTime<chrono::Utc>>,
}

/// Participación por canal y tiempos de cada pregunta.
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/reportes/canales",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    responses((status = OK, description = "Participación por canal", body = Vec<CanalesRespuesta>))
)]
pub async fn canales(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<CanalesRespuesta>>> {
    let id = ruta.1;
    let v = reportes::participacion_por_canal(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
    )
    .await?;
    Ok(Json(
        v.into_iter()
            .map(|p| CanalesRespuesta {
                pregunta_id: p.pregunta_id.as_uuid(),
                orden: p.orden,
                enunciado: p.enunciado,
                por_canal: p
                    .por_canal
                    .into_iter()
                    .map(|(c, n)| (c.as_str().to_owned(), n))
                    .collect(),
                primero: p.primero,
                ultimo: p.ultimo,
            })
            .collect(),
    ))
}

/// Moneda del costo.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct Moneda {
    /// `COP` (por defecto) o `USD`.
    #[param(example = "COP")]
    pub moneda: Option<String>,
}

/// Mensajes de una línea.
#[derive(Debug, Serialize, ToSchema)]
pub struct MensajesLineaRespuesta {
    pub linea: Option<String>,
    pub es_plantilla: bool,
    pub enviados: u64,
    pub fallidos: u64,
}

/// Costo estimado de WhatsApp.
#[derive(Debug, Serialize, ToSchema)]
pub struct CostoRespuesta {
    pub moneda: String,
    pub plantillas: u64,
    pub servicio: u64,
    pub fallidos: u64,
    #[schema(value_type = String)]
    pub tarifa_utilidad: rust_decimal::Decimal,
    #[schema(value_type = String)]
    pub tarifa_servicio: rust_decimal::Decimal,
    /// Sin descontar la franquicia gratuita de cada línea (el valor exacto sale de la factura).
    #[schema(value_type = String)]
    pub valor_estimado: rust_decimal::Decimal,
    pub por_linea: Vec<MensajesLineaRespuesta>,
}

/// Costo estimado de WhatsApp de la asamblea (para cobrarlo dentro de su valor).
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/reportes/costo-whatsapp",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), Moneda),
    responses((status = OK, description = "Costo", body = CostoRespuesta))
)]
pub async fn costo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(m): Query<Moneda>,
) -> ApiResult<Json<CostoRespuesta>> {
    let id = ruta.1;
    let moneda = m.moneda.unwrap_or_else(|| "COP".into());
    let c = reportes::costo_whatsapp(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        &moneda,
    )
    .await?;
    Ok(Json(CostoRespuesta {
        moneda: c.moneda,
        plantillas: c.plantillas,
        servicio: c.servicio,
        fallidos: c.fallidos,
        tarifa_utilidad: c.tarifa_utilidad,
        tarifa_servicio: c.tarifa_servicio,
        valor_estimado: c.valor_estimado,
        por_linea: c
            .por_linea
            .into_iter()
            .map(|l| MensajesLineaRespuesta {
                linea: l.linea,
                es_plantilla: l.es_plantilla,
                enviados: l.enviados,
                fallidos: l.fallidos,
            })
            .collect(),
    }))
}
