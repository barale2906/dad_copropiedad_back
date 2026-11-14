//! Motor propio de documentos electrónicos de la DIAN (plan 17, D3 etapa 2).
//!
//! Genera, firma y transmite los documentos directamente a la DIAN, sin proveedor tecnológico:
//! la entidad se habilita con este software en la modalidad de desarrollo informático propio o
//! adquirido (Res. 000165 de 2023, art. 28 num. 2.1; el fabricante es GISLA SAS). Sigue el
//! anexo técnico de factura electrónica de venta **versión 1.9**.
//!
//! | Pieza | Módulo |
//! |---|---|
//! | CUFE, CUDE, código de seguridad del software y QR (fase 1) | [`control`] |
//! | Factura, nota crédito y nota débito en UBL 2.1 (fase 2) | [`ubl`] |
//! | Firma XAdES-EPES con el certificado del facturador (fase 3) | [`firma`] |
//! | Cliente SOAP 1.2 + WS-Security de los servicios web (fase 4) | [`servicio`] |
//!
//! La forma canónica XML (módulo privado `xml`), la lectura de certificados X.509 y el ZIP de envío son
//! propios y mínimos: cubren solo lo que el motor genera y evitan OpenSSL y `libxmlsec1`
//! (AGENTS.md §4). Nada de esto escribe logs con el contenido de los documentos, el PIN del
//! software ni la clave técnica.

pub mod control;
pub mod firma;
pub mod servicio;
pub mod ubl;
mod x509;
mod xml;
mod zip;

use chrono::Datelike;

use firma::CredencialFirma;
use ubl::{Clase, DocumentoUbl, Software};

/// Documento generado y firmado, listo para transmitir y para entregar al adquirente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentoFirmado {
    /// XML firmado (UTF-8).
    pub xml: Vec<u8>,
    /// CUFE (factura) o CUDE (notas).
    pub codigo: String,
    /// Prefijo y número.
    pub numero: String,
    /// Nombre del XML según el estándar de la DIAN (anexo §6.5.7).
    pub nombre_xml: String,
    /// Nombre del ZIP de envío (anexo §6.5.8).
    pub nombre_zip: String,
    /// Texto del QR de la representación gráfica.
    pub qr: String,
}

impl DocumentoFirmado {
    /// ZIP con el XML, como lo reciben los servicios web.
    pub fn zip(&self) -> Vec<u8> {
        zip::comprimir(&[(&self.nombre_xml, &self.xml)])
    }
}

/// Nombre del archivo según el estándar de la DIAN: clase, NIT a diez dígitos, `000` (software
/// propio), año a dos dígitos y consecutivo de archivos enviados en el año, en hexadecimal a
/// ocho dígitos (anexo §6.5.7: `fv08001972680001900000011.xml`).
fn nombre_archivo(clase: &str, nit: &str, anio: i32, consecutivo: u32) -> String {
    format!(
        "{clase}{nit:0>10}000{:02}{consecutivo:08x}",
        anio.rem_euclid(100)
    )
}

/// Genera y firma un documento.
///
/// `consecutivo_archivo` es el número de archivos enviados por el facturador en el año (se
/// reinicia cada 1 de enero); `id_firma` identifica la firma dentro del XML (el id del documento
/// electrónico sirve). La hora de la firma es la de emisión.
///
/// # Errors
///
/// [`ErrorMotor::Datos`] si el documento no se puede generar; [`ErrorMotor::Certificado`] o
/// [`ErrorMotor::Firma`] si no se puede firmar.
pub fn preparar(
    documento: &DocumentoUbl,
    software: &Software,
    credencial: &CredencialFirma,
    consecutivo_archivo: u32,
    id_firma: &str,
) -> Result<DocumentoFirmado, ErrorMotor> {
    let mut generado = ubl::generar(documento, software)?;
    firma::firmar_documento(&mut generado.raiz, credencial, documento.emitido, id_firma)?;
    let clase = match documento.clase {
        Clase::Factura { .. } => "fv",
        Clase::NotaCredito { .. } => "nc",
        Clase::NotaDebito { .. } => "nd",
    };
    let anio = documento.emitido.year();
    let nit = &documento.emisor.documento;
    Ok(DocumentoFirmado {
        xml: xml::documento(&generado.raiz),
        codigo: generado.codigo,
        numero: generado.numero,
        nombre_xml: format!(
            "{}.xml",
            nombre_archivo(clase, nit, anio, consecutivo_archivo)
        ),
        nombre_zip: format!(
            "{}.zip",
            nombre_archivo("z", nit, anio, consecutivo_archivo)
        ),
        qr: generado.qr,
    })
}

/// Error del motor propio.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorMotor {
    /// Los datos del documento no permiten generarlo (falta un dato o no cuadra).
    #[error("datos del documento inválidos: {0}")]
    Datos(String),
    /// El certificado digital no se puede usar (ilegible, vencido o sin no repudio).
    #[error("certificado digital: {0}")]
    Certificado(String),
    /// Falló el cálculo de la firma.
    #[error("firma: {0}")]
    Firma(String),
    /// No se pudo hablar con el servicio web (red, TLS, HTTP): el documento queda en
    /// contingencia.
    #[error("servicio web de la DIAN no disponible: {0}")]
    Transporte(String),
    /// El servicio respondió algo que no se entiende o un SOAP Fault.
    #[error("respuesta del servicio web: {0}")]
    Respuesta(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::propio::{firma::tests as firma_pruebas, ubl::tests as ubl_pruebas};

    #[test]
    fn nombre_de_archivo_sigue_el_estandar_de_la_dian() {
        // Ejemplo del anexo §6.5.7: undécima factura del NIT 800197268 en 2019 (0x11 = 17 en
        // el ejemplo, que usa hexadecimal).
        assert_eq!(
            nombre_archivo("fv", "800197268", 2019, 0x11),
            "fv08001972680001900000011"
        );
        assert_eq!(
            nombre_archivo("z", "901713998", 2027, 255),
            "z090171399800027000000ff"
        );
    }

    #[test]
    fn preparar_genera_firma_y_nombra_el_documento() {
        let firmado = preparar(
            &ubl_pruebas::factura(),
            &ubl_pruebas::software(),
            &firma_pruebas::credencial(),
            1,
            "doc-1",
        )
        .expect("documento firmado");
        let xml = String::from_utf8(firmado.xml.clone()).expect("utf-8");
        assert!(xml.contains("<ds:Signature Id=\"xmldsig-doc-1\">"));
        assert!(xml.contains(&firmado.codigo));
        assert_eq!(firmado.nombre_xml, "fv09017139980002700000001.xml");
        assert_eq!(firmado.nombre_zip, "z09017139980002700000001.zip");
        let zip = firmado.zip();
        assert!(
            zip.windows(firmado.xml.len())
                .any(|w| w == firmado.xml.as_slice())
        );
    }

    /// Escribe documentos firmados en `DAD_FE_SALIDA` para verificarlos con herramientas
    /// externas (esquemas XSD de la caja de herramientas de la DIAN y un verificador XAdES
    /// independiente). Manual: `DAD_FE_SALIDA=/ruta cargo test -p infra-facturacion
    /// exportar_documentos -- --ignored`.
    #[test]
    #[ignore = "verificación manual con herramientas externas"]
    fn exportar_documentos_firmados_para_verificacion_externa() {
        let salida = std::env::var("DAD_FE_SALIDA").expect("DAD_FE_SALIDA");
        let factura = ubl_pruebas::factura();
        let mut nota_credito = factura.clone();
        nota_credito.clase = Clase::NotaCredito {
            referencia: ubl::Referencia {
                numero: "SETP990000001".into(),
                cufe: "a".repeat(96),
                fecha: factura.emitido.date_naive(),
            },
            correccion: ubl::Correccion {
                concepto: "2".into(),
                descripcion: "Anulación".into(),
            },
        };
        nota_credito.prefijo = "NC".into();
        let mut nota_debito = nota_credito.clone();
        if let Clase::NotaCredito {
            referencia,
            correccion,
        } = nota_credito.clase.clone()
        {
            nota_debito.clase = Clase::NotaDebito {
                referencia,
                correccion,
            };
        }
        nota_debito.prefijo = "ND".into();
        for (nombre, d) in [
            ("factura", factura),
            ("nota_credito", nota_credito),
            ("nota_debito", nota_debito),
        ] {
            let firmado = preparar(
                &d,
                &ubl_pruebas::software(),
                &firma_pruebas::credencial(),
                1,
                nombre,
            )
            .expect("firmado");
            std::fs::write(format!("{salida}/{nombre}.xml"), &firmado.xml).expect("escribir");
        }
    }
}
