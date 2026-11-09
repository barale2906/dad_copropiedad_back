//! Doble en memoria del repositorio de asambleas (plan 06). Reproduce las restricciones de la
//! BD que importan a los casos de uso: un bien en un solo asistente, una presencia abierta,
//! rótulo único, `UNIQUE (pregunta_id, bien_privado_id)` y el trigger `validar_participacion`.

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, AsistenteId, BienPrivadoId, CalidadRepresentacion, CodigoRotulo, Coeficiente,
    Conteo, DominioError, EstadoActa, EstadoPregunta, EstadoReunion, Modalidad, MotivoSalida,
    ParticipacionNueva, PersonaId, PoderId, PreguntaId, PuntoOrdenId, ReunionId, VotoEmitido,
    VotoNuevo, VotosOpcion,
};
use rust_decimal::Decimal;

use super::TransaccionEnMemoria;
use crate::{
    AppError, AppResult, Pagina, SolicitudPagina,
    outbox::EventoSaliente,
    puertos::{
        RepositorioAsambleas,
        asambleas::{
            Asistente, AsistenteNuevo, BienDeAsistente, BienPadron, BienParaPadron, BienPresente,
            CanalConvocatoria, CoincidenciaMesa, ConteoVotos, DetalleVoto, DocumentoReunion,
            EnvioConvocatoria, FotoQuorum, MensajesLinea, ParticipacionActa, Poder,
            PreguntaGuardada, Presencia, PuntoOrden, Reunion,
        },
        copropiedades::CargoVigente,
    },
};

#[derive(Debug, Clone)]
pub struct AsistenteMem {
    pub a: AsistenteNuevo,
    pub linea: Option<String>,
    pub confirmado_en: Option<DateTime<Utc>>,
    pub outbox: Option<i64>,
    pub token: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct VinculoMem {
    pub reunion: ReunionId,
    pub asistente: AsistenteId,
    pub bien: BienPrivadoId,
    pub calidad: CalidadRepresentacion,
    pub poder: Option<PoderId>,
    pub hasta: Option<DateTime<Utc>>,
}

/// Datos de asambleas en memoria.
#[derive(Default, Clone)]
pub struct DatosAsambleas {
    pub reuniones: Vec<Reunion>,
    pub puntos: Vec<(ReunionId, PuntoOrden)>,
    pub documentos: Vec<(ReunionId, DocumentoReunion)>,
    pub padron: Vec<(ReunionId, BienParaPadron)>,
    pub morosos: Vec<(ReunionId, BienPrivadoId)>,
    pub asistentes: Vec<AsistenteMem>,
    pub vinculos: Vec<VinculoMem>,
    pub poderes: Vec<Poder>,
    pub presencias: Vec<(ReunionId, Presencia)>,
    pub rotulos: Vec<(ReunionId, CodigoRotulo, AsistenteId, bool)>,
    pub fotos: Vec<(ReunionId, FotoQuorum)>,
    pub preguntas: Vec<PreguntaGuardada>,
    pub habilitados: Vec<(PreguntaId, BienPrivadoId, Coeficiente)>,
    pub participaciones: Vec<ParticipacionNueva>,
    pub votos: Vec<VotoNuevo>,
    pub avisos_en_vivo: Vec<ReunionId>,
    pub outbox_reunion: Vec<(ReunionId, i64)>,
    pub envios: Vec<(ReunionId, PersonaId, CanalConvocatoria, i64)>,
    pub cargos_vigentes: Vec<CargoVigente>,
    pub linea: Option<String>,
    pub siguiente_outbox: i64,
}

impl TransaccionEnMemoria {
    fn a(&self) -> &DatosAsambleas {
        &self.datos.asambleas
    }

    fn codigo(&self, bien: BienPrivadoId) -> String {
        self.datos
            .bienes
            .get(&bien)
            .map(|b| b.codigo.as_str().to_owned())
            .unwrap_or_default()
    }

    fn es_moroso(&self, reunion: ReunionId, bien: BienPrivadoId) -> bool {
        self.a().morosos.contains(&(reunion, bien))
    }

    fn coef_padron(&self, reunion: ReunionId, bien: BienPrivadoId) -> Option<Coeficiente> {
        self.a()
            .padron
            .iter()
            .find(|(r, b)| *r == reunion && b.bien_privado_id == bien)
            .map(|(_, b)| b.coeficiente)
    }

    fn armar_asistente(&self, m: &AsistenteMem) -> Asistente {
        let r = m.a.reunion_id;
        let bienes = self
            .a()
            .vinculos
            .iter()
            .filter(|v| v.asistente == m.a.id && v.hasta.is_none())
            .map(|v| BienDeAsistente {
                bien_privado_id: v.bien,
                codigo: self.codigo(v.bien),
                coeficiente: self
                    .coef_padron(r, v.bien)
                    .unwrap_or_else(|| Coeficiente::new(Decimal::ONE).expect("1 es válido")),
                calidad: v.calidad,
                poder_id: v.poder,
                moroso: self.es_moroso(r, v.bien),
            })
            .collect();
        Asistente {
            id: m.a.id,
            reunion_id: r,
            persona_id: m.a.persona_id,
            nombre: self
                .datos
                .personas
                .iter()
                .find(|p| p.id == m.a.persona_id)
                .map(|p| p.nombre.clone())
                .unwrap_or_default(),
            metodo: m.a.metodo,
            telefono: m.a.telefono.clone(),
            linea_whatsapp: m.linea.clone(),
            whatsapp_confirmado_en: m.confirmado_en,
            confirmacion_outbox_id: m.outbox,
            registrado_por: m.a.registrado_por,
            creado_en: m.a.creado_en,
            presente: self
                .a()
                .presencias
                .iter()
                .any(|(_, p)| p.asistente_id == m.a.id && p.salida_en.is_none()),
            rotulo: self
                .a()
                .rotulos
                .iter()
                .find(|(_, _, a, liberado)| *a == m.a.id && !liberado)
                .map(|(_, c, _, _)| c.clone()),
            bienes,
        }
    }

    fn asistente_mem(&mut self, id: AsistenteId) -> AppResult<&mut AsistenteMem> {
        self.datos
            .asambleas
            .asistentes
            .iter_mut()
            .find(|m| m.a.id == id)
            .ok_or(AppError::NoEncontrado("asistente"))
    }
}

impl RepositorioAsambleas for TransaccionEnMemoria {
    async fn insertar_reunion(&mut self, r: &Reunion) -> AppResult<()> {
        self.datos.asambleas.reuniones.push(r.clone());
        Ok(())
    }

    async fn actualizar_reunion(&mut self, r: &Reunion) -> AppResult<()> {
        let x = self
            .datos
            .asambleas
            .reuniones
            .iter_mut()
            .find(|x| x.id == r.id)
            .ok_or(AppError::NoEncontrado("reunión"))?;
        *x = r.clone();
        Ok(())
    }

    async fn reunion(&mut self, id: ReunionId) -> AppResult<Option<Reunion>> {
        Ok(self.a().reuniones.iter().find(|r| r.id == id).cloned())
    }

    async fn bloquear_reunion(&mut self, id: ReunionId) -> AppResult<Option<Reunion>> {
        self.reunion(id).await
    }

    async fn reuniones(
        &mut self,
        estado: Option<EstadoReunion>,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<Reunion>> {
        let mut v: Vec<_> = self
            .a()
            .reuniones
            .iter()
            .filter(|r| estado.is_none_or(|e| r.estado == e))
            .cloned()
            .collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.id));
        Ok(Pagina::desde_filas(v, pagina, |r| r.id.as_uuid()))
    }

    async fn actas_publicadas(&mut self) -> AppResult<Vec<Reunion>> {
        Ok(self
            .a()
            .reuniones
            .iter()
            .filter(|r| r.acta_estado == EstadoActa::Publicada)
            .cloned()
            .collect())
    }

    async fn puntos(&mut self, reunion: ReunionId) -> AppResult<Vec<PuntoOrden>> {
        let mut v: Vec<_> = self
            .a()
            .puntos
            .iter()
            .filter(|(r, _)| *r == reunion)
            .map(|(_, p)| p.clone())
            .collect();
        v.sort_by_key(|p| p.orden);
        Ok(v)
    }

    async fn guardar_punto(
        &mut self,
        reunion: ReunionId,
        p: &PuntoOrden,
        nuevo: bool,
    ) -> AppResult<()> {
        let puntos = &mut self.datos.asambleas.puntos;
        if nuevo {
            puntos.push((reunion, p.clone()));
        } else if let Some((_, x)) = puntos.iter_mut().find(|(_, x)| x.id == p.id) {
            *x = p.clone();
        }
        Ok(())
    }

    async fn eliminar_punto(&mut self, id: PuntoOrdenId) -> AppResult<bool> {
        let antes = self.a().puntos.len();
        self.datos.asambleas.puntos.retain(|(_, p)| p.id != id);
        Ok(self.a().puntos.len() < antes)
    }

    async fn documentos(&mut self, reunion: ReunionId) -> AppResult<Vec<DocumentoReunion>> {
        Ok(self
            .a()
            .documentos
            .iter()
            .filter(|(r, _)| *r == reunion)
            .map(|(_, d)| d.clone())
            .collect())
    }

    async fn agregar_documento(
        &mut self,
        reunion: ReunionId,
        d: &DocumentoReunion,
    ) -> AppResult<()> {
        self.datos.asambleas.documentos.push((reunion, d.clone()));
        Ok(())
    }

    async fn quitar_documento(
        &mut self,
        reunion: ReunionId,
        archivo: ArchivoId,
    ) -> AppResult<bool> {
        let antes = self.a().documentos.len();
        self.datos
            .asambleas
            .documentos
            .retain(|(r, d)| !(*r == reunion && d.archivo_id == archivo));
        Ok(self.a().documentos.len() < antes)
    }

    async fn bienes_para_padron(&mut self) -> AppResult<Vec<BienParaPadron>> {
        Ok(self
            .datos
            .bienes
            .values()
            .filter(|b| b.activo)
            .filter_map(|b| {
                b.coeficiente.map(|c| BienParaPadron {
                    bien_privado_id: b.id,
                    coeficiente: c,
                    representante: self
                        .datos
                        .titularidades
                        .iter()
                        .find(|t| {
                            t.bien_privado_id == b.id
                                && t.es_representante
                                && t.vigente_hasta.is_none()
                        })
                        .map(|t| t.persona_id),
                })
            })
            .collect())
    }

    async fn insertar_padron(
        &mut self,
        reunion: ReunionId,
        bienes: &[BienParaPadron],
    ) -> AppResult<()> {
        self.datos
            .asambleas
            .padron
            .extend(bienes.iter().map(|b| (reunion, *b)));
        Ok(())
    }

    async fn padron(&mut self, reunion: ReunionId) -> AppResult<Vec<BienPadron>> {
        let mut v: Vec<_> = self
            .a()
            .padron
            .iter()
            .filter(|(r, _)| *r == reunion)
            .map(|(_, b)| BienPadron {
                bien_privado_id: b.bien_privado_id,
                codigo: self.codigo(b.bien_privado_id),
                coeficiente: b.coeficiente,
                representante: b.representante,
                moroso: self.es_moroso(reunion, b.bien_privado_id),
            })
            .collect();
        v.sort_by(|a, b| a.codigo.cmp(&b.codigo));
        Ok(v)
    }

    async fn buscar_en_padron(
        &mut self,
        reunion: ReunionId,
        texto: &str,
        limite: u32,
    ) -> AppResult<Vec<CoincidenciaMesa>> {
        let t = texto.to_lowercase();
        let padron = self.padron(reunion).await?;
        Ok(padron
            .iter()
            .filter(|b| b.codigo.to_lowercase().contains(&t))
            .take(limite as usize)
            .map(|b| CoincidenciaMesa {
                bien: b.clone(),
                representante_nombre: b
                    .representante
                    .and_then(|p| self.datos.personas.iter().find(|x| x.id == p))
                    .map(|p| p.nombre.clone()),
                representante_telefono: None,
                otros_bienes: padron
                    .iter()
                    .filter(|o| {
                        o.representante.is_some()
                            && o.representante == b.representante
                            && o.bien_privado_id != b.bien_privado_id
                    })
                    .map(|o| o.bien_privado_id)
                    .collect(),
                asistente_actual: self
                    .a()
                    .vinculos
                    .iter()
                    .find(|v| {
                        v.reunion == reunion && v.bien == b.bien_privado_id && v.hasta.is_none()
                    })
                    .map(|v| v.asistente),
            })
            .collect())
    }

    async fn reemplazar_morosos(
        &mut self,
        reunion: ReunionId,
        bienes: &[BienPrivadoId],
    ) -> AppResult<()> {
        let m = &mut self.datos.asambleas.morosos;
        m.retain(|(r, _)| *r != reunion);
        m.extend(bienes.iter().map(|b| (reunion, *b)));
        Ok(())
    }

    async fn insertar_asistente(&mut self, a: &AsistenteNuevo) -> AppResult<()> {
        if self
            .a()
            .asistentes
            .iter()
            .any(|m| m.a.reunion_id == a.reunion_id && m.a.persona_id == a.persona_id)
        {
            return Err(AppError::Conflicto("asistente repetido".into()));
        }
        self.datos.asambleas.asistentes.push(AsistenteMem {
            a: a.clone(),
            linea: None,
            confirmado_en: None,
            outbox: None,
            token: None,
        });
        Ok(())
    }

    async fn asistente(&mut self, id: AsistenteId) -> AppResult<Option<Asistente>> {
        Ok(self
            .a()
            .asistentes
            .iter()
            .find(|m| m.a.id == id)
            .map(|m| self.armar_asistente(m)))
    }

    async fn asistente_de_persona(
        &mut self,
        reunion: ReunionId,
        persona: PersonaId,
    ) -> AppResult<Option<Asistente>> {
        Ok(self
            .a()
            .asistentes
            .iter()
            .find(|m| m.a.reunion_id == reunion && m.a.persona_id == persona)
            .map(|m| self.armar_asistente(m)))
    }

    async fn asistentes(&mut self, reunion: ReunionId) -> AppResult<Vec<Asistente>> {
        Ok(self
            .a()
            .asistentes
            .iter()
            .filter(|m| m.a.reunion_id == reunion)
            .map(|m| self.armar_asistente(m))
            .collect())
    }

    async fn asistente_por_token(&mut self, token_hash: &[u8]) -> AppResult<Option<Asistente>> {
        Ok(self
            .a()
            .asistentes
            .iter()
            .find(|m| m.token.as_deref() == Some(token_hash))
            .map(|m| self.armar_asistente(m)))
    }

    async fn asistente_por_rotulo(
        &mut self,
        reunion: ReunionId,
        codigo: &CodigoRotulo,
    ) -> AppResult<Option<Asistente>> {
        let id = self
            .a()
            .rotulos
            .iter()
            .find(|(r, c, _, liberado)| *r == reunion && c == codigo && !liberado)
            .map(|(_, _, a, _)| *a);
        match id {
            Some(id) => self.asistente(id).await,
            None => Ok(None),
        }
    }

    async fn asistentes_por_telefono(&mut self, telefono: &str) -> AppResult<Vec<Asistente>> {
        Ok(self
            .a()
            .asistentes
            .iter()
            .filter(|m| m.a.telefono.as_deref() == Some(telefono))
            .map(|m| self.armar_asistente(m))
            .collect())
    }

    async fn actualizar_medios(
        &mut self,
        id: AsistenteId,
        telefono: Option<&str>,
        linea: Option<&str>,
        confirmado_en: Option<DateTime<Utc>>,
        confirmacion_outbox_id: Option<i64>,
    ) -> AppResult<()> {
        let m = self.asistente_mem(id)?;
        m.a.telefono = telefono.map(str::to_owned);
        m.linea = linea.map(str::to_owned);
        m.confirmado_en = confirmado_en;
        m.outbox = confirmacion_outbox_id;
        Ok(())
    }

    async fn fijar_token(&mut self, id: AsistenteId, token_hash: &[u8]) -> AppResult<()> {
        self.asistente_mem(id)?.token = Some(token_hash.to_vec());
        Ok(())
    }

    async fn vincular_bien(
        &mut self,
        reunion: ReunionId,
        asistente: AsistenteId,
        bien: BienPrivadoId,
        calidad: CalidadRepresentacion,
        poder: Option<PoderId>,
        _desde: DateTime<Utc>,
    ) -> AppResult<()> {
        if self
            .a()
            .vinculos
            .iter()
            .any(|v| v.reunion == reunion && v.bien == bien && v.hasta.is_none())
        {
            return Err(AppError::Conflicto("ux_bien_en_un_asistente".into()));
        }
        self.datos.asambleas.vinculos.push(VinculoMem {
            reunion,
            asistente,
            bien,
            calidad,
            poder,
            hasta: None,
        });
        Ok(())
    }

    async fn desvincular_bien(
        &mut self,
        reunion: ReunionId,
        bien: BienPrivadoId,
        hasta: DateTime<Utc>,
    ) -> AppResult<Option<AsistenteId>> {
        Ok(self
            .datos
            .asambleas
            .vinculos
            .iter_mut()
            .find(|v| v.reunion == reunion && v.bien == bien && v.hasta.is_none())
            .map(|v| {
                v.hasta = Some(hasta);
                v.asistente
            }))
    }

    async fn insertar_poder(&mut self, p: &Poder) -> AppResult<()> {
        if self.a().poderes.iter().any(|x| {
            x.reunion_id == p.reunion_id
                && x.bien_privado_id == p.bien_privado_id
                && x.estado.vigente()
        }) {
            return Err(AppError::Conflicto("ux_poder_vigente".into()));
        }
        self.datos.asambleas.poderes.push(p.clone());
        Ok(())
    }

    async fn poder(&mut self, id: PoderId) -> AppResult<Option<Poder>> {
        Ok(self.a().poderes.iter().find(|p| p.id == id).cloned())
    }

    async fn actualizar_poder(&mut self, p: &Poder) -> AppResult<()> {
        if let Some(x) = self
            .datos
            .asambleas
            .poderes
            .iter_mut()
            .find(|x| x.id == p.id)
        {
            *x = p.clone();
        }
        Ok(())
    }

    async fn poderes(&mut self, reunion: ReunionId) -> AppResult<Vec<Poder>> {
        Ok(self
            .a()
            .poderes
            .iter()
            .filter(|p| p.reunion_id == reunion)
            .cloned()
            .collect())
    }

    async fn abrir_presencia(&mut self, reunion: ReunionId, p: &Presencia) -> AppResult<()> {
        if self
            .a()
            .presencias
            .iter()
            .any(|(_, x)| x.asistente_id == p.asistente_id && x.salida_en.is_none())
        {
            return Err(AppError::Conflicto("ux_presencia_abierta".into()));
        }
        self.datos.asambleas.presencias.push((reunion, p.clone()));
        Ok(())
    }

    async fn cerrar_presencia(
        &mut self,
        asistente: AsistenteId,
        salida: DateTime<Utc>,
        motivo: MotivoSalida,
        detalle: Option<&str>,
    ) -> AppResult<bool> {
        Ok(self
            .datos
            .asambleas
            .presencias
            .iter_mut()
            .find(|(_, p)| p.asistente_id == asistente && p.salida_en.is_none())
            .map(|(_, p)| {
                p.salida_en = Some(salida);
                p.motivo_salida = Some(motivo);
                p.detalle_salida = detalle.map(str::to_owned);
            })
            .is_some())
    }

    async fn presencias(&mut self, reunion: ReunionId) -> AppResult<Vec<Presencia>> {
        Ok(self
            .a()
            .presencias
            .iter()
            .filter(|(r, _)| *r == reunion)
            .map(|(_, p)| p.clone())
            .collect())
    }

    async fn bienes_presentes(&mut self, reunion: ReunionId) -> AppResult<Vec<BienPresente>> {
        let presentes: Vec<AsistenteId> = self
            .a()
            .presencias
            .iter()
            .filter(|(r, p)| *r == reunion && p.salida_en.is_none())
            .map(|(_, p)| p.asistente_id)
            .collect();
        Ok(self
            .a()
            .vinculos
            .iter()
            .filter(|v| {
                v.reunion == reunion && v.hasta.is_none() && presentes.contains(&v.asistente)
            })
            .filter_map(|v| {
                self.coef_padron(reunion, v.bien).map(|c| BienPresente {
                    bien_privado_id: v.bien,
                    coeficiente: c,
                    asistente_id: v.asistente,
                    moroso: self.es_moroso(reunion, v.bien),
                })
            })
            .collect())
    }

    async fn asignar_rotulo(
        &mut self,
        reunion: ReunionId,
        codigo: &CodigoRotulo,
        asistente: AsistenteId,
        _ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        if self.a().rotulos.iter().any(|(r, c, a, liberado)| {
            !liberado && *r == reunion && (c == codigo || *a == asistente)
        }) {
            return Err(AppError::Conflicto("ux_rotulo_vigente".into()));
        }
        self.datos
            .asambleas
            .rotulos
            .push((reunion, codigo.clone(), asistente, false));
        Ok(())
    }

    async fn liberar_rotulo(
        &mut self,
        asistente: AsistenteId,
        _ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut liberado = false;
        for (_, _, a, l) in &mut self.datos.asambleas.rotulos {
            if *a == asistente && !*l {
                *l = true;
                liberado = true;
            }
        }
        Ok(liberado)
    }

    async fn insertar_foto(&mut self, reunion: ReunionId, f: &FotoQuorum) -> AppResult<()> {
        self.datos.asambleas.fotos.push((reunion, f.clone()));
        Ok(())
    }

    async fn fotos(&mut self, reunion: ReunionId) -> AppResult<Vec<FotoQuorum>> {
        Ok(self
            .a()
            .fotos
            .iter()
            .filter(|(r, _)| *r == reunion)
            .map(|(_, f)| f.clone())
            .collect())
    }

    async fn guardar_pregunta(&mut self, p: &PreguntaGuardada, nueva: bool) -> AppResult<()> {
        let v = &mut self.datos.asambleas.preguntas;
        if nueva {
            v.push(p.clone());
        } else if let Some(x) = v.iter_mut().find(|x| x.pregunta.id == p.pregunta.id) {
            *x = p.clone();
        }
        Ok(())
    }

    async fn eliminar_pregunta(&mut self, id: PreguntaId) -> AppResult<bool> {
        let antes = self.a().preguntas.len();
        self.datos
            .asambleas
            .preguntas
            .retain(|p| p.pregunta.id != id);
        Ok(self.a().preguntas.len() < antes)
    }

    async fn pregunta(&mut self, id: PreguntaId) -> AppResult<Option<PreguntaGuardada>> {
        Ok(self
            .a()
            .preguntas
            .iter()
            .find(|p| p.pregunta.id == id)
            .cloned())
    }

    async fn bloquear_pregunta(&mut self, id: PreguntaId) -> AppResult<Option<PreguntaGuardada>> {
        self.pregunta(id).await
    }

    async fn preguntas(&mut self, reunion: ReunionId) -> AppResult<Vec<PreguntaGuardada>> {
        let mut v: Vec<_> = self
            .a()
            .preguntas
            .iter()
            .filter(|p| p.pregunta.reunion_id == reunion)
            .cloned()
            .collect();
        v.sort_by_key(|p| p.orden);
        Ok(v)
    }

    async fn preguntas_vencidas(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<PreguntaId>> {
        Ok(self
            .a()
            .preguntas
            .iter()
            .filter(|p| {
                p.pregunta.estado == EstadoPregunta::Abierta
                    && p.cierra_en.is_some_and(|c| c <= ahora)
            })
            .map(|p| p.pregunta.id)
            .collect())
    }

    async fn consultas_vencidas(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<ReunionId>> {
        Ok(self
            .a()
            .reuniones
            .iter()
            .filter(|r| {
                r.modalidad == Modalidad::Escrita
                    && r.estado == EstadoReunion::Instalada
                    && r.plazo_respuesta.is_some_and(|p| p <= ahora)
            })
            .map(|r| r.id)
            .collect())
    }

    async fn actualizar_pregunta(&mut self, p: &PreguntaGuardada) -> AppResult<()> {
        self.guardar_pregunta(p, false).await
    }

    async fn habilitar(
        &mut self,
        pregunta: PreguntaId,
        bienes: &[(BienPrivadoId, Coeficiente)],
        _desde: DateTime<Utc>,
    ) -> AppResult<()> {
        for (b, c) in bienes {
            if !self
                .a()
                .habilitados
                .iter()
                .any(|(p, x, _)| *p == pregunta && x == b)
            {
                self.datos.asambleas.habilitados.push((pregunta, *b, *c));
            }
        }
        Ok(())
    }

    async fn conteo_habilitados(&mut self, pregunta: PreguntaId) -> AppResult<Conteo> {
        let h: Vec<_> = self
            .a()
            .habilitados
            .iter()
            .filter(|(p, _, _)| *p == pregunta)
            .collect();
        Ok(Conteo::new(
            h.iter().map(|(_, _, c)| c.valor()).sum(),
            u32::try_from(h.len()).unwrap_or(u32::MAX),
        ))
    }

    async fn pendientes_de_voto(
        &mut self,
        pregunta: PreguntaId,
        bienes: &[BienPrivadoId],
    ) -> AppResult<Vec<(BienPrivadoId, Coeficiente)>> {
        Ok(self
            .a()
            .habilitados
            .iter()
            .filter(|(p, b, _)| *p == pregunta && bienes.contains(b))
            .filter(|(p, b, _)| {
                !self
                    .a()
                    .participaciones
                    .iter()
                    .any(|x| x.pregunta_id == *p && x.bien_privado_id == *b)
            })
            .map(|(_, b, c)| (*b, *c))
            .collect())
    }

    async fn participaciones_con_clave(
        &mut self,
        pregunta: PreguntaId,
        asistente: AsistenteId,
        clave: &str,
    ) -> AppResult<u32> {
        Ok(u32::try_from(
            self.a()
                .participaciones
                .iter()
                .filter(|p| {
                    p.pregunta_id == pregunta
                        && p.asistente_id == asistente
                        && p.idempotency_key.as_str() == clave
                })
                .count(),
        )
        .unwrap_or(u32::MAX))
    }

    async fn registrar_voto(&mut self, v: &VotoEmitido) -> AppResult<u32> {
        let mut n = 0;
        for p in &v.participaciones {
            let estado = self
                .a()
                .preguntas
                .iter()
                .find(|x| x.pregunta.id == p.pregunta_id)
                .map(|x| x.pregunta.estado);
            if estado != Some(EstadoPregunta::Abierta) {
                return Err(DominioError::PreguntaNoAbierta(
                    estado.unwrap_or(EstadoPregunta::Anulada),
                )
                .into());
            }
            if !self.a().habilitados.iter().any(|(q, b, c)| {
                *q == p.pregunta_id && *b == p.bien_privado_id && *c == p.coeficiente
            }) {
                return Err(AppError::Conflicto(
                    "el bien privado no está habilitado para votar esta pregunta".into(),
                ));
            }
            if self
                .a()
                .participaciones
                .iter()
                .any(|x| x.pregunta_id == p.pregunta_id && x.bien_privado_id == p.bien_privado_id)
            {
                continue;
            }
            self.datos.asambleas.participaciones.push(p.clone());
            self.datos.asambleas.votos.extend(
                v.votos
                    .iter()
                    .filter(|x| x.participacion_id == p.id)
                    .cloned(),
            );
            n += 1;
        }
        Ok(n)
    }

    async fn conteo_votos(&mut self, pregunta: PreguntaId) -> AppResult<ConteoVotos> {
        let parts: Vec<_> = self
            .a()
            .participaciones
            .iter()
            .filter(|p| p.pregunta_id == pregunta)
            .collect();
        let mut por_opcion: Vec<VotosOpcion> = Vec::new();
        for v in self.a().votos.iter().filter(|v| v.pregunta_id == pregunta) {
            match por_opcion.iter_mut().find(|o| o.opcion_id == v.opcion_id) {
                Some(o) => {
                    o.votos.coeficiente += v.coeficiente.valor();
                    o.votos.unidades += 1;
                }
                None => por_opcion.push(VotosOpcion {
                    opcion_id: v.opcion_id,
                    votos: Conteo::new(v.coeficiente.valor(), 1),
                }),
            }
        }
        Ok(ConteoVotos {
            votantes: Conteo::new(
                parts.iter().map(|p| p.coeficiente.valor()).sum(),
                u32::try_from(parts.len()).unwrap_or(u32::MAX),
            ),
            por_opcion,
        })
    }

    async fn detalle_votos(&mut self, pregunta: PreguntaId) -> AppResult<Vec<DetalleVoto>> {
        Ok(self
            .a()
            .votos
            .iter()
            .filter(|v| v.pregunta_id == pregunta)
            .filter_map(|v| {
                self.a()
                    .participaciones
                    .iter()
                    .find(|p| p.id == v.participacion_id)
                    .map(|p| DetalleVoto {
                        bien_privado_id: v.bien_privado_id,
                        codigo: self.codigo(v.bien_privado_id),
                        coeficiente: v.coeficiente,
                        opcion_id: v.opcion_id,
                        canal: p.canal,
                        asistente: String::new(),
                        apoderado: p.apoderado_id.is_some(),
                        registrado_en: p.registrado_en,
                    })
            })
            .collect())
    }

    async fn participaciones(&mut self, pregunta: PreguntaId) -> AppResult<Vec<ParticipacionActa>> {
        Ok(self
            .a()
            .participaciones
            .iter()
            .filter(|p| p.pregunta_id == pregunta)
            .map(|p| ParticipacionActa {
                bien_privado_id: p.bien_privado_id,
                codigo: self.codigo(p.bien_privado_id),
                coeficiente: p.coeficiente,
                canal: p.canal,
                asistente: String::new(),
                apoderado: p.apoderado_id.is_some(),
                registrado_en: p.registrado_en,
            })
            .collect())
    }

    async fn asistente_voto(
        &mut self,
        pregunta: PreguntaId,
        asistente: AsistenteId,
    ) -> AppResult<bool> {
        Ok(self
            .a()
            .participaciones
            .iter()
            .any(|p| p.pregunta_id == pregunta && p.asistente_id == asistente))
    }

    async fn avisar_en_vivo(&mut self, reunion: ReunionId) -> AppResult<()> {
        self.datos.asambleas.avisos_en_vivo.push(reunion);
        Ok(())
    }

    async fn encolar_de_reunion(
        &mut self,
        reunion: ReunionId,
        evento: &EventoSaliente,
    ) -> AppResult<i64> {
        self.datos.asambleas.siguiente_outbox += 1;
        let id = self.a().siguiente_outbox;
        self.datos.asambleas.outbox_reunion.push((reunion, id));
        self.salientes.push(evento.clone());
        Ok(id)
    }

    async fn registrar_envio_convocatoria(
        &mut self,
        reunion: ReunionId,
        persona: PersonaId,
        canal: CanalConvocatoria,
        outbox_id: i64,
    ) -> AppResult<()> {
        self.datos
            .asambleas
            .envios
            .push((reunion, persona, canal, outbox_id));
        Ok(())
    }

    async fn envios_convocatoria(
        &mut self,
        reunion: ReunionId,
    ) -> AppResult<Vec<EnvioConvocatoria>> {
        Ok(self
            .a()
            .envios
            .iter()
            .filter(|(r, ..)| *r == reunion)
            .map(|(_, p, c, _)| EnvioConvocatoria {
                persona_id: *p,
                nombre: String::new(),
                canal: *c,
                estado_envio: "pendiente".into(),
                acuse: None,
                enviado_en: None,
            })
            .collect())
    }

    async fn estados_entrega(&mut self, outbox: &[i64]) -> AppResult<Vec<(i64, Option<String>)>> {
        Ok(outbox.iter().map(|o| (*o, None)).collect())
    }

    async fn mensajes_whatsapp(&mut self, _reunion: ReunionId) -> AppResult<Vec<MensajesLinea>> {
        Ok(Vec::new())
    }

    async fn tarifas_whatsapp(
        &mut self,
        _pais: &str,
        _moneda: &str,
        _fecha: NaiveDate,
    ) -> AppResult<Vec<(String, Decimal)>> {
        Ok(vec![
            ("utilidad".into(), Decimal::new(295, 2)),
            ("servicio".into(), Decimal::new(295, 2)),
        ])
    }

    async fn elegir_linea(&mut self) -> AppResult<Option<String>> {
        Ok(self.a().linea.clone())
    }
}
