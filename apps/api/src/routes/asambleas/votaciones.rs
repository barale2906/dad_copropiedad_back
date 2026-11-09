//! Preguntas, votos registrados por un operador o un puesto de escaneo, resultados, detalle y
//! elecciones.

use application::{
    casos_uso::asambleas::votaciones::{
        self as uc, DefinicionPregunta, Elegido, NuevaOpcion, SolicitudVoto, Votante,
    },
    puertos::{RelojSistema, asambleas::DetalleVoto},
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use domain::{
    AsistenteId, BaseComputo, Canal, CargoEleccion, ClaveIdempotencia, CodigoRotulo, OpcionId,
    PersonaId, PreguntaId, PuntoOrdenId, ReunionId, TipoPregunta, TipoVoto,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::dto::{PreguntaRespuesta, ReglaDto, VotoRespuesta, exigir};
use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::{Json, Path},
    openapi::ETIQUETA_ASAMBLEAS,
    sesion::EnCopropiedad,
    state::AppState,
};

pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(listar, crear))
        .routes(routes!(editar, eliminar))
        .routes(routes!(estado))
        .routes(routes!(votar))
        .routes(routes!(resultados))
        .routes(routes!(detalle))
        .routes(routes!(aplicar_eleccion))
}

fn ids(r: (Uuid, Uuid, Uuid)) -> (ReunionId, PreguntaId) {
    (ReunionId::from_uuid(r.1), PreguntaId::from_uuid(r.2))
}

/// Opción nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct OpcionPeticion {
    pub texto: String,
    /// Voto en blanco (solo elecciones).
    #[serde(default)]
    pub en_blanco: bool,
    /// Integrantes de la plancha o candidato (elecciones), en orden.
    #[serde(default)]
    pub candidatos: Vec<Uuid>,
}

/// Definición de una pregunta.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PreguntaPeticion {
    #[schema(example = "¿Aprueba el presupuesto 2027?")]
    pub enunciado: String,
    #[schema(value_type = String, example = "si_no")]
    pub tipo: TipoPregunta,
    /// `nominal` (por defecto) o `reservado`.
    #[schema(value_type = Option<String>, example = "nominal")]
    pub tipo_voto: Option<TipoVoto>,
    /// Por defecto, la de la reunión.
    #[schema(value_type = Option<String>, example = "coeficiente")]
    pub base_computo: Option<BaseComputo>,
    /// Por defecto, mayoría simple.
    pub regla_mayoria: Option<ReglaDto>,
    /// Selección múltiple: máximo de opciones.
    pub max_selecciones: Option<u16>,
    /// Elección: cargos a proveer.
    pub cupos: Option<u16>,
    /// Elección: cargo que se elige (`consejero`, `revisor_fiscal`, `comite_convivencia`,
    /// `presidente_asamblea`, `secretario_asamblea`, `comision_verificadora`).
    #[schema(value_type = Option<String>, example = "consejero")]
    pub cargo: Option<CargoEleccion>,
    /// En sí/no se puede omitir (se usan «Sí», «No» y «Abstención» si `con_abstencion`).
    #[serde(default)]
    pub opciones: Vec<OpcionPeticion>,
    #[serde(default)]
    pub con_abstencion: bool,
    pub punto_id: Option<Uuid>,
    /// Segundos; `0` = sin temporizador; omitido = el de la reunión.
    pub temporizador_segundos: Option<u32>,
}

impl PreguntaPeticion {
    fn definicion(self) -> ApiResult<DefinicionPregunta> {
        Ok(DefinicionPregunta {
            enunciado: self.enunciado,
            tipo: self.tipo,
            tipo_voto: self.tipo_voto.unwrap_or(TipoVoto::Nominal),
            base_computo: self.base_computo,
            regla_mayoria: match self.regla_mayoria {
                Some(r) => r.validar()?,
                None => domain::ReglaMayoria::Simple,
            },
            max_selecciones: self.max_selecciones.unwrap_or(1),
            cupos: self.cupos,
            cargo: self.cargo,
            opciones: self
                .opciones
                .into_iter()
                .map(|o| NuevaOpcion {
                    texto: o.texto,
                    en_blanco: o.en_blanco,
                    candidatos: o.candidatos.into_iter().map(PersonaId::from_uuid).collect(),
                })
                .collect(),
            con_abstencion: self.con_abstencion,
            punto_id: self.punto_id.map(PuntoOrdenId::from_uuid),
            temporizador_segundos: self.temporizador_segundos,
        })
    }
}

/// Preguntas de la reunión, en orden.
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/preguntas",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    responses((status = OK, description = "Preguntas", body = Vec<PreguntaRespuesta>))
)]
pub async fn listar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<PreguntaRespuesta>>> {
    let id = ruta.1;
    let v = uc::listar_preguntas(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Crea una pregunta en borrador (antes o durante la reunión).
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/preguntas",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión")),
    request_body = PreguntaPeticion,
    responses(
        (status = CREATED, description = "Pregunta creada", body = PreguntaRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Definición incoherente (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn crear(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid)>,
    Json(p): Json<PreguntaPeticion>,
) -> ApiResult<(StatusCode, Json<PreguntaRespuesta>)> {
    let id = ruta.1;
    let q = uc::crear_pregunta(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        ReunionId::from_uuid(id),
        p.definicion()?,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(q.into())))
}

/// Reemplaza la definición de una pregunta en borrador.
#[utoipa::path(
    put,
    path = "/asambleas/{reunion_id}/preguntas/{pregunta_id}",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("pregunta_id" = Uuid, Path, description = "Pregunta")),
    request_body = PreguntaPeticion,
    responses(
        (status = OK, description = "Pregunta actualizada", body = PreguntaRespuesta),
        (status = CONFLICT, description = "Ya no está en borrador (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn editar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    Json(p): Json<PreguntaPeticion>,
) -> ApiResult<Json<PreguntaRespuesta>> {
    let (r, q) = ids(ruta);
    Ok(Json(
        uc::editar_pregunta(&state.unidad, &RelojSistema, &en.ctx, r, q, p.definicion()?)
            .await?
            .into(),
    ))
}

/// Elimina una pregunta en borrador.
#[utoipa::path(
    delete,
    path = "/asambleas/{reunion_id}/preguntas/{pregunta_id}",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("pregunta_id" = Uuid, Path, description = "Pregunta")),
    responses(
        (status = NO_CONTENT, description = "Eliminada"),
        (status = CONFLICT, description = "Ya se abrió: se anula (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn eliminar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let (r, q) = ids(ruta);
    uc::eliminar_pregunta(&state.unidad, &RelojSistema, &en.ctx, r, q).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Acción sobre la pregunta.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccionPregunta {
    Abrir,
    Cerrar,
    Anular,
}

/// Cambio de estado de una pregunta.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EstadoPreguntaPeticion {
    pub accion: AccionPregunta,
    /// Obligatorio al anular.
    pub motivo: Option<String>,
}

/// Abre (habilita a los presentes y les envía la pregunta por WhatsApp), cierra (guarda el
/// escrutinio) o anula la pregunta.
#[utoipa::path(
    put,
    path = "/asambleas/{reunion_id}/preguntas/{pregunta_id}/estado",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("pregunta_id" = Uuid, Path, description = "Pregunta")),
    request_body = EstadoPreguntaPeticion,
    responses(
        (status = OK, description = "Pregunta actualizada", body = PreguntaRespuesta),
        (status = CONFLICT, description = "Reunión no instalada o transición inválida (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn estado(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    Json(p): Json<EstadoPreguntaPeticion>,
) -> ApiResult<Json<PreguntaRespuesta>> {
    let (r, q) = ids(ruta);
    let (u, c) = (&state.unidad, &en.ctx);
    let p = match p.accion {
        AccionPregunta::Abrir => uc::abrir_pregunta(u, &RelojSistema, c, r, q).await?,
        AccionPregunta::Cerrar => uc::cerrar_pregunta(u, &RelojSistema, c, r, q).await?,
        AccionPregunta::Anular => {
            uc::anular_pregunta(
                u,
                &RelojSistema,
                c,
                r,
                q,
                p.motivo.as_deref().unwrap_or_default(),
            )
            .await?
        }
    };
    Ok(Json(p.into()))
}

/// Voto registrado por un operador o un puesto de escaneo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VotoPeticion {
    /// El asistente que buscó el operador …
    pub asistente_id: Option<Uuid>,
    /// … o el rótulo escaneado en el puesto.
    #[schema(example = "R-0042")]
    pub rotulo: Option<String>,
    pub opciones: Vec<Uuid>,
    /// `presencial` (operador) o `rotulo` (puesto de escaneo).
    #[schema(value_type = String, example = "rotulo")]
    pub canal: Canal,
}

/// Registra el voto del asistente por **todos** sus bienes habilitados. Exige
/// `asambleas.votos.registrar` y la cabecera `Idempotency-Key` (una por lectura: la consola
/// de la mesa la conserva si reintenta tras un corte).
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/preguntas/{pregunta_id}/votos",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(
        ("copropiedad_id" = Uuid, Path, description = "Copropiedad"),
        ("reunion_id" = Uuid, Path, description = "Reunión"),
        ("pregunta_id" = Uuid, Path, description = "Pregunta"),
        ("Idempotency-Key" = String, Header, description = "Clave única de la lectura"),
    ),
    request_body = VotoPeticion,
    responses(
        (status = CREATED, description = "Voto registrado (o el original si se reintenta)", body = VotoRespuesta),
        (status = FORBIDDEN, description = "Sin permiso (`sin_permiso`) o el asistente no está presente (`no_inscrito`)", body = ErrorRespuesta),
        (status = CONFLICT, description = "Sus bienes ya votaron (`ya_voto`) o la pregunta cerró (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn votar(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    cabeceras: HeaderMap,
    Json(p): Json<VotoPeticion>,
) -> ApiResult<(StatusCode, Json<VotoRespuesta>)> {
    let (r, q) = ids(ruta);
    exigir(
        matches!(p.canal, Canal::Presencial | Canal::Rotulo),
        "el canal es `presencial` o `rotulo`",
    )?;
    let clave = cabeceras
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::PeticionInvalida("falta la cabecera `Idempotency-Key`".into()))?;
    let votante = match (p.asistente_id, p.rotulo) {
        (Some(a), None) => Votante::Asistente(AsistenteId::from_uuid(a)),
        (None, Some(c)) => Votante::Rotulo(CodigoRotulo::new(&c)?),
        _ => {
            return Err(ApiError::PeticionInvalida(
                "indique `asistente_id` o `rotulo` (uno solo)".into(),
            ));
        }
    };
    let v = uc::emitir_voto(
        &state.unidad,
        &RelojSistema,
        &en.ctx,
        SolicitudVoto {
            reunion_id: r,
            pregunta_id: q,
            votante,
            opciones: p.opciones.into_iter().map(OpcionId::from_uuid).collect(),
            canal: p.canal,
            idempotency_key: ClaveIdempotencia::new(clave)?,
            wamid: None,
            confirmar: true,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(v.into())))
}

/// Escrutinio de una pregunta.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResultadosRespuesta {
    pub pregunta: PreguntaRespuesta,
    /// Totales por opción por coeficiente y por unidades, habilitados, votantes, decisión y
    /// reparto de cupos (elecciones). Decimales como texto.
    #[schema(value_type = Object)]
    pub escrutinio: serde_json::Value,
}

/// Resultados (en vivo si está abierta; los guardados si cerró). En reservadas se publican
/// solo los totales.
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/preguntas/{pregunta_id}/resultados",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("pregunta_id" = Uuid, Path, description = "Pregunta")),
    responses((status = OK, description = "Resultados", body = ResultadosRespuesta))
)]
pub async fn resultados(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
) -> ApiResult<Json<ResultadosRespuesta>> {
    let (r, q) = ids(ruta);
    let (p, e) = uc::resultados(&state.unidad, &RelojSistema, &en.ctx, r, q).await?;
    Ok(Json(ResultadosRespuesta {
        pregunta: p.into(),
        escrutinio: serde_json::to_value(e).map_err(|e| ApiError::Interno(e.into()))?,
    }))
}

/// Voto de un bien.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleVotoRespuesta {
    pub bien_privado_id: Uuid,
    pub codigo: String,
    #[schema(value_type = String, example = "0.01250000")]
    pub coeficiente: Decimal,
    pub opcion_id: Uuid,
    #[schema(value_type = String, example = "whatsapp")]
    pub canal: Canal,
    pub asistente: String,
    pub apoderado: bool,
    pub registrado_en: DateTime<Utc>,
}

impl From<DetalleVoto> for DetalleVotoRespuesta {
    fn from(d: DetalleVoto) -> Self {
        Self {
            bien_privado_id: d.bien_privado_id.as_uuid(),
            codigo: d.codigo,
            coeficiente: d.coeficiente.valor(),
            opcion_id: d.opcion_id.as_uuid(),
            canal: d.canal,
            asistente: d.asistente,
            apoderado: d.apoderado,
            registrado_en: d.registrado_en,
        }
    }
}

/// Detalle bien → opción. En reservadas exige `asambleas.votos.ver_detalle` y cada consulta
/// queda auditada.
#[utoipa::path(
    get,
    path = "/asambleas/{reunion_id}/preguntas/{pregunta_id}/detalle",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("pregunta_id" = Uuid, Path, description = "Pregunta")),
    responses(
        (status = OK, description = "Detalle", body = Vec<DetalleVotoRespuesta>),
        (status = FORBIDDEN, description = "Reservada sin `asambleas.votos.ver_detalle` (`sin_permiso`)", body = ErrorRespuesta),
    )
)]
pub async fn detalle(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
) -> ApiResult<Json<Vec<DetalleVotoRespuesta>>> {
    let (r, q) = ids(ruta);
    let v = uc::detalle_votos(&state.unidad, &RelojSistema, &en.ctx, r, q).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Vigencia de los roles de los elegidos.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct EleccionPeticion {
    /// Fin del periodo (p. ej. la próxima asamblea ordinaria); `null` = sin fecha.
    pub vigente_hasta: Option<DateTime<Utc>>,
}

/// Elegido.
#[derive(Debug, Serialize, ToSchema)]
pub struct ElegidoRespuesta {
    pub persona_id: Uuid,
    pub nombre: String,
    /// `false`: la persona no tiene cuenta; el rol se asigna cuando la cree.
    pub rol_asignado: bool,
}

impl From<Elegido> for ElegidoRespuesta {
    fn from(e: Elegido) -> Self {
        Self {
            persona_id: e.persona_id.as_uuid(),
            nombre: e.nombre,
            rol_asignado: e.rol_asignado,
        }
    }
}

/// Aplica una elección cerrada de consejo, revisor fiscal o comité: crea los roles de los
/// elegidos (origen `asamblea`) y termina los del periodo anterior.
#[utoipa::path(
    post,
    path = "/asambleas/{reunion_id}/preguntas/{pregunta_id}/eleccion",
    tag = ETIQUETA_ASAMBLEAS,
    security(("bearer" = [])),
    params(("copropiedad_id" = Uuid, Path, description = "Copropiedad"), ("reunion_id" = Uuid, Path, description = "Reunión"), ("pregunta_id" = Uuid, Path, description = "Pregunta")),
    request_body = EleccionPeticion,
    responses(
        (status = OK, description = "Elegidos", body = Vec<ElegidoRespuesta>),
        (status = CONFLICT, description = "No es una elección cerrada de un cargo o ya se aplicó (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn aplicar_eleccion(
    State(state): State<AppState>,
    en: EnCopropiedad,
    Path(ruta): Path<(Uuid, Uuid, Uuid)>,
    Json(p): Json<EleccionPeticion>,
) -> ApiResult<Json<Vec<ElegidoRespuesta>>> {
    let (r, q) = ids(ruta);
    let v =
        uc::aplicar_eleccion(&state.unidad, &RelojSistema, &en.ctx, r, q, p.vigente_hasta).await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}
