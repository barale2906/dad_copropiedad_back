//! Doble en memoria del repositorio del censo.

use chrono::{DateTime, Utc};
use domain::{
    BienPrivadoId, CambioCensoId, CuentaId, EstadoCambioCenso, EstadoMascota, MascotaId, PersonaId,
    Placa, ResidenciaId, VehiculoId,
};

use super::TransaccionEnMemoria;
use crate::{
    AppError, AppResult,
    puertos::{
        RepositorioCenso,
        censo::{
            CambioCenso, FilaLibroPropietarios, FilaLibroResidentes, Mascota, Residencia, Vehiculo,
        },
    },
};

impl RepositorioCenso for TransaccionEnMemoria {
    async fn residencias(
        &mut self,
        bien: Option<BienPrivadoId>,
        incluir_terminadas: bool,
    ) -> AppResult<Vec<Residencia>> {
        Ok(self
            .datos
            .residencias
            .iter()
            .filter(|r| bien.is_none_or(|b| r.bien_privado_id == b))
            .filter(|r| incluir_terminadas || r.vigente_hasta.is_none())
            .cloned()
            .collect())
    }

    async fn residencia(&mut self, id: ResidenciaId) -> AppResult<Option<Residencia>> {
        Ok(self.datos.residencias.iter().find(|r| r.id == id).cloned())
    }

    async fn insertar_residencia(&mut self, r: &Residencia) -> AppResult<()> {
        self.datos.residencias.push(r.clone());
        Ok(())
    }

    async fn terminar_residencia(
        &mut self,
        id: ResidenciaId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        if let Some(r) = self.datos.residencias.iter_mut().find(|r| r.id == id) {
            r.vigente_hasta = Some(ahora.max(r.vigente_desde));
        }
        Ok(())
    }

    async fn vehiculos(
        &mut self,
        bien: Option<BienPrivadoId>,
        placa: Option<&Placa>,
        incluir_retirados: bool,
    ) -> AppResult<Vec<Vehiculo>> {
        Ok(self
            .datos
            .vehiculos
            .iter()
            .filter(|v| bien.is_none_or(|b| v.bien_privado_id == b))
            .filter(|v| placa.is_none_or(|p| v.placa.as_ref() == Some(p)))
            .filter(|v| incluir_retirados || v.vigente_hasta.is_none())
            .cloned()
            .collect())
    }

    async fn vehiculo(&mut self, id: VehiculoId) -> AppResult<Option<Vehiculo>> {
        Ok(self.datos.vehiculos.iter().find(|v| v.id == id).cloned())
    }

    async fn insertar_vehiculo(&mut self, v: &Vehiculo) -> AppResult<()> {
        if v.placa.is_some()
            && self
                .datos
                .vehiculos
                .iter()
                .any(|x| x.vigente_hasta.is_none() && x.placa == v.placa)
        {
            return Err(AppError::Conflicto("placa vigente repetida".into()));
        }
        self.datos.vehiculos.push(v.clone());
        Ok(())
    }

    async fn retirar_vehiculo(&mut self, id: VehiculoId, ahora: DateTime<Utc>) -> AppResult<()> {
        if let Some(v) = self.datos.vehiculos.iter_mut().find(|v| v.id == id) {
            v.vigente_hasta = Some(ahora.max(v.vigente_desde));
        }
        Ok(())
    }

    async fn mascotas(
        &mut self,
        bien: Option<BienPrivadoId>,
        solo_activas: bool,
    ) -> AppResult<Vec<Mascota>> {
        Ok(self
            .datos
            .mascotas
            .iter()
            .filter(|m| bien.is_none_or(|b| m.bien_privado_id == b))
            .filter(|m| !solo_activas || m.estado == EstadoMascota::Activa)
            .cloned()
            .collect())
    }

    async fn mascota(&mut self, id: MascotaId) -> AppResult<Option<Mascota>> {
        Ok(self.datos.mascotas.iter().find(|m| m.id == id).cloned())
    }

    async fn insertar_mascota(&mut self, m: &Mascota) -> AppResult<()> {
        self.datos.mascotas.push(m.clone());
        Ok(())
    }

    async fn actualizar_mascota(&mut self, m: &Mascota) -> AppResult<()> {
        if let Some(x) = self.datos.mascotas.iter_mut().find(|x| x.id == m.id) {
            *x = m.clone();
        }
        Ok(())
    }

    async fn cambios(
        &mut self,
        estado: Option<EstadoCambioCenso>,
        propuesto_por: Option<CuentaId>,
    ) -> AppResult<Vec<CambioCenso>> {
        Ok(self
            .datos
            .cambios_censo
            .iter()
            .filter(|c| estado.is_none_or(|e| c.estado == e))
            .filter(|c| propuesto_por.is_none_or(|p| c.propuesto_por == p))
            .cloned()
            .collect())
    }

    async fn cambio(&mut self, id: CambioCensoId) -> AppResult<Option<CambioCenso>> {
        Ok(self
            .datos
            .cambios_censo
            .iter()
            .find(|c| c.id == id)
            .cloned())
    }

    async fn insertar_cambio(&mut self, c: &CambioCenso) -> AppResult<()> {
        self.datos.cambios_censo.push(c.clone());
        Ok(())
    }

    async fn resolver_cambio(&mut self, c: &CambioCenso) -> AppResult<()> {
        if let Some(x) = self.datos.cambios_censo.iter_mut().find(|x| x.id == c.id) {
            *x = c.clone();
        }
        Ok(())
    }

    async fn cuenta_vinculada_a_bien(
        &mut self,
        cuenta: CuentaId,
        bien: BienPrivadoId,
    ) -> AppResult<bool> {
        let personas: Vec<PersonaId> = self
            .datos
            .personas
            .iter()
            .filter(|p| p.cuenta_id == Some(cuenta))
            .map(|p| p.id)
            .collect();
        let titular = self.datos.titularidades.iter().any(|t| {
            t.bien_privado_id == bien
                && t.vigente_hasta.is_none()
                && personas.contains(&t.persona_id)
        });
        let residente = self.datos.residencias.iter().any(|r| {
            r.bien_privado_id == bien
                && r.vigente_hasta.is_none()
                && personas.contains(&r.persona_id)
        });
        Ok(titular || residente)
    }

    async fn libro_propietarios(
        &mut self,
        _corte: DateTime<Utc>,
    ) -> AppResult<Vec<FilaLibroPropietarios>> {
        Ok(Vec::new())
    }

    async fn libro_residentes(
        &mut self,
        _corte: DateTime<Utc>,
    ) -> AppResult<Vec<FilaLibroResidentes>> {
        Ok(Vec::new())
    }

    async fn residentes_sin_autorizacion(&mut self) -> AppResult<Vec<PersonaId>> {
        Ok(self
            .datos
            .residencias
            .iter()
            .filter(|r| r.vigente_hasta.is_none())
            .filter(|r| {
                !self
                    .datos
                    .autorizaciones
                    .iter()
                    .any(|(a, revocada)| revocada.is_none() && a.persona_id == r.persona_id)
            })
            .map(|r| r.persona_id)
            .collect())
    }
}
