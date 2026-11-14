//! Proveedor de facturación simulado: no transmite a la DIAN.

use application::{
    AppError, AppResult,
    puertos::tributario::{DocumentoAEmitir, ProveedorFacturacion, RespuestaProveedor},
};
use domain::contabilidad::tributario::{EstadoDocumentoElectronico, EventoRadian};
use sha2::{Digest, Sha384};

/// Proveedor simulado. Determinista: el mismo documento produce siempre el mismo CUFE, así que
/// reintentar es idempotente igual que con un proveedor real.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProveedorSimulado {
    caido: bool,
}

impl ProveedorSimulado {
    /// Proveedor que responde.
    pub const fn new() -> Self {
        Self { caido: false }
    }

    /// Proveedor que no responde (toda llamada falla como infraestructura): sirve para probar la
    /// contingencia.
    pub const fn caido() -> Self {
        Self { caido: true }
    }

    fn disponible(self) -> AppResult<()> {
        if self.caido {
            Err(AppError::Infraestructura(
                "proveedor de facturación simulado sin servicio".into(),
            ))
        } else {
            Ok(())
        }
    }
}

fn sha384_hex(texto: &str) -> String {
    hex::encode(Sha384::digest(texto.as_bytes()))
}

/// CUFE simulado: SHA-384 (96 hexadecimales, como el de la DIAN) de los datos que lo componen.
fn cufe(d: &DocumentoAEmitir) -> String {
    sha384_hex(&format!(
        "{}{}|{}|{}|{}|{}|{}|{}|{}|{}",
        d.prefijo,
        d.numero,
        d.fecha,
        d.subtotal,
        d.iva,
        d.total,
        d.emisor.documento,
        d.adquirente.documento,
        d.clave_tecnica.as_deref().unwrap_or_default(),
        d.id
    ))
}

fn escapar(texto: &str) -> String {
    texto
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// XML de muestra (no es un UBL firmado: lo deja claro en una nota).
fn xml(d: &DocumentoAEmitir, cufe: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <DocumentoSimulado tipo=\"{}\">\n  \
         <Nota>Documento de prueba sin validez ante la DIAN</Nota>\n  \
         <ID>{}{}</ID>\n  <CUFE>{}</CUFE>\n  <Fecha>{}</Fecha>\n  \
         <Emisor documento=\"{}\">{}</Emisor>\n  <Adquirente documento=\"{}\">{}</Adquirente>\n  \
         <Subtotal>{}</Subtotal>\n  <IVA>{}</IVA>\n  <Total>{}</Total>\n\
         </DocumentoSimulado>\n",
        d.tipo.as_str(),
        escapar(&d.prefijo),
        d.numero,
        cufe,
        d.fecha,
        escapar(&d.emisor.documento),
        escapar(&d.emisor.nombre),
        escapar(&d.adquirente.documento),
        escapar(&d.adquirente.nombre),
        d.subtotal,
        d.iva,
        d.total
    )
    .into_bytes()
}

fn rechazo(mensaje: &str) -> RespuestaProveedor {
    RespuestaProveedor {
        estado: EstadoDocumentoElectronico::Rechazado,
        id_externo: None,
        cufe: None,
        xml: None,
        mensaje: Some(mensaje.into()),
    }
}

impl ProveedorFacturacion for ProveedorSimulado {
    fn nombre(&self) -> &'static str {
        "simulado"
    }

    async fn emitir(&self, d: &DocumentoAEmitir) -> AppResult<RespuestaProveedor> {
        self.disponible()?;
        // Las mismas reglas mínimas que rechazaría la DIAN (FAK, FAJ): sin documento de la
        // contraparte o sin líneas el documento no es válido.
        if d.adquirente.documento.trim().is_empty() {
            return Ok(rechazo(
                "la contraparte no tiene documento de identificación",
            ));
        }
        if d.emisor.documento.trim().is_empty() {
            return Ok(rechazo("el emisor no tiene NIT"));
        }
        if d.lineas.is_empty() {
            return Ok(rechazo("el documento no tiene líneas"));
        }
        let cufe = cufe(d);
        Ok(RespuestaProveedor {
            estado: EstadoDocumentoElectronico::Validado,
            id_externo: Some(format!("sim-{}", d.id)),
            xml: Some(xml(d, &cufe)),
            cufe: Some(cufe),
            mensaje: Some("validado por el proveedor simulado".into()),
        })
    }

    async fn consultar(&self, id_externo: &str) -> AppResult<RespuestaProveedor> {
        self.disponible()?;
        Ok(RespuestaProveedor {
            estado: EstadoDocumentoElectronico::Validado,
            id_externo: Some(id_externo.into()),
            cufe: Some(sha384_hex(id_externo)),
            xml: None,
            mensaje: Some("validado por el proveedor simulado".into()),
        })
    }

    async fn evento_radian(
        &self,
        cufe: &str,
        evento: EventoRadian,
        _motivo: Option<&str>,
    ) -> AppResult<RespuestaProveedor> {
        self.disponible()?;
        let id = sha384_hex(&format!("{cufe}|{}", evento.as_str()));
        Ok(RespuestaProveedor {
            estado: EstadoDocumentoElectronico::Validado,
            id_externo: Some(format!("sim-radian-{}", &id[..32])),
            cufe: None,
            xml: None,
            mensaje: Some(format!("evento {} registrado (simulado)", evento.as_str())),
        })
    }
}

#[cfg(test)]
mod tests {
    use application::puertos::tributario::{LineaDocumento, ParteDocumento};
    use chrono::NaiveDate;
    use domain::{DocumentoElectronicoId, contabilidad::tributario::TipoDocumentoElectronico};
    use rust_decimal::Decimal;

    use super::*;

    fn parte(documento: &str) -> ParteDocumento {
        ParteDocumento {
            documento: documento.into(),
            dv: None,
            nombre: "Parte <S.A.S.>".into(),
            correo: None,
        }
    }

    fn documento(adquirente: &str) -> DocumentoAEmitir {
        DocumentoAEmitir {
            id: DocumentoElectronicoId::nuevo(),
            tipo: TipoDocumentoElectronico::Factura,
            prefijo: "SETP".into(),
            numero: 990_000_001,
            fecha: NaiveDate::from_ymd_opt(2026, 3, 1).expect("fecha válida"),
            emisor: parte("900123456"),
            adquirente: parte(adquirente),
            lineas: vec![LineaDocumento {
                descripcion: "Alquiler".into(),
                cantidad: Decimal::ONE,
                valor_unitario: Decimal::from(100_000),
                subtotal: Decimal::from(100_000),
                tarifa_iva: Decimal::from(19),
                iva: Decimal::from(19_000),
            }],
            subtotal: Decimal::from(100_000),
            iva: Decimal::from(19_000),
            total: Decimal::from(119_000),
            clave_tecnica: Some("clave".into()),
            referencia_cufe: None,
        }
    }

    #[tokio::test]
    async fn valida_con_un_cufe_de_96_hexadecimales_y_es_idempotente() {
        let d = documento("800111222");
        let p = ProveedorSimulado::new();
        let a = p.emitir(&d).await.expect("respuesta");
        let b = p.emitir(&d).await.expect("respuesta");
        assert_eq!(a.estado, EstadoDocumentoElectronico::Validado);
        let cufe = a.cufe.clone().expect("cufe");
        assert_eq!(cufe.len(), 96);
        assert!(
            cufe.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn el_xml_escapa_los_nombres() {
        let r = ProveedorSimulado::new()
            .emitir(&documento("800111222"))
            .await
            .expect("respuesta");
        let xml = String::from_utf8(r.xml.expect("xml")).expect("utf-8");
        assert!(xml.contains("Parte &lt;S.A.S.&gt;"));
        assert!(roxmltree::Document::parse(&xml).is_ok());
    }

    #[tokio::test]
    async fn rechaza_sin_documento_del_adquirente() {
        let r = ProveedorSimulado::new()
            .emitir(&documento(" "))
            .await
            .expect("respuesta");
        assert_eq!(r.estado, EstadoDocumentoElectronico::Rechazado);
        assert!(r.cufe.is_none());
    }

    #[tokio::test]
    async fn caido_falla_como_infraestructura() {
        let r = ProveedorSimulado::caido()
            .emitir(&documento("800111222"))
            .await;
        assert!(matches!(r, Err(AppError::Infraestructura(_))));
    }

    #[tokio::test]
    async fn el_evento_radian_queda_registrado() {
        let r = ProveedorSimulado::new()
            .evento_radian("abc", EventoRadian::Acuse, None)
            .await
            .expect("respuesta");
        assert_eq!(r.estado, EstadoDocumentoElectronico::Validado);
        assert!(r.id_externo.expect("id").starts_with("sim-radian-"));
    }
}
