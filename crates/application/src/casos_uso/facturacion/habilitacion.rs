//! Habilitación de la entidad ante la DIAN con el motor propio (plan 18 parte B, plan 17 fase 5).
//!
//! Pasos: registrar el software (SoftwareID, PIN y TestSetId que asigna la DIAN al registrarlo en
//! el catálogo de participantes) → cargar el certificado de firma (`.p12`) → enviar el set de
//! pruebas → consultarlo hasta que la DIAN lo acepte (queda `habilitado`) → pasar a producción →
//! sincronizar los rangos de numeración autorizados. Los secretos (PIN, `.p12` y su contraseña)
//! se guardan cifrados con el [`Cofre`] y atados a la entidad; nunca vuelven por la API ni van a
//! la auditoría.

use chrono::{DateTime, Duration, Utc};
use domain::{
    CertificadoFirmaId, EnvioSetPruebasId, Modulo, Permiso, ResolucionNumeracionId,
    facturacion::{AmbienteDian, DIAS_AVISO_CERTIFICADO, EstadoEnvioSet, EstadoHabilitacion},
    tributario::{EstadoDocumentoElectronico, TipoDocumentoElectronico, TipoNumeracion},
};
use rust_decimal::Decimal;

use super::documentos::{TxFacturacion, cargar_credenciales, contexto_secreto, emisor};
use crate::casos_uso::contabilidad::prelude::*;
use crate::casos_uso::modulos::abrir_con_modulos;
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        Reloj, UnidadDeTrabajo,
        cofre::Cofre,
        facturacion::{
            CertificadoFirma, ConfiguracionFacturacion, EnvioSetPruebas,
            RepositorioFacturacion as _,
        },
        tributario::{
            DocumentoAEmitir, HabilitacionDian, LectorCertificados, LineaDocumento, ParteDocumento,
            RangoAutorizado, ResolucionDocumento, ResolucionNumeracion,
        },
    },
};

const ACCION_SOFTWARE: AccionAuditoria = AccionAuditoria::new("facturacion.habilitacion.software");
const ACCION_CERTIFICADO: AccionAuditoria =
    AccionAuditoria::new("facturacion.habilitacion.certificado");
const ACCION_SET: AccionAuditoria = AccionAuditoria::new("facturacion.habilitacion.set_pruebas");
const ACCION_CONSULTA: AccionAuditoria =
    AccionAuditoria::new("facturacion.habilitacion.consulta_set");
const ACCION_PRODUCCION: AccionAuditoria =
    AccionAuditoria::new("facturacion.habilitacion.produccion");
const ACCION_RANGOS: AccionAuditoria = AccionAuditoria::new("facturacion.habilitacion.rangos");

/// Prefijo del rango de numeración de pruebas de la DIAN (ambiente de habilitación).
pub const PREFIJO_PRUEBAS: &str = "SETP";
/// Primer número del rango de pruebas de la DIAN.
pub const DESDE_PRUEBAS: u64 = 990_000_000;
/// Último número del rango de pruebas de la DIAN.
pub const HASTA_PRUEBAS: u64 = 995_000_000;
/// Clave técnica pública del rango de pruebas de la DIAN (catálogo de habilitación).
const CLAVE_PRUEBAS: &str = "fc8eac422eba16e22ffd8c6f94b3f40a6e38162c";
/// Máximo de documentos por envío del set de pruebas.
pub const MAXIMO_SET: u32 = 50;

fn validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

async fn abrir<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    permiso: Permiso,
) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    abrir_con_modulos(
        unidad,
        reloj,
        ctx,
        &[permiso],
        &[Modulo::FacturacionElectronica],
    )
    .await
}

fn codigo_dian(texto: &str, campo: &str) -> AppResult<String> {
    let t = texto.trim();
    if (1..=100).contains(&t.len()) && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Ok(t.to_owned())
    } else {
        Err(validacion(format!(
            "{campo}: de 1 a 100 letras, dígitos o guiones, como lo entrega la DIAN"
        )))
    }
}

/// Certificado visto desde la API (sin secretos).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VistaCertificado {
    pub id: CertificadoFirmaId,
    pub sujeto: String,
    pub emisor: String,
    pub serial: String,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: DateTime<Utc>,
    /// Vence en menos de [`DIAS_AVISO_CERTIFICADO`] días (o ya venció).
    pub por_vencer: bool,
}

/// Estado de la habilitación de la entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Habilitacion {
    pub software_id: Option<String>,
    pub test_set_id: Option<String>,
    pub estado: Option<EstadoHabilitacion>,
    pub ambiente: Option<AmbienteDian>,
    pub certificado: Option<VistaCertificado>,
    pub envios: Vec<EnvioSetPruebas>,
}

fn vista(c: &CertificadoFirma, ahora: DateTime<Utc>) -> VistaCertificado {
    VistaCertificado {
        id: c.id,
        sujeto: c.sujeto.clone(),
        emisor: c.emisor.clone(),
        serial: c.serial.clone(),
        vigente_desde: c.vigente_desde,
        vigente_hasta: c.vigente_hasta,
        por_vencer: c.vigente_hasta - ahora < Duration::days(DIAS_AVISO_CERTIFICADO),
    }
}

/// Estado de la habilitación: software, certificado (con aviso de vencimiento) y set de pruebas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.configurar`; [`AppError::ModuloNoHabilitado`].
pub async fn habilitacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Habilitacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::FacturacionConfigurar).await?;
    let config = tx.configuracion_facturacion().await?;
    let certificado = tx.certificado_activo().await?;
    Ok(Habilitacion {
        software_id: config.as_ref().map(|c| c.software_id.clone()),
        test_set_id: config.as_ref().and_then(|c| c.test_set_id.clone()),
        estado: config.as_ref().map(|c| c.estado),
        ambiente: config.as_ref().map(|c| c.estado.ambiente()),
        certificado: certificado.as_ref().map(|c| vista(c, reloj.ahora())),
        envios: tx.envios_set().await?,
    })
}

/// Datos del software registrado en el catálogo de participantes de la DIAN.
#[derive(Clone)]
pub struct DatosSoftware {
    pub software_id: String,
    /// PIN que la entidad definió al registrar el software (secreto).
    pub pin: String,
    pub test_set_id: Option<String>,
}

/// Registra (o cambia) el software de la entidad. Cambiarlo vuelve la habilitación a
/// `registrado`; en producción ya no se cambia.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.configurar`; [`AppError::Validacion`] si un dato
/// no cumple o el servidor no tiene clave maestra; [`domain::DominioError::TransicionInvalida`]
/// si la entidad ya está en producción.
pub async fn registrar_software<U, C>(
    unidad: &U,
    cofre: &C,
    reloj: &impl Reloj,
    ctx: &Contexto,
    datos: DatosSoftware,
) -> AppResult<Habilitacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    C: Cofre,
{
    let software_id = codigo_dian(&datos.software_id, "software_id")?;
    let test_set_id = datos
        .test_set_id
        .as_deref()
        .map(|t| codigo_dian(t, "test_set_id"))
        .transpose()?;
    let pin = datos.pin.trim();
    if !(1..=50).contains(&pin.len()) {
        return Err(validacion("el PIN tiene de 1 a 50 caracteres"));
    }
    let pin_cifrado = cofre.cifrar(&contexto_secreto(ctx, "pin"), pin.as_bytes())?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::FacturacionConfigurar).await?;
    let anterior = tx.configuracion_facturacion().await?;
    let estado = match &anterior {
        Some(c) => c.estado.transicionar(EstadoHabilitacion::Registrado)?,
        None => EstadoHabilitacion::Registrado,
    };
    let certificado_id = match anterior.as_ref().and_then(|c| c.certificado_id) {
        Some(c) => Some(c),
        None => tx.certificado_activo().await?.map(|c| c.id),
    };
    tx.guardar_configuracion_facturacion(&ConfiguracionFacturacion {
        software_id: software_id.clone(),
        pin_cifrado,
        test_set_id,
        estado,
        certificado_id,
        actualizado_por: ctx.actor_id,
        actualizado_en: reloj.ahora(),
    })
    .await?;
    let detalle = DetalleAuditoria::vacio().con("software_id", software_id)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_SOFTWARE, "entidad", ctx.tenant_id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    habilitacion(unidad, reloj, ctx).await
}

/// Carga el certificado de firma (`.p12`) de la entidad: lo valida con su contraseña (vigente y
/// con no repudio), lo cifra y lo deja como el activo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.certificado.gestionar`; [`AppError::Validacion`]
/// si el archivo, la contraseña o el certificado no sirven, o ya venció.
pub async fn cargar_certificado<U, C, L>(
    unidad: &U,
    (cofre, lector): (&C, &L),
    reloj: &impl Reloj,
    ctx: &Contexto,
    p12: &[u8],
    contrasena: &str,
) -> AppResult<VistaCertificado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    C: Cofre,
    L: LectorCertificados,
{
    if p12.is_empty() || p12.len() > 65_536 {
        return Err(validacion("el .p12 debe pesar entre 1 byte y 64 KB"));
    }
    let leido = lector.leer(p12, contrasena)?;
    let ahora = reloj.ahora();
    if leido.vigente_hasta <= ahora {
        return Err(validacion("el certificado ya venció"));
    }
    let certificado = CertificadoFirma {
        id: CertificadoFirmaId::nuevo(),
        p12_cifrado: cofre.cifrar(&contexto_secreto(ctx, "certificado"), p12)?,
        contrasena_cifrada: cofre
            .cifrar(&contexto_secreto(ctx, "contrasena"), contrasena.as_bytes())?,
        sujeto: leido.sujeto,
        emisor: leido.emisor,
        serial: leido.serial,
        vigente_desde: leido.vigente_desde,
        vigente_hasta: leido.vigente_hasta,
        activo: true,
        cargado_por: ctx.actor_id,
        cargado_en: ahora,
    };
    let mut tx = abrir(unidad, reloj, ctx, Permiso::FacturacionCertificadoGestionar).await?;
    tx.insertar_certificado(&certificado).await?;
    if let Some(mut config) = tx.configuracion_facturacion().await? {
        config.certificado_id = Some(certificado.id);
        config.actualizado_por = ctx.actor_id;
        config.actualizado_en = ahora;
        tx.guardar_configuracion_facturacion(&config).await?;
    }
    let detalle = DetalleAuditoria::vacio()
        .con("serial", certificado.serial.clone())?
        .con("vence", certificado.vigente_hasta.to_rfc3339())?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_CERTIFICADO,
            "certificado_firma",
            certificado.id.as_uuid(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(vista(&certificado, ahora))
}

/// Configuración que exige la habilitación (registrada, con certificado y TestSetId).
async fn configuracion_lista<T: TxFacturacion>(tx: &mut T) -> AppResult<ConfiguracionFacturacion> {
    let config = tx
        .configuracion_facturacion()
        .await?
        .ok_or_else(|| validacion("registre primero el software de la entidad"))?;
    if config.certificado_id.is_none() {
        return Err(validacion("cargue el certificado de firma (.p12)"));
    }
    Ok(config)
}

/// Comprador de los documentos de prueba: consumidor final.
fn adquirente_pruebas() -> ParteDocumento {
    ParteDocumento {
        documento: "222222222222".into(),
        nombre: "Consumidor final".into(),
        persona_natural: true,
        responsabilidades: vec!["R-99-PN".into()],
        ..ParteDocumento::default()
    }
}

/// Envía `cantidad` facturas de prueba al ambiente de habilitación con el TestSetId (rango de
/// pruebas `SETP`). Cada una queda registrada con su clave para consultarla.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.configurar`; [`AppError::Validacion`] si falta el
/// software, el certificado o el TestSetId, o la cantidad no va de 1 a [`MAXIMO_SET`];
/// [`domain::DominioError::TransicionInvalida`] en producción; [`AppError::Infraestructura`] si
/// la DIAN no responde.
pub async fn enviar_set_pruebas<U, H, C>(
    unidad: &U,
    (dian, cofre): (&H, &C),
    reloj: &impl Reloj,
    ctx: &Contexto,
    cantidad: u32,
) -> AppResult<Vec<EnvioSetPruebas>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    H: HabilitacionDian,
    C: Cofre,
{
    if !(1..=MAXIMO_SET).contains(&cantidad) {
        return Err(validacion(format!(
            "el set de pruebas se envía de 1 a {MAXIMO_SET} documentos"
        )));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::FacturacionConfigurar).await?;
    let mut config = configuracion_lista(&mut tx).await?;
    if config.test_set_id.is_none() {
        return Err(validacion("registre el TestSetId que entrega la DIAN"));
    }
    config.estado = config.estado.transicionar(EstadoHabilitacion::EnPruebas)?;
    let ahora = reloj.ahora();
    let hoy = ahora.date_naive();
    let emisor = emisor(&mut tx, hoy).await?;
    let mut siguiente = tx
        .envios_set()
        .await?
        .iter()
        .map(|e| e.numero)
        .max()
        .unwrap_or(DESDE_PRUEBAS)
        + 1;
    let mut enviados = Vec::new();
    for _ in 0..cantidad {
        if siguiente > HASTA_PRUEBAS {
            return Err(validacion("se agotó el rango de pruebas de la DIAN"));
        }
        let credenciales = cargar_credenciales(
            &mut tx,
            cofre,
            reloj,
            ctx,
            &config,
            AmbienteDian::Habilitacion,
        )
        .await?;
        let valor = Decimal::from(100_000);
        let documento = DocumentoAEmitir {
            id: domain::DocumentoElectronicoId::nuevo(),
            tipo: TipoDocumentoElectronico::Factura,
            prefijo: PREFIJO_PRUEBAS.into(),
            numero: siguiente,
            fecha: hoy,
            emisor: emisor.clone(),
            adquirente: adquirente_pruebas(),
            lineas: vec![LineaDocumento {
                descripcion: "Servicio de prueba de habilitación".into(),
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
            clave_tecnica: Some(CLAVE_PRUEBAS.into()),
            resolucion: Some(ResolucionDocumento {
                numero: "18760000001".into(),
                vigente_desde: hoy,
                vigente_hasta: hoy,
                desde: DESDE_PRUEBAS,
                hasta: HASTA_PRUEBAS,
            }),
            vencimiento: None,
            referencia_cufe: None,
            referencia_numero: None,
            referencia_fecha: None,
            credenciales: Some(credenciales),
        };
        let respuesta = dian.enviar_set_pruebas(&documento).await?;
        let envio = EnvioSetPruebas {
            id: EnvioSetPruebasId::nuevo(),
            tipo: TipoDocumentoElectronico::Factura,
            prefijo: PREFIJO_PRUEBAS.into(),
            numero: siguiente,
            codigo: respuesta.cufe.clone().unwrap_or_default(),
            clave: respuesta.id_externo.clone(),
            estado: EstadoEnvioSet::Enviado,
            respuesta: respuesta.mensaje.clone(),
            enviado_en: ahora,
            consultado_en: None,
        };
        tx.insertar_envio_set(&envio).await?;
        enviados.push(envio);
        siguiente += 1;
    }
    config.actualizado_por = ctx.actor_id;
    config.actualizado_en = ahora;
    tx.guardar_configuracion_facturacion(&config).await?;
    let detalle = DetalleAuditoria::vacio().con("documentos", enviados.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_SET, "entidad", ctx.tenant_id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(enviados)
}

/// Consulta en la DIAN los envíos del set aún sin resultado. Si todos los enviados fueron
/// aceptados, la entidad queda `habilitado`.
///
/// # Errors
///
/// Los de [`enviar_set_pruebas`].
pub async fn consultar_set_pruebas<U, H, C>(
    unidad: &U,
    (dian, cofre): (&H, &C),
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Habilitacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    H: HabilitacionDian,
    C: Cofre,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::FacturacionConfigurar).await?;
    let mut config = configuracion_lista(&mut tx).await?;
    let credenciales = cargar_credenciales(
        &mut tx,
        cofre,
        reloj,
        ctx,
        &config,
        AmbienteDian::Habilitacion,
    )
    .await?;
    let ahora = reloj.ahora();
    let mut envios = tx.envios_set().await?;
    for e in envios.iter_mut().filter(|e| {
        matches!(
            e.estado,
            EstadoEnvioSet::Enviado | EstadoEnvioSet::SinRespuesta
        )
    }) {
        let Some(clave) = e.clave.clone() else {
            continue;
        };
        let r = dian.consultar_set(&credenciales, &clave).await?;
        e.estado = match r.estado {
            EstadoDocumentoElectronico::Validado => EstadoEnvioSet::Aceptado,
            EstadoDocumentoElectronico::Rechazado => EstadoEnvioSet::Rechazado,
            _ => EstadoEnvioSet::SinRespuesta,
        };
        e.respuesta = r.mensaje;
        e.consultado_en = Some(ahora);
        tx.actualizar_envio_set(e).await?;
    }
    let todos_aceptados =
        !envios.is_empty() && envios.iter().all(|e| e.estado == EstadoEnvioSet::Aceptado);
    if todos_aceptados && config.estado == EstadoHabilitacion::EnPruebas {
        config.estado = config.estado.transicionar(EstadoHabilitacion::Habilitado)?;
        config.actualizado_por = ctx.actor_id;
        config.actualizado_en = ahora;
        tx.guardar_configuracion_facturacion(&config).await?;
    }
    let detalle = DetalleAuditoria::vacio().con("estado", config.estado.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CONSULTA, "entidad", ctx.tenant_id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    habilitacion(unidad, reloj, ctx).await
}

/// Pasa la entidad a producción (exige la habilitación aceptada): desde entonces sus documentos
/// salen con validez por el motor propio si su perfil tributario tiene el canal `propio`.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.configurar`;
/// [`domain::DominioError::TransicionInvalida`] si no está `habilitado`.
pub async fn pasar_a_produccion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Habilitacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::FacturacionConfigurar).await?;
    let mut config = configuracion_lista(&mut tx).await?;
    config.estado = config.estado.transicionar(EstadoHabilitacion::Produccion)?;
    config.actualizado_por = ctx.actor_id;
    config.actualizado_en = reloj.ahora();
    tx.guardar_configuracion_facturacion(&config).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PRODUCCION,
        "entidad",
        ctx.tenant_id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    habilitacion(unidad, reloj, ctx).await
}

/// Trae de la DIAN los rangos de numeración autorizados al NIT para el software y registra como
/// resoluciones de factura los que aún no estén (con su clave técnica). Devuelve los nuevos.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.configurar`; [`AppError::Validacion`] si la
/// entidad no está en producción; [`AppError::Infraestructura`] si la DIAN no responde.
pub async fn sincronizar_rangos<U, H, C>(
    unidad: &U,
    (dian, cofre): (&H, &C),
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<ResolucionNumeracion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    H: HabilitacionDian,
    C: Cofre,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::FacturacionConfigurar).await?;
    let config = configuracion_lista(&mut tx).await?;
    if config.estado != EstadoHabilitacion::Produccion {
        return Err(validacion(
            "los rangos se sincronizan en producción: termine la habilitación",
        ));
    }
    let credenciales = cargar_credenciales(
        &mut tx,
        cofre,
        reloj,
        ctx,
        &config,
        AmbienteDian::Produccion,
    )
    .await?;
    let nit = emisor(&mut tx, reloj.ahora().date_naive()).await?.documento;
    let rangos: Vec<RangoAutorizado> = dian.rangos_numeracion(&credenciales, &nit).await?;
    let existentes = tx.resoluciones().await?;
    let mut nuevas = Vec::new();
    for r in rangos {
        let repetida = existentes.iter().any(|e| {
            e.tipo == TipoNumeracion::Factura && e.prefijo == r.prefijo && e.desde == r.desde
        });
        if repetida {
            continue;
        }
        let resolucion = ResolucionNumeracion {
            id: ResolucionNumeracionId::nuevo(),
            tipo: TipoNumeracion::Factura,
            prefijo: r.prefijo,
            desde: r.desde,
            hasta: r.hasta,
            numero_resolucion: r.resolucion,
            fecha_resolucion: r.fecha_resolucion.unwrap_or(r.vigente_desde),
            vigente_desde: r.vigente_desde,
            vigente_hasta: r.vigente_hasta,
            clave_tecnica: Some(r.clave_tecnica),
            activa: true,
            creada_en: reloj.ahora(),
        };
        tx.insertar_resolucion(&resolucion).await?;
        nuevas.push(resolucion);
    }
    let detalle = DetalleAuditoria::vacio().con("nuevas", nuevas.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RANGOS, "entidad", ctx.tenant_id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(nuevas)
}
