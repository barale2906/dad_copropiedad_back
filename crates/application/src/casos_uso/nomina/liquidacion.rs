//! Liquidación de la nómina (plan 16 fase 2): configuración de la entidad, periodos (mes o
//! quincena), liquidación de cada contrato con [`domain::nomina::liquidacion::liquidar`],
//! aprobación por **otra cuenta** y pago.
//!
//! Mientras el periodo no esté aprobado se puede liquidar de nuevo (reemplaza todo). Aprobado,
//! nada cambia: ni las liquidaciones ni las novedades de sus fechas (la corrección será la nómina
//! de ajuste). La retención de la nómina quincenal se calcula sobre el mes completo en la
//! segunda quincena, con lo liquidado en la primera.

use chrono::{Datelike, Days, NaiveDate};
use domain::{
    LiquidacionNominaId, PeriodoNominaId, Permiso,
    nomina::{
        PeriodicidadPago,
        liquidacion::{
            AcumuladoMes, EntradaLiquidacion, Normativa, Retencion, TablaLaboral, TramoContrato,
            liquidar,
        },
        periodo::{EstadoPeriodoNomina, fin_de_periodo},
    },
};

use super::{
    TxNomina, abrir_nomina,
    catalogo::catalogo_en,
    contabilizacion::{contabilizar_aprobacion, contabilizar_pago},
    prestaciones::prestaciones_del_periodo,
};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::contabilidad::TxContable,
    puertos::{
        RegistroAuditoria as _, Reloj, Transaccion, UnidadDeTrabajo,
        nomina::{
            ConfiguracionNomina, ContratoLaboral, LiquidacionContrato, PeriodoNomina,
            RepositorioLiquidacionNomina as _, RepositorioNomina as _,
        },
    },
};

const ACCION_CONFIGURAR: AccionAuditoria = AccionAuditoria::new("nomina.configuracion.guardar");
const ACCION_PERIODO: AccionAuditoria = AccionAuditoria::new("nomina.periodo.crear");
const ACCION_LIQUIDAR: AccionAuditoria = AccionAuditoria::new("nomina.periodo.liquidar");
const ACCION_APROBAR: AccionAuditoria = AccionAuditoria::new("nomina.periodo.aprobar");
const ACCION_PAGAR: AccionAuditoria = AccionAuditoria::new("nomina.periodo.pagar");

/// Periodos que se listan como máximo.
pub const LIMITE_PERIODOS: u32 = 120;

fn regla(texto: impl Into<String>) -> AppError {
    domain::DominioError::ReglaLaboral(texto.into()).into()
}

// ---------------------------------------------------------------------------------------------
// Configuración
// ---------------------------------------------------------------------------------------------

/// Configuración de la nómina de la entidad (la predeterminada si nunca se guardó).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.ver`; [`AppError::ModuloNoHabilitado`].
pub async fn configuracion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<ConfiguracionNomina>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaVer).await?;
    Ok(tx
        .configuracion()
        .await?
        .unwrap_or_else(|| ConfiguracionNomina {
            exonerada_aportes: false,
            actualizada_por: None,
            actualizada_en: reloj.ahora(),
        }))
}

/// Guarda la configuración (hoy, si aplica la exoneración del art. 114-1 del ET: la deciden el
/// contador y el revisor fiscal según la situación tributaria de la entidad).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.configurar`; [`AppError::ModuloNoHabilitado`].
pub async fn guardar_configuracion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    exonerada_aportes: bool,
) -> AppResult<ConfiguracionNomina>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaConfigurar).await?;
    let c = ConfiguracionNomina {
        exonerada_aportes,
        actualizada_por: ctx.actor_id,
        actualizada_en: reloj.ahora(),
    };
    tx.guardar_configuracion(&c).await?;
    let detalle = DetalleAuditoria::vacio().con("exonerada_aportes", exonerada_aportes)?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_CONFIGURAR,
            "configuracion_nomina",
            ctx.tenant_id.as_uuid(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

// ---------------------------------------------------------------------------------------------
// Periodos
// ---------------------------------------------------------------------------------------------

/// Abre el periodo que empieza en `desde` (el día 1, o el 16 en la quincenal).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.liquidar`; [`domain::DominioError::ReglaLaboral`] si
/// `desde` no inicia un periodo; [`AppError::Conflicto`] si ya existe.
pub async fn crear_periodo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    periodicidad: PeriodicidadPago,
    desde: NaiveDate,
) -> AppResult<PeriodoNomina>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let hasta = fin_de_periodo(periodicidad, desde)?;
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaLiquidar).await?;
    let p = PeriodoNomina {
        id: PeriodoNominaId::nuevo(),
        periodicidad,
        desde,
        hasta,
        estado: EstadoPeriodoNomina::Abierto,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
        liquidado_por: None,
        liquidado_en: None,
        aprobado_por: None,
        aprobado_en: None,
        pagado_por: None,
        pagado_en: None,
        fecha_pago: None,
    };
    tx.insertar_periodo(&p).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("periodicidad", periodicidad.as_str())?
        .con("desde", desde.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PERIODO, "periodo_nomina", p.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Periodos de la entidad, el más reciente primero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.ver`; [`AppError::ModuloNoHabilitado`].
pub async fn periodos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<PeriodoNomina>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaVer).await?;
    tx.periodos(LIMITE_PERIODOS).await
}

/// Periodo con sus liquidaciones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodoLiquidado {
    pub periodo: PeriodoNomina,
    pub liquidaciones: Vec<LiquidacionContrato>,
}

/// Periodo con sus liquidaciones (datos personales: exige también `nomina.salarios.ver`).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.salarios.ver`; [`AppError::NoEncontrado`].
pub async fn periodo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PeriodoNominaId,
) -> AppResult<PeriodoLiquidado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaSalariosVer).await?;
    let periodo = tx
        .periodo(id)
        .await?
        .ok_or(AppError::NoEncontrado("periodo de nómina"))?;
    let liquidaciones = tx.liquidaciones(id).await?;
    Ok(PeriodoLiquidado {
        periodo,
        liquidaciones,
    })
}

async fn periodo_para<T: TxNomina>(tx: &mut T, id: PeriodoNominaId) -> AppResult<PeriodoNomina> {
    tx.periodo(id)
        .await?
        .ok_or(AppError::NoEncontrado("periodo de nómina"))
}

// ---------------------------------------------------------------------------------------------
// Liquidación
// ---------------------------------------------------------------------------------------------

/// Normativa vigente al cierre del periodo: catálogo, tablas y UVT del año.
async fn normativa<T: TxNomina>(tx: &mut T, fecha: NaiveDate) -> AppResult<Normativa> {
    let catalogo = catalogo_en(tx, fecha).await?;
    let mut tablas = tx.tablas(Some(fecha)).await?;
    let mut tabla = |t: TablaLaboral| {
        tablas
            .iter()
            .position(|v| v.tabla == t)
            .map(|i| tablas.swap_remove(i).tramos)
            .ok_or_else(|| {
                regla(format!(
                    "el catálogo laboral no tiene la tabla `{t}` vigente"
                ))
            })
    };
    let retencion = tabla(TablaLaboral::RetencionSalarios)?;
    let fondo_solidaridad = tabla(TablaLaboral::FondoSolidaridad)?;
    let uvt = tx
        .uvt(fecha.year())
        .await?
        .ok_or_else(|| regla(format!("el catálogo no tiene la UVT de {}", fecha.year())))?;
    Ok(Normativa {
        catalogo,
        retencion,
        fondo_solidaridad,
        uvt,
    })
}

/// Tramos del contrato dentro del periodo, cada uno con su versión vigente. `None` si el
/// contrato no se paga con la periodicidad del periodo.
async fn tramos<T: TxNomina>(
    tx: &mut T,
    c: &ContratoLaboral,
    p: &PeriodoNomina,
) -> AppResult<Option<Vec<TramoContrato>>> {
    let fin_contrato = c.fecha_terminacion.or(c.fecha_fin).unwrap_or(p.hasta);
    let desde = c.fecha_inicio.max(p.desde);
    let hasta = fin_contrato.min(p.hasta);
    if desde > hasta {
        return Ok(None);
    }
    let versiones = tx.versiones(c.id).await?;
    let mut tramos = Vec::new();
    for (i, v) in versiones.iter().enumerate() {
        let fin_version = versiones
            .get(i + 1)
            .and_then(|s| s.vigente_desde.checked_sub_days(Days::new(1)))
            .unwrap_or(hasta);
        let d = v.vigente_desde.max(desde);
        let h = fin_version.min(hasta);
        if d > h {
            continue;
        }
        let salario = v
            .salario
            .ok_or_else(|| AppError::Validacion("la versión no trae el salario".into()))?;
        tramos.push((
            v.periodicidad,
            TramoContrato {
                desde: d,
                hasta: h,
                salario,
                tipo_salario: v.tipo_salario,
                etapa_aprendizaje: v.etapa_aprendizaje,
                jornada: v.jornada,
                horas_semanales: v.horas_semanales,
                pensionado: v.pensionado,
                clase_riesgo: v.clase_riesgo,
            },
        ));
    }
    match tramos.last() {
        Some((periodicidad, _)) if *periodicidad == p.periodicidad => {
            Ok(Some(tramos.into_iter().map(|(_, t)| t).collect()))
        }
        _ => Ok(None),
    }
}

/// Retención del periodo: en la mensual y en la segunda quincena cierra el mes (con lo
/// liquidado en la primera quincena para ese contrato).
fn retencion(p: &PeriodoNomina, previas: &[LiquidacionContrato], c: &ContratoLaboral) -> Retencion {
    match (p.periodicidad, p.desde.day()) {
        (PeriodicidadPago::Quincenal, 1) => Retencion::NoAplica,
        (PeriodicidadPago::Quincenal, _) => Retencion::CierreMes(
            previas
                .iter()
                .find(|l| l.contrato_id == c.id)
                .map(|l| l.liquidacion.acumulado)
                .unwrap_or_default(),
        ),
        (PeriodicidadPago::Mensual, _) => Retencion::CierreMes(AcumuladoMes::default()),
    }
}

/// Liquida (o vuelve a liquidar) todos los contratos del periodo con su periodicidad.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `nomina.liquidar`; [`AppError::NoEncontrado`].
/// - [`domain::DominioError::TransicionInvalida`] si el periodo ya fue aprobado.
/// - [`domain::DominioError::ReglaLaboral`] si falta un parámetro, una tabla o la UVT vigentes,
///   o un contrato no se puede liquidar (el mensaje dice el código del empleado).
pub async fn liquidar_periodo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PeriodoNominaId,
) -> AppResult<PeriodoLiquidado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaLiquidar).await?;
    let mut p = periodo_para(&mut tx, id).await?;
    p.estado = p.estado.transicionar(EstadoPeriodoNomina::Liquidado)?;
    let normativa = normativa(&mut tx, p.hasta).await?;
    let exonerada = tx
        .configuracion()
        .await?
        .is_some_and(|c| c.exonerada_aportes);
    let previas = match (p.periodicidad, p.desde.day()) {
        (PeriodicidadPago::Quincenal, 16) => {
            match tx
                .periodo_por_inicio(
                    PeriodicidadPago::Quincenal,
                    p.desde.with_day(1).unwrap_or(p.desde),
                )
                .await?
            {
                Some(primera) => tx.liquidaciones(primera.id).await?,
                None => Vec::new(),
            }
        }
        _ => Vec::new(),
    };
    let novedades = tx.novedades(None, p.desde, p.hasta).await?;
    let ahora = reloj.ahora();
    let mut liquidaciones = Vec::new();
    for c in tx.contratos_en(p.desde, p.hasta).await? {
        let Some(tramos) = tramos(&mut tx, &c, &p).await? else {
            continue;
        };
        let entrada = EntradaLiquidacion {
            desde: p.desde,
            hasta: p.hasta,
            tramos,
            novedades: novedades
                .iter()
                .filter(|n| n.contrato_id == c.id && n.anulacion.is_none())
                .map(|n| n.novedad)
                .collect(),
            exonerada,
            retencion: retencion(&p, &previas, &c),
            prestaciones: prestaciones_del_periodo(&mut tx, &c, &p, &normativa.catalogo).await?,
            deducciones: super::deducciones::vigentes_en(&mut tx, c.empleado_id, p.hasta).await?,
        };
        let liquidacion = match liquidar(&entrada, &normativa) {
            Ok(l) => l,
            Err(domain::DominioError::ReglaLaboral(m)) => {
                let codigo = tx
                    .empleado(c.empleado_id)
                    .await?
                    .map(|e| e.codigo.as_str().to_owned())
                    .unwrap_or_default();
                return Err(regla(format!("empleado {codigo}: {m}")));
            }
            Err(otro) => return Err(otro.into()),
        };
        liquidaciones.push(LiquidacionContrato {
            id: LiquidacionNominaId::nuevo(),
            periodo_id: p.id,
            contrato_id: c.id,
            empleado_id: c.empleado_id,
            liquidacion,
            creado_en: ahora,
        });
    }
    tx.reemplazar_liquidaciones(p.id, &liquidaciones).await?;
    p.liquidado_por = ctx.actor_id;
    p.liquidado_en = Some(ahora);
    tx.actualizar_periodo(&p).await?;
    let detalle = DetalleAuditoria::vacio().con("liquidaciones", liquidaciones.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_LIQUIDAR, "periodo_nomina", p.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(PeriodoLiquidado {
        periodo: p,
        liquidaciones,
    })
}

/// Aprueba la liquidación. La aprueba **otra cuenta** distinta de la que liquidó, y solo si no
/// hubo novedades después de liquidar. Con contabilidad, contabiliza cada liquidación
/// (`nomina.liquidacion`) en la misma transacción.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `nomina.aprobar`; [`AppError::NoEncontrado`].
/// - [`domain::DominioError::TransicionInvalida`] si no está liquidado.
/// - [`AppError::Conflicto`] si la aprueba quien la liquidó o hay novedades posteriores.
pub async fn aprobar_periodo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PeriodoNominaId,
) -> AppResult<PeriodoNomina>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina + TxContable,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaAprobar).await?;
    let mut p = periodo_para(&mut tx, id).await?;
    p.estado = p.estado.transicionar(EstadoPeriodoNomina::Aprobado)?;
    if ctx.actor_id.is_none() || ctx.actor_id == p.liquidado_por {
        return Err(AppError::Conflicto(
            "la nómina la aprueba una cuenta distinta de la que la liquidó".into(),
        ));
    }
    if tx
        .ultimo_cambio_novedades(p.desde, p.hasta)
        .await?
        .is_some_and(|cambio| p.liquidado_en.is_none_or(|l| cambio > l))
    {
        return Err(AppError::Conflicto(
            "hay novedades registradas o anuladas después de liquidar: liquide de nuevo".into(),
        ));
    }
    p.aprobado_por = ctx.actor_id;
    p.aprobado_en = Some(reloj.ahora());
    tx.actualizar_periodo(&p).await?;
    let liquidaciones = tx.liquidaciones(p.id).await?;
    contabilizar_aprobacion(&mut tx, reloj, ctx, &p, &liquidaciones).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_APROBAR,
        "periodo_nomina",
        p.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Marca el periodo aprobado como pagado en `fecha_pago` (la dispersión la hace el banco). Con
/// contabilidad, contabiliza el pago (`nomina.pago`).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.liquidar`; [`AppError::NoEncontrado`];
/// [`domain::DominioError::TransicionInvalida`] si no está aprobado.
pub async fn marcar_pagado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PeriodoNominaId,
    fecha_pago: NaiveDate,
) -> AppResult<PeriodoNomina>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina + TxContable,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaLiquidar).await?;
    let mut p = periodo_para(&mut tx, id).await?;
    p.estado = p.estado.transicionar(EstadoPeriodoNomina::Pagado)?;
    p.pagado_por = ctx.actor_id;
    p.pagado_en = Some(reloj.ahora());
    p.fecha_pago = Some(fecha_pago);
    tx.actualizar_periodo(&p).await?;
    let liquidaciones = tx.liquidaciones(p.id).await?;
    contabilizar_pago(&mut tx, reloj, ctx, &p, &liquidaciones, fecha_pago).await?;
    let detalle = DetalleAuditoria::vacio().con("fecha_pago", fecha_pago.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PAGAR, "periodo_nomina", p.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(p)
}
