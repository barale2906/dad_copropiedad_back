//! Lo que ve una entidad de su relación con la plataforma (plan 19): el plan contratado, el
//! consumo del periodo en curso y sus cortes facturados. Exige `suscripcion.ver` en la entidad;
//! nunca muestra cortes en borrador ni descartados.

use domain::{
    Permiso,
    cobro_plataforma::{EstadoCorte, Metrica},
};

use super::{DetalleSuscripcion, TxCobros, cortes::hoy, detalle_en};
use crate::puertos::RepositorioCobros as _;
use crate::{
    AppResult, Contexto, ContextoPlataforma, SolicitudPagina,
    puertos::{
        Reloj, UnidadDeTrabajoPlataforma,
        cobros::{Corte, FiltroCortes},
    },
};

/// Plan, consumo y facturas de la entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiSuscripcion {
    pub detalle: DetalleSuscripcion,
    /// Consumo del periodo en curso por métrica (lo que se cobra por unidad, paquete o consumo).
    pub consumo: Vec<(Metrica, u64)>,
    /// Cortes aprobados, facturados o pagados, los más recientes primero.
    pub cortes: Vec<Corte>,
}

/// La suscripción de la entidad de `ctx`; `None` si no tiene.
///
/// # Errors
///
/// [`crate::AppError::SinPermiso`] sin `suscripcion.ver`.
pub async fn mi_suscripcion<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Option<MiSuscripcion>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::SuscripcionVer)?;
    // Lectura de las tablas de plataforma solo para la entidad de la sesión.
    let ctx_p = ContextoPlataforma {
        actor_id: ctx.actor_id,
        request_id: ctx.request_id.clone(),
        permisos: domain::ConjuntoPermisos::vacio(),
    };
    let mut tx = unidad.iniciar_plataforma(&ctx_p).await?;
    let Some(s) = tx.suscripcion_de_entidad(ctx.tenant_id).await? else {
        return Ok(None);
    };
    let periodo = match s.ciclo.periodo_desde(s.cobra_desde()) {
        Some(primero) => {
            let mut p = primero;
            let hoy = hoy(reloj);
            while p.hasta <= hoy {
                match s.ciclo.periodo_desde(p.hasta) {
                    Some(x) => p = x,
                    None => break,
                }
            }
            Some(p)
        }
        None => None,
    };
    let mut detalle = detalle_en(&mut tx, reloj, s.clone()).await?;
    let mut consumo = Vec::new();
    if let Some(p) = periodo {
        let metricas: std::collections::BTreeSet<Metrica> = {
            let mut m = std::collections::BTreeSet::new();
            for (l, _) in &detalle.lineas {
                if let Some(t) = tx.tarifas(l.producto_id).await?.into_iter().next()
                    && let Some(x) = t.tarifa.metrica
                {
                    m.insert(x);
                }
            }
            m
        };
        for m in metricas {
            let n = tx.medir(s.tenant_id, &m, &p).await?;
            consumo.push((m, n));
        }
    }
    let cortes = tx
        .cortes(
            &FiltroCortes {
                estado: None,
                suscripcion: Some(s.id),
            },
            SolicitudPagina::new(Some(24), None)?,
        )
        .await?
        .datos
        .into_iter()
        .filter(|c| !matches!(c.estado, EstadoCorte::Borrador | EstadoCorte::Descartado))
        .collect();
    // El pagador y su correo son datos comerciales de la plataforma: se muestran tal cual.
    detalle.modulos_sin_cobro.clear();
    Ok(Some(MiSuscripcion {
        detalle,
        consumo,
        cortes,
    }))
}
