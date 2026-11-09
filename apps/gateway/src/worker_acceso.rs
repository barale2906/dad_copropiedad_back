//! Worker de control de acceso (plan 05):
//!
//! - Cada [`INTERVALO_ESCALAMIENTO`]: segundo aviso de visitas a los 2 min, llamada a los 5 min,
//!   vencimiento de autorizaciones y avisos de correspondencia cada 24 h
//!   (`casos_uso::acceso::tareas::escalar_avisos`). Si encoló avisos, despierta al worker del
//!   outbox.
//! - Cada [`INTERVALO_RETENCION`]: borra firmas y fotos vencidas, fotos de placas y eventos
//!   viejos; prepara las particiones de los próximos meses.

use std::{future::Future, sync::Arc, time::Duration};

use application::{
    casos_uso::acceso::{
        TxAcceso,
        tareas::{aplicar_retencion, escalar_avisos},
    },
    puertos::{AlmacenArchivos, DirectorioAcceso, RelojSistema, UnidadDeTrabajo},
};
use tokio::{sync::Notify, time::Instant};

/// Las visitas escalan a los 2 y 5 minutos: se revisa con margen de segundos.
pub const INTERVALO_ESCALAMIENTO: Duration = Duration::from_secs(15);
/// La retención corre una vez al día.
pub const INTERVALO_RETENCION: Duration = Duration::from_secs(24 * 60 * 60);

/// Ejecuta el worker hasta que `apagado` se resuelva.
pub async fn ejecutar<D, U, A>(
    directorio: D,
    unidad: U,
    almacen: Arc<A>,
    hay_salida: Arc<Notify>,
    apagado: impl Future<Output = ()>,
) where
    D: DirectorioAcceso,
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
    A: AlmacenArchivos,
{
    tokio::pin!(apagado);
    tracing::info!("worker de control de acceso iniciado");
    let mut proxima_retencion = Instant::now();
    loop {
        match escalar_avisos(&directorio, &unidad, &RelojSistema).await {
            Ok(r) if r.hubo_trabajo() => {
                tracing::info!(
                    copropiedades = r.copropiedades,
                    segundos_avisos = r.segundos_avisos,
                    llamadas = r.llamadas,
                    vencidas = r.vencidas,
                    avisos_correspondencia = r.avisos_correspondencia,
                    alertas_administracion = r.alertas_administracion,
                    "avisos de acceso escalados"
                );
                hay_salida.notify_one();
            }
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "no se pudieron escalar los avisos de acceso"),
        }
        if Instant::now() >= proxima_retencion {
            match aplicar_retencion(&directorio, &unidad, almacen.as_ref(), &RelojSistema).await {
                Ok(r) => tracing::info!(
                    soportes_purgados = r.soportes_purgados,
                    fotos_purgadas = r.fotos_purgadas,
                    eventos_borrados = r.eventos_borrados,
                    "retención de control de acceso aplicada"
                ),
                Err(e) => tracing::error!(error = %e, "no se pudo aplicar la retención de acceso"),
            }
            proxima_retencion = Instant::now() + INTERVALO_RETENCION;
        }
        tokio::select! {
            biased;
            () = &mut apagado => break,
            () = tokio::time::sleep(INTERVALO_ESCALAMIENTO) => {}
        }
    }
    tracing::info!("worker de control de acceso detenido");
}
