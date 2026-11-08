//! Worker de envíos: despacha el outbox en bucle mientras el gateway esté vivo.
//!
//! - Despacha los canales configurados: correo (si hay SMTP) y WhatsApp (si hay token).
//! - Con trabajo, encadena lotes sin pausa; sin trabajo, espera [`ESPERA_INACTIVO`] o a que el
//!   chat avise que encoló respuestas.
//! - Un error de la cola (BD caída) no tumba el proceso: se registra y se reintenta tras
//!   [`ESPERA_ERROR`].
//! - Al recibir la señal de apagado termina el lote en curso y sale (los mensajes no
//!   confirmados vuelven a quedar pendientes).

use std::{future::Future, time::Duration};

use application::{
    casos_uso::{Despachadores, despachar_lote},
    outbox::PoliticaReintentos,
    puertos::{ColaSalida, EnvioCorreo, Mensajeria},
};
use tokio::sync::Notify;

/// Mensajes por lote: acota el tiempo que las filas quedan bloqueadas.
pub const TAMANO_LOTE: u32 = 20;
/// Pausa cuando no hay mensajes pendientes.
pub const ESPERA_INACTIVO: Duration = Duration::from_secs(2);
/// Pausa tras un error de la cola.
pub const ESPERA_ERROR: Duration = Duration::from_secs(10);

/// Ejecuta el worker hasta que `apagado` se resuelva.
pub async fn ejecutar<C, E, M>(
    cola: C,
    correo: Option<E>,
    whatsapp: Option<M>,
    hay_salida: &Notify,
    apagado: impl Future<Output = ()>,
) where
    C: ColaSalida,
    E: EnvioCorreo,
    M: Mensajeria,
{
    tokio::pin!(apagado);
    let envios = Despachadores {
        correo: correo.as_ref(),
        whatsapp: whatsapp.as_ref(),
    };
    tracing::info!(canales = ?envios.canales(), "worker de envíos iniciado");
    loop {
        let espera = match despachar_lote(
            &cola,
            &envios,
            PoliticaReintentos::default(),
            TAMANO_LOTE,
        )
        .await
        {
            Ok(resumen) if resumen.vacio() => ESPERA_INACTIVO,
            Ok(resumen) => {
                tracing::info!(
                    enviados = resumen.enviados,
                    fallidos = resumen.fallidos,
                    "lote del outbox despachado"
                );
                Duration::ZERO
            }
            Err(e) => {
                tracing::error!(error = %e, "no se pudo despachar el outbox");
                ESPERA_ERROR
            }
        };

        tokio::select! {
            biased;
            () = &mut apagado => break,
            () = hay_salida.notified() => {}
            () = tokio::time::sleep(espera) => {}
        }
    }
    tracing::info!("worker de envíos detenido");
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use application::{
        AppResult,
        outbox::Canal,
        puertos::{LoteSalida, MensajePendiente, correo::CorreoSaliente},
    };

    use super::*;

    /// Cola vacía que cuenta cuántas veces se reclamó.
    #[derive(Clone, Default)]
    struct ColaVacia(Arc<AtomicUsize>);

    struct LoteVacio;

    impl ColaSalida for ColaVacia {
        type Lote = LoteVacio;

        async fn reclamar(&self, _canales: &[Canal], _limite: u32) -> AppResult<LoteVacio> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(LoteVacio)
        }
    }

    impl LoteSalida for LoteVacio {
        fn tomar_mensajes(&mut self) -> Vec<MensajePendiente> {
            Vec::new()
        }

        async fn marcar_enviado(&mut self, _id: i64, _referencia: Option<String>) -> AppResult<()> {
            Ok(())
        }

        async fn marcar_fallido(
            &mut self,
            _id: i64,
            _error: String,
            _espera: Duration,
            _max_intentos: u32,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn confirmar(self) -> AppResult<()> {
            Ok(())
        }
    }

    struct SinCorreo;

    impl EnvioCorreo for SinCorreo {
        async fn enviar(&self, _correo: &CorreoSaliente) -> AppResult<Option<String>> {
            Ok(None)
        }
    }

    struct SinWhatsapp;

    impl Mensajeria for SinWhatsapp {
        async fn enviar(
            &self,
            _m: &application::puertos::mensajeria::MensajeSaliente,
        ) -> AppResult<String> {
            Ok(String::new())
        }

        async fn descargar_media(
            &self,
            _id: &str,
        ) -> AppResult<application::puertos::mensajeria::MediaDescargada> {
            Err(application::AppError::Validacion("sin media".into()))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn un_aviso_despierta_al_worker_sin_esperar() {
        let cola = ColaVacia::default();
        let consultas = Arc::clone(&cola.0);
        let aviso = Arc::new(Notify::new());
        aviso.notify_one();

        // Con el aviso pendiente, a los 1,5 s ya consultó dos veces (0 s y de inmediato).
        ejecutar(
            cola,
            Some(SinCorreo),
            None::<SinWhatsapp>,
            &aviso,
            tokio::time::sleep(Duration::from_millis(1500)),
        )
        .await;

        assert_eq!(consultas.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn sin_trabajo_espera_entre_consultas_y_se_detiene_al_apagar() {
        let cola = ColaVacia::default();
        let consultas = Arc::clone(&cola.0);

        // Apagado a los 5 s simulados: con esperas de 2 s caben 3 consultas (0 s, 2 s, 4 s).
        ejecutar(
            cola,
            Some(SinCorreo),
            None::<SinWhatsapp>,
            &Notify::new(),
            tokio::time::sleep(Duration::from_secs(5)),
        )
        .await;

        assert_eq!(consultas.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn apagado_inmediato_procesa_a_lo_sumo_un_lote() {
        let cola = ColaVacia::default();
        let consultas = Arc::clone(&cola.0);

        ejecutar(
            cola,
            Some(SinCorreo),
            None::<SinWhatsapp>,
            &Notify::new(),
            async {},
        )
        .await;

        assert_eq!(consultas.load(Ordering::SeqCst), 1);
    }
}
