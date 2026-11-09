//! Dispositivos de la app móvil (Flutter) de la cuenta autenticada: la app registra su token de
//! FCM al iniciar sesión y lo retira al cerrarla. Con un dispositivo activo, los avisos de
//! portería salen por push en lugar de WhatsApp (invariante 3).

use domain::{CuentaId, DispositivoPushId, PlataformaPush, copropiedad::texto_opcional};

use crate::{
    AppError, AppResult, ContextoPlataforma,
    puertos::{
        Reloj, Transaccion, UnidadDeTrabajoPlataforma,
        acceso::{DispositivoPush, RepositorioDispositivosPush},
    },
};

/// Largo admitido de un token de FCM.
const LARGO_TOKEN: std::ops::RangeInclusive<usize> = 20..=4096;

fn cuenta(ctx: &ContextoPlataforma) -> AppResult<CuentaId> {
    ctx.actor_id.ok_or(AppError::NoAutenticado)
}

/// Registra (o reactiva) el dispositivo de la cuenta.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] sin cuenta; [`AppError::Validacion`] si el token no tiene el
/// largo de un token de FCM; [`AppError::Dominio`] si el nombre es inválido.
pub async fn registrar_dispositivo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    plataforma: PlataformaPush,
    token: &str,
    nombre: Option<&str>,
) -> AppResult<DispositivoPush>
where
    U: UnidadDeTrabajoPlataforma,
    U::Transaccion: RepositorioDispositivosPush,
{
    let cuenta = cuenta(ctx)?;
    let token = token.trim();
    if !LARGO_TOKEN.contains(&token.len()) || token.chars().any(char::is_whitespace) {
        return Err(AppError::Validacion(
            "token de notificaciones inválido".into(),
        ));
    }
    let nombre = texto_opcional("nombre", nombre, 80)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let d = tx
        .registrar_dispositivo_push(&DispositivoPush {
            id: DispositivoPushId::nuevo(),
            cuenta_id: cuenta,
            plataforma,
            token: token.to_owned(),
            nombre,
            activo: true,
            registrado_en: reloj.ahora(),
        })
        .await?;
    tx.confirmar().await?;
    Ok(d)
}

/// Dispositivos activos de la cuenta.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] sin cuenta.
pub async fn listar_dispositivos<U>(
    unidad: &U,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<DispositivoPush>>
where
    U: UnidadDeTrabajoPlataforma,
    U::Transaccion: RepositorioDispositivosPush,
{
    let cuenta = cuenta(ctx)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.dispositivos_push(cuenta).await
}

/// Retira un dispositivo de la cuenta (al cerrar sesión en la app).
///
/// # Errors
///
/// [`AppError::NoAutenticado`] sin cuenta; [`AppError::NoEncontrado`] si no es suyo o ya se
/// retiró.
pub async fn retirar_dispositivo<U>(
    unidad: &U,
    ctx: &ContextoPlataforma,
    id: DispositivoPushId,
) -> AppResult<()>
where
    U: UnidadDeTrabajoPlataforma,
    U::Transaccion: RepositorioDispositivosPush,
{
    let cuenta = cuenta(ctx)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    if !tx.retirar_dispositivo_push(cuenta, id).await? {
        return Err(AppError::NoEncontrado("dispositivo"));
    }
    tx.confirmar().await
}
