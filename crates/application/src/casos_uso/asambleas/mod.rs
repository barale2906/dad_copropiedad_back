//! Asambleas y votaciones (plan 06, módulo vendible `asambleas`, ADR 0010).
//!
//! Submódulos:
//! - [`reuniones`]: crear, configurar, orden del día, documentos, convocar (padrón y envío
//!   multicanal con constancia), abrir el registro, instalar con quórum, suspender, reanudar,
//!   cerrar, cancelar y segunda convocatoria (art. 41).
//! - [`registro`]: mesa de registro asistido (bienes propios y poderes en un toque), rótulos,
//!   salidas con motivo y reingresos, morosos, prerregistro y papeleta virtual.
//! - [`quorum`]: quórum en vivo por coeficiente y por unidades, fotos para el acta y el
//!   tablero de las pantallas en vivo.
//! - [`votaciones`]: preguntas (sí/no, selección, elección por cociente), abrir, cerrar
//!   (también por temporizador), emitir el voto del asistente por **todos** sus bienes desde
//!   cualquier canal, resultados y detalle nominal o reservado.
//! - [`whatsapp`]: el voto y la confirmación de asistencia por WhatsApp (gateway).
//! - [`acta`]: acta en PDF, firmas escaneadas, aprobación, publicación y libro de actas.
//! - [`reportes`]: asistencia, votos, participación por canal y costo de WhatsApp.
//! - [`lineas`]: líneas de WhatsApp y tarifas (plataforma).
//!
//! Invariantes (AGENTS.md §9):
//! 1. Un bien privado vota una vez por pregunta; el asistente vota una vez por todos sus bienes.
//! 2. Votar es idempotente por (pregunta, asistente, clave): un reintento devuelve el original.
//! 3. Verificar estado + participaciones + votos + outbox + auditoría en una sola transacción.
//! 4. La auditoría y las confirmaciones nunca llevan la opción elegida.
//! 5. Quórum y resultados suman lo copiado en las filas (coeficiente del padrón).
//! 6. Solo preguntas `abierta` de reuniones `instalada` reciben votos, y solo de bienes
//!    habilitados al abrirlas (o que ingresaron mientras estaban abiertas).
//! 7. Una pregunta lleva su id en cada botón de WhatsApp: dos asambleas simultáneas nunca
//!    mezclan votos.

pub mod acta;
pub mod lineas;
pub mod quorum;
pub mod registro;
pub mod reportes;
pub mod reuniones;
pub mod votaciones;
pub mod whatsapp;

#[cfg(test)]
mod tests;

use domain::{Modulo, ReunionId, reserva::Tz};

use crate::{
    AppError, AppResult, Contexto,
    auditoria::AccionAuditoria,
    casos_uso::{modulos::exigir_modulo_en, reservas::zona},
    puertos::{
        RegistroAuditoria, RegistroOutbox, Reloj, RepositorioAsambleas, RepositorioCopropiedad,
        RepositorioModulos, RepositorioPersonas, RepositorioRoles, UnidadDeTrabajo,
        asambleas::Reunion,
    },
};

pub const ACCION_CREAR: AccionAuditoria = AccionAuditoria::new("asambleas.reunion.crear");
pub const ACCION_EDITAR: AccionAuditoria = AccionAuditoria::new("asambleas.reunion.editar");
pub const ACCION_TRANSICIONAR: AccionAuditoria =
    AccionAuditoria::new("asambleas.reunion.transicionar");
pub const ACCION_CONVOCAR: AccionAuditoria = AccionAuditoria::new("asambleas.reunion.convocar");
pub const ACCION_MOROSOS: AccionAuditoria = AccionAuditoria::new("asambleas.morosos.fijar");
pub const ACCION_REGISTRAR: AccionAuditoria = AccionAuditoria::new("asambleas.asistente.registrar");
pub const ACCION_SALIDA: AccionAuditoria = AccionAuditoria::new("asambleas.asistente.salir");
pub const ACCION_REINGRESO: AccionAuditoria =
    AccionAuditoria::new("asambleas.asistente.reingresar");
pub const ACCION_PODER: AccionAuditoria = AccionAuditoria::new("asambleas.poder.registrar");
pub const ACCION_REVISAR_PODER: AccionAuditoria = AccionAuditoria::new("asambleas.poder.revisar");
pub const ACCION_ROTULO: AccionAuditoria = AccionAuditoria::new("asambleas.rotulo.asignar");
pub const ACCION_PREGUNTA: AccionAuditoria = AccionAuditoria::new("asambleas.pregunta.editar");
pub const ACCION_ABRIR: AccionAuditoria = AccionAuditoria::new("asambleas.pregunta.abrir");
pub const ACCION_CERRAR: AccionAuditoria = AccionAuditoria::new("asambleas.pregunta.cerrar");
pub const ACCION_ANULAR: AccionAuditoria = AccionAuditoria::new("asambleas.pregunta.anular");
pub const ACCION_VOTAR: AccionAuditoria = AccionAuditoria::new("asambleas.participacion.registrar");
pub const ACCION_VER_DETALLE: AccionAuditoria = AccionAuditoria::new("asambleas.detalle.consultar");
pub const ACCION_ROLES: AccionAuditoria = AccionAuditoria::new("asambleas.eleccion.aplicar");
pub const ACCION_ACTA: AccionAuditoria = AccionAuditoria::new("asambleas.acta.editar");
pub const ACCION_PUNTO: AccionAuditoria = AccionAuditoria::new("asambleas.punto.editar");
pub const ACCION_LINEA: AccionAuditoria = AccionAuditoria::new("plataforma.linea.editar");

/// Puertos que usan los casos de uso de asambleas dentro de la transacción de la copropiedad.
pub trait TxAsambleas:
    RepositorioAsambleas
    + RepositorioModulos
    + RepositorioCopropiedad
    + RepositorioPersonas
    + RepositorioRoles
    + RegistroOutbox
    + RegistroAuditoria
{
}

impl<T> TxAsambleas for T where
    T: RepositorioAsambleas
        + RepositorioModulos
        + RepositorioCopropiedad
        + RepositorioPersonas
        + RepositorioRoles
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
    U::Transaccion: TxAsambleas,
{
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::Asambleas).await?;
    Ok(tx)
}

/// La reunión (sin bloqueo) o `NoEncontrado`.
pub(crate) async fn reunion_de<T: TxAsambleas>(tx: &mut T, id: ReunionId) -> AppResult<Reunion> {
    tx.reunion(id)
        .await?
        .ok_or(AppError::NoEncontrado("reunión"))
}

/// La reunión bloqueada para la transacción (transiciones) o `NoEncontrado`.
pub(crate) async fn reunion_bloqueada<T: TxAsambleas>(
    tx: &mut T,
    id: ReunionId,
) -> AppResult<Reunion> {
    tx.bloquear_reunion(id)
        .await?
        .ok_or(AppError::NoEncontrado("reunión"))
}

/// Zona horaria de la copropiedad (los plazos y el acta van en hora local).
pub(crate) async fn zona_de<T: TxAsambleas>(tx: &mut T) -> AppResult<Tz> {
    zona(tx).await
}

/// Recorta un texto para WhatsApp (títulos de botones y filas).
pub(crate) fn corto(t: &str, max: usize) -> String {
    if t.chars().count() <= max {
        t.to_owned()
    } else {
        let mut s: String = t.chars().take(max.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

/// Texto obligatorio recortado y con longitud máxima.
pub(crate) fn texto(campo: &'static str, valor: &str, max: usize) -> AppResult<String> {
    domain::copropiedad::texto_obligatorio(campo, valor, max).map_err(AppError::from)
}

/// Texto opcional recortado (vacío = `None`).
pub(crate) fn texto_opcional(
    campo: &'static str,
    valor: Option<&str>,
    max: usize,
) -> AppResult<Option<String>> {
    domain::copropiedad::texto_opcional(campo, valor, max).map_err(AppError::from)
}
