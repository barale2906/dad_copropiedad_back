//! Tablero en vivo de las asambleas (quórum, punto en curso y escrutinio de las preguntas
//! abiertas) para las pantallas del conductor, el proyector y la mesa.
//!
//! - Los votos llegan por la API (web, operador, puestos) y por el gateway (WhatsApp). Cada
//!   cambio hace `pg_notify` dentro de su transacción: solo se avisa si se confirma.
//! - [`escuchar`] recibe los avisos (`tenant:reunion`) y marca la reunión como «sucia».
//! - [`emitir`] cada [`INTERVALO`] recalcula el tablero **desde la BD** (la única verdad) de las
//!   reuniones sucias que tienen pantallas conectadas y lo difunde: nunca un mensaje por voto.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use application::{
    AppResult, Contexto, casos_uso::asambleas::quorum::tablero_en_vivo, puertos::RelojSistema,
};
use domain::{ReunionId, TenantId};
use infra_postgres::{OyenteEnVivo, PgPool, UnidadDeTrabajoPg};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::routes::asambleas::TableroRespuesta;

/// Cada cuánto se emiten los tableros que cambiaron.
pub const INTERVALO: Duration = Duration::from_millis(500);

type Clave = (TenantId, ReunionId);

/// Canales de difusión por reunión y reuniones con cambios pendientes de emitir.
#[derive(Debug, Default)]
pub struct EnVivo {
    canales: Mutex<HashMap<Clave, broadcast::Sender<Arc<str>>>>,
    sucias: Mutex<HashSet<Clave>>,
}

impl EnVivo {
    /// Hub sin suscriptores.
    pub fn new() -> Self {
        Self::default()
    }

    /// Suscribe una pantalla a la reunión.
    pub fn suscribir(&self, tenant: TenantId, reunion: ReunionId) -> broadcast::Receiver<Arc<str>> {
        let mut canales = self
            .canales
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        canales
            .entry((tenant, reunion))
            .or_insert_with(|| broadcast::channel(16).0)
            .subscribe()
    }

    /// Marca la reunión de un aviso `tenant:reunion` (los mal formados se ignoran).
    pub fn marcar(&self, aviso: &str) {
        let Some((t, r)) = aviso.split_once(':') else {
            return;
        };
        if let (Ok(t), Ok(r)) = (Uuid::try_parse(t), Uuid::try_parse(r)) {
            self.sucias
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert((TenantId::from_uuid(t), ReunionId::from_uuid(r)));
        }
    }

    /// Reuniones sucias con pantallas conectadas (y olvida los canales sin suscriptores).
    fn pendientes(&self) -> Vec<(Clave, broadcast::Sender<Arc<str>>)> {
        let sucias: Vec<Clave> = self
            .sucias
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain()
            .collect();
        let mut canales = self
            .canales
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        canales.retain(|_, tx| tx.receiver_count() > 0);
        sucias
            .into_iter()
            .filter_map(|c| canales.get(&c).map(|tx| (c, tx.clone())))
            .collect()
    }
}

/// Tablero de la reunión en JSON.
///
/// La conexión inicial pasa el contexto de la cuenta (el caso de uso exige `asambleas.ver`);
/// la difusión periódica usa el del sistema, porque cada suscriptor ya fue autorizado al
/// conectarse.
///
/// # Errors
///
/// Los del caso de uso [`tablero_en_vivo`] (sin permiso, reunión inexistente, módulo no
/// habilitado) o de serialización.
pub async fn tablero_json(
    unidad: &UnidadDeTrabajoPg,
    ctx: &Contexto,
    reunion: ReunionId,
) -> AppResult<String> {
    let t = tablero_en_vivo(unidad, &RelojSistema, ctx, reunion).await?;
    serde_json::to_string(&TableroRespuesta::from(t)).map_err(application::AppError::infra)
}

/// Escucha los avisos de PostgreSQL hasta el apagado (se reconecta si la conexión cae).
pub async fn escuchar(pool: PgPool, hub: Arc<EnVivo>, apagado: impl Future<Output = ()>) {
    tokio::pin!(apagado);
    loop {
        let mut oyente = match OyenteEnVivo::conectar(&pool).await {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(error = %e, "no se pudo conectar el oyente en vivo");
                tokio::select! {
                    () = &mut apagado => return,
                    () = tokio::time::sleep(Duration::from_secs(2)) => continue,
                }
            }
        };
        loop {
            tokio::select! {
                () = &mut apagado => return,
                aviso = oyente.recibir() => match aviso {
                    Ok(a) => hub.marcar(&a),
                    Err(e) => {
                        tracing::warn!(error = %e, "se perdió el oyente en vivo; reconectando");
                        break;
                    }
                },
            }
        }
    }
}

/// Emite los tableros pendientes cada [`INTERVALO`] hasta el apagado.
pub async fn emitir(
    hub: Arc<EnVivo>,
    unidad: UnidadDeTrabajoPg,
    apagado: impl Future<Output = ()>,
) {
    tokio::pin!(apagado);
    let mut intervalo = tokio::time::interval(INTERVALO);
    intervalo.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            () = &mut apagado => return,
            _ = intervalo.tick() => {}
        }
        for ((tenant, reunion), tx) in hub.pendientes() {
            match tablero_json(&unidad, &Contexto::new(tenant), reunion).await {
                Ok(json) => {
                    let _ = tx.send(json.into());
                }
                Err(e) => {
                    tracing::warn!(tenant_id = %tenant, reunion_id = %reunion, error = %e, "no se pudo calcular el tablero")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_emite_reuniones_sucias_con_pantallas() {
        let hub = EnVivo::new();
        let (t, r) = (TenantId::nuevo(), ReunionId::nuevo());
        hub.marcar(&format!("{t}:{r}"));
        assert!(
            hub.pendientes().is_empty(),
            "sin pantallas conectadas no se calcula"
        );

        let _rx = hub.suscribir(t, r);
        hub.marcar(&format!("{t}:{r}"));
        hub.marcar("basura");
        assert_eq!(hub.pendientes().len(), 1);
        assert!(hub.pendientes().is_empty(), "ya se emitió");
    }
}
