//! Firma XAdES-EPES de los documentos electrónicos (anexo técnico 1.9, §6.5.10 y Suplemento A).
//!
//! XMLDSig *enveloped* dentro de la última `ext:UBLExtension`, canonicalización C14N 1.0 sin
//! comentarios, RSA-SHA256 y digests SHA-256. Se firman tres referencias: el documento (con la
//! transformación *enveloped-signature*), el `KeyInfo` y las `SignedProperties` (hora de firma,
//! certificados de la cadena, política de firma v2 de la DIAN y rol del firmante).
//!
//! Lo firmado es la forma canónica del árbol `Elemento`, que es también lo que se envía: el
//! documento se serializa siempre canónico (módulo `xml`).

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, FixedOffset, Utc};
use ring::{
    rand::SystemRandom,
    signature::{RSA_PKCS1_SHA256, RsaKeyPair},
};
use sha2::{Digest, Sha256};

use super::{
    ErrorMotor,
    x509::Certificado,
    xml::{Canon, Elemento, canonica},
};

const DS: &str = "http://www.w3.org/2000/09/xmldsig#";
const C14N: &str = "http://www.w3.org/TR/2001/REC-xml-c14n-20010315";
const RSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256";
const SHA256: &str = "http://www.w3.org/2001/04/xmlenc#sha256";
const ENVELOPED: &str = "http://www.w3.org/2000/09/xmldsig#enveloped-signature";
const TIPO_SIGNED_PROPERTIES: &str = "http://uri.etsi.org/01903#SignedProperties";
/// Política de firma vigente (anexo §10.10) y el SHA-256 en Base64 de ese PDF, verificado el
/// 2026-11-13 contra el archivo publicado por la DIAN.
const POLITICA: &str =
    "https://facturaelectronica.dian.gov.co/politicadefirma/v2/politicadefirmav2.pdf";
const HASH_POLITICA: &str = "dMoMvtcG5aIzgYo0tIsSQeVJBDnUnfSOfBpxXrmor0Y=";
const DESCRIPCION_POLITICA: &str =
    "Política de firma para facturas electrónicas de la República de Colombia.";

/// Certificado digital del facturador: su clave privada RSA y la cadena (el del firmante
/// primero). Lo expide una entidad de certificación acreditada por la ONAC.
///
/// La clave nunca sale de este tipo ni aparece en su `Debug`.
pub struct CredencialFirma {
    clave: RsaKeyPair,
    cadena: Vec<Certificado>,
}

impl std::fmt::Debug for CredencialFirma {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredencialFirma")
            .field("emisor", &self.cadena.first().map(|c| c.emisor.as_str()))
            .field("clave", &"<oculta>")
            .finish()
    }
}

impl CredencialFirma {
    /// Credencial desde la clave privada PKCS#8 (DER) y la cadena de certificados (DER, el del
    /// firmante primero).
    ///
    /// # Errors
    ///
    /// [`ErrorMotor::Certificado`] si la clave no es RSA PKCS#8, si no hay certificados, si
    /// alguno no se puede leer o si el del firmante no permite firmar con no repudio.
    pub fn desde_der(clave_pkcs8: &[u8], cadena: &[Vec<u8>]) -> Result<Self, ErrorMotor> {
        let clave = RsaKeyPair::from_pkcs8(clave_pkcs8)
            .map_err(|_| ErrorMotor::Certificado("la clave privada no es RSA PKCS#8".into()))?;
        let cadena = cadena
            .iter()
            .map(|der| Certificado::leer(der))
            .collect::<Result<Vec<_>, _>>()?;
        let firmante = cadena
            .first()
            .ok_or_else(|| ErrorMotor::Certificado("falta el certificado del firmante".into()))?;
        if !firmante.permite_firmar() {
            return Err(ErrorMotor::Certificado(
                "el certificado no permite firma digital con no repudio".into(),
            ));
        }
        Ok(Self { clave, cadena })
    }

    /// Credencial desde PEM: la clave privada (`PRIVATE KEY`, PKCS#8) y los certificados
    /// (`CERTIFICATE`, el del firmante primero, en uno o varios bloques).
    ///
    /// # Errors
    ///
    /// Los de [`CredencialFirma::desde_der`], o [`ErrorMotor::Certificado`] si el PEM no se
    /// puede leer.
    pub fn desde_pem(clave_pem: &str, certificados_pem: &str) -> Result<Self, ErrorMotor> {
        let clave = bloques_pem(clave_pem, "PRIVATE KEY")?
            .into_iter()
            .next()
            .ok_or_else(|| ErrorMotor::Certificado("no hay una clave PKCS#8 en el PEM".into()))?;
        let cadena = bloques_pem(certificados_pem, "CERTIFICATE")?;
        Self::desde_der(&clave, &cadena)
    }

    /// Certificado del firmante en DER (para `X509Data` y el `BinarySecurityToken`).
    pub(crate) fn certificado_der(&self) -> &[u8] {
        self.cadena.first().map_or(&[], |c| c.der.as_slice())
    }

    /// Si el certificado del firmante está vigente en ese momento.
    pub fn vigente_en(&self, momento: DateTime<Utc>) -> bool {
        self.cadena.first().is_some_and(|c| c.vigente_en(momento))
    }

    /// Fin de la vigencia del certificado del firmante (para avisar antes de que venza).
    pub fn vence(&self) -> Option<DateTime<Utc>> {
        self.cadena.first().map(|c| c.vigente_hasta)
    }

    /// Firma RSA-SHA256 (PKCS#1 v1.5) de `datos`.
    pub(crate) fn firmar_bytes(&self, datos: &[u8]) -> Result<Vec<u8>, ErrorMotor> {
        let mut firma = vec![0u8; self.clave.public().modulus_len()];
        self.clave
            .sign(&RSA_PKCS1_SHA256, &SystemRandom::new(), datos, &mut firma)
            .map_err(|_| ErrorMotor::Firma("no se pudo calcular la firma RSA".into()))?;
        Ok(firma)
    }
}

/// Bloques PEM con esa etiqueta, en DER.
fn bloques_pem(pem: &str, etiqueta: &str) -> Result<Vec<Vec<u8>>, ErrorMotor> {
    let inicio = format!("-----BEGIN {etiqueta}-----");
    let fin = format!("-----END {etiqueta}-----");
    let mut bloques = Vec::new();
    let mut actual: Option<String> = None;
    for linea in pem.lines().map(str::trim) {
        if linea == inicio {
            actual = Some(String::new());
        } else if linea == fin {
            let cuerpo = actual.take().unwrap_or_default();
            bloques.push(
                STANDARD
                    .decode(cuerpo)
                    .map_err(|_| ErrorMotor::Certificado("PEM con Base64 inválido".into()))?,
            );
        } else if let Some(cuerpo) = actual.as_mut() {
            cuerpo.push_str(linea);
        }
    }
    Ok(bloques)
}

pub(crate) fn sha256_b64(datos: &[u8]) -> String {
    STANDARD.encode(Sha256::digest(datos))
}

fn digest_method() -> Elemento {
    Elemento::new("ds:DigestMethod").attr("Algorithm", SHA256)
}

/// Firma el documento: inserta `ds:Signature` en la última `ext:ExtensionContent`, que debe
/// existir y estar vacía. `id` identifica la firma (`xmldsig-<id>`); `momento` es la hora de la
/// firma, que debe caer dentro de la vigencia del certificado.
///
/// Todos los espacios de nombres (`ds`, `xades`…) deben estar declarados en la raíz, como en los
/// ejemplos de la DIAN: así los subconjuntos firmados heredan exactamente los de la raíz.
///
/// # Errors
///
/// [`ErrorMotor::Certificado`] si el certificado no está vigente en `momento`;
/// [`ErrorMotor::Firma`] si falta el lugar de la firma o falla el cálculo.
pub(crate) fn firmar_documento(
    raiz: &mut Elemento,
    credencial: &CredencialFirma,
    momento: DateTime<FixedOffset>,
    id: &str,
) -> Result<(), ErrorMotor> {
    if !credencial.vigente_en(momento.with_timezone(&Utc)) {
        return Err(ErrorMotor::Certificado(
            "el certificado no está vigente en la fecha de la firma".into(),
        ));
    }
    let heredados = raiz.espacios.clone();
    if !heredados.iter().any(|(p, u)| p == "ds" && u == DS) {
        return Err(ErrorMotor::Firma(
            "el documento no declara el espacio de nombres ds".into(),
        ));
    }

    // Referencia 1: el documento sin la firma (lo que deja la transformación enveloped).
    let digest_documento = sha256_b64(canonica(raiz, &[], Canon::Inclusiva).as_bytes());

    let id_firma = format!("xmldsig-{id}");
    let id_key_info = format!("{id_firma}-keyinfo");
    let id_props = format!("{id_firma}-signedprops");

    // Anexo §10.5.2: el certificado y toda su cadena van en `ds:X509Data` (el del firmante
    // primero) y en las propiedades firmadas.
    let key_info = Elemento::new("ds:KeyInfo").attr("Id", &id_key_info).hijo(
        Elemento::new("ds:X509Data").hijos(
            credencial
                .cadena
                .iter()
                .map(|c| Elemento::hoja("ds:X509Certificate", STANDARD.encode(&c.der))),
        ),
    );
    let props = propiedades_firmadas(credencial, momento, &id_props);
    let digest_key_info = sha256_b64(canonica(&key_info, &heredados, Canon::Inclusiva).as_bytes());
    let digest_props = sha256_b64(canonica(&props, &heredados, Canon::Inclusiva).as_bytes());

    let referencia = |uri: String, digest: String| {
        Elemento::new("ds:Reference")
            .attr("URI", uri)
            .hijo(digest_method())
            .hijo(Elemento::hoja("ds:DigestValue", digest))
    };
    let signed_info = Elemento::new("ds:SignedInfo")
        .hijo(Elemento::new("ds:CanonicalizationMethod").attr("Algorithm", C14N))
        .hijo(Elemento::new("ds:SignatureMethod").attr("Algorithm", RSA_SHA256))
        .hijo(
            Elemento::new("ds:Reference")
                .attr("Id", format!("{id_firma}-ref0"))
                .attr("URI", "")
                .hijo(
                    Elemento::new("ds:Transforms")
                        .hijo(Elemento::new("ds:Transform").attr("Algorithm", ENVELOPED)),
                )
                .hijo(digest_method())
                .hijo(Elemento::hoja("ds:DigestValue", digest_documento)),
        )
        .hijo(referencia(format!("#{id_key_info}"), digest_key_info))
        .hijo(
            referencia(format!("#{id_props}"), digest_props).attr("Type", TIPO_SIGNED_PROPERTIES),
        );
    let valor =
        credencial.firmar_bytes(canonica(&signed_info, &heredados, Canon::Inclusiva).as_bytes())?;

    let firma = Elemento::new("ds:Signature")
        .attr("Id", &id_firma)
        .hijo(signed_info)
        .hijo(
            Elemento::hoja("ds:SignatureValue", STANDARD.encode(valor))
                .attr("Id", format!("{id_firma}-sigvalue")),
        )
        .hijo(key_info)
        .hijo(
            Elemento::new("ds:Object").hijo(
                Elemento::new("xades:QualifyingProperties")
                    .attr("Target", format!("#{id_firma}"))
                    .hijo(props),
            ),
        );

    let contenedor = raiz
        .buscar_mut("ext:UBLExtensions")
        .and_then(|e| e.ultimo_hijo_mut("ext:UBLExtension"))
        .and_then(|e| e.ultimo_hijo_mut("ext:ExtensionContent"))
        .filter(|e| e.hijos.is_empty())
        .ok_or_else(|| {
            ErrorMotor::Firma("falta la ext:ExtensionContent vacía para la firma".into())
        })?;
    contenedor.hijos.push(super::xml::Nodo::Elemento(firma));
    Ok(())
}

fn propiedades_firmadas(
    credencial: &CredencialFirma,
    momento: DateTime<FixedOffset>,
    id: &str,
) -> Elemento {
    let certificados = credencial.cadena.iter().map(|c| {
        Elemento::new("xades:Cert")
            .hijo(
                Elemento::new("xades:CertDigest")
                    .hijo(digest_method())
                    .hijo(Elemento::hoja("ds:DigestValue", sha256_b64(&c.der))),
            )
            .hijo(
                Elemento::new("xades:IssuerSerial")
                    .hijo(Elemento::hoja("ds:X509IssuerName", c.emisor.clone()))
                    .hijo(Elemento::hoja("ds:X509SerialNumber", c.serial.clone())),
            )
    });
    Elemento::new("xades:SignedProperties").attr("Id", id).hijo(
        Elemento::new("xades:SignedSignatureProperties")
            .hijo(Elemento::hoja(
                "xades:SigningTime",
                momento.format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string(),
            ))
            .hijo(Elemento::new("xades:SigningCertificate").hijos(certificados))
            .hijo(
                Elemento::new("xades:SignaturePolicyIdentifier").hijo(
                    Elemento::new("xades:SignaturePolicyId")
                        .hijo(
                            Elemento::new("xades:SigPolicyId")
                                .hijo(Elemento::hoja("xades:Identifier", POLITICA))
                                .hijo(Elemento::hoja("xades:Description", DESCRIPCION_POLITICA)),
                        )
                        .hijo(
                            Elemento::new("xades:SigPolicyHash")
                                .hijo(digest_method())
                                .hijo(Elemento::hoja("ds:DigestValue", HASH_POLITICA)),
                        ),
                ),
            )
            .hijo(
                Elemento::new("xades:SignerRole").hijo(
                    Elemento::new("xades:ClaimedRoles")
                        .hijo(Elemento::hoja("xades:ClaimedRole", "supplier")),
                ),
            ),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::propio::xml::{Nodo, documento};

    pub(crate) const CLAVE: &str = include_str!("../../tests/fixtures/firma/firmante.key");
    pub(crate) const FIRMANTE: &str = include_str!("../../tests/fixtures/firma/firmante.crt");
    pub(crate) const CA: &str = include_str!("../../tests/fixtures/firma/ca.crt");
    const SIN_NO_REPUDIO: &str = include_str!("../../tests/fixtures/firma/sin_no_repudio.crt");

    pub(crate) fn credencial() -> CredencialFirma {
        CredencialFirma::desde_pem(CLAVE, &format!("{FIRMANTE}{CA}")).expect("credencial")
    }

    pub(crate) fn momento() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2027-01-15T10:30:00-05:00").expect("fecha")
    }

    fn documento_minimo() -> Elemento {
        Elemento::new("Invoice")
            .ns("", "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2")
            .ns(
                "cbc",
                "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2",
            )
            .ns("ds", DS)
            .ns(
                "ext",
                "urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2",
            )
            .ns("xades", "http://uri.etsi.org/01903/v1.3.2#")
            .hijo(Elemento::new("ext:UBLExtensions").hijo(
                Elemento::new("ext:UBLExtension").hijo(Elemento::new("ext:ExtensionContent")),
            ))
            .hijo(Elemento::hoja("cbc:ID", "SETP1"))
    }

    #[test]
    fn lee_la_cadena_con_emisor_y_serial_del_certificado() {
        let c = credencial();
        let firmante = &c.cadena[0];
        assert_eq!(firmante.serial, "7785324499979575522");
        assert_eq!(
            firmante.emisor,
            "CN=CA DE PRUEBAS (SIN VALIDEZ),OU=Autoridad de pruebas,O=Pruebas DAD,L=Bogota D.C.,C=CO"
        );
        assert_eq!(c.cadena.len(), 2);
        assert!(format!("{c:?}").contains("<oculta>"));
    }

    #[test]
    fn rechaza_un_certificado_sin_no_repudio() {
        let error = CredencialFirma::desde_pem(CLAVE, SIN_NO_REPUDIO).expect_err("sin no repudio");
        assert!(matches!(error, ErrorMotor::Certificado(_)));
    }

    #[test]
    fn rechaza_firmar_fuera_de_la_vigencia_del_certificado() {
        let mut doc = documento_minimo();
        let despues = DateTime::parse_from_rfc3339("2040-01-01T00:00:00-05:00").expect("fecha");
        let error = firmar_documento(&mut doc, &credencial(), despues, "x").expect_err("vencido");
        assert!(matches!(error, ErrorMotor::Certificado(_)));
    }

    #[test]
    fn firma_queda_en_la_ultima_extension_con_tres_referencias() {
        let mut doc = documento_minimo();
        firmar_documento(&mut doc, &credencial(), momento(), "abc").expect("firma");
        let firma = doc
            .buscar_mut("ds:Signature")
            .expect("firma insertada")
            .clone();
        assert_eq!(firma.atributos[0], ("Id".into(), "xmldsig-abc".into()));
        let xml = String::from_utf8(documento(&doc)).expect("utf-8");
        assert_eq!(xml.matches("<ds:Reference ").count(), 3);
        assert!(
            xml.contains("<xades:SigningTime>2027-01-15T10:30:00.000-05:00</xades:SigningTime>")
        );
        assert!(xml.contains(HASH_POLITICA));
        assert!(xml.contains("<xades:ClaimedRole>supplier</xades:ClaimedRole>"));
    }

    #[test]
    fn la_firma_es_determinista_y_depende_del_contenido() {
        let firmar = |id: &str| {
            let mut doc = documento_minimo();
            if let Some(Nodo::Elemento(e)) = doc.hijos.get_mut(1) {
                e.hijos = vec![Nodo::Texto(id.to_owned())];
            }
            firmar_documento(&mut doc, &credencial(), momento(), "abc").expect("firma");
            documento(&doc)
        };
        assert_eq!(firmar("SETP1"), firmar("SETP1"));
        assert_ne!(firmar("SETP1"), firmar("SETP2"));
    }

    #[test]
    fn sin_lugar_para_la_firma_es_error() {
        let mut doc = documento_minimo();
        firmar_documento(&mut doc, &credencial(), momento(), "a").expect("primera firma");
        let error = firmar_documento(&mut doc, &credencial(), momento(), "b").expect_err("ocupado");
        assert!(matches!(error, ErrorMotor::Firma(_)));
    }
}
