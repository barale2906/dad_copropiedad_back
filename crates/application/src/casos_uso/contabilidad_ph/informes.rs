//! Informes para el consejo y la asamblea (plan 14 fase 7): estado del fondo de imprevistos,
//! flujo de caja mensual, proyecto de presupuesto del año siguiente y el paquete en PDF para la
//! asamblea ordinaria (ejecución presupuestal, cartera por edades y fondo). La ejecución y las
//! edades están en [`super::presupuesto`] y [`super::mora`].

use chrono::{Datelike, NaiveDate};
use domain::{
    Permiso, PresupuestoId, RubroPresupuestoId,
    contabilidad::Monto,
    contabilidad_ph::{
        EstadoPresupuesto, TipoConceptoCobro, fondo_llego_a_la_mitad, valores_mensuales,
    },
};
use rust_decimal::{Decimal, RoundingStrategy};

use super::{TxCartera, abrir, mora, presupuesto, validacion};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::contabilidad::prelude::*,
    puertos::{
        GeneradorActas, Reloj, RepositorioCartera as _, UnidadDeTrabajo,
        actas::TablaPdf,
        cartera::{FiltroObligaciones, Presupuesto, RubroPresupuesto},
        contabilidad::AgrupacionSaldos,
    },
};

const ACCION_PROYECTO: AccionAuditoria = AccionAuditoria::new("cartera.presupuesto.proyectar");

/// Estado del fondo de imprevistos (Ley 675 art. 35).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstadoFondo {
    pub anio: i32,
    /// Liquidado al fondo (vigente, todos los años).
    pub liquidado: Decimal,
    /// Recaudado de lo liquidado al fondo.
    pub recaudado: Decimal,
    /// Presupuesto ordinario anual vigente del año (cero si no hay).
    pub presupuesto_anual: Decimal,
    /// Liquidado / presupuesto, en %.
    pub porcentaje: Decimal,
    /// Llegó a la mitad del presupuesto: la asamblea puede suspender los aportes.
    pub llego_a_la_mitad: bool,
    pub porcentaje_politica: Decimal,
    pub suspendido: bool,
}

/// Estado del fondo de imprevistos frente al presupuesto ordinario del año.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`; [`AppError::Validacion`] sin política vigente.
pub async fn estado_fondo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    anio: Option<i32>,
) -> AppResult<EstadoFondo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let hoy = reloj.ahora().date_naive();
    let anio = anio.unwrap_or_else(|| hoy.year());
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
    let politica = super::configuracion::politica_en(&mut tx, hoy).await?;
    let (mut liquidado, mut recaudado) = (Decimal::ZERO, Decimal::ZERO);
    for o in tx
        .saldos_obligaciones(&FiltroObligaciones::default())
        .await?
        .into_iter()
        .filter(|o| o.concepto_tipo == TipoConceptoCobro::FondoImprevistos)
        .filter(|o| o.obligacion.estado == domain::contabilidad_ph::EstadoObligacion::Vigente)
    {
        liquidado += o.obligacion.valor.valor() - o.ajustes;
        recaudado += o.aplicado;
    }
    let presupuesto_anual: Decimal = tx
        .presupuesto_vigente(anio)
        .await?
        .map(|p| p.rubros.iter().map(RubroPresupuesto::valor_anual).sum())
        .unwrap_or_default();
    let porcentaje = if presupuesto_anual > Decimal::ZERO {
        (liquidado * Decimal::ONE_HUNDRED / presupuesto_anual).round_dp(2)
    } else {
        Decimal::ZERO
    };
    Ok(EstadoFondo {
        anio,
        liquidado,
        recaudado,
        presupuesto_anual,
        porcentaje,
        llego_a_la_mitad: fondo_llego_a_la_mitad(liquidado, presupuesto_anual),
        porcentaje_politica: politica.parametros.porcentaje_fondo,
        suspendido: politica.parametros.fondo_suspendido,
    })
}

/// Entradas y salidas de dinero de un mes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MesFlujo {
    /// Primer día del mes.
    pub mes: NaiveDate,
    pub saldo_inicial: Decimal,
    pub entradas: Decimal,
    pub salidas: Decimal,
    pub saldo_final: Decimal,
}

fn fin_de_mes(mes: NaiveDate) -> Option<NaiveDate> {
    mes.checked_add_months(chrono::Months::new(1))?.pred_opt()
}

/// Flujo de caja mensual de las cuentas de tesorería (bancos, cajas y fondos) según el mayor,
/// entre dos meses.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`; [`AppError::Validacion`] si el rango es inválido
/// o pasa de 24 meses.
pub async fn flujo_caja<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<Vec<MesFlujo>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let desde = domain::contabilidad_ph::periodo_de(desde);
    let hasta = domain::contabilidad_ph::periodo_de(hasta);
    let meses = domain::contabilidad_ph::meses_entre(desde, hasta);
    if !(0..24).contains(&meses) {
        return Err(validacion("el flujo de caja va de 1 a 24 meses"));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
    let cuentas: Vec<_> = tx
        .cuentas_tesoreria()
        .await?
        .into_iter()
        .map(|c| c.cuenta_contable_id)
        .collect();
    let mut flujo = Vec::new();
    let mut mes = desde;
    while mes <= hasta {
        let fin = fin_de_mes(mes).ok_or_else(|| validacion("mes fuera de rango"))?;
        let filas: Vec<_> = tx
            .saldos(mes, fin, AgrupacionSaldos::default())
            .await?
            .into_iter()
            .filter(|f| cuentas.contains(&f.cuenta_id))
            .collect();
        let saldo_inicial: Decimal = filas.iter().map(|f| f.saldo_inicial).sum();
        let entradas: Decimal = filas.iter().map(|f| f.debitos).sum();
        let salidas: Decimal = filas.iter().map(|f| f.creditos).sum();
        flujo.push(MesFlujo {
            mes,
            saldo_inicial,
            entradas,
            salidas,
            saldo_final: saldo_inicial + entradas - salidas,
        });
        mes = mes
            .checked_add_months(chrono::Months::new(1))
            .ok_or_else(|| validacion("mes fuera de rango"))?;
    }
    Ok(flujo)
}

/// Proyecto de presupuesto del año siguiente: copia el vigente del año con un ajuste porcentual
/// (redondeado al peso) como borrador nuevo. La asamblea lo aprueba después.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `presupuesto.gestionar`; [`AppError::Validacion`] si no hay
/// presupuesto vigente del año o el ajuste no está entre −50 % y 100 %.
pub async fn proyectar_presupuesto<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    anio_base: i32,
    ajuste_porcentual: Decimal,
) -> AppResult<Presupuesto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    if ajuste_porcentual < Decimal::from(-50) || ajuste_porcentual > Decimal::ONE_HUNDRED {
        return Err(validacion("el ajuste está entre -50 % y 100 %"));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::PresupuestoGestionar).await?;
    let base = tx.presupuesto_vigente(anio_base).await?.ok_or_else(|| {
        validacion(format!(
            "no hay un presupuesto vigente de {anio_base} para proyectar"
        ))
    })?;
    let factor = Decimal::ONE + ajuste_porcentual / Decimal::ONE_HUNDRED;
    let mut rubros = Vec::with_capacity(base.rubros.len());
    for r in &base.rubros {
        let valores: Vec<Monto> = r
            .valores_mensuales
            .iter()
            .map(|v| {
                Monto::new(
                    (v.valor() * factor)
                        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero),
                )
            })
            .collect::<Result<_, _>>()?;
        rubros.push(RubroPresupuesto {
            id: RubroPresupuestoId::nuevo(),
            valores_mensuales: valores_mensuales(&valores)?,
            ..r.clone()
        });
    }
    let anio = anio_base + 1;
    let p = Presupuesto {
        id: PresupuestoId::nuevo(),
        anio,
        version: tx.ultima_version_presupuesto(anio).await? + 1,
        estado: EstadoPresupuesto::Borrador,
        descripcion: Some(format!(
            "Proyecto desde el presupuesto {anio_base} v{} con ajuste de {ajuste_porcentual} %",
            base.version
        )),
        pregunta_id: None,
        vigente_desde: None,
        aprobado_por: None,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
        rubros,
    };
    tx.insertar_presupuesto(&p).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("anio", anio)?
        .con("base", base.id.to_string())?
        .con("ajuste", ajuste_porcentual.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PROYECTO, "presupuesto", p.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Paquete en PDF para la asamblea ordinaria: ejecución presupuestal del año hasta el mes, cartera
/// por edades al corte y estado del fondo de imprevistos, en una sola tabla por secciones.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`; [`AppError::Validacion`] sin presupuesto vigente
/// del año o sin política; [`AppError::Infraestructura`] si falla el PDF.
pub async fn informe_asamblea_pdf<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    anio: i32,
    hasta_mes: u32,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
    G: GeneradorActas,
{
    let (vigente, entidad) = {
        let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
        (
            tx.presupuesto_vigente(anio).await?,
            tx.datos_entidad().await?,
        )
    };
    let vigente = vigente.ok_or_else(|| {
        validacion(format!(
            "no hay un presupuesto vigente de {anio} para el informe"
        ))
    })?;
    let ejecucion =
        presupuesto::ejecucion_presupuestal(unidad, reloj, ctx, vigente.id, hasta_mes).await?;
    let corte = NaiveDate::from_ymd_opt(anio, hasta_mes, 1)
        .and_then(fin_de_mes)
        .ok_or(AppError::Validacion("mes inválido".into()))?;
    let (_, edades) = mora::cartera_por_edades(unidad, reloj, ctx, Some(corte)).await?;
    let fondo = estado_fondo(unidad, reloj, ctx, Some(anio)).await?;
    let mut filas: Vec<Vec<String>> = Vec::new();
    let txt = |d: Decimal| d.round_dp(2).to_string();
    for r in &ejecucion.rubros {
        filas.push(vec![
            "Ejecución presupuestal".into(),
            format!("{} ({})", r.nombre, r.cuenta_codigo),
            txt(r.presupuestado),
            txt(r.ejecutado),
        ]);
    }
    for (nombre, valor) in [
        ("Corriente", edades.corriente),
        ("1 a 30 días", edades.de_1_a_30),
        ("31 a 60 días", edades.de_31_a_60),
        ("61 a 90 días", edades.de_61_a_90),
        ("Más de 90 días", edades.mas_de_90),
        ("Total cartera", edades.total),
    ] {
        filas.push(vec![
            "Cartera por edades".into(),
            nombre.into(),
            txt(valor),
            String::new(),
        ]);
    }
    filas.push(vec![
        "Fondo de imprevistos".into(),
        format!("Liquidado ({} % del presupuesto)", fondo.porcentaje),
        txt(fondo.liquidado),
        txt(fondo.recaudado),
    ]);
    generador
        .tabla(&TablaPdf {
            titulo: format!("Informe para la asamblea {anio}"),
            copropiedad: match &entidad.nit {
                Some(nit) => format!("{} · NIT {nit}", entidad.nombre),
                None => entidad.nombre,
            },
            subtitulo: format!(
                "Ejecución de enero a {} · Cartera al {corte} · Fondo: liquidado y recaudado",
                corte.format("%m/%Y")
            ),
            columnas: [
                "Sección",
                "Concepto",
                "Presupuestado / valor",
                "Ejecutado / recaudado",
            ]
            .map(str::to_owned)
            .to_vec(),
            filas,
            generada: reloj.ahora().format("%Y-%m-%d %H:%M UTC").to_string(),
        })
        .await
}
