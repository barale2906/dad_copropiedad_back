//! Registro de asistentes: mesa de registro asistido (la mesa hace el trabajo y el asistente
//! solo toca «Confirmar» en WhatsApp), poderes, rótulos, salidas con motivo y reingresos,
//! morosos, prerregistro y la papeleta virtual (enlace personal).
//!
//! Reglas (plan 06, decisiones del 2026-11-08):
//! - La asistencia la certifica la mesa al firmar: cuenta para el quórum **de inmediato**; el
//!   «Confirmar» de WhatsApp solo habilita votar por ese canal.
//! - Un bien está en un solo asistente a la vez; un asistente vota por todos sus bienes.
//! - Los poderes respetan la configuración de la asamblea (máximo de bienes por apoderado,
//!   cargos que no pueden ser apoderados, soporte obligatorio).
//! - Una salida saca sus bienes del quórum y de las preguntas que se abran después; un
//!   reingreso abre un nuevo periodo de presencia.

use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, AsistenteId, BienPrivadoId, CalidadRepresentacion, CodigoRotulo, Documento,
    EstadoPoder, EstadoReunion, MetodoRegistro, Modalidad, MotivoSalida, Permiso, PersonaId,
    PoderId, PresenciaId, ReunionId, Telefono, TipoArchivo, TipoPersona,
};
use sha2::{Digest, Sha256};

use super::{
    ACCION_MOROSOS, ACCION_PODER, ACCION_REGISTRAR, ACCION_REINGRESO, ACCION_REVISAR_PODER,
    ACCION_ROTULO, ACCION_SALIDA, TxAsambleas, abrir, reunion_de, texto, texto_opcional,
    votaciones::{
        ResultadoVoto, SolicitudVoto, Votante, emitir_en, enviar_pregunta, habilitar_en_abiertas,
    },
    whatsapp::payload_confirmar,
};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{DetalleAuditoria, EventoAuditoria},
    casos_uso::avisos::IDIOMA_PLANTILLAS,
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        LectorPlanillas, RegistroAuditoria, Reloj, RepositorioAsambleas, RepositorioPersonas,
        Transaccion, UnidadDeTrabajo,
        asambleas::{Asistente, AsistenteNuevo, CoincidenciaMesa, Poder, Presencia, Reunion},
        copropiedades::Persona,
        mensajeria::{ContenidoSaliente, MensajeSaliente},
    },
    seguridad::token_aleatorio,
};

/// Plantilla de utilidad «confirmar_asistencia». Parámetros: nombre, copropiedad, número de
/// bienes y códigos. Botón de respuesta rápida «Confirmar» con el asistente en el `payload`.
pub const PLANTILLA_CONFIRMAR_ASISTENCIA: &str = "confirmar_asistencia";

/// SHA-256 del token de la papeleta (lo único que se guarda).
pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.trim().as_bytes()).to_vec()
}

/// Normaliza un celular colombiano de 10 dígitos agregando el indicativo 57.
pub fn normalizar_telefono(valor: &str) -> AppResult<Telefono> {
    let digitos: String = valor.chars().filter(char::is_ascii_digit).collect();
    let con_pais = if digitos.len() == 10 && digitos.starts_with('3') {
        format!("57{digitos}")
    } else {
        valor.to_owned()
    };
    Ok(Telefono::new(&con_pais)?)
}

/// Busca en el padrón por código del bien («506», «T3-506») o documento del representante.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.registrar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si el texto es muy corto.
pub async fn buscar_en_mesa<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    texto_busqueda: &str,
) -> AppResult<Vec<CoincidenciaMesa>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasRegistrar)?;
    let t = texto_busqueda.trim();
    if t.chars().count() < 2 {
        return Err(AppError::Validacion("escriba al menos 2 caracteres".into()));
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    reunion_de(&mut tx, reunion).await?;
    tx.buscar_en_padron(reunion, t, 20).await
}

/// Persona que se registra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonaAsistente {
    Existente(PersonaId),
    /// Apoderado u otra persona que no está en la base de la copropiedad.
    Nueva {
        nombre: String,
        documento: Option<Documento>,
    },
}

/// Poder que se recibe en la mesa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoderRecibido {
    pub bien_privado_id: BienPrivadoId,
    /// Foto o PDF del poder (puede quedar pendiente de verificar sin frenar la fila).
    pub soporte_archivo_id: Option<ArchivoId>,
}

/// Registro de un asistente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistroAsistente {
    pub reunion_id: ReunionId,
    pub persona: PersonaAsistente,
    /// Celular (10 dígitos o con indicativo). `None` = el del censo, si existe.
    pub telefono: Option<String>,
    /// Bienes de los que la persona es representante.
    pub bienes_propios: Vec<BienPrivadoId>,
    pub poderes: Vec<PoderRecibido>,
    pub rotulo: Option<CodigoRotulo>,
    /// Enviar la plantilla `confirmar_asistencia` por WhatsApp.
    pub enviar_confirmacion: bool,
    /// `Mesa` (el día de la reunión, cuenta para el quórum) o `Prerregistro`.
    pub metodo: MetodoRegistro,
}

/// Resultado del registro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultadoRegistro {
    pub asistente: Asistente,
    /// Token de la papeleta virtual: se entrega **una sola vez** (QR en la mesa o enlace).
    pub token_papeleta: Option<String>,
    /// Avisos para la mesa (bienes en mora, poderes pendientes…).
    pub avisos: Vec<String>,
}

/// Registra (o completa) un asistente con sus bienes propios y poderes, y lo marca presente si
/// es la mesa del día. Todo en una transacción.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `asambleas.registrar`.
/// - [`AppError::NoEncontrado`] si la reunión o la persona no existen.
/// - [`AppError::Conflicto`] si un bien no lo representa la persona, ya está en otro
///   asistente presente, el apoderado tiene un cargo prohibido o supera el máximo de poderes.
/// - [`AppError::Validacion`] si falta el soporte obligatorio, no hay bienes o el teléfono es
///   inválido.
pub async fn registrar_asistente<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    registro: RegistroAsistente,
) -> AppResult<ResultadoRegistro>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasRegistrar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, registro.reunion_id).await?;
    match registro.metodo {
        MetodoRegistro::Mesa if r.estado.admite_registro() => {}
        MetodoRegistro::Prerregistro
            if r.estado == EstadoReunion::Convocada || r.estado.admite_registro() => {}
        MetodoRegistro::Mesa | MetodoRegistro::Prerregistro => {
            return Err(AppError::Conflicto(format!(
                "la reunión está {}: no admite este registro",
                r.estado.as_str().replace('_', " ")
            )));
        }
        _ => {
            return Err(AppError::Validacion(
                "método de registro no admitido".into(),
            ));
        }
    }
    if r.modalidad == Modalidad::Escrita {
        return Err(AppError::Conflicto(
            "la consulta por escrito no tiene registro".into(),
        ));
    }
    if registro.bienes_propios.is_empty() && registro.poderes.is_empty() {
        return Err(AppError::Validacion(
            "el asistente debe representar al menos un bien privado".into(),
        ));
    }
    let ahora = reloj.ahora();
    let persona = persona_de(&mut tx, &registro.persona).await?;
    let telefono = match registro
        .telefono
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        Some(t) => Some(normalizar_telefono(t)?),
        None => persona.telefono.clone(),
    };

    let (asistente_id, nuevo) = match tx.asistente_de_persona(r.id, persona.id).await? {
        Some(a) => (a.id, false),
        None => {
            let a = AsistenteNuevo {
                id: AsistenteId::nuevo(),
                reunion_id: r.id,
                persona_id: persona.id,
                metodo: registro.metodo,
                telefono: telefono.as_ref().map(|t| t.as_str().to_owned()),
                registrado_por: ctx.actor_id,
                creado_en: ahora,
            };
            tx.insertar_asistente(&a).await?;
            (a.id, true)
        }
    };

    let padron = tx.padron(r.id).await?;
    let mut avisos = Vec::new();
    for bien in &registro.bienes_propios {
        let b = padron
            .iter()
            .find(|b| b.bien_privado_id == *bien)
            .ok_or(AppError::NoEncontrado("bien privado del padrón"))?;
        if b.representante != Some(persona.id) {
            return Err(AppError::Conflicto(format!(
                "{} no lo representa esta persona: registre un poder",
                b.codigo
            )));
        }
        liberar_si_ausente(&mut tx, &r, *bien, asistente_id, ahora).await?;
        tx.vincular_bien(
            r.id,
            asistente_id,
            *bien,
            CalidadRepresentacion::Propietario,
            None,
            ahora,
        )
        .await
        .map_err(|e| ocupado(e, &b.codigo))?;
        if b.moroso {
            avisos.push(format!(
                "{} está en el listado de morosos ({})",
                b.codigo, r.configuracion.efecto_mora
            ));
        }
    }
    for poder in &registro.poderes {
        let aviso = agregar_poder(&mut tx, ctx, &r, asistente_id, &persona, poder, ahora).await?;
        avisos.extend(aviso);
    }

    if registro.metodo == MetodoRegistro::Mesa
        && !tx
            .asistente(asistente_id)
            .await?
            .is_some_and(|a| a.presente)
    {
        tx.abrir_presencia(r.id, &presencia(asistente_id, ctx, ahora))
            .await?;
    }
    if let Some(codigo) = &registro.rotulo {
        tx.liberar_rotulo(asistente_id, ahora).await?;
        tx.asignar_rotulo(r.id, codigo, asistente_id, ahora)
            .await
            .map_err(|e| ocupado(e, codigo.as_str()))?;
    }
    let token = if nuevo {
        let t = token_aleatorio()?;
        tx.fijar_token(asistente_id, &hash_token(&t)).await?;
        Some(t)
    } else {
        None
    };
    if let Some(tel) = &telefono {
        tx.actualizar_medios(asistente_id, Some(tel.as_str()), None, None, None)
            .await?;
    }
    if registro.enviar_confirmacion && telefono.is_some() {
        enviar_confirmacion(&mut tx, ctx, &r, asistente_id).await?;
    }
    habilitar_en_abiertas(&mut tx, ctx, &r, asistente_id, ahora).await?;

    let detalle = DetalleAuditoria::vacio()
        .con("metodo", registro.metodo.as_str())?
        .con("bienes_propios", registro.bienes_propios.len())?
        .con("poderes", registro.poderes.len())?
        .con("con_rotulo", registro.rotulo.is_some())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_REGISTRAR, "asistente", asistente_id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.avisar_en_vivo(r.id).await?;
    let asistente = tx
        .asistente(asistente_id)
        .await?
        .ok_or(AppError::NoEncontrado("asistente"))?;
    tx.confirmar().await?;
    Ok(ResultadoRegistro {
        asistente,
        token_papeleta: token,
        avisos,
    })
}

fn presencia(asistente: AsistenteId, ctx: &Contexto, ahora: DateTime<Utc>) -> Presencia {
    Presencia {
        id: PresenciaId::nuevo(),
        asistente_id: asistente,
        entrada_en: ahora,
        salida_en: None,
        motivo_salida: None,
        detalle_salida: None,
        registrada_por: ctx.actor_id,
    }
}

/// Un bien ocupado por otro asistente se reporta con su código (no con un error genérico).
fn ocupado(e: AppError, codigo: &str) -> AppError {
    match e {
        AppError::Conflicto(_) => {
            AppError::Conflicto(format!("{codigo} ya está registrado con otro asistente"))
        }
        otro => otro,
    }
}

/// Si el bien está en un asistente **ausente** (se retiró), se libera para que lo represente
/// quien llega (p. ej. delegó el poder al salir).
async fn liberar_si_ausente<T: TxAsambleas>(
    tx: &mut T,
    r: &Reunion,
    bien: BienPrivadoId,
    nuevo: AsistenteId,
    ahora: DateTime<Utc>,
) -> AppResult<()> {
    let actual = tx
        .asistentes(r.id)
        .await?
        .into_iter()
        .find(|a| a.bienes.iter().any(|b| b.bien_privado_id == bien));
    if let Some(a) = actual
        && a.id != nuevo
        && !a.presente
    {
        tx.desvincular_bien(r.id, bien, ahora).await?;
    }
    Ok(())
}

async fn persona_de<T: TxAsambleas>(tx: &mut T, p: &PersonaAsistente) -> AppResult<Persona> {
    match p {
        PersonaAsistente::Existente(id) => tx
            .persona(*id)
            .await?
            .ok_or(AppError::NoEncontrado("persona")),
        PersonaAsistente::Nueva { nombre, documento } => {
            if let Some(doc) = documento
                && let Some(existente) = tx.persona_por_documento(doc).await?
            {
                return Ok(existente);
            }
            let persona = Persona::basica(
                PersonaId::nuevo(),
                TipoPersona::Natural,
                texto("nombre", nombre, 200)?,
                documento.clone(),
            );
            tx.insertar_persona(&persona).await?;
            Ok(persona)
        }
    }
}

async fn agregar_poder<T: TxAsambleas>(
    tx: &mut T,
    ctx: &Contexto,
    r: &Reunion,
    asistente: AsistenteId,
    apoderado: &Persona,
    poder: &PoderRecibido,
    ahora: DateTime<Utc>,
) -> AppResult<Option<String>> {
    let config = &r.configuracion;
    let padron = tx.padron(r.id).await?;
    let b = padron
        .iter()
        .find(|b| b.bien_privado_id == poder.bien_privado_id)
        .ok_or(AppError::NoEncontrado("bien privado del padrón"))?;
    if b.representante == Some(apoderado.id) {
        return Err(AppError::Conflicto(format!(
            "{} ya lo representa esta persona como propietario",
            b.codigo
        )));
    }
    if config.exigir_soporte_poder && poder.soporte_archivo_id.is_none() {
        return Err(AppError::Validacion(format!(
            "esta asamblea exige el soporte del poder de {}",
            b.codigo
        )));
    }
    if let Some(cuenta) = apoderado.cuenta_id
        && let Some(cargo) = tx
            .cargos_vigentes()
            .await?
            .into_iter()
            .find(|c| c.cuenta_id == cuenta && config.apoderados_prohibidos.contains(&c.rol))
    {
        return Err(AppError::Conflicto(format!(
            "quien ejerce el cargo «{}» no puede ser apoderado en esta asamblea",
            cargo.rol_nombre
        )));
    }
    if let Some(maximo) = config.max_bienes_apoderado {
        let actuales = tx.asistente(asistente).await?.map_or(0, |a| {
            a.bienes
                .iter()
                .filter(|x| x.calidad == CalidadRepresentacion::Apoderado)
                .count()
        });
        if actuales + 1 > usize::from(maximo) {
            return Err(AppError::Conflicto(format!(
                "un apoderado representa como máximo {maximo} bien(es) en esta asamblea"
            )));
        }
    }
    liberar_si_ausente(tx, r, poder.bien_privado_id, asistente, ahora).await?;
    let p = Poder {
        id: PoderId::nuevo(),
        reunion_id: r.id,
        bien_privado_id: poder.bien_privado_id,
        poderdante: b.representante,
        apoderado: apoderado.id,
        soporte_archivo_id: poder.soporte_archivo_id,
        estado: EstadoPoder::Pendiente,
        motivo: None,
        registrado_por: ctx.actor_id,
        registrado_en: ahora,
        revisado_por: None,
        revisado_en: None,
    };
    tx.insertar_poder(&p)
        .await
        .map_err(|e| ocupado(e, &b.codigo))?;
    tx.vincular_bien(
        r.id,
        asistente,
        poder.bien_privado_id,
        CalidadRepresentacion::Apoderado,
        Some(p.id),
        ahora,
    )
    .await
    .map_err(|e| ocupado(e, &b.codigo))?;
    tx.registrar(EventoAuditoria::ok(ACCION_PODER, "poder", p.id.as_uuid()))
        .await?;
    Ok(b.moroso
        .then(|| format!("{} está en el listado de morosos", b.codigo)))
}

/// Agrega un poder a un asistente ya registrado.
///
/// # Errors
///
/// Como [`registrar_asistente`].
pub async fn registrar_poder<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    asistente: AsistenteId,
    poder: PoderRecibido,
) -> AppResult<Asistente>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasRegistrar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, reunion).await?;
    if !(r.estado == EstadoReunion::Convocada || r.estado.admite_registro()) {
        return Err(AppError::Conflicto("la reunión no admite registro".into()));
    }
    let a = asistente_de(&mut tx, reunion, asistente).await?;
    let persona = tx
        .persona(a.persona_id)
        .await?
        .ok_or(AppError::NoEncontrado("persona"))?;
    let ahora = reloj.ahora();
    agregar_poder(&mut tx, ctx, &r, asistente, &persona, &poder, ahora).await?;
    habilitar_en_abiertas(&mut tx, ctx, &r, asistente, ahora).await?;
    tx.avisar_en_vivo(reunion).await?;
    let a = asistente_de(&mut tx, reunion, asistente).await?;
    tx.confirmar().await?;
    Ok(a)
}

async fn asistente_de<T: TxAsambleas>(
    tx: &mut T,
    reunion: ReunionId,
    id: AsistenteId,
) -> AppResult<Asistente> {
    tx.asistente(id)
        .await?
        .filter(|a| a.reunion_id == reunion)
        .ok_or(AppError::NoEncontrado("asistente"))
}

/// Verifica, rechaza o revoca un poder. Rechazado o revocado, el bien deja de estar
/// representado por el apoderado.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si la
/// transición no es válida.
pub async fn revisar_poder<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    poder: PoderId,
    hacia: EstadoPoder,
    motivo: Option<&str>,
) -> AppResult<Poder>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasRegistrar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    reunion_de(&mut tx, reunion).await?;
    let mut p = tx
        .poder(poder)
        .await?
        .filter(|p| p.reunion_id == reunion)
        .ok_or(AppError::NoEncontrado("poder"))?;
    let ahora = reloj.ahora();
    p.estado = p.estado.transicionar(hacia)?;
    p.motivo = texto_opcional("motivo", motivo, 500)?;
    p.revisado_por = ctx.actor_id;
    p.revisado_en = Some(ahora);
    tx.actualizar_poder(&p).await?;
    if !p.estado.vigente() {
        tx.desvincular_bien(reunion, p.bien_privado_id, ahora)
            .await?;
    }
    tx.registrar(
        EventoAuditoria::ok(ACCION_REVISAR_PODER, "poder", p.id.as_uuid())
            .con_detalle(DetalleAuditoria::vacio().con("estado", p.estado.as_str())?),
    )
    .await?;
    tx.avisar_en_vivo(reunion).await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Poderes de la reunión.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.registrar`; [`AppError::NoEncontrado`].
pub async fn listar_poderes<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
) -> AppResult<Vec<Poder>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasRegistrar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    reunion_de(&mut tx, reunion).await?;
    tx.poderes(reunion).await
}

/// Registra la salida del asistente con hora y motivo: sus bienes dejan el quórum y las
/// preguntas que se abran después; sus votos ya emitidos se conservan.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si no estaba
/// presente; [`AppError::Validacion`] si el motivo es `otro` sin detalle.
pub async fn registrar_salida<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    asistente: AsistenteId,
    motivo: MotivoSalida,
    detalle: Option<&str>,
) -> AppResult<Asistente>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasRegistrar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let a = salida_en(&mut tx, reloj, reunion, asistente, motivo, detalle).await?;
    tx.confirmar().await?;
    Ok(a)
}

async fn salida_en<T: TxAsambleas>(
    tx: &mut T,
    reloj: &impl Reloj,
    reunion: ReunionId,
    asistente: AsistenteId,
    motivo: MotivoSalida,
    detalle: Option<&str>,
) -> AppResult<Asistente> {
    let r = reunion_de(tx, reunion).await?;
    if !r.estado.admite_registro() {
        return Err(AppError::Conflicto("la reunión no admite registro".into()));
    }
    asistente_de(tx, reunion, asistente).await?;
    let detalle = texto_opcional("detalle", detalle, 300)?;
    if motivo == MotivoSalida::Otro && detalle.is_none() {
        return Err(AppError::Validacion(
            "indique el motivo de la salida".into(),
        ));
    }
    if !tx
        .cerrar_presencia(asistente, reloj.ahora(), motivo, detalle.as_deref())
        .await?
    {
        return Err(AppError::Conflicto("el asistente no está presente".into()));
    }
    tx.registrar(
        EventoAuditoria::ok(ACCION_SALIDA, "asistente", asistente.as_uuid())
            .con_detalle(DetalleAuditoria::vacio().con("motivo", motivo.as_str())?),
    )
    .await?;
    tx.avisar_en_vivo(reunion).await?;
    asistente_de(tx, reunion, asistente).await
}

/// Registra el reingreso (nuevo periodo de presencia). Si hay preguntas abiertas, sus bienes
/// quedan habilitados y le llegan por WhatsApp.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si ya está
/// presente.
pub async fn registrar_reingreso<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    asistente: AsistenteId,
) -> AppResult<Asistente>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasRegistrar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let a = ingreso_en(&mut tx, reloj, ctx, reunion, asistente, ACCION_REINGRESO).await?;
    tx.confirmar().await?;
    Ok(a)
}

async fn ingreso_en<T: TxAsambleas>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    asistente: AsistenteId,
    accion: crate::auditoria::AccionAuditoria,
) -> AppResult<Asistente> {
    let r = reunion_de(tx, reunion).await?;
    if !r.estado.admite_registro() {
        return Err(AppError::Conflicto("la reunión no admite registro".into()));
    }
    let a = asistente_de(tx, reunion, asistente).await?;
    if a.presente {
        return Err(AppError::Conflicto("el asistente ya está presente".into()));
    }
    let ahora = reloj.ahora();
    tx.abrir_presencia(reunion, &presencia(asistente, ctx, ahora))
        .await?;
    habilitar_en_abiertas(tx, ctx, &r, asistente, ahora).await?;
    tx.registrar(EventoAuditoria::ok(
        accion,
        "asistente",
        asistente.as_uuid(),
    ))
    .await?;
    tx.avisar_en_vivo(reunion).await?;
    asistente_de(tx, reunion, asistente).await
}

/// Liga (o cambia) el rótulo del asistente. `None` lo libera.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si el código
/// ya está ligado a otro asistente de la reunión.
pub async fn asignar_rotulo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    asistente: AsistenteId,
    codigo: Option<CodigoRotulo>,
) -> AppResult<Asistente>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasRegistrar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    reunion_de(&mut tx, reunion).await?;
    asistente_de(&mut tx, reunion, asistente).await?;
    let ahora = reloj.ahora();
    tx.liberar_rotulo(asistente, ahora).await?;
    if let Some(c) = &codigo {
        tx.asignar_rotulo(reunion, c, asistente, ahora)
            .await
            .map_err(|e| ocupado(e, c.as_str()))?;
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_ROTULO,
        "asistente",
        asistente.as_uuid(),
    ))
    .await?;
    let a = asistente_de(&mut tx, reunion, asistente).await?;
    tx.confirmar().await?;
    Ok(a)
}

/// Encola la plantilla `confirmar_asistencia` por la línea con más franquicia libre.
pub(crate) async fn enviar_confirmacion<T: TxAsambleas>(
    tx: &mut T,
    ctx: &Contexto,
    r: &Reunion,
    asistente: AsistenteId,
) -> AppResult<bool> {
    let a = asistente_de(tx, r.id, asistente).await?;
    let Some(tel) = a.telefono.clone() else {
        return Ok(false);
    };
    let linea = match a.linea_whatsapp.clone() {
        Some(l) => Some(l),
        None => tx.elegir_linea().await?,
    };
    let Some(linea) = linea else {
        return Ok(false);
    };
    let ficha = tx
        .ficha()
        .await?
        .ok_or(AppError::NoEncontrado("copropiedad"))?;
    let codigos: Vec<_> = a.bienes.iter().map(|b| b.codigo.clone()).collect();
    let evento = EventoSaliente {
        prioridad: PrioridadSalida::Confirmacion,
        mensaje: MensajeOutbox::WhatsApp(MensajeSaliente {
            linea: linea.clone(),
            telefono: tel.clone(),
            contenido: ContenidoSaliente::Plantilla {
                nombre: PLANTILLA_CONFIRMAR_ASISTENCIA.into(),
                idioma: IDIOMA_PLANTILLAS.into(),
                parametros: vec![
                    super::corto(&a.nombre, 60),
                    super::corto(&ficha.nombre, 60),
                    codigos.len().to_string(),
                    super::corto(&codigos.join(", "), 120),
                ],
                botones: vec![payload_confirmar(ctx.tenant_id, a.id)],
            },
        }),
    };
    let outbox = tx.encolar_de_reunion(r.id, &evento).await?;
    tx.actualizar_medios(
        a.id,
        Some(&tel),
        Some(&linea),
        a.whatsapp_confirmado_en,
        Some(outbox),
    )
    .await?;
    Ok(true)
}

/// Reenvía la confirmación por WhatsApp (p. ej. con otro número si no llegó).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Validacion`] si el
/// teléfono es inválido o no hay líneas activas.
pub async fn reenviar_confirmacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    asistente: AsistenteId,
    telefono: Option<&str>,
) -> AppResult<Asistente>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasRegistrar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, reunion).await?;
    let a = asistente_de(&mut tx, reunion, asistente).await?;
    if let Some(t) = telefono {
        let tel = normalizar_telefono(t)?;
        // Un número nuevo empieza su propia ventana: se olvida la línea y la confirmación.
        let mismo = a.telefono.as_deref() == Some(tel.as_str());
        tx.actualizar_medios(
            asistente,
            Some(tel.as_str()),
            if mismo {
                a.linea_whatsapp.as_deref()
            } else {
                None
            },
            if mismo {
                a.whatsapp_confirmado_en
            } else {
                None
            },
            a.confirmacion_outbox_id,
        )
        .await?;
    }
    if !enviar_confirmacion(&mut tx, ctx, &r, asistente).await? {
        return Err(AppError::Validacion(
            "no hay teléfono o línea de WhatsApp activa para enviar la confirmación".into(),
        ));
    }
    let a = asistente_de(&mut tx, reunion, asistente).await?;
    tx.confirmar().await?;
    Ok(a)
}

/// Fila del tablero de la mesa: el asistente y si le llegó la confirmación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaMesa {
    pub asistente: Asistente,
    /// Último acuse de la plantilla de confirmación (`failed` = «no llegó: entregue rótulo»).
    pub entrega_confirmacion: Option<String>,
}

/// Asistentes de la reunión con el estado de entrega de su confirmación.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.registrar` ni `asambleas.ver`;
/// [`AppError::NoEncontrado`].
pub async fn asistentes_de_reunion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
) -> AppResult<Vec<FilaMesa>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    if ctx.exigir(Permiso::AsambleasRegistrar).is_err() {
        ctx.exigir(Permiso::AsambleasVer)?;
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    reunion_de(&mut tx, reunion).await?;
    let asistentes = tx.asistentes(reunion).await?;
    let ids: Vec<i64> = asistentes
        .iter()
        .filter_map(|a| a.confirmacion_outbox_id)
        .collect();
    let estados = tx.estados_entrega(&ids).await?;
    Ok(asistentes
        .into_iter()
        .map(|a| {
            let entrega = a
                .confirmacion_outbox_id
                .and_then(|id| estados.iter().find(|(o, _)| *o == id))
                .and_then(|(_, e)| e.clone());
            FilaMesa {
                asistente: a,
                entrega_confirmacion: entrega,
            }
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Morosos
// ---------------------------------------------------------------------------------------------

/// Resultado de fijar los morosos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultadoMorosos {
    pub marcados: u32,
    /// Códigos de la planilla que no están en el padrón.
    pub no_encontrados: Vec<String>,
}

/// Reemplaza el listado de morosos con los códigos de bien privado indicados (la PH entrega
/// el listado; este módulo no depende de la contabilidad). Se permite hasta antes de instalar.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `asambleas.convocar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si la reunión ya se instaló o aún no tiene padrón.
pub async fn fijar_morosos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    codigos: &[String],
) -> AppResult<ResultadoMorosos>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let r = reunion_de(&mut tx, reunion).await?;
    if !matches!(
        r.estado,
        EstadoReunion::Convocada | EstadoReunion::EnRegistro
    ) {
        return Err(AppError::Conflicto(
            "los morosos se fijan después de convocar y antes de instalar".into(),
        ));
    }
    let padron = tx.padron(reunion).await?;
    let normal = |c: &str| c.trim().to_uppercase();
    let mut bienes = Vec::new();
    let mut no_encontrados = Vec::new();
    for c in codigos.iter().map(|c| normal(c)).filter(|c| !c.is_empty()) {
        match padron.iter().find(|b| normal(&b.codigo) == c) {
            Some(b) if !bienes.contains(&b.bien_privado_id) => bienes.push(b.bien_privado_id),
            Some(_) => {}
            None => no_encontrados.push(c),
        }
    }
    tx.reemplazar_morosos(reunion, &bienes).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("marcados", bienes.len())?
        .con("efecto_mora", r.configuracion.efecto_mora.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_MOROSOS, "reunion", reunion.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.avisar_en_vivo(reunion).await?;
    tx.confirmar().await?;
    Ok(ResultadoMorosos {
        marcados: u32::try_from(bienes.len()).unwrap_or(u32::MAX),
        no_encontrados,
    })
}

/// Importa los morosos desde un CSV o XLSX: la primera columna (sin encabezado obligatorio) es
/// el código del bien privado.
///
/// # Errors
///
/// Como [`fijar_morosos`], y [`AppError::Dominio`] o [`AppError::Validacion`] si el archivo
/// no es una planilla legible.
pub async fn importar_morosos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    lector: &impl LectorPlanillas,
    ctx: &Contexto,
    reunion: ReunionId,
    contenido: &[u8],
) -> AppResult<ResultadoMorosos>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let tipo = TipoArchivo::validar_planilla(contenido)?;
    let filas = lector.leer(tipo, contenido)?;
    let codigos: Vec<String> = filas
        .into_iter()
        .filter_map(|f| f.into_iter().next())
        .filter(|c| {
            let c = c.trim().to_lowercase();
            !c.is_empty() && !matches!(c.as_str(), "codigo" | "código" | "bien" | "bien privado")
        })
        .collect();
    fijar_morosos(unidad, reloj, ctx, reunion, &codigos).await
}

// ---------------------------------------------------------------------------------------------
// Papeleta virtual (enlace personal: el token es la credencial)
// ---------------------------------------------------------------------------------------------

/// Pregunta abierta en la papeleta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreguntaPapeleta {
    pub pregunta: crate::puertos::asambleas::PreguntaGuardada,
    pub ya_voto: bool,
}

/// Lo que ve el asistente con su enlace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Papeleta {
    pub reunion: Reunion,
    pub asistente: Asistente,
    pub preguntas_abiertas: Vec<PreguntaPapeleta>,
}

async fn asistente_por_token<T: TxAsambleas>(tx: &mut T, token: &str) -> AppResult<Asistente> {
    // Mismo error para token inexistente o de otra copropiedad: no da pistas.
    tx.asistente_por_token(&hash_token(token))
        .await?
        .ok_or(AppError::NoAutenticado)
}

/// Papeleta del asistente (contexto del sistema de la copropiedad del enlace).
///
/// # Errors
///
/// [`AppError::NoAutenticado`] si el token no es válido; [`AppError::ModuloNoHabilitado`].
pub async fn ver_papeleta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    token: &str,
) -> AppResult<Papeleta>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let asistente = asistente_por_token(&mut tx, token).await?;
    let reunion = reunion_de(&mut tx, asistente.reunion_id).await?;
    let mut preguntas_abiertas = Vec::new();
    for p in tx.preguntas(reunion.id).await? {
        if p.pregunta.estado == domain::EstadoPregunta::Abierta {
            let ya_voto = tx.asistente_voto(p.pregunta.id, asistente.id).await?;
            preguntas_abiertas.push(PreguntaPapeleta {
                pregunta: p,
                ya_voto,
            });
        }
    }
    Ok(Papeleta {
        reunion,
        asistente,
        preguntas_abiertas,
    })
}

/// Ingreso virtual con el enlace (reuniones virtuales o mixtas): abre la presencia.
///
/// # Errors
///
/// [`AppError::NoAutenticado`]; [`AppError::Conflicto`] si la reunión es presencial (la
/// asistencia la certifica la mesa), no admite registro o ya está presente.
pub async fn ingresar_con_papeleta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    token: &str,
) -> AppResult<Asistente>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let a = asistente_por_token(&mut tx, token).await?;
    let r = reunion_de(&mut tx, a.reunion_id).await?;
    if !matches!(r.modalidad, Modalidad::Virtual | Modalidad::Mixta) {
        return Err(AppError::Conflicto(
            "en una reunión presencial la asistencia la registra la mesa".into(),
        ));
    }
    let a = ingreso_en(&mut tx, reloj, ctx, r.id, a.id, super::ACCION_REGISTRAR).await?;
    tx.confirmar().await?;
    Ok(a)
}

/// Salida voluntaria desde la papeleta («salir de la asamblea»).
///
/// # Errors
///
/// [`AppError::NoAutenticado`]; [`AppError::Conflicto`] si no estaba presente.
pub async fn salir_con_papeleta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    token: &str,
) -> AppResult<Asistente>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let a = asistente_por_token(&mut tx, token).await?;
    let a = salida_en(
        &mut tx,
        reloj,
        a.reunion_id,
        a.id,
        MotivoSalida::SeRetiro,
        None,
    )
    .await?;
    tx.confirmar().await?;
    Ok(a)
}

/// Vota desde la papeleta (canal web).
///
/// # Errors
///
/// [`AppError::NoAutenticado`] y los de [`super::votaciones::emitir_voto`].
pub async fn votar_con_papeleta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    token: &str,
    pregunta: domain::PreguntaId,
    opciones: Vec<domain::OpcionId>,
    clave: domain::ClaveIdempotencia,
) -> AppResult<ResultadoVoto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let a = asistente_por_token(&mut tx, token).await?;
    let resultado = emitir_en(
        &mut tx,
        reloj,
        ctx,
        SolicitudVoto {
            reunion_id: a.reunion_id,
            pregunta_id: pregunta,
            votante: Votante::Asistente(a.id),
            opciones,
            canal: domain::Canal::Web,
            idempotency_key: clave,
            wamid: None,
            confirmar: false,
        },
    )
    .await?;
    tx.confirmar().await?;
    Ok(resultado)
}

/// Reenvía las preguntas abiertas por WhatsApp a un asistente (p. ej. al confirmar).
pub(crate) async fn enviar_abiertas<T: TxAsambleas>(
    tx: &mut T,
    ctx: &Contexto,
    r: &Reunion,
    asistente: AsistenteId,
) -> AppResult<u32> {
    let mut n = 0;
    for p in tx.preguntas(r.id).await? {
        if p.pregunta.estado == domain::EstadoPregunta::Abierta
            && !tx.asistente_voto(p.pregunta.id, asistente).await?
            && enviar_pregunta(tx, ctx, r, &p, asistente).await?
        {
            n += 1;
        }
    }
    Ok(n)
}
