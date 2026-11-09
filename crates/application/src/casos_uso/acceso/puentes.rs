//! Infraestructura de acceso: puntos de acceso, puentes (equipo en sitio) y dispositivos; y la
//! API de los puentes: sincronización incremental, reporte de eventos (idempotente) y latido.
//!
//! - **Decisión en el borde, verdad en la nube**: el puente aplica la última lista recibida si se
//!   cae internet; la nube registra y audita.
//! - El puente se autentica con un token de máquina (en BD solo su SHA-256); se rota o se
//!   retira desde la administración.
//! - La configuración de un dispositivo nunca lleva secretos (se rechazan claves como
//!   `password`, `clave` o `token`).

use chrono::{DateTime, TimeDelta, Utc};
use domain::{
    ClasificacionArchivo, CredencialId, DispositivoAccesoId, EstadoPuente, EstadoVisita,
    NombreArchivo, Permiso, Placa, PuenteId, PuntoAccesoId, RecursoId, ResultadoAcceso,
    SentidoAcceso, TipoArchivo, TipoDispositivoAcceso, TipoEventoVisita, TipoPuntoAcceso, VisitaId,
    copropiedad::texto_obligatorio,
};

use super::{TxAcceso, abrir, tiene, visitas::evento};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::archivos::{ArchivoSubido, borrar_huerfano, nuevo_archivo, registrar_en},
    puertos::{
        AlmacenArchivos, DirectorioAcceso, FirmaQr, RegistroAuditoria, Reloj, RepositorioAcceso,
        Transaccion, UnidadDeTrabajo,
        acceso::{CambiosAcceso, DispositivoAcceso, EventoAcceso, Puente, PuntoAcceso},
        archivos::Archivo,
    },
    seguridad::{hash_secreto, token_aleatorio},
};

pub const ACCION_PUNTO: AccionAuditoria = AccionAuditoria::new("acceso.punto.editar");
pub const ACCION_PUENTE: AccionAuditoria = AccionAuditoria::new("acceso.puente.registrar");
pub const ACCION_PUENTE_CREDENCIAL: AccionAuditoria =
    AccionAuditoria::new("acceso.puente.rotar_credencial");
pub const ACCION_PUENTE_RETIRAR: AccionAuditoria = AccionAuditoria::new("acceso.puente.retirar");
pub const ACCION_DISPOSITIVO: AccionAuditoria = AccionAuditoria::new("acceso.dispositivo.editar");
pub const ACCION_REPORTE: AccionAuditoria = AccionAuditoria::new("acceso.puente.reportar_eventos");

/// Eventos máximos por reporte.
pub const MAX_EVENTOS_POR_REPORTE: usize = 500;
/// Cambios máximos por sincronización (de cada clase).
pub const MAX_CAMBIOS_POR_SINCRONIZACION: u32 = 1000;
/// Tolerancia de un evento con hora futura (relojes desfasados).
const TOLERANCIA_RELOJ: TimeDelta = TimeDelta::minutes(5);

/// Claves que no pueden ir en la configuración de un dispositivo.
const CLAVES_SECRETAS: &[&str] = &[
    "password",
    "contrasena",
    "clave",
    "secreto",
    "secret",
    "token",
    "pin",
];

fn sin_secretos(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Object(m) => m.iter().all(|(k, x)| {
            let k = k.to_ascii_lowercase();
            !CLAVES_SECRETAS.iter().any(|s| k.contains(s)) && sin_secretos(x)
        }),
        serde_json::Value::Array(a) => a.iter().all(sin_secretos),
        _ => true,
    }
}

// ---------------------------------------------------------------------------------------------
// Puntos de acceso
// ---------------------------------------------------------------------------------------------

/// Datos de un punto de acceso.
#[derive(Debug, Clone)]
pub struct DatosPunto {
    pub nombre: String,
    pub tipo: TipoPuntoAcceso,
    pub sentido: SentidoAcceso,
    pub recurso_id: Option<RecursoId>,
    pub activo: bool,
}

/// Crea (`id = None`) o edita un punto de acceso.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.configurar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya existe otro con el mismo nombre.
pub async fn guardar_punto<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: Option<PuntoAccesoId>,
    d: DatosPunto,
) -> AppResult<PuntoAcceso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoConfigurar)?;
    let nombre = texto_obligatorio("nombre", &d.nombre, 80)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    if let Some(i) = id
        && !tx.puntos_acceso().await?.iter().any(|p| p.id == i)
    {
        return Err(AppError::NoEncontrado("punto de acceso"));
    }
    let p = PuntoAcceso {
        id: id.unwrap_or_else(PuntoAccesoId::nuevo),
        nombre,
        tipo: d.tipo,
        sentido: d.sentido,
        recurso_id: d.recurso_id,
        activo: d.activo,
    };
    tx.guardar_punto_acceso(&p, id.is_none()).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", p.tipo.as_str())?
        .con("activo", p.activo)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PUNTO, "punto_acceso", p.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Puntos de acceso (la portería los usa para registrar ingresos).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.registrar` ni `acceso.configurar`.
pub async fn listar_puntos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<PuntoAcceso>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    if !tiene(ctx, Permiso::AccesoConfigurar) {
        ctx.exigir(Permiso::AccesoRegistrar)?;
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.puntos_acceso().await
}

// ---------------------------------------------------------------------------------------------
// Puentes y dispositivos
// ---------------------------------------------------------------------------------------------

/// Registra un puente y devuelve su token de máquina (se muestra **una sola vez**).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.configurar`.
pub async fn registrar_puente<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nombre: &str,
) -> AppResult<(Puente, String)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoConfigurar)?;
    let nombre = texto_obligatorio("nombre", nombre, 80)?;
    let token = token_aleatorio()?;
    let p = Puente {
        id: PuenteId::nuevo(),
        nombre,
        estado: EstadoPuente::Activo,
        credencial_hash: hash_secreto(&token),
        version: None,
        ultimo_latido_en: None,
        creado_en: reloj.ahora(),
    };
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.insertar_puente(&p).await?;
    tx.registrar(EventoAuditoria::ok(ACCION_PUENTE, "puente", p.id.as_uuid()))
        .await?;
    tx.confirmar().await?;
    Ok((p, token))
}

async fn puente_de<T: TxAcceso>(tx: &mut T, id: PuenteId) -> AppResult<Puente> {
    tx.puente(id).await?.ok_or(AppError::NoEncontrado("puente"))
}

/// Emite un token nuevo para el puente; el anterior deja de servir de inmediato.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si está
/// retirado.
pub async fn rotar_credencial_puente<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PuenteId,
) -> AppResult<String>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoConfigurar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut p = puente_de(&mut tx, id).await?;
    if p.estado == EstadoPuente::Retirado {
        return Err(AppError::Conflicto("el puente está retirado".into()));
    }
    let token = token_aleatorio()?;
    p.credencial_hash = hash_secreto(&token);
    tx.actualizar_puente(&p).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PUENTE_CREDENCIAL,
        "puente",
        p.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(token)
}

/// Retira el puente: su token deja de servir y ya no sincroniza.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`].
pub async fn retirar_puente<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PuenteId,
) -> AppResult<Puente>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoConfigurar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut p = puente_de(&mut tx, id).await?;
    p.estado = EstadoPuente::Retirado;
    tx.actualizar_puente(&p).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PUENTE_RETIRAR,
        "puente",
        p.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Puentes de la copropiedad.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.configurar`.
pub async fn listar_puentes<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<Puente>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoConfigurar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.puentes().await
}

/// Datos de un dispositivo.
#[derive(Debug, Clone)]
pub struct DatosDispositivo {
    pub puente_id: PuenteId,
    pub punto_acceso_id: PuntoAccesoId,
    pub tipo: TipoDispositivoAcceso,
    pub fabricante: String,
    pub modelo: Option<String>,
    pub config: serde_json::Value,
    pub activo: bool,
}

/// Crea (`id = None`) o edita un dispositivo de un puente.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`] si el puente o el punto no existen;
/// [`AppError::Validacion`] si la configuración trae secretos.
pub async fn guardar_dispositivo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: Option<DispositivoAccesoId>,
    d: DatosDispositivo,
) -> AppResult<DispositivoAcceso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoConfigurar)?;
    if !sin_secretos(&d.config) {
        return Err(AppError::Validacion(
            "la configuración del dispositivo no puede llevar contraseñas ni tokens: van en el puente".into(),
        ));
    }
    let fabricante = texto_obligatorio("fabricante", &d.fabricante, 60)?;
    let modelo = domain::copropiedad::texto_opcional("modelo", d.modelo.as_deref(), 60)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    puente_de(&mut tx, d.puente_id).await?;
    if !tx
        .puntos_acceso()
        .await?
        .iter()
        .any(|p| p.id == d.punto_acceso_id)
    {
        return Err(AppError::NoEncontrado("punto de acceso"));
    }
    if let Some(i) = id
        && !tx.dispositivos(None).await?.iter().any(|x| x.id == i)
    {
        return Err(AppError::NoEncontrado("dispositivo"));
    }
    let x = DispositivoAcceso {
        id: id.unwrap_or_else(DispositivoAccesoId::nuevo),
        puente_id: d.puente_id,
        punto_acceso_id: d.punto_acceso_id,
        tipo: d.tipo,
        fabricante,
        modelo,
        config: d.config,
        activo: d.activo,
    };
    tx.guardar_dispositivo(&x, id.is_none()).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", x.tipo.as_str())?
        .con("activo", x.activo)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_DISPOSITIVO, "dispositivo_acceso", x.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(x)
}

/// Dispositivos (de un puente o todos).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.configurar`.
pub async fn listar_dispositivos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    puente: Option<PuenteId>,
) -> AppResult<Vec<DispositivoAcceso>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoConfigurar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.dispositivos(puente).await
}

// ---------------------------------------------------------------------------------------------
// API de los puentes
// ---------------------------------------------------------------------------------------------

/// Puente autenticado: el contexto es del sistema en su copropiedad.
#[derive(Debug, Clone)]
pub struct PuenteAutenticado {
    pub ctx: Contexto,
    pub puente: PuenteId,
}

/// Autentica un puente por su token de máquina.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] si el token no corresponde a un puente activo.
pub async fn autenticar_puente(
    directorio: &impl DirectorioAcceso,
    token: &str,
    request_id: Option<String>,
) -> AppResult<PuenteAutenticado> {
    if token.len() < 32 {
        return Err(AppError::NoAutenticado);
    }
    let (tenant, puente) = directorio
        .puente_por_credencial(&hash_secreto(token))
        .await?
        .ok_or(AppError::NoAutenticado)?;
    Ok(PuenteAutenticado {
        ctx: Contexto::new(tenant).con_request_id(request_id),
        puente,
    })
}

/// Lo que recibe el puente al sincronizar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sincronizacion {
    pub cambios: CambiosAcceso,
    /// Clave pública Ed25519 de la PH para validar los QR temporales sin internet.
    pub clave_publica: [u8; 32],
    pub puntos: Vec<PuntoAcceso>,
    pub dispositivos: Vec<DispositivoAcceso>,
    pub ahora: DateTime<Utc>,
}

/// Cambios desde el cursor del puente (cuenta como latido).
///
/// # Errors
///
/// [`AppError::NoAutenticado`] si el puente se retiró; [`AppError::ModuloNoHabilitado`].
pub async fn sincronizar<U>(
    unidad: &U,
    reloj: &impl Reloj,
    firma: &impl FirmaQr,
    p: &PuenteAutenticado,
    cursor: i64,
    version: Option<&str>,
) -> AppResult<Sincronizacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, &p.ctx).await?;
    marcar_latido(&mut tx, p.puente, version, ahora).await?;
    let cambios = tx
        .cambios_desde(cursor.max(0), MAX_CAMBIOS_POR_SINCRONIZACION, ahora)
        .await?;
    let puntos = tx.puntos_acceso().await?;
    let dispositivos = tx.dispositivos(Some(p.puente)).await?;
    tx.confirmar().await?;
    Ok(Sincronizacion {
        cambios,
        clave_publica: firma.clave_publica(p.ctx.tenant_id),
        puntos,
        dispositivos,
        ahora,
    })
}

async fn marcar_latido<T: TxAcceso>(
    tx: &mut T,
    id: PuenteId,
    version: Option<&str>,
    ahora: DateTime<Utc>,
) -> AppResult<()> {
    let mut puente = puente_de(tx, id).await?;
    if puente.estado != EstadoPuente::Activo {
        return Err(AppError::NoAutenticado);
    }
    puente.ultimo_latido_en = Some(ahora);
    if let Some(v) = version {
        puente.version = Some(texto_obligatorio("version", v, 40)?);
    }
    tx.actualizar_puente(&puente).await
}

/// Latido del puente (versión del binario).
///
/// # Errors
///
/// [`AppError::NoAutenticado`] si se retiró.
pub async fn latido<U>(
    unidad: &U,
    reloj: &impl Reloj,
    p: &PuenteAutenticado,
    version: Option<&str>,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let mut tx = abrir(unidad, reloj, &p.ctx).await?;
    marcar_latido(&mut tx, p.puente, version, reloj.ahora()).await?;
    tx.confirmar().await
}

/// Evento que reporta un puente.
#[derive(Debug, Clone)]
pub struct EventoReportado {
    /// Id del evento en el puente (idempotencia).
    pub id_local: String,
    pub dispositivo_id: Option<DispositivoAccesoId>,
    pub punto_acceso_id: Option<PuntoAccesoId>,
    pub credencial_id: Option<CredencialId>,
    pub visita_id: Option<VisitaId>,
    pub sentido: SentidoAcceso,
    pub resultado: ResultadoAcceso,
    pub motivo: Option<String>,
    /// Placa leída por la cámara LPR.
    pub placa: Option<String>,
    /// Foto de la placa (se borra a los días configurados).
    pub foto: Option<Vec<u8>>,
    pub ocurrido_en: DateTime<Utc>,
}

/// Resultado de un reporte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumenReporte {
    pub nuevos: u32,
    pub repetidos: u32,
    pub borrados_confirmados: u32,
}

/// Recibe un lote de eventos del puente (reintentos con el mismo `id_local` no duplican) y las
/// confirmaciones de borrado de plantillas biométricas.
///
/// # Errors
///
/// [`AppError::Validacion`] con más de [`MAX_EVENTOS_POR_REPORTE`] eventos o un `id_local`
/// inválido; [`AppError::NoAutenticado`] si el puente se retiró.
pub async fn reportar_eventos<U, A>(
    unidad: &U,
    almacen: &A,
    reloj: &impl Reloj,
    p: &PuenteAutenticado,
    eventos: Vec<EventoReportado>,
    borrados_biometricos: Vec<CredencialId>,
) -> AppResult<ResumenReporte>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
    A: AlmacenArchivos,
{
    if eventos.len() > MAX_EVENTOS_POR_REPORTE {
        return Err(AppError::Validacion(format!(
            "a lo sumo {MAX_EVENTOS_POR_REPORTE} eventos por reporte"
        )));
    }
    let ahora = reloj.ahora();
    let ctx = &p.ctx;
    // Fotos antes de la transacción (se borran si el reporte falla).
    let mut fotos: Vec<(usize, Archivo)> = Vec::new();
    for (i, e) in eventos.iter().enumerate() {
        if let Some(contenido) = &e.foto {
            let guardada = async {
                let tipo = TipoArchivo::validar(contenido)?;
                let subido = ArchivoSubido {
                    nombre_original: NombreArchivo::new("placa")?,
                    clasificacion: ClasificacionArchivo::Personal,
                    contenido: contenido.clone(),
                };
                let a = nuevo_archivo(ctx, almacen, reloj, &subido, tipo);
                almacen.guardar(&a.clave, &subido.contenido).await?;
                AppResult::Ok(a)
            }
            .await;
            match guardada {
                Ok(a) => fotos.push((i, a)),
                Err(err) => {
                    tracing::warn!(tenant_id = %ctx.tenant_id, error = %err, "foto de evento descartada")
                }
            }
        }
    }
    let resultado = async {
        let mut tx = abrir(unidad, reloj, ctx).await?;
        marcar_latido(&mut tx, p.puente, None, ahora).await?;
        let config = tx.configuracion_acceso().await?;
        let mut r = ResumenReporte::default();
        let mut registradas = Vec::new();
        for (i, e) in eventos.into_iter().enumerate() {
            let id_local = texto_obligatorio("id_local", &e.id_local, 80)?;
            let evento_acceso = EventoAcceso {
                id: domain::EventoAccesoId::nuevo(),
                punto_acceso_id: e.punto_acceso_id,
                dispositivo_id: e.dispositivo_id,
                puente_id: Some(p.puente),
                id_local: Some(id_local),
                credencial_id: e.credencial_id,
                visita_id: e.visita_id,
                sentido: e.sentido,
                resultado: e.resultado,
                motivo: domain::copropiedad::texto_opcional("motivo", e.motivo.as_deref(), 300)?,
                placa: e.placa.as_deref().and_then(|x| Placa::new(x).ok()),
                registrado_por: None,
                ocurrido_en: e.ocurrido_en.min(ahora + TOLERANCIA_RELOJ),
            };
            if !tx.insertar_evento_acceso(&evento_acceso).await? {
                r.repetidos += 1;
                continue;
            }
            r.nuevos += 1;
            if let Some((_, a)) = fotos.iter().find(|(j, _)| *j == i) {
                registrar_en(&mut tx, a).await?;
                let purgar = evento_acceso.ocurrido_en
                    + TimeDelta::days(i64::from(config.retencion_fotos_placas_dias));
                tx.adjuntar_foto_evento(evento_acceso.id, a.id, purgar)
                    .await?;
                registradas.push(a.id);
            }
            if let Some(v) = evento_acceso.visita_id
                && evento_acceso.resultado == ResultadoAcceso::Permitido
            {
                aplicar_uso_visita(
                    &mut tx,
                    ctx,
                    v,
                    evento_acceso.sentido,
                    evento_acceso.ocurrido_en,
                )
                .await?;
            }
        }
        for id in borrados_biometricos {
            if let Some(mut c) = tx.credencial(id).await?
                && c.tipo == domain::TipoCredencial::BiometriaRef
                && c.borrado_confirmado_en.is_none()
                && c.estado != domain::EstadoCredencial::Activa
            {
                c.borrado_confirmado_en = Some(ahora);
                tx.actualizar_credencial(&c).await?;
                r.borrados_confirmados += 1;
            }
        }
        let detalle = DetalleAuditoria::vacio()
            .con("nuevos", r.nuevos)?
            .con("repetidos", r.repetidos)?
            .con("borrados_confirmados", r.borrados_confirmados)?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_REPORTE, "puente", p.puente.as_uuid()).con_detalle(detalle),
        )
        .await?;
        tx.confirmar().await?;
        Ok((r, registradas))
    }
    .await;
    // Las fotos de eventos repetidos o de un reporte fallido no quedan registradas: se borran.
    let registradas = resultado
        .as_ref()
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    for (_, a) in fotos.iter().filter(|(_, a)| !registradas.contains(&a.id)) {
        borrar_huerfano(almacen, ctx, a).await;
    }
    resultado.map(|(r, _)| r)
}

/// Un QR de visita usado en un torniquete: cuenta el uso y mueve la visita.
async fn aplicar_uso_visita<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    id: VisitaId,
    sentido: SentidoAcceso,
    ocurrido: DateTime<Utc>,
) -> AppResult<()> {
    let Some(mut v) = tx.visita_para_actualizar(id).await? else {
        return Ok(());
    };
    match (sentido, v.estado) {
        (SentidoAcceso::Entrada | SentidoAcceso::Ambos, EstadoVisita::Autorizada)
            if v.usos < v.usos_permitidos =>
        {
            v.estado = v.estado.transicionar(EstadoVisita::Ingresada)?;
            v.usos += 1;
            v.ingreso_en = Some(ocurrido);
            tx.actualizar_visita(&v).await?;
            tx.insertar_evento_visita(&evento(&v, TipoEventoVisita::Ingreso, ctx, ocurrido))
                .await?;
        }
        (SentidoAcceso::Salida, EstadoVisita::Ingresada) => {
            let quedan =
                v.usos < v.usos_permitidos && v.ventana_hasta.is_some_and(|h| ocurrido < h);
            v.estado = v.estado.transicionar(if quedan {
                EstadoVisita::Autorizada
            } else {
                EstadoVisita::Finalizada
            })?;
            v.salida_en = Some(ocurrido);
            tx.actualizar_visita(&v).await?;
            tx.insertar_evento_visita(&evento(&v, TipoEventoVisita::Salida, ctx, ocurrido))
                .await?;
        }
        _ => {}
    }
    Ok(())
}
