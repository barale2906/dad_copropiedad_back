//! Credenciales (tarjetas, PIN, QR permanentes, placas, app, referencias biométricas),
//! autorizaciones manuales, verificación en la portería digital y consentimientos biométricos.
//!
//! - El identificador nunca se guarda en claro: solo su huella
//!   ([`domain::acceso::huella_credencial`]). Los QR permanentes se generan aquí y su texto se
//!   entrega una sola vez.
//! - La biometría exige un consentimiento vigente (Ley 1581 de 2012, dato sensible) y siempre
//!   existe un método alterno. Revocar el consentimiento revoca las credenciales biométricas: el
//!   puente borra la plantilla del equipo y lo confirma.

use chrono::{DateTime, Utc};
use domain::{
    AutorizacionAccesoId, ClasificacionArchivo, ConsentimientoBiometricoId, CredencialId,
    EstadoCredencial, Horario, NombreArchivo, OrigenAutorizacionAcceso, Permiso, PersonaId,
    PuntoAccesoId, ResultadoAcceso, SentidoAcceso, TipoArchivo, TipoCredencial, VisitaId,
    acceso::{CargaQr, VentanaAcceso, huella_credencial},
    copropiedad::{texto_obligatorio, texto_opcional},
};

use super::{TxAcceso, abrir, zona_ph};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::archivos::{ArchivoSubido, borrar_huerfano, nuevo_archivo, registrar_en},
    puertos::{
        AlmacenArchivos, FirmaQr, RegistroAuditoria, Reloj, RepositorioAcceso, RepositorioPersonas,
        Transaccion, UnidadDeTrabajo,
        acceso::{
            AutorizacionAcceso, ConsentimientoBiometrico, Credencial, EventoAcceso, TitularAcceso,
        },
    },
    seguridad::token_aleatorio,
};

pub const ACCION_EMITIR: AccionAuditoria = AccionAuditoria::new("acceso.credencial.emitir");
pub const ACCION_ESTADO: AccionAuditoria = AccionAuditoria::new("acceso.credencial.cambiar_estado");
pub const ACCION_AUTORIZAR: AccionAuditoria = AccionAuditoria::new("acceso.autorizacion.crear");
pub const ACCION_REVOCAR_AUTORIZACION: AccionAuditoria =
    AccionAuditoria::new("acceso.autorizacion.revocar");
pub const ACCION_VERIFICAR: AccionAuditoria = AccionAuditoria::new("acceso.credencial.verificar");
pub const ACCION_CONSENTIMIENTO: AccionAuditoria =
    AccionAuditoria::new("acceso.consentimiento_biometrico.registrar");
pub const ACCION_REVOCAR_CONSENTIMIENTO: AccionAuditoria =
    AccionAuditoria::new("acceso.consentimiento_biometrico.revocar");

/// Credencial nueva.
#[derive(Debug, Clone)]
pub struct NuevaCredencial {
    pub titular: TitularAcceso,
    pub tipo: TipoCredencial,
    /// Número de tarjeta, PIN, placa o id de enrolamiento. Se ignora en los QR permanentes y la
    /// app (se generan aquí).
    pub valor: Option<String>,
    pub vigente_desde: Option<DateTime<Utc>>,
    pub vigente_hasta: Option<DateTime<Utc>>,
}

/// Credencial emitida y, si se generó aquí, su texto (se muestra una sola vez).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredencialEmitida {
    pub credencial: Credencial,
    pub secreto: Option<String>,
}

async fn exigir_titular<T: TxAcceso>(tx: &mut T, titular: TitularAcceso) -> AppResult<()> {
    let existe = match titular {
        TitularAcceso::Persona(p) => tx.persona(p).await?.is_some(),
        TitularAcceso::Vehiculo(v) => tx.vehiculo(v).await?.is_some(),
    };
    if existe {
        Ok(())
    } else {
        Err(AppError::NoEncontrado("titular"))
    }
}

async fn auditar_credencial<T: TxAcceso>(
    tx: &mut T,
    accion: AccionAuditoria,
    c: &Credencial,
) -> AppResult<()> {
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", c.tipo.as_str())?
        .con("estado", c.estado.as_str())?;
    tx.registrar(EventoAuditoria::ok(accion, "credencial", c.id.as_uuid()).con_detalle(detalle))
        .await
}

/// Emite una credencial. Las biométricas exigen `acceso.biometria.enrolar` y un consentimiento
/// vigente de la persona.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`] si el titular no existe;
/// [`AppError::Dominio`] si el identificador no tiene el formato del tipo;
/// [`AppError::Validacion`] sin consentimiento biométrico o con un tipo que no se emite aquí;
/// [`AppError::Conflicto`] si ya hay otra credencial vigente con el mismo identificador.
pub async fn emitir_credencial<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: NuevaCredencial,
) -> AppResult<CredencialEmitida>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoCredencialesGestionar)?;
    let ahora = reloj.ahora();
    let (valor, secreto) = match n.tipo {
        TipoCredencial::QrTemporal => {
            return Err(AppError::Validacion(
                "los QR temporales se emiten al pre-autorizar una visita".into(),
            ));
        }
        TipoCredencial::QrPermanente | TipoCredencial::AppMovil => {
            let t = token_aleatorio()?;
            (t.clone(), Some(t))
        }
        _ => (
            n.valor.clone().ok_or(AppError::Validacion(
                "indique el identificador de la credencial".into(),
            ))?,
            None,
        ),
    };
    let normalizado = n.tipo.normalizar(&valor)?;
    if let (Some(d), Some(h)) = (n.vigente_desde, n.vigente_hasta)
        && h <= d
    {
        return Err(AppError::Validacion(
            "la vigencia termina antes de empezar".into(),
        ));
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    exigir_titular(&mut tx, n.titular).await?;
    if n.tipo == TipoCredencial::BiometriaRef {
        ctx.exigir(Permiso::AccesoBiometriaEnrolar)?;
        let TitularAcceso::Persona(p) = n.titular else {
            return Err(AppError::Validacion(
                "la biometría solo aplica a personas".into(),
            ));
        };
        if tx.consentimiento_vigente(p).await?.is_none() {
            return Err(AppError::Validacion(
                "la persona no ha dado su consentimiento biométrico (debe ofrecerse un método alterno)".into(),
            ));
        }
    }
    if n.tipo == TipoCredencial::Placa && matches!(n.titular, TitularAcceso::Persona(_)) {
        return Err(AppError::Validacion(
            "las placas se emiten a vehículos".into(),
        ));
    }
    let c = Credencial {
        id: CredencialId::nuevo(),
        titular: n.titular,
        tipo: n.tipo,
        huella: huella_credencial(ctx.tenant_id, n.tipo, &normalizado),
        estado: EstadoCredencial::Activa,
        vigente_desde: n.vigente_desde.unwrap_or(ahora),
        vigente_hasta: n.vigente_hasta,
        borrado_confirmado_en: None,
        creada_en: ahora,
    };
    tx.insertar_credencial(&c).await?;
    auditar_credencial(&mut tx, ACCION_EMITIR, &c).await?;
    tx.confirmar().await?;
    Ok(CredencialEmitida {
        credencial: c,
        secreto,
    })
}

/// Suspende, reactiva, revoca o reporta perdida una credencial. El puente la retira en la
/// siguiente sincronización (invariante 1 del plan: menos de un minuto).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.credenciales.gestionar`; [`AppError::Dominio`] si la
/// transición no existe (revocada y perdida son definitivas).
pub async fn cambiar_estado_credencial<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CredencialId,
    hacia: EstadoCredencial,
) -> AppResult<Credencial>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoCredencialesGestionar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut c = tx
        .credencial(id)
        .await?
        .ok_or(AppError::NoEncontrado("credencial"))?;
    c.estado = c.estado.transicionar(hacia)?;
    tx.actualizar_credencial(&c).await?;
    auditar_credencial(&mut tx, ACCION_ESTADO, &c).await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Rota un QR permanente o de la app: revoca el actual y emite otro (p. ej. tras compartirlo
/// por error).
///
/// # Errors
///
/// Los de [`cambiar_estado_credencial`] y [`emitir_credencial`]; [`AppError::Validacion`] si no
/// es un QR permanente o de la app.
pub async fn rotar_qr<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CredencialId,
) -> AppResult<CredencialEmitida>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoCredencialesGestionar)?;
    let anterior = {
        let mut tx = abrir(unidad, reloj, ctx).await?;
        tx.credencial(id)
            .await?
            .ok_or(AppError::NoEncontrado("credencial"))?
    };
    if !matches!(
        anterior.tipo,
        TipoCredencial::QrPermanente | TipoCredencial::AppMovil
    ) {
        return Err(AppError::Validacion(
            "solo se rotan los QR permanentes y los de la app".into(),
        ));
    }
    cambiar_estado_credencial(unidad, reloj, ctx, id, EstadoCredencial::Revocada).await?;
    emitir_credencial(
        unidad,
        reloj,
        ctx,
        NuevaCredencial {
            titular: anterior.titular,
            tipo: anterior.tipo,
            valor: None,
            vigente_desde: None,
            vigente_hasta: anterior.vigente_hasta,
        },
    )
    .await
}

/// Credenciales de una persona o vehículo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.credenciales.gestionar`.
pub async fn credenciales_de<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    titular: TitularAcceso,
) -> AppResult<Vec<Credencial>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoCredencialesGestionar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.credenciales_de(titular).await
}

/// Autorización manual nueva.
#[derive(Debug, Clone)]
pub struct NuevaAutorizacion {
    pub titular: TitularAcceso,
    pub puntos: Vec<PuntoAccesoId>,
    pub horario: Option<Horario>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub motivo: Option<String>,
}

/// Crea una autorización manual (personal de servicio, proveedores frecuentes, horarios
/// especiales). Las de residentes y propietarios se derivan solas del censo y la titularidad.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.credenciales.gestionar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si la vigencia está invertida o un punto no existe.
pub async fn crear_autorizacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: NuevaAutorizacion,
) -> AppResult<AutorizacionAcceso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoCredencialesGestionar)?;
    if n.vigente_hasta.is_some_and(|h| h <= n.vigente_desde) {
        return Err(AppError::Validacion(
            "la vigencia termina antes de empezar".into(),
        ));
    }
    let motivo = texto_opcional("motivo", n.motivo.as_deref(), 300)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    exigir_titular(&mut tx, n.titular).await?;
    let puntos = tx.puntos_acceso().await?;
    if n.puntos.iter().any(|p| !puntos.iter().any(|x| x.id == *p)) {
        return Err(AppError::Validacion("un punto de acceso no existe".into()));
    }
    let a = AutorizacionAcceso {
        id: AutorizacionAccesoId::nuevo(),
        titular: n.titular,
        origen: OrigenAutorizacionAcceso::Manual,
        puntos: n.puntos,
        horario: n.horario,
        vigente_desde: n.vigente_desde,
        vigente_hasta: n.vigente_hasta,
        motivo,
        revocada_en: None,
    };
    tx.insertar_autorizacion_acceso(&a).await?;
    let detalle = DetalleAuditoria::vacio().con("puntos", a.puntos.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_AUTORIZAR, "autorizacion_acceso", a.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(a)
}

/// Revoca una autorización manual.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`] si no existe o ya estaba revocada.
pub async fn revocar_autorizacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: AutorizacionAccesoId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoCredencialesGestionar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    if !tx.revocar_autorizacion_acceso(id, reloj.ahora()).await? {
        return Err(AppError::NoEncontrado("autorización de acceso"));
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_REVOCAR_AUTORIZACION,
        "autorizacion_acceso",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Autorizaciones manuales de un titular.
///
/// # Errors
///
/// [`AppError::SinPermiso`].
pub async fn autorizaciones_de<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    titular: TitularAcceso,
) -> AppResult<Vec<AutorizacionAcceso>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoCredencialesGestionar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.autorizaciones_de(titular, true).await
}

/// Resultado de verificar una credencial en la portería.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verificacion {
    pub resultado: ResultadoAcceso,
    pub motivo: &'static str,
    pub titular: Option<TitularAcceso>,
    /// QR de una visita: la portería registra el ingreso con `ingreso_por_qr`.
    pub visita: Option<VisitaId>,
}

/// Verifica lo leído en la portería digital (QR con la cámara, PIN o tarjeta digitados) y deja
/// el intento en la bitácora. Un QR de visita no se consume aquí.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.registrar`; [`AppError::Dominio`] si el identificador
/// no tiene el formato del tipo.
pub async fn verificar_en_porteria<U>(
    unidad: &U,
    reloj: &impl Reloj,
    firma: &impl FirmaQr,
    ctx: &Contexto,
    texto: &str,
    tipo: TipoCredencial,
    punto: PuntoAccesoId,
) -> AppResult<Verificacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoRegistrar)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    if let Some((carga, sello)) = CargaQr::leer_texto(texto) {
        let valido =
            carga.tenant == ctx.tenant_id && firma.verificar(ctx.tenant_id, &carga.bytes(), &sello);
        let visita = if valido {
            tx.visita(carga.visita).await?
        } else {
            None
        };
        let (resultado, motivo) = match &visita {
            None => (ResultadoAcceso::Denegado, "QR inválido"),
            Some(v) if v.estado != domain::EstadoVisita::Autorizada => {
                (ResultadoAcceso::Denegado, "visita no autorizada")
            }
            Some(v) if !carga.admite(ahora, u32::from(v.usos)) => {
                (ResultadoAcceso::Denegado, "QR vencido o sin usos")
            }
            Some(_) => (ResultadoAcceso::Permitido, "visita autorizada"),
        };
        if resultado == ResultadoAcceso::Denegado {
            registrar_intento(&mut tx, ctx, punto, None, None, resultado, motivo, ahora).await?;
            tx.confirmar().await?;
        }
        return Ok(Verificacion {
            resultado,
            motivo,
            titular: None,
            visita: visita.map(|v| v.id),
        });
    }
    let huella = huella_credencial(ctx.tenant_id, tipo, &tipo.normalizar(texto)?);
    let credencial = tx.credencial_por_huella(&huella).await?;
    let tz = zona_ph(&mut tx).await?;
    let (resultado, motivo) = match &credencial {
        None => (ResultadoAcceso::Denegado, "credencial desconocida"),
        Some(c) if c.estado != EstadoCredencial::Activa => {
            (ResultadoAcceso::Denegado, "credencial inactiva")
        }
        Some(c) if c.vigente_desde > ahora || c.vigente_hasta.is_some_and(|h| h <= ahora) => {
            (ResultadoAcceso::Denegado, "credencial fuera de vigencia")
        }
        Some(c) => {
            let ventanas = tx.ventanas_de(c.titular, ahora).await?;
            let permite = ventanas.into_iter().any(|v| {
                VentanaAcceso {
                    puntos: v.puntos,
                    vigente_desde: v.vigente_desde,
                    vigente_hasta: v.vigente_hasta,
                    horario: v.horario,
                }
                .permite(punto, ahora, tz)
            });
            if permite {
                (ResultadoAcceso::Permitido, "autorizado")
            } else {
                (
                    ResultadoAcceso::Denegado,
                    "sin autorización en este punto u horario",
                )
            }
        }
    };
    let id = credencial.as_ref().map(|c| c.id);
    registrar_intento(&mut tx, ctx, punto, id, None, resultado, motivo, ahora).await?;
    tx.confirmar().await?;
    Ok(Verificacion {
        resultado,
        motivo,
        titular: credencial.map(|c| c.titular),
        visita: None,
    })
}

#[allow(clippy::too_many_arguments)]
async fn registrar_intento<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
    punto: PuntoAccesoId,
    credencial: Option<CredencialId>,
    visita: Option<VisitaId>,
    resultado: ResultadoAcceso,
    motivo: &str,
    ahora: DateTime<Utc>,
) -> AppResult<()> {
    let e = EventoAcceso {
        id: domain::EventoAccesoId::nuevo(),
        punto_acceso_id: Some(punto),
        dispositivo_id: None,
        puente_id: None,
        id_local: None,
        credencial_id: credencial,
        visita_id: visita,
        sentido: SentidoAcceso::Entrada,
        resultado,
        motivo: Some(motivo.to_owned()),
        placa: None,
        registrado_por: ctx.actor_id,
        ocurrido_en: ahora,
    };
    tx.insertar_evento_acceso(&e).await?;
    let detalle = DetalleAuditoria::vacio().con("resultado", resultado.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_VERIFICAR, "evento_acceso", e.id.as_uuid()).con_detalle(detalle),
    )
    .await
}

/// Consentimiento biométrico nuevo.
#[derive(Debug, Clone)]
pub struct NuevoConsentimiento {
    pub persona_id: PersonaId,
    /// Versión del texto de autorización que la persona aceptó.
    pub version_texto: String,
    /// Soporte firmado (PDF o foto), opcional.
    pub soporte: Option<Vec<u8>>,
}

/// Registra el consentimiento biométrico explícito de una persona.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.biometria.enrolar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya tiene uno vigente.
pub async fn registrar_consentimiento<U, A>(
    unidad: &U,
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: NuevoConsentimiento,
) -> AppResult<ConsentimientoBiometrico>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
    A: AlmacenArchivos,
{
    ctx.exigir(Permiso::AccesoBiometriaEnrolar)?;
    let version = texto_obligatorio("version_texto", &n.version_texto, 40)?;
    let soporte = match n.soporte {
        Some(contenido) => {
            let tipo = TipoArchivo::validar(&contenido)?;
            let subido = ArchivoSubido {
                nombre_original: NombreArchivo::new("consentimiento-biometrico")?,
                clasificacion: ClasificacionArchivo::Sensible,
                contenido,
            };
            let a = nuevo_archivo(ctx, almacen, reloj, &subido, tipo);
            almacen.guardar(&a.clave, &subido.contenido).await?;
            Some(a)
        }
        None => None,
    };
    let resultado = async {
        let mut tx = abrir(unidad, reloj, ctx).await?;
        tx.persona(n.persona_id)
            .await?
            .ok_or(AppError::NoEncontrado("persona"))?;
        if tx.consentimiento_vigente(n.persona_id).await?.is_some() {
            return Err(AppError::Conflicto(
                "la persona ya tiene un consentimiento biométrico vigente".into(),
            ));
        }
        if let Some(a) = &soporte {
            registrar_en(&mut tx, a).await?;
        }
        let c = ConsentimientoBiometrico {
            id: ConsentimientoBiometricoId::nuevo(),
            persona_id: n.persona_id,
            version_texto: version,
            soporte_archivo_id: soporte.as_ref().map(|a| a.id),
            otorgado_en: reloj.ahora(),
            revocado_en: None,
            registrado_por: ctx.actor_id,
        };
        tx.insertar_consentimiento(&c).await?;
        let detalle = DetalleAuditoria::vacio().con("version_texto", c.version_texto.clone())?;
        tx.registrar(
            EventoAuditoria::ok(
                ACCION_CONSENTIMIENTO,
                "consentimiento_biometrico",
                c.id.as_uuid(),
            )
            .con_detalle(detalle),
        )
        .await?;
        tx.confirmar().await?;
        Ok(c)
    }
    .await;
    if resultado.is_err()
        && let Some(a) = &soporte
    {
        borrar_huerfano(almacen, ctx, a).await;
    }
    resultado
}

/// Revoca el consentimiento biométrico y todas las credenciales biométricas de la persona: el
/// puente borra las plantillas del equipo y confirma el borrado.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.biometria.enrolar` (o la persona misma con
/// `acceso.visitantes.autorizar`); [`AppError::NoEncontrado`] si no tiene consentimiento
/// vigente.
pub async fn revocar_consentimiento<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    persona: PersonaId,
) -> AppResult<u32>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let propia = super::persona_del_actor(&mut tx, ctx).await? == Some(persona);
    if !propia {
        ctx.exigir(Permiso::AccesoBiometriaEnrolar)?;
    }
    let c = tx
        .consentimiento_vigente(persona)
        .await?
        .ok_or(AppError::NoEncontrado("consentimiento biométrico"))?;
    tx.revocar_consentimiento(c.id, ahora).await?;
    let mut revocadas = 0;
    for mut cred in tx.credenciales_de(TitularAcceso::Persona(persona)).await? {
        if cred.tipo == TipoCredencial::BiometriaRef
            && let Ok(e) = cred.estado.transicionar(EstadoCredencial::Revocada)
        {
            cred.estado = e;
            tx.actualizar_credencial(&cred).await?;
            revocadas += 1;
        }
    }
    let detalle = DetalleAuditoria::vacio().con("credenciales_revocadas", revocadas)?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_REVOCAR_CONSENTIMIENTO,
            "consentimiento_biometrico",
            c.id.as_uuid(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(revocadas)
}

/// Consentimiento biométrico vigente de una persona.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.biometria.enrolar`.
pub async fn consentimiento_de<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    persona: PersonaId,
) -> AppResult<Option<ConsentimientoBiometrico>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoBiometriaEnrolar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.consentimiento_vigente(persona).await
}
