//! Habilitación de la entidad ante la DIAN con el motor propio:
//! `/entidades/{entidad_id}/facturacion/habilitacion/…`. Los secretos (PIN, `.p12` y su
//! contraseña) entran por aquí y nunca vuelven en una respuesta.

use application::{
    casos_uso::facturacion::habilitacion::{
        self as uc, DatosSoftware, Habilitacion, VistaCertificado,
    },
    puertos::{RelojSistema, facturacion::EnvioSetPruebas, tributario::ResolucionNumeracion},
};
use axum::extract::State;
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    facturacion::{AmbienteDian, EstadoEnvioSet, EstadoHabilitacion},
    tributario::TipoDocumentoElectronico,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult, ErrorRespuesta},
    extractores::Json,
    openapi::ETIQUETA_FACTURACION,
    sesion::EnEntidad,
    state::AppState,
};

/// Rutas de la habilitación.
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(ver_habilitacion))
        .routes(routes!(registrar_software))
        .routes(routes!(cargar_certificado))
        .routes(routes!(enviar_set_pruebas))
        .routes(routes!(consultar_set_pruebas))
        .routes(routes!(pasar_a_produccion))
        .routes(routes!(sincronizar_rangos))
}

/// Certificado de firma activo (sin secretos).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CertificadoFirmaRespuesta)]
pub struct CertificadoRespuesta {
    pub id: Uuid,
    /// Titular (RFC 2253).
    pub sujeto: String,
    /// Entidad de certificación (RFC 2253).
    pub emisor: String,
    pub serial: String,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: DateTime<Utc>,
    /// Vence en menos de 30 días (o ya venció): cargue uno nuevo.
    pub por_vencer: bool,
}

impl From<VistaCertificado> for CertificadoRespuesta {
    fn from(c: VistaCertificado) -> Self {
        Self {
            id: c.id.as_uuid(),
            sujeto: c.sujeto,
            emisor: c.emisor,
            serial: c.serial,
            vigente_desde: c.vigente_desde,
            vigente_hasta: c.vigente_hasta,
            por_vencer: c.por_vencer,
        }
    }
}

/// Documento del set de pruebas enviado a la DIAN.
#[derive(Debug, Serialize, ToSchema)]
pub struct EnvioSetRespuesta {
    pub id: Uuid,
    #[schema(value_type = crate::esquemas::TipoDocumentoElectronico)]
    pub tipo: TipoDocumentoElectronico,
    pub numero: String,
    /// CUFE del documento.
    pub codigo: String,
    #[schema(value_type = crate::esquemas::EstadoEnvioSet)]
    pub estado: EstadoEnvioSet,
    /// Respuesta de la DIAN (reglas incumplidas, si las hay).
    pub respuesta: Option<String>,
    pub enviado_en: DateTime<Utc>,
    pub consultado_en: Option<DateTime<Utc>>,
}

impl From<EnvioSetPruebas> for EnvioSetRespuesta {
    fn from(e: EnvioSetPruebas) -> Self {
        Self {
            id: e.id.as_uuid(),
            tipo: e.tipo,
            numero: format!("{}{}", e.prefijo, e.numero),
            codigo: e.codigo,
            estado: e.estado,
            respuesta: e.respuesta,
            enviado_en: e.enviado_en,
            consultado_en: e.consultado_en,
        }
    }
}

/// Estado de la habilitación ante la DIAN.
#[derive(Debug, Serialize, ToSchema)]
pub struct HabilitacionRespuesta {
    /// `null` si aún no registra el software.
    pub software_id: Option<String>,
    pub test_set_id: Option<String>,
    #[schema(value_type = Option<crate::esquemas::EstadoHabilitacion>)]
    pub estado: Option<EstadoHabilitacion>,
    #[schema(value_type = Option<crate::esquemas::AmbienteDian>)]
    pub ambiente: Option<AmbienteDian>,
    pub certificado: Option<CertificadoRespuesta>,
    pub envios: Vec<EnvioSetRespuesta>,
}

impl From<Habilitacion> for HabilitacionRespuesta {
    fn from(h: Habilitacion) -> Self {
        Self {
            software_id: h.software_id,
            test_set_id: h.test_set_id,
            estado: h.estado,
            ambiente: h.ambiente,
            certificado: h.certificado.map(Into::into),
            envios: h.envios.into_iter().map(Into::into).collect(),
        }
    }
}

/// Estado de la habilitación: software, certificado y set de pruebas.
#[utoipa::path(
    get,
    path = "/habilitacion",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Habilitación", body = HabilitacionRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.configurar` o sin el módulo", body = ErrorRespuesta),
    )
)]
pub async fn ver_habilitacion(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<HabilitacionRespuesta>> {
    Ok(Json(
        uc::habilitacion(&state.unidad, &RelojSistema, &en.ctx)
            .await?
            .into(),
    ))
}

/// Software registrado en el catálogo de participantes de la DIAN.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SoftwarePeticion {
    pub software_id: String,
    /// PIN que la entidad definió al registrar el software: se guarda cifrado y no vuelve.
    pub pin: String,
    pub test_set_id: Option<String>,
}

/// Registra (o cambia) el software de la entidad; cambiarlo vuelve la habilitación a
/// `registrado`.
#[utoipa::path(
    put,
    path = "/habilitacion/software",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = SoftwarePeticion,
    responses(
        (status = OK, description = "Software registrado", body = HabilitacionRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Ya está en producción (`conflicto_estado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Dato inválido o servidor sin clave maestra (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn registrar_software(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<SoftwarePeticion>,
) -> ApiResult<Json<HabilitacionRespuesta>> {
    let h = uc::registrar_software(
        &state.unidad,
        state.cofre.as_ref(),
        &RelojSistema,
        &en.ctx,
        DatosSoftware {
            software_id: p.software_id,
            pin: p.pin,
            test_set_id: p.test_set_id,
        },
    )
    .await?;
    Ok(Json(h.into()))
}

/// Certificado de firma (`.p12`) en Base64 con su contraseña.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CertificadoPeticion {
    /// El `.p12` o `.pfx` en Base64 (hasta 64 KB).
    pub archivo_base64: String,
    /// Contraseña del `.p12`: se guarda cifrada y no vuelve.
    pub contrasena: String,
}

/// Carga el certificado de firma de la entidad (lo valida, lo cifra y lo deja activo).
#[utoipa::path(
    post,
    path = "/habilitacion/certificado",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = CertificadoPeticion,
    responses(
        (status = OK, description = "Certificado cargado", body = CertificadoRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.certificado.gestionar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Archivo o contraseña inválidos, certificado vencido o sin no repudio (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn cargar_certificado(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<CertificadoPeticion>,
) -> ApiResult<Json<CertificadoRespuesta>> {
    let p12 = STANDARD
        .decode(p.archivo_base64.trim())
        .map_err(|_| ApiError::PeticionInvalida("archivo_base64 no es Base64 válido".into()))?;
    let c = uc::cargar_certificado(
        &state.unidad,
        (state.cofre.as_ref(), &state.lector_certificados),
        &RelojSistema,
        &en.ctx,
        &p12,
        &p.contrasena,
    )
    .await?;
    Ok(Json(c.into()))
}

/// Cuántos documentos del set de pruebas enviar.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetPruebasPeticion {
    /// Facturas de prueba (la DIAN indica cuántas pide al registrar el software). El total de
    /// documentos va de 1 a 50.
    #[schema(example = 8)]
    pub cantidad: u32,
    /// Notas crédito, cada una referida a una factura de este envío.
    #[serde(default)]
    #[schema(example = 1)]
    pub notas_credito: u32,
    /// Notas débito, cada una referida a una factura de este envío.
    #[serde(default)]
    #[schema(example = 1)]
    pub notas_debito: u32,
}

/// Envía al ambiente de habilitación las facturas de prueba y las notas crédito y débito que
/// pida la DIAN, con el TestSetId.
#[utoipa::path(
    post,
    path = "/habilitacion/set-pruebas",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    request_body = SetPruebasPeticion,
    responses(
        (status = OK, description = "Documentos enviados", body = Vec<EnvioSetRespuesta>),
        (status = FORBIDDEN, description = "Sin `facturacion.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Falta el software, el certificado o el TestSetId (`validacion`)", body = ErrorRespuesta),
        (status = INTERNAL_SERVER_ERROR, description = "La DIAN no responde (`interno`): reintente", body = ErrorRespuesta),
    )
)]
pub async fn enviar_set_pruebas(
    State(state): State<AppState>,
    en: EnEntidad,
    Json(p): Json<SetPruebasPeticion>,
) -> ApiResult<Json<Vec<EnvioSetRespuesta>>> {
    let v = uc::enviar_set_pruebas(
        &state.unidad,
        (state.facturacion.propio(), state.cofre.as_ref()),
        &RelojSistema,
        &en.ctx,
        uc::CantidadesSet {
            facturas: p.cantidad,
            notas_credito: p.notas_credito,
            notas_debito: p.notas_debito,
        },
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}

/// Consulta en la DIAN los envíos del set; si todos fueron aceptados, la entidad queda
/// `habilitado`.
#[utoipa::path(
    post,
    path = "/habilitacion/set-pruebas/consulta",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Habilitación actualizada", body = HabilitacionRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Falta el software o el certificado (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn consultar_set_pruebas(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<HabilitacionRespuesta>> {
    let h = uc::consultar_set_pruebas(
        &state.unidad,
        (state.facturacion.propio(), state.cofre.as_ref()),
        &RelojSistema,
        &en.ctx,
    )
    .await?;
    Ok(Json(h.into()))
}

/// Pasa la entidad a producción con el motor propio (exige `habilitado`).
#[utoipa::path(
    post,
    path = "/habilitacion/produccion",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "En producción", body = HabilitacionRespuesta),
        (status = FORBIDDEN, description = "Sin `facturacion.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = CONFLICT, description = "Aún no está habilitada (`conflicto_estado`)", body = ErrorRespuesta),
    )
)]
pub async fn pasar_a_produccion(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<HabilitacionRespuesta>> {
    Ok(Json(
        uc::pasar_a_produccion(&state.unidad, &RelojSistema, &en.ctx)
            .await?
            .into(),
    ))
}

/// Resolución de numeración registrada desde la DIAN (sin la clave técnica).
#[derive(Debug, Serialize, ToSchema)]
pub struct RangoSincronizadoRespuesta {
    pub id: Uuid,
    pub prefijo: String,
    pub desde: u64,
    pub hasta: u64,
    pub numero_resolucion: String,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: NaiveDate,
}

impl From<ResolucionNumeracion> for RangoSincronizadoRespuesta {
    fn from(r: ResolucionNumeracion) -> Self {
        Self {
            id: r.id.as_uuid(),
            prefijo: r.prefijo,
            desde: r.desde,
            hasta: r.hasta,
            numero_resolucion: r.numero_resolucion,
            vigente_desde: r.vigente_desde,
            vigente_hasta: r.vigente_hasta,
        }
    }
}

/// Trae de la DIAN los rangos autorizados al software y registra los que falten como
/// resoluciones de factura (con su clave técnica). Devuelve los nuevos.
#[utoipa::path(
    post,
    path = "/habilitacion/rangos",
    tag = ETIQUETA_FACTURACION,
    security(("bearer" = [])),
    params(("entidad_id" = Uuid, Path, description = "Entidad")),
    responses(
        (status = OK, description = "Rangos nuevos", body = Vec<RangoSincronizadoRespuesta>),
        (status = FORBIDDEN, description = "Sin `facturacion.configurar` o sin el módulo", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "La entidad no está en producción (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn sincronizar_rangos(
    State(state): State<AppState>,
    en: EnEntidad,
) -> ApiResult<Json<Vec<RangoSincronizadoRespuesta>>> {
    let v = uc::sincronizar_rangos(
        &state.unidad,
        (state.facturacion.propio(), state.cofre.as_ref()),
        &RelojSistema,
        &en.ctx,
    )
    .await?;
    Ok(Json(v.into_iter().map(Into::into).collect()))
}
