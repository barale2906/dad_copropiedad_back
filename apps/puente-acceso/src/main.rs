//! `dad_puente_acceso`: puente de acceso en sitio (ver la documentación del crate).
//!
//! Bucles:
//! - Sincronización cada `PUENTE_INTERVALO_SEGUNDOS` (cuenta como latido): aplica los cambios a
//!   la cache, la guarda en disco y borra las plantillas biométricas revocadas.
//! - Lecturas del controlador: decide con la cache (sin red), abre si corresponde y anota el
//!   evento en la cola en disco.
//! - Reporte de la cola cuando hay conexión.
//!
//! Controlador: `PUENTE_CONTROLADOR=simulado` lee líneas de la entrada estándar
//! (`<punto> <entrada|salida> <tipo|qr> <valor>`). Los adaptadores de fabricantes se agregan con
//! el hardware de la primera PH.

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::Context;
use chrono::Utc;
use chrono_tz::Tz;
use puente_acceso::{
    Cache, ColaEventos, ControladorDispositivo, ControladorSimulado, cliente::ClienteNube, decidir,
    leer_linea_simulador,
};
use shared::{PuenteConfig, telemetry};
use tokio::{io::AsyncBufReadExt, sync::Mutex};

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Estado {
    cache: Mutex<Cache>,
    cola: Mutex<ColaEventos>,
    ruta_cache: PathBuf,
    ruta_cola: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = PuenteConfig::from_env().context("configuración del puente inválida")?;
    telemetry::init(config.log_format);
    tracing::info!(?config, "iniciando dad_puente_acceso v{VERSION}");
    let tz: Tz = config
        .zona_horaria
        .parse()
        .map_err(|e| anyhow::anyhow!("PUENTE_ZONA_HORARIA inválida: {e}"))?;
    let dir = PathBuf::from(&config.datos_dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("no se pudo crear {}", dir.display()))?;
    let estado = Arc::new(Estado {
        cache: Mutex::new(Cache::cargar(&dir.join("cache.json"))?),
        cola: Mutex::new(ColaEventos::cargar(&dir.join("cola.json"))?),
        ruta_cache: dir.join("cache.json"),
        ruta_cola: dir.join("cola.json"),
    });
    let nube = ClienteNube::new(&config.api_url, &config.token)?;
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let mut controlador = ControladorSimulado {
        lecturas: rx,
        aperturas: Vec::new(),
        plantillas_borradas: Vec::new(),
    };
    tokio::spawn(async move {
        let mut lineas = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(l)) = lineas.next_line().await {
            match leer_linea_simulador(&l) {
                Some(lectura) => {
                    if tx.send(lectura).await.is_err() {
                        break;
                    }
                }
                None => tracing::warn!("línea del simulador inválida"),
            }
        }
    });

    let intervalo = Duration::from_secs(config.intervalo_segundos.max(5));
    let sincronizador = {
        let estado = Arc::clone(&estado);
        let nube = nube.clone();
        tokio::spawn(async move {
            loop {
                sincronizar(&estado, &nube).await;
                reportar(&estado, &nube).await;
                tokio::time::sleep(intervalo).await;
            }
        })
    };

    let apagado = shared::apagado::senal_apagado();
    tokio::pin!(apagado);
    loop {
        tokio::select! {
            () = &mut apagado => break,
            lectura = controlador.leer() => {
                let Some(l) = lectura else { break };
                let ahora = Utc::now();
                let d = decidir(&mut *estado.cache.lock().await, l.punto, &l.lectura, ahora, tz);
                if d.resultado == domain::ResultadoAcceso::Permitido {
                    controlador.abrir(&l).await;
                }
                tracing::info!(resultado = %d.resultado, motivo = d.motivo, "lectura decidida");
                let placa = match &l.lectura {
                    puente_acceso::Lectura::Credencial { tipo: domain::TipoCredencial::Placa, valor } => Some(valor.clone()),
                    _ => None,
                };
                let mut cola = estado.cola.lock().await;
                cola.anotar(&d, l.punto, l.dispositivo_id, l.sentido, placa, ahora);
                if let Err(e) = cola.guardar(&estado.ruta_cola) {
                    tracing::error!(error = %e, "no se pudo guardar la cola de eventos");
                }
                if let Err(e) = estado.cache.lock().await.guardar(&estado.ruta_cache) {
                    tracing::error!(error = %e, "no se pudo guardar la cache");
                }
            }
        }
    }
    sincronizador.abort();
    tracing::info!("puente detenido");
    Ok(())
}

async fn sincronizar(estado: &Estado, nube: &ClienteNube) {
    loop {
        let cursor = estado.cache.lock().await.cursor;
        let s = match nube.sincronizar(cursor, VERSION).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "sin conexión con la nube: se decide con la cache local");
                return;
            }
        };
        let nuevo = s.cursor;
        let mut cache = estado.cache.lock().await;
        let borrar = cache.aplicar(s);
        if let Err(e) = cache.guardar(&estado.ruta_cache) {
            tracing::error!(error = %e, "no se pudo guardar la cache");
        }
        drop(cache);
        if !borrar.is_empty() {
            // El adaptador real borra la plantilla en el equipo; el simulado solo confirma.
            let mut cola = estado.cola.lock().await;
            cola.borrados_biometricos
                .extend(borrar.iter().map(|c| c.as_uuid()));
            let _ = cola.guardar(&estado.ruta_cola);
        }
        if nuevo <= cursor {
            return;
        }
    }
}

async fn reportar(estado: &Estado, nube: &ClienteNube) {
    let (eventos, borrados) = {
        let cola = estado.cola.lock().await;
        let (e, b) = cola.lote();
        (e.to_vec(), b.to_vec())
    };
    if eventos.is_empty() && borrados.is_empty() {
        return;
    }
    match nube.reportar(&eventos, &borrados).await {
        Ok(r) => {
            let mut cola = estado.cola.lock().await;
            cola.confirmar(eventos.len(), borrados.len());
            let _ = cola.guardar(&estado.ruta_cola);
            tracing::info!(
                nuevos = r.nuevos,
                repetidos = r.repetidos,
                "eventos reportados"
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, pendientes = eventos.len(), "no se pudieron reportar los eventos")
        }
    }
}
