//! La asamblea por WhatsApp (el gateway llama a [`atender_mensaje_asamblea`] antes del chat de
//! la copropiedad).
//!
//! Reglas que impiden mezclar votos entre asambleas simultáneas (plan 06 §3):
//! 1. Cada botón o fila lleva la copropiedad, la pregunta y la opción en su id
//!    (`vt:{tenant}:{pregunta}:{opcion}`): el voto se aplica a **esa** pregunta, nunca a «la que
//!    esté activa». Un toque tardío sobre una pregunta cerrada recibe «la votación ya cerró».
//! 2. Antes de registrar se verifica que el teléfono sea de un asistente de esa reunión.
//! 3. El texto libre («sí», «1», «1,3») solo vale si el teléfono tiene **exactamente una**
//!    pregunta abierta pendiente en todas sus asambleas; si no, se le reenvían las preguntas.
//! 4. Todo dentro del tenant (RLS); el cruce de copropiedades solo por funciones
//!    `SECURITY DEFINER` que devuelven ids.
//!
//! El botón «Confirmar» de la plantilla `confirmar_asistencia` (`asis:{tenant}:{asistente}`)
//! abre la ventana de 24 h de Meta: desde ahí las preguntas llegan como mensajes interactivos.

use domain::{
    AsistenteId, Canal, ClaveIdempotencia, MotivoSalida, OpcionId, PreguntaId, TenantId,
    TipoPregunta,
};
use uuid::Uuid;

use super::{
    TxAsambleas, corto,
    registro::enviar_abiertas,
    votaciones::{ResultadoVoto, SolicitudVoto, Votante, emitir_en},
};
use crate::{
    AppError, AppResult, Contexto,
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        Reloj, RepositorioAsambleas, SesionesConversacion, Transaccion, UnidadDeTrabajo,
        asambleas::{
            AsistenciaTelefono, Asistente, DirectorioAsambleas, PreguntaGuardada, Reunion,
        },
        mensajeria::{
            ContenidoEntrante, ContenidoSaliente, Eleccion, MensajeEntrante, MensajeSaliente,
        },
    },
};

const PREFIJO_VOTO: &str = "vt:";
const PREFIJO_CONFIRMAR: &str = "asis:";

/// `payload` del botón «Confirmar» de la plantilla.
pub fn payload_confirmar(tenant: TenantId, asistente: AsistenteId) -> String {
    format!("{PREFIJO_CONFIRMAR}{tenant}:{asistente}")
}

/// Id de la fila o botón de una opción.
pub fn id_voto(tenant: TenantId, pregunta: PreguntaId, opcion: OpcionId) -> String {
    format!("{PREFIJO_VOTO}{tenant}:{pregunta}:{opcion}")
}

fn uuids(resto: &str, n: usize) -> Option<Vec<Uuid>> {
    let partes: Vec<_> = resto.split(':').collect();
    (partes.len() == n)
        .then(|| partes.iter().map(|p| Uuid::try_parse(p).ok()).collect())
        .flatten()
}

/// Interpreta un id de voto.
pub fn leer_id_voto(id: &str) -> Option<(TenantId, PreguntaId, OpcionId)> {
    let u = uuids(id.strip_prefix(PREFIJO_VOTO)?, 3)?;
    Some((
        TenantId::from_uuid(u[0]),
        PreguntaId::from_uuid(u[1]),
        OpcionId::from_uuid(u[2]),
    ))
}

/// Interpreta el `payload` de «Confirmar».
pub fn leer_payload_confirmar(id: &str) -> Option<(TenantId, AsistenteId)> {
    let u = uuids(id.strip_prefix(PREFIJO_CONFIRMAR)?, 2)?;
    Some((TenantId::from_uuid(u[0]), AsistenteId::from_uuid(u[1])))
}

fn codigos(a: &Asistente) -> String {
    match a.bienes.as_slice() {
        [] => "sin bienes".into(),
        [uno] => uno.codigo.clone(),
        [primero, resto @ ..] => format!("{} (+{} bienes)", primero.codigo, resto.len()),
    }
}

/// Mensaje interactivo con la pregunta (botones si hay hasta 3 opciones; lista si hay más).
pub fn mensaje_pregunta(
    tenant: TenantId,
    titulo: &str,
    p: &PreguntaGuardada,
    a: &Asistente,
) -> ContenidoSaliente {
    let pregunta = &p.pregunta;
    let mut cuerpo = format!(
        "🗳️ {}\n\n{}. {}\n\nVota por: {}",
        corto(titulo, 80),
        p.orden,
        corto(&pregunta.enunciado, 700),
        codigos(a)
    );
    if pregunta.tipo == TipoPregunta::SeleccionMultiple {
        cuerpo.push_str(&format!(
            "\n\nPuede elegir hasta {}: responda con los números separados por coma (p. ej. 1,3).",
            pregunta.max_selecciones
        ));
    }
    let lista = pregunta.opciones.len() > ContenidoSaliente::MAX_BOTONES;
    let opciones = pregunta
        .opciones
        .iter()
        .map(|o| {
            let titulo = format!("{}. {}", o.orden, o.texto);
            let e = Eleccion::new(
                id_voto(tenant, pregunta.id, o.id),
                corto(&titulo, if lista { 24 } else { 20 }),
            );
            if lista && titulo.chars().count() > 24 {
                e.con_descripcion(corto(&o.texto, 72))
            } else {
                e
            }
        })
        .collect();
    ContenidoSaliente::elecciones(cuerpo, opciones)
}

/// Confirmación del voto: **sin la opción elegida** (invariante 4).
pub fn confirmacion_voto(enunciado: &str, r: &ResultadoVoto) -> String {
    let mut t = format!(
        "✅ Voto registrado por {}: «{}».",
        r.bienes.join(", "),
        corto(enunciado, 120)
    );
    if !r.omitidos.is_empty() {
        t.push_str(&format!(
            "\nNo votaron esta vez (ya habían votado o no están habilitados): {}.",
            r.omitidos.join(", ")
        ));
    }
    t
}

fn mensaje_error(e: &AppError) -> String {
    match e {
        AppError::Dominio(domain::DominioError::PreguntaNoAbierta(_)) => {
            "⏱️ La votación de esa pregunta ya cerró.".into()
        }
        AppError::Dominio(domain::DominioError::ReunionNoInstalada(_)) => {
            "La reunión no está instalada en este momento.".into()
        }
        AppError::NoInscrito => {
            "Este número no está registrado como asistente presente de esa asamblea. Acérquese a la mesa de registro.".into()
        }
        AppError::BienPrivadoYaVoto => "Sus bienes privados ya votaron esta pregunta.".into(),
        AppError::Dominio(domain::DominioError::VotoInvalido(m)) => format!("Voto no válido: {m}."),
        AppError::Conflicto(m) => format!("{m}."),
        AppError::NoEncontrado(_) => "Esa pregunta no pertenece a una asamblea suya.".into(),
        _ => "No pudimos registrar su respuesta. Intente de nuevo en un momento.".into(),
    }
}

/// Adaptadores que usa la asamblea por WhatsApp.
pub struct ServiciosAsamblea<'a, U, D, S, R> {
    pub unidad: &'a U,
    pub directorio: &'a D,
    pub sesiones: &'a S,
    pub reloj: &'a R,
}

/// Resultado de atender un mensaje.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultadoAsamblea {
    /// No es de una asamblea: lo atiende el chat de la copropiedad.
    NoAplica,
    /// Reintento de Meta ya procesado.
    Repetido,
    /// Atendido; `respuestas` mensajes encolados.
    Atendido { respuestas: usize },
}

/// Atiende un mensaje entrante si corresponde a una asamblea activa del teléfono.
///
/// # Errors
///
/// Solo errores de infraestructura del directorio o de las sesiones; los de los casos de uso
/// se responden al asistente con un mensaje seguro.
pub async fn atender_mensaje_asamblea<U, D, S, R>(
    s: &ServiciosAsamblea<'_, U, D, S, R>,
    m: &MensajeEntrante,
) -> AppResult<ResultadoAsamblea>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
    D: DirectorioAsambleas,
    S: SesionesConversacion,
    R: Reloj,
{
    let respuesta = match &m.contenido {
        ContenidoEntrante::Respuesta { id, .. } => {
            if let Some(voto) = leer_id_voto(id) {
                Some(voto).map(|v| Accion::Votar(v.0, v.1, vec![v.2]))
            } else {
                leer_payload_confirmar(id).map(|(t, a)| Accion::Confirmar(t, a))
            }
        }
        _ => None,
    };
    let accion = match respuesta {
        Some(a) => a,
        None => {
            let asistencias = s.directorio.asistencias_por_telefono(&m.telefono).await?;
            if asistencias.is_empty() || pide_menu(&m.contenido) {
                return Ok(ResultadoAsamblea::NoAplica);
            }
            Accion::Texto(asistencias)
        }
    };
    if s.sesiones.ya_procesado(&m.wamid).await? {
        return Ok(ResultadoAsamblea::Repetido);
    }
    let respuestas = match accion {
        Accion::Votar(tenant, pregunta, opciones) => {
            votar(s, m, tenant, pregunta, opciones).await?
        }
        Accion::Confirmar(tenant, asistente) => confirmar(s, m, tenant, asistente).await?,
        Accion::Texto(asistencias) => texto_libre(s, m, &asistencias).await?,
    };
    s.sesiones.marcar_procesado(&m.wamid).await?;
    Ok(ResultadoAsamblea::Atendido { respuestas })
}

enum Accion {
    Votar(TenantId, PreguntaId, Vec<OpcionId>),
    Confirmar(TenantId, AsistenteId),
    Texto(Vec<AsistenciaTelefono>),
}

fn pide_menu(c: &ContenidoEntrante) -> bool {
    matches!(c, ContenidoEntrante::Texto { cuerpo } if matches!(
        cuerpo.trim().to_lowercase().as_str(), "menu" | "menú" | "inicio"))
}

fn ctx_de(tenant: TenantId, m: &MensajeEntrante) -> Contexto {
    Contexto::new(tenant).con_request_id(Some(m.wamid.clone()))
}

/// Encola respuestas de texto en una transacción nueva (la del caso de uso pudo abortar).
async fn responder<U>(
    unidad: &U,
    ctx: &Contexto,
    reunion: Option<domain::ReunionId>,
    m: &MensajeEntrante,
    textos: Vec<ContenidoSaliente>,
) -> AppResult<usize>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    let n = textos.len();
    let mut tx = unidad.iniciar(ctx).await?;
    for contenido in textos {
        let evento = EventoSaliente {
            prioridad: PrioridadSalida::Confirmacion,
            mensaje: MensajeOutbox::WhatsApp(MensajeSaliente {
                linea: m.linea.clone(),
                telefono: m.telefono.clone(),
                contenido,
            }),
        };
        match reunion {
            Some(r) => {
                tx.encolar_de_reunion(r, &evento).await?;
            }
            None => crate::puertos::RegistroOutbox::encolar(&mut tx, evento).await?,
        }
    }
    tx.confirmar().await?;
    Ok(n)
}

fn texto(t: impl Into<String>) -> ContenidoSaliente {
    ContenidoSaliente::Texto { cuerpo: t.into() }
}

/// Marca que el asistente escribió por esta línea (ventana de 24 h abierta).
async fn marcar_ventana<T: TxAsambleas>(
    tx: &mut T,
    a: &Asistente,
    m: &MensajeEntrante,
    ahora: chrono::DateTime<chrono::Utc>,
) -> AppResult<()> {
    if a.whatsapp_confirmado_en.is_none() || a.linea_whatsapp.as_deref() != Some(m.linea.as_str()) {
        tx.actualizar_medios(
            a.id,
            Some(&m.telefono),
            Some(&m.linea),
            Some(a.whatsapp_confirmado_en.unwrap_or(ahora)),
            a.confirmacion_outbox_id,
        )
        .await?;
    }
    Ok(())
}

async fn votar<U, D, S, R>(
    s: &ServiciosAsamblea<'_, U, D, S, R>,
    m: &MensajeEntrante,
    tenant: TenantId,
    pregunta: PreguntaId,
    opciones: Vec<OpcionId>,
) -> AppResult<usize>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
    R: Reloj,
{
    let ctx = ctx_de(tenant, m);
    let mut reunion = None;
    let resultado = async {
        let mut tx = super::abrir(s.unidad, s.reloj, &ctx).await?;
        let p = tx
            .pregunta(pregunta)
            .await?
            .ok_or(AppError::NoEncontrado("pregunta"))?;
        reunion = Some(p.pregunta.reunion_id);
        let a = tx
            .asistentes_por_telefono(&m.telefono)
            .await?
            .into_iter()
            .find(|a| a.reunion_id == p.pregunta.reunion_id)
            .ok_or(AppError::NoInscrito)?;
        marcar_ventana(&mut tx, &a, m, s.reloj.ahora()).await?;
        let clave = ClaveIdempotencia::new(m.wamid.clone())?;
        let r = emitir_en(
            &mut tx,
            s.reloj,
            &ctx,
            SolicitudVoto {
                reunion_id: p.pregunta.reunion_id,
                pregunta_id: pregunta,
                votante: Votante::Asistente(a.id),
                opciones,
                canal: Canal::Whatsapp,
                idempotency_key: clave,
                wamid: Some(m.wamid.clone()),
                confirmar: true,
            },
        )
        .await?;
        tx.confirmar().await?;
        Ok::<_, AppError>(r)
    }
    .await;
    match resultado {
        Ok(r) => Ok(usize::from(r.registrado)),
        Err(AppError::Infraestructura(e)) => Err(AppError::Infraestructura(e)),
        Err(e) => responder(s.unidad, &ctx, reunion, m, vec![texto(mensaje_error(&e))]).await,
    }
}

async fn confirmar<U, D, S, R>(
    s: &ServiciosAsamblea<'_, U, D, S, R>,
    m: &MensajeEntrante,
    tenant: TenantId,
    asistente: AsistenteId,
) -> AppResult<usize>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
    R: Reloj,
{
    let ctx = ctx_de(tenant, m);
    let resultado = async {
        let mut tx = super::abrir(s.unidad, s.reloj, &ctx).await?;
        // El botón solo vale desde el teléfono al que se envió.
        let a = tx
            .asistente(asistente)
            .await?
            .filter(|a| a.telefono.as_deref() == Some(m.telefono.as_str()))
            .ok_or(AppError::NoInscrito)?;
        let r = super::reunion_de(&mut tx, a.reunion_id).await?;
        if !r.estado.admite_registro() {
            return Err(AppError::Conflicto("la asamblea no está en curso".into()));
        }
        marcar_ventana(&mut tx, &a, m, s.reloj.ahora()).await?;
        let a = tx.asistente(asistente).await?.ok_or(AppError::NoInscrito)?;
        let evento = EventoSaliente {
            prioridad: PrioridadSalida::Confirmacion,
            mensaje: MensajeOutbox::WhatsApp(MensajeSaliente {
                linea: m.linea.clone(),
                telefono: m.telefono.clone(),
                contenido: texto(format!(
                    "✅ Listo, {}. Votará por {} en «{}». Las preguntas le llegarán por aquí cuando se abran; un solo voto cuenta para todos sus bienes.",
                    a.nombre,
                    codigos(&a),
                    r.titulo
                )),
            }),
        };
        tx.encolar_de_reunion(r.id, &evento).await?;
        let n = enviar_abiertas(&mut tx, &ctx, &r, a.id).await?;
        tx.confirmar().await?;
        Ok::<_, AppError>(1 + n as usize)
    }
    .await;
    match resultado {
        Ok(n) => Ok(n),
        Err(AppError::Infraestructura(e)) => Err(AppError::Infraestructura(e)),
        Err(e) => responder(s.unidad, &ctx, None, m, vec![texto(mensaje_error(&e))]).await,
    }
}

/// Opciones elegidas en un texto libre: números («1», «1,3») o «sí»/«no»/«abstención».
pub fn opciones_de_texto(p: &PreguntaGuardada, t: &str) -> Option<Vec<OpcionId>> {
    let t = t.trim().to_lowercase();
    let por_texto = |buscado: &[&str]| {
        p.pregunta
            .opciones
            .iter()
            .find(|o| buscado.contains(&o.texto.to_lowercase().as_str()))
            .map(|o| vec![o.id])
    };
    match t.as_str() {
        "si" | "sí" => return por_texto(&["sí", "si"]),
        "no" => return por_texto(&["no"]),
        "abstencion" | "abstención" => return por_texto(&["abstención", "abstencion"]),
        _ => {}
    }
    let numeros: Option<Vec<u16>> = t
        .split([',', ' ', ';', 'y'])
        .filter(|x| !x.is_empty())
        .map(|x| x.parse().ok())
        .collect();
    let mut ids = Vec::new();
    for n in numeros? {
        ids.push(p.pregunta.opciones.iter().find(|o| o.orden == n)?.id);
    }
    (!ids.is_empty()).then_some(ids)
}

struct Pendiente {
    tenant: TenantId,
    reunion: Reunion,
    asistente: Asistente,
    pregunta: PreguntaGuardada,
}

async fn texto_libre<U, D, S, R>(
    s: &ServiciosAsamblea<'_, U, D, S, R>,
    m: &MensajeEntrante,
    asistencias: &[AsistenciaTelefono],
) -> AppResult<usize>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
    R: Reloj,
{
    let ahora = s.reloj.ahora();
    let cuerpo = match &m.contenido {
        ContenidoEntrante::Texto { cuerpo } => Some(cuerpo.trim().to_owned()),
        _ => None,
    };
    let mut pendientes = Vec::new();
    let mut registradas = Vec::new();
    for asis in asistencias {
        let ctx = ctx_de(asis.tenant_id, m);
        let mut tx = super::abrir(s.unidad, s.reloj, &ctx).await?;
        let Some(a) = tx.asistente(asis.asistente_id).await? else {
            continue;
        };
        let r = super::reunion_de(&mut tx, asis.reunion_id).await?;
        marcar_ventana(&mut tx, &a, m, ahora).await?;
        if cuerpo
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case("salir de la asamblea"))
            && a.presente
            && asistencias.len() == 1
        {
            tx.cerrar_presencia(a.id, ahora, MotivoSalida::SeRetiro, None)
                .await?;
            tx.avisar_en_vivo(r.id).await?;
            tx.confirmar().await?;
            return responder(
                s.unidad,
                &ctx,
                Some(r.id),
                m,
                vec![texto(format!(
                    "Registramos su salida de «{}». Sus votos se conservan; si regresa, acérquese a la mesa.",
                    r.titulo
                ))],
            )
            .await;
        }
        for p in tx.preguntas(r.id).await? {
            if p.pregunta.estado == domain::EstadoPregunta::Abierta
                && a.presente
                && !tx.asistente_voto(p.pregunta.id, a.id).await?
            {
                pendientes.push(Pendiente {
                    tenant: asis.tenant_id,
                    reunion: r.clone(),
                    asistente: a.clone(),
                    pregunta: p,
                });
            }
        }
        registradas.push((asis.tenant_id, r, a));
        tx.confirmar().await?;
    }

    // Texto libre solo con una única pregunta pendiente (rule 3 del módulo).
    if let ([unica], Some(t)) = (pendientes.as_slice(), cuerpo.as_deref())
        && let Some(opciones) = opciones_de_texto(&unica.pregunta, t)
    {
        return votar(s, m, unica.tenant, unica.pregunta.pregunta.id, opciones).await;
    }

    if pendientes.is_empty() {
        let mut n = 0;
        for (tenant, r, a) in &registradas {
            let estado = if a.presente {
                "Las preguntas le llegarán por aquí cuando se abran."
            } else {
                "Su salida está registrada; si regresa, acérquese a la mesa."
            };
            n += responder(
                s.unidad,
                &ctx_de(*tenant, m),
                Some(r.id),
                m,
                vec![texto(format!(
                    "Está registrado en «{}» por {}. {estado}",
                    r.titulo,
                    codigos(a)
                ))],
            )
            .await?;
        }
        return Ok(n);
    }

    // Varias pendientes (o texto que no se entendió): se reenvían como mensajes interactivos;
    // cada botón lleva su pregunta.
    let mut n = 0;
    for p in &pendientes {
        let contenido = mensaje_pregunta(p.tenant, &p.reunion.titulo, &p.pregunta, &p.asistente);
        n += responder(
            s.unidad,
            &ctx_de(p.tenant, m),
            Some(p.reunion.id),
            m,
            vec![contenido],
        )
        .await?;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_de_voto_ida_y_vuelta() {
        let (t, p, o) = (TenantId::nuevo(), PreguntaId::nuevo(), OpcionId::nuevo());
        let id = id_voto(t, p, o);
        assert!(id.len() <= 256, "Meta admite ids de hasta 256 caracteres");
        assert_eq!(leer_id_voto(&id), Some((t, p, o)));
        assert_eq!(leer_id_voto("vt:x:y:z"), None);
        assert_eq!(leer_id_voto("v:0"), None, "no se confunde con el chat");
        let a = AsistenteId::nuevo();
        assert_eq!(
            leer_payload_confirmar(&payload_confirmar(t, a)),
            Some((t, a))
        );
        assert_eq!(leer_payload_confirmar(&id), None);
    }
}
