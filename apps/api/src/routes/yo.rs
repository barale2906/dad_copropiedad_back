//! Rutas de la cuenta autenticada: `/api/v1/yo/…`.
//!
//! Datos propios de quien inició sesión, sin depender de una copropiedad: perfil, copropiedades
//! donde tiene roles (para elegir en cuál trabajar), correo, contraseña, sesiones, segundo
//! factor y sus solicitudes en todas sus copropiedades.

use application::{casos_uso::autenticacion as uc, puertos::RelojSistema};
use axum::{extract::State, http::StatusCode};
use domain::{ContrasenaNueva, Permiso};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    error::{ApiResult, ErrorRespuesta},
    extractores::{Json, Query},
    openapi::ETIQUETA_CUENTA,
    sesion::CuentaAutenticada,
    state::AppState,
};

/// Router de la cuenta autenticada (se monta en `/api/v1/yo`).
pub fn rutas() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(perfil))
        .routes(routes!(mis_copropiedades))
        .routes(routes!(mis_tickets))
        .routes(routes!(verificar_correo))
        .routes(routes!(reenviar_verificacion))
        .routes(routes!(cambiar_contrasena))
        .routes(routes!(cerrar_todas))
        .routes(routes!(iniciar_segundo_factor))
        .routes(routes!(confirmar_segundo_factor))
}

/// Perfil de la cuenta.
#[derive(Debug, Serialize, ToSchema)]
pub struct PerfilRespuesta {
    pub id: Uuid,
    pub nombre: String,
    pub usuario: Option<String>,
    pub correo: Option<String>,
    pub correo_verificado: bool,
    pub segundo_factor_activo: bool,
    /// Acceso total a la plataforma y a todas las copropiedades.
    pub es_superusuario: bool,
    /// Permisos de plataforma (vacío para la mayoría de las cuentas).
    #[schema(value_type = Vec<crate::esquemas::Permiso>)]
    pub permisos_plataforma: Vec<Permiso>,
}

/// Perfil de la cuenta autenticada.
#[utoipa::path(
    get,
    path = "",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Perfil", body = PerfilRespuesta),
        (status = UNAUTHORIZED, description = "Sin sesión (`no_autenticado`)", body = ErrorRespuesta),
    )
)]
pub async fn perfil(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
) -> ApiResult<Json<PerfilRespuesta>> {
    let p = uc::mi_perfil(&state.plataforma, &RelojSistema, &cuenta.ctx).await?;
    Ok(Json(PerfilRespuesta {
        id: p.id.as_uuid(),
        nombre: p.nombre_mostrar,
        usuario: p.usuario.map(String::from),
        correo: p.correo.map(String::from),
        correo_verificado: p.correo_verificado,
        segundo_factor_activo: p.segundo_factor_activo,
        es_superusuario: p.es_superusuario,
        permisos_plataforma: p.permisos_plataforma.iter().collect(),
    }))
}

/// Copropiedad donde la cuenta tiene roles vigentes.
#[derive(Debug, Serialize, ToSchema)]
pub struct MembresiaRespuesta {
    /// Va en las rutas `/api/v1/copropiedades/{copropiedad_id}/…`.
    pub copropiedad_id: Uuid,
    #[schema(example = "Conjunto Residencial Alameda")]
    pub nombre: String,
    /// Códigos de los roles vigentes (p. ej. `administrador`, `propietario`).
    pub roles: Vec<String>,
}

/// Copropiedades donde la cuenta tiene roles: el cliente elige en cuál entrar.
#[utoipa::path(
    get,
    path = "/copropiedades",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Copropiedades con roles vigentes", body = Vec<MembresiaRespuesta>),
        (status = UNAUTHORIZED, description = "Sin sesión (`no_autenticado`)", body = ErrorRespuesta),
    )
)]
pub async fn mis_copropiedades(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
) -> ApiResult<Json<Vec<MembresiaRespuesta>>> {
    let lista = uc::mis_copropiedades(&state.plataforma, &cuenta.ctx).await?;
    Ok(Json(
        lista
            .into_iter()
            .map(|m| MembresiaRespuesta {
                copropiedad_id: m.tenant_id.as_uuid(),
                nombre: m.nombre,
                roles: m.roles,
            })
            .collect(),
    ))
}

/// Código de 6 dígitos.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CodigoPeticion {
    #[schema(example = "123456")]
    pub codigo: String,
}

/// Verifica el correo con el código que se envió.
#[utoipa::path(
    post,
    path = "/correo/verificar",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    request_body = CodigoPeticion,
    responses(
        (status = NO_CONTENT, description = "Correo verificado"),
        (status = UNPROCESSABLE_ENTITY, description = "Código inválido o vencido (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn verificar_correo(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
    Json(p): Json<CodigoPeticion>,
) -> ApiResult<StatusCode> {
    uc::verificar_correo(&state.plataforma, &RelojSistema, &cuenta.ctx, &p.codigo).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Vuelve a enviar el código de verificación del correo.
#[utoipa::path(
    post,
    path = "/correo/reenviar",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    responses(
        (status = ACCEPTED, description = "Código enviado"),
        (status = UNPROCESSABLE_ENTITY, description = "Sin correo pendiente de verificar", body = ErrorRespuesta),
    )
)]
pub async fn reenviar_verificacion(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
) -> ApiResult<StatusCode> {
    uc::reenviar_verificacion(&state.plataforma, &RelojSistema, &cuenta.ctx).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Cambio de contraseña.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CambioContrasenaPeticion {
    pub contrasena_actual: String,
    pub contrasena_nueva: String,
}

/// Cambia la contraseña y cierra **todas** las sesiones (hay que volver a entrar).
#[utoipa::path(
    put,
    path = "/contrasena",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    request_body = CambioContrasenaPeticion,
    responses(
        (status = NO_CONTENT, description = "Contraseña cambiada"),
        (status = UNAUTHORIZED, description = "Contraseña actual incorrecta (`credenciales_invalidas`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "Contraseña nueva débil (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn cambiar_contrasena(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
    Json(p): Json<CambioContrasenaPeticion>,
) -> ApiResult<StatusCode> {
    let nueva = ContrasenaNueva::new(&p.contrasena_nueva)?;
    uc::cambiar_contrasena(
        &state.plataforma,
        &state.contrasenas,
        &RelojSistema,
        &cuenta.ctx,
        &p.contrasena_actual,
        &nueva,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sesiones cerradas.
#[derive(Debug, Serialize, ToSchema)]
pub struct SesionesCerradas {
    pub cerradas: u64,
}

/// Cierra la sesión en todos los dispositivos.
#[utoipa::path(
    delete,
    path = "/sesiones",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    responses((status = OK, description = "Sesiones cerradas", body = SesionesCerradas))
)]
pub async fn cerrar_todas(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
) -> ApiResult<Json<SesionesCerradas>> {
    let cerradas =
        uc::cerrar_todas_las_sesiones(&state.plataforma, &RelojSistema, &cuenta.ctx).await?;
    Ok(Json(SesionesCerradas { cerradas }))
}

/// Secreto para la app autenticadora.
#[derive(Debug, Serialize, ToSchema)]
pub struct InscripcionRespuesta {
    /// Para escribirlo a mano en la app.
    pub secreto: String,
    /// Para mostrarlo como código QR.
    pub uri: String,
}

/// Empieza a activar el segundo factor (TOTP). Se confirma con un código de la app.
#[utoipa::path(
    post,
    path = "/segundo-factor",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    responses(
        (status = OK, description = "Secreto generado (pendiente de confirmar)", body = InscripcionRespuesta),
        (status = CONFLICT, description = "Ya está activo (`conflicto`)", body = ErrorRespuesta),
    )
)]
pub async fn iniciar_segundo_factor(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
) -> ApiResult<Json<InscripcionRespuesta>> {
    let i = uc::iniciar_segundo_factor(&state.plataforma, state.totp.as_ref(), &cuenta.ctx).await?;
    Ok(Json(InscripcionRespuesta {
        secreto: i.secreto,
        uri: i.uri,
    }))
}

/// Confirma el segundo factor. Desde el próximo inicio de sesión se pedirá el código.
#[utoipa::path(
    post,
    path = "/segundo-factor/confirmar",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    request_body = CodigoPeticion,
    responses(
        (status = NO_CONTENT, description = "Segundo factor activo"),
        (status = UNPROCESSABLE_ENTITY, description = "Código incorrecto (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn confirmar_segundo_factor(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
    Json(p): Json<CodigoPeticion>,
) -> ApiResult<StatusCode> {
    uc::confirmar_segundo_factor(
        &state.plataforma,
        state.totp.as_ref(),
        &RelojSistema,
        &cuenta.ctx,
        &p.codigo,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Límite de «mis solicitudes».
#[derive(Debug, Default, serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MisTicketsQuery {
    /// 1–100 (20 si se omite).
    pub limite: Option<u32>,
}

/// Solicitud de la cuenta en una de sus copropiedades.
#[derive(Debug, Serialize, ToSchema)]
pub struct TicketDeCuentaRespuesta {
    pub copropiedad_id: Uuid,
    pub copropiedad: String,
    pub ticket_id: Uuid,
    #[schema(example = "PQR-2026-00042")]
    pub numero: String,
    #[schema(value_type = crate::esquemas::TipoTicket)]
    pub tipo: String,
    #[schema(value_type = crate::esquemas::EstadoTicket)]
    pub estado: String,
    pub asunto: String,
    pub creado_en: chrono::DateTime<chrono::Utc>,
    pub actualizado_en: chrono::DateTime<chrono::Utc>,
}

/// Solicitudes (tickets) que la cuenta radicó en todas sus copropiedades, las movidas más
/// recientemente primero. El detalle se consulta dentro de cada copropiedad.
#[utoipa::path(
    get,
    path = "/tickets",
    tag = ETIQUETA_CUENTA,
    security(("bearer" = [])),
    params(MisTicketsQuery),
    responses(
        (status = OK, description = "Solicitudes de la cuenta", body = Vec<TicketDeCuentaRespuesta>),
        (status = UNAUTHORIZED, description = "Sin sesión (`no_autenticado`)", body = ErrorRespuesta),
        (status = UNPROCESSABLE_ENTITY, description = "`limite` fuera de rango (`validacion`)", body = ErrorRespuesta),
    )
)]
pub async fn mis_tickets(
    State(state): State<AppState>,
    cuenta: CuentaAutenticada,
    Query(q): Query<MisTicketsQuery>,
) -> ApiResult<Json<Vec<TicketDeCuentaRespuesta>>> {
    let v = application::casos_uso::tickets::tickets_de_mi_cuenta(
        &state.plataforma,
        &cuenta.ctx,
        q.limite.unwrap_or(20),
    )
    .await?;
    Ok(Json(
        v.into_iter()
            .map(|t| TicketDeCuentaRespuesta {
                copropiedad_id: t.tenant_id.as_uuid(),
                copropiedad: t.copropiedad,
                ticket_id: t.ticket_id.as_uuid(),
                numero: t.numero.to_string(),
                tipo: t.tipo.as_str().into(),
                estado: t.estado.as_str().into(),
                asunto: t.asunto,
                creado_en: t.creado_en,
                actualizado_en: t.actualizado_en,
            })
            .collect(),
    ))
}
