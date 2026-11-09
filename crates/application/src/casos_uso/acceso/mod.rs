//! Control de acceso, visitas y correspondencia (plan 05, módulo vendible `control_acceso`).
//!
//! - [`visitas`]: anunciar, pre-autorizar (QR temporal firmado), decidir (app, web, WhatsApp o
//!   llamada), ingreso y salida.
//! - [`correspondencia`]: recibir, avisar cada 24 h, entregar con firma o foto y devolver.
//! - [`porteria`]: configuración, minuta, bitácora de eventos, alertas y costo de los avisos.
//! - [`credenciales`]: credenciales, autorizaciones, verificación en la portería digital y
//!   consentimientos biométricos.
//! - [`puentes`]: puntos de acceso, puentes y dispositivos; sincronización, eventos y latido.
//! - [`tareas`]: lo que corre el worker (escalamiento, vencimientos y retención).
//! - [`whatsapp`]: botones «Autorizar» / «Rechazar» de la plantilla `visita_anunciada`.
//!
//! Invariantes (plan 05 §8):
//! 1. Una visita tiene **una** decisión: la primera respuesta válida, bajo `FOR UPDATE`.
//! 2. Un aviso sale una vez por nivel (el nivel se guarda en la misma transacción).
//! 3. Push primero; WhatsApp solo como respaldo ([`domain::acceso::elegir_canal_aviso`]).
//! 4. Una correspondencia entregada no recibe más avisos y tiene una entrega con firma o foto.
//! 5. Historiales append-only (eventos de visita, entregas, eventos de acceso, minuta).
//! 6. Ningún identificador de credencial en claro; ninguna plantilla biométrica sale del equipo.
//! 7. Un QR temporal no sirve fuera de su ventana ni más veces que sus usos.
//! 8. Las autorizaciones derivadas se calculan del censo y la titularidad vigentes: terminar la
//!    residencia o vender el bien las retira en la siguiente sincronización.
//! 9. Firmas y fotos solo por `AlmacenArchivos`; nombres y teléfonos no van a logs.

pub mod correspondencia;
pub mod credenciales;
pub mod porteria;
pub mod puentes;
pub mod push;
pub mod tareas;
pub mod visitas;
pub mod whatsapp;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use domain::{
    BienPrivadoId, CuentaId, Modulo, NivelAviso, PersonaId, ResidenteAviso, TitularAviso,
    acceso::{destinatarios_aviso, elegir_canal_aviso},
    reserva::Tz,
};

use crate::{
    AppError, AppResult, Contexto,
    casos_uso::{avisos::IDIOMA_PLANTILLAS, modulos::exigir_modulo_en, reservas::zona},
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        RegistroAuditoria, RegistroOutbox, Reloj, RepositorioAcceso, RepositorioArchivos,
        RepositorioAsambleas, RepositorioBienesPrivados, RepositorioCenso, RepositorioCopropiedad,
        RepositorioModulos, RepositorioPersonas, RepositorioTickets, RepositorioTitulares,
        UnidadDeTrabajo,
        acceso::NotificacionPush,
        copropiedades::Persona,
        mensajeria::{ContenidoSaliente, MensajeSaliente},
    },
};

/// Puertos que usan los casos de uso de acceso dentro de la transacción de la copropiedad.
pub trait TxAcceso:
    RepositorioAcceso
    + RepositorioCenso
    + RepositorioPersonas
    + RepositorioTitulares
    + RepositorioBienesPrivados
    + RepositorioCopropiedad
    + RepositorioModulos
    + RepositorioArchivos
    + RepositorioTickets
    + RepositorioAsambleas
    + RegistroOutbox
    + RegistroAuditoria
{
}

impl<T> TxAcceso for T where
    T: RepositorioAcceso
        + RepositorioCenso
        + RepositorioPersonas
        + RepositorioTitulares
        + RepositorioBienesPrivados
        + RepositorioCopropiedad
        + RepositorioModulos
        + RepositorioArchivos
        + RepositorioTickets
        + RepositorioAsambleas
        + RegistroOutbox
        + RegistroAuditoria
{
}

/// Abre la transacción y exige el módulo contratado.
pub(crate) async fn abrir<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::ControlAcceso).await?;
    Ok(tx)
}

pub(crate) fn tiene(ctx: &Contexto, p: domain::Permiso) -> bool {
    ctx.exigir(p).is_ok()
}

pub(crate) fn hoy(ahora: DateTime<Utc>, tz: Tz) -> chrono::NaiveDate {
    ahora.with_timezone(&tz).date_naive()
}

/// Código legible del bien privado («T3-506»).
pub(crate) async fn codigo_bien<T: TxAcceso>(tx: &mut T, bien: BienPrivadoId) -> AppResult<String> {
    Ok(tx
        .bien(bien)
        .await?
        .ok_or(AppError::NoEncontrado("bien privado"))?
        .codigo
        .as_str()
        .to_owned())
}

/// Residentes vigentes y titulares del bien, con lo necesario para decidir a quién avisar.
pub(crate) async fn vinculados<T: TxAcceso>(
    tx: &mut T,
    bien: BienPrivadoId,
) -> AppResult<(Vec<ResidenteAviso>, Vec<TitularAviso>)> {
    let mut residentes = Vec::new();
    for r in tx.residencias(Some(bien), false).await? {
        let nacimiento = tx
            .persona(r.persona_id)
            .await?
            .and_then(|p| p.fecha_nacimiento);
        residentes.push(ResidenteAviso {
            persona: r.persona_id,
            es_responsable: r.es_responsable,
            fecha_nacimiento: nacimiento,
        });
    }
    let titulares = tx
        .titulares_vigentes(bien)
        .await?
        .into_iter()
        .map(|t| TitularAviso {
            persona: t.persona_id,
            es_representante: t.es_representante,
        })
        .collect();
    Ok((residentes, titulares))
}

/// Persona de la copropiedad vinculada a la cuenta del contexto.
pub(crate) async fn persona_del_actor<T: TxAcceso>(
    tx: &mut T,
    ctx: &Contexto,
) -> AppResult<Option<PersonaId>> {
    match ctx.actor_id {
        Some(c) => tx.persona_de_cuenta(c).await,
        None => Ok(None),
    }
}

/// Plantilla de WhatsApp de un aviso (respaldo cuando no hay app).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlantillaAviso {
    pub nombre: &'static str,
    pub parametros: Vec<String>,
    pub botones: Vec<String>,
}

/// Aviso a residentes: push con datos para la app y plantilla de WhatsApp de respaldo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvisoAcceso {
    pub prioridad: PrioridadSalida,
    pub titulo: String,
    pub cuerpo: String,
    pub datos: BTreeMap<String, String>,
    pub plantilla: PlantillaAviso,
}

/// Máximo de caracteres de un parámetro de plantilla.
const MAX_PARAMETRO: usize = 200;

fn parametro(texto: &str) -> String {
    texto
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_PARAMETRO)
        .collect()
}

fn whatsapp(linea: &str, telefono: &str, p: &PlantillaAviso) -> MensajeSaliente {
    MensajeSaliente {
        linea: linea.to_owned(),
        telefono: telefono.to_owned(),
        contenido: ContenidoSaliente::Plantilla {
            nombre: p.nombre.to_owned(),
            idioma: IDIOMA_PLANTILLAS.to_owned(),
            parametros: p.parametros.iter().map(|x| parametro(x)).collect(),
            botones: p.botones.clone(),
        },
    }
}

/// Envía el aviso a cada persona por su mejor canal: push a cada dispositivo de su cuenta (con
/// el WhatsApp de respaldo en el primero) o, si no tiene app, la plantilla de WhatsApp.
/// Devuelve a cuántas personas les llegó por algún canal.
pub(crate) async fn avisar_personas<T: TxAcceso>(
    tx: &mut T,
    personas: &[PersonaId],
    aviso: &AvisoAcceso,
) -> AppResult<u32> {
    let mut datos: Vec<Persona> = Vec::new();
    for p in personas {
        if let Some(x) = tx.persona(*p).await? {
            datos.push(x);
        }
    }
    let cuentas: Vec<CuentaId> = datos.iter().filter_map(|p| p.cuenta_id).collect();
    let tokens = if cuentas.is_empty() {
        Vec::new()
    } else {
        tx.tokens_push(&cuentas).await?
    };
    let linea = tx.linea_predeterminada().await?;
    let mut avisadas = 0;
    for p in &datos {
        let suyos: Vec<&String> = tokens
            .iter()
            .filter(|(c, _)| Some(*c) == p.cuenta_id)
            .map(|(_, t)| t)
            .collect();
        let respaldo = match (&linea, &p.telefono) {
            (Some(l), Some(t)) => Some(whatsapp(l, t.as_str(), &aviso.plantilla)),
            _ => None,
        };
        match elegir_canal_aviso(!suyos.is_empty(), respaldo.is_some()) {
            Some(domain::CanalAviso::Push) => {
                for (i, token) in suyos.into_iter().enumerate() {
                    let n = NotificacionPush {
                        token: token.clone(),
                        titulo: aviso.titulo.clone(),
                        cuerpo: aviso.cuerpo.clone(),
                        datos: aviso.datos.clone(),
                        respaldo: if i == 0 { respaldo.clone() } else { None },
                    };
                    tx.encolar_aviso_acceso(&EventoSaliente {
                        prioridad: aviso.prioridad,
                        mensaje: MensajeOutbox::Push(n),
                    })
                    .await?;
                }
                avisadas += 1;
            }
            Some(domain::CanalAviso::Whatsapp) => {
                if let Some(m) = respaldo {
                    tx.encolar_aviso_acceso(&EventoSaliente {
                        prioridad: aviso.prioridad,
                        mensaje: MensajeOutbox::WhatsApp(m),
                    })
                    .await?;
                    avisadas += 1;
                }
            }
            None => {}
        }
    }
    Ok(avisadas)
}

/// Destinatarios del nivel y aviso. Devuelve a cuántas personas les llegó.
pub(crate) async fn avisar_bien<T: TxAcceso>(
    tx: &mut T,
    bien: BienPrivadoId,
    nivel: NivelAviso,
    hoy: chrono::NaiveDate,
    aviso: &AvisoAcceso,
) -> AppResult<u32> {
    let (residentes, titulares) = vinculados(tx, bien).await?;
    let personas = destinatarios_aviso(nivel, &residentes, &titulares, hoy);
    avisar_personas(tx, &personas, aviso).await
}

/// Zona horaria de la PH.
pub(crate) async fn zona_ph<T: TxAcceso>(tx: &mut T) -> AppResult<Tz> {
    zona(tx).await
}

/// Texto corto y limpio para un parámetro o el cuerpo de un push.
pub(crate) fn corto(texto: &str, max: usize) -> String {
    let t: String = texto.chars().take(max).collect();
    if texto.chars().count() > max {
        format!("{t}…")
    } else {
        t
    }
}
