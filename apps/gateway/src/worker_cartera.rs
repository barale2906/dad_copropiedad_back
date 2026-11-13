//! Worker de avisos de la cartera de la PH: cada [`INTERVALO`] encola el recordatorio de pago
//! (antes del vencimiento) y el aviso de mora (después) de las cuentas de cobro de todas las
//! copropiedades con la cartera contratada, una sola vez por cuenta y clase. El envío lo hace el
//! worker del outbox. La causación de intereses y la restricción de reservas no son automáticas.

use std::{future::Future, sync::Arc, time::Duration};

use application::{
    casos_uso::contabilidad_ph::{TxCartera, avisos::avisar_cartera},
    puertos::{RelojSistema, UnidadDeTrabajo, cartera::DirectorioCartera},
};
use tokio::sync::Notify;

/// Cada cuánto se buscan avisos pendientes.
pub const INTERVALO: Duration = Duration::from_secs(60 * 60);

/// Ejecuta el worker hasta que `apagado` se resuelva.
pub async fn ejecutar<D, U>(
    directorio: D,
    unidad: U,
    hay_salida: Arc<Notify>,
    apagado: impl Future<Output = ()>,
) where
    D: DirectorioCartera,
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    tokio::pin!(apagado);
    tracing::info!("worker de avisos de cartera iniciado");
    loop {
        match avisar_cartera(&directorio, &unidad, &RelojSistema).await {
            Ok(0) => {}
            Ok(n) => {
                tracing::info!(avisos = n, "avisos de cartera encolados");
                hay_salida.notify_one();
            }
            Err(e) => tracing::error!(error = %e, "no se pudieron buscar avisos de cartera"),
        }
        tokio::select! {
            biased;
            () = &mut apagado => break,
            () = tokio::time::sleep(INTERVALO) => {}
        }
    }
    tracing::info!("worker de avisos de cartera detenido");
}
