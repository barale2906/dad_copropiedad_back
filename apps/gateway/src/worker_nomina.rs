//! Worker de avisos de la nómina: cada [`INTERVALO`] encola, a quienes gestionan la nómina de
//! cada entidad, los contratos a término fijo por vencer (preaviso de 30 días) y los recordatorios
//! de las fechas legales (intereses y consignación de cesantías, prima), una sola vez por
//! referencia. El envío lo hace el worker del outbox.

use std::{future::Future, sync::Arc, time::Duration};

use application::{
    casos_uso::nomina::tareas::{TxAvisosNomina, avisar_nomina},
    puertos::{RelojSistema, UnidadDeTrabajo, nomina::DirectorioNomina},
};
use tokio::sync::Notify;

/// Cada cuánto se buscan avisos pendientes.
pub const INTERVALO: Duration = Duration::from_secs(6 * 60 * 60);

/// Ejecuta el worker hasta que `apagado` se resuelva.
pub async fn ejecutar<D, U>(
    directorio: D,
    unidad: U,
    hay_salida: Arc<Notify>,
    apagado: impl Future<Output = ()>,
) where
    D: DirectorioNomina,
    U: UnidadDeTrabajo,
    U::Transaccion: TxAvisosNomina,
{
    tokio::pin!(apagado);
    tracing::info!("worker de avisos de nómina iniciado");
    loop {
        match avisar_nomina(&directorio, &unidad, &RelojSistema).await {
            Ok(0) => {}
            Ok(n) => {
                tracing::info!(avisos = n, "avisos de nómina encolados");
                hay_salida.notify_one();
            }
            Err(e) => tracing::error!(error = %e, "no se pudieron buscar avisos de nómina"),
        }
        tokio::select! {
            biased;
            () = &mut apagado => break,
            () = tokio::time::sleep(INTERVALO) => {}
        }
    }
    tracing::info!("worker de avisos de nómina detenido");
}
