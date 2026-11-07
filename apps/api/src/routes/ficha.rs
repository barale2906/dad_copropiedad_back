//! Ficha de la copropiedad: `GET/PATCH /api/v1/copropiedades/{copropiedad_id}`,
//! `/cargos-vigentes` y `/puntos-atencion`.

use application::{
    casos_uso::copropiedades::{self as uc, CambiosCopropiedad, FichaCompleta, NuevoPuntoAtencion},
    puertos::copropiedades::{CargoVigente, FichaCopropiedad, PuntoAtencion},
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc, Weekday};
use domain::{
    AdvertenciaFicha, ArchivoId, CodigoDane, DireccionCorreo, ExcepcionHorario, FormaCopropiedad,
    FranjaSemanal, Horario, Nit, PlantillaNomenclatura, PuntoAtencionId, Telefono, TipoCopropiedad,
    TipoPuntoAtencion,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path},
    openapi::ETIQUETA_COPROPIEDAD,
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de la ficha de la copropiedad.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(ver, actualizar))
        .routes(routes!(cargos_vigentes))
        .routes(routes!(listar_puntos, crear_punto))
        .routes(routes!(definir_horario))
}

/// Datos de la copropiedad.
#[derive(Debug, Serialize, ToSchema)]
pub struct DatosCopropiedadRespuesta {
    #[schema(example = "Conjunto Residencial Alameda")]
    pub nombre: String,
    pub razon_social: Option<String>,
    #[schema(example = "900123456-8")]
    pub nit: Option<String>,
    /// `residencial`, `comercial` o `mixta`.
    #[schema(value_type = String, example = "residencial")]
    pub tipo: TipoCopropiedad,
    /// `edificio`, `conjunto`, `condominio`, `centro_comercial`, `parcelacion` u `otro`.
    #[schema(value_type = String, example = "conjunto")]
    pub forma: FormaCopropiedad,
    pub direccion: Option<String>,
    #[schema(example = "11001")]
    pub municipio_dane: Option<String>,
    pub departamento: Option<String>,
    pub barrio: Option<String>,
    #[schema(value_type = Option<String>, example = "4.711000")]
    pub latitud: Option<Decimal>,
    #[schema(value_type = Option<String>, example = "-74.072100")]
    pub longitud: Option<Decimal>,
    pub estrato: Option<u8>,
    pub telefono: Option<String>,
    pub correo: Option<String>,
    pub sitio_web: Option<String>,
    pub escritura_numero: Option<String>,
    pub notaria: Option<String>,
    pub escritura_fecha: Option<NaiveDate>,
    pub matricula_matriz: Option<String>,
    pub reglamento_archivo_id: Option<Uuid>,
    pub certificado_archivo_id: Option<Uuid>,
    #[schema(example = "America/Bogota")]
    pub zona_horaria: String,
    /// `en_configuracion`, `activa`, `suspendida` o `retirada`.
    #[schema(example = "activa")]
    pub estado: String,
    /// Plantilla de los códigos de bienes privados (`{agrupacion}`, `{ruta}`, `{numero}`).
    #[schema(example = "{agrupacion}-{numero}")]
    pub plantilla_codigo: String,
}

impl From<FichaCopropiedad> for DatosCopropiedadRespuesta {
    fn from(f: FichaCopropiedad) -> Self {
        Self {
            nombre: f.nombre,
            razon_social: f.razon_social,
            nit: f.nit.map(|n| n.to_string()),
            tipo: f.tipo,
            forma: f.forma,
            direccion: f.direccion,
            municipio_dane: f.municipio_dane.map(|c| c.as_str().to_owned()),
            departamento: f.departamento,
            barrio: f.barrio,
            latitud: f.latitud,
            longitud: f.longitud,
            estrato: f.estrato,
            telefono: f.telefono.map(|t| t.as_str().to_owned()),
            correo: f.correo.map(|c| c.as_str().to_owned()),
            sitio_web: f.sitio_web,
            escritura_numero: f.escritura_numero,
            notaria: f.notaria,
            escritura_fecha: f.escritura_fecha,
            matricula_matriz: f.matricula_matriz,
            reglamento_archivo_id: f.reglamento_archivo_id.map(ArchivoId::as_uuid),
            certificado_archivo_id: f.certificado_archivo_id.map(ArchivoId::as_uuid),
            zona_horaria: f.zona_horaria,
            estado: f.estado.as_str().into(),
            plantilla_codigo: f.plantilla_codigo.as_str().into(),
        }
    }
}

/// Cargo vigente en la copropiedad.
#[derive(Debug, Serialize, ToSchema)]
pub struct CargoRespuesta {
    #[schema(example = "administrador")]
    pub rol: String,
    #[schema(example = "Administrador")]
    pub rol_nombre: String,
    pub cuenta_id: Uuid,
    pub nombre: String,
    /// Empresa por la que ejerce el cargo (contrato de servicio).
    pub organizacion: Option<String>,
    pub vigente_desde: DateTime<Utc>,
}

impl From<CargoVigente> for CargoRespuesta {
    fn from(c: CargoVigente) -> Self {
        Self {
            rol: c.rol,
            rol_nombre: c.rol_nombre,
            cuenta_id: c.cuenta_id.as_uuid(),
            nombre: c.nombre,
            organizacion: c.organizacion,
            vigente_desde: c.vigente_desde,
        }
    }
}

/// Ficha completa: datos, cargos y advertencias legales.
#[derive(Debug, Serialize, ToSchema)]
pub struct FichaRespuesta {
    pub datos: DatosCopropiedadRespuesta,
    pub cargos: Vec<CargoRespuesta>,
    /// `falta_revisor_fiscal` (art. 56), `falta_consejo_administracion` (art. 53) o
    /// `sin_cuadro_vigente`.
    #[schema(value_type = Vec<String>, example = json!(["sin_cuadro_vigente"]))]
    pub advertencias: Vec<AdvertenciaFicha>,
}

impl From<FichaCompleta> for FichaRespuesta {
    fn from(f: FichaCompleta) -> Self {
        Self {
            datos: f.datos.into(),
            cargos: f.cargos.into_iter().map(Into::into).collect(),
            advertencias: f.advertencias,
        }
    }
}

/// Ficha de la copropiedad (la ve cualquier rol con `copropiedad.datos.ver`, también los
/// propietarios).
#[utoipa::path(
    get,
    path = "",
    tag = ETIQUETA_COPROPIEDAD,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Ficha", body = FichaRespuesta),
        (status = FORBIDDEN, description = "Sin `copropiedad.datos.ver` (`sin_permiso`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "La cuenta no tiene roles en la copropiedad", body = ErrorRespuesta),
    )
)]
pub async fn ver(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<FichaRespuesta>> {
    Ok(Json(uc::ver_ficha(&state.unidad, &en.ctx).await?.into()))
}

/// Cambios a la ficha. Los campos omitidos no cambian; un texto vacío (`""`) lo borra.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct CambiosCopropiedadPeticion {
    pub nombre: Option<String>,
    pub razon_social: Option<String>,
    /// Con o sin dígito de verificación; se valida con el algoritmo de la DIAN.
    #[schema(value_type = Option<String>, example = "900123456-8")]
    pub nit: Option<Nit>,
    #[schema(value_type = Option<String>, example = "mixta")]
    pub tipo: Option<TipoCopropiedad>,
    #[schema(value_type = Option<String>, example = "edificio")]
    pub forma: Option<FormaCopropiedad>,
    pub direccion: Option<String>,
    #[schema(value_type = Option<String>, example = "11001")]
    pub municipio_dane: Option<CodigoDane>,
    pub departamento: Option<String>,
    pub barrio: Option<String>,
    #[schema(value_type = Option<String>, example = "4.711")]
    pub latitud: Option<Decimal>,
    #[schema(value_type = Option<String>, example = "-74.0721")]
    pub longitud: Option<Decimal>,
    pub estrato: Option<u8>,
    #[schema(value_type = Option<String>, example = "576015551234")]
    pub telefono: Option<Telefono>,
    #[schema(value_type = Option<String>, example = "administracion@alameda.co")]
    pub correo: Option<DireccionCorreo>,
    pub sitio_web: Option<String>,
    pub escritura_numero: Option<String>,
    pub notaria: Option<String>,
    pub escritura_fecha: Option<NaiveDate>,
    pub matricula_matriz: Option<String>,
    /// Archivo del reglamento de propiedad horizontal (subido antes en `/archivos`).
    pub reglamento_archivo_id: Option<Uuid>,
    /// Certificado de existencia y representación legal (alcaldía).
    pub certificado_archivo_id: Option<Uuid>,
    pub zona_horaria: Option<String>,
    /// Debe contener `{numero}`; admite `{agrupacion}` y `{ruta}`. Ej.: `{agrupacion}{numero}`
    /// da `3506`.
    #[schema(value_type = Option<String>, example = "{agrupacion}{numero}")]
    pub plantilla_codigo: Option<PlantillaNomenclatura>,
}

impl From<CambiosCopropiedadPeticion> for CambiosCopropiedad {
    fn from(p: CambiosCopropiedadPeticion) -> Self {
        Self {
            nombre: p.nombre,
            razon_social: p.razon_social,
            nit: p.nit,
            tipo: p.tipo,
            forma: p.forma,
            direccion: p.direccion,
            municipio_dane: p.municipio_dane,
            departamento: p.departamento,
            barrio: p.barrio,
            latitud: p.latitud,
            longitud: p.longitud,
            estrato: p.estrato,
            telefono: p.telefono,
            correo: p.correo,
            sitio_web: p.sitio_web,
            escritura_numero: p.escritura_numero,
            notaria: p.notaria,
            escritura_fecha: p.escritura_fecha,
            matricula_matriz: p.matricula_matriz,
            reglamento_archivo_id: p.reglamento_archivo_id.map(ArchivoId::from_uuid),
            certificado_archivo_id: p.certificado_archivo_id.map(ArchivoId::from_uuid),
            zona_horaria: p.zona_horaria,
            plantilla_codigo: p.plantilla_codigo,
        }
    }
}

/// Actualiza los datos legales, de ubicación y de contacto. Cambiar la plantilla no recalcula
/// los códigos ya asignados.
#[utoipa::path(
    patch,
    path = "",
    tag = ETIQUETA_COPROPIEDAD,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = CambiosCopropiedadPeticion,
    responses(
        (status = OK, description = "Datos actualizados", body = DatosCopropiedadRespuesta),
        (status = FORBIDDEN, description = "Sin `copropiedad.datos.editar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Otra copropiedad tiene ese NIT (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Dato inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn actualizar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<CambiosCopropiedadPeticion>,
) -> ApiResult<Json<DatosCopropiedadRespuesta>> {
    Ok(Json(
        uc::actualizar_datos(&state.unidad, &en.ctx, p.into())
            .await?
            .into(),
    ))
}

/// Administrador, contador, revisor fiscal, consejo y comité de convivencia vigentes.
#[utoipa::path(
    get,
    path = "/cargos-vigentes",
    tag = ETIQUETA_COPROPIEDAD,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Cargos vigentes", body = Vec<CargoRespuesta>),
        (status = FORBIDDEN, description = "Sin `copropiedad.datos.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn cargos_vigentes(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<CargoRespuesta>>> {
    let cargos = uc::cargos_vigentes(&state.unidad, &en.ctx).await?;
    Ok(Json(cargos.into_iter().map(Into::into).collect()))
}

/// Franja semanal de atención.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FranjaDto {
    /// Día ISO: 1 = lunes … 7 = domingo.
    #[schema(minimum = 1, maximum = 7, example = 1)]
    pub dia: u8,
    #[schema(value_type = String, example = "08:00:00")]
    pub inicio: NaiveTime,
    #[schema(value_type = String, example = "12:00:00")]
    pub fin: NaiveTime,
    /// ¿También atiende los festivos?
    #[serde(default)]
    pub aplica_festivos: bool,
}

/// Excepción de una fecha: sin `inicio`/`fin` = cerrado.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExcepcionDto {
    pub fecha: NaiveDate,
    #[schema(value_type = Option<String>)]
    pub inicio: Option<NaiveTime>,
    #[schema(value_type = Option<String>)]
    pub fin: Option<NaiveTime>,
    pub motivo: Option<String>,
}

/// Horario semanal con excepciones por fecha.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct HorarioDto {
    #[serde(default)]
    pub franjas: Vec<FranjaDto>,
    #[serde(default)]
    pub excepciones: Vec<ExcepcionDto>,
}

fn dia_semana(n: u8) -> Result<Weekday, ApiError> {
    n.checked_sub(1)
        .and_then(|d| Weekday::try_from(d).ok())
        .ok_or_else(|| ApiError::PeticionInvalida("`dia` va de 1 (lunes) a 7 (domingo)".into()))
}

impl HorarioDto {
    fn a_dominio(self) -> Result<Horario, ApiError> {
        let franjas = self
            .franjas
            .into_iter()
            .map(|f| {
                Ok(FranjaSemanal {
                    dia: dia_semana(f.dia)?,
                    inicio: f.inicio,
                    fin: f.fin,
                    aplica_festivos: f.aplica_festivos,
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?;
        let excepciones = self
            .excepciones
            .into_iter()
            .map(|e| ExcepcionHorario {
                fecha: e.fecha,
                franja: e.inicio.zip(e.fin),
                motivo: e.motivo,
            })
            .collect();
        Ok(Horario::new(franjas, excepciones)?)
    }

    fn desde(h: &Horario) -> Self {
        Self {
            franjas: h
                .franjas()
                .iter()
                .map(|f| FranjaDto {
                    dia: u8::try_from(f.dia.number_from_monday()).unwrap_or(1),
                    inicio: f.inicio,
                    fin: f.fin,
                    aplica_festivos: f.aplica_festivos,
                })
                .collect(),
            excepciones: h
                .excepciones()
                .iter()
                .map(|e| ExcepcionDto {
                    fecha: e.fecha,
                    inicio: e.franja.map(|(i, _)| i),
                    fin: e.franja.map(|(_, f)| f),
                    motivo: e.motivo.clone(),
                })
                .collect(),
        }
    }
}

/// Punto de atención.
#[derive(Debug, Serialize, ToSchema)]
pub struct PuntoAtencionRespuesta {
    pub id: Uuid,
    /// `oficina_administracion`, `porteria`, `recepcion` u `otro`.
    #[schema(value_type = String, example = "oficina_administracion")]
    pub tipo: TipoPuntoAtencion,
    pub nombre: String,
    pub ubicacion: Option<String>,
    pub telefono: Option<String>,
    pub horario: HorarioDto,
}

impl From<PuntoAtencion> for PuntoAtencionRespuesta {
    fn from(p: PuntoAtencion) -> Self {
        Self {
            id: p.id.as_uuid(),
            tipo: p.tipo,
            horario: HorarioDto::desde(&p.horario),
            nombre: p.nombre,
            ubicacion: p.ubicacion,
            telefono: p.telefono.map(|t| t.as_str().to_owned()),
        }
    }
}

/// Puntos de atención con su horario.
#[utoipa::path(
    get,
    path = "/puntos-atencion",
    tag = ETIQUETA_COPROPIEDAD,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Puntos de atención", body = Vec<PuntoAtencionRespuesta>),
        (status = FORBIDDEN, description = "Sin `copropiedad.datos.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_puntos(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<Vec<PuntoAtencionRespuesta>>> {
    let puntos = uc::listar_puntos_atencion(&state.unidad, &en.ctx).await?;
    Ok(Json(puntos.into_iter().map(Into::into).collect()))
}

/// Punto de atención nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoPuntoPeticion {
    #[schema(value_type = String, example = "oficina_administracion")]
    pub tipo: TipoPuntoAtencion,
    #[schema(example = "Oficina de administración")]
    pub nombre: String,
    pub ubicacion: Option<String>,
    #[schema(value_type = Option<String>)]
    pub telefono: Option<Telefono>,
    #[serde(default)]
    pub horario: HorarioDto,
}

/// Registra un punto de atención (oficina, portería, recepción) con su horario.
#[utoipa::path(
    post,
    path = "/puntos-atencion",
    tag = ETIQUETA_COPROPIEDAD,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoPuntoPeticion,
    responses(
        (status = CREATED, description = "Punto creado", body = PuntoAtencionRespuesta),
        (status = FORBIDDEN, description = "Sin `copropiedad.datos.editar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Horario inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_punto(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoPuntoPeticion>,
) -> ApiResult<(StatusCode, Json<PuntoAtencionRespuesta>)> {
    let punto = uc::registrar_punto_atencion(
        &state.unidad,
        &en.ctx,
        NuevoPuntoAtencion {
            tipo: p.tipo,
            nombre: p.nombre,
            ubicacion: p.ubicacion,
            telefono: p.telefono,
            horario: p.horario.a_dominio()?,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(punto.into())))
}

/// Reemplaza el horario de un punto de atención.
#[utoipa::path(
    put,
    path = "/puntos-atencion/{punto_id}/horario",
    tag = ETIQUETA_COPROPIEDAD,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("punto_id" = Uuid, Path, description = "Punto de atención"),
    ),
    request_body = HorarioDto,
    responses(
        (status = NO_CONTENT, description = "Horario reemplazado"),
        (status = NOT_FOUND, description = "No existe el punto", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Franjas solapadas o invertidas (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn definir_horario(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(h): Json<HorarioDto>,
) -> ApiResult<StatusCode> {
    uc::definir_horario(
        &state.unidad,
        &en.ctx,
        PuntoAtencionId::from_uuid(ruta.1),
        h.a_dominio()?,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
