//! Lector de facturas electrónicas UBL 2.1 (anexo técnico de la DIAN).
//!
//! Acepta la `Invoice` o el `AttachedDocument` que envía el proveedor (la factura va como texto
//! dentro de `cac:Attachment/cac:ExternalReference/cbc:Description`). Busca por nombre local, sin
//! depender de los prefijos de los espacios de nombres. `roxmltree` no procesa DTD ni entidades
//! externas: un XML con `<!DOCTYPE>` se rechaza (sin XXE).

use std::str::FromStr;

use application::{
    AppError, AppResult,
    puertos::tributario::{FacturaLeida, LectorFacturas},
};
use chrono::NaiveDate;
use roxmltree::{Document, Node};
use rust_decimal::Decimal;

/// Código del IVA en `cac:TaxScheme/cbc:ID`.
const ESQUEMA_IVA: &str = "01";

/// Lector de facturas UBL.
#[derive(Debug, Clone, Copy, Default)]
pub struct LectorUbl;

fn ilegible(detalle: &str) -> AppError {
    AppError::Validacion(format!(
        "el archivo no es una factura electrónica legible: {detalle}"
    ))
}

fn hijo<'a, 'i>(n: Node<'a, 'i>, nombre: &str) -> Option<Node<'a, 'i>> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == nombre)
}

fn ruta<'a, 'i>(n: Node<'a, 'i>, nombres: &[&str]) -> Option<Node<'a, 'i>> {
    nombres
        .iter()
        .try_fold(n, |actual, nombre| hijo(actual, nombre))
}

fn texto(n: Option<Node<'_, '_>>) -> Option<String> {
    n.and_then(|x| x.text())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

fn decimal(n: Option<Node<'_, '_>>, campo: &str) -> AppResult<Decimal> {
    let t = texto(n).ok_or_else(|| ilegible(&format!("falta {campo}")))?;
    Decimal::from_str(&t).map_err(|_| ilegible(&format!("{campo} no es un número")))
}

fn fecha(t: &str, campo: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(t, "%Y-%m-%d").map_err(|_| ilegible(&format!("{campo} inválida")))
}

/// Busca la `Invoice` (la raíz o la incrustada en un `AttachedDocument`).
fn factura_xml(contenido: &str) -> AppResult<String> {
    let doc = Document::parse(contenido).map_err(|_| ilegible("XML mal formado"))?;
    let raiz = doc.root_element();
    match raiz.tag_name().name() {
        "Invoice" => Ok(contenido.to_owned()),
        "AttachedDocument" => raiz
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "Description")
            .filter_map(|n| n.text())
            .map(str::trim)
            .find(|t| t.contains("Invoice"))
            .map(str::to_owned)
            .ok_or_else(|| ilegible("el AttachedDocument no trae la factura")),
        _ => Err(ilegible(
            "no es una factura (Invoice) ni un AttachedDocument",
        )),
    }
}

fn leer_invoice(xml: &str) -> AppResult<FacturaLeida> {
    let doc = Document::parse(xml).map_err(|_| ilegible("XML de la factura mal formado"))?;
    let f = doc.root_element();
    if f.tag_name().name() != "Invoice" {
        return Err(ilegible("no es una factura (Invoice)"));
    }
    let id = texto(hijo(f, "ID")).ok_or_else(|| ilegible("falta el número"))?;
    // El prefijo autorizado viene en las extensiones de la DIAN (`sts:Prefix`).
    let prefijo = f
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "Prefix")
        .and_then(|n| texto(Some(n)))
        .filter(|p| id.starts_with(p.as_str()))
        .unwrap_or_default();
    let numero = id[prefijo.len()..].to_owned();
    if numero.is_empty() {
        return Err(ilegible("falta el número"));
    }
    let cufe = texto(hijo(f, "UUID")).ok_or_else(|| ilegible("falta el CUFE"))?;
    let emision = fecha(
        &texto(hijo(f, "IssueDate")).ok_or_else(|| ilegible("falta la fecha"))?,
        "fecha",
    )?;
    let vencimiento = texto(hijo(f, "DueDate"))
        .or_else(|| {
            texto(
                f.descendants()
                    .find(|n| n.is_element() && n.tag_name().name() == "PaymentDueDate"),
            )
        })
        .map(|t| fecha(&t, "fecha de vencimiento"))
        .transpose()?;
    let parte = ruta(f, &["AccountingSupplierParty", "Party"])
        .ok_or_else(|| ilegible("falta el emisor"))?;
    let esquema = hijo(parte, "PartyTaxScheme");
    let emisor_nit: String = texto(esquema.and_then(|e| hijo(e, "CompanyID")))
        .ok_or_else(|| ilegible("falta el NIT del emisor"))?
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    if !(5..=15).contains(&emisor_nit.len()) {
        return Err(ilegible("NIT del emisor inválido"));
    }
    let emisor_nombre = texto(esquema.and_then(|e| hijo(e, "RegistrationName")))
        .or_else(|| texto(ruta(parte, &["PartyName", "Name"])))
        .ok_or_else(|| ilegible("falta el nombre del emisor"))?;
    let totales = hijo(f, "LegalMonetaryTotal").ok_or_else(|| ilegible("faltan los totales"))?;
    let subtotal = decimal(hijo(totales, "LineExtensionAmount"), "el subtotal")?;
    let total = decimal(hijo(totales, "PayableAmount"), "el total")?;
    let mut iva = Decimal::ZERO;
    for t in f
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "TaxTotal")
    {
        let es_iva = t
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "TaxScheme")
            .and_then(|s| texto(hijo(s, "ID")))
            .is_some_and(|c| c == ESQUEMA_IVA);
        if es_iva {
            iva += decimal(hijo(t, "TaxAmount"), "el IVA")?;
        }
    }
    Ok(FacturaLeida {
        emisor_nit,
        emisor_nombre,
        prefijo,
        numero,
        fecha: emision,
        vencimiento,
        cufe: cufe.to_lowercase(),
        subtotal,
        iva,
        total,
    })
}

impl LectorFacturas for LectorUbl {
    fn leer(&self, contenido: &[u8]) -> AppResult<FacturaLeida> {
        let texto = std::str::from_utf8(contenido).map_err(|_| ilegible("no está en UTF-8"))?;
        let texto = texto.trim_start_matches('\u{feff}');
        leer_invoice(&factura_xml(texto)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACTURA: &str = include_str!("../tests/fixtures/factura.xml");
    const ADJUNTO: &str = include_str!("../tests/fixtures/attached_document.xml");

    #[test]
    fn lee_una_factura_ubl() {
        let f = LectorUbl.leer(FACTURA.as_bytes()).expect("factura");
        assert_eq!(f.emisor_nit, "900373115");
        assert_eq!(f.emisor_nombre, "Proveedor de Aseo S.A.S.");
        assert_eq!(f.prefijo, "SETP");
        assert_eq!(f.numero, "990000123");
        assert_eq!(f.fecha, NaiveDate::from_ymd_opt(2026, 3, 5).expect("fecha"));
        assert_eq!(f.vencimiento, NaiveDate::from_ymd_opt(2026, 4, 4));
        assert_eq!(f.cufe.len(), 96);
        assert_eq!(f.subtotal, Decimal::from(1_000_000));
        assert_eq!(f.iva, Decimal::from(190_000));
        assert_eq!(f.total, Decimal::from(1_190_000));
    }

    #[test]
    fn lee_la_factura_dentro_del_attached_document() {
        let a = LectorUbl.leer(ADJUNTO.as_bytes()).expect("adjunto");
        let f = LectorUbl.leer(FACTURA.as_bytes()).expect("factura");
        assert_eq!(a, f);
    }

    #[test]
    fn rechaza_lo_que_no_es_una_factura() {
        let r = LectorUbl.leer(b"<?xml version=\"1.0\"?><CreditNote/>");
        assert!(matches!(r, Err(AppError::Validacion(_))));
    }

    #[test]
    fn rechaza_un_xml_con_entidades_externas() {
        let xxe = "<?xml version=\"1.0\"?><!DOCTYPE x [<!ENTITY e SYSTEM \"file:///etc/passwd\">]>\
                   <Invoice><ID>&e;</ID></Invoice>";
        assert!(LectorUbl.leer(xxe.as_bytes()).is_err());
    }

    #[test]
    fn rechaza_una_factura_sin_cufe() {
        let sin = FACTURA
            .replace("<cbc:UUID", "<cbc:Otro")
            .replace("</cbc:UUID>", "</cbc:Otro>");
        assert!(matches!(
            LectorUbl.leer(sin.as_bytes()),
            Err(AppError::Validacion(_))
        ));
    }
}
