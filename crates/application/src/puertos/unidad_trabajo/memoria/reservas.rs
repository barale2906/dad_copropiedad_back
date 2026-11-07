//! Doble en memoria del repositorio de reservas, parqueaderos y sorteos. Repite las
//! restricciones de la BD que importan a los casos de uso (solape exclusivo, idempotencia,
//! asignaciones sin solape).

use chrono::{DateTime, Utc};
use domain::{
    AsignacionParqueaderoId, BienPrivadoId, CuentaId, EstadoSorteo, PersonaId, RecursoId,
    ReservaId, ResidenciaId, RestriccionId, SorteoId, TipoRecurso, VinculoSolicitante,
};

use super::TransaccionEnMemoria;
use crate::{
    AppError, AppResult,
    puertos::{
        RepositorioReservas,
        reservas::{
            AsignacionParqueadero, CargoPendiente, Entrega, FiltroReservas, InscripcionSorteo,
            Recurso, Reserva, Restriccion, Sorteo,
        },
    },
};

fn se_cruzan(a: &AsignacionParqueadero, b: &AsignacionParqueadero) -> bool {
    a.desde < b.hasta.unwrap_or(DateTime::<Utc>::MAX_UTC)
        && b.desde < a.hasta.unwrap_or(DateTime::<Utc>::MAX_UTC)
}

impl RepositorioReservas for TransaccionEnMemoria {
    async fn recursos(&mut self, tipo: Option<TipoRecurso>) -> AppResult<Vec<Recurso>> {
        Ok(self
            .datos
            .recursos
            .iter()
            .filter(|r| tipo.is_none_or(|t| r.tipo == t))
            .cloned()
            .collect())
    }

    async fn recurso(&mut self, id: RecursoId) -> AppResult<Option<Recurso>> {
        Ok(self.datos.recursos.iter().find(|r| r.id == id).cloned())
    }

    async fn recurso_para_reservar(&mut self, id: RecursoId) -> AppResult<Option<Recurso>> {
        self.recurso(id).await
    }

    async fn insertar_recurso(&mut self, r: &Recurso) -> AppResult<()> {
        let codigo = r.parqueadero.as_ref().map(|p| &p.codigo);
        if codigo.is_some()
            && self
                .datos
                .recursos
                .iter()
                .any(|x| x.parqueadero.as_ref().map(|p| &p.codigo) == codigo)
        {
            return Err(AppError::Conflicto("código repetido".into()));
        }
        self.datos.recursos.push(r.clone());
        Ok(())
    }

    async fn actualizar_recurso(&mut self, r: &Recurso) -> AppResult<()> {
        if let Some(x) = self.datos.recursos.iter_mut().find(|x| x.id == r.id) {
            *x = r.clone();
        }
        Ok(())
    }

    async fn reservas(&mut self, f: &FiltroReservas) -> AppResult<Vec<Reserva>> {
        Ok(self
            .datos
            .reservas
            .iter()
            .filter(|r| f.recurso_id.is_none_or(|x| r.recurso_id == x))
            .filter(|r| f.bien_privado_id.is_none_or(|x| r.bien_privado_id == x))
            .filter(|r| f.estado.is_none_or(|x| r.estado == x))
            .filter(|r| f.solicitada_por.is_none_or(|x| r.solicitada_por == Some(x)))
            .filter(|r| f.entre.is_none_or(|x| r.franja.se_solapa(&x)))
            .cloned()
            .collect())
    }

    async fn reserva(&mut self, id: ReservaId) -> AppResult<Option<Reserva>> {
        Ok(self.datos.reservas.iter().find(|r| r.id == id).cloned())
    }

    async fn reserva_por_clave(&mut self, clave: &str) -> AppResult<Option<Reserva>> {
        Ok(self
            .datos
            .reservas
            .iter()
            .find(|r| r.idempotency_key == clave)
            .cloned())
    }

    async fn insertar_reserva(&mut self, r: &Reserva) -> AppResult<()> {
        let solapa = r.exclusiva
            && self.datos.reservas.iter().any(|x| {
                x.exclusiva
                    && x.recurso_id == r.recurso_id
                    && x.estado.activa()
                    && x.franja.se_solapa(&r.franja)
            });
        if solapa
            || self
                .datos
                .reservas
                .iter()
                .any(|x| x.idempotency_key == r.idempotency_key)
        {
            return Err(AppError::Conflicto("franja ocupada".into()));
        }
        self.datos.reservas.push(r.clone());
        Ok(())
    }

    async fn actualizar_reserva(&mut self, r: &Reserva) -> AppResult<()> {
        if let Some(x) = self.datos.reservas.iter_mut().find(|x| x.id == r.id) {
            *x = r.clone();
        }
        Ok(())
    }

    async fn contar_reservas_bien(
        &mut self,
        recurso: RecursoId,
        bien: BienPrivadoId,
        desde: DateTime<Utc>,
        hasta: DateTime<Utc>,
    ) -> AppResult<u32> {
        Ok(self
            .datos
            .reservas
            .iter()
            .filter(|r| r.recurso_id == recurso && r.bien_privado_id == bien && r.estado.activa())
            .filter(|r| desde <= r.franja.inicio() && r.franja.inicio() < hasta)
            .count() as u32)
    }

    async fn vinculo_solicitante(
        &mut self,
        cuenta: CuentaId,
        bien: BienPrivadoId,
    ) -> AppResult<Option<(PersonaId, VinculoSolicitante)>> {
        let Some(persona) = self
            .datos
            .personas
            .iter()
            .find(|p| p.cuenta_id == Some(cuenta))
            .map(|p| p.id)
        else {
            return Ok(None);
        };
        let es_titular = self.datos.titularidades.iter().any(|t| {
            t.bien_privado_id == bien && t.persona_id == persona && t.vigente_hasta.is_none()
        });
        let residencia = self
            .datos
            .residencias
            .iter()
            .find(|r| {
                r.bien_privado_id == bien && r.persona_id == persona && r.vigente_hasta.is_none()
            })
            .map(|r| (r.calidad, r.reservas_autorizadas));
        Ok((es_titular || residencia.is_some()).then_some((
            persona,
            VinculoSolicitante {
                es_titular,
                residencia,
            },
        )))
    }

    async fn restricciones(&mut self, bien: Option<BienPrivadoId>) -> AppResult<Vec<Restriccion>> {
        Ok(self
            .datos
            .restricciones
            .iter()
            .filter(|r| bien.is_none_or(|b| r.bien_privado_id == b))
            .cloned()
            .collect())
    }

    async fn insertar_restriccion(&mut self, r: &Restriccion) -> AppResult<()> {
        self.datos.restricciones.push(r.clone());
        Ok(())
    }

    async fn levantar_restriccion(
        &mut self,
        id: RestriccionId,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        match self
            .datos
            .restricciones
            .iter_mut()
            .find(|r| r.id == id && r.hasta.is_none_or(|h| h > ahora))
        {
            Some(r) => {
                r.hasta = Some(ahora);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn insertar_entrega(&mut self, e: &Entrega) -> AppResult<()> {
        if self
            .datos
            .entregas
            .iter()
            .any(|x| x.reserva_id == e.reserva_id && x.tipo == e.tipo)
        {
            return Err(AppError::Conflicto("ya registrada".into()));
        }
        self.datos.entregas.push(e.clone());
        Ok(())
    }

    async fn entregas(&mut self, reserva: ReservaId) -> AppResult<Vec<Entrega>> {
        Ok(self
            .datos
            .entregas
            .iter()
            .filter(|e| e.reserva_id == reserva)
            .cloned()
            .collect())
    }

    async fn insertar_cargo(&mut self, c: &CargoPendiente) -> AppResult<()> {
        self.datos.cargos.push(c.clone());
        Ok(())
    }

    async fn autorizar_reservas(
        &mut self,
        residencia: ResidenciaId,
        autorizadas: bool,
    ) -> AppResult<()> {
        if let Some(r) = self
            .datos
            .residencias
            .iter_mut()
            .find(|r| r.id == residencia)
        {
            r.reservas_autorizadas = autorizadas;
        }
        Ok(())
    }

    async fn asignaciones_parqueadero(
        &mut self,
        recurso: Option<RecursoId>,
        bien: Option<BienPrivadoId>,
    ) -> AppResult<Vec<AsignacionParqueadero>> {
        Ok(self
            .datos
            .asignaciones_parqueadero
            .iter()
            .filter(|a| recurso.is_none_or(|r| a.recurso_id == r))
            .filter(|a| bien.is_none_or(|b| a.bien_privado_id == b))
            .cloned()
            .collect())
    }

    async fn asignacion_parqueadero(
        &mut self,
        id: AsignacionParqueaderoId,
    ) -> AppResult<Option<AsignacionParqueadero>> {
        Ok(self
            .datos
            .asignaciones_parqueadero
            .iter()
            .find(|a| a.id == id)
            .cloned())
    }

    async fn insertar_asignacion_parqueadero(
        &mut self,
        a: &AsignacionParqueadero,
    ) -> AppResult<()> {
        if self
            .datos
            .asignaciones_parqueadero
            .iter()
            .any(|x| x.recurso_id == a.recurso_id && se_cruzan(x, a))
        {
            return Err(AppError::Conflicto("parqueadero ya asignado".into()));
        }
        self.datos.asignaciones_parqueadero.push(a.clone());
        Ok(())
    }

    async fn terminar_asignacion_parqueadero(
        &mut self,
        id: AsignacionParqueaderoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        if let Some(a) = self
            .datos
            .asignaciones_parqueadero
            .iter_mut()
            .find(|a| a.id == id)
        {
            let fin = ahora.max(a.desde);
            if a.hasta.is_none_or(|h| h > fin) {
                a.hasta = Some(fin);
            }
        }
        Ok(())
    }

    async fn insertar_sorteo(&mut self, s: &Sorteo) -> AppResult<()> {
        self.datos.sorteos.push(s.clone());
        Ok(())
    }

    async fn sorteos(&mut self) -> AppResult<Vec<Sorteo>> {
        Ok(self.datos.sorteos.clone())
    }

    async fn sorteo(&mut self, id: SorteoId) -> AppResult<Option<Sorteo>> {
        Ok(self.datos.sorteos.iter().find(|s| s.id == id).cloned())
    }

    async fn inscripciones(&mut self, sorteo: SorteoId) -> AppResult<Vec<InscripcionSorteo>> {
        Ok(self
            .datos
            .inscripciones_sorteo
            .iter()
            .filter(|i| i.sorteo_id == sorteo)
            .cloned()
            .collect())
    }

    async fn insertar_inscripcion(&mut self, i: &InscripcionSorteo) -> AppResult<()> {
        if self
            .datos
            .inscripciones_sorteo
            .iter()
            .any(|x| x.sorteo_id == i.sorteo_id && x.bien_privado_id == i.bien_privado_id)
        {
            return Err(AppError::Conflicto("ya inscrito".into()));
        }
        self.datos.inscripciones_sorteo.push(i.clone());
        Ok(())
    }

    async fn guardar_resultado_sorteo(
        &mut self,
        id: SorteoId,
        resultado: &[BienPrivadoId],
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        if let Some(s) = self.datos.sorteos.iter_mut().find(|s| s.id == id) {
            s.estado = EstadoSorteo::Ejecutado;
            s.resultado = Some(resultado.to_vec());
            s.ejecutado_en = Some(ahora);
        }
        Ok(())
    }

    async fn reservas_por_recordar(
        &mut self,
        ahora: DateTime<Utc>,
        hasta: DateTime<Utc>,
    ) -> AppResult<Vec<Reserva>> {
        Ok(self
            .datos
            .reservas
            .iter()
            .filter(|r| r.estado == domain::EstadoReserva::Aprobada)
            .filter(|r| !self.datos.recordadas.contains(&r.id))
            .filter(|r| ahora < r.franja.inicio() && r.franja.inicio() <= hasta)
            .cloned()
            .collect())
    }

    async fn marcar_recordada(&mut self, id: ReservaId, _ahora: DateTime<Utc>) -> AppResult<()> {
        self.datos.recordadas.push(id);
        Ok(())
    }
}
