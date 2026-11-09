//! Doble en memoria del repositorio de control de acceso (plan 05).

use chrono::{DateTime, TimeDelta, Utc};
use domain::{
    ArchivoId, AutorizacionAccesoId, ConfiguracionAcceso, ConsentimientoBiometricoId,
    CorrespondenciaId, CredencialId, CuentaId, EstadoCorrespondencia, EstadoCredencial,
    EventoAccesoId, PuenteId, VisitaId,
};

use super::TransaccionEnMemoria;
use crate::{
    AppError, AppResult, Pagina, SolicitudPagina,
    outbox::{EventoSaliente, MensajeOutbox},
    puertos::{
        RepositorioAcceso,
        acceso::{
            AlertaAcceso, AutorizacionAcceso, CambiosAcceso, ConsentimientoBiometrico,
            ConteoAvisos, Correspondencia, Credencial, CredencialSincronizada, DispositivoAcceso,
            EntregaCorrespondencia, EventoAcceso, EventoVisita, FiltroCorrespondencia,
            FiltroEventosAcceso, FiltroMinuta, FiltroVisitas, NovedadMinuta, Puente, PuntoAcceso,
            SoportesVencidos, TitularAcceso, VentanaSincronizada, Visita, VisitaSincronizada,
        },
    },
};

/// Datos de control de acceso en memoria.
#[derive(Default, Clone)]
pub struct DatosAcceso {
    pub configuracion: Option<ConfiguracionAcceso>,
    pub visitas: Vec<Visita>,
    pub eventos_visita: Vec<EventoVisita>,
    pub correspondencia: Vec<Correspondencia>,
    pub entregas: Vec<EntregaCorrespondencia>,
    pub soportes_purgados: Vec<CorrespondenciaId>,
    pub minuta: Vec<NovedadMinuta>,
    /// Tokens push activos por cuenta.
    pub tokens: Vec<(CuentaId, String)>,
    /// Avisos de acceso encolados (para contar el costo).
    pub avisos: Vec<EventoSaliente>,
    pub puntos: Vec<PuntoAcceso>,
    pub puentes: Vec<Puente>,
    pub dispositivos: Vec<DispositivoAcceso>,
    /// Credencial con su versión de sincronización.
    pub credenciales: Vec<(Credencial, i64)>,
    pub autorizaciones: Vec<AutorizacionAcceso>,
    pub eventos: Vec<EventoAcceso>,
    /// (evento, archivo, purgar_en, purgada).
    pub fotos: Vec<(EventoAccesoId, ArchivoId, DateTime<Utc>, bool)>,
    pub consentimientos: Vec<ConsentimientoBiometrico>,
    pub version: i64,
}

fn pagina<T: Clone>(mut v: Vec<T>, p: SolicitudPagina, id: impl Fn(&T) -> uuid::Uuid) -> Pagina<T> {
    v.sort_by_key(|x| std::cmp::Reverse(id(x)));
    let v: Vec<T> = v
        .into_iter()
        .filter(|x| p.despues().is_none_or(|d| id(x) < d))
        .take(usize::try_from(p.filas_a_consultar()).unwrap_or(usize::MAX))
        .collect();
    Pagina::desde_filas(v, p, id)
}

impl TransaccionEnMemoria {
    fn siguiente_version(&mut self) -> i64 {
        self.datos.acceso.version += 1;
        self.datos.acceso.version
    }

    fn vigente(desde: DateTime<Utc>, hasta: Option<DateTime<Utc>>, ahora: DateTime<Utc>) -> bool {
        desde <= ahora && hasta.is_none_or(|h| h > ahora)
    }
}

impl RepositorioAcceso for TransaccionEnMemoria {
    async fn configuracion_acceso(&mut self) -> AppResult<ConfiguracionAcceso> {
        Ok(self.datos.acceso.configuracion.unwrap_or_default())
    }

    async fn guardar_configuracion_acceso(&mut self, c: &ConfiguracionAcceso) -> AppResult<()> {
        self.datos.acceso.configuracion = Some(*c);
        Ok(())
    }

    async fn insertar_visita(&mut self, v: &Visita) -> AppResult<()> {
        self.datos.acceso.visitas.push(v.clone());
        Ok(())
    }

    async fn actualizar_visita(&mut self, v: &Visita) -> AppResult<()> {
        let x = self
            .datos
            .acceso
            .visitas
            .iter_mut()
            .find(|x| x.id == v.id)
            .ok_or(AppError::NoEncontrado("visita"))?;
        *x = v.clone();
        Ok(())
    }

    async fn visita_para_actualizar(&mut self, id: VisitaId) -> AppResult<Option<Visita>> {
        self.visita(id).await
    }

    async fn visita(&mut self, id: VisitaId) -> AppResult<Option<Visita>> {
        Ok(self
            .datos
            .acceso
            .visitas
            .iter()
            .find(|v| v.id == id)
            .cloned())
    }

    async fn visitas(
        &mut self,
        f: &FiltroVisitas,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<Visita>> {
        let v = self
            .datos
            .acceso
            .visitas
            .iter()
            .filter(|v| f.estado.is_none_or(|e| v.estado == e))
            .filter(|v| f.bien_privado_id.is_none_or(|b| v.bien_privado_id == b))
            .filter(|v| {
                f.bienes
                    .as_ref()
                    .is_none_or(|b| b.contains(&v.bien_privado_id))
            })
            .filter(|v| f.desde.is_none_or(|d| v.creada_en >= d))
            .filter(|v| !f.dentro || v.estado.dentro())
            .cloned()
            .collect();
        Ok(pagina(v, p, |v| v.id.as_uuid()))
    }

    async fn visitas_con_paso_pendiente(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<Visita>> {
        Ok(self
            .datos
            .acceso
            .visitas
            .iter()
            .filter(|v| v.siguiente_paso_en.is_some_and(|s| s <= ahora))
            .cloned()
            .collect())
    }

    async fn insertar_evento_visita(&mut self, e: &EventoVisita) -> AppResult<()> {
        self.datos.acceso.eventos_visita.push(e.clone());
        Ok(())
    }

    async fn eventos_visita(&mut self, id: VisitaId) -> AppResult<Vec<EventoVisita>> {
        Ok(self
            .datos
            .acceso
            .eventos_visita
            .iter()
            .filter(|e| e.visita_id == id)
            .cloned()
            .collect())
    }

    async fn insertar_correspondencia(&mut self, c: &Correspondencia) -> AppResult<()> {
        self.datos.acceso.correspondencia.push(c.clone());
        Ok(())
    }

    async fn actualizar_correspondencia(&mut self, c: &Correspondencia) -> AppResult<()> {
        let x = self
            .datos
            .acceso
            .correspondencia
            .iter_mut()
            .find(|x| x.id == c.id)
            .ok_or(AppError::NoEncontrado("correspondencia"))?;
        *x = c.clone();
        Ok(())
    }

    async fn correspondencia_para_actualizar(
        &mut self,
        id: CorrespondenciaId,
    ) -> AppResult<Option<Correspondencia>> {
        self.correspondencia(id).await
    }

    async fn correspondencia(
        &mut self,
        id: CorrespondenciaId,
    ) -> AppResult<Option<Correspondencia>> {
        Ok(self
            .datos
            .acceso
            .correspondencia
            .iter()
            .find(|c| c.id == id)
            .cloned())
    }

    async fn correspondencias(
        &mut self,
        f: &FiltroCorrespondencia,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<Correspondencia>> {
        let v = self
            .datos
            .acceso
            .correspondencia
            .iter()
            .filter(|c| f.estado.is_none_or(|e| c.estado == e))
            .filter(|c| f.bien_privado_id.is_none_or(|b| c.bien_privado_id == b))
            .filter(|c| {
                f.bienes
                    .as_ref()
                    .is_none_or(|b| b.contains(&c.bien_privado_id))
            })
            .cloned()
            .collect();
        Ok(pagina(v, p, |c| c.id.as_uuid()))
    }

    async fn correspondencia_con_aviso_pendiente(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<Correspondencia>> {
        Ok(self
            .datos
            .acceso
            .correspondencia
            .iter()
            .filter(|c| {
                c.estado == EstadoCorrespondencia::Recibida
                    && (c.siguiente_aviso_en.is_some_and(|s| s <= ahora)
                        || (c.siguiente_aviso_en.is_none() && c.alerta_administracion_en.is_none()))
            })
            .cloned()
            .collect())
    }

    async fn insertar_entrega(&mut self, e: &EntregaCorrespondencia) -> AppResult<()> {
        if self
            .datos
            .acceso
            .entregas
            .iter()
            .any(|x| x.correspondencia_id == e.correspondencia_id)
        {
            return Err(AppError::Conflicto(
                "la correspondencia ya fue entregada".into(),
            ));
        }
        self.datos.acceso.entregas.push(e.clone());
        Ok(())
    }

    async fn entrega(
        &mut self,
        id: CorrespondenciaId,
    ) -> AppResult<Option<EntregaCorrespondencia>> {
        Ok(self
            .datos
            .acceso
            .entregas
            .iter()
            .find(|e| e.correspondencia_id == id)
            .cloned())
    }

    async fn soportes_vencidos(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<SoportesVencidos>> {
        let purgados = self.datos.acceso.soportes_purgados.clone();
        Ok(self
            .datos
            .acceso
            .entregas
            .iter()
            .filter(|e| e.soportes_hasta <= ahora && !purgados.contains(&e.correspondencia_id))
            .map(|e| SoportesVencidos {
                correspondencia_id: e.correspondencia_id,
                archivos: e
                    .firma_archivo_id
                    .into_iter()
                    .chain(e.foto_archivo_id)
                    .collect(),
            })
            .collect())
    }

    async fn marcar_soportes_purgados(
        &mut self,
        id: CorrespondenciaId,
        _ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        self.datos.acceso.soportes_purgados.push(id);
        Ok(())
    }

    async fn insertar_novedad(&mut self, n: &NovedadMinuta) -> AppResult<()> {
        self.datos.acceso.minuta.push(n.clone());
        Ok(())
    }

    async fn novedades(
        &mut self,
        f: &FiltroMinuta,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<NovedadMinuta>> {
        let v = self
            .datos
            .acceso
            .minuta
            .iter()
            .filter(|n| f.tipo.is_none_or(|t| n.tipo == t))
            .filter(|n| f.desde.is_none_or(|d| n.ocurrida_en >= d))
            .filter(|n| f.hasta.is_none_or(|h| n.ocurrida_en < h))
            .cloned()
            .collect();
        Ok(pagina(v, p, |n| n.id.as_uuid()))
    }

    async fn tokens_push(&mut self, cuentas: &[CuentaId]) -> AppResult<Vec<(CuentaId, String)>> {
        Ok(self
            .datos
            .acceso
            .tokens
            .iter()
            .filter(|(c, _)| cuentas.contains(c))
            .cloned()
            .collect())
    }

    async fn encolar_aviso_acceso(&mut self, evento: &EventoSaliente) -> AppResult<()> {
        self.datos.acceso.avisos.push(evento.clone());
        self.salientes.push(evento.clone());
        Ok(())
    }

    async fn mensajes_avisos_acceso(
        &mut self,
        _desde: DateTime<Utc>,
        _hasta: DateTime<Utc>,
    ) -> AppResult<ConteoAvisos> {
        let mut c = ConteoAvisos::default();
        for a in &self.datos.acceso.avisos {
            match a.mensaje {
                MensajeOutbox::Push(_) => c.push_enviados += 1,
                MensajeOutbox::WhatsApp(_) => c.whatsapp_enviados += 1,
                MensajeOutbox::Correo(_) => {}
            }
        }
        Ok(c)
    }

    async fn guardar_punto_acceso(&mut self, p: &PuntoAcceso, nuevo: bool) -> AppResult<()> {
        if self
            .datos
            .acceso
            .puntos
            .iter()
            .any(|x| x.nombre == p.nombre && x.id != p.id)
        {
            return Err(AppError::Conflicto(
                "ya existe un punto con ese nombre".into(),
            ));
        }
        if nuevo {
            self.datos.acceso.puntos.push(p.clone());
        } else if let Some(x) = self.datos.acceso.puntos.iter_mut().find(|x| x.id == p.id) {
            *x = p.clone();
        }
        Ok(())
    }

    async fn puntos_acceso(&mut self) -> AppResult<Vec<PuntoAcceso>> {
        Ok(self.datos.acceso.puntos.clone())
    }

    async fn insertar_puente(&mut self, p: &Puente) -> AppResult<()> {
        self.datos.acceso.puentes.push(p.clone());
        Ok(())
    }

    async fn actualizar_puente(&mut self, p: &Puente) -> AppResult<()> {
        if let Some(x) = self.datos.acceso.puentes.iter_mut().find(|x| x.id == p.id) {
            *x = p.clone();
        }
        Ok(())
    }

    async fn puente(&mut self, id: PuenteId) -> AppResult<Option<Puente>> {
        Ok(self
            .datos
            .acceso
            .puentes
            .iter()
            .find(|p| p.id == id)
            .cloned())
    }

    async fn puentes(&mut self) -> AppResult<Vec<Puente>> {
        Ok(self.datos.acceso.puentes.clone())
    }

    async fn guardar_dispositivo(&mut self, d: &DispositivoAcceso, nuevo: bool) -> AppResult<()> {
        if nuevo {
            self.datos.acceso.dispositivos.push(d.clone());
        } else if let Some(x) = self
            .datos
            .acceso
            .dispositivos
            .iter_mut()
            .find(|x| x.id == d.id)
        {
            *x = d.clone();
        }
        Ok(())
    }

    async fn dispositivos(
        &mut self,
        puente: Option<PuenteId>,
    ) -> AppResult<Vec<DispositivoAcceso>> {
        Ok(self
            .datos
            .acceso
            .dispositivos
            .iter()
            .filter(|d| puente.is_none_or(|p| d.puente_id == p))
            .cloned()
            .collect())
    }

    async fn insertar_credencial(&mut self, c: &Credencial) -> AppResult<()> {
        let repetida = self.datos.acceso.credenciales.iter().any(|(x, _)| {
            x.huella == c.huella
                && matches!(
                    x.estado,
                    EstadoCredencial::Activa | EstadoCredencial::Suspendida
                )
        });
        if repetida {
            return Err(AppError::Conflicto(
                "ya existe una credencial vigente con ese identificador".into(),
            ));
        }
        let v = self.siguiente_version();
        self.datos.acceso.credenciales.push((c.clone(), v));
        Ok(())
    }

    async fn actualizar_credencial(&mut self, c: &Credencial) -> AppResult<()> {
        let v = self.siguiente_version();
        if let Some(x) = self
            .datos
            .acceso
            .credenciales
            .iter_mut()
            .find(|(x, _)| x.id == c.id)
        {
            *x = (c.clone(), v);
        }
        Ok(())
    }

    async fn credencial(&mut self, id: CredencialId) -> AppResult<Option<Credencial>> {
        Ok(self
            .datos
            .acceso
            .credenciales
            .iter()
            .find(|(c, _)| c.id == id)
            .map(|(c, _)| c.clone()))
    }

    async fn credencial_por_huella(&mut self, huella: &str) -> AppResult<Option<Credencial>> {
        Ok(self
            .datos
            .acceso
            .credenciales
            .iter()
            .rev()
            .find(|(c, _)| c.huella == huella)
            .map(|(c, _)| c.clone()))
    }

    async fn credenciales_de(&mut self, t: TitularAcceso) -> AppResult<Vec<Credencial>> {
        Ok(self
            .datos
            .acceso
            .credenciales
            .iter()
            .filter(|(c, _)| c.titular == t)
            .map(|(c, _)| c.clone())
            .collect())
    }

    async fn insertar_autorizacion_acceso(&mut self, a: &AutorizacionAcceso) -> AppResult<()> {
        self.datos.acceso.autorizaciones.push(a.clone());
        self.siguiente_version();
        Ok(())
    }

    async fn revocar_autorizacion_acceso(
        &mut self,
        id: AutorizacionAccesoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        let Some(a) = self
            .datos
            .acceso
            .autorizaciones
            .iter_mut()
            .find(|a| a.id == id && a.revocada_en.is_none())
        else {
            return Ok(false);
        };
        a.revocada_en = Some(ahora);
        Ok(true)
    }

    async fn autorizaciones_de(
        &mut self,
        t: TitularAcceso,
        incluir_revocadas: bool,
    ) -> AppResult<Vec<AutorizacionAcceso>> {
        Ok(self
            .datos
            .acceso
            .autorizaciones
            .iter()
            .filter(|a| a.titular == t && (incluir_revocadas || a.revocada_en.is_none()))
            .cloned()
            .collect())
    }

    async fn ventanas_de(
        &mut self,
        t: TitularAcceso,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<VentanaSincronizada>> {
        let mut v: Vec<VentanaSincronizada> = self
            .datos
            .acceso
            .autorizaciones
            .iter()
            .filter(|a| {
                a.titular == t
                    && a.revocada_en.is_none()
                    && Self::vigente(a.vigente_desde, a.vigente_hasta, ahora)
            })
            .map(|a| VentanaSincronizada {
                puntos: a.puntos.clone(),
                vigente_desde: a.vigente_desde,
                vigente_hasta: a.vigente_hasta,
                horario: a.horario.clone(),
            })
            .collect();
        if let TitularAcceso::Persona(p) = t {
            let residencias = self
                .datos
                .residencias
                .iter()
                .filter(|r| {
                    r.persona_id == p && Self::vigente(r.vigente_desde, r.vigente_hasta, ahora)
                })
                .map(|r| r.vigente_desde);
            let titulos = self
                .datos
                .titularidades
                .iter()
                .filter(|x| {
                    x.persona_id == p && Self::vigente(x.vigente_desde, x.vigente_hasta, ahora)
                })
                .map(|x| x.vigente_desde);
            for desde in residencias.chain(titulos) {
                v.push(VentanaSincronizada {
                    puntos: Vec::new(),
                    vigente_desde: desde,
                    vigente_hasta: None,
                    horario: None,
                });
            }
        }
        Ok(v)
    }

    async fn cambios_desde(
        &mut self,
        cursor: i64,
        limite: u32,
        ahora: DateTime<Utc>,
    ) -> AppResult<CambiosAcceso> {
        let mut credenciales = Vec::new();
        let lista: Vec<(Credencial, i64)> = self
            .datos
            .acceso
            .credenciales
            .iter()
            .filter(|(_, v)| *v > cursor)
            .take(limite as usize)
            .cloned()
            .collect();
        for (c, version) in lista {
            let activa = c.estado == EstadoCredencial::Activa
                && Self::vigente(c.vigente_desde, c.vigente_hasta, ahora);
            let ventanas = if activa {
                self.ventanas_de(c.titular, ahora).await?
            } else {
                Vec::new()
            };
            credenciales.push(CredencialSincronizada {
                id: c.id,
                tipo: c.tipo,
                huella: c.huella.clone(),
                activa,
                vigente_hasta: c.vigente_hasta,
                ventanas,
                borrar_plantilla: c.tipo == domain::TipoCredencial::BiometriaRef
                    && !activa
                    && c.borrado_confirmado_en.is_none(),
                version,
            });
        }
        let visitas = self
            .datos
            .acceso
            .visitas
            .iter()
            .filter(|v| v.origen == domain::OrigenVisita::Preautorizada)
            .map(|v| VisitaSincronizada {
                id: v.id,
                activa: matches!(
                    v.estado,
                    domain::EstadoVisita::Autorizada | domain::EstadoVisita::Ingresada
                ),
                usos: v.usos,
                version: 0,
            })
            .collect();
        let cursor = credenciales
            .iter()
            .map(|c| c.version)
            .max()
            .unwrap_or(cursor);
        Ok(CambiosAcceso {
            credenciales,
            visitas,
            cursor,
        })
    }

    async fn insertar_evento_acceso(&mut self, e: &EventoAcceso) -> AppResult<bool> {
        let repetido = e.id_local.is_some()
            && self
                .datos
                .acceso
                .eventos
                .iter()
                .any(|x| x.puente_id == e.puente_id && x.id_local == e.id_local);
        if repetido {
            return Ok(false);
        }
        self.datos.acceso.eventos.push(e.clone());
        Ok(true)
    }

    async fn adjuntar_foto_evento(
        &mut self,
        evento: EventoAccesoId,
        archivo: ArchivoId,
        purgar_en: DateTime<Utc>,
    ) -> AppResult<()> {
        self.datos
            .acceso
            .fotos
            .push((evento, archivo, purgar_en, false));
        Ok(())
    }

    async fn eventos_acceso(
        &mut self,
        f: &FiltroEventosAcceso,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<EventoAcceso>> {
        let v = self
            .datos
            .acceso
            .eventos
            .iter()
            .filter(|e| f.desde.is_none_or(|d| e.ocurrido_en >= d))
            .filter(|e| f.hasta.is_none_or(|h| e.ocurrido_en < h))
            .filter(|e| {
                f.punto_acceso_id
                    .is_none_or(|x| e.punto_acceso_id == Some(x))
            })
            .filter(|e| f.resultado.is_none_or(|r| e.resultado == r))
            .cloned()
            .collect();
        Ok(pagina(v, p, |e| e.id.as_uuid()))
    }

    async fn fotos_eventos_vencidas(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<(EventoAccesoId, ArchivoId)>> {
        Ok(self
            .datos
            .acceso
            .fotos
            .iter()
            .filter(|(_, _, p, purgada)| *p <= ahora && !purgada)
            .map(|(e, a, _, _)| (*e, *a))
            .collect())
    }

    async fn marcar_foto_evento_purgada(
        &mut self,
        evento: EventoAccesoId,
        _ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        for f in self.datos.acceso.fotos.iter_mut().filter(|f| f.0 == evento) {
            f.3 = true;
        }
        Ok(())
    }

    async fn purgar_eventos_acceso(&mut self, dias: u32) -> AppResult<u64> {
        let Some(ahora) = self
            .datos
            .acceso
            .eventos
            .iter()
            .map(|e| e.ocurrido_en)
            .max()
        else {
            return Ok(0);
        };
        let limite = ahora - TimeDelta::days(i64::from(dias.max(365)));
        let antes = self.datos.acceso.eventos.len();
        self.datos
            .acceso
            .eventos
            .retain(|e| e.ocurrido_en >= limite);
        Ok((antes - self.datos.acceso.eventos.len()) as u64)
    }

    async fn alertas_acceso(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<AlertaAcceso>> {
        Ok(self
            .datos
            .acceso
            .puentes
            .iter()
            .filter(|p| {
                p.estado == domain::EstadoPuente::Activo
                    && p.ultimo_latido_en
                        .is_none_or(|l| ahora - l > TimeDelta::minutes(5))
            })
            .map(|p| AlertaAcceso::PuenteSinLatido {
                puente: p.id,
                nombre: p.nombre.clone(),
                ultimo: p.ultimo_latido_en,
            })
            .collect())
    }

    async fn insertar_consentimiento(&mut self, c: &ConsentimientoBiometrico) -> AppResult<()> {
        self.datos.acceso.consentimientos.push(c.clone());
        Ok(())
    }

    async fn consentimiento_vigente(
        &mut self,
        persona: domain::PersonaId,
    ) -> AppResult<Option<ConsentimientoBiometrico>> {
        Ok(self
            .datos
            .acceso
            .consentimientos
            .iter()
            .find(|c| c.persona_id == persona && c.revocado_en.is_none())
            .cloned())
    }

    async fn revocar_consentimiento(
        &mut self,
        id: ConsentimientoBiometricoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        if let Some(c) = self
            .datos
            .acceso
            .consentimientos
            .iter_mut()
            .find(|c| c.id == id)
        {
            c.revocado_en = Some(ahora);
        }
        Ok(())
    }
}
