//! Cliente de los servicios web de la DIAN (anexo técnico 1.9, §7).
//!
//! SOAP 1.2 *document/literal* con WS-Addressing y WS-Security 1.0 (X.509 Token Profile 1.1):
//! la cabecera lleva el certificado del facturador (`BinarySecurityToken`), un `Timestamp` y una
//! firma RSA-SHA256 de `wsa:To` con C14N exclusiva. Los documentos van como ZIP en Base64.
//!
//! La URL del servicio no se fija en el código: la publica la DIAN en el catálogo de
//! participantes (habilitación o producción) y llega por configuración.

use std::{sync::Arc, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, NaiveDate, Utc};

use super::{
    ErrorMotor,
    firma::{CredencialFirma, sha256_b64},
    xml::{Canon, Elemento, canonica},
};

const SOAP: &str = "http://www.w3.org/2003/05/soap-envelope";
const WCF: &str = "http://wcf.dian.colombia";
const WSA: &str = "http://www.w3.org/2005/08/addressing";
const WSSE: &str =
    "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd";
const WSU: &str =
    "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd";
const DS: &str = "http://www.w3.org/2000/09/xmldsig#";
const EXC_C14N: &str = "http://www.w3.org/2001/10/xml-exc-c14n#";
const X509V3: &str =
    "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-x509-token-profile-1.0#X509v3";
const BASE64_BINARY: &str = "http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary";
const ACCION: &str = "http://wcf.dian.colombia/IWcfDianCustomerServices/";
/// Vigencia del `Timestamp` de seguridad (la DIAN responde con cinco minutos).
const VIGENCIA_TIMESTAMP: chrono::Duration = chrono::Duration::minutes(5);
const TIMEOUT: Duration = Duration::from_secs(60);

/// Respuesta de validación de la DIAN (`SendBillSync`, `GetStatus`, `GetStatusZip`,
/// `SendEventUpdateStatus`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RespuestaDian {
    /// `IsValid`: el documento quedó validado.
    pub valido: bool,
    /// `StatusCode` (`00` procesado, `99` con errores, `66` no encontrado…).
    pub codigo: String,
    pub descripcion: Option<String>,
    pub mensaje: Option<String>,
    /// Reglas incumplidas (`Regla: FAD06 …`).
    pub errores: Vec<String>,
    /// `ApplicationResponse` de la DIAN, decodificado de `XmlBase64Bytes`.
    pub application_response: Option<Vec<u8>>,
    /// `XmlDocumentKey`: CUFE/CUDE o identificador del envío.
    pub clave: Option<String>,
}

/// Rango de numeración autorizado (`GetNumberingRange`).
#[derive(Clone, PartialEq, Eq)]
pub struct RangoNumeracion {
    pub resolucion: String,
    pub fecha_resolucion: Option<NaiveDate>,
    pub prefijo: String,
    pub desde: u64,
    pub hasta: u64,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: NaiveDate,
    /// Clave técnica del rango: secreta (entra al CUFE).
    pub clave_tecnica: String,
}

impl std::fmt::Debug for RangoNumeracion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RangoNumeracion")
            .field("resolucion", &self.resolucion)
            .field("prefijo", &self.prefijo)
            .field("desde", &self.desde)
            .field("hasta", &self.hasta)
            .field("vigente_hasta", &self.vigente_hasta)
            .field("clave_tecnica", &"<oculta>")
            .finish()
    }
}

/// Cliente de los servicios web. Uno por ambiente (URL); el certificado de cada facturador se
/// pasa en cada llamada.
#[derive(Debug, Clone)]
pub struct ServicioDian {
    http: reqwest::Client,
    url: Arc<str>,
}

impl ServicioDian {
    /// Cliente para la URL del servicio (`https://…/WcfDianCustomerServices.svc`), con TLS de
    /// rustls + ring y las raíces de `webpki-roots` (sin OpenSSL).
    ///
    /// # Errors
    ///
    /// [`ErrorMotor::Transporte`] si no se puede construir el cliente HTTP.
    pub fn new(url: &str) -> Result<Self, ErrorMotor> {
        let mut raices = rustls::RootCertStore::empty();
        raices.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| ErrorMotor::Transporte(e.to_string()))?
        .with_root_certificates(raices)
        .with_no_client_auth();
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .timeout(TIMEOUT)
            .build()
            .map_err(|e| ErrorMotor::Transporte(e.to_string()))?;
        Ok(Self {
            http,
            url: url.into(),
        })
    }

    /// Envía un documento y espera su validación (`SendBillSync`).
    ///
    /// # Errors
    ///
    /// [`ErrorMotor::Transporte`] sin servicio; [`ErrorMotor::Respuesta`] si responde un SOAP
    /// Fault o algo ilegible.
    pub async fn enviar(
        &self,
        credencial: &CredencialFirma,
        ahora: DateTime<Utc>,
        nombre_zip: &str,
        zip: &[u8],
    ) -> Result<RespuestaDian, ErrorMotor> {
        let cuerpo = Elemento::new("wcf:SendBillSync")
            .hijo(Elemento::hoja("wcf:fileName", nombre_zip))
            .hijo(Elemento::hoja("wcf:contentFile", STANDARD.encode(zip)));
        let xml = self
            .llamar(credencial, ahora, "SendBillSync", cuerpo)
            .await?;
        respuesta_unica(&xml)
    }

    /// Envía un lote de hasta 50 documentos sin esperar (`SendBillAsync`); devuelve el
    /// identificador para [`ServicioDian::estado_lote`].
    ///
    /// # Errors
    ///
    /// Los de [`ServicioDian::enviar`].
    pub async fn enviar_lote(
        &self,
        credencial: &CredencialFirma,
        ahora: DateTime<Utc>,
        nombre_zip: &str,
        zip: &[u8],
    ) -> Result<String, ErrorMotor> {
        let cuerpo = Elemento::new("wcf:SendBillAsync")
            .hijo(Elemento::hoja("wcf:fileName", nombre_zip))
            .hijo(Elemento::hoja("wcf:contentFile", STANDARD.encode(zip)));
        let xml = self
            .llamar(credencial, ahora, "SendBillAsync", cuerpo)
            .await?;
        clave_zip(&xml)
    }

    /// Envía documentos del set de pruebas de habilitación (`SendTestSetAsync`).
    ///
    /// # Errors
    ///
    /// Los de [`ServicioDian::enviar`].
    pub async fn enviar_set_pruebas(
        &self,
        credencial: &CredencialFirma,
        ahora: DateTime<Utc>,
        nombre_zip: &str,
        zip: &[u8],
        test_set_id: &str,
    ) -> Result<String, ErrorMotor> {
        let cuerpo = Elemento::new("wcf:SendTestSetAsync")
            .hijo(Elemento::hoja("wcf:fileName", nombre_zip))
            .hijo(Elemento::hoja("wcf:contentFile", STANDARD.encode(zip)))
            .hijo(Elemento::hoja("wcf:testSetId", test_set_id));
        let xml = self
            .llamar(credencial, ahora, "SendTestSetAsync", cuerpo)
            .await?;
        clave_zip(&xml)
    }

    /// Estado de un documento por su CUFE/CUDE o identificador (`GetStatus`).
    ///
    /// # Errors
    ///
    /// Los de [`ServicioDian::enviar`].
    pub async fn estado(
        &self,
        credencial: &CredencialFirma,
        ahora: DateTime<Utc>,
        clave: &str,
    ) -> Result<RespuestaDian, ErrorMotor> {
        let cuerpo = Elemento::new("wcf:GetStatus").hijo(Elemento::hoja("wcf:trackId", clave));
        let xml = self.llamar(credencial, ahora, "GetStatus", cuerpo).await?;
        respuesta_unica(&xml)
    }

    /// Estado de los documentos de un lote o set de pruebas (`GetStatusZip`).
    ///
    /// # Errors
    ///
    /// Los de [`ServicioDian::enviar`].
    pub async fn estado_lote(
        &self,
        credencial: &CredencialFirma,
        ahora: DateTime<Utc>,
        clave: &str,
    ) -> Result<Vec<RespuestaDian>, ErrorMotor> {
        let cuerpo = Elemento::new("wcf:GetStatusZip").hijo(Elemento::hoja("wcf:trackId", clave));
        let xml = self
            .llamar(credencial, ahora, "GetStatusZip", cuerpo)
            .await?;
        respuestas(&xml)
    }

    /// Rangos de numeración autorizados con su clave técnica (`GetNumberingRange`).
    ///
    /// # Errors
    ///
    /// Los de [`ServicioDian::enviar`], o [`ErrorMotor::Respuesta`] si la DIAN no completa la
    /// consulta.
    pub async fn rangos_numeracion(
        &self,
        credencial: &CredencialFirma,
        ahora: DateTime<Utc>,
        nit: &str,
        software_id: &str,
    ) -> Result<Vec<RangoNumeracion>, ErrorMotor> {
        let cuerpo = Elemento::new("wcf:GetNumberingRange")
            .hijo(Elemento::hoja("wcf:accountCode", nit))
            .hijo(Elemento::hoja("wcf:accountCodeT", nit))
            .hijo(Elemento::hoja("wcf:softwareCode", software_id));
        let xml = self
            .llamar(credencial, ahora, "GetNumberingRange", cuerpo)
            .await?;
        rangos(&xml)
    }

    /// XML de un documento validado por su CUFE/CUDE (`GetXmlByDocumentKey`).
    ///
    /// # Errors
    ///
    /// Los de [`ServicioDian::enviar`].
    pub async fn xml_documento(
        &self,
        credencial: &CredencialFirma,
        ahora: DateTime<Utc>,
        clave: &str,
    ) -> Result<Vec<u8>, ErrorMotor> {
        let cuerpo =
            Elemento::new("wcf:GetXmlByDocumentKey").hijo(Elemento::hoja("wcf:trackId", clave));
        let xml = self
            .llamar(credencial, ahora, "GetXmlByDocumentKey", cuerpo)
            .await?;
        let doc = leer(&xml)?;
        let b64 = texto_de(&doc, "XmlBytesBase64")
            .or_else(|| texto_de(&doc, "XmlBase64Bytes"))
            .ok_or_else(|| ErrorMotor::Respuesta("la respuesta no trae el XML".into()))?;
        decodificar(&b64)
    }

    /// Envía un evento RADIAN (`ApplicationResponse` firmado, en ZIP) (`SendEventUpdateStatus`).
    ///
    /// # Errors
    ///
    /// Los de [`ServicioDian::enviar`].
    pub async fn enviar_evento(
        &self,
        credencial: &CredencialFirma,
        ahora: DateTime<Utc>,
        zip: &[u8],
    ) -> Result<RespuestaDian, ErrorMotor> {
        let cuerpo = Elemento::new("wcf:SendEventUpdateStatus")
            .hijo(Elemento::hoja("wcf:contentFile", STANDARD.encode(zip)));
        let xml = self
            .llamar(credencial, ahora, "SendEventUpdateStatus", cuerpo)
            .await?;
        respuesta_unica(&xml)
    }

    async fn llamar(
        &self,
        credencial: &CredencialFirma,
        ahora: DateTime<Utc>,
        operacion: &str,
        cuerpo: Elemento,
    ) -> Result<String, ErrorMotor> {
        let sobre = sobre(credencial, ahora, &self.url, operacion, cuerpo)?;
        let respuesta = self
            .http
            .post(&*self.url)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("application/soap+xml;charset=UTF-8;action=\"{ACCION}{operacion}\""),
            )
            .body(sobre)
            .send()
            .await
            .map_err(|e| ErrorMotor::Transporte(e.without_url().to_string()))?;
        let estado = respuesta.status();
        let texto = respuesta
            .text()
            .await
            .map_err(|e| ErrorMotor::Transporte(e.without_url().to_string()))?;
        // Un SOAP Fault llega con HTTP 500: se lee el cuerpo antes de mirar el estado.
        if let Ok(doc) = roxmltree::Document::parse(&texto)
            && let Some(falla) = doc.descendants().find(|n| n.tag_name().name() == "Fault")
        {
            let razon = falla
                .descendants()
                .find(|n| n.tag_name().name() == "Text")
                .and_then(|n| n.text())
                .unwrap_or("sin detalle");
            return Err(ErrorMotor::Respuesta(format!("SOAP Fault: {razon}")));
        }
        if estado.is_server_error() {
            return Err(ErrorMotor::Transporte(format!("HTTP {estado}")));
        }
        if !estado.is_success() {
            return Err(ErrorMotor::Respuesta(format!("HTTP {estado}")));
        }
        Ok(texto)
    }
}

/// Sobre SOAP firmado con WS-Security.
fn sobre(
    credencial: &CredencialFirma,
    ahora: DateTime<Utc>,
    url: &str,
    operacion: &str,
    cuerpo: Elemento,
) -> Result<String, ErrorMotor> {
    let espacios_sobre = vec![
        ("soap".to_owned(), SOAP.to_owned()),
        ("wcf".to_owned(), WCF.to_owned()),
    ];
    let mut espacios_cabecera = espacios_sobre.clone();
    espacios_cabecera.push(("wsa".to_owned(), WSA.to_owned()));
    let mut espacios_seguridad = espacios_cabecera.clone();
    espacios_seguridad.push(("wsse".to_owned(), WSSE.to_owned()));
    espacios_seguridad.push(("wsu".to_owned(), WSU.to_owned()));
    let mut espacios_firma = espacios_seguridad.clone();
    espacios_firma.push(("ds".to_owned(), DS.to_owned()));

    let instante = |t: DateTime<Utc>| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let id_token = "X509-DAD";
    let id_destino = "id-DAD-To";

    let destino = Elemento::hoja("wsa:To", url)
        .ns("wsu", WSU)
        .attr("wsu:Id", id_destino);
    let digest = sha256_b64(canonica(&destino, &espacios_cabecera, Canon::Exclusiva).as_bytes());
    let firmado = Elemento::new("ds:SignedInfo")
        .hijo(Elemento::new("ds:CanonicalizationMethod").attr("Algorithm", EXC_C14N))
        .hijo(Elemento::new("ds:SignatureMethod").attr(
            "Algorithm",
            "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256",
        ))
        .hijo(
            Elemento::new("ds:Reference")
                .attr("URI", format!("#{id_destino}"))
                .hijo(
                    Elemento::new("ds:Transforms")
                        .hijo(Elemento::new("ds:Transform").attr("Algorithm", EXC_C14N)),
                )
                .hijo(
                    Elemento::new("ds:DigestMethod")
                        .attr("Algorithm", "http://www.w3.org/2001/04/xmlenc#sha256"),
                )
                .hijo(Elemento::hoja("ds:DigestValue", digest)),
        );
    let valor = credencial
        .firmar_bytes(canonica(&firmado, &espacios_firma, Canon::Exclusiva).as_bytes())?;

    let seguridad = Elemento::new("wsse:Security")
        .ns("wsse", WSSE)
        .ns("wsu", WSU)
        .hijo(
            Elemento::new("wsu:Timestamp")
                .attr("wsu:Id", "TS-DAD")
                .hijo(Elemento::hoja("wsu:Created", instante(ahora)))
                .hijo(Elemento::hoja(
                    "wsu:Expires",
                    instante(ahora + VIGENCIA_TIMESTAMP),
                )),
        )
        .hijo(
            Elemento::hoja(
                "wsse:BinarySecurityToken",
                STANDARD.encode(credencial.certificado_der()),
            )
            .attr("EncodingType", BASE64_BINARY)
            .attr("ValueType", X509V3)
            .attr("wsu:Id", id_token),
        )
        .hijo(
            Elemento::new("ds:Signature")
                .ns("ds", DS)
                .attr("Id", "SIG-DAD")
                .hijo(firmado)
                .hijo(Elemento::hoja("ds:SignatureValue", STANDARD.encode(valor)))
                .hijo(
                    Elemento::new("ds:KeyInfo").hijo(
                        Elemento::new("wsse:SecurityTokenReference").hijo(
                            Elemento::new("wsse:Reference")
                                .attr("URI", format!("#{id_token}"))
                                .attr("ValueType", X509V3),
                        ),
                    ),
                ),
        );
    let sobre = Elemento::new("soap:Envelope")
        .ns("soap", SOAP)
        .ns("wcf", WCF)
        .hijo(
            Elemento::new("soap:Header")
                .ns("wsa", WSA)
                .hijo(seguridad)
                .hijo(Elemento::hoja("wsa:Action", format!("{ACCION}{operacion}")))
                .hijo(destino),
        )
        .hijo(Elemento::new("soap:Body").hijo(cuerpo));
    Ok(canonica(&sobre, &[], Canon::Inclusiva))
}

fn leer(xml: &str) -> Result<roxmltree::Document<'_>, ErrorMotor> {
    roxmltree::Document::parse(xml)
        .map_err(|_| ErrorMotor::Respuesta("la respuesta no es XML válido".into()))
}

/// Texto del primer descendiente con ese nombre local; `None` si falta, está vacío o es nulo.
fn texto_de(nodo: &roxmltree::Document<'_>, nombre: &str) -> Option<String> {
    texto_en(nodo.root(), nombre)
}

fn texto_en(nodo: roxmltree::Node<'_, '_>, nombre: &str) -> Option<String> {
    nodo.descendants()
        .find(|n| n.tag_name().name() == nombre)
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

fn decodificar(b64: &str) -> Result<Vec<u8>, ErrorMotor> {
    let limpio: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    STANDARD
        .decode(limpio)
        .map_err(|_| ErrorMotor::Respuesta("Base64 inválido en la respuesta".into()))
}

/// Respuesta de validación desde el nodo que contiene `IsValid`.
fn respuesta_en(nodo: roxmltree::Node<'_, '_>) -> Result<RespuestaDian, ErrorMotor> {
    let valido = texto_en(nodo, "IsValid").is_some_and(|t| t.eq_ignore_ascii_case("true"));
    let errores = nodo
        .descendants()
        .find(|n| n.tag_name().name() == "ErrorMessage")
        .map(|lista| {
            lista
                .children()
                .filter(|n| n.is_element())
                .filter_map(|n| n.text())
                .map(|t| t.trim().to_owned())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let application_response = texto_en(nodo, "XmlBase64Bytes")
        .map(|b| decodificar(&b))
        .transpose()?;
    Ok(RespuestaDian {
        valido,
        codigo: texto_en(nodo, "StatusCode").unwrap_or_default(),
        descripcion: texto_en(nodo, "StatusDescription"),
        mensaje: texto_en(nodo, "StatusMessage"),
        errores,
        application_response,
        clave: texto_en(nodo, "XmlDocumentKey"),
    })
}

fn respuesta_unica(xml: &str) -> Result<RespuestaDian, ErrorMotor> {
    let doc = leer(xml)?;
    let nodo = doc
        .descendants()
        .find(|n| n.tag_name().name() == "IsValid")
        .and_then(|n| n.parent())
        .ok_or_else(|| ErrorMotor::Respuesta("la respuesta no trae IsValid".into()))?;
    respuesta_en(nodo)
}

fn respuestas(xml: &str) -> Result<Vec<RespuestaDian>, ErrorMotor> {
    let doc = leer(xml)?;
    doc.descendants()
        .filter(|n| n.tag_name().name() == "IsValid")
        .filter_map(|n| n.parent())
        .map(respuesta_en)
        .collect()
}

/// `ZipKey` de un envío asíncrono; si la DIAN no lo da, sus mensajes de error.
fn clave_zip(xml: &str) -> Result<String, ErrorMotor> {
    let doc = leer(xml)?;
    if let Some(clave) = texto_de(&doc, "ZipKey") {
        return Ok(clave);
    }
    let mensajes: Vec<String> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "ProcessedMessage")
        .filter_map(|n| n.text())
        .map(str::to_owned)
        .collect();
    Err(ErrorMotor::Respuesta(if mensajes.is_empty() {
        "la DIAN no devolvió el identificador del envío".into()
    } else {
        mensajes.join("; ")
    }))
}

fn rangos(xml: &str) -> Result<Vec<RangoNumeracion>, ErrorMotor> {
    let doc = leer(xml)?;
    let codigo = texto_de(&doc, "OperationCode").unwrap_or_default();
    if codigo != "100" {
        let detalle = texto_de(&doc, "OperationDescription").unwrap_or_default();
        return Err(ErrorMotor::Respuesta(format!(
            "consulta de rangos sin éxito ({codigo}): {detalle}"
        )));
    }
    let fecha = |n: roxmltree::Node<'_, '_>, campo: &str| {
        texto_en(n, campo).and_then(|t| NaiveDate::parse_from_str(&t, "%Y-%m-%d").ok())
    };
    doc.descendants()
        .filter(|n| n.tag_name().name() == "NumberRangeResponse")
        .map(|n| {
            let numero = |campo: &str| {
                texto_en(n, campo)
                    .and_then(|t| t.parse::<u64>().ok())
                    .ok_or_else(|| ErrorMotor::Respuesta(format!("rango sin {campo}")))
            };
            Ok(RangoNumeracion {
                resolucion: texto_en(n, "ResolutionNumber").unwrap_or_default(),
                fecha_resolucion: fecha(n, "ResolutionDate"),
                prefijo: texto_en(n, "Prefix").unwrap_or_default(),
                desde: numero("FromNumber")?,
                hasta: numero("ToNumber")?,
                vigente_desde: fecha(n, "ValidDateFrom")
                    .ok_or_else(|| ErrorMotor::Respuesta("rango sin vigencia".into()))?,
                vigente_hasta: fecha(n, "ValidDateTo")
                    .ok_or_else(|| ErrorMotor::Respuesta("rango sin vigencia".into()))?,
                clave_tecnica: texto_en(n, "TechnicalKey").unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::propio::firma::tests::credencial;

    pub(crate) fn ahora() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2027-01-15T15:30:00Z")
            .expect("fecha")
            .with_timezone(&Utc)
    }

    #[test]
    fn el_sobre_lleva_token_timestamp_accion_y_firma_de_to() {
        let xml = sobre(
            &credencial(),
            ahora(),
            "https://vpfe-hab.dian.gov.co/WcfDianCustomerServices.svc",
            "GetStatus",
            Elemento::new("wcf:GetStatus").hijo(Elemento::hoja("wcf:trackId", "abc")),
        )
        .expect("sobre");
        assert!(
            xml.starts_with(
                "<soap:Envelope xmlns:soap=\"http://www.w3.org/2003/05/soap-envelope\""
            )
        );
        assert!(xml.contains("<wsu:Created>2027-01-15T15:30:00.000Z</wsu:Created>"));
        assert!(xml.contains("<wsu:Expires>2027-01-15T15:35:00.000Z</wsu:Expires>"));
        assert!(xml.contains(
            "<wsa:Action>http://wcf.dian.colombia/IWcfDianCustomerServices/GetStatus</wsa:Action>"
        ));
        assert!(xml.contains("wsu:Id=\"X509-DAD\""));
        assert!(xml.contains("<ds:Reference URI=\"#id-DAD-To\">"));
        assert!(xml.contains("<wcf:trackId>abc</wcf:trackId>"));
    }

    /// Escribe un sobre firmado en `DAD_FE_SALIDA` para verificar la firma WS-Security con
    /// herramientas externas (ver `exportar_documentos_firmados_para_verificacion_externa`).
    #[test]
    #[ignore = "verificación manual con herramientas externas"]
    fn exportar_sobre_firmado_para_verificacion_externa() {
        let salida = std::env::var("DAD_FE_SALIDA").expect("DAD_FE_SALIDA");
        let xml = sobre(
            &credencial(),
            ahora(),
            "https://vpfe-hab.dian.gov.co/WcfDianCustomerServices.svc",
            "GetStatus",
            Elemento::new("wcf:GetStatus").hijo(Elemento::hoja("wcf:trackId", "abc")),
        )
        .expect("sobre");
        std::fs::write(format!("{salida}/sobre.xml"), xml).expect("escribir");
    }

    #[test]
    fn lee_una_respuesta_con_errores_de_validacion() {
        let xml = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body>
            <SendBillSyncResponse xmlns="http://wcf.dian.colombia">
            <SendBillSyncResult xmlns:b="http://schemas.datacontract.org/2004/07/DianResponse" xmlns:i="http://www.w3.org/2001/XMLSchema-instance">
            <b:ErrorMessage xmlns:c="http://schemas.microsoft.com/2003/10/Serialization/Arrays">
              <c:string>Regla: AA09 Valor del CUFE no está calculado correctamente.</c:string>
              <c:string>Regla: AC38b Documento fue enviado para el ambiente errado</c:string>
            </b:ErrorMessage>
            <b:IsValid>false</b:IsValid><b:StatusCode>99</b:StatusCode>
            <b:StatusDescription>Validación contiene errores en campos mandatorios.</b:StatusDescription>
            <b:StatusMessage i:nil="true"/>
            <b:XmlBase64Bytes>PEFwcGxpY2F0aW9uUmVzcG9uc2UvPg==</b:XmlBase64Bytes>
            <b:XmlDocumentKey>abc123</b:XmlDocumentKey>
            </SendBillSyncResult></SendBillSyncResponse></s:Body></s:Envelope>"#;
        let r = respuesta_unica(xml).expect("respuesta");
        assert!(!r.valido);
        assert_eq!(r.codigo, "99");
        assert_eq!(r.errores.len(), 2);
        assert!(r.errores[0].contains("AA09"));
        assert_eq!(
            r.application_response.as_deref(),
            Some(&b"<ApplicationResponse/>"[..])
        );
        assert_eq!(r.clave.as_deref(), Some("abc123"));
        assert_eq!(r.mensaje, None);
    }

    #[test]
    fn lee_los_rangos_de_numeracion_con_su_clave_tecnica() {
        let xml = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body>
            <GetNumberingRangeResponse xmlns="http://wcf.dian.colombia">
            <GetNumberingRangeResult xmlns:b="x" xmlns:c="y">
              <b:OperationCode>100</b:OperationCode><b:OperationDescription>Acción completada OK.</b:OperationDescription>
              <b:ResponseList><c:NumberRangeResponse>
                <c:ResolutionNumber>18760000001</c:ResolutionNumber><c:ResolutionDate>2026-07-25</c:ResolutionDate>
                <c:Prefix>SETP</c:Prefix><c:FromNumber>990000000</c:FromNumber><c:ToNumber>995000000</c:ToNumber>
                <c:ValidDateFrom>2026-07-25</c:ValidDateFrom><c:ValidDateTo>2030-07-28</c:ValidDateTo>
                <c:TechnicalKey>fc8eac422eba16e22ffd8c6f94b3f40a6e38162c</c:TechnicalKey>
              </c:NumberRangeResponse></b:ResponseList>
            </GetNumberingRangeResult></GetNumberingRangeResponse></s:Body></s:Envelope>"#;
        let r = rangos(xml).expect("rangos");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].prefijo, "SETP");
        assert_eq!(r[0].hasta, 995_000_000);
        assert_eq!(
            r[0].clave_tecnica,
            "fc8eac422eba16e22ffd8c6f94b3f40a6e38162c"
        );
        assert!(!format!("{:?}", r[0]).contains("fc8eac"));
    }

    #[test]
    fn rangos_sin_exito_es_error() {
        let xml = r#"<s:Envelope xmlns:s="x"><s:Body><R><OperationCode>301</OperationCode>
            <OperationDescription>NIT no autorizado</OperationDescription></R></s:Body></s:Envelope>"#;
        assert!(
            matches!(rangos(xml), Err(ErrorMotor::Respuesta(m)) if m.contains("NIT no autorizado"))
        );
    }

    #[test]
    fn clave_zip_o_sus_mensajes() {
        assert_eq!(
            clave_zip("<E><ZipKey>358f9538</ZipKey></E>").expect("clave"),
            "358f9538"
        );
        let error = clave_zip(
            "<E><ZipKey/><ProcessedMessage>Set de pruebas inválido</ProcessedMessage></E>",
        )
        .expect_err("sin clave");
        assert!(matches!(error, ErrorMotor::Respuesta(m) if m.contains("Set de pruebas")));
    }

    #[test]
    fn varias_respuestas_de_un_lote() {
        let xml = "<E><D><IsValid>true</IsValid><StatusCode>00</StatusCode></D>\
                   <D><IsValid>false</IsValid><StatusCode>99</StatusCode></D></E>";
        let r = respuestas(xml).expect("lote");
        assert_eq!(r.len(), 2);
        assert!(r[0].valido && !r[1].valido);
    }
}
