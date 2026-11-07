//! Worker de recordatorios: cada [`INTERVALO`] encola el recordatorio (24 h antes) de las
//! reservas aprobadas de todas las copropiedades. El envío lo hace el worker del outbox.

use std::{future::Future, time::Duration};

use application::{
    casos_uso::reservas::recordar_reservas,
    puertos::{
        DirectorioRecordatorios, RegistroOutbox, RelojSistema, RepositorioCopropiedad,
        RepositorioPersonas, RepositorioReservas, UnidadDeTrabajo,
    },
};

/// Cada cuánto se buscan recordatorios pendientes.
pub const INTERVALO: Duration = Duration::from_secs(5 * 60);

/// Ejecuta el worker hasta que `apagado` se resuelva.
pub async fn ejecutar<D, U>(directorio: D, unidad: U, apagado: impl Future<Output = ()>)
where
    D: DirectorioRecordatorios,
    U: UnidadDeTrabajo,
    U::Transaccion:
        RepositorioReservas + RepositorioCopropiedad + RepositorioPersonas + RegistroOutbox,
{
    tokio::pin!(apagado);
    tracing::info!("worker de recordatorios iniciado");
    loop {
        match recordar_reservas(&directorio, &unidad, &RelojSistema).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(recordatorios = n, "recordatorios de reservas encolados"),
            Err(e) => tracing::error!(error = %e, "no se pudieron buscar recordatorios"),
        }
        tokio::select! {
            biased;
            () = &mut apagado => break,
            () = tokio::time::sleep(INTERVALO) => {}
        }
    }
    tracing::info!("worker de recordatorios detenido");
}
