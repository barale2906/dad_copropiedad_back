//! Soportes en PDF de la tesorería: el **recibo de caja** que se entrega a quien paga y el
//! **comprobante de egreso** que firma el beneficiario. Se generan al vuelo (los documentos son
//! inmutables; una anulación se marca en el PDF) con la plantilla de tabla de `infra-pdf`.

use domain::{
    EgresoId, Permiso, PersonaId, ReciboCajaId,
    contabilidad::tesoreria::{EstadoEgreso, EstadoRecibo},
};
use rust_decimal::Decimal;

use super::TxTesoreria;
use super::prelude::*;
use super::tesoreria::abrir;
use crate::{
    AppError, AppResult, Contexto,
    puertos::{
        GeneradorActas, Reloj, UnidadDeTrabajo,
        actas::TablaPdf,
        tesoreria::{DatosEntidad, Egreso, ReciboCaja},
    },
};

fn encabezado(entidad: &DatosEntidad) -> String {
    match &entidad.nit {
        Some(nit) => format!("{} · NIT {nit}", entidad.nombre),
        None => entidad.nombre.clone(),
    }
}

/// Número visible del comprobante contable del documento (`RC-2026-12`), si ya tiene número.
async fn numero_comprobante<T: TxTesoreria>(
    tx: &mut T,
    id: Option<domain::ComprobanteId>,
) -> AppResult<Option<String>> {
    let Some(id) = id else {
        return Ok(None);
    };
    Ok(tx.comprobante(id).await?.and_then(|c| {
        c.numero
            .map(|n| format!("{}-{}-{n}", c.tipo_codigo, c.fecha.format("%Y")))
    }))
}

/// Nombre y documento del tercero (`Ana Pérez (CC 1020304050)`).
async fn tercero<T: TxTesoreria>(tx: &mut T, id: PersonaId) -> AppResult<String> {
    let p = tx
        .persona(id)
        .await?
        .ok_or(AppError::NoEncontrado("tercero"))?;
    Ok(match p.documento {
        Some(d) => format!("{} ({})", p.nombre, d.as_str()),
        None => p.nombre,
    })
}

fn total(valor: Decimal) -> Vec<String> {
    vec!["Total".into(), valor.to_string()]
}

/// Filas del recibo: a qué se aplicó (facturas y obligaciones vigentes), lo que quedó sin aplicar
/// y el total.
async fn filas_recibo<T: TxTesoreria>(tx: &mut T, r: &ReciboCaja) -> AppResult<Vec<Vec<String>>> {
    let mut filas = Vec::new();
    for a in r.aplicaciones.iter().filter(|a| a.anulada_en.is_none()) {
        let concepto = match (a.factura_venta_id, a.obligacion_id) {
            (Some(f), _) => match tx.factura_venta(f).await?.and_then(|f| f.numero) {
                Some(n) => format!("Abono a la factura {n}"),
                None => "Abono a una factura".to_owned(),
            },
            (None, Some(_)) => "Abono a expensas y obligaciones".to_owned(),
            (None, None) => "Abono".to_owned(),
        };
        filas.push(vec![concepto, a.valor.valor().to_string()]);
    }
    let sin_aplicar = r.sin_aplicar();
    if !sin_aplicar.is_zero() {
        filas.push(vec![
            "Sin aplicar (anticipo)".into(),
            sin_aplicar.to_string(),
        ]);
    }
    filas.push(total(r.valor.valor()));
    Ok(filas)
}

/// El recibo de caja en PDF: quién pagó, cuánto, por qué medio, a qué se aplicó y su número de
/// comprobante. Si está anulado, el título lo dice.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::NoEncontrado`];
/// [`AppError::Infraestructura`] si falla el PDF.
pub async fn recibo_pdf<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ReciboCajaId,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
    G: GeneradorActas,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    let r = tx
        .recibo(id)
        .await?
        .ok_or(AppError::NoEncontrado("recibo de caja"))?;
    let entidad = tx.datos_entidad().await?;
    let pagador = match r.persona_id {
        Some(p) => tercero(&mut tx, p).await?,
        None => "Pagador no identificado".to_owned(),
    };
    let numero = numero_comprobante(&mut tx, Some(r.comprobante_id)).await?;
    let filas = filas_recibo(&mut tx, &r).await?;
    let anulado = r.estado == EstadoRecibo::Anulado;
    generador
        .tabla(&TablaPdf {
            titulo: if anulado {
                "Recibo de caja (ANULADO)".into()
            } else {
                "Recibo de caja".into()
            },
            copropiedad: encabezado(&entidad),
            subtitulo: subtitulo(
                numero,
                &format!("Recibido de {pagador}"),
                r.fecha,
                r.medio.as_str(),
                r.referencia.as_deref(),
                &r.descripcion,
            ),
            columnas: vec!["Concepto".into(), "Valor".into()],
            filas,
            generada: reloj.ahora().format("%Y-%m-%d %H:%M UTC").to_string(),
            qr: None,
            pie: r
                .motivo_anulacion
                .as_deref()
                .map(|m| format!("Anulado: {m}")),
        })
        .await
}

fn subtitulo(
    numero: Option<String>,
    quien: &str,
    fecha: chrono::NaiveDate,
    medio: &str,
    referencia: Option<&str>,
    descripcion: &str,
) -> String {
    let mut partes = Vec::new();
    if let Some(n) = numero {
        partes.push(format!("Comprobante {n}"));
    }
    partes.push(quien.to_owned());
    partes.push(format!("Fecha {fecha}"));
    partes.push(match referencia {
        Some(r) => format!("Medio: {medio} ({r})"),
        None => format!("Medio: {medio}"),
    });
    if !descripcion.is_empty() {
        partes.push(descripcion.to_owned());
    }
    partes.join(" · ")
}

/// Filas del egreso: las facturas de proveedor que paga, los gastos directos y el total.
async fn filas_egreso<T: TxTesoreria>(tx: &mut T, e: &Egreso) -> AppResult<Vec<Vec<String>>> {
    let mut filas = Vec::new();
    for (factura, valor) in &e.facturas {
        let concepto = match tx.factura_proveedor(*factura).await? {
            Some(f) => format!("Factura {}{}", f.prefijo, f.numero),
            None => "Factura de proveedor".to_owned(),
        };
        filas.push(vec![concepto, valor.valor().to_string()]);
    }
    for g in &e.gastos {
        filas.push(vec![g.descripcion.clone(), g.valor.valor().to_string()]);
    }
    filas.push(total(e.valor.valor()));
    Ok(filas)
}

/// El comprobante de egreso en PDF: a quién se pagó, cuánto, por qué medio, qué facturas y gastos
/// cubre, su estado y su número de comprobante (al pagarse). Lleva espacio para la firma del
/// beneficiario.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::NoEncontrado`];
/// [`AppError::Infraestructura`] si falla el PDF.
pub async fn egreso_pdf<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: EgresoId,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
    G: GeneradorActas,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    let e = tx
        .egreso(id)
        .await?
        .ok_or(AppError::NoEncontrado("egreso"))?;
    let entidad = tx.datos_entidad().await?;
    let beneficiario = tercero(&mut tx, e.persona_id).await?;
    let numero = numero_comprobante(&mut tx, e.comprobante_id).await?;
    let filas = filas_egreso(&mut tx, &e).await?;
    let titulo = match e.estado {
        EstadoEgreso::Anulado => "Comprobante de egreso (ANULADO)".to_owned(),
        EstadoEgreso::Pagado => "Comprobante de egreso".to_owned(),
        otro => format!("Comprobante de egreso ({})", otro.as_str()),
    };
    let pie = match &e.motivo_anulacion {
        Some(m) => format!("Anulado: {m}"),
        None => "Recibí conforme: ______________________   C.C./NIT: ______________".to_owned(),
    };
    generador
        .tabla(&TablaPdf {
            titulo,
            copropiedad: encabezado(&entidad),
            subtitulo: subtitulo(
                numero,
                &format!("Pagado a {beneficiario}"),
                e.fecha,
                e.medio.as_str(),
                e.referencia.as_deref(),
                &e.descripcion,
            ),
            columnas: vec!["Concepto".into(), "Valor".into()],
            filas,
            generada: reloj.ahora().format("%Y-%m-%d %H:%M UTC").to_string(),
            qr: None,
            pie: Some(pie),
        })
        .await
}
