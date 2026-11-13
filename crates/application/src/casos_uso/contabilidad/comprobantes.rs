//! Comprobantes: borradores, previsualización, contabilización, reverso, el **contabilizador**
//! de documentos origen, saldos iniciales y cierre anual.
//!
//! Todo asiento que llega a la base de datos pasó por [`AsientoBalanceado`] y por
//! `validar_contra` del plan de cuentas; la base de datos lo vuelve a verificar (periodo
//! abierto, cuentas, partida doble). Un comprobante contabilizado no se edita: se revierte.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{
    CentroCostoId, ComprobanteId, CuentaContableId, Permiso, PersonaId,
    contabilidad::{
        AsientoBalanceado, CodigoCuenta, EstadoComprobante, EstadoPeriodo, ImporteDocumento,
        LineaAsiento, Monto, PeriodoContable,
        estados::{SaldoResultado, asiento_de_cierre},
    },
};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::prelude::*;
use super::{TxContable, exigir_contabilidad, texto};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        Reloj, UnidadDeTrabajo,
        contabilidad::{
            AgrupacionSaldos, Comprobante, CuentaContable, FiltroComprobantes, Origen,
            TipoComprobante,
        },
    },
};

const ACCION_BORRADOR: AccionAuditoria = AccionAuditoria::new("contabilidad.comprobante.borrador");
const ACCION_CONTABILIZAR: AccionAuditoria =
    AccionAuditoria::new("contabilidad.comprobante.contabilizar");
const ACCION_REVERTIR: AccionAuditoria = AccionAuditoria::new("contabilidad.comprobante.revertir");

/// Origen de los saldos iniciales.
pub const ORIGEN_SALDOS_INICIALES: &str = "contabilidad.saldos_iniciales";
/// Origen del cierre anual.
pub const ORIGEN_CIERRE_ANUAL: &str = "contabilidad.cierre_anual";
/// Máximo de comprobantes en un listado.
pub const LIMITE_LISTADO: u32 = 500;

/// Línea de entrada (por códigos, como la escribe el contador).
#[derive(Debug, Clone)]
pub struct LineaEntrada {
    pub cuenta: CodigoCuenta,
    pub persona: Option<PersonaId>,
    /// Código del centro de costo.
    pub centro_costo: Option<String>,
    pub debito: Monto,
    pub credito: Monto,
    pub base: Option<Monto>,
    pub descripcion: Option<String>,
}

/// Comprobante manual en borrador.
#[derive(Debug, Clone)]
pub struct BorradorComprobante {
    /// Código del tipo de comprobante.
    pub tipo: String,
    pub fecha: NaiveDate,
    pub descripcion: String,
    pub lineas: Vec<LineaEntrada>,
}

/// Documento origen que un módulo entrega al contabilizador (la "API interna" del núcleo).
#[derive(Debug, Clone)]
pub struct DocumentoOrigen {
    /// Evento de la regla de contabilización (`ventas.factura`, `cartera.liquidacion`…).
    pub evento: String,
    /// Tipo e id del documento: un documento produce un solo comprobante.
    pub origen: Origen,
    pub fecha: NaiveDate,
    pub descripcion: String,
    pub importes: Vec<ImporteDocumento>,
}

fn error_validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

async fn abrir<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    permiso: Permiso,
) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    ctx.exigir(permiso)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_contabilidad(&mut tx, reloj).await?;
    Ok(tx)
}

async fn tipo_por_codigo<T: TxContable>(tx: &mut T, codigo: &str) -> AppResult<TipoComprobante> {
    tx.tipos_comprobante()
        .await?
        .into_iter()
        .find(|t| t.codigo == codigo && t.activo)
        .ok_or_else(|| {
            error_validacion(format!(
                "el tipo de comprobante {codigo} no existe o está inactivo"
            ))
        })
}

/// Convierte las líneas de entrada (códigos) en líneas de asiento (ids).
async fn resolver_lineas<T: TxContable>(
    tx: &mut T,
    entradas: Vec<LineaEntrada>,
) -> AppResult<Vec<LineaAsiento>> {
    let cuentas: BTreeMap<String, CuentaContableId> = tx
        .cuentas()
        .await?
        .into_iter()
        .map(|c| (c.codigo.as_str().to_owned(), c.id))
        .collect();
    let centros: BTreeMap<String, CentroCostoId> = tx
        .centros_costo()
        .await?
        .into_iter()
        .map(|c| (c.codigo, c.id))
        .collect();
    entradas
        .into_iter()
        .enumerate()
        .map(|(i, e)| {
            if e.debito.es_cero() == e.credito.es_cero() {
                return Err(error_validacion(format!(
                    "la línea {} debe tener valor en una sola cara (débito o crédito)",
                    i + 1
                )));
            }
            let cuenta = *cuentas
                .get(e.cuenta.as_str())
                .ok_or_else(|| error_validacion(format!("la cuenta {} no existe", e.cuenta)))?;
            let centro = match &e.centro_costo {
                None => None,
                Some(c) => Some(*centros.get(c).ok_or_else(|| {
                    error_validacion(format!("el centro de costo {c} no existe"))
                })?),
            };
            let descripcion = e
                .descripcion
                .map(|d| texto("descripcion", &d, 300))
                .transpose()?;
            Ok(LineaAsiento {
                cuenta,
                persona: e.persona,
                centro_costo: centro,
                debito: e.debito,
                credito: e.credito,
                base: e.base,
                descripcion,
            })
        })
        .collect()
}

/// Valida el asiento contra el plan de cuentas de la entidad.
async fn validar_cuentas<T: TxContable>(tx: &mut T, asiento: &AsientoBalanceado) -> AppResult<()> {
    let ids: Vec<CuentaContableId> = asiento.lineas().iter().map(|l| l.cuenta).collect();
    let cuentas: BTreeMap<CuentaContableId, CuentaContable> = tx
        .cuentas_por_id(&ids)
        .await?
        .into_iter()
        .map(|c| (c.id, c))
        .collect();
    asiento.validar_contra(|id| cuentas.get(&id).map(CuentaContable::reglas))?;
    Ok(())
}

/// Periodo abierto de la fecha (lo crea si falta), después del inicio de la contabilidad.
async fn periodo_abierto<T: TxContable>(
    tx: &mut T,
    periodo: PeriodoContable,
    fecha: NaiveDate,
) -> AppResult<crate::puertos::contabilidad::Periodo> {
    let inicio = tx.inicio_contabilidad().await?.ok_or_else(|| {
        error_validacion(
            "configure la contabilidad (marco técnico y fecha de inicio) antes de contabilizar",
        )
    })?;
    if fecha < inicio {
        return Err(error_validacion(format!(
            "la fecha {fecha} es anterior al inicio de la contabilidad ({inicio})"
        )));
    }
    if !periodo.contiene(fecha) {
        return Err(error_validacion("la fecha no corresponde al periodo"));
    }
    let p = tx.periodo_o_crear(periodo).await?;
    if p.estado != EstadoPeriodo::Abierto {
        return Err(domain::DominioError::ReglaContable(format!(
            "el periodo {}-{:02} está {}",
            periodo.anio,
            periodo.mes,
            p.estado.as_str()
        ))
        .into());
    }
    Ok(p)
}

/// Lo que hace falta para guardar y contabilizar un comprobante en una transacción.
struct Contabilizacion {
    tipo: TipoComprobante,
    periodo: PeriodoContable,
    fecha: NaiveDate,
    descripcion: String,
    origen: Option<Origen>,
    regla_id: Option<domain::ReglaContabilizacionId>,
    revierte_a: Option<(ComprobanteId, String)>,
    asiento: AsientoBalanceado,
}

async fn guardar_y_contabilizar<T: TxContable>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    c: Contabilizacion,
) -> AppResult<Comprobante> {
    validar_cuentas(tx, &c.asiento).await?;
    let periodo = periodo_abierto(tx, c.periodo, c.fecha).await?;
    let ahora = reloj.ahora();
    let mut comprobante = Comprobante {
        id: ComprobanteId::nuevo(),
        tipo_comprobante_id: c.tipo.id,
        tipo_codigo: c.tipo.codigo.clone(),
        periodo_id: periodo.id,
        periodo: periodo.periodo,
        numero: None,
        fecha: c.fecha,
        descripcion: c.descripcion,
        estado: EstadoComprobante::Borrador,
        origen: c.origen,
        regla_id: c.regla_id,
        revierte_a: c.revierte_a.as_ref().map(|r| r.0),
        motivo_reverso: c.revierte_a.map(|r| r.1),
        revertido_por: None,
        lineas: c.asiento.lineas().to_vec(),
        creado_por: ctx.actor_id,
        creado_en: ahora,
        contabilizado_por: None,
        contabilizado_en: None,
    };
    tx.insertar_comprobante(&comprobante).await?;
    contabilizar_en(tx, ctx, ahora, &mut comprobante).await?;
    Ok(comprobante)
}

async fn contabilizar_en<T: TxContable>(
    tx: &mut T,
    ctx: &Contexto,
    ahora: chrono::DateTime<chrono::Utc>,
    c: &mut Comprobante,
) -> AppResult<()> {
    let numero = tx.siguiente_numero(&c.tipo_codigo, c.periodo.anio).await?;
    tx.marcar_contabilizado(c.id, numero, ctx.actor_id, ahora)
        .await?;
    c.numero = Some(numero);
    c.estado = EstadoComprobante::Contabilizado;
    c.contabilizado_por = ctx.actor_id;
    c.contabilizado_en = Some(ahora);
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", c.tipo_codigo.as_str())?
        .con("numero", numero)?
        .con(
            "total",
            c.lineas
                .iter()
                .map(|l| l.debito.valor())
                .sum::<Decimal>()
                .to_string(),
        )?
        .con("origen", c.origen.as_ref().map(|o| o.tipo.clone()))?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CONTABILIZAR, "comprobante", c.id.as_uuid())
            .con_detalle(detalle),
    )
    .await
}

// ---------------------------------------------------------------------------------------------
// Comprobantes manuales
// ---------------------------------------------------------------------------------------------

/// Comprobante por id, con sus líneas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::NoEncontrado`].
pub async fn comprobante<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComprobanteId,
) -> AppResult<Comprobante>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .comprobante(id)
        .await?
        .ok_or(AppError::NoEncontrado("comprobante"))
}

/// Comprobantes por fecha (sin líneas), a lo sumo [`LIMITE_LISTADO`].
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn comprobantes<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroComprobantes,
) -> AppResult<Vec<Comprobante>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .comprobantes(&filtro, LIMITE_LISTADO)
        .await
}

async fn armar_borrador<T: TxContable>(
    tx: &mut T,
    ctx: &Contexto,
    reloj: &impl Reloj,
    id: ComprobanteId,
    b: BorradorComprobante,
) -> AppResult<Comprobante> {
    let descripcion = texto("descripcion", &b.descripcion, 500)?;
    let tipo = tipo_por_codigo(tx, &b.tipo).await?;
    if !tipo.manual {
        return Err(error_validacion(format!(
            "el tipo {} es del sistema: no admite comprobantes manuales",
            tipo.codigo
        )));
    }
    let periodo = PeriodoContable::de_fecha(b.fecha)?;
    let p = tx.periodo_o_crear(periodo).await?;
    let lineas = resolver_lineas(tx, b.lineas).await?;
    Ok(Comprobante {
        id,
        tipo_comprobante_id: tipo.id,
        tipo_codigo: tipo.codigo,
        periodo_id: p.id,
        periodo,
        numero: None,
        fecha: b.fecha,
        descripcion,
        estado: EstadoComprobante::Borrador,
        origen: None,
        regla_id: None,
        revierte_a: None,
        motivo_reverso: None,
        revertido_por: None,
        lineas,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
        contabilizado_por: None,
        contabilizado_en: None,
    })
}

/// Guarda un comprobante manual en borrador (puede estar incompleto; se valida al
/// contabilizar).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.comprobantes.crear`; [`AppError::Validacion`] si el
/// tipo no es manual o una cuenta o centro no existe.
pub async fn crear_borrador<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    b: BorradorComprobante,
) -> AppResult<Comprobante>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadComprobantesCrear).await?;
    let c = armar_borrador(&mut tx, ctx, reloj, ComprobanteId::nuevo(), b).await?;
    tx.insertar_comprobante(&c).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_BORRADOR,
        "comprobante",
        c.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

async fn borrador_existente<T: TxContable>(
    tx: &mut T,
    id: ComprobanteId,
) -> AppResult<Comprobante> {
    let c = tx
        .comprobante(id)
        .await?
        .ok_or(AppError::NoEncontrado("comprobante"))?;
    if c.estado != EstadoComprobante::Borrador {
        return Err(AppError::Conflicto(
            "un comprobante contabilizado no se modifica: se revierte".into(),
        ));
    }
    Ok(c)
}

/// Reemplaza un borrador.
///
/// # Errors
///
/// [`AppError::Conflicto`] si ya está contabilizado (QA-003).
pub async fn editar_borrador<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComprobanteId,
    b: BorradorComprobante,
) -> AppResult<Comprobante>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadComprobantesCrear).await?;
    let actual = borrador_existente(&mut tx, id).await?;
    let c = Comprobante {
        creado_por: actual.creado_por,
        creado_en: actual.creado_en,
        ..armar_borrador(&mut tx, ctx, reloj, id, b).await?
    };
    tx.actualizar_borrador(&c).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_BORRADOR,
        "comprobante",
        c.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Borra un borrador.
///
/// # Errors
///
/// [`AppError::Conflicto`] si ya está contabilizado.
pub async fn eliminar_borrador<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComprobanteId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadComprobantesCrear).await?;
    borrador_existente(&mut tx, id).await?;
    tx.eliminar_borrador(id).await?;
    tx.confirmar().await
}

/// Valida un comprobante sin guardarlo: devuelve el asiento tal como quedaría.
///
/// # Errors
///
/// Los mismos que al contabilizar (partida doble, cuentas, tercero y centro exigidos).
pub async fn previsualizar<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    b: BorradorComprobante,
) -> AppResult<AsientoBalanceado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    tipo_por_codigo(&mut tx, &b.tipo).await?;
    let asiento = AsientoBalanceado::nuevo(resolver_lineas(&mut tx, b.lineas).await?)?;
    validar_cuentas(&mut tx, &asiento).await?;
    Ok(asiento)
}

/// Contabiliza un borrador: valida la partida doble y las cuentas, exige el periodo abierto y le
/// asigna el siguiente número del tipo en el año.
///
/// # Errors
///
/// [`AppError::Conflicto`] si ya está contabilizado; [`domain::DominioError::ReglaContable`] si
/// no cuadra, una cuenta no admite la línea o el periodo no está abierto (QA-001, QA-002).
pub async fn contabilizar<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComprobanteId,
) -> AppResult<Comprobante>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(
        unidad,
        reloj,
        ctx,
        Permiso::ContabilidadComprobantesContabilizar,
    )
    .await?;
    let mut c = borrador_existente(&mut tx, id).await?;
    let asiento = AsientoBalanceado::nuevo(c.lineas.clone())?;
    validar_cuentas(&mut tx, &asiento).await?;
    periodo_abierto(&mut tx, c.periodo, c.fecha).await?;
    contabilizar_en(&mut tx, ctx, reloj.ahora(), &mut c).await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Revierte un comprobante contabilizado con otro de caras invertidas, del mismo tipo, en la
/// fecha dada (por defecto, hoy) y con el motivo. El original no cambia.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.comprobantes.revertir`; [`AppError::Conflicto`] si
/// no está contabilizado, ya se revirtió o es un reverso; los del periodo de la fecha.
pub async fn revertir<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComprobanteId,
    motivo: &str,
    fecha: Option<NaiveDate>,
) -> AppResult<Comprobante>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(
        unidad,
        reloj,
        ctx,
        Permiso::ContabilidadComprobantesRevertir,
    )
    .await?;
    let reverso = revertir_en(&mut tx, reloj, ctx, id, motivo, fecha).await?;
    tx.confirmar().await?;
    Ok(reverso)
}

/// Revierte un comprobante **dentro de la transacción del módulo** que anula su documento (un
/// recibo, un egreso, una factura). El permiso lo exige quien llama.
///
/// # Errors
///
/// Los mismos de [`revertir`], salvo el permiso.
pub(crate) async fn revertir_en<T: TxContable>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ComprobanteId,
    motivo: &str,
    fecha: Option<NaiveDate>,
) -> AppResult<Comprobante> {
    let motivo = texto("motivo", motivo, 500)?;
    if motivo.chars().count() < 5 {
        return Err(error_validacion("explique el motivo del reverso"));
    }
    let original = tx
        .comprobante(id)
        .await?
        .ok_or(AppError::NoEncontrado("comprobante"))?;
    if original.estado != EstadoComprobante::Contabilizado {
        return Err(AppError::Conflicto(
            "solo se revierte un comprobante contabilizado".into(),
        ));
    }
    if original.revertido_por.is_some() || original.revierte_a.is_some() {
        return Err(AppError::Conflicto(
            "el comprobante ya se revirtió o es un reverso".into(),
        ));
    }
    let fecha = fecha.unwrap_or_else(|| reloj.ahora().date_naive());
    let tipo = tx
        .tipos_comprobante()
        .await?
        .into_iter()
        .find(|t| t.id == original.tipo_comprobante_id)
        .ok_or(AppError::NoEncontrado("tipo de comprobante"))?;
    let reverso = guardar_y_contabilizar(
        tx,
        reloj,
        ctx,
        Contabilizacion {
            tipo,
            periodo: PeriodoContable::de_fecha(fecha)?,
            fecha,
            descripcion: format!(
                "Reverso de {} {}",
                original.tipo_codigo,
                original.numero.unwrap_or(0)
            ),
            origen: None,
            regla_id: None,
            revierte_a: Some((original.id, motivo)),
            asiento: AsientoBalanceado::nuevo(original.lineas.clone())?.reverso(),
        },
    )
    .await?;
    let detalle = DetalleAuditoria::vacio().con("reverso", reverso.id.as_uuid().to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_REVERTIR, "comprobante", original.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    Ok(reverso)
}

// ---------------------------------------------------------------------------------------------
// Contabilizador de documentos origen
// ---------------------------------------------------------------------------------------------

/// Contabiliza un documento origen **dentro de la transacción del módulo que lo produce**: busca
/// la regla vigente del evento en la fecha, arma el asiento y lo contabiliza. Un reintento con el
/// mismo origen devuelve el comprobante ya hecho (QA-011). El permiso lo exige el caso de uso que
/// llama; aquí se exige el módulo de contabilidad.
///
/// # Errors
///
/// [`AppError::Validacion`] si no hay regla vigente o un concepto no está en ella;
/// [`domain::DominioError::ReglaContable`] si no cuadra o el periodo no está abierto.
pub async fn contabilizar_documento<T: TxContable>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    doc: DocumentoOrigen,
) -> AppResult<ComprobanteId> {
    exigir_contabilidad(tx, reloj).await?;
    if let Some(id) = tx.comprobante_por_origen(&doc.origen).await? {
        return Ok(id);
    }
    let (regla, tipo, asiento) = asiento_de_documento(tx, &doc).await?;
    let c = guardar_y_contabilizar(
        tx,
        reloj,
        ctx,
        Contabilizacion {
            tipo,
            periodo: PeriodoContable::de_fecha(doc.fecha)?,
            fecha: doc.fecha,
            descripcion: texto("descripcion", &doc.descripcion, 500)?,
            origen: Some(doc.origen),
            regla_id: Some(regla),
            revierte_a: None,
            asiento,
        },
    )
    .await?;
    Ok(c.id)
}

async fn asiento_de_documento<T: TxContable>(
    tx: &mut T,
    doc: &DocumentoOrigen,
) -> AppResult<(
    domain::ReglaContabilizacionId,
    TipoComprobante,
    AsientoBalanceado,
)> {
    let regla = tx
        .regla_vigente(&doc.evento, doc.fecha)
        .await?
        .ok_or_else(|| {
            error_validacion(format!(
                "no hay regla vigente para `{}` en {}",
                doc.evento, doc.fecha
            ))
        })?;
    let tipo = tx
        .tipos_comprobante()
        .await?
        .into_iter()
        .find(|t| t.id == regla.tipo_comprobante_id)
        .ok_or(AppError::NoEncontrado("tipo de comprobante"))?;
    let asiento = regla.regla.aplicar(&doc.importes)?;
    Ok((regla.id, tipo, asiento))
}

/// Muestra el asiento que produciría un documento origen, sin guardarlo (para verificar una
/// regla antes de usarla).
///
/// # Errors
///
/// Los mismos de [`contabilizar_documento`], salvo el periodo.
pub async fn previsualizar_documento<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    doc: DocumentoOrigen,
) -> AppResult<AsientoBalanceado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    let (_, _, asiento) = asiento_de_documento(&mut tx, &doc).await?;
    validar_cuentas(&mut tx, &asiento).await?;
    Ok(asiento)
}

// ---------------------------------------------------------------------------------------------
// Saldos iniciales y cierre anual
// ---------------------------------------------------------------------------------------------

/// Saldos con que la entidad empieza (o viene de otro programa).
#[derive(Debug, Clone)]
pub struct SaldosIniciales {
    pub fecha: NaiveDate,
    /// Código del tipo de comprobante (p. ej. `AP`).
    pub tipo: String,
    pub descripcion: String,
    pub lineas: Vec<LineaEntrada>,
}

/// Contabiliza los saldos iniciales en un comprobante de apertura balanceado.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.comprobantes.contabilizar`; los de la partida doble,
/// las cuentas y el periodo.
pub async fn registrar_saldos_iniciales<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    s: SaldosIniciales,
) -> AppResult<Comprobante>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(
        unidad,
        reloj,
        ctx,
        Permiso::ContabilidadComprobantesContabilizar,
    )
    .await?;
    let tipo = tipo_por_codigo(&mut tx, &s.tipo).await?;
    let asiento = AsientoBalanceado::nuevo(resolver_lineas(&mut tx, s.lineas).await?)?;
    let c = guardar_y_contabilizar(
        &mut tx,
        reloj,
        ctx,
        Contabilizacion {
            tipo,
            periodo: PeriodoContable::de_fecha(s.fecha)?,
            fecha: s.fecha,
            descripcion: texto("descripcion", &s.descripcion, 500)?,
            origen: Some(Origen {
                tipo: ORIGEN_SALDOS_INICIALES.into(),
                id: Uuid::now_v7(),
            }),
            regla_id: None,
            revierte_a: None,
            asiento,
        },
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Cierre anual: en el periodo 13 (31 de diciembre) lleva a cero las cuentas de resultado
/// (clases 4 a 7) contra la cuenta de excedente o déficit de la configuración. Exige que los
/// meses del año que existan estén cerrados y que no haya otro cierre vigente.
///
/// # Errors
///
/// [`AppError::Validacion`] sin cuenta de resultado configurada; [`AppError::Conflicto`] si hay
/// meses sin cerrar o ya hay un cierre vigente; [`domain::DominioError::ReglaContable`] si no
/// hay saldos que cerrar.
pub async fn cierre_anual<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    anio: i32,
    tipo: &str,
) -> AppResult<Comprobante>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(
        unidad,
        reloj,
        ctx,
        Permiso::ContabilidadComprobantesContabilizar,
    )
    .await?;
    let periodo = PeriodoContable::cierre_anual(anio)?;
    let cuenta_resultado = tx
        .configuracion_vigente()
        .await?
        .and_then(|c| c.cuenta_resultado)
        .ok_or_else(|| {
            error_validacion("configure la cuenta de excedente o déficit del ejercicio")
        })?;
    let abiertos: Vec<u8> = tx
        .periodos(Some(anio))
        .await?
        .into_iter()
        .filter(|p| !p.periodo.es_cierre() && p.estado != EstadoPeriodo::Cerrado)
        .map(|p| p.periodo.mes)
        .collect();
    if !abiertos.is_empty() {
        return Err(AppError::Conflicto(format!(
            "cierre primero los meses {abiertos:?} de {anio}"
        )));
    }
    if tx.cierre_anual_vigente(anio).await?.is_some() {
        return Err(AppError::Conflicto(format!(
            "{anio} ya tiene un cierre anual vigente: revíertalo para rehacerlo"
        )));
    }
    let (desde, hasta) = (
        NaiveDate::from_ymd_opt(anio, 1, 1).ok_or_else(|| error_validacion("año inválido"))?,
        NaiveDate::from_ymd_opt(anio, 12, 31).ok_or_else(|| error_validacion("año inválido"))?,
    );
    let saldos: Vec<SaldoResultado> = tx
        .saldos(
            desde,
            hasta,
            AgrupacionSaldos {
                por_tercero: true,
                por_centro_costo: true,
                sin_cierre_anual: false,
            },
        )
        .await?
        .into_iter()
        .filter(|f| f.codigo.es_de_resultado())
        .map(|f| SaldoResultado {
            cuenta: f.cuenta_id,
            persona: f.persona_id,
            centro_costo: f.centro_costo_id,
            neto: f.saldo_final(),
        })
        .collect();
    let asiento = asiento_de_cierre(&saldos, cuenta_resultado)?;
    let tipo = tipo_por_codigo(&mut tx, tipo).await?;
    let c = guardar_y_contabilizar(
        &mut tx,
        reloj,
        ctx,
        Contabilizacion {
            tipo,
            periodo,
            fecha: hasta,
            descripcion: format!("Cierre del ejercicio {anio}"),
            origen: Some(Origen {
                tipo: ORIGEN_CIERRE_ANUAL.into(),
                id: Uuid::now_v7(),
            }),
            regla_id: None,
            revierte_a: None,
            asiento,
        },
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}
