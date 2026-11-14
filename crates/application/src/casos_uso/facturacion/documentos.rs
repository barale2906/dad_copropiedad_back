//! Documentos electrónicos (plan 15 fases 2–4): factura electrónica de una factura de venta,
//! nota crédito que la anula, documento soporte de una compra a un no obligado a facturar,
//! transmisión por el proveedor ([`ProveedorFacturacion`]: proveedor tecnológico o, más
//! adelante, el motor propio), reintentos, representación gráfica y recepción de facturas de
//! proveedores con sus eventos RADIAN.
//!
//! Reglas: solo factura electrónicamente una entidad con perfil tributario **confirmado** de
//! facturador electrónico; el número sale de una resolución de numeración activa y vigente; un
//! documento de negocio produce un solo documento electrónico de cada tipo (reintentar o volver a
//! pedirlo devuelve el mismo, con el mismo CUFE); un documento rechazado conserva la respuesta y
//! no se marca validado (QA-015): se corrige con un documento nuevo o una nota.

use chrono::NaiveDate;
use domain::{
    ClasificacionArchivo, DocumentoElectronicoId, FacturaProveedorId, FacturaRecibidaId,
    FacturaVentaId, Modulo, NombreArchivo, Permiso, TipoArchivo,
    contabilidad::{
        Monto,
        tesoreria::{EstadoFacturaProveedor, EstadoFacturaVenta},
    },
    facturacion::{AmbienteDian, EstadoHabilitacion},
    tributario::{
        CanalFacturacion, EstadoDocumentoElectronico, EstadoFacturaRecibida, EventoRadian,
        TipoDocumentoElectronico, rango_por_agotarse, siguiente_numero,
    },
};

use crate::casos_uso::contabilidad::prelude::*;
use crate::casos_uso::contabilidad::tesoreria::eventos as eventos_origen;
use crate::casos_uso::contabilidad::tesoreria::origen;
use crate::casos_uso::contabilidad::{TxTesoreria, texto, texto_opcional};
use crate::casos_uso::tributario::perfil_confirmado_en;
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{
        archivos::{ArchivoSubido, borrar_huerfano, nuevo_archivo, registrar_en, sha256_hex},
        modulos::exigir_modulo_en,
    },
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        AlmacenArchivos, GeneradorActas, RegistroOutbox, Reloj, RepositorioArchivos,
        UnidadDeTrabajo,
        actas::TablaPdf,
        cofre::Cofre,
        contabilidad::Origen,
        correo::CorreoSaliente,
        facturacion::{ConfiguracionFacturacion, RepositorioFacturacion},
        tributario::{
            CredencialesFacturador, DireccionDocumento, DocumentoAEmitir, DocumentoElectronico,
            EventoDocumento, FacturaRecibida, FiltroDocumentos, FiltroRecibidas, LectorFacturas,
            LineaDocumento, ParteDocumento, ProveedorFacturacion, RegistroRadian,
            ResolucionDocumento, RespuestaProveedor,
        },
    },
};

const ACCION_EMITIR: AccionAuditoria = AccionAuditoria::new("facturacion.documento.emitir");
const ACCION_REINTENTAR: AccionAuditoria = AccionAuditoria::new("facturacion.documento.reintentar");
const ACCION_RECIBIR: AccionAuditoria = AccionAuditoria::new("facturacion.recibida.cargar");
const ACCION_VINCULAR: AccionAuditoria = AccionAuditoria::new("facturacion.recibida.vincular");
const ACCION_RECHAZAR: AccionAuditoria = AccionAuditoria::new("facturacion.recibida.rechazar");
const ACCION_RADIAN: AccionAuditoria = AccionAuditoria::new("facturacion.radian.evento");

/// Máximo de documentos en un listado.
pub const LIMITE_DOCUMENTOS: u32 = 500;

/// Lo que usa una transacción de facturación: la de tesorería, los archivos y el outbox.
pub trait TxFacturacion:
    TxTesoreria + RepositorioArchivos + RegistroOutbox + RepositorioFacturacion
{
}

impl<T> TxFacturacion for T where
    T: TxTesoreria + RepositorioArchivos + RegistroOutbox + RepositorioFacturacion
{
}

fn validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

async fn abrir_fe<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    permiso: Permiso,
) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    ctx.exigir(permiso)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::FacturacionElectronica).await?;
    Ok(tx)
}

/// La entidad debe tener un perfil confirmado de facturador electrónico con canal.
async fn exigir_facturador<T: TxFacturacion>(tx: &mut T, fecha: NaiveDate) -> AppResult<()> {
    let perfil = perfil_confirmado_en(tx, fecha).await?;
    match perfil {
        Some(p)
            if p.parametros.facturador_electronico
                && p.parametros.canal_facturacion != CanalFacturacion::Ninguno =>
        {
            Ok(())
        }
        _ => Err(validacion(
            "la entidad no factura electrónicamente: confirme un perfil tributario de facturador \
             electrónico con su canal",
        )),
    }
}

async fn evento<T: TxFacturacion>(
    tx: &mut T,
    reloj: &impl Reloj,
    documento: DocumentoElectronicoId,
    tipo: &str,
    detalle: Option<String>,
) -> AppResult<()> {
    tx.insertar_evento_documento(
        documento,
        &EventoDocumento {
            tipo: tipo.into(),
            detalle,
            ocurrido_en: reloj.ahora(),
        },
    )
    .await
}

/// Emisor (la entidad) para el proveedor: sus datos y lo de su perfil tributario confirmado.
pub(crate) async fn emisor<T: TxFacturacion>(
    tx: &mut T,
    fecha: NaiveDate,
) -> AppResult<ParteDocumento> {
    let entidad = tx.datos_entidad().await?;
    let nit = entidad
        .nit
        .ok_or_else(|| validacion("la entidad necesita su NIT para facturar electrónicamente"))?;
    let (numero, dv) = match nit.split_once('-') {
        Some((n, d)) => (n.to_owned(), Some(d.to_owned())),
        None => (nit, None),
    };
    let perfil = perfil_confirmado_en(tx, fecha).await?.map(|p| p.parametros);
    Ok(ParteDocumento {
        documento: numero,
        dv,
        nombre: entidad.razon_social.unwrap_or(entidad.nombre),
        correo: entidad.correo,
        persona_natural: false,
        responsable_iva: perfil.as_ref().is_some_and(|p| p.responsable_iva),
        responsabilidades: perfil
            .map(|p| {
                p.responsabilidades
                    .iter()
                    .map(|r| r.as_str().to_owned())
                    .collect()
            })
            .unwrap_or_default(),
        direccion: match (entidad.direccion, entidad.municipio_dane) {
            (Some(linea), Some(municipio_dane)) => Some(DireccionDocumento {
                linea,
                municipio_dane,
                departamento: entidad.departamento,
            }),
            _ => None,
        },
    })
}

/// Contraparte (cliente o proveedor) para el proveedor, con sus datos fiscales.
async fn contraparte<T: TxFacturacion>(
    tx: &mut T,
    persona: domain::PersonaId,
) -> AppResult<ParteDocumento> {
    let p = tx
        .persona(persona)
        .await?
        .ok_or(AppError::NoEncontrado("tercero"))?;
    let fiscal = tx.datos_fiscales(persona).await?.unwrap_or_default();
    let documento = p
        .documento
        .map(|d| d.as_str().to_owned())
        .unwrap_or_default();
    let persona_natural = p.tipo == domain::TipoPersona::Natural;
    // A una persona jurídica la identifica su NIT, con su dígito de verificación.
    let dv = (!persona_natural)
        .then(|| domain::Nit::new(&documento).ok())
        .flatten()
        .map(|n| n.dv().to_string());
    Ok(ParteDocumento {
        documento,
        dv,
        nombre: p.nombre,
        correo: fiscal
            .correo_facturacion
            .clone()
            .or_else(|| p.correo.map(|c| c.as_str().to_owned())),
        persona_natural,
        responsable_iva: fiscal.regimen_iva == Some(domain::contabilidad::RegimenIva::Responsable),
        responsabilidades: fiscal
            .responsabilidades
            .iter()
            .map(|r| r.as_str().to_owned())
            .collect(),
        direccion: match (fiscal.direccion_fiscal, fiscal.municipio_fiscal_dane) {
            (Some(linea), Some(municipio_dane)) => Some(DireccionDocumento {
                linea,
                municipio_dane,
                departamento: None,
            }),
            _ => None,
        },
    })
}

/// Aplica la respuesta del proveedor (o la falla de transmisión) al documento.
async fn aplicar_respuesta<T: TxFacturacion, A: AlmacenArchivos>(
    tx: &mut T,
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    d: &mut DocumentoElectronico,
    resultado: AppResult<RespuestaProveedor>,
) -> AppResult<()> {
    let ahora = reloj.ahora();
    d.intentos += 1;
    match resultado {
        Ok(r) => {
            d.estado = d.estado.transicionar(r.estado)?;
            d.cufe = r.cufe.or(d.cufe.take());
            d.id_externo = r.id_externo.or(d.id_externo.take());
            d.respuesta = r.mensaje.map(|m| m.chars().take(4000).collect());
            d.enviado_en = d.enviado_en.or(Some(ahora));
            if d.estado == EstadoDocumentoElectronico::Validado {
                d.validado_en = Some(ahora);
            }
            if let Some(xml) = r.xml.filter(|x| !x.is_empty()) {
                let subido = ArchivoSubido {
                    nombre_original: NombreArchivo::new(&format!(
                        "{}-{}{}.xml",
                        d.tipo.as_str(),
                        d.prefijo,
                        d.numero.unwrap_or_default()
                    ))?,
                    clasificacion: ClasificacionArchivo::Interno,
                    contenido: xml,
                };
                let archivo = nuevo_archivo(ctx, almacen, reloj, &subido, TipoArchivo::Xml);
                almacen.guardar(&archivo.clave, &subido.contenido).await?;
                if let Err(e) = registrar_en(tx, &archivo).await {
                    borrar_huerfano(almacen, ctx, &archivo).await;
                    return Err(e);
                }
                d.xml_archivo_id = Some(archivo.id);
            }
            let tipo = d.estado.as_str().to_owned();
            evento(tx, reloj, d.id, &tipo, d.respuesta.clone()).await?;
        }
        Err(AppError::Infraestructura(e)) => {
            // Caída del proveedor o de la DIAN: queda en contingencia para reintentar.
            d.estado = d
                .estado
                .transicionar(EstadoDocumentoElectronico::Contingencia)?;
            d.respuesta = Some("no se pudo transmitir: se reintentará".into());
            tracing::warn!(tenant_id = %ctx.tenant_id, documento = %d.id, error = %e, "transmisión fallida");
            evento(tx, reloj, d.id, "contingencia", None).await?;
        }
        Err(e) => return Err(e),
    }
    tx.actualizar_documento(d).await
}

/// Envía al adquirente el aviso del documento validado (correo con el CUFE; el XML y el PDF los
/// descarga la administración: el correo saliente aún no lleva adjuntos).
async fn avisar_adquirente<T: TxFacturacion>(
    tx: &mut T,
    d: &DocumentoElectronico,
    adquirente: &ParteDocumento,
) -> AppResult<()> {
    let (Some(correo), Some(cufe)) = (adquirente.correo.as_deref(), d.cufe.as_deref()) else {
        return Ok(());
    };
    let Ok(para) = domain::DireccionCorreo::new(correo) else {
        return Ok(());
    };
    let entidad = tx.datos_entidad().await?.nombre;
    tx.encolar(EventoSaliente {
        prioridad: PrioridadSalida::Evento,
        mensaje: MensajeOutbox::Correo(CorreoSaliente {
            para,
            asunto: format!(
                "{} {}{} · {entidad}",
                nombre_tipo(d.tipo),
                d.prefijo,
                d.numero.unwrap_or_default()
            ),
            texto: format!(
                "{entidad} le envía {} {}{} por {}.\nCUFE: {cufe}",
                nombre_tipo(d.tipo).to_lowercase(),
                d.prefijo,
                d.numero.unwrap_or_default(),
                d.total.valor()
            ),
            html: None,
        }),
    })
    .await
}

fn nombre_tipo(t: TipoDocumentoElectronico) -> &'static str {
    match t {
        TipoDocumentoElectronico::Factura => "Factura electrónica",
        TipoDocumentoElectronico::NotaCredito => "Nota crédito",
        TipoDocumentoElectronico::NotaDebito => "Nota débito",
        TipoDocumentoElectronico::DocumentoSoporte => "Documento soporte",
        TipoDocumentoElectronico::NotaAjusteDs => "Nota de ajuste",
    }
}

/// Lo que identifica un documento electrónico por crear.
struct Pedido {
    tipo: TipoDocumentoElectronico,
    origen: Origen,
    fecha: NaiveDate,
    total: Monto,
    referencia: Option<DocumentoElectronicoId>,
}

/// Documento nuevo con número (si su tipo usa resolución) o el existente del mismo origen.
async fn preparar_documento<T: TxFacturacion, P: ProveedorFacturacion>(
    tx: &mut T,
    proveedor: &P,
    reloj: &impl Reloj,
    pedido: Pedido,
) -> AppResult<(DocumentoElectronico, bool)> {
    let Pedido {
        tipo,
        origen: origen_documento,
        fecha,
        total,
        referencia,
    } = pedido;
    if let Some(d) = tx.documento_por_origen(tipo, &origen_documento).await? {
        return Ok((d, false));
    }
    let (resolucion_id, prefijo, numero) = match tipo.numeracion() {
        Some(t) => {
            let r = tx
                .resoluciones()
                .await?
                .into_iter()
                .filter(|r| {
                    r.tipo == t && r.activa && r.vigente_desde <= fecha && fecha <= r.vigente_hasta
                })
                .max_by_key(|r| r.vigente_desde)
                .ok_or_else(|| {
                    validacion(format!(
                        "no hay una resolución de numeración activa y vigente para {}",
                        t.as_str()
                    ))
                })?;
            let ultimo = tx.ultimo_numero(r.id).await?;
            let n = siguiente_numero(ultimo, r.desde, r.hasta)?;
            if rango_por_agotarse(n, r.desde, r.hasta) {
                tracing::info!(resolucion = %r.id, numero = n, "el rango de numeración está por agotarse");
            }
            (Some(r.id), r.prefijo, Some(n))
        }
        None => (None, String::new(), None),
    };
    // Con el canal `propio` transmite el motor propio, sea cual sea el adaptador por defecto.
    let propio = perfil_confirmado_en(tx, fecha)
        .await?
        .is_some_and(|p| p.parametros.canal_facturacion == CanalFacturacion::Propio);
    let d = DocumentoElectronico {
        id: DocumentoElectronicoId::nuevo(),
        tipo,
        origen: origen_documento,
        resolucion_id,
        prefijo,
        numero,
        referencia_id: referencia,
        cufe: None,
        estado: EstadoDocumentoElectronico::PorEnviar,
        proveedor: if propio {
            "propio".into()
        } else {
            proveedor.nombre().into()
        },
        id_externo: None,
        xml_archivo_id: None,
        respuesta: None,
        intentos: 0,
        total,
        creado_en: reloj.ahora(),
        enviado_en: None,
        validado_en: None,
    };
    tx.insertar_documento(&d).await?;
    evento(tx, reloj, d.id, "creado", None).await?;
    Ok((d, true))
}

/// Datos para transmitir un documento según su origen.
async fn armar_envio<T: TxFacturacion>(
    tx: &mut T,
    d: &DocumentoElectronico,
) -> AppResult<DocumentoAEmitir> {
    let resolucion = match d.resolucion_id {
        Some(r) => tx.resoluciones().await?.into_iter().find(|x| x.id == r),
        None => None,
    };
    let referencia = match d.referencia_id {
        Some(r) => tx.documento(r).await?,
        None => None,
    };
    let base =
        |fecha: NaiveDate, emisor: ParteDocumento, adquirente: ParteDocumento| DocumentoAEmitir {
            id: d.id,
            tipo: d.tipo,
            prefijo: d.prefijo.clone(),
            numero: d.numero.unwrap_or_default(),
            fecha,
            emisor,
            adquirente,
            lineas: Vec::new(),
            subtotal: rust_decimal::Decimal::ZERO,
            iva: rust_decimal::Decimal::ZERO,
            total: rust_decimal::Decimal::ZERO,
            clave_tecnica: resolucion.as_ref().and_then(|x| x.clave_tecnica.clone()),
            resolucion: resolucion.as_ref().map(|x| ResolucionDocumento {
                numero: x.numero_resolucion.clone(),
                vigente_desde: x.vigente_desde,
                vigente_hasta: x.vigente_hasta,
                desde: x.desde,
                hasta: x.hasta,
            }),
            vencimiento: None,
            referencia_cufe: referencia.as_ref().and_then(|x| x.cufe.clone()),
            referencia_numero: referencia
                .as_ref()
                .map(|x| format!("{}{}", x.prefijo, x.numero.unwrap_or_default())),
            referencia_fecha: referencia.as_ref().map(|x| x.creado_en.date_naive()),
            credenciales: None,
        };
    match d.origen.tipo.as_str() {
        eventos_origen::FACTURA_VENTA => {
            let f = tx
                .factura_venta(FacturaVentaId::from_uuid(d.origen.id))
                .await?
                .ok_or(AppError::NoEncontrado("factura de venta"))?;
            let emisor = emisor(tx, f.fecha).await?;
            let adquirente = contraparte(tx, f.persona_id).await?;
            let mut lineas = Vec::with_capacity(f.lineas.len());
            for l in &f.lineas {
                let unidad = match l.item_id {
                    Some(i) => tx.item(i).await?.map(|i| i.unidad),
                    None => None,
                };
                lineas.push(LineaDocumento {
                    descripcion: l.descripcion.clone(),
                    unidad: unidad.unwrap_or_else(|| "94".into()),
                    cantidad: l.cantidad,
                    valor_unitario: l.valor_unitario.valor(),
                    subtotal: l.subtotal.valor(),
                    tarifa_iva: l.tarifa_iva,
                    iva: l.iva.valor(),
                });
            }
            Ok(DocumentoAEmitir {
                lineas,
                subtotal: f.subtotal.valor(),
                iva: f.iva.valor(),
                total: f.total.valor(),
                vencimiento: (f.vencimiento > f.fecha).then_some(f.vencimiento),
                ..base(f.fecha, emisor, adquirente)
            })
        }
        eventos_origen::FACTURA_PROVEEDOR => {
            let f = tx
                .factura_proveedor(FacturaProveedorId::from_uuid(d.origen.id))
                .await?
                .ok_or(AppError::NoEncontrado("factura de proveedor"))?;
            // En el documento soporte la entidad (compradora) lo emite; el vendedor va como
            // contraparte.
            let emisor = emisor(tx, f.fecha).await?;
            let vendedor = contraparte(tx, f.persona_id).await?;
            Ok(DocumentoAEmitir {
                lineas: f
                    .lineas
                    .iter()
                    .map(|l| LineaDocumento {
                        descripcion: l.descripcion.clone(),
                        unidad: "94".into(),
                        cantidad: rust_decimal::Decimal::ONE,
                        valor_unitario: l.base.valor(),
                        subtotal: l.base.valor(),
                        tarifa_iva: rust_decimal::Decimal::ZERO,
                        iva: l.iva.valor(),
                    })
                    .collect(),
                subtotal: f.subtotal.valor(),
                iva: f.iva.valor(),
                total: f.subtotal.valor() + f.iva.valor(),
                vencimiento: (f.vencimiento > f.fecha).then_some(f.vencimiento),
                ..base(f.fecha, emisor, vendedor)
            })
        }
        otro => Err(validacion(format!(
            "origen de documento no admitido: {otro}"
        ))),
    }
}

/// Contexto de un secreto de la entidad en el cofre (lo ata a la entidad y a su uso).
pub(crate) fn contexto_secreto(ctx: &Contexto, uso: &str) -> String {
    format!("facturacion.{uso}:{}", ctx.tenant_id)
}

/// Credenciales descifradas para transmitir con el motor propio en el `ambiente`.
pub(crate) async fn cargar_credenciales<T: TxFacturacion, C: Cofre>(
    tx: &mut T,
    cofre: &C,
    reloj: &impl Reloj,
    ctx: &Contexto,
    config: &ConfiguracionFacturacion,
    ambiente: AmbienteDian,
) -> AppResult<CredencialesFacturador> {
    let certificado = tx
        .certificado_activo()
        .await?
        .ok_or_else(|| validacion("cargue el certificado de firma (.p12) de la entidad"))?;
    if certificado.vigente_hasta <= reloj.ahora() {
        return Err(validacion(
            "el certificado de firma está vencido: cargue uno vigente",
        ));
    }
    let p12 = cofre.descifrar(
        &contexto_secreto(ctx, "certificado"),
        &certificado.p12_cifrado,
    )?;
    let texto = |bytes: Vec<u8>| {
        String::from_utf8(bytes).map_err(|_| validacion("secreto de facturación ilegible"))
    };
    let contrasena = texto(cofre.descifrar(
        &contexto_secreto(ctx, "contrasena"),
        &certificado.contrasena_cifrada,
    )?)?;
    let pin = texto(cofre.descifrar(&contexto_secreto(ctx, "pin"), &config.pin_cifrado)?)?;
    use chrono::Datelike;
    let consecutivo_archivo = tx
        .siguiente_consecutivo_archivo(reloj.ahora().year())
        .await?;
    Ok(CredencialesFacturador {
        ambiente,
        software_id: config.software_id.clone(),
        pin,
        p12,
        contrasena,
        test_set_id: config.test_set_id.clone(),
        consecutivo_archivo,
    })
}

/// Credenciales del motor propio si la entidad factura con él (perfil con canal `propio`): exige
/// la habilitación en producción. Con otro canal, `None` (lo transmite el proveedor).
async fn credenciales<T: TxFacturacion, C: Cofre>(
    tx: &mut T,
    cofre: &C,
    reloj: &impl Reloj,
    ctx: &Contexto,
    fecha: NaiveDate,
) -> AppResult<Option<CredencialesFacturador>> {
    let canal = perfil_confirmado_en(tx, fecha)
        .await?
        .map(|p| p.parametros.canal_facturacion);
    if canal != Some(CanalFacturacion::Propio) {
        return Ok(None);
    }
    let config = tx.configuracion_facturacion().await?.ok_or_else(|| {
        validacion("la entidad factura con el motor propio pero no ha registrado su software")
    })?;
    if config.estado != EstadoHabilitacion::Produccion {
        return Err(validacion(
            "la entidad aún no está en producción con el motor propio: termine su habilitación",
        ));
    }
    cargar_credenciales(tx, cofre, reloj, ctx, &config, AmbienteDian::Produccion)
        .await
        .map(Some)
}

/// Transmite el documento si está por enviar o en contingencia y aplica la respuesta.
async fn transmitir<T: TxFacturacion, P: ProveedorFacturacion, C: Cofre, A: AlmacenArchivos>(
    tx: &mut T,
    (proveedor, cofre): (&P, &C),
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    d: &mut DocumentoElectronico,
) -> AppResult<()> {
    if !d.estado.transmisible() {
        return Ok(());
    }
    let mut envio = armar_envio(tx, d).await?;
    envio.credenciales = credenciales(tx, cofre, reloj, ctx, envio.fecha).await?;
    let resultado = proveedor.emitir(&envio).await;
    aplicar_respuesta(tx, almacen, reloj, ctx, d, resultado).await?;
    if d.estado == EstadoDocumentoElectronico::Validado {
        avisar_adquirente(tx, d, &envio.adquirente).await?;
    }
    Ok(())
}

fn auditar(d: &DocumentoElectronico) -> AppResult<DetalleAuditoria> {
    DetalleAuditoria::vacio()
        .con("tipo", d.tipo.as_str())?
        .con("estado", d.estado.as_str())?
        .con("numero", d.numero)
}

/// Emite la factura electrónica de una factura de venta ya emitida (contabilizada). Pedirla otra
/// vez devuelve el mismo documento (y lo reintenta si quedó en contingencia).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.emitir`; [`AppError::ModuloNoHabilitado`] sin
/// `facturacion_electronica`; [`AppError::NoEncontrado`]; [`AppError::Validacion`] sin perfil de
/// facturador, sin resolución vigente o con la factura en borrador o anulada;
/// [`AppError::Dominio`] si el rango se agotó.
pub async fn emitir_factura_electronica<U, P, C, A>(
    unidad: &U,
    (proveedor, cofre): (&P, &C),
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    factura: FacturaVentaId,
) -> AppResult<DocumentoElectronico>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    P: ProveedorFacturacion,
    C: Cofre,
    A: AlmacenArchivos,
{
    let mut tx = abrir_fe(unidad, reloj, ctx, Permiso::FacturacionEmitir).await?;
    let f = tx
        .factura_venta(factura)
        .await?
        .ok_or(AppError::NoEncontrado("factura de venta"))?;
    if matches!(
        f.estado,
        EstadoFacturaVenta::Borrador | EstadoFacturaVenta::Anulada
    ) {
        return Err(validacion(
            "solo una factura de venta emitida (y no anulada) se factura electrónicamente",
        ));
    }
    exigir_facturador(&mut tx, f.fecha).await?;
    let (mut d, _) = preparar_documento(
        &mut tx,
        proveedor,
        reloj,
        Pedido {
            tipo: TipoDocumentoElectronico::Factura,
            origen: origen(eventos_origen::FACTURA_VENTA, factura.as_uuid()),
            fecha: f.fecha,
            total: f.total,
            referencia: None,
        },
    )
    .await?;
    transmitir(&mut tx, (proveedor, cofre), almacen, reloj, ctx, &mut d).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EMITIR, "documento_electronico", d.id.as_uuid())
            .con_detalle(auditar(&d)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(d)
}

/// Anula una factura de venta con factura electrónica validada mediante una **nota crédito
/// electrónica** por el total (revierte su comprobante en la misma transacción).
///
/// # Errors
///
/// Los de [`emitir_factura_electronica`]; [`AppError::Conflicto`] si la factura no tiene factura
/// electrónica validada o ya está anulada.
pub async fn emitir_nota_credito<U, P, C, A>(
    unidad: &U,
    (proveedor, cofre): (&P, &C),
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    factura: FacturaVentaId,
    motivo: &str,
) -> AppResult<DocumentoElectronico>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    P: ProveedorFacturacion,
    C: Cofre,
    A: AlmacenArchivos,
{
    ctx.exigir(Permiso::FacturacionFacturasGestionar)?;
    let mut tx = abrir_fe(unidad, reloj, ctx, Permiso::FacturacionEmitir).await?;
    let origen_factura = origen(eventos_origen::FACTURA_VENTA, factura.as_uuid());
    let fe = tx
        .documento_por_origen(TipoDocumentoElectronico::Factura, &origen_factura)
        .await?
        .filter(|d| d.estado == EstadoDocumentoElectronico::Validado)
        .ok_or_else(|| {
            AppError::Conflicto(
                "la factura no tiene factura electrónica validada: anúlela como factura de venta"
                    .into(),
            )
        })?;
    let f = super::ventas::anular_factura_venta_en(&mut tx, reloj, ctx, factura, motivo).await?;
    let (mut d, _) = preparar_documento(
        &mut tx,
        proveedor,
        reloj,
        Pedido {
            tipo: TipoDocumentoElectronico::NotaCredito,
            origen: origen_factura,
            fecha: reloj.ahora().date_naive(),
            total: f.total,
            referencia: Some(fe.id),
        },
    )
    .await?;
    transmitir(&mut tx, (proveedor, cofre), almacen, reloj, ctx, &mut d).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EMITIR, "documento_electronico", d.id.as_uuid())
            .con_detalle(auditar(&d)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(d)
}

/// Emite el **documento soporte** de una factura de proveedor (compra a un no obligado a
/// facturar). La factura debe estar causada y no traer CUFE (si lo trae, ya es electrónica).
///
/// # Errors
///
/// Los de [`emitir_factura_electronica`]; [`AppError::Validacion`] si la factura trae CUFE, no
/// está causada o está anulada.
pub async fn emitir_documento_soporte<U, P, C, A>(
    unidad: &U,
    (proveedor, cofre): (&P, &C),
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    factura: FacturaProveedorId,
) -> AppResult<DocumentoElectronico>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    P: ProveedorFacturacion,
    C: Cofre,
    A: AlmacenArchivos,
{
    let mut tx = abrir_fe(unidad, reloj, ctx, Permiso::FacturacionEmitir).await?;
    let f = tx
        .factura_proveedor(factura)
        .await?
        .ok_or(AppError::NoEncontrado("factura de proveedor"))?;
    if f.cufe.is_some() {
        return Err(validacion(
            "la factura ya es electrónica (trae CUFE): no lleva documento soporte",
        ));
    }
    // Con contabilidad la compra debe estar causada; sin ella basta con registrarla (no hay
    // causación).
    let exige_causada = super::con_contabilidad(&mut tx, reloj).await?;
    let admitida = match f.estado {
        EstadoFacturaProveedor::Anulada => false,
        EstadoFacturaProveedor::Registrada => !exige_causada,
        _ => true,
    };
    if !admitida {
        return Err(validacion(if exige_causada {
            "el documento soporte se emite de una compra causada y no anulada"
        } else {
            "el documento soporte se emite de una compra registrada y no anulada"
        }));
    }
    exigir_facturador(&mut tx, f.fecha).await?;
    let (mut d, _) = preparar_documento(
        &mut tx,
        proveedor,
        reloj,
        Pedido {
            tipo: TipoDocumentoElectronico::DocumentoSoporte,
            origen: origen(eventos_origen::FACTURA_PROVEEDOR, factura.as_uuid()),
            fecha: f.fecha,
            total: Monto::new(f.subtotal.valor() + f.iva.valor())?,
            referencia: None,
        },
    )
    .await?;
    transmitir(&mut tx, (proveedor, cofre), almacen, reloj, ctx, &mut d).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EMITIR, "documento_electronico", d.id.as_uuid())
            .con_detalle(auditar(&d)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(d)
}

/// Reintenta un documento por enviar o en contingencia, o consulta el estado de uno enviado.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.emitir`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya está validado o rechazado (QA-015: se corrige con otro
/// documento o una nota).
pub async fn reintentar_documento<U, P, C, A>(
    unidad: &U,
    (proveedor, cofre): (&P, &C),
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: DocumentoElectronicoId,
) -> AppResult<DocumentoElectronico>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    P: ProveedorFacturacion,
    C: Cofre,
    A: AlmacenArchivos,
{
    let mut tx = abrir_fe(unidad, reloj, ctx, Permiso::FacturacionEmitir).await?;
    let mut d = tx
        .documento(id)
        .await?
        .ok_or(AppError::NoEncontrado("documento electrónico"))?;
    match d.estado {
        EstadoDocumentoElectronico::PorEnviar | EstadoDocumentoElectronico::Contingencia => {
            transmitir(&mut tx, (proveedor, cofre), almacen, reloj, ctx, &mut d).await?;
        }
        EstadoDocumentoElectronico::Enviado => {
            let externo = d
                .id_externo
                .clone()
                .ok_or_else(|| validacion("el documento enviado no tiene id del proveedor"))?;
            let fecha = reloj.ahora().date_naive();
            let credenciales = credenciales(&mut tx, cofre, reloj, ctx, fecha).await?;
            let resultado = proveedor.consultar(&externo, credenciales.as_ref()).await;
            aplicar_respuesta(&mut tx, almacen, reloj, ctx, &mut d, resultado).await?;
        }
        EstadoDocumentoElectronico::Validado | EstadoDocumentoElectronico::Rechazado => {
            return Err(AppError::Conflicto(format!(
                "el documento está {}: no se reintenta",
                d.estado.as_str()
            )));
        }
    }
    tx.registrar(
        EventoAuditoria::ok(ACCION_REINTENTAR, "documento_electronico", id.as_uuid())
            .con_detalle(auditar(&d)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(d)
}

/// Documentos electrónicos por fecha descendente.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.ver`.
pub async fn documentos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroDocumentos,
) -> AppResult<Vec<DocumentoElectronico>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    super::abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionVer)
        .await?
        .documentos(&filtro, LIMITE_DOCUMENTOS)
        .await
}

/// Documento con sus eventos.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.ver`; [`AppError::NoEncontrado`].
pub async fn documento<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: DocumentoElectronicoId,
) -> AppResult<(DocumentoElectronico, Vec<EventoDocumento>)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    let mut tx = super::abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionVer).await?;
    let d = tx
        .documento(id)
        .await?
        .ok_or(AppError::NoEncontrado("documento electrónico"))?;
    let eventos = tx.eventos_documento(id).await?;
    Ok((d, eventos))
}

/// Representación gráfica en PDF (con el CUFE).
///
/// # Errors
///
/// Los de [`documento`]; [`AppError::Infraestructura`] si falla el PDF.
pub async fn representacion_grafica<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: DocumentoElectronicoId,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    G: GeneradorActas,
{
    let mut tx = super::abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionVer).await?;
    let d = tx
        .documento(id)
        .await?
        .ok_or(AppError::NoEncontrado("documento electrónico"))?;
    pdf_documento(&mut tx, generador, reloj, &d).await
}

/// PDF de un documento dentro de la transacción (lo usa también el paquete del periodo).
pub(crate) async fn pdf_documento<T: TxFacturacion, G: GeneradorActas>(
    tx: &mut T,
    generador: &G,
    reloj: &impl Reloj,
    d: &DocumentoElectronico,
) -> AppResult<Vec<u8>> {
    let envio = armar_envio(tx, d).await?;
    let mut filas: Vec<Vec<String>> = envio
        .lineas
        .iter()
        .map(|l| {
            vec![
                l.descripcion.clone(),
                l.cantidad.normalize().to_string(),
                l.subtotal.round_dp(2).to_string(),
                l.iva.round_dp(2).to_string(),
            ]
        })
        .collect();
    filas.push(vec![
        "Total".into(),
        String::new(),
        envio.subtotal.round_dp(2).to_string(),
        envio.iva.round_dp(2).to_string(),
    ]);
    generador
        .tabla(&TablaPdf {
            titulo: format!(
                "{} {}{}",
                nombre_tipo(d.tipo),
                d.prefijo,
                d.numero.unwrap_or_default()
            ),
            copropiedad: format!(
                "{} · NIT {}{}",
                envio.emisor.nombre,
                envio.emisor.documento,
                envio.emisor.dv.map(|v| format!("-{v}")).unwrap_or_default()
            ),
            subtitulo: format!(
                "{} · {} · Estado: {} · CUFE: {}",
                envio.adquirente.nombre,
                envio.fecha,
                d.estado.as_str(),
                d.cufe.as_deref().unwrap_or("pendiente")
            ),
            columnas: ["Descripción", "Cantidad", "Subtotal", "IVA"]
                .map(str::to_owned)
                .to_vec(),
            filas,
            generada: reloj.ahora().format("%Y-%m-%d %H:%M UTC").to_string(),
        })
        .await
}

// ---------------------------------------------------------------------------------------------
// Recepción de facturas de proveedores (fase 4)
// ---------------------------------------------------------------------------------------------

/// Carga una factura electrónica recibida (su XML): lee emisor, número, fechas, CUFE y valores,
/// guarda el archivo y la deja pendiente de registrar. Si el emisor es un tercero de la entidad
/// (por su NIT), queda asociado.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.recibir`; [`AppError::Dominio`] o
/// [`AppError::Validacion`] si el archivo no es una factura electrónica legible;
/// [`AppError::Conflicto`] si el CUFE o el archivo ya se cargaron.
pub async fn cargar_factura_recibida<U, A, L>(
    unidad: &U,
    almacen: &A,
    lector: &L,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nombre: NombreArchivo,
    contenido: Vec<u8>,
) -> AppResult<FacturaRecibida>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    A: AlmacenArchivos,
    L: LectorFacturas,
{
    ctx.exigir(Permiso::FacturacionRecibir)?;
    let tipo = TipoArchivo::validar_xml(&contenido)?;
    let leida = lector.leer(&contenido)?;
    let sha256 = sha256_hex(&contenido);
    let subido = ArchivoSubido {
        nombre_original: nombre,
        clasificacion: ClasificacionArchivo::Interno,
        contenido,
    };
    let archivo = nuevo_archivo(ctx, almacen, reloj, &subido, tipo);
    let mut tx = super::abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionRecibir).await?;
    let persona = match domain::Documento::new(&leida.emisor_nit) {
        Ok(doc) => tx.persona_por_documento(&doc).await?.map(|p| p.id),
        Err(_) => None,
    };
    almacen.guardar(&archivo.clave, &subido.contenido).await?;
    let resultado = async {
        registrar_en(&mut tx, &archivo).await?;
        let f = FacturaRecibida {
            id: FacturaRecibidaId::nuevo(),
            archivo_id: archivo.id,
            sha256,
            emisor_nit: leida.emisor_nit,
            emisor_nombre: leida.emisor_nombre,
            prefijo: leida.prefijo,
            numero: leida.numero,
            fecha: leida.fecha,
            vencimiento: leida.vencimiento,
            cufe: leida.cufe.to_lowercase(),
            subtotal: Monto::new(leida.subtotal.round_dp(2))?,
            iva: Monto::new(leida.iva.round_dp(2))?,
            total: Monto::new(leida.total.round_dp(2))?,
            estado: EstadoFacturaRecibida::Pendiente,
            persona_id: persona,
            factura_proveedor_id: None,
            motivo_rechazo: None,
            recibida_por: ctx.actor_id,
            recibida_en: reloj.ahora(),
        };
        tx.insertar_factura_recibida(&f).await?;
        let detalle = DetalleAuditoria::vacio().con("con_tercero", f.persona_id.is_some())?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_RECIBIR, "factura_recibida", f.id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
        tx.confirmar().await?;
        Ok(f)
    }
    .await;
    if resultado.is_err() {
        borrar_huerfano(almacen, ctx, &archivo).await;
    }
    resultado
}

/// Facturas recibidas (del estado, si se da), las más recientes primero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.ver`.
pub async fn facturas_recibidas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    estado: Option<EstadoFacturaRecibida>,
) -> AppResult<Vec<FacturaRecibida>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    super::abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionVer)
        .await?
        .facturas_recibidas(
            &FiltroRecibidas {
                estado,
                ..FiltroRecibidas::default()
            },
            LIMITE_DOCUMENTOS,
        )
        .await
}

/// Factura recibida con sus eventos RADIAN.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.ver`; [`AppError::NoEncontrado`].
pub async fn factura_recibida<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaRecibidaId,
) -> AppResult<(FacturaRecibida, Vec<RegistroRadian>)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    let mut tx = super::abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionVer).await?;
    let f = tx
        .factura_recibida(id)
        .await?
        .ok_or(AppError::NoEncontrado("factura recibida"))?;
    let eventos = tx.eventos_radian(id).await?;
    Ok((f, eventos))
}

async fn pendiente<T: TxFacturacion>(
    tx: &mut T,
    id: FacturaRecibidaId,
) -> AppResult<FacturaRecibida> {
    let f = tx
        .factura_recibida(id)
        .await?
        .ok_or(AppError::NoEncontrado("factura recibida"))?;
    if f.estado != EstadoFacturaRecibida::Pendiente {
        return Err(AppError::Conflicto(format!(
            "la factura recibida ya está {}",
            f.estado.as_str()
        )));
    }
    Ok(f)
}

/// Marca registrada una factura recibida con la factura de proveedor que se creó a partir de
/// ella (la de proveedor debe traer el mismo CUFE).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.recibir`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya no está pendiente; [`AppError::Validacion`] si el CUFE no
/// coincide.
pub async fn vincular_factura_recibida<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaRecibidaId,
    factura_proveedor: FacturaProveedorId,
) -> AppResult<FacturaRecibida>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    let mut tx = super::abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionRecibir).await?;
    let mut f = pendiente(&mut tx, id).await?;
    let fp = tx
        .factura_proveedor(factura_proveedor)
        .await?
        .ok_or(AppError::NoEncontrado("factura de proveedor"))?;
    if fp.cufe.as_deref().map(str::to_lowercase).as_deref() != Some(f.cufe.as_str()) {
        return Err(validacion(
            "la factura de proveedor debe traer el CUFE de la factura recibida",
        ));
    }
    f.estado = EstadoFacturaRecibida::Registrada;
    f.factura_proveedor_id = Some(factura_proveedor);
    f.persona_id = f.persona_id.or(Some(fp.persona_id));
    tx.actualizar_factura_recibida(&f).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_VINCULAR,
        "factura_recibida",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(f)
}

/// Rechaza una factura recibida (no se registrará). El reclamo ante la DIAN es un evento RADIAN
/// aparte.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.recibir`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya no está pendiente; [`AppError::Validacion`] si el motivo es
/// corto.
pub async fn rechazar_factura_recibida<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaRecibidaId,
    motivo: &str,
) -> AppResult<FacturaRecibida>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    let motivo = texto("motivo", motivo, 500)?;
    if motivo.chars().count() < 5 {
        return Err(validacion("el motivo tiene al menos 5 caracteres"));
    }
    let mut tx = super::abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionRecibir).await?;
    let mut f = pendiente(&mut tx, id).await?;
    f.estado = EstadoFacturaRecibida::Rechazada;
    f.motivo_rechazo = Some(motivo);
    tx.actualizar_factura_recibida(&f).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_RECHAZAR,
        "factura_recibida",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(f)
}

/// Registra un evento RADIAN (acuse, recibo del bien, aceptación o reclamo) por el proveedor:
/// exige los eventos previos; el reclamo exige motivo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.recibir`; [`AppError::ModuloNoHabilitado`];
/// [`AppError::NoEncontrado`]; [`AppError::Validacion`] si faltan los eventos previos o el
/// motivo; [`AppError::Conflicto`] si el evento ya se registró.
pub async fn registrar_evento_radian<U, P>(
    unidad: &U,
    proveedor: &P,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaRecibidaId,
    tipo: EventoRadian,
    motivo: Option<&str>,
) -> AppResult<RegistroRadian>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    P: ProveedorFacturacion,
{
    let motivo = texto_opcional("motivo", motivo, 500)?;
    if tipo == EventoRadian::Reclamo && motivo.is_none() {
        return Err(validacion("el reclamo de una factura lleva su motivo"));
    }
    let mut tx = abrir_fe(unidad, reloj, ctx, Permiso::FacturacionRecibir).await?;
    let f = tx
        .factura_recibida(id)
        .await?
        .ok_or(AppError::NoEncontrado("factura recibida"))?;
    let previos = tx.eventos_radian(id).await?;
    let faltan: Vec<&str> = tipo
        .previos()
        .iter()
        .filter(|p| {
            !previos
                .iter()
                .any(|e| e.tipo == **p && e.estado == "enviado")
        })
        .map(|p| p.as_str())
        .collect();
    if !faltan.is_empty() {
        return Err(validacion(format!("antes registre: {}", faltan.join(", "))));
    }
    let respuesta = proveedor
        .evento_radian(&f.cufe, tipo, motivo.as_deref())
        .await?;
    let registro = RegistroRadian {
        id: uuid::Uuid::now_v7(),
        factura_recibida_id: id,
        tipo,
        estado: if respuesta.estado == EstadoDocumentoElectronico::Rechazado {
            "rechazado".into()
        } else {
            "enviado".into()
        },
        id_externo: respuesta.id_externo,
        motivo,
        respuesta: respuesta.mensaje,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
    };
    tx.insertar_evento_radian(&registro).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", tipo.as_str())?
        .con("estado", registro.estado.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RADIAN, "factura_recibida", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(registro)
}
