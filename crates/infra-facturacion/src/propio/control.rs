//! Códigos de control de la DIAN (anexo técnico 1.9, Suplemento B, §11): CUFE, CUDE, código de
//! seguridad del software y contenido del QR.
//!
//! Son SHA-384 de cadenas concatenadas. Los montos van con **dos decimales truncados** (no
//! redondeados), con punto y sin separadores de miles; un error de formato hace que la DIAN
//! rechace el documento (regla AA09/FAD06), así que todo sale de [`Decimal`], nunca de `f64`.

use chrono::{DateTime, FixedOffset, NaiveDate};
use rust_decimal::{Decimal, RoundingStrategy};
use sha2::{Digest, Sha384};

/// Ambiente de destino del documento (`cbc:ProfileExecutionID`, tabla 13.2 del anexo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ambiente {
    /// Producción (`1`): el documento tiene efectos fiscales.
    Produccion,
    /// Pruebas o habilitación (`2`).
    Pruebas,
}

impl Ambiente {
    /// Código del ambiente en el XML y en los códigos de control.
    pub const fn codigo(self) -> &'static str {
        match self {
            Ambiente::Produccion => "1",
            Ambiente::Pruebas => "2",
        }
    }

    /// Servidor del catálogo de la DIAN donde se consulta el documento por el QR.
    const fn catalogo(self) -> &'static str {
        match self {
            Ambiente::Produccion => "https://catalogo-vpfe.dian.gov.co",
            Ambiente::Pruebas => "https://catalogo-vpfe-hab.dian.gov.co",
        }
    }
}

/// Monto en el formato de los códigos de control: dos decimales **truncados**, punto decimal y
/// sin separadores (`1500000.009` → `1500000.00`).
pub fn monto_control(valor: Decimal) -> String {
    let truncado = valor.round_dp_with_strategy(2, RoundingStrategy::ToZero);
    format!("{truncado:.2}")
}

/// Hora de emisión con su zona (`10:53:10-05:00`), como `cbc:IssueTime`.
pub fn hora_dian(momento: DateTime<FixedOffset>) -> String {
    momento.format("%H:%M:%S%:z").to_string()
}

/// Datos de un documento que entran en su CUFE o CUDE (anexo §11.2 y §11.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosControl<'a> {
    /// Prefijo y número concatenados (`SETP990000002`).
    pub numero: &'a str,
    pub fecha: NaiveDate,
    /// Hora con zona, ver [`hora_dian`].
    pub hora: &'a str,
    /// Valor sin impuestos (`LineExtensionAmount`).
    pub valor_bruto: Decimal,
    /// Impuesto 01 (IVA).
    pub iva: Decimal,
    /// Impuesto 04 (impuesto nacional al consumo).
    pub inc: Decimal,
    /// Impuesto 03 (ICA).
    pub ica: Decimal,
    /// Valor a pagar (`PayableAmount`).
    pub total: Decimal,
    /// NIT del facturador, sin DV.
    pub nit_emisor: &'a str,
    /// Documento del adquirente, sin DV.
    pub documento_adquirente: &'a str,
    pub ambiente: Ambiente,
}

impl DatosControl<'_> {
    /// Cadena común a CUFE y CUDE; `secreto` es la clave técnica o el PIN del software.
    fn cadena(&self, secreto: &str) -> String {
        format!(
            "{}{}{}{}01{}04{}03{}{}{}{}{}{}",
            self.numero,
            self.fecha.format("%Y-%m-%d"),
            self.hora,
            monto_control(self.valor_bruto),
            monto_control(self.iva),
            monto_control(self.inc),
            monto_control(self.ica),
            monto_control(self.total),
            self.nit_emisor,
            self.documento_adquirente,
            secreto,
            self.ambiente.codigo()
        )
    }
}

fn sha384_hex(texto: &str) -> String {
    hex::encode(Sha384::digest(texto.as_bytes()))
}

/// CUFE de una factura electrónica de venta: usa la **clave técnica** del rango de numeración
/// (anexo §11.2).
pub fn cufe(datos: &DatosControl<'_>, clave_tecnica: &str) -> String {
    sha384_hex(&datos.cadena(clave_tecnica))
}

/// CUDE de una nota crédito, nota débito o factura de contingencia del facturador: usa el **PIN
/// del software** en lugar de la clave técnica (anexo §11.4).
pub fn cude(datos: &DatosControl<'_>, pin_software: &str) -> String {
    sha384_hex(&datos.cadena(pin_software))
}

/// Huella del software que produjo el documento: SHA-384(SoftwareID + PIN + número del
/// documento) (anexo §11.8, `sts:SoftwareSecurityCode`).
pub fn codigo_seguridad_software(software_id: &str, pin: &str, numero_documento: &str) -> String {
    sha384_hex(&format!("{software_id}{pin}{numero_documento}"))
}

/// URL de consulta del documento en el catálogo de la DIAN (anexo §11.7.1).
pub fn url_qr(codigo: &str, ambiente: Ambiente) -> String {
    format!(
        "{}/document/searchqr?documentkey={codigo}",
        ambiente.catalogo()
    )
}

/// Datos que van en el código QR de la representación gráfica y en `sts:QRCode` (anexo §11.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosQr<'a> {
    pub numero: &'a str,
    pub fecha: NaiveDate,
    pub hora: &'a str,
    pub nit_emisor: &'a str,
    pub documento_adquirente: &'a str,
    pub valor_bruto: Decimal,
    pub iva: Decimal,
    /// Suma de los impuestos distintos del IVA.
    pub otros_impuestos: Decimal,
    pub total: Decimal,
    /// CUFE o CUDE.
    pub codigo: &'a str,
    pub ambiente: Ambiente,
}

/// Texto del QR, una línea por dato, en el orden del anexo §11.7.
pub fn texto_qr(d: &DatosQr<'_>) -> String {
    format!(
        "NumFac: {}\nFecFac: {}\nHorFac: {}\nNitFac: {}\nDocAdq: {}\nValFac: {}\nValIva: {}\n\
         ValOtroIm: {}\nValTolFac: {}\nCUFE: {}\nQRCode: {}",
        d.numero,
        d.fecha.format("%Y-%m-%d"),
        d.hora,
        d.nit_emisor,
        d.documento_adquirente,
        monto_control(d.valor_bruto),
        monto_control(d.iva),
        monto_control(d.otros_impuestos),
        monto_control(d.total),
        d.codigo,
        url_qr(d.codigo, d.ambiente)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn dec(texto: &str) -> Decimal {
        Decimal::from_str(texto).expect("decimal válido")
    }

    fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, d).expect("fecha válida")
    }

    #[test]
    fn cufe_coincide_con_el_ejemplo_oficial_del_anexo() {
        // Anexo técnico 1.9, §11.2.1.
        let datos = DatosControl {
            numero: "323200000129",
            fecha: fecha(2019, 1, 16),
            hora: "10:53:10-05:00",
            valor_bruto: dec("1500000.00"),
            iva: dec("285000.00"),
            inc: Decimal::ZERO,
            ica: Decimal::ZERO,
            total: dec("1785000.00"),
            nit_emisor: "700085371",
            documento_adquirente: "800199436",
            ambiente: Ambiente::Produccion,
        };
        assert_eq!(
            cufe(&datos, "693ff6f2a553c3646a063436fd4dd9ded0311471"),
            "8bb918b19ba22a694f1da11c643b5e9de39adf60311cf179179e9b33381030bcd4c3c3f156c506ed5908f9276f5bd9b4"
        );
    }

    #[test]
    fn cude_de_nota_credito_coincide_con_el_ejemplo_oficial_del_anexo() {
        // Anexo técnico 1.9, §11.4.3.
        let datos = DatosControl {
            numero: "8110007871",
            fecha: fecha(2019, 1, 12),
            hora: "07:00:00-05:00",
            valor_bruto: dec("5000.00"),
            iva: dec("950.00"),
            inc: Decimal::ZERO,
            ica: Decimal::ZERO,
            total: dec("5950.00"),
            nit_emisor: "900373076",
            documento_adquirente: "8355990",
            ambiente: Ambiente::Produccion,
        };
        assert_eq!(
            cude(&datos, "12301"),
            "907e4444decc9e59c160a2fb3b6659b33dc5b632a5008922b9a62f83f757b1c448e47f5867f2b50dbdb96f48c7681168"
        );
    }

    #[test]
    fn cude_de_contingencia_coincide_con_el_ejemplo_oficial_del_anexo() {
        // Anexo técnico 1.9, §11.4.1 (incluye ICA y ambiente de pruebas).
        let datos = DatosControl {
            numero: "8110007871",
            fecha: fecha(2019, 2, 20),
            hora: "16:46:55-05:00",
            valor_bruto: dec("235.28"),
            iva: dec("19.00"),
            inc: Decimal::ZERO,
            ica: dec("8.28"),
            total: dec("262.56"),
            nit_emisor: "900373076",
            documento_adquirente: "8355990",
            ambiente: Ambiente::Pruebas,
        };
        assert_eq!(
            cude(&datos, "12345"),
            "955327eb55f8bdf16d069358a063d87e1577a292cb088ec186ed60bbc38e750b7b3980659b278ead789b95f9c51a9ef7"
        );
    }

    #[test]
    fn los_montos_se_truncan_y_no_se_redondean() {
        assert_eq!(monto_control(dec("10.999")), "10.99");
        assert_eq!(monto_control(dec("10.005")), "10.00");
        assert_eq!(monto_control(dec("7")), "7.00");
        assert_eq!(monto_control(dec("1500000.5")), "1500000.50");
    }

    #[test]
    fn el_cufe_cambia_si_cambia_el_ambiente() {
        let mut datos = DatosControl {
            numero: "SETP1",
            fecha: fecha(2026, 11, 13),
            hora: "08:00:00-05:00",
            valor_bruto: dec("100"),
            iva: dec("19"),
            inc: Decimal::ZERO,
            ica: Decimal::ZERO,
            total: dec("119"),
            nit_emisor: "901713998",
            documento_adquirente: "222222222222",
            ambiente: Ambiente::Pruebas,
        };
        let pruebas = cufe(&datos, "clave");
        datos.ambiente = Ambiente::Produccion;
        assert_ne!(pruebas, cufe(&datos, "clave"));
    }

    #[test]
    fn codigo_de_seguridad_es_sha384_de_software_pin_y_numero() {
        let codigo = codigo_seguridad_software("56f2ae4e", "12345", "SETP990000002");
        assert_eq!(codigo, sha384_hex("56f2ae4e12345SETP990000002"));
        assert_eq!(codigo.len(), 96);
    }

    #[test]
    fn hora_lleva_la_zona_horaria() {
        let momento = DateTime::parse_from_rfc3339("2019-01-16T10:53:10-05:00").expect("fecha");
        assert_eq!(hora_dian(momento), "10:53:10-05:00");
    }

    #[test]
    fn qr_apunta_al_catalogo_del_ambiente() {
        assert_eq!(
            url_qr("abc", Ambiente::Produccion),
            "https://catalogo-vpfe.dian.gov.co/document/searchqr?documentkey=abc"
        );
        assert!(url_qr("abc", Ambiente::Pruebas).starts_with("https://catalogo-vpfe-hab."));
    }

    #[test]
    fn texto_qr_tiene_los_datos_del_anexo_en_orden() {
        let texto = texto_qr(&DatosQr {
            numero: "323200000129",
            fecha: fecha(2019, 1, 16),
            hora: "10:53:10-05:00",
            nit_emisor: "700085371",
            documento_adquirente: "800199436",
            valor_bruto: dec("1500000"),
            iva: dec("285000"),
            otros_impuestos: Decimal::ZERO,
            total: dec("1785000"),
            codigo: "e5bac48e",
            ambiente: Ambiente::Produccion,
        });
        let lineas: Vec<&str> = texto.lines().collect();
        assert_eq!(lineas[0], "NumFac: 323200000129");
        assert_eq!(lineas[5], "ValFac: 1500000.00");
        assert_eq!(lineas[8], "ValTolFac: 1785000.00");
        assert_eq!(
            lineas[10],
            "QRCode: https://catalogo-vpfe.dian.gov.co/document/searchqr?documentkey=e5bac48e"
        );
    }
}
