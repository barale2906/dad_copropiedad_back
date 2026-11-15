//! Eventos RADIAN propios (anexo técnico 1.9, §6.5, §8.5–8.6 y §11.5): `ApplicationResponse`
//! firmado por quien genera el evento, con su CUDE calculado con el PIN del software. Se envía
//! comprimido con `SendEventUpdateStatus`.

use chrono::{DateTime, Datelike, FixedOffset};
use sha2::{Digest, Sha384};

use super::control::{Ambiente, hora_dian, url_qr};
use super::firma::{CredencialFirma, firmar_documento};
use super::ubl::{
    Parte, Software, TIPO_DOCUMENTO_NIT, TipoPersona, extensiones_dian, nit_dian, raiz, tributo,
};
use super::xml::{Elemento, documento};
use super::{DocumentoFirmado, ErrorMotor, nombre_archivo};

const ESPACIO: &str = "urn:oasis:names:specification:ubl:schema:xsd:ApplicationResponse-2";
const PERFIL: &str = "DIAN 2.1: ApplicationResponse de la Factura Electrónica de Venta";
/// Tipo del documento referenciado: factura electrónica de venta.
const TIPO_FACTURA: &str = "01";

/// Persona que recibió la factura o el bien (eventos 030 y 032).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaEvento {
    /// Código DIAN del tipo de documento (`13` cédula, `31` NIT…).
    pub tipo_documento: String,
    pub documento: String,
    /// DV, solo si el documento es un NIT.
    pub dv: Option<String>,
    pub nombres: String,
    pub apellidos: String,
    pub cargo: Option<String>,
    pub area: Option<String>,
}

/// Datos de un evento RADIAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventoUbl {
    /// Código del evento (`030`…`034`).
    pub codigo: String,
    /// Nombre literal del evento.
    pub descripcion: String,
    /// Número del evento.
    pub numero: String,
    pub emitido: DateTime<FixedOffset>,
    pub ambiente: Ambiente,
    pub generador: Parte,
    pub receptor: Parte,
    /// Prefijo y número de la factura.
    pub factura_numero: String,
    pub factura_cufe: String,
    /// Concepto del reclamo (`listID` y `name`).
    pub concepto: Option<(String, String)>,
    pub quien_recibe: Option<PersonaEvento>,
    pub nota: Option<String>,
}

/// CUDE del evento: SHA-384(Num_DE + Fec_Emi + Hor_Emi + NitFE + DocAdq + ResponseCode + ID +
/// DocumentTypeCode + Software-PIN) (anexo §11.5).
#[allow(clippy::too_many_arguments)]
pub fn cude_evento(
    numero: &str,
    fecha: &str,
    hora: &str,
    generador: &str,
    receptor: &str,
    codigo: &str,
    factura: &str,
    tipo_documento: &str,
    pin: &str,
) -> String {
    hex::encode(Sha384::digest(
        format!("{numero}{fecha}{hora}{generador}{receptor}{codigo}{factura}{tipo_documento}{pin}")
            .as_bytes(),
    ))
}

fn parte(nombre: &str, p: &Parte) -> Elemento {
    let organizacion = match p.tipo_persona {
        TipoPersona::Juridica => "1",
        TipoPersona::Natural => "2",
    };
    Elemento::new(nombre).hijo(
        Elemento::new("cac:PartyTaxScheme")
            .hijo(Elemento::hoja("cbc:RegistrationName", p.nombre.clone()))
            .hijo(
                nit_dian(
                    "cbc:CompanyID",
                    &p.documento,
                    p.dv.as_deref(),
                    &p.tipo_documento,
                )
                .attr("schemeVersionID", organizacion),
            )
            .hijo(tributo(p.regimen)),
    )
}

fn persona(p: &PersonaEvento) -> Elemento {
    let id = Elemento::hoja("cbc:ID", p.documento.clone()).attr("schemeName", &p.tipo_documento);
    let id = match &p.dv {
        Some(dv) if p.tipo_documento == TIPO_DOCUMENTO_NIT => id.attr("schemeID", dv),
        _ => id,
    };
    Elemento::new("cac:IssuerParty").hijo(
        Elemento::new("cac:Person")
            .hijo(id)
            .hijo(Elemento::hoja("cbc:FirstName", p.nombres.clone()))
            .hijo(Elemento::hoja("cbc:FamilyName", p.apellidos.clone()))
            .hijo_si(p.cargo.clone().map(|c| Elemento::hoja("cbc:JobTitle", c)))
            .hijo_si(
                p.area
                    .clone()
                    .map(|a| Elemento::hoja("cbc:OrganizationDepartment", a)),
            ),
    )
}

/// Genera el evento sin firmar y su CUDE.
///
/// # Errors
///
/// [`ErrorMotor::Datos`] si al evento le falta un dato que su tipo exige.
pub(crate) fn generar(
    e: &EventoUbl,
    software: &Software,
) -> Result<(Elemento, String), ErrorMotor> {
    let datos = |t: &str| ErrorMotor::Datos(t.to_owned());
    if (e.codigo == "030" || e.codigo == "032") && e.quien_recibe.is_none() {
        return Err(datos("el evento necesita a la persona que recibió"));
    }
    if e.codigo == "031" && e.concepto.is_none() {
        return Err(datos("el reclamo necesita su concepto"));
    }
    if e.codigo == "034" && e.nota.is_none() {
        return Err(datos("la aceptación tácita necesita su constancia"));
    }
    let fecha = e.emitido.date_naive().to_string();
    let hora = hora_dian(e.emitido);
    let cude = cude_evento(
        &e.numero,
        &fecha,
        &hora,
        &e.generador.documento,
        &e.receptor.documento,
        &e.codigo,
        &e.factura_numero,
        TIPO_FACTURA,
        &software.pin,
    );
    let qr = url_qr(&cude, e.ambiente);
    let codigo = {
        let c = Elemento::hoja("cbc:ResponseCode", e.codigo.clone());
        match &e.concepto {
            Some((id, nombre)) => c.attr("listID", id).attr("name", nombre),
            None => c,
        }
    };
    let doc = raiz("ApplicationResponse", ESPACIO)
        .hijo(
            Elemento::new("ext:UBLExtensions")
                .hijo(
                    Elemento::new("ext:UBLExtension").hijo(
                        Elemento::new("ext:ExtensionContent")
                            .hijo(extensiones_dian(None, software, &e.numero, &qr)),
                    ),
                )
                .hijo(
                    Elemento::new("ext:UBLExtension").hijo(Elemento::new("ext:ExtensionContent")),
                ),
        )
        .hijo(Elemento::hoja("cbc:UBLVersionID", "UBL 2.1"))
        .hijo(Elemento::hoja("cbc:CustomizationID", "1"))
        .hijo(Elemento::hoja("cbc:ProfileID", PERFIL))
        .hijo(Elemento::hoja(
            "cbc:ProfileExecutionID",
            e.ambiente.codigo(),
        ))
        .hijo(Elemento::hoja("cbc:ID", e.numero.clone()))
        .hijo(
            Elemento::hoja("cbc:UUID", cude.clone())
                .attr("schemeID", e.ambiente.codigo())
                .attr("schemeName", "CUDE-SHA384"),
        )
        .hijo(Elemento::hoja("cbc:IssueDate", fecha))
        .hijo(Elemento::hoja("cbc:IssueTime", hora))
        .hijo_si(e.nota.clone().map(|n| Elemento::hoja("cbc:Note", n)))
        .hijo(parte("cac:SenderParty", &e.generador))
        .hijo(parte("cac:ReceiverParty", &e.receptor))
        .hijo(
            Elemento::new("cac:DocumentResponse")
                .hijo(
                    Elemento::new("cac:Response")
                        .hijo(codigo)
                        .hijo(Elemento::hoja("cbc:Description", e.descripcion.clone())),
                )
                .hijo(
                    Elemento::new("cac:DocumentReference")
                        .hijo(Elemento::hoja("cbc:ID", e.factura_numero.clone()))
                        .hijo(
                            Elemento::hoja("cbc:UUID", e.factura_cufe.clone())
                                .attr("schemeName", "CUFE-SHA384"),
                        )
                        .hijo(Elemento::hoja("cbc:DocumentTypeCode", TIPO_FACTURA)),
                )
                .hijo_si(e.quien_recibe.as_ref().map(persona)),
        );
    Ok((doc, cude))
}

/// Genera y firma el evento. `consecutivo_archivo` es el número de archivos enviados en el año.
///
/// # Errors
///
/// Los de `generar`; [`ErrorMotor::Certificado`] o [`ErrorMotor::Firma`] si no se puede firmar.
pub fn preparar_evento(
    e: &EventoUbl,
    software: &Software,
    credencial: &CredencialFirma,
    consecutivo_archivo: u32,
    id_firma: &str,
) -> Result<DocumentoFirmado, ErrorMotor> {
    let (mut raiz, cude) = generar(e, software)?;
    firmar_documento(&mut raiz, credencial, e.emitido, id_firma)?;
    let anio = e.emitido.year();
    let nit = &e.generador.documento;
    Ok(DocumentoFirmado {
        xml: documento(&raiz),
        qr: url_qr(&cude, e.ambiente),
        codigo: cude,
        numero: e.numero.clone(),
        nombre_xml: format!(
            "{}.xml",
            nombre_archivo("ar", nit, anio, consecutivo_archivo)
        ),
        nombre_zip: format!(
            "{}.zip",
            nombre_archivo("z", nit, anio, consecutivo_archivo)
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::propio::{firma::tests as firma_pruebas, ubl::tests as ubl_pruebas};

    #[test]
    fn el_cude_del_evento_es_el_del_ejemplo_oficial() {
        // Anexo técnico 1.9, §11.5.1.
        assert_eq!(
            cude_evento(
                "1",
                "2019-04-30",
                "19:48:50-05:00",
                "99998888",
                "800197268",
                "030",
                "FE123",
                "01",
                "11111"
            ),
            "0d91ba25b01f5e7dbda870a11b274501d3a62a73e91932c473c86c93f12a142a2ac45876efcde3e679024a01c0be41f9"
        );
    }

    fn evento(codigo: &str) -> EventoUbl {
        let f = ubl_pruebas::factura();
        EventoUbl {
            codigo: codigo.into(),
            descripcion: "Acuse de recibo de Factura Electrónica de Venta".into(),
            numero: "EV2026000001".into(),
            emitido: f.emitido,
            ambiente: Ambiente::Pruebas,
            generador: f.adquirente.clone(),
            receptor: f.emisor.clone(),
            factura_numero: "SETP990000001".into(),
            factura_cufe: "a".repeat(96),
            concepto: None,
            quien_recibe: Some(PersonaEvento {
                tipo_documento: "13".into(),
                documento: "1020304050".into(),
                dv: None,
                nombres: "Ana".into(),
                apellidos: "Pérez".into(),
                cargo: Some("Auxiliar contable".into()),
                area: None,
            }),
            nota: None,
        }
    }

    #[test]
    fn el_acuse_firmado_lleva_su_cude_la_factura_y_quien_recibio() {
        let credencial = firma_pruebas::credencial();
        let firmado = preparar_evento(
            &evento("030"),
            &ubl_pruebas::software(),
            &credencial,
            7,
            "e1",
        )
        .expect("evento firmado");

        let xml = String::from_utf8(firmado.xml).expect("utf-8");
        assert!(
            firmado.nombre_xml.starts_with("ar"),
            "{}",
            firmado.nombre_xml
        );
        assert!(xml.contains("<ApplicationResponse"));
        assert!(xml.contains("<cbc:ResponseCode>030</cbc:ResponseCode>"));
        assert!(xml.contains(&format!("schemeName=\"CUDE-SHA384\">{}<", firmado.codigo)));
        assert!(xml.contains("<cbc:DocumentTypeCode>01</cbc:DocumentTypeCode>"));
        assert!(xml.contains("<cbc:FirstName>Ana</cbc:FirstName>"));
        assert!(xml.contains("ds:Signature"));
    }

    #[test]
    fn un_evento_sin_sus_datos_obligatorios_no_se_genera() {
        let software = ubl_pruebas::software();
        let mut sin_persona = evento("032");
        sin_persona.quien_recibe = None;
        let mut reclamo = evento("031");
        reclamo.quien_recibe = None;

        assert!(generar(&sin_persona, &software).is_err());
        assert!(generar(&reclamo, &software).is_err(), "sin concepto");
        reclamo.concepto = Some(("01".into(), "Documento con inconsistencias".into()));
        let (doc, _) = generar(&reclamo, &software).expect("reclamo");
        let xml = String::from_utf8(documento(&doc)).expect("utf-8");
        assert!(xml.contains("listID=\"01\""));
    }
}
