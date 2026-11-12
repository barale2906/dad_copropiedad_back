//! Comunicados de la administración (plan 10, fase 4; módulo `gestion_administrativa`), dentro
//! de `/api/v1/copropiedades/{copropiedad_id}`:
//!
//! - `/comunicados…`: redactar, publicar (inmediato o programado), retirar, anular y ver el
//!   tablero de lectura (`comunicados.gestionar`).
//! - `/mis-comunicados…`: la cartelera de la persona de la cuenta; abrir uno registra la lectura
//!   y `enterado` registra el «Enterado».

use application::{
    AppError,
    casos_uso::comunicados::{self as uc, AdjuntoComunicado, ComunicadoAbierto, NuevoComunicado},
    puertos::{
        RelojSistema,
        comunicados::{Comunicado, ComunicadoConResumen, ComunicadoRecibido, ResumenLectura},
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{
    AgrupacionId, ArchivoId, Audiencia, BienPrivadoId, ComunicadoId, FinalidadTratamiento,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_COMUNICADOS,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de los comunicados.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar, crear))
        .routes(routes!(ver, editar))
        .routes(routes!(publicar))
        .routes(routes!(retirar))
        .routes(routes!(anular))
        .routes(routes!(mis_comunicados))
        .routes(routes!(abrir_mio))
        .routes(routes!(enterado))
}

// ---------------------------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------------------------

/// A quién va el comunicado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TipoAudiencia {
    /// Propietarios y residentes adultos de toda la PH.
    Todos,
    Propietarios,
    /// Residentes mayores de edad.
    Residentes,
    /// Bienes de esas agrupaciones (y sus subagrupaciones); `ids` = agrupaciones.
    Agrupaciones,
    /// `ids` = bienes privados.
    Bienes,
    /// `ids` = códigos de rol (p. ej. `presidente_consejo`).
    Roles,
}

/// Audiencia: el tipo y, si aplica, los destinos.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AudienciaDto {
    pub tipo: TipoAudiencia,
    /// Ids de agrupaciones o bienes, o códigos de rol. Vacío para `todos`, `propietarios` y
    /// `residentes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schema(example = json!([]))]
    pub ids: Vec<String>,
}

fn uuids<T>(ids: &[String], f: impl Fn(Uuid) -> T) -> Result<Vec<T>, AppError> {
    ids.iter()
        .map(|s| {
            Uuid::parse_str(s).map(&f).map_err(|_| {
                AppError::Validacion("`audiencia.ids` debe tener identificadores válidos".into())
            })
        })
        .collect()
}

impl AudienciaDto {
    fn a_dominio(&self) -> ApiResult<Audiencia> {
        Ok(match self.tipo {
            TipoAudiencia::Todos => Audiencia::Todos,
            TipoAudiencia::Propietarios => Audiencia::Propietarios,
            TipoAudiencia::Residentes => Audiencia::Residentes,
            TipoAudiencia::Agrupaciones => {
                Audiencia::Agrupaciones(uuids(&self.ids, AgrupacionId::from_uuid)?)
            }
            TipoAudiencia::Bienes => Audiencia::Bienes(uuids(&self.ids, BienPrivadoId::from_uuid)?),
            TipoAudiencia::Roles => Audiencia::Roles(self.ids.clone()),
        })
    }
}

impl From<&Audiencia> for AudienciaDto {
    fn from(a: &Audiencia) -> Self {
        let texto = |v: &[Uuid]| v.iter().map(Uuid::to_string).collect();
        match a {
            Audiencia::Todos => Self {
                tipo: TipoAudiencia::Todos,
                ids: Vec::new(),
            },
            Audiencia::Propietarios => Self {
                tipo: TipoAudiencia::Propietarios,
                ids: Vec::new(),
            },
            Audiencia::Residentes => Self {
                tipo: TipoAudiencia::Residentes,
                ids: Vec::new(),
            },
            Audiencia::Agrupaciones(v) => Self {
                tipo: TipoAudiencia::Agrupaciones,
                ids: texto(&v.iter().map(|x| x.as_uuid()).collect::<Vec<_>>()),
            },
            Audiencia::Bienes(v) => Self {
                tipo: TipoAudiencia::Bienes,
                ids: texto(&v.iter().map(|x| x.as_uuid()).collect::<Vec<_>>()),
            },
            Audiencia::Roles(v) => Self {
                tipo: TipoAudiencia::Roles,
                ids: v.clone(),
            },
        }
    }
}

/// Contenido del comunicado (crear o reemplazar el borrador).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ComunicadoPeticion {
    #[schema(example = "Corte de agua el jueves")]
    pub titulo: String,
    /// Texto; admite saltos de línea (máximo 10 000 caracteres).
    pub cuerpo: String,
    /// `administracion` (llega a todos) o `comunicaciones` (solo a quien lo autorizó, Ley 1581).
    #[schema(value_type = crate::esquemas::FinalidadTratamiento)]
    pub finalidad: String,
    pub audiencia: AudienciaDto,
    /// También por WhatsApp a quien no tiene la app (plantilla `comunicado_importante`, con costo).
    #[serde(default)]
    pub importante: bool,
    /// Pide el botón «Enterado».
    #[serde(default)]
    pub requiere_confirmacion: bool,
    /// Archivos ya subidos (PDF o imágenes), máximo 10.
    #[serde(default)]
    pub adjuntos: Vec<Uuid>,
}

impl ComunicadoPeticion {
    fn a_caso_uso(self) -> ApiResult<NuevoComunicado> {
        Ok(NuevoComunicado {
            audiencia: self.audiencia.a_dominio()?,
            finalidad: FinalidadTratamiento::try_from(self.finalidad.as_str())?,
            titulo: self.titulo,
            cuerpo: self.cuerpo,
            importante: self.importante,
            requiere_confirmacion: self.requiere_confirmacion,
            adjuntos: self
                .adjuntos
                .into_iter()
                .map(ArchivoId::from_uuid)
                .collect(),
        })
    }
}

/// Publicación: sin fecha (o con una pasada) sale de inmediato.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct PublicarPeticion {
    /// Hora de envío; máximo un año adelante. El worker lo envía a lo sumo 10 min después.
    pub programado_para: Option<DateTime<Utc>>,
}

/// Comunicado.
#[derive(Debug, Serialize, ToSchema)]
pub struct ComunicadoRespuesta {
    pub id: Uuid,
    pub titulo: String,
    pub cuerpo: String,
    #[schema(value_type = crate::esquemas::FinalidadTratamiento)]
    pub finalidad: String,
    pub audiencia: AudienciaDto,
    pub importante: bool,
    pub requiere_confirmacion: bool,
    /// `borrador`, `programado`, `enviado` o `anulado`.
    #[schema(value_type = crate::esquemas::EstadoComunicado)]
    pub estado: String,
    pub programado_para: Option<DateTime<Utc>>,
    pub enviado_en: Option<DateTime<Utc>>,
    pub adjuntos: Vec<Uuid>,
    pub creado_en: DateTime<Utc>,
    pub actualizado_en: DateTime<Utc>,
}

impl From<Comunicado> for ComunicadoRespuesta {
    fn from(c: Comunicado) -> Self {
        Self {
            id: c.id.as_uuid(),
            audiencia: (&c.audiencia).into(),
            titulo: c.titulo,
            cuerpo: c.cuerpo,
            finalidad: c.finalidad.as_str().into(),
            importante: c.importante,
            requiere_confirmacion: c.requiere_confirmacion,
            estado: c.estado.as_str().into(),
            programado_para: c.programado_para,
            enviado_en: c.enviado_en,
            adjuntos: c.adjuntos.into_iter().map(ArchivoId::as_uuid).collect(),
            creado_en: c.creado_en,
            actualizado_en: c.actualizado_en,
        }
    }
}

/// Entrega y lectura.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResumenLecturaRespuesta {
    pub destinatarios: u32,
    pub por_push: u32,
    pub por_correo: u32,
    pub por_whatsapp: u32,
    /// Sin ningún medio de contacto: solo lo verán en la cartelera.
    pub sin_canal: u32,
    pub leidos: u32,
    pub enterados: u32,
}

impl From<ResumenLectura> for ResumenLecturaRespuesta {
    fn from(r: ResumenLectura) -> Self {
        Self {
            destinatarios: r.destinatarios,
            por_push: r.por_push,
            por_correo: r.por_correo,
            por_whatsapp: r.por_whatsapp,
            sin_canal: r.sin_canal,
            leidos: r.leidos,
            enterados: r.enterados,
        }
    }
}

/// Comunicado con su tablero.
#[derive(Debug, Serialize, ToSchema)]
pub struct ComunicadoConResumenRespuesta {
    pub comunicado: ComunicadoRespuesta,
    pub resumen: ResumenLecturaRespuesta,
}

impl From<ComunicadoConResumen> for ComunicadoConResumenRespuesta {
    fn from(c: ComunicadoConResumen) -> Self {
        Self {
            comunicado: c.comunicado.into(),
            resumen: c.resumen.into(),
        }
    }
}

/// Comunicado en la cartelera de la persona.
#[derive(Debug, Serialize, ToSchema)]
pub struct ComunicadoRecibidoRespuesta {
    pub comunicado: ComunicadoRespuesta,
    pub leido_en: Option<DateTime<Utc>>,
    pub enterado_en: Option<DateTime<Utc>>,
}

impl From<ComunicadoRecibido> for ComunicadoRecibidoRespuesta {
    fn from(r: ComunicadoRecibido) -> Self {
        Self {
            comunicado: r.comunicado.into(),
            leido_en: r.leido_en,
            enterado_en: r.enterado_en,
        }
    }
}

/// Adjunto con enlace de descarga de 5 minutos.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdjuntoComunicadoRespuesta {
    pub archivo_id: Uuid,
    pub nombre: String,
    #[schema(example = "application/pdf")]
    pub tipo_mime: String,
    pub url: String,
}

impl From<AdjuntoComunicado> for AdjuntoComunicadoRespuesta {
    fn from(a: AdjuntoComunicado) -> Self {
        Self {
            archivo_id: a.archivo_id.as_uuid(),
            nombre: a.nombre,
            tipo_mime: a.tipo_mime,
            url: a.url,
        }
    }
}

/// Comunicado abierto por su destinatario.
#[derive(Debug, Serialize, ToSchema)]
pub struct ComunicadoAbiertoRespuesta {
    #[serde(flatten)]
    pub recibido: ComunicadoRecibidoRespuesta,
    pub enlaces_adjuntos: Vec<AdjuntoComunicadoRespuesta>,
}

impl From<ComunicadoAbierto> for ComunicadoAbiertoRespuesta {
    fn from(a: ComunicadoAbierto) -> Self {
        Self {
            recibido: a.recibido.into(),
            enlaces_adjuntos: a.adjuntos.into_iter().map(Into::into).collect(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Administración
// ---------------------------------------------------------------------------------------------

/// Comunicados de la PH con sus contadores, del más reciente al más antiguo.
#[utoipa::path(
    get,
    path = "/comunicados",
    tag = ETIQUETA_COMUNICADOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ParametrosPagina),
    responses(
        (status = OK, description = "Comunicados", body = PaginaRespuesta<ComunicadoConResumenRespuesta>),
        (status = FORBIDDEN, description = "Sin `comunicados.gestionar` (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<ComunicadoConResumenRespuesta>>> {
    let pagina =
        uc::listar_comunicados(&state.unidad, &RelojSistema, &en.ctx, p.validar()?).await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Crea un comunicado en borrador.
#[utoipa::path(
    post,
    path = "/comunicados",
    tag = ETIQUETA_COMUNICADOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = ComunicadoPeticion,
    responses(
        (status = CREATED, description = "Borrador creado", body = ComunicadoRespuesta),
        (status = FORBIDDEN, description = "Sin `comunicados.gestionar` (`sin_permiso`) o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "Un adjunto no existe (`no_encontrado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Contenido, finalidad o audiencia inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<ComunicadoPeticion>,
) -> ApiResult<(StatusCode, Json<ComunicadoRespuesta>)> {
    let c = uc::crear_comunicado(&state.unidad, &RelojSistema, &en.ctx, p.a_caso_uso()?).await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Un comunicado con su tablero de entrega y lectura.
#[utoipa::path(
    get,
    path = "/comunicados/{comunicado_id}",
    tag = ETIQUETA_COMUNICADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("comunicado_id" = Uuid, Path, description = "Comunicado"),
    ),
    responses(
        (status = OK, description = "Comunicado y tablero", body = ComunicadoConResumenRespuesta),
        (status = FORBIDDEN, description = "Sin `comunicados.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe (`no_encontrado`)", body = ErrorRespuesta),
    )
)]
pub async fn ver(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ComunicadoConResumenRespuesta>> {
    let c = uc::ver_comunicado(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ComunicadoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(c.into()))
}

/// Reemplaza el contenido de un borrador.
#[utoipa::path(
    put,
    path = "/comunicados/{comunicado_id}",
    tag = ETIQUETA_COMUNICADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("comunicado_id" = Uuid, Path, description = "Comunicado"),
    ),
    request_body = ComunicadoPeticion,
    responses(
        (status = OK, description = "Borrador actualizado", body = ComunicadoRespuesta),
        (status = FORBIDDEN, description = "Sin `comunicados.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe (`no_encontrado`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya no es borrador (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Datos inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn editar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<ComunicadoPeticion>,
) -> ApiResult<Json<ComunicadoRespuesta>> {
    let c = uc::editar_comunicado(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ComunicadoId::from_uuid(ruta.1),
        p.a_caso_uso()?,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Publica un borrador: sin `programado_para` sale de inmediato (push, correo y, si es
/// importante, WhatsApp a quien no tiene la app).
#[utoipa::path(
    post,
    path = "/comunicados/{comunicado_id}/publicar",
    tag = ETIQUETA_COMUNICADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("comunicado_id" = Uuid, Path, description = "Comunicado"),
    ),
    request_body = PublicarPeticion,
    responses(
        (status = OK, description = "Enviado o programado", body = ComunicadoRespuesta),
        (status = FORBIDDEN, description = "Sin `comunicados.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe (`no_encontrado`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "No es borrador (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Programado a más de un año (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn publicar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PublicarPeticion>,
) -> ApiResult<Json<ComunicadoRespuesta>> {
    let c = uc::publicar_comunicado(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ComunicadoId::from_uuid(ruta.1),
        p.programado_para,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Devuelve un comunicado programado a borrador.
#[utoipa::path(
    post,
    path = "/comunicados/{comunicado_id}/retirar",
    tag = ETIQUETA_COMUNICADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("comunicado_id" = Uuid, Path, description = "Comunicado"),
    ),
    responses(
        (status = OK, description = "De nuevo en borrador", body = ComunicadoRespuesta),
        (status = FORBIDDEN, description = "Sin `comunicados.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe (`no_encontrado`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está programado (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn retirar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ComunicadoRespuesta>> {
    let c = uc::retirar_comunicado(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ComunicadoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(c.into()))
}

/// Anula un borrador o un programado (lo enviado no se anula).
#[utoipa::path(
    post,
    path = "/comunicados/{comunicado_id}/anular",
    tag = ETIQUETA_COMUNICADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("comunicado_id" = Uuid, Path, description = "Comunicado"),
    ),
    responses(
        (status = OK, description = "Anulado", body = ComunicadoRespuesta),
        (status = FORBIDDEN, description = "Sin `comunicados.gestionar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe (`no_encontrado`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya se envió (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn anular(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ComunicadoRespuesta>> {
    let c = uc::anular_comunicado(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ComunicadoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(c.into()))
}

// ---------------------------------------------------------------------------------------------
// Cartelera
// ---------------------------------------------------------------------------------------------

/// Comunicados recibidos por la persona de la cuenta (vacío si no tiene persona en la PH).
#[utoipa::path(
    get,
    path = "/mis-comunicados",
    tag = ETIQUETA_COMUNICADOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ParametrosPagina),
    responses(
        (status = OK, description = "Cartelera", body = PaginaRespuesta<ComunicadoRecibidoRespuesta>),
        (status = FORBIDDEN, description = "Sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn mis_comunicados(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<ComunicadoRecibidoRespuesta>>> {
    let pagina = uc::mis_comunicados(&state.unidad, &RelojSistema, &en.ctx, p.validar()?).await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Abre un comunicado recibido: registra la lectura y entrega los adjuntos con enlaces de
/// descarga de 5 minutos.
#[utoipa::path(
    get,
    path = "/mis-comunicados/{comunicado_id}",
    tag = ETIQUETA_COMUNICADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("comunicado_id" = Uuid, Path, description = "Comunicado"),
    ),
    responses(
        (status = OK, description = "Comunicado", body = ComunicadoAbiertoRespuesta),
        (status = NOT_FOUND, description = "No es destinatario (`no_encontrado`)", body = ErrorRespuesta),
    )
)]
pub async fn abrir_mio(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ComunicadoAbiertoRespuesta>> {
    let a = uc::abrir_mi_comunicado(
        &state.unidad,
        &RelojSistema,
        state.archivos.as_ref(),
        &en.ctx,
        ComunicadoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(a.into()))
}

/// Registra el «Enterado» (idempotente).
#[utoipa::path(
    post,
    path = "/mis-comunicados/{comunicado_id}/enterado",
    tag = ETIQUETA_COMUNICADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("comunicado_id" = Uuid, Path, description = "Comunicado"),
    ),
    responses(
        (status = OK, description = "Enterado registrado", body = ComunicadoRecibidoRespuesta),
        (status = NOT_FOUND, description = "No es destinatario (`no_encontrado`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "El comunicado no pide confirmación (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn enterado(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ComunicadoRecibidoRespuesta>> {
    let r = uc::confirmar_enterado(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ComunicadoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(r.into()))
}
