//! Cuentas y sesiones: registro, verificación del correo, inicio de sesión con usuario o correo
//! y contraseña, segundo factor (TOTP), renovación de tokens, cierre y recuperación de la
//! contraseña.
//!
//! Tokens **opacos** (no JWT): acceso de 30 minutos y refresh rotativo hasta el vencimiento
//! absoluto de la sesión (30 días; 12 horas en un equipo compartido como el de portería). Si un
//! refresh ya usado vuelve a llegar, alguien lo copió: se revoca la sesión entera.

use chrono::{DateTime, Duration, Utc};
use domain::{
    ConjuntoPermisos, ContrasenaNueva, CuentaId, DireccionCorreo, EstadoCuenta, Identificador,
    NombreUsuario, SesionId,
};

use crate::{
    AppError, AppResult, ContextoPlataforma,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria, ResultadoAuditoria},
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        HashContrasenas, RegistroAuditoria, RegistroOutbox, Reloj, RepositorioCuentas,
        RepositorioPlataforma, RepositorioSesiones, SegundoFactor, Transaccion,
        UnidadDeTrabajoPlataforma,
        correo::CorreoSaliente,
        identidad::{CodigoVerificacion, Cuenta, HashesTokens, Membresia, PropositoCodigo, Sesion},
    },
    seguridad::{codigo_numerico, hash_secreto, iguales, token_aleatorio},
};

pub const ACCION_REGISTRAR: AccionAuditoria = AccionAuditoria::new("cuentas.cuenta.registrar");
pub const ACCION_VERIFICAR_CORREO: AccionAuditoria =
    AccionAuditoria::new("cuentas.correo.verificar");
pub const ACCION_INICIAR_SESION: AccionAuditoria = AccionAuditoria::new("cuentas.sesion.iniciar");
pub const ACCION_REUTILIZACION: AccionAuditoria =
    AccionAuditoria::new("cuentas.sesion.reutilizacion_detectada");
pub const ACCION_CERRAR_SESIONES: AccionAuditoria =
    AccionAuditoria::new("cuentas.sesion.cerrar_todas");
pub const ACCION_CAMBIAR_CONTRASENA: AccionAuditoria =
    AccionAuditoria::new("cuentas.contrasena.cambiar");
pub const ACCION_RESTABLECER_CONTRASENA: AccionAuditoria =
    AccionAuditoria::new("cuentas.contrasena.restablecer");
pub const ACCION_ACTIVAR_SEGUNDO_FACTOR: AccionAuditoria =
    AccionAuditoria::new("cuentas.segundo_factor.activar");

/// Vida del token de acceso.
pub const VIDA_ACCESO: Duration = Duration::minutes(30);
/// Vencimiento absoluto de una sesión normal.
pub const VIDA_SESION: Duration = Duration::days(30);
/// Vencimiento absoluto en un equipo compartido (un turno de portería).
pub const VIDA_SESION_COMPARTIDA: Duration = Duration::hours(12);
/// Tiempo para escribir el código TOTP tras la contraseña.
pub const VIDA_DESAFIO: Duration = Duration::minutes(5);
/// Vida de un código enviado por correo.
pub const VIDA_CODIGO: Duration = Duration::minutes(30);
/// Intentos fallidos de contraseña antes del bloqueo temporal.
pub const MAX_INTENTOS: u32 = 5;
pub const DURACION_BLOQUEO: Duration = Duration::minutes(15);
/// Intentos para escribir bien un código.
pub const MAX_INTENTOS_CODIGO: u32 = 5;

/// Datos de registro de una cuenta.
#[derive(Debug, Clone)]
pub struct Registro {
    pub nombre_mostrar: String,
    pub usuario: Option<NombreUsuario>,
    pub correo: Option<DireccionCorreo>,
    pub contrasena: ContrasenaNueva,
}

/// Lo que recibe el cliente al iniciar sesión. Los tokens solo existen aquí, en claro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokensSesion {
    pub sesion_id: SesionId,
    pub acceso: String,
    pub acceso_expira_en: DateTime<Utc>,
    pub refresh: String,
    pub expira_maximo_en: DateTime<Utc>,
}

/// Resultado de usuario + contraseña.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultadoInicio {
    Sesion(TokensSesion),
    /// La cuenta tiene segundo factor: falta el código TOTP. `desafio` es un token de un solo
    /// uso que solo sirve para [`completar_segundo_factor`].
    SegundoFactor {
        desafio: String,
        expira_en: DateTime<Utc>,
    },
}

/// Petición de inicio de sesión.
#[derive(Debug, Clone)]
pub struct SolicitudInicio {
    pub identificador: Identificador,
    pub contrasena: String,
    /// Equipo compartido (portería): la sesión dura un turno.
    pub equipo_compartido: bool,
    /// `User-Agent` recortado, para que la persona reconozca sus sesiones.
    pub agente: Option<String>,
}

/// Sesión válida de una petición.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SesionAutenticada {
    pub sesion_id: SesionId,
    pub cuenta_id: CuentaId,
    pub es_superusuario: bool,
    pub permisos_plataforma: ConjuntoPermisos,
    pub segundo_factor_verificado: bool,
}

/// Perfil de la cuenta autenticada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Perfil {
    pub id: CuentaId,
    pub nombre_mostrar: String,
    pub usuario: Option<NombreUsuario>,
    pub correo: Option<DireccionCorreo>,
    pub correo_verificado: bool,
    pub segundo_factor_activo: bool,
    pub es_superusuario: bool,
    pub permisos_plataforma: ConjuntoPermisos,
}

fn evento_cuenta(accion: AccionAuditoria, cuenta: CuentaId) -> EventoAuditoria {
    EventoAuditoria::ok(accion, "cuenta", cuenta.as_uuid())
}

/// Crea una cuenta. Si trae correo, envía el código para verificarlo.
///
/// # Errors
///
/// - [`AppError::Validacion`] si el nombre está vacío o no hay usuario ni correo.
/// - [`AppError::Conflicto`] si el usuario o el correo ya están registrados.
pub async fn registrar_cuenta<UP, H>(
    unidad: &UP,
    hasher: &H,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    registro: Registro,
) -> AppResult<CuentaId>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas + RegistroAuditoria + RegistroOutbox,
    H: HashContrasenas,
{
    let nombre = registro.nombre_mostrar.trim().to_owned();
    if nombre.is_empty() || nombre.chars().count() > 120 {
        return Err(AppError::Validacion(
            "el nombre debe tener entre 1 y 120 caracteres".into(),
        ));
    }
    if registro.usuario.is_none() && registro.correo.is_none() {
        return Err(AppError::Validacion(
            "indique un nombre de usuario o un correo".into(),
        ));
    }
    let ahora = reloj.ahora();
    let cuenta = Cuenta {
        id: CuentaId::nuevo(),
        nombre_mostrar: nombre,
        usuario: registro.usuario,
        correo: registro.correo,
        correo_verificado: false,
        estado: EstadoCuenta::Activa,
        hash_contrasena: hasher.hash(&registro.contrasena).await?,
        intentos_fallidos: 0,
        bloqueo_hasta: None,
        totp_secreto: None,
        totp_activo: false,
        creada_en: ahora,
    };

    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.insertar_cuenta(&cuenta).await?;
    if let Some(correo) = &cuenta.correo {
        enviar_codigo(
            &mut tx,
            &cuenta,
            correo,
            PropositoCodigo::VerificarCorreo,
            ahora,
        )
        .await?;
    }
    tx.registrar(evento_cuenta(ACCION_REGISTRAR, cuenta.id))
        .await?;
    tx.confirmar().await?;
    Ok(cuenta.id)
}

async fn enviar_codigo<T>(
    tx: &mut T,
    cuenta: &Cuenta,
    correo: &DireccionCorreo,
    proposito: PropositoCodigo,
    ahora: DateTime<Utc>,
) -> AppResult<()>
where
    T: RepositorioCuentas + RegistroOutbox,
{
    let codigo = codigo_numerico()?;
    tx.insertar_codigo(&CodigoVerificacion {
        id: uuid::Uuid::now_v7(),
        cuenta_id: cuenta.id,
        proposito,
        hash_codigo: hash_secreto(&codigo),
        expira_en: ahora + VIDA_CODIGO,
        intentos: 0,
        creado_en: ahora,
    })
    .await?;
    let (asunto, texto) = match proposito {
        PropositoCodigo::VerificarCorreo => (
            "Verifique su correo en DAD Copropiedad",
            format!(
                "Hola, {}.\n\nSu código para verificar el correo es: {codigo}\n\nVence en 30 \
                 minutos. Si usted no creó la cuenta, ignore este mensaje.",
                cuenta.nombre_mostrar
            ),
        ),
        PropositoCodigo::RecuperarContrasena => (
            "Recupere su contraseña de DAD Copropiedad",
            format!(
                "Hola, {}.\n\nSu código para restablecer la contraseña es: {codigo}\n\nVence en \
                 30 minutos. Si usted no lo pidió, ignore este mensaje: su contraseña no cambia.",
                cuenta.nombre_mostrar
            ),
        ),
    };
    tx.encolar(EventoSaliente {
        prioridad: PrioridadSalida::Confirmacion,
        mensaje: MensajeOutbox::Correo(CorreoSaliente {
            para: correo.clone(),
            asunto: asunto.into(),
            texto,
            html: None,
        }),
    })
    .await
}

/// Verifica un código de un solo uso: lo marca usado si es correcto, cuenta el intento si no.
async fn consumir_codigo<T: RepositorioCuentas>(
    tx: &mut T,
    cuenta: CuentaId,
    proposito: PropositoCodigo,
    codigo: &str,
    ahora: DateTime<Utc>,
) -> AppResult<bool> {
    let Some(guardado) = tx.codigo_vigente(cuenta, proposito, ahora).await? else {
        return Ok(false);
    };
    if guardado.intentos >= MAX_INTENTOS_CODIGO {
        return Ok(false);
    }
    if iguales(&guardado.hash_codigo, &hash_secreto(codigo.trim())) {
        tx.marcar_codigo_usado(guardado.id, ahora).await?;
        Ok(true)
    } else {
        tx.registrar_intento_codigo(guardado.id).await?;
        Ok(false)
    }
}

/// Vuelve a enviar el código de verificación del correo de la cuenta autenticada.
///
/// # Errors
///
/// [`AppError::Validacion`] si la cuenta no tiene correo o ya está verificado.
pub async fn reenviar_verificacion<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas + RegistroOutbox,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let cuenta = cuenta_existente(&mut tx, ctx.cuenta()?).await?;
    let correo = match (&cuenta.correo, cuenta.correo_verificado) {
        (Some(c), false) => c.clone(),
        _ => {
            return Err(AppError::Validacion(
                "la cuenta no tiene un correo pendiente de verificar".into(),
            ));
        }
    };
    enviar_codigo(
        &mut tx,
        &cuenta,
        &correo,
        PropositoCodigo::VerificarCorreo,
        reloj.ahora(),
    )
    .await?;
    tx.confirmar().await
}

/// Verifica el correo de la cuenta autenticada con el código recibido.
///
/// # Errors
///
/// [`AppError::Validacion`] si el código no es válido, venció o se agotaron los intentos.
pub async fn verificar_correo<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    codigo: &str,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas + RegistroAuditoria,
{
    let ahora = reloj.ahora();
    let cuenta = ctx.cuenta()?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let valido = consumir_codigo(
        &mut tx,
        cuenta,
        PropositoCodigo::VerificarCorreo,
        codigo,
        ahora,
    )
    .await?;
    if !valido {
        // Se confirma igual: el intento fallido debe quedar contado.
        tx.confirmar().await?;
        return Err(AppError::Validacion("código inválido o vencido".into()));
    }
    tx.marcar_correo_verificado(cuenta, ahora).await?;
    tx.registrar(evento_cuenta(ACCION_VERIFICAR_CORREO, cuenta))
        .await?;
    tx.confirmar().await
}

async fn cuenta_existente<T: RepositorioCuentas>(tx: &mut T, id: CuentaId) -> AppResult<Cuenta> {
    tx.cuenta_por_id(id).await?.ok_or(AppError::NoAutenticado)
}

fn nuevos_tokens(
    ahora: DateTime<Utc>,
    expira_maximo_en: DateTime<Utc>,
) -> AppResult<(String, String, HashesTokens)> {
    let acceso = token_aleatorio()?;
    let refresh = token_aleatorio()?;
    let hashes = HashesTokens {
        acceso: hash_secreto(&acceso),
        acceso_expira_en: (ahora + VIDA_ACCESO).min(expira_maximo_en),
        refresh: Some(hash_secreto(&refresh)),
    };
    Ok((acceso, refresh, hashes))
}

/// Inicia sesión con usuario o correo y contraseña.
///
/// Tras [`MAX_INTENTOS`] fallos seguidos la cuenta queda bloqueada [`DURACION_BLOQUEO`]. Una
/// cuenta inexistente, bloqueada o con la contraseña equivocada responden igual.
///
/// # Errors
///
/// [`AppError::CredencialesInvalidas`] o [`AppError::DemasiadosIntentos`].
pub async fn iniciar_sesion<UP, H>(
    unidad: &UP,
    hasher: &H,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    solicitud: SolicitudInicio,
) -> AppResult<ResultadoInicio>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas + RepositorioSesiones + RegistroAuditoria,
    H: HashContrasenas,
{
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let Some(cuenta) = tx
        .cuenta_por_identificador(&solicitud.identificador)
        .await?
    else {
        hasher.igualar_tiempo(&solicitud.contrasena).await;
        return Err(AppError::CredencialesInvalidas);
    };
    if cuenta.bloqueo_hasta.is_some_and(|b| b > ahora) {
        return Err(AppError::DemasiadosIntentos);
    }
    let correcta = hasher
        .verificar(&solicitud.contrasena, &cuenta.hash_contrasena)
        .await;
    if !correcta || cuenta.estado != EstadoCuenta::Activa {
        let intentos = cuenta.intentos_fallidos + 1;
        let bloqueo = (intentos >= MAX_INTENTOS).then(|| ahora + DURACION_BLOQUEO);
        tx.registrar_fallo(
            cuenta.id,
            if bloqueo.is_some() { 0 } else { intentos },
            bloqueo,
        )
        .await?;
        let detalle = DetalleAuditoria::vacio().con("bloqueada", bloqueo.is_some())?;
        tx.registrar(EventoAuditoria {
            resultado: ResultadoAuditoria::Rechazado,
            ..evento_cuenta(ACCION_INICIAR_SESION, cuenta.id).con_detalle(detalle)
        })
        .await?;
        tx.confirmar().await?;
        return Err(AppError::CredencialesInvalidas);
    }

    tx.registrar_acceso(cuenta.id, ahora).await?;
    let vida = if solicitud.equipo_compartido {
        VIDA_SESION_COMPARTIDA
    } else {
        VIDA_SESION
    };
    let expira_maximo_en = ahora + vida;
    let mut sesion = Sesion {
        id: SesionId::nuevo(),
        cuenta_id: cuenta.id,
        hash_acceso: String::new(),
        acceso_expira_en: ahora,
        hash_refresh: None,
        expira_maximo_en,
        segundo_factor_pendiente: cuenta.totp_activo,
        segundo_factor_verificado: false,
        equipo_compartido: solicitud.equipo_compartido,
        agente: solicitud.agente.map(|a| a.chars().take(200).collect()),
        creada_en: ahora,
        revocada: false,
    };
    let resultado = if cuenta.totp_activo {
        let desafio = token_aleatorio()?;
        sesion.hash_acceso = hash_secreto(&desafio);
        sesion.acceso_expira_en = ahora + VIDA_DESAFIO;
        ResultadoInicio::SegundoFactor {
            desafio,
            expira_en: sesion.acceso_expira_en,
        }
    } else {
        let (acceso, refresh, hashes) = nuevos_tokens(ahora, expira_maximo_en)?;
        sesion.hash_acceso = hashes.acceso;
        sesion.acceso_expira_en = hashes.acceso_expira_en;
        sesion.hash_refresh = hashes.refresh;
        ResultadoInicio::Sesion(TokensSesion {
            sesion_id: sesion.id,
            acceso,
            acceso_expira_en: sesion.acceso_expira_en,
            refresh,
            expira_maximo_en,
        })
    };
    tx.insertar_sesion(&sesion).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("sesion_id", sesion.id.to_string())?
        .con("equipo_compartido", sesion.equipo_compartido)?
        .con("segundo_factor_pendiente", sesion.segundo_factor_pendiente)?;
    tx.registrar(evento_cuenta(ACCION_INICIAR_SESION, cuenta.id).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    Ok(resultado)
}

/// Completa el inicio de sesión con el código TOTP.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] si el desafío no existe o venció;
/// [`AppError::CredencialesInvalidas`] si el código no es correcto.
pub async fn completar_segundo_factor<UP, F>(
    unidad: &UP,
    totp: &F,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    desafio: &str,
    codigo: &str,
) -> AppResult<TokensSesion>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas + RepositorioSesiones + RegistroAuditoria,
    F: SegundoFactor,
{
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let sesion = tx
        .sesion_por_acceso(&hash_secreto(desafio))
        .await?
        .filter(|s| s.segundo_factor_pendiente && !s.revocada && s.acceso_expira_en > ahora)
        .ok_or(AppError::NoAutenticado)?;
    let cuenta = cuenta_existente(&mut tx, sesion.cuenta_id).await?;
    let secreto = cuenta.totp_secreto.as_deref().unwrap_or_default();
    if !cuenta.totp_activo || !totp.verificar(secreto, codigo, ahora) {
        tx.revocar_sesion(sesion.id, "segundo factor incorrecto", ahora)
            .await?;
        tx.confirmar().await?;
        return Err(AppError::CredencialesInvalidas);
    }
    let (acceso, refresh, hashes) = nuevos_tokens(ahora, sesion.expira_maximo_en)?;
    tx.completar_segundo_factor(sesion.id, &hashes).await?;
    tx.confirmar().await?;
    Ok(TokensSesion {
        sesion_id: sesion.id,
        acceso,
        acceso_expira_en: hashes.acceso_expira_en,
        refresh,
        expira_maximo_en: sesion.expira_maximo_en,
    })
}

/// Valida el token de acceso de una petición.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] si el token no existe, venció, la sesión fue revocada, espera el
/// segundo factor o la cuenta ya no está activa.
pub async fn autenticar<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    token_acceso: &str,
) -> AppResult<SesionAutenticada>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas + RepositorioSesiones + RepositorioPlataforma,
{
    let ahora = reloj.ahora();
    let mut tx = unidad
        .iniciar_plataforma(&ContextoPlataforma::anonimo(None))
        .await?;
    let sesion = tx
        .sesion_por_acceso(&hash_secreto(token_acceso))
        .await?
        .filter(|s| {
            !s.revocada
                && !s.segundo_factor_pendiente
                && s.acceso_expira_en > ahora
                && s.expira_maximo_en > ahora
        })
        .ok_or(AppError::NoAutenticado)?;
    let cuenta = cuenta_existente(&mut tx, sesion.cuenta_id).await?;
    if cuenta.estado != EstadoCuenta::Activa {
        return Err(AppError::NoAutenticado);
    }
    let (permisos_plataforma, es_superusuario) = tx.permisos_plataforma(cuenta.id, ahora).await?;
    Ok(SesionAutenticada {
        sesion_id: sesion.id,
        cuenta_id: cuenta.id,
        es_superusuario,
        permisos_plataforma,
        segundo_factor_verificado: sesion.segundo_factor_verificado,
    })
}

/// Cambia el refresh por un par de tokens nuevos. Si llega un refresh **ya usado**, revoca la
/// sesión (alguien lo copió) y lo audita.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] si el refresh no es válido.
pub async fn renovar_sesion<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    refresh: &str,
) -> AppResult<TokensSesion>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioSesiones + RegistroAuditoria,
{
    let ahora = reloj.ahora();
    let hash = hash_secreto(refresh);
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let Some(sesion) = tx.sesion_por_refresh(&hash).await? else {
        if let Some(reutilizada) = tx.sesion_por_refresh_anterior(&hash).await? {
            tx.revocar_sesion(reutilizada, "refresh reutilizado", ahora)
                .await?;
            let detalle = DetalleAuditoria::vacio().con("sesion_id", reutilizada.to_string())?;
            tx.registrar(EventoAuditoria {
                accion: ACCION_REUTILIZACION,
                entidad: "sesion",
                entidad_id: Some(reutilizada.as_uuid()),
                resultado: ResultadoAuditoria::Rechazado,
                detalle,
            })
            .await?;
            tx.confirmar().await?;
        }
        return Err(AppError::NoAutenticado);
    };
    if sesion.revocada || sesion.segundo_factor_pendiente || sesion.expira_maximo_en <= ahora {
        return Err(AppError::NoAutenticado);
    }
    let (acceso, nuevo_refresh, hashes) = nuevos_tokens(ahora, sesion.expira_maximo_en)?;
    tx.rotar_tokens(sesion.id, &hashes, Some(&hash), ahora)
        .await?;
    tx.confirmar().await?;
    Ok(TokensSesion {
        sesion_id: sesion.id,
        acceso,
        acceso_expira_en: hashes.acceso_expira_en,
        refresh: nuevo_refresh,
        expira_maximo_en: sesion.expira_maximo_en,
    })
}

/// Cierra la sesión actual.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn cerrar_sesion<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    sesion: SesionId,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioSesiones,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.revocar_sesion(sesion, "cerrada por la persona", reloj.ahora())
        .await?;
    tx.confirmar().await
}

/// Cierra todas las sesiones de la cuenta autenticada (todos los dispositivos).
///
/// # Errors
///
/// [`AppError::NoAutenticado`] sin sesión.
pub async fn cerrar_todas_las_sesiones<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
) -> AppResult<u64>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioSesiones + RegistroAuditoria,
{
    let cuenta = ctx.cuenta()?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let cerradas = tx
        .revocar_sesiones_de(cuenta, "cerradas por la persona", reloj.ahora())
        .await?;
    tx.registrar(evento_cuenta(ACCION_CERRAR_SESIONES, cuenta))
        .await?;
    tx.confirmar().await?;
    Ok(cerradas)
}

/// Envía un código para restablecer la contraseña. **Siempre** responde bien, exista o no la
/// cuenta, para no revelar qué correos están registrados.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn solicitar_recuperacion<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    correo: &DireccionCorreo,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas + RegistroOutbox,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let cuenta = tx
        .cuenta_por_identificador(&Identificador::Correo(correo.clone()))
        .await?
        .filter(|c| c.estado == EstadoCuenta::Activa);
    if let Some(cuenta) = cuenta {
        enviar_codigo(
            &mut tx,
            &cuenta,
            correo,
            PropositoCodigo::RecuperarContrasena,
            reloj.ahora(),
        )
        .await?;
        tx.confirmar().await?;
    }
    Ok(())
}

/// Restablece la contraseña con el código recibido por correo y cierra todas las sesiones.
///
/// # Errors
///
/// [`AppError::Validacion`] si el código no es válido o venció.
pub async fn restablecer_contrasena<UP, H>(
    unidad: &UP,
    hasher: &H,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    correo: &DireccionCorreo,
    codigo: &str,
    nueva: &ContrasenaNueva,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas + RepositorioSesiones + RegistroAuditoria,
    H: HashContrasenas,
{
    let invalido = || AppError::Validacion("código inválido o vencido".into());
    let ahora = reloj.ahora();
    let hash = hasher.hash(nueva).await?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let cuenta = tx
        .cuenta_por_identificador(&Identificador::Correo(correo.clone()))
        .await?
        .ok_or_else(invalido)?;
    if !consumir_codigo(
        &mut tx,
        cuenta.id,
        PropositoCodigo::RecuperarContrasena,
        codigo,
        ahora,
    )
    .await?
    {
        tx.confirmar().await?;
        return Err(invalido());
    }
    tx.cambiar_contrasena(cuenta.id, &hash, ahora).await?;
    // Quien recibió el código controla el correo: queda verificado.
    tx.marcar_correo_verificado(cuenta.id, ahora).await?;
    tx.revocar_sesiones_de(cuenta.id, "contraseña restablecida", ahora)
        .await?;
    tx.registrar(evento_cuenta(ACCION_RESTABLECER_CONTRASENA, cuenta.id))
        .await?;
    tx.confirmar().await
}

/// Cambia la contraseña de la cuenta autenticada (exige la actual) y cierra **todas** sus
/// sesiones: el cliente vuelve a iniciar sesión con la nueva.
///
/// # Errors
///
/// [`AppError::CredencialesInvalidas`] si la contraseña actual no es correcta.
pub async fn cambiar_contrasena<UP, H>(
    unidad: &UP,
    hasher: &H,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    actual: &str,
    nueva: &ContrasenaNueva,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas + RepositorioSesiones + RegistroAuditoria,
    H: HashContrasenas,
{
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let cuenta = cuenta_existente(&mut tx, ctx.cuenta()?).await?;
    if !hasher.verificar(actual, &cuenta.hash_contrasena).await {
        return Err(AppError::CredencialesInvalidas);
    }
    let hash = hasher.hash(nueva).await?;
    tx.cambiar_contrasena(cuenta.id, &hash, ahora).await?;
    tx.revocar_sesiones_de(cuenta.id, "contraseña cambiada", ahora)
        .await?;
    tx.registrar(evento_cuenta(ACCION_CAMBIAR_CONTRASENA, cuenta.id))
        .await?;
    tx.confirmar().await
}

/// Secreto TOTP nuevo (pendiente de confirmar) y su URI para el código QR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InscripcionSegundoFactor {
    pub secreto: String,
    pub uri: String,
}

/// Empieza a activar el segundo factor: genera un secreto que se confirma con
/// [`confirmar_segundo_factor`].
///
/// # Errors
///
/// [`AppError::Conflicto`] si ya está activo.
pub async fn iniciar_segundo_factor<UP, F>(
    unidad: &UP,
    totp: &F,
    ctx: &ContextoPlataforma,
) -> AppResult<InscripcionSegundoFactor>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas,
    F: SegundoFactor,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let cuenta = cuenta_existente(&mut tx, ctx.cuenta()?).await?;
    if cuenta.totp_activo {
        return Err(AppError::Conflicto(
            "el segundo factor ya está activo".into(),
        ));
    }
    let secreto = totp.generar_secreto();
    let etiqueta = cuenta
        .usuario
        .as_ref()
        .map(|u| u.as_str().to_owned())
        .or_else(|| cuenta.correo.as_ref().map(|c| c.as_str().to_owned()))
        .unwrap_or_else(|| cuenta.id.to_string());
    tx.guardar_totp(cuenta.id, Some(&secreto), None).await?;
    tx.confirmar().await?;
    Ok(InscripcionSegundoFactor {
        uri: totp.uri(&secreto, &etiqueta),
        secreto,
    })
}

/// Confirma el segundo factor con un código de la app autenticadora.
///
/// # Errors
///
/// [`AppError::Validacion`] si no hay inscripción pendiente o el código no es correcto.
pub async fn confirmar_segundo_factor<UP, F>(
    unidad: &UP,
    totp: &F,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    codigo: &str,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas + RegistroAuditoria,
    F: SegundoFactor,
{
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let cuenta = cuenta_existente(&mut tx, ctx.cuenta()?).await?;
    let secreto = match (&cuenta.totp_secreto, cuenta.totp_activo) {
        (Some(s), false) => s.clone(),
        _ => {
            return Err(AppError::Validacion(
                "no hay una activación de segundo factor pendiente".into(),
            ));
        }
    };
    if !totp.verificar(&secreto, codigo, ahora) {
        return Err(AppError::Validacion("código incorrecto".into()));
    }
    tx.guardar_totp(cuenta.id, Some(&secreto), Some(ahora))
        .await?;
    tx.registrar(evento_cuenta(ACCION_ACTIVAR_SEGUNDO_FACTOR, cuenta.id))
        .await?;
    tx.confirmar().await
}

/// Perfil de la cuenta autenticada.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] sin sesión.
pub async fn mi_perfil<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
) -> AppResult<Perfil>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioCuentas + RepositorioPlataforma,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let cuenta = cuenta_existente(&mut tx, ctx.cuenta()?).await?;
    let (permisos_plataforma, es_superusuario) =
        tx.permisos_plataforma(cuenta.id, reloj.ahora()).await?;
    Ok(Perfil {
        id: cuenta.id,
        nombre_mostrar: cuenta.nombre_mostrar,
        usuario: cuenta.usuario,
        correo: cuenta.correo,
        correo_verificado: cuenta.correo_verificado,
        segundo_factor_activo: cuenta.totp_activo,
        es_superusuario,
        permisos_plataforma,
    })
}

/// Copropiedades donde la cuenta autenticada tiene roles vigentes.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] sin sesión.
pub async fn mis_copropiedades<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<Membresia>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.membresias(ctx.cuenta()?).await
}
