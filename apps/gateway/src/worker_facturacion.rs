//! Worker de la facturación electrónica: cada [`INTERVALO`] retransmite a la DIAN los documentos
//! que quedaron en `contingencia` (la DIAN no respondió al emitirlos) en todas las entidades; al
//! validarse, el caso de uso entrega al adquirente el contenedor con la respuesta de la DIAN. Los
//! que pasan de 48 horas se avisan en los logs, sin datos del documento. Cada
//! [`INTERVALO_CORREOS`] sincroniza los correos de recepción registrados en la DIAN.

use std::{future::Future, sync::Arc, time::Duration};

use application::{
    casos_uso::facturacion::{
        documentos::{Expedicion, TxFacturacion, reintentar_contingencias},
        habilitacion::sincronizar_correos_recepcion,
    },
    puertos::{
        AlmacenArchivos, GeneradorActas, RelojSistema, RepositorioPlataforma, UnidadDeTrabajo,
        UnidadDeTrabajoPlataforma,
        archivos::Empaquetador,
        cofre::Cofre,
        tributario::{HabilitacionDian, ProveedorFacturacion},
    },
};
use tokio::sync::Notify;

/// Cada cuánto se reintentan los documentos en contingencia.
pub const INTERVALO: Duration = Duration::from_secs(30 * 60);

/// Cada cuánto se sincronizan los correos de recepción de la DIAN.
pub const INTERVALO_CORREOS: Duration = Duration::from_secs(24 * 60 * 60);

/// Documentos por ronda (los demás esperan a la siguiente).
const LIMITE: u32 = 100;

/// Medios de expedición que el worker presta al caso de uso.
pub struct Medios<P, H, C, G, E> {
    /// Proveedor de documentos electrónicos (simulado o motor propio).
    pub proveedor: P,
    /// Servicios de habilitación del motor propio (correos de recepción).
    pub habilitacion: H,
    /// Cofre de los secretos de cada entidad.
    pub cofre: Arc<C>,
    /// Generador de la representación gráfica.
    pub generador: G,
    /// Empaquetador del ZIP que recibe el adquirente.
    pub empaquetador: E,
    /// Pie de la representación gráfica (fabricante del software).
    pub pie: Option<String>,
}

/// Ejecuta el worker hasta que `apagado` se resuelva.
pub async fn ejecutar<UP, U, P, H, C, G, E, A>(
    plataforma: UP,
    unidad: U,
    medios: Medios<P, H, C, G, E>,
    almacen: Arc<A>,
    hay_salida: Arc<Notify>,
    apagado: impl Future<Output = ()>,
) where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma,
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    P: ProveedorFacturacion,
    H: HabilitacionDian,
    C: Cofre,
    G: GeneradorActas,
    E: Empaquetador,
    A: AlmacenArchivos,
{
    tokio::pin!(apagado);
    tracing::info!("worker de facturación electrónica iniciado");
    let expedicion = Expedicion {
        proveedor: &medios.proveedor,
        cofre: medios.cofre.as_ref(),
        generador: &medios.generador,
        empaquetador: &medios.empaquetador,
        pie: medios.pie.as_deref(),
    };
    let mut correos_sincronizados: Option<tokio::time::Instant> = None;
    loop {
        if correos_sincronizados.is_none_or(|t| t.elapsed() >= INTERVALO_CORREOS) {
            match sincronizar_correos_recepcion(
                (&plataforma, &unidad),
                (&medios.habilitacion, medios.cofre.as_ref()),
                &RelojSistema,
            )
            .await
            {
                Ok(Some(n)) => {
                    tracing::info!(correos = n, "correos de recepción de la DIAN sincronizados")
                }
                Ok(None) => tracing::debug!(
                    "sin entidad de plataforma habilitada: no se sincronizan correos"
                ),
                Err(e) => {
                    tracing::warn!(error = %e, "no se pudieron sincronizar los correos de recepción")
                }
            }
            correos_sincronizados = Some(tokio::time::Instant::now());
        }
        match reintentar_contingencias(
            (&plataforma, &unidad),
            &expedicion,
            almacen.as_ref(),
            &RelojSistema,
            LIMITE,
        )
        .await
        {
            Ok(r) if r.reintentados == 0 => {}
            Ok(r) => {
                tracing::info!(
                    reintentados = r.reintentados,
                    validados = r.validados,
                    pendientes = r.pendientes,
                    "documentos en contingencia reintentados"
                );
                if r.validados > 0 {
                    hay_salida.notify_one();
                }
            }
            Err(e) => tracing::error!(error = %e, "no se pudieron reintentar las contingencias"),
        }
        tokio::select! {
            biased;
            () = &mut apagado => break,
            () = tokio::time::sleep(INTERVALO) => {}
        }
    }
    tracing::info!("worker de facturación electrónica detenido");
}
