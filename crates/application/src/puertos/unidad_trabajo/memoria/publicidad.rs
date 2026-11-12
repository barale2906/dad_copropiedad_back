//! Doble en memoria de la publicidad en una copropiedad. Las piezas que alcanzan a la PH se
//! fijan en [`DatosPublicidad::piezas`] (el alcance es SQL y se prueba en `infra-postgres`);
//! aquí se exige el módulo `publicidad` y se llevan el consumo, los envíos y la frecuencia.

use chrono::{DateTime, Utc};
use domain::{
    AutorizacionId, CampanaId, CanalPublicidad, CreativoId, FinalidadTratamiento, FormatoCreativo,
    MedioAutorizacion, Modulo, PersonaId, PoliticaPublicidadId,
    publicidad::{Conteo, PiezaCandidata, puede_recibir_masivo},
};
use rust_decimal::Decimal;

use super::TransaccionEnMemoria;
use crate::{
    AppResult,
    outbox::EventoSaliente,
    puertos::{
        RepositorioPublicidad,
        proteccion_datos::Autorizacion,
        publicidad::{
            DestinatarioMasivo, PantallaPublicidad, PiezaPublicidad, PoliticaPublicidad,
            ReportePublicidad,
        },
    },
};

/// Una entrega registrada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntregaRegistrada {
    pub creativo: CreativoId,
    pub canal: CanalPublicidad,
    pub conteo: Conteo,
    pub costo: Decimal,
    pub finalizada: bool,
}

#[derive(Default, Clone)]
pub struct DatosPublicidad {
    pub piezas: Vec<PiezaPublicidad>,
    pub entregas: Vec<EntregaRegistrada>,
    pub politica: Option<PoliticaPublicidad>,
    /// Personas con datos de contacto (las que reciben son las autorizadas).
    pub personas: Vec<DestinatarioMasivo>,
    pub envios: Vec<(CampanaId, PersonaId, CanalPublicidad, DateTime<Utc>)>,
    pub encolados: Vec<(EventoSaliente, CampanaId)>,
    pub reportes: Vec<ReportePublicidad>,
    pub pantalla: Option<PantallaPublicidad>,
}

impl TransaccionEnMemoria {
    fn autorizada_publicidad(&self, persona: PersonaId) -> bool {
        self.datos.autorizaciones.iter().any(|(a, r)| {
            r.is_none()
                && a.persona_id == persona
                && a.finalidad == FinalidadTratamiento::Publicidad
        })
    }
}

impl RepositorioPublicidad for TransaccionEnMemoria {
    async fn piezas(
        &mut self,
        formatos: &[FormatoCreativo],
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<PiezaPublicidad>> {
        let con_modulo = self
            .datos
            .modulos
            .iter()
            .any(|p| p.modulo == Modulo::Publicidad && p.cubre(ahora));
        if !con_modulo {
            return Ok(Vec::new());
        }
        Ok(self
            .datos
            .publicidad
            .piezas
            .iter()
            .filter(|p| formatos.contains(&p.contenido.formato))
            .cloned()
            .collect())
    }

    async fn pieza_para_entrega(
        &mut self,
        creativo: CreativoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<Option<(PiezaCandidata, FormatoCreativo)>> {
        let todos = [
            FormatoCreativo::Banner,
            FormatoCreativo::TarjetaDestacada,
            FormatoCreativo::OfertaCupon,
            FormatoCreativo::MensajeWhatsapp,
            FormatoCreativo::BoletinCorreo,
            FormatoCreativo::PantallaLobby,
        ];
        Ok(self
            .piezas(&todos, ahora)
            .await?
            .into_iter()
            .find(|p| p.candidata.creativo == creativo)
            .map(|p| (p.candidata, p.contenido.formato)))
    }

    async fn aplicar_entrega(
        &mut self,
        creativo: CreativoId,
        canal: CanalPublicidad,
        _ahora: DateTime<Utc>,
        conteo: Conteo,
        costo: Decimal,
        finalizar: bool,
    ) -> AppResult<()> {
        let d = &mut self.datos.publicidad;
        let campana = d
            .piezas
            .iter()
            .find(|p| p.candidata.creativo == creativo)
            .map(|p| p.candidata.campana);
        for p in &mut d.piezas {
            if Some(p.candidata.campana) == campana {
                p.candidata.consumido_total += costo;
                p.candidata.consumido_hoy += costo;
                if finalizar {
                    p.candidata.estado_campana = domain::EstadoCampana::Finalizada;
                }
            }
            if p.candidata.creativo == creativo {
                p.candidata.impresiones_hoy += conteo.impresiones;
            }
        }
        d.entregas.push(EntregaRegistrada {
            creativo,
            canal,
            conteo,
            costo,
            finalizada: finalizar,
        });
        Ok(())
    }

    async fn politica_publicidad_vigente(
        &mut self,
        _ahora: DateTime<Utc>,
    ) -> AppResult<Option<PoliticaPublicidad>> {
        Ok(self.datos.publicidad.politica.clone())
    }

    async fn autorizar_publicidad(
        &mut self,
        persona: PersonaId,
        _politica: PoliticaPublicidadId,
        medio: MedioAutorizacion,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        if self.autorizada_publicidad(persona) {
            return Ok(false);
        }
        self.datos.autorizaciones.push((
            Autorizacion {
                id: AutorizacionId::nuevo(),
                persona_id: persona,
                politica_id: None,
                finalidad: FinalidadTratamiento::Publicidad,
                medio,
                evidencia_archivo_id: None,
                otorgada_en: ahora,
            },
            None,
        ));
        Ok(true)
    }

    async fn insertar_reporte_publicidad(&mut self, r: &ReportePublicidad) -> AppResult<bool> {
        let v = &mut self.datos.publicidad.reportes;
        if v.iter()
            .any(|x| x.creativo_id == r.creativo_id && x.cuenta_id == r.cuenta_id)
        {
            return Ok(false);
        }
        v.push(r.clone());
        Ok(true)
    }

    async fn destinatarios_masivos(
        &mut self,
        campana: CampanaId,
        canal: CanalPublicidad,
        ahora: DateTime<Utc>,
        limite: u32,
    ) -> AppResult<Vec<DestinatarioMasivo>> {
        let d = &self.datos.publicidad;
        Ok(d.personas
            .iter()
            .filter(|p| self.autorizada_publicidad(p.persona_id))
            .filter(|p| {
                !d.envios
                    .iter()
                    .any(|(c, x, k, _)| *c == campana && *x == p.persona_id && *k == canal)
            })
            .filter(|p| {
                let ultimo = d
                    .envios
                    .iter()
                    .filter(|(_, x, k, _)| *x == p.persona_id && *k == canal)
                    .map(|(_, _, _, t)| *t)
                    .max();
                puede_recibir_masivo(ultimo, ahora)
            })
            .take(limite as usize)
            .cloned()
            .collect())
    }

    async fn registrar_envio_publicidad(
        &mut self,
        campana: CampanaId,
        persona: PersonaId,
        canal: CanalPublicidad,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        self.datos
            .publicidad
            .envios
            .push((campana, persona, canal, ahora));
        Ok(())
    }

    async fn encolar_publicidad(
        &mut self,
        evento: &EventoSaliente,
        campana: CampanaId,
    ) -> AppResult<()> {
        self.datos
            .publicidad
            .encolados
            .push((evento.clone(), campana));
        self.salientes.push(evento.clone());
        Ok(())
    }

    async fn pantalla(&mut self) -> AppResult<Option<PantallaPublicidad>> {
        Ok(self.datos.publicidad.pantalla.clone())
    }

    async fn guardar_pantalla(&mut self, p: &PantallaPublicidad) -> AppResult<()> {
        self.datos.publicidad.pantalla = Some(p.clone());
        Ok(())
    }
}
