//! Factura electrónica de venta, nota crédito y nota débito en UBL 2.1 con las extensiones de la
//! DIAN (anexo técnico 1.9, §6.1 a §6.3).
//!
//! [`generar`] arma el árbol del documento **sin firmar** (con la `ext:UBLExtension` vacía
//! donde va la firma), calcula los totales desde las líneas y su CUFE o CUDE. El orden de los
//! elementos sigue los esquemas UBL 2.1 de la caja de herramientas de la DIAN.
//!
//! Este modelo es completo y propio del motor: lo llena el adaptador a partir del
//! `DocumentoAEmitir` del puerto y de la configuración del facturador (fase 5 del plan 17).

use chrono::{DateTime, FixedOffset, NaiveDate};
use rust_decimal::Decimal;

use super::{
    ErrorMotor,
    control::{
        Ambiente, DatosControl, DatosQr, codigo_seguridad_software, cude, cuds, cufe, hora_dian,
        texto_qr, url_qr,
    },
    xml::Elemento,
};

const AGENCIA_DIAN: &str = "195";
const NOMBRE_AGENCIA_DIAN: &str = "CO, DIAN (Dirección de Impuestos y Aduanas Nacionales)";
/// NIT de la DIAN, que autoriza el documento (`sts:AuthorizationProvider`).
const NIT_DIAN: &str = "800197268";
const DV_NIT_DIAN: &str = "4";
/// Tipo de documento NIT (tabla 13.2.1 del anexo).
pub const TIPO_DOCUMENTO_NIT: &str = "31";

/// Software con que se genera el documento, tal como quedó registrado en la DIAN.
#[derive(Clone, PartialEq, Eq)]
pub struct Software {
    /// NIT (sin DV) de quien presta el software: en software propio, el del facturador.
    pub nit_proveedor: String,
    pub dv_proveedor: String,
    /// Identificador asignado por la DIAN al registrar el software.
    pub id: String,
    /// PIN que el facturador definió al registrar el software. Secreto: entra al CUDE y al
    /// código de seguridad, nunca al XML ni a los logs.
    pub pin: String,
}

impl std::fmt::Debug for Software {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Software")
            .field("nit_proveedor", &self.nit_proveedor)
            .field("id", &self.id)
            .field("pin", &"<oculto>")
            .finish()
    }
}

/// Tipo de organización jurídica (`cbc:AdditionalAccountID`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoPersona {
    Juridica,
    Natural,
}

impl TipoPersona {
    const fn codigo(self) -> &'static str {
        match self {
            TipoPersona::Juridica => "1",
            TipoPersona::Natural => "2",
        }
    }
}

/// Régimen frente al IVA: decide el `listName` de las responsabilidades y el tributo de la
/// parte (`01` IVA o `ZZ` no aplica).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegimenIva {
    Responsable,
    NoResponsable,
}

impl RegimenIva {
    pub(super) const fn lista(self) -> &'static str {
        match self {
            RegimenIva::Responsable => "48",
            RegimenIva::NoResponsable => "49",
        }
    }

    const fn tributo(self) -> (&'static str, &'static str) {
        match self {
            RegimenIva::Responsable => ("01", "IVA"),
            RegimenIva::NoResponsable => ("ZZ", "No aplica"),
        }
    }
}

/// Dirección con los códigos DANE (tablas 13.4.2 y 13.4.3 del anexo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Direccion {
    /// Código DANE del municipio, cinco dígitos (`11001`).
    pub municipio: String,
    pub ciudad: String,
    pub departamento: String,
    /// Código DANE del departamento, dos dígitos (`11`).
    pub codigo_departamento: String,
    pub linea: String,
}

/// Emisor o adquirente del documento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parte {
    pub tipo_persona: TipoPersona,
    /// Tipo de documento (tabla 13.2.1: `31` NIT, `13` cédula…).
    pub tipo_documento: String,
    /// Número sin puntos ni DV. Consumidor final: `222222222222`.
    pub documento: String,
    /// Dígito de verificación; obligatorio con NIT.
    pub dv: Option<String>,
    pub nombre: String,
    pub regimen: RegimenIva,
    /// Responsabilidades fiscales del RUT (`O-13`, `O-15`, `R-99-PN`…).
    pub responsabilidades: Vec<String>,
    pub direccion: Option<Direccion>,
    /// Correo de recepción de documentos electrónicos.
    pub correo: Option<String>,
}

/// Línea del documento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linea {
    pub descripcion: String,
    pub cantidad: Decimal,
    /// Unidad de medida (tabla 13.3.6, `94` = unidad).
    pub unidad: String,
    pub valor_unitario: Decimal,
    /// Cantidad × valor unitario, ya redondeado a pesos con centavos.
    pub subtotal: Decimal,
    /// IVA de la línea: tarifa en porcentaje y valor. `None` si la línea no lo causa.
    pub iva: Option<(Decimal, Decimal)>,
}

/// Rango de numeración autorizado por la DIAN (`sts:InvoiceControl`).
#[derive(Clone, PartialEq, Eq)]
pub struct Resolucion {
    pub numero: String,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: NaiveDate,
    pub prefijo: String,
    pub desde: u64,
    pub hasta: u64,
    /// Clave técnica del rango (`GetNumberingRange`). Secreta: solo entra al CUFE.
    pub clave_tecnica: String,
}

impl std::fmt::Debug for Resolucion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolucion")
            .field("numero", &self.numero)
            .field("prefijo", &self.prefijo)
            .field("desde", &self.desde)
            .field("hasta", &self.hasta)
            .field("clave_tecnica", &"<oculta>")
            .finish()
    }
}

/// Factura a la que se refiere una nota.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referencia {
    /// Prefijo y número de la factura.
    pub numero: String,
    pub cufe: String,
    pub fecha: NaiveDate,
}

/// Concepto de corrección de una nota (tablas 13.2.4 y 13.2.5) y su descripción.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correccion {
    pub concepto: String,
    pub descripcion: String,
}

/// Clase de documento con sus datos propios.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clase {
    Factura {
        resolucion: Resolucion,
        /// Contingencia de la DIAN (tipo 04, anexo §12.2): mismo número y CUFE con la clave
        /// técnica, se entrega sin validación previa y se transmite después.
        contingencia_dian: bool,
        /// Transcripción de una factura de talonario de la contingencia del facturador (tipo 03,
        /// anexo §12.1): CUDE con el PIN (su resolución no tiene clave técnica) y referencia a
        /// la factura de papel.
        contingencia_facturador: bool,
    },
    NotaCredito {
        referencia: Referencia,
        correccion: Correccion,
    },
    NotaDebito {
        referencia: Referencia,
        correccion: Correccion,
    },
    /// Documento soporte en adquisiciones a no obligados a facturar (anexo DS v1.1): en
    /// [`DocumentoUbl`], `emisor` es el **vendedor** no obligado (SNO, va en
    /// `AccountingSupplierParty`) y `adquirente` la entidad que compra y emite (ABS).
    DocumentoSoporte {
        resolucion: Resolucion,
        /// Vendedor residente fiscal en Colombia (`10`) o no residente (`11`).
        residente: bool,
    },
    /// Nota de ajuste al documento soporte (mismas partes que el documento soporte).
    NotaAjusteSoporte {
        referencia: Referencia,
        correccion: Correccion,
    },
}

impl Clase {
    /// ¿Es del documento soporte (vendedor en `emisor`, entidad en `adquirente`)?
    pub fn es_soporte(&self) -> bool {
        matches!(
            self,
            Clase::DocumentoSoporte { .. } | Clase::NotaAjusteSoporte { .. }
        )
    }
}

/// Forma de pago (tabla 13.3.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormaPago {
    Contado,
    Credito { vence: NaiveDate },
}

/// Documento completo para generar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentoUbl {
    pub clase: Clase,
    pub prefijo: String,
    pub numero: u64,
    /// Fecha y hora de emisión con la zona de Colombia (`-05:00`).
    pub emitido: DateTime<FixedOffset>,
    pub ambiente: Ambiente,
    pub emisor: Parte,
    pub adquirente: Parte,
    pub lineas: Vec<Linea>,
    pub forma_pago: FormaPago,
    /// Medio de pago (tabla 13.3.4.2: `10` efectivo, `42` consignación, `ZZZ` acuerdo mutuo…).
    pub medio_pago: String,
}

/// Documento generado sin firmar, con su código de control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentoGenerado {
    pub(crate) raiz: Elemento,
    /// CUFE (factura) o CUDE (notas).
    pub codigo: String,
    /// Prefijo y número.
    pub numero: String,
    pub valor_bruto: Decimal,
    pub iva: Decimal,
    pub total: Decimal,
    /// Texto del QR de la representación gráfica.
    pub qr: String,
}

/// Totales de IVA agrupados por tarifa.
struct Totales {
    bruto: Decimal,
    iva: Decimal,
    por_tarifa: Vec<(Decimal, Decimal, Decimal)>,
}

fn totales(lineas: &[Linea]) -> Totales {
    let mut por_tarifa: Vec<(Decimal, Decimal, Decimal)> = Vec::new();
    for l in lineas {
        if let Some((tarifa, valor)) = l.iva {
            match por_tarifa.iter_mut().find(|(t, _, _)| *t == tarifa) {
                Some((_, base, impuesto)) => {
                    *base += l.subtotal;
                    *impuesto += valor;
                }
                None => por_tarifa.push((tarifa, l.subtotal, valor)),
            }
        }
    }
    Totales {
        bruto: lineas.iter().map(|l| l.subtotal).sum(),
        iva: por_tarifa.iter().map(|(_, _, v)| *v).sum(),
        por_tarifa,
    }
}

fn validar(d: &DocumentoUbl) -> Result<(), ErrorMotor> {
    let datos = |m: &str| Err(ErrorMotor::Datos(m.into()));
    if d.lineas.is_empty() {
        return datos("el documento no tiene líneas");
    }
    if d.emisor.documento.trim().is_empty() || d.adquirente.documento.trim().is_empty() {
        return datos("falta el documento del emisor o del adquirente");
    }
    // En el documento soporte factura la entidad (`adquirente`): su NIT lleva DV; el vendedor
    // (`emisor`) puede identificarse con cédula y debe informar su ubicación.
    let (facturador, contraparte) = if d.clase.es_soporte() {
        (&d.adquirente, &d.emisor)
    } else {
        (&d.emisor, &d.adquirente)
    };
    if facturador.dv.is_none() {
        return datos("falta el dígito de verificación del NIT del facturador");
    }
    if contraparte.tipo_documento == TIPO_DOCUMENTO_NIT && contraparte.dv.is_none() {
        return datos("falta el dígito de verificación del NIT de la contraparte");
    }
    if d.emisor.direccion.is_none() {
        return datos(if d.clase.es_soporte() {
            "falta la dirección del vendedor del documento soporte"
        } else {
            "falta la dirección fiscal del emisor"
        });
    }
    if d.lineas
        .iter()
        .any(|l| l.subtotal < Decimal::ZERO || l.cantidad <= Decimal::ZERO)
    {
        return datos("una línea tiene cantidad o subtotal inválidos");
    }
    if let Clase::Factura { resolucion: r, .. } | Clase::DocumentoSoporte { resolucion: r, .. } =
        &d.clase
    {
        if r.prefijo != d.prefijo {
            return datos("el prefijo no es el de la resolución");
        }
        if d.numero < r.desde || d.numero > r.hasta {
            return datos("el número está fuera del rango autorizado");
        }
        let fecha = d.emitido.date_naive();
        if fecha < r.vigente_desde || fecha > r.vigente_hasta {
            return datos("la resolución de numeración no está vigente en la fecha de emisión");
        }
    }
    if let FormaPago::Credito { vence } = d.forma_pago
        && vence < d.emitido.date_naive()
    {
        return datos("el vencimiento es anterior a la emisión");
    }
    Ok(())
}

fn monto(valor: Decimal) -> String {
    format!("{:.2}", valor.round_dp(2))
}

fn con_moneda(nombre: &str, valor: Decimal) -> Elemento {
    Elemento::hoja(nombre, monto(valor)).attr("currencyID", "COP")
}

fn identificador_dian(nombre: &str, texto: &str) -> Elemento {
    Elemento::hoja(nombre, texto)
        .attr("schemeAgencyID", AGENCIA_DIAN)
        .attr("schemeAgencyName", NOMBRE_AGENCIA_DIAN)
}

pub(super) fn nit_dian(nombre: &str, numero: &str, dv: Option<&str>, tipo: &str) -> Elemento {
    let e = identificador_dian(nombre, numero).attr("schemeName", tipo);
    match dv {
        Some(dv) => e.attr("schemeID", dv),
        None => e,
    }
}

fn direccion(nombre: &str, d: &Direccion) -> Elemento {
    Elemento::new(nombre)
        .hijo(Elemento::hoja("cbc:ID", d.municipio.clone()))
        .hijo(Elemento::hoja("cbc:CityName", d.ciudad.clone()))
        .hijo(Elemento::hoja(
            "cbc:CountrySubentity",
            d.departamento.clone(),
        ))
        .hijo(Elemento::hoja(
            "cbc:CountrySubentityCode",
            d.codigo_departamento.clone(),
        ))
        .hijo(Elemento::new("cac:AddressLine").hijo(Elemento::hoja("cbc:Line", d.linea.clone())))
        .hijo(
            Elemento::new("cac:Country")
                .hijo(Elemento::hoja("cbc:IdentificationCode", "CO"))
                .hijo(Elemento::hoja("cbc:Name", "Colombia").attr("languageID", "es")),
        )
}

pub(super) fn tributo(regimen: RegimenIva) -> Elemento {
    let (id, nombre) = regimen.tributo();
    Elemento::new("cac:TaxScheme")
        .hijo(Elemento::hoja("cbc:ID", id))
        .hijo(Elemento::hoja("cbc:Name", nombre))
}

fn parte(nombre: &str, p: &Parte, emisor: bool, prefijo: Option<&str>) -> Elemento {
    let id_fiscal = || {
        nit_dian(
            "cbc:CompanyID",
            &p.documento,
            p.dv.as_deref(),
            &p.tipo_documento,
        )
    };
    let responsabilidades = if p.responsabilidades.is_empty() {
        "R-99-PN".to_owned()
    } else {
        p.responsabilidades.join(";")
    };
    let identificacion = (!emisor).then(|| {
        let id =
            Elemento::hoja("cbc:ID", p.documento.clone()).attr("schemeName", &p.tipo_documento);
        let id = match &p.dv {
            Some(dv) => id.attr("schemeID", dv),
            None => id,
        };
        Elemento::new("cac:PartyIdentification").hijo(id)
    });
    let ubicacion = p
        .direccion
        .as_ref()
        .map(|d| Elemento::new("cac:PhysicalLocation").hijo(direccion("cac:Address", d)));
    let esquema = Elemento::new("cac:PartyTaxScheme")
        .hijo(Elemento::hoja("cbc:RegistrationName", p.nombre.clone()))
        .hijo(id_fiscal())
        .hijo(
            Elemento::hoja("cbc:TaxLevelCode", responsabilidades)
                .attr("listName", p.regimen.lista()),
        )
        .hijo_si(
            p.direccion
                .as_ref()
                .map(|d| direccion("cac:RegistrationAddress", d)),
        )
        .hijo(tributo(p.regimen));
    let entidad = Elemento::new("cac:PartyLegalEntity")
        .hijo(Elemento::hoja("cbc:RegistrationName", p.nombre.clone()))
        .hijo(id_fiscal())
        .hijo_si(prefijo.map(|pr| {
            Elemento::new("cac:CorporateRegistrationScheme").hijo(Elemento::hoja("cbc:ID", pr))
        }));
    let contacto = p.correo.as_ref().map(|c| {
        Elemento::new("cac:Contact").hijo(Elemento::hoja("cbc:ElectronicMail", c.clone()))
    });
    Elemento::new(nombre)
        .hijo(Elemento::hoja(
            "cbc:AdditionalAccountID",
            p.tipo_persona.codigo(),
        ))
        .hijo(
            Elemento::new("cac:Party")
                .hijo_si(identificacion)
                .hijo(
                    Elemento::new("cac:PartyName")
                        .hijo(Elemento::hoja("cbc:Name", p.nombre.clone())),
                )
                .hijo_si(ubicacion)
                .hijo(esquema)
                .hijo(entidad)
                .hijo_si(contacto),
        )
}

fn subtotal_iva(base: Decimal, impuesto: Decimal, tarifa: Decimal) -> Elemento {
    Elemento::new("cac:TaxSubtotal")
        .hijo(con_moneda("cbc:TaxableAmount", base))
        .hijo(con_moneda("cbc:TaxAmount", impuesto))
        .hijo(
            Elemento::new("cac:TaxCategory")
                .hijo(Elemento::hoja("cbc:Percent", monto(tarifa)))
                .hijo(tributo(RegimenIva::Responsable)),
        )
}

fn linea(nombre: &str, cantidad: &str, i: usize, l: &Linea) -> Elemento {
    let iva = l.iva.map(|(tarifa, valor)| {
        Elemento::new("cac:TaxTotal")
            .hijo(con_moneda("cbc:TaxAmount", valor))
            .hijo(subtotal_iva(l.subtotal, valor, tarifa))
    });
    Elemento::new(nombre)
        .hijo(Elemento::hoja("cbc:ID", (i + 1).to_string()))
        .hijo(Elemento::hoja(cantidad, format!("{:.6}", l.cantidad)).attr("unitCode", &l.unidad))
        .hijo(con_moneda("cbc:LineExtensionAmount", l.subtotal))
        .hijo_si(iva)
        .hijo(
            Elemento::new("cac:Item")
                .hijo(Elemento::hoja("cbc:Description", l.descripcion.clone())),
        )
        .hijo(
            Elemento::new("cac:Price")
                .hijo(con_moneda("cbc:PriceAmount", l.valor_unitario))
                .hijo(
                    Elemento::hoja("cbc:BaseQuantity", format!("{:.6}", l.cantidad))
                        .attr("unitCode", &l.unidad),
                ),
        )
}

/// Extensiones de la DIAN de cualquier documento (anexo §6.1): control de numeración (solo
/// las facturas), país, proveedor y huella del software, autorizador y QR.
pub(super) fn extensiones_dian(
    control: Option<Elemento>,
    software: &Software,
    numero: &str,
    qr: &str,
) -> Elemento {
    Elemento::new("sts:DianExtensions")
        .hijo_si(control)
        .hijo(
            Elemento::new("sts:InvoiceSource").hijo(
                Elemento::hoja("cbc:IdentificationCode", "CO")
                    .attr("listAgencyID", "6")
                    .attr("listAgencyName", "United Nations Economic Commission for Europe")
                    .attr(
                        "listSchemeURI",
                        "urn:oasis:names:specification:ubl:codelist:gc:CountryIdentificationCode-2.1",
                    ),
            ),
        )
        .hijo(
            Elemento::new("sts:SoftwareProvider")
                .hijo(nit_dian(
                    "sts:ProviderID",
                    &software.nit_proveedor,
                    Some(&software.dv_proveedor),
                    TIPO_DOCUMENTO_NIT,
                ))
                .hijo(identificador_dian("sts:SoftwareID", &software.id)),
        )
        .hijo(identificador_dian(
            "sts:SoftwareSecurityCode",
            &codigo_seguridad_software(&software.id, &software.pin, numero),
        ))
        .hijo(Elemento::new("sts:AuthorizationProvider").hijo(nit_dian(
            "sts:AuthorizationProviderID",
            NIT_DIAN,
            Some(DV_NIT_DIAN),
            TIPO_DOCUMENTO_NIT,
        )))
        .hijo(Elemento::hoja("sts:QRCode", qr))
}

/// Espacios de nombres de la raíz: todos declarados ahí, como en los ejemplos de la DIAN, para
/// que la firma de los subconjuntos herede siempre los mismos.
pub(super) fn raiz(nombre: &str, espacio_documento: &str) -> Elemento {
    Elemento::new(nombre)
        .ns("", espacio_documento)
        .ns(
            "cac",
            "urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2",
        )
        .ns(
            "cbc",
            "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2",
        )
        .ns("ds", "http://www.w3.org/2000/09/xmldsig#")
        .ns(
            "ext",
            "urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2",
        )
        .ns("sts", "dian:gov:co:facturaelectronica:Structures-2-1")
        .ns("xades", "http://uri.etsi.org/01903/v1.3.2#")
        .ns("xades141", "http://uri.etsi.org/01903/v1.4.1#")
        .ns("xsi", "http://www.w3.org/2001/XMLSchema-instance")
}

/// Genera el documento sin firmar.
///
/// # Errors
///
/// [`ErrorMotor::Datos`] si al documento le falta un dato obligatorio o no cuadra con su
/// resolución de numeración.
pub fn generar(d: &DocumentoUbl, software: &Software) -> Result<DocumentoGenerado, ErrorMotor> {
    validar(d)?;
    let t = totales(&d.lineas);
    if d.clase.es_soporte() {
        return Ok(generar_soporte(d, software, &t));
    }
    let total = t.bruto + t.iva;
    let numero = format!("{}{}", d.prefijo, d.numero);
    let hora = hora_dian(d.emitido);
    let fecha = d.emitido.date_naive();
    let datos = DatosControl {
        numero: &numero,
        fecha,
        hora: &hora,
        valor_bruto: t.bruto,
        iva: t.iva,
        inc: Decimal::ZERO,
        ica: Decimal::ZERO,
        total,
        nit_emisor: &d.emisor.documento,
        documento_adquirente: &d.adquirente.documento,
        ambiente: d.ambiente,
    };
    let (codigo, esquema_codigo) = match &d.clase {
        Clase::Factura {
            contingencia_facturador: false,
            resolucion,
            ..
        } => (cufe(&datos, &resolucion.clave_tecnica), "CUFE-SHA384"),
        _ => (cude(&datos, &software.pin), "CUDE-SHA384"),
    };
    let qr = texto_qr(&DatosQr {
        numero: &numero,
        fecha,
        hora: &hora,
        nit_emisor: &d.emisor.documento,
        documento_adquirente: &d.adquirente.documento,
        valor_bruto: t.bruto,
        iva: t.iva,
        otros_impuestos: Decimal::ZERO,
        total,
        codigo: &codigo,
        ambiente: d.ambiente,
    });

    let (nombre, espacio, personalizacion, perfil, linea_nombre, cantidad) = match &d.clase {
        Clase::Factura { .. } => (
            "Invoice",
            "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2",
            "10",
            "DIAN 2.1: Factura Electrónica de Venta",
            "cac:InvoiceLine",
            "cbc:InvoicedQuantity",
        ),
        Clase::NotaCredito { .. } => (
            "CreditNote",
            "urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2",
            "20",
            "DIAN 2.1: Nota Crédito de Factura Electrónica de Venta",
            "cac:CreditNoteLine",
            "cbc:CreditedQuantity",
        ),
        Clase::NotaDebito { .. }
        | Clase::DocumentoSoporte { .. }
        | Clase::NotaAjusteSoporte { .. } => (
            "DebitNote",
            "urn:oasis:names:specification:ubl:schema:xsd:DebitNote-2",
            "30",
            "DIAN 2.1: Nota Débito de Factura Electrónica de Venta",
            "cac:DebitNoteLine",
            "cbc:DebitedQuantity",
        ),
    };

    let control = match &d.clase {
        Clase::Factura { resolucion: r, .. } => Some(
            Elemento::new("sts:InvoiceControl")
                .hijo(Elemento::hoja("sts:InvoiceAuthorization", r.numero.clone()))
                .hijo(
                    Elemento::new("sts:AuthorizationPeriod")
                        .hijo(Elemento::hoja("cbc:StartDate", r.vigente_desde.to_string()))
                        .hijo(Elemento::hoja("cbc:EndDate", r.vigente_hasta.to_string())),
                )
                .hijo(
                    Elemento::new("sts:AuthorizedInvoices")
                        .hijo(Elemento::hoja("sts:Prefix", r.prefijo.clone()))
                        .hijo(Elemento::hoja("sts:From", r.desde.to_string()))
                        .hijo(Elemento::hoja("sts:To", r.hasta.to_string())),
                ),
        ),
        _ => None,
    };
    let extensiones_dian = extensiones_dian(control, software, &numero, &qr);

    let mut doc = raiz(nombre, espacio)
        .attr(
            "xsi:schemaLocation",
            format!("{espacio} http://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/maindoc/UBL-{nombre}-2.1.xsd"),
        )
        .hijo(
            Elemento::new("ext:UBLExtensions")
                .hijo(
                    Elemento::new("ext:UBLExtension")
                        .hijo(Elemento::new("ext:ExtensionContent").hijo(extensiones_dian)),
                )
                .hijo(Elemento::new("ext:UBLExtension").hijo(Elemento::new("ext:ExtensionContent"))),
        )
        .hijo(Elemento::hoja("cbc:UBLVersionID", "UBL 2.1"))
        .hijo(Elemento::hoja("cbc:CustomizationID", personalizacion))
        .hijo(Elemento::hoja("cbc:ProfileID", perfil))
        .hijo(Elemento::hoja("cbc:ProfileExecutionID", d.ambiente.codigo()))
        .hijo(Elemento::hoja("cbc:ID", numero.clone()))
        .hijo(
            Elemento::hoja("cbc:UUID", codigo.clone())
                .attr("schemeID", d.ambiente.codigo())
                .attr("schemeName", esquema_codigo),
        )
        .hijo(Elemento::hoja("cbc:IssueDate", fecha.to_string()))
        .hijo(Elemento::hoja("cbc:IssueTime", hora));
    doc = match &d.clase {
        Clase::Factura {
            contingencia_dian,
            contingencia_facturador,
            ..
        } => {
            let vence = match d.forma_pago {
                FormaPago::Credito { vence } => {
                    Some(Elemento::hoja("cbc:DueDate", vence.to_string()))
                }
                FormaPago::Contado => None,
            };
            let tipo = if *contingencia_facturador {
                "03"
            } else if *contingencia_dian {
                "04"
            } else {
                "01"
            };
            doc.hijo_si(vence)
                .hijo(Elemento::hoja("cbc:InvoiceTypeCode", tipo))
        }
        Clase::NotaCredito { .. } => doc.hijo(Elemento::hoja("cbc:CreditNoteTypeCode", "91")),
        Clase::NotaDebito { .. }
        | Clase::DocumentoSoporte { .. }
        | Clase::NotaAjusteSoporte { .. } => doc,
    };
    doc = doc
        .hijo(
            Elemento::hoja("cbc:DocumentCurrencyCode", "COP")
                .attr("listAgencyID", "6")
                .attr(
                    "listAgencyName",
                    "United Nations Economic Commission for Europe",
                )
                .attr("listID", "ISO 4217 Alpha"),
        )
        .hijo(Elemento::hoja(
            "cbc:LineCountNumeric",
            d.lineas.len().to_string(),
        ));
    if let Clase::NotaCredito {
        referencia,
        correccion,
    }
    | Clase::NotaDebito {
        referencia,
        correccion,
    } = &d.clase
    {
        doc = doc
            .hijo(
                Elemento::new("cac:DiscrepancyResponse")
                    .hijo(Elemento::hoja("cbc:ReferenceID", referencia.numero.clone()))
                    .hijo(Elemento::hoja(
                        "cbc:ResponseCode",
                        correccion.concepto.clone(),
                    ))
                    .hijo(Elemento::hoja(
                        "cbc:Description",
                        correccion.descripcion.clone(),
                    )),
            )
            .hijo(
                Elemento::new("cac:BillingReference").hijo(
                    Elemento::new("cac:InvoiceDocumentReference")
                        .hijo(Elemento::hoja("cbc:ID", referencia.numero.clone()))
                        .hijo(
                            Elemento::hoja("cbc:UUID", referencia.cufe.clone())
                                .attr("schemeName", "CUFE-SHA384"),
                        )
                        .hijo(Elemento::hoja(
                            "cbc:IssueDate",
                            referencia.fecha.to_string(),
                        )),
                ),
            );
    }
    if let Clase::Factura {
        contingencia_facturador: true,
        ..
    } = &d.clase
    {
        // La factura de papel que se transcribe (anexo FAI01–FAI06): su número y su fecha.
        doc = doc.hijo(
            Elemento::new("cac:AdditionalDocumentReference")
                .hijo(Elemento::hoja("cbc:ID", numero.clone()))
                .hijo(Elemento::hoja("cbc:UUID", codigo.clone()).attr("schemeName", "CUDE-SHA384"))
                .hijo(Elemento::hoja("cbc:IssueDate", fecha.to_string()))
                .hijo(Elemento::hoja("cbc:DocumentTypeCode", "FTC")),
        );
    }
    let prefijo_emisor = matches!(d.clase, Clase::Factura { .. }).then_some(d.prefijo.as_str());
    let (forma, vence) = match d.forma_pago {
        FormaPago::Contado => ("1", None),
        FormaPago::Credito { vence } => ("2", Some(vence)),
    };
    let impuestos = (!t.por_tarifa.is_empty()).then(|| {
        Elemento::new("cac:TaxTotal")
            .hijo(con_moneda("cbc:TaxAmount", t.iva))
            .hijos(
                t.por_tarifa
                    .iter()
                    .map(|(tarifa, base, valor)| subtotal_iva(*base, *valor, *tarifa)),
            )
    });
    let totales_nombre = if matches!(d.clase, Clase::NotaDebito { .. }) {
        "cac:RequestedMonetaryTotal"
    } else {
        "cac:LegalMonetaryTotal"
    };
    let base_gravable: Decimal = t.por_tarifa.iter().map(|(_, base, _)| *base).sum();
    doc = doc
        .hijo(parte(
            "cac:AccountingSupplierParty",
            &d.emisor,
            true,
            prefijo_emisor,
        ))
        .hijo(parte(
            "cac:AccountingCustomerParty",
            &d.adquirente,
            false,
            None,
        ))
        .hijo(
            Elemento::new("cac:PaymentMeans")
                .hijo(Elemento::hoja("cbc:ID", forma))
                .hijo(Elemento::hoja("cbc:PaymentMeansCode", d.medio_pago.clone()))
                .hijo_si(vence.map(|v| Elemento::hoja("cbc:PaymentDueDate", v.to_string()))),
        )
        .hijo_si(impuestos)
        .hijo(
            Elemento::new(totales_nombre)
                .hijo(con_moneda("cbc:LineExtensionAmount", t.bruto))
                .hijo(con_moneda("cbc:TaxExclusiveAmount", base_gravable))
                .hijo(con_moneda("cbc:TaxInclusiveAmount", total))
                .hijo(con_moneda("cbc:PayableAmount", total)),
        )
        .hijos(
            d.lineas
                .iter()
                .enumerate()
                .map(|(i, l)| linea(linea_nombre, cantidad, i, l)),
        );

    Ok(DocumentoGenerado {
        raiz: doc,
        codigo,
        numero,
        valor_bruto: t.bruto,
        iva: t.iva,
        total,
        qr,
    })
}

const PERFIL_SOPORTE: &str =
    "DIAN 2.1: documento soporte en adquisiciones efectuadas a no obligados a facturar.";
const PERFIL_AJUSTE_SOPORTE: &str = "DIAN 2.1: Nota de ajuste al documento soporte en \
     adquisiciones efectuadas a sujetos no obligados a expedir factura o documento equivalente";
/// Estándar de identificación del ítem adoptado por el contribuyente (anexo DS §16.3.5).
const ESTANDAR_CONTRIBUYENTE: &str = "999";

/// Esquema tributario de una parte del documento soporte: nombre, identificación,
/// responsabilidades y tributo.
fn esquema_soporte(p: &Parte) -> Elemento {
    let responsabilidades = if p.responsabilidades.is_empty() {
        "R-99-PN".to_owned()
    } else {
        p.responsabilidades.join(";")
    };
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
        .hijo(tributo(p.regimen))
}

/// Documento soporte o nota de ajuste: vendedor no obligado como proveedor (con su ubicación),
/// entidad como adquirente, CUDS, `InvoicePeriod` por operación en cada línea del documento
/// soporte e identificación estándar del ítem (anexo DS v1.1, §8.1 y §8.2).
fn generar_soporte(d: &DocumentoUbl, software: &Software, t: &Totales) -> DocumentoGenerado {
    let total = t.bruto + t.iva;
    let numero = format!("{}{}", d.prefijo, d.numero);
    let hora = hora_dian(d.emitido);
    let fecha = d.emitido.date_naive();
    let codigo = cuds(
        &DatosControl {
            numero: &numero,
            fecha,
            hora: &hora,
            valor_bruto: t.bruto,
            iva: t.iva,
            inc: Decimal::ZERO,
            ica: Decimal::ZERO,
            total,
            nit_emisor: &d.emisor.documento,
            documento_adquirente: &d.adquirente.documento,
            ambiente: d.ambiente,
        },
        &software.pin,
    );
    let qr = url_qr(&codigo, d.ambiente);
    let soporte = matches!(d.clase, Clase::DocumentoSoporte { .. });
    let (nombre, espacio, linea_nombre, cantidad) = if soporte {
        (
            "Invoice",
            "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2",
            "cac:InvoiceLine",
            "cbc:InvoicedQuantity",
        )
    } else {
        (
            "CreditNote",
            "urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2",
            "cac:CreditNoteLine",
            "cbc:CreditedQuantity",
        )
    };
    let (personalizacion, perfil, control) = match &d.clase {
        Clase::DocumentoSoporte {
            resolucion: r,
            residente,
        } => (
            if *residente { "10" } else { "11" },
            PERFIL_SOPORTE,
            Some(
                Elemento::new("sts:InvoiceControl")
                    .hijo(Elemento::hoja("sts:InvoiceAuthorization", r.numero.clone()))
                    .hijo(
                        Elemento::new("sts:AuthorizationPeriod")
                            .hijo(Elemento::hoja("cbc:StartDate", r.vigente_desde.to_string()))
                            .hijo(Elemento::hoja("cbc:EndDate", r.vigente_hasta.to_string())),
                    )
                    .hijo(
                        Elemento::new("sts:AuthorizedInvoices")
                            .hijo(Elemento::hoja("sts:Prefix", r.prefijo.clone()))
                            .hijo(Elemento::hoja("sts:From", r.desde.to_string()))
                            .hijo(Elemento::hoja("sts:To", r.hasta.to_string())),
                    ),
            ),
        ),
        _ => ("10", PERFIL_AJUSTE_SOPORTE, None),
    };
    let mut doc = raiz(nombre, espacio)
        .attr(
            "xsi:schemaLocation",
            format!("{espacio} http://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/maindoc/UBL-{nombre}-2.1.xsd"),
        )
        .hijo(
            Elemento::new("ext:UBLExtensions")
                .hijo(Elemento::new("ext:UBLExtension").hijo(
                    Elemento::new("ext:ExtensionContent").hijo(extensiones_dian(
                        control, software, &numero, &qr,
                    )),
                ))
                .hijo(Elemento::new("ext:UBLExtension").hijo(Elemento::new("ext:ExtensionContent"))),
        )
        .hijo(Elemento::hoja("cbc:UBLVersionID", "UBL 2.1"))
        .hijo(Elemento::hoja("cbc:CustomizationID", personalizacion))
        .hijo(Elemento::hoja("cbc:ProfileID", perfil))
        .hijo(Elemento::hoja("cbc:ProfileExecutionID", d.ambiente.codigo()))
        .hijo(Elemento::hoja("cbc:ID", numero.clone()))
        .hijo(
            Elemento::hoja("cbc:UUID", codigo.clone())
                .attr("schemeID", d.ambiente.codigo())
                .attr("schemeName", "CUDS-SHA384"),
        )
        .hijo(Elemento::hoja("cbc:IssueDate", fecha.to_string()))
        .hijo(Elemento::hoja("cbc:IssueTime", hora));
    let vence = match d.forma_pago {
        FormaPago::Credito { vence } => Some(vence),
        FormaPago::Contado => None,
    };
    doc = if soporte {
        doc.hijo_si(vence.map(|v| Elemento::hoja("cbc:DueDate", v.to_string())))
            .hijo(Elemento::hoja("cbc:InvoiceTypeCode", "05"))
    } else {
        doc.hijo(Elemento::hoja("cbc:CreditNoteTypeCode", "95"))
    };
    doc = doc
        .hijo(
            Elemento::hoja("cbc:DocumentCurrencyCode", "COP")
                .attr("listAgencyID", "6")
                .attr(
                    "listAgencyName",
                    "United Nations Economic Commission for Europe",
                )
                .attr("listID", "ISO 4217 Alpha"),
        )
        .hijo(Elemento::hoja(
            "cbc:LineCountNumeric",
            d.lineas.len().to_string(),
        ));
    if let Clase::NotaAjusteSoporte {
        referencia,
        correccion,
    } = &d.clase
    {
        doc = doc
            .hijo(
                Elemento::new("cac:DiscrepancyResponse")
                    .hijo(Elemento::hoja(
                        "cbc:ResponseCode",
                        correccion.concepto.clone(),
                    ))
                    .hijo(Elemento::hoja(
                        "cbc:Description",
                        correccion.descripcion.clone(),
                    )),
            )
            .hijo(
                Elemento::new("cac:BillingReference").hijo(
                    Elemento::new("cac:InvoiceDocumentReference")
                        .hijo(Elemento::hoja("cbc:ID", referencia.numero.clone()))
                        .hijo(
                            Elemento::hoja("cbc:UUID", referencia.cufe.clone())
                                .attr("schemeName", "CUDS-SHA384"),
                        )
                        .hijo(Elemento::hoja(
                            "cbc:IssueDate",
                            referencia.fecha.to_string(),
                        )),
                ),
            );
    }
    let vendedor = Elemento::new("cac:AccountingSupplierParty")
        .hijo(Elemento::hoja(
            "cbc:AdditionalAccountID",
            d.emisor.tipo_persona.codigo(),
        ))
        .hijo(
            Elemento::new("cac:Party")
                .hijo_si(d.emisor.direccion.as_ref().map(|dir| {
                    Elemento::new("cac:PhysicalLocation").hijo(direccion("cac:Address", dir))
                }))
                .hijo(esquema_soporte(&d.emisor)),
        );
    let adquirente = Elemento::new("cac:AccountingCustomerParty")
        .hijo(Elemento::hoja(
            "cbc:AdditionalAccountID",
            d.adquirente.tipo_persona.codigo(),
        ))
        .hijo(Elemento::new("cac:Party").hijo(esquema_soporte(&d.adquirente)));
    let impuestos = (!t.por_tarifa.is_empty()).then(|| {
        Elemento::new("cac:TaxTotal")
            .hijo(con_moneda("cbc:TaxAmount", t.iva))
            .hijos(
                t.por_tarifa
                    .iter()
                    .map(|(tarifa, base, valor)| subtotal_iva(*base, *valor, *tarifa)),
            )
    });
    let base_gravable: Decimal = t.por_tarifa.iter().map(|(_, base, _)| *base).sum();
    let lineas = d.lineas.iter().enumerate().map(|(i, l)| {
        let iva = l.iva.map(|(tarifa, valor)| {
            Elemento::new("cac:TaxTotal")
                .hijo(con_moneda("cbc:TaxAmount", valor))
                .hijo(subtotal_iva(l.subtotal, valor, tarifa))
        });
        let periodo = soporte.then(|| {
            Elemento::new("cac:InvoicePeriod")
                .hijo(Elemento::hoja("cbc:StartDate", fecha.to_string()))
                .hijo(Elemento::hoja("cbc:DescriptionCode", "1"))
                .hijo(Elemento::hoja("cbc:Description", "Por operación"))
        });
        Elemento::new(linea_nombre)
            .hijo(Elemento::hoja("cbc:ID", (i + 1).to_string()))
            .hijo(
                Elemento::hoja(cantidad, format!("{:.6}", l.cantidad)).attr("unitCode", &l.unidad),
            )
            .hijo(con_moneda("cbc:LineExtensionAmount", l.subtotal))
            .hijo_si(periodo)
            .hijo_si(iva)
            .hijo(
                Elemento::new("cac:Item")
                    .hijo(Elemento::hoja("cbc:Description", l.descripcion.clone()))
                    .hijo(
                        Elemento::new("cac:StandardItemIdentification").hijo(
                            Elemento::hoja("cbc:ID", (i + 1).to_string())
                                .attr("schemeID", ESTANDAR_CONTRIBUYENTE),
                        ),
                    ),
            )
            .hijo(
                Elemento::new("cac:Price")
                    .hijo(con_moneda("cbc:PriceAmount", l.valor_unitario))
                    .hijo(
                        Elemento::hoja("cbc:BaseQuantity", format!("{:.6}", l.cantidad))
                            .attr("unitCode", &l.unidad),
                    ),
            )
    });
    let (forma, vence_pago) = match d.forma_pago {
        FormaPago::Contado => ("1", None),
        FormaPago::Credito { vence } => ("2", Some(vence)),
    };
    doc = doc
        .hijo(vendedor)
        .hijo(adquirente)
        .hijo(
            Elemento::new("cac:PaymentMeans")
                .hijo(Elemento::hoja("cbc:ID", forma))
                .hijo(Elemento::hoja("cbc:PaymentMeansCode", d.medio_pago.clone()))
                .hijo_si(vence_pago.map(|v| Elemento::hoja("cbc:PaymentDueDate", v.to_string()))),
        )
        .hijo_si(impuestos)
        .hijo(
            Elemento::new("cac:LegalMonetaryTotal")
                .hijo(con_moneda("cbc:LineExtensionAmount", t.bruto))
                .hijo(con_moneda("cbc:TaxExclusiveAmount", base_gravable))
                .hijo(con_moneda("cbc:TaxInclusiveAmount", total))
                .hijo(con_moneda("cbc:PayableAmount", total)),
        )
        .hijos(lineas);
    DocumentoGenerado {
        raiz: doc,
        codigo,
        numero,
        valor_bruto: t.bruto,
        iva: t.iva,
        total,
        qr,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::propio::xml::documento;
    use std::str::FromStr;

    pub(crate) fn dec(texto: &str) -> Decimal {
        Decimal::from_str(texto).expect("decimal válido")
    }

    pub(crate) fn software() -> Software {
        Software {
            nit_proveedor: "901713998".into(),
            dv_proveedor: "3".into(),
            id: "56f2ae4e-9812-4fad-9255-08fcfcd5ccb0".into(),
            pin: "12345".into(),
        }
    }

    fn bogota() -> Direccion {
        Direccion {
            municipio: "11001".into(),
            ciudad: "Bogotá, D.C.".into(),
            departamento: "Bogotá".into(),
            codigo_departamento: "11".into(),
            linea: "Calle 1 # 2-3".into(),
        }
    }

    pub(crate) fn factura() -> DocumentoUbl {
        DocumentoUbl {
            clase: Clase::Factura {
                resolucion: Resolucion {
                    numero: "18760000001".into(),
                    vigente_desde: NaiveDate::from_ymd_opt(2026, 1, 1).expect("fecha"),
                    vigente_hasta: NaiveDate::from_ymd_opt(2030, 12, 31).expect("fecha"),
                    prefijo: "SETP".into(),
                    desde: 990_000_000,
                    hasta: 995_000_000,
                    clave_tecnica: "fc8eac422eba16e22ffd8c6f94b3f40a6e38162c".into(),
                },
                contingencia_dian: false,
                contingencia_facturador: false,
            },
            prefijo: "SETP".into(),
            numero: 990_000_001,
            emitido: DateTime::parse_from_rfc3339("2027-01-15T10:30:00-05:00").expect("fecha"),
            ambiente: Ambiente::Pruebas,
            emisor: Parte {
                tipo_persona: TipoPersona::Juridica,
                tipo_documento: TIPO_DOCUMENTO_NIT.into(),
                documento: "901713998".into(),
                dv: Some("3".into()),
                nombre: "GISLA SAS".into(),
                regimen: RegimenIva::Responsable,
                responsabilidades: vec!["O-13".into(), "O-15".into()],
                direccion: Some(bogota()),
                correo: Some("facturas@ejemplo.co".into()),
            },
            adquirente: Parte {
                tipo_persona: TipoPersona::Natural,
                tipo_documento: "13".into(),
                documento: "79000000".into(),
                dv: None,
                nombre: "Cliente & Cía".into(),
                regimen: RegimenIva::NoResponsable,
                responsabilidades: vec![],
                direccion: None,
                correo: Some("cliente@ejemplo.co".into()),
            },
            lineas: vec![
                Linea {
                    descripcion: "Alquiler del salón comunal".into(),
                    cantidad: dec("1"),
                    unidad: "94".into(),
                    valor_unitario: dec("100000"),
                    subtotal: dec("100000"),
                    iva: Some((dec("19"), dec("19000"))),
                },
                Linea {
                    descripcion: "Parqueadero de visitantes".into(),
                    cantidad: dec("2"),
                    unidad: "94".into(),
                    valor_unitario: dec("5000.50"),
                    subtotal: dec("10001"),
                    iva: None,
                },
            ],
            forma_pago: FormaPago::Contado,
            medio_pago: "10".into(),
        }
    }

    fn texto(g: &DocumentoGenerado) -> String {
        String::from_utf8(documento(&g.raiz)).expect("utf-8")
    }

    /// Documento soporte: el vendedor no obligado (persona natural con dirección) va en
    /// `emisor` y la entidad que compra, en `adquirente`.
    pub(crate) fn documento_soporte() -> DocumentoUbl {
        let f = factura();
        let mut vendedor = f.adquirente.clone();
        vendedor.direccion = Some(bogota());
        DocumentoUbl {
            clase: Clase::DocumentoSoporte {
                resolucion: Resolucion {
                    prefijo: "DS".into(),
                    desde: 1,
                    hasta: 1000,
                    clave_tecnica: String::new(),
                    ..match f.clase {
                        Clase::Factura { resolucion, .. } => resolucion,
                        _ => unreachable!("la factura de prueba trae resolución"),
                    }
                },
                residente: true,
            },
            prefijo: "DS".into(),
            numero: 1,
            emisor: vendedor,
            adquirente: f.emisor.clone(),
            ..f
        }
    }

    #[test]
    fn el_documento_soporte_lleva_cuds_vendedor_como_proveedor_y_periodo_por_operacion() {
        let g = generar(&documento_soporte(), &software()).expect("documento soporte");
        let xml = texto(&g);

        assert_eq!(g.numero, "DS1");
        assert!(xml.contains("<cbc:InvoiceTypeCode>05</cbc:InvoiceTypeCode>"));
        assert!(xml.contains("<cbc:CustomizationID>10</cbc:CustomizationID>"));
        assert!(xml.contains("documento soporte en adquisiciones efectuadas a no obligados"));
        assert!(xml.contains(&format!("schemeName=\"CUDS-SHA384\">{}<", g.codigo)));
        assert!(xml.contains("<cbc:DescriptionCode>1</cbc:DescriptionCode>"));
        assert!(
            xml.contains("schemeID=\"999\""),
            "identificación estándar del ítem"
        );
        let proveedor = &xml[xml
            .find("<cac:AccountingSupplierParty>")
            .expect("proveedor")
            ..xml.find("</cac:AccountingSupplierParty>").expect("fin")];
        assert!(proveedor.contains("79000000") && proveedor.contains("<cac:PhysicalLocation>"));
        assert!(xml.contains("searchqr?documentkey="), "QR con el CUDS");
    }

    #[test]
    fn la_nota_de_ajuste_referencia_el_cuds_del_documento_soporte() {
        let ds = documento_soporte();
        let nota = DocumentoUbl {
            clase: Clase::NotaAjusteSoporte {
                referencia: Referencia {
                    numero: "DS1".into(),
                    cufe: "d".repeat(96),
                    fecha: ds.emitido.date_naive(),
                },
                correccion: Correccion {
                    concepto: "2".into(),
                    descripcion: "Anulación del documento soporte".into(),
                },
            },
            prefijo: "NA27".into(),
            numero: 1,
            ..ds
        };
        let xml = texto(&generar(&nota, &software()).expect("nota de ajuste"));

        assert!(xml.contains("<CreditNote"));
        assert!(xml.contains("<cbc:CreditNoteTypeCode>95</cbc:CreditNoteTypeCode>"));
        assert!(xml.contains(&format!("schemeName=\"CUDS-SHA384\">{}<", "d".repeat(96))));
        assert!(!xml.contains("InvoicePeriod"), "la nota no lleva periodo");
    }

    #[test]
    fn un_documento_soporte_sin_direccion_del_vendedor_no_se_genera() {
        let mut ds = documento_soporte();
        ds.emisor.direccion = None;
        assert!(generar(&ds, &software()).is_err());
    }

    #[test]
    fn factura_calcula_totales_y_cufe_con_la_clave_tecnica() {
        let d = factura();
        let g = generar(&d, &software()).expect("factura");
        assert_eq!(g.numero, "SETP990000001");
        assert_eq!(g.valor_bruto, dec("110001"));
        assert_eq!(g.iva, dec("19000"));
        assert_eq!(g.total, dec("129001"));
        let esperado = cufe(
            &DatosControl {
                numero: "SETP990000001",
                fecha: NaiveDate::from_ymd_opt(2027, 1, 15).expect("fecha"),
                hora: "10:30:00-05:00",
                valor_bruto: dec("110001"),
                iva: dec("19000"),
                inc: Decimal::ZERO,
                ica: Decimal::ZERO,
                total: dec("129001"),
                nit_emisor: "901713998",
                documento_adquirente: "79000000",
                ambiente: Ambiente::Pruebas,
            },
            "fc8eac422eba16e22ffd8c6f94b3f40a6e38162c",
        );
        assert_eq!(g.codigo, esperado);
        let xml = texto(&g);
        assert!(xml.contains(&format!(
            "<cbc:UUID schemeID=\"2\" schemeName=\"CUFE-SHA384\">{esperado}</cbc:UUID>"
        )));
        assert!(xml.contains("<cbc:InvoiceTypeCode>01</cbc:InvoiceTypeCode>"));
        assert!(xml.contains("<cbc:TaxLevelCode listName=\"48\">O-13;O-15</cbc:TaxLevelCode>"));
        assert!(
            xml.contains("<cbc:PayableAmount currencyID=\"COP\">129001.00</cbc:PayableAmount>")
        );
        assert!(xml.contains(
            "<cbc:TaxExclusiveAmount currencyID=\"COP\">100000.00</cbc:TaxExclusiveAmount>"
        ));
        assert!(xml.contains("Cliente &amp; Cía"));
    }

    #[test]
    fn el_xml_no_lleva_el_pin_ni_la_clave_tecnica() {
        let g = generar(&factura(), &software()).expect("factura");
        let xml = texto(&g);
        assert!(!xml.contains("fc8eac422eba16e22ffd8c6f94b3f40a6e38162c"));
        assert!(!xml.contains(">12345<"));
        assert!(xml.contains(&codigo_seguridad_software(
            "56f2ae4e-9812-4fad-9255-08fcfcd5ccb0",
            "12345",
            "SETP990000001"
        )));
        assert!(format!("{:?}", software()).contains("<oculto>"));
    }

    #[test]
    fn deja_una_extension_vacia_para_la_firma() {
        let g = generar(&factura(), &software()).expect("factura");
        let xml = texto(&g);
        assert!(xml.contains("<ext:UBLExtension><ext:ExtensionContent></ext:ExtensionContent></ext:UBLExtension></ext:UBLExtensions>"));
    }

    #[test]
    fn nota_credito_usa_cude_con_el_pin_y_referencia_la_factura() {
        let mut d = factura();
        d.clase = Clase::NotaCredito {
            referencia: Referencia {
                numero: "SETP990000001".into(),
                cufe: "abc".into(),
                fecha: NaiveDate::from_ymd_opt(2027, 1, 15).expect("fecha"),
            },
            correccion: Correccion {
                concepto: "2".into(),
                descripcion: "Anulación de la factura".into(),
            },
        };
        d.prefijo = "NC".into();
        d.numero = 1;
        let g = generar(&d, &software()).expect("nota");
        let xml = texto(&g);
        assert!(xml.starts_with(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?><CreditNote "
        ));
        assert!(xml.contains("schemeName=\"CUDE-SHA384\""));
        assert!(xml.contains("<cbc:CreditNoteTypeCode>91</cbc:CreditNoteTypeCode>"));
        assert!(xml.contains("<cbc:ReferenceID>SETP990000001</cbc:ReferenceID>"));
        assert!(!xml.contains("sts:InvoiceControl"));
        assert!(xml.contains("<cac:CreditNoteLine>"));
        assert_eq!(g.codigo.len(), 96);
    }

    #[test]
    fn nota_debito_usa_requested_monetary_total() {
        let mut d = factura();
        d.clase = Clase::NotaDebito {
            referencia: Referencia {
                numero: "SETP990000001".into(),
                cufe: "abc".into(),
                fecha: NaiveDate::from_ymd_opt(2027, 1, 15).expect("fecha"),
            },
            correccion: Correccion {
                concepto: "3".into(),
                descripcion: "Cambio del valor".into(),
            },
        };
        let xml = texto(&generar(&d, &software()).expect("nota"));
        assert!(xml.contains("<cac:RequestedMonetaryTotal>"));
        assert!(xml.contains("<cbc:DebitedQuantity unitCode=\"94\">"));
        assert!(!xml.contains("CreditNoteTypeCode"));
    }

    #[test]
    fn la_transcripcion_de_contingencia_es_tipo_03_con_cude_y_referencia_al_talonario() {
        let mut d = factura();
        if let Clase::Factura {
            contingencia_facturador,
            resolucion,
            ..
        } = &mut d.clase
        {
            *contingencia_facturador = true;
            resolucion.clave_tecnica = String::new();
        }
        let g = generar(&d, &software()).expect("transcripción");
        let xml = texto(&g);

        assert!(xml.contains("<cbc:InvoiceTypeCode>03</cbc:InvoiceTypeCode>"));
        assert!(xml.contains(&format!("schemeName=\"CUDE-SHA384\">{}<", g.codigo)));
        assert!(xml.contains("<cac:AdditionalDocumentReference>"));
        assert!(xml.contains("<cbc:DocumentTypeCode>FTC</cbc:DocumentTypeCode>"));
    }

    #[test]
    fn contingencia_dian_es_tipo_04_con_el_mismo_numero() {
        let mut d = factura();
        if let Clase::Factura {
            contingencia_dian, ..
        } = &mut d.clase
        {
            *contingencia_dian = true;
        }
        let normal = generar(&factura(), &software()).expect("factura");
        let contingencia = generar(&d, &software()).expect("contingencia");
        assert!(texto(&contingencia).contains("<cbc:InvoiceTypeCode>04</cbc:InvoiceTypeCode>"));
        assert_eq!(normal.numero, contingencia.numero);
        assert_eq!(normal.codigo, contingencia.codigo);
    }

    #[test]
    fn credito_lleva_vencimiento() {
        let mut d = factura();
        d.forma_pago = FormaPago::Credito {
            vence: NaiveDate::from_ymd_opt(2027, 2, 15).expect("fecha"),
        };
        let xml = texto(&generar(&d, &software()).expect("factura"));
        assert!(xml.contains("<cbc:DueDate>2027-02-15</cbc:DueDate>"));
        assert!(xml.contains("<cbc:ID>2</cbc:ID><cbc:PaymentMeansCode>10</cbc:PaymentMeansCode><cbc:PaymentDueDate>2027-02-15</cbc:PaymentDueDate>"));
    }

    #[test]
    fn rechaza_numero_fuera_del_rango() {
        let mut d = factura();
        d.numero = 1;
        assert!(matches!(
            generar(&d, &software()),
            Err(ErrorMotor::Datos(_))
        ));
    }

    #[test]
    fn rechaza_resolucion_vencida() {
        let mut d = factura();
        d.emitido = DateTime::parse_from_rfc3339("2031-01-15T10:30:00-05:00").expect("fecha");
        assert!(matches!(
            generar(&d, &software()),
            Err(ErrorMotor::Datos(_))
        ));
    }

    #[test]
    fn rechaza_sin_lineas_y_nit_sin_dv() {
        let mut d = factura();
        d.lineas.clear();
        assert!(matches!(
            generar(&d, &software()),
            Err(ErrorMotor::Datos(_))
        ));
        let mut d = factura();
        d.adquirente.tipo_documento = TIPO_DOCUMENTO_NIT.into();
        assert!(matches!(
            generar(&d, &software()),
            Err(ErrorMotor::Datos(_))
        ));
    }
}
