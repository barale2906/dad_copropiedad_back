//! Moderación de la publicidad (`plataforma.publicidad.moderar`, plan 08, fase 3): la
//! verificación del anunciante es la de [`crate::casos_uso::directorio::cambiar_verificacion`];
//! aquí, los creativos (uno por uno), las campañas (**todas** pasan por aquí; las de conflicto
//! de interés llegan marcadas), los reportes de los residentes y las pantallas del lobby.

use domain::{
    CampanaId, ConjuntoPermisos, CreativoId, EstadoCampana, EstadoCreativo, Permiso, TenantId,
};

use super::{TxPublicidad, TxPublicidadPlataforma, motivo};
use crate::puertos::{RepositorioDirectorioPlataforma, RepositorioPublicidadPlataforma};
use crate::{
    AppError, AppResult, Contexto, ContextoPlataforma, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::busqueda_proveedores::huella,
    puertos::{
        RegistroAuditoria, Reloj, RepositorioPublicidad, Transaccion, UnidadDeTrabajo,
        UnidadDeTrabajoPlataforma,
        publicidad::{Campana, Creativo, CreativoReportado, PantallaPublicidad},
    },
    seguridad::token_aleatorio,
};

pub const ACCION_CREATIVO: AccionAuditoria = AccionAuditoria::new("publicidad.creativo.moderar");
pub const ACCION_CAMPANA: AccionAuditoria = AccionAuditoria::new("publicidad.campana.moderar");
pub const ACCION_REPORTES: AccionAuditoria = AccionAuditoria::new("publicidad.reporte.atender");
pub const ACCION_PANTALLA: AccionAuditoria = AccionAuditoria::new("publicidad.pantalla.generar");

/// Decisión de la moderación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Aprobar,
    Rechazar { motivo: String },
}

/// Creativos pendientes de moderación.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.publicidad.moderar`.
pub async fn creativos_pendientes<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Creativo>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    ctx.exigir(Permiso::PlataformaPublicidadModerar)?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .creativos_pendientes(pagina)
        .await
}

/// Campañas en revisión (las de conflicto de interés vienen marcadas).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.publicidad.moderar`.
pub async fn campanas_en_revision<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Campana>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    ctx.exigir(Permiso::PlataformaPublicidadModerar)?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .campanas_en_revision(pagina)
        .await
}

/// Aprueba o rechaza un creativo pendiente.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si no está
/// pendiente; [`AppError::Validacion`] si se rechaza sin motivo.
pub async fn moderar_creativo<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: CreativoId,
    decision: Decision,
) -> AppResult<Creativo>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    ctx.exigir(Permiso::PlataformaPublicidadModerar)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut c = tx
        .creativo(id)
        .await?
        .ok_or(AppError::NoEncontrado("creativo"))?;
    if c.estado != EstadoCreativo::Pendiente {
        return Err(AppError::Conflicto("el creativo no está pendiente".into()));
    }
    let (hacia, m) = match &decision {
        Decision::Aprobar => (EstadoCreativo::Aprobado, None),
        Decision::Rechazar { motivo: m } => (EstadoCreativo::Rechazado, Some(motivo(m)?)),
    };
    c.estado = c.estado.transicionar(hacia)?;
    c.motivo_estado = m;
    c.actualizado_en = reloj.ahora();
    tx.actualizar_creativo(&c).await?;
    let detalle = DetalleAuditoria::vacio().con("hacia", hacia.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREATIVO, "creativo", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Aprueba o rechaza una campaña en revisión. Aprobar exige el anunciante verificado, todos
/// sus creativos aprobados y una tarifa vigente del modelo, que queda fijada en la campaña.
/// El conflicto de interés se vuelve a calcular.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si no está en
/// revisión; [`AppError::Validacion`] si falta algo para aprobarla o se rechaza sin motivo.
pub async fn moderar_campana<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: CampanaId,
    decision: Decision,
) -> AppResult<Campana>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    ctx.exigir(Permiso::PlataformaPublicidadModerar)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut c = tx
        .campana(id, true)
        .await?
        .ok_or(AppError::NoEncontrado("campaña"))?;
    c.conflicto_interes = tx.conflicto_interes(id).await?;
    match &decision {
        Decision::Aprobar => {
            c.estado = c.estado.transicionar(EstadoCampana::Aprobada)?;
            let anunciante = tx
                .anunciante(c.anunciante_id)
                .await?
                .ok_or(AppError::NoEncontrado("anunciante"))?;
            if !anunciante.estado.visible() {
                return Err(AppError::Validacion(
                    "verifique primero al anunciante".into(),
                ));
            }
            let creativos = tx.creativos_de_campana(id).await?;
            if creativos.is_empty()
                || creativos
                    .iter()
                    .any(|x| x.estado != EstadoCreativo::Aprobado)
            {
                return Err(AppError::Validacion(
                    "todos los creativos de la campaña deben estar aprobados".into(),
                ));
            }
            let tarifa = tx
                .tarifa_vigente(c.modelo, ahora.date_naive())
                .await?
                .ok_or_else(|| {
                    AppError::Validacion(format!(
                        "no hay tarifa vigente para el modelo {}",
                        c.modelo.as_str()
                    ))
                })?;
            c.tarifa = Some(tarifa.valor);
            c.motivo_estado = None;
        }
        Decision::Rechazar { motivo: m } => {
            c.estado = c.estado.transicionar(EstadoCampana::Rechazada)?;
            c.motivo_estado = Some(motivo(m)?);
        }
    }
    c.revisada_por = ctx.actor_id;
    c.revisada_en = Some(ahora);
    c.actualizada_en = ahora;
    tx.actualizar_campana(&c).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("hacia", c.estado.as_str())?
        .con("conflicto_interes", c.conflicto_interes)?;
    tx.registrar(EventoAuditoria::ok(ACCION_CAMPANA, "campana", id.as_uuid()).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Creativos con reportes de los residentes sin atender (de todas las copropiedades).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.publicidad.moderar`.
pub async fn reportes_pendientes<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<CreativoReportado>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    ctx.exigir(Permiso::PlataformaPublicidadModerar)?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .reportes_pendientes(pagina)
        .await
}

/// Atiende los reportes de un creativo: lo retira (`retirar` = motivo; deja de entregarse) o
/// lo mantiene. Devuelve cuántos reportes atendió.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`].
pub async fn atender_reportes<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: CreativoId,
    retirar: Option<&str>,
) -> AppResult<u64>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    ctx.exigir(Permiso::PlataformaPublicidadModerar)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut c = tx
        .creativo(id)
        .await?
        .ok_or(AppError::NoEncontrado("creativo"))?;
    if let Some(m) = retirar {
        let m = motivo(m)?;
        if c.estado != EstadoCreativo::Rechazado {
            c.estado = c.estado.transicionar(EstadoCreativo::Rechazado)?;
        }
        c.motivo_estado = Some(m);
        c.actualizado_en = ahora;
        tx.actualizar_creativo(&c).await?;
    }
    let n = tx.atender_reportes(id, ahora).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("retirado", retirar.is_some())?
        .con("reportes", n)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_REPORTES, "creativo", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(n)
}

/// Genera el token de la pantalla del lobby de una copropiedad (reemplaza el anterior). Lo
/// hace la plataforma, que instala la pantalla; se entrega una sola vez.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.modulos.gestionar`.
pub async fn generar_pantalla<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    tenant: TenantId,
) -> AppResult<String>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
{
    ctx.exigir(Permiso::PlataformaModulosGestionar)?;
    // La plataforma actúa en la PH solo sobre la pantalla, sin permisos de copropiedad.
    let ctx_ph = Contexto {
        tenant_id: tenant,
        actor_id: ctx.actor_id,
        request_id: ctx.request_id.clone(),
        permisos: ConjuntoPermisos::vacio(),
    };
    let token = token_aleatorio()?;
    let mut tx = unidad.iniciar(&ctx_ph).await?;
    tx.guardar_pantalla(&PantallaPublicidad {
        token_hash: huella(&token)?,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
    })
    .await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PANTALLA,
        "pantalla_publicidad",
        tenant.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(token)
}
