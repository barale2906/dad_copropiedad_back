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
