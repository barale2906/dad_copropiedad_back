//! Periodos contables: estados (`EstadoPeriodo::transicionar`) y reaperturas. Reabrir un periodo
//! cerrado exige una solicitud con motivo que **aprueba otra cuenta** (segregación de
//! funciones, QA-018); todo queda auditado.

use domain::{
    PeriodoContableId, Permiso, ReaperturaPeriodoId,
    contabilidad::{EstadoPeriodo, EstadoReapertura, PeriodoContable},
};

use super::prelude::*;
use super::{TxContable, exigir_contabilidad, texto};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        Reloj, UnidadDeTrabajo,
        contabilidad::{Periodo, Reapertura},
    },
};

const ACCION_ESTADO: AccionAuditoria = AccionAuditoria::new("contabilidad.periodo.estado");
const ACCION_SOLICITAR: AccionAuditoria = AccionAuditoria::new("contabilidad.reapertura.solicitar");
const ACCION_DECIDIR: AccionAuditoria = AccionAuditoria::new("contabilidad.reapertura.decidir");

/// Periodos del año (o todos).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn periodos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    anio: Option<i32>,
) -> AppResult<Vec<Periodo>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    ctx.exigir(Permiso::ContabilidadVer)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_contabilidad(&mut tx, reloj).await?;
    tx.periodos(anio).await
}

/// Abre (crea) el periodo si no existe; si existe, lo devuelve como está.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.periodos.cerrar`.
pub async fn abrir_periodo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    periodo: PeriodoContable,
) -> AppResult<Periodo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    ctx.exigir(Permiso::ContabilidadPeriodosCerrar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_contabilidad(&mut tx, reloj).await?;
    let p = tx.periodo_o_crear(periodo).await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Pasa el periodo a revisión, lo cierra o lo devuelve a abierto desde revisión. Para reabrir
/// uno **cerrado** se usa [`solicitar_reapertura`] + [`decidir_reapertura`].
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.periodos.cerrar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si se intenta reabrir un cerrado por aquí;
/// [`domain::DominioError::TransicionInvalida`].
pub async fn cambiar_estado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PeriodoContableId,
    hacia: EstadoPeriodo,
) -> AppResult<Periodo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    ctx.exigir(Permiso::ContabilidadPeriodosCerrar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_contabilidad(&mut tx, reloj).await?;
    let actual = tx
        .periodo(id)
        .await?
        .ok_or(AppError::NoEncontrado("periodo"))?;
    if actual.estado == EstadoPeriodo::Cerrado {
        return Err(AppError::Conflicto(
            "un periodo cerrado se reabre con una solicitud de reapertura aprobada".into(),
        ));
    }
    let estado = actual.estado.transicionar(hacia)?;
    let cerrado = estado == EstadoPeriodo::Cerrado;
    let p = Periodo {
        estado,
        cerrado_por: if cerrado { ctx.actor_id } else { None },
        cerrado_en: cerrado.then(|| reloj.ahora()),
        ..actual
    };
    tx.cambiar_estado_periodo(&p).await?;
    auditar_estado(&mut tx, &p).await?;
    tx.confirmar().await?;
    Ok(p)
}

async fn auditar_estado<T: TxContable>(tx: &mut T, p: &Periodo) -> AppResult<()> {
    let detalle = DetalleAuditoria::vacio()
        .con("anio", p.periodo.anio)?
        .con("mes", p.periodo.mes)?
        .con("estado", p.estado.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ESTADO, "periodo_contable", p.id.as_uuid()).con_detalle(detalle),
    )
    .await
}

/// Reaperturas de un periodo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn reaperturas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    periodo: PeriodoContableId,
) -> AppResult<Vec<Reapertura>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    ctx.exigir(Permiso::ContabilidadVer)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_contabilidad(&mut tx, reloj).await?;
    tx.reaperturas(periodo).await
}

/// Solicita reabrir un periodo cerrado.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.periodos.reabrir`; [`AppError::Conflicto`] si el
/// periodo no está cerrado o ya hay una solicitud pendiente; [`AppError::Validacion`] sin motivo.
pub async fn solicitar_reapertura<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    periodo: PeriodoContableId,
    motivo: &str,
    alcance: Option<&str>,
) -> AppResult<Reapertura>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    ctx.exigir(Permiso::ContabilidadPeriodosReabrir)?;
    let solicitante = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    let motivo = texto("motivo", motivo, 1000)?;
    if motivo.chars().count() < 5 {
        return Err(AppError::Validacion(
            "explique el motivo de la reapertura".into(),
        ));
    }
    let alcance = alcance.map(|a| texto("alcance", a, 1000)).transpose()?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_contabilidad(&mut tx, reloj).await?;
    let p = tx
        .periodo(periodo)
        .await?
        .ok_or(AppError::NoEncontrado("periodo"))?;
    if p.estado != EstadoPeriodo::Cerrado {
        return Err(AppError::Conflicto(
            "solo se reabre un periodo cerrado".into(),
        ));
    }
    let r = Reapertura {
        id: ReaperturaPeriodoId::nuevo(),
        periodo_id: periodo,
        motivo,
        alcance,
        solicitada_por: solicitante,
        solicitada_en: reloj.ahora(),
        estado: EstadoReapertura::Pendiente,
        decidida_por: None,
        decidida_en: None,
    };
    tx.insertar_reapertura(&r).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_SOLICITAR,
        "reapertura_periodo",
        r.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Aprueba o rechaza una reapertura pendiente. Al aprobarla, el periodo vuelve a `abierto`.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.periodos.aprobar_reapertura`;
/// [`AppError::Conflicto`] si quien decide es quien la solicitó o ya no está pendiente.
pub async fn decidir_reapertura<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReaperturaPeriodoId,
    aprobar: bool,
) -> AppResult<Reapertura>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    ctx.exigir(Permiso::ContabilidadPeriodosAprobarReapertura)?;
    let quien = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_contabilidad(&mut tx, reloj).await?;
    let actual = tx
        .reapertura(id)
        .await?
        .ok_or(AppError::NoEncontrado("reapertura"))?;
    if actual.estado != EstadoReapertura::Pendiente {
        return Err(AppError::Conflicto("la reapertura ya se decidió".into()));
    }
    if actual.solicitada_por == quien {
        return Err(AppError::Conflicto(
            "quien solicita la reapertura no puede aprobarla (segregación de funciones)".into(),
        ));
    }
    let r = Reapertura {
        estado: if aprobar {
            EstadoReapertura::Aprobada
        } else {
            EstadoReapertura::Rechazada
        },
        decidida_por: Some(quien),
        decidida_en: Some(reloj.ahora()),
        ..actual
    };
    tx.decidir_reapertura(&r).await?;
    if aprobar {
        let p = tx
            .periodo(r.periodo_id)
            .await?
            .ok_or(AppError::NoEncontrado("periodo"))?;
        let abierto = Periodo {
            estado: p.estado.transicionar(EstadoPeriodo::Abierto)?,
            cerrado_por: None,
            cerrado_en: None,
            ..p
        };
        tx.cambiar_estado_periodo(&abierto).await?;
        auditar_estado(&mut tx, &abierto).await?;
    }
    let detalle = DetalleAuditoria::vacio().con("aprobada", aprobar)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_DECIDIR, "reapertura_periodo", r.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(r)
}
