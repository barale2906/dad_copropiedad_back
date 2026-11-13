//! Portal del propietario (plan 14 fase 6): lo que ve de la cartera una cuenta que es titular
//! vigente de un bien privado o su pagador (tenedor). Solo sus bienes: el de otro responde
//! "no encontrado" (no se revela si existe). No exige permisos de cartera; exige el módulo.

use chrono::NaiveDate;
use domain::{
    BienPrivadoId, CuentaCobroId, CuentaId, Modulo, Permiso, contabilidad_ph::mora::TipoCertificado,
};

use super::TxCartera;
use super::liquidacion::pdf_cuenta_cobro_en;
use super::mora::{CertificadoEmitido, emitir_certificado_en};
use super::recaudo::{EstadoCuenta, estado_cuenta_en};
use crate::{
    AppError, AppResult, Contexto,
    casos_uso::{contabilidad::prelude::*, modulos::exigir_modulo_en},
    puertos::{
        GeneradorActas, Reloj, RepositorioCartera as _, UnidadDeTrabajo,
        cartera::{BienCartera, CuentaCobro, ResumenBien, ResumenCartera},
    },
};

/// Abre la transacción de la cuenta del contexto (sin exigir permisos de cartera).
async fn abrir_propio<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<(U::Transaccion, CuentaId)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let cuenta = ctx
        .actor_id
        .ok_or(AppError::SinPermiso(Permiso::CopropiedadDatosVer))?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::ContabilidadPh).await?;
    Ok((tx, cuenta))
}

/// Bienes de los que la cuenta es titular vigente o pagador.
async fn propios<T: TxCartera>(tx: &mut T, cuenta: CuentaId) -> AppResult<Vec<BienCartera>> {
    let titular = tx.bienes_de_cuenta(cuenta).await?;
    let persona = tx.persona_de_cuenta(cuenta).await?;
    Ok(tx
        .bienes_cartera()
        .await?
        .into_iter()
        .filter(|b| {
            titular.contains(&b.id) || (persona.is_some() && b.pagador_persona_id == persona)
        })
        .collect())
}

async fn exigir_propio<T: TxCartera>(
    tx: &mut T,
    cuenta: CuentaId,
    bien: BienPrivadoId,
) -> AppResult<()> {
    if propios(tx, cuenta).await?.iter().any(|b| b.id == bien) {
        Ok(())
    } else {
        Err(AppError::NoEncontrado("bien privado"))
    }
}

/// Bienes de la cuenta (titular o pagador) con sus datos de pago.
///
/// # Errors
///
/// [`AppError::ModuloNoHabilitado`] sin el módulo; [`AppError::SinPermiso`] sin cuenta.
pub async fn mis_bienes<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<BienCartera>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let (mut tx, cuenta) = abrir_propio(unidad, reloj, ctx).await?;
    propios(&mut tx, cuenta).await
}

/// Estado de cuenta de un bien propio.
///
/// # Errors
///
/// Los de [`mis_bienes`]; [`AppError::NoEncontrado`] si el bien no es de la cuenta.
pub async fn mi_estado_cuenta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    corte: Option<NaiveDate>,
) -> AppResult<EstadoCuenta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let (mut tx, cuenta) = abrir_propio(unidad, reloj, ctx).await?;
    exigir_propio(&mut tx, cuenta, bien).await?;
    estado_cuenta_en(
        &mut tx,
        bien,
        corte.unwrap_or_else(|| reloj.ahora().date_naive()),
    )
    .await
}

/// Cuentas de cobro de un bien propio.
///
/// # Errors
///
/// Los de [`mi_estado_cuenta`].
pub async fn mis_cuentas_cobro<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
) -> AppResult<Vec<CuentaCobro>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let (mut tx, cuenta) = abrir_propio(unidad, reloj, ctx).await?;
    exigir_propio(&mut tx, cuenta, bien).await?;
    tx.cuentas_cobro(None, Some(bien)).await
}

/// PDF de una cuenta de cobro de un bien propio.
///
/// # Errors
///
/// Los de [`mi_estado_cuenta`]; [`AppError::Infraestructura`] si falla el PDF.
pub async fn mi_cuenta_cobro_pdf<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CuentaCobroId,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
    G: GeneradorActas,
{
    let (mut tx, cuenta) = abrir_propio(unidad, reloj, ctx).await?;
    let c = tx
        .cuenta_cobro(id)
        .await?
        .ok_or(AppError::NoEncontrado("cuenta de cobro"))?;
    exigir_propio(&mut tx, cuenta, c.bien_privado_id)
        .await
        .map_err(|_| AppError::NoEncontrado("cuenta de cobro"))?;
    pdf_cuenta_cobro_en(&mut tx, generador, reloj, c).await
}

/// El propietario o pagador descarga el paz y salvo de su bien (solo si no debe nada). El
/// código de verificación y el PDF se entregan una sola vez.
///
/// # Errors
///
/// Los de [`mi_estado_cuenta`]; [`AppError::Conflicto`] si el bien tiene saldo.
pub async fn mi_paz_y_salvo<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
) -> AppResult<CertificadoEmitido>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
    G: GeneradorActas,
{
    let (mut tx, cuenta) = abrir_propio(unidad, reloj, ctx).await?;
    exigir_propio(&mut tx, cuenta, bien).await?;
    let emitido = emitir_certificado_en(
        &mut tx,
        generador,
        reloj,
        ctx,
        bien,
        TipoCertificado::PazYSalvo,
    )
    .await?;
    tx.confirmar().await?;
    Ok(emitido)
}

/// Lo que el chat muestra en «Mi saldo».
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaldoChat {
    /// La copropiedad no tiene la cartera contratada.
    SinCartera,
    /// Quien escribe no es propietario ni pagador del bien: no ve el saldo.
    NoAutorizado,
    Resumen(ResumenBien),
}

/// Saldo del bien para el chat de WhatsApp: solo lo ve el propietario o el pagador del bien que
/// escribe (el vínculo lo resolvió el gateway por el teléfono). No exige el puerto completo de
/// la cartera.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn saldo_para_chat<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    persona: domain::PersonaId,
    es_propietario: bool,
) -> AppResult<SaldoChat>
where
    U: UnidadDeTrabajo,
    U::Transaccion: ResumenCartera + crate::puertos::RepositorioModulos,
{
    use crate::puertos::RepositorioModulos as _;
    let mut tx = unidad.iniciar(ctx).await?;
    if !tx
        .modulo_habilitado(Modulo::ContabilidadPh, reloj.ahora())
        .await?
    {
        return Ok(SaldoChat::SinCartera);
    }
    let Some(r) = tx.resumen_bien(bien, reloj.ahora().date_naive()).await? else {
        return Ok(SaldoChat::SinCartera);
    };
    if !es_propietario && r.pagador_persona_id != Some(persona) {
        return Ok(SaldoChat::NoAutorizado);
    }
    Ok(SaldoChat::Resumen(r))
}
