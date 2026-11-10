//! Doble en memoria de las campañas de actualización del censo (plan 10, fase 5). Los
//! destinatarios se fijan en [`DatosCampanas::destinatarios`] (la regla es SQL y se prueba en
//! `infra-postgres`).

use chrono::{DateTime, Utc};
use domain::{BienPrivadoId, CampanaCensoId, CuentaId, PersonaId, RespuestaCampanaCenso};

use super::TransaccionEnMemoria;
use crate::{
    AppResult,
    puertos::{
        RepositorioCampanasCenso,
        censo::{
            CampanaCenso, DestinatarioCampanaCenso, RespuestaCenso, RespuestaCensoListada,
            ResumenCampanaCenso,
        },
    },
};

#[derive(Default, Clone)]
pub struct DatosCampanas {
    pub campanas: Vec<CampanaCenso>,
    pub respuestas: Vec<RespuestaCenso>,
    pub destinatarios: Vec<DestinatarioCampanaCenso>,
}

impl RepositorioCampanasCenso for TransaccionEnMemoria {
    async fn insertar_campana_censo(&mut self, c: &CampanaCenso) -> AppResult<()> {
        self.datos.campanas.campanas.push(c.clone());
        Ok(())
    }

    async fn campana_censo(&mut self, id: CampanaCensoId) -> AppResult<Option<CampanaCenso>> {
        Ok(self
            .datos
            .campanas
            .campanas
            .iter()
            .find(|c| c.id == id)
            .cloned())
    }

    async fn campanas_censo(&mut self) -> AppResult<Vec<ResumenCampanaCenso>> {
        let d = &self.datos.campanas;
        let mut v: Vec<ResumenCampanaCenso> = d
            .campanas
            .iter()
            .map(|c| {
                let de = d.respuestas.iter().filter(|r| r.campana == c.id);
                let cambios = |r: &&RespuestaCenso| r.respuesta == RespuestaCampanaCenso::Cambios;
                ResumenCampanaCenso {
                    campana: c.clone(),
                    confirmados: u32::try_from(de.clone().filter(|r| !cambios(r)).count())
                        .unwrap_or(u32::MAX),
                    con_cambios: u32::try_from(de.clone().filter(cambios).count())
                        .unwrap_or(u32::MAX),
                    por_atender: u32::try_from(
                        de.filter(cambios)
                            .filter(|r| r.atendida_en.is_none())
                            .count(),
                    )
                    .unwrap_or(u32::MAX),
                }
            })
            .collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.campana.id));
        Ok(v)
    }

    async fn destinatarios_campana_censo(
        &mut self,
        _ahora: DateTime<Utc>,
    ) -> AppResult<Vec<DestinatarioCampanaCenso>> {
        Ok(self.datos.campanas.destinatarios.clone())
    }

    async fn guardar_respuesta_censo(&mut self, r: &RespuestaCenso) -> AppResult<()> {
        let v = &mut self.datos.campanas.respuestas;
        v.retain(|x| !(x.campana == r.campana && x.persona == r.persona && x.bien == r.bien));
        v.push(r.clone());
        Ok(())
    }

    async fn respuestas_censo(
        &mut self,
        campana: CampanaCensoId,
    ) -> AppResult<Vec<RespuestaCensoListada>> {
        let d = &self.datos;
        Ok(d.campanas
            .respuestas
            .iter()
            .filter(|r| r.campana == campana)
            .map(|r| RespuestaCensoListada {
                respuesta: r.clone(),
                nombre: d
                    .personas
                    .iter()
                    .find(|p| p.id == r.persona)
                    .map(|p| p.nombre.clone())
                    .unwrap_or_default(),
                codigo: d
                    .bienes
                    .get(&r.bien)
                    .map(|b| b.codigo.as_str().to_owned())
                    .unwrap_or_default(),
            })
            .collect())
    }

    async fn atender_respuesta_censo(
        &mut self,
        campana: CampanaCensoId,
        persona: PersonaId,
        bien: BienPrivadoId,
        por: Option<CuentaId>,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        let r = self.datos.campanas.respuestas.iter_mut().find(|r| {
            r.campana == campana
                && r.persona == persona
                && r.bien == bien
                && r.respuesta == RespuestaCampanaCenso::Cambios
        });
        Ok(r.map(|r| {
            r.atendida_en = Some(ahora);
            r.atendida_por = por;
        })
        .is_some())
    }
}
