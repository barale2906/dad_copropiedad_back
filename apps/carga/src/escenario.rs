//! Escenario de carga sobre un [`Plan`] sembrado: todas las copropiedades a la vez.
//!
//! - **Navegación**: en cada copropiedad, `usuarios` sesiones recorren en bucle capacidades,
//!   listado de bienes, tablero y quórum de la asamblea, con una pausa entre peticiones (el
//!   tiempo que una persona tarda en leer).
//! - **Asamblea**: al empezar se abre una pregunta en cada copropiedad; tras `inicio_votacion`,
//!   todos los asistentes votan a la vez, con `concurrencia_votos` operadores por copropiedad
//!   (puestos de la mesa). Es el pico real: varias asambleas votando al mismo tiempo.
//!
//! Cada ejecución abre su propia pregunta y la cierra al final, así que se puede repetir sobre
//! la misma siembra.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use reqwest::Method;
use serde::Serialize;
use serde_json::json;
use tokio::{sync::Semaphore, task::JoinSet};

use crate::{
    cliente::{Cliente, Cuerpo},
    estadisticas::{Resumen, resumir},
    siembra::{Plan, token},
};

/// Parámetros de la ejecución.
#[derive(Debug, Clone)]
pub struct OpcionesEjecucion {
    /// Cuánto dura la medición.
    pub duracion: Duration,
    /// Sesiones navegando por copropiedad.
    pub usuarios_por_copropiedad: usize,
    /// Pausa de cada sesión entre peticiones.
    pub pausa: Duration,
    /// Votos en paralelo por copropiedad.
    pub concurrencia_votos: usize,
    /// Cuándo empieza la votación, desde el inicio.
    pub inicio_votacion: Duration,
    /// Contraseña de los administradores sembrados.
    pub contrasena: String,
}

/// Resultado de una ejecución.
#[derive(Debug, Clone, Serialize)]
pub struct Informe {
    /// Copropiedades simultáneas.
    pub copropiedades: usize,
    /// Duración real de la medición en milisegundos.
    pub duracion_ms: u64,
    /// Votos que debían registrarse (uno por asistente).
    pub votos_esperados: usize,
    /// Votos registrados.
    pub votos_registrados: usize,
    /// Tiempo desde el primer voto hasta el último registrado, en milisegundos.
    pub votacion_ms: u64,
    /// Resumen por operación y total.
    pub resumenes: Vec<Resumen>,
}

/// Pregunta abierta para esta ejecución en una copropiedad.
struct Preparada {
    token: String,
    base: String,
    reunion: String,
    pregunta: String,
    opcion: serde_json::Value,
    asistentes: Vec<String>,
}

async fn preparar(c: &Cliente, plan: &Plan, contrasena: &str) -> anyhow::Result<Vec<Preparada>> {
    let mut listas = Vec::with_capacity(plan.copropiedades.len());
    for ph in &plan.copropiedades {
        let t = token(c, &ph.admin, contrasena).await?;
        let base = format!("/api/v1/copropiedades/{}", ph.id);
        let ruta = format!("{base}/asambleas/{}/preguntas", ph.reunion);
        let p = c
            .pedir(
                "preparar",
                Method::POST,
                &ruta,
                Some(&t),
                Some(&json!({ "enunciado": "¿Aprueba la prueba de carga?", "tipo": "si_no" })),
            )
            .await
            .exigir(&[201], "crear la pregunta")?;
        let pregunta = p["id"].as_str().context("id de la pregunta")?.to_owned();
        c.pedir(
            "preparar",
            Method::PUT,
            &format!("{ruta}/{pregunta}/estado"),
            Some(&t),
            Some(&json!({ "accion": "abrir" })),
        )
        .await
        .exigir(&[200], "abrir la pregunta")?;
        listas.push(Preparada {
            token: t,
            base,
            reunion: ph.reunion.clone(),
            pregunta,
            opcion: p["opciones"][0]["id"].clone(),
            asistentes: ph.asistentes.clone(),
        });
    }
    Ok(listas)
}

async fn navegar(c: Cliente, ph: Arc<Preparada>, pausa: Duration, fin: Instant) {
    let rutas = [
        ("capacidades", format!("{}/capacidades", ph.base)),
        ("bienes", format!("{}/bienes-privados?limite=50", ph.base)),
        (
            "tablero",
            format!("{}/asambleas/{}/tablero", ph.base, ph.reunion),
        ),
        (
            "quorum",
            format!("{}/asambleas/{}/quorum", ph.base, ph.reunion),
        ),
    ];
    while Instant::now() < fin {
        for (op, ruta) in &rutas {
            if Instant::now() >= fin {
                return;
            }
            c.pedir(op, Method::GET, ruta, Some(&ph.token), None).await;
            tokio::time::sleep(pausa).await;
        }
    }
}

/// Vota con todos los asistentes; devuelve cuántos votos quedaron registrados.
async fn votar(c: Cliente, ph: Arc<Preparada>, espera: Duration, concurrencia: usize) -> usize {
    tokio::time::sleep(espera).await;
    let limite = Arc::new(Semaphore::new(concurrencia.max(1)));
    let ruta = format!(
        "{}/asambleas/{}/preguntas/{}/votos",
        ph.base, ph.reunion, ph.pregunta
    );
    let mut tareas = JoinSet::new();
    for asistente in ph.asistentes.clone() {
        let (c, ph, ruta, limite) = (c.clone(), ph.clone(), ruta.clone(), limite.clone());
        tareas.spawn(async move {
            let Ok(_turno) = limite.acquire_owned().await else {
                return false;
            };
            let cuerpo = json!({ "asistente_id": asistente, "opciones": [ph.opcion], "canal": "presencial" });
            let clave = uuid::Uuid::new_v4().to_string();
            let r = c
                .pedir_con("votar", Method::POST, &ruta, Some(&ph.token), &[("idempotency-key", clave)], Cuerpo::Json(&cuerpo))
                .await;
            r.estado == 201 && r.cuerpo["registrado"] == true
        });
    }
    let mut registrados = 0;
    while let Some(r) = tareas.join_next().await {
        registrados += usize::from(r.unwrap_or(false));
    }
    registrados
}

/// Ejecuta el escenario y devuelve el informe. Las peticiones de preparación y de cierre no
/// cuentan en el informe.
///
/// # Errors
///
/// Si no se puede preparar una copropiedad (sesión o pregunta).
pub async fn ejecutar(c: &Cliente, plan: &Plan, o: &OpcionesEjecucion) -> anyhow::Result<Informe> {
    let listas: Vec<Arc<Preparada>> = preparar(c, plan, &o.contrasena)
        .await?
        .into_iter()
        .map(Arc::new)
        .collect();
    c.reiniciar_muestras();
    let inicio = Instant::now();
    let fin = inicio + o.duracion;

    let mut navegacion = JoinSet::new();
    for ph in &listas {
        for _ in 0..o.usuarios_por_copropiedad {
            navegacion.spawn(navegar(c.clone(), ph.clone(), o.pausa, fin));
        }
    }
    let mut votacion = JoinSet::new();
    for ph in &listas {
        votacion.spawn(votar(
            c.clone(),
            ph.clone(),
            o.inicio_votacion,
            o.concurrencia_votos,
        ));
    }
    let mut registrados = 0;
    while let Some(r) = votacion.join_next().await {
        registrados += r.unwrap_or(0);
    }
    let votacion_ms = u64::try_from(
        inicio
            .elapsed()
            .saturating_sub(o.inicio_votacion)
            .as_millis(),
    )
    .unwrap_or(u64::MAX);
    while navegacion.join_next().await.is_some() {}
    let duracion = inicio.elapsed();
    let muestras = c.muestras();

    // Cierre: la pregunta de esta ejecución queda cerrada (no cuenta en el informe).
    for ph in &listas {
        let ruta = format!(
            "{}/asambleas/{}/preguntas/{}/estado",
            ph.base, ph.reunion, ph.pregunta
        );
        c.pedir(
            "cerrar",
            Method::PUT,
            &ruta,
            Some(&ph.token),
            Some(&json!({ "accion": "cerrar" })),
        )
        .await;
    }

    Ok(Informe {
        copropiedades: listas.len(),
        duracion_ms: u64::try_from(duracion.as_millis()).unwrap_or(u64::MAX),
        votos_esperados: listas.iter().map(|p| p.asistentes.len()).sum(),
        votos_registrados: registrados,
        votacion_ms,
        resumenes: resumir(&muestras, duracion),
    })
}
