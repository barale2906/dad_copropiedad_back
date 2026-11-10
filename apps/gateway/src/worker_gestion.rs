//! Worker de gestión administrativa: cada [`INTERVALO`] cierra los tickets resueltos cuyo plazo
//! de reapertura pasó, alerta los vencidos, genera las órdenes preventivas de los planes, envía
//! los comunicados programados cuya hora llegó (a lo sumo 10 min tarde) y purga las sesiones del
//! chat y las claves `Idempotency-Key` de la API vencidas. Los avisos salen por el worker del outbox.

use std::{future::Future, time::Duration};

use application::{
    casos_uso::{
        comunicados::TxComunicados,
        mantenimiento::{TxMantenimiento, tareas_gestion},
    },
    puertos::{DirectorioGestion, RegistroIdempotencia, Reloj, RelojSistema, UnidadDeTrabajo},
};
use infra_postgres::{IdempotenciaPg, SesionesChatPg};

/// Cada cuánto se revisan las tareas.
pub const INTERVALO: Duration = Duration::from_secs(10 * 60);

/// Ejecuta el worker hasta que `apagado` se resuelva.
pub async fn ejecutar<D, U>(
    directorio: D,
    unidad: U,
    sesiones: SesionesChatPg,
    idempotencia: IdempotenciaPg,
    apagado: impl Future<Output = ()>,
) where
    D: DirectorioGestion,
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento + TxComunicados,
{
    tokio::pin!(apagado);
    tracing::info!("worker de gestión iniciado");
    loop {
        match tareas_gestion(&directorio, &unidad, &RelojSistema).await {
            Ok(r)
                if r.tickets_cerrados
                    + r.tickets_alertados
                    + r.ordenes_generadas
                    + r.comunicados_enviados
                    > 0 =>
            {
                tracing::info!(
                    copropiedades = r.copropiedades,
                    tickets_cerrados = r.tickets_cerrados,
                    tickets_alertados = r.tickets_alertados,
                    ordenes_generadas = r.ordenes_generadas,
                    comunicados_enviados = r.comunicados_enviados,
                    "tareas de gestión ejecutadas"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "no se pudieron revisar las tareas de gestión"),
        }
        if let Err(e) = sesiones.purgar(RelojSistema.ahora()).await {
            tracing::warn!(error = %e, "no se pudieron purgar las sesiones del chat");
        }
        if let Err(e) = idempotencia.purgar(RelojSistema.ahora()).await {
            tracing::warn!(error = %e, "no se pudieron purgar las claves de idempotencia");
        }
        tokio::select! {
            biased;
            () = &mut apagado => break,
            () = tokio::time::sleep(INTERVALO) => {}
        }
    }
    tracing::info!("worker de gestión detenido");
}
