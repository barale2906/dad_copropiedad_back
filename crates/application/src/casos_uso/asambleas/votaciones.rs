//! Preguntas y votos: crear (sí/no, selección única o múltiple, elección por cociente), abrir
//! (habilita a los presentes y les envía la pregunta por WhatsApp), emitir el voto del
//! asistente por **todos** sus bienes desde cualquier canal, cerrar (también por temporizador)
//! con su escrutinio, detalle nominal o reservado y roles derivados de las elecciones.

use chrono::{DateTime, Duration, Utc};
use domain::{
    AsignacionRolId, AsistenteId, BaseComputo, BienRepresentado, Canal, CargoEleccion,
    ClaveIdempotencia, CodigoRotulo, Conteo, Decision, Escrutinio, EstadoPregunta, MomentoQuorum,
    OpcionId, OpcionPregunta, OrigenAsignacion, Papeleta, Permiso, PersonaId, Pregunta, PreguntaId,
    PuntoOrdenId, ReglaMayoria, ReunionId, TEXTOS_SI_NO, TipoPregunta, TipoVoto, VotosOpcion,
};

use super::{
    ACCION_ABRIR, ACCION_ANULAR, ACCION_CERRAR, ACCION_PREGUNTA, ACCION_ROLES, ACCION_VER_DETALLE,
    ACCION_VOTAR, TxAsambleas, abrir,
    quorum::{calcular_quorum, pueden_votar, tomar_foto},
    reunion_de, texto,
    whatsapp::{confirmacion_voto, mensaje_pregunta},
};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{DetalleAuditoria, EventoAuditoria},
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioAsambleas, RepositorioPersonas, RepositorioRoles,
        Transaccion, UnidadDeTrabajo,
        asambleas::{Asistente, BienPresente, DetalleVoto, PreguntaGuardada, Reunion},
        identidad::Asignacion,
        mensajeria::{ContenidoSaliente, MensajeSaliente},
    },
};

/// Opción nueva de una pregunta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevaOpcion {
    pub texto: String,
    pub en_blanco: bool,
    /// Integrantes de la plancha o candidato (elecciones), en orden.
    pub candidatos: Vec<PersonaId>,
}

/// Definición de una pregunta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinicionPregunta {
    pub enunciado: String,
    pub tipo: TipoPregunta,
    pub tipo_voto: TipoVoto,
    /// `None` = la de la reunión.
    pub base_computo: Option<BaseComputo>,
    pub regla_mayoria: ReglaMayoria,
    pub max_selecciones: u16,
    pub cupos: Option<u16>,
    pub cargo: Option<CargoEleccion>,
    /// En sí/no se puede omitir: se usan «Sí», «No» y, si `con_abstencion`, «Abstención».
    pub opciones: Vec<NuevaOpcion>,
    pub con_abstencion: bool,
    pub punto_id: Option<PuntoOrdenId>,
    /// `None` = el de la reunión; `Some(0)` = sin temporizador.
    pub temporizador_segundos: Option<u32>,
}

fn construir(
    reunion: &Reunion,
    id: PreguntaId,
    orden: u16,
    d: DefinicionPregunta,
) -> AppResult<PreguntaGuardada> {
    let mut opciones = d.opciones;
    if d.tipo == TipoPregunta::SiNo && opciones.is_empty() {
        let n = if d.con_abstencion { 3 } else { 2 };
        opciones = TEXTOS_SI_NO[..n]
            .iter()
            .map(|t| NuevaOpcion {
                texto: (*t).to_owned(),
                en_blanco: false,
                candidatos: Vec::new(),
            })
            .collect();
    }
    if opciones.len() > 50 {
        return Err(AppError::Validacion("máximo 50 opciones".into()));
    }
    let mut candidatos = Vec::new();
    let mut ops = Vec::with_capacity(opciones.len());
    for (i, o) in opciones.into_iter().enumerate() {
        let opcion = OpcionPregunta {
            id: OpcionId::nuevo(),
            orden: u16::try_from(i + 1).unwrap_or(u16::MAX),
            texto: texto("opcion", &o.texto, 300)?,
            en_blanco: o.en_blanco,
        };
        if !o.candidatos.is_empty() {
            if d.tipo != TipoPregunta::Eleccion {
                return Err(AppError::Validacion(
                    "los candidatos solo aplican a elecciones".into(),
                ));
            }
            candidatos.push((opcion.id, o.candidatos));
        }
        ops.push(opcion);
    }
    let pregunta = Pregunta {
        id,
        reunion_id: reunion.id,
        enunciado: texto("enunciado", &d.enunciado, 1000)?,
        tipo: d.tipo,
        tipo_voto: d.tipo_voto,
        base_computo: d.base_computo.unwrap_or(reunion.configuracion.base_computo),
        regla_mayoria: d.regla_mayoria,
        max_selecciones: d.max_selecciones.max(1),
        cupos: d.cupos,
        cargo: d.cargo,
        estado: EstadoPregunta::Borrador,
        opciones: ops,
    };
    pregunta.validar_definicion()?;
    let temporizador = match d.temporizador_segundos {
        Some(0) => None,
        Some(s) if !(10..=86_400).contains(&s) => {
            return Err(AppError::Validacion(
                "el temporizador va de 10 segundos a 24 horas".into(),
            ));
        }
        Some(s) => Some(s),
        None => reunion.configuracion.temporizador_segundos,
    };
    Ok(PreguntaGuardada {
        pregunta,
        orden,
        punto_id: d.punto_id,
        temporizador_segundos: temporizador,
        cierra_en: None,
        abierta_en: None,
        cerrada_en: None,
        resultado: None,
        candidatos,
        roles_aplicados_en: None,
    })
}

fn exigir_preparacion(ctx: &Contexto) -> AppResult<()> {
    if ctx.exigir(Permiso::AsambleasConducir).is_ok() {
        Ok(())
    } else {
        ctx.exigir(Permiso::AsambleasConvocar)
    }
}

/// Crea una pregunta en borrador (antes o durante la reunión).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.convocar` ni `asambleas.conducir`;
/// [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si la reunión terminó;
/// [`AppError::Dominio`] o [`AppError::Validacion`] si la definición no es coherente.
pub async fn crear_pregunta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    definicion: DefinicionPregunta,
) -> AppResult<PreguntaGuardada>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    exigir_preparacion(ctx)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = super::reunion_bloqueada(&mut tx, reunion).await?;
    exigir_no_terminada(&r)?;
    validar_punto(&mut tx, reunion, definicion.punto_id).await?;
    let orden = tx
        .preguntas(reunion)
        .await?
        .iter()
        .map(|p| p.orden)
        .max()
        .unwrap_or(0)
        + 1;
    let p = construir(&r, PreguntaId::nuevo(), orden, definicion)?;
    tx.guardar_pregunta(&p, true).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PREGUNTA,
        "pregunta",
        p.pregunta.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

fn exigir_no_terminada(r: &Reunion) -> AppResult<()> {
    if matches!(
        r.estado,
        domain::EstadoReunion::Cerrada | domain::EstadoReunion::Cancelada
    ) {
        Err(AppError::Conflicto("la reunión ya terminó".into()))
    } else {
        Ok(())
    }
}

async fn validar_punto<T: TxAsambleas>(
    tx: &mut T,
    reunion: ReunionId,
    punto: Option<PuntoOrdenId>,
) -> AppResult<()> {
    if let Some(p) = punto
        && !tx.puntos(reunion).await?.iter().any(|x| x.id == p)
    {
        return Err(AppError::NoEncontrado("punto del orden del día"));
    }
    Ok(())
}

async fn pregunta_de_reunion<T: TxAsambleas>(
    tx: &mut T,
    reunion: ReunionId,
    id: PreguntaId,
    bloquear: bool,
) -> AppResult<PreguntaGuardada> {
    let p = if bloquear {
        tx.bloquear_pregunta(id).await?
    } else {
        tx.pregunta(id).await?
    };
    // Una pregunta de otra reunión no existe para esta (asambleas simultáneas no se mezclan).
    p.filter(|p| p.pregunta.reunion_id == reunion)
        .ok_or(AppError::NoEncontrado("pregunta"))
}

/// Reemplaza la definición de una pregunta en borrador.
///
/// # Errors
///
/// Como [`crear_pregunta`], y [`AppError::Dominio`] si ya no está en borrador.
pub async fn editar_pregunta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    id: PreguntaId,
    definicion: DefinicionPregunta,
) -> AppResult<PreguntaGuardada>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    exigir_preparacion(ctx)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, reunion).await?;
    let actual = pregunta_de_reunion(&mut tx, reunion, id, true).await?;
    if actual.pregunta.estado != EstadoPregunta::Borrador {
        return Err(domain::DominioError::TransicionInvalida {
            entidad: "pregunta",
            desde: actual.pregunta.estado.as_str(),
            hacia: EstadoPregunta::Borrador.as_str(),
        }
        .into());
    }
    validar_punto(&mut tx, reunion, definicion.punto_id).await?;
    let p = construir(&r, id, actual.orden, definicion)?;
    tx.guardar_pregunta(&p, false).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PREGUNTA,
        "pregunta",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Elimina una pregunta en borrador.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si ya se
/// abrió (entonces se anula).
pub async fn eliminar_pregunta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    id: PreguntaId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    exigir_preparacion(ctx)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let p = pregunta_de_reunion(&mut tx, reunion, id, true).await?;
    if p.pregunta.estado != EstadoPregunta::Borrador {
        return Err(AppError::Conflicto(
            "una pregunta ya abierta se anula, no se elimina".into(),
        ));
    }
    tx.eliminar_pregunta(id).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PREGUNTA,
        "pregunta",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Preguntas de la reunión, en orden.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.ver`; [`AppError::NoEncontrado`].
pub async fn listar_preguntas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
) -> AppResult<Vec<PreguntaGuardada>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    reunion_de(&mut tx, reunion).await?;
    tx.preguntas(reunion).await
}

// ---------------------------------------------------------------------------------------------
// Abrir, cerrar, anular
// ---------------------------------------------------------------------------------------------

/// Abre la pregunta: la reunión debe estar instalada; quedan habilitados los bienes presentes
/// que pueden votar (los morosos sin voto no), se toma la foto del quórum y se envía la
/// pregunta por WhatsApp a cada asistente presente que confirmó ese canal.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.conducir`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si la reunión no está instalada o la pregunta no está en borrador.
pub async fn abrir_pregunta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    id: PreguntaId,
) -> AppResult<PreguntaGuardada>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConducir)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = super::reunion_bloqueada(&mut tx, reunion).await?;
    let p = abrir_en(&mut tx, reloj, ctx, &r, id, None).await?;
    tx.confirmar().await?;
    Ok(p)
}

pub(crate) async fn abrir_en<T: TxAsambleas>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    r: &Reunion,
    id: PreguntaId,
    cierra_en: Option<DateTime<Utc>>,
) -> AppResult<PreguntaGuardada> {
    r.estado.exigir_instalada()?;
    let mut p = pregunta_de_reunion(tx, r.id, id, true).await?;
    p.pregunta.validar_definicion()?;
    p.pregunta.estado = p.pregunta.estado.transicionar(EstadoPregunta::Abierta)?;
    let ahora = reloj.ahora();
    p.abierta_en = Some(ahora);
    p.cierra_en = cierra_en.or_else(|| {
        p.temporizador_segundos
            .map(|s| ahora + Duration::seconds(i64::from(s)))
    });
    tx.actualizar_pregunta(&p).await?;

    let presentes = tx.bienes_presentes(r.id).await?;
    let habilitados = pueden_votar(&presentes, r.configuracion.efecto_mora);
    tx.habilitar(
        id,
        &habilitados
            .iter()
            .map(|b| (b.bien_privado_id, b.coeficiente))
            .collect::<Vec<_>>(),
        ahora,
    )
    .await?;
    let q = calcular_quorum(tx, r).await?;
    tomar_foto(tx, r, Some(id), MomentoQuorum::Apertura, q, ahora).await?;

    let mut votantes: Vec<AsistenteId> = habilitados.iter().map(|b| b.asistente_id).collect();
    votantes.sort();
    votantes.dedup();
    let mut enviados = 0u32;
    for a in votantes {
        if enviar_pregunta(tx, ctx, r, &p, a).await? {
            enviados += 1;
        }
    }
    let detalle = DetalleAuditoria::vacio()
        .con("bienes_habilitados", habilitados.len())?
        .con("enviados_whatsapp", enviados)?;
    tx.registrar(EventoAuditoria::ok(ACCION_ABRIR, "pregunta", id.as_uuid()).con_detalle(detalle))
        .await?;
    tx.avisar_en_vivo(r.id).await?;
    Ok(p)
}

/// Envía la pregunta por WhatsApp al asistente si confirmó ese canal. Devuelve si la envió.
pub(crate) async fn enviar_pregunta<T: TxAsambleas>(
    tx: &mut T,
    ctx: &Contexto,
    r: &Reunion,
    p: &PreguntaGuardada,
    asistente: AsistenteId,
) -> AppResult<bool> {
    let Some(a) = tx.asistente(asistente).await? else {
        return Ok(false);
    };
    let (Some(tel), Some(linea), Some(_)) =
        (&a.telefono, &a.linea_whatsapp, a.whatsapp_confirmado_en)
    else {
        return Ok(false);
    };
    if !a.presente {
        return Ok(false);
    }
    let evento = EventoSaliente {
        prioridad: PrioridadSalida::Evento,
        mensaje: MensajeOutbox::WhatsApp(MensajeSaliente {
            linea: linea.clone(),
            telefono: tel.clone(),
            contenido: mensaje_pregunta(ctx.tenant_id, &r.titulo, p, &a),
        }),
    };
    tx.encolar_de_reunion(r.id, &evento).await?;
    Ok(true)
}

/// Habilita en las preguntas abiertas los bienes de un asistente que acaba de entrar o
/// reingresar (la pregunta sigue abierta: puede votarla) y se la envía por WhatsApp.
pub(crate) async fn habilitar_en_abiertas<T: TxAsambleas>(
    tx: &mut T,
    ctx: &Contexto,
    r: &Reunion,
    asistente: AsistenteId,
    ahora: DateTime<Utc>,
) -> AppResult<()> {
    if r.estado != domain::EstadoReunion::Instalada {
        return Ok(());
    }
    let presentes: Vec<BienPresente> = tx
        .bienes_presentes(r.id)
        .await?
        .into_iter()
        .filter(|b| b.asistente_id == asistente)
        .collect();
    let bienes = pueden_votar(&presentes, r.configuracion.efecto_mora)
        .iter()
        .map(|b| (b.bien_privado_id, b.coeficiente))
        .collect::<Vec<_>>();
    if bienes.is_empty() {
        return Ok(());
    }
    for p in tx.preguntas(r.id).await? {
        if p.pregunta.estado == EstadoPregunta::Abierta {
            tx.habilitar(p.pregunta.id, &bienes, ahora).await?;
            enviar_pregunta(tx, ctx, r, &p, asistente).await?;
        }
    }
    Ok(())
}

/// Cierra la pregunta y guarda su escrutinio.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.conducir`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si no está abierta.
pub async fn cerrar_pregunta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    id: PreguntaId,
) -> AppResult<PreguntaGuardada>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConducir)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, reunion).await?;
    let p = cerrar_en(&mut tx, reloj, ctx, &r, id).await?;
    tx.confirmar().await?;
    Ok(p)
}

pub(crate) async fn cerrar_en<T: TxAsambleas>(
    tx: &mut T,
    reloj: &impl Reloj,
    _ctx: &Contexto,
    r: &Reunion,
    id: PreguntaId,
) -> AppResult<PreguntaGuardada> {
    let mut p = pregunta_de_reunion(tx, r.id, id, true).await?;
    p.pregunta.estado = p.pregunta.estado.transicionar(EstadoPregunta::Cerrada)?;
    let ahora = reloj.ahora();
    p.cerrada_en = Some(ahora);
    let escrutinio = escrutinio_de(tx, &p).await?;
    p.resultado = Some(escrutinio);
    tx.actualizar_pregunta(&p).await?;
    if let Ok(q) = calcular_quorum(tx, r).await {
        tomar_foto(tx, r, Some(id), MomentoQuorum::Cierre, q, ahora).await?;
    }
    tx.registrar(EventoAuditoria::ok(ACCION_CERRAR, "pregunta", id.as_uuid()))
        .await?;
    tx.avisar_en_vivo(r.id).await?;
    Ok(p)
}

/// Anula la pregunta (no produce decisión; sus votos se conservan para el acta).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si ya cerró.
pub async fn anular_pregunta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    id: PreguntaId,
    motivo: &str,
) -> AppResult<PreguntaGuardada>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConducir)?;
    let motivo = texto("motivo", motivo, 300)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut p = pregunta_de_reunion(&mut tx, reunion, id, true).await?;
    p.pregunta.estado = p.pregunta.estado.transicionar(EstadoPregunta::Anulada)?;
    p.cerrada_en = Some(reloj.ahora());
    tx.actualizar_pregunta(&p).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ANULAR, "pregunta", id.as_uuid())
            .con_detalle(DetalleAuditoria::vacio().con("motivo", motivo)?),
    )
    .await?;
    tx.avisar_en_vivo(reunion).await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Cierra las preguntas con el temporizador vencido y las consultas por escrito con el plazo
/// cumplido (worker del gateway). Devuelve cuántas preguntas cerró.
///
/// # Errors
///
/// Errores de la base de datos.
pub async fn cerrar_vencidas<U>(unidad: &U, reloj: &impl Reloj, ctx: &Contexto) -> AppResult<u32>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let ahora = reloj.ahora();
    let mut cerradas = 0;
    for id in tx.preguntas_vencidas(ahora).await? {
        let Some(p) = tx.pregunta(id).await? else {
            continue;
        };
        let r = reunion_de(&mut tx, p.pregunta.reunion_id).await?;
        cerrar_en(&mut tx, reloj, ctx, &r, id).await?;
        cerradas += 1;
    }
    for reunion in tx.consultas_vencidas(ahora).await? {
        super::reuniones::cerrar_reunion_en(&mut tx, reloj, ctx, reunion).await?;
    }
    tx.confirmar().await?;
    Ok(cerradas)
}

// ---------------------------------------------------------------------------------------------
// Emitir el voto
// ---------------------------------------------------------------------------------------------

/// Quién vota.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Votante {
    /// Ya identificado (papeleta virtual, WhatsApp, operador que lo buscó).
    Asistente(AsistenteId),
    /// Rótulo escaneado en un puesto.
    Rotulo(CodigoRotulo),
}

/// Petición de voto. La identidad la resolvió el adaptador de entrada: el token de la
/// papeleta (web), el teléfono (WhatsApp) o la sesión del operador (presencial y rótulo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolicitudVoto {
    pub reunion_id: ReunionId,
    pub pregunta_id: PreguntaId,
    pub votante: Votante,
    pub opciones: Vec<OpcionId>,
    pub canal: Canal,
    pub idempotency_key: ClaveIdempotencia,
    pub wamid: Option<String>,
    /// Enviar la confirmación por WhatsApp (sin la opción elegida).
    pub confirmar: bool,
}

/// Resultado de una emisión.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultadoVoto {
    /// `false` = reintento de una emisión ya registrada (no hay filas nuevas).
    pub registrado: bool,
    pub asistente_id: AsistenteId,
    pub nombre: String,
    /// Códigos de los bienes por los que votó (en un reintento, vacío).
    pub bienes: Vec<String>,
    /// Bienes del asistente que no votaron esta vez (ya habían votado o no estaban
    /// habilitados, p. ej. por mora).
    pub omitidos: Vec<String>,
}

/// Emite el voto del asistente por **todos** sus bienes habilitados que aún no votaron, en una
/// sola transacción (participaciones + votos + confirmación en el outbox + auditoría).
///
/// Los canales presencial y rótulo exigen `asambleas.votos.registrar` (queda quién lo
/// registró).
///
/// # Errors
///
/// - [`AppError::NoEncontrado`] si la pregunta no es de la reunión.
/// - [`AppError::Dominio`] si la reunión no está instalada, la pregunta no está abierta o la
///   papeleta no corresponde a la pregunta.
/// - [`AppError::NoInscrito`] si el asistente no es de la reunión o no está presente.
/// - [`AppError::BienPrivadoYaVoto`] si sus bienes ya votaron con otra clave.
/// - [`AppError::Conflicto`] si ninguno de sus bienes está habilitado (p. ej. mora).
pub async fn emitir_voto<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    s: SolicitudVoto,
) -> AppResult<ResultadoVoto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    if matches!(s.canal, Canal::Presencial | Canal::Rotulo) {
        ctx.exigir(Permiso::AsambleasVotosRegistrar)?;
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let resultado = emitir_en(&mut tx, reloj, ctx, s).await?;
    tx.confirmar().await?;
    Ok(resultado)
}

pub(crate) async fn emitir_en<T: TxAsambleas>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    s: SolicitudVoto,
) -> AppResult<ResultadoVoto> {
    let r = reunion_de(tx, s.reunion_id).await?;
    r.estado.exigir_instalada()?;
    let p = pregunta_de_reunion(tx, r.id, s.pregunta_id, false).await?;
    let abierta = p.pregunta.para_votar()?;
    let asistente = match &s.votante {
        Votante::Asistente(id) => tx.asistente(*id).await?,
        Votante::Rotulo(c) => tx.asistente_por_rotulo(r.id, c).await?,
    }
    .filter(|a| a.reunion_id == r.id)
    .ok_or(AppError::NoInscrito)?;
    if !asistente.presente {
        return Err(AppError::NoInscrito);
    }
    let base = ResultadoVoto {
        registrado: false,
        asistente_id: asistente.id,
        nombre: asistente.nombre.clone(),
        bienes: Vec::new(),
        omitidos: Vec::new(),
    };
    let clave = s.idempotency_key.as_str().to_owned();
    if tx
        .participaciones_con_clave(p.pregunta.id, asistente.id, &clave)
        .await?
        > 0
    {
        return Ok(base);
    }
    let ids: Vec<_> = asistente.bienes.iter().map(|b| b.bien_privado_id).collect();
    let pendientes = tx.pendientes_de_voto(p.pregunta.id, &ids).await?;
    if pendientes.is_empty() {
        return Err(if tx.asistente_voto(p.pregunta.id, asistente.id).await? {
            AppError::BienPrivadoYaVoto
        } else {
            AppError::Conflicto(
                "sus bienes privados no están habilitados para votar esta pregunta".into(),
            )
        });
    }
    let bienes: Vec<BienRepresentado> = pendientes
        .iter()
        .filter_map(|(id, coef)| {
            asistente
                .bienes
                .iter()
                .find(|b| b.bien_privado_id == *id)
                .map(|b| BienRepresentado {
                    bien_privado_id: *id,
                    coeficiente: *coef,
                    calidad: b.calidad,
                })
        })
        .collect();
    let papeleta = Papeleta {
        asistente_id: asistente.id,
        persona_id: asistente.persona_id,
        bienes,
        opciones: s.opciones.clone(),
        canal: s.canal,
        registrado_por: matches!(s.canal, Canal::Presencial | Canal::Rotulo)
            .then_some(ctx.actor_id)
            .flatten(),
        idempotency_key: s.idempotency_key.clone(),
        wamid: s.wamid.clone(),
    };
    let emitido = abierta.emitir(papeleta, reloj.ahora())?;
    let insertadas = tx.registrar_voto(&emitido).await?;
    if insertadas == 0 {
        // Otra emisión concurrente del mismo asistente ganó (doble escaneo en dos puestos).
        if tx
            .participaciones_con_clave(p.pregunta.id, asistente.id, &clave)
            .await?
            > 0
        {
            return Ok(base);
        }
        return Err(AppError::BienPrivadoYaVoto);
    }
    let codigo = |id| {
        asistente
            .bienes
            .iter()
            .find(|b| b.bien_privado_id == id)
            .map(|b| b.codigo.clone())
            .unwrap_or_default()
    };
    let votados: Vec<_> = pendientes.iter().map(|(id, _)| *id).collect();
    let resultado = ResultadoVoto {
        registrado: true,
        bienes: votados.iter().map(|id| codigo(*id)).collect(),
        omitidos: ids
            .iter()
            .filter(|id| !votados.contains(id))
            .map(|id| codigo(*id))
            .collect(),
        ..base
    };
    // Sin la opción elegida (invariante 4).
    let detalle = DetalleAuditoria::vacio()
        .con("canal", s.canal.as_str())?
        .con("bienes", insertadas)?
        .con("asistente_id", asistente.id.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_VOTAR, "pregunta", p.pregunta.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    if s.confirmar
        && let (Some(tel), Some(linea)) = (&asistente.telefono, &asistente.linea_whatsapp)
    {
        let evento = EventoSaliente {
            prioridad: PrioridadSalida::Confirmacion,
            mensaje: MensajeOutbox::WhatsApp(MensajeSaliente {
                linea: linea.clone(),
                telefono: tel.clone(),
                contenido: ContenidoSaliente::Texto {
                    cuerpo: confirmacion_voto(&p.pregunta.enunciado, &resultado),
                },
            }),
        };
        tx.encolar_de_reunion(r.id, &evento).await?;
    }
    tracing::info!(
        tenant_id = %ctx.tenant_id,
        reunion_id = %r.id,
        pregunta_id = %p.pregunta.id,
        canal = %s.canal,
        bienes = insertadas,
        "voto registrado"
    );
    tx.avisar_en_vivo(r.id).await?;
    Ok(resultado)
}

// ---------------------------------------------------------------------------------------------
// Resultados, detalle y roles
// ---------------------------------------------------------------------------------------------

/// Escrutinio de la pregunta desde la BD (en vivo o al cerrar).
pub(crate) async fn escrutinio_de<T: TxAsambleas>(
    tx: &mut T,
    p: &PreguntaGuardada,
) -> AppResult<Escrutinio> {
    let padron = tx.padron(p.pregunta.reunion_id).await?;
    let total = Conteo::new(
        padron.iter().map(|b| b.coeficiente.valor()).sum(),
        u32::try_from(padron.len()).unwrap_or(u32::MAX),
    );
    let habilitados = tx.conteo_habilitados(p.pregunta.id).await?;
    let conteo = tx.conteo_votos(p.pregunta.id).await?;
    let votos: Vec<VotosOpcion> = conteo.por_opcion;
    Ok(Escrutinio::calcular(
        &p.pregunta,
        habilitados,
        total,
        conteo.votantes,
        &votos,
        |o| {
            let c = p.candidatos_de(o);
            (!c.is_empty()).then(|| u16::try_from(c.len()).unwrap_or(u16::MAX))
        },
    ))
}

/// Escrutinio de una pregunta (guardado si ya cerró; en vivo si está abierta). Los totales no
/// revelan qué votó cada bien, así que se publican igual en votaciones reservadas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.ver`; [`AppError::NoEncontrado`].
pub async fn resultados<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    id: PreguntaId,
) -> AppResult<(PreguntaGuardada, Escrutinio)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let p = pregunta_de_reunion(&mut tx, reunion, id, false).await?;
    let e = match &p.resultado {
        Some(e) => e.clone(),
        None => escrutinio_de(&mut tx, &p).await?,
    };
    Ok((p, e))
}

/// Detalle bien privado → opción. En votaciones nominales lo ve quien ve la asamblea; en
/// reservadas exige `asambleas.votos.ver_detalle` y **cada consulta queda auditada**.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`].
pub async fn detalle_votos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    id: PreguntaId,
) -> AppResult<Vec<DetalleVoto>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let p = pregunta_de_reunion(&mut tx, reunion, id, false).await?;
    if p.pregunta.tipo_voto == TipoVoto::Reservado {
        ctx.exigir(Permiso::AsambleasVotosVerDetalle)?;
    }
    let detalle = tx.detalle_votos(id).await?;
    if p.pregunta.tipo_voto == TipoVoto::Reservado {
        tx.registrar(
            EventoAuditoria::ok(ACCION_VER_DETALLE, "pregunta", id.as_uuid())
                .con_detalle(DetalleAuditoria::vacio().con("filas", detalle.len())?),
        )
        .await?;
        tx.confirmar().await?;
    }
    Ok(detalle)
}

/// Elegido de una elección.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elegido {
    pub persona_id: PersonaId,
    pub nombre: String,
    /// `false` si la persona no tiene cuenta en la plataforma (no se le puede asignar el rol:
    /// se asigna a mano cuando la cree).
    pub rol_asignado: bool,
}

/// Elegidos de una elección cerrada: por cada plancha, sus primeros candidatos hasta los cupos
/// que ganó.
pub(crate) fn elegidos_de(p: &PreguntaGuardada) -> Vec<PersonaId> {
    let Some(Escrutinio {
        decision: Decision::Reparto(reparto),
        ..
    }) = &p.resultado
    else {
        return Vec::new();
    };
    reparto
        .cupos
        .iter()
        .flat_map(|c| {
            p.candidatos_de(c.opcion_id)
                .iter()
                .take(usize::from(c.total()))
                .copied()
        })
        .collect()
}

/// Aplica una elección cerrada de consejo, revisor fiscal o comité de convivencia: termina las
/// asignaciones de ese rol que vinieron de una asamblea anterior y crea las de los elegidos
/// (origen `asamblea`) con la vigencia indicada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.conducir`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si la pregunta no es una elección cerrada de un cargo con rol o ya
/// se aplicó; [`AppError::Validacion`] si la vigencia termina antes de empezar.
pub async fn aplicar_eleccion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    id: PreguntaId,
    vigente_hasta: Option<DateTime<Utc>>,
) -> AppResult<Vec<Elegido>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConducir)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut p = pregunta_de_reunion(&mut tx, reunion, id, true).await?;
    let rol_codigo = p
        .pregunta
        .cargo
        .and_then(CargoEleccion::rol_plantilla)
        .filter(|_| p.pregunta.estado == EstadoPregunta::Cerrada)
        .ok_or_else(|| {
            AppError::Conflicto(
                "solo una elección cerrada de consejo, revisor fiscal o comité crea roles".into(),
            )
        })?;
    if p.roles_aplicados_en.is_some() {
        return Err(AppError::Conflicto("la elección ya se aplicó".into()));
    }
    let ahora = reloj.ahora();
    if vigente_hasta.is_some_and(|h| h <= ahora) {
        return Err(AppError::Validacion(
            "la vigencia termina antes de empezar".into(),
        ));
    }
    let rol = tx
        .rol_por_codigo(rol_codigo)
        .await?
        .ok_or(AppError::NoEncontrado("rol"))?;
    // El periodo anterior termina: las asignaciones de ese rol que vinieron de una asamblea.
    for a in tx.asignaciones(None).await? {
        if a.rol_id == rol.id && a.origen == OrigenAsignacion::Asamblea && a.vigente_en(ahora) {
            tx.terminar_asignacion(a.id, ahora).await?;
        }
    }
    let mut elegidos = Vec::new();
    for persona_id in elegidos_de(&p) {
        let Some(persona) = tx.persona(persona_id).await? else {
            continue;
        };
        let rol_asignado = if let Some(cuenta) = persona.cuenta_id {
            tx.insertar_asignacion(&Asignacion {
                id: AsignacionRolId::nuevo(),
                cuenta_id: cuenta,
                rol_id: rol.id,
                vigente_desde: ahora,
                vigente_hasta,
                origen: OrigenAsignacion::Asamblea,
                origen_id: Some(id.as_uuid()),
                asignado_por: ctx.actor_id,
            })
            .await?;
            true
        } else {
            false
        };
        elegidos.push(Elegido {
            persona_id,
            nombre: persona.nombre,
            rol_asignado,
        });
    }
    p.roles_aplicados_en = Some(ahora);
    tx.actualizar_pregunta(&p).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("rol", rol_codigo)?
        .con("elegidos", elegidos.len())?
        .con(
            "sin_cuenta",
            elegidos.iter().filter(|e| !e.rol_asignado).count(),
        )?;
    tx.registrar(EventoAuditoria::ok(ACCION_ROLES, "pregunta", id.as_uuid()).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    Ok(elegidos)
}

/// Copia las preguntas de una reunión a otra en borrador (segunda convocatoria).
pub(crate) async fn copiar_preguntas<T: TxAsambleas>(
    tx: &mut T,
    origen: ReunionId,
    destino: ReunionId,
) -> AppResult<()> {
    for p in tx.preguntas(origen).await? {
        let mapa: Vec<(OpcionId, OpcionId)> = p
            .pregunta
            .opciones
            .iter()
            .map(|o| (o.id, OpcionId::nuevo()))
            .collect();
        let nueva = PreguntaGuardada {
            pregunta: Pregunta {
                id: PreguntaId::nuevo(),
                reunion_id: destino,
                estado: EstadoPregunta::Borrador,
                opciones: p
                    .pregunta
                    .opciones
                    .iter()
                    .zip(&mapa)
                    .map(|(o, (_, nuevo))| OpcionPregunta {
                        id: *nuevo,
                        ..o.clone()
                    })
                    .collect(),
                ..p.pregunta.clone()
            },
            // Los puntos de la reunión nueva tienen otros ids.
            punto_id: None,
            cierra_en: None,
            abierta_en: None,
            cerrada_en: None,
            resultado: None,
            roles_aplicados_en: None,
            candidatos: p
                .candidatos
                .iter()
                .filter_map(|(o, c)| {
                    mapa.iter()
                        .find(|(viejo, _)| viejo == o)
                        .map(|(_, nuevo)| (*nuevo, c.clone()))
                })
                .collect(),
            ..p.clone()
        };
        tx.guardar_pregunta(&nueva, true).await?;
    }
    Ok(())
}

/// Al cerrar la reunión: un solo mensaje por asistente con WhatsApp con el resultado de cada
/// pregunta (ahorra mensajes de servicio).
pub(crate) async fn resumen_final<T: TxAsambleas>(tx: &mut T, r: &Reunion) -> AppResult<()> {
    let preguntas = tx.preguntas(r.id).await?;
    let lineas: Vec<String> = preguntas
        .iter()
        .filter(|p| p.pregunta.estado == EstadoPregunta::Cerrada)
        .map(|p| {
            format!(
                "{}. {}: {}",
                p.orden,
                super::corto(&p.pregunta.enunciado, 80),
                texto_decision(p)
            )
        })
        .collect();
    if lineas.is_empty() {
        return Ok(());
    }
    let cuerpo = format!(
        "La reunión «{}» terminó. Resultados:\n{}\n\nGracias por participar.",
        r.titulo,
        lineas.join("\n")
    );
    let asistentes: Vec<Asistente> = tx.asistentes(r.id).await?;
    for a in asistentes {
        if let (Some(tel), Some(linea), Some(_)) =
            (&a.telefono, &a.linea_whatsapp, a.whatsapp_confirmado_en)
        {
            let evento = EventoSaliente {
                prioridad: PrioridadSalida::Evento,
                mensaje: MensajeOutbox::WhatsApp(MensajeSaliente {
                    linea: linea.clone(),
                    telefono: tel.clone(),
                    contenido: ContenidoSaliente::Texto {
                        cuerpo: cuerpo.clone(),
                    },
                }),
            };
            tx.encolar_de_reunion(r.id, &evento).await?;
        }
    }
    Ok(())
}

/// Decisión legible de una pregunta cerrada.
pub(crate) fn texto_decision(p: &PreguntaGuardada) -> String {
    let Some(e) = &p.resultado else {
        return "sin escrutinio".into();
    };
    let texto_de = |id: OpcionId| {
        p.pregunta
            .opcion(id)
            .map(|o| o.texto.clone())
            .unwrap_or_default()
    };
    let base = if e.advertencia_unidades {
        " (por unidades)"
    } else {
        ""
    };
    match &e.decision {
        Decision::Aprobada => format!("aprobada{base}"),
        Decision::NoAprobada => format!("no aprobada{base}"),
        Decision::Ganadora { opcion_id } => format!("ganó «{}»{base}", texto_de(*opcion_id)),
        Decision::SinGanadora { empate: true } => "empate".into(),
        Decision::SinGanadora { empate: false } => "ninguna opción alcanzó la mayoría".into(),
        Decision::PorOpcion => {
            let aprobadas: Vec<_> = e
                .opciones
                .iter()
                .filter(|o| o.aprobada == Some(true))
                .map(|o| o.texto.clone())
                .collect();
            if aprobadas.is_empty() {
                "ninguna opción aprobada".into()
            } else {
                format!("aprobadas: {}", aprobadas.join(", "))
            }
        }
        Decision::Reparto(r) => {
            let cupos: Vec<_> = r
                .cupos
                .iter()
                .filter(|c| c.total() > 0)
                .map(|c| format!("{} {}", texto_de(c.opcion_id), c.total()))
                .collect();
            let mut t = format!("cupos: {}", cupos.join(", "));
            if r.empate {
                t.push_str(" (empate en el último cupo)");
            }
            t
        }
    }
}
