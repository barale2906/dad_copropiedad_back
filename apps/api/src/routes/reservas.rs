//! Bienes comunes y reservas (plan 04): `/recursos`, `/reservas`, `/restricciones-reserva` y
//! la autorización de tenedores, dentro de `/api/v1/copropiedades/{copropiedad_id}`.
//!
//! Las citas con la administración son reservas de un recurso `agenda_administracion` con
//! `modalidad` y `motivo`.

use application::{
    casos_uso::reservas::{
        self as uc, CambiosRecurso, DatosEntrega, NuevoRecurso, SolicitudReserva,
    },
    puertos::{
        RelojSistema,
        reservas::{
            DetalleBienComun, DetalleParqueadero, ElementoChecklist, Entrega, FiltroReservas,
            Recurso, Reserva, Restriccion,
        },
    },
};
use axum::{extract::State, http::StatusCode};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, BienPrivadoId, CanalReserva, CategoriaBienComun, CobroPor, EstadoPago,
    EstadoRecurso, EstadoReserva, Franja, MetodoEntrega, ModalidadCita, ModoAsignacion,
    NaturalezaParqueadero, Placa, PoliticaReserva, QuienReserva, RecursoId, ReservaId,
    ResidenciaId, RestriccionId, TipoEntrega, TipoParqueadero, TipoRecurso,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Path, Query},
    openapi::ETIQUETA_RESERVAS,
    routes::ficha::HorarioDto,
    sesion::EnCopropiedad,
    state::AppState,
};

/// Rutas de bienes comunes y reservas.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar_recursos, crear_recurso))
        .routes(routes!(ver_recurso, editar_recurso))
        .routes(routes!(cambiar_estado))
        .routes(routes!(disponibilidad))
        .routes(routes!(listar_reservas, solicitar))
        .routes(routes!(ver_reserva))
        .routes(routes!(aprobar))
        .routes(routes!(rechazar))
        .routes(routes!(cancelar))
        .routes(routes!(entrega))
        .routes(routes!(recibo))
        .routes(routes!(no_asistio))
        .routes(routes!(pago))
        .routes(routes!(listar_restricciones, restringir))
        .routes(routes!(levantar))
        .routes(routes!(autorizar_tenedor))
}

// ---------------------------------------------------------------------------------------------
// Política y recursos
// ---------------------------------------------------------------------------------------------

/// Política de reserva de un recurso. Montos en pesos (texto decimal).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PoliticaDto {
    /// `libre`, `franja_inmediata`, `franja_con_aprobacion`, `sorteo` o `turno_rotativo`.
    #[schema(value_type = String, example = "franja_con_aprobacion")]
    pub modo: ModoAsignacion,
    #[schema(example = 60)]
    pub duracion_minima_min: u32,
    #[schema(example = 240)]
    pub duracion_maxima_min: u32,
    #[schema(example = 60)]
    pub granularidad_min: u32,
    pub anticipacion_minima_horas: u32,
    #[schema(example = 60)]
    pub anticipacion_maxima_dias: u32,
    pub max_reservas_mes: Option<u32>,
    pub max_simultaneas: Option<u32>,
    pub alistamiento_min: u32,
    /// `propietario`, `residente` o `ambos`.
    #[schema(value_type = String, example = "ambos")]
    pub quien: QuienReserva,
    /// Los tenedores necesitan la autorización del propietario.
    pub tenedor_requiere_autorizacion: bool,
    /// Cupo de personas por franja; `null` = uso exclusivo.
    pub aforo: Option<u32>,
    #[schema(value_type = String, example = "150000")]
    pub tarifa: Decimal,
    /// `reserva` u `hora`.
    #[schema(value_type = String, example = "reserva")]
    pub cobro_por: CobroPor,
    #[schema(value_type = Option<String>)]
    pub tarifa_festivo: Option<Decimal>,
    #[schema(value_type = String, example = "0")]
    pub deposito: Decimal,
    pub cancelacion_sin_penalidad_horas: u32,
    #[schema(value_type = String, example = "0")]
    pub penalidad_porcentaje: Decimal,
    pub max_invitados: u32,
    /// `sin_entrega`, `entrega_llaves`, `entrega_con_inventario` o `acceso_automatico`.
    #[schema(value_type = String, example = "entrega_con_inventario")]
    pub metodo_entrega: MetodoEntrega,
}

impl From<PoliticaReserva> for PoliticaDto {
    fn from(p: PoliticaReserva) -> Self {
        Self {
            modo: p.modo,
            duracion_minima_min: p.duracion_minima_min,
            duracion_maxima_min: p.duracion_maxima_min,
            granularidad_min: p.granularidad_min,
            anticipacion_minima_horas: p.anticipacion_minima_horas,
            anticipacion_maxima_dias: p.anticipacion_maxima_dias,
            max_reservas_mes: p.max_reservas_mes,
            max_simultaneas: p.max_simultaneas,
            alistamiento_min: p.alistamiento_min,
            quien: p.quien,
            tenedor_requiere_autorizacion: p.tenedor_requiere_autorizacion,
            aforo: p.aforo,
            tarifa: p.tarifa,
            cobro_por: p.cobro_por,
            tarifa_festivo: p.tarifa_festivo,
            deposito: p.deposito,
            cancelacion_sin_penalidad_horas: p.cancelacion_sin_penalidad_horas,
            penalidad_porcentaje: p.penalidad_porcentaje,
            max_invitados: p.max_invitados,
            metodo_entrega: p.metodo_entrega,
        }
    }
}

impl From<PoliticaDto> for PoliticaReserva {
    fn from(p: PoliticaDto) -> Self {
        Self {
            modo: p.modo,
            duracion_minima_min: p.duracion_minima_min,
            duracion_maxima_min: p.duracion_maxima_min,
            granularidad_min: p.granularidad_min,
            anticipacion_minima_horas: p.anticipacion_minima_horas,
            anticipacion_maxima_dias: p.anticipacion_maxima_dias,
            max_reservas_mes: p.max_reservas_mes,
            max_simultaneas: p.max_simultaneas,
            alistamiento_min: p.alistamiento_min,
            quien: p.quien,
            tenedor_requiere_autorizacion: p.tenedor_requiere_autorizacion,
            aforo: p.aforo,
            tarifa: p.tarifa,
            cobro_por: p.cobro_por,
            tarifa_festivo: p.tarifa_festivo,
            deposito: p.deposito,
            cancelacion_sin_penalidad_horas: p.cancelacion_sin_penalidad_horas,
            penalidad_porcentaje: p.penalidad_porcentaje,
            max_invitados: p.max_invitados,
            metodo_entrega: p.metodo_entrega,
        }
    }
}

/// Detalle de un bien común.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BienComunDto {
    #[schema(value_type = String, example = "salon_social")]
    pub categoria: CategoriaBienComun,
    #[schema(value_type = Option<String>)]
    pub area_m2: Option<Decimal>,
    /// Art. 59: un bien esencial no se restringe por mora.
    #[serde(default)]
    pub es_esencial: bool,
    pub reglas: Option<String>,
    pub reglas_archivo_id: Option<Uuid>,
}

impl From<BienComunDto> for DetalleBienComun {
    fn from(b: BienComunDto) -> Self {
        Self {
            categoria: b.categoria,
            area_m2: b.area_m2,
            es_esencial: b.es_esencial,
            reglas: b.reglas,
            reglas_archivo_id: b.reglas_archivo_id.map(ArchivoId::from_uuid),
        }
    }
}

impl From<DetalleBienComun> for BienComunDto {
    fn from(b: DetalleBienComun) -> Self {
        Self {
            categoria: b.categoria,
            area_m2: b.area_m2,
            es_esencial: b.es_esencial,
            reglas: b.reglas,
            reglas_archivo_id: b.reglas_archivo_id.map(ArchivoId::as_uuid),
        }
    }
}

/// Detalle de un parqueadero.
#[derive(Debug, Serialize, ToSchema)]
pub struct ParqueaderoDto {
    pub codigo: String,
    #[schema(value_type = String, example = "carro")]
    pub tipo_vehiculo: TipoParqueadero,
    #[schema(value_type = String, example = "comunal")]
    pub naturaleza: NaturalezaParqueadero,
    pub bien_privado_id: Option<Uuid>,
}

impl From<DetalleParqueadero> for ParqueaderoDto {
    fn from(p: DetalleParqueadero) -> Self {
        Self {
            codigo: p.codigo,
            tipo_vehiculo: p.tipo_vehiculo,
            naturaleza: p.naturaleza,
            bien_privado_id: p.bien_privado_id.map(BienPrivadoId::as_uuid),
        }
    }
}

/// Recurso reservable.
#[derive(Debug, Serialize, ToSchema)]
pub struct RecursoRespuesta {
    pub id: Uuid,
    /// `bien_comun`, `parqueadero`, `agenda_administracion` o `herramienta`.
    #[schema(value_type = String, example = "bien_comun")]
    pub tipo: TipoRecurso,
    pub nombre: String,
    /// `disponible`, `en_mantenimiento`, `fuera_de_servicio` o `inactivo`.
    #[schema(value_type = String, example = "disponible")]
    pub estado: EstadoRecurso,
    pub ubicacion: Option<String>,
    pub descripcion: Option<String>,
    pub politica: PoliticaDto,
    pub horario: HorarioDto,
    pub bien_comun: Option<BienComunDto>,
    pub parqueadero: Option<ParqueaderoDto>,
}

impl From<Recurso> for RecursoRespuesta {
    fn from(r: Recurso) -> Self {
        Self {
            id: r.id.as_uuid(),
            tipo: r.tipo,
            horario: HorarioDto::desde(&r.horario),
            nombre: r.nombre,
            estado: r.estado,
            ubicacion: r.ubicacion,
            descripcion: r.descripcion,
            politica: r.politica.into(),
            bien_comun: r.bien_comun.map(Into::into),
            parqueadero: r.parqueadero.map(Into::into),
        }
    }
}

/// Filtro de recursos.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroRecursos {
    #[param(value_type = Option<String>)]
    pub tipo: Option<TipoRecurso>,
}

/// Recursos reservables (bienes comunes, parqueaderos, agenda, herramientas).
#[utoipa::path(
    get,
    path = "/recursos",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroRecursos),
    responses(
        (status = OK, description = "Recursos", body = Vec<RecursoRespuesta>),
        (status = FORBIDDEN, description = "Sin `bienes_comunes.ver` o sin el módulo (`modulo_no_habilitado`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_recursos(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroRecursos>,
) -> ApiResult<Json<Vec<RecursoRespuesta>>> {
    let lista = uc::listar_recursos(&state.unidad, &RelojSistema, &en.ctx, f.tipo).await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Recurso nuevo (los parqueaderos se crean en `/parqueaderos`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoRecursoPeticion {
    /// `bien_comun`, `agenda_administracion` o `herramienta`.
    #[schema(value_type = String, example = "bien_comun")]
    pub tipo: TipoRecurso,
    #[schema(example = "Salón social")]
    pub nombre: String,
    pub ubicacion: Option<String>,
    pub descripcion: Option<String>,
    pub politica: PoliticaDto,
    #[serde(default)]
    pub horario: HorarioDto,
    /// Obligatorio para los bienes comunes.
    pub bien_comun: Option<BienComunDto>,
}

/// Crea un bien común, la agenda de la administración o una herramienta prestable.
#[utoipa::path(
    post,
    path = "/recursos",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = NuevoRecursoPeticion,
    responses(
        (status = CREATED, description = "Recurso creado", body = RecursoRespuesta),
        (status = FORBIDDEN, description = "Sin `bienes_comunes.editar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Política u horario inválidos (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear_recurso(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<NuevoRecursoPeticion>,
) -> ApiResult<(StatusCode, Json<RecursoRespuesta>)> {
    let r = uc::crear_recurso(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        NuevoRecurso {
            tipo: p.tipo,
            nombre: p.nombre,
            ubicacion: p.ubicacion,
            descripcion: p.descripcion,
            politica: p.politica.into(),
            horario: p.horario.a_dominio()?,
            bien_comun: p.bien_comun.map(Into::into),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Un recurso.
#[utoipa::path(
    get,
    path = "/recursos/{recurso_id}",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("recurso_id" = Uuid, Path, description = "Recurso"),
    ),
    responses(
        (status = OK, description = "Recurso", body = RecursoRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn ver_recurso(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<RecursoRespuesta>> {
    let r = uc::ver_recurso(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        RecursoId::from_uuid(ruta.1),
    )
    .await?;
    Ok(Json(r.into()))
}

/// Cambios a un recurso (los omitidos no cambian).
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct CambiosRecursoPeticion {
    pub nombre: Option<String>,
    pub ubicacion: Option<String>,
    pub descripcion: Option<String>,
    pub politica: Option<PoliticaDto>,
    pub horario: Option<HorarioDto>,
    pub bien_comun: Option<BienComunDto>,
}

/// Edita datos, política, horario o detalle de un recurso.
#[utoipa::path(
    patch,
    path = "/recursos/{recurso_id}",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("recurso_id" = Uuid, Path, description = "Recurso"),
    ),
    request_body = CambiosRecursoPeticion,
    responses(
        (status = OK, description = "Recurso actualizado", body = RecursoRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn editar_recurso(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(c): Json<CambiosRecursoPeticion>,
) -> ApiResult<Json<RecursoRespuesta>> {
    let r = uc::editar_recurso(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        RecursoId::from_uuid(ruta.1),
        CambiosRecurso {
            nombre: c.nombre,
            ubicacion: c.ubicacion,
            descripcion: c.descripcion,
            politica: c.politica.map(Into::into),
            horario: c.horario.map(HorarioDto::a_dominio).transpose()?,
            bien_comun: c.bien_comun.map(Into::into),
        },
    )
    .await?;
    Ok(Json(r.into()))
}

/// Estado nuevo de un recurso.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EstadoRecursoPeticion {
    #[schema(value_type = String, example = "en_mantenimiento")]
    pub estado: EstadoRecurso,
    pub motivo: Option<String>,
}

/// Reservas canceladas al cerrar el recurso.
#[derive(Debug, Serialize, ToSchema)]
pub struct EstadoRecursoRespuesta {
    pub reservas_canceladas: u32,
}

/// Cambia el estado del recurso. Cerrarlo (mantenimiento, fuera de servicio, inactivo) cancela
/// las reservas futuras y avisa a los solicitantes.
#[utoipa::path(
    post,
    path = "/recursos/{recurso_id}/estado",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("recurso_id" = Uuid, Path, description = "Recurso"),
    ),
    request_body = EstadoRecursoPeticion,
    responses(
        (status = OK, description = "Estado cambiado", body = EstadoRecursoRespuesta),
        (status = NOT_FOUND, description = "No existe", body = ErrorRespuesta),
    )
)]
pub async fn cambiar_estado(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<EstadoRecursoPeticion>,
) -> ApiResult<Json<EstadoRecursoRespuesta>> {
    let n = uc::cambiar_estado_recurso(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        RecursoId::from_uuid(ruta.1),
        p.estado,
        p.motivo,
    )
    .await?;
    Ok(Json(EstadoRecursoRespuesta {
        reservas_canceladas: n,
    }))
}

/// Rango de fechas locales (inclusive, máximo 62 días).
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RangoFechas {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
}

/// Franja libre.
#[derive(Debug, Serialize, ToSchema)]
pub struct FranjaDto {
    pub inicio: DateTime<Utc>,
    pub fin: DateTime<Utc>,
}

impl From<Franja> for FranjaDto {
    fn from(f: Franja) -> Self {
        Self {
            inicio: f.inicio(),
            fin: f.fin(),
        }
    }
}

/// Franjas libres del recurso (horario − excepciones − reservas − alistamiento).
#[utoipa::path(
    get,
    path = "/recursos/{recurso_id}/disponibilidad",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("recurso_id" = Uuid, Path, description = "Recurso"),
        RangoFechas,
    ),
    responses(
        (status = OK, description = "Franjas libres", body = Vec<FranjaDto>),
        (status = UNPROCESSABLE_ENTITY, description = "Rango inválido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn disponibilidad(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Query(r): Query<RangoFechas>,
) -> ApiResult<Json<Vec<FranjaDto>>> {
    let libres = uc::disponibilidad(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        RecursoId::from_uuid(ruta.1),
        r.desde,
        r.hasta,
    )
    .await?;
    Ok(Json(libres.into_iter().map(Into::into).collect()))
}

// ---------------------------------------------------------------------------------------------
// Reservas
// ---------------------------------------------------------------------------------------------

/// Reserva (o cita).
#[derive(Debug, Serialize, ToSchema)]
pub struct ReservaRespuesta {
    pub id: Uuid,
    pub recurso_id: Uuid,
    pub bien_privado_id: Uuid,
    pub solicitante_persona_id: Option<Uuid>,
    pub inicio: DateTime<Utc>,
    pub fin: DateTime<Utc>,
    pub invitados: u32,
    /// `solicitada`, `aprobada`, `rechazada`, `en_curso`, `finalizada`, `con_novedad`,
    /// `cancelada` o `no_asistio`.
    #[schema(value_type = String, example = "aprobada")]
    pub estado: EstadoReserva,
    #[schema(value_type = String, example = "150000.00")]
    pub valor: Decimal,
    #[schema(value_type = String)]
    pub deposito: Decimal,
    #[schema(value_type = String)]
    pub penalidad: Decimal,
    /// `no_aplica`, `pendiente`, `pagado` o `exonerado`.
    #[schema(value_type = String, example = "pendiente")]
    pub estado_pago: EstadoPago,
    #[schema(value_type = String, example = "web")]
    pub canal: CanalReserva,
    #[schema(value_type = Option<String>)]
    pub modalidad: Option<ModalidadCita>,
    pub motivo: Option<String>,
    pub placa_visitante: Option<String>,
    pub observacion: Option<String>,
    pub creada_en: DateTime<Utc>,
}

impl From<Reserva> for ReservaRespuesta {
    fn from(r: Reserva) -> Self {
        Self {
            id: r.id.as_uuid(),
            recurso_id: r.recurso_id.as_uuid(),
            bien_privado_id: r.bien_privado_id.as_uuid(),
            solicitante_persona_id: r.solicitante_persona_id.map(domain::PersonaId::as_uuid),
            inicio: r.franja.inicio(),
            fin: r.franja.fin(),
            invitados: r.invitados,
            estado: r.estado,
            valor: r.valor,
            deposito: r.deposito,
            penalidad: r.penalidad,
            estado_pago: r.estado_pago,
            canal: r.canal,
            modalidad: r.modalidad,
            motivo: r.motivo,
            placa_visitante: r.placa_visitante.map(|p| p.as_str().to_owned()),
            observacion: r.observacion,
            creada_en: r.creada_en,
        }
    }
}

/// Filtros del listado de reservas.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroReservasParams {
    /// Solo las que pidió la cuenta (no exige `reservas.ver_todas`).
    #[serde(default)]
    pub mias: bool,
    pub recurso_id: Option<Uuid>,
    pub bien_privado_id: Option<Uuid>,
    #[param(value_type = Option<String>)]
    pub estado: Option<EstadoReserva>,
    pub desde: Option<DateTime<Utc>>,
    pub hasta: Option<DateTime<Utc>>,
}

/// Reservas (todas con `reservas.ver_todas`, o las propias con `mias=true`).
#[utoipa::path(
    get,
    path = "/reservas",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroReservasParams),
    responses(
        (status = OK, description = "Reservas", body = Vec<ReservaRespuesta>),
        (status = FORBIDDEN, description = "Sin `reservas.ver_todas` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_reservas(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroReservasParams>,
) -> ApiResult<Json<Vec<ReservaRespuesta>>> {
    let entre = match (f.desde, f.hasta) {
        (Some(d), Some(h)) => Some(Franja::new(d, h)?),
        _ => None,
    };
    let lista = uc::listar_reservas(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        FiltroReservas {
            recurso_id: f.recurso_id.map(RecursoId::from_uuid),
            bien_privado_id: f.bien_privado_id.map(BienPrivadoId::from_uuid),
            estado: f.estado,
            solicitada_por: None,
            entre,
        },
        f.mias,
    )
    .await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Solicitud de una reserva, cita o parqueadero de visitantes.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SolicitudPeticion {
    pub recurso_id: Uuid,
    pub bien_privado_id: Uuid,
    pub inicio: DateTime<Utc>,
    pub fin: DateTime<Utc>,
    #[serde(default)]
    pub invitados: u32,
    /// La genera el cliente; un reintento con la misma clave devuelve la reserva original.
    #[schema(example = "2b0c6a4e-…")]
    pub idempotency_key: String,
    #[serde(default = "canal_web")]
    #[schema(value_type = String, example = "web")]
    pub canal: CanalReserva,
    /// Citas: `presencial`, `llamada` o `videollamada`.
    #[schema(value_type = Option<String>)]
    pub modalidad: Option<ModalidadCita>,
    pub motivo: Option<String>,
    /// Obligatoria en parqueaderos de visitantes.
    #[schema(value_type = Option<String>, example = "ABC123")]
    pub placa_visitante: Option<Placa>,
}

const fn canal_web() -> CanalReserva {
    CanalReserva::Web
}

/// Solicita una reserva. Con confirmación inmediata queda aprobada; si no, solicitada.
#[utoipa::path(
    post,
    path = "/reservas",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = SolicitudPeticion,
    responses(
        (status = CREATED, description = "Reserva creada (o la original si se reintenta)", body = ReservaRespuesta),
        (status = CONFLICT, description = "Franja ocupada (`conflicto`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Incumple la política, el horario, los límites, el cupo o hay restricción (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn solicitar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<SolicitudPeticion>,
) -> ApiResult<(StatusCode, Json<ReservaRespuesta>)> {
    let r = uc::solicitar_reserva(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        SolicitudReserva {
            recurso_id: RecursoId::from_uuid(p.recurso_id),
            bien_privado_id: BienPrivadoId::from_uuid(p.bien_privado_id),
            franja: Franja::new(p.inicio, p.fin)?,
            invitados: p.invitados,
            idempotency_key: p.idempotency_key,
            canal: p.canal,
            modalidad: p.modalidad,
            motivo: p.motivo,
            placa_visitante: p.placa_visitante,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Entrega o recibo registrado.
#[derive(Debug, Serialize, ToSchema)]
pub struct EntregaRespuesta {
    #[schema(value_type = String, example = "recibo")]
    pub tipo: TipoEntrega,
    pub checklist: Vec<ChecklistDto>,
    pub fotos: Vec<Uuid>,
    pub novedad: Option<String>,
    #[schema(value_type = Option<String>)]
    pub valor_danos: Option<Decimal>,
    pub registrado_en: DateTime<Utc>,
}

impl From<Entrega> for EntregaRespuesta {
    fn from(e: Entrega) -> Self {
        Self {
            tipo: e.tipo,
            checklist: e.checklist.into_iter().map(Into::into).collect(),
            fotos: e.fotos.into_iter().map(ArchivoId::as_uuid).collect(),
            novedad: e.novedad,
            valor_danos: e.valor_danos,
            registrado_en: e.registrado_en,
        }
    }
}

/// Reserva con su entrega y su recibo.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleReservaRespuesta {
    pub reserva: ReservaRespuesta,
    pub entregas: Vec<EntregaRespuesta>,
}

fn id_reserva(ruta: (Uuid, Uuid)) -> ReservaId {
    ReservaId::from_uuid(ruta.1)
}

/// Una reserva con su entrega y su recibo (propia, o con `reservas.ver_todas`).
#[utoipa::path(
    get,
    path = "/reservas/{reserva_id}",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("reserva_id" = Uuid, Path, description = "Reserva"),
    ),
    responses(
        (status = OK, description = "Reserva", body = DetalleReservaRespuesta),
        (status = NOT_FOUND, description = "No existe o no es visible", body = ErrorRespuesta),
    )
)]
pub async fn ver_reserva(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DetalleReservaRespuesta>> {
    let (r, entregas) =
        uc::ver_reserva(&state.unidad, &RelojSistema, &en.ctx, id_reserva(ruta)).await?;
    Ok(Json(DetalleReservaRespuesta {
        reserva: r.into(),
        entregas: entregas.into_iter().map(Into::into).collect(),
    }))
}

/// Motivo opcional.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct MotivoPeticion {
    pub motivo: Option<String>,
}

/// Aprueba una reserva solicitada (avisa al solicitante).
#[utoipa::path(
    post,
    path = "/reservas/{reserva_id}/aprobar",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("reserva_id" = Uuid, Path, description = "Reserva"),
    ),
    responses(
        (status = OK, description = "Aprobada", body = ReservaRespuesta),
        (status = CONFLICT, description = "No está solicitada (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn aprobar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ReservaRespuesta>> {
    let r = uc::resolver_reserva(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_reserva(ruta),
        true,
        None,
    )
    .await?;
    Ok(Json(r.into()))
}

/// Rechaza una reserva solicitada.
#[utoipa::path(
    post,
    path = "/reservas/{reserva_id}/rechazar",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("reserva_id" = Uuid, Path, description = "Reserva"),
    ),
    request_body = MotivoPeticion,
    responses(
        (status = OK, description = "Rechazada", body = ReservaRespuesta),
        (status = CONFLICT, description = "No está solicitada (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn rechazar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<MotivoPeticion>,
) -> ApiResult<Json<ReservaRespuesta>> {
    let r = uc::resolver_reserva(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_reserva(ruta),
        false,
        p.motivo,
    )
    .await?;
    Ok(Json(r.into()))
}

/// Cancela una reserva (la propia, con penalidad si es tarde; o cualquiera con
/// `reservas.cancelar_cualquiera`, sin penalidad).
#[utoipa::path(
    post,
    path = "/reservas/{reserva_id}/cancelar",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("reserva_id" = Uuid, Path, description = "Reserva"),
    ),
    request_body = MotivoPeticion,
    responses(
        (status = OK, description = "Cancelada (con la penalidad aplicada)", body = ReservaRespuesta),
        (status = FORBIDDEN, description = "No es suya (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya empezó o terminó (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn cancelar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<MotivoPeticion>,
) -> ApiResult<Json<ReservaRespuesta>> {
    let r = uc::cancelar_reserva(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_reserva(ruta),
        p.motivo,
    )
    .await?;
    Ok(Json(r.into()))
}

/// Elemento del checklist.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChecklistDto {
    #[schema(example = "Sillas")]
    pub elemento: String,
    pub cantidad: Option<u32>,
    #[schema(example = "bueno")]
    pub estado: Option<String>,
}

impl From<ElementoChecklist> for ChecklistDto {
    fn from(e: ElementoChecklist) -> Self {
        Self {
            elemento: e.elemento,
            cantidad: e.cantidad,
            estado: e.estado,
        }
    }
}

/// Datos de una entrega o un recibo.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct EntregaPeticion {
    #[serde(default)]
    pub checklist: Vec<ChecklistDto>,
    /// Fotos (archivos ya subidos).
    #[serde(default)]
    pub fotos: Vec<Uuid>,
    pub novedad: Option<String>,
    /// Solo en el recibo: valor de los daños (queda como cargo pendiente del bien privado).
    #[schema(value_type = Option<String>)]
    pub valor_danos: Option<Decimal>,
}

impl From<EntregaPeticion> for DatosEntrega {
    fn from(p: EntregaPeticion) -> Self {
        Self {
            checklist: p
                .checklist
                .into_iter()
                .map(|c| ElementoChecklist {
                    elemento: c.elemento,
                    cantidad: c.cantidad,
                    estado: c.estado,
                })
                .collect(),
            fotos: p.fotos.into_iter().map(ArchivoId::from_uuid).collect(),
            novedad: p.novedad,
            valor_danos: p.valor_danos,
        }
    }
}

/// Registra la entrega del recurso (aprobada → en curso).
#[utoipa::path(
    post,
    path = "/reservas/{reserva_id}/entrega",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("reserva_id" = Uuid, Path, description = "Reserva"),
    ),
    request_body = EntregaPeticion,
    responses(
        (status = OK, description = "Entregado", body = ReservaRespuesta),
        (status = FORBIDDEN, description = "Sin `reservas.entregas.registrar` (`sin_permiso`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "No está aprobada (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn entrega(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<EntregaPeticion>,
) -> ApiResult<Json<ReservaRespuesta>> {
    let r = uc::registrar_entrega(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_reserva(ruta),
        TipoEntrega::Entrega,
        p.into(),
    )
    .await?;
    Ok(Json(r.into()))
}

/// Registra el recibo (en curso → finalizada, o con novedad si hay novedad o daños).
#[utoipa::path(
    post,
    path = "/reservas/{reserva_id}/recibo",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("reserva_id" = Uuid, Path, description = "Reserva"),
    ),
    request_body = EntregaPeticion,
    responses(
        (status = OK, description = "Recibido", body = ReservaRespuesta),
        (status = CONFLICT, description = "No está en curso (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn recibo(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<EntregaPeticion>,
) -> ApiResult<Json<ReservaRespuesta>> {
    let r = uc::registrar_entrega(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_reserva(ruta),
        TipoEntrega::Recibo,
        p.into(),
    )
    .await?;
    Ok(Json(r.into()))
}

/// Marca que el solicitante no asistió.
#[utoipa::path(
    post,
    path = "/reservas/{reserva_id}/no-asistio",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("reserva_id" = Uuid, Path, description = "Reserva"),
    ),
    responses(
        (status = OK, description = "Marcada", body = ReservaRespuesta),
        (status = CONFLICT, description = "No está aprobada (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn no_asistio(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ReservaRespuesta>> {
    let r = uc::marcar_no_asistio(&state.unidad, &RelojSistema, &en.ctx, id_reserva(ruta)).await?;
    Ok(Json(r.into()))
}

/// Estado del pago.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PagoPeticion {
    /// `pagado` o `exonerado`.
    #[schema(value_type = String, example = "pagado")]
    pub estado_pago: EstadoPago,
}

/// Registra a mano el pago de una reserva (pagado o exonerado).
#[utoipa::path(
    post,
    path = "/reservas/{reserva_id}/pago",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("reserva_id" = Uuid, Path, description = "Reserva"),
    ),
    request_body = PagoPeticion,
    responses(
        (status = OK, description = "Pago registrado", body = ReservaRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "No tiene valor que pagar (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn pago(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PagoPeticion>,
) -> ApiResult<Json<ReservaRespuesta>> {
    let r = uc::registrar_pago(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        id_reserva(ruta),
        p.estado_pago,
    )
    .await?;
    Ok(Json(r.into()))
}

// ---------------------------------------------------------------------------------------------
// Restricciones y tenedores
// ---------------------------------------------------------------------------------------------

/// Restricción de reservas de un bien privado.
#[derive(Debug, Serialize, ToSchema)]
pub struct RestriccionRespuesta {
    pub id: Uuid,
    pub bien_privado_id: Uuid,
    pub motivo: String,
    pub soporte_archivo_id: Option<Uuid>,
    pub desde: DateTime<Utc>,
    pub hasta: Option<DateTime<Utc>>,
}

impl From<Restriccion> for RestriccionRespuesta {
    fn from(r: Restriccion) -> Self {
        Self {
            id: r.id.as_uuid(),
            bien_privado_id: r.bien_privado_id.as_uuid(),
            motivo: r.motivo,
            soporte_archivo_id: r.soporte_archivo_id.map(ArchivoId::as_uuid),
            desde: r.desde,
            hasta: r.hasta,
        }
    }
}

/// Filtro de restricciones.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FiltroRestricciones {
    pub bien_privado_id: Option<Uuid>,
}

/// Restricciones de reserva (mora o sanción).
#[utoipa::path(
    get,
    path = "/restricciones-reserva",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), FiltroRestricciones),
    responses(
        (status = OK, description = "Restricciones", body = Vec<RestriccionRespuesta>),
        (status = FORBIDDEN, description = "Sin `reservas.restringir` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn listar_restricciones(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Query(f): Query<FiltroRestricciones>,
) -> ApiResult<Json<Vec<RestriccionRespuesta>>> {
    let lista = uc::listar_restricciones(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        f.bien_privado_id.map(BienPrivadoId::from_uuid),
    )
    .await?;
    Ok(Json(lista.into_iter().map(Into::into).collect()))
}

/// Restricción nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RestriccionPeticion {
    pub bien_privado_id: Uuid,
    #[schema(example = "Mora superior a tres cuotas de administración")]
    pub motivo: String,
    pub soporte_archivo_id: Option<Uuid>,
    /// `null` = hasta levantarla.
    pub hasta: Option<DateTime<Utc>>,
}

/// Restringe las reservas de un bien privado (marca manual de mora o sanción; no aplica a los
/// bienes comunes esenciales, art. 59).
#[utoipa::path(
    post,
    path = "/restricciones-reserva",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad")),
    request_body = RestriccionPeticion,
    responses(
        (status = CREATED, description = "Restricción creada", body = RestriccionRespuesta),
        (status = FORBIDDEN, description = "Sin `reservas.restringir` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn restringir(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Json(p): Json<RestriccionPeticion>,
) -> ApiResult<(StatusCode, Json<RestriccionRespuesta>)> {
    let r = uc::restringir_bien(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        BienPrivadoId::from_uuid(p.bien_privado_id),
        &p.motivo,
        p.soporte_archivo_id.map(ArchivoId::from_uuid),
        p.hasta,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(r.into())))
}

/// Levanta una restricción desde ahora.
#[utoipa::path(
    post,
    path = "/restricciones-reserva/{restriccion_id}/levantar",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("restriccion_id" = Uuid, Path, description = "Restricción"),
    ),
    responses(
        (status = NO_CONTENT, description = "Levantada"),
        (status = NOT_FOUND, description = "No existe o ya terminó", body = ErrorRespuesta),
    )
)]
pub async fn levantar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    uc::levantar_restriccion(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        RestriccionId::from_uuid(ruta.1),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Autorización de reservas de un tenedor.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AutorizacionPeticion {
    pub autorizadas: bool,
}

/// El propietario (o la administración) autoriza o retira las reservas de su tenedor, cuando
/// la política del recurso lo exige.
#[utoipa::path(
    put,
    path = "/residencias/{residencia_id}/autorizacion-reservas",
    tag = ETIQUETA_RESERVAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("residencia_id" = Uuid, Path, description = "Residencia del tenedor"),
    ),
    request_body = AutorizacionPeticion,
    responses(
        (status = NO_CONTENT, description = "Guardada"),
        (status = FORBIDDEN, description = "No es titular del bien ni tiene `censo.editar`", body = ErrorRespuesta),
        (status = NOT_FOUND, description = "No es una residencia de tenedor vigente", body = ErrorRespuesta),
    )
)]
pub async fn autorizar_tenedor(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<AutorizacionPeticion>,
) -> ApiResult<StatusCode> {
    uc::autorizar_reservas_tenedor(
        &state.unidad,
        &en.ctx,
        ResidenciaId::from_uuid(ruta.1),
        p.autorizadas,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
