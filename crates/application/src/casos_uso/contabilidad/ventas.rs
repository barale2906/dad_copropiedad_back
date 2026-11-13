//! Facturas de venta y cuentas por cobrar genéricas (plan 13 fase 2b): borrador → emitir
//! (contabiliza) → recaudo con los **recibos de caja** de la tesorería (un solo mecanismo de
//! recaudo) → anular sin recaudos. Cartera por edades de clientes.
//!
//! La numeración DIAN y el envío electrónico llegan con el plan 15; mientras tanto la factura
//! toma el número de su comprobante `FV`. El IVA lo elige quien factura en cada línea (nunca un
//! 19 % automático).

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{
    FacturaVentaId, Permiso, PersonaId,
    contabilidad::{
        CodigoCuenta, ImporteDocumento, Monto,
        tesoreria::{EstadoFacturaVenta, TramoEdad, liquidar_linea_venta, situacion_pago},
    },
};
use rust_decimal::Decimal;

use super::comprobantes::{self, DocumentoOrigen, revertir_en};
use super::prelude::*;
use super::tesoreria::{
    LIMITE_LISTADO, abrir, centro_por_codigo, conceptos, cuenta_de_movimiento, eventos,
    exigir_persona, origen,
};
use super::{TxTesoreria, texto};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        Reloj, UnidadDeTrabajo,
        contabilidad::Origen,
        tesoreria::{FacturaVenta, FiltroFacturas, LineaFacturaVenta},
    },
};

const ACCION_CREAR: AccionAuditoria = AccionAuditoria::new("tesoreria.factura_venta.crear");
const ACCION_ELIMINAR: AccionAuditoria = AccionAuditoria::new("tesoreria.factura_venta.eliminar");
const ACCION_EMITIR: AccionAuditoria = AccionAuditoria::new("tesoreria.factura_venta.emitir");
const ACCION_ANULAR: AccionAuditoria = AccionAuditoria::new("tesoreria.factura_venta.anular");

fn validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

/// Línea de una factura de venta nueva (por códigos).
#[derive(Debug, Clone)]
pub struct NuevaLineaVenta {
    pub descripcion: String,
    /// Cuenta de ingreso.
    pub cuenta: CodigoCuenta,
    /// Código del centro de costo.
    pub centro_costo: Option<String>,
    pub cantidad: Decimal,
    pub valor_unitario: Monto,
    /// Porcentaje de IVA (0 si no aplica).
    pub tarifa_iva: Decimal,
}

/// Factura de venta nueva (queda en borrador).
#[derive(Debug, Clone)]
pub struct NuevaFacturaVenta {
    pub persona: PersonaId,
    pub fecha: NaiveDate,
    pub vencimiento: NaiveDate,
    pub descripcion: String,
    /// Documento que la originó en otro módulo (p. ej. una reserva).
    pub origen: Option<Origen>,
    pub lineas: Vec<NuevaLineaVenta>,
}

/// Crea una factura de venta en borrador con el subtotal y el IVA de cada línea.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.facturas_venta.gestionar`; [`AppError::Validacion`]
/// si una línea no cumple; [`AppError::Conflicto`] si el documento origen ya tiene factura.
pub async fn crear_factura_venta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaFacturaVenta,
) -> AppResult<FacturaVenta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    if nueva.vencimiento < nueva.fecha {
        return Err(validacion(
            "el vencimiento no puede ser anterior a la fecha",
        ));
    }
    if nueva.lineas.is_empty() || nueva.lineas.len() > super::cuentas_por_pagar::MAXIMO_LINEAS {
        return Err(validacion("la factura lleva de 1 a 200 líneas"));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaFacturasVentaGestionar).await?;
    exigir_persona(&mut tx, nueva.persona).await?;
    let mut lineas = Vec::with_capacity(nueva.lineas.len());
    for l in nueva.lineas {
        let (subtotal, iva) = liquidar_linea_venta(l.cantidad, l.valor_unitario, l.tarifa_iva)?;
        lineas.push(LineaFacturaVenta {
            descripcion: texto("descripcion", &l.descripcion, 300)?,
            cuenta_id: cuenta_de_movimiento(&mut tx, &l.cuenta).await?,
            centro_costo_id: centro_por_codigo(&mut tx, l.centro_costo.as_deref()).await?,
            cantidad: l.cantidad,
            valor_unitario: l.valor_unitario,
            tarifa_iva: l.tarifa_iva,
            subtotal,
            iva,
        });
    }
    let subtotal: Monto = lineas.iter().map(|l| l.subtotal).sum();
    let iva: Monto = lineas.iter().map(|l| l.iva).sum();
    let factura = FacturaVenta {
        id: FacturaVentaId::nuevo(),
        persona_id: nueva.persona,
        numero: None,
        fecha: nueva.fecha,
        vencimiento: nueva.vencimiento,
        descripcion: texto("descripcion", &nueva.descripcion, 300)?,
        origen: nueva.origen,
        subtotal,
        iva,
        total: subtotal + iva,
        pagado: Monto::CERO,
        estado: EstadoFacturaVenta::Borrador,
        comprobante_id: None,
        motivo_anulacion: None,
        creada_por: ctx.actor_id,
        creada_en: reloj.ahora(),
        emitida_en: None,
        lineas,
    };
    tx.insertar_factura_venta(&factura).await?;
    let detalle = DetalleAuditoria::vacio().con("total", factura.total.valor().to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR, "factura_venta", factura.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(factura)
}

async fn factura_existente<T: TxTesoreria>(
    tx: &mut T,
    id: FacturaVentaId,
) -> AppResult<FacturaVenta> {
    tx.factura_venta(id)
        .await?
        .ok_or(AppError::NoEncontrado("factura de venta"))
}

/// Borra una factura en borrador.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.facturas_venta.gestionar`; [`AppError::Conflicto`] si
/// ya se emitió.
pub async fn eliminar_factura_venta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaVentaId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaFacturasVentaGestionar).await?;
    let f = factura_existente(&mut tx, id).await?;
    if f.estado != EstadoFacturaVenta::Borrador {
        return Err(AppError::Conflicto(
            "solo se borra una factura en borrador: anúlela".into(),
        ));
    }
    tx.eliminar_factura_venta(id).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_ELIMINAR,
        "factura_venta",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Emite la factura: la contabiliza (cuenta por cobrar, ingresos por línea e IVA generado) y le
/// asigna el número de su comprobante.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.facturas_venta.gestionar`;
/// [`domain::DominioError::TransicionInvalida`] si no está en borrador; los del contabilizador.
pub async fn emitir_factura_venta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaVentaId,
) -> AppResult<FacturaVenta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaFacturasVentaGestionar).await?;
    let mut f = factura_existente(&mut tx, id).await?;
    f.estado = f.estado.transicionar(EstadoFacturaVenta::Emitida)?;
    let cliente = Some(f.persona_id);
    let mut importes =
        vec![ImporteDocumento::new(conceptos::POR_COBRAR, f.total).con_persona(cliente)];
    for l in &f.lineas {
        importes.push(
            ImporteDocumento::new(conceptos::INGRESO, l.subtotal)
                .con_persona(cliente)
                .con_centro(l.centro_costo_id)
                .con_cuenta(Some(l.cuenta_id))
                .con_descripcion(Some(l.descripcion.clone())),
        );
    }
    if !f.iva.es_cero() {
        let base: Monto = f
            .lineas
            .iter()
            .filter(|l| !l.iva.es_cero())
            .map(|l| l.subtotal)
            .sum();
        importes.push(
            ImporteDocumento::new(conceptos::IVA_GENERADO, f.iva)
                .con_persona(cliente)
                .con_base(Some(base)),
        );
    }
    let comprobante = comprobantes::contabilizar_documento(
        &mut tx,
        reloj,
        ctx,
        DocumentoOrigen {
            evento: eventos::FACTURA_VENTA.into(),
            origen: origen(eventos::FACTURA_VENTA, id.as_uuid()),
            fecha: f.fecha,
            descripcion: format!("Factura de venta: {}", f.descripcion),
            importes,
        },
    )
    .await?;
    f.numero = tx.comprobante(comprobante).await?.and_then(|c| c.numero);
    f.comprobante_id = Some(comprobante);
    f.emitida_en = Some(reloj.ahora());
    tx.actualizar_factura_venta(&f).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("numero", f.numero)?
        .con("total", f.total.valor().to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EMITIR, "factura_venta", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(f)
}

/// Anula una factura emitida sin recaudos revirtiendo su comprobante (la nota crédito
/// electrónica llega con el plan 15).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.facturas_venta.gestionar`;
/// [`domain::DominioError::TransicionInvalida`] si tiene recaudos o no se emitió; los del
/// reverso.
pub async fn anular_factura_venta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaVentaId,
    motivo: &str,
) -> AppResult<FacturaVenta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaFacturasVentaGestionar).await?;
    let mut f = factura_existente(&mut tx, id).await?;
    f.estado = f.estado.transicionar(EstadoFacturaVenta::Anulada)?;
    if let Some(c) = f.comprobante_id {
        revertir_en(&mut tx, reloj, ctx, c, motivo, None).await?;
    }
    f.motivo_anulacion = Some(texto("motivo", motivo, 500)?);
    tx.actualizar_factura_venta(&f).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_ANULAR,
        "factura_venta",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(f)
}

/// Factura de venta por id, con sus líneas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::NoEncontrado`].
pub async fn factura_venta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaVentaId,
) -> AppResult<FacturaVenta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    factura_existente(&mut tx, id).await
}

/// Facturas de venta por vencimiento (a lo sumo [`LIMITE_LISTADO`]).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn facturas_venta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroFacturas,
) -> AppResult<Vec<FacturaVenta>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    tx.facturas_venta(&filtro, LIMITE_LISTADO).await
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
        return Err(validacion("la factura es de otro cliente"));
    }
    if !f.estado.admite_recaudo() {
        return Err(AppError::Conflicto(format!(
            "la factura está {} y no admite recaudos",
            f.estado.as_str()
        )));
    }
    let saldo = f.total.valor() - f.pagado.valor();
    if valor.es_cero() || valor.valor() > saldo {
        return Err(validacion(format!("el saldo de la factura es {saldo}")));
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

/// Cartera de un cliente por tramos de edad.
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
    let mut por_cliente: BTreeMap<PersonaId, FilaCartera> = BTreeMap::new();
    for f in tx.facturas_venta_con_saldo(corte).await? {
        let saldo = f.total.valor() - f.pagado.valor();
        if saldo.is_zero() {
            continue;
        }
        let fila = match por_cliente.entry(f.persona_id) {
            std::collections::btree_map::Entry::Occupied(o) => o.into_mut(),
            std::collections::btree_map::Entry::Vacant(v) => {
                let nombre = tx
                    .persona(f.persona_id)
                    .await?
                    .map(|p| p.nombre)
                    .unwrap_or_default();
                v.insert(FilaCartera {
                    persona_id: f.persona_id,
                    nombre,
                    tramos: BTreeMap::new(),
                    total: Decimal::ZERO,
                })
            }
        };
        *fila
            .tramos
            .entry(TramoEdad::de(f.vencimiento, corte))
            .or_default() += saldo;
        fila.total += saldo;
    }
    let mut filas: Vec<FilaCartera> = por_cliente.into_values().collect();
    filas.sort_by(|a, b| b.total.cmp(&a.total).then(a.nombre.cmp(&b.nombre)));
    Ok(filas)
}
