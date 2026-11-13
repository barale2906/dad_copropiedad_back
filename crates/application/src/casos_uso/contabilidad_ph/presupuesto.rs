//! Presupuesto de la PH (plan 14 fase 2): versiones por año, rubros mensuales por concepto de
//! cobro y cuenta, aprobación con la pregunta de la asamblea y ejecución presupuestal contra el
//! mayor (un reporte, sin tabla).

use chrono::{Datelike, NaiveDate};
use domain::{
    CentroCostoId, ConceptoCobroId, CuentaContableId, Permiso, PreguntaId, PresupuestoId,
    RubroPresupuestoId,
    contabilidad::{Monto, Naturaleza},
    contabilidad_ph::{EstadoPresupuesto, TipoConceptoCobro, valores_mensuales},
};
use rust_decimal::Decimal;

use super::{TxCartera, abrir, validacion};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::contabilidad::{prelude::*, texto, texto_opcional},
    puertos::{
        Reloj, RepositorioCartera as _, UnidadDeTrabajo,
        cartera::{Presupuesto, RubroPresupuesto},
        contabilidad::AgrupacionSaldos,
    },
};

const ACCION_CREAR: AccionAuditoria = AccionAuditoria::new("cartera.presupuesto.crear");
const ACCION_RUBROS: AccionAuditoria = AccionAuditoria::new("cartera.presupuesto.rubros");
const ACCION_APROBAR: AccionAuditoria = AccionAuditoria::new("cartera.presupuesto.aprobar");

/// Rubro de un presupuesto.
#[derive(Debug, Clone)]
pub struct NuevoRubro {
    /// Concepto ordinario que lo financia (se reparte por coeficiente, módulo o área).
    pub concepto: ConceptoCobroId,
    /// Cuenta de gasto (o de ingreso) contra la que se mide la ejecución.
    pub cuenta: CuentaContableId,
    pub centro_costo: Option<CentroCostoId>,
    pub nombre: String,
    /// Enero a diciembre.
    pub valores_mensuales: Vec<Monto>,
}

/// Presupuestos del año (o todos), del más reciente al más antiguo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn presupuestos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    anio: Option<i32>,
) -> AppResult<Vec<Presupuesto>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    abrir(unidad, reloj, ctx, Permiso::CarteraVer)
        .await?
        .presupuestos(anio)
        .await
}

/// Presupuesto por id, con sus rubros.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`; [`AppError::NoEncontrado`].
pub async fn presupuesto<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PresupuestoId,
) -> AppResult<Presupuesto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    abrir(unidad, reloj, ctx, Permiso::CarteraVer)
        .await?
        .presupuesto(id)
        .await?
        .ok_or(AppError::NoEncontrado("presupuesto"))
}

/// Valida los rubros: conceptos ordinarios que reparten un total, cuentas de movimiento activas
/// y doce valores.
async fn armar_rubros<T: TxCartera>(
    tx: &mut T,
    rubros: Vec<NuevoRubro>,
) -> AppResult<Vec<RubroPresupuesto>> {
    let conceptos = tx.conceptos_cobro().await?;
    let ids: Vec<CuentaContableId> = rubros.iter().map(|r| r.cuenta).collect();
    let cuentas = tx.cuentas_por_id(&ids).await?;
    let mut armados = Vec::with_capacity(rubros.len());
    for (i, r) in rubros.into_iter().enumerate() {
        let concepto = conceptos
            .iter()
            .find(|c| c.id == r.concepto)
            .ok_or(AppError::NoEncontrado("concepto de cobro"))?;
        if concepto.tipo != TipoConceptoCobro::Ordinaria || !concepto.criterio.reparte_total() {
            return Err(validacion(format!(
                "el rubro {} debe financiar un concepto ordinario que se reparta por \
                 coeficiente, módulo o área",
                i + 1
            )));
        }
        let cuenta = cuentas
            .iter()
            .find(|c| c.id == r.cuenta)
            .ok_or(AppError::NoEncontrado("cuenta contable"))?;
        if !(cuenta.acepta_movimiento && cuenta.activa) {
            return Err(validacion(format!(
                "la cuenta {} del rubro {} no es de movimiento o está inactiva",
                cuenta.codigo.as_str(),
                i + 1
            )));
        }
        armados.push(RubroPresupuesto {
            id: RubroPresupuestoId::nuevo(),
            concepto_cobro_id: r.concepto,
            cuenta_contable_id: r.cuenta,
            centro_costo_id: r.centro_costo,
            nombre: texto("nombre", &r.nombre, 150)?,
            valores_mensuales: valores_mensuales(&r.valores_mensuales)?,
        });
    }
    Ok(armados)
}

/// Crea un presupuesto en borrador (siguiente versión del año) con sus rubros.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `presupuesto.gestionar`; [`AppError::Validacion`] si el año o
/// un rubro no son válidos; [`AppError::NoEncontrado`] si un concepto o una cuenta no existen.
pub async fn crear_presupuesto<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    anio: i32,
    descripcion: Option<&str>,
    rubros: Vec<NuevoRubro>,
) -> AppResult<Presupuesto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    if !(2000..=2100).contains(&anio) {
        return Err(validacion("el año del presupuesto no es válido"));
    }
    let descripcion = texto_opcional("descripcion", descripcion, 300)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::PresupuestoGestionar).await?;
    let rubros = armar_rubros(&mut tx, rubros).await?;
    let p = Presupuesto {
        id: PresupuestoId::nuevo(),
        anio,
        version: tx.ultima_version_presupuesto(anio).await? + 1,
        estado: EstadoPresupuesto::Borrador,
        descripcion,
        pregunta_id: None,
        vigente_desde: None,
        aprobado_por: None,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
        rubros,
    };
    tx.insertar_presupuesto(&p).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("anio", p.anio)?
        .con("version", p.version)?
        .con("rubros", p.rubros.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR, "presupuesto", p.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Reemplaza los rubros de un presupuesto en borrador.
///
/// # Errors
///
/// Los de [`crear_presupuesto`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si ya no es
/// un borrador.
pub async fn reemplazar_rubros<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PresupuestoId,
    rubros: Vec<NuevoRubro>,
) -> AppResult<Presupuesto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::PresupuestoGestionar).await?;
    let mut p = tx
        .presupuesto(id)
        .await?
        .ok_or(AppError::NoEncontrado("presupuesto"))?;
    if p.estado != EstadoPresupuesto::Borrador {
        return Err(AppError::Conflicto(
            "el presupuesto ya fue aprobado: cree una versión nueva".into(),
        ));
    }
    p.rubros = armar_rubros(&mut tx, rubros).await?;
    tx.reemplazar_rubros_presupuesto(id, &p.rubros).await?;
    let detalle = DetalleAuditoria::vacio().con("rubros", p.rubros.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RUBROS, "presupuesto", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Aprueba el borrador (decisión de la asamblea, con su pregunta): pasa a vigente desde la fecha
/// dada y la versión vigente anterior del año queda reemplazada. Las liquidaciones ya hechas no
/// cambian.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `presupuesto.gestionar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si no es un borrador; [`AppError::Validacion`] si no tiene rubros o la
/// fecha no es del año del presupuesto.
pub async fn aprobar_presupuesto<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PresupuestoId,
    vigente_desde: NaiveDate,
    pregunta: Option<PreguntaId>,
) -> AppResult<Presupuesto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::PresupuestoGestionar).await?;
    let mut p = tx
        .presupuesto(id)
        .await?
        .ok_or(AppError::NoEncontrado("presupuesto"))?;
    p.estado = p.estado.transicionar(EstadoPresupuesto::Vigente)?;
    if p.rubros.is_empty() {
        return Err(validacion("un presupuesto sin rubros no se aprueba"));
    }
    if vigente_desde.year() != p.anio {
        return Err(validacion(
            "la vigencia debe empezar en el año del presupuesto",
        ));
    }
    let anterior = tx.presupuesto_vigente(p.anio).await?.map(|a| a.id);
    p.vigente_desde = Some(vigente_desde);
    p.pregunta_id = pregunta;
    p.aprobado_por = ctx.actor_id;
    tx.aprobar_presupuesto(&p, anterior).await?;
    let mut detalle = DetalleAuditoria::vacio()
        .con("anio", p.anio)?
        .con("version", p.version)?
        .con("vigente_desde", vigente_desde.to_string())?;
    if let Some(a) = anterior {
        detalle = detalle.con("reemplaza", a.to_string())?;
    }
    tx.registrar(
        EventoAuditoria::ok(ACCION_APROBAR, "presupuesto", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Ejecución de un rubro hasta un mes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EjecucionRubro {
    pub rubro_id: RubroPresupuestoId,
    pub nombre: String,
    pub cuenta_id: CuentaContableId,
    pub cuenta_codigo: String,
    pub centro_costo_id: Option<CentroCostoId>,
    /// Lo presupuestado de enero al mes del corte.
    pub presupuestado: Decimal,
    /// Movimiento neto de la cuenta (y del centro) en el mismo lapso, según su naturaleza.
    pub ejecutado: Decimal,
    /// Presupuestado − ejecutado.
    pub diferencia: Decimal,
    pub presupuesto_anual: Decimal,
}

/// Ejecución presupuestal: lo presupuestado contra el mayor, de enero al mes indicado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EjecucionPresupuestal {
    pub presupuesto: Presupuesto,
    pub hasta_mes: u32,
    pub rubros: Vec<EjecucionRubro>,
}

/// Ejecución presupuestal de un presupuesto hasta un mes (1 a 12). Si dos rubros usan la misma
/// cuenta sin centro de costo, cada uno muestra el movimiento de la cuenta completo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si el mes no está entre 1 y 12.
pub async fn ejecucion_presupuestal<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PresupuestoId,
    hasta_mes: u32,
) -> AppResult<EjecucionPresupuestal>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    if !(1..=12).contains(&hasta_mes) {
        return Err(validacion("el mes está entre 1 y 12"));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
    let p = tx
        .presupuesto(id)
        .await?
        .ok_or(AppError::NoEncontrado("presupuesto"))?;
    let desde = NaiveDate::from_ymd_opt(p.anio, 1, 1)
        .ok_or_else(|| validacion("el año del presupuesto no es válido"))?;
    let hasta = if hasta_mes == 12 {
        NaiveDate::from_ymd_opt(p.anio, 12, 31)
    } else {
        NaiveDate::from_ymd_opt(p.anio, hasta_mes + 1, 1).and_then(|d| d.pred_opt())
    }
    .ok_or_else(|| validacion("el mes no es válido"))?;
    let saldos = tx
        .saldos(
            desde,
            hasta,
            AgrupacionSaldos {
                por_centro_costo: true,
                sin_cierre_anual: true,
                ..AgrupacionSaldos::default()
            },
        )
        .await?;
    let ids: Vec<CuentaContableId> = p.rubros.iter().map(|r| r.cuenta_contable_id).collect();
    let cuentas = tx.cuentas_por_id(&ids).await?;
    let rubros = p
        .rubros
        .iter()
        .map(|r| {
            let filas: Vec<_> = saldos
                .iter()
                .filter(|s| {
                    s.cuenta_id == r.cuenta_contable_id
                        && (r.centro_costo_id.is_none() || s.centro_costo_id == r.centro_costo_id)
                })
                .collect();
            let ejecutado: Decimal = filas
                .iter()
                .map(|s| match s.naturaleza {
                    Naturaleza::Debito => s.debitos - s.creditos,
                    Naturaleza::Credito => s.creditos - s.debitos,
                })
                .sum();
            let presupuestado: Decimal = r.valores_mensuales[..hasta_mes as usize]
                .iter()
                .map(|v| v.valor())
                .sum();
            EjecucionRubro {
                rubro_id: r.id,
                nombre: r.nombre.clone(),
                cuenta_id: r.cuenta_contable_id,
                cuenta_codigo: cuentas
                    .iter()
                    .find(|c| c.id == r.cuenta_contable_id)
                    .map(|c| c.codigo.as_str().to_owned())
                    .unwrap_or_default(),
                centro_costo_id: r.centro_costo_id,
                presupuestado,
                ejecutado,
                diferencia: presupuestado - ejecutado,
                presupuesto_anual: r.valor_anual(),
            }
        })
        .collect();
    Ok(EjecucionPresupuestal {
        presupuesto: p,
        hasta_mes,
        rubros,
    })
}
