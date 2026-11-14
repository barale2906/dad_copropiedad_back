//! Exportaciones para el contador externo (plan 18 fase A4): la entidad que solo factura con la
//! plataforma entrega a quien lleva su contabilidad (en otro programa) el **libro de ventas**, el
//! **libro de compras** (facturas recibidas) en CSV y el **paquete** con los XML y PDF de sus
//! documentos electrónicos del periodo en un ZIP. Cada exportación queda en la auditoría con su
//! huella SHA-256, como los libros contables.

use chrono::NaiveDate;
use domain::{
    Permiso,
    tributario::{EstadoDocumentoElectronico, TipoDocumentoElectronico},
};
use sha2::{Digest, Sha256};

use super::documentos::{LIMITE_DOCUMENTOS, TxFacturacion, pdf_documento};
use super::{TxVentas, abrir_ventas};
use crate::casos_uso::contabilidad::prelude::*;
use crate::casos_uso::contabilidad::reportes::a_csv;
use crate::casos_uso::contabilidad::tesoreria::{LIMITE_LISTADO, eventos, origen};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        AlmacenArchivos, GeneradorActas, RegistroAuditoria, Reloj, RepositorioArchivos as _,
        Transaccion, UnidadDeTrabajo,
        archivos::Empaquetador,
        tesoreria::FiltroFacturas,
        tributario::{FiltroDocumentos, FiltroRecibidas},
    },
};

const ACCION: AccionAuditoria = AccionAuditoria::new("facturacion.libro.exportar");

/// Archivo exportado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exportacion {
    /// Nombre sugerido del archivo (`libro_ventas_2026-11-01_2026-11-30.csv`).
    pub nombre: String,
    /// Tipo MIME.
    pub tipo: &'static str,
    pub contenido: Vec<u8>,
    /// SHA-256 del contenido en hexadecimal (queda en la auditoría).
    pub huella: String,
    /// Filas del libro o documentos del paquete.
    pub registros: usize,
}

fn validar_rango(desde: NaiveDate, hasta: NaiveDate) -> AppResult<()> {
    if hasta < desde {
        return Err(AppError::Validacion(
            "`hasta` no puede ser anterior a `desde`".into(),
        ));
    }
    Ok(())
}

/// Audita la exportación y la devuelve con su huella; confirma la transacción.
async fn cerrar<T: RegistroAuditoria + Transaccion>(
    mut tx: T,
    ctx: &Contexto,
    libro: &str,
    (desde, hasta): (NaiveDate, NaiveDate),
    tipo: &'static str,
    extension: &str,
    (contenido, registros): (Vec<u8>, usize),
) -> AppResult<Exportacion> {
    let huella = hex::encode(Sha256::digest(&contenido));
    let detalle = DetalleAuditoria::vacio()
        .con("libro", libro)?
        .con("desde", desde.to_string())?
        .con("hasta", hasta.to_string())?
        .con("registros", registros)?
        .con("sha256", huella.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION, "entidad", ctx.tenant_id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(Exportacion {
        nombre: format!("{libro}_{desde}_{hasta}.{extension}"),
        tipo,
        contenido,
        huella,
        registros,
    })
}

fn sin_truncar(cantidad: usize, limite: u32) -> AppResult<()> {
    if cantidad >= limite as usize {
        return Err(AppError::Validacion(format!(
            "el periodo tiene {limite} o más registros: acótelo"
        )));
    }
    Ok(())
}

/// Libro de ventas en CSV: las facturas emitidas en el periodo con su número interno, el número y
/// el CUFE de la DIAN, el cliente y los valores.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.exportar`; [`AppError::ModuloNoHabilitado`];
/// [`AppError::Validacion`] si el rango está invertido o es demasiado grande.
pub async fn libro_ventas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<Exportacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    validar_rango(desde, hasta)?;
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionExportar).await?;
    let facturas = tx
        .facturas_venta(
            &FiltroFacturas {
                desde: Some(desde),
                hasta: Some(hasta),
                ..FiltroFacturas::default()
            },
            LIMITE_LISTADO,
        )
        .await?;
    sin_truncar(facturas.len(), LIMITE_LISTADO)?;
    let mut filas = Vec::with_capacity(facturas.len());
    for f in facturas.into_iter().filter(|f| f.numero.is_some()) {
        let cliente = tx.persona(f.persona_id).await?;
        let electronica = tx
            .documento_por_origen(
                TipoDocumentoElectronico::Factura,
                &origen(eventos::FACTURA_VENTA, f.id.as_uuid()),
            )
            .await?;
        filas.push(vec![
            f.fecha.to_string(),
            f.numero.map(|n| n.to_string()).unwrap_or_default(),
            electronica
                .as_ref()
                .map(|d| format!("{}{}", d.prefijo, d.numero.unwrap_or_default()))
                .unwrap_or_default(),
            electronica
                .as_ref()
                .and_then(|d| d.cufe.clone())
                .unwrap_or_default(),
            electronica
                .as_ref()
                .map(|d| d.estado.as_str().to_owned())
                .unwrap_or_default(),
            cliente
                .as_ref()
                .map(|p| p.nombre.clone())
                .unwrap_or_default(),
            cliente
                .as_ref()
                .and_then(|p| p.documento.as_ref())
                .map(|d| d.as_str().to_owned())
                .unwrap_or_default(),
            f.subtotal.valor().to_string(),
            f.iva.valor().to_string(),
            f.total.valor().to_string(),
            f.estado.as_str().to_owned(),
        ]);
    }
    let csv = a_csv(
        &[
            "fecha",
            "numero_interno",
            "numero_dian",
            "cufe",
            "estado_dian",
            "cliente",
            "documento_cliente",
            "subtotal",
            "iva",
            "total",
            "estado",
        ],
        &filas,
    );
    let registros = filas.len();
    cerrar(
        tx,
        ctx,
        "libro_ventas",
        (desde, hasta),
        "text/csv; charset=utf-8",
        "csv",
        (csv.into_bytes(), registros),
    )
    .await
}

/// Libro de compras en CSV: las facturas electrónicas recibidas con fecha en el periodo.
///
/// # Errors
///
/// Los de [`libro_ventas`].
pub async fn libro_compras<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<Exportacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    validar_rango(desde, hasta)?;
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionExportar).await?;
    let recibidas = tx
        .facturas_recibidas(
            &FiltroRecibidas {
                estado: None,
                desde: Some(desde),
                hasta: Some(hasta),
            },
            LIMITE_DOCUMENTOS,
        )
        .await?;
    sin_truncar(recibidas.len(), LIMITE_DOCUMENTOS)?;
    let filas: Vec<Vec<String>> = recibidas
        .iter()
        .map(|f| {
            vec![
                f.fecha.to_string(),
                f.emisor_nit.clone(),
                f.emisor_nombre.clone(),
                format!("{}{}", f.prefijo, f.numero),
                f.cufe.clone(),
                f.subtotal.valor().to_string(),
                f.iva.valor().to_string(),
                f.total.valor().to_string(),
                f.estado.as_str().to_owned(),
            ]
        })
        .collect();
    let csv = a_csv(
        &[
            "fecha",
            "nit_emisor",
            "emisor",
            "numero",
            "cufe",
            "subtotal",
            "iva",
            "total",
            "estado",
        ],
        &filas,
    );
    cerrar(
        tx,
        ctx,
        "libro_compras",
        (desde, hasta),
        "text/csv; charset=utf-8",
        "csv",
        (csv.into_bytes(), filas.len()),
    )
    .await
}

/// Paquete ZIP con el XML y el PDF de cada documento electrónico transmitido (validado, enviado
/// o en contingencia) creado en el periodo.
///
/// # Errors
///
/// Los de [`libro_ventas`]; [`AppError::Infraestructura`] si no se lee un XML o falla el PDF.
pub async fn paquete_documentos<U, A, G, E>(
    unidad: &U,
    (almacen, generador, empaquetador, pie): (&A, &G, &E, Option<&str>),
    reloj: &impl Reloj,
    ctx: &Contexto,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<Exportacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    A: AlmacenArchivos,
    G: GeneradorActas,
    E: Empaquetador,
{
    validar_rango(desde, hasta)?;
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionExportar).await?;
    let documentos = tx
        .documentos(
            &FiltroDocumentos {
                desde: Some(desde),
                hasta: Some(hasta),
                ..FiltroDocumentos::default()
            },
            LIMITE_DOCUMENTOS,
        )
        .await?;
    sin_truncar(documentos.len(), LIMITE_DOCUMENTOS)?;
    let mut archivos = Vec::new();
    let mut registros = 0;
    for d in documentos.iter().filter(|d| {
        matches!(
            d.estado,
            EstadoDocumentoElectronico::Validado
                | EstadoDocumentoElectronico::Enviado
                | EstadoDocumentoElectronico::Contingencia
        )
    }) {
        let base = format!(
            "{}-{}{}",
            d.tipo.as_str(),
            d.prefijo,
            d.numero.unwrap_or_default()
        );
        if let Some(xml) = d.xml_archivo_id {
            let archivo = tx
                .buscar(xml)
                .await?
                .ok_or(AppError::NoEncontrado("XML del documento"))?;
            archivos.push((
                format!("{base}.xml"),
                almacen.obtener(&archivo.clave).await?,
            ));
        }
        archivos.push((
            format!("{base}.pdf"),
            pdf_documento(&mut tx, generador, reloj, d, pie).await?,
        ));
        registros += 1;
    }
    let zip = empaquetador.empaquetar(&archivos)?;
    cerrar(
        tx,
        ctx,
        "documentos_electronicos",
        (desde, hasta),
        "application/zip",
        "zip",
        (zip, registros),
    )
    .await
}
