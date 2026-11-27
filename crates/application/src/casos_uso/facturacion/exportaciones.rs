//! Exportaciones para el contador externo (plan 18 fase A4): la entidad que solo factura con la
//! plataforma entrega a quien lleva su contabilidad (en otro programa) el **libro de ventas**, el
//! **libro de compras** (facturas recibidas) en CSV y el **paquete** ZIP con los documentos
//! electrónicos del periodo (emitidos, recibidos y eventos RADIAN, con un índice de huellas).
//! Cada exportación queda en la auditoría con su huella SHA-256, como los libros contables.

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
        AlmacenArchivos, GeneradorActas, RegistroAuditoria, Reloj, Transaccion, UnidadDeTrabajo,
        archivos::Empaquetador,
        tesoreria::FiltroFacturas,
        tributario::{FiltroDocumentos, FiltroRecibidas, RegistroRadian},
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

/// Encabezado de `indice.csv` dentro del paquete.
const ENCABEZADO_INDICE: [&str; 7] = [
    "archivo",
    "clase",
    "documento",
    "cufe",
    "estado",
    "fecha",
    "sha256",
];

/// Archivos del paquete y su índice: cada archivo agregado deja una fila con su huella SHA-256,
/// para que quien conserva los documentos compruebe que no cambiaron.
#[derive(Default)]
struct Paquete {
    archivos: Vec<(String, Vec<u8>)>,
    indice: Vec<Vec<String>>,
}

/// Datos de la fila del índice que no salen del archivo (documento, CUFE, estado y fecha).
struct Referencia<'a> {
    documento: &'a str,
    cufe: &'a str,
    estado: &'a str,
    fecha: String,
}

impl Paquete {
    fn agregar(&mut self, nombre: String, clase: &str, r: &Referencia<'_>, contenido: Vec<u8>) {
        self.indice.push(vec![
            nombre.clone(),
            clase.to_owned(),
            r.documento.to_owned(),
            r.cufe.to_owned(),
            r.estado.to_owned(),
            r.fecha.clone(),
            hex::encode(Sha256::digest(&contenido)),
        ]);
        self.archivos.push((nombre, contenido));
    }

    /// Archivos con `indice.csv` al principio.
    fn cerrar(mut self) -> Vec<(String, Vec<u8>)> {
        let indice = a_csv(&ENCABEZADO_INDICE, &self.indice);
        self.archivos
            .insert(0, ("indice.csv".to_owned(), indice.into_bytes()));
        self.archivos
    }
}

/// Lee del almacén el archivo registrado con ese id.
async fn leer<T: TxFacturacion>(
    tx: &mut T,
    almacen: &impl AlmacenArchivos,
    archivo: domain::ArchivoId,
    que: &'static str,
) -> AppResult<Vec<u8>> {
    let a = tx
        .buscar(archivo)
        .await?
        .ok_or(AppError::NoEncontrado(que))?;
    almacen.obtener(&a.clave).await
}

/// Agrega los `ApplicationResponse` firmados de unos eventos RADIAN a `eventos/`.
async fn agregar_eventos<T: TxFacturacion>(
    tx: &mut T,
    almacen: &impl AlmacenArchivos,
    paquete: &mut Paquete,
    base: &str,
    cufe: &str,
    eventos: Vec<RegistroRadian>,
) -> AppResult<()> {
    for e in eventos {
        let Some(xml) = e.xml_archivo_id else {
            continue;
        };
        let nombre = format!(
            "eventos/{base}-{}-{}.xml",
            e.tipo.codigo(),
            e.numero.as_deref().unwrap_or("sin-numero")
        );
        let contenido = leer(tx, almacen, xml, "XML del evento RADIAN").await?;
        let referencia = Referencia {
            documento: base,
            cufe,
            estado: &e.estado,
            fecha: e.creado_en.date_naive().to_string(),
        };
        paquete.agregar(nombre, "evento_radian", &referencia, contenido);
    }
    Ok(())
}

/// Paquete ZIP para conservar los documentos electrónicos del periodo (y entregarlos al
/// contador):
///
/// - `emitidos/`: de cada documento transmitido (validado, enviado o en contingencia) creado en
///   el periodo, el XML firmado, el `AttachedDocument` (con la respuesta de la DIAN) y el PDF;
/// - `recibidos/`: el XML de cada factura recibida con fecha en el periodo;
/// - `eventos/`: los eventos RADIAN firmados de unos y otros;
/// - `indice.csv`: una fila por archivo con su documento, CUFE, estado, fecha y SHA-256.
///
/// El facturador es quien conserva sus documentos (Res. 000227 de 2025, título 5): con este
/// paquete puede llevárselos en cualquier momento, periodo por periodo.
///
/// # Errors
///
/// Los de [`libro_ventas`]; [`AppError::Infraestructura`] si no se lee un archivo o falla el PDF.
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

    let mut paquete = Paquete::default();
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
        let cufe = d.cufe.clone().unwrap_or_default();
        let referencia = Referencia {
            documento: &base,
            cufe: &cufe,
            estado: d.estado.as_str(),
            fecha: d.creado_en.date_naive().to_string(),
        };
        if let Some(xml) = d.xml_archivo_id {
            let contenido = leer(&mut tx, almacen, xml, "XML del documento").await?;
            paquete.agregar(
                format!("emitidos/{base}.xml"),
                "xml",
                &referencia,
                contenido,
            );
        }
        if let Some(adjunto) = d.adjunto_archivo_id {
            let contenido = leer(&mut tx, almacen, adjunto, "AttachedDocument").await?;
            paquete.agregar(
                format!("emitidos/{base}-adjunto.xml"),
                "attached_document",
                &referencia,
                contenido,
            );
        }
        let pdf = pdf_documento(&mut tx, generador, reloj, d, pie).await?;
        paquete.agregar(
            format!("emitidos/{base}.pdf"),
            "representacion_grafica",
            &referencia,
            pdf,
        );
        let eventos = tx.eventos_radian_documento(d.id).await?;
        agregar_eventos(&mut tx, almacen, &mut paquete, &base, &cufe, eventos).await?;
        registros += 1;
    }
    for f in &recibidas {
        let base = format!("{}-{}{}", f.emisor_nit, f.prefijo, f.numero);
        let referencia = Referencia {
            documento: &base,
            cufe: &f.cufe,
            estado: f.estado.as_str(),
            fecha: f.fecha.to_string(),
        };
        let contenido = leer(&mut tx, almacen, f.archivo_id, "XML de la factura recibida").await?;
        paquete.agregar(
            format!("recibidos/{base}.xml"),
            "factura_recibida",
            &referencia,
            contenido,
        );
        let eventos = tx.eventos_radian(f.id).await?;
        agregar_eventos(&mut tx, almacen, &mut paquete, &base, &f.cufe, eventos).await?;
        registros += 1;
    }
    let zip = empaquetador.empaquetar(&paquete.cerrar())?;
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
