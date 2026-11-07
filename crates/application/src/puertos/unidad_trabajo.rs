//! Unidad de trabajo: agrupa en **una sola transacción** el cambio de negocio, su auditoría y
//! los mensajes del outbox, todo dentro del tenant del [`Contexto`].
//!
//! Patrón de uso en un caso de uso:
//!
//! ```ignore
//! let mut tx = unidad.iniciar(ctx).await?;      // fija el tenant (RLS)
//! tx.guardar(&bien_privado).await?;             // repositorio implementado por la transacción
//! tx.registrar(evento_auditoria).await?;        // RegistroAuditoria, misma transacción
//! tx.encolar(notificacion).await?;              // RegistroOutbox: sale solo si hay COMMIT
//! tx.confirmar().await?;                        // COMMIT; si no se llama, ROLLBACK al soltarla
//! ```
//!
//! La transacción implementa los puertos que necesita cada caso de uso (empezando por
//! [`RegistroAuditoria`] y [`RegistroOutbox`]); el caso de uso los exige con *bounds* sobre
//! [`UnidadDeTrabajo::Transaccion`]. Así la capa de aplicación no conoce SQL y se prueba con
//! dobles en memoria.

use std::future::Future;

use crate::{AppResult, Contexto, auditoria::EventoAuditoria, outbox::EventoSaliente};

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

/// Encolado de mensajes salientes (outbox) dentro de una transacción: el mensaje solo sale si
/// el hecho que lo origina se confirma.
pub trait RegistroOutbox: Send {
    /// Agrega el mensaje al outbox del tenant de la transacción.
    fn encolar(&mut self, evento: EventoSaliente) -> impl Future<Output = AppResult<()>> + Send;
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

    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    use chrono::{DateTime, Utc};
    use domain::{Almacen, ArchivoId, FinalidadTratamiento, Modulo, PersonaId};

    use super::*;
    use crate::{
        AppError,
        puertos::{
            archivos::{Archivo, RepositorioArchivos},
            modulos::{PeriodoModulo, RepositorioModulos},
            proteccion_datos::{
                Autorizacion, PoliticaTratamiento, RepositorioProteccionDatos, SolicitudTitular,
            },
        },
    };

    /// Datos de negocio en memoria. La transacción trabaja sobre una copia y la reemplaza
    /// al confirmar.
    #[derive(Default, Clone)]
    pub struct Datos {
        pub archivos: BTreeMap<ArchivoId, Archivo>,
        pub politicas: Vec<PoliticaTratamiento>,
        /// Autorizaciones con su fecha de revocación.
        pub autorizaciones: Vec<(Autorizacion, Option<DateTime<Utc>>)>,
        pub solicitudes: Vec<SolicitudTitular>,
        pub modulos: Vec<PeriodoModulo>,
    }

    /// Evento confirmado junto con el contexto de su transacción.
    pub type EventoConfirmado = (Contexto, EventoAuditoria);

    #[derive(Default, Clone)]
    pub struct UnidadEnMemoria {
        confirmados: Arc<Mutex<Vec<EventoConfirmado>>>,
        outbox: Arc<Mutex<Vec<(Contexto, EventoSaliente)>>>,
        datos: Arc<Mutex<Datos>>,
        /// Si es `true`, `confirmar` falla (simula una caída de la BD al hacer COMMIT).
        falla_al_confirmar: bool,
    }

    impl UnidadEnMemoria {
        /// Unidad cuyas transacciones fallan al confirmar.
        pub fn que_falla_al_confirmar() -> Self {
            Self {
                falla_al_confirmar: true,
                ..Self::default()
            }
        }

        pub fn confirmados(&self) -> Vec<EventoConfirmado> {
            self.confirmados.lock().expect("mutex sano").clone()
        }

        pub fn outbox(&self) -> Vec<(Contexto, EventoSaliente)> {
            self.outbox.lock().expect("mutex sano").clone()
        }

        /// Datos confirmados.
        pub fn datos(&self) -> Datos {
            self.datos.lock().expect("mutex sano").clone()
        }

        pub fn archivos(&self) -> Vec<Archivo> {
            self.datos().archivos.into_values().collect()
        }
    }

    pub struct TransaccionEnMemoria {
        ctx: Contexto,
        auditoria: Vec<EventoAuditoria>,
        salientes: Vec<EventoSaliente>,
        /// Copia de los datos confirmados; reemplaza al original solo al confirmar.
        datos: Datos,
        destino: UnidadEnMemoria,
    }

    impl UnidadDeTrabajo for UnidadEnMemoria {
        type Transaccion = TransaccionEnMemoria;

        async fn iniciar(&self, ctx: &Contexto) -> AppResult<TransaccionEnMemoria> {
            Ok(TransaccionEnMemoria {
                ctx: ctx.clone(),
                auditoria: Vec::new(),
                salientes: Vec::new(),
                datos: self.datos(),
                destino: self.clone(),
            })
        }
    }

    impl Transaccion for TransaccionEnMemoria {
        async fn confirmar(self) -> AppResult<()> {
            if self.destino.falla_al_confirmar {
                return Err(AppError::infra(std::io::Error::other("COMMIT fallido")));
            }
            *self.destino.datos.lock().expect("mutex sano") = self.datos;
            let ctx = self.ctx;
            self.destino
                .confirmados
                .lock()
                .expect("mutex sano")
                .extend(self.auditoria.into_iter().map(|e| (ctx.clone(), e)));
            self.destino
                .outbox
                .lock()
                .expect("mutex sano")
                .extend(self.salientes.into_iter().map(|e| (ctx.clone(), e)));
            Ok(())
        }
    }

    impl RegistroAuditoria for TransaccionEnMemoria {
        async fn registrar(&mut self, evento: EventoAuditoria) -> AppResult<()> {
            self.auditoria.push(evento);
            Ok(())
        }
    }

    impl RepositorioArchivos for TransaccionEnMemoria {
        async fn insertar(&mut self, archivo: &Archivo) -> AppResult<()> {
            self.datos.archivos.insert(archivo.id, archivo.clone());
            Ok(())
        }

        async fn buscar(&mut self, id: ArchivoId) -> AppResult<Option<Archivo>> {
            Ok(self.datos.archivos.get(&id).cloned())
        }

        async fn en_almacen(&mut self, almacen: Almacen, limite: u32) -> AppResult<Vec<Archivo>> {
            Ok(self
                .datos
                .archivos
                .values()
                .filter(|a| a.almacen == almacen)
                .take(limite as usize)
                .cloned()
                .collect())
        }

        async fn cambiar_almacen(&mut self, id: ArchivoId, almacen: Almacen) -> AppResult<()> {
            let archivo = self
                .datos
                .archivos
                .get_mut(&id)
                .ok_or(AppError::NoEncontrado("archivo"))?;
            archivo.almacen = almacen;
            Ok(())
        }
    }

    impl RepositorioProteccionDatos for TransaccionEnMemoria {
        async fn ultima_version_politica(&mut self) -> AppResult<u32> {
            Ok(self
                .datos
                .politicas
                .iter()
                .map(|p| p.version)
                .max()
                .unwrap_or(0))
        }

        async fn insertar_politica(&mut self, politica: &PoliticaTratamiento) -> AppResult<()> {
            self.datos.politicas.push(politica.clone());
            Ok(())
        }

        async fn politica_vigente(
            &mut self,
            ahora: DateTime<Utc>,
        ) -> AppResult<Option<PoliticaTratamiento>> {
            Ok(self
                .datos
                .politicas
                .iter()
                .filter(|p| p.vigente_desde <= ahora)
                .max_by_key(|p| p.version)
                .cloned())
        }

        async fn insertar_autorizacion(&mut self, a: &Autorizacion) -> AppResult<bool> {
            let ya_vigente = self.datos.autorizaciones.iter().any(|(x, revocada)| {
                revocada.is_none() && x.persona_id == a.persona_id && x.finalidad == a.finalidad
            });
            if !ya_vigente {
                self.datos.autorizaciones.push((a.clone(), None));
            }
            Ok(!ya_vigente)
        }

        async fn revocar_autorizaciones(
            &mut self,
            persona_id: PersonaId,
            finalidades: &[FinalidadTratamiento],
            ahora: DateTime<Utc>,
        ) -> AppResult<u64> {
            let mut revocadas = 0;
            for (a, revocada) in &mut self.datos.autorizaciones {
                if revocada.is_none()
                    && a.persona_id == persona_id
                    && finalidades.contains(&a.finalidad)
                {
                    *revocada = Some(ahora);
                    revocadas += 1;
                }
            }
            Ok(revocadas)
        }

        async fn finalidades_autorizadas(
            &mut self,
            persona_id: PersonaId,
        ) -> AppResult<Vec<FinalidadTratamiento>> {
            Ok(self
                .datos
                .autorizaciones
                .iter()
                .filter(|(a, revocada)| revocada.is_none() && a.persona_id == persona_id)
                .map(|(a, _)| a.finalidad)
                .collect())
        }

        async fn insertar_solicitud(&mut self, solicitud: &SolicitudTitular) -> AppResult<()> {
            self.datos.solicitudes.push(solicitud.clone());
            Ok(())
        }
    }

    impl RepositorioModulos for TransaccionEnMemoria {
        async fn modulo_habilitado(
            &mut self,
            modulo: Modulo,
            ahora: DateTime<Utc>,
        ) -> AppResult<bool> {
            Ok(self
                .datos
                .modulos
                .iter()
                .any(|p| p.modulo == modulo && p.cubre(ahora)))
        }

        async fn insertar_periodo(&mut self, periodo: &PeriodoModulo) -> AppResult<()> {
            self.datos.modulos.push(*periodo);
            Ok(())
        }

        async fn terminar_periodos(
            &mut self,
            modulo: Modulo,
            hasta: DateTime<Utc>,
        ) -> AppResult<u64> {
            let mut terminados = 0;
            for p in &mut self.datos.modulos {
                if p.modulo == modulo && p.desde < hasta && p.hasta.is_none_or(|h| h > hasta) {
                    p.hasta = Some(hasta);
                    terminados += 1;
                }
            }
            Ok(terminados)
        }
    }

    impl RegistroOutbox for TransaccionEnMemoria {
        async fn encolar(&mut self, evento: EventoSaliente) -> AppResult<()> {
            self.salientes.push(evento);
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
    async fn outbox_y_auditoria_se_confirman_juntos() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::new(TenantId::nuevo());
        let correo = crate::outbox::EventoSaliente {
            prioridad: crate::outbox::PrioridadSalida::Evento,
            mensaje: crate::outbox::MensajeOutbox::Correo(crate::puertos::correo::CorreoSaliente {
                para: domain::DireccionCorreo::new("a@b.co").expect("válida"),
                asunto: "Asunto".into(),
                texto: "Texto".into(),
                html: None,
            }),
        };

        let mut tx = unidad.iniciar(&ctx).await.expect("inicia");
        tx.registrar(evento()).await.expect("registra");
        tx.encolar(correo.clone()).await.expect("encola");
        assert!(unidad.outbox().is_empty());
        tx.confirmar().await.expect("confirma");

        assert_eq!(unidad.outbox(), vec![(ctx, correo)]);
        assert_eq!(unidad.confirmados().len(), 1);
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
