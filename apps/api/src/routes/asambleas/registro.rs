//! Mesa de registro: búsqueda, asistentes, poderes, rótulos, salidas, reingresos,
//! confirmación por WhatsApp y morosos.

use application::{
    casos_uso::asambleas::registro::{
        self as uc, PersonaAsistente, PoderRecibido, RegistroAsistente, ResultadoMorosos,
    },
    puertos::RelojSistema,
};
use axum::{body::Bytes, extract::State, http::StatusCode};
use domain::{
    ArchivoId, AsistenteId, BienPrivadoId, CodigoRotulo, Documento, EstadoPoder, MetodoRegistro,
    MotivoSalida, PersonaId, PoderId, ReunionId,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::{AsistenteRespuesta, CoincidenciaRespuesta, PoderRespuesta, RegistroRespuesta};
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_ASAMBLEAS,
    sesion::EnCopropiedad,
    state::AppState,
};

pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(buscar))
        .routes(routes!(listar_asistentes, registrar))
        .routes(routes!(agregar_poder))
        .routes(routes!(salida))
        .routes(routes!(reingreso))
        .routes(routes!(rotulo))
        .routes(routes!(confirmacion))
        .routes(routes!(listar_poderes))
        .routes(routes!(revisar_poder))
        .routes(routes!(fijar_morosos))
        .routes(routes!(importar_morosos))
}

fn ids(r: (Uuid, Uuid, Uuid)) -> (ReunionId, AsistenteId) {
    (ReunionId::from_uuid(r.1), AsistenteId::from_uuid(r.2))
}

/// Texto de búsqueda de la mesa.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BusquedaMesa {
    /// Código del bien («506», «T3-506») o documento del representante.
    #[param(example = "506")]
    pub texto: String,
}

/// Busca en el padrón por código del bien o documento del representante (sin tildes ni
/// mayúsculas). Trae el representante, su teléfono del censo y sus otros bienes.
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/mesa",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), BusquedaMesa),
    responses(
        (status = OK, description = "Coincidencias (máximo 20)", body = Vec<CoincidenciaRespuesta>),
        (status = FORBIDDEN, description = "Sin `asambleas.registrar`", body = ErrorRespuesta),
    )
)]
pub async fn buscar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(b): Query<BusquedaMesa>,
) -> ApiResult<Json<Vec<CoincidenciaRespuesta>>> {
    let id = ruta.1;
    let v = uc::buscar_en_mesa(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        &b.texto,
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Asistentes con el estado de entrega de su confirmación por WhatsApp.
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/asistentes",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    responses((status = OK, description = "Asistentes", body = Vec<AsistenteRespuesta>))
)]
pub async fn listar_asistentes(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<AsistenteRespuesta>>> {
    let id = ruta.1;
    let v = uc::asistentes_de_reunion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Poder recibido en la mesa.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PoderPeticion {
    pub bien_privado_id: Uuid,
    /// Foto o PDF del poder subido con `POST /archivos` (puede quedar pendiente).
    pub soporte_archivo_id: Option<Uuid>,
}

impl From<PoderPeticion> for PoderRecibido {
    fn from(p: PoderPeticion) -> Self {
        Self {
            bien_privado_id: BienPrivadoId::from_uuid(p.bien_privado_id),
            soporte_archivo_id: p.soporte_archivo_id.map(ArchivoId::from_uuid),
        }
    }
}

/// Registro de un asistente.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegistroAsistentePeticion {
    /// Persona existente (representante del censo) …
    pub persona_id: Option<Uuid>,
    /// … o una persona nueva (apoderado externo).
    pub nombre: Option<String>,
    pub documento: Option<String>,
    /// Celular (10 dígitos o con indicativo). Si se omite, el del censo.
    #[schema(example = "3001234567")]
    pub telefono: Option<String>,
    #[serde(default)]
    pub bienes_propios: Vec<Uuid>,
    #[serde(default)]
    pub poderes: Vec<PoderPeticion>,
    /// Rótulo preimpreso que se le entrega.
    #[schema(example = "R-0042")]
    pub rotulo: Option<String>,
    /// Enviar la plantilla `confirmar_asistencia` por WhatsApp (por defecto, sí).
    pub enviar_confirmacion: Option<bool>,
    /// `mesa` (el día, cuenta para el quórum; por defecto) o `prerregistro`.
    #[schema(value_type = Option<String>, example = "mesa")]
    pub metodo: Option<MetodoRegistro>,
}

/// Registra al asistente al firmar la planilla: cuenta para el quórum de inmediato; con un
/// toque agrega sus otros bienes y los poderes. Le envía la plantilla de confirmación por
/// WhatsApp y entrega el token de su papeleta virtual.
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/asistentes",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    request_body = RegistroAsistentePeticion,
    responses(
        (status = CREATED, description = "Asistente registrado", body = RegistroRespuesta),
        (status = CONFLICT, description = "Bien en otro asistente, cargo prohibido o máximo de poderes (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin bienes, teléfono inválido o falta el soporte (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<RegistroAsistentePeticion>,
) -> ApiResult<(StatusCode, Json<RegistroRespuesta>)> {
    let id = ruta.1;
    let persona = match (p.persona_id, p.nombre) {
        (Some(id), _) => PersonaAsistente::Existente(PersonaId::from_uuid(id)),
        (None, Some(nombre)) => PersonaAsistente::Nueva {
            nombre,
            documento: p.documento.as_deref().map(Documento::new).transpose()?,
        },
        (None, None) => {
            return Err(ApiError::PeticionInvalida(
                "indique `persona_id` o `nombre`".into(),
            ));
        }
    };
    let r = uc::registrar_asistente(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        RegistroAsistente {
            reunion_id: ReunionId::from_uuid(id),
            persona,
            telefono: p.telefono,
            bienes_propios: p
                .bienes_propios
                .into_iter()
                .map(BienPrivadoId::from_uuid)
                .collect(),
            poderes: p.poderes.into_iter().map(Into::into).collect(),
            rotulo: p.rotulo.as_deref().map(CodigoRotulo::new).transpose()?,
            enviar_confirmacion: p.enviar_confirmacion.unwrap_or(true),
            metodo: p.metodo.unwrap_or(MetodoRegistro::Mesa),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Agrega un poder a un asistente ya registrado.
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/asistentes/{asistente_id}/poderes",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("asistente_id" = Uuid, Path, description = "Asistente")),
    request_body = PoderPeticion,
    responses(
        (status = CREATED, description = "Poder registrado", body = AsistenteRespuesta),
        (status = CONFLICT, description = "Bien ocupado, cargo prohibido o máximo (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn agregar_poder(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    Json(p): Json<PoderPeticion>,
) -> ApiResult<(StatusCode, Json<AsistenteRespuesta>)> {
    let (r, a) = ids(ruta);
    let a = uc::registrar_poder(&state.unidad, &RelojSistema, &en.ctx, r, a, p.into()).await?;
    Ok((StatusCode::CREATED, Json(a.into())))
}

/// Salida del asistente.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SalidaPeticion {
    /// `se_retiro`, `delego_poder`, `problema_tecnico` u `otro` (con `detalle`).
    #[schema(value_type = String, example = "se_retiro")]
    pub motivo: MotivoSalida,
    pub detalle: Option<String>,
}

/// Registra la salida con hora y motivo: sus bienes dejan el quórum y las preguntas que se
/// abran después; sus votos se conservan.
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/asistentes/{asistente_id}/salida",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("asistente_id" = Uuid, Path, description = "Asistente")),
    request_body = SalidaPeticion,
    responses(
        (status = OK, description = "Salida registrada", body = AsistenteRespuesta),
        (status = CONFLICT, description = "No estaba presente (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn salida(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    Json(p): Json<SalidaPeticion>,
) -> ApiResult<Json<AsistenteRespuesta>> {
    let (r, a) = ids(ruta);
    let a = uc::registrar_salida(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        r,
        a,
        p.motivo,
        p.detalle.as_deref(),
    )
    .await?;
    Ok(Json(a.into()))
}

/// Registra el reingreso (nuevo periodo de presencia).
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/asistentes/{asistente_id}/reingreso",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("asistente_id" = Uuid, Path, description = "Asistente")),
    responses(
        (status = OK, description = "Reingreso registrado", body = AsistenteRespuesta),
        (status = CONFLICT, description = "Ya estaba presente (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn reingreso(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
) -> ApiResult<Json<AsistenteRespuesta>> {
    let (r, a) = ids(ruta);
    Ok(Json(
        uc::registrar_reingreso(&state.unidad, &RelojSistema, &en.ctx, r, a)
            .await?
            .into(),
    ))
}

/// Rótulo del asistente (`null` lo libera).
#[derive(Debug, Deserialize, ToSchema)]
pub struct RotuloPeticion {
    #[schema(example = "R-0042")]
    pub codigo: Option<String>,
}

/// Liga (o cambia) el rótulo preimpreso del asistente.
#[utoipa::path(
    put,
    path = "/asambleas/{reunion_id}/asistentes/{asistente_id}/rotulo",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("asistente_id" = Uuid, Path, description = "Asistente")),
    request_body = RotuloPeticion,
    responses(
        (status = OK, description = "Rótulo ligado", body = AsistenteRespuesta),
        (status = CONFLICT, description = "El rótulo ya está en uso (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn rotulo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    Json(p): Json<RotuloPeticion>,
) -> ApiResult<Json<AsistenteRespuesta>> {
    let (r, a) = ids(ruta);
    let codigo = p.codigo.as_deref().map(CodigoRotulo::new).transpose()?;
    Ok(Json(
        uc::asignar_rotulo(&state.unidad, &RelojSistema, &en.ctx, r, a, codigo)
            .await?
            .into(),
    ))
}

/// Reenvío de la confirmación.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ConfirmacionPeticion {
    /// Otro número si el anterior no recibió la plantilla.
    pub telefono: Option<String>,
}

/// Reenvía la plantilla `confirmar_asistencia` (p. ej. con otro número si no llegó).
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/asistentes/{asistente_id}/confirmacion",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("asistente_id" = Uuid, Path, description = "Asistente")),
    request_body = ConfirmacionPeticion,
    responses(
        (status = OK, description = "Confirmación encolada", body = AsistenteRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin teléfono o sin líneas activas (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn confirmacion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    Json(p): Json<ConfirmacionPeticion>,
) -> ApiResult<Json<AsistenteRespuesta>> {
    let (r, a) = ids(ruta);
    let a = uc::reenviar_confirmacion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        r,
        a,
        p.telefono.as_deref(),
    )
    .await?;
    Ok(Json(a.into()))
}

/// Poderes de la reunión.
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/poderes",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    responses((status = OK, description = "Poderes", body = Vec<PoderRespuesta>))
)]
pub async fn listar_poderes(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<PoderRespuesta>>> {
    let id = ruta.1;
    let v = uc::listar_poderes(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Revisión de un poder.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RevisionPoderPeticion {
    /// `verificado`, `rechazado` o `revocado`.
    #[schema(value_type = String, example = "verificado")]
    pub estado: EstadoPoder,
    pub motivo: Option<String>,
}

/// Verifica, rechaza o revoca un poder (rechazado o revocado, el bien deja de estar
/// representado por el apoderado).
#[utoipa::path(
    put,
    path = "/asambleas/{reunion_id}/poderes/{poder_id}/estado",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("poder_id" = Uuid, Path, description = "Poder")),
    request_body = RevisionPoderPeticion,
    responses(
        (status = OK, description = "Poder actualizado", body = PoderRespuesta),
        (status = CONFLICT, description = "Transición inválida (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn revisar_poder(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    Json(p): Json<RevisionPoderPeticion>,
) -> ApiResult<Json<PoderRespuesta>> {
    let (id, poder) = (ruta.1, ruta.2);
    let p = uc::revisar_poder(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        PoderId::from_uuid(poder),
        p.estado,
        p.motivo.as_deref(),
    )
    .await?;
    Ok(Json(p.into()))
}

/// Listado de morosos que entrega la copropiedad.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MorososPeticion {
    /// Códigos de bien privado (reemplazan el listado).
    #[schema(example = json!(["T1-301", "T2-405"]))]
    pub codigos: Vec<String>,
}

/// Resultado de fijar los morosos.
#[derive(Debug, Serialize, ToSchema)]
pub struct MorososRespuesta {
    pub marcados: u32,
    /// Códigos que no están en el padrón.
    pub no_encontrados: Vec<String>,
}

impl From<ResultadoMorosos> for MorososRespuesta {
    fn from(r: ResultadoMorosos) -> Self {
        Self {
            marcados: r.marcados,
            no_encontrados: r.no_encontrados,
        }
    }
}

/// Fija el listado de morosos (después de convocar y antes de instalar). El efecto lo define la
/// configuración de la reunión y queda auditado.
#[utoipa::path(
    put,
    path = "/asambleas/{reunion_id}/morosos",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    request_body = MorososPeticion,
    responses(
        (status = OK, description = "Morosos fijados", body = MorososRespuesta),
        (status = CONFLICT, description = "La reunión ya se instaló (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn fijar_morosos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<MorososPeticion>,
) -> ApiResult<Json<MorososRespuesta>> {
    let id = ruta.1;
    let r = uc::fijar_morosos(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        &p.codigos,
    )
    .await?;
    Ok(Json(r.into()))
}

/// Importa los morosos desde un CSV o XLSX (primera columna: código del bien).
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/morosos/planilla",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    request_body(content = Vec<u8>, content_type = "application/octet-stream", description = "CSV o XLSX"),
    responses(
        (status = OK, description = "Morosos fijados", body = MorososRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es una planilla legible (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn importar_morosos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    cuerpo: Bytes,
) -> ApiResult<Json<MorososRespuesta>> {
    let id = ruta.1;
    let r = uc::importar_morosos(
        &state.unidad,
        &RelojSistema,
        &state.planillas,
        &en.ctx,
        ReunionId::from_uuid(id),
        &cuerpo,
    )
    .await?;
    Ok(Json(r.into()))
}
