//! Búsqueda de proveedores y enlace público del directorio (plan 08, fase 2; ADR 0014), en
//! `/api/v1/copropiedades/{copropiedad_id}/directorio/…`:
//!
//! - `busquedas`: el residente busca desde la app y, si quiere, autoriza que los proveedores
//!   sugeridos lo contacten (`busquedas/{solicitud_id}/contacto`).
//! - `enlace-publico`: la administración genera o revoca el enlace para los grupos de WhatsApp
//!   (`directorio.compartir`). El token se entrega una sola vez.
//! - `publico`: el directorio sin sesión; el token del enlace va en `X-Enlace-Directorio`.

use application::{
    casos_uso::busqueda_proveedores::{self as uc, NuevaBusqueda},
    puertos::{RelojSistema, directorio::FiltroDirectorio},
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{CanalSolicitud, SolicitudProveedorId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::directorio::{
    CategoriaDirectorioRespuesta, FichaDirectorioRespuesta, FiltroDirectorioQuery, categoria,
};
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    limite::LimitePublico,
    openapi::ETIQUETA_DIRECTORIO,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::{EnCopropiedad, EnlaceDirectorioPublico},
    state::AppState,
};

pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(buscar))
        .routes(routes!(autorizar_contacto))
        .routes(routes!(estado_enlace, generar_enlace, revocar_enlace))
        .routes(routes!(publico))
}

// ---------------------------------------------------------------------------------------------
// Búsqueda desde la app
// ---------------------------------------------------------------------------------------------

/// Lo que busca el residente.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BusquedaPeticion {
    /// Texto libre («busco costurera», «técnico de lavadoras»), hasta 100 caracteres útiles.
    #[schema(example = "busco costurera")]
    pub texto: String,
}

/// Sugerencias para una búsqueda.
#[derive(Debug, Serialize, ToSchema)]
pub struct BusquedaRespuesta {
    /// Para autorizar el contacto con estos proveedores.
    pub solicitud_id: Uuid,
    /// Categoría reconocida en el texto, si la hubo.
    pub categoria: Option<CategoriaDirectorioRespuesta>,
    /// Hasta 3 fichas: destacadas (solo con el módulo `publicidad`), mejor calificadas y más
    /// cercanas.
    pub sugerencias: Vec<FichaDirectorioRespuesta>,
    /// ¿Alguna recibe contactos por WhatsApp? (solo entonces se ofrece autorizar el contacto).
    pub contactables: bool,
}

/// Busca proveedores que alcanzan a la copropiedad y registra la búsqueda (el texto se borra a
/// los 90 días). No comparte nada con los proveedores.
#[utoipa::path(
    post,
    path = "/directorio/busquedas",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = BusquedaPeticion,
    responses(
        (status = CREATED, description = "Sugerencias", body = BusquedaRespuesta),
        (status = FORBIDDEN, description = "Sin `directorio.ver` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No se entiende qué busca o la cuenta no está en el censo (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn buscar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<BusquedaPeticion>,
) -> ApiResult<(StatusCode, Json<BusquedaRespuesta>)> {
    let persona = uc::persona_de_la_cuenta(&state.unidad, &en.ctx).await?;
    let r = uc::buscar_proveedores(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevaBusqueda {
            persona_id: persona,
            canal: CanalSolicitud::App,
            texto: p.texto,
            // Los reintentos de la app los resuelve `Idempotency-Key`; cada petición es una
            // búsqueda nueva.
            clave: Uuid::new_v4().to_string(),
        },
    )
    .await?;
    let contactables = r.hay_contactables();
    Ok((
        StatusCode::CREATED,
        Json(BusquedaRespuesta {
            solicitud_id: r.solicitud.id.as_uuid(),
            categoria: r.categoria.map(Into::into),
            sugerencias: r.sugerencias.into_iter().map(Into::into).collect(),
            contactables,
        }),
    ))
}

/// Contacto compartido.
#[derive(Debug, Serialize, ToSchema)]
pub struct ContactoAutorizadoRespuesta {
    /// Proveedores que recibirán su nombre y su celular por WhatsApp.
    #[schema(example = json!(["Modistería Rosa"]))]
    pub proveedores: Vec<String>,
    /// ¿Se programó la pregunta «¿Le sirvió?» (en 7 días, por WhatsApp)?
    pub seguimiento: bool,
}

/// Autoriza expresamente que los proveedores sugeridos en la búsqueda lo contacten: solo a
/// ellos y solo para esta solicitud se les comparte su nombre y su celular del censo (Ley
/// 1581). Repetirlo no vuelve a enviar nada.
#[utoipa::path(
    post,
    path = "/directorio/busquedas/{solicitud_id}/contacto",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("solicitud_id" = Uuid, Path, description = "Búsqueda"),
    ),
    responses(
        (status = OK, description = "Contacto compartido", body = ContactoAutorizadoRespuesta),
        (status = FORBIDDEN, description = "Sin `directorio.ver` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La búsqueda no existe o es de otra persona", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Sin celular en el censo o ningún proveedor recibe WhatsApp (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn autorizar_contacto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ContactoAutorizadoRespuesta>> {
    let persona = uc::persona_de_la_cuenta(&state.unidad, &en.ctx).await?;
    let c = uc::autorizar_contacto(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        SolicitudProveedorId::from_uuid(ruta.1),
        persona,
        None,
    )
    .await?;
    Ok(Json(ContactoAutorizadoRespuesta {
        proveedores: c.proveedores,
        seguimiento: c.seguimiento,
    }))
}

// ---------------------------------------------------------------------------------------------
// Enlace público (administración)
// ---------------------------------------------------------------------------------------------

/// Estado del enlace público del directorio.
#[derive(Debug, Serialize, ToSchema)]
pub struct EstadoEnlaceRespuesta {
    /// ¿Hay un enlace vigente?
    pub activo: bool,
    pub creado_en: Option<DateTime<Utc>>,
    /// Enlace que abre el chat de la PH con «Busco …» escrito.
    #[schema(example = "https://wa.me/573000000000?text=Busco%20")]
    pub whatsapp_url: Option<String>,
}

/// Enlace recién generado.
#[derive(Debug, Serialize, ToSchema)]
pub struct EnlaceGeneradoRespuesta {
    /// Token del enlace: se entrega **una sola vez** (el front arma la página pública con él).
    pub token: String,
    pub creado_en: DateTime<Utc>,
    #[schema(example = "https://wa.me/573000000000?text=Busco%20")]
    pub whatsapp_url: Option<String>,
}

/// Estado del enlace público del directorio y el enlace `wa.me` para los grupos.
#[utoipa::path(
    get,
    path = "/directorio/enlace-publico",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Estado", body = EstadoEnlaceRespuesta),
        (status = FORBIDDEN, description = "Sin `directorio.compartir` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn estado_enlace(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<EstadoEnlaceRespuesta>> {
    let e = uc::estado_enlace_directorio(&state.unidad, &en.ctx).await?;
    Ok(Json(EstadoEnlaceRespuesta {
        activo: e.creado_en.is_some(),
        creado_en: e.creado_en,
        whatsapp_url: e.whatsapp,
    }))
}

/// Genera el enlace público del directorio para fijarlo en los grupos de WhatsApp de la PH. Si
/// ya había uno, el anterior deja de funcionar.
#[utoipa::path(
    post,
    path = "/directorio/enlace-publico",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = CREATED, description = "Enlace generado", body = EnlaceGeneradoRespuesta),
        (status = FORBIDDEN, description = "Sin `directorio.compartir` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn generar_enlace(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<(StatusCode, Json<EnlaceGeneradoRespuesta>)> {
    let e = uc::generar_enlace_directorio(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok((
        StatusCode::CREATED,
        Json(EnlaceGeneradoRespuesta {
            token: e.token,
            creado_en: e.creado_en,
            whatsapp_url: e.whatsapp,
        }),
    ))
}

/// Revoca el enlace público del directorio.
#[utoipa::path(
    delete,
    path = "/directorio/enlace-publico",
    tag = ETIQUETA_DIRECTORIO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = NO_CONTENT, description = "Revocado"),
        (status = FORBIDDEN, description = "Sin `directorio.compartir` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No había enlace (`no_encontrado`)", body = ErrorRespuesta),
    )
)]
pub async fn revocar_enlace(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<StatusCode> {
    if uc::revocar_enlace_directorio(&state.unidad, &en.ctx).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::App(application::AppError::NoEncontrado(
            "enlace del directorio",
        )))
    }
}

// ---------------------------------------------------------------------------------------------
// Directorio público (sin sesión)
// ---------------------------------------------------------------------------------------------

/// Directorio público de la copropiedad.
#[derive(Debug, Serialize, ToSchema)]
pub struct DirectorioPublicoRespuesta {
    #[schema(example = "Conjunto Alameda")]
    pub copropiedad: String,
    /// Abre el chat de la PH con «Busco …» escrito (para pegarlo en el grupo).
    #[schema(example = "https://wa.me/573000000000?text=Busco%20")]
    pub whatsapp_url: Option<String>,
    pub fichas: PaginaRespuesta<FichaDirectorioRespuesta>,
}

/// Directorio de comercios y servicios de la copropiedad para los grupos de WhatsApp (sin
/// sesión: el token del enlace en `X-Enlace-Directorio` es la credencial). Sin reseñas: solo
/// el promedio.
#[utoipa::path(
    get,
    path = "/directorio/publico",
    tag = ETIQUETA_DIRECTORIO,
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("X-Enlace-Directorio" = String, Header, description = "Token del enlace público"),
        FiltroDirectorioQuery,
        ParametrosPagina,
    ),
    responses(
        (status = OK, description = "Directorio", body = DirectorioPublicoRespuesta),
        (status = UNAUTHORIZED, description = "Token inválido o revocado (`no_autenticado`)", body = ErrorRespuesta),
        (status = TOO_MANY_REQUESTS, description = "Demasiadas peticiones (`demasiadas_peticiones`)", body = ErrorRespuesta),
    )
)]
pub async fn publico(
    _limite: LimitePublico,
    State(state): State<AppState>,
    enlace: EnlaceDirectorioPublico,
    Query(f): Query<FiltroDirectorioQuery>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<DirectorioPublicoRespuesta>> {
    let filtro = FiltroDirectorio {
        categoria: categoria(f.categoria.as_deref())?,
        texto: f.q,
        ..FiltroDirectorio::default()
    };
    let d = uc::directorio_publico(
        &state.unidad,
        &RelojSistema,
        enlace.tenant,
        &enlace.token,
        filtro,
        p.validar()?,
    )
    .await?;
    Ok(Json(DirectorioPublicoRespuesta {
        copropiedad: d.copropiedad,
        whatsapp_url: d.whatsapp,
        fichas: d.fichas.mapear(Into::into).into(),
    }))
}
