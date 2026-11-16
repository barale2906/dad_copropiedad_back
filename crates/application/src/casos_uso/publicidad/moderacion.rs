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
        cofre::Cofre,
        publicidad::{Campana, Creativo, CreativoReportado, PantallaPublicidad},
    },
    seguridad::token_aleatorio,
};

pub(crate) const ACCION_CREATIVO: AccionAuditoria =
    AccionAuditoria::new("publicidad.creativo.moderar");
pub(crate) const ACCION_CAMPANA: AccionAuditoria =
    AccionAuditoria::new("publicidad.campana.moderar");
pub(crate) const ACCION_REPORTES: AccionAuditoria =
    AccionAuditoria::new("publicidad.reporte.atender");
pub(crate) const ACCION_PANTALLA: AccionAuditoria =
    AccionAuditoria::new("publicidad.pantalla.generar");

/// Cuatro ojos (plan 20): quien cargó o editó lo que se modera no lo modera.
fn otra_cuenta(ctx: &ContextoPlataforma, cargado: &[Option<domain::CuentaId>]) -> AppResult<()> {
    match ctx.actor_id {
        Some(actor) if cargado.contains(&Some(actor)) => Err(AppError::Conflicto(
            "otra cuenta debe moderar lo que usted cargó (cuatro ojos)".into(),
        )),
        _ => Ok(()),
    }
}

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

/// Aprueba o rechaza un creativo pendiente. Lo modera una cuenta distinta de la que lo cargó.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si no está
/// pendiente o lo cargó quien modera; [`AppError::Validacion`] si se rechaza sin motivo.
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
    otra_cuenta(ctx, &[c.cargado_por])?;
    let (hacia, m) = match &decision {
        Decision::Aprobar => (EstadoCreativo::Aprobado, None),
        Decision::Rechazar { motivo: m } => (EstadoCreativo::Rechazado, Some(motivo(m)?)),
    };
    c.estado = c.estado.transicionar(hacia)?;
    c.motivo_estado = m;
    c.revisado_por = ctx.actor_id;
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
/// El conflicto de interés se vuelve a calcular. La modera una cuenta distinta de la que la creó
/// o la editó por última vez.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si no está en
/// revisión; [`AppError::Conflicto`] si la cargó quien modera; [`AppError::Validacion`] si
/// falta algo para aprobarla o se rechaza sin motivo.
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
    otra_cuenta(ctx, &[c.creada_por, c.editada_por])?;
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

fn contexto_pantalla(tenant: TenantId) -> String {
    format!("pantalla:{tenant}")
}

/// Contexto de la plataforma en la PH solo para su pantalla, sin permisos de copropiedad.
fn en_pantalla(ctx: &ContextoPlataforma, tenant: TenantId) -> Contexto {
    Contexto {
        tenant_id: tenant,
        actor_id: ctx.actor_id,
        request_id: ctx.request_id.clone(),
        permisos: ConjuntoPermisos::vacio(),
    }
}

/// Genera el token de la pantalla del lobby de una copropiedad (reemplaza el anterior, que deja
/// de servir). Lo hace la plataforma, que instala la pantalla; se guarda también cifrado para
/// volver a consultarlo con [`token_pantalla`].
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.modulos.gestionar`; [`AppError::Validacion`] si el
/// servidor no tiene clave del Cofre.
pub async fn generar_pantalla<U, C>(
    unidad: &U,
    cofre: &C,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    tenant: TenantId,
) -> AppResult<String>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
    C: Cofre,
{
    ctx.exigir(Permiso::PlataformaModulosGestionar)?;
    let token = token_aleatorio()?;
    let cifrado = cofre.cifrar(&contexto_pantalla(tenant), token.as_bytes())?;
    let mut tx = unidad.iniciar(&en_pantalla(ctx, tenant)).await?;
    tx.guardar_pantalla(&PantallaPublicidad {
        token_hash: huella(&token)?,
        token_cifrado: Some(cifrado),
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

/// El token vigente de la pantalla, sin cambiarlo; `None` si no tiene o es anterior al plan 20
/// (no se guardó cifrado: genere otro).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.modulos.gestionar`; los del Cofre.
pub async fn token_pantalla<U, C>(
    unidad: &U,
    cofre: &C,
    ctx: &ContextoPlataforma,
    tenant: TenantId,
) -> AppResult<Option<String>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
    C: Cofre,
{
    ctx.exigir(Permiso::PlataformaModulosGestionar)?;
    let mut tx = unidad.iniciar(&en_pantalla(ctx, tenant)).await?;
    tx.pantalla()
        .await?
        .and_then(|p| p.token_cifrado)
        .map(|c| {
            let claro = cofre.descifrar(&contexto_pantalla(tenant), &c)?;
            String::from_utf8(claro).map_err(AppError::infra)
        })
        .transpose()
}
