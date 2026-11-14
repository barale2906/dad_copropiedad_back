//! Rutas de una entidad cualquiera: `/api/v1/entidades/{entidad_id}/…` (ADR 0016). Sirven a
//! cualquier empresa —PH, empresa cliente o la plataforma— con el extractor
//! [`crate::sesion::EnEntidad`]; en una copropiedad, el id es el mismo de
//! `/copropiedades/{copropiedad_id}`. Lo común: capacidades, roles, archivos, auditoría,
//! terceros y lo tributario; y los módulos de cualquier entidad: contabilidad y facturación.
//! La ficha genérica (nombre, NIT, ubicación, contacto) es `GET/PATCH /entidades/{entidad_id}`;
//! la PH tiene además su ficha completa en `/copropiedades/{copropiedad_id}`.

use application::{
    casos_uso::{
        entidades::{self as uc_ficha, CambiosEntidad},
        modulos::capacidades as uc_capacidades,
    },
    puertos::{RelojSistema, entidades::FichaEntidad},
};
use axum::extract::State;
use domain::{CodigoDane, DireccionCorreo, EstadoCopropiedad, Nit, Telefono};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::ficha::CapacidadesRespuesta;
use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::Json,
    openapi::ETIQUETA_CONTABILIDAD,
    sesion::EnEntidad,
    state::AppState,
};

/// Router de una entidad (se monta en `/api/v1/entidades/{entidad_id}`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(ver_ficha, actualizar_ficha))
        .routes(routes!(capacidades_entidad))
        .merge(super::roles::rutas())
        .merge(super::archivos::rutas_entidad())
        .merge(super::auditoria::rutas())
        .merge(super::contabilidad::rutas_terceros())
        .nest(
            "/tributario",
            super::contabilidad::tributario::rutas_entidad(),
        )
        .nest("/contabilidad", super::contabilidad::rutas())
        .nest("/facturacion", super::facturacion::rutas())
}

/// Permisos de la cuenta en la entidad y módulos habilitados (para cualquier entidad).
#[utoipa::path(
    get,
    path = "/capacidades",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Permisos y módulos", body = CapacidadesRespuesta),
        (status = NOT_FOUND, description = "La cuenta no tiene roles en la entidad (`no_encontrado`)", body = ErrorRespuesta),
    )
)]
pub async fn capacidades_entidad(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<CapacidadesRespuesta>> {
    let c = uc_capacidades(&state.unidad, &RelojSistema, &en.ctx).await?;
    Ok(Json(c.into()))
}

/// Datos generales de la entidad.
#[derive(Debug, Serialize, ToSchema)]
pub struct FichaEntidadRespuesta {
    #[schema(example = "Tienda La Esquina SAS")]
    pub nombre: String,
    pub razon_social: Option<String>,
    #[schema(example = "900123456-8")]
    pub nit: Option<String>,
    pub direccion: Option<String>,
    /// Código DANE del municipio.
    #[schema(example = "11001")]
    pub municipio_dane: Option<String>,
    pub departamento: Option<String>,
    pub barrio: Option<String>,
    #[schema(value_type = Option<String>, example = "4.711")]
    pub latitud: Option<Decimal>,
    #[schema(value_type = Option<String>, example = "-74.0721")]
    pub longitud: Option<Decimal>,
    pub telefono: Option<String>,
    pub correo: Option<String>,
    pub sitio_web: Option<String>,
    /// Zona IANA en que se muestran fechas y se evalúan horarios.
    #[schema(example = "America/Bogota")]
    pub zona_horaria: String,
    /// Lo cambia solo la plataforma.
    #[schema(value_type = crate::esquemas::EstadoCopropiedad)]
    pub estado: EstadoCopropiedad,
}

impl From<FichaEntidad> for FichaEntidadRespuesta {
    fn from(f: FichaEntidad) -> Self {
        Self {
            nombre: f.nombre,
            razon_social: f.razon_social,
            nit: f.nit.map(|n| n.to_string()),
            direccion: f.direccion,
            municipio_dane: f.municipio_dane.map(|c| c.as_str().to_owned()),
            departamento: f.departamento,
            barrio: f.barrio,
            latitud: f.latitud,
            longitud: f.longitud,
            telefono: f.telefono.map(|t| t.as_str().to_owned()),
            correo: f.correo.map(|c| c.as_str().to_owned()),
            sitio_web: f.sitio_web,
            zona_horaria: f.zona_horaria,
            estado: f.estado,
        }
    }
}

/// Ficha de la entidad: nombre, NIT, ubicación, contacto y zona horaria (cualquier entidad).
#[utoipa::path(
    get,
    path = "",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Ficha", body = FichaEntidadRespuesta),
        (status = FORBIDDEN, description = "Sin `copropiedad.datos.ver` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La cuenta no tiene roles en la entidad (`no_encontrado`)", body = ErrorRespuesta),
    )
)]
pub async fn ver_ficha(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<FichaEntidadRespuesta>> {
    Ok(Json(
        uc_ficha::ver_ficha_entidad(&state.unidad, &en.ctx)
            .await?
            .into(),
    ))
}

/// Cambios a la ficha de la entidad. Los campos omitidos no cambian; un texto vacío (`""`) lo
/// borra.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct CambiosEntidadPeticion {
    pub nombre: Option<String>,
    pub razon_social: Option<String>,
    /// Con o sin dígito de verificación; se valida con el algoritmo de la DIAN.
    #[schema(value_type = Option<String>, example = "900123456-8")]
    pub nit: Option<Nit>,
    pub direccion: Option<String>,
    #[schema(value_type = Option<String>, example = "11001")]
    pub municipio_dane: Option<CodigoDane>,
    pub departamento: Option<String>,
    pub barrio: Option<String>,
    #[schema(value_type = Option<String>, example = "4.711")]
    pub latitud: Option<Decimal>,
    #[schema(value_type = Option<String>, example = "-74.0721")]
    pub longitud: Option<Decimal>,
    #[schema(value_type = Option<String>, example = "576015551234")]
    pub telefono: Option<Telefono>,
    #[schema(value_type = Option<String>, example = "contacto@empresa.co")]
    pub correo: Option<DireccionCorreo>,
    pub sitio_web: Option<String>,
    #[schema(example = "America/Bogota")]
    pub zona_horaria: Option<String>,
}

impl From<CambiosEntidadPeticion> for CambiosEntidad {
    fn from(p: CambiosEntidadPeticion) -> Self {
        Self {
            nombre: p.nombre,
            razon_social: p.razon_social,
            nit: p.nit,
            direccion: p.direccion,
            municipio_dane: p.municipio_dane,
            departamento: p.departamento,
            barrio: p.barrio,
            latitud: p.latitud,
            longitud: p.longitud,
            telefono: p.telefono,
            correo: p.correo,
            sitio_web: p.sitio_web,
            zona_horaria: p.zona_horaria,
        }
    }
}

/// Actualiza los datos generales de la entidad (en una PH, lo mismo que su ficha completa).
#[utoipa::path(
    patch,
    path = "",
    tag = ETIQUETA_CONTABILIDAD,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = CambiosEntidadPeticion,
    responses(
        (status = OK, description = "Datos actualizados", body = FichaEntidadRespuesta),
        (status = FORBIDDEN, description = "Sin `copropiedad.datos.editar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La cuenta no tiene roles en la entidad (`no_encontrado`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Otra entidad tiene ese NIT (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Dato inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn actualizar_ficha(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<CambiosEntidadPeticion>,
) -> ApiResult<Json<FichaEntidadRespuesta>> {
    Ok(Json(
        uc_ficha::actualizar_ficha_entidad(&state.unidad, &en.ctx, p.into())
            .await?
            .into(),
    ))
}
