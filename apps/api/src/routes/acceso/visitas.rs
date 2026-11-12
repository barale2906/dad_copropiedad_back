//! `/visitas`: anuncio en la portería, pre-autorización con QR, decisión, llamada, ingreso,
//! salida y cancelación.

use application::{
    casos_uso::acceso::visitas::{
        self as uc, NuevaPreautorizacion, NuevaVisita, ResultadoDecision,
    },
    puertos::{RelojSistema, acceso::FiltroVisitas},
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{
    BienPrivadoId, CanalRespuestaVisita, EstadoVisita, PersonaId, PuntoAccesoId, ResultadoLlamada,
    TipoVisita, VisitaId,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::{EventoVisitaRespuesta, VisitaRespuesta};
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_ACCESO,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::EnCopropiedad,
    state::AppState,
};

pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar, anunciar))
        .routes(routes!(preautorizar))
        .routes(routes!(ingreso_qr))
        .routes(routes!(ver))
        .routes(routes!(qr))
        .routes(routes!(responder))
        .routes(routes!(contactos))
        .routes(routes!(llamada))
        .routes(routes!(ingreso))
        .routes(routes!(salida))
        .routes(routes!(cancelar))
}

fn id(ruta: (Uuid, Uuid)) -> VisitaId {
    VisitaId::from_uuid(ruta.1)
}

/// Visitante que anuncia la portería.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AnunciarPeticion {
    pub bien_privado_id: Uuid,
    #[schema(value_type = crate::esquemas::TipoVisita)]
    pub tipo: TipoVisita,
    #[schema(example = "Mensajero de domicilios")]
    pub visitante_nombre: String,
    pub visitante_documento: Option<String>,
    #[schema(example = "ABC123")]
    pub placa: Option<String>,
    pub observacion: Option<String>,
}

impl AnunciarPeticion {
    fn nueva(self) -> NuevaVisita {
        NuevaVisita {
            bien_privado_id: BienPrivadoId::from_uuid(self.bien_privado_id),
            tipo: self.tipo,
            visitante_nombre: self.visitante_nombre,
            visitante_documento: self.visitante_documento,
            placa: self.placa,
            observacion: self.observacion,
        }
    }
}

/// Filtros del listado.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroVisitasQuery {
    #[param(value_type = Option<crate::esquemas::EstadoVisita>)]
    pub estado: Option<EstadoVisita>,
    /// Obligatorio para residentes y propietarios (solo ven las de su bien).
    pub bien_privado_id: Option<Uuid>,
    pub desde: Option<DateTime<Utc>>,
    /// Solo los visitantes que están dentro.
    #[serde(default)]
    pub dentro: bool,
}

/// Visitas, más recientes primero. La portería ve todas; un residente, las de su bien.
#[utoipa::path(
    get,
    path = "/visitas",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroVisitasQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Visitas", body = PaginaRespuesta<VisitaRespuesta>),
        (status = FORBIDDEN, description = "Sin permiso (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroVisitasQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<VisitaRespuesta>>> {
    let filtro = FiltroVisitas {
        estado: f.estado,
        bien_privado_id: f.bien_privado_id.map(BienPrivadoId::from_uuid),
        bienes: None,
        desde: f.desde,
        dentro: f.dentro,
    };
    let pagina =
        uc::listar_visitas(&state.unidad, &RelojSistema, &en.ctx, filtro, p.validar()?).await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// La portería anuncia una visita: aviso al primer respondiente (app o WhatsApp); a los 2 min a
/// todos los residentes adultos y a los 5 min queda en `requiere_llamada`.
#[utoipa::path(
    post,
    path = "/visitas",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = AnunciarPeticion,
    responses(
        (status = CREATED, description = "Visita anunciada", body = VisitaRespuesta),
        (status = FORBIDDEN, description = "Sin `acceso.registrar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "El bien privado no existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn anunciar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<AnunciarPeticion>,
) -> ApiResult<(StatusCode, Json<VisitaRespuesta>)> {
    let v = uc::anunciar_visita(&state.unidad, &RelojSistema, &en.ctx, p.nueva()).await?;
    Ok((StatusCode::CREATED, Json(v.into())))
}

/// Pre-autorización de un residente.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PreautorizarPeticion {
    #[serde(flatten)]
    pub visita: AnunciarPeticion,
    pub desde: DateTime<Utc>,
    pub hasta: DateTime<Utc>,
    /// Ingresos permitidos (1–60).
    #[schema(example = 1)]
    pub usos: u16,
}

/// Visita con su QR temporal.
#[derive(Debug, Serialize, ToSchema)]
pub struct VisitaConQrRespuesta {
    pub visita: VisitaRespuesta,
    /// Texto del QR (`DQ1.<carga>.<firma>`), firmado con la clave Ed25519 de la PH: el puente lo
    /// valida sin internet.
    pub qr: String,
}

/// Un residente pre-autoriza una visita y recibe el QR temporal para compartirlo.
#[utoipa::path(
    post,
    path = "/visitas/preautorizaciones",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = PreautorizarPeticion,
    responses(
        (status = CREATED, description = "Visita pre-autorizada", body = VisitaConQrRespuesta),
        (status = FORBIDDEN, description = "La cuenta no puede decidir por ese bien (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Ventana o usos fuera de rango (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn preautorizar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<PreautorizarPeticion>,
) -> ApiResult<(StatusCode, Json<VisitaConQrRespuesta>)> {
    let (v, qr) = uc::preautorizar_visita(
        &state.unidad,
        &RelojSistema,
        state.firma_qr.as_ref(),
        &en.ctx,
        NuevaPreautorizacion {
            visita: p.visita.nueva(),
            desde: p.desde,
            hasta: p.hasta,
            usos: p.usos,
        },
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(VisitaConQrRespuesta {
            visita: v.into(),
            qr,
        }),
    ))
}

/// Punto por el que pasa la visita.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct PuntoVisitaPeticion {
    pub punto_acceso_id: Option<Uuid>,
}

fn punto(p: Option<Uuid>) -> Option<PuntoAccesoId> {
    p.map(PuntoAccesoId::from_uuid)
}

/// QR leído con la cámara de la tablet.
#[derive(Debug, Deserialize, ToSchema)]
pub struct IngresoQrPeticion {
    pub qr: String,
    pub punto_acceso_id: Option<Uuid>,
}

/// Ingreso con el QR temporal: verifica la firma de la PH, la ventana y los usos.
#[utoipa::path(
    post,
    path = "/visitas/ingreso-qr",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = IngresoQrPeticion,
    responses(
        (status = OK, description = "Ingreso registrado", body = VisitaRespuesta),
        (status = CONFLICT, description = "Fuera de la ventana o sin usos (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "QR inválido o de otra copropiedad (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn ingreso_qr(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<IngresoQrPeticion>,
) -> ApiResult<Json<VisitaRespuesta>> {
    let v = uc::ingreso_por_qr(
        &state.unidad,
        &RelojSistema,
        state.firma_qr.as_ref(),
        &en.ctx,
        &p.qr,
        punto(p.punto_acceso_id),
    )
    .await?;
    Ok(Json(v.into()))
}

/// Visita con su historial.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleVisitaRespuesta {
    pub visita: VisitaRespuesta,
    pub eventos: Vec<EventoVisitaRespuesta>,
}

/// Visita con su historial (avisos, decisiones, llamadas, ingreso y salida).
#[utoipa::path(
    get,
    path = "/visitas/{visita_id}",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("visita_id" = Uuid, Path, description = "Visita")),
    responses(
        (status = OK, description = "Visita", body = DetalleVisitaRespuesta),
        (status = FORBIDDEN, description = "Sin permiso (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DetalleVisitaRespuesta>> {
    let d = uc::ver_visita(&state.unidad, &RelojSistema, &en.ctx, id(ruta)).await?;
    Ok(Json(DetalleVisitaRespuesta {
        visita: d.visita.into(),
        eventos: d.eventos.into_iter().map(Into::into).collect(),
    }))
}

/// QR de la pre-autorización.
#[derive(Debug, Serialize, ToSchema)]
pub struct QrRespuesta {
    pub qr: String,
}

/// QR vigente de una visita pre-autorizada (se regenera igual: la firma es determinista).
#[utoipa::path(
    get,
    path = "/visitas/{visita_id}/qr",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("visita_id" = Uuid, Path, description = "Visita")),
    responses(
        (status = OK, description = "QR", body = QrRespuesta),
        (status = CONFLICT, description = "No es una pre-autorización vigente (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn qr(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<QrRespuesta>> {
    let qr = uc::qr_de_visita(
        &state.unidad,
        &RelojSistema,
        state.firma_qr.as_ref(),
        &en.ctx,
        id(ruta),
    )
    .await?;
    Ok(Json(QrRespuesta { qr }))
}

/// Decisión de un residente.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResponderPeticion {
    pub autorizar: bool,
    /// `app` (por defecto) o `web`.
    #[schema(value_type = Option<crate::esquemas::CanalRespuestaVisita>)]
    pub canal: Option<CanalRespuestaVisita>,
}

/// Resultado de la decisión: la primera respuesta gana.
#[derive(Debug, Serialize, ToSchema)]
pub struct DecisionRespuesta {
    /// `false` si otra persona ya había decidido; `visita` trae esa decisión.
    pub decidida_ahora: bool,
    pub visita: VisitaRespuesta,
}

/// Un residente autoriza o rechaza la visita desde la app o la web.
#[utoipa::path(
    post,
    path = "/visitas/{visita_id}/respuesta",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("visita_id" = Uuid, Path, description = "Visita")),
    request_body = ResponderPeticion,
    responses(
        (status = OK, description = "Decisión", body = DecisionRespuesta),
        (status = FORBIDDEN, description = "La cuenta no puede decidir por ese bien (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn responder(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ResponderPeticion>,
) -> ApiResult<Json<DecisionRespuesta>> {
    let canal = match p.canal.unwrap_or(CanalRespuestaVisita::App) {
        c @ (CanalRespuestaVisita::App | CanalRespuestaVisita::Web) => c,
        _ => {
            return Err(ApiError::PeticionInvalida(
                "`canal` debe ser `app` o `web`".into(),
            ));
        }
    };
    let r = uc::decidir_visita(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id(ruta),
        p.autorizar,
        canal,
    )
    .await?;
    let (decidida_ahora, v) = match r {
        ResultadoDecision::Decidida(v) => (true, v),
        ResultadoDecision::YaDecidida(v) => (false, v),
    };
    Ok(Json(DecisionRespuesta {
        decidida_ahora,
        visita: v.into(),
    }))
}

/// Contacto para la llamada.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContactoRespuesta {
    pub persona_id: Uuid,
    pub nombre: String,
    pub telefono: String,
}

/// Nombres y teléfonos para llamar (solo mientras la visita está en `requiere_llamada`; cada
/// consulta se audita).
#[utoipa::path(
    get,
    path = "/visitas/{visita_id}/contactos",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("visita_id" = Uuid, Path, description = "Visita")),
    responses(
        (status = OK, description = "Contactos", body = Vec<ContactoRespuesta>),
        (status = FORBIDDEN, description = "Sin `acceso.registrar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "La visita no espera una llamada (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn contactos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<ContactoRespuesta>>> {
    let v = uc::contactos_para_llamar(&state.unidad, &RelojSistema, &en.ctx, id(ruta)).await?;
    Ok(Json(
        v.into_iter()
            .map(|c| ContactoRespuesta {
                persona_id: c.persona_id.as_uuid(),
                nombre: c.nombre,
                telefono: c.telefono,
            })
            .collect(),
    ))
}

/// Resultado de la llamada.
#[derive(Debug, Deserialize, ToSchema)]
pub struct LlamadaPeticion {
    /// `autorizo`, `rechazo` o `no_contesto`.
    #[schema(value_type = crate::esquemas::ResultadoLlamada)]
    pub resultado: ResultadoLlamada,
    /// Persona que atendió la llamada (si contestó).
    pub persona_id: Option<Uuid>,
}

/// La portería registra el resultado de la llamada.
#[utoipa::path(
    post,
    path = "/visitas/{visita_id}/llamada",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("visita_id" = Uuid, Path, description = "Visita")),
    request_body = LlamadaPeticion,
    responses(
        (status = OK, description = "Visita", body = VisitaRespuesta),
        (status = CONFLICT, description = "La visita ya fue atendida (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "La persona no puede decidir por ese bien (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn llamada(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<LlamadaPeticion>,
) -> ApiResult<Json<VisitaRespuesta>> {
    let v = uc::registrar_llamada(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id(ruta),
        p.resultado,
        p.persona_id.map(PersonaId::from_uuid),
    )
    .await?;
    Ok(Json(v.into()))
}

/// Ingreso de una visita autorizada.
#[utoipa::path(
    post,
    path = "/visitas/{visita_id}/ingreso",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("visita_id" = Uuid, Path, description = "Visita")),
    request_body = PuntoVisitaPeticion,
    responses(
        (status = OK, description = "Ingreso registrado", body = VisitaRespuesta),
        (status = CONFLICT, description = "No autorizada, fuera de la ventana o sin usos (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn ingreso(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PuntoVisitaPeticion>,
) -> ApiResult<Json<VisitaRespuesta>> {
    let v = uc::registrar_ingreso(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id(ruta),
        punto(p.punto_acceso_id),
    )
    .await?;
    Ok(Json(v.into()))
}

/// Salida de la visita (una pre-autorización con usos vuelve a quedar autorizada).
#[utoipa::path(
    post,
    path = "/visitas/{visita_id}/salida",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("visita_id" = Uuid, Path, description = "Visita")),
    request_body = PuntoVisitaPeticion,
    responses(
        (status = OK, description = "Salida registrada", body = VisitaRespuesta),
        (status = CONFLICT, description = "No estaba dentro (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn salida(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PuntoVisitaPeticion>,
) -> ApiResult<Json<VisitaRespuesta>> {
    let v = uc::registrar_salida(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id(ruta),
        punto(p.punto_acceso_id),
    )
    .await?;
    Ok(Json(v.into()))
}

/// Cancela una visita que aún no ingresó.
#[utoipa::path(
    post,
    path = "/visitas/{visita_id}/cancelacion",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("visita_id" = Uuid, Path, description = "Visita")),
    responses(
        (status = OK, description = "Visita cancelada", body = VisitaRespuesta),
        (status = FORBIDDEN, description = "Sin permiso (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya ingresó o terminó (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn cancelar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<VisitaRespuesta>> {
    let v = uc::cancelar_visita(&state.unidad, &RelojSistema, &en.ctx, id(ruta)).await?;
    Ok(Json(v.into()))
}
