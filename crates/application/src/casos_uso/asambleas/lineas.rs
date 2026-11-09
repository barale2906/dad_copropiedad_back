//! Líneas de WhatsApp y tarifas de Meta (plataforma).
//!
//! Cualquier línea atiende cualquier copropiedad. Varias líneas no suben el límite de
//! clientes por día (es del portafolio de negocio), pero sí suman rendimiento (80 mps por
//! línea), resiliencia (calidad propia) y **franquicia** de mensajes de servicio gratuitos
//! (1 000 al mes por número desde el 1-oct-2026). Las tarifas cambian cada trimestre: se
//! editan aquí y el valor exacto sale de la factura de Meta.

use chrono::NaiveDate;
use domain::Permiso;
use rust_decimal::Decimal;

use super::ACCION_LINEA;
use crate::{
    AppError, AppResult, ContextoPlataforma,
    auditoria::{DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, Transaccion,
        asambleas::{EstadoLinea, LineaWhatsapp, RepositorioLineasWhatsapp, TarifaWhatsapp},
        identidad::UnidadDeTrabajoPlataforma,
    },
};

/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.whatsapp.gestionar`.
pub async fn listar_lineas<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<LineaWhatsapp>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioLineasWhatsapp + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaWhatsappGestionar)?;
    unidad.iniciar_plataforma(ctx).await?.lineas().await
}

/// Registra una línea nueva (ya verificada en Meta).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::Validacion`] si faltan datos o el límite no es
/// positivo; [`AppError::Conflicto`] si el `phone_number_id` ya existe.
pub async fn crear_linea<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    phone_number_id: &str,
    numero_visible: &str,
    nombre: &str,
) -> AppResult<LineaWhatsapp>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioLineasWhatsapp + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaWhatsappGestionar)?;
    let linea = LineaWhatsapp {
        id: uuid::Uuid::now_v7(),
        phone_number_id: super::texto("phone_number_id", phone_number_id, 64)?,
        numero_visible: super::texto("numero_visible", numero_visible, 32)?,
        nombre: super::texto("nombre", nombre, 100)?,
        limite_mps: 80,
        estado: EstadoLinea::Activa,
        calidad: None,
        franquicia_servicio_mes: 1000,
        servicio_mes: 0,
        plantillas_mes: 0,
    };
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.insertar_linea(&linea).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_LINEA,
        "linea_whatsapp",
        linea.id,
    ))
    .await?;
    tx.confirmar().await?;
    Ok(linea)
}

/// Cambia el estado, el ritmo y la franquicia de una línea.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Validacion`].
pub async fn actualizar_linea<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    id: uuid::Uuid,
    estado: EstadoLinea,
    limite_mps: u32,
    franquicia: u32,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioLineasWhatsapp + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaWhatsappGestionar)?;
    if !(1..=1000).contains(&limite_mps) {
        return Err(AppError::Validacion("`limite_mps` va de 1 a 1000".into()));
    }
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    if !tx
        .actualizar_linea(id, estado, limite_mps, franquicia)
        .await?
    {
        return Err(AppError::NoEncontrado("línea de WhatsApp"));
    }
    tx.registrar(
        EventoAuditoria::ok(ACCION_LINEA, "linea_whatsapp", id)
            .con_detalle(DetalleAuditoria::vacio().con("estado", estado.as_str())?),
    )
    .await?;
    tx.confirmar().await
}

/// # Errors
///
/// [`AppError::SinPermiso`].
pub async fn listar_tarifas<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<TarifaWhatsapp>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioLineasWhatsapp + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaWhatsappGestionar)?;
    unidad.iniciar_plataforma(ctx).await?.tarifas().await
}

/// Registra la tarifa vigente desde una fecha (Meta publica tablas nuevas cada trimestre).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::Validacion`] si el país, la moneda o la categoría
/// no son válidos o el valor es negativo.
pub async fn guardar_tarifa<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    pais: &str,
    categoria: &str,
    moneda: &str,
    valor: Decimal,
    vigente_desde: NaiveDate,
) -> AppResult<TarifaWhatsapp>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioLineasWhatsapp + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaWhatsappGestionar)?;
    let mayusculas = |v: &str, n: usize| v.len() == n && v.chars().all(|c| c.is_ascii_uppercase());
    if !mayusculas(pais, 2) || !mayusculas(moneda, 3) {
        return Err(AppError::Validacion(
            "país ISO de 2 letras y moneda ISO de 3".into(),
        ));
    }
    if !matches!(
        categoria,
        "utilidad" | "servicio" | "marketing" | "autenticacion"
    ) {
        return Err(AppError::Validacion("categoría desconocida".into()));
    }
    if valor < Decimal::ZERO {
        return Err(AppError::Validacion(
            "el valor no puede ser negativo".into(),
        ));
    }
    let t = TarifaWhatsapp {
        pais: pais.into(),
        categoria: categoria.into(),
        moneda: moneda.into(),
        valor,
        vigente_desde,
    };
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.guardar_tarifa(&t).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_LINEA,
        "tarifa_whatsapp",
        uuid::Uuid::nil(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(t)
}
