//! Recaudo y cartera por bien privado (plan 14 fase 4): recaudo con imputación según la
//! política y descuento por pronto pago, recaudo desde el extracto por referencia de pago,
//! aplicación de anticipos, cobros manuales, notas crédito, cartera inicial, estado de cuenta,
//! cartera por bien y cuadre del sub-libro contra el mayor.
//!
//! El recaudo es un recibo de caja de la tesorería (ADR 0018) con aplicaciones a obligaciones:
//! el saldo de cada obligación se reconstruye sumando aplicaciones y ajustes vigentes (QA-012).
//! Lo que sobra queda sin aplicar en el recibo (anticipo, sin tabla nueva).

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{
    AjusteObligacionId, ArchivoId, BienPrivadoId, ConceptoCobroId, CuentaContableId,
    CuentaTesoreriaId, ObligacionCobroId, Permiso, PersonaId, ReciboCajaId,
    contabilidad::{
        Cara, ImporteDocumento, Monto,
        tesoreria::{EstadoMovimientoExtracto, EstadoRecibo, MedioRecibo},
    },
    contabilidad_ph::{
        EstadoObligacion, OrigenObligacion, Pendiente, TipoAjuste, TipoConceptoCobro, imputar,
        periodo_de, repartir,
    },
};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::configuracion::politica_en;
use super::{TxCartera, abrir, eventos, monto, validacion};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::contabilidad::{
        comprobantes::{self, DocumentoOrigen, revertir_en},
        conciliacion::recibo_de_movimiento_en,
        prelude::*,
        tesoreria::{
            AplicacionesExternas, NuevoRecibo, anular_recibo_en, conceptos, origen,
            registrar_recibo_con,
        },
        texto,
    },
    puertos::{
        Reloj, RepositorioCartera as _, UnidadDeTrabajo,
        cartera::{
            AjusteObligacion, BienCartera, FiltroObligaciones, ObligacionCobro, PagoAplicado,
            SaldoObligacion,
        },
        contabilidad::AgrupacionSaldos,
        tesoreria::{AplicacionPago, ReciboCaja},
    },
};

const ACCION_RECAUDO: AccionAuditoria = AccionAuditoria::new("cartera.recaudo.registrar");
const ACCION_RECAUDO_ANULAR: AccionAuditoria = AccionAuditoria::new("cartera.recaudo.anular");
const ACCION_APLICAR: AccionAuditoria = AccionAuditoria::new("cartera.recaudo.aplicar");
const ACCION_COBRO: AccionAuditoria = AccionAuditoria::new("cartera.cobro.registrar");
const ACCION_COBRO_ANULAR: AccionAuditoria = AccionAuditoria::new("cartera.cobro.anular");
const ACCION_NOTA: AccionAuditoria = AccionAuditoria::new("cartera.nota_credito.registrar");
const ACCION_INICIAL: AccionAuditoria = AccionAuditoria::new("cartera.inicial.cargar");

async fn bien_cartera<T: TxCartera>(tx: &mut T, id: BienPrivadoId) -> AppResult<BienCartera> {
    tx.bienes_cartera()
        .await?
        .into_iter()
        .find(|b| b.id == id)
        .ok_or(AppError::NoEncontrado("bien privado"))
}

async fn pendientes_de<T: TxCartera>(
    tx: &mut T,
    bien: BienPrivadoId,
) -> AppResult<Vec<SaldoObligacion>> {
    tx.saldos_obligaciones(&FiltroObligaciones {
        bien: Some(bien),
        con_saldo: true,
        ..FiltroObligaciones::default()
    })
    .await
}

fn a_pendiente(s: &SaldoObligacion) -> Pendiente {
    Pendiente {
        id: s.obligacion.id,
        periodo: s.obligacion.periodo,
        es_interes: s.concepto_tipo == TipoConceptoCobro::InteresMora,
        saldo: s.saldo,
    }
}

/// Un importe por concepto contable y deudor de lo aplicado (crédito a la cartera).
fn importes_aplicados(
    aplicado: &[(ObligacionCobroId, Monto)],
    obligaciones: &[SaldoObligacion],
) -> AppResult<Vec<ImporteDocumento>> {
    let mut agrupado: BTreeMap<(String, PersonaId), Decimal> = BTreeMap::new();
    for (id, valor) in aplicado {
        let o = obligaciones
            .iter()
            .find(|o| o.obligacion.id == *id)
            .ok_or(AppError::NoEncontrado("obligación"))?;
        *agrupado
            .entry((o.concepto_contable.clone(), o.obligacion.deudor_persona_id))
            .or_default() += valor.valor();
    }
    agrupado
        .into_iter()
        .map(|((concepto, deudor), valor)| {
            Ok(ImporteDocumento::new(&concepto, monto(valor)?).con_persona(Some(deudor)))
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Recaudo
// ---------------------------------------------------------------------------------------------

/// Pago recibido para un bien privado.
#[derive(Debug, Clone)]
pub struct NuevoRecaudo {
    pub bien: BienPrivadoId,
    pub cuenta_tesoreria: CuentaTesoreriaId,
    pub fecha: NaiveDate,
    pub valor: Monto,
    pub medio: MedioRecibo,
    pub referencia: Option<String>,
    pub descripcion: Option<String>,
    pub soporte_archivo_id: Option<ArchivoId>,
    /// Obligaciones que el pagador quiere cubrir primero (solo si la política lo permite).
    pub elegidas: Vec<ObligacionCobroId>,
}

/// Lo que dejó un recaudo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recaudo {
    pub bien_id: BienPrivadoId,
    pub recibo: ReciboCaja,
    pub aplicado: Vec<(ObligacionCobroId, Monto)>,
    /// Lo que quedó sin aplicar en el recibo.
    pub anticipo: Decimal,
    /// Descuento por pronto pago ganado.
    pub descuento: Option<Monto>,
}

/// Datos del recibo del recaudo (manual o desde el extracto).
struct DatosRecibo {
    cuenta_tesoreria: CuentaTesoreriaId,
    fecha: NaiveDate,
    valor: Monto,
    medio: MedioRecibo,
    referencia: Option<String>,
    descripcion: String,
    soporte_archivo_id: Option<ArchivoId>,
}

/// Descuento por pronto pago que gana el pago: el de la última cuenta de cobro del bien, si el
/// pago llega a tiempo, cubre todo lo que se debe menos el descuento y no se había concedido.
/// Devuelve cuánto se descuenta de cada obligación ordinaria de esa cuenta.
async fn pronto_pago<T: TxCartera>(
    tx: &mut T,
    bien: BienPrivadoId,
    fecha: NaiveDate,
    valor: Monto,
    pendientes: &[SaldoObligacion],
) -> AppResult<Vec<(ObligacionCobroId, Monto)>> {
    let Some(cuenta) = tx
        .cuentas_cobro(None, Some(bien))
        .await?
        .into_iter()
        .max_by_key(|c| (c.periodo, c.numero))
    else {
        return Ok(Vec::new());
    };
    let (Some(hasta), Some(descuento)) = (cuenta.pronto_pago_hasta, cuenta.descuento_pronto_pago)
    else {
        return Ok(Vec::new());
    };
    let deuda: Decimal = pendientes.iter().map(|p| p.saldo).sum();
    if fecha > hasta || valor.valor() < deuda - descuento.valor() {
        return Ok(Vec::new());
    }
    let ordinarias: Vec<&SaldoObligacion> = pendientes
        .iter()
        .filter(|p| {
            p.obligacion.cuenta_cobro_id == Some(cuenta.id)
                && p.concepto_tipo == TipoConceptoCobro::Ordinaria
        })
        .collect();
    // Concedido ya (o algo pagado de la cuenta): sin descuento.
    if ordinarias.is_empty()
        || ordinarias
            .iter()
            .any(|p| !p.ajustes.is_zero() || !p.aplicado.is_zero())
    {
        return Ok(Vec::new());
    }
    let pesos: Vec<(ObligacionCobroId, Decimal)> = ordinarias
        .iter()
        .map(|p| (p.obligacion.id, p.obligacion.valor.valor()))
        .collect();
    repartir(descuento.valor(), &pesos, Decimal::new(1, 2))?
        .into_iter()
        .filter(|(_, v)| *v > Decimal::ZERO)
        .map(|(id, v)| Ok((id, monto(v)?)))
        .collect()
}

/// Registra el recaudo en la transacción: pronto pago, imputación, recibo (con su comprobante
/// del evento [`eventos::RECAUDO`]) y, si viene del extracto, la conciliación del movimiento.
async fn recaudar_en<T: TxCartera>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: &BienCartera,
    datos: DatosRecibo,
    elegidas: &[ObligacionCobroId],
    movimiento: Option<domain::MovimientoExtractoId>,
) -> AppResult<Recaudo> {
    let pagador = bien
        .pagador()
        .ok_or_else(|| validacion("el bien no tiene propietario representante ni pagador"))?;
    let politica = politica_en(tx, datos.fecha).await?;
    if !elegidas.is_empty() && !politica.parametros.permite_elegir_obligacion {
        return Err(validacion(
            "la política de cartera no permite elegir qué obligación se paga",
        ));
    }
    let mut pendientes = pendientes_de(tx, bien.id).await?;
    let descuentos = pronto_pago(tx, bien.id, datos.fecha, datos.valor, &pendientes).await?;
    for (id, d) in &descuentos {
        if let Some(p) = pendientes.iter_mut().find(|p| p.obligacion.id == *id) {
            p.saldo -= d.valor();
        }
    }
    let candidatos: Vec<Pendiente> = pendientes.iter().map(a_pendiente).collect();
    let (imputado, sobra) = imputar(
        datos.valor.valor(),
        &candidatos,
        politica.parametros.imputacion,
        elegidas,
    );
    let aplicado: Vec<(ObligacionCobroId, Monto)> = imputado
        .into_iter()
        .map(|(id, v)| Ok((id, monto(v)?)))
        .collect::<AppResult<_>>()?;
    // El descuento se contabiliza antes del recibo (su comprobante es el del ajuste).
    let comprobante_descuento = if descuentos.is_empty() {
        None
    } else {
        Some(
            comprobantes::contabilizar_documento(
                tx,
                reloj,
                ctx,
                DocumentoOrigen {
                    evento: eventos::PRONTO_PAGO.into(),
                    origen: origen(eventos::PRONTO_PAGO, Uuid::now_v7()),
                    fecha: datos.fecha,
                    descripcion: format!(
                        "Descuento por pronto pago · bien {}",
                        bien.codigo.as_str()
                    ),
                    importes: importes_aplicados(&descuentos, &pendientes)?,
                },
            )
            .await?,
        )
    };
    let externas = AplicacionesExternas {
        evento: Some(eventos::RECAUDO),
        importes: importes_aplicados(&aplicado, &pendientes)?,
        aplicaciones: aplicado.clone(),
    };
    let recibo = match movimiento {
        None => {
            registrar_recibo_con(
                tx,
                reloj,
                ctx,
                NuevoRecibo {
                    cuenta_tesoreria: datos.cuenta_tesoreria,
                    persona: Some(pagador),
                    fecha: datos.fecha,
                    valor: datos.valor,
                    medio: datos.medio,
                    referencia: datos.referencia,
                    descripcion: datos.descripcion,
                    soporte_archivo_id: datos.soporte_archivo_id,
                    aplicaciones: Vec::new(),
                },
                externas,
            )
            .await?
        }
        Some(m) => {
            recibo_de_movimiento_en(
                tx,
                reloj,
                ctx,
                m,
                Some(pagador),
                &datos.descripcion,
                externas,
            )
            .await?
        }
    };
    let descuento = match comprobante_descuento {
        None => None,
        Some(comprobante_id) => {
            for (obligacion, valor) in &descuentos {
                tx.insertar_ajuste(&AjusteObligacion {
                    id: AjusteObligacionId::nuevo(),
                    obligacion_id: *obligacion,
                    tipo: TipoAjuste::ProntoPago,
                    valor: *valor,
                    motivo: "Descuento por pronto pago".into(),
                    comprobante_id,
                    recibo_id: Some(recibo.id),
                    creado_por: ctx.actor_id,
                    creado_en: reloj.ahora(),
                    anulado_en: None,
                })
                .await?;
            }
            Some(descuentos.iter().map(|d| d.1).sum())
        }
    };
    let detalle = DetalleAuditoria::vacio()
        .con("bien_privado_id", bien.id.to_string())?
        .con("valor", recibo.valor.valor().to_string())?
        .con("obligaciones", aplicado.len())?
        .con("anticipo", sobra.to_string())?
        .con("pronto_pago", descuento.is_some())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RECAUDO, "recibo_caja", recibo.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    Ok(Recaudo {
        bien_id: bien.id,
        recibo,
        aplicado,
        anticipo: sobra,
        descuento,
    })
}

/// Registra un pago para un bien privado: el recibo de caja (pagador = tenedor o propietario
/// representante) se imputa a sus obligaciones según la política (intereses antes que capital
/// salvo consentimiento); si llega a tiempo y cubre la deuda, gana el pronto pago; lo que sobra
/// queda como anticipo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.recibos.registrar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] sin política vigente, sin pagador o si se eligen obligaciones sin
/// permitirlo; los del recibo y del contabilizador.
pub async fn recaudar<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoRecaudo,
) -> AppResult<Recaudo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaRecibosRegistrar).await?;
    let bien = bien_cartera(&mut tx, nuevo.bien).await?;
    let descripcion = nuevo
        .descripcion
        .unwrap_or_else(|| format!("Pago del bien {}", bien.codigo.as_str()));
    let recaudo = recaudar_en(
        &mut tx,
        reloj,
        ctx,
        &bien,
        DatosRecibo {
            cuenta_tesoreria: nuevo.cuenta_tesoreria,
            fecha: nuevo.fecha,
            valor: nuevo.valor,
            medio: nuevo.medio,
            referencia: nuevo.referencia,
            descripcion,
            soporte_archivo_id: nuevo.soporte_archivo_id,
        },
        &nuevo.elegidas,
        None,
    )
    .await?;
    tx.confirmar().await?;
    Ok(recaudo)
}

/// Resultado del recaudo desde el extracto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecaudoExtracto {
    pub recaudos: Vec<Recaudo>,
    /// Créditos pendientes cuya referencia no es de ningún bien (quedan por identificar).
    pub sin_identificar: usize,
}

/// Referencia sin ceros a la izquierda (Asobancaria rellena con ceros).
fn referencia_normalizada(r: &str) -> String {
    let limpio = r.trim().trim_start_matches('0');
    if limpio.is_empty() {
        "0".into()
    } else {
        limpio.into()
    }
}

/// Recauda los créditos pendientes del extracto de la cuenta entre dos fechas cuya referencia
/// es la referencia de pago de un bien (archivo Asobancaria o extracto con referencia): cada
/// uno se vuelve un recaudo del bien conciliado con su movimiento.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.conciliar` o sin `tesoreria.recibos.registrar`; los
/// de [`recaudar`].
pub async fn recaudar_por_referencia<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    cuenta: CuentaTesoreriaId,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<RecaudoExtracto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    ctx.exigir(Permiso::TesoreriaRecibosRegistrar)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaConciliar).await?;
    let bienes = tx.bienes_cartera().await?;
    let por_referencia: BTreeMap<String, &BienCartera> = bienes
        .iter()
        .filter_map(|b| {
            b.referencia_pago
                .as_deref()
                .map(|r| (referencia_normalizada(r), b))
        })
        .collect();
    let mut resultado = RecaudoExtracto {
        recaudos: Vec::new(),
        sin_identificar: 0,
    };
    let creditos: Vec<_> = tx
        .movimientos_extracto(
            cuenta,
            desde,
            hasta,
            Some(EstadoMovimientoExtracto::Pendiente),
        )
        .await?
        .into_iter()
        .filter(|m| m.valor > Decimal::ZERO)
        .collect();
    for m in creditos {
        let bien = m
            .referencia
            .as_deref()
            .and_then(|r| por_referencia.get(&referencia_normalizada(r)));
        let Some(bien) = bien else {
            resultado.sin_identificar += 1;
            continue;
        };
        let recaudo = recaudar_en(
            &mut tx,
            reloj,
            ctx,
            bien,
            DatosRecibo {
                cuenta_tesoreria: cuenta,
                fecha: m.fecha,
                valor: Monto::new(m.valor)?,
                medio: MedioRecibo::Consignacion,
                referencia: m.referencia.clone(),
                descripcion: format!("Recaudo bancario del bien {}", bien.codigo.as_str()),
                soporte_archivo_id: None,
            },
            &[],
            Some(m.id),
        )
        .await?;
        resultado.recaudos.push(recaudo);
    }
    tx.confirmar().await?;
    Ok(resultado)
}

/// Anula un recaudo: revierte el descuento por pronto pago que ganó y anula el recibo (revierte
/// su comprobante y sus aplicaciones; las obligaciones recuperan su saldo).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.recibos.registrar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya está anulado; los del reverso.
pub async fn anular_recaudo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    recibo: ReciboCajaId,
    motivo: &str,
) -> AppResult<ReciboCaja>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaRecibosRegistrar).await?;
    let ahora = reloj.ahora();
    let ajustes = tx.ajustes(Some(recibo), None).await?;
    let mut revertidos = Vec::new();
    for a in &ajustes {
        if !revertidos.contains(&a.comprobante_id) {
            revertir_en(&mut tx, reloj, ctx, a.comprobante_id, motivo, None).await?;
            revertidos.push(a.comprobante_id);
        }
        tx.anular_ajuste(a.id, ahora).await?;
    }
    let anulado = anular_recibo_en(&mut tx, reloj, ctx, recibo, motivo).await?;
    let detalle = DetalleAuditoria::vacio().con("ajustes_anulados", ajustes.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RECAUDO_ANULAR, "recibo_caja", recibo.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(anulado)
}

/// Aplica lo que un recibo tiene sin aplicar a las obligaciones del bien (con su comprobante del
/// evento [`eventos::APLICACION`]). Un recibo sin pagador queda identificado con el pagador del
/// bien. Devuelve lo aplicado (vacío si no había saldo o deuda).
pub(crate) async fn aplicar_recibo_a_bien_en<T: TxCartera>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    recibo_id: ReciboCajaId,
    bien_id: BienPrivadoId,
    fecha: NaiveDate,
) -> AppResult<Vec<(ObligacionCobroId, Monto)>> {
    let recibo = tx
        .recibo(recibo_id)
        .await?
        .ok_or(AppError::NoEncontrado("recibo de caja"))?;
    if recibo.estado != EstadoRecibo::Registrado {
        return Err(AppError::Conflicto("el recibo está anulado".into()));
    }
    let bien = bien_cartera(tx, bien_id).await?;
    let pagador = bien
        .pagador()
        .ok_or_else(|| validacion("el bien no tiene propietario representante ni pagador"))?;
    if recibo
        .persona_id
        .is_some_and(|p| p != pagador && Some(p) != bien.deudor_persona_id)
    {
        return Err(validacion("el recibo es de otra persona"));
    }
    let sin_aplicar = recibo.sin_aplicar();
    if sin_aplicar <= Decimal::ZERO {
        return Ok(Vec::new());
    }
    let politica = politica_en(tx, fecha).await?;
    let pendientes = pendientes_de(tx, bien_id).await?;
    let candidatos: Vec<Pendiente> = pendientes.iter().map(a_pendiente).collect();
    let (imputado, _) = imputar(
        sin_aplicar,
        &candidatos,
        politica.parametros.imputacion,
        &[],
    );
    if imputado.is_empty() {
        return Ok(Vec::new());
    }
    let aplicado: Vec<(ObligacionCobroId, Monto)> = imputado
        .into_iter()
        .map(|(id, v)| Ok((id, monto(v)?)))
        .collect::<AppResult<_>>()?;
    let total: Monto = aplicado.iter().map(|a| a.1).sum();
    let persona = recibo.persona_id.or(Some(pagador));
    let mut importes =
        vec![ImporteDocumento::new(conceptos::SIN_APLICAR, total).con_persona(persona)];
    importes.extend(importes_aplicados(&aplicado, &pendientes)?);
    let grupo = Uuid::now_v7();
    let comprobante = comprobantes::contabilizar_documento(
        tx,
        reloj,
        ctx,
        DocumentoOrigen {
            evento: eventos::APLICACION.into(),
            origen: origen(eventos::APLICACION, grupo),
            fecha,
            descripcion: format!("Aplicación de anticipo · bien {}", bien.codigo.as_str()),
            importes,
        },
    )
    .await?;
    let ahora = reloj.ahora();
    for (obligacion, valor) in &aplicado {
        tx.insertar_aplicacion(&AplicacionPago {
            id: domain::AplicacionPagoId::nuevo(),
            recibo_id,
            factura_venta_id: None,
            obligacion_id: Some(*obligacion),
            valor: *valor,
            aplicada_en: ahora,
            comprobante_id: Some(comprobante),
            anulada_en: None,
        })
        .await?;
    }
    if recibo.persona_id.is_none() {
        tx.identificar_recibo(recibo_id, pagador).await?;
    }
    let detalle = DetalleAuditoria::vacio()
        .con("bien_privado_id", bien_id.to_string())?
        .con("valor", total.valor().to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_APLICAR, "recibo_caja", recibo_id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    Ok(aplicado)
}

/// Aplica lo sin aplicar de un recibo (anticipo o recaudo por identificar) a las obligaciones de
/// un bien.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.recibos.registrar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si el recibo está anulado; [`AppError::Validacion`] si es de otra
/// persona o no había nada que aplicar.
pub async fn aplicar_recibo_a_bien<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    recibo: ReciboCajaId,
    bien: BienPrivadoId,
    fecha: Option<NaiveDate>,
) -> AppResult<Vec<(ObligacionCobroId, Monto)>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaRecibosRegistrar).await?;
    let fecha = fecha.unwrap_or_else(|| reloj.ahora().date_naive());
    let aplicado = aplicar_recibo_a_bien_en(&mut tx, reloj, ctx, recibo, bien, fecha).await?;
    if aplicado.is_empty() {
        return Err(validacion(
            "no hay nada que aplicar: el recibo no tiene saldo o el bien no debe",
        ));
    }
    tx.confirmar().await?;
    Ok(aplicado)
}

// ---------------------------------------------------------------------------------------------
// Cobros manuales y notas crédito
// ---------------------------------------------------------------------------------------------

/// Cobro manual a un bien (multa, uso de un bien común, daño, otro).
#[derive(Debug, Clone)]
pub struct NuevoCobro {
    pub bien: BienPrivadoId,
    pub concepto: ConceptoCobroId,
    pub valor: Monto,
    /// Fecha del cobro (su comprobante); el periodo es su mes.
    pub fecha: NaiveDate,
    pub vencimiento: NaiveDate,
    pub descripcion: String,
}

/// Registra y contabiliza un cobro manual (evento [`eventos::COBRO`]); sale en la siguiente
/// cuenta de cobro como parte del saldo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ajustar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si el concepto no admite cobros manuales, el valor es cero, el
/// vencimiento es anterior a la fecha o el bien no tiene propietario representante.
pub async fn registrar_cobro<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoCobro,
) -> AppResult<SaldoObligacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    if nuevo.valor.es_cero() || nuevo.vencimiento < nuevo.fecha {
        return Err(validacion(
            "el cobro necesita un valor mayor que cero y un vencimiento no anterior a su fecha",
        ));
    }
    let descripcion = texto("descripcion", &nuevo.descripcion, 300)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraAjustar).await?;
    let bien = bien_cartera(&mut tx, nuevo.bien).await?;
    let deudor = bien
        .deudor_persona_id
        .ok_or_else(|| validacion("el bien no tiene propietario representante"))?;
    let concepto = tx
        .conceptos_cobro()
        .await?
        .into_iter()
        .find(|c| c.id == nuevo.concepto && c.activo)
        .ok_or(AppError::NoEncontrado("concepto de cobro activo"))?;
    if !concepto.tipo.admite_cobro_manual() {
        return Err(validacion(
            "lo ordinario, el fondo y las cuotas extraordinarias solo salen de la liquidación",
        ));
    }
    let id = ObligacionCobroId::nuevo();
    let comprobante = comprobantes::contabilizar_documento(
        &mut tx,
        reloj,
        ctx,
        DocumentoOrigen {
            evento: eventos::COBRO.into(),
            origen: origen(eventos::COBRO, id.as_uuid()),
            fecha: nuevo.fecha,
            descripcion: format!("{} · bien {}", descripcion, bien.codigo.as_str()),
            importes: vec![
                ImporteDocumento::new(&concepto.concepto_contable, nuevo.valor)
                    .con_persona(Some(deudor)),
            ],
        },
    )
    .await?;
    let obligacion = ObligacionCobro {
        id,
        bien_privado_id: bien.id,
        concepto_cobro_id: concepto.id,
        periodo: periodo_de(nuevo.fecha),
        valor: nuevo.valor,
        vencimiento: nuevo.vencimiento,
        deudor_persona_id: deudor,
        origen: OrigenObligacion::Manual,
        liquidacion_id: None,
        cuota_extraordinaria_id: None,
        cuenta_cobro_id: None,
        comprobante_id: Some(comprobante),
        causacion_id: None,
        descripcion: Some(descripcion),
        estado: EstadoObligacion::Vigente,
        creada_en: reloj.ahora(),
        anulada_en: None,
    };
    tx.insertar_obligaciones(std::slice::from_ref(&obligacion))
        .await?;
    let detalle = DetalleAuditoria::vacio()
        .con("bien_privado_id", bien.id.to_string())?
        .con("concepto", concepto.codigo.as_str())?
        .con("valor", nuevo.valor.valor().to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_COBRO, "obligacion", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    let saldo = tx
        .obligacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("obligación"))?;
    tx.confirmar().await?;
    Ok(saldo)
}

/// Anula un cobro manual sin pagos ni ajustes (revierte su comprobante).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ajustar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si no es manual, ya está anulado o tiene pagos o ajustes.
pub async fn anular_cobro<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ObligacionCobroId,
    motivo: &str,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraAjustar).await?;
    let s = tx
        .obligacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("obligación"))?;
    let comprobante = match (s.obligacion.origen, s.obligacion.comprobante_id) {
        (OrigenObligacion::Manual, Some(c)) => c,
        _ => {
            return Err(AppError::Conflicto(
                "solo se anula un cobro manual; lo liquidado se corrige con una nota crédito"
                    .into(),
            ));
        }
    };
    if s.obligacion.estado == EstadoObligacion::Anulada {
        return Err(AppError::Conflicto("el cobro ya está anulado".into()));
    }
    if !s.aplicado.is_zero() || !s.ajustes.is_zero() {
        return Err(AppError::Conflicto(
            "el cobro tiene pagos o ajustes: corríjalo con una nota crédito".into(),
        ));
    }
    revertir_en(&mut tx, reloj, ctx, comprobante, motivo, None).await?;
    tx.anular_obligacion(id, reloj.ahora()).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_COBRO_ANULAR,
        "obligacion",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Registra una nota crédito sobre una obligación (corrección o condonación con motivo): reduce
/// su saldo con un comprobante del evento [`eventos::NOTA_CREDITO`]. Es la forma de corregir lo
/// ya liquidado (nunca se edita).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ajustar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si el valor es cero o supera el saldo, o el motivo es muy corto.
pub async fn registrar_nota_credito<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    obligacion: ObligacionCobroId,
    valor: Monto,
    motivo: &str,
    fecha: Option<NaiveDate>,
) -> AppResult<AjusteObligacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let motivo = texto("motivo", motivo, 500)?;
    if motivo.chars().count() < 5 {
        return Err(validacion("el motivo tiene al menos 5 caracteres"));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraAjustar).await?;
    let s = tx
        .obligacion(obligacion)
        .await?
        .ok_or(AppError::NoEncontrado("obligación"))?;
    if s.obligacion.estado != EstadoObligacion::Vigente
        || valor.es_cero()
        || valor.valor() > s.saldo
    {
        return Err(validacion(format!(
            "la nota crédito debe ser mayor que cero y a lo sumo el saldo ({})",
            s.saldo
        )));
    }
    let id = AjusteObligacionId::nuevo();
    let comprobante_id = comprobantes::contabilizar_documento(
        &mut tx,
        reloj,
        ctx,
        DocumentoOrigen {
            evento: eventos::NOTA_CREDITO.into(),
            origen: origen(eventos::NOTA_CREDITO, id.as_uuid()),
            fecha: fecha.unwrap_or_else(|| reloj.ahora().date_naive()),
            descripcion: format!("Nota crédito · {motivo}"),
            importes: vec![
                ImporteDocumento::new(&s.concepto_contable, valor)
                    .con_persona(Some(s.obligacion.deudor_persona_id)),
            ],
        },
    )
    .await?;
    let ajuste = AjusteObligacion {
        id,
        obligacion_id: obligacion,
        tipo: TipoAjuste::NotaCredito,
        valor,
        motivo,
        comprobante_id,
        recibo_id: None,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
        anulado_en: None,
    };
    tx.insertar_ajuste(&ajuste).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("obligacion_id", obligacion.to_string())?
        .con("valor", valor.valor().to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_NOTA, "ajuste_obligacion", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(ajuste)
}

// ---------------------------------------------------------------------------------------------
// Cartera inicial
// ---------------------------------------------------------------------------------------------

/// Fila de la cartera inicial: deuda de un bien por concepto y antigüedad.
#[derive(Debug, Clone)]
pub struct FilaCarteraInicial {
    /// Código del bien privado.
    pub bien: String,
    /// Código del concepto de cobro.
    pub concepto: String,
    pub periodo: NaiveDate,
    pub valor: Monto,
    pub vencimiento: NaiveDate,
}

/// Resultado de validar o cargar la cartera inicial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarteraInicial {
    pub filas: usize,
    pub total: Decimal,
    /// Errores por fila (número de fila desde 1); con errores no se carga nada.
    pub errores: Vec<(usize, String)>,
    pub cargada: bool,
}

/// Carga la cartera inicial por bien, concepto y antigüedad (obligaciones de origen
/// `saldo_inicial`, sin comprobante: su contabilidad está en los saldos iniciales del núcleo y
/// el cuadre lo muestra [`cuadre_cartera`]). Con `solo_validar` o con errores no guarda nada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ajustar`; [`AppError::Validacion`] si no trae filas.
pub async fn cargar_cartera_inicial<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filas: Vec<FilaCarteraInicial>,
    solo_validar: bool,
) -> AppResult<CarteraInicial>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    if filas.is_empty() {
        return Err(validacion("la cartera inicial no trae filas"));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraAjustar).await?;
    let bienes = tx.bienes_cartera().await?;
    let conceptos = tx.conceptos_cobro().await?;
    let ahora = reloj.ahora();
    let mut errores = Vec::new();
    let mut obligaciones = Vec::with_capacity(filas.len());
    for (i, f) in filas.iter().enumerate() {
        let n = i + 1;
        let Some(bien) = bienes.iter().find(|b| b.codigo.as_str() == f.bien.trim()) else {
            errores.push((n, "el bien no existe".into()));
            continue;
        };
        let Some(deudor) = bien.deudor_persona_id else {
            errores.push((n, "el bien no tiene propietario representante".into()));
            continue;
        };
        let Some(concepto) = conceptos.iter().find(|c| c.codigo == f.concepto.trim()) else {
            errores.push((n, "el concepto de cobro no existe".into()));
            continue;
        };
        if f.valor.es_cero() {
            errores.push((n, "el valor debe ser mayor que cero".into()));
            continue;
        }
        obligaciones.push(ObligacionCobro {
            id: ObligacionCobroId::nuevo(),
            bien_privado_id: bien.id,
            concepto_cobro_id: concepto.id,
            periodo: periodo_de(f.periodo),
            valor: f.valor,
            vencimiento: f.vencimiento,
            deudor_persona_id: deudor,
            origen: OrigenObligacion::SaldoInicial,
            liquidacion_id: None,
            cuota_extraordinaria_id: None,
            cuenta_cobro_id: None,
            comprobante_id: None,
            causacion_id: None,
            descripcion: Some("Cartera inicial".into()),
            estado: EstadoObligacion::Vigente,
            creada_en: ahora,
            anulada_en: None,
        });
    }
    let total = obligaciones.iter().map(|o| o.valor.valor()).sum();
    let cargar = errores.is_empty() && !solo_validar;
    if cargar {
        tx.insertar_obligaciones(&obligaciones).await?;
        let detalle = DetalleAuditoria::vacio()
            .con("filas", obligaciones.len())?
            .con("total", Decimal::to_string(&total))?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_INICIAL, "cartera", ctx.tenant_id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
        tx.confirmar().await?;
    }
    Ok(CarteraInicial {
        filas: filas.len(),
        total,
        errores,
        cargada: cargar,
    })
}

// ---------------------------------------------------------------------------------------------
// Consultas
// ---------------------------------------------------------------------------------------------

/// Estado de cuenta de un bien (consulta en cualquier fecha; no es una tabla).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstadoCuenta {
    pub bien: BienCartera,
    pub corte: NaiveDate,
    /// Obligaciones hasta el periodo del corte, con su saldo actual.
    pub obligaciones: Vec<SaldoObligacion>,
    pub pagos: Vec<PagoAplicado>,
    pub saldo: Decimal,
    /// Saldo de lo vencido al corte.
    pub vencido: Decimal,
    /// Dinero sin aplicar de los recibos del pagador y del deudor.
    pub anticipos: Decimal,
}

/// Estado de cuenta de un bien al corte (por defecto, hoy).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`; [`AppError::NoEncontrado`].
pub async fn estado_cuenta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    corte: Option<NaiveDate>,
) -> AppResult<EstadoCuenta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let corte = corte.unwrap_or_else(|| reloj.ahora().date_naive());
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
    estado_cuenta_en(&mut tx, bien, corte).await
}

/// Estado de cuenta en la transacción (sin exigir permiso: lo usa también el portal del
/// propietario, que exige ser titular o pagador).
pub(crate) async fn estado_cuenta_en<T: TxCartera>(
    tx: &mut T,
    bien: BienPrivadoId,
    corte: NaiveDate,
) -> AppResult<EstadoCuenta> {
    let b = bien_cartera(tx, bien).await?;
    let obligaciones: Vec<SaldoObligacion> = tx
        .saldos_obligaciones(&FiltroObligaciones {
            bien: Some(bien),
            hasta_periodo: Some(periodo_de(corte)),
            ..FiltroObligaciones::default()
        })
        .await?
        .into_iter()
        .filter(|o| o.obligacion.estado == EstadoObligacion::Vigente)
        .collect();
    let pagos = tx.pagos_aplicados(bien).await?;
    let personas: Vec<PersonaId> = [b.pagador_persona_id, b.deudor_persona_id]
        .into_iter()
        .flatten()
        .collect();
    let anticipos = tx
        .anticipos(&personas)
        .await?
        .iter()
        .map(|a| a.sin_aplicar)
        .sum();
    let saldo = obligaciones.iter().map(|o| o.saldo).sum();
    let vencido = obligaciones
        .iter()
        .filter(|o| o.obligacion.vencimiento < corte)
        .map(|o| o.saldo)
        .sum();
    Ok(EstadoCuenta {
        bien: b,
        corte,
        obligaciones,
        pagos,
        saldo,
        vencido,
        anticipos,
    })
}

/// Saldo de un bien en la cartera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaldoBien {
    pub bien_id: BienPrivadoId,
    pub codigo: String,
    pub deudor_persona_id: Option<PersonaId>,
    pub saldo: Decimal,
    pub vencido: Decimal,
}

/// Cartera por bien al corte (solo los bienes con saldo).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn cartera_por_bien<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    corte: Option<NaiveDate>,
) -> AppResult<Vec<SaldoBien>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let corte = corte.unwrap_or_else(|| reloj.ahora().date_naive());
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
    let bienes = tx.bienes_cartera().await?;
    let mut saldos: BTreeMap<BienPrivadoId, (Decimal, Decimal)> = BTreeMap::new();
    for o in tx
        .saldos_obligaciones(&FiltroObligaciones {
            con_saldo: true,
            hasta_periodo: Some(periodo_de(corte)),
            ..FiltroObligaciones::default()
        })
        .await?
    {
        let e = saldos.entry(o.obligacion.bien_privado_id).or_default();
        e.0 += o.saldo;
        if o.obligacion.vencimiento < corte {
            e.1 += o.saldo;
        }
    }
    Ok(bienes
        .into_iter()
        .filter_map(|b| {
            saldos.get(&b.id).map(|(saldo, vencido)| SaldoBien {
                bien_id: b.id,
                codigo: b.codigo.as_str().to_owned(),
                deudor_persona_id: b.deudor_persona_id,
                saldo: *saldo,
                vencido: *vencido,
            })
        })
        .collect())
}

/// Cuadre de una cuenta de cartera: el mayor contra el sub-libro de obligaciones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuadreCuenta {
    pub cuenta_id: CuentaContableId,
    pub codigo: String,
    pub nombre: String,
    pub saldo_mayor: Decimal,
    pub saldo_cartera: Decimal,
    /// Mayor − cartera.
    pub diferencia: Decimal,
}

/// Cuadre de la cartera contra el mayor con todo lo contabilizado (también lo fechado después
/// de hoy, como una liquidación anticipada): las cuentas de cartera son las que debitan las
/// reglas vigentes de [`eventos::LIQUIDACION`], [`eventos::COBRO`] e [`eventos::INTERESES`] para
/// cada concepto contable; el saldo de cada una en el
/// mayor se compara con la suma de los saldos de las obligaciones de sus conceptos (incluida la
/// cartera inicial).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`; [`AppError::Validacion`] si no hay regla vigente
/// para la liquidación.
pub async fn cuadre_cartera<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<CuadreCuenta>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let hoy = reloj.ahora().date_naive();
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
    let mut cuenta_de: BTreeMap<String, CuentaContableId> = BTreeMap::new();
    let mut hay_regla = false;
    for evento in [eventos::LIQUIDACION, eventos::COBRO, eventos::INTERESES] {
        if let Some(regla) = tx.regla_vigente(evento, hoy).await? {
            hay_regla = true;
            for l in regla.regla.lineas.iter().filter(|l| l.cara == Cara::Debito) {
                cuenta_de.entry(l.concepto.clone()).or_insert(l.cuenta);
            }
        }
    }
    if !hay_regla {
        return Err(validacion(format!(
            "no hay una regla vigente para `{}`",
            eventos::LIQUIDACION
        )));
    }
    let mut cartera: BTreeMap<CuentaContableId, Decimal> =
        cuenta_de.values().map(|c| (*c, Decimal::ZERO)).collect();
    for o in tx
        .saldos_obligaciones(&FiltroObligaciones {
            con_saldo: true,
            ..FiltroObligaciones::default()
        })
        .await?
    {
        if let Some(c) = cuenta_de.get(&o.concepto_contable) {
            *cartera.entry(*c).or_default() += o.saldo;
        }
    }
    let fin = NaiveDate::from_ymd_opt(2100, 12, 31).unwrap_or(NaiveDate::MAX);
    let saldos = tx.saldos(fin, fin, AgrupacionSaldos::default()).await?;
    let ids: Vec<CuentaContableId> = cartera.keys().copied().collect();
    let cuentas = tx.cuentas_por_id(&ids).await?;
    Ok(cuentas
        .into_iter()
        .map(|c| {
            let saldo_mayor = saldos
                .iter()
                .filter(|s| s.cuenta_id == c.id)
                .map(|s| s.saldo_final())
                .sum::<Decimal>();
            let saldo_cartera = cartera.get(&c.id).copied().unwrap_or_default();
            CuadreCuenta {
                cuenta_id: c.id,
                codigo: c.codigo.as_str().to_owned(),
                nombre: c.nombre,
                saldo_mayor,
                saldo_cartera,
                diferencia: saldo_mayor - saldo_cartera,
            }
        })
        .collect())
}
