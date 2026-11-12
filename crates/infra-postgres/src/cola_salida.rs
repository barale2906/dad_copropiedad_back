//! Consumo del outbox por el worker de envíos, mediante las funciones `SECURITY DEFINER`
//! `outbox_reclamar`, `outbox_marcar_enviado` y `outbox_marcar_fallido`: el worker atiende a
//! todas las copropiedades sin fijar tenant y sin acceso directo a la tabla de otros tenants.

use std::time::Duration;

use application::{
    AppError, AppResult,
    outbox::{Canal, MensajeOutbox},
    puertos::{ColaSalida, LoteSalida, MensajePendiente, mensajeria::MensajeSaliente},
};
use domain::TenantId;
use sqlx::{PgPool, Postgres, postgres::types::PgInterval};

/// Implementación de [`ColaSalida`] sobre el pool de la aplicación (rol `dad_app`).
#[derive(Debug, Clone)]
pub struct ColaSalidaPg {
    pool: PgPool,
}

impl ColaSalidaPg {
    /// Cola sobre el pool de la app (`dad_app`).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Lote reclamado: una transacción abierta que mantiene bloqueadas sus filas.
#[derive(Debug)]
pub struct LoteSalidaPg {
    tx: sqlx::Transaction<'static, Postgres>,
    mensajes: Vec<MensajePendiente>,
}

impl ColaSalida for ColaSalidaPg {
    type Lote = LoteSalidaPg;

    async fn reclamar(&self, canales: &[Canal], limite: u32) -> AppResult<LoteSalidaPg> {
        let canales: Vec<String> = canales.iter().map(|c| c.as_str().to_owned()).collect();
        let limite = i32::try_from(limite).unwrap_or(i32::MAX);
        let mut tx = self.pool.begin().await.map_err(AppError::infra)?;

        let filas = sqlx::query!(
            r#"SELECT id AS "id!", tenant_id AS "tenant_id?", intentos AS "intentos!",
                      payload AS "payload!"
               FROM outbox_reclamar($1, $2)"#,
            &canales,
            limite,
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(AppError::infra)?;

        let mensajes = filas
            .into_iter()
            .map(|f| MensajePendiente {
                id: f.id,
                tenant_id: f.tenant_id.map(TenantId::from_uuid),
                intentos: u32::try_from(f.intentos).unwrap_or(0),
                mensaje: serde_json::from_value::<MensajeOutbox>(f.payload)
                    .map_err(|e| e.to_string()),
            })
            .collect();

        Ok(LoteSalidaPg { tx, mensajes })
    }
}

impl LoteSalida for LoteSalidaPg {
    fn tomar_mensajes(&mut self) -> Vec<MensajePendiente> {
        std::mem::take(&mut self.mensajes)
    }

    async fn marcar_enviado(&mut self, id: i64, referencia: Option<String>) -> AppResult<()> {
        sqlx::query!("SELECT outbox_marcar_enviado($1, $2)", id, referencia)
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        Ok(())
    }

    async fn marcar_fallido(
        &mut self,
        id: i64,
        error: String,
        espera: Duration,
        max_intentos: u32,
    ) -> AppResult<()> {
        let espera = PgInterval::try_from(espera).map_err(AppError::Infraestructura)?;
        let max_intentos = i32::try_from(max_intentos).unwrap_or(i32::MAX);
        sqlx::query!(
            "SELECT outbox_marcar_fallido($1, $2, $3, $4)",
            id,
            error,
            espera,
            max_intentos,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn desviar_push(&mut self, id: i64, respaldo: Option<MensajeSaliente>) -> AppResult<()> {
        let (linea, destino, payload) = match respaldo {
            Some(m) => {
                let mensaje = MensajeOutbox::WhatsApp(m);
                (
                    mensaje.linea().map(str::to_owned),
                    Some(mensaje.destino().to_owned()),
                    Some(serde_json::to_value(&mensaje).map_err(AppError::infra)?),
                )
            }
            None => (None, None, None),
        };
        sqlx::query!(
            "SELECT outbox_desviar_push($1, $2, $3, $4)",
            id,
            linea,
            destino,
            payload,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn confirmar(self) -> AppResult<()> {
        self.tx.commit().await.map_err(AppError::infra)
    }
}
