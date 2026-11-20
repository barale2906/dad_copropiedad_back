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
//! Contabilizar es idempotente por origen (un documento, un comprobante).

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
        nomina::{LiquidacionContrato, PeriodoNomina},
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
) -> ImporteDocumento {
    ImporteDocumento {
        persona,
        ..ImporteDocumento::new(concepto, monto)
    }
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
        let mut importes: Vec<ImporteDocumento> = l
            .liquidacion
            .lineas
            .iter()
            .map(|x| importe(x.concepto.as_str(), x.valor, persona))
            .collect();
        importes.push(importe(conceptos::NETO, l.liquidacion.neto, persona));
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
            importe(conceptos::NETO, l.liquidacion.neto, persona),
            importe(conceptos::GIRADO, l.liquidacion.neto, persona),
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
