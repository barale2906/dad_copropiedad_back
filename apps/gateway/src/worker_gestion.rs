//! Worker de gestión administrativa: cada [`INTERVALO`] cierra los tickets resueltos cuyo plazo
//! de reapertura pasó, alerta los vencidos, genera las órdenes preventivas de los planes y
//! purga las sesiones del chat vencidas. Los avisos salen por el worker del outbox.

use std::{future::Future, time::Duration};

use application::{
    casos_uso::mantenimiento::{TxMantenimiento, tareas_gestion},
    puertos::{DirectorioGestion, Reloj, RelojSistema, UnidadDeTrabajo},
};
use infra_postgres::SesionesChatPg;

/// Cada cuánto se revisan las tareas.
pub const INTERVALO: Duration = Duration::from_secs(10 * 60);

/// Ejecuta el worker hasta que `apagado` se resuelva.
pub async fn ejecutar<D, U>(
    directorio: D,
    unidad: U,
    sesiones: SesionesChatPg,
    apagado: impl Future<Output = ()>,
) where
    D: DirectorioGestion,
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    tokio::pin!(apagado);
    tracing::info!("worker de gestión iniciado");
    loop {
        match tareas_gestion(&directorio, &unidad, &RelojSistema).await {
            Ok(r) if r.tickets_cerrados + r.tickets_alertados + r.ordenes_generadas > 0 => {
                tracing::info!(
                    copropiedades = r.copropiedades,
                    tickets_cerrados = r.tickets_cerrados,
                    tickets_alertados = r.tickets_alertados,
                    ordenes_generadas = r.ordenes_generadas,
                    "tareas de gestión ejecutadas"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "no se pudieron revisar las tareas de gestión"),
        }
        if let Err(e) = sesiones.purgar(RelojSistema.ahora()).await {
            tracing::warn!(error = %e, "no se pudieron purgar las sesiones del chat");
        }
        tokio::select! {
            biased;
            () = &mut apagado => break,
            () = tokio::time::sleep(INTERVALO) => {}
        }
    }
    tracing::info!("worker de gestión detenido");
}
