//! Portería: configuración del módulo, minuta digital, bitácora de eventos de acceso, alertas
//! operativas y costo de los avisos de WhatsApp de la PH.

use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, ClasificacionArchivo, ConfiguracionAcceso, NombreArchivo, NovedadMinutaId, Permiso,
    PuntoAccesoId, ResultadoAcceso, SentidoAcceso, TipoArchivo, TipoNovedadMinuta,
    copropiedad::{texto_obligatorio, texto_opcional},
};
use rust_decimal::Decimal;

use super::{TxAcceso, abrir, zona_ph};
use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::archivos::{ArchivoSubido, borrar_huerfano, nuevo_archivo, registrar_en},
    puertos::{
        AlmacenArchivos, RegistroAuditoria, Reloj, RepositorioAcceso, RepositorioAsambleas,
        Transaccion, UnidadDeTrabajo,
        acceso::{
            AlertaAcceso, ConteoAvisos, EventoAcceso, FiltroEventosAcceso, FiltroMinuta,
            NovedadMinuta,
        },
        archivos::Archivo,
    },
};

pub const ACCION_CONFIGURAR: AccionAuditoria = AccionAuditoria::new("acceso.configuracion.editar");
pub const ACCION_MINUTA: AccionAuditoria = AccionAuditoria::new("acceso.minuta.registrar");
pub const ACCION_EVENTO_MANUAL: AccionAuditoria = AccionAuditoria::new("acceso.evento.registrar");

/// Adjuntos máximos de una novedad de la minuta.
pub const MAX_ADJUNTOS_MINUTA: usize = 5;

/// Configuración del módulo (plazos de retención, días de aviso y silencio).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.configurar` ni `acceso.registrar`.
pub async fn ver_configuracion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<ConfiguracionAcceso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    if ctx.exigir(Permiso::AccesoConfigurar).is_err() {
        ctx.exigir(Permiso::AccesoRegistrar)?;
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.configuracion_acceso().await
}

/// Reemplaza la configuración. Las entregas ya registradas conservan su plazo de retención.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.configurar`; [`AppError::Dominio`] con plazos fuera de
/// rango.
pub async fn guardar_configuracion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    c: ConfiguracionAcceso,
) -> AppResult<ConfiguracionAcceso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoConfigurar)?;
    let c = c.validar()?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.guardar_configuracion_acceso(&c).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("retencion_soportes_dias", c.retencion_soportes_dias)?
        .con("dias_avisos_correspondencia", c.dias_avisos_correspondencia)?
        .con("retencion_eventos_dias", c.retencion_eventos_dias)?
        .con("retencion_fotos_placas_dias", c.retencion_fotos_placas_dias)?
        .con(
            "silencio",
            c.silencio.map(|s| format!("{}-{}", s.inicio, s.fin)),
        )?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_CONFIGURAR,
            "configuracion_acceso",
            ctx.tenant_id.as_uuid(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Novedad nueva de la minuta.
#[derive(Debug, Clone)]
pub struct NuevaNovedad {
    pub tipo: TipoNovedadMinuta,
    pub texto: String,
    pub punto_acceso_id: Option<PuntoAccesoId>,
    /// Fotos o documentos adjuntos (bytes).
    pub adjuntos: Vec<Vec<u8>>,
}

/// Registra una novedad en la minuta (append-only) con sus adjuntos.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.minuta.registrar`; [`AppError::Dominio`] con texto o
/// adjuntos inválidos; [`AppError::Validacion`] con más de [`MAX_ADJUNTOS_MINUTA`] adjuntos.
pub async fn registrar_novedad<U, A>(
    unidad: &U,
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: NuevaNovedad,
) -> AppResult<NovedadMinuta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
    A: AlmacenArchivos,
{
    ctx.exigir(Permiso::AccesoMinutaRegistrar)?;
    let texto = texto_obligatorio("texto", &n.texto, 2000)?;
    if n.adjuntos.len() > MAX_ADJUNTOS_MINUTA {
        return Err(AppError::Validacion(format!(
            "a lo sumo {MAX_ADJUNTOS_MINUTA} adjuntos"
        )));
    }
    let mut archivos: Vec<Archivo> = Vec::new();
    for (i, contenido) in n.adjuntos.into_iter().enumerate() {
        let r = async {
            let tipo = TipoArchivo::validar(&contenido)?;
            let subido = ArchivoSubido {
                nombre_original: NombreArchivo::new(&format!("minuta-{}", i + 1))?,
                clasificacion: ClasificacionArchivo::Personal,
                contenido,
            };
            let a = nuevo_archivo(ctx, almacen, reloj, &subido, tipo);
            almacen.guardar(&a.clave, &subido.contenido).await?;
            AppResult::Ok(a)
        }
        .await;
        match r {
            Ok(a) => archivos.push(a),
            Err(e) => {
                for a in &archivos {
                    borrar_huerfano(almacen, ctx, a).await;
                }
                return Err(e);
            }
        }
    }
    let resultado = async {
        let mut tx = abrir(unidad, reloj, ctx).await?;
        for a in &archivos {
            registrar_en(&mut tx, a).await?;
        }
        let novedad = NovedadMinuta {
            id: NovedadMinutaId::nuevo(),
            tipo: n.tipo,
            texto,
            punto_acceso_id: n.punto_acceso_id,
            adjuntos: archivos.iter().map(|a| a.id).collect::<Vec<ArchivoId>>(),
            registrada_por: ctx.actor_id,
            ocurrida_en: reloj.ahora(),
        };
        tx.insertar_novedad(&novedad).await?;
        let detalle = DetalleAuditoria::vacio()
            .con("tipo", novedad.tipo.as_str())?
            .con("adjuntos", novedad.adjuntos.len())?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_MINUTA, "novedad_minuta", novedad.id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
        tx.confirmar().await?;
        Ok(novedad)
    }
    .await;
    if resultado.is_err() {
        for a in &archivos {
            borrar_huerfano(almacen, ctx, a).await;
        }
    }
    resultado
}

/// Minuta (más reciente primero).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.minuta.ver`.
pub async fn consultar_minuta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroMinuta,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<NovedadMinuta>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoMinutaVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.novedades(&filtro, pagina).await
}

/// Evento manual de la portería (apertura manual, salida de un residente…).
#[derive(Debug, Clone)]
pub struct NuevoEventoManual {
    pub punto_acceso_id: Option<PuntoAccesoId>,
    pub sentido: SentidoAcceso,
    pub resultado: ResultadoAcceso,
    pub motivo: Option<String>,
    pub placa: Option<String>,
}

/// Registra un evento en la bitácora desde la portería digital.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.registrar`; [`AppError::Dominio`] con datos inválidos.
pub async fn registrar_evento_manual<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: NuevoEventoManual,
) -> AppResult<EventoAcceso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoRegistrar)?;
    let e = EventoAcceso {
        id: domain::EventoAccesoId::nuevo(),
        punto_acceso_id: n.punto_acceso_id,
        dispositivo_id: None,
        puente_id: None,
        id_local: None,
        credencial_id: None,
        visita_id: None,
        sentido: n.sentido,
        resultado: n.resultado,
        motivo: texto_opcional("motivo", n.motivo.as_deref(), 300)?,
        placa: n
            .placa
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .map(domain::Placa::new)
            .transpose()?,
        registrado_por: ctx.actor_id,
        ocurrido_en: reloj.ahora(),
    };
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.insertar_evento_acceso(&e).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("resultado", e.resultado.as_str())?
        .con("sentido", e.sentido.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EVENTO_MANUAL, "evento_acceso", e.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(e)
}

/// Bitácora de eventos de acceso.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.bitacora.ver`.
pub async fn listar_eventos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroEventosAcceso,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<EventoAcceso>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoBitacoraVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.eventos_acceso(&filtro, pagina).await
}

/// Alertas operativas (puentes sin latido, puertas forzadas, denegados repetidos).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.bitacora.ver`.
pub async fn alertas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<AlertaAcceso>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoBitacoraVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.alertas_acceso(reloj.ahora()).await
}

/// Costo estimado de los avisos de control de acceso de un periodo (decisión del 2026-11-09: se
/// lleva la cuenta por PH para poder cobrarlo o limitarlo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostoAvisosAcceso {
    pub desde: DateTime<Utc>,
    pub hasta: DateTime<Utc>,
    pub moneda: String,
    pub conteo: ConteoAvisos,
    pub tarifa_utilidad: Decimal,
    /// Solo las plantillas de WhatsApp tienen costo; el push es gratuito.
    pub valor_estimado: Decimal,
}

/// Avisos del periodo valorizados con la tarifa de utilidad vigente al final del periodo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.configurar`; [`AppError::Validacion`] si el periodo
/// está invertido.
pub async fn costo_avisos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    desde: DateTime<Utc>,
    hasta: DateTime<Utc>,
    moneda: &str,
) -> AppResult<CostoAvisosAcceso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoConfigurar)?;
    if hasta <= desde {
        return Err(AppError::Validacion(
            "`hasta` debe ser posterior a `desde`".into(),
        ));
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let tz = zona_ph(&mut tx).await?;
    let conteo = tx.mensajes_avisos_acceso(desde, hasta).await?;
    let fecha = hasta.with_timezone(&tz).date_naive();
    let tarifa = tx
        .tarifas_whatsapp("CO", moneda, fecha)
        .await?
        .into_iter()
        .find(|(c, _)| c == "utilidad")
        .map_or(Decimal::ZERO, |(_, v)| v);
    Ok(CostoAvisosAcceso {
        desde,
        hasta,
        moneda: moneda.to_owned(),
        conteo,
        tarifa_utilidad: tarifa,
        valor_estimado: (Decimal::from(conteo.whatsapp_enviados) * tarifa).round_dp(2),
    })
}
