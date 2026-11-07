//! Caso de uso: despachar un lote del outbox (lo ejecuta en bucle el worker del gateway).
//!
//! Hoy despacha el canal **correo**. WhatsApp se agregará cuando exista su cliente de envío
//! (porción vertical de votaciones); mientras tanto sus mensajes siguen pendientes y este
//! worker no los toca, porque solo reclama los canales que sabe enviar.

use crate::{
    AppResult,
    outbox::{Canal, MensajeOutbox, PoliticaReintentos},
    puertos::{ColaSalida, EnvioCorreo, LoteSalida, MensajePendiente},
};

/// Canales que este despachador sabe enviar.
pub const CANALES: &[Canal] = &[Canal::Correo];

/// Resultado de un lote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumenDespacho {
    pub enviados: usize,
    /// Intentos fallidos (se reintentarán o quedaron `fallido` si agotaron los intentos).
    pub fallidos: usize,
}

impl ResumenDespacho {
    /// ¿El lote vino vacío? El worker lo usa para esperar antes de volver a consultar.
    pub const fn vacio(self) -> bool {
        self.enviados == 0 && self.fallidos == 0
    }
}

/// Reclama hasta `limite` mensajes, los envía y registra el resultado de cada uno.
///
/// Un fallo de envío **no** aborta el lote: se registra como intento fallido con la espera
/// de [`PoliticaReintentos`] y se sigue con el siguiente mensaje.
///
/// # Errors
///
/// Solo errores de la cola (base de datos); en ese caso el lote se revierte y los mensajes
/// vuelven a quedar pendientes.
pub async fn despachar_lote<C, E>(
    cola: &C,
    correo: &E,
    politica: PoliticaReintentos,
    limite: u32,
) -> AppResult<ResumenDespacho>
where
    C: ColaSalida,
    E: EnvioCorreo,
{
    let mut lote = cola.reclamar(CANALES, limite).await?;
    let mut resumen = ResumenDespacho::default();

    for pendiente in lote.tomar_mensajes() {
        let MensajePendiente {
            id,
            tenant_id,
            intentos,
            mensaje,
        } = pendiente;

        let envio = match mensaje {
            Ok(MensajeOutbox::Correo(c)) => correo.enviar(&c).await.map_err(|e| e.to_string()),
            Ok(otro) => Err(format!("canal {} sin despachador", otro.canal().as_str())),
            Err(e) => {
                // Payload ilegible: reintentar no lo arreglará.
                lote.marcar_fallido(id, format!("payload inválido: {e}"), politica.espera(0), 1)
                    .await?;
                resumen.fallidos += 1;
                tracing::error!(outbox_id = id, tenant_id = ?tenant_id, "mensaje del outbox con payload inválido");
                continue;
            }
        };

        match envio {
            Ok(referencia) => {
                lote.marcar_enviado(id, referencia).await?;
                resumen.enviados += 1;
            }
            Err(error) => {
                // El texto del error puede incluir el destino (dato personal): va a la fila del
                // outbox, no a los logs.
                lote.marcar_fallido(id, error, politica.espera(intentos), politica.max_intentos)
                    .await?;
                resumen.fallidos += 1;
                tracing::warn!(outbox_id = id, tenant_id = ?tenant_id, intentos = intentos + 1, "envío fallido; se reintentará");
            }
        }
    }

    lote.confirmar().await?;
    Ok(resumen)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use domain::{DireccionCorreo, TenantId};

    use super::*;
    use crate::{
        AppError,
        puertos::{
            correo::CorreoSaliente,
            mensajeria::{ContenidoSaliente, MensajeSaliente},
        },
    };

    #[derive(Debug, Clone, PartialEq)]
    enum Marca {
        Enviado(i64, Option<String>),
        Fallido(i64, Duration, u32),
    }

    #[derive(Default, Clone)]
    struct Registro {
        canales: Arc<Mutex<Vec<Canal>>>,
        marcas: Arc<Mutex<Vec<Marca>>>,
        confirmado: Arc<Mutex<bool>>,
    }

    struct ColaFalsa {
        mensajes: Mutex<Vec<MensajePendiente>>,
        registro: Registro,
    }

    struct LoteFalso {
        mensajes: Vec<MensajePendiente>,
        marcas: Vec<Marca>,
        registro: Registro,
    }

    impl ColaSalida for ColaFalsa {
        type Lote = LoteFalso;

        async fn reclamar(&self, canales: &[Canal], _limite: u32) -> AppResult<LoteFalso> {
            self.registro
                .canales
                .lock()
                .expect("mutex")
                .extend_from_slice(canales);
            Ok(LoteFalso {
                mensajes: std::mem::take(&mut *self.mensajes.lock().expect("mutex")),
                marcas: Vec::new(),
                registro: self.registro.clone(),
            })
        }
    }

    impl LoteSalida for LoteFalso {
        fn tomar_mensajes(&mut self) -> Vec<MensajePendiente> {
            std::mem::take(&mut self.mensajes)
        }

        async fn marcar_enviado(&mut self, id: i64, referencia: Option<String>) -> AppResult<()> {
            self.marcas.push(Marca::Enviado(id, referencia));
            Ok(())
        }

        async fn marcar_fallido(
            &mut self,
            id: i64,
            _error: String,
            espera: Duration,
            max_intentos: u32,
        ) -> AppResult<()> {
            self.marcas.push(Marca::Fallido(id, espera, max_intentos));
            Ok(())
        }

        async fn confirmar(self) -> AppResult<()> {
            self.registro
                .marcas
                .lock()
                .expect("mutex")
                .extend(self.marcas);
            *self.registro.confirmado.lock().expect("mutex") = true;
            Ok(())
        }
    }

    /// Falla con los correos dirigidos a `falla@…`.
    struct CorreoFalso;

    impl EnvioCorreo for CorreoFalso {
        async fn enviar(&self, correo: &CorreoSaliente) -> AppResult<Option<String>> {
            if correo.para.as_str().starts_with("falla@") {
                Err(AppError::Validacion("buzón rechazado".into()))
            } else {
                Ok(Some(format!("ref-{}", correo.asunto)))
            }
        }
    }

    fn correo(id: i64, para: &str, intentos: u32) -> MensajePendiente {
        MensajePendiente {
            id,
            tenant_id: Some(TenantId::nuevo()),
            intentos,
            mensaje: Ok(MensajeOutbox::Correo(CorreoSaliente {
                para: DireccionCorreo::new(para).expect("válida"),
                asunto: id.to_string(),
                texto: "Texto".into(),
                html: None,
            })),
        }
    }

    async fn despachar(mensajes: Vec<MensajePendiente>) -> (ResumenDespacho, Registro) {
        let registro = Registro::default();
        let cola = ColaFalsa {
            mensajes: Mutex::new(mensajes),
            registro: registro.clone(),
        };
        let resumen = despachar_lote(&cola, &CorreoFalso, PoliticaReintentos::default(), 10)
            .await
            .expect("el lote se procesa");
        (resumen, registro)
    }

    #[tokio::test]
    async fn solo_reclama_los_canales_que_sabe_enviar() {
        let (resumen, registro) = despachar(vec![]).await;

        assert!(resumen.vacio());
        assert_eq!(
            *registro.canales.lock().expect("mutex"),
            vec![Canal::Correo]
        );
        assert!(*registro.confirmado.lock().expect("mutex"));
    }

    #[tokio::test]
    async fn envia_y_marca_con_la_referencia_del_proveedor() {
        let (resumen, registro) = despachar(vec![correo(1, "ok@b.co", 0)]).await;

        assert_eq!(
            resumen,
            ResumenDespacho {
                enviados: 1,
                fallidos: 0
            }
        );
        assert_eq!(
            *registro.marcas.lock().expect("mutex"),
            vec![Marca::Enviado(1, Some("ref-1".into()))]
        );
    }

    #[tokio::test]
    async fn un_fallo_se_reintenta_con_espera_creciente_sin_detener_el_lote() {
        let (resumen, registro) =
            despachar(vec![correo(1, "falla@b.co", 2), correo(2, "ok@b.co", 0)]).await;

        assert_eq!(
            resumen,
            ResumenDespacho {
                enviados: 1,
                fallidos: 1
            }
        );
        assert_eq!(
            *registro.marcas.lock().expect("mutex"),
            vec![
                Marca::Fallido(1, Duration::from_secs(120), 8),
                Marca::Enviado(2, Some("ref-2".into())),
            ]
        );
    }

    #[tokio::test]
    async fn payload_invalido_queda_fallido_sin_reintentos() {
        let roto = MensajePendiente {
            id: 7,
            tenant_id: Some(TenantId::nuevo()),
            intentos: 0,
            mensaje: Err("formato desconocido".into()),
        };

        let (resumen, registro) = despachar(vec![roto]).await;

        assert_eq!(resumen.fallidos, 1);
        assert!(matches!(
            registro.marcas.lock().expect("mutex")[0],
            Marca::Fallido(7, _, 1)
        ));
    }

    #[tokio::test]
    async fn canal_sin_despachador_se_registra_como_fallo() {
        let whatsapp = MensajePendiente {
            id: 3,
            tenant_id: Some(TenantId::nuevo()),
            intentos: 0,
            mensaje: Ok(MensajeOutbox::WhatsApp(MensajeSaliente {
                linea: "1".into(),
                telefono: "573001112233".into(),
                contenido: ContenidoSaliente::Texto { cuerpo: "x".into() },
            })),
        };

        let (resumen, _) = despachar(vec![whatsapp]).await;

        assert_eq!(resumen.fallidos, 1);
    }
}
