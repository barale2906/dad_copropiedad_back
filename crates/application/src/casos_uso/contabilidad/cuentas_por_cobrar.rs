//! Cuentas por cobrar de las facturas de venta (plan 13 fase 2b): el recaudo con los **recibos
//! de caja** de la tesorería (aplicar y desaplicar) y la cartera de clientes por edades.
//!
//! Es de la contabilidad: la facturación ([`crate::casos_uso::facturacion`]) emite las facturas y
//! la contabilidad lleva su cobro; la contabilidad no importa la facturación (ADR 0022).

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{
    FacturaVentaId, Permiso, PersonaId,
    contabilidad::{
        Monto,
        tesoreria::{EstadoFacturaVenta, TramoEdad, situacion_pago},
    },
};
use rust_decimal::Decimal;

use super::TxTesoreria;
use super::prelude::*;
use super::tesoreria::abrir;
use crate::{
    AppError, AppResult, Contexto,
    puertos::{Reloj, UnidadDeTrabajo, tesoreria::FacturaVenta},
};

async fn factura_existente<T: TxTesoreria>(
    tx: &mut T,
    id: FacturaVentaId,
) -> AppResult<FacturaVenta> {
    tx.factura_venta(id)
        .await?
        .ok_or(AppError::NoEncontrado("factura de venta"))
}

/// Abona `valor` a la factura del cliente (dentro de la transacción del recibo).
pub(crate) async fn aplicar_a_factura<T: TxTesoreria>(
    tx: &mut T,
    id: FacturaVentaId,
    valor: Monto,
    cliente: Option<PersonaId>,
) -> AppResult<()> {
    let mut f = factura_existente(tx, id).await?;
    if cliente != Some(f.persona_id) {
        return Err(AppError::Validacion("la factura es de otro cliente".into()));
    }
    if !f.estado.admite_recaudo() {
        return Err(AppError::Conflicto(format!(
            "la factura está {} y no admite recaudos",
            f.estado.as_str()
        )));
    }
    let saldo = f.total.valor() - f.pagado.valor();
    if valor.es_cero() || valor.valor() > saldo {
        return Err(AppError::Validacion(format!(
            "el saldo de la factura es {saldo}"
        )));
    }
    f.pagado = f.pagado + valor;
    let hacia = match situacion_pago(f.total, f.pagado)? {
        Some(true) => EstadoFacturaVenta::Pagada,
        _ => EstadoFacturaVenta::PagadaParcial,
    };
    f.estado = f.estado.transicionar(hacia)?;
    tx.actualizar_factura_venta(&f).await
}

/// Devuelve `valor` al saldo de la factura (anulación del recibo).
pub(crate) async fn desaplicar_de_factura<T: TxTesoreria>(
    tx: &mut T,
    id: FacturaVentaId,
    valor: Monto,
) -> AppResult<()> {
    let mut f = factura_existente(tx, id).await?;
    f.pagado = Monto::new(f.pagado.valor() - valor.valor())?;
    let hacia = if f.pagado.es_cero() {
        EstadoFacturaVenta::Emitida
    } else {
        EstadoFacturaVenta::PagadaParcial
    };
    f.estado = f.estado.transicionar(hacia)?;
    tx.actualizar_factura_venta(&f).await
}

/// Saldo de un tercero (cliente o proveedor) por tramos de edad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaCartera {
    pub persona_id: PersonaId,
    pub nombre: String,
    /// Saldo por tramo (solo los tramos con saldo).
    pub tramos: BTreeMap<TramoEdad, Decimal>,
    pub total: Decimal,
}

/// Cartera de clientes por edades a la fecha de corte: el saldo actual de cada factura emitida
/// hasta el corte, en el tramo de sus días vencidos. Ordenada por mayor saldo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn cartera_por_edades<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    corte: NaiveDate,
) -> AppResult<Vec<FilaCartera>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    let saldos: Vec<_> = tx
        .facturas_venta_con_saldo(corte)
        .await?
        .into_iter()
        .map(|f| {
            (
                f.persona_id,
                f.vencimiento,
                f.total.valor() - f.pagado.valor(),
            )
        })
        .collect();
    agrupar_por_edades(&mut tx, corte, saldos).await
}

/// Agrupa saldos `(tercero, vencimiento, saldo)` por tercero y tramo de edad a la fecha de corte
/// (omite los saldos en cero). Ordenada por mayor saldo y luego por nombre. La usan la cartera de
/// clientes y la de proveedores.
pub(crate) async fn agrupar_por_edades<T: TxTesoreria>(
    tx: &mut T,
    corte: NaiveDate,
    saldos: Vec<(PersonaId, NaiveDate, Decimal)>,
) -> AppResult<Vec<FilaCartera>> {
    let mut por_tercero: BTreeMap<PersonaId, FilaCartera> = BTreeMap::new();
    for (persona, vencimiento, saldo) in saldos {
        if saldo.is_zero() {
            continue;
        }
        let fila = match por_tercero.entry(persona) {
            std::collections::btree_map::Entry::Occupied(o) => o.into_mut(),
            std::collections::btree_map::Entry::Vacant(v) => {
                let nombre = tx
                    .persona(persona)
                    .await?
                    .map(|p| p.nombre)
                    .unwrap_or_default();
                v.insert(FilaCartera {
                    persona_id: persona,
                    nombre,
                    tramos: BTreeMap::new(),
                    total: Decimal::ZERO,
                })
            }
        };
        *fila
            .tramos
            .entry(TramoEdad::de(vencimiento, corte))
            .or_default() += saldo;
        fila.total += saldo;
    }
    let mut filas: Vec<FilaCartera> = por_tercero.into_values().collect();
    filas.sort_by(|a, b| b.total.cmp(&a.total).then(a.nombre.cmp(&b.nombre)));
    Ok(filas)
}
