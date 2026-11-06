//! Unidad de trabajo: agrupa en **una sola transacción** el cambio de negocio, su auditoría y
//! (más adelante) el evento del outbox, todo dentro del tenant del [`Contexto`].
//!
//! Patrón de uso en un caso de uso:
//!
//! ```ignore
//! let mut tx = unidad.iniciar(ctx).await?;      // fija el tenant (RLS)
//! tx.guardar(&bien_privado).await?;             // repositorio implementado por la transacción
//! tx.registrar(evento_auditoria).await?;        // RegistroAuditoria, misma transacción
//! tx.confirmar().await?;                        // COMMIT; si no se llama, ROLLBACK al soltarla
//! ```
//!
//! La transacción implementa los puertos que necesita cada caso de uso (empezando por
//! [`RegistroAuditoria`]); el caso de uso los exige con *bounds* sobre
//! [`UnidadDeTrabajo::Transaccion`]. Así la capa de aplicación no conoce SQL y se prueba con
//! dobles en memoria.

use std::future::Future;

use crate::{AppResult, Contexto, auditoria::EventoAuditoria};

/// Abre transacciones dentro del tenant del contexto.
pub trait UnidadDeTrabajo: Send + Sync {
    /// Transacción abierta. Implementa los puertos que se usan dentro de ella.
    type Transaccion: Transaccion;

    /// Inicia una transacción en el tenant de `ctx`. El tenant, el actor y el `request_id`
    /// quedan asociados a la transacción (los usa, por ejemplo, la auditoría).
    fn iniciar(&self, ctx: &Contexto) -> impl Future<Output = AppResult<Self::Transaccion>> + Send;
}

/// Transacción abierta. Si se suelta sin [`Transaccion::confirmar`], se revierte todo lo hecho.
pub trait Transaccion: Send {
    /// Confirma todos los cambios de la transacción (COMMIT).
    fn confirmar(self) -> impl Future<Output = AppResult<()>> + Send;
}

/// Registro de eventos de auditoría dentro de una transacción.
pub trait RegistroAuditoria: Send {
    /// Agrega el evento a la transacción; se persiste solo si la transacción se confirma.
    fn registrar(&mut self, evento: EventoAuditoria) -> impl Future<Output = AppResult<()>> + Send;
}

#[cfg(test)]
pub(crate) mod memoria {
    //! Dobles en memoria de la unidad de trabajo, reutilizables por las pruebas de los casos
    //! de uso: los eventos solo quedan en [`UnidadEnMemoria::confirmados`] tras `confirmar`.

    use std::sync::{Arc, Mutex};

    use super::*;

    /// Evento confirmado junto con el contexto de su transacción.
    pub type EventoConfirmado = (Contexto, EventoAuditoria);

    #[derive(Default, Clone)]
    pub struct UnidadEnMemoria {
        confirmados: Arc<Mutex<Vec<EventoConfirmado>>>,
    }

    impl UnidadEnMemoria {
        pub fn confirmados(&self) -> Vec<EventoConfirmado> {
            self.confirmados.lock().expect("mutex sano").clone()
        }
    }

    pub struct TransaccionEnMemoria {
        ctx: Contexto,
        pendientes: Vec<EventoAuditoria>,
        destino: Arc<Mutex<Vec<EventoConfirmado>>>,
    }

    impl UnidadDeTrabajo for UnidadEnMemoria {
        type Transaccion = TransaccionEnMemoria;

        async fn iniciar(&self, ctx: &Contexto) -> AppResult<TransaccionEnMemoria> {
            Ok(TransaccionEnMemoria {
                ctx: ctx.clone(),
                pendientes: Vec::new(),
                destino: Arc::clone(&self.confirmados),
            })
        }
    }

    impl Transaccion for TransaccionEnMemoria {
        async fn confirmar(self) -> AppResult<()> {
            let mut destino = self.destino.lock().expect("mutex sano");
            destino.extend(self.pendientes.into_iter().map(|e| (self.ctx.clone(), e)));
            Ok(())
        }
    }

    impl RegistroAuditoria for TransaccionEnMemoria {
        async fn registrar(&mut self, evento: EventoAuditoria) -> AppResult<()> {
            self.pendientes.push(evento);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use domain::TenantId;
    use uuid::Uuid;

    use super::{memoria::UnidadEnMemoria, *};
    use crate::auditoria::AccionAuditoria;

    const ACCION: AccionAuditoria = AccionAuditoria::new("pruebas.recurso.crear");

    fn evento() -> EventoAuditoria {
        EventoAuditoria::ok(ACCION, "recurso", Uuid::now_v7())
    }

    #[tokio::test]
    async fn auditoria_se_persiste_solo_al_confirmar() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());

        let mut tx = unidad.iniciar(&ctx).await.expect("inicia");
        tx.registrar(evento()).await.expect("registra");
        assert!(unidad.confirmados().is_empty());

        tx.confirmar().await.expect("confirma");
        let confirmados = unidad.confirmados();
        assert_eq!(confirmados.len(), 1);
        assert_eq!(confirmados[0].0, ctx);
    }

    #[tokio::test]
    async fn transaccion_soltada_sin_confirmar_no_deja_auditoria() {
        let unidad = UnidadEnMemoria::default();

        {
            let mut tx = unidad
                .iniciar(&Contexto::new(TenantId::nuevo()))
                .await
                .expect("inicia");
            tx.registrar(evento()).await.expect("registra");
        }

        assert!(unidad.confirmados().is_empty());
    }
}
