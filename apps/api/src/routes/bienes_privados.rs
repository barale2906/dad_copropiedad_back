//! Agrupaciones, bienes privados, anexos, usos exclusivos y titulares:
//! `/api/v1/copropiedades/{copropiedad_id}/agrupaciones`, `/bienes-privados/…` y
//! `/mis-bienes-privados`.

use application::{
    casos_uso::{
        bienes_privados::{
            self as uc, ArbolAgrupaciones, CambiosBienPrivado, FichaBienPrivado, NodoAgrupacion,
            NuevaAgrupacion, NuevoBienPrivado, NuevoUsoExclusivo,
        },
        titulares::{self as tit, DatosPersona, RefPersona, SolicitudTitular, TitularConPersona},
    },
    puertos::{
        RelojSistema,
        copropiedades::{Agrupacion, BienPrivado, FiltroBienes, Titularidad, UsoExclusivo},
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use domain::{
    AgrupacionId, ArchivoId, BienPrivadoId, CodigoAgrupacion, CodigoBienPrivado, CuentaId,
    DireccionCorreo, Documento, NumeroBienPrivado, PersonaId, Porcentaje, Telefono, TipoAgrupacion,
    TipoBienPrivado, TipoDocumento, TipoPersona, TipoTitularidad, TipoUsoExclusivo, UsoBienPrivado,
    UsoExclusivoId,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_BIENES_PRIVADOS,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de bienes privados dentro de una copropiedad.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(arbol, crear_agrupacion))
        .routes(routes!(listar, crear))
        .routes(routes!(ficha, editar))
        .routes(routes!(inactivar))
        .routes(routes!(vincular))
        .routes(routes!(asignar_uso_exclusivo))
        .routes(routes!(terminar_uso_exclusivo))
        .routes(routes!(listar_titulares, registrar_titulares))
        .routes(routes!(transferir))
        .routes(routes!(designar_representante))
        .routes(routes!(mis_bienes))
}

// ---------------------------------------------------------------------------------------------
// Agrupaciones
// ---------------------------------------------------------------------------------------------

/// Agrupación (edificio, etapa, bloque…).
#[derive(Debug, Serialize, ToSchema)]
pub struct AgrupacionRespuesta {
    pub id: Uuid,
    /// `edificio`, `etapa`, `bloque`, `manzana`, `interior` o `sector`.
    #[schema(value_type = String, example = "edificio")]
    pub tipo: TipoAgrupacion,
    #[schema(example = "Torre 3")]
    pub nombre: String,
    /// Código corto para la nomenclatura.
    #[schema(value_type = String, example = "T3")]
    pub codigo: CodigoAgrupacion,
    pub padre_id: Option<Uuid>,
    pub orden: i32,
}

impl From<Agrupacion> for AgrupacionRespuesta {
    fn from(a: Agrupacion) -> Self {
        Self {
            id: a.id.as_uuid(),
            tipo: a.tipo,
            nombre: a.nombre,
            codigo: a.codigo,
            padre_id: a.padre_id.map(AgrupacionId::as_uuid),
            orden: a.orden,
        }
    }
}

/// Bien privado en un listado o en el árbol.
#[derive(Debug, Serialize, ToSchema)]
pub struct BienPrivadoRespuesta {
    pub id: Uuid,
    #[schema(value_type = String, example = "T3-506")]
    pub codigo: CodigoBienPrivado,
    /// `apartamento`, `casa`, `local`, `oficina`, `consultorio`, `parqueadero`, `deposito`,
    /// `bodega`, `lote`, `finca` u `otro`.
    #[schema(value_type = String, example = "apartamento")]
    pub tipo: TipoBienPrivado,
    pub agrupacion_id: Option<Uuid>,
    #[schema(value_type = Option<String>, example = "506")]
    pub numero: Option<NumeroBienPrivado>,
    pub piso: Option<i16>,
    #[schema(value_type = Option<String>, example = "72.50")]
    pub area_privada_m2: Option<Decimal>,
    pub matricula_inmobiliaria: Option<String>,
    pub cedula_catastral: Option<String>,
    #[schema(value_type = String, example = "residencial")]
    pub uso: UsoBienPrivado,
    pub activo: bool,
    /// Bien al que está vinculado como anexo.
    pub bien_principal_id: Option<Uuid>,
    /// Fracción del cuadro vigente (`null` si aún no hay cuadro que lo incluya).
    #[schema(value_type = Option<String>, example = "0.00523000")]
    pub coeficiente: Option<Decimal>,
}

impl From<BienPrivado> for BienPrivadoRespuesta {
    fn from(b: BienPrivado) -> Self {
        Self {
            id: b.id.as_uuid(),
            codigo: b.codigo,
            tipo: b.tipo,
            agrupacion_id: b.agrupacion_id.map(AgrupacionId::as_uuid),
            numero: b.numero,
            piso: b.piso,
            area_privada_m2: b.area_privada_m2,
            matricula_inmobiliaria: b.matricula_inmobiliaria,
            cedula_catastral: b.cedula_catastral,
            uso: b.uso,
            activo: b.activo,
            bien_principal_id: b.bien_principal_id.map(BienPrivadoId::as_uuid),
            coeficiente: b.coeficiente.map(|c| c.valor()),
        }
    }
}

/// Nodo del árbol de agrupaciones.
#[derive(Debug, Serialize, ToSchema)]
#[schema(no_recursion)]
pub struct NodoRespuesta {
    pub agrupacion: AgrupacionRespuesta,
    pub bienes_privados: Vec<BienPrivadoRespuesta>,
    pub hijas: Vec<NodoRespuesta>,
}

impl From<NodoAgrupacion> for NodoRespuesta {
    fn from(n: NodoAgrupacion) -> Self {
        Self {
            agrupacion: n.agrupacion.into(),
            bienes_privados: n.bienes.into_iter().map(Into::into).collect(),
            hijas: n.hijas.into_iter().map(Into::into).collect(),
        }
    }
}

/// Árbol de agrupaciones con los bienes privados activos.
#[derive(Debug, Serialize, ToSchema)]
pub struct ArbolRespuesta {
    pub agrupaciones: Vec<NodoRespuesta>,
    /// Bienes privados sin agrupación (casas, lotes…).
    pub sin_agrupacion: Vec<BienPrivadoRespuesta>,
}

impl From<ArbolAgrupaciones> for ArbolRespuesta {
    fn from(a: ArbolAgrupaciones) -> Self {
        Self {
            agrupaciones: a.raices.into_iter().map(Into::into).collect(),
            sin_agrupacion: a.sin_agrupacion.into_iter().map(Into::into).collect(),
        }
    }
}

/// Árbol de agrupaciones (hasta 3 niveles) con sus bienes privados activos.
#[utoipa::path(
    get,
    path = "/agrupaciones",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Árbol", body = ArbolRespuesta),
        (status = FORBIDDEN, description = "Sin `bienes_privados.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn arbol(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<ArbolRespuesta>> {
    Ok(Json(
        uc::arbol_agrupaciones(&state.unidad, &en.ctx).await?.into(),
    ))
}

/// Agrupación nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaAgrupacionPeticion {
    #[schema(value_type = String, example = "edificio")]
    pub tipo: TipoAgrupacion,
    /// Etiqueta que elige la PH.
    #[schema(example = "Torre 3")]
    pub nombre: String,
    /// 1 a 10 letras o dígitos.
    #[schema(value_type = String, example = "T3")]
    pub codigo: CodigoAgrupacion,
    pub padre_id: Option<Uuid>,
    #[serde(default)]
    pub orden: i32,
}

/// Crea un edificio, etapa, bloque… (máximo 3 niveles).
#[utoipa::path(
    post,
    path = "/agrupaciones",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevaAgrupacionPeticion,
    responses(
        (status = CREATED, description = "Agrupación creada", body = AgrupacionRespuesta),
        (status = FORBIDDEN, description = "Sin `agrupaciones.editar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Código repetido en ese nivel (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Más de 3 niveles (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_agrupacion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevaAgrupacionPeticion>,
) -> ApiResult<(StatusCode, Json<AgrupacionRespuesta>)> {
    let a = uc::crear_agrupacion(
        &state.unidad,
        &en.ctx,
        NuevaAgrupacion {
            tipo: p.tipo,
            nombre: p.nombre,
            codigo: p.codigo,
            padre_id: p.padre_id.map(AgrupacionId::from_uuid),
            orden: p.orden,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(a.into())))
}

// ---------------------------------------------------------------------------------------------
// Bienes privados
// ---------------------------------------------------------------------------------------------

/// Filtros del listado.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroBienesParams {
    /// Fragmento del código (sin importar tildes ni mayúsculas).
    pub q: Option<String>,
    pub agrupacion_id: Option<Uuid>,
    /// Tipo de bien privado.
    #[param(value_type = Option<String>)]
    pub tipo: Option<TipoBienPrivado>,
    /// Incluir los inactivos.
    #[serde(default)]
    pub incluir_inactivos: bool,
}

/// Listado paginado de bienes privados.
#[utoipa::path(
    get,
    path = "/bienes-privados",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroBienesParams, ParametrosPagina),
    responses(
        (status = OK, description = "Página de bienes privados", body = PaginaRespuesta<BienPrivadoRespuesta>),
        (status = FORBIDDEN, description = "Sin `bienes_privados.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroBienesParams>,
    Query(pagina): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<BienPrivadoRespuesta>>> {
    let pagina = uc::listar_bienes_privados(
        &state.unidad,
        &en.ctx,
        FiltroBienes {
            texto: f.q.filter(|q| !q.trim().is_empty()),
            agrupacion_id: f.agrupacion_id.map(AgrupacionId::from_uuid),
            tipo: f.tipo,
            incluir_inactivos: f.incluir_inactivos,
        },
        pagina.validar()?,
    )
    .await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Bien privado nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoBienPeticion {
    #[schema(value_type = String, example = "apartamento")]
    pub tipo: TipoBienPrivado,
    pub agrupacion_id: Option<Uuid>,
    #[schema(value_type = Option<String>, example = "506")]
    pub numero: Option<NumeroBienPrivado>,
    /// Código propio (p. ej. `3506`). Si no viene, se calcula con la plantilla de la PH.
    #[schema(value_type = Option<String>, example = "3506")]
    pub codigo: Option<CodigoBienPrivado>,
    pub piso: Option<i16>,
    #[schema(value_type = Option<String>, example = "72.50")]
    pub area_privada_m2: Option<Decimal>,
    pub matricula_inmobiliaria: Option<String>,
    pub cedula_catastral: Option<String>,
    #[serde(default = "uso_residencial")]
    #[schema(value_type = String, example = "residencial")]
    pub uso: UsoBienPrivado,
}

const fn uso_residencial() -> UsoBienPrivado {
    UsoBienPrivado::Residencial
}

/// Crea un bien privado. Queda sin coeficiente hasta que se active un cuadro que lo incluya.
#[utoipa::path(
    post,
    path = "/bienes-privados",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoBienPeticion,
    responses(
        (status = CREATED, description = "Bien privado creado", body = BienPrivadoRespuesta),
        (status = FORBIDDEN, description = "Sin `bienes_privados.editar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Código repetido (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Falta el número o el código (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoBienPeticion>,
) -> ApiResult<(StatusCode, Json<BienPrivadoRespuesta>)> {
    let b = uc::crear_bien_privado(
        &state.unidad,
        &en.ctx,
        NuevoBienPrivado {
            tipo: p.tipo,
            agrupacion_id: p.agrupacion_id.map(AgrupacionId::from_uuid),
            numero: p.numero,
            codigo: p.codigo,
            piso: p.piso,
            area_privada_m2: p.area_privada_m2,
            matricula_inmobiliaria: p.matricula_inmobiliaria,
            cedula_catastral: p.cedula_catastral,
            uso: p.uso,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(b.into())))
}

/// Bien común en uso exclusivo.
#[derive(Debug, Serialize, ToSchema)]
pub struct UsoExclusivoRespuesta {
    pub id: Uuid,
    #[schema(value_type = String, example = "parqueadero")]
    pub tipo: TipoUsoExclusivo,
    #[schema(example = "Parqueadero 45")]
    pub codigo: String,
    pub descripcion: Option<String>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
}

impl From<UsoExclusivo> for UsoExclusivoRespuesta {
    fn from(u: UsoExclusivo) -> Self {
        Self {
            id: u.id.as_uuid(),
            tipo: u.tipo,
            codigo: u.codigo,
            descripcion: u.descripcion,
            vigente_desde: u.vigente_desde,
            vigente_hasta: u.vigente_hasta,
        }
    }
}

/// Titularidad vigente (sin los datos de la persona).
#[derive(Debug, Serialize, ToSchema)]
pub struct TitularidadRespuesta {
    pub id: Uuid,
    pub persona_id: Uuid,
    /// `propietario`, `nudo_propietario`, `usufructuario`, `fideicomiso` o `leasing`.
    #[schema(value_type = String, example = "propietario")]
    pub tipo: TipoTitularidad,
    #[schema(value_type = String, example = "50")]
    pub porcentaje: Decimal,
    /// Representa al bien ante la copropiedad (votos, notificaciones).
    pub es_representante: bool,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
}

impl From<Titularidad> for TitularidadRespuesta {
    fn from(t: Titularidad) -> Self {
        Self {
            id: t.id.as_uuid(),
            persona_id: t.persona_id.as_uuid(),
            tipo: t.tipo,
            porcentaje: t.porcentaje.valor(),
            es_representante: t.es_representante,
            vigente_desde: t.vigente_desde,
            vigente_hasta: t.vigente_hasta,
        }
    }
}

/// Ficha de un bien privado.
#[derive(Debug, Serialize, ToSchema)]
pub struct FichaBienRespuesta {
    pub bien_privado: BienPrivadoRespuesta,
    /// Parqueaderos o depósitos con folio propio vinculados a este bien.
    pub anexos: Vec<BienPrivadoRespuesta>,
    pub usos_exclusivos: Vec<UsoExclusivoRespuesta>,
    /// Vacío si no tiene `titulares.ver` (salvo que sea titular).
    pub titulares: Vec<TitularidadRespuesta>,
}

impl From<FichaBienPrivado> for FichaBienRespuesta {
    fn from(f: FichaBienPrivado) -> Self {
        Self {
            bien_privado: f.bien.into(),
            anexos: f.anexos.into_iter().map(Into::into).collect(),
            usos_exclusivos: f.usos_exclusivos.into_iter().map(Into::into).collect(),
            titulares: f.titulares.into_iter().map(Into::into).collect(),
        }
    }
}

/// Ficha de un bien privado: coeficiente vigente, anexos, usos exclusivos y titulares. Un
/// titular vigente ve la de su bien.
#[utoipa::path(
    get,
    path = "/bienes-privados/{bien_privado_id}",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("bien_privado_id" = Uuid, Path, description = "Bien privado"),
    ),
    responses(
        (status = OK, description = "Ficha", body = FichaBienRespuesta),
        (status = FORBIDDEN, description = "Sin `bienes_privados.ver` ni ser titular", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ficha(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<FichaBienRespuesta>> {
    let f = uc::ficha_bien_privado(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(f.into()))
}

/// Cambios a un bien privado (los omitidos no cambian; un texto vacío lo borra). El
/// coeficiente no se edita aquí: lo fija la activación de un cuadro.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct CambiosBienPeticion {
    #[schema(value_type = Option<String>)]
    pub tipo: Option<TipoBienPrivado>,
    pub agrupacion_id: Option<Uuid>,
    #[schema(value_type = Option<String>)]
    pub numero: Option<NumeroBienPrivado>,
    /// Código propio; si no viene y cambian número o agrupación, se recalcula.
    #[schema(value_type = Option<String>)]
    pub codigo: Option<CodigoBienPrivado>,
    pub piso: Option<i16>,
    #[schema(value_type = Option<String>)]
    pub area_privada_m2: Option<Decimal>,
    pub matricula_inmobiliaria: Option<String>,
    pub cedula_catastral: Option<String>,
    #[schema(value_type = Option<String>)]
    pub uso: Option<UsoBienPrivado>,
}

/// Edita un bien privado.
#[utoipa::path(
    patch,
    path = "/bienes-privados/{bien_privado_id}",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("bien_privado_id" = Uuid, Path, description = "Bien privado"),
    ),
    request_body = CambiosBienPeticion,
    responses(
        (status = OK, description = "Bien privado actualizado", body = BienPrivadoRespuesta),
        (status = FORBIDDEN, description = "Sin `bienes_privados.editar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Código repetido (`conflicto`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn editar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<CambiosBienPeticion>,
) -> ApiResult<Json<BienPrivadoRespuesta>> {
    let b = uc::editar_bien_privado(
        &state.unidad,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
        CambiosBienPrivado {
            tipo: p.tipo,
            agrupacion_id: p.agrupacion_id.map(AgrupacionId::from_uuid),
            numero: p.numero,
            codigo: p.codigo,
            piso: p.piso,
            area_privada_m2: p.area_privada_m2,
            matricula_inmobiliaria: p.matricula_inmobiliaria,
            cedula_catastral: p.cedula_catastral,
            uso: p.uso,
        },
    )
    .await?;
    Ok(Json(b.into()))
}

/// Inactiva un bien privado (conserva su historia; el próximo cuadro ya no lo incluye).
#[utoipa::path(
    post,
    path = "/bienes-privados/{bien_privado_id}/inactivar",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("bien_privado_id" = Uuid, Path, description = "Bien privado"),
    ),
    responses(
        (status = OK, description = "Bien privado inactivo", body = BienPrivadoRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn inactivar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<BienPrivadoRespuesta>> {
    let b = uc::inactivar_bien_privado(&state.unidad, &en.ctx, BienPrivadoId::from_uuid(ruta.1))
        .await?;
    Ok(Json(b.into()))
}

/// Bien principal de un anexo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VinculoPeticion {
    /// `null` desvincula.
    pub bien_principal_id: Option<Uuid>,
}

/// Vincula un parqueadero, depósito o bodega con folio propio a su bien principal (conserva su
/// coeficiente propio).
#[utoipa::path(
    put,
    path = "/bienes-privados/{bien_privado_id}/principal",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("bien_privado_id" = Uuid, Path, description = "Anexo"),
    ),
    request_body = VinculoPeticion,
    responses(
        (status = OK, description = "Vínculo actualizado", body = BienPrivadoRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No es parqueadero/depósito/bodega o el principal no sirve (`validacion`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn vincular(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<VinculoPeticion>,
) -> ApiResult<Json<BienPrivadoRespuesta>> {
    let b = uc::vincular_bienes_privados(
        &state.unidad,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
        p.bien_principal_id.map(BienPrivadoId::from_uuid),
    )
    .await?;
    Ok(Json(b.into()))
}

/// Asignación de un bien común en uso exclusivo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoUsoPeticion {
    #[schema(value_type = String, example = "parqueadero")]
    pub tipo: TipoUsoExclusivo,
    /// Identificación del bien común.
    #[schema(example = "Parqueadero 45")]
    pub codigo: String,
    pub descripcion: Option<String>,
    /// Desde cuándo (por defecto, ahora).
    pub vigente_desde: Option<DateTime<Utc>>,
}

/// Asigna un bien común en uso exclusivo (art. 22) al bien privado. No tiene coeficiente.
#[utoipa::path(
    post,
    path = "/bienes-privados/{bien_privado_id}/usos-exclusivos",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("bien_privado_id" = Uuid, Path, description = "Bien privado"),
    ),
    request_body = NuevoUsoPeticion,
    responses(
        (status = CREATED, description = "Asignado", body = UsoExclusivoRespuesta),
        (status = CONFLICT, description = "Ese bien común ya está asignado (`conflicto`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe o está inactivo", body = ErrorRespuesta),
    )
)]
pub async fn asignar_uso_exclusivo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<NuevoUsoPeticion>,
) -> ApiResult<(StatusCode, Json<UsoExclusivoRespuesta>)> {
    let u = uc::asignar_uso_exclusivo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
        NuevoUsoExclusivo {
            tipo: p.tipo,
            codigo: p.codigo,
            descripcion: p.descripcion,
            vigente_desde: p.vigente_desde,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(u.into())))
}

/// Termina una asignación de uso exclusivo (desde ahora; queda en el historial).
#[utoipa::path(
    delete,
    path = "/bienes-privados/{bien_privado_id}/usos-exclusivos/{uso_id}",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("bien_privado_id" = Uuid, Path, description = "Bien privado"),
        ("uso_id" = Uuid, Path, description = "Asignación"),
    ),
    responses(
        (status = NO_CONTENT, description = "Terminada"),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn terminar_uso_exclusivo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    uc::terminar_uso_exclusivo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
        UsoExclusivoId::from_uuid(ruta.2),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Titulares
// ---------------------------------------------------------------------------------------------

/// Titular con los datos de su persona (datos personales: solo con `titulares.ver`).
#[derive(Debug, Serialize, ToSchema)]
pub struct TitularRespuesta {
    pub titularidad: TitularidadRespuesta,
    pub nombre: String,
    /// `natural` o `juridica`.
    #[schema(value_type = String, example = "natural")]
    pub tipo_persona: TipoPersona,
    pub documento: Option<String>,
    pub telefono: Option<String>,
    pub correo: Option<String>,
    /// Cuenta de la plataforma vinculada.
    pub cuenta_id: Option<Uuid>,
}

impl From<TitularConPersona> for TitularRespuesta {
    fn from(t: TitularConPersona) -> Self {
        Self {
            titularidad: t.titularidad.into(),
            nombre: t.persona.nombre,
            tipo_persona: t.persona.tipo,
            documento: t.persona.documento.map(|d| d.as_str().to_owned()),
            telefono: t.persona.telefono.map(|x| x.as_str().to_owned()),
            correo: t.persona.correo.map(|c| c.as_str().to_owned()),
            cuenta_id: t.persona.cuenta_id.map(CuentaId::as_uuid),
        }
    }
}

/// Historial de titulares del bien privado (vigentes primero).
#[utoipa::path(
    get,
    path = "/bienes-privados/{bien_privado_id}/titulares",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("bien_privado_id" = Uuid, Path, description = "Bien privado"),
    ),
    responses(
        (status = OK, description = "Titulares", body = Vec<TitularRespuesta>),
        (status = FORBIDDEN, description = "Sin `titulares.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_titulares(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<TitularRespuesta>>> {
    let lista =
        tit::listar_titulares(&state.unidad, &en.ctx, BienPrivadoId::from_uuid(ruta.1)).await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Persona nueva (si el documento ya existe en la copropiedad se usa la registrada).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PersonaPeticion {
    #[serde(default = "persona_natural")]
    #[schema(value_type = String, example = "natural")]
    pub tipo: TipoPersona,
    #[schema(example = "Ana Pérez")]
    pub nombre: String,
    #[schema(value_type = String, example = "52123456")]
    pub documento: Documento,
    /// `CC`, `CE`, `TI`, `RC`, `PA`, `PPT` o `NIT`.
    #[schema(value_type = Option<String>, example = "CC")]
    pub tipo_documento: Option<TipoDocumento>,
    #[schema(value_type = Option<String>, example = "573001234567")]
    pub telefono: Option<Telefono>,
    #[schema(value_type = Option<String>)]
    pub correo: Option<DireccionCorreo>,
    /// Cuenta de la plataforma de esa persona (búsquela con `GET /cuentas`): recibe el rol
    /// `propietario`.
    pub cuenta_id: Option<Uuid>,
}

const fn persona_natural() -> TipoPersona {
    TipoPersona::Natural
}

/// Titular solicitado: `persona_id` de una persona registrada o `persona` nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TitularPeticion {
    pub persona_id: Option<Uuid>,
    pub persona: Option<PersonaPeticion>,
    #[serde(default = "propietario")]
    #[schema(value_type = String, example = "propietario")]
    pub tipo: TipoTitularidad,
    /// Porcentaje de propiedad (suma 100 por clase de derecho).
    #[schema(value_type = String, example = "100")]
    pub porcentaje: Porcentaje,
    /// Con varios titulares, exactamente uno lo es.
    #[serde(default)]
    pub es_representante: bool,
}

const fn propietario() -> TipoTitularidad {
    TipoTitularidad::Propietario
}

/// Persona registrada (`persona_id`) o nueva (`persona`), exactamente una de las dos.
///
/// # Errors
///
/// [`crate::error::ApiError::PeticionInvalida`] si vienen ambas o ninguna.
pub fn ref_persona(
    persona_id: Option<Uuid>,
    persona: Option<PersonaPeticion>,
) -> Result<RefPersona, crate::error::ApiError> {
    match (persona_id, persona) {
        (Some(id), None) => Ok(RefPersona::Existente(PersonaId::from_uuid(id))),
        (None, Some(p)) => Ok(RefPersona::Nueva(DatosPersona {
            tipo: p.tipo,
            nombre: p.nombre,
            documento: p.documento,
            telefono: p.telefono,
            correo: p.correo,
            cuenta_id: p.cuenta_id.map(CuentaId::from_uuid),
            tipo_documento: p.tipo_documento,
        })),
        _ => Err(crate::error::ApiError::PeticionInvalida(
            "indique `persona_id` o `persona`, no ambos".into(),
        )),
    }
}

impl TitularPeticion {
    fn a_solicitud(self) -> Result<SolicitudTitular, crate::error::ApiError> {
        let persona = ref_persona(self.persona_id, self.persona)?;
        Ok(SolicitudTitular {
            persona,
            tipo: self.tipo,
            porcentaje: self.porcentaje,
            es_representante: self.es_representante,
        })
    }
}

/// Conjunto de titulares.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TitularesPeticion {
    pub titulares: Vec<TitularPeticion>,
    /// Certificado de tradición o escritura (archivo ya subido), opcional.
    pub soporte_archivo_id: Option<Uuid>,
}

impl TitularesPeticion {
    fn solicitudes(self) -> Result<Vec<SolicitudTitular>, crate::error::ApiError> {
        self.titulares
            .into_iter()
            .map(TitularPeticion::a_solicitud)
            .collect()
    }
}

/// Registra los titulares de un bien privado que aún no tiene. Los que tienen cuenta reciben
/// el rol `propietario`.
#[utoipa::path(
    post,
    path = "/bienes-privados/{bien_privado_id}/titulares",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("bien_privado_id" = Uuid, Path, description = "Bien privado"),
    ),
    request_body = TitularesPeticion,
    responses(
        (status = CREATED, description = "Titulares registrados", body = Vec<TitularidadRespuesta>),
        (status = FORBIDDEN, description = "Sin `titulares.editar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya tiene titulares: use una transferencia (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No suman 100 % o sin representante (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_titulares(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<TitularesPeticion>,
) -> ApiResult<(StatusCode, Json<Vec<TitularidadRespuesta>>)> {
    let creadas = tit::registrar_titulares(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
        p.solicitudes()?,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(creadas.into_iter().map(Into::into).collect()),
    ))
}

/// Transfiere el bien privado (venta, sucesión): termina a los titulares vigentes —y su rol
/// `propietario`— y registra a los nuevos en una sola transacción.
#[utoipa::path(
    post,
    path = "/bienes-privados/{bien_privado_id}/transferencias",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("bien_privado_id" = Uuid, Path, description = "Bien privado"),
    ),
    request_body = TitularesPeticion,
    responses(
        (status = CREATED, description = "Titulares nuevos", body = Vec<TitularidadRespuesta>),
        (status = CONFLICT, description = "No tiene titulares vigentes (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Conjunto inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn transferir(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<TitularesPeticion>,
) -> ApiResult<(StatusCode, Json<Vec<TitularidadRespuesta>>)> {
    let soporte = p.soporte_archivo_id.map(ArchivoId::from_uuid);
    let creadas = tit::transferir_titularidad(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
        p.solicitudes()?,
        soporte,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(creadas.into_iter().map(Into::into).collect()),
    ))
}

/// Representante ante la copropiedad.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RepresentantePeticion {
    pub persona_id: Uuid,
}

/// Designa al representante del bien ante la copropiedad entre sus titulares vigentes.
#[utoipa::path(
    put,
    path = "/bienes-privados/{bien_privado_id}/representante",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("bien_privado_id" = Uuid, Path, description = "Bien privado"),
    ),
    request_body = RepresentantePeticion,
    responses(
        (status = NO_CONTENT, description = "Designado"),
        (status = NOT_FOUND, description = "La persona no es titular vigente", body = ErrorRespuesta),
    )
)]
pub async fn designar_representante(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<RepresentantePeticion>,
) -> ApiResult<StatusCode> {
    tit::designar_representante(
        &state.unidad,
        &en.ctx,
        BienPrivadoId::from_uuid(ruta.1),
        PersonaId::from_uuid(p.persona_id),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Bienes privados de los que la cuenta autenticada es titular vigente.
#[utoipa::path(
    get,
    path = "/mis-bienes-privados",
    tag = ETIQUETA_BIENES_PRIVADOS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Mis bienes privados", body = Vec<BienPrivadoRespuesta>),
        (status = NOT_FOUND, description = "La cuenta no tiene roles en la copropiedad", body = ErrorRespuesta),
    )
)]
pub async fn mis_bienes(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<BienPrivadoRespuesta>>> {
    let bienes = uc::mis_bienes_privados(&state.unidad, &en.ctx).await?;
    Ok(Json(bienes.into_iter().map(Into::into).collect()))
}
