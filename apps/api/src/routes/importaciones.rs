//! Importaciones masivas: `/api/v1/copropiedades/{copropiedad_id}/importaciones/…`.
//!
//! Flujo: `POST /importaciones/bienes-privados` (cuerpo = el archivo CSV o XLSX) devuelve la
//! vista previa con los errores por fila → si no hay errores, `POST /importaciones/{id}/confirmar`
//! lo aplica en una transacción. `GET /importaciones/bienes-privados/plantilla` entrega la
//! plantilla CSV.

use application::{
    casos_uso::importaciones::{self as uc, FilaVistaPrevia, VistaPrevia},
    puertos::{
        RelojSistema,
        copropiedades::{Importacion, ResumenImportacion},
    },
};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use domain::{
    CuadroCoeficientesId, EstadoImportacion, ImportacionId, NombreArchivo, TipoArchivo,
    TipoBienPrivado, importacion::ErrorFila,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_IMPORTACIONES,
    sesion::EnCopropiedad,
    state::AppState,
};

/// Plantilla CSV de bienes privados (separada por `;`, como la guarda Excel en español).
pub const PLANTILLA_BIENES_PRIVADOS: &str = "agrupacion;numero;codigo;tipo;area_m2;coeficiente;\
propietario;documento;telefono;correo;porcentaje\n\
T3;506;;apartamento;72,5;0,5230 %;Ana Pérez;52123456;573001234567;ana@correo.co;50\n\
T3;506;;;;;Luis Gómez;79123456;;;50\n\
T3;P-12;;parqueadero;12;0,0800 %;;;;;\n";

/// Rutas de importaciones.
pub fn rutas() -> OpenApiRouter<AppState> {
    let limite = usize::try_from(TipoArchivo::Xlsx.tamano_maximo()).unwrap_or(usize::MAX);
    OpenApiRouter::new()
        .routes(routes!(importar))
        .layer(DefaultBodyLimit::max(limite))
        .routes(routes!(plantilla))
        .routes(routes!(ver))
        .routes(routes!(confirmar))
}

/// Error de una fila (sin el valor de la celda).
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorFilaRespuesta {
    /// Fila como se ve en la hoja de cálculo (la 1 es el encabezado).
    pub fila: u32,
    pub columna: Option<String>,
    pub mensaje: String,
}

impl From<ErrorFila> for ErrorFilaRespuesta {
    fn from(e: ErrorFila) -> Self {
        Self {
            fila: e.fila,
            columna: e.columna,
            mensaje: e.mensaje,
        }
    }
}

/// Resumen de lo que hace (o hizo) la importación.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResumenRespuesta {
    pub bienes_privados: u32,
    pub agrupaciones_nuevas: u32,
    pub titulares: u32,
    /// Solo se conoce al aplicar.
    pub personas_nuevas: u32,
    pub coeficientes: u32,
    /// Borrador con los coeficientes del archivo (actívelo en `/cuadros-coeficientes`).
    pub cuadro_borrador_id: Option<Uuid>,
}

impl From<ResumenImportacion> for ResumenRespuesta {
    fn from(r: ResumenImportacion) -> Self {
        Self {
            bienes_privados: r.bienes_privados,
            agrupaciones_nuevas: r.agrupaciones_nuevas,
            titulares: r.titulares,
            personas_nuevas: r.personas_nuevas,
            coeficientes: r.coeficientes,
            cuadro_borrador_id: r.cuadro_borrador_id.map(CuadroCoeficientesId::as_uuid),
        }
    }
}

/// Importación.
#[derive(Debug, Serialize, ToSchema)]
pub struct ImportacionRespuesta {
    pub id: Uuid,
    pub archivo_id: Uuid,
    /// `validada` (se puede confirmar), `con_errores` o `aplicada`.
    #[schema(value_type = String, example = "validada")]
    pub estado: EstadoImportacion,
    pub total_filas: u32,
    pub filas_validas: u32,
    pub errores: Vec<ErrorFilaRespuesta>,
    pub resumen: ResumenRespuesta,
    pub creada_en: DateTime<Utc>,
    pub aplicada_en: Option<DateTime<Utc>>,
}

impl From<Importacion> for ImportacionRespuesta {
    fn from(i: Importacion) -> Self {
        Self {
            id: i.id.as_uuid(),
            archivo_id: i.archivo_id.as_uuid(),
            estado: i.estado,
            total_filas: i.total_filas,
            filas_validas: i.filas_validas,
            errores: i.errores.into_iter().map(Into::into).collect(),
            resumen: i.resumen.into(),
            creada_en: i.creada_en,
            aplicada_en: i.aplicada_en,
        }
    }
}

/// Fila de la vista previa (sin datos personales).
#[derive(Debug, Serialize, ToSchema)]
pub struct FilaVistaPreviaRespuesta {
    pub fila: u32,
    #[schema(example = "T3-506")]
    pub codigo: String,
    #[schema(value_type = String, example = "apartamento")]
    pub tipo: TipoBienPrivado,
    pub agrupacion: Option<String>,
    /// La agrupación no existe y se creará.
    pub agrupacion_nueva: bool,
    #[schema(value_type = Option<String>, example = "0.00523")]
    pub coeficiente: Option<Decimal>,
    pub titulares: u32,
}

impl From<FilaVistaPrevia> for FilaVistaPreviaRespuesta {
    fn from(f: FilaVistaPrevia) -> Self {
        Self {
            fila: f.fila,
            codigo: f.codigo.as_str().to_owned(),
            tipo: f.tipo,
            agrupacion_nueva: f.agrupacion.as_ref().is_some_and(|(_, nueva)| *nueva),
            agrupacion: f.agrupacion.map(|(c, _)| c.as_str().to_owned()),
            coeficiente: f.coeficiente.map(|c| c.valor()),
            titulares: f.titulares,
        }
    }
}

/// Importación con su vista previa.
#[derive(Debug, Serialize, ToSchema)]
pub struct VistaPreviaRespuesta {
    pub importacion: ImportacionRespuesta,
    pub filas: Vec<FilaVistaPreviaRespuesta>,
}

impl From<VistaPrevia> for VistaPreviaRespuesta {
    fn from(v: VistaPrevia) -> Self {
        Self {
            importacion: v.importacion.into(),
            filas: v.filas.into_iter().map(Into::into).collect(),
        }
    }
}

/// Nombre del archivo que se sube.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DatosPlanilla {
    #[param(example = "bienes_privados.xlsx")]
    pub nombre: String,
}

/// Sube y valida una planilla de bienes privados (CSV o XLSX). No aplica nada: devuelve la
/// vista previa con los errores por fila. El tipo se detecta por el contenido.
#[utoipa::path(
    post,
    path = "/importaciones/bienes-privados",
    tag = ETIQUETA_IMPORTACIONES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), DatosPlanilla),
    request_body(content = Vec<u8>, content_type = "application/octet-stream",
                 description = "Contenido del CSV o XLSX"),
    responses(
        (status = CREATED, description = "Importación registrada (revise `estado` y `errores`)", body = VistaPreviaRespuesta),
        (status = FORBIDDEN, description = "Sin `importaciones.ejecutar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es CSV/XLSX o no se puede leer (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn importar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(d): Query<DatosPlanilla>,
    cuerpo: Bytes,
) -> ApiResult<(StatusCode, Json<VistaPreviaRespuesta>)> {
    let vista = uc::importar_bienes_privados(
        &state.unidad,
        state.archivos.as_ref(),
        &state.planillas,
        &RelojSistema,
        &en.ctx,
        NombreArchivo::new(&d.nombre)?,
        cuerpo.to_vec(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(vista.into())))
}

/// Plantilla CSV de bienes privados con ejemplos (copropietarios en filas seguidas).
#[utoipa::path(
    get,
    path = "/importaciones/bienes-privados/plantilla",
    tag = ETIQUETA_IMPORTACIONES,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses((status = OK, description = "Plantilla CSV", content_type = "text/csv", body = String))
)]
pub async fn plantilla(_en: EnCopropiedad) -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/csv; charset=utf-8"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"bienes_privados.csv\""),
            ),
        ],
        PLANTILLA_BIENES_PRIVADOS,
    )
        .into_response()
}

fn id_importacion(ruta: (Uuid, Uuid)) -> ImportacionId {
    ImportacionId::from_uuid(ruta.1)
}

/// Una importación con sus errores y su resumen.
#[utoipa::path(
    get,
    path = "/importaciones/{importacion_id}",
    tag = ETIQUETA_IMPORTACIONES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("importacion_id" = Uuid, Path, description = "Importación"),
    ),
    responses(
        (status = OK, description = "Importación", body = ImportacionRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ImportacionRespuesta>> {
    Ok(Json(
        uc::ver_importacion(&state.unidad, &en.ctx, id_importacion(ruta))
            .await?
            .into(),
    ))
}

/// Aplica una importación validada en una sola transacción: agrupaciones, bienes privados,
/// propietarios (con su rol si tienen cuenta) y un cuadro **borrador** con los coeficientes.
#[utoipa::path(
    post,
    path = "/importaciones/{importacion_id}/confirmar",
    tag = ETIQUETA_IMPORTACIONES,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("importacion_id" = Uuid, Path, description = "Importación"),
    ),
    responses(
        (status = OK, description = "Aplicada", body = ImportacionRespuesta),
        (status = CONFLICT, description = "Ya aplicada, con errores o los datos cambiaron (`conflicto`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn confirmar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ImportacionRespuesta>> {
    let i = uc::confirmar_importacion(
        &state.unidad,
        state.archivos.as_ref(),
        &state.planillas,
        &RelojSistema,
        &en.ctx,
        id_importacion(ruta),
    )
    .await?;
    Ok(Json(i.into()))
}
