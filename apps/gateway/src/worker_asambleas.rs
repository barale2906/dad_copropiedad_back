//! Worker de asambleas: cada [`INTERVALO`] cierra las preguntas cuyo temporizador venció y las
//! consultas por escrito con el plazo cumplido (art. 43). Las copropiedades con algo pendiente
//! se obtienen con una función `SECURITY DEFINER` que solo devuelve ids de tenant.

use std::{future::Future, sync::Arc, time::Duration};

use application::{
    Contexto,
    casos_uso::asambleas::{TxAsambleas, votaciones::cerrar_vencidas},
    puertos::{DirectorioAsambleas, RelojSistema, UnidadDeTrabajo},
};
use tokio::sync::Notify;

/// Cada cuánto se revisan los temporizadores (las preguntas suelen durar minutos).
pub const INTERVALO: Duration = Duration::from_secs(5);

/// Ejecuta el worker hasta que `apagado` se resuelva.
pub async fn ejecutar<D, U>(
    directorio: D,
    unidad: U,
    hay_salida: Arc<Notify>,
    apagado: impl Future<Output = ()>,
) where
    D: DirectorioAsambleas,
    U: UnidadDeTrabajo,
    U::Transaccion: TxAsambleas,
{
    tokio::pin!(apagado);
    tracing::info!("worker de asambleas iniciado");
    loop {
        match directorio.copropiedades_con_asambleas_pendientes().await {
            Ok(tenants) => {
                for tenant in tenants {
                    match cerrar_vencidas(&unidad, &RelojSistema, &Contexto::new(tenant)).await {
                        Ok(n) => {
                            if n > 0 {
                                tracing::info!(tenant_id = %tenant, preguntas = n, "preguntas cerradas por temporizador");
                            }
                            hay_salida.notify_one();
                        }
                        Err(e) => {
                            tracing::error!(tenant_id = %tenant, error = %e, "no se pudieron cerrar las preguntas vencidas");
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "no se pudieron consultar las asambleas pendientes")
            }
        }
        tokio::select! {
            biased;
            () = &mut apagado => break,
            () = tokio::time::sleep(INTERVALO) => {}
        }
    }
    tracing::info!("worker de asambleas detenido");
}
