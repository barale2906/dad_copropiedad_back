//! Extractos y conciliación bancaria (plan 13 fase 3): importar el extracto (planilla con mapeo
//! de columnas o recaudo Asobancaria 2001) sin duplicarlo, conciliación automática por valor,
//! fecha y referencia, conciliación manual (varios con varios si las sumas coinciden), deshacer
//! con un registro nuevo, recibo no identificado desde un crédito del extracto e informe de
//! conciliación con sus partidas conciliatorias.

use std::collections::BTreeSet;

use chrono::{Duration, NaiveDate};
use domain::{
    ClasificacionArchivo, ComprobanteId, ConciliacionId, CuentaTesoreriaId, ExtractoBancarioId,
    MovimientoExtractoId, NombreArchivo, Permiso, PersonaId, TipoArchivo,
    contabilidad::{
        Monto,
        extracto::{
            ExtractoLeido, MapeoColumnas, VERSION_LECTOR_ASOBANCARIA_2001, VERSION_LECTOR_PLANILLA,
            leer_asobancaria_2001,
        },
        tesoreria::{
            EstadoMovimientoExtracto, FormatoExtracto, MedioRecibo, ModoConciliacion,
            PartidaConciliable, conciliar_automaticamente, validar_conciliacion,
        },
    },
};
use rust_decimal::Decimal;

use super::prelude::*;
use super::tesoreria::{AplicacionesExternas, NuevoRecibo, abrir, registrar_recibo_con};
use super::{TxTesoreria, texto};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::archivos::{
        ArchivoSubido, borrar_huerfano, nuevo_archivo, registrar_en, sha256_hex,
    },
    puertos::{
        AlmacenArchivos, LectorPlanillas, Reloj, RepositorioArchivos, UnidadDeTrabajo,
        tesoreria::{
            AccionConciliacion, Conciliacion, CuentaTesoreria, ExtractoBancario, LineaBanco,
            MovimientoExtracto, ReciboCaja,
        },
    },
};

const ACCION_IMPORTAR: AccionAuditoria = AccionAuditoria::new("tesoreria.extracto.importar");
const ACCION_CONCILIAR: AccionAuditoria = AccionAuditoria::new("tesoreria.conciliacion.crear");
const ACCION_DESHACER: AccionAuditoria = AccionAuditoria::new("tesoreria.conciliacion.deshacer");
const ACCION_PARTIDA: AccionAuditoria =
    AccionAuditoria::new("tesoreria.movimiento_extracto.marcar");

/// Máxima tolerancia de días de la conciliación automática.
pub const TOLERANCIA_MAXIMA_DIAS: u32 = 10;
/// Máximo de errores que se repiten en el mensaje al rechazar un extracto.
const ERRORES_EN_MENSAJE: usize = 5;

fn validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

fn sin_fin() -> NaiveDate {
    NaiveDate::from_ymd_opt(2999, 12, 31).unwrap_or(NaiveDate::MAX)
}

fn sin_inicio() -> NaiveDate {
    NaiveDate::from_ymd_opt(2000, 1, 1).unwrap_or(NaiveDate::MIN)
}

/// Cómo leer el archivo.
#[derive(Debug, Clone)]
pub enum FormatoEntrada {
    /// CSV o XLSX con el mapeo de columnas del banco.
    Planilla(MapeoColumnas),
    /// Recaudo Asobancaria 2001.
    Asobancaria2001,
}

impl FormatoEntrada {
    fn formato(&self) -> FormatoExtracto {
        match self {
            Self::Planilla(_) => FormatoExtracto::Planilla,
            Self::Asobancaria2001 => FormatoExtracto::Asobancaria2001,
        }
    }

    fn version(&self) -> &'static str {
        match self {
            Self::Planilla(_) => VERSION_LECTOR_PLANILLA,
            Self::Asobancaria2001 => VERSION_LECTOR_ASOBANCARIA_2001,
        }
    }
}

/// Lee el archivo según el formato (sin guardar nada).
fn leer(
    lector: &impl LectorPlanillas,
    contenido: &[u8],
    formato: &FormatoEntrada,
) -> AppResult<(ExtractoLeido, TipoArchivo)> {
    let tipo = TipoArchivo::validar_planilla(contenido)?;
    let leido = match formato {
        FormatoEntrada::Planilla(mapeo) => mapeo.leer(&lector.leer(tipo, contenido)?)?,
        FormatoEntrada::Asobancaria2001 => {
            if tipo != TipoArchivo::Csv {
                return Err(validacion("un archivo Asobancaria es de texto"));
            }
            let texto = std::str::from_utf8(contenido)
                .map_err(|_| validacion("el archivo no es texto UTF-8"))?;
            leer_asobancaria_2001(texto)?
        }
    };
    Ok((leido, tipo))
}

/// Lee el extracto y muestra lo que se importaría, con los problemas por fila.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.conciliar`; [`AppError::Dominio`] si el archivo o el
/// mapeo no sirven.
pub async fn previsualizar_extracto<U, L>(
    unidad: &U,
    lector: &L,
    reloj: &impl Reloj,
    ctx: &Contexto,
    contenido: &[u8],
    formato: &FormatoEntrada,
) -> AppResult<ExtractoLeido>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
    L: LectorPlanillas,
{
    abrir(unidad, reloj, ctx, Permiso::TesoreriaConciliar).await?;
    Ok(leer(lector, contenido, formato)?.0)
}

/// Extracto a importar.
#[derive(Debug, Clone)]
pub struct ImportacionExtracto {
    pub cuenta_tesoreria: CuentaTesoreriaId,
    pub nombre: NombreArchivo,
    pub contenido: Vec<u8>,
    pub formato: FormatoEntrada,
    pub saldo_inicial: Option<Decimal>,
    pub saldo_final: Option<Decimal>,
}

/// Importa el extracto: guarda el archivo (soporte) y sus movimientos como pendientes. El mismo
/// archivo no se importa dos veces en la cuenta; un extracto con filas inválidas se rechaza
/// (revíselo con la vista previa).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.conciliar`; [`AppError::Validacion`] si hay filas
/// inválidas o ningún movimiento; [`AppError::Conflicto`] si ya se importó.
pub async fn importar_extracto<U, A, L>(
    unidad: &U,
    almacen: &A,
    lector: &L,
    reloj: &impl Reloj,
    ctx: &Contexto,
    imp: ImportacionExtracto,
) -> AppResult<ExtractoBancario>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria + RepositorioArchivos,
    A: AlmacenArchivos,
    L: LectorPlanillas,
{
    ctx.exigir(Permiso::TesoreriaConciliar)?;
    let (leido, tipo) = leer(lector, &imp.contenido, &imp.formato)?;
    if !leido.errores.is_empty() {
        let detalle: Vec<String> = leido
            .errores
            .iter()
            .take(ERRORES_EN_MENSAJE)
            .map(|e| format!("fila {}: {}", e.fila, e.mensaje))
            .collect();
        return Err(validacion(format!(
            "el extracto tiene {} filas con problemas: {}",
            leido.errores.len(),
            detalle.join("; ")
        )));
    }
    let (desde, hasta) = leido
        .rango()
        .ok_or_else(|| validacion("el extracto no tiene movimientos"))?;
    let sha256 = sha256_hex(&imp.contenido);
    let subido = ArchivoSubido {
        nombre_original: imp.nombre,
        clasificacion: ClasificacionArchivo::Personal,
        contenido: imp.contenido,
    };
    let archivo = nuevo_archivo(ctx, almacen, reloj, &subido, tipo);
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaConciliar).await?;
    tx.cuenta_tesoreria(imp.cuenta_tesoreria)
        .await?
        .ok_or(AppError::NoEncontrado("cuenta de tesorería"))?;
    if tx
        .extracto_por_hash(imp.cuenta_tesoreria, &sha256)
        .await?
        .is_some()
    {
        return Err(AppError::Conflicto(
            "ese extracto ya se importó en la cuenta".into(),
        ));
    }
    almacen.guardar(&archivo.clave, &subido.contenido).await?;
    let resultado = async {
        registrar_en(&mut tx, &archivo).await?;
        let extracto = ExtractoBancario {
            id: ExtractoBancarioId::nuevo(),
            cuenta_tesoreria_id: imp.cuenta_tesoreria,
            desde,
            hasta,
            formato: imp.formato.formato(),
            version_lector: imp.formato.version().to_owned(),
            archivo_id: archivo.id,
            sha256,
            saldo_inicial: imp.saldo_inicial,
            saldo_final: imp.saldo_final,
            cargado_por: ctx.actor_id,
            cargado_en: reloj.ahora(),
            movimientos: u32::try_from(leido.movimientos.len()).unwrap_or(u32::MAX),
        };
        let movimientos: Vec<MovimientoExtracto> = leido
            .movimientos
            .into_iter()
            .enumerate()
            .map(|(i, m)| MovimientoExtracto {
                id: MovimientoExtractoId::nuevo(),
                extracto_id: extracto.id,
                cuenta_tesoreria_id: imp.cuenta_tesoreria,
                orden: u32::try_from(i + 1).unwrap_or(u32::MAX),
                fecha: m.fecha,
                descripcion: m.descripcion,
                referencia: m.referencia,
                valor: m.valor,
                estado: EstadoMovimientoExtracto::Pendiente,
            })
            .collect();
        tx.insertar_extracto(&extracto, &movimientos).await?;
        let detalle = DetalleAuditoria::vacio()
            .con("formato", extracto.formato.as_str())?
            .con("movimientos", extracto.movimientos)?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_IMPORTAR, "extracto_bancario", extracto.id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
        tx.confirmar().await?;
        Ok(extracto)
    }
    .await;
    if resultado.is_err() {
        borrar_huerfano(almacen, ctx, &archivo).await;
    }
    resultado
}

/// Extractos de la cuenta.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn extractos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    cuenta: CuentaTesoreriaId,
) -> AppResult<Vec<ExtractoBancario>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    tx.extractos(cuenta).await
}

/// Movimientos del extracto de la cuenta entre dos fechas (del estado, si se da).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn movimientos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    cuenta: CuentaTesoreriaId,
    desde: NaiveDate,
    hasta: NaiveDate,
    estado: Option<EstadoMovimientoExtracto>,
) -> AppResult<Vec<MovimientoExtracto>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    tx.movimientos_extracto(cuenta, desde, hasta, estado).await
}

/// Líneas de libros de la cuenta entre dos fechas (`solo_pendientes`: las no conciliadas).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::NoEncontrado`].
pub async fn lineas_libros<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    cuenta: CuentaTesoreriaId,
    desde: NaiveDate,
    hasta: NaiveDate,
    solo_pendientes: bool,
) -> AppResult<Vec<LineaBanco>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    let c = cuenta_existente(&mut tx, cuenta).await?;
    tx.lineas_banco(c.cuenta_contable_id, Some(desde), hasta, solo_pendientes)
        .await
}

async fn cuenta_existente<T: TxTesoreria>(
    tx: &mut T,
    id: CuentaTesoreriaId,
) -> AppResult<CuentaTesoreria> {
    tx.cuenta_tesoreria(id)
        .await?
        .ok_or(AppError::NoEncontrado("cuenta de tesorería"))
}

async fn guardar_conciliacion<T: TxTesoreria>(tx: &mut T, c: &Conciliacion) -> AppResult<()> {
    tx.insertar_conciliacion(c).await?;
    let estado = match c.accion {
        AccionConciliacion::Conciliar => EstadoMovimientoExtracto::Conciliado,
        AccionConciliacion::Deshacer(_) => EstadoMovimientoExtracto::Pendiente,
    };
    tx.cambiar_estado_movimientos(&c.movimientos, estado)
        .await?;
    let detalle = DetalleAuditoria::vacio()
        .con("modo", c.modo.as_str())?
        .con("movimientos", c.movimientos.len())?
        .con("lineas", c.lineas.len())?;
    let accion = match c.accion {
        AccionConciliacion::Conciliar => ACCION_CONCILIAR,
        AccionConciliacion::Deshacer(_) => ACCION_DESHACER,
    };
    tx.registrar(EventoAuditoria::ok(accion, "conciliacion", c.id.as_uuid()).con_detalle(detalle))
        .await
}

/// Concilia automáticamente los movimientos pendientes de la cuenta entre dos fechas con las
/// líneas de libros pendientes: mismo valor, fecha a lo sumo a `tolerancia_dias` y, entre
/// varias, la de la misma referencia. Lo que no casa queda para la conciliación manual.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.conciliar`; [`AppError::Validacion`] si la tolerancia
/// supera [`TOLERANCIA_MAXIMA_DIAS`].
pub async fn conciliar_automaticamente_cuenta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    cuenta: CuentaTesoreriaId,
    desde: NaiveDate,
    hasta: NaiveDate,
    tolerancia_dias: u32,
) -> AppResult<Vec<Conciliacion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    if tolerancia_dias > TOLERANCIA_MAXIMA_DIAS {
        return Err(validacion(format!(
            "la tolerancia es de a lo sumo {TOLERANCIA_MAXIMA_DIAS} días"
        )));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaConciliar).await?;
    let c = cuenta_existente(&mut tx, cuenta).await?;
    let pendientes = tx
        .movimientos_extracto(
            cuenta,
            desde,
            hasta,
            Some(EstadoMovimientoExtracto::Pendiente),
        )
        .await?;
    let margen = Duration::days(i64::from(tolerancia_dias));
    let lineas = tx
        .lineas_banco(
            c.cuenta_contable_id,
            Some(desde - margen),
            hasta + margen,
            true,
        )
        .await?;
    let partidas_m: Vec<PartidaConciliable<MovimientoExtractoId>> = pendientes
        .iter()
        .map(|m| PartidaConciliable {
            clave: m.id,
            fecha: m.fecha,
            valor: m.valor,
            referencia: m.referencia.clone(),
        })
        .collect();
    let partidas_l: Vec<PartidaConciliable<(ComprobanteId, u16)>> = lineas
        .iter()
        .map(|l| PartidaConciliable {
            clave: (l.comprobante_id, l.orden),
            fecha: l.fecha,
            valor: l.valor,
            referencia: Some(l.descripcion.clone()),
        })
        .collect();
    let mut hechas = Vec::new();
    for (m, l) in conciliar_automaticamente(&partidas_m, &partidas_l, tolerancia_dias) {
        let c = Conciliacion {
            id: ConciliacionId::nuevo(),
            cuenta_tesoreria_id: cuenta,
            accion: AccionConciliacion::Conciliar,
            modo: ModoConciliacion::Automatica,
            por: ctx.actor_id,
            en: reloj.ahora(),
            movimientos: vec![m],
            lineas: vec![l],
            deshecha: false,
        };
        guardar_conciliacion(&mut tx, &c).await?;
        hechas.push(c);
    }
    tx.confirmar().await?;
    Ok(hechas)
}

/// Concilia a mano movimientos pendientes con líneas de libros pendientes de la cuenta cuyas
/// sumas coinciden exactamente (uno a uno, varios a uno o varios a varios).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.conciliar`; [`AppError::Validacion`] si un movimiento
/// o una línea no es de la cuenta o ya está conciliada; [`domain::DominioError::ReglaContable`]
/// si las sumas difieren.
pub async fn conciliar_manual<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    cuenta: CuentaTesoreriaId,
    movimientos: Vec<MovimientoExtractoId>,
    lineas: Vec<(ComprobanteId, u16)>,
) -> AppResult<Conciliacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaConciliar).await?;
    let c = conciliar_manual_en(&mut tx, reloj, ctx, cuenta, movimientos, lineas).await?;
    tx.confirmar().await?;
    Ok(c)
}

async fn conciliar_manual_en<T: TxTesoreria>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    cuenta: CuentaTesoreriaId,
    movimientos: Vec<MovimientoExtractoId>,
    lineas: Vec<(ComprobanteId, u16)>,
) -> AppResult<Conciliacion> {
    let c = cuenta_existente(tx, cuenta).await?;
    let unicos_m: BTreeSet<_> = movimientos.iter().collect();
    let unicos_l: BTreeSet<_> = lineas.iter().collect();
    if unicos_m.len() != movimientos.len() || unicos_l.len() != lineas.len() {
        return Err(validacion("hay partidas repetidas"));
    }
    let movs = tx.movimientos_por_id(&movimientos).await?;
    if movs.len() != movimientos.len()
        || movs.iter().any(|m| {
            m.cuenta_tesoreria_id != cuenta || m.estado == EstadoMovimientoExtracto::Conciliado
        })
    {
        return Err(validacion(
            "un movimiento no es de la cuenta o ya está conciliado",
        ));
    }
    let pendientes = tx
        .lineas_banco(c.cuenta_contable_id, None, sin_fin(), true)
        .await?;
    let mut valores_l = Vec::with_capacity(lineas.len());
    for (comprobante, orden) in &lineas {
        let l = pendientes
            .iter()
            .find(|l| l.comprobante_id == *comprobante && l.orden == *orden)
            .ok_or_else(|| {
                validacion("una línea no es de la cuenta bancaria o ya está conciliada")
            })?;
        valores_l.push(l.valor);
    }
    let valores_m: Vec<Decimal> = movs.iter().map(|m| m.valor).collect();
    validar_conciliacion(&valores_m, &valores_l)?;
    let conciliacion = Conciliacion {
        id: ConciliacionId::nuevo(),
        cuenta_tesoreria_id: cuenta,
        accion: AccionConciliacion::Conciliar,
        modo: ModoConciliacion::Manual,
        por: ctx.actor_id,
        en: reloj.ahora(),
        movimientos,
        lineas,
        deshecha: false,
    };
    guardar_conciliacion(tx, &conciliacion).await?;
    Ok(conciliacion)
}

/// Deshace una conciliación con un registro nuevo (las conciliaciones no se borran): sus
/// movimientos vuelven a pendientes.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.conciliar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya se deshizo o es un deshacer.
pub async fn deshacer_conciliacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ConciliacionId,
) -> AppResult<Conciliacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaConciliar).await?;
    let original = tx
        .conciliacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("conciliación"))?;
    if original.accion != AccionConciliacion::Conciliar || original.deshecha {
        return Err(AppError::Conflicto(
            "la conciliación ya se deshizo o es un deshacer".into(),
        ));
    }
    let deshacer = Conciliacion {
        id: ConciliacionId::nuevo(),
        accion: AccionConciliacion::Deshacer(id),
        modo: ModoConciliacion::Manual,
        por: ctx.actor_id,
        en: reloj.ahora(),
        deshecha: false,
        ..original
    };
    guardar_conciliacion(&mut tx, &deshacer).await?;
    tx.confirmar().await?;
    Ok(deshacer)
}

/// Conciliaciones de la cuenta, más recientes primero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn conciliaciones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    cuenta: CuentaTesoreriaId,
) -> AppResult<Vec<Conciliacion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    tx.conciliaciones(cuenta, super::tesoreria::LIMITE_LISTADO)
        .await
}

/// Marca un movimiento pendiente como partida conciliatoria (o lo devuelve a pendiente).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.conciliar`; [`AppError::Conflicto`] si está
/// conciliado.
pub async fn marcar_partida_conciliatoria<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    movimiento: MovimientoExtractoId,
    partida: bool,
) -> AppResult<MovimientoExtracto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaConciliar).await?;
    let mut m = tx
        .movimientos_por_id(&[movimiento])
        .await?
        .pop()
        .ok_or(AppError::NoEncontrado("movimiento del extracto"))?;
    if m.estado == EstadoMovimientoExtracto::Conciliado {
        return Err(AppError::Conflicto("el movimiento está conciliado".into()));
    }
    m.estado = if partida {
        EstadoMovimientoExtracto::PartidaConciliatoria
    } else {
        EstadoMovimientoExtracto::Pendiente
    };
    tx.cambiar_estado_movimientos(&[movimiento], m.estado)
        .await?;
    let detalle = DetalleAuditoria::vacio().con("estado", m.estado.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PARTIDA, "movimiento_extracto", movimiento.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(m)
}

/// Crea un **recibo no identificado** por un crédito del extracto que no tiene recibo en libros
/// y lo concilia con ese movimiento en la misma transacción.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.conciliar` o `tesoreria.recibos.registrar`;
/// [`AppError::Validacion`] si el movimiento no es un crédito pendiente; los del recibo.
pub async fn recibo_no_identificado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    movimiento: MovimientoExtractoId,
    descripcion: &str,
) -> AppResult<ReciboCaja>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    ctx.exigir(Permiso::TesoreriaRecibosRegistrar)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TesoreriaConciliar).await?;
    let recibo = recibo_de_movimiento_en(
        &mut tx,
        reloj,
        ctx,
        movimiento,
        None,
        descripcion,
        AplicacionesExternas::default(),
    )
    .await?;
    tx.confirmar().await?;
    Ok(recibo)
}

/// Crédito pendiente del extracto, bloqueado para conciliar.
pub(crate) async fn credito_pendiente<T: TxTesoreria>(
    tx: &mut T,
    movimiento: MovimientoExtractoId,
) -> AppResult<MovimientoExtracto> {
    let m = tx
        .movimientos_por_id(&[movimiento])
        .await?
        .pop()
        .ok_or(AppError::NoEncontrado("movimiento del extracto"))?;
    if m.valor <= Decimal::ZERO || m.estado == EstadoMovimientoExtracto::Conciliado {
        return Err(validacion(
            "solo un crédito (entrada de dinero) pendiente genera un recibo",
        ));
    }
    Ok(m)
}

/// Registra el recibo de un crédito del extracto (del pagador dado, o no identificado) con las
/// aplicaciones de una vertical, y lo concilia con el movimiento en la misma transacción.
pub(crate) async fn recibo_de_movimiento_en<T: TxTesoreria>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    movimiento: MovimientoExtractoId,
    persona: Option<PersonaId>,
    descripcion: &str,
    externas: AplicacionesExternas,
) -> AppResult<ReciboCaja> {
    let descripcion = texto("descripcion", descripcion, 300)?;
    let m = credito_pendiente(tx, movimiento).await?;
    let cuenta = cuenta_existente(tx, m.cuenta_tesoreria_id).await?;
    let recibo = registrar_recibo_con(
        tx,
        reloj,
        ctx,
        NuevoRecibo {
            cuenta_tesoreria: cuenta.id,
            persona,
            fecha: m.fecha,
            valor: Monto::new(m.valor)?,
            medio: MedioRecibo::Consignacion,
            referencia: m.referencia.clone(),
            descripcion,
            soporte_archivo_id: None,
            aplicaciones: Vec::new(),
        },
        externas,
    )
    .await?;
    let orden = tx
        .comprobante(recibo.comprobante_id)
        .await?
        .and_then(|c| {
            c.lineas
                .iter()
                .position(|l| l.cuenta == cuenta.cuenta_contable_id && !l.debito.es_cero())
        })
        .and_then(|i| u16::try_from(i + 1).ok())
        .ok_or_else(|| {
            validacion("la regla del recibo no lleva el dinero a la cuenta del banco")
        })?;
    conciliar_manual_en(
        tx,
        reloj,
        ctx,
        cuenta.id,
        vec![movimiento],
        vec![(recibo.comprobante_id, orden)],
    )
    .await?;
    Ok(recibo)
}

/// Informe de conciliación a una fecha de corte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InformeConciliacion {
    pub corte: NaiveDate,
    /// Saldo final del último extracto hasta el corte que lo trae.
    pub saldo_extracto: Option<Decimal>,
    pub saldo_libros: Decimal,
    /// En libros y no en el extracto (cheques en tránsito, consignaciones en tránsito).
    pub libros_pendientes: Vec<LineaBanco>,
    /// En el extracto y no en libros (por identificar, gastos bancarios sin causar…).
    pub extracto_pendientes: Vec<MovimientoExtracto>,
    /// Saldo de libros − pendientes de libros + pendientes del extracto: debe igualar al saldo
    /// del extracto.
    pub saldo_conciliado: Decimal,
    /// Saldo del extracto − saldo conciliado (cero si todo cuadra).
    pub diferencia: Option<Decimal>,
}

/// Informe de conciliación de la cuenta a la fecha de corte.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::NoEncontrado`].
pub async fn informe_conciliacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    cuenta: CuentaTesoreriaId,
    corte: NaiveDate,
) -> AppResult<InformeConciliacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    let c = cuenta_existente(&mut tx, cuenta).await?;
    let saldo_extracto = tx
        .extractos(cuenta)
        .await?
        .into_iter()
        .filter(|e| e.hasta <= corte)
        .find_map(|e| e.saldo_final);
    let saldo_libros = tx.saldo_libros(c.cuenta_contable_id, corte).await?;
    let libros_pendientes = tx
        .lineas_banco(c.cuenta_contable_id, None, corte, true)
        .await?;
    let extracto_pendientes: Vec<MovimientoExtracto> = tx
        .movimientos_extracto(cuenta, sin_inicio(), corte, None)
        .await?
        .into_iter()
        .filter(|m| m.estado != EstadoMovimientoExtracto::Conciliado)
        .collect();
    let saldo_conciliado = saldo_libros
        - libros_pendientes.iter().map(|l| l.valor).sum::<Decimal>()
        + extracto_pendientes.iter().map(|m| m.valor).sum::<Decimal>();
    Ok(InformeConciliacion {
        corte,
        diferencia: saldo_extracto.map(|s| s - saldo_conciliado),
        saldo_extracto,
        saldo_libros,
        libros_pendientes,
        extracto_pendientes,
        saldo_conciliado,
    })
}
