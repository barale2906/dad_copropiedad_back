//! Adaptador del motor propio a los puertos de la aplicación (plan 18 fase B3):
//! [`MotorPropio`] transmite con las credenciales de cada entidad (puerto
//! [`HabilitacionDian`] y la transmisión de [`ProveedorDian`]), [`LectorPkcs12`] valida el
//! certificado que carga la entidad, y [`ProveedorDian`] elige por documento entre el motor
//! propio (si trae credenciales) y el proveedor simulado.
//!
//! Traduce el documento de negocio a UBL 2.1, lo firma con el `.p12` de la entidad y lo envía al
//! servicio web del ambiente que corresponda. Un error de red o del servicio deja el documento en
//! contingencia (`AppError::Infraestructura`); un dato o un certificado que no sirven es un error
//! de validación.

use application::{
    AppError, AppResult,
    puertos::tributario::{
        CertificadoLeido, CredencialesFacturador, DocumentoAEmitir, EventoAEmitir,
        HabilitacionDian, LectorCertificados, ParteDocumento, PersonaRecibe, ProveedorFacturacion,
        RangoAutorizado, RespuestaProveedor,
    },
};
use std::time::Duration;

use chrono::{FixedOffset, TimeZone, Utc};
use domain::{
    TipoDocumento,
    facturacion::AmbienteDian,
    tributario::{EstadoDocumentoElectronico, TipoDocumentoElectronico},
};

use super::control::Ambiente;
use super::evento::{EventoUbl, PersonaEvento, preparar_evento};
use super::firma::CredencialFirma;
use super::servicio::{RespuestaDian, ServicioDian};
use super::ubl::{
    Clase, Correccion, Direccion, DocumentoUbl, FormaPago, Linea, Parte, Referencia, RegimenIva,
    Resolucion, Software, TIPO_DOCUMENTO_NIT, TipoPersona,
};
use super::{DocumentoFirmado, ErrorMotor, adjunto, preparar};
use crate::ProveedorSimulado;

/// Medio de pago «instrumento no definido» (tabla 13.3.4.2 del anexo).
const MEDIO_PAGO: &str = "1";
/// Largo máximo del mensaje que se guarda con el documento.
const LARGO_MENSAJE: usize = 4000;

fn a_app(e: ErrorMotor) -> AppError {
    match e {
        ErrorMotor::Transporte(_) | ErrorMotor::Respuesta(_) => AppError::infra(e),
        otro => AppError::Validacion(otro.to_string()),
    }
}

/// Zona horaria de Colombia (UTC−5, sin horario de verano).
fn colombia() -> FixedOffset {
    FixedOffset::west_opt(5 * 3600).unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC"))
}

fn parte(p: &ParteDocumento) -> Parte {
    let direccion = p.direccion.as_ref().map(|d| Direccion {
        codigo_departamento: d.municipio_dane.chars().take(2).collect(),
        departamento: d
            .departamento
            .clone()
            .unwrap_or_else(|| d.municipio_dane.chars().take(2).collect()),
        ciudad: d.municipio_dane.clone(),
        municipio: d.municipio_dane.clone(),
        linea: d.linea.clone(),
    });
    let responsabilidades = if p.responsabilidades.is_empty() {
        vec!["R-99-PN".into()]
    } else {
        p.responsabilidades.clone()
    };
    Parte {
        tipo_persona: if p.persona_natural {
            TipoPersona::Natural
        } else {
            TipoPersona::Juridica
        },
        tipo_documento: if p.persona_natural {
            "13".into()
        } else {
            TIPO_DOCUMENTO_NIT.into()
        },
        documento: p.documento.clone(),
        dv: p.dv.clone(),
        nombre: p.nombre.clone(),
        regimen: if p.responsable_iva {
            RegimenIva::Responsable
        } else {
            RegimenIva::NoResponsable
        },
        responsabilidades,
        direccion,
        correo: p.correo.clone(),
    }
}

fn ambiente(a: AmbienteDian) -> Ambiente {
    match a {
        AmbienteDian::Produccion => Ambiente::Produccion,
        AmbienteDian::Habilitacion => Ambiente::Pruebas,
    }
}

/// Documento de negocio → UBL.
fn a_ubl(
    d: &DocumentoAEmitir,
    credenciales: &CredencialesFacturador,
) -> Result<DocumentoUbl, ErrorMotor> {
    let datos = |t: &str| ErrorMotor::Datos(t.to_owned());
    let clase = match d.tipo {
        TipoDocumentoElectronico::Factura => {
            let r = d
                .resolucion
                .as_ref()
                .ok_or_else(|| datos("la factura necesita su resolución de numeración"))?;
            Clase::Factura {
                resolucion: Resolucion {
                    numero: r.numero.clone(),
                    vigente_desde: r.vigente_desde,
                    vigente_hasta: r.vigente_hasta,
                    prefijo: d.prefijo.clone(),
                    desde: r.desde,
                    hasta: r.hasta,
                    // La resolución de contingencia del facturador no tiene clave técnica: su
                    // código es un CUDE con el PIN.
                    clave_tecnica: if d.transcripcion_contingencia {
                        String::new()
                    } else {
                        d.clave_tecnica
                            .clone()
                            .ok_or_else(|| datos("falta la clave técnica de la resolución"))?
                    },
                },
                contingencia_dian: false,
                contingencia_facturador: d.transcripcion_contingencia,
            }
        }
        TipoDocumentoElectronico::NotaCredito | TipoDocumentoElectronico::NotaDebito => {
            let referencia = Referencia {
                numero: d
                    .referencia_numero
                    .clone()
                    .ok_or_else(|| datos("la nota necesita el documento al que se refiere"))?,
                cufe: d
                    .referencia_cufe
                    .clone()
                    .ok_or_else(|| datos("el documento referido no tiene CUFE"))?,
                fecha: d.referencia_fecha.unwrap_or(d.fecha),
            };
            // Conceptos de corrección del anexo: la nota crédito anula (2); la nota débito
            // cambia el valor (3).
            let correccion = if let Some((concepto, descripcion)) = &d.correccion {
                Correccion {
                    concepto: concepto.clone(),
                    descripcion: descripcion.clone(),
                }
            } else if d.tipo == TipoDocumentoElectronico::NotaCredito {
                Correccion {
                    concepto: "2".into(),
                    descripcion: "Anulación de la factura electrónica".into(),
                }
            } else {
                Correccion {
                    concepto: "3".into(),
                    descripcion: "Cambio del valor de la factura electrónica".into(),
                }
            };
            if d.tipo == TipoDocumentoElectronico::NotaCredito {
                Clase::NotaCredito {
                    referencia,
                    correccion,
                }
            } else {
                Clase::NotaDebito {
                    referencia,
                    correccion,
                }
            }
        }
        TipoDocumentoElectronico::DocumentoSoporte => {
            let r = d.resolucion.as_ref().ok_or_else(|| {
                datos("el documento soporte necesita su resolución de numeración")
            })?;
            Clase::DocumentoSoporte {
                resolucion: Resolucion {
                    numero: r.numero.clone(),
                    vigente_desde: r.vigente_desde,
                    vigente_hasta: r.vigente_hasta,
                    prefijo: d.prefijo.clone(),
                    desde: r.desde,
                    hasta: r.hasta,
                    // El CUDS usa el PIN del software, no la clave técnica.
                    clave_tecnica: d.clave_tecnica.clone().unwrap_or_default(),
                },
                residente: true,
            }
        }
        TipoDocumentoElectronico::NotaAjusteDs => Clase::NotaAjusteSoporte {
            referencia: Referencia {
                numero: d.referencia_numero.clone().ok_or_else(|| {
                    datos("la nota de ajuste necesita el documento soporte al que se refiere")
                })?,
                cufe: d
                    .referencia_cufe
                    .clone()
                    .ok_or_else(|| datos("el documento soporte referido no tiene CUDS"))?,
                fecha: d.referencia_fecha.unwrap_or(d.fecha),
            },
            correccion: Correccion {
                concepto: "2".into(),
                descripcion: "Anulación del documento soporte en adquisiciones a no obligados"
                    .into(),
            },
        },
    };
    // En el documento soporte el vendedor no obligado va como proveedor y la entidad que compra
    // (quien emite) como adquirente.
    let (emisor, adquirente) = if clase.es_soporte() {
        (parte(&d.adquirente), parte(&d.emisor))
    } else {
        (parte(&d.emisor), parte(&d.adquirente))
    };
    let ahora = Utc::now().with_timezone(&colombia());
    let emitido = colombia()
        .from_local_datetime(&d.fecha.and_time(ahora.time()))
        .single()
        .ok_or_else(|| datos("fecha de emisión inválida"))?;
    Ok(DocumentoUbl {
        clase,
        prefijo: d.prefijo.clone(),
        numero: d.numero,
        emitido,
        ambiente: ambiente(credenciales.ambiente),
        emisor,
        adquirente,
        lineas: d
            .lineas
            .iter()
            .map(|l| Linea {
                descripcion: l.descripcion.clone(),
                cantidad: l.cantidad,
                unidad: l.unidad.clone(),
                valor_unitario: l.valor_unitario,
                subtotal: l.subtotal,
                iva: (!l.tarifa_iva.is_zero()).then_some((l.tarifa_iva, l.iva)),
            })
            .collect(),
        forma_pago: match d.vencimiento {
            Some(vence) => FormaPago::Credito { vence },
            None => FormaPago::Contado,
        },
        medio_pago: MEDIO_PAGO.into(),
    })
}

/// Documento listo: firmado, con la credencial, las credenciales de la entidad y su UBL.
type Preparado<'a> = (
    DocumentoFirmado,
    CredencialFirma,
    &'a CredencialesFacturador,
    DocumentoUbl,
);

/// Genera y firma el documento; con `contingencia`, la versión tipo 04 de la factura.
fn firmar(d: &DocumentoAEmitir, contingencia: bool) -> Result<Preparado<'_>, ErrorMotor> {
    let credenciales = d.credenciales.as_ref().ok_or_else(|| {
        ErrorMotor::Datos("el motor propio necesita las credenciales de la entidad".into())
    })?;
    let firma = CredencialFirma::desde_pkcs12(&credenciales.p12, &credenciales.contrasena)?;
    if !firma.vigente_en(Utc::now()) {
        return Err(ErrorMotor::Certificado(
            "el certificado de firma está vencido".into(),
        ));
    }
    let software = Software {
        nit_proveedor: d.emisor.documento.clone(),
        dv_proveedor: d.emisor.dv.clone().unwrap_or_default(),
        id: credenciales.software_id.clone(),
        pin: credenciales.pin.clone(),
    };
    let mut ubl = a_ubl(d, credenciales)?;
    if let Clase::Factura {
        contingencia_dian, ..
    } = &mut ubl.clase
    {
        *contingencia_dian = contingencia;
    }
    let firmado = preparar(
        &ubl,
        &software,
        &firma,
        credenciales.consecutivo_archivo,
        &d.id.to_string(),
    )?;
    Ok((firmado, firma, credenciales, ubl))
}

/// `AttachedDocument` firmado del documento, con la respuesta de la DIAN si la hay.
fn contenedor(
    preparado: &Preparado<'_>,
    respuesta: Option<&[u8]>,
    id: &str,
) -> Result<Vec<u8>, ErrorMotor> {
    let (firmado, firma, credenciales, ubl) = preparado;
    let ahora = Utc::now().with_timezone(&colombia());
    Ok(adjunto::generar(
        ubl,
        firmado,
        respuesta.map(|r| (r, ahora)),
        firma,
        credenciales.consecutivo_archivo,
        &format!("{id}-ad"),
    )?
    .xml)
}

fn mensaje(r: &RespuestaDian) -> Option<String> {
    let mut partes: Vec<String> = Vec::new();
    partes.extend(r.descripcion.clone());
    partes.extend(r.mensaje.clone());
    partes.extend(r.errores.iter().cloned());
    let texto: String = partes.join(" | ").chars().take(LARGO_MENSAJE).collect();
    (!texto.is_empty()).then_some(texto)
}

/// Código DIAN del tipo de documento de identidad (tabla 13.2.1 del anexo).
fn tipo_documento_dian(t: TipoDocumento) -> &'static str {
    match t {
        TipoDocumento::Rc => "11",
        TipoDocumento::Ti => "12",
        TipoDocumento::Cc => "13",
        TipoDocumento::Ce => "22",
        TipoDocumento::Nit => TIPO_DOCUMENTO_NIT,
        TipoDocumento::Pa => "41",
        TipoDocumento::Ppt => "48",
    }
}

fn persona_evento(p: &PersonaRecibe) -> PersonaEvento {
    let tipo = tipo_documento_dian(p.tipo_documento);
    let dv = (p.tipo_documento == TipoDocumento::Nit)
        .then(|| {
            domain::Nit::new(&p.documento)
                .ok()
                .map(|n| n.dv().to_string())
        })
        .flatten();
    PersonaEvento {
        tipo_documento: tipo.into(),
        documento: p.documento.clone(),
        dv,
        nombres: p.nombres.clone(),
        apellidos: p.apellidos.clone(),
        cargo: p.cargo.clone(),
        area: p.area.clone(),
    }
}

/// Evento de negocio → `ApplicationResponse`.
fn a_evento(
    e: &EventoAEmitir,
    credenciales: &CredencialesFacturador,
) -> Result<EventoUbl, ErrorMotor> {
    let ahora = Utc::now().with_timezone(&colombia());
    let emitido = colombia()
        .from_local_datetime(&e.fecha.and_time(ahora.time()))
        .single()
        .ok_or_else(|| ErrorMotor::Datos("fecha del evento inválida".into()))?;
    Ok(EventoUbl {
        codigo: e.tipo.codigo().into(),
        descripcion: e.tipo.descripcion().into(),
        numero: e.numero.clone(),
        emitido,
        ambiente: ambiente(credenciales.ambiente),
        generador: parte(&e.generador),
        receptor: parte(&e.receptor),
        factura_numero: e.factura_numero.clone(),
        factura_cufe: e.factura_cufe.clone(),
        concepto: e.concepto_reclamo.map(|c| {
            let (codigo, nombre) = c.codigo_y_nombre();
            (codigo.to_owned(), nombre.to_owned())
        }),
        quien_recibe: e.quien_recibe.as_ref().map(persona_evento),
        nota: e.nota.clone(),
    })
}

/// Motor propio: un cliente por ambiente del servicio web de la DIAN.
#[derive(Debug, Clone)]
pub struct MotorPropio {
    habilitacion: ServicioDian,
    produccion: ServicioDian,
    /// Espera entre los intentos de transmisión (5 s, anexo técnico).
    espera: Duration,
}

/// Intentos de transmisión antes de declarar la contingencia (anexo técnico).
const INTENTOS: u32 = 3;

impl MotorPropio {
    /// Motor con las URL de habilitación y de producción.
    ///
    /// # Errors
    ///
    /// [`ErrorMotor::Transporte`] si no se puede preparar el cliente TLS.
    pub fn new(url_habilitacion: &str, url_produccion: &str) -> Result<Self, ErrorMotor> {
        Ok(Self {
            habilitacion: ServicioDian::new(url_habilitacion)?,
            produccion: ServicioDian::new(url_produccion)?,
            espera: Duration::from_secs(5),
        })
    }

    /// Cambia la espera entre intentos (las pruebas no esperan 5 s).
    pub fn con_espera(mut self, espera: Duration) -> Self {
        self.espera = espera;
        self
    }

    fn servicio(&self, ambiente: AmbienteDian) -> &ServicioDian {
        match ambiente {
            AmbienteDian::Habilitacion => &self.habilitacion,
            AmbienteDian::Produccion => &self.produccion,
        }
    }

    /// Genera, firma y transmite el documento (`SendBillSync`) con hasta 3 intentos separados
    /// 5 s. Validado, devuelve también el `AttachedDocument` para el adquirente. Si la DIAN no
    /// responde, una factura sale en **contingencia tipo 04** (mismo prefijo y número, nueva firma)
    /// con su `AttachedDocument` sin validación, para entregarla y transmitirla después.
    ///
    /// # Errors
    ///
    /// [`AppError::Validacion`] si el documento o el certificado no sirven;
    /// [`AppError::Infraestructura`] si la DIAN no responde y el documento no admite contingencia
    /// (notas) o ya estaba en ella.
    pub async fn emitir(&self, d: &DocumentoAEmitir) -> AppResult<RespuestaProveedor> {
        let preparado = firmar(d, d.contingencia).map_err(a_app)?;
        let (firmado, firma, credenciales, _) = &preparado;
        let mut caida = None;
        for intento in 0..INTENTOS {
            match self
                .servicio(credenciales.ambiente)
                .enviar(firma, Utc::now(), &firmado.nombre_zip, &firmado.zip())
                .await
            {
                Ok(r) => {
                    let adjunto = if r.valido && !preparado.3.clase.es_soporte() {
                        Some(
                            contenedor(
                                &preparado,
                                r.application_response.as_deref(),
                                &d.id.to_string(),
                            )
                            .map_err(a_app)?,
                        )
                    } else {
                        None
                    };
                    return Ok(RespuestaProveedor {
                        estado: if r.valido {
                            EstadoDocumentoElectronico::Validado
                        } else {
                            EstadoDocumentoElectronico::Rechazado
                        },
                        id_externo: Some(firmado.codigo.clone()),
                        mensaje: mensaje(&r),
                        cufe: Some(firmado.codigo.clone()),
                        xml: Some(firmado.xml.clone()),
                        adjunto,
                    });
                }
                Err(ErrorMotor::Transporte(e)) => {
                    caida = Some(e);
                    if intento + 1 < INTENTOS {
                        tokio::time::sleep(self.espera).await;
                    }
                }
                Err(otro) => return Err(a_app(otro)),
            }
        }
        let caida = ErrorMotor::Transporte(caida.unwrap_or_default());
        // La contingencia de la DIAN (tipo 04) solo aplica a la factura normal: las notas, el
        // documento soporte y la transcripción tipo 03 esperan a que la DIAN responda.
        if d.tipo != TipoDocumentoElectronico::Factura
            || d.contingencia
            || d.transcripcion_contingencia
        {
            return Err(a_app(caida));
        }
        let tipo_04 = firmar(d, true).map_err(a_app)?;
        let adjunto = contenedor(&tipo_04, None, &d.id.to_string()).map_err(a_app)?;
        let (f04, ..) = tipo_04;
        Ok(RespuestaProveedor {
            estado: EstadoDocumentoElectronico::Contingencia,
            id_externo: Some(f04.codigo.clone()),
            cufe: Some(f04.codigo),
            xml: Some(f04.xml),
            adjunto: Some(adjunto),
            mensaje: Some(format!(
                "la DIAN no respondió ({caida}): factura de contingencia tipo 04 entregada; se \
                 transmite dentro de las 48 horas"
            )),
        })
    }

    /// Estado de un documento transmitido (`GetStatus` con su CUFE o la clave del envío).
    ///
    /// # Errors
    ///
    /// Los de [`MotorPropio::emitir`].
    pub async fn consultar(
        &self,
        id_externo: &str,
        credenciales: &CredencialesFacturador,
    ) -> AppResult<RespuestaProveedor> {
        let firma = CredencialFirma::desde_pkcs12(&credenciales.p12, &credenciales.contrasena)
            .map_err(a_app)?;
        let r = self
            .servicio(credenciales.ambiente)
            .estado(&firma, Utc::now(), id_externo)
            .await
            .map_err(a_app)?;
        Ok(RespuestaProveedor {
            estado: if r.valido {
                EstadoDocumentoElectronico::Validado
            } else if r.errores.is_empty() {
                EstadoDocumentoElectronico::Enviado
            } else {
                EstadoDocumentoElectronico::Rechazado
            },
            id_externo: Some(id_externo.to_owned()),
            cufe: None,
            xml: None,
            adjunto: None,
            mensaje: mensaje(&r),
        })
    }
}

impl MotorPropio {
    /// Firma y envía un evento RADIAN con las credenciales de la entidad
    /// (`SendEventUpdateStatus`). Los eventos no tienen contingencia: si la DIAN no responde, es
    /// un error de infraestructura y se registran después.
    ///
    /// # Errors
    ///
    /// [`AppError::Validacion`] si faltan las credenciales o un dato; [`AppError::Infraestructura`]
    /// si la DIAN no responde.
    pub async fn evento(&self, e: &EventoAEmitir) -> AppResult<RespuestaProveedor> {
        let credenciales = e.credenciales.as_ref().ok_or_else(|| {
            AppError::Validacion("el motor propio necesita las credenciales de la entidad".into())
        })?;
        let firma = CredencialFirma::desde_pkcs12(&credenciales.p12, &credenciales.contrasena)
            .map_err(a_app)?;
        if !firma.vigente_en(Utc::now()) {
            return Err(a_app(ErrorMotor::Certificado(
                "el certificado de firma está vencido".into(),
            )));
        }
        let software = Software {
            nit_proveedor: e.generador.documento.clone(),
            dv_proveedor: e.generador.dv.clone().unwrap_or_default(),
            id: credenciales.software_id.clone(),
            pin: credenciales.pin.clone(),
        };
        let ubl = a_evento(e, credenciales).map_err(a_app)?;
        let firmado = preparar_evento(
            &ubl,
            &software,
            &firma,
            credenciales.consecutivo_archivo,
            &e.id.to_string(),
        )
        .map_err(a_app)?;
        let r = self
            .servicio(credenciales.ambiente)
            .enviar_evento(&firma, Utc::now(), &firmado.zip())
            .await
            .map_err(a_app)?;
        Ok(RespuestaProveedor {
            estado: if r.valido {
                EstadoDocumentoElectronico::Validado
            } else {
                EstadoDocumentoElectronico::Rechazado
            },
            id_externo: Some(firmado.codigo.clone()),
            mensaje: mensaje(&r),
            cufe: Some(firmado.codigo),
            xml: Some(firmado.xml),
            adjunto: None,
        })
    }
}

impl HabilitacionDian for MotorPropio {
    async fn enviar_set_pruebas(&self, d: &DocumentoAEmitir) -> AppResult<RespuestaProveedor> {
        let (firmado, firma, credenciales, _) = firmar(d, false).map_err(a_app)?;
        let test_set = credenciales
            .test_set_id
            .as_deref()
            .ok_or_else(|| AppError::Validacion("falta el TestSetId del set de pruebas".into()))?;
        let clave = self
            .habilitacion
            .enviar_set_pruebas(
                &firma,
                Utc::now(),
                &firmado.nombre_zip,
                &firmado.zip(),
                test_set,
            )
            .await
            .map_err(a_app)?;
        Ok(RespuestaProveedor {
            estado: EstadoDocumentoElectronico::Enviado,
            id_externo: Some(clave),
            cufe: Some(firmado.codigo),
            xml: Some(firmado.xml),
            adjunto: None,
            mensaje: None,
        })
    }

    async fn consultar_set(
        &self,
        credenciales: &CredencialesFacturador,
        clave: &str,
    ) -> AppResult<RespuestaProveedor> {
        let firma = CredencialFirma::desde_pkcs12(&credenciales.p12, &credenciales.contrasena)
            .map_err(a_app)?;
        let respuestas = self
            .habilitacion
            .estado_lote(&firma, Utc::now(), clave)
            .await
            .map_err(a_app)?;
        let estado = match respuestas.first() {
            Some(r) if r.valido => EstadoDocumentoElectronico::Validado,
            Some(r) if !r.errores.is_empty() => EstadoDocumentoElectronico::Rechazado,
            _ => EstadoDocumentoElectronico::Enviado,
        };
        Ok(RespuestaProveedor {
            estado,
            id_externo: Some(clave.to_owned()),
            cufe: None,
            xml: None,
            adjunto: None,
            mensaje: respuestas.first().and_then(mensaje),
        })
    }

    async fn rangos_numeracion(
        &self,
        credenciales: &CredencialesFacturador,
        nit: &str,
    ) -> AppResult<Vec<RangoAutorizado>> {
        let firma = CredencialFirma::desde_pkcs12(&credenciales.p12, &credenciales.contrasena)
            .map_err(a_app)?;
        let rangos = self
            .servicio(credenciales.ambiente)
            .rangos_numeracion(&firma, Utc::now(), nit, &credenciales.software_id)
            .await
            .map_err(a_app)?;
        Ok(rangos
            .into_iter()
            .map(|r| RangoAutorizado {
                resolucion: r.resolucion,
                fecha_resolucion: r.fecha_resolucion,
                prefijo: r.prefijo,
                desde: r.desde,
                hasta: r.hasta,
                vigente_desde: r.vigente_desde,
                vigente_hasta: r.vigente_hasta,
                clave_tecnica: r.clave_tecnica,
            })
            .collect())
    }

    async fn correos_recepcion(
        &self,
        credenciales: &CredencialesFacturador,
    ) -> AppResult<Vec<(String, String)>> {
        let firma = CredencialFirma::desde_pkcs12(&credenciales.p12, &credenciales.contrasena)
            .map_err(a_app)?;
        self.servicio(credenciales.ambiente)
            .correos_recepcion(&firma, Utc::now())
            .await
            .map_err(a_app)
    }
}

/// Lee el `.p12` que carga la entidad y valida que sirva para firmar.
#[derive(Debug, Clone, Copy, Default)]
pub struct LectorPkcs12;

impl LectorCertificados for LectorPkcs12 {
    fn leer(&self, p12: &[u8], contrasena: &str) -> AppResult<CertificadoLeido> {
        let firma = CredencialFirma::desde_pkcs12(p12, contrasena)
            .map_err(|e| AppError::Validacion(e.to_string()))?;
        let r = firma.resumen().ok_or_else(|| {
            AppError::Validacion("el .p12 no trae el certificado del firmante".into())
        })?;
        Ok(CertificadoLeido {
            sujeto: r.sujeto,
            emisor: r.emisor,
            serial: r.serial,
            vigente_desde: r.vigente_desde,
            vigente_hasta: r.vigente_hasta,
        })
    }
}

/// Proveedor de la API: cada documento con credenciales (canal `propio`) sale por el
/// [`MotorPropio`]; los demás, por el proveedor simulado (hasta elegir el proveedor tecnológico).
/// Los eventos RADIAN propios llegan con la fase 7 del plan 17.
#[derive(Debug, Clone)]
pub struct ProveedorDian {
    simulado: ProveedorSimulado,
    propio: MotorPropio,
}

impl ProveedorDian {
    /// Proveedor con el simulado y el motor propio.
    pub fn new(simulado: ProveedorSimulado, propio: MotorPropio) -> Self {
        Self { simulado, propio }
    }

    /// El motor propio (habilitación).
    pub fn propio(&self) -> &MotorPropio {
        &self.propio
    }
}

impl ProveedorFacturacion for ProveedorDian {
    fn nombre(&self) -> &'static str {
        self.simulado.nombre()
    }

    async fn emitir(&self, d: &DocumentoAEmitir) -> AppResult<RespuestaProveedor> {
        if d.credenciales.is_some() {
            self.propio.emitir(d).await
        } else {
            self.simulado.emitir(d).await
        }
    }

    async fn consultar(
        &self,
        id_externo: &str,
        credenciales: Option<&CredencialesFacturador>,
    ) -> AppResult<RespuestaProveedor> {
        match credenciales {
            Some(c) => self.propio.consultar(id_externo, c).await,
            None => self.simulado.consultar(id_externo, None).await,
        }
    }

    async fn evento_radian(&self, e: &EventoAEmitir) -> AppResult<RespuestaProveedor> {
        if e.credenciales.is_some() {
            self.propio.evento(e).await
        } else {
            self.simulado.evento_radian(e).await
        }
    }
}
