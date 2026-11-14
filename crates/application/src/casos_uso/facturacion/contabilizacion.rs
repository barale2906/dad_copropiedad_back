//! Contabilizar lo facturado antes de tener contabilidad (plan 18 fase A4). Una entidad que
//! facturó solo con el módulo de facturación y luego habilita la contabilidad contabiliza sus
//! facturas emitidas pendientes. Cada factura va en su propia transacción: una que no se pueda
//! (periodo cerrado, regla faltante) queda en el informe y no frena a las demás. Es idempotente:
//! la factura ya contabilizada no se vuelve a tomar.

use chrono::NaiveDate;
use domain::{FacturaVentaId, Modulo, Permiso};

use super::TxVentas;
use super::ventas::contabilizar_factura;
use crate::casos_uso::contabilidad::prelude::*;
use crate::casos_uso::modulos::abrir_con_modulos;
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{Reloj, UnidadDeTrabajo, facturacion::RepositorioFacturacion as _},
};

const ACCION: AccionAuditoria = AccionAuditoria::new("facturacion.factura_venta.contabilizar");

/// Máximo de facturas por ejecución (se repite para las demás).
pub const LIMITE_CONTABILIZAR: u32 = 500;

/// Resultado de contabilizar lo pendiente.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InformeContabilizacion {
    pub contabilizadas: Vec<FacturaVentaId>,
    /// Contabilizadas que se habían pagado a mano: su recaudo no pasó por la tesorería y la
    /// cuenta por cobrar queda abierta hasta que el contador lo registre.
    pub pagadas_a_mano: Vec<FacturaVentaId>,
    /// Las que no se pudieron contabilizar, con el motivo.
    pub fallidas: Vec<(FacturaVentaId, String)>,
}

async fn abrir<U>(unidad: &U, reloj: &impl Reloj, ctx: &Contexto) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    abrir_con_modulos(
        unidad,
        reloj,
        ctx,
        &[Permiso::ContabilidadComprobantesContabilizar],
        &[Modulo::Contabilidad],
    )
    .await
}

/// Contabiliza las facturas emitidas sin comprobante con fecha entre `desde` y `hasta` (a lo
/// sumo [`LIMITE_CONTABILIZAR`]).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.comprobantes.contabilizar`;
/// [`AppError::ModuloNoHabilitado`] sin contabilidad; [`AppError::Validacion`] si el rango está
/// invertido; [`AppError::Infraestructura`] si falla la base de datos.
pub async fn contabilizar_pendientes<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<InformeContabilizacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    if hasta < desde {
        return Err(AppError::Validacion(
            "`hasta` no puede ser anterior a `desde`".into(),
        ));
    }
    let pendientes = abrir(unidad, reloj, ctx)
        .await?
        .facturas_sin_contabilizar(desde, hasta, LIMITE_CONTABILIZAR)
        .await?;
    let mut informe = InformeContabilizacion::default();
    for id in pendientes {
        let mut tx = abrir(unidad, reloj, ctx).await?;
        let Some(mut f) = tx.factura_venta(id).await? else {
            continue;
        };
        if f.comprobante_id.is_some() {
            continue;
        }
        match contabilizar_factura(&mut tx, reloj, ctx, &f).await {
            Ok(comprobante) => {
                f.comprobante_id = Some(comprobante);
                tx.actualizar_factura_venta(&f).await?;
                let detalle =
                    DetalleAuditoria::vacio().con("comprobante", comprobante.to_string())?;
                tx.registrar(
                    EventoAuditoria::ok(ACCION, "factura_venta", id.as_uuid()).con_detalle(detalle),
                )
                .await?;
                tx.confirmar().await?;
                if f.pago_manual.is_some() {
                    informe.pagadas_a_mano.push(id);
                }
                informe.contabilizadas.push(id);
            }
            Err(AppError::Infraestructura(e)) => return Err(AppError::Infraestructura(e)),
            Err(e) => informe.fallidas.push((id, e.to_string())),
        }
    }
    Ok(informe)
}
