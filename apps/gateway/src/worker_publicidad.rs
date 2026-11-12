//! Worker de publicidad (plan 08, fases 4 y 5): cada [`INTERVALO`] finaliza las campañas cuya
//! vigencia terminó y envía los mensajes masivos de WhatsApp y correo de las campañas activas,
//! solo a quienes autorizaron la publicidad y con el tope de frecuencia. Los mensajes salen
//! por el worker del outbox con prioridad 9 (nunca adelantan votos, avisos ni comunicados).

use std::{future::Future, time::Duration};

use application::{
    casos_uso::publicidad::{
        TxPublicidad, TxPublicidadPlataforma, entrega::enviar_masivos,
        liquidacion::finalizar_vencidas,
    },
    puertos::{
        DirectorioPublicidad, Reloj, RelojSistema, UnidadDeTrabajo, UnidadDeTrabajoPlataforma,
    },
};

/// Cada cuánto se revisan las campañas.
pub const INTERVALO: Duration = Duration::from_secs(15 * 60);

/// Ejecuta el worker hasta que `apagado` se resuelva.
pub async fn ejecutar<D, U, UP>(
    directorio: D,
    unidad: U,
    plataforma: UP,
    apagado: impl Future<Output = ()>,
) where
    D: DirectorioPublicidad,
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    tokio::pin!(apagado);
    tracing::info!("worker de publicidad iniciado");
    loop {
        match finalizar_vencidas(&plataforma, RelojSistema.ahora()).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(finalizadas = n, "campañas de publicidad finalizadas"),
            Err(e) => tracing::warn!(error = %e, "no se pudieron finalizar las campañas"),
        }
        match enviar_masivos(&directorio, &unidad, &RelojSistema).await {
            Ok(r) if r.mensajes > 0 => tracing::info!(
                copropiedades = r.copropiedades,
                mensajes = r.mensajes,
                "publicidad masiva encolada"
            ),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "no se pudo enviar la publicidad masiva"),
        }
        tokio::select! {
            biased;
            () = &mut apagado => break,
            () = tokio::time::sleep(INTERVALO) => {}
        }
    }
    tracing::info!("worker de publicidad detenido");
}
