//! Censo de la copropiedad (plan 07a): `/residencias`, `/vehiculos`, `/mascotas`,
//! `/cambios-censo`, `/libros/…`, `/censo/alertas` y `/personas`, dentro de
//! `/api/v1/copropiedades/{copropiedad_id}`.

use application::{
    casos_uso::censo::{
        self as uc, AlertasCenso, CambiosMascota, CambiosPersona, NuevaMascota, NuevaResidencia,
        NuevoVehiculo, PersonaVista, PropuestaCenso, ResidenciaConPersona,
    },
    puertos::{
        RelojSistema,
        censo::{CambioCenso, FilaLibroPropietarios, FilaLibroResidentes, Mascota, Vehiculo},
    },
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AlertaMascota, ArchivoId, BienPrivadoId, CalidadResidencia, CambioCensoId, CuentaId,
    DireccionCorreo, Documento, EspecieMascota, EstadoCambioCenso, EstadoMascota, MascotaId,
    PersonaId, Placa, ResidenciaId, Telefono, TipoCambioCenso, TipoDocumento, TipoPersona,
    TipoTitularidad, TipoVehiculo, VehiculoId,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    csv::csv,
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_CENSO,
    paginacion::{PaginaRespuesta, ParametrosPagina},
    routes::bienes_privados::{PersonaPeticion, ref_persona},
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas del censo.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_residencias, registrar_residencia))
        .routes(routes!(terminar_residencia))
        .routes(routes!(listar_vehiculos, registrar_vehiculo))
        .routes(routes!(retirar_vehiculo))
        .routes(routes!(listar_mascotas, registrar_mascota))
        .routes(routes!(actualizar_mascota))
        .routes(routes!(listar_cambios, proponer_cambio))
        .routes(routes!(aprobar_cambio))
        .routes(routes!(rechazar_cambio))
        .routes(routes!(libro_propietarios))
        .routes(routes!(libro_residentes))
        .routes(routes!(alertas))
        .routes(routes!(listar_personas))
        .routes(routes!(ver_persona, actualizar_persona))
}

// ---------------------------------------------------------------------------------------------
// Personas
// ---------------------------------------------------------------------------------------------

/// Persona (datos personales). La fecha de nacimiento y la movilidad reducida solo se ven con
/// `personas.datos_sensibles.ver`.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonaRespuesta {
    pub id: Uuid,
    #[schema(value_type = String, example = "natural")]
    pub tipo: TipoPersona,
    pub nombre: String,
    #[schema(value_type = Option<String>, example = "CC")]
    pub tipo_documento: Option<TipoDocumento>,
    pub documento: Option<String>,
    pub telefono: Option<String>,
    pub correo: Option<String>,
    pub cuenta_id: Option<Uuid>,
    pub fecha_nacimiento: Option<NaiveDate>,
    pub contacto_emergencia_nombre: Option<String>,
    pub contacto_emergencia_telefono: Option<String>,
    pub movilidad_reducida: bool,
    /// Menor de edad (si se conoce la fecha de nacimiento): datos con tratamiento especial.
    pub es_menor: Option<bool>,
    /// Se ocultaron los datos sensibles por falta de permiso.
    pub datos_sensibles_ocultos: bool,
}

impl From<PersonaVista> for PersonaRespuesta {
    fn from(v: PersonaVista) -> Self {
        let p = v.persona;
        Self {
            id: p.id.as_uuid(),
            tipo: p.tipo,
            nombre: p.nombre,
            tipo_documento: p.tipo_documento,
            documento: p.documento.map(|d| d.as_str().to_owned()),
            telefono: p.telefono.map(|t| t.as_str().to_owned()),
            correo: p.correo.map(|c| c.as_str().to_owned()),
            cuenta_id: p.cuenta_id.map(CuentaId::as_uuid),
            fecha_nacimiento: p.fecha_nacimiento,
            contacto_emergencia_nombre: p.contacto_emergencia_nombre,
            contacto_emergencia_telefono: p
                .contacto_emergencia_telefono
                .map(|t| t.as_str().to_owned()),
            movilidad_reducida: p.movilidad_reducida,
            es_menor: v.es_menor,
            datos_sensibles_ocultos: v.datos_sensibles_ocultos,
        }
    }
}

/// Búsqueda de personas.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroPersonas {
    /// Fragmento del nombre (sin tildes ni mayúsculas) o del documento.
    pub q: Option<String>,
}

/// Personas registradas en la copropiedad (paginado).
#[utoipa::path(
    get,
    path = "/personas",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroPersonas, ParametrosPagina),
    responses(
        (status = OK, description = "Personas", body = PaginaRespuesta<PersonaRespuesta>),
        (status = FORBIDDEN, description = "Sin `personas.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_personas(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroPersonas>,
    Query(p): Query<ParametrosPagina>,
) -> ApiResult<Json<PaginaRespuesta<PersonaRespuesta>>> {
    let pagina =
        uc::listar_personas(&state.unidad, &RelojSistema, &en.ctx, f.q, p.validar()?).await?;
    Ok(Json(pagina.mapear(Into::into).into()))
}

/// Una persona.
#[utoipa::path(
    get,
    path = "/personas/{persona_id}",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("persona_id" = Uuid, Path, description = "Persona"),
    ),
    responses(
        (status = OK, description = "Persona", body = PersonaRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver_persona(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<PersonaRespuesta>> {
    let p = uc::ver_persona(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PersonaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(p.into()))
}

/// Cambios a una persona (los omitidos no cambian).
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct CambiosPersonaPeticion {
    pub nombre: Option<String>,
    #[schema(value_type = Option<String>, example = "TI")]
    pub tipo_documento: Option<TipoDocumento>,
    #[schema(value_type = Option<String>)]
    pub documento: Option<Documento>,
    #[schema(value_type = Option<String>)]
    pub telefono: Option<Telefono>,
    #[schema(value_type = Option<String>)]
    pub correo: Option<DireccionCorreo>,
    pub contacto_emergencia_nombre: Option<String>,
    #[schema(value_type = Option<String>)]
    pub contacto_emergencia_telefono: Option<Telefono>,
    /// Dato sensible: exige `personas.datos_sensibles.ver`.
    pub fecha_nacimiento: Option<NaiveDate>,
    /// Dato sensible (salud): exige `personas.datos_sensibles.ver` y el consentimiento del
    /// titular.
    pub movilidad_reducida: Option<bool>,
}

/// Actualiza los datos de una persona.
#[utoipa::path(
    patch,
    path = "/personas/{persona_id}",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("persona_id" = Uuid, Path, description = "Persona"),
    ),
    request_body = CambiosPersonaPeticion,
    responses(
        (status = OK, description = "Persona actualizada", body = PersonaRespuesta),
        (status = FORBIDDEN, description = "Sin `personas.editar` o sin permiso de datos sensibles", body = ErrorRespuesta),
        (status = CONFLICT, description = "Documento repetido (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn actualizar_persona(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(c): Json<CambiosPersonaPeticion>,
) -> ApiResult<Json<PersonaRespuesta>> {
    let p = uc::actualizar_persona(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        PersonaId::from_uuid(ruta.1),
        CambiosPersona {
            nombre: c.nombre,
            tipo_documento: c.tipo_documento,
            documento: c.documento,
            telefono: c.telefono,
            correo: c.correo,
            contacto_emergencia_nombre: c.contacto_emergencia_nombre,
            contacto_emergencia_telefono: c.contacto_emergencia_telefono,
            fecha_nacimiento: c.fecha_nacimiento,
            movilidad_reducida: c.movilidad_reducida,
        },
    )
    .await?;
    Ok(Json(p.into()))
}

// ---------------------------------------------------------------------------------------------
// Residencias
// ---------------------------------------------------------------------------------------------

/// Datos de una residencia (sin el bien privado).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ResidenciaDatos {
    /// Persona registrada…
    pub persona_id: Option<Uuid>,
    /// …o persona nueva.
    pub persona: Option<PersonaPeticion>,
    /// `propietario_residente`, `tenedor`, `familiar`, `huesped_temporal` o
    /// `empleado_domestico`.
    #[schema(value_type = String, example = "tenedor")]
    pub calidad: CalidadResidencia,
    /// Cabeza del bien ante la copropiedad (a lo sumo uno vigente).
    #[serde(default)]
    pub es_responsable: bool,
    /// Titular vigente que autoriza al tenedor (obligatorio para tenedores).
    pub autorizado_por: Option<Uuid>,
    /// Contrato de arrendamiento (archivo ya subido).
    pub contrato_archivo_id: Option<Uuid>,
}

impl ResidenciaDatos {
    fn a_dominio(self) -> Result<NuevaResidencia, ApiError> {
        Ok(NuevaResidencia {
            persona: ref_persona(self.persona_id, self.persona)?,
            calidad: self.calidad,
            es_responsable: self.es_responsable,
            autorizado_por: self.autorizado_por.map(PersonaId::from_uuid),
            contrato_archivo_id: self.contrato_archivo_id.map(ArchivoId::from_uuid),
        })
    }
}

/// Residencia nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaResidenciaPeticion {
    pub bien_privado_id: Uuid,
    #[serde(flatten)]
    pub datos: ResidenciaDatos,
}

/// Residencia con su persona.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResidenciaRespuesta {
    pub id: Uuid,
    pub bien_privado_id: Uuid,
    pub persona: PersonaRespuesta,
    #[schema(value_type = String, example = "tenedor")]
    pub calidad: CalidadResidencia,
    pub es_responsable: bool,
    pub autorizado_por: Option<Uuid>,
    pub contrato_archivo_id: Option<Uuid>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
}

impl From<ResidenciaConPersona> for ResidenciaRespuesta {
    fn from(r: ResidenciaConPersona) -> Self {
        let x = r.residencia;
        Self {
            id: x.id.as_uuid(),
            bien_privado_id: x.bien_privado_id.as_uuid(),
            persona: r.persona.into(),
            calidad: x.calidad,
            es_responsable: x.es_responsable,
            autorizado_por: x.autorizado_por.map(PersonaId::as_uuid),
            contrato_archivo_id: x.contrato_archivo_id.map(ArchivoId::as_uuid),
            vigente_desde: x.vigente_desde,
            vigente_hasta: x.vigente_hasta,
        }
    }
}

/// Filtro de un listado del censo.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroCenso {
    pub bien_privado_id: Option<Uuid>,
    /// Incluir los terminados, retirados o inactivos (historial).
    #[serde(default)]
    pub historial: bool,
}

/// Residentes (de un bien o de todos).
#[utoipa::path(
    get,
    path = "/residencias",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroCenso),
    responses(
        (status = OK, description = "Residencias", body = Vec<ResidenciaRespuesta>),
        (status = FORBIDDEN, description = "Sin `censo.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_residencias(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroCenso>,
) -> ApiResult<Json<Vec<ResidenciaRespuesta>>> {
    let lista = uc::listar_residencias(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        f.bien_privado_id.map(BienPrivadoId::from_uuid),
        f.historial,
    )
    .await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Registra que una persona reside en un bien privado. Tenedores, familiares y propietarios
/// residentes con cuenta reciben el rol `residente`.
#[utoipa::path(
    post,
    path = "/residencias",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevaResidenciaPeticion,
    responses(
        (status = CREATED, description = "Residencia registrada", body = ResidenciaRespuesta),
        (status = FORBIDDEN, description = "Sin `censo.editar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya reside allí o ya hay responsable (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Propietario no titular o tenedor sin autorización (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_residencia(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevaResidenciaPeticion>,
) -> ApiResult<(StatusCode, Json<ResidenciaRespuesta>)> {
    let r = uc::registrar_residencia(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(p.bien_privado_id),
        p.datos.a_dominio()?,
    )
    .await?;
    let persona = uc::ver_persona(&state.unidad, &RelojSistema, &en.ctx, r.persona_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(
            ResidenciaConPersona {
                residencia: r,
                persona,
            }
            .into(),
        ),
    ))
}

/// Termina una residencia (vence su rol `residente` en la misma transacción).
#[utoipa::path(
    post,
    path = "/residencias/{residencia_id}/terminar",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("residencia_id" = Uuid, Path, description = "Residencia"),
    ),
    responses(
        (status = NO_CONTENT, description = "Terminada"),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn terminar_residencia(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    uc::terminar_residencia(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ResidenciaId::from_uuid(ruta.1),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Vehículos
// ---------------------------------------------------------------------------------------------

/// Datos de un vehículo (sin el bien privado).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct VehiculoDatos {
    #[schema(value_type = String, example = "carro")]
    pub tipo: TipoVehiculo,
    /// Obligatoria para carros y motos; se normaliza (`abc-123` → `ABC123`).
    #[schema(value_type = Option<String>, example = "ABC123")]
    pub placa: Option<Placa>,
    pub marca: Option<String>,
    pub color: Option<String>,
    pub persona_id: Option<Uuid>,
    pub tarjeta_archivo_id: Option<Uuid>,
}

impl From<VehiculoDatos> for NuevoVehiculo {
    fn from(v: VehiculoDatos) -> Self {
        Self {
            tipo: v.tipo,
            placa: v.placa,
            marca: v.marca,
            color: v.color,
            persona_id: v.persona_id.map(PersonaId::from_uuid),
            tarjeta_archivo_id: v.tarjeta_archivo_id.map(ArchivoId::from_uuid),
        }
    }
}

/// Vehículo nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoVehiculoPeticion {
    pub bien_privado_id: Uuid,
    #[serde(flatten)]
    pub datos: VehiculoDatos,
}

/// Vehículo.
#[derive(Debug, Serialize, ToSchema)]
pub struct VehiculoRespuesta {
    pub id: Uuid,
    pub bien_privado_id: Uuid,
    pub persona_id: Option<Uuid>,
    #[schema(value_type = String, example = "carro")]
    pub tipo: TipoVehiculo,
    pub placa: Option<String>,
    pub marca: Option<String>,
    pub color: Option<String>,
    pub tarjeta_archivo_id: Option<Uuid>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
}

impl From<Vehiculo> for VehiculoRespuesta {
    fn from(v: Vehiculo) -> Self {
        Self {
            id: v.id.as_uuid(),
            bien_privado_id: v.bien_privado_id.as_uuid(),
            persona_id: v.persona_id.map(PersonaId::as_uuid),
            tipo: v.tipo,
            placa: v.placa.map(|p| p.as_str().to_owned()),
            marca: v.marca,
            color: v.color,
            tarjeta_archivo_id: v.tarjeta_archivo_id.map(ArchivoId::as_uuid),
            vigente_desde: v.vigente_desde,
            vigente_hasta: v.vigente_hasta,
        }
    }
}

/// Filtro de vehículos.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroVehiculos {
    pub bien_privado_id: Option<Uuid>,
    /// Placa exacta (se normaliza).
    pub placa: Option<String>,
    #[serde(default)]
    pub historial: bool,
}

/// Vehículos (de un bien, por placa o todos).
#[utoipa::path(
    get,
    path = "/vehiculos",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroVehiculos),
    responses(
        (status = OK, description = "Vehículos", body = Vec<VehiculoRespuesta>),
        (status = FORBIDDEN, description = "Sin `censo.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_vehiculos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroVehiculos>,
) -> ApiResult<Json<Vec<VehiculoRespuesta>>> {
    let placa = f.placa.as_deref().map(Placa::new).transpose()?;
    let lista = uc::listar_vehiculos(
        &state.unidad,
        &en.ctx,
        f.bien_privado_id.map(BienPrivadoId::from_uuid),
        placa,
        f.historial,
    )
    .await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Registra un vehículo de un bien privado.
#[utoipa::path(
    post,
    path = "/vehiculos",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoVehiculoPeticion,
    responses(
        (status = CREATED, description = "Vehículo registrado", body = VehiculoRespuesta),
        (status = CONFLICT, description = "Placa ya registrada (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Carro o moto sin placa (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_vehiculo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoVehiculoPeticion>,
) -> ApiResult<(StatusCode, Json<VehiculoRespuesta>)> {
    let v = uc::registrar_vehiculo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(p.bien_privado_id),
        p.datos.into(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(v.into())))
}

/// Retira un vehículo (su placa queda libre).
#[utoipa::path(
    post,
    path = "/vehiculos/{vehiculo_id}/retirar",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("vehiculo_id" = Uuid, Path, description = "Vehículo"),
    ),
    responses(
        (status = NO_CONTENT, description = "Retirado"),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn retirar_vehiculo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    uc::retirar_vehiculo(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        VehiculoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// Mascotas
// ---------------------------------------------------------------------------------------------

/// Datos de una mascota (sin el bien privado).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct MascotaDatos {
    #[schema(value_type = String, example = "perro")]
    pub especie: EspecieMascota,
    pub nombre: String,
    pub raza: Option<String>,
    /// Raza potencialmente peligrosa (Ley 1801): exige póliza.
    #[serde(default)]
    pub potencialmente_peligrosa: bool,
    pub poliza_numero: Option<String>,
    pub poliza_vence: Option<NaiveDate>,
    pub poliza_archivo_id: Option<Uuid>,
    /// Vencimiento de la vacunación vigente.
    pub vacunas_vencen: Option<NaiveDate>,
    pub carne_archivo_id: Option<Uuid>,
    #[serde(default)]
    pub esterilizada: bool,
    pub foto_archivo_id: Option<Uuid>,
    pub responsable_id: Option<Uuid>,
}

impl From<MascotaDatos> for NuevaMascota {
    fn from(m: MascotaDatos) -> Self {
        Self {
            especie: m.especie,
            nombre: m.nombre,
            raza: m.raza,
            potencialmente_peligrosa: m.potencialmente_peligrosa,
            poliza_numero: m.poliza_numero,
            poliza_vence: m.poliza_vence,
            poliza_archivo_id: m.poliza_archivo_id.map(ArchivoId::from_uuid),
            vacunas_vencen: m.vacunas_vencen,
            carne_archivo_id: m.carne_archivo_id.map(ArchivoId::from_uuid),
            esterilizada: m.esterilizada,
            foto_archivo_id: m.foto_archivo_id.map(ArchivoId::from_uuid),
            responsable_id: m.responsable_id.map(PersonaId::from_uuid),
        }
    }
}

/// Mascota nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaMascotaPeticion {
    pub bien_privado_id: Uuid,
    #[serde(flatten)]
    pub datos: MascotaDatos,
}

/// Mascota.
#[derive(Debug, Serialize, ToSchema)]
pub struct MascotaRespuesta {
    pub id: Uuid,
    pub bien_privado_id: Uuid,
    pub responsable_id: Option<Uuid>,
    #[schema(value_type = String, example = "perro")]
    pub especie: EspecieMascota,
    pub nombre: String,
    pub raza: Option<String>,
    pub potencialmente_peligrosa: bool,
    pub poliza_numero: Option<String>,
    pub poliza_vence: Option<NaiveDate>,
    pub poliza_archivo_id: Option<Uuid>,
    pub vacunas_vencen: Option<NaiveDate>,
    pub carne_archivo_id: Option<Uuid>,
    pub esterilizada: bool,
    pub foto_archivo_id: Option<Uuid>,
    /// `activa`, `fallecida` o `trasladada`.
    #[schema(value_type = String, example = "activa")]
    pub estado: EstadoMascota,
}

impl From<Mascota> for MascotaRespuesta {
    fn from(m: Mascota) -> Self {
        Self {
            id: m.id.as_uuid(),
            bien_privado_id: m.bien_privado_id.as_uuid(),
            responsable_id: m.responsable_id.map(PersonaId::as_uuid),
            especie: m.especie,
            nombre: m.nombre,
            raza: m.raza,
            potencialmente_peligrosa: m.potencialmente_peligrosa,
            poliza_numero: m.poliza_numero,
            poliza_vence: m.poliza_vence,
            poliza_archivo_id: m.poliza_archivo_id.map(ArchivoId::as_uuid),
            vacunas_vencen: m.vacunas_vencen,
            carne_archivo_id: m.carne_archivo_id.map(ArchivoId::as_uuid),
            esterilizada: m.esterilizada,
            foto_archivo_id: m.foto_archivo_id.map(ArchivoId::as_uuid),
            estado: m.estado,
        }
    }
}

/// Mascotas (activas, o todas con `historial=true`).
#[utoipa::path(
    get,
    path = "/mascotas",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroCenso),
    responses(
        (status = OK, description = "Mascotas", body = Vec<MascotaRespuesta>),
        (status = FORBIDDEN, description = "Sin `censo.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_mascotas(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroCenso>,
) -> ApiResult<Json<Vec<MascotaRespuesta>>> {
    let lista = uc::listar_mascotas(
        &state.unidad,
        &en.ctx,
        f.bien_privado_id.map(BienPrivadoId::from_uuid),
        !f.historial,
    )
    .await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Registra una mascota de un bien privado.
#[utoipa::path(
    post,
    path = "/mascotas",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevaMascotaPeticion,
    responses(
        (status = CREATED, description = "Mascota registrada", body = MascotaRespuesta),
        (status = FORBIDDEN, description = "Sin `censo.editar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_mascota(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevaMascotaPeticion>,
) -> ApiResult<(StatusCode, Json<MascotaRespuesta>)> {
    let m = uc::registrar_mascota(
        &state.unidad,
        &en.ctx,
        BienPrivadoId::from_uuid(p.bien_privado_id),
        p.datos.into(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(m.into())))
}

/// Cambios a una mascota (los omitidos no cambian).
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct CambiosMascotaPeticion {
    pub raza: Option<String>,
    pub potencialmente_peligrosa: Option<bool>,
    pub poliza_numero: Option<String>,
    pub poliza_vence: Option<NaiveDate>,
    pub poliza_archivo_id: Option<Uuid>,
    pub vacunas_vencen: Option<NaiveDate>,
    pub carne_archivo_id: Option<Uuid>,
    pub esterilizada: Option<bool>,
    pub foto_archivo_id: Option<Uuid>,
    pub responsable_id: Option<Uuid>,
    /// `fallecida` o `trasladada` (desde `activa`).
    #[schema(value_type = Option<String>)]
    pub estado: Option<EstadoMascota>,
}

/// Actualiza una mascota (vacunas, póliza, estado…).
#[utoipa::path(
    patch,
    path = "/mascotas/{mascota_id}",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("mascota_id" = Uuid, Path, description = "Mascota"),
    ),
    request_body = CambiosMascotaPeticion,
    responses(
        (status = OK, description = "Mascota actualizada", body = MascotaRespuesta),
        (status = CONFLICT, description = "Cambio de estado no permitido (`conflicto_estado`)", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn actualizar_mascota(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(c): Json<CambiosMascotaPeticion>,
) -> ApiResult<Json<MascotaRespuesta>> {
    let m = uc::actualizar_mascota(
        &state.unidad,
        &en.ctx,
        MascotaId::from_uuid(ruta.1),
        CambiosMascota {
            raza: c.raza,
            potencialmente_peligrosa: c.potencialmente_peligrosa,
            poliza_numero: c.poliza_numero,
            poliza_vence: c.poliza_vence,
            poliza_archivo_id: c.poliza_archivo_id.map(ArchivoId::from_uuid),
            vacunas_vencen: c.vacunas_vencen,
            carne_archivo_id: c.carne_archivo_id.map(ArchivoId::from_uuid),
            esterilizada: c.esterilizada,
            foto_archivo_id: c.foto_archivo_id.map(ArchivoId::from_uuid),
            responsable_id: c.responsable_id.map(PersonaId::from_uuid),
            estado: c.estado,
        },
    )
    .await?;
    Ok(Json(m.into()))
}

// ---------------------------------------------------------------------------------------------
// Autoservicio
// ---------------------------------------------------------------------------------------------

/// Propuesta de cambio: `tipo` y los `datos` del registro (los mismos campos que en
/// `POST /residencias`, `/vehiculos` o `/mascotas`, sin `bien_privado_id`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct PropuestaPeticion {
    pub bien_privado_id: Uuid,
    #[schema(value_type = String, example = "vehiculo")]
    pub tipo: TipoCambioCenso,
    #[schema(value_type = Object)]
    pub datos: serde_json::Value,
}

impl PropuestaPeticion {
    fn a_dominio(self) -> Result<PropuestaCenso, ApiError> {
        let invalida = |e: serde_json::Error| ApiError::PeticionInvalida(format!("`datos`: {e}"));
        Ok(match self.tipo {
            TipoCambioCenso::Residencia => PropuestaCenso::Residencia(
                serde_json::from_value::<ResidenciaDatos>(self.datos)
                    .map_err(invalida)?
                    .a_dominio()?,
            ),
            TipoCambioCenso::Vehiculo => PropuestaCenso::Vehiculo(
                serde_json::from_value::<VehiculoDatos>(self.datos)
                    .map_err(invalida)?
                    .into(),
            ),
            TipoCambioCenso::Mascota => PropuestaCenso::Mascota(
                serde_json::from_value::<MascotaDatos>(self.datos)
                    .map_err(invalida)?
                    .into(),
            ),
        })
    }
}

/// Propuesta de cambio del censo.
#[derive(Debug, Serialize, ToSchema)]
pub struct CambioRespuesta {
    pub id: Uuid,
    pub bien_privado_id: Uuid,
    #[schema(value_type = String, example = "vehiculo")]
    pub tipo: TipoCambioCenso,
    #[schema(value_type = Object)]
    pub datos: serde_json::Value,
    /// `pendiente`, `aprobado` o `rechazado`.
    #[schema(value_type = String, example = "pendiente")]
    pub estado: EstadoCambioCenso,
    pub propuesto_por: Uuid,
    pub propuesto_en: DateTime<Utc>,
    pub revisado_por: Option<Uuid>,
    pub revisado_en: Option<DateTime<Utc>>,
    pub motivo: Option<String>,
    /// Residencia, vehículo o mascota creados al aprobar.
    pub resultado_id: Option<Uuid>,
}

impl From<CambioCenso> for CambioRespuesta {
    fn from(c: CambioCenso) -> Self {
        Self {
            id: c.id.as_uuid(),
            bien_privado_id: c.bien_privado_id.as_uuid(),
            tipo: c.tipo,
            datos: c.datos,
            estado: c.estado,
            propuesto_por: c.propuesto_por.as_uuid(),
            propuesto_en: c.propuesto_en,
            revisado_por: c.revisado_por.map(CuentaId::as_uuid),
            revisado_en: c.revisado_en,
            motivo: c.motivo,
            resultado_id: c.resultado_id,
        }
    }
}

/// Filtro de propuestas.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroCambios {
    #[param(value_type = Option<String>)]
    pub estado: Option<EstadoCambioCenso>,
    /// Solo las propias (no exige `censo.aprobar`).
    #[serde(default)]
    pub mias: bool,
}

/// Propuestas de cambio: todas (con `censo.aprobar`) o las propias (`mias=true`).
#[utoipa::path(
    get,
    path = "/cambios-censo",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroCambios),
    responses(
        (status = OK, description = "Propuestas", body = Vec<CambioRespuesta>),
        (status = FORBIDDEN, description = "Sin `censo.aprobar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_cambios(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroCambios>,
) -> ApiResult<Json<Vec<CambioRespuesta>>> {
    let lista = uc::listar_cambios(&state.unidad, &en.ctx, f.estado, f.mias).await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Un residente o propietario propone un cambio del censo de su bien privado (queda pendiente
/// de aprobación).
#[utoipa::path(
    post,
    path = "/cambios-censo",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = PropuestaPeticion,
    responses(
        (status = CREATED, description = "Propuesta registrada", body = CambioRespuesta),
        (status = FORBIDDEN, description = "No es titular ni residente del bien (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn proponer_cambio(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<PropuestaPeticion>,
) -> ApiResult<(StatusCode, Json<CambioRespuesta>)> {
    let bien = BienPrivadoId::from_uuid(p.bien_privado_id);
    let c =
        uc::proponer_cambio(&state.unidad, &RelojSistema, &en.ctx, bien, p.a_dominio()?).await?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Aprueba una propuesta: crea el registro en la misma transacción.
#[utoipa::path(
    post,
    path = "/cambios-censo/{cambio_id}/aprobar",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("cambio_id" = Uuid, Path, description = "Propuesta"),
    ),
    responses(
        (status = OK, description = "Aprobada", body = CambioRespuesta),
        (status = CONFLICT, description = "Ya resuelta (`conflicto_estado`) o choca con el censo (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn aprobar_cambio(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<CambioRespuesta>> {
    let c = uc::aprobar_cambio(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CambioCensoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(c.into()))
}

/// Motivo del rechazo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RechazoPeticion {
    #[schema(example = "La placa no corresponde a la tarjeta de propiedad")]
    pub motivo: String,
}

/// Rechaza una propuesta con un motivo.
#[utoipa::path(
    post,
    path = "/cambios-censo/{cambio_id}/rechazar",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("cambio_id" = Uuid, Path, description = "Propuesta"),
    ),
    request_body = RechazoPeticion,
    responses(
        (status = OK, description = "Rechazada", body = CambioRespuesta),
        (status = CONFLICT, description = "Ya resuelta (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn rechazar_cambio(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<RechazoPeticion>,
) -> ApiResult<Json<CambioRespuesta>> {
    let c = uc::rechazar_cambio(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        CambioCensoId::from_uuid(ruta.1),
        &p.motivo,
    )
    .await?;
    Ok(Json(c.into()))
}

// ---------------------------------------------------------------------------------------------
// Libros y alertas
// ---------------------------------------------------------------------------------------------

/// Fecha de corte y formato de un libro.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ParametrosLibro {
    /// Fecha y hora de corte (por defecto, ahora).
    pub corte: Option<DateTime<Utc>>,
    /// `json` (por defecto) o `csv` (separado por `;`, para Excel).
    pub formato: Option<String>,
}

impl ParametrosLibro {
    fn es_csv(&self) -> Result<bool, ApiError> {
        match self.formato.as_deref() {
            None | Some("json") => Ok(false),
            Some("csv") => Ok(true),
            Some(_) => Err(ApiError::PeticionInvalida(
                "`formato` es `json` o `csv`".into(),
            )),
        }
    }
}

/// Fila del libro de propietarios.
#[derive(Debug, Serialize, ToSchema)]
pub struct FilaPropietarioRespuesta {
    #[schema(example = "T3-506")]
    pub codigo: String,
    #[schema(value_type = Option<String>)]
    pub coeficiente: Option<Decimal>,
    pub nombre: String,
    pub tipo_documento: Option<String>,
    pub documento: Option<String>,
    #[schema(value_type = String, example = "propietario")]
    pub tipo: TipoTitularidad,
    #[schema(value_type = String)]
    pub porcentaje: Decimal,
    pub es_representante: bool,
    pub vigente_desde: DateTime<Utc>,
}

impl From<FilaLibroPropietarios> for FilaPropietarioRespuesta {
    fn from(f: FilaLibroPropietarios) -> Self {
        Self {
            codigo: f.codigo.as_str().to_owned(),
            coeficiente: f.coeficiente,
            nombre: f.nombre,
            tipo_documento: f.tipo_documento.map(|t| t.as_str().to_owned()),
            documento: f.documento.map(|d| d.as_str().to_owned()),
            tipo: f.tipo,
            porcentaje: f.porcentaje.valor(),
            es_representante: f.es_representante,
            vigente_desde: f.vigente_desde,
        }
    }
}

/// Fila del libro de residentes.
#[derive(Debug, Serialize, ToSchema)]
pub struct FilaResidenteRespuesta {
    pub codigo: String,
    pub nombre: String,
    pub tipo_documento: Option<String>,
    pub documento: Option<String>,
    #[schema(value_type = String, example = "tenedor")]
    pub calidad: CalidadResidencia,
    pub es_responsable: bool,
    pub vigente_desde: DateTime<Utc>,
}

impl From<FilaLibroResidentes> for FilaResidenteRespuesta {
    fn from(f: FilaLibroResidentes) -> Self {
        Self {
            codigo: f.codigo.as_str().to_owned(),
            nombre: f.nombre,
            tipo_documento: f.tipo_documento.map(|t| t.as_str().to_owned()),
            documento: f.documento.map(|d| d.as_str().to_owned()),
            calidad: f.calidad,
            es_responsable: f.es_responsable,
            vigente_desde: f.vigente_desde,
        }
    }
}

fn opt(v: Option<String>) -> String {
    v.unwrap_or_default()
}

/// Libro de propietarios a una fecha de corte (Ley 675, art. 51), en JSON o CSV.
#[utoipa::path(
    get,
    path = "/libros/propietarios",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ParametrosLibro),
    responses(
        (status = OK, description = "Libro de propietarios (JSON o `text/csv`)", body = Vec<FilaPropietarioRespuesta>),
        (status = FORBIDDEN, description = "Sin `censo.exportar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn libro_propietarios(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(p): Query<ParametrosLibro>,
) -> ApiResult<Response> {
    let es_csv = p.es_csv()?;
    let filas: Vec<FilaPropietarioRespuesta> =
        uc::libro_propietarios(&state.unidad, &en.ctx, p.corte.unwrap_or_else(Utc::now))
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    if !es_csv {
        return Ok(Json(filas).into_response());
    }
    Ok(csv(
        "attachment; filename=\"libro_propietarios.csv\"",
        &[
            "Bien privado",
            "Coeficiente",
            "Titular",
            "Tipo de documento",
            "Documento",
            "Calidad",
            "Porcentaje",
            "Representante",
            "Desde",
        ],
        filas
            .into_iter()
            .map(|f| {
                vec![
                    f.codigo,
                    f.coeficiente.map(|c| c.to_string()).unwrap_or_default(),
                    f.nombre,
                    opt(f.tipo_documento),
                    opt(f.documento),
                    f.tipo.as_str().to_owned(),
                    f.porcentaje.to_string(),
                    if f.es_representante { "sí" } else { "no" }.to_owned(),
                    f.vigente_desde.date_naive().to_string(),
                ]
            })
            .collect(),
    ))
}

/// Libro de residentes a una fecha de corte (Ley 675, art. 51), en JSON o CSV.
#[utoipa::path(
    get,
    path = "/libros/residentes",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ParametrosLibro),
    responses(
        (status = OK, description = "Libro de residentes (JSON o `text/csv`)", body = Vec<FilaResidenteRespuesta>),
        (status = FORBIDDEN, description = "Sin `censo.exportar` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn libro_residentes(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(p): Query<ParametrosLibro>,
) -> ApiResult<Response> {
    let es_csv = p.es_csv()?;
    let filas: Vec<FilaResidenteRespuesta> =
        uc::libro_residentes(&state.unidad, &en.ctx, p.corte.unwrap_or_else(Utc::now))
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    if !es_csv {
        return Ok(Json(filas).into_response());
    }
    Ok(csv(
        "attachment; filename=\"libro_residentes.csv\"",
        &[
            "Bien privado",
            "Residente",
            "Tipo de documento",
            "Documento",
            "Calidad",
            "Responsable",
            "Desde",
        ],
        filas
            .into_iter()
            .map(|f| {
                vec![
                    f.codigo,
                    f.nombre,
                    opt(f.tipo_documento),
                    opt(f.documento),
                    f.calidad.as_str().to_owned(),
                    if f.es_responsable { "sí" } else { "no" }.to_owned(),
                    f.vigente_desde.date_naive().to_string(),
                ]
            })
            .collect(),
    ))
}

/// Alertas de una mascota.
#[derive(Debug, Serialize, ToSchema)]
pub struct AlertaMascotaRespuesta {
    pub mascota: MascotaRespuesta,
    /// `vacunas_vencidas`, `vacunas_por_vencer`, `sin_registro_de_vacunas`, `sin_poliza`,
    /// `poliza_vencida` o `poliza_por_vencer`.
    #[schema(value_type = Vec<String>)]
    pub alertas: Vec<AlertaMascota>,
}

/// Alertas del censo.
#[derive(Debug, Serialize, ToSchema)]
pub struct AlertasCensoRespuesta {
    pub mascotas: Vec<AlertaMascotaRespuesta>,
    /// Residentes sin autorización de tratamiento de datos para administración (Ley 1581).
    pub residentes_sin_autorizacion: Vec<Uuid>,
}

impl From<AlertasCenso> for AlertasCensoRespuesta {
    fn from(a: AlertasCenso) -> Self {
        Self {
            mascotas: a
                .mascotas
                .into_iter()
                .map(|(m, alertas)| AlertaMascotaRespuesta {
                    mascota: m.into(),
                    alertas,
                })
                .collect(),
            residentes_sin_autorizacion: a
                .residentes_sin_autorizacion
                .into_iter()
                .map(PersonaId::as_uuid)
                .collect(),
        }
    }
}

/// Vacunas y pólizas vencidas o por vencer (30 días) y residentes sin autorización de datos.
#[utoipa::path(
    get,
    path = "/censo/alertas",
    tag = ETIQUETA_CENSO,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    responses(
        (status = OK, description = "Alertas", body = AlertasCensoRespuesta),
        (status = FORBIDDEN, description = "Sin `censo.ver` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn alertas(
    State(state): State<AppState>,
    en: EnCopropiedad,
) -> ApiResult<Json<AlertasCensoRespuesta>> {
    Ok(Json(
        uc::alertas_censo(&state.unidad, &RelojSistema, &en.ctx)
            .await?
            .into(),
    ))
}
