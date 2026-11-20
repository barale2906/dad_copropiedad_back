//! Prestaciones sociales en la liquidación (plan 16 fase 4): qué se paga en cada periodo y con
//! qué base, y las cesantías por consignar del año.
//!
//! En la liquidación de un periodo se agregan:
//! - la **prima** del semestre en junio (periodo que termina el 30) y en diciembre (periodo que
//!   incluye el día 20);
//! - los **intereses** de las cesantías del año anterior en el primer periodo de enero;
//! - al **terminar el contrato** en el periodo: cesantías, intereses y prima proporcionales,
//!   vacaciones pendientes en dinero e indemnización.
//!
//! La base de cada lapso sale de las versiones del contrato y de lo variable ya liquidado en
//! otros periodos del lapso ([`domain::nomina::prestaciones::base_mensual`]). El aprendiz no las
//! causa y el salario integral solo causa vacaciones.

use chrono::{Datelike, Months, NaiveDate};
use domain::{
    AdministradoraId, ContratoLaboralId, EmpleadoId, Permiso,
    calendario::es_dia_habil,
    nomina::{
        CatalogoLaboral, ParametroLaboral as P, TipoSalario,
        liquidacion::{ConceptoNomina, LineaLiquidacion, TipoNovedad, dias_comerciales},
        prestaciones::{
            BaseLapso, Terminacion, cesantias, dias_indemnizacion, dias_vacaciones_pendientes,
            indemnizacion, intereses, linea, prima, vacaciones_pendientes,
        },
    },
};
use rust_decimal::Decimal;

use super::{TxNomina, abrir_nomina, catalogo::catalogo_en};
use crate::{
    AppError, AppResult, Contexto,
    puertos::{
        Reloj, UnidadDeTrabajo,
        nomina::{
            ContratoLaboral, PeriodoNomina, RepositorioLiquidacionNomina as _,
            RepositorioNomina as _, VersionContrato,
        },
    },
};

fn fecha(a: i32, m: u32, d: u32) -> AppResult<NaiveDate> {
    NaiveDate::from_ymd_opt(a, m, d).ok_or_else(|| AppError::Validacion("fecha inválida".into()))
}

/// Versión vigente en la fecha (la de mayor `vigente_desde` hasta ella).
fn vigente(versiones: &[VersionContrato], dia: NaiveDate) -> Option<&VersionContrato> {
    versiones.iter().rfind(|v| v.vigente_desde <= dia)
}

fn salario(v: &VersionContrato) -> Decimal {
    v.salario.map_or(Decimal::ZERO, |s| s.valor())
}

/// Base del lapso `[desde, hasta]` de un contrato.
async fn base_lapso<T: TxNomina>(
    tx: &mut T,
    c: &ContratoLaboral,
    versiones: &[VersionContrato],
    desde: NaiveDate,
    hasta: NaiveDate,
    catalogo: &CatalogoLaboral,
    excluir: Option<domain::PeriodoNominaId>,
) -> AppResult<BaseLapso> {
    let Some(actual) = vigente(versiones, hasta) else {
        return Err(AppError::Validacion(
            "el contrato no tiene versión vigente".into(),
        ));
    };
    let mut no_remunerados = 0;
    for n in tx.novedades(Some(c.id), desde, hasta).await? {
        if n.anulacion.is_none()
            && matches!(
                n.novedad.tipo,
                TipoNovedad::LicenciaNoRemunerada | TipoNovedad::Ausencia | TipoNovedad::Suspension
            )
        {
            no_remunerados +=
                dias_comerciales(n.novedad.desde.max(desde), n.novedad.hasta.min(hasta));
        }
    }
    let dias_lapso = dias_comerciales(desde, hasta);
    let mut ponderado = Decimal::ZERO;
    for (i, v) in versiones.iter().enumerate() {
        let fin = versiones
            .get(i + 1)
            .and_then(|s| s.vigente_desde.pred_opt())
            .unwrap_or(hasta)
            .min(hasta);
        let ini = v.vigente_desde.max(desde);
        if ini <= fin {
            ponderado += salario(v) * Decimal::from(dias_comerciales(ini, fin));
        }
    }
    let hace_tres = hasta.checked_sub_months(Months::new(3)).unwrap_or(desde);
    let cambio_reciente = versiones.windows(2).any(|par| {
        par[1].vigente_desde > hace_tres
            && par[1].vigente_desde <= hasta
            && salario(&par[1]) != salario(&par[0])
    });
    let tope = catalogo.salarios_minimos(catalogo.valor(P::TopeAuxilioTransporte)?)?;
    let auxilio = if actual.tipo_salario == TipoSalario::Ordinario && salario(actual) <= tope {
        catalogo.valor(P::AuxilioTransporte)?
    } else {
        Decimal::ZERO
    };
    Ok(BaseLapso {
        salario_actual: salario(actual),
        salario_promedio: if dias_lapso == 0 {
            salario(actual)
        } else {
            ponderado / Decimal::from(dias_lapso)
        },
        cambio_reciente,
        auxilio,
        variables: tx.variables_liquidadas(c.id, desde, hasta, excluir).await?,
        dias: dias_lapso.saturating_sub(no_remunerados),
    })
}

fn agregar(lineas: &mut Vec<LineaLiquidacion>, l: Option<LineaLiquidacion>) {
    if let Some(l) = l {
        lineas.push(l);
    }
}

/// Saldo de vacaciones de un contrato a una fecha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaldoVacaciones {
    /// Días comerciales trabajados (sin licencias no remuneradas, ausencias ni suspensiones).
    pub dias_trabajados: u32,
    pub causados: Decimal,
    /// Días hábiles disfrutados (novedades `vacaciones`).
    pub disfrutados: Decimal,
    pub pendientes: Decimal,
}

async fn saldo_en<T: TxNomina>(
    tx: &mut T,
    c: &ContratoLaboral,
    versiones: &[VersionContrato],
    fin: NaiveDate,
    catalogo: &CatalogoLaboral,
    excluir: Option<domain::PeriodoNominaId>,
) -> AppResult<SaldoVacaciones> {
    let mut disfrutados = Decimal::ZERO;
    for n in tx.novedades(Some(c.id), c.fecha_inicio, fin).await? {
        if n.anulacion.is_none() && n.novedad.tipo == TipoNovedad::Vacaciones {
            let mut d = n.novedad.desde;
            while d <= n.novedad.hasta.min(fin) {
                if es_dia_habil(d) {
                    disfrutados += Decimal::ONE;
                }
                match d.succ_opt() {
                    Some(s) => d = s,
                    None => break,
                }
            }
        }
    }
    let total = base_lapso(tx, c, versiones, c.fecha_inicio, fin, catalogo, excluir).await?;
    let causados = (Decimal::from(total.dias) * catalogo.valor(P::DiasVacaciones)?
        / Decimal::from(360))
    .round_dp(2);
    Ok(SaldoVacaciones {
        dias_trabajados: total.dias,
        causados,
        disfrutados,
        pendientes: dias_vacaciones_pendientes(total.dias, disfrutados, catalogo)?,
    })
}

/// Saldo de vacaciones del contrato a la fecha (por defecto hoy, o su terminación si es antes).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.ver`; [`AppError::NoEncontrado`];
/// [`domain::DominioError::ReglaLaboral`] si falta un parámetro del catálogo.
pub async fn saldo_vacaciones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    contrato: ContratoLaboralId,
    fecha: Option<NaiveDate>,
) -> AppResult<SaldoVacaciones>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaVer).await?;
    let c = tx
        .contrato(contrato)
        .await?
        .ok_or(AppError::NoEncontrado("contrato"))?;
    let fin = fecha
        .unwrap_or_else(|| reloj.ahora().date_naive())
        .min(c.fecha_terminacion.unwrap_or(NaiveDate::MAX));
    if fin < c.fecha_inicio {
        return Err(AppError::Validacion(
            "la fecha es anterior al inicio del contrato".into(),
        ));
    }
    let catalogo = catalogo_en(&mut tx, fin).await?;
    let versiones = tx.versiones(contrato).await?;
    saldo_en(&mut tx, &c, &versiones, fin, &catalogo, None).await
}

/// Cuánto costaría terminar el contrato en `fecha` con `motivo`: las prestaciones, las
/// vacaciones pendientes y la indemnización (sin el salario del periodo ni guardar nada).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.salarios.ver`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si el contrato ya terminó; [`domain::DominioError::ReglaLaboral`] si
/// la fecha o el motivo no aplican o falta un parámetro del catálogo.
pub async fn simular_terminacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    contrato: ContratoLaboralId,
    fecha: NaiveDate,
    motivo: domain::nomina::MotivoTerminacion,
) -> AppResult<Vec<LineaLiquidacion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaSalariosVer).await?;
    let mut c = tx
        .contrato(contrato)
        .await?
        .ok_or(AppError::NoEncontrado("contrato"))?;
    if c.fecha_terminacion.is_some() {
        return Err(AppError::Conflicto("el contrato ya terminó".into()));
    }
    domain::nomina::validar_terminacion(c.tipo, c.fecha_inicio, c.fecha_fin, fecha, motivo)?;
    c.fecha_terminacion = Some(fecha);
    c.motivo_terminacion = Some(motivo);
    let desde = fecha
        .with_day(1)
        .ok_or_else(|| AppError::Validacion("fecha inválida".into()))?;
    let periodo = PeriodoNomina {
        id: domain::PeriodoNominaId::nuevo(),
        periodicidad: domain::nomina::PeriodicidadPago::Mensual,
        desde,
        hasta: fecha,
        estado: domain::nomina::periodo::EstadoPeriodoNomina::Abierto,
        creado_por: None,
        creado_en: reloj.ahora(),
        liquidado_por: None,
        liquidado_en: None,
        aprobado_por: None,
        aprobado_en: None,
        pagado_por: None,
        pagado_en: None,
        fecha_pago: None,
    };
    let catalogo = catalogo_en(&mut tx, fecha).await?;
    prestaciones_del_periodo(&mut tx, &c, &periodo, &catalogo).await
}

/// Prestaciones que el contrato cobra en el periodo.
pub(crate) async fn prestaciones_del_periodo<T: TxNomina>(
    tx: &mut T,
    c: &ContratoLaboral,
    p: &PeriodoNomina,
    catalogo: &CatalogoLaboral,
) -> AppResult<Vec<LineaLiquidacion>> {
    let versiones = tx.versiones(c.id).await?;
    let fin_contrato = c.fecha_terminacion.unwrap_or(p.hasta).min(p.hasta);
    let Some(ultima) = vigente(&versiones, fin_contrato) else {
        return Ok(Vec::new());
    };
    let tipo_salario = ultima.tipo_salario;
    if tipo_salario == TipoSalario::ApoyoSostenimiento {
        return Ok(Vec::new());
    }
    let ordinario = tipo_salario == TipoSalario::Ordinario;
    let termina = c
        .fecha_terminacion
        .is_some_and(|f| f >= p.desde && f <= p.hasta);
    let anio = p.hasta.year();
    let mut lineas = Vec::new();

    // Prima del semestre.
    let (sem_ini, sem_fin) = if p.hasta.month() <= 6 {
        (fecha(anio, 1, 1)?, fecha(anio, 6, 30)?)
    } else {
        (fecha(anio, 7, 1)?, fecha(anio, 12, 31)?)
    };
    let fecha_pago_prima = p.hasta == fecha(anio, 6, 30)?
        || (p.hasta.month() == 12 && p.desde.day() <= 20 && p.hasta.day() >= 20);
    if ordinario && (termina || fecha_pago_prima) {
        let desde = sem_ini.max(c.fecha_inicio);
        let hasta = if termina { fin_contrato } else { sem_fin };
        if desde <= hasta {
            let b = base_lapso(tx, c, &versiones, desde, hasta, catalogo, Some(p.id)).await?;
            agregar(
                &mut lineas,
                linea(
                    ConceptoNomina::Prima,
                    Decimal::from(b.dias),
                    prima(&b, catalogo)?,
                )?,
            );
        }
    }

    // Intereses de las cesantías del año anterior, en el primer periodo de enero.
    if ordinario && p.desde.month() == 1 && p.desde.day() == 1 && c.fecha_inicio.year() < anio {
        let desde = fecha(anio - 1, 1, 1)?.max(c.fecha_inicio);
        let hasta = fecha(anio - 1, 12, 31)?.min(c.fecha_terminacion.unwrap_or(NaiveDate::MAX));
        if desde <= hasta {
            let b = base_lapso(tx, c, &versiones, desde, hasta, catalogo, Some(p.id)).await?;
            let ces = cesantias(&b, catalogo)?;
            agregar(
                &mut lineas,
                linea(
                    ConceptoNomina::InteresesCesantias,
                    Decimal::from(b.dias),
                    intereses(ces, b.dias, catalogo)?,
                )?,
            );
        }
    }

    if !termina {
        return Ok(lineas);
    }
    let fin = fin_contrato;

    // Cesantías e intereses del año del retiro.
    if ordinario {
        let desde = fecha(fin.year(), 1, 1)?.max(c.fecha_inicio);
        let b = base_lapso(tx, c, &versiones, desde, fin, catalogo, Some(p.id)).await?;
        let ces = cesantias(&b, catalogo)?;
        agregar(
            &mut lineas,
            linea(ConceptoNomina::Cesantias, Decimal::from(b.dias), ces)?,
        );
        agregar(
            &mut lineas,
            linea(
                ConceptoNomina::InteresesCesantias,
                Decimal::from(b.dias),
                intereses(ces, b.dias, catalogo)?,
            )?,
        );
    }

    // Vacaciones pendientes: causadas en todo el contrato menos las disfrutadas (días hábiles).
    let saldo = saldo_en(tx, c, &versiones, fin, catalogo, Some(p.id)).await?;
    let pendientes = saldo.pendientes;
    let ultimo_ano = fin
        .checked_sub_months(Months::new(12))
        .and_then(|d| d.succ_opt())
        .unwrap_or(c.fecha_inicio)
        .max(c.fecha_inicio);
    let b = base_lapso(tx, c, &versiones, ultimo_ano, fin, catalogo, Some(p.id)).await?;
    agregar(
        &mut lineas,
        linea(
            ConceptoNomina::VacacionesCompensadas,
            pendientes,
            vacaciones_pendientes(&b, pendientes),
        )?,
    );

    // Indemnización.
    if let Some(motivo) = c.motivo_terminacion {
        let t = Terminacion {
            tipo: c.tipo,
            motivo,
            inicio: c.fecha_inicio,
            fin_pactado: c.fecha_fin,
            fecha: fin,
            salario: salario(ultima),
        };
        agregar(
            &mut lineas,
            linea(
                ConceptoNomina::Indemnizacion,
                dias_indemnizacion(&t, catalogo)?,
                indemnizacion(&t, catalogo)?,
            )?,
        );
    }
    Ok(lineas)
}

/// Cesantías e intereses de un contrato en un año, para consignarlas al fondo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CesantiasContrato {
    pub empleado_id: EmpleadoId,
    pub contrato_id: ContratoLaboralId,
    pub dias: u32,
    /// Base mensual.
    pub base: Decimal,
    pub cesantias: Decimal,
    pub intereses: Decimal,
    /// Fondo de cesantías de la versión vigente al 31 de diciembre.
    pub fondo_id: Option<AdministradoraId>,
}

/// Cesantías del año por consignar (antes del 14 de febrero) e intereses por pagar (antes del 31
/// de enero) de los contratos con salario ordinario vigentes al cierre del año. Los terminados en
/// el año ya las recibieron en su liquidación. La base no incluye lo variable de diciembre si
/// aún no se liquidó.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.salarios.ver`; [`AppError::ModuloNoHabilitado`];
/// [`domain::DominioError::ReglaLaboral`] si falta un parámetro del catálogo.
pub async fn cesantias_del_ano<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    anio: i32,
) -> AppResult<Vec<CesantiasContrato>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let desde = fecha(anio, 1, 1)?;
    let hasta = fecha(anio, 12, 31)?;
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaSalariosVer).await?;
    let catalogo = catalogo_en(&mut tx, hasta).await?;
    let mut resultado = Vec::new();
    for c in tx.contratos_en(desde, hasta).await? {
        if c.fecha_terminacion.is_some_and(|f| f <= hasta) {
            continue;
        }
        let versiones = tx.versiones(c.id).await?;
        let Some(v) = vigente(&versiones, hasta) else {
            continue;
        };
        if v.tipo_salario != TipoSalario::Ordinario {
            continue;
        }
        let fondo_id = v.cesantias_id;
        let inicio = desde.max(c.fecha_inicio);
        let b = base_lapso(&mut tx, &c, &versiones, inicio, hasta, &catalogo, None).await?;
        let ces = cesantias(&b, &catalogo)?;
        resultado.push(CesantiasContrato {
            empleado_id: c.empleado_id,
            contrato_id: c.id,
            dias: b.dias,
            base: domain::nomina::prestaciones::base_mensual(&b, true).round_dp(2),
            cesantias: ces.round_dp(0),
            intereses: intereses(ces, b.dias, &catalogo)?.round_dp(0),
            fondo_id,
        });
    }
    Ok(resultado)
}
