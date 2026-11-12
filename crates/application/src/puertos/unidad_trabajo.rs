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
    use domain::{
        Almacen, Ambito, ArchivoId, AsignacionRolId, ConjuntoPermisos, CuentaId,
        FinalidadTratamiento, Identificador, Modulo, OrigenAsignacion, PersonaId, RolId, TenantId,
        permisos::{CODIGO_ADMINISTRADOR, ROLES_PLANTILLA},
    };

    use super::*;
    use crate::{
        AppError,
        puertos::{
            archivos::{Archivo, RepositorioArchivos},
            identidad::{Asignacion, CuentaResumen, RepositorioRoles, Rol},
            modulos::{PeriodoModulo, RepositorioModulos},
            proteccion_datos::{
                Autorizacion, PoliticaTratamiento, RepositorioProteccionDatos, SolicitudTitular,
            },
        },
    };

    pub mod acceso;
    pub mod asambleas;
    pub mod campanas;
    mod censo;
    pub mod comunicados;
    mod copropiedades;
    pub mod directorio;
    pub mod gestion;
    pub mod publicidad;
    mod reservas;

    /// Datos de negocio en memoria. La transacción trabaja sobre una copia y la reemplaza
    /// al confirmar.
    #[derive(Default, Clone)]
    pub struct Datos {
        pub agrupaciones: Vec<crate::puertos::copropiedades::Agrupacion>,
        pub bienes: BTreeMap<domain::BienPrivadoId, crate::puertos::copropiedades::BienPrivado>,
        pub usos_exclusivos: Vec<crate::puertos::copropiedades::UsoExclusivo>,
        pub cuadros: Vec<crate::puertos::copropiedades::CuadroCoeficientes>,
        pub coeficientes: BTreeMap<
            domain::CuadroCoeficientesId,
            BTreeMap<domain::BienPrivadoId, domain::Coeficiente>,
        >,
        pub personas: Vec<crate::puertos::copropiedades::Persona>,
        pub titularidades: Vec<crate::puertos::copropiedades::Titularidad>,
        pub reuniones_instaladas: u64,
        pub residencias: Vec<crate::puertos::censo::Residencia>,
        pub vehiculos: Vec<crate::puertos::censo::Vehiculo>,
        pub mascotas: Vec<crate::puertos::censo::Mascota>,
        pub cambios_censo: Vec<crate::puertos::censo::CambioCenso>,
        pub recursos: Vec<crate::puertos::reservas::Recurso>,
        pub reservas: Vec<crate::puertos::reservas::Reserva>,
        pub entregas: Vec<crate::puertos::reservas::Entrega>,
        pub cargos: Vec<crate::puertos::reservas::CargoPendiente>,
        pub restricciones: Vec<crate::puertos::reservas::Restriccion>,
        pub asignaciones_parqueadero: Vec<crate::puertos::reservas::AsignacionParqueadero>,
        pub sorteos: Vec<crate::puertos::reservas::Sorteo>,
        pub inscripciones_sorteo: Vec<crate::puertos::reservas::InscripcionSorteo>,
        pub recordadas: Vec<domain::ReservaId>,
        pub archivos: BTreeMap<ArchivoId, Archivo>,
        pub politicas: Vec<PoliticaTratamiento>,
        /// Autorizaciones con su fecha de revocación.
        pub autorizaciones: Vec<(Autorizacion, Option<DateTime<Utc>>)>,
        pub solicitudes: Vec<SolicitudTitular>,
        pub modulos: Vec<PeriodoModulo>,
        /// Roles propios de copropiedades (las plantillas salen de `ROLES_PLANTILLA`).
        pub roles: Vec<Rol>,
        pub asignaciones: Vec<(TenantId, Asignacion)>,
        /// Tickets, mantenimiento y chat (planes 07b y 07c).
        pub gestion: gestion::DatosGestion,
        /// Asambleas y votaciones (plan 06).
        pub asambleas: asambleas::DatosAsambleas,
        /// Control de acceso, visitas y correspondencia (plan 05).
        pub acceso: acceso::DatosAcceso,
        /// Comunicados de la administración (plan 10).
        pub comunicados: comunicados::DatosComunicados,
        /// Campañas de actualización del censo (plan 10).
        pub campanas: campanas::DatosCampanas,
        /// Directorio de comercios y servicios (plan 08).
        pub directorio: directorio::DatosDirectorio,
        /// Publicidad en la copropiedad (plan 08, fases 4 a 6).
        pub publicidad: publicidad::DatosPublicidad,
    }

    /// Plantillas con ids fijos (en la BD se derivan del código).
    fn plantillas() -> Vec<Rol> {
        ROLES_PLANTILLA
            .iter()
            .enumerate()
            .map(|(i, p)| Rol {
                id: RolId::from_uuid(uuid::Uuid::from_u128(i as u128 + 1)),
                tenant_id: None,
                codigo: p.codigo.into(),
                nombre: p.nombre.into(),
                descripcion: None,
                ambito: p.ambito,
                es_plantilla: true,
                permisos: p.conjunto(),
            })
            .collect()
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

        /// Habilita un módulo desde siempre y sin fecha de fin.
        pub fn habilitar(&self, modulo: Modulo) {
            self.datos
                .lock()
                .expect("mutex sano")
                .modulos
                .push(PeriodoModulo {
                    modulo,
                    desde: DateTime::<Utc>::MIN_UTC,
                    hasta: None,
                });
        }

        /// Modifica los datos confirmados (preparar escenarios de prueba).
        pub fn con_datos(&self, f: impl FnOnce(&mut Datos)) {
            f(&mut self.datos.lock().expect("mutex sano"));
        }

        /// Simula reuniones instaladas en la copropiedad.
        pub fn fijar_reuniones_instaladas(&self, n: u64) {
            self.datos.lock().expect("mutex sano").reuniones_instaladas = n;
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

    impl crate::puertos::auditoria::LecturaAuditoria for TransaccionEnMemoria {
        async fn consultar_auditoria(
            &mut self,
            f: &crate::puertos::auditoria::FiltroAuditoria,
            antes_de: Option<i64>,
            limite: i64,
        ) -> AppResult<Vec<crate::puertos::auditoria::RegistroAuditado>> {
            let tenant = self.ctx.tenant_id;
            let mut v: Vec<_> = self
                .destino
                .confirmados()
                .into_iter()
                .enumerate()
                .filter(|(_, (c, _))| c.tenant_id == tenant)
                .map(|(i, (c, e))| crate::puertos::auditoria::RegistroAuditado {
                    id: i64::try_from(i).unwrap_or(0) + 1,
                    ocurrido_en: DateTime::<Utc>::MIN_UTC,
                    actor_id: c.actor_id,
                    accion: e.accion.as_str().to_owned(),
                    entidad: e.entidad.to_owned(),
                    entidad_id: e.entidad_id,
                    resultado: e.resultado.as_str().to_owned(),
                    detalle: e.detalle.a_json(),
                    request_id: c.request_id,
                })
                .filter(|r| f.entidad.as_ref().is_none_or(|x| &r.entidad == x))
                .filter(|r| f.entidad_id.is_none_or(|x| r.entidad_id == Some(x)))
                .filter(|r| {
                    f.accion.as_ref().is_none_or(|a| {
                        if a.ends_with('.') {
                            r.accion.starts_with(a.as_str())
                        } else {
                            &r.accion == a
                        }
                    })
                })
                .filter(|r| f.actor_id.is_none_or(|x| r.actor_id == Some(x)))
                .filter(|r| antes_de.is_none_or(|x| r.id < x))
                .collect();
            v.sort_by_key(|r| std::cmp::Reverse(r.id));
            v.truncate(usize::try_from(limite).unwrap_or(usize::MAX));
            Ok(v)
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

        async fn historial_politicas(&mut self) -> AppResult<Vec<PoliticaTratamiento>> {
            let mut v = self.datos.politicas.clone();
            v.sort_by_key(|p| std::cmp::Reverse(p.version));
            Ok(v)
        }

        async fn autorizaciones_de_persona(
            &mut self,
            persona_id: PersonaId,
        ) -> AppResult<Vec<crate::puertos::proteccion_datos::AutorizacionRegistrada>> {
            let politicas = self.datos.politicas.clone();
            let mut v: Vec<_> = self
                .datos
                .autorizaciones
                .iter()
                .filter(|(a, _)| a.persona_id == persona_id)
                .map(
                    |(a, revocada)| crate::puertos::proteccion_datos::AutorizacionRegistrada {
                        autorizacion: a.clone(),
                        politica_version: politicas
                            .iter()
                            .find(|p| Some(p.id) == a.politica_id)
                            .map_or(0, |p| p.version),
                        revocada_en: *revocada,
                    },
                )
                .collect();
            v.sort_by_key(|r| std::cmp::Reverse(r.autorizacion.otorgada_en));
            Ok(v)
        }

        async fn solicitudes(
            &mut self,
            f: &crate::puertos::proteccion_datos::FiltroSolicitudes,
            p: crate::SolicitudPagina,
        ) -> AppResult<crate::Pagina<SolicitudTitular>> {
            let mut v: Vec<SolicitudTitular> = self
                .datos
                .solicitudes
                .iter()
                .filter(|s| f.estado.is_none_or(|e| s.estado == e))
                .filter(|s| f.persona_id.is_none_or(|x| s.persona_id == x))
                .filter(|s| {
                    f.vencen_hasta.is_none_or(|h| {
                        s.estado != domain::EstadoSolicitudTitular::Respondida && s.vence() <= h
                    })
                })
                .filter(|s| p.despues().is_none_or(|d| s.id.as_uuid() < d))
                .cloned()
                .collect();
            v.sort_by_key(|s| std::cmp::Reverse(s.id.as_uuid()));
            v.truncate(usize::try_from(p.filas_a_consultar()).unwrap_or(usize::MAX));
            Ok(crate::Pagina::desde_filas(v, p, |s| s.id.as_uuid()))
        }

        async fn solicitud_para_actualizar(
            &mut self,
            id: domain::SolicitudTitularId,
        ) -> AppResult<Option<SolicitudTitular>> {
            Ok(self.datos.solicitudes.iter().find(|s| s.id == id).cloned())
        }

        async fn actualizar_solicitud(&mut self, solicitud: &SolicitudTitular) -> AppResult<()> {
            let s = self
                .datos
                .solicitudes
                .iter_mut()
                .find(|s| s.id == solicitud.id)
                .ok_or(AppError::NoEncontrado("solicitud"))?;
            *s = solicitud.clone();
            Ok(())
        }
    }

    impl TransaccionEnMemoria {
        fn roles_visibles(&self) -> Vec<Rol> {
            plantillas()
                .into_iter()
                .filter(|r| r.ambito == Ambito::Copropiedad)
                .chain(
                    self.datos
                        .roles
                        .iter()
                        .filter(|r| r.tenant_id == Some(self.ctx.tenant_id))
                        .cloned(),
                )
                .collect()
        }

        fn asignaciones_tenant(&self) -> impl Iterator<Item = &Asignacion> {
            self.datos
                .asignaciones
                .iter()
                .filter(|(t, _)| *t == self.ctx.tenant_id)
                .map(|(_, a)| a)
        }
    }

    impl RepositorioRoles for TransaccionEnMemoria {
        async fn roles(&mut self) -> AppResult<Vec<Rol>> {
            Ok(self.roles_visibles())
        }

        async fn rol(&mut self, id: RolId) -> AppResult<Option<Rol>> {
            Ok(self.roles_visibles().into_iter().find(|r| r.id == id))
        }

        async fn rol_por_codigo(&mut self, codigo: &str) -> AppResult<Option<Rol>> {
            Ok(self
                .roles_visibles()
                .into_iter()
                .find(|r| r.codigo == codigo))
        }

        async fn insertar_rol(&mut self, rol: &Rol) -> AppResult<()> {
            if self.roles_visibles().iter().any(|r| r.codigo == rol.codigo) {
                return Err(AppError::Conflicto("código de rol repetido".into()));
            }
            self.datos.roles.push(rol.clone());
            Ok(())
        }

        async fn reemplazar_permisos(
            &mut self,
            rol: RolId,
            permisos: &ConjuntoPermisos,
        ) -> AppResult<()> {
            if let Some(r) = self.datos.roles.iter_mut().find(|r| r.id == rol) {
                r.permisos = permisos.clone();
            }
            Ok(())
        }

        async fn insertar_asignacion(&mut self, a: &Asignacion) -> AppResult<()> {
            let tenant = self.ctx.tenant_id;
            self.datos.asignaciones.push((tenant, a.clone()));
            Ok(())
        }

        async fn asignacion(&mut self, id: AsignacionRolId) -> AppResult<Option<Asignacion>> {
            Ok(self.asignaciones_tenant().find(|a| a.id == id).cloned())
        }

        async fn asignaciones(&mut self, cuenta: Option<CuentaId>) -> AppResult<Vec<Asignacion>> {
            Ok(self
                .asignaciones_tenant()
                .filter(|a| cuenta.is_none_or(|c| a.cuenta_id == c))
                .cloned()
                .collect())
        }

        async fn terminar_asignacion(
            &mut self,
            id: AsignacionRolId,
            ahora: DateTime<Utc>,
        ) -> AppResult<()> {
            let tenant = self.ctx.tenant_id;
            for (t, a) in &mut self.datos.asignaciones {
                if *t == tenant && a.id == id && a.vigente_hasta.is_none_or(|h| h > ahora) {
                    a.vigente_hasta = Some(ahora.max(a.vigente_desde));
                }
            }
            Ok(())
        }

        async fn terminar_por_origen(
            &mut self,
            origen: OrigenAsignacion,
            origen_id: uuid::Uuid,
            ahora: DateTime<Utc>,
        ) -> AppResult<u64> {
            let tenant = self.ctx.tenant_id;
            let mut n = 0;
            for (t, a) in &mut self.datos.asignaciones {
                if *t == tenant
                    && a.origen == origen
                    && a.origen_id == Some(origen_id)
                    && a.vigente_hasta.is_none_or(|h| h > ahora)
                {
                    a.vigente_hasta = Some(ahora.max(a.vigente_desde));
                    n += 1;
                }
            }
            Ok(n)
        }

        async fn permisos_de_cuenta(
            &mut self,
            cuenta: CuentaId,
            ahora: DateTime<Utc>,
        ) -> AppResult<ConjuntoPermisos> {
            let roles = self.roles_visibles();
            Ok(self
                .asignaciones_tenant()
                .filter(|a| a.cuenta_id == cuenta && a.vigente_en(ahora))
                .filter_map(|a| roles.iter().find(|r| r.id == a.rol_id))
                .flat_map(|r| r.permisos.iter())
                .collect())
        }

        async fn administradores_vigentes(&mut self, ahora: DateTime<Utc>) -> AppResult<u64> {
            let admin = self
                .roles_visibles()
                .into_iter()
                .find(|r| r.es_plantilla && r.codigo == CODIGO_ADMINISTRADOR)
                .map(|r| r.id);
            Ok(self
                .asignaciones_tenant()
                .filter(|a| Some(a.rol_id) == admin && a.vigente_en(ahora))
                .count() as u64)
        }

        async fn buscar_cuenta(
            &mut self,
            _identificador: &Identificador,
        ) -> AppResult<Option<CuentaResumen>> {
            Ok(None)
        }

        async fn insertar_copropiedad(
            &mut self,
            _alta: &crate::puertos::copropiedades::AltaCopropiedad,
        ) -> AppResult<()> {
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
