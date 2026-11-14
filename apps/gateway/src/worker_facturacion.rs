//! Worker de contingencias de la facturación electrónica: cada [`INTERVALO`] retransmite a la
//! DIAN los documentos que quedaron en `contingencia` (la DIAN no respondió al emitirlos) en
//! todas las entidades. Al validarse, el caso de uso entrega al adquirente el contenedor con la
//! respuesta de la DIAN. Los que pasan de 48 horas se avisan en los logs, sin datos del documento.

use std::{future::Future, sync::Arc, time::Duration};

use application::{
    casos_uso::facturacion::documentos::{Expedicion, TxFacturacion, reintentar_contingencias},
    puertos::{
        AlmacenArchivos, GeneradorActas, RelojSistema, RepositorioPlataforma, UnidadDeTrabajo,
        UnidadDeTrabajoPlataforma, archivos::Empaquetador, cofre::Cofre,
        tributario::ProveedorFacturacion,
    },
};
use tokio::sync::Notify;

/// Cada cuánto se reintentan los documentos en contingencia.
pub const INTERVALO: Duration = Duration::from_secs(30 * 60);

/// Documentos por ronda (los demás esperan a la siguiente).
const LIMITE: u32 = 100;

/// Medios de expedición que el worker presta al caso de uso.
pub struct Medios<P, C, G, E> {
    /// Proveedor de documentos electrónicos (simulado o motor propio).
    pub proveedor: P,
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
pub async fn ejecutar<UP, U, P, C, G, E, A>(
    plataforma: UP,
    unidad: U,
    medios: Medios<P, C, G, E>,
    almacen: Arc<A>,
    hay_salida: Arc<Notify>,
    apagado: impl Future<Output = ()>,
) where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioPlataforma,
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    P: ProveedorFacturacion,
    C: Cofre,
    G: GeneradorActas,
    E: Empaquetador,
    A: AlmacenArchivos,
{
    tokio::pin!(apagado);
    tracing::info!("worker de contingencias de facturación iniciado");
    let expedicion = Expedicion {
        proveedor: &medios.proveedor,
        cofre: medios.cofre.as_ref(),
        generador: &medios.generador,
        empaquetador: &medios.empaquetador,
        pie: medios.pie.as_deref(),
    };
    loop {
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
    tracing::info!("worker de contingencias de facturación detenido");
}
