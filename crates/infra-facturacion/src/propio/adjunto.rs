//! Contenedor que se entrega al adquirente (`AttachedDocument`, anexo técnico 1.9 §7.10): el
//! documento firmado y, si la DIAN ya respondió, su `ApplicationResponse` con el resultado de la
//! validación. En contingencia (la DIAN no responde) va sin respuesta. Se firma con el mismo
//! certificado del facturador. Los XML internos van como texto del `cbc:Description` (escapados:
//! equivale al `CDATA` de los ejemplos de la DIAN y la forma canónica no cambia).

use chrono::{DateTime, Datelike, FixedOffset};

use super::firma::{CredencialFirma, firmar_documento};
use super::ubl::{Clase, DocumentoUbl, Parte, nit_dian, raiz, tributo};
use super::xml::{Elemento, documento};
use super::{DocumentoFirmado, ErrorMotor, nombre_archivo};

const ESPACIO: &str = "urn:oasis:names:specification:ubl:schema:xsd:AttachedDocument-2";
const VALIDADOR: &str = "Unidad Especial Dirección de Impuestos y Aduanas Nacionales";

/// `AttachedDocument` firmado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdjuntoFirmado {
    /// XML firmado (UTF-8).
    pub xml: Vec<u8>,
    /// Nombre del archivo según el estándar de la DIAN (`ad…xml`).
    pub nombre: String,
}

fn parte(nombre: &str, p: &Parte) -> Elemento {
    let responsabilidades = if p.responsabilidades.is_empty() {
        "R-99-PN".to_owned()
    } else {
        p.responsabilidades.join(";")
    };
    Elemento::new(nombre).hijo(
        Elemento::new("cac:PartyTaxScheme")
            .hijo(Elemento::hoja("cbc:RegistrationName", p.nombre.clone()))
            .hijo(nit_dian(
                "cbc:CompanyID",
                &p.documento,
                p.dv.as_deref(),
                &p.tipo_documento,
            ))
            .hijo(
                Elemento::hoja("cbc:TaxLevelCode", responsabilidades)
                    .attr("listName", p.regimen.lista()),
            )
            .hijo(tributo(p.regimen)),
    )
}

fn adjunto_xml(contenido: &[u8]) -> Elemento {
    Elemento::new("cac:Attachment").hijo(
        Elemento::new("cac:ExternalReference")
            .hijo(Elemento::hoja("cbc:MimeCode", "text/xml"))
            .hijo(Elemento::hoja("cbc:EncodingCode", "UTF-8"))
            .hijo(Elemento::hoja(
                "cbc:Description",
                String::from_utf8_lossy(contenido).into_owned(),
            )),
    )
}

/// Arma y firma el `AttachedDocument` del documento ya firmado. `respuesta` es el
/// `ApplicationResponse` de la DIAN (`None` en contingencia) y `validado`, cuándo lo validó.
///
/// # Errors
///
/// [`ErrorMotor::Firma`] o [`ErrorMotor::Certificado`] si no se puede firmar.
pub fn generar(
    d: &DocumentoUbl,
    firmado: &DocumentoFirmado,
    respuesta: Option<(&[u8], DateTime<FixedOffset>)>,
    credencial: &CredencialFirma,
    consecutivo_archivo: u32,
    id_firma: &str,
) -> Result<AdjuntoFirmado, ErrorMotor> {
    let (perfil, tipo) = match d.clase {
        Clase::Factura { .. } => (
            "Factura Electrónica de Venta",
            "Contenedor de Factura Electrónica",
        ),
        Clase::NotaCredito { .. } => ("Nota Crédito", "Contenedor de Nota Crédito"),
        Clase::NotaDebito { .. } => ("Nota Débito", "Contenedor de Nota Débito"),
    };
    let fecha = d.emitido.date_naive().to_string();
    let hora = d.emitido.format("%H:%M:%S%:z").to_string();
    let referencia = Elemento::new("cac:DocumentReference")
        .hijo(Elemento::hoja("cbc:ID", firmado.numero.clone()))
        .hijo(Elemento::hoja("cbc:UUID", firmado.codigo.clone()).attr("schemeName", "CUFE-SHA384"))
        .hijo(Elemento::hoja("cbc:IssueDate", fecha.clone()))
        .hijo(Elemento::hoja("cbc:DocumentType", "ApplicationResponse"))
        .hijo_si(respuesta.map(|(xml, _)| adjunto_xml(xml)))
        .hijo(
            Elemento::new("cac:ResultOfVerification")
                .hijo(Elemento::hoja("cbc:ValidatorID", VALIDADOR))
                .hijo(Elemento::hoja(
                    "cbc:ValidationResultCode",
                    if respuesta.is_some() { "02" } else { "04" },
                ))
                .hijo_si(respuesta.map(|(_, cuando)| {
                    Elemento::hoja("cbc:ValidationDate", cuando.date_naive().to_string())
                }))
                .hijo_si(respuesta.map(|(_, cuando)| {
                    Elemento::hoja(
                        "cbc:ValidationTime",
                        cuando.format("%H:%M:%S%:z").to_string(),
                    )
                })),
        );
    let mut doc =
        raiz("AttachedDocument", ESPACIO)
            .hijo(Elemento::new("ext:UBLExtensions").hijo(
                Elemento::new("ext:UBLExtension").hijo(Elemento::new("ext:ExtensionContent")),
            ))
            .hijo(Elemento::hoja("cbc:UBLVersionID", "UBL 2.1"))
            .hijo(Elemento::hoja("cbc:CustomizationID", "Documentos adjuntos"))
            .hijo(Elemento::hoja("cbc:ProfileID", perfil))
            .hijo(Elemento::hoja(
                "cbc:ProfileExecutionID",
                d.ambiente.codigo(),
            ))
            .hijo(Elemento::hoja("cbc:ID", firmado.numero.clone()))
            .hijo(Elemento::hoja("cbc:IssueDate", fecha))
            .hijo(Elemento::hoja("cbc:IssueTime", hora))
            .hijo(Elemento::hoja("cbc:DocumentType", tipo))
            .hijo(Elemento::hoja(
                "cbc:ParentDocumentID",
                firmado.numero.clone(),
            ))
            .hijo(parte("cac:SenderParty", &d.emisor))
            .hijo(parte("cac:ReceiverParty", &d.adquirente))
            .hijo(adjunto_xml(&firmado.xml))
            .hijo(
                Elemento::new("cac:ParentDocumentLineReference")
                    .hijo(Elemento::hoja("cbc:LineID", "1"))
                    .hijo(referencia),
            );
    firmar_documento(&mut doc, credencial, d.emitido, id_firma)?;
    Ok(AdjuntoFirmado {
        xml: documento(&doc),
        nombre: format!(
            "{}.xml",
            nombre_archivo(
                "ad",
                &d.emisor.documento,
                d.emitido.year(),
                consecutivo_archivo
            )
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::propio::{firma::tests as firma_pruebas, preparar, ubl::tests as ubl_pruebas};

    #[test]
    fn el_contenedor_lleva_el_documento_la_respuesta_y_su_firma() {
        let d = ubl_pruebas::factura();
        let credencial = firma_pruebas::credencial();
        let firmado =
            preparar(&d, &ubl_pruebas::software(), &credencial, 1, "f1").expect("firmado");

        let a = generar(
            &d,
            &firmado,
            Some((b"<ApplicationResponse/>", d.emitido)),
            &credencial,
            1,
            "a1",
        )
        .expect("adjunto");

        let xml = String::from_utf8(a.xml).expect("utf-8");
        assert!(
            a.nombre.starts_with("ad") && a.nombre.ends_with(".xml"),
            "{}",
            a.nombre
        );
        assert!(xml.contains("<AttachedDocument"));
        assert!(xml.contains(&firmado.codigo), "el CUFE va en la referencia");
        assert!(
            xml.contains("&lt;ApplicationResponse/&gt;"),
            "la respuesta va adjunta"
        );
        assert!(xml.contains("<cbc:ValidationResultCode>02</cbc:ValidationResultCode>"));
        assert!(xml.contains("ds:Signature"), "el contenedor va firmado");
    }

    #[test]
    fn en_contingencia_va_sin_respuesta_de_la_dian() {
        let d = ubl_pruebas::factura();
        let credencial = firma_pruebas::credencial();
        let firmado =
            preparar(&d, &ubl_pruebas::software(), &credencial, 1, "f1").expect("firmado");

        let a = generar(&d, &firmado, None, &credencial, 1, "a1").expect("adjunto");

        let xml = String::from_utf8(a.xml).expect("utf-8");
        assert!(xml.contains("<cbc:ValidationResultCode>04</cbc:ValidationResultCode>"));
        assert!(!xml.contains("ValidationDate"));
    }
}
