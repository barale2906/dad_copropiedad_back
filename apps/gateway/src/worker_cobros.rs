//! Worker del cobro de la plataforma (plan 19, ADR 0024): cada [`INTERVALO`] calcula los cortes
//! de los periodos ya cerrados de las suscripciones por meses (los iguales al anterior quedan
//! aprobados solos) y sigue el calendario de mora de las facturas sin pagar: recordatorio,
//! avisos de vencida y suspensión en solo lectura a los 30 días (nunca con una asamblea
//! instalada). Facturar y registrar pagos siguen siendo manuales en la consola.

use std::{future::Future, sync::Arc, time::Duration};

use application::{
    casos_uso::cobros::{
        TxCobros,
        tareas::{generar_cortes, procesar_mora},
    },
    puertos::{
        RegistroAuditoria, RelojSistema, RepositorioCoeficientes, RepositorioEntidad,
        UnidadDeTrabajo, UnidadDeTrabajoPlataforma,
    },
};
use tokio::sync::Notify;

/// Cada cuánto se corre una ronda.
pub const INTERVALO: Duration = Duration::from_secs(6 * 60 * 60);

/// Ejecuta el worker hasta que `apagado` se resuelva.
pub async fn ejecutar<UP, U>(
    plataforma: UP,
    unidad: U,
    hay_salida: Arc<Notify>,
    apagado: impl Future<Output = ()>,
) where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioEntidad + RepositorioCoeficientes + RegistroAuditoria,
{
    tokio::pin!(apagado);
    tracing::info!("worker de cobros de la plataforma iniciado");
    loop {
        match generar_cortes(&plataforma, &RelojSistema).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(cortes = n, "cortes de cobro calculados"),
            Err(e) => tracing::error!(error = %e, "no se pudieron calcular los cortes de cobro"),
        }
        match procesar_mora(&plataforma, &unidad, &RelojSistema).await {
            Ok(r) => {
                if r.avisos > 0 || r.suspendidas > 0 {
                    tracing::info!(
                        avisos = r.avisos,
                        suspendidas = r.suspendidas,
                        "ronda de mora de la plataforma"
                    );
                }
                if r.avisos > 0 {
                    hay_salida.notify_one();
                }
            }
            Err(e) => tracing::error!(error = %e, "no se pudo procesar la mora de la plataforma"),
        }
        tokio::select! {
            biased;
            () = &mut apagado => break,
            () = tokio::time::sleep(INTERVALO) => {}
        }
    }
    tracing::info!("worker de cobros de la plataforma detenido");
}
