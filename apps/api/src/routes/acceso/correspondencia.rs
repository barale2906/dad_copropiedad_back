//! `/correspondencia`: recibir paquetes, sobres y recibos; listado de quién puede recoger;
//! entrega con firma o foto; devolución.

use application::{
    casos_uso::acceso::correspondencia::{
        self as uc, NuevaCorrespondencia, NuevaEntrega, QuienRecogeEntrada,
    },
    puertos::{RelojSistema, acceso::FiltroCorrespondencia},
};
use axum::{extract::State, http::StatusCode};
use domain::{
    BienPrivadoId, CorrespondenciaId, EstadoCorrespondencia, PersonaId, TipoCorrespondencia,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::{CorrespondenciaRespuesta, EntregaCorrespondenciaRespuesta, bytes_base64};
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
        .routes(routes!(listar, registrar))
        .routes(routes!(personas_para_entrega))
        .routes(routes!(ver))
        .routes(routes!(entregar))
        .routes(routes!(devolver))
}

fn id(ruta: (Uuid, Uuid)) -> CorrespondenciaId {
    CorrespondenciaId::from_uuid(ruta.1)
}

/// Correspondencia que recibe la portería.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegistrarCorrespondenciaPeticion {
    pub bien_privado_id: Uuid,
    #[schema(value_type = crate::esquemas::TipoCorrespondencia)]
    pub tipo: TipoCorrespondencia,
    #[schema(example = "Servientrega")]
    pub remitente: Option<String>,
    pub guia: Option<String>,
    /// Dónde quedó en la portería (estante, casillero).
    pub ubicacion: Option<String>,
    /// Foto del paquete en Base64 (JPEG, PNG, WebP o HEIC), opcional.
    pub foto_base64: Option<String>,
}

/// Registra la correspondencia y avisa al primer respondiente (o al terminar el horario de
/// silencio). Cada 24 h sin recoger se avisa a todos los residentes, hasta el tope de días.
#[utoipa::path(
    post,
    path = "/correspondencia",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = RegistrarCorrespondenciaPeticion,
    responses(
        (status = CREATED, description = "Correspondencia registrada", body = CorrespondenciaRespuesta),
        (status = FORBIDDEN, description = "Sin `acceso.registrar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "El bien privado no existe", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos o foto inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<RegistrarCorrespondenciaPeticion>,
) -> ApiResult<(StatusCode, Json<CorrespondenciaRespuesta>)> {
    let c = uc::registrar_correspondencia(
        &state.unidad,
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        NuevaCorrespondencia {
            bien_privado_id: BienPrivadoId::from_uuid(p.bien_privado_id),
            tipo: p.tipo,
            remitente: p.remitente,
            guia: p.guia,
            ubicacion: p.ubicacion,
            foto: bytes_base64("foto_base64", p.foto_base64.as_deref())?,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Filtros del listado.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroCorrespondenciaQuery {
    #[param(value_type = Option<crate::esquemas::EstadoCorrespondencia>)]
    pub estado: Option<EstadoCorrespondencia>,
    /// Obligatorio para residentes y propietarios.
    pub bien_privado_id: Option<Uuid>,
}

/// Correspondencia, más reciente primero. Un residente ve la de su bien («mis paquetes»).
#[utoipa::path(
    get,
    path = "/correspondencia",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroCorrespondenciaQuery, ParametrosPagina),
    responses(
        (status = OK, description = "Correspondencia", body = PaginaRespuesta<CorrespondenciaRespuesta>),
        (status = FORBIDDEN, description = "Sin permiso (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroCorrespondenciaQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<CorrespondenciaRespuesta>>> {
    let filtro = FiltroCorrespondencia {
        estado: f.estado,
        bien_privado_id: f.bien_privado_id.map(BienPrivadoId::from_uuid),
        bienes: None,
    };
    let pagina =
        uc::listar_correspondencia(&state.unidad, &RelojSistema, &en.ctx, filtro, p.validar()?)
            .await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Bien privado del listado de entrega.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BienQuery {
    pub bien_privado_id: Uuid,
}

/// Persona que puede recoger.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonaEntregaRespuesta {
    pub persona_id: Uuid,
    pub nombre: String,
    /// `residente` o `propietario`.
    #[schema(example = "residente")]
    pub vinculo: String,
}

/// Residentes adultos y propietarios del bien, para elegir a quien recoge.
#[utoipa::path(
    get,
    path = "/correspondencia/personas-para-entrega",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), BienQuery),
    responses(
        (status = OK, description = "Personas", body = Vec<PersonaEntregaRespuesta>),
        (status = FORBIDDEN, description = "Sin `acceso.correspondencia.entregar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn personas_para_entrega(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(q): Query<BienQuery>,
) -> ApiResult<Json<Vec<PersonaEntregaRespuesta>>> {
    let v = uc::personas_para_entrega(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(q.bien_privado_id),
    )
    .await?;
    Ok(Json(
        v.into_iter()
            .map(|p| PersonaEntregaRespuesta {
                persona_id: p.persona_id.as_uuid(),
                nombre: p.nombre,
                vinculo: p.vinculo.into(),
            })
            .collect(),
    ))
}

/// Correspondencia con su entrega.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleCorrespondenciaRespuesta {
    pub correspondencia: CorrespondenciaRespuesta,
    pub entrega: Option<EntregaCorrespondenciaRespuesta>,
}

/// Correspondencia con su entrega (el documento del tercero solo lo ve la portería).
#[utoipa::path(
    get,
    path = "/correspondencia/{correspondencia_id}",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("correspondencia_id" = Uuid, Path, description = "Correspondencia")),
    responses(
        (status = OK, description = "Correspondencia", body = DetalleCorrespondenciaRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DetalleCorrespondenciaRespuesta>> {
    let d = uc::ver_correspondencia(&state.unidad, &RelojSistema, &en.ctx, id(ruta)).await?;
    Ok(Json(DetalleCorrespondenciaRespuesta {
        correspondencia: d.correspondencia.into(),
        entrega: d.entrega.map(Into::into),
    }))
}

/// Entrega: quien recoge (persona del listado **o** tercero) y firma o foto en Base64.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EntregarPeticion {
    /// Persona registrada del bien (del listado de entrega).
    pub persona_id: Option<Uuid>,
    /// Tercero no registrado: nombre y documento.
    pub tercero_nombre: Option<String>,
    pub tercero_documento: Option<String>,
    /// Firma en pantalla (PNG en Base64).
    pub firma_base64: Option<String>,
    /// Foto de quien recoge (JPEG o PNG en Base64).
    pub foto_base64: Option<String>,
}

/// Entrega la correspondencia (se exige firma o foto) y confirma al primer respondiente.
#[utoipa::path(
    post,
    path = "/correspondencia/{correspondencia_id}/entrega",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("correspondencia_id" = Uuid, Path, description = "Correspondencia")),
    request_body = EntregarPeticion,
    responses(
        (status = OK, description = "Entregada", body = DetalleCorrespondenciaRespuesta),
        (status = BAD_REQUEST, description = "Falta quién recoge o el Base64 es inválido (`peticion_invalida`)", body = ErrorRespuesta),
        (status = FORBIDDEN, description = "Sin `acceso.correspondencia.entregar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya se entregó o devolvió (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin firma ni foto, o la persona no es del bien (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn entregar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<EntregarPeticion>,
) -> ApiResult<Json<DetalleCorrespondenciaRespuesta>> {
    let quien = match (p.persona_id, p.tercero_nombre, p.tercero_documento) {
        (Some(persona), None, None) => {
            QuienRecogeEntrada::Registrada(PersonaId::from_uuid(persona))
        }
        (None, Some(nombre), Some(documento)) => QuienRecogeEntrada::Tercero { nombre, documento },
        _ => {
            return Err(ApiError::PeticionInvalida(
                "indique `persona_id` o bien `tercero_nombre` y `tercero_documento`".into(),
            ));
        }
    };
    let d = uc::entregar_correspondencia(
        &state.unidad,
        state.archivos.as_ref(),
        &RelojSistema,
        &en.ctx,
        id(ruta),
        NuevaEntrega {
            quien,
            firma: bytes_base64("firma_base64", p.firma_base64.as_deref())?,
            foto: bytes_base64("foto_base64", p.foto_base64.as_deref())?,
        },
    )
    .await?;
    Ok(Json(DetalleCorrespondenciaRespuesta {
        correspondencia: d.correspondencia.into(),
        entrega: d.entrega.map(Into::into),
    }))
}

/// Motivo de la devolución.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DevolverPeticion {
    #[schema(example = "Destinatario desconocido")]
    pub motivo: String,
}

/// Devuelve la correspondencia al remitente (deja de avisarse).
#[utoipa::path(
    post,
    path = "/correspondencia/{correspondencia_id}/devolucion",
    tag = ETIQUETA_ACCESO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("correspondencia_id" = Uuid, Path, description = "Correspondencia")),
    request_body = DevolverPeticion,
    responses(
        (status = OK, description = "Devuelta", body = CorrespondenciaRespuesta),
        (status = CONFLICT, description = "Ya se entregó o devolvió (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn devolver(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<DevolverPeticion>,
) -> ApiResult<Json<CorrespondenciaRespuesta>> {
    let c =
        uc::devolver_correspondencia(&state.unidad, &RelojSistema, &en.ctx, id(ruta), &p.motivo)
            .await?;
    Ok(Json(c.into()))
}
