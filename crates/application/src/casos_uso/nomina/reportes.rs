//! Reportes de la nómina (plan 16 fases 5 y 7) con lo **aprobado o pagado**:
//!
//! - [`resumen_aportes`]: aportes del mes por empleado y por administradora (EPS, pensiones, ARL,
//!   caja) más ICBF y SENA: el insumo para diligenciar la PILA en el operador. El archivo plano
//!   llega cuando se confirme su versión vigente.
//! - [`ingresos_y_retenciones`]: totales del año por empleado en los grupos del certificado de
//!   ingresos y retenciones por rentas de trabajo (insumo del formulario 220).
//!
//! Ambos exigen `nomina.salarios.ver`.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{
    AdministradoraId, ContratoLaboralId, Documento, EmpleadoId, Permiso,
    nomina::{
        ClaseRiesgo,
        liquidacion::ConceptoNomina,
        periodo::EstadoPeriodoNomina,
        reportes::{GrupoCertificado, tipo_cotizante},
    },
};
use rust_decimal::Decimal;

use super::{TxNomina, abrir_nomina};
use crate::{
    AppError, AppResult, Contexto,
    puertos::{
        Reloj, UnidadDeTrabajo,
        nomina::{LiquidacionContrato, RepositorioNomina as _},
        personas::RepositorioPersonas as _,
    },
};

/// Periodos que se revisan como máximo (diez años de quincenas).
const LIMITE_PERIODOS: u32 = 240;

/// Liquidaciones aprobadas o pagadas de los periodos que terminan entre las fechas.
async fn aprobadas<T: TxNomina>(
    tx: &mut T,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<Vec<LiquidacionContrato>> {
    let mut v = Vec::new();
    for p in tx.periodos(LIMITE_PERIODOS).await? {
        if p.hasta >= desde
            && p.hasta <= hasta
            && matches!(
                p.estado,
                EstadoPeriodoNomina::Aprobado | EstadoPeriodoNomina::Pagado
            )
        {
            v.extend(tx.liquidaciones(p.id).await?);
        }
    }
    Ok(v)
}

fn mes(anio: i32, mes: u32) -> AppResult<(NaiveDate, NaiveDate)> {
    let desde = NaiveDate::from_ymd_opt(anio, mes, 1)
        .ok_or_else(|| AppError::Validacion("mes inválido".into()))?;
    let hasta = desde
        .checked_add_months(chrono::Months::new(1))
        .and_then(|d| d.pred_opt())
        .ok_or_else(|| AppError::Validacion("mes inválido".into()))?;
    Ok((desde, hasta))
}

/// Aportes del mes de un empleado (un contrato).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AportesEmpleado {
    pub empleado_id: EmpleadoId,
    pub contrato_id: ContratoLaboralId,
    pub codigo: String,
    /// Dato personal.
    pub documento: Option<Documento>,
    /// Tipo de cotizante de la PILA (`01`, `12`, `19`).
    pub tipo_cotizante: &'static str,
    pub dias: u32,
    pub ibc: Decimal,
    /// Salud del empleado y del empleador.
    pub salud: Decimal,
    /// Pensión del empleado y del empleador.
    pub pension: Decimal,
    pub fondo_solidaridad: Decimal,
    pub arl: Decimal,
    pub caja: Decimal,
    pub icbf: Decimal,
    pub sena: Decimal,
    pub eps_id: Option<AdministradoraId>,
    pub afp_id: Option<AdministradoraId>,
    pub arl_id: Option<AdministradoraId>,
    pub ccf_id: Option<AdministradoraId>,
    pub clase_riesgo: Option<ClaseRiesgo>,
}

/// Total del mes a una administradora.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotalAdministradora {
    pub administradora_id: AdministradoraId,
    pub valor: Decimal,
}

/// Aportes del mes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumenAportes {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    pub empleados: Vec<AportesEmpleado>,
    pub administradoras: Vec<TotalAdministradora>,
    pub icbf: Decimal,
    pub sena: Decimal,
}

/// Lo liquidado al contrato en el mes: empleado, días, IBC y líneas.
type Acumulado = (EmpleadoId, u32, Decimal, Vec<(ConceptoNomina, Decimal)>);

/// Aportes a la seguridad social y parafiscales del mes (suma las dos quincenas).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.salarios.ver`; [`AppError::Validacion`] si el mes no
/// existe.
pub async fn resumen_aportes<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    anio: i32,
    numero_mes: u32,
) -> AppResult<ResumenAportes>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let (desde, hasta) = mes(anio, numero_mes)?;
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaSalariosVer).await?;
    let mut por_contrato: BTreeMap<ContratoLaboralId, Acumulado> = BTreeMap::new();
    for l in aprobadas(&mut tx, desde, hasta).await? {
        let e = por_contrato.entry(l.contrato_id).or_insert((
            l.empleado_id,
            0,
            Decimal::ZERO,
            Vec::new(),
        ));
        e.1 += l.liquidacion.dias;
        e.2 += l.liquidacion.ibc.valor();
        e.3.extend(
            l.liquidacion
                .lineas
                .iter()
                .map(|x| (x.concepto, x.valor.valor())),
        );
    }
    let mut empleados = Vec::new();
    let mut por_administradora: BTreeMap<AdministradoraId, Decimal> = BTreeMap::new();
    let (mut icbf_total, mut sena_total) = (Decimal::ZERO, Decimal::ZERO);
    for (contrato_id, (empleado_id, dias, ibc, lineas)) in por_contrato {
        let suma = |cs: &[ConceptoNomina]| -> Decimal {
            lineas
                .iter()
                .filter(|(c, _)| cs.contains(c))
                .map(|(_, v)| *v)
                .sum()
        };
        use ConceptoNomina as C;
        let contrato = tx
            .contrato(contrato_id)
            .await?
            .ok_or(AppError::NoEncontrado("contrato"))?;
        let version = tx
            .versiones(contrato_id)
            .await?
            .into_iter()
            .rfind(|v| v.vigente_desde <= hasta);
        let empleado = tx
            .empleado(empleado_id)
            .await?
            .ok_or(AppError::NoEncontrado("empleado"))?;
        let documento = tx
            .persona(empleado.persona_id)
            .await?
            .and_then(|p| p.documento);
        let fila = AportesEmpleado {
            empleado_id,
            contrato_id,
            codigo: empleado.codigo.as_str().to_owned(),
            documento,
            tipo_cotizante: tipo_cotizante(
                contrato.tipo,
                version.as_ref().and_then(|v| v.etapa_aprendizaje),
            ),
            dias: dias.min(30),
            ibc,
            salud: suma(&[C::Salud, C::SaludEmpleador]),
            pension: suma(&[C::Pension, C::PensionEmpleador]),
            fondo_solidaridad: suma(&[C::FondoSolidaridad]),
            arl: suma(&[C::Arl]),
            caja: suma(&[C::CajaCompensacion]),
            icbf: suma(&[C::Icbf]),
            sena: suma(&[C::Sena]),
            eps_id: version.as_ref().map(|v| v.eps_id),
            afp_id: version.as_ref().and_then(|v| v.afp_id),
            arl_id: version.as_ref().and_then(|v| v.arl_id),
            ccf_id: version.as_ref().and_then(|v| v.ccf_id),
            clase_riesgo: version.as_ref().and_then(|v| v.clase_riesgo),
        };
        for (adm, valor) in [
            (fila.eps_id, fila.salud),
            (fila.afp_id, fila.pension + fila.fondo_solidaridad),
            (fila.arl_id, fila.arl),
            (fila.ccf_id, fila.caja),
        ] {
            if let Some(a) = adm
                && !valor.is_zero()
            {
                *por_administradora.entry(a).or_default() += valor;
            }
        }
        icbf_total += fila.icbf;
        sena_total += fila.sena;
        empleados.push(fila);
    }
    Ok(ResumenAportes {
        desde,
        hasta,
        empleados,
        administradoras: por_administradora
            .into_iter()
            .map(|(administradora_id, valor)| TotalAdministradora {
                administradora_id,
                valor,
            })
            .collect(),
        icbf: icbf_total,
        sena: sena_total,
    })
}

/// Totales del año de un empleado por grupo del certificado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngresosEmpleado {
    pub empleado_id: EmpleadoId,
    pub codigo: String,
    /// Dato personal.
    pub nombre: String,
    /// Dato personal.
    pub documento: Option<Documento>,
    pub totales: BTreeMap<GrupoCertificado, Decimal>,
}

/// Ingresos y retenciones del año por empleado (de los periodos aprobados o pagados que
/// terminan en el año).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.salarios.ver`; [`AppError::Validacion`] si el año no es
/// válido.
pub async fn ingresos_y_retenciones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    anio: i32,
) -> AppResult<Vec<IngresosEmpleado>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let (desde, _) = mes(anio, 1)?;
    let (_, hasta) = mes(anio, 12)?;
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaSalariosVer).await?;
    let mut por_empleado: BTreeMap<EmpleadoId, BTreeMap<GrupoCertificado, Decimal>> =
        BTreeMap::new();
    for l in aprobadas(&mut tx, desde, hasta).await? {
        let totales = por_empleado.entry(l.empleado_id).or_default();
        for x in &l.liquidacion.lineas {
            if let Some(g) = GrupoCertificado::de(x.concepto) {
                *totales.entry(g).or_default() += x.valor.valor();
            }
        }
    }
    let mut v = Vec::new();
    for (empleado_id, totales) in por_empleado {
        let empleado = tx
            .empleado(empleado_id)
            .await?
            .ok_or(AppError::NoEncontrado("empleado"))?;
        let persona = tx
            .persona(empleado.persona_id)
            .await?
            .ok_or(AppError::NoEncontrado("persona"))?;
        v.push(IngresosEmpleado {
            empleado_id,
            codigo: empleado.codigo.as_str().to_owned(),
            nombre: persona.nombre,
            documento: persona.documento,
            totales,
        });
    }
    Ok(v)
}
