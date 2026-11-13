//! Tesorería (plan 13 fases 1 y 4): cuentas de tesorería, recibos de caja con su aplicación a
//! facturas de venta, egresos con segregación de funciones y los eventos de la pasarela de
//! pagos.
//!
//! Ningún caso de uso escribe asientos: cada documento se entrega al contabilizador del núcleo
//! ([`comprobantes::contabilizar_documento`]) con los conceptos de [`conceptos`] y el evento de
//! [`eventos`]; el contador define la regla de cada evento. Anular = revertir el comprobante.

use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, CuentaContableId, CuentaTesoreriaId, EgresoId, EventoPasarelaId, FacturaProveedorId,
    FacturaVentaId, Permiso, PersonaId, ReciboCajaId,
    contabilidad::{
        CodigoCuenta, ImporteDocumento, Monto, Naturaleza,
        tesoreria::{
            EstadoEgreso, EstadoEventoPasarela, EstadoRecibo, MedioEgreso, MedioRecibo,
            TipoCuentaBancaria, TipoCuentaTesoreria, enmascarar_numero_cuenta,
        },
    },
};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::comprobantes::{self, DocumentoOrigen, revertir_en};
use super::prelude::*;
use super::{TxTesoreria, cuentas_por_pagar, exigir_contabilidad, texto, texto_opcional, ventas};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        Reloj, UnidadDeTrabajo,
        contabilidad::Origen,
        tesoreria::{
            AplicacionPago, CuentaTesoreria, Egreso, EventoPasarela, FiltroRecibos, GastoEgreso,
            ReciboCaja,
        },
    },
};

/// Eventos de las reglas de contabilización que usa el plan 13 (y tipo de origen del
/// comprobante).
pub mod eventos {
    /// Recibo de caja.
    pub const RECIBO: &str = "tesoreria.recibo";
    /// Aplicación posterior de un recibo (reclasifica lo que estaba sin aplicar).
    pub const APLICACION: &str = "tesoreria.aplicacion";
    /// Egreso pagado.
    pub const EGRESO: &str = "tesoreria.egreso";
    /// Causación de una factura de proveedor.
    pub const FACTURA_PROVEEDOR: &str = "cxp.factura";
    /// Emisión de una factura de venta.
    pub const FACTURA_VENTA: &str = "ventas.factura";
}

/// Conceptos de los documentos origen del plan 13 (los que la regla de cada evento debe tener).
pub mod conceptos {
    /// Recibo: dinero que entra (débito; la cuenta es la de la cuenta de tesorería).
    pub const RECIBIDO: &str = "recibido";
    /// Recibo y aplicación: lo aplicado a facturas de venta (crédito a la cuenta por cobrar).
    pub const APLICADO: &str = "aplicado";
    /// Recibo: lo que queda sin aplicar (crédito a anticipos o recaudos por identificar); en la
    /// aplicación, débito.
    pub const SIN_APLICAR: &str = "sin_aplicar";
    /// Egreso: lo pagado a facturas de proveedor (débito a la cuenta por pagar).
    pub const PAGADO_FACTURAS: &str = "pagado_facturas";
    /// Egreso y factura de proveedor: gasto (débito; la cuenta es la de la línea).
    pub const GASTO: &str = "gasto";
    /// Egreso: dinero que sale (crédito; la cuenta es la de la cuenta de tesorería).
    pub const GIRADO: &str = "girado";
    /// Factura de proveedor: IVA descontable (débito).
    pub const IVA_DESCONTABLE: &str = "iva_descontable";
    /// Factura de proveedor: lo que se le debe (crédito).
    pub const POR_PAGAR: &str = "por_pagar";
    /// Factura de venta: lo que debe el cliente (débito).
    pub const POR_COBRAR: &str = "por_cobrar";
    /// Factura de venta: ingreso (crédito; la cuenta es la de la línea).
    pub const INGRESO: &str = "ingreso";
    /// Factura de venta: IVA generado (crédito).
    pub const IVA_GENERADO: &str = "iva_generado";
}

const ACCION_CUENTA_CREAR: AccionAuditoria = AccionAuditoria::new("tesoreria.cuenta.crear");
const ACCION_CUENTA_EDITAR: AccionAuditoria = AccionAuditoria::new("tesoreria.cuenta.editar");
const ACCION_RECIBO: AccionAuditoria = AccionAuditoria::new("tesoreria.recibo.registrar");
const ACCION_RECIBO_APLICAR: AccionAuditoria = AccionAuditoria::new("tesoreria.recibo.aplicar");
const ACCION_RECIBO_ANULAR: AccionAuditoria = AccionAuditoria::new("tesoreria.recibo.anular");
const ACCION_EGRESO: AccionAuditoria = AccionAuditoria::new("tesoreria.egreso.registrar");
const ACCION_EGRESO_AUTORIZAR: AccionAuditoria = AccionAuditoria::new("tesoreria.egreso.autorizar");
const ACCION_EGRESO_PAGAR: AccionAuditoria = AccionAuditoria::new("tesoreria.egreso.pagar");
const ACCION_EGRESO_ANULAR: AccionAuditoria = AccionAuditoria::new("tesoreria.egreso.anular");
const ACCION_PASARELA: AccionAuditoria = AccionAuditoria::new("tesoreria.pasarela.evento");

/// Máximo de documentos en un listado.
pub const LIMITE_LISTADO: u32 = 500;

pub(crate) async fn abrir<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    permiso: Permiso,
) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    ctx.exigir(permiso)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_contabilidad(&mut tx, reloj).await?;
    Ok(tx)
}

fn validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

pub(crate) fn origen(tipo: &str, id: Uuid) -> Origen {
    Origen {
        tipo: tipo.to_owned(),
        id,
    }
}

/// La persona debe existir en la entidad.
pub(crate) async fn exigir_persona<T: TxTesoreria>(tx: &mut T, id: PersonaId) -> AppResult<()> {
    tx.persona(id)
        .await?
        .map(|_| ())
        .ok_or(AppError::NoEncontrado("tercero"))
}

/// Cuenta contable de movimiento por código.
pub(crate) async fn cuenta_de_movimiento<T: TxTesoreria>(
    tx: &mut T,
    codigo: &CodigoCuenta,
) -> AppResult<CuentaContableId> {
    let c = tx
        .cuenta_por_codigo(codigo)
        .await?
        .ok_or_else(|| validacion(format!("la cuenta {codigo} no existe")))?;
    if !c.acepta_movimiento || !c.activa {
        return Err(validacion(format!(
            "la cuenta {codigo} no acepta movimientos o está inactiva"
        )));
    }
    Ok(c.id)
}

/// Centro de costo por código.
pub(crate) async fn centro_por_codigo<T: TxTesoreria>(
    tx: &mut T,
    codigo: Option<&str>,
) -> AppResult<Option<domain::CentroCostoId>> {
    let Some(codigo) = codigo else {
        return Ok(None);
    };
    tx.centros_costo()
        .await?
        .into_iter()
        .find(|c| c.codigo == codigo && c.activo)
        .map(|c| Some(c.id))
        .ok_or_else(|| validacion(format!("el centro de costo {codigo} no existe")))
}

async fn cuenta_activa<T: TxTesoreria>(
    tx: &mut T,
    id: CuentaTesoreriaId,
) -> AppResult<CuentaTesoreria> {
    let c = tx
        .cuenta_tesoreria(id)
        .await?
        .ok_or(AppError::NoEncontrado("cuenta de tesorería"))?;
    if !c.activa {
        return Err(AppError::Conflicto(
            "la cuenta de tesorería está inactiva".into(),
        ));
    }
    Ok(c)
}

fn monto_de(valor: Decimal) -> AppResult<Monto> {
    Ok(Monto::new(valor)?)
}

// ---------------------------------------------------------------------------------------------
// Cuentas de tesorería
// ---------------------------------------------------------------------------------------------

/// Datos de una cuenta de tesorería nueva.
#[derive(Debug, Clone)]
pub struct NuevaCuentaTesoreria {
    pub tipo: TipoCuentaTesoreria,
    pub nombre: String,
    /// Obligatorio en las de tipo banco.
    pub banco: Option<String>,
    /// Número completo: solo se guardan los cuatro últimos dígitos.
    pub numero_cuenta: Option<String>,
    pub tipo_cuenta: Option<TipoCuentaBancaria>,
    /// Cuenta contable de movimiento, de naturaleza débito, que la respalda.
    pub cuenta_contable: CodigoCuenta,
    pub destinacion: Option<String>,
}

/// Cambios de una cuenta de tesorería.
#[derive(Debug, Clone)]
pub struct CambioCuentaTesoreria {
    pub nombre: String,
    pub destinacion: Option<String>,
    pub activa: bool,
}

/// Cuentas de tesorería de la entidad.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::ModuloNoHabilitado`].
pub async fn cuentas_tesoreria<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<CuentaTesoreria>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    tx.cuentas_tesoreria().await
}

/// Crea una cuenta de tesorería respaldada por una cuenta contable de movimiento de naturaleza
/// débito (una cuenta contable respalda una sola cuenta de tesorería).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.configurar`; [`AppError::Validacion`] si faltan los
/// datos del banco o la cuenta contable no sirve; [`AppError::Conflicto`] si el nombre o la
/// cuenta contable ya se usan.
pub async fn crear_cuenta_tesoreria<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaCuentaTesoreria,
) -> AppResult<CuentaTesoreria>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let es_banco = nueva.tipo == TipoCuentaTesoreria::Banco;
    let banco = texto_opcional("banco", nueva.banco.as_deref(), 120)?;
    if es_banco != banco.is_some() {
        return Err(validacion(
            "el banco se indica solo, y siempre, en las cuentas bancarias",
        ));
    }
    if !es_banco && (nueva.numero_cuenta.is_some() || nueva.tipo_cuenta.is_some()) {
        return Err(validacion(
            "el número y el tipo de cuenta son solo de las cuentas bancarias",
        ));
    }
    let numero_enmascarado = nueva
        .numero_cuenta
        .as_deref()
        .map(enmascarar_numero_cuenta)
        .transpose()?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaConfigurar).await?;
    let contable = tx
        .cuenta_por_codigo(&nueva.cuenta_contable)
        .await?
        .ok_or_else(|| validacion(format!("la cuenta {} no existe", nueva.cuenta_contable)))?;
    if !contable.acepta_movimiento || !contable.activa || contable.naturaleza != Naturaleza::Debito
    {
        return Err(validacion(
            "la cuenta contable debe ser de movimiento, activa y de naturaleza débito",
        ));
    }
    let cuenta = CuentaTesoreria {
        id: CuentaTesoreriaId::nuevo(),
        tipo: nueva.tipo,
        nombre: texto("nombre", &nueva.nombre, 120)?,
        banco,
        numero_enmascarado,
        tipo_cuenta: nueva.tipo_cuenta,
        cuenta_contable_id: contable.id,
        destinacion: texto_opcional("destinacion", nueva.destinacion.as_deref(), 200)?,
        activa: true,
        creada_en: reloj.ahora(),
    };
    tx.insertar_cuenta_tesoreria(&cuenta).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", cuenta.tipo.as_str())?
        .con("cuenta_contable", nueva.cuenta_contable.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CUENTA_CREAR, "cuenta_tesoreria", cuenta.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(cuenta)
}

/// Cambia el nombre, la destinación o si la cuenta está activa (una inactiva no recibe
/// documentos nuevos).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.configurar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si el nombre ya se usa.
pub async fn actualizar_cuenta_tesoreria<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CuentaTesoreriaId,
    cambio: CambioCuentaTesoreria,
) -> AppResult<CuentaTesoreria>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaConfigurar).await?;
    let mut cuenta = tx
        .cuenta_tesoreria(id)
        .await?
        .ok_or(AppError::NoEncontrado("cuenta de tesorería"))?;
    cuenta.nombre = texto("nombre", &cambio.nombre, 120)?;
    cuenta.destinacion = texto_opcional("destinacion", cambio.destinacion.as_deref(), 200)?;
    cuenta.activa = cambio.activa;
    tx.actualizar_cuenta_tesoreria(&cuenta).await?;
    let detalle = DetalleAuditoria::vacio().con("activa", cuenta.activa)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CUENTA_EDITAR, "cuenta_tesoreria", id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(cuenta)
}

// ---------------------------------------------------------------------------------------------
// Recibos de caja
// ---------------------------------------------------------------------------------------------

/// Recibo nuevo. Lo que no se aplica a facturas queda **sin aplicar** (anticipo o recaudo por
/// identificar) y puede aplicarse después.
#[derive(Debug, Clone)]
pub struct NuevoRecibo {
    pub cuenta_tesoreria: CuentaTesoreriaId,
    /// `None` = pagador no identificado (no admite aplicaciones).
    pub persona: Option<PersonaId>,
    pub fecha: NaiveDate,
    pub valor: Monto,
    pub medio: MedioRecibo,
    pub referencia: Option<String>,
    pub descripcion: String,
    pub soporte_archivo_id: Option<ArchivoId>,
    /// Facturas de venta del pagador y cuánto se aplica a cada una.
    pub aplicaciones: Vec<(FacturaVentaId, Monto)>,
}

/// Registra y contabiliza un recibo de caja, y aplica lo indicado a facturas de venta del
/// pagador.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.recibos.registrar`; [`AppError::Validacion`] si el
/// valor es cero, se aplica más de lo recibido o del saldo de una factura, o hay aplicaciones sin
/// pagador; [`AppError::Conflicto`] si una factura no admite recaudos; los del contabilizador.
pub async fn registrar_recibo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoRecibo,
) -> AppResult<ReciboCaja>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaRecibosRegistrar).await?;
    let recibo = registrar_recibo_en(&mut tx, reloj, ctx, nuevo).await?;
    tx.confirmar().await?;
    Ok(recibo)
}

pub(crate) async fn registrar_recibo_en<T: TxTesoreria>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoRecibo,
) -> AppResult<ReciboCaja> {
    if nuevo.valor.es_cero() {
        return Err(validacion("el valor del recibo debe ser mayor que cero"));
    }
    let descripcion = texto("descripcion", &nuevo.descripcion, 300)?;
    let referencia = texto_opcional("referencia", nuevo.referencia.as_deref(), 80)?;
    let cuenta = cuenta_activa(tx, nuevo.cuenta_tesoreria).await?;
    if let Some(p) = nuevo.persona {
        exigir_persona(tx, p).await?;
    }
    if !nuevo.aplicaciones.is_empty() && nuevo.persona.is_none() {
        return Err(validacion(
            "un recibo sin pagador identificado no se aplica a facturas",
        ));
    }
    let facturas: BTreeSet<FacturaVentaId> = nuevo.aplicaciones.iter().map(|a| a.0).collect();
    if facturas.len() != nuevo.aplicaciones.len() {
        return Err(validacion(
            "una factura aparece dos veces en las aplicaciones",
        ));
    }
    let aplicado: Monto = nuevo.aplicaciones.iter().map(|a| a.1).sum();
    if aplicado > nuevo.valor {
        return Err(validacion("se aplica más de lo recibido"));
    }
    let id = ReciboCajaId::nuevo();
    let ahora = reloj.ahora();
    let mut importes = vec![
        ImporteDocumento::new(conceptos::RECIBIDO, nuevo.valor)
            .con_persona(nuevo.persona)
            .con_cuenta(Some(cuenta.cuenta_contable_id)),
    ];
    let mut aplicaciones = Vec::with_capacity(nuevo.aplicaciones.len());
    for (factura, valor) in &nuevo.aplicaciones {
        ventas::aplicar_a_factura(tx, *factura, *valor, nuevo.persona).await?;
        importes
            .push(ImporteDocumento::new(conceptos::APLICADO, *valor).con_persona(nuevo.persona));
        aplicaciones.push(AplicacionPago {
            id: domain::AplicacionPagoId::nuevo(),
            recibo_id: id,
            factura_venta_id: *factura,
            valor: *valor,
            aplicada_en: ahora,
            comprobante_id: None,
            anulada_en: None,
        });
    }
    let sin_aplicar = monto_de(nuevo.valor.valor() - aplicado.valor())?;
    importes.push(
        ImporteDocumento::new(conceptos::SIN_APLICAR, sin_aplicar).con_persona(nuevo.persona),
    );
    let comprobante_id = comprobantes::contabilizar_documento(
        tx,
        reloj,
        ctx,
        DocumentoOrigen {
            evento: eventos::RECIBO.into(),
            origen: origen(eventos::RECIBO, id.as_uuid()),
            fecha: nuevo.fecha,
            descripcion: descripcion.clone(),
            importes,
        },
    )
    .await?;
    let recibo = ReciboCaja {
        id,
        cuenta_tesoreria_id: cuenta.id,
        persona_id: nuevo.persona,
        fecha: nuevo.fecha,
        valor: nuevo.valor,
        medio: nuevo.medio,
        referencia,
        descripcion,
        soporte_archivo_id: nuevo.soporte_archivo_id,
        estado: EstadoRecibo::Registrado,
        comprobante_id,
        motivo_anulacion: None,
        registrado_por: ctx.actor_id,
        registrado_en: ahora,
        aplicaciones,
    };
    tx.insertar_recibo(&recibo).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("valor", recibo.valor.valor().to_string())?
        .con("medio", recibo.medio.as_str())?
        .con("aplicaciones", recibo.aplicaciones.len())?
        .con("identificado", recibo.persona_id.is_some())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RECIBO, "recibo_caja", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    Ok(recibo)
}

/// Aplica lo que un recibo tiene sin aplicar a una factura de venta (con su propio comprobante:
/// débito a lo sin aplicar, crédito a la cuenta por cobrar). Si el recibo no tenía pagador,
/// queda identificado con el cliente de la factura.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.recibos.registrar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si el recibo está anulado o la factura no admite recaudos;
/// [`AppError::Validacion`] si supera lo sin aplicar o el saldo de la factura.
pub async fn aplicar_recibo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    recibo_id: ReciboCajaId,
    factura: FacturaVentaId,
    valor: Monto,
    fecha: Option<NaiveDate>,
) -> AppResult<ReciboCaja>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaRecibosRegistrar).await?;
    let mut recibo = tx
        .recibo(recibo_id)
        .await?
        .ok_or(AppError::NoEncontrado("recibo de caja"))?;
    if recibo.estado != EstadoRecibo::Registrado {
        return Err(AppError::Conflicto("el recibo está anulado".into()));
    }
    if valor.es_cero() || valor.valor() > recibo.sin_aplicar() {
        return Err(validacion(format!(
            "el recibo tiene {} sin aplicar",
            recibo.sin_aplicar()
        )));
    }
    let cliente = tx
        .factura_venta(factura)
        .await?
        .ok_or(AppError::NoEncontrado("factura de venta"))?
        .persona_id;
    if recibo.persona_id.is_some_and(|p| p != cliente) {
        return Err(validacion("la factura es de otro cliente"));
    }
    ventas::aplicar_a_factura(&mut tx, factura, valor, Some(cliente)).await?;
    let mut aplicacion = AplicacionPago {
        id: domain::AplicacionPagoId::nuevo(),
        recibo_id,
        factura_venta_id: factura,
        valor,
        aplicada_en: reloj.ahora(),
        comprobante_id: None,
        anulada_en: None,
    };
    let comprobante = comprobantes::contabilizar_documento(
        &mut tx,
        reloj,
        ctx,
        DocumentoOrigen {
            evento: eventos::APLICACION.into(),
            origen: origen(eventos::APLICACION, aplicacion.id.as_uuid()),
            fecha: fecha.unwrap_or_else(|| reloj.ahora().date_naive()),
            descripcion: format!("Aplicación del recibo {}", recibo.descripcion),
            importes: vec![
                ImporteDocumento::new(conceptos::SIN_APLICAR, valor).con_persona(recibo.persona_id),
                ImporteDocumento::new(conceptos::APLICADO, valor).con_persona(Some(cliente)),
            ],
        },
    )
    .await?;
    aplicacion.comprobante_id = Some(comprobante);
    tx.insertar_aplicacion(&aplicacion).await?;
    if recibo.persona_id.is_none() {
        tx.identificar_recibo(recibo_id, cliente).await?;
        recibo.persona_id = Some(cliente);
    }
    recibo.aplicaciones.push(aplicacion);
    let detalle = DetalleAuditoria::vacio().con("valor", valor.valor().to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RECIBO_APLICAR, "recibo_caja", recibo_id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(recibo)
}

/// Anula un recibo: revierte su comprobante (y el de sus aplicaciones posteriores) y devuelve el
/// saldo a las facturas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.recibos.registrar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya está anulado; los del reverso (motivo, periodo).
pub async fn anular_recibo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReciboCajaId,
    motivo: &str,
) -> AppResult<ReciboCaja>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaRecibosRegistrar).await?;
    let recibo = anular_recibo_en(&mut tx, reloj, ctx, id, motivo).await?;
    tx.confirmar().await?;
    Ok(recibo)
}

pub(crate) async fn anular_recibo_en<T: TxTesoreria>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReciboCajaId,
    motivo: &str,
) -> AppResult<ReciboCaja> {
    let mut recibo = tx
        .recibo(id)
        .await?
        .ok_or(AppError::NoEncontrado("recibo de caja"))?;
    if recibo.estado == EstadoRecibo::Anulado {
        return Err(AppError::Conflicto("el recibo ya está anulado".into()));
    }
    revertir_en(tx, reloj, ctx, recibo.comprobante_id, motivo, None).await?;
    let ahora = reloj.ahora();
    for a in recibo
        .aplicaciones
        .iter_mut()
        .filter(|a| a.anulada_en.is_none())
    {
        if let Some(c) = a.comprobante_id {
            revertir_en(tx, reloj, ctx, c, motivo, None).await?;
        }
        ventas::desaplicar_de_factura(tx, a.factura_venta_id, a.valor).await?;
        a.anulada_en = Some(ahora);
    }
    tx.anular_aplicaciones(id, ahora).await?;
    let motivo = texto("motivo", motivo, 500)?;
    tx.anular_recibo(id, &motivo).await?;
    recibo.estado = EstadoRecibo::Anulado;
    recibo.motivo_anulacion = Some(motivo);
    tx.registrar(EventoAuditoria::ok(
        ACCION_RECIBO_ANULAR,
        "recibo_caja",
        id.as_uuid(),
    ))
    .await?;
    Ok(recibo)
}

/// Recibo por id, con sus aplicaciones.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::NoEncontrado`].
pub async fn recibo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReciboCajaId,
) -> AppResult<ReciboCaja>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    tx.recibo(id)
        .await?
        .ok_or(AppError::NoEncontrado("recibo de caja"))
}

/// Recibos por fecha descendente (a lo sumo [`LIMITE_LISTADO`]).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn recibos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroRecibos,
) -> AppResult<Vec<ReciboCaja>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    tx.recibos(&filtro, LIMITE_LISTADO).await
}

// ---------------------------------------------------------------------------------------------
// Egresos
// ---------------------------------------------------------------------------------------------

/// Gasto pagado directamente por un egreso (por códigos, como lo escribe el usuario).
#[derive(Debug, Clone)]
pub struct NuevoGasto {
    pub cuenta: CodigoCuenta,
    pub persona: Option<PersonaId>,
    /// Código del centro de costo.
    pub centro_costo: Option<String>,
    pub valor: Monto,
    pub descripcion: String,
}

/// Egreso nuevo: paga facturas de proveedor del beneficiario, gastos directos (caja menor,
/// gastos bancarios) o ambos. El valor es la suma.
#[derive(Debug, Clone)]
pub struct NuevoEgreso {
    pub cuenta_tesoreria: CuentaTesoreriaId,
    pub persona: PersonaId,
    pub fecha: NaiveDate,
    pub medio: MedioEgreso,
    /// Obligatoria con cheque (su número).
    pub referencia: Option<String>,
    pub descripcion: String,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub facturas: Vec<(FacturaProveedorId, Monto)>,
    pub gastos: Vec<NuevoGasto>,
}

/// Registra un egreso por autorizar. Lo registra una cuenta (no el sistema): la segregación de
/// funciones exige saber quién.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.egresos.registrar`; [`AppError::Validacion`] si no
/// paga nada, una factura es de otro proveedor o se paga más que su saldo, o falta el número del
/// cheque; [`AppError::Conflicto`] si una factura no admite pagos.
pub async fn registrar_egreso<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoEgreso,
) -> AppResult<Egreso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let registrado_por = ctx
        .actor_id
        .ok_or_else(|| validacion("un egreso lo registra una cuenta"))?;
    if nuevo.facturas.is_empty() && nuevo.gastos.is_empty() {
        return Err(validacion("el egreso debe pagar facturas o gastos"));
    }
    let referencia = texto_opcional("referencia", nuevo.referencia.as_deref(), 80)?;
    if nuevo.medio == MedioEgreso::Cheque && referencia.is_none() {
        return Err(validacion("indique el número del cheque en la referencia"));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaEgresosRegistrar).await?;
    cuenta_activa(&mut tx, nuevo.cuenta_tesoreria).await?;
    exigir_persona(&mut tx, nuevo.persona).await?;
    let ids: BTreeSet<FacturaProveedorId> = nuevo.facturas.iter().map(|f| f.0).collect();
    if ids.len() != nuevo.facturas.len() {
        return Err(validacion("una factura aparece dos veces en el egreso"));
    }
    for (factura, valor) in &nuevo.facturas {
        cuentas_por_pagar::validar_pago(&mut tx, *factura, *valor, nuevo.persona).await?;
    }
    let mut gastos = Vec::with_capacity(nuevo.gastos.len());
    for g in nuevo.gastos {
        if g.valor.es_cero() {
            return Err(validacion("un gasto del egreso no puede ser cero"));
        }
        if let Some(p) = g.persona {
            exigir_persona(&mut tx, p).await?;
        }
        gastos.push(GastoEgreso {
            cuenta_id: cuenta_de_movimiento(&mut tx, &g.cuenta).await?,
            persona_id: g.persona,
            centro_costo_id: centro_por_codigo(&mut tx, g.centro_costo.as_deref()).await?,
            valor: g.valor,
            descripcion: texto("descripcion", &g.descripcion, 300)?,
        });
    }
    let valor: Monto = nuevo.facturas.iter().map(|f| f.1).sum::<Monto>()
        + gastos.iter().map(|g| g.valor).sum::<Monto>();
    let egreso = Egreso {
        id: EgresoId::nuevo(),
        cuenta_tesoreria_id: nuevo.cuenta_tesoreria,
        persona_id: nuevo.persona,
        fecha: nuevo.fecha,
        valor,
        medio: nuevo.medio,
        referencia,
        descripcion: texto("descripcion", &nuevo.descripcion, 300)?,
        soporte_archivo_id: nuevo.soporte_archivo_id,
        estado: EstadoEgreso::PorAutorizar,
        registrado_por,
        registrado_en: reloj.ahora(),
        autorizado_por: None,
        autorizado_en: None,
        pagado_en: None,
        comprobante_id: None,
        motivo_anulacion: None,
        facturas: nuevo.facturas,
        gastos,
    };
    tx.insertar_egreso(&egreso).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("valor", egreso.valor.valor().to_string())?
        .con("facturas", egreso.facturas.len())?
        .con("gastos", egreso.gastos.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EGRESO, "egreso", egreso.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(egreso)
}

async fn egreso_existente<T: TxTesoreria>(tx: &mut T, id: EgresoId) -> AppResult<Egreso> {
    tx.egreso(id).await?.ok_or(AppError::NoEncontrado("egreso"))
}

/// Autoriza un egreso que registró **otra** cuenta.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.egresos.autorizar`; [`AppError::Conflicto`] si quien
/// autoriza es quien lo registró o el estado no lo permite.
pub async fn autorizar_egreso<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: EgresoId,
) -> AppResult<Egreso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaEgresosAutorizar).await?;
    let mut egreso = egreso_existente(&mut tx, id).await?;
    let quien = ctx
        .actor_id
        .ok_or_else(|| validacion("un egreso lo autoriza una cuenta"))?;
    if quien == egreso.registrado_por {
        return Err(AppError::Conflicto(
            "quien registró el egreso no puede autorizarlo".into(),
        ));
    }
    egreso.estado = egreso.estado.transicionar(EstadoEgreso::Autorizado)?;
    egreso.autorizado_por = Some(quien);
    egreso.autorizado_en = Some(reloj.ahora());
    tx.actualizar_egreso(&egreso).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_EGRESO_AUTORIZAR,
        "egreso",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(egreso)
}

/// Registra el pago de un egreso autorizado (en `fecha`, por defecto la del egreso): lo
/// contabiliza y abona las facturas que paga.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.egresos.registrar`; [`AppError::Conflicto`] si no está
/// autorizado o una factura ya no admite el pago; los del contabilizador.
pub async fn pagar_egreso<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: EgresoId,
    fecha: Option<NaiveDate>,
) -> AppResult<Egreso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaEgresosRegistrar).await?;
    let mut egreso = egreso_existente(&mut tx, id).await?;
    egreso.estado = egreso.estado.transicionar(EstadoEgreso::Pagado)?;
    let cuenta = cuenta_activa(&mut tx, egreso.cuenta_tesoreria_id).await?;
    let fecha = fecha.unwrap_or(egreso.fecha);
    let mut importes = Vec::new();
    for (factura, valor) in &egreso.facturas {
        cuentas_por_pagar::pagar_factura(&mut tx, *factura, *valor, egreso.persona_id).await?;
        importes.push(
            ImporteDocumento::new(conceptos::PAGADO_FACTURAS, *valor)
                .con_persona(Some(egreso.persona_id)),
        );
    }
    for g in &egreso.gastos {
        importes.push(
            ImporteDocumento::new(conceptos::GASTO, g.valor)
                .con_persona(g.persona_id)
                .con_centro(g.centro_costo_id)
                .con_cuenta(Some(g.cuenta_id))
                .con_descripcion(Some(g.descripcion.clone())),
        );
    }
    importes.push(
        ImporteDocumento::new(conceptos::GIRADO, egreso.valor)
            .con_persona(Some(egreso.persona_id))
            .con_cuenta(Some(cuenta.cuenta_contable_id)),
    );
    let comprobante = comprobantes::contabilizar_documento(
        &mut tx,
        reloj,
        ctx,
        DocumentoOrigen {
            evento: eventos::EGRESO.into(),
            origen: origen(eventos::EGRESO, id.as_uuid()),
            fecha,
            descripcion: egreso.descripcion.clone(),
            importes,
        },
    )
    .await?;
    egreso.fecha = fecha;
    egreso.pagado_en = Some(reloj.ahora());
    egreso.comprobante_id = Some(comprobante);
    tx.actualizar_egreso(&egreso).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_EGRESO_PAGAR,
        "egreso",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(egreso)
}

/// Anula un egreso; si ya se pagó, revierte su comprobante y devuelve el saldo a las facturas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.egresos.registrar`; [`AppError::Conflicto`] si ya está
/// anulado; los del reverso.
pub async fn anular_egreso<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: EgresoId,
    motivo: &str,
) -> AppResult<Egreso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaEgresosRegistrar).await?;
    let mut egreso = egreso_existente(&mut tx, id).await?;
    let pagado = egreso.estado == EstadoEgreso::Pagado;
    egreso.estado = egreso.estado.transicionar(EstadoEgreso::Anulado)?;
    if let (true, Some(c)) = (pagado, egreso.comprobante_id) {
        revertir_en(&mut tx, reloj, ctx, c, motivo, None).await?;
        for (factura, valor) in &egreso.facturas {
            cuentas_por_pagar::despagar_factura(&mut tx, *factura, *valor).await?;
        }
    }
    egreso.motivo_anulacion = Some(texto("motivo", motivo, 500)?);
    tx.actualizar_egreso(&egreso).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_EGRESO_ANULAR,
        "egreso",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(egreso)
}

/// Egreso por id.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::NoEncontrado`].
pub async fn egreso<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: EgresoId,
) -> AppResult<Egreso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    egreso_existente(&mut tx, id).await
}

/// Egresos (del estado, si se da) por fecha descendente (a lo sumo [`LIMITE_LISTADO`]).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn egresos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    estado: Option<EstadoEgreso>,
) -> AppResult<Vec<Egreso>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    tx.egresos(estado, LIMITE_LISTADO).await
}

// ---------------------------------------------------------------------------------------------
// Pasarela de pagos (fase 4): el adaptador verifica la firma y entrega el evento ya validado.
// ---------------------------------------------------------------------------------------------

/// Evento de la pasarela con la firma ya verificada por su adaptador.
#[derive(Debug, Clone)]
pub struct EventoPasarelaVerificado {
    pub proveedor: String,
    pub evento_externo: String,
    pub referencia: String,
    pub estado: EstadoEventoPasarela,
    pub valor: Monto,
    pub comision: Monto,
    pub retenciones: Monto,
    /// Cuenta de tesorería donde liquida la pasarela.
    pub cuenta_tesoreria: CuentaTesoreriaId,
    pub persona: Option<PersonaId>,
    pub ocurrido_en: DateTime<Utc>,
}

/// Registra un evento de la pasarela **una sola vez** (un reintento devuelve el ya guardado):
/// `aprobado` crea el recibo (uno por referencia), `reversado` o `contracargo` lo anulan, y
/// `liquidado` solo queda registrado (la disponibilidad se confirma al conciliar).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.recibos.registrar`; [`AppError::Validacion`] si el
/// proveedor, el id o la referencia son inválidos; los de registrar o anular el recibo.
pub async fn registrar_evento_pasarela<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    ev: EventoPasarelaVerificado,
) -> AppResult<EventoPasarela>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let proveedor = ev.proveedor.trim().to_owned();
    let valido = (2..=40).contains(&proveedor.len())
        && proveedor.starts_with(|c: char| c.is_ascii_lowercase())
        && proveedor
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valido {
        return Err(validacion("código de proveedor inválido"));
    }
    let externo = texto("evento_externo", &ev.evento_externo, 120)?;
    let referencia = texto("referencia", &ev.referencia, 120)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaRecibosRegistrar).await?;
    if let Some(ya) = tx.evento_pasarela(&proveedor, &externo).await? {
        return Ok(ya);
    }
    let existente = tx.recibo_de_referencia(&proveedor, &referencia).await?;
    let recibo_id = match ev.estado {
        EstadoEventoPasarela::Aprobado => match existente {
            Some(r) => Some(r),
            None => {
                let r = registrar_recibo_en(
                    &mut tx,
                    reloj,
                    ctx,
                    NuevoRecibo {
                        cuenta_tesoreria: ev.cuenta_tesoreria,
                        persona: ev.persona,
                        fecha: ev.ocurrido_en.date_naive(),
                        valor: ev.valor,
                        medio: MedioRecibo::Pasarela,
                        referencia: Some(referencia.chars().take(80).collect()),
                        descripcion: format!("Pago en línea ({proveedor})"),
                        soporte_archivo_id: None,
                        aplicaciones: Vec::new(),
                    },
                )
                .await?;
                Some(r.id)
            }
        },
        e if e.deshace_pago() => {
            if let Some(r) = existente {
                let vigente = tx
                    .recibo(r)
                    .await?
                    .is_some_and(|r| r.estado == EstadoRecibo::Registrado);
                if vigente {
                    let motivo = format!("Pago {} por la pasarela", e.as_str());
                    anular_recibo_en(&mut tx, reloj, ctx, r, &motivo).await?;
                }
            }
            existente
        }
        _ => existente,
    };
    let evento = EventoPasarela {
        id: EventoPasarelaId::nuevo(),
        proveedor,
        evento_externo: externo,
        referencia,
        estado: ev.estado,
        valor: ev.valor,
        comision: ev.comision,
        retenciones: ev.retenciones,
        cuenta_tesoreria_id: ev.cuenta_tesoreria,
        persona_id: ev.persona,
        recibo_id,
        ocurrido_en: ev.ocurrido_en,
        recibido_en: reloj.ahora(),
    };
    tx.insertar_evento_pasarela(&evento).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("estado", evento.estado.as_str())?
        .con("proveedor", evento.proveedor.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PASARELA, "evento_pasarela", evento.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(evento)
}
