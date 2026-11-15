//! Pruebas funcionales del motor propio (plan 17, fases 1–4) contra un simulador local del
//! servicio web de la DIAN: el cliente prepara y firma documentos, los envía como lo haría con
//! la DIAN y lee sus respuestas. Nunca se habla con la DIAN real (AGENTS.md §6).

use std::sync::{Arc, Mutex};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, NaiveDate, Utc};
use infra_facturacion::propio::{
    ErrorMotor,
    control::Ambiente,
    firma::CredencialFirma,
    preparar,
    servicio::ServicioDian,
    ubl::{
        Clase, Direccion, DocumentoUbl, FormaPago, Linea, Parte, RegimenIva, Resolucion, Software,
        TipoPersona,
    },
};
use rust_decimal::Decimal;

const CLAVE: &str = include_str!("fixtures/firma/firmante.key");
const FIRMANTE: &str = include_str!("fixtures/firma/firmante.crt");
const CA: &str = include_str!("fixtures/firma/ca.crt");

/// Petición recibida por el simulador: acción SOAP y cuerpo.
#[derive(Debug, Clone)]
struct Peticion {
    accion: String,
    cuerpo: String,
}

/// Respuesta que el simulador da a cada petición: estado HTTP y cuerpo.
type Respondedor = Arc<dyn Fn(&Peticion) -> (StatusCode, String) + Send + Sync>;

#[derive(Clone)]
struct Simulador {
    peticiones: Arc<Mutex<Vec<Peticion>>>,
    respuesta: Respondedor,
}

async fn atender(
    State(sim): State<Simulador>,
    cabeceras: HeaderMap,
    cuerpo: Bytes,
) -> impl IntoResponse {
    let tipo = cabeceras
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let accion = tipo
        .split("action=\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .unwrap_or_default()
        .to_owned();
    let peticion = Peticion {
        accion,
        cuerpo: String::from_utf8_lossy(&cuerpo).into_owned(),
    };
    let respuesta = (sim.respuesta)(&peticion);
    sim.peticiones.lock().expect("candado").push(peticion);
    respuesta
}

/// Levanta el simulador en un puerto libre y devuelve su URL y las peticiones recibidas.
async fn simulador(
    respuesta: impl Fn(&Peticion) -> (StatusCode, String) + Send + Sync + 'static,
) -> (String, Arc<Mutex<Vec<Peticion>>>) {
    let peticiones = Arc::new(Mutex::new(Vec::new()));
    let estado = Simulador {
        peticiones: peticiones.clone(),
        respuesta: Arc::new(respuesta),
    };
    let app = Router::new()
        .route("/WcfDianCustomerServices.svc", post(atender))
        .with_state(estado);
    let escucha = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("puerto");
    let url = format!(
        "http://{}/WcfDianCustomerServices.svc",
        escucha.local_addr().expect("dirección")
    );
    tokio::spawn(async move {
        axum::serve(escucha, app).await.expect("simulador");
    });
    (url, peticiones)
}

fn sobre(contenido: &str) -> String {
    format!(
        "<s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\"><s:Body>{contenido}</s:Body></s:Envelope>"
    )
}

fn respuesta_validacion(valido: bool, codigo: &str, errores: &[&str], clave: &str) -> String {
    let errores: String = errores
        .iter()
        .map(|e| format!("<c:string>{e}</c:string>"))
        .collect();
    sobre(&format!(
        "<SendBillSyncResponse xmlns=\"http://wcf.dian.colombia\"><SendBillSyncResult \
         xmlns:b=\"http://schemas.datacontract.org/2004/07/DianResponse\">\
         <b:ErrorMessage xmlns:c=\"arr\">{errores}</b:ErrorMessage>\
         <b:IsValid>{valido}</b:IsValid><b:StatusCode>{codigo}</b:StatusCode>\
         <b:StatusDescription>Procesado</b:StatusDescription>\
         <b:XmlBase64Bytes>{}</b:XmlBase64Bytes><b:XmlDocumentKey>{clave}</b:XmlDocumentKey>\
         </SendBillSyncResult></SendBillSyncResponse>",
        STANDARD.encode("<ApplicationResponse>validado</ApplicationResponse>")
    ))
}

fn credencial() -> CredencialFirma {
    CredencialFirma::desde_pem(CLAVE, &format!("{FIRMANTE}{CA}")).expect("credencial")
}

fn ahora() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2027-01-15T15:30:00Z")
        .expect("fecha")
        .with_timezone(&Utc)
}

fn dec(texto: &str) -> Decimal {
    texto.parse().expect("decimal")
}

fn software() -> Software {
    Software {
        nit_proveedor: "901713998".into(),
        dv_proveedor: "3".into(),
        id: "56f2ae4e-9812-4fad-9255-08fcfcd5ccb0".into(),
        pin: "12345".into(),
    }
}

fn factura(resolucion: Resolucion) -> DocumentoUbl {
    DocumentoUbl {
        prefijo: resolucion.prefijo.clone(),
        numero: resolucion.desde,
        clase: Clase::Factura {
            resolucion,
            contingencia_dian: false,
            contingencia_facturador: false,
        },
        emitido: DateTime::parse_from_rfc3339("2027-01-15T10:30:00-05:00").expect("fecha"),
        ambiente: Ambiente::Pruebas,
        emisor: Parte {
            tipo_persona: TipoPersona::Juridica,
            tipo_documento: "31".into(),
            documento: "901713998".into(),
            dv: Some("3".into()),
            nombre: "GISLA SAS".into(),
            regimen: RegimenIva::Responsable,
            responsabilidades: vec!["R-99-PN".into()],
            direccion: Some(Direccion {
                municipio: "11001".into(),
                ciudad: "Bogotá, D.C.".into(),
                departamento: "Bogotá".into(),
                codigo_departamento: "11".into(),
                linea: "Calle 71 A Sur".into(),
            }),
            correo: None,
        },
        adquirente: Parte {
            tipo_persona: TipoPersona::Juridica,
            tipo_documento: "31".into(),
            documento: "900000001".into(),
            dv: Some("1".into()),
            nombre: "Conjunto Residencial de Pruebas PH".into(),
            regimen: RegimenIva::NoResponsable,
            responsabilidades: vec![],
            direccion: None,
            correo: Some("administracion@ejemplo.co".into()),
        },
        lineas: vec![Linea {
            descripcion: "Suscripción mensual DAD Copropiedad".into(),
            cantidad: dec("1"),
            unidad: "94".into(),
            valor_unitario: dec("250000"),
            subtotal: dec("250000"),
            iva: Some((dec("19"), dec("47500"))),
        }],
        forma_pago: FormaPago::Contado,
        medio_pago: "42".into(),
    }
}

fn resolucion(clave_tecnica: &str) -> Resolucion {
    Resolucion {
        numero: "18760000001".into(),
        vigente_desde: NaiveDate::from_ymd_opt(2026, 1, 1).expect("fecha"),
        vigente_hasta: NaiveDate::from_ymd_opt(2030, 12, 31).expect("fecha"),
        prefijo: "SETP".into(),
        desde: 990_000_000,
        hasta: 995_000_000,
        clave_tecnica: clave_tecnica.into(),
    }
}

/// Contenido de `wcf:contentFile` decodificado (el ZIP enviado).
fn zip_enviado(cuerpo: &str) -> Vec<u8> {
    let b64 = cuerpo
        .split("<wcf:contentFile>")
        .nth(1)
        .and_then(|r| r.split("</wcf:contentFile>").next())
        .expect("contentFile");
    STANDARD.decode(b64).expect("Base64")
}

#[tokio::test]
async fn envia_una_factura_firmada_y_la_dian_la_valida() {
    let firmado = preparar(
        &factura(resolucion("fc8eac422eba16e22ffd8c6f94b3f40a6e38162c")),
        &software(),
        &credencial(),
        1,
        "doc-1",
    )
    .expect("factura firmada");
    let cufe = firmado.codigo.clone();
    let (url, peticiones) = simulador(move |p| {
        let zip = zip_enviado(&p.cuerpo);
        let contiene_cufe = String::from_utf8_lossy(&zip).contains(&cufe);
        if contiene_cufe {
            (StatusCode::OK, respuesta_validacion(true, "00", &[], &cufe))
        } else {
            (
                StatusCode::OK,
                respuesta_validacion(false, "99", &["Regla: AA09"], ""),
            )
        }
    })
    .await;
    let servicio = ServicioDian::new(&url).expect("cliente");

    let respuesta = servicio
        .enviar(&credencial(), ahora(), &firmado.nombre_zip, &firmado.zip())
        .await
        .expect("respuesta");

    assert!(respuesta.valido);
    assert_eq!(respuesta.codigo, "00");
    assert_eq!(respuesta.clave.as_deref(), Some(firmado.codigo.as_str()));
    assert_eq!(
        respuesta.application_response.as_deref(),
        Some(&b"<ApplicationResponse>validado</ApplicationResponse>"[..])
    );
    let recibidas = peticiones.lock().expect("candado");
    assert_eq!(
        recibidas[0].accion,
        "http://wcf.dian.colombia/IWcfDianCustomerServices/SendBillSync"
    );
    let cuerpo = &recibidas[0].cuerpo;
    assert!(cuerpo.contains("<wsse:BinarySecurityToken "));
    assert!(cuerpo.contains(
        "<wsa:Action>http://wcf.dian.colombia/IWcfDianCustomerServices/SendBillSync</wsa:Action>"
    ));
    assert!(cuerpo.contains(&format!(
        "<wcf:fileName>{}</wcf:fileName>",
        firmado.nombre_zip
    )));
}

#[tokio::test]
async fn una_factura_rechazada_trae_las_reglas_incumplidas() {
    let firmado = preparar(
        &factura(resolucion("clave")),
        &software(),
        &credencial(),
        2,
        "doc-2",
    )
    .expect("factura firmada");
    let (url, _) = simulador(|_| {
        (
            StatusCode::OK,
            respuesta_validacion(
                false,
                "99",
                &["Regla: AA09 Valor del CUFE no está calculado correctamente."],
                "",
            ),
        )
    })
    .await;

    let respuesta = ServicioDian::new(&url)
        .expect("cliente")
        .enviar(&credencial(), ahora(), &firmado.nombre_zip, &firmado.zip())
        .await
        .expect("respuesta");

    assert!(!respuesta.valido);
    assert_eq!(respuesta.codigo, "99");
    assert_eq!(respuesta.errores.len(), 1);
    assert!(respuesta.errores[0].contains("AA09"));
}

#[tokio::test]
async fn la_clave_tecnica_de_get_numbering_range_entra_al_cufe() {
    let (url, peticiones) = simulador(|_| {
        (
            StatusCode::OK,
            sobre(
                "<GetNumberingRangeResponse xmlns=\"http://wcf.dian.colombia\"><GetNumberingRangeResult>\
                 <OperationCode>100</OperationCode><OperationDescription>OK</OperationDescription>\
                 <ResponseList><NumberRangeResponse><ResolutionNumber>18760000001</ResolutionNumber>\
                 <ResolutionDate>2026-01-01</ResolutionDate><Prefix>SETP</Prefix>\
                 <FromNumber>990000000</FromNumber><ToNumber>995000000</ToNumber>\
                 <ValidDateFrom>2026-01-01</ValidDateFrom><ValidDateTo>2030-12-31</ValidDateTo>\
                 <TechnicalKey>clave-del-rango</TechnicalKey></NumberRangeResponse></ResponseList>\
                 </GetNumberingRangeResult></GetNumberingRangeResponse>",
            ),
        )
    })
    .await;
    let rangos = ServicioDian::new(&url)
        .expect("cliente")
        .rangos_numeracion(&credencial(), ahora(), "901713998", &software().id)
        .await
        .expect("rangos");
    assert_eq!(rangos.len(), 1);
    let r = &rangos[0];
    assert!(
        peticiones.lock().expect("candado")[0]
            .cuerpo
            .contains("<wcf:softwareCode>56f2ae4e-9812-4fad-9255-08fcfcd5ccb0</wcf:softwareCode>")
    );

    let con_clave = |clave: &str| {
        preparar(
            &factura(Resolucion {
                numero: r.resolucion.clone(),
                vigente_desde: r.vigente_desde,
                vigente_hasta: r.vigente_hasta,
                prefijo: r.prefijo.clone(),
                desde: r.desde,
                hasta: r.hasta,
                clave_tecnica: clave.into(),
            }),
            &software(),
            &credencial(),
            1,
            "doc",
        )
        .expect("firmado")
        .codigo
    };
    assert_eq!(con_clave(&r.clave_tecnica), con_clave("clave-del-rango"));
    assert_ne!(con_clave(&r.clave_tecnica), con_clave("otra-clave"));
}

#[tokio::test]
async fn set_de_pruebas_devuelve_la_clave_y_se_consulta_su_estado() {
    let (url, peticiones) = simulador(|p| {
        if p.accion.ends_with("SendTestSetAsync") {
            (
                StatusCode::OK,
                sobre("<SendTestSetAsyncResponse><SendTestSetAsyncResult><ErrorMessageList/>\
                       <ZipKey>358f9538-1f80-4ed5-a3f6-aaa1ef36bebd</ZipKey>\
                       </SendTestSetAsyncResult></SendTestSetAsyncResponse>"),
            )
        } else {
            (
                StatusCode::OK,
                sobre("<GetStatusZipResponse><GetStatusZipResult>\
                       <DianResponse><IsValid>true</IsValid><StatusCode>00</StatusCode></DianResponse>\
                       </GetStatusZipResult></GetStatusZipResponse>"),
            )
        }
    })
    .await;
    let servicio = ServicioDian::new(&url).expect("cliente");
    let firmado = preparar(
        &factura(resolucion("clave")),
        &software(),
        &credencial(),
        1,
        "set",
    )
    .expect("firmado");

    let clave = servicio
        .enviar_set_pruebas(
            &credencial(),
            ahora(),
            &firmado.nombre_zip,
            &firmado.zip(),
            "4de36cb4-9973-4ea4-a156-34e909aa24dc",
        )
        .await
        .expect("clave del envío");
    let estado = servicio
        .estado_lote(&credencial(), ahora(), &clave)
        .await
        .expect("estado");

    assert_eq!(clave, "358f9538-1f80-4ed5-a3f6-aaa1ef36bebd");
    assert_eq!(estado.len(), 1);
    assert!(estado[0].valido);
    let recibidas = peticiones.lock().expect("candado");
    assert!(
        recibidas[0]
            .cuerpo
            .contains("<wcf:testSetId>4de36cb4-9973-4ea4-a156-34e909aa24dc</wcf:testSetId>")
    );
    assert!(
        recibidas[1]
            .cuerpo
            .contains(&format!("<wcf:trackId>{clave}</wcf:trackId>"))
    );
}

#[tokio::test]
async fn un_soap_fault_es_error_de_respuesta() {
    let (url, _) = simulador(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            sobre("<s:Fault><s:Code><s:Value>s:Sender</s:Value></s:Code><s:Reason>\
                   <s:Text xml:lang=\"es\">An error occurred when verifying security for the message.</s:Text>\
                   </s:Reason></s:Fault>"),
        )
    })
    .await;
    let error = ServicioDian::new(&url)
        .expect("cliente")
        .estado(&credencial(), ahora(), "abc")
        .await
        .expect_err("fault");
    assert!(matches!(error, ErrorMotor::Respuesta(m) if m.contains("verifying security")));
}

#[tokio::test]
async fn un_error_del_servidor_sin_fault_es_contingencia() {
    let (url, _) = simulador(|_| (StatusCode::SERVICE_UNAVAILABLE, "no disponible".into())).await;
    let error = ServicioDian::new(&url)
        .expect("cliente")
        .estado(&credencial(), ahora(), "abc")
        .await
        .expect_err("503");
    assert!(matches!(error, ErrorMotor::Transporte(_)));
}

#[tokio::test]
async fn sin_servicio_es_error_de_transporte() {
    // Un puerto que se abrió y se cerró: nadie escucha ahí.
    let escucha = std::net::TcpListener::bind("127.0.0.1:0").expect("puerto");
    let direccion = escucha.local_addr().expect("dirección");
    drop(escucha);
    let error = ServicioDian::new(&format!("http://{direccion}/WcfDianCustomerServices.svc"))
        .expect("cliente")
        .estado(&credencial(), ahora(), "abc")
        .await
        .expect_err("sin servicio");
    assert!(matches!(error, ErrorMotor::Transporte(_)));
}

// ---------------------------------------------------------------------------------------------
// Adaptador del motor propio a los puertos (plan 18, parte B)
// ---------------------------------------------------------------------------------------------

mod adaptador {
    use application::puertos::tributario::{
        CredencialesFacturador, DireccionDocumento, DocumentoAEmitir, EventoAEmitir,
        HabilitacionDian, LectorCertificados, LineaDocumento, ParteDocumento, PersonaRecibe,
        ProveedorFacturacion, ResolucionDocumento,
    };
    use domain::{
        DocumentoElectronicoId,
        facturacion::AmbienteDian,
        tributario::{EstadoDocumentoElectronico, EventoRadian, TipoDocumentoElectronico},
    };
    use infra_facturacion::{LectorPkcs12, MotorPropio, ProveedorDian, ProveedorSimulado};

    use super::*;

    const P12: &[u8] = include_bytes!("fixtures/firma/firmante_legado.p12");

    fn credenciales(ambiente: AmbienteDian) -> CredencialesFacturador {
        CredencialesFacturador {
            ambiente,
            software_id: "56f2ae4e-9812-4fad-9255-08fcfcd5ccb0".into(),
            pin: "12345".into(),
            p12: P12.to_vec(),
            contrasena: "prueba-123".into(),
            test_set_id: Some("ab8a5fa2-1c4b-4f3c-93df-d1c2f9f7a1e2".into()),
            consecutivo_archivo: 1,
        }
    }

    fn documento(credenciales: Option<CredencialesFacturador>) -> DocumentoAEmitir {
        let valor = Decimal::from(100_000);
        DocumentoAEmitir {
            id: DocumentoElectronicoId::nuevo(),
            tipo: TipoDocumentoElectronico::Factura,
            prefijo: "SETP".into(),
            numero: 990_000_001,
            fecha: Utc::now().date_naive(),
            emisor: ParteDocumento {
                documento: "901713998".into(),
                dv: Some("3".into()),
                nombre: "GISLA SAS".into(),
                responsable_iva: true,
                responsabilidades: vec!["O-13".into()],
                direccion: Some(DireccionDocumento {
                    linea: "Calle 1 # 2-3".into(),
                    municipio_dane: "11001".into(),
                    departamento: Some("Bogotá".into()),
                }),
                ..ParteDocumento::default()
            },
            adquirente: ParteDocumento {
                documento: "222222222222".into(),
                nombre: "Consumidor final".into(),
                persona_natural: true,
                ..ParteDocumento::default()
            },
            lineas: vec![LineaDocumento {
                descripcion: "Servicio".into(),
                unidad: "94".into(),
                cantidad: Decimal::ONE,
                valor_unitario: valor,
                subtotal: valor,
                tarifa_iva: Decimal::ZERO,
                iva: Decimal::ZERO,
            }],
            subtotal: valor,
            iva: Decimal::ZERO,
            total: valor,
            clave_tecnica: Some("fc8eac422eba16e22ffd8c6f94b3f40a6e38162c".into()),
            resolucion: Some(ResolucionDocumento {
                numero: "18760000001".into(),
                vigente_desde: NaiveDate::from_ymd_opt(2026, 1, 1).expect("fecha"),
                vigente_hasta: NaiveDate::from_ymd_opt(2030, 12, 31).expect("fecha"),
                desde: 990_000_000,
                hasta: 995_000_000,
            }),
            vencimiento: None,
            referencia_cufe: None,
            referencia_numero: None,
            referencia_fecha: None,
            credenciales,
            contingencia: false,
            transcripcion_contingencia: false,
            correccion: None,
        }
    }

    #[test]
    fn el_lector_valida_el_p12_y_devuelve_el_firmante() {
        let c = LectorPkcs12.leer(P12, "prueba-123").expect("certificado");
        assert!(c.sujeto.contains("CN="));
        assert!(LectorPkcs12.leer(P12, "otra").is_err(), "contraseña errada");
    }

    #[tokio::test]
    async fn con_credenciales_transmite_el_motor_propio_firmado_con_el_p12() {
        let (url, peticiones) = simulador(|_| {
            (
                StatusCode::OK,
                respuesta_validacion(true, "00", &[], "clave"),
            )
        })
        .await;
        let proveedor = ProveedorDian::new(
            ProveedorSimulado::new(),
            MotorPropio::new(&url, &url).expect("motor"),
        );

        let r = proveedor
            .emitir(&documento(Some(credenciales(AmbienteDian::Produccion))))
            .await
            .expect("emitida");

        assert_eq!(r.estado, EstadoDocumentoElectronico::Validado);
        let cufe = r.cufe.expect("cufe");
        assert_eq!(cufe.len(), 96, "CUFE SHA-384");
        let xml = String::from_utf8(r.xml.expect("xml")).expect("utf-8");
        assert!(xml.contains("ds:Signature") && xml.contains(&cufe));
        let enviadas = peticiones.lock().expect("candado");
        assert_eq!(enviadas.len(), 1);
        assert!(
            enviadas[0].accion.ends_with("SendBillSync"),
            "{}",
            enviadas[0].accion
        );
    }

    #[tokio::test]
    async fn el_acuse_se_firma_y_se_envia_con_send_event_update_status() {
        let (url, peticiones) = simulador(|_| {
            (
                StatusCode::OK,
                respuesta_validacion(true, "00", &[], "clave"),
            )
        })
        .await;
        let d = documento(None);
        let evento = EventoAEmitir {
            id: uuid::Uuid::now_v7(),
            tipo: EventoRadian::Acuse,
            numero: "EV2026000001".into(),
            fecha: Utc::now().date_naive(),
            generador: d.emisor.clone(),
            receptor: ParteDocumento {
                documento: "900373115".into(),
                dv: Some("4".into()),
                nombre: "Proveedor SAS".into(),
                ..ParteDocumento::default()
            },
            factura_numero: "SETP990000123".into(),
            factura_cufe: "c".repeat(96),
            concepto_reclamo: None,
            quien_recibe: Some(PersonaRecibe {
                tipo_documento: domain::TipoDocumento::Cc,
                documento: "1020304050".into(),
                nombres: "Ana".into(),
                apellidos: "Pérez".into(),
                cargo: None,
                area: None,
            }),
            nota: None,
            credenciales: Some(credenciales(AmbienteDian::Produccion)),
        };

        let r = motor(&url).evento(&evento).await.expect("evento");

        assert_eq!(r.estado, EstadoDocumentoElectronico::Validado);
        let cude = r.cufe.expect("cude");
        let xml = String::from_utf8(r.xml.expect("xml")).expect("utf-8");
        assert!(xml.contains("<ApplicationResponse") && xml.contains(&cude));
        let enviadas = peticiones.lock().expect("candado");
        assert!(
            enviadas[0].accion.ends_with("SendEventUpdateStatus"),
            "{}",
            enviadas[0].accion
        );
    }

    #[tokio::test]
    async fn el_documento_soporte_se_transmite_con_el_vendedor_como_proveedor_y_sin_contenedor() {
        let (url, _) = simulador(|_| {
            (
                StatusCode::OK,
                respuesta_validacion(true, "00", &[], "clave"),
            )
        })
        .await;
        let mut d = documento(Some(credenciales(AmbienteDian::Produccion)));
        d.tipo = TipoDocumentoElectronico::DocumentoSoporte;
        d.prefijo = "DS".into();
        d.numero = 1;
        d.clave_tecnica = None;
        d.resolucion = Some(ResolucionDocumento {
            desde: 1,
            hasta: 100,
            ..d.resolucion.clone().expect("resolución")
        });
        d.adquirente = ParteDocumento {
            documento: "79123456".into(),
            nombre: "Pedro Gómez".into(),
            persona_natural: true,
            direccion: d.emisor.direccion.clone(),
            ..ParteDocumento::default()
        };

        let r = motor(&url).emitir(&d).await.expect("documento soporte");

        assert_eq!(r.estado, EstadoDocumentoElectronico::Validado);
        assert!(
            r.adjunto.is_none(),
            "el documento soporte no lleva contenedor"
        );
        let xml = String::from_utf8(r.xml.expect("xml")).expect("utf-8");
        assert!(xml.contains("<cbc:InvoiceTypeCode>05</cbc:InvoiceTypeCode>"));
        let proveedor = &xml[xml
            .find("<cac:AccountingSupplierParty>")
            .expect("proveedor")
            ..xml.find("</cac:AccountingSupplierParty>").expect("fin")];
        assert!(
            proveedor.contains("79123456"),
            "el vendedor va como proveedor"
        );
    }

    fn motor(url: &str) -> MotorPropio {
        MotorPropio::new(url, url)
            .expect("motor")
            .con_espera(std::time::Duration::from_millis(1))
    }

    #[tokio::test]
    async fn validada_trae_el_attached_document_con_la_respuesta_de_la_dian() {
        let (url, _) = simulador(|_| {
            (
                StatusCode::OK,
                respuesta_validacion(true, "00", &[], "clave"),
            )
        })
        .await;

        let r = motor(&url)
            .emitir(&documento(Some(credenciales(AmbienteDian::Produccion))))
            .await
            .expect("emitida");

        let adjunto = String::from_utf8(r.adjunto.expect("adjunto")).expect("utf-8");
        assert!(adjunto.contains("<AttachedDocument"));
        assert!(
            adjunto.contains("ApplicationResponse"),
            "lleva la respuesta de la DIAN"
        );
        assert!(adjunto.contains("<cbc:ValidationResultCode>02</cbc:ValidationResultCode>"));
    }

    #[tokio::test]
    async fn si_la_dian_no_responde_tras_tres_intentos_sale_la_factura_de_contingencia_04() {
        let (url, peticiones) =
            simulador(|_| (StatusCode::SERVICE_UNAVAILABLE, String::new())).await;

        let r = motor(&url)
            .emitir(&documento(Some(credenciales(AmbienteDian::Produccion))))
            .await
            .expect("contingencia");

        assert_eq!(
            peticiones.lock().expect("candado").len(),
            3,
            "tres intentos"
        );
        assert_eq!(r.estado, EstadoDocumentoElectronico::Contingencia);
        let xml = String::from_utf8(r.xml.expect("xml")).expect("utf-8");
        assert!(xml.contains(">04</cbc:InvoiceTypeCode>"), "tipo 04");
        let adjunto = String::from_utf8(r.adjunto.expect("adjunto")).expect("utf-8");
        assert!(adjunto.contains("<cbc:ValidationResultCode>04</cbc:ValidationResultCode>"));
    }

    #[tokio::test]
    async fn una_factura_ya_en_contingencia_se_retransmite_como_04_y_si_sigue_caida_espera() {
        let (url, peticiones) =
            simulador(|_| (StatusCode::SERVICE_UNAVAILABLE, String::new())).await;
        let mut d = documento(Some(credenciales(AmbienteDian::Produccion)));
        d.contingencia = true;

        let r = motor(&url).emitir(&d).await;

        assert!(
            matches!(r, Err(application::AppError::Infraestructura(_))),
            "{r:?}"
        );
        let p = peticiones.lock().expect("candado");
        let xml_enviado = p.iter().any(|x| x.accion.ends_with("SendBillSync"));
        assert!(xml_enviado, "se intentó transmitir la versión 04");
    }

    #[tokio::test]
    async fn sin_credenciales_responde_el_simulado_sin_salir_a_la_red() {
        let proveedor = ProveedorDian::new(
            ProveedorSimulado::new(),
            MotorPropio::new("http://127.0.0.1:9/no", "http://127.0.0.1:9/no").expect("motor"),
        );

        let r = proveedor.emitir(&documento(None)).await.expect("simulado");

        assert_eq!(r.estado, EstadoDocumentoElectronico::Validado);
        assert!(r.id_externo.expect("id").starts_with("sim-"));
    }

    #[tokio::test]
    async fn el_set_de_pruebas_viaja_con_el_test_set_id_y_se_consulta() {
        let (url, peticiones) = simulador(|p| {
            if p.accion.ends_with("SendTestSetAsync") {
                (
                    StatusCode::OK,
                    sobre("<SendTestSetAsyncResponse><SendTestSetAsyncResult><ErrorMessageList/>\
                           <ZipKey>clave-del-zip</ZipKey>\
                           </SendTestSetAsyncResult></SendTestSetAsyncResponse>"),
                )
            } else {
                (
                    StatusCode::OK,
                    sobre("<GetStatusZipResponse><GetStatusZipResult>\
                           <DianResponse><IsValid>true</IsValid><StatusCode>00</StatusCode></DianResponse>\
                           </GetStatusZipResult></GetStatusZipResponse>"),
                )
            }
        })
        .await;
        let motor = MotorPropio::new(&url, &url).expect("motor");
        let cred = credenciales(AmbienteDian::Habilitacion);

        let enviado = motor
            .enviar_set_pruebas(&documento(Some(cred.clone())))
            .await
            .expect("enviado");
        let consulta = motor
            .consultar_set(&cred, "clave-del-zip")
            .await
            .expect("consulta");

        assert_eq!(enviado.id_externo.as_deref(), Some("clave-del-zip"));
        assert_eq!(consulta.estado, EstadoDocumentoElectronico::Validado);
        let p = peticiones.lock().expect("candado");
        assert!(
            p[0].cuerpo.contains("ab8a5fa2-1c4b-4f3c-93df-d1c2f9f7a1e2"),
            "TestSetId"
        );
    }

    #[tokio::test]
    async fn un_documento_soporte_aun_no_sale_por_el_motor_propio() {
        let motor =
            MotorPropio::new("http://127.0.0.1:9/no", "http://127.0.0.1:9/no").expect("motor");
        let mut d = documento(Some(credenciales(AmbienteDian::Produccion)));
        d.tipo = TipoDocumentoElectronico::DocumentoSoporte;

        let r = motor.emitir(&d).await;

        assert!(
            matches!(r, Err(application::AppError::Validacion(_))),
            "{r:?}"
        );
    }
}
