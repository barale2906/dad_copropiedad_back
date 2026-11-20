//! Contabilización de la nómina (plan 16 fase 7), **solo si la entidad lleva contabilidad**: la
//! nómina no escribe asientos, entrega un documento origen al contabilizador del núcleo
//! ([`contabilizar_documento`]) en la misma transacción. Las cuentas las elige el contador en la
//! regla de cada evento; aquí no hay cuentas.
//!
//! - [`eventos::LIQUIDACION`], al **aprobar** el periodo: un comprobante por liquidación (tercero:
//!   la persona del empleado). Conceptos: cada [`ConceptoNomina`] por su texto (`basico`,
//!   `salud`, `salud_empleador`, `provision_prima`…) y [`conceptos::NETO`]. Cuadra porque los
//!   devengos son las deducciones más el neto, y los aportes y provisiones llevan su débito y su
//!   crédito en la misma regla.
//! - [`eventos::PAGO`], al marcar **pagado**: [`conceptos::NETO`] (débito) y
//!   [`conceptos::GIRADO`] (crédito).
//!
//! El centro de costo de la versión vigente del contrato va en cada importe de la liquidación (la
//! regla lo usa en las líneas con centro `documento`). Contabilizar es idempotente por origen (un
//! documento, un comprobante).

use chrono::NaiveDate;
use domain::{Modulo, contabilidad::ImporteDocumento, nomina::liquidacion::ConceptoNomina};

use super::TxNomina;
use crate::{
    AppResult, Contexto,
    casos_uso::contabilidad::{
        TxContable,
        comprobantes::{DocumentoOrigen, contabilizar_documento},
    },
    puertos::{
        Reloj,
        contabilidad::Origen,
        nomina::{LiquidacionContrato, PeriodoNomina, RepositorioLiquidacionNomina as _},
    },
};

/// Eventos de las reglas de contabilización de la nómina (y tipo de origen del comprobante).
pub mod eventos {
    /// Liquidación aprobada de un contrato en un periodo.
    pub const LIQUIDACION: &str = "nomina.liquidacion";
    /// Pago de la liquidación.
    pub const PAGO: &str = "nomina.pago";
}

/// Conceptos propios de la nómina además de los de [`ConceptoNomina`].
pub mod conceptos {
    /// Neto a pagar al empleado (crédito al liquidar; débito al pagar).
    pub const NETO: &str = "neto";
    /// Pago: dinero que sale (crédito a la cuenta del banco).
    pub const GIRADO: &str = "girado";
}

/// Conceptos que una regla de `nomina.liquidacion` puede necesitar (para configurarla).
pub fn conceptos_de_liquidacion() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = ConceptoNomina::TODOS.iter().map(|c| c.as_str()).collect();
    v.push(conceptos::NETO);
    v
}

/// ¿La entidad lleva contabilidad ahora?
async fn con_contabilidad<T: TxNomina>(tx: &mut T, reloj: &impl Reloj) -> AppResult<bool> {
    tx.modulo_habilitado(Modulo::Contabilidad, reloj.ahora())
        .await
}

async fn persona_de<T: TxNomina>(
    tx: &mut T,
    l: &LiquidacionContrato,
) -> AppResult<Option<domain::PersonaId>> {
    Ok(tx.empleado(l.empleado_id).await?.map(|e| e.persona_id))
}

fn importe(
    concepto: &str,
    monto: domain::contabilidad::Monto,
    persona: Option<domain::PersonaId>,
    centro: Option<domain::CentroCostoId>,
) -> ImporteDocumento {
    ImporteDocumento {
        persona,
        centro_costo: centro,
        ..ImporteDocumento::new(concepto, monto)
    }
}

/// Centro de costo de la versión vigente al cierre del periodo.
async fn centro_de<T: TxNomina>(
    tx: &mut T,
    l: &LiquidacionContrato,
    p: &PeriodoNomina,
) -> AppResult<Option<domain::CentroCostoId>> {
    Ok(tx
        .versiones(l.contrato_id)
        .await?
        .into_iter()
        .rfind(|v| v.vigente_desde <= p.hasta)
        .and_then(|v| v.centro_costo_id))
}

/// Contabiliza las liquidaciones del periodo aprobado (no hace nada sin contabilidad).
///
/// # Errors
///
/// Los del contabilizador: sin regla vigente para [`eventos::LIQUIDACION`], un concepto que la
/// regla no tiene, periodo contable cerrado…
pub(crate) async fn contabilizar_aprobacion<T: TxNomina + TxContable>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    p: &PeriodoNomina,
    liquidaciones: &[LiquidacionContrato],
) -> AppResult<u32> {
    if !con_contabilidad(tx, reloj).await? {
        return Ok(0);
    }
    let mut n = 0;
    for l in liquidaciones {
        let persona = persona_de(tx, l).await?;
        let centro = centro_de(tx, l, p).await?;
        let mut importes: Vec<ImporteDocumento> = l
            .liquidacion
            .lineas
            .iter()
            .map(|x| importe(x.concepto.as_str(), x.valor, persona, centro))
            .collect();
        importes.push(importe(
            conceptos::NETO,
            l.liquidacion.neto,
            persona,
            centro,
        ));
        contabilizar_documento(
            tx,
            reloj,
            ctx,
            documento(eventos::LIQUIDACION, l, p, p.hasta, importes),
        )
        .await?;
        n += 1;
    }
    Ok(n)
}

/// Contabiliza el pago del periodo (no hace nada sin contabilidad).
///
/// # Errors
///
/// Los del contabilizador (sin regla vigente para [`eventos::PAGO`]…).
pub(crate) async fn contabilizar_pago<T: TxNomina + TxContable>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    p: &PeriodoNomina,
    liquidaciones: &[LiquidacionContrato],
    fecha_pago: NaiveDate,
) -> AppResult<u32> {
    if !con_contabilidad(tx, reloj).await? {
        return Ok(0);
    }
    let mut n = 0;
    for l in liquidaciones
        .iter()
        .filter(|l| !l.liquidacion.neto.es_cero())
    {
        let persona = persona_de(tx, l).await?;
        let importes = vec![
            importe(conceptos::NETO, l.liquidacion.neto, persona, None),
            importe(conceptos::GIRADO, l.liquidacion.neto, persona, None),
        ];
        contabilizar_documento(
            tx,
            reloj,
            ctx,
            documento(eventos::PAGO, l, p, fecha_pago, importes),
        )
        .await?;
        n += 1;
    }
    Ok(n)
}

fn documento(
    evento: &str,
    l: &LiquidacionContrato,
    p: &PeriodoNomina,
    fecha: NaiveDate,
    importes: Vec<ImporteDocumento>,
) -> DocumentoOrigen {
    DocumentoOrigen {
        evento: evento.into(),
        origen: Origen {
            tipo: evento.into(),
            id: l.id.as_uuid(),
        },
        fecha,
        descripcion: format!("Nómina del {} al {}", p.desde, p.hasta),
        importes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_regla_de_liquidacion_lista_todos_los_conceptos_y_el_neto() {
        let c = conceptos_de_liquidacion();
        assert!(c.contains(&"basico"));
        assert!(c.contains(&"provision_prima"));
        assert!(c.contains(&conceptos::NETO));
        assert_eq!(c.len(), ConceptoNomina::TODOS.len() + 1);
    }
}

/// Resultado de contabilizar la nómina aprobada antes de tener contabilidad.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InformeContabilizacionNomina {
    /// Periodos contabilizados (o que ya lo estaban: es idempotente).
    pub contabilizados: Vec<domain::PeriodoNominaId>,
    /// Los que no se pudieron contabilizar, con el motivo (regla faltante, periodo contable
    /// cerrado…).
    pub fallidos: Vec<(domain::PeriodoNominaId, String)>,
}

async fn abrir_pendientes<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<U::Transaccion>
where
    U: crate::puertos::UnidadDeTrabajo,
    U::Transaccion: TxNomina + TxContable,
{
    ctx.exigir(domain::Permiso::ContabilidadComprobantesContabilizar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    crate::casos_uso::modulos::exigir_modulo_en(&mut tx, reloj, Modulo::Nomina).await?;
    crate::casos_uso::modulos::exigir_modulo_en(&mut tx, reloj, Modulo::Contabilidad).await?;
    Ok(tx)
}

/// Contabiliza los periodos aprobados o pagados que terminan entre `desde` y `hasta` (la nómina
/// que se aprobó antes de habilitar la contabilidad). Cada periodo va en su propia transacción:
/// uno que falle queda en el informe y no frena a los demás.
///
/// # Errors
///
/// [`crate::AppError::SinPermiso`] sin `contabilidad.comprobantes.contabilizar`;
/// [`crate::AppError::ModuloNoHabilitado`] sin nómina o sin contabilidad;
/// [`crate::AppError::Validacion`] si el rango está invertido.
pub async fn contabilizar_pendientes<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<InformeContabilizacionNomina>
where
    U: crate::puertos::UnidadDeTrabajo,
    U::Transaccion: TxNomina + TxContable,
{
    use crate::puertos::{RegistroAuditoria as _, Transaccion as _};
    use domain::nomina::periodo::EstadoPeriodoNomina as E;
    if hasta < desde {
        return Err(crate::AppError::Validacion(
            "`hasta` no puede ser anterior a `desde`".into(),
        ));
    }
    let periodos: Vec<PeriodoNomina> = abrir_pendientes(unidad, reloj, ctx)
        .await?
        .periodos(super::liquidacion::LIMITE_PERIODOS)
        .await?
        .into_iter()
        .filter(|p| {
            p.hasta >= desde && p.hasta <= hasta && matches!(p.estado, E::Aprobado | E::Pagado)
        })
        .collect();
    let mut informe = InformeContabilizacionNomina::default();
    for p in periodos {
        let mut tx = abrir_pendientes(unidad, reloj, ctx).await?;
        let resultado = async {
            let liquidaciones = tx.liquidaciones(p.id).await?;
            contabilizar_aprobacion(&mut tx, reloj, ctx, &p, &liquidaciones).await?;
            if let (E::Pagado, Some(fecha)) = (p.estado, p.fecha_pago) {
                contabilizar_pago(&mut tx, reloj, ctx, &p, &liquidaciones, fecha).await?;
            }
            Ok::<(), crate::AppError>(())
        }
        .await;
        match resultado {
            Ok(()) => {
                tx.registrar(crate::auditoria::EventoAuditoria::ok(
                    ACCION_CONTABILIZAR,
                    "periodo_nomina",
                    p.id.as_uuid(),
                ))
                .await?;
                tx.confirmar().await?;
                informe.contabilizados.push(p.id);
            }
            Err(crate::AppError::Infraestructura(e)) => {
                return Err(crate::AppError::Infraestructura(e));
            }
            Err(e) => informe.fallidos.push((p.id, e.to_string())),
        }
    }
    Ok(informe)
}

const ACCION_CONTABILIZAR: crate::auditoria::AccionAuditoria =
    crate::auditoria::AccionAuditoria::new("nomina.periodo.contabilizar");
