//! Eventos RADIAN (anexo técnico 1.9, §6.5): la entidad que **recibe** una factura electrónica
//! registra el acuse de recibo, el recibo del bien o la prestación del servicio, la aceptación
//! expresa o el reclamo; la que la **expidió** registra la aceptación tácita cuando el adquirente
//! calló tres días hábiles después del recibo del bien. Con el canal `propio` el evento es un
//! `ApplicationResponse` firmado por la entidad; sin él, lo registra el proveedor configurado.
//!
//! Reglas: el orden acuse → recibo del bien → aceptación o reclamo se exige aquí (la DIAN lo
//! vuelve a exigir); un evento de cada tipo por factura; el reclamo lleva su concepto; el acuse y
//! el recibo del bien identifican a quien recibió. El XML firmado se conserva en el almacén.

use chrono::Datelike;
use domain::{
    DocumentoElectronicoId, FacturaRecibidaId, Nit, Permiso,
    tributario::{
        ConceptoReclamo, EstadoDocumentoElectronico, EventoRadian, TipoDocumentoElectronico,
    },
};

use super::documentos::{
    Expedicion, TxFacturacion, abrir_fe, armar_envio, credenciales, emisor, guardar_xml, validacion,
};
use crate::casos_uso::contabilidad::{texto, texto_opcional};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        AlmacenArchivos, GeneradorActas, Reloj, Transaccion, UnidadDeTrabajo,
        archivos::Empaquetador,
        cofre::Cofre,
        tributario::{
            EventoAEmitir, ParteDocumento, PersonaRecibe, ProveedorFacturacion, RegistroRadian,
            RepositorioTributario, RespuestaProveedor,
        },
    },
};

const ACCION_RADIAN: AccionAuditoria = AccionAuditoria::new("facturacion.radian.evento");

/// NIT y nombre de la DIAN: el receptor de la aceptación tácita (anexo §8.5.2, AAG01e).
const NIT_DIAN: &str = "800197268";
const DV_NIT_DIAN: &str = "4";
const NOMBRE_DIAN: &str = "Unidad Especial Dirección de Impuestos y Aduanas Nacionales";

/// Lo que se declara al registrar un evento sobre una factura recibida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosEvento {
    pub tipo: EventoRadian,
    /// Observaciones (en el reclamo, el detalle de lo reclamado).
    pub motivo: Option<String>,
    /// Obligatorio en el reclamo.
    pub concepto_reclamo: Option<ConceptoReclamo>,
    /// Obligatorio en el acuse y el recibo del bien.
    pub quien_recibe: Option<PersonaRecibe>,
}

fn validar_quien_recibe(p: PersonaRecibe) -> AppResult<PersonaRecibe> {
    let documento = texto("documento", &p.documento, 20)?;
    if !documento.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(validacion(
            "el documento de quien recibe va sin puntos ni guiones",
        ));
    }
    Ok(PersonaRecibe {
        documento,
        nombres: texto("nombres", &p.nombres, 100)?,
        apellidos: texto("apellidos", &p.apellidos, 100)?,
        cargo: texto_opcional("cargo", p.cargo.as_deref(), 100)?,
        area: texto_opcional("area", p.area.as_deref(), 100)?,
        tipo_documento: p.tipo_documento,
    })
}

impl DatosEvento {
    fn validar(self) -> AppResult<Self> {
        if self.tipo.es_del_facturador() {
            return Err(validacion(
                "la aceptación tácita la registra el facturador sobre su propia factura",
            ));
        }
        if self.tipo == EventoRadian::Reclamo && self.concepto_reclamo.is_none() {
            return Err(validacion("el reclamo de una factura lleva su concepto"));
        }
        let quien_recibe = match (self.tipo.exige_quien_recibe(), self.quien_recibe) {
            (true, None) => {
                return Err(validacion(
                    "el acuse y el recibo del bien identifican a la persona que recibió",
                ));
            }
            (true, Some(p)) => Some(validar_quien_recibe(p)?),
            (false, _) => None,
        };
        Ok(Self {
            motivo: texto_opcional("motivo", self.motivo.as_deref(), 500)?,
            concepto_reclamo: (self.tipo == EventoRadian::Reclamo)
                .then_some(self.concepto_reclamo)
                .flatten(),
            quien_recibe,
            tipo: self.tipo,
        })
    }
}

/// Número del evento: `EV` + año + consecutivo a seis dígitos (único por entidad).
async fn numero_evento<T: TxFacturacion>(tx: &mut T, anio: i32) -> AppResult<String> {
    Ok(format!(
        "EV{anio}{:06}",
        tx.siguiente_numero_evento(anio).await?
    ))
}

/// La DIAN como parte (receptor de la aceptación tácita).
fn dian() -> ParteDocumento {
    ParteDocumento {
        documento: NIT_DIAN.into(),
        dv: Some(DV_NIT_DIAN.into()),
        nombre: NOMBRE_DIAN.into(),
        correo: None,
        persona_natural: false,
        responsable_iva: false,
        responsabilidades: Vec::new(),
        direccion: None,
    }
}

/// Registro del evento con lo que respondió el emisor (motor propio o proveedor).
#[allow(clippy::too_many_arguments)]
async fn registrar<T, A>(
    tx: &mut T,
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    evento: &EventoAEmitir,
    referencia: (Option<FacturaRecibidaId>, Option<DocumentoElectronicoId>),
    motivo: Option<String>,
    respuesta: RespuestaProveedor,
) -> AppResult<RegistroRadian>
where
    T: TxFacturacion,
    A: AlmacenArchivos,
{
    let xml_archivo_id = match respuesta.xml.filter(|x| !x.is_empty()) {
        Some(xml) => Some(
            guardar_xml(
                tx,
                almacen,
                reloj,
                ctx,
                &format!("evento-{}-{}.xml", evento.tipo.codigo(), evento.numero),
                xml,
            )
            .await?,
        ),
        None => None,
    };
    let registro = RegistroRadian {
        id: evento.id,
        factura_recibida_id: referencia.0,
        documento_id: referencia.1,
        tipo: evento.tipo,
        numero: Some(evento.numero.clone()),
        concepto_reclamo: evento.concepto_reclamo,
        xml_archivo_id,
        estado: if respuesta.estado == EstadoDocumentoElectronico::Rechazado {
            "rechazado".into()
        } else {
            "enviado".into()
        },
        id_externo: respuesta.cufe.or(respuesta.id_externo),
        motivo,
        respuesta: respuesta.mensaje.map(|m| m.chars().take(4000).collect()),
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
    };
    tx.insertar_evento_radian(&registro).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", registro.tipo.as_str())?
        .con("estado", registro.estado.as_str())?;
    let (entidad, id) = match referencia {
        (Some(f), _) => ("factura_recibida", f.as_uuid()),
        (None, Some(d)) => ("documento_electronico", d.as_uuid()),
        (None, None) => return Err(validacion("el evento necesita una factura")),
    };
    tx.registrar(EventoAuditoria::ok(ACCION_RADIAN, entidad, id).con_detalle(detalle))
        .await?;
    Ok(registro)
}

/// Registra un evento RADIAN sobre una factura recibida: exige los eventos previos, el concepto
/// en el reclamo y a quien recibió en el acuse y el recibo del bien.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.recibir`; [`AppError::ModuloNoHabilitado`];
/// [`AppError::NoEncontrado`]; [`AppError::Validacion`] si faltan los eventos previos o un dato;
/// [`AppError::Conflicto`] si el evento ya se registró.
pub async fn registrar_evento_radian<U, P, C, G, E, A>(
    unidad: &U,
    medios: &Expedicion<'_, P, C, G, E>,
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaRecibidaId,
    datos: DatosEvento,
) -> AppResult<RegistroRadian>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    P: ProveedorFacturacion,
    C: Cofre,
    G: GeneradorActas,
    E: Empaquetador,
    A: AlmacenArchivos,
{
    let datos = datos.validar()?;
    let mut tx = abrir_fe(unidad, reloj, ctx, Permiso::FacturacionRecibir).await?;
    let f = tx
        .factura_recibida(id)
        .await?
        .ok_or(AppError::NoEncontrado("factura recibida"))?;
    let previos = tx.eventos_radian(id).await?;
    let faltan: Vec<&str> = datos
        .tipo
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
    let hoy = reloj.ahora().date_naive();
    let nit = Nit::new(&f.emisor_nit).map_err(|_| {
        validacion("el NIT del emisor de la factura no es válido: no se puede emitir el evento")
    })?;
    let receptor = ParteDocumento {
        documento: nit.numero().to_owned(),
        dv: Some(nit.dv().to_string()),
        nombre: f.emisor_nombre.clone(),
        correo: None,
        persona_natural: false,
        responsable_iva: true,
        responsabilidades: Vec::new(),
        direccion: None,
    };
    let evento = EventoAEmitir {
        id: uuid::Uuid::now_v7(),
        tipo: datos.tipo,
        numero: numero_evento(&mut tx, hoy.year()).await?,
        fecha: hoy,
        generador: emisor(&mut tx, hoy).await?,
        receptor,
        factura_numero: format!("{}{}", f.prefijo, f.numero),
        factura_cufe: f.cufe.clone(),
        concepto_reclamo: datos.concepto_reclamo,
        quien_recibe: datos.quien_recibe,
        nota: datos.motivo.clone(),
        credenciales: credenciales(&mut tx, medios.cofre, reloj, ctx, hoy).await?,
    };
    let respuesta = medios.proveedor.evento_radian(&evento).await?;
    let registro = registrar(
        &mut tx,
        almacen,
        reloj,
        ctx,
        &evento,
        (Some(id), None),
        datos.motivo,
        respuesta,
    )
    .await?;
    tx.confirmar().await?;
    Ok(registro)
}

/// Registra la aceptación tácita (034) de una factura electrónica propia, validada y a crédito:
/// el facturador declara bajo juramento que el adquirente no la aceptó ni la reclamó en los tres
/// días hábiles siguientes al recibo del bien (la DIAN verifica el plazo y los eventos previos).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.emitir`; [`AppError::ModuloNoHabilitado`];
/// [`AppError::NoEncontrado`]; [`AppError::Validacion`] si no es una factura validada a crédito;
/// [`AppError::Conflicto`] si ya se registró.
pub async fn registrar_aceptacion_tacita<U, P, C, G, E, A>(
    unidad: &U,
    medios: &Expedicion<'_, P, C, G, E>,
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: DocumentoElectronicoId,
) -> AppResult<RegistroRadian>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    P: ProveedorFacturacion,
    C: Cofre,
    G: GeneradorActas,
    E: Empaquetador,
    A: AlmacenArchivos,
{
    let mut tx = abrir_fe(unidad, reloj, ctx, Permiso::FacturacionEmitir).await?;
    let d = tx
        .documento(id)
        .await?
        .ok_or(AppError::NoEncontrado("documento electrónico"))?;
    if d.tipo != TipoDocumentoElectronico::Factura
        || d.estado != EstadoDocumentoElectronico::Validado
    {
        return Err(validacion(
            "la aceptación tácita aplica a una factura electrónica validada por la DIAN",
        ));
    }
    let cufe = d
        .cufe
        .clone()
        .ok_or_else(|| validacion("la factura no tiene CUFE"))?;
    let envio = armar_envio(&mut tx, &d).await?;
    if envio.vencimiento.is_none_or(|v| v <= envio.fecha) {
        return Err(validacion(
            "solo una factura a crédito (con vencimiento posterior a su fecha) circula como \
             título valor",
        ));
    }
    let hoy = reloj.ahora().date_naive();
    let adquirente = &envio.adquirente;
    let nota = format!(
        "Manifiesto bajo la gravedad de juramento que transcurridos 3 días hábiles contados \
         desde la creación del Recibo de bienes y servicios {} identificado con documento {} no \
         manifestó expresamente la aceptación o rechazo de la referida factura, ni reclamó en \
         contra de su contenido.",
        adquirente.nombre, adquirente.documento
    );
    let evento = EventoAEmitir {
        id: uuid::Uuid::now_v7(),
        tipo: EventoRadian::AceptacionTacita,
        numero: numero_evento(&mut tx, hoy.year()).await?,
        fecha: hoy,
        generador: envio.emisor.clone(),
        receptor: dian(),
        factura_numero: format!("{}{}", d.prefijo, d.numero.unwrap_or_default()),
        factura_cufe: cufe,
        concepto_reclamo: None,
        quien_recibe: None,
        nota: Some(nota),
        credenciales: credenciales(&mut tx, medios.cofre, reloj, ctx, hoy).await?,
    };
    let respuesta = medios.proveedor.evento_radian(&evento).await?;
    let registro = registrar(
        &mut tx,
        almacen,
        reloj,
        ctx,
        &evento,
        (None, Some(id)),
        None,
        respuesta,
    )
    .await?;
    tx.confirmar().await?;
    Ok(registro)
}

/// Eventos que la entidad registró sobre su propia factura electrónica.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.ver`; [`AppError::ModuloNoHabilitado`].
pub async fn eventos_de_documento<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: DocumentoElectronicoId,
) -> AppResult<Vec<RegistroRadian>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    let mut tx = abrir_fe(unidad, reloj, ctx, Permiso::FacturacionVer).await?;
    tx.eventos_radian_documento(id).await
}
