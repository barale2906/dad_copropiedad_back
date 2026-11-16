//! Tareas periódicas del cobro (las ejecuta el worker del gateway, plan 19):
//!
//! - [`generar_cortes`]: calcula el corte de cada periodo ya cerrado de las suscripciones por
//!   meses (idempotente: un periodo con corte no se recalcula si ya se aprobó).
//! - [`procesar_mora`]: sigue el calendario de cada factura sin pagar
//!   (`domain::cobro_plataforma::accion_de_mora`): recordatorio, avisos de vencida y, a los 30
//!   días, suspensión de la entidad en solo lectura — **nunca** con una asamblea instalada.

use domain::{
    EstadoCopropiedad,
    cobro_plataforma::{AccionMora, Ciclo, EstadoSuscripcion, accion_de_mora},
};

use super::{
    TxCobros,
    cortes::{aviso, calcular_en, cambiar_estado_entidad, hoy},
};
use crate::puertos::{RegistroOutbox as _, RepositorioCobros as _};
use crate::{
    AppResult, Contexto, ContextoPlataforma,
    auditoria::AccionAuditoria,
    puertos::{
        RegistroAuditoria, Reloj, RepositorioCoeficientes, RepositorioEntidad, Transaccion,
        UnidadDeTrabajo, UnidadDeTrabajoPlataforma, cobros::OrigenCorte,
    },
};

pub(crate) const ACCION_SUSPENDER: AccionAuditoria =
    AccionAuditoria::new("plataforma.entidad.suspender_por_mora");

/// Periodos por suscripción que el worker calcula en una ronda (los demás, en la siguiente).
const MAXIMO_PERIODOS: usize = 12;

/// Calcula los cortes de los periodos ya cerrados. Devuelve cuántos calculó.
///
/// # Errors
///
/// [`crate::AppError::Infraestructura`] si falla la base de datos. Un corte que no se puede
/// calcular (p. ej. un producto sin tarifa) se salta y queda en los logs.
pub async fn generar_cortes<UP>(unidad: &UP, reloj: &impl Reloj) -> AppResult<u32>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    let ctx = ContextoPlataforma::sistema();
    let hoy = hoy(reloj);
    let suscripciones = unidad
        .iniciar_plataforma(&ctx)
        .await?
        .suscripciones_vigentes()
        .await?;
    let mut n = 0;
    for s in suscripciones {
        let Ciclo::Meses(_) = s.ciclo else { continue };
        let mut tx = unidad.iniciar_plataforma(&ctx).await?;
        let mut desde = match tx.ultimo_corte(s.id).await? {
            Some(c) if c.estado == domain::cobro_plataforma::EstadoCorte::Borrador => {
                c.periodo.desde
            }
            Some(c) => c.periodo.hasta,
            None => s.cobra_desde(),
        };
        for _ in 0..MAXIMO_PERIODOS {
            let Some(periodo) = s.ciclo.periodo_desde(desde) else {
                break;
            };
            if periodo.hasta > hoy {
                break;
            }
            match calcular_en(&mut tx, reloj, &ctx, &s, periodo).await {
                Ok(_) => n += 1,
                Err(e) => {
                    tracing::warn!(suscripcion_id = %s.id, error = %e, "no se pudo calcular el corte");
                    break;
                }
            }
            desde = periodo.hasta;
        }
        tx.confirmar().await?;
    }
    Ok(n)
}

/// Resultado de una ronda de mora.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RondaMora {
    pub avisos: u32,
    pub suspendidas: u32,
}

/// Recorre las facturas sin pagar de las suscripciones y aplica el calendario de mora.
///
/// # Errors
///
/// [`crate::AppError::Infraestructura`] si falla la base de datos.
pub async fn procesar_mora<UP, U>(
    plataforma: &UP,
    unidad: &U,
    reloj: &impl Reloj,
) -> AppResult<RondaMora>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioEntidad + RepositorioCoeficientes + RegistroAuditoria,
{
    let ctx = ContextoPlataforma::sistema();
    let hoy = hoy(reloj);
    let mut ronda = RondaMora::default();
    let cortes = plataforma
        .iniciar_plataforma(&ctx)
        .await?
        .cortes_por_cobrar()
        .await?;
    for corte in cortes {
        let (OrigenCorte::Suscripcion(sid), Some(vencimiento)) = (corte.origen, corte.vencimiento)
        else {
            continue;
        };
        let mut tx = plataforma.iniciar_plataforma(&ctx).await?;
        let Some(mut s) = tx.suscripcion(sid).await? else {
            continue;
        };
        match accion_de_mora(
            hoy,
            vencimiento,
            corte.ultimo_aviso_en,
            s.suspendida_por_mora,
        ) {
            AccionMora::Esperar => {}
            accion @ (AccionMora::Recordar | AccionMora::AvisarVencida) => {
                let (asunto, texto) = if accion == AccionMora::Recordar {
                    (
                        "Recordatorio de pago de DAD Copropiedad",
                        format!(
                            "Su factura de los servicios de la plataforma ({}) por ${} vence el \
                             {vencimiento}.",
                            corte.periodo.texto(),
                            corte.total
                        ),
                    )
                } else {
                    (
                        "Factura vencida de DAD Copropiedad",
                        format!(
                            "Su factura de los servicios de la plataforma ({}) por ${} venció el \
                             {vencimiento}. A los 30 días de vencida la cuenta pasa a solo \
                             lectura hasta que se pague.",
                            corte.periodo.texto(),
                            corte.total
                        ),
                    )
                };
                tx.encolar(aviso(&s.correo_facturacion, asunto, texto))
                    .await?;
                let mut c = corte.clone();
                c.ultimo_aviso_en = Some(hoy);
                tx.actualizar_corte(&c).await?;
                ronda.avisos += 1;
            }
            AccionMora::Suspender => {
                let ctx_e = Contexto::new(s.tenant_id);
                let instaladas = unidad.iniciar(&ctx_e).await?.reuniones_instaladas().await?;
                if instaladas > 0 {
                    // Nunca en plena asamblea: se intenta en la siguiente ronda.
                    continue;
                }
                if let Err(e) = cambiar_estado_entidad(
                    unidad,
                    &ctx,
                    s.tenant_id,
                    EstadoCopropiedad::Suspendida,
                    ACCION_SUSPENDER,
                )
                .await
                {
                    // Una entidad aún en configuración o retirada no se suspende.
                    tracing::warn!(tenant_id = %s.tenant_id, error = %e, "no se suspendió por mora");
                    continue;
                }
                s.suspendida_por_mora = true;
                s.estado = EstadoSuscripcion::Suspendida;
                tx.actualizar_suscripcion(&s).await?;
                ronda.suspendidas += 1;
            }
        }
        tx.confirmar().await?;
    }
    Ok(ronda)
}
