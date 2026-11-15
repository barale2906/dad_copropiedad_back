//! Facturas de venta (plan 13 fase 2b y plan 18 fase A3): borrador → emitir → pagada o anulada.
//!
//! Sirven a una entidad con facturación electrónica, con contabilidad o con las dos. Al emitir,
//! la factura toma su **número interno** del año (el legal es el del documento electrónico de la
//! DIAN) y **se contabiliza solo si la entidad lleva contabilidad**: sin ella no hay cuentas ni
//! centros de costo y el pago se **marca a mano**. Con contabilidad el recaudo es un recibo de caja
//! ([`crate::casos_uso::contabilidad::cuentas_por_cobrar`]) y la factura que se emitió sin
//! contabilizar se contabiliza después (fase A4).
//!
//! Una línea puede salir del catálogo ([`super::items`]) y tomar de él descripción, precio, IVA,
//! clasificación y cuenta. El IVA lo elige quien factura en cada línea (nunca un 19 %
//! automático) y debe ser una tarifa gravada vigente con el perfil tributario confirmado.

use chrono::{Datelike, NaiveDate};
use domain::{
    CentroCostoId, CuentaContableId, FacturaVentaId, ItemFacturacionId, Permiso, PersonaId,
    contabilidad::{
        CodigoCuenta, ImporteDocumento, Monto,
        tesoreria::{EstadoFacturaVenta, liquidar_linea_venta},
    },
    facturacion::MedioPagoManual,
    tributario::{
        ClasificacionTributaria, EstadoDocumentoElectronico, TipoDocumentoElectronico,
        validar_iva_linea,
    },
};
use rust_decimal::Decimal;

use super::{TxVentas, abrir_ventas, con_contabilidad};
use crate::casos_uso::contabilidad::comprobantes::{self, DocumentoOrigen, revertir_en};
use crate::casos_uso::contabilidad::prelude::*;
use crate::casos_uso::contabilidad::tesoreria::{
    LIMITE_LISTADO, centro_por_codigo, conceptos, cuenta_de_movimiento, eventos, exigir_persona,
    origen,
};
use crate::casos_uso::contabilidad::{TxTesoreria, texto, texto_opcional};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        Reloj, UnidadDeTrabajo,
        contabilidad::Origen,
        facturacion::ItemFacturacion,
        tesoreria::{FacturaVenta, FiltroFacturas, LineaFacturaVenta, NotaDebitoVenta, PagoManual},
    },
};

const ACCION_CREAR: AccionAuditoria = AccionAuditoria::new("facturacion.factura_venta.crear");
const ACCION_ELIMINAR: AccionAuditoria = AccionAuditoria::new("facturacion.factura_venta.eliminar");
const ACCION_EMITIR: AccionAuditoria = AccionAuditoria::new("facturacion.factura_venta.emitir");
const ACCION_PAGAR: AccionAuditoria = AccionAuditoria::new("facturacion.factura_venta.pagar");
const ACCION_ANULAR: AccionAuditoria = AccionAuditoria::new("facturacion.factura_venta.anular");

fn validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

/// Línea de una factura de venta nueva. Con `item`, lo que la línea no traiga sale del catálogo.
#[derive(Debug, Clone, Default)]
pub struct NuevaLineaVenta {
    /// Producto o servicio del catálogo (activo).
    pub item: Option<ItemFacturacionId>,
    /// Obligatoria sin ítem.
    pub descripcion: Option<String>,
    /// Cuenta de ingreso, solo con contabilidad; si falta, la del ítem o la de la regla del
    /// evento al contabilizar.
    pub cuenta: Option<CodigoCuenta>,
    /// Código del centro de costo, solo con contabilidad.
    pub centro_costo: Option<String>,
    pub cantidad: Decimal,
    /// Obligatorio sin ítem o si el ítem no tiene precio.
    pub valor_unitario: Option<Monto>,
    /// Porcentaje de IVA (0 si no aplica; si falta, el del ítem). Debe ser una tarifa gravada
    /// vigente del catálogo y solo en alquiler o servicio y venta, con el perfil tributario
    /// confirmado de responsable de IVA.
    pub tarifa_iva: Option<Decimal>,
    /// Si falta, la del ítem o `otra`.
    pub clasificacion: Option<ClasificacionTributaria>,
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

/// Línea con todo resuelto (catálogo y cuentas).
struct LineaResuelta {
    item_id: Option<ItemFacturacionId>,
    descripcion: String,
    cuenta_id: Option<CuentaContableId>,
    centro_costo_id: Option<CentroCostoId>,
    valor_unitario: Monto,
    tarifa_iva: Decimal,
    clasificacion: ClasificacionTributaria,
}

async fn resolver_linea<T: TxVentas>(
    tx: &mut T,
    contable: bool,
    l: &NuevaLineaVenta,
) -> AppResult<LineaResuelta> {
    let item: Option<ItemFacturacion> = match l.item {
        None => None,
        Some(id) => {
            let i = tx.item(id).await?.ok_or(AppError::NoEncontrado("ítem"))?;
            if !i.activo {
                return Err(validacion(format!("el ítem {} está inactivo", i.codigo)));
            }
            Some(i)
        }
    };
    if !contable && (l.cuenta.is_some() || l.centro_costo.is_some()) {
        return Err(validacion(
            "sin el módulo de contabilidad la factura no lleva cuentas ni centros de costo",
        ));
    }
    let descripcion = match (&l.descripcion, &item) {
        (Some(d), _) => texto("descripcion", d, 300)?,
        (None, Some(i)) => i.descripcion.clone(),
        (None, None) => return Err(validacion("cada línea lleva descripción o un ítem")),
    };
    let valor_unitario = l
        .valor_unitario
        .or_else(|| item.as_ref().and_then(|i| i.precio))
        .ok_or_else(|| validacion("cada línea lleva su valor unitario (o un ítem con precio)"))?;
    let cuenta_id = match &l.cuenta {
        Some(c) => Some(cuenta_de_movimiento(tx, c).await?),
        None if contable => item.as_ref().and_then(|i| i.cuenta_id),
        None => None,
    };
    let centro_costo_id = if contable {
        centro_por_codigo(tx, l.centro_costo.as_deref()).await?
    } else {
        None
    };
    Ok(LineaResuelta {
        item_id: item.as_ref().map(|i| i.id),
        descripcion,
        cuenta_id,
        centro_costo_id,
        valor_unitario,
        tarifa_iva: l
            .tarifa_iva
            .or_else(|| item.as_ref().map(|i| i.tarifa_iva))
            .unwrap_or(Decimal::ZERO),
        clasificacion: l
            .clasificacion
            .or_else(|| item.as_ref().map(|i| i.clasificacion))
            .unwrap_or(ClasificacionTributaria::Otra),
    })
}

/// Crea una factura de venta en borrador con el subtotal y el IVA de cada línea.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.facturas.gestionar`;
/// [`AppError::ModuloNoHabilitado`] sin facturación ni contabilidad; [`AppError::Validacion`] si
/// una línea no cumple; [`AppError::Conflicto`] si el documento origen ya tiene factura.
pub async fn crear_factura_venta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaFacturaVenta,
) -> AppResult<FacturaVenta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionFacturasGestionar).await?;
    let factura = crear_factura_en(&mut tx, reloj, ctx, nueva, None).await?;
    tx.confirmar().await?;
    Ok(factura)
}

/// Valida, liquida y guarda la factura en borrador dentro de la transacción; con `nota_debito`,
/// es la nota débito de otra factura (la usa también la nota débito electrónica).
pub(crate) async fn crear_factura_en<T: TxVentas>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaFacturaVenta,
    nota_debito: Option<NotaDebitoVenta>,
) -> AppResult<FacturaVenta> {
    if nueva.vencimiento < nueva.fecha {
        return Err(validacion(
            "el vencimiento no puede ser anterior a la fecha",
        ));
    }
    if nueva.lineas.is_empty()
        || nueva.lineas.len() > crate::casos_uso::contabilidad::cuentas_por_pagar::MAXIMO_LINEAS
    {
        return Err(validacion("la factura lleva de 1 a 200 líneas"));
    }
    exigir_persona(tx, nueva.persona).await?;
    let contable = con_contabilidad(tx, reloj).await?;
    let mut resueltas = Vec::with_capacity(nueva.lineas.len());
    for l in &nueva.lineas {
        resueltas.push(resolver_linea(tx, contable, l).await?);
    }
    let con_iva = resueltas.iter().any(|l| !l.tarifa_iva.is_zero());
    let (perfil, gravadas) = if con_iva {
        (
            crate::casos_uso::tributario::perfil_confirmado_en(tx, nueva.fecha).await?,
            crate::casos_uso::tributario::tarifas_gravadas(tx, nueva.fecha).await?,
        )
    } else {
        (None, Vec::new())
    };
    let mut lineas = Vec::with_capacity(resueltas.len());
    for (l, r) in nueva.lineas.iter().zip(resueltas) {
        validar_iva_linea(
            r.clasificacion,
            r.tarifa_iva,
            perfil.as_ref().map(|p| &p.parametros),
            &gravadas,
        )?;
        let (subtotal, iva) = liquidar_linea_venta(l.cantidad, r.valor_unitario, r.tarifa_iva)?;
        lineas.push(LineaFacturaVenta {
            descripcion: r.descripcion,
            cuenta_id: r.cuenta_id,
            item_id: r.item_id,
            centro_costo_id: r.centro_costo_id,
            cantidad: l.cantidad,
            valor_unitario: r.valor_unitario,
            tarifa_iva: r.tarifa_iva,
            clasificacion: r.clasificacion,
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
        pago_manual: None,
        nota_debito,
        lineas,
    };
    tx.insertar_factura_venta(&factura).await?;
    let detalle = DetalleAuditoria::vacio().con("total", factura.total.valor().to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR, "factura_venta", factura.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
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
/// [`AppError::SinPermiso`] sin `facturacion.facturas.gestionar`; [`AppError::Conflicto`] si
/// ya se emitió.
pub async fn eliminar_factura_venta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaVentaId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionFacturasGestionar).await?;
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

/// Contabiliza una factura emitida: cuenta por cobrar, ingresos por línea (con su cuenta o la de
/// la regla del evento) e IVA generado. Devuelve el comprobante.
pub(crate) async fn contabilizar_factura<T: TxTesoreria>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    f: &FacturaVenta,
) -> AppResult<domain::ComprobanteId> {
    let cliente = Some(f.persona_id);
    let mut importes =
        vec![ImporteDocumento::new(conceptos::POR_COBRAR, f.total).con_persona(cliente)];
    for l in &f.lineas {
        importes.push(
            ImporteDocumento::new(conceptos::INGRESO, l.subtotal)
                .con_persona(cliente)
                .con_centro(l.centro_costo_id)
                .con_cuenta(l.cuenta_id)
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
    comprobantes::contabilizar_documento(
        tx,
        reloj,
        ctx,
        DocumentoOrigen {
            evento: eventos::FACTURA_VENTA.into(),
            origen: origen(eventos::FACTURA_VENTA, f.id.as_uuid()),
            fecha: f.fecha,
            descripcion: if f.nota_debito.is_some() {
                format!("Nota débito: {}", f.descripcion)
            } else {
                format!("Factura de venta: {}", f.descripcion)
            },
            importes,
        },
    )
    .await
}

/// Emite la factura: le da su número interno del año y, si la entidad lleva contabilidad, la
/// contabiliza en la misma transacción.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.facturas.gestionar`;
/// [`domain::DominioError::TransicionInvalida`] si no está en borrador; los del contabilizador.
pub async fn emitir_factura_venta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaVentaId,
) -> AppResult<FacturaVenta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionFacturasGestionar).await?;
    let f = factura_existente(&mut tx, id).await?;
    let f = emitir_factura_en(&mut tx, reloj, ctx, f).await?;
    tx.confirmar().await?;
    Ok(f)
}

/// Emite la factura dentro de la transacción: número interno del año y, con contabilidad, su
/// comprobante.
pub(crate) async fn emitir_factura_en<T: TxVentas>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    mut f: FacturaVenta,
) -> AppResult<FacturaVenta> {
    f.estado = f.estado.transicionar(EstadoFacturaVenta::Emitida)?;
    f.numero = Some(tx.siguiente_numero_factura(f.fecha.year()).await?);
    if con_contabilidad(tx, reloj).await? {
        f.comprobante_id = Some(contabilizar_factura(tx, reloj, ctx, &f).await?);
    }
    f.emitida_en = Some(reloj.ahora());
    tx.actualizar_factura_venta(&f).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("numero", f.numero)?
        .con("total", f.total.valor().to_string())?
        .con("contabilizada", f.comprobante_id.is_some())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EMITIR, "factura_venta", f.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    Ok(f)
}

/// Pago de una factura que se marca a mano.
#[derive(Debug, Clone)]
pub struct NuevoPagoManual {
    pub fecha: NaiveDate,
    pub medio: MedioPagoManual,
    pub referencia: Option<String>,
}

/// Marca pagada una factura emitida de una entidad **sin contabilidad** (sin tesorería no hay
/// recibos de caja). Con contabilidad el pago se registra con un recibo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.facturas.gestionar`; [`AppError::Conflicto`] si
/// la entidad lleva contabilidad; [`domain::DominioError::TransicionInvalida`] si no está
/// emitida; [`AppError::Validacion`] si la fecha es anterior a la factura.
pub async fn marcar_pagada<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaVentaId,
    pago: NuevoPagoManual,
) -> AppResult<FacturaVenta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    let por = ctx
        .actor_id
        .ok_or_else(|| validacion("el pago lo marca una cuenta"))?;
    let referencia = texto_opcional("referencia", pago.referencia.as_deref(), 100)?;
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionFacturasGestionar).await?;
    if con_contabilidad(&mut tx, reloj).await? {
        return Err(AppError::Conflicto(
            "la entidad lleva contabilidad: registre el pago con un recibo de caja".into(),
        ));
    }
    let mut f = factura_existente(&mut tx, id).await?;
    if pago.fecha < f.fecha {
        return Err(validacion(
            "el pago no puede ser anterior a la fecha de la factura",
        ));
    }
    f.estado = f.estado.transicionar(EstadoFacturaVenta::Pagada)?;
    f.pagado = f.total;
    f.pago_manual = Some(PagoManual {
        fecha: pago.fecha,
        medio: pago.medio,
        referencia,
        por,
    });
    tx.actualizar_factura_venta(&f).await?;
    let detalle = DetalleAuditoria::vacio().con("medio", pago.medio.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PAGAR, "factura_venta", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(f)
}

/// Anula una factura emitida sin pagos (revierte su comprobante si se contabilizó). Con
/// factura electrónica validada se anula con una nota crédito electrónica.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.facturas.gestionar`;
/// [`domain::DominioError::TransicionInvalida`] si tiene pagos o no se emitió; los del
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
    U::Transaccion: TxVentas,
{
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionFacturasGestionar).await?;
    let electronica = tx
        .documento_por_origen(
            TipoDocumentoElectronico::Factura,
            &origen(eventos::FACTURA_VENTA, id.as_uuid()),
        )
        .await?
        .is_some_and(|d| d.estado == EstadoDocumentoElectronico::Validado);
    if electronica {
        return Err(AppError::Conflicto(
            "la factura tiene factura electrónica validada: anúlela con una nota crédito \
             electrónica"
                .into(),
        ));
    }
    let f = anular_factura_venta_en(&mut tx, reloj, ctx, id, motivo).await?;
    tx.confirmar().await?;
    Ok(f)
}

/// Anula la factura en la transacción (revierte su comprobante si lo tiene); la usa también la
/// nota crédito electrónica.
pub(crate) async fn anular_factura_venta_en<T: TxTesoreria>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaVentaId,
    motivo: &str,
) -> AppResult<FacturaVenta> {
    let mut f = factura_existente(tx, id).await?;
    f.estado = f.estado.transicionar(EstadoFacturaVenta::Anulada)?;
    if let Some(c) = f.comprobante_id {
        revertir_en(tx, reloj, ctx, c, motivo, None).await?;
    }
    f.motivo_anulacion = Some(texto("motivo", motivo, 500)?);
    tx.actualizar_factura_venta(&f).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_ANULAR,
        "factura_venta",
        id.as_uuid(),
    ))
    .await?;
    Ok(f)
}

/// Factura de venta por id, con sus líneas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.ver`; [`AppError::NoEncontrado`].
pub async fn factura_venta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaVentaId,
) -> AppResult<FacturaVenta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionVer).await?;
    factura_existente(&mut tx, id).await
}

/// Facturas de venta por vencimiento (a lo sumo [`LIMITE_LISTADO`]).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.ver`.
pub async fn facturas_venta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroFacturas,
) -> AppResult<Vec<FacturaVenta>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionVer).await?;
    tx.facturas_venta(&filtro, LIMITE_LISTADO).await
}
