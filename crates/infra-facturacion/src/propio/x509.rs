//! Lectura mínima de certificados X.509 (DER) para la firma XAdES.
//!
//! La firma necesita, de cada certificado de la cadena: su DER (para el digest), el emisor en
//! formato RFC 2253 y el número de serie en decimal (`xades:IssuerSerial`), la vigencia (la DIAN
//! rechaza una firma hecha fuera de ella) y el uso de la clave (firma digital y no repudio,
//! anexo §10.14). No valida la cadena de confianza ni la revocación: eso lo hace la DIAN y queda
//! como decisión abierta del plan 17.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};

use super::ErrorMotor;

/// Datos de un certificado que usa la firma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Certificado {
    pub(crate) der: Vec<u8>,
    /// Número de serie en decimal.
    pub(crate) serial: String,
    /// Emisor en formato RFC 2253 (`C=CO,O=…,CN=…`).
    pub(crate) emisor: String,
    pub(crate) vigente_desde: DateTime<Utc>,
    pub(crate) vigente_hasta: DateTime<Utc>,
    /// Bits de `keyUsage` (bit 0 = firma digital, bit 1 = no repudio), si el certificado los
    /// declara.
    uso_clave: Option<[bool; 2]>,
}

impl Certificado {
    /// Lee un certificado DER.
    ///
    /// # Errors
    ///
    /// [`ErrorMotor::Certificado`] si el DER no es un certificado X.509 legible.
    pub(crate) fn leer(der: &[u8]) -> Result<Self, ErrorMotor> {
        leer(der).ok_or_else(|| ErrorMotor::Certificado("el certificado no es X.509 válido".into()))
    }

    /// Si la clave sirve para firmar documentos (firma digital y no repudio). Un certificado que
    /// no declara `keyUsage` no tiene restricciones.
    pub(crate) fn permite_firmar(&self) -> bool {
        self.uso_clave
            .is_none_or(|[firma, no_repudio]| firma && no_repudio)
    }

    pub(crate) fn vigente_en(&self, momento: DateTime<Utc>) -> bool {
        self.vigente_desde <= momento && momento <= self.vigente_hasta
    }
}

/// Lector de TLV (etiqueta, longitud, valor) de DER.
struct Lector<'a> {
    datos: &'a [u8],
}

impl<'a> Lector<'a> {
    fn new(datos: &'a [u8]) -> Self {
        Self { datos }
    }

    fn vacio(&self) -> bool {
        self.datos.is_empty()
    }

    fn mirar(&self) -> Option<u8> {
        self.datos.first().copied()
    }

    /// Lee el siguiente TLV: devuelve la etiqueta, el valor y el TLV completo.
    fn siguiente(&mut self) -> Option<(u8, &'a [u8], &'a [u8])> {
        let etiqueta = *self.datos.first()?;
        let primero = *self.datos.get(1)?;
        let (largo, cabecera) = if primero < 0x80 {
            (usize::from(primero), 2)
        } else {
            let n = usize::from(primero & 0x7f);
            if n == 0 || n > 4 {
                return None;
            }
            let mut largo = 0usize;
            for b in self.datos.get(2..2 + n)? {
                largo = (largo << 8) | usize::from(*b);
            }
            (largo, 2 + n)
        };
        let fin = cabecera.checked_add(largo)?;
        let completo = self.datos.get(..fin)?;
        let valor = completo.get(cabecera..)?;
        self.datos = self.datos.get(fin..)?;
        Some((etiqueta, valor, completo))
    }

    fn esperar(&mut self, etiqueta: u8) -> Option<&'a [u8]> {
        match self.siguiente()? {
            (e, valor, _) if e == etiqueta => Some(valor),
            _ => None,
        }
    }
}

const SECUENCIA: u8 = 0x30;
const CONJUNTO: u8 = 0x31;
const ENTERO: u8 = 0x02;
const OID: u8 = 0x06;

fn leer(der: &[u8]) -> Option<Certificado> {
    let mut certificado = Lector::new(Lector::new(der).esperar(SECUENCIA)?);
    let mut tbs = Lector::new(certificado.esperar(SECUENCIA)?);
    if tbs.mirar() == Some(0xa0) {
        tbs.siguiente()?; // versión
    }
    let serial = decimal(tbs.esperar(ENTERO)?);
    tbs.esperar(SECUENCIA)?; // algoritmo de firma
    let emisor = nombre_rfc2253(tbs.esperar(SECUENCIA)?)?;
    let mut vigencia = Lector::new(tbs.esperar(SECUENCIA)?);
    let vigente_desde = tiempo(vigencia.siguiente()?)?;
    let vigente_hasta = tiempo(vigencia.siguiente()?)?;
    tbs.esperar(SECUENCIA)?; // sujeto
    tbs.esperar(SECUENCIA)?; // clave pública
    let mut uso_clave = None;
    while !tbs.vacio() {
        let (etiqueta, valor, _) = tbs.siguiente()?;
        if etiqueta == 0xa3 {
            uso_clave = uso_de_clave(valor)?;
        }
    }
    Some(Certificado {
        der: der.to_vec(),
        serial,
        emisor,
        vigente_desde,
        vigente_hasta,
        uso_clave,
    })
}

/// Entero sin signo big-endian a decimal.
fn decimal(bytes: &[u8]) -> String {
    let mut numero: Vec<u8> = bytes.iter().copied().skip_while(|b| *b == 0).collect();
    if numero.is_empty() {
        return "0".into();
    }
    let mut digitos = Vec::new();
    while !numero.is_empty() {
        let mut resto = 0u32;
        let mut cociente = Vec::with_capacity(numero.len());
        for b in &numero {
            let actual = (resto << 8) | u32::from(*b);
            let q = actual / 10;
            resto = actual % 10;
            if !(cociente.is_empty() && q == 0) {
                cociente.push(u8::try_from(q).unwrap_or(0));
            }
        }
        digitos.push(char::from(b'0' + u8::try_from(resto).unwrap_or(0)));
        numero = cociente;
    }
    digitos.iter().rev().collect()
}

fn oid(bytes: &[u8]) -> Option<String> {
    let (primero, resto) = bytes.split_first()?;
    let mut partes = vec![u64::from(primero / 40), u64::from(primero % 40)];
    let mut actual = 0u64;
    for b in resto {
        actual = (actual << 7) | u64::from(b & 0x7f);
        if b & 0x80 == 0 {
            partes.push(actual);
            actual = 0;
        }
    }
    Some(
        partes
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join("."),
    )
}

/// Nombre corto RFC 2253 de los atributos que el estándar reconoce; los demás van como OID.
fn nombre_corto(oid: &str) -> Option<&'static str> {
    Some(match oid {
        "2.5.4.3" => "CN",
        "2.5.4.6" => "C",
        "2.5.4.7" => "L",
        "2.5.4.8" => "ST",
        "2.5.4.9" => "STREET",
        "2.5.4.10" => "O",
        "2.5.4.11" => "OU",
        "0.9.2342.19200300.100.1.25" => "DC",
        "0.9.2342.19200300.100.1.1" => "UID",
        _ => return None,
    })
}

/// Valor de cadena de un atributo (UTF8, Printable, IA5, Teletex/Latin-1 o BMP).
fn texto(etiqueta: u8, valor: &[u8]) -> Option<String> {
    match etiqueta {
        0x0c | 0x13 | 0x16 => String::from_utf8(valor.to_vec()).ok(),
        0x14 => Some(valor.iter().map(|b| char::from(*b)).collect()),
        0x1e => {
            let unidades: Vec<u16> = valor
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect();
            String::from_utf16(&unidades).ok()
        }
        _ => None,
    }
}

fn escapar_rfc2253(valor: &str) -> String {
    let mut salida = String::new();
    let total = valor.chars().count();
    for (i, c) in valor.chars().enumerate() {
        let especial = matches!(c, ',' | '+' | '"' | '\\' | '<' | '>' | ';')
            || (i == 0 && (c == '#' || c == ' '))
            || (i + 1 == total && c == ' ');
        if especial {
            salida.push('\\');
        }
        salida.push(c);
    }
    salida
}

/// `Name` X.509 en formato RFC 2253: los RDN en orden inverso, separados por comas.
fn nombre_rfc2253(secuencia: &[u8]) -> Option<String> {
    let mut rdns = Vec::new();
    let mut lector = Lector::new(secuencia);
    while !lector.vacio() {
        let mut conjunto = Lector::new(lector.esperar(CONJUNTO)?);
        let mut avas = Vec::new();
        while !conjunto.vacio() {
            let mut ava = Lector::new(conjunto.esperar(SECUENCIA)?);
            let tipo = oid(ava.esperar(OID)?)?;
            let (etiqueta, valor, completo) = ava.siguiente()?;
            avas.push(match (nombre_corto(&tipo), texto(etiqueta, valor)) {
                (Some(corto), Some(t)) => format!("{corto}={}", escapar_rfc2253(&t)),
                _ => format!("{tipo}=#{}", hex::encode(completo)),
            });
        }
        rdns.push(avas.join("+"));
    }
    rdns.reverse();
    Some(rdns.join(","))
}

/// `UTCTime` (`AAMMDDhhmmssZ`) o `GeneralizedTime` (`AAAAMMDDhhmmssZ`).
fn tiempo((etiqueta, valor, _): (u8, &[u8], &[u8])) -> Option<DateTime<Utc>> {
    let texto = std::str::from_utf8(valor).ok()?.strip_suffix('Z')?;
    let completo = match etiqueta {
        0x17 => {
            let anio: u32 = texto.get(..2)?.parse().ok()?;
            let siglo = if anio < 50 { "20" } else { "19" };
            format!("{siglo}{texto}")
        }
        0x18 => texto.to_owned(),
        _ => return None,
    };
    let fecha = NaiveDate::parse_from_str(completo.get(..8)?, "%Y%m%d").ok()?;
    let hora = chrono::NaiveTime::parse_from_str(completo.get(8..14)?, "%H%M%S").ok()?;
    Some(NaiveDateTime::new(fecha, hora).and_utc())
}

/// Busca `keyUsage` (2.5.29.15) en las extensiones (`[3] EXPLICIT`).
fn uso_de_clave(explicito: &[u8]) -> Option<Option<[bool; 2]>> {
    let mut extensiones = Lector::new(Lector::new(explicito).esperar(SECUENCIA)?);
    while !extensiones.vacio() {
        let mut extension = Lector::new(extensiones.esperar(SECUENCIA)?);
        let tipo = oid(extension.esperar(OID)?)?;
        if extension.mirar() == Some(0x01) {
            extension.siguiente()?; // crítica
        }
        let contenido = extension.esperar(0x04)?;
        if tipo == "2.5.29.15" {
            let bits = Lector::new(contenido).esperar(0x03)?;
            let primero = bits.get(1).copied().unwrap_or(0);
            return Some(Some([primero & 0x80 != 0, primero & 0x40 != 0]));
        }
    }
    Some(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_convierte_enteros_grandes() {
        assert_eq!(decimal(&[0x00]), "0");
        assert_eq!(decimal(&[0x01, 0x00]), "256");
        // 0x6C0B0762626DA0E2 = 7785324499979575522 (serial del ejemplo de la DIAN).
        assert_eq!(
            decimal(&[0x6c, 0x0b, 0x07, 0x62, 0x62, 0x6d, 0xa0, 0xe2]),
            "7785324499979575522"
        );
    }

    #[test]
    fn oid_se_decodifica_en_notacion_de_puntos() {
        assert_eq!(oid(&[0x55, 0x04, 0x03]).as_deref(), Some("2.5.4.3"));
        assert_eq!(
            oid(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x01]).as_deref(),
            Some("1.2.840.113549.1.9.1")
        );
    }

    #[test]
    fn rfc2253_escapa_caracteres_especiales() {
        assert_eq!(escapar_rfc2253("Andes, SCD"), "Andes\\, SCD");
        assert_eq!(escapar_rfc2253("#raro "), "\\#raro\\ ");
    }

    #[test]
    fn der_invalido_es_error() {
        assert!(Certificado::leer(&[0x30, 0x03, 0x02]).is_err());
    }
}
