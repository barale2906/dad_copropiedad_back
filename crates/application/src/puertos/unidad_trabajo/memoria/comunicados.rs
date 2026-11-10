//! Doble en memoria de los comunicados (plan 10, fase 4). La audiencia no se resuelve aquí (es
//! SQL, se prueba en `infra-postgres`): las pruebas fijan los destinatarios en
//! [`DatosComunicados::audiencia`].

use chrono::{DateTime, Utc};
use domain::{Audiencia, ComunicadoId, EstadoComunicado, FinalidadTratamiento, PersonaId};

use super::TransaccionEnMemoria;
use crate::{
    AppResult, Pagina, SolicitudPagina,
    outbox::EventoSaliente,
    puertos::comunicados::{
        Comunicado, ComunicadoConResumen, ComunicadoRecibido, DestinatarioComunicado,
        EntregaComunicado, RepositorioComunicados, ResumenLectura,
    },
};

/// Entrega con su lectura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntregaGuardada {
    pub comunicado: ComunicadoId,
    pub entrega: EntregaComunicado,
    pub leido_en: Option<DateTime<Utc>>,
    pub enterado_en: Option<DateTime<Utc>>,
}

#[derive(Default, Clone)]
pub struct DatosComunicados {
    pub comunicados: Vec<Comunicado>,
    pub entregas: Vec<EntregaGuardada>,
    /// Lo que devuelve `destinatarios_comunicado` para cualquier audiencia.
    pub audiencia: Vec<DestinatarioComunicado>,
    /// Envíos con el concepto `comunicados`.
    pub enviados: Vec<EventoSaliente>,
}

impl DatosComunicados {
    fn resumen(&self, id: ComunicadoId) -> ResumenLectura {
        let mut r = ResumenLectura::default();
        for e in self.entregas.iter().filter(|e| e.comunicado == id) {
            let c = e.entrega.canales;
            r.destinatarios += 1;
            r.por_push += u32::from(c.push);
            r.por_correo += u32::from(c.correo);
            r.por_whatsapp += u32::from(c.whatsapp);
            r.sin_canal += u32::from(!c.alguno());
            r.leidos += u32::from(e.leido_en.is_some());
            r.enterados += u32::from(e.enterado_en.is_some());
        }
        r
    }

    fn recibido(&self, persona: PersonaId, id: ComunicadoId) -> Option<ComunicadoRecibido> {
        let e = self
            .entregas
            .iter()
            .find(|e| e.comunicado == id && e.entrega.persona == persona)?;
        let c = self.comunicados.iter().find(|c| c.id == id)?;
        Some(ComunicadoRecibido {
            comunicado: c.clone(),
            leido_en: e.leido_en,
            enterado_en: e.enterado_en,
        })
    }

    fn entrega_mut(
        &mut self,
        persona: PersonaId,
        id: ComunicadoId,
    ) -> Option<&mut EntregaGuardada> {
        self.entregas
            .iter_mut()
            .find(|e| e.comunicado == id && e.entrega.persona == persona)
    }
}

impl RepositorioComunicados for TransaccionEnMemoria {
    async fn insertar_comunicado(&mut self, c: &Comunicado) -> AppResult<()> {
        self.datos.comunicados.comunicados.push(c.clone());
        Ok(())
    }

    async fn actualizar_comunicado(&mut self, c: &Comunicado) -> AppResult<()> {
        if let Some(x) = self
            .datos
            .comunicados
            .comunicados
            .iter_mut()
            .find(|x| x.id == c.id)
        {
            *x = c.clone();
        }
        Ok(())
    }

    async fn comunicado(&mut self, id: ComunicadoId) -> AppResult<Option<Comunicado>> {
        Ok(self
            .datos
            .comunicados
            .comunicados
            .iter()
            .find(|c| c.id == id)
            .cloned())
    }

    async fn bloquear_comunicado(&mut self, id: ComunicadoId) -> AppResult<Option<Comunicado>> {
        self.comunicado(id).await
    }

    async fn comunicados(&mut self, p: SolicitudPagina) -> AppResult<Pagina<ComunicadoConResumen>> {
        let d = &self.datos.comunicados;
        let mut v: Vec<ComunicadoConResumen> = d
            .comunicados
            .iter()
            .filter(|c| p.despues().is_none_or(|x| c.id.as_uuid() < x))
            .map(|c| ComunicadoConResumen {
                comunicado: c.clone(),
                resumen: d.resumen(c.id),
            })
            .collect();
        v.sort_by_key(|c| std::cmp::Reverse(c.comunicado.id));
        Ok(Pagina::desde_filas(v, p, |c| c.comunicado.id.as_uuid()))
    }

    async fn resumen_lectura(&mut self, id: ComunicadoId) -> AppResult<ResumenLectura> {
        Ok(self.datos.comunicados.resumen(id))
    }

    async fn programados_vencidos(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<ComunicadoId>> {
        Ok(self
            .datos
            .comunicados
            .comunicados
            .iter()
            .filter(|c| {
                c.estado == EstadoComunicado::Programado
                    && c.programado_para.is_some_and(|p| p <= ahora)
            })
            .map(|c| c.id)
            .collect())
    }

    async fn destinatarios_comunicado(
        &mut self,
        _audiencia: &Audiencia,
        _finalidad: FinalidadTratamiento,
        _ahora: DateTime<Utc>,
    ) -> AppResult<Vec<DestinatarioComunicado>> {
        Ok(self.datos.comunicados.audiencia.clone())
    }

    async fn insertar_entregas(
        &mut self,
        id: ComunicadoId,
        entregas: &[EntregaComunicado],
    ) -> AppResult<()> {
        self.datos
            .comunicados
            .entregas
            .extend(entregas.iter().map(|e| EntregaGuardada {
                comunicado: id,
                entrega: *e,
                leido_en: None,
                enterado_en: None,
            }));
        Ok(())
    }

    async fn encolar_comunicado(&mut self, evento: &EventoSaliente) -> AppResult<()> {
        self.datos.comunicados.enviados.push(evento.clone());
        self.salientes.push(evento.clone());
        Ok(())
    }

    async fn recibidos(
        &mut self,
        persona: PersonaId,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<ComunicadoRecibido>> {
        let d = &self.datos.comunicados;
        let mut v: Vec<ComunicadoRecibido> = d
            .entregas
            .iter()
            .filter(|e| e.entrega.persona == persona)
            .filter(|e| p.despues().is_none_or(|x| e.comunicado.as_uuid() < x))
            .filter_map(|e| d.recibido(persona, e.comunicado))
            .collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.comunicado.id));
        Ok(Pagina::desde_filas(v, p, |r| r.comunicado.id.as_uuid()))
    }

    async fn recibido(
        &mut self,
        persona: PersonaId,
        id: ComunicadoId,
    ) -> AppResult<Option<ComunicadoRecibido>> {
        Ok(self.datos.comunicados.recibido(persona, id))
    }

    async fn marcar_leido(
        &mut self,
        persona: PersonaId,
        id: ComunicadoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        if let Some(e) = self.datos.comunicados.entrega_mut(persona, id) {
            e.leido_en.get_or_insert(ahora);
        }
        Ok(())
    }

    async fn marcar_enterado(
        &mut self,
        persona: PersonaId,
        id: ComunicadoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        if let Some(e) = self.datos.comunicados.entrega_mut(persona, id) {
            e.leido_en.get_or_insert(ahora);
            e.enterado_en.get_or_insert(ahora);
        }
        Ok(())
    }
}
