//! Control de acceso sobre la transacción del tenant (plan 05): configuración, visitas,
//! correspondencia, minuta, puntos, puentes, dispositivos, credenciales, autorizaciones,
//! eventos y consentimientos biométricos. Lo que cruza copropiedades va por
//! [`DirectorioAccesoPg`] (funciones `SECURITY DEFINER`); los dispositivos push, por la
//! transacción de plataforma.

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    outbox::EventoSaliente,
    puertos::{
        DirectorioAcceso, RepositorioAcceso, RepositorioDispositivosPush,
        acceso::{
            AlertaAcceso, AutorizacionAcceso, CambiosAcceso, ConsentimientoBiometrico,
            ConteoAvisos, Correspondencia, Credencial, CredencialSincronizada, DispositivoAcceso,
            DispositivoPush, EntregaCorrespondencia, EventoAcceso, EventoVisita,
            FiltroCorrespondencia, FiltroEventosAcceso, FiltroMinuta, FiltroVisitas, NovedadMinuta,
            Puente, PuntoAcceso, SoportesVencidos, TitularAcceso, VentanaSincronizada, Visita,
            VisitaSincronizada,
        },
    },
};
use chrono::{DateTime, NaiveTime, TimeDelta, Utc};
use domain::{
    ArchivoId, AutorizacionAccesoId, BienPrivadoId, CanalRespuestaVisita, ConfiguracionAcceso,
    ConsentimientoBiometricoId, CorrespondenciaId, CredencialId, CuentaId, DispositivoAccesoId,
    DispositivoPushId, Documento, EstadoCorrespondencia, EstadoCredencial, EstadoPuente,
    EstadoVisita, EventoAccesoId, Horario, HorarioSilencio, NovedadMinutaId,
    OrigenAutorizacionAcceso, OrigenVisita, PersonaId, Placa, PlataformaPush, PuenteId,
    PuntoAccesoId, RecursoId, ResultadoAcceso, SentidoAcceso, TenantId, TipoCorrespondencia,
    TipoCredencial, TipoDispositivoAcceso, TipoEventoVisita, TipoNovedadMinuta, TipoPuntoAcceso,
    TipoVisita, VehiculoId, VisitaId,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    TransaccionPg, TransaccionPlataformaPg,
    copropiedades::{error_escritura, opcional},
};

fn u16_de(v: i16) -> u16 {
    u16::try_from(v).unwrap_or(0)
}

fn u32_de(v: i16) -> u32 {
    u32::try_from(v).unwrap_or(0)
}

fn i16_de(v: u32) -> i16 {
    i16::try_from(v).unwrap_or(i16::MAX)
}

fn ids<T>(v: Vec<Uuid>, f: impl Fn(Uuid) -> T) -> Vec<T> {
    v.into_iter().map(f).collect()
}

fn horario_de(v: Option<serde_json::Value>) -> AppResult<Option<Horario>> {
    v.map(serde_json::from_value)
        .transpose()
        .map_err(AppError::infra)
}

fn json_de(h: Option<&Horario>) -> AppResult<Option<serde_json::Value>> {
    h.map(serde_json::to_value)
        .transpose()
        .map_err(AppError::infra)
}

// ---------------------------------------------------------------------------------------------
// Filas
// ---------------------------------------------------------------------------------------------

struct FilaVisita {
    id: Uuid,
    bien_privado_id: Uuid,
    origen: String,
    tipo: String,
    estado: String,
    visitante_nombre: String,
    visitante_documento: Option<String>,
    placa: Option<String>,
    observacion: Option<String>,
    decidida_por_persona_id: Option<Uuid>,
    canal_decision: Option<String>,
    ventana_desde: Option<DateTime<Utc>>,
    ventana_hasta: Option<DateTime<Utc>>,
    usos_permitidos: i16,
    usos: i16,
    avisos_enviados: i16,
    siguiente_paso_en: Option<DateTime<Utc>>,
    registrada_por: Option<Uuid>,
    creada_en: DateTime<Utc>,
    decidida_en: Option<DateTime<Utc>>,
    ingreso_en: Option<DateTime<Utc>>,
    salida_en: Option<DateTime<Utc>>,
}

impl TryFrom<FilaVisita> for Visita {
    type Error = AppError;

    fn try_from(f: FilaVisita) -> AppResult<Self> {
        Ok(Self {
            id: VisitaId::from_uuid(f.id),
            bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
            origen: OrigenVisita::try_from(f.origen.as_str())?,
            tipo: TipoVisita::try_from(f.tipo.as_str())?,
            estado: EstadoVisita::try_from(f.estado.as_str())?,
            visitante_nombre: f.visitante_nombre,
            visitante_documento: opcional(f.visitante_documento, Documento::new)?,
            placa: opcional(f.placa, Placa::new)?,
            observacion: f.observacion,
            decidida_por: f.decidida_por_persona_id.map(PersonaId::from_uuid),
            canal_decision: opcional(f.canal_decision, |c| CanalRespuestaVisita::try_from(c))?,
            ventana_desde: f.ventana_desde,
            ventana_hasta: f.ventana_hasta,
            usos_permitidos: u16_de(f.usos_permitidos),
            usos: u16_de(f.usos),
            avisos_enviados: u32_de(f.avisos_enviados),
            siguiente_paso_en: f.siguiente_paso_en,
            registrada_por: f.registrada_por.map(CuentaId::from_uuid),
            creada_en: f.creada_en,
            decidida_en: f.decidida_en,
            ingreso_en: f.ingreso_en,
            salida_en: f.salida_en,
        })
    }
}

fn visitas_de(v: Vec<FilaVisita>) -> AppResult<Vec<Visita>> {
    v.into_iter().map(Visita::try_from).collect()
}

struct FilaCorrespondencia {
    id: Uuid,
    bien_privado_id: Uuid,
    tipo: String,
    estado: String,
    remitente: Option<String>,
    guia: Option<String>,
    ubicacion: Option<String>,
    foto_archivo_id: Option<Uuid>,
    recibida_por: Option<Uuid>,
    recibida_en: DateTime<Utc>,
    avisos_enviados: i16,
    ultimo_aviso_en: Option<DateTime<Utc>>,
    siguiente_aviso_en: Option<DateTime<Utc>>,
    alerta_administracion_en: Option<DateTime<Utc>>,
    cerrada_en: Option<DateTime<Utc>>,
    motivo_devolucion: Option<String>,
}

impl TryFrom<FilaCorrespondencia> for Correspondencia {
    type Error = AppError;

    fn try_from(f: FilaCorrespondencia) -> AppResult<Self> {
        Ok(Self {
            id: CorrespondenciaId::from_uuid(f.id),
            bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
            tipo: TipoCorrespondencia::try_from(f.tipo.as_str())?,
            estado: EstadoCorrespondencia::try_from(f.estado.as_str())?,
            remitente: f.remitente,
            guia: f.guia,
            ubicacion: f.ubicacion,
            foto_archivo_id: f.foto_archivo_id.map(ArchivoId::from_uuid),
            recibida_por: f.recibida_por.map(CuentaId::from_uuid),
            recibida_en: f.recibida_en,
            avisos_enviados: u32_de(f.avisos_enviados),
            ultimo_aviso_en: f.ultimo_aviso_en,
            siguiente_aviso_en: f.siguiente_aviso_en,
            alerta_administracion_en: f.alerta_administracion_en,
            cerrada_en: f.cerrada_en,
            motivo_devolucion: f.motivo_devolucion,
        })
    }
}

fn correspondencias_de(v: Vec<FilaCorrespondencia>) -> AppResult<Vec<Correspondencia>> {
    v.into_iter().map(Correspondencia::try_from).collect()
}

struct FilaCredencial {
    id: Uuid,
    persona_id: Option<Uuid>,
    vehiculo_id: Option<Uuid>,
    tipo: String,
    huella: String,
    estado: String,
    vigente_desde: DateTime<Utc>,
    vigente_hasta: Option<DateTime<Utc>>,
    borrado_confirmado_en: Option<DateTime<Utc>>,
    creada_en: DateTime<Utc>,
    version: i64,
}

fn titular(p: Option<Uuid>, v: Option<Uuid>) -> AppResult<TitularAcceso> {
    TitularAcceso::desde_columnas(p.map(PersonaId::from_uuid), v.map(VehiculoId::from_uuid)).ok_or(
        AppError::infra(std::io::Error::other("credencial sin titular")),
    )
}

impl TryFrom<&FilaCredencial> for Credencial {
    type Error = AppError;

    fn try_from(f: &FilaCredencial) -> AppResult<Self> {
        Ok(Self {
            id: CredencialId::from_uuid(f.id),
            titular: titular(f.persona_id, f.vehiculo_id)?,
            tipo: TipoCredencial::try_from(f.tipo.as_str())?,
            huella: f.huella.clone(),
            estado: EstadoCredencial::try_from(f.estado.as_str())?,
            vigente_desde: f.vigente_desde,
            vigente_hasta: f.vigente_hasta,
            borrado_confirmado_en: f.borrado_confirmado_en,
            creada_en: f.creada_en,
        })
    }
}

struct FilaAutorizacion {
    id: Uuid,
    persona_id: Option<Uuid>,
    vehiculo_id: Option<Uuid>,
    origen: String,
    puntos: Vec<Uuid>,
    horario: Option<serde_json::Value>,
    vigente_desde: DateTime<Utc>,
    vigente_hasta: Option<DateTime<Utc>>,
    motivo: Option<String>,
    revocada_en: Option<DateTime<Utc>>,
}

impl TryFrom<FilaAutorizacion> for AutorizacionAcceso {
    type Error = AppError;

    fn try_from(f: FilaAutorizacion) -> AppResult<Self> {
        Ok(Self {
            id: AutorizacionAccesoId::from_uuid(f.id),
            titular: titular(f.persona_id, f.vehiculo_id)?,
            origen: OrigenAutorizacionAcceso::try_from(f.origen.as_str())?,
            puntos: ids(f.puntos, PuntoAccesoId::from_uuid),
            horario: horario_de(f.horario)?,
            vigente_desde: f.vigente_desde,
            vigente_hasta: f.vigente_hasta,
            motivo: f.motivo,
            revocada_en: f.revocada_en,
        })
    }
}

struct FilaEvento {
    id: Uuid,
    punto_acceso_id: Option<Uuid>,
    dispositivo_id: Option<Uuid>,
    puente_id: Option<Uuid>,
    id_local: Option<String>,
    credencial_id: Option<Uuid>,
    visita_id: Option<Uuid>,
    sentido: String,
    resultado: String,
    motivo: Option<String>,
    placa: Option<String>,
    registrado_por: Option<Uuid>,
    ocurrido_en: DateTime<Utc>,
}

impl TryFrom<FilaEvento> for EventoAcceso {
    type Error = AppError;

    fn try_from(f: FilaEvento) -> AppResult<Self> {
        Ok(Self {
            id: EventoAccesoId::from_uuid(f.id),
            punto_acceso_id: f.punto_acceso_id.map(PuntoAccesoId::from_uuid),
            dispositivo_id: f.dispositivo_id.map(DispositivoAccesoId::from_uuid),
            puente_id: f.puente_id.map(PuenteId::from_uuid),
            id_local: f.id_local,
            credencial_id: f.credencial_id.map(CredencialId::from_uuid),
            visita_id: f.visita_id.map(VisitaId::from_uuid),
            sentido: SentidoAcceso::try_from(f.sentido.as_str())?,
            resultado: ResultadoAcceso::try_from(f.resultado.as_str())?,
            motivo: f.motivo,
            placa: opcional(f.placa, Placa::new)?,
            registrado_por: f.registrado_por.map(CuentaId::from_uuid),
            ocurrido_en: f.ocurrido_en,
        })
    }
}

struct FilaPuente {
    id: Uuid,
    nombre: String,
    estado: String,
    credencial_hash: String,
    version: Option<String>,
    ultimo_latido_en: Option<DateTime<Utc>>,
    creado_en: DateTime<Utc>,
}

impl TryFrom<FilaPuente> for Puente {
    type Error = AppError;

    fn try_from(f: FilaPuente) -> AppResult<Self> {
        Ok(Self {
            id: PuenteId::from_uuid(f.id),
            nombre: f.nombre,
            estado: EstadoPuente::try_from(f.estado.as_str())?,
            credencial_hash: f.credencial_hash,
            version: f.version,
            ultimo_latido_en: f.ultimo_latido_en,
            creado_en: f.creado_en,
        })
    }
}

struct FilaConsentimiento {
    id: Uuid,
    persona_id: Uuid,
    version_texto: String,
    soporte_archivo_id: Option<Uuid>,
    otorgado_en: DateTime<Utc>,
    revocado_en: Option<DateTime<Utc>>,
    registrado_por: Option<Uuid>,
}

impl From<FilaConsentimiento> for ConsentimientoBiometrico {
    fn from(f: FilaConsentimiento) -> Self {
        Self {
            id: ConsentimientoBiometricoId::from_uuid(f.id),
            persona_id: PersonaId::from_uuid(f.persona_id),
            version_texto: f.version_texto,
            soporte_archivo_id: f.soporte_archivo_id.map(ArchivoId::from_uuid),
            otorgado_en: f.otorgado_en,
            revocado_en: f.revocado_en,
            registrado_por: f.registrado_por.map(CuentaId::from_uuid),
        }
    }
}

impl TransaccionPg {
    async fn ventanas_sql(
        &mut self,
        t: TitularAcceso,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<VentanaSincronizada>> {
        let (p, v) = t.columnas();
        let filas = sqlx::query!(
            r#"SELECT puntos AS "puntos!", vigente_desde AS "vigente_desde!", vigente_hasta, horario
               FROM ventanas_acceso($1, $2, $3)"#,
            p.map(PersonaId::as_uuid),
            v.map(VehiculoId::as_uuid),
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        filas
            .into_iter()
            .map(|f| {
                Ok(VentanaSincronizada {
                    puntos: ids(f.puntos, PuntoAccesoId::from_uuid),
                    vigente_desde: f.vigente_desde,
                    vigente_hasta: f.vigente_hasta,
                    horario: horario_de(f.horario)?,
                })
            })
            .collect()
    }
}

impl RepositorioAcceso for TransaccionPg {
    async fn configuracion_acceso(&mut self) -> AppResult<ConfiguracionAcceso> {
        let fila = sqlx::query!(
            "SELECT retencion_soportes_dias, dias_avisos_correspondencia, silencio_inicio,
                    silencio_fin, retencion_eventos_dias, retencion_fotos_placas_dias
             FROM configuracion_acceso"
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let Some(f) = fila else {
            return Ok(ConfiguracionAcceso::default());
        };
        let silencio = match (f.silencio_inicio, f.silencio_fin) {
            (Some(i), Some(fin)) => Some(HorarioSilencio::new(i, fin)?),
            _ => None,
        };
        Ok(ConfiguracionAcceso {
            retencion_soportes_dias: u32_de(f.retencion_soportes_dias),
            dias_avisos_correspondencia: u32_de(f.dias_avisos_correspondencia),
            silencio,
            retencion_eventos_dias: u32_de(f.retencion_eventos_dias),
            retencion_fotos_placas_dias: u32_de(f.retencion_fotos_placas_dias),
        })
    }

    async fn guardar_configuracion_acceso(&mut self, c: &ConfiguracionAcceso) -> AppResult<()> {
        let (inicio, fin): (Option<NaiveTime>, Option<NaiveTime>) = c
            .silencio
            .map_or((None, None), |s| (Some(s.inicio), Some(s.fin)));
        sqlx::query!(
            "INSERT INTO configuracion_acceso (tenant_id, retencion_soportes_dias,
                 dias_avisos_correspondencia, silencio_inicio, silencio_fin,
                 retencion_eventos_dias, retencion_fotos_placas_dias)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (tenant_id) DO UPDATE SET
                 retencion_soportes_dias = EXCLUDED.retencion_soportes_dias,
                 dias_avisos_correspondencia = EXCLUDED.dias_avisos_correspondencia,
                 silencio_inicio = EXCLUDED.silencio_inicio, silencio_fin = EXCLUDED.silencio_fin,
                 retencion_eventos_dias = EXCLUDED.retencion_eventos_dias,
                 retencion_fotos_placas_dias = EXCLUDED.retencion_fotos_placas_dias,
                 actualizado_en = now()",
            self.ctx.tenant_id.as_uuid(),
            i16_de(c.retencion_soportes_dias),
            i16_de(c.dias_avisos_correspondencia),
            inicio,
            fin,
            i16_de(c.retencion_eventos_dias),
            i16_de(c.retencion_fotos_placas_dias),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    // --- Visitas ---

    async fn insertar_visita(&mut self, v: &Visita) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO visitas (id, tenant_id, bien_privado_id, origen, tipo, estado,
                 visitante_nombre, visitante_documento, placa, observacion,
                 decidida_por_persona_id, canal_decision, ventana_desde, ventana_hasta,
                 usos_permitidos, usos, avisos_enviados, siguiente_paso_en, registrada_por,
                 creada_en, decidida_en, ingreso_en, salida_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18, $19, $20, $21, $22, $23)",
            v.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            v.bien_privado_id.as_uuid(),
            v.origen.as_str(),
            v.tipo.as_str(),
            v.estado.as_str(),
            v.visitante_nombre,
            v.visitante_documento.as_ref().map(Documento::as_str),
            v.placa.as_ref().map(Placa::as_str),
            v.observacion,
            v.decidida_por.map(PersonaId::as_uuid),
            v.canal_decision.map(CanalRespuestaVisita::as_str),
            v.ventana_desde,
            v.ventana_hasta,
            i16::try_from(v.usos_permitidos).unwrap_or(i16::MAX),
            i16::try_from(v.usos).unwrap_or(i16::MAX),
            i16_de(v.avisos_enviados),
            v.siguiente_paso_en,
            v.registrada_por.map(CuentaId::as_uuid),
            v.creada_en,
            v.decidida_en,
            v.ingreso_en,
            v.salida_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "visita repetida", "bien privado"))?;
        Ok(())
    }

    async fn actualizar_visita(&mut self, v: &Visita) -> AppResult<()> {
        sqlx::query!(
            "UPDATE visitas SET estado = $2, decidida_por_persona_id = $3, canal_decision = $4,
                 ventana_desde = $5, ventana_hasta = $6, usos = $7, avisos_enviados = $8,
                 siguiente_paso_en = $9, decidida_en = $10, ingreso_en = $11, salida_en = $12
             WHERE id = $1",
            v.id.as_uuid(),
            v.estado.as_str(),
            v.decidida_por.map(PersonaId::as_uuid),
            v.canal_decision.map(CanalRespuestaVisita::as_str),
            v.ventana_desde,
            v.ventana_hasta,
            i16::try_from(v.usos).unwrap_or(i16::MAX),
            i16_de(v.avisos_enviados),
            v.siguiente_paso_en,
            v.decidida_en,
            v.ingreso_en,
            v.salida_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn visita_para_actualizar(&mut self, id: VisitaId) -> AppResult<Option<Visita>> {
        sqlx::query_as!(
            FilaVisita,
            "SELECT id, bien_privado_id, origen, tipo, estado, visitante_nombre,
                    visitante_documento, placa, observacion, decidida_por_persona_id,
                    canal_decision, ventana_desde, ventana_hasta, usos_permitidos, usos,
                    avisos_enviados, siguiente_paso_en, registrada_por, creada_en, decidida_en,
                    ingreso_en, salida_en
             FROM visitas WHERE id = $1 FOR UPDATE",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Visita::try_from)
        .transpose()
    }

    async fn visita(&mut self, id: VisitaId) -> AppResult<Option<Visita>> {
        sqlx::query_as!(
            FilaVisita,
            "SELECT id, bien_privado_id, origen, tipo, estado, visitante_nombre,
                    visitante_documento, placa, observacion, decidida_por_persona_id,
                    canal_decision, ventana_desde, ventana_hasta, usos_permitidos, usos,
                    avisos_enviados, siguiente_paso_en, registrada_por, creada_en, decidida_en,
                    ingreso_en, salida_en
             FROM visitas WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Visita::try_from)
        .transpose()
    }

    async fn visitas(
        &mut self,
        f: &FiltroVisitas,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<Visita>> {
        let bienes: Option<Vec<Uuid>> = f
            .bienes
            .as_ref()
            .map(|b| b.iter().map(|x| x.as_uuid()).collect());
        let filas = sqlx::query_as!(
            FilaVisita,
            "SELECT id, bien_privado_id, origen, tipo, estado, visitante_nombre,
                    visitante_documento, placa, observacion, decidida_por_persona_id,
                    canal_decision, ventana_desde, ventana_hasta, usos_permitidos, usos,
                    avisos_enviados, siguiente_paso_en, registrada_por, creada_en, decidida_en,
                    ingreso_en, salida_en
             FROM visitas
             WHERE ($1::text IS NULL OR estado = $1)
               AND ($2::uuid IS NULL OR bien_privado_id = $2)
               AND ($3::uuid[] IS NULL OR bien_privado_id = ANY($3))
               AND ($4::timestamptz IS NULL OR creada_en >= $4)
               AND (NOT $5 OR estado = 'ingresada')
               AND ($6::uuid IS NULL OR id < $6)
             ORDER BY id DESC LIMIT $7",
            f.estado.map(EstadoVisita::as_str),
            f.bien_privado_id.map(BienPrivadoId::as_uuid),
            bienes.as_deref(),
            f.desde,
            f.dentro,
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(Pagina::desde_filas(visitas_de(filas)?, p, |v| {
            v.id.as_uuid()
        }))
    }

    async fn visitas_con_paso_pendiente(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<Visita>> {
        let filas = sqlx::query_as!(
            FilaVisita,
            "SELECT id, bien_privado_id, origen, tipo, estado, visitante_nombre,
                    visitante_documento, placa, observacion, decidida_por_persona_id,
                    canal_decision, ventana_desde, ventana_hasta, usos_permitidos, usos,
                    avisos_enviados, siguiente_paso_en, registrada_por, creada_en, decidida_en,
                    ingreso_en, salida_en
             FROM visitas WHERE siguiente_paso_en <= $1 ORDER BY siguiente_paso_en LIMIT 500",
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        visitas_de(filas)
    }

    async fn insertar_evento_visita(&mut self, e: &EventoVisita) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO visita_eventos (tenant_id, visita_id, tipo, canal, persona_id, cuenta_id,
                                         nivel, destinatarios, ocurrido_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            self.ctx.tenant_id.as_uuid(),
            e.visita_id.as_uuid(),
            e.tipo.as_str(),
            e.canal.map(CanalRespuestaVisita::as_str),
            e.persona_id.map(PersonaId::as_uuid),
            e.cuenta_id.map(CuentaId::as_uuid),
            e.nivel.map(i16_de),
            e.destinatarios.map(i16_de),
            e.ocurrido_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn eventos_visita(&mut self, id: VisitaId) -> AppResult<Vec<EventoVisita>> {
        sqlx::query!(
            "SELECT tipo, canal, persona_id, cuenta_id, nivel, destinatarios, ocurrido_en
             FROM visita_eventos WHERE visita_id = $1 ORDER BY id",
            id.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(EventoVisita {
                visita_id: id,
                tipo: TipoEventoVisita::try_from(f.tipo.as_str())?,
                canal: opcional(f.canal, |c| CanalRespuestaVisita::try_from(c))?,
                persona_id: f.persona_id.map(PersonaId::from_uuid),
                cuenta_id: f.cuenta_id.map(CuentaId::from_uuid),
                nivel: f.nivel.map(u32_de),
                destinatarios: f.destinatarios.map(u32_de),
                ocurrido_en: f.ocurrido_en,
            })
        })
        .collect()
    }

    // --- Correspondencia ---

    async fn insertar_correspondencia(&mut self, c: &Correspondencia) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO correspondencia (id, tenant_id, bien_privado_id, tipo, estado, remitente,
                 guia, ubicacion, foto_archivo_id, recibida_por, recibida_en, avisos_enviados,
                 ultimo_aviso_en, siguiente_aviso_en, alerta_administracion_en, cerrada_en,
                 motivo_devolucion)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.bien_privado_id.as_uuid(),
            c.tipo.as_str(),
            c.estado.as_str(),
            c.remitente,
            c.guia,
            c.ubicacion,
            c.foto_archivo_id.map(ArchivoId::as_uuid),
            c.recibida_por.map(CuentaId::as_uuid),
            c.recibida_en,
            i16_de(c.avisos_enviados),
            c.ultimo_aviso_en,
            c.siguiente_aviso_en,
            c.alerta_administracion_en,
            c.cerrada_en,
            c.motivo_devolucion,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "correspondencia repetida", "bien privado"))?;
        Ok(())
    }

    async fn actualizar_correspondencia(&mut self, c: &Correspondencia) -> AppResult<()> {
        sqlx::query!(
            "UPDATE correspondencia SET estado = $2, avisos_enviados = $3, ultimo_aviso_en = $4,
                 siguiente_aviso_en = $5, alerta_administracion_en = $6, cerrada_en = $7,
                 motivo_devolucion = $8
             WHERE id = $1",
            c.id.as_uuid(),
            c.estado.as_str(),
            i16_de(c.avisos_enviados),
            c.ultimo_aviso_en,
            c.siguiente_aviso_en,
            c.alerta_administracion_en,
            c.cerrada_en,
            c.motivo_devolucion,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn correspondencia_para_actualizar(
        &mut self,
        id: CorrespondenciaId,
    ) -> AppResult<Option<Correspondencia>> {
        sqlx::query_as!(
            FilaCorrespondencia,
            "SELECT id, bien_privado_id, tipo, estado, remitente, guia, ubicacion, foto_archivo_id,
                    recibida_por, recibida_en, avisos_enviados, ultimo_aviso_en,
                    siguiente_aviso_en, alerta_administracion_en, cerrada_en, motivo_devolucion
             FROM correspondencia WHERE id = $1 FOR UPDATE",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Correspondencia::try_from)
        .transpose()
    }

    async fn correspondencia(
        &mut self,
        id: CorrespondenciaId,
    ) -> AppResult<Option<Correspondencia>> {
        sqlx::query_as!(
            FilaCorrespondencia,
            "SELECT id, bien_privado_id, tipo, estado, remitente, guia, ubicacion, foto_archivo_id,
                    recibida_por, recibida_en, avisos_enviados, ultimo_aviso_en,
                    siguiente_aviso_en, alerta_administracion_en, cerrada_en, motivo_devolucion
             FROM correspondencia WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Correspondencia::try_from)
        .transpose()
    }

    async fn correspondencias(
        &mut self,
        f: &FiltroCorrespondencia,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<Correspondencia>> {
        let bienes: Option<Vec<Uuid>> = f
            .bienes
            .as_ref()
            .map(|b| b.iter().map(|x| x.as_uuid()).collect());
        let filas = sqlx::query_as!(
            FilaCorrespondencia,
            "SELECT id, bien_privado_id, tipo, estado, remitente, guia, ubicacion, foto_archivo_id,
                    recibida_por, recibida_en, avisos_enviados, ultimo_aviso_en,
                    siguiente_aviso_en, alerta_administracion_en, cerrada_en, motivo_devolucion
             FROM correspondencia
             WHERE ($1::text IS NULL OR estado = $1)
               AND ($2::uuid IS NULL OR bien_privado_id = $2)
               AND ($3::uuid[] IS NULL OR bien_privado_id = ANY($3))
               AND ($4::uuid IS NULL OR id < $4)
             ORDER BY id DESC LIMIT $5",
            f.estado.map(EstadoCorrespondencia::as_str),
            f.bien_privado_id.map(BienPrivadoId::as_uuid),
            bienes.as_deref(),
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(Pagina::desde_filas(correspondencias_de(filas)?, p, |c| {
            c.id.as_uuid()
        }))
    }

    async fn correspondencia_con_aviso_pendiente(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<Correspondencia>> {
        let filas = sqlx::query_as!(
            FilaCorrespondencia,
            "SELECT id, bien_privado_id, tipo, estado, remitente, guia, ubicacion, foto_archivo_id,
                    recibida_por, recibida_en, avisos_enviados, ultimo_aviso_en,
                    siguiente_aviso_en, alerta_administracion_en, cerrada_en, motivo_devolucion
             FROM correspondencia
             WHERE estado = 'recibida'
               AND (siguiente_aviso_en <= $1
                    OR (siguiente_aviso_en IS NULL AND alerta_administracion_en IS NULL))
             ORDER BY siguiente_aviso_en NULLS FIRST LIMIT 500",
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        correspondencias_de(filas)
    }

    async fn insertar_entrega(&mut self, e: &EntregaCorrespondencia) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO entregas_correspondencia (tenant_id, correspondencia_id, persona_id,
                 tercero_nombre, tercero_documento, firma_archivo_id, foto_archivo_id,
                 entregada_por, entregada_en, soportes_hasta)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            self.ctx.tenant_id.as_uuid(),
            e.correspondencia_id.as_uuid(),
            e.persona_id.map(PersonaId::as_uuid),
            e.tercero_nombre,
            e.tercero_documento.as_ref().map(Documento::as_str),
            e.firma_archivo_id.map(ArchivoId::as_uuid),
            e.foto_archivo_id.map(ArchivoId::as_uuid),
            e.entregada_por.map(CuentaId::as_uuid),
            e.entregada_en,
            e.soportes_hasta,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|err| error_escritura(err, "la correspondencia ya fue entregada", "persona"))?;
        Ok(())
    }

    async fn entrega(
        &mut self,
        id: CorrespondenciaId,
    ) -> AppResult<Option<EntregaCorrespondencia>> {
        sqlx::query!(
            "SELECT persona_id, tercero_nombre, tercero_documento, firma_archivo_id,
                    foto_archivo_id, entregada_por, entregada_en, soportes_hasta
             FROM entregas_correspondencia WHERE correspondencia_id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| {
            Ok(EntregaCorrespondencia {
                correspondencia_id: id,
                persona_id: f.persona_id.map(PersonaId::from_uuid),
                tercero_nombre: f.tercero_nombre,
                tercero_documento: opcional(f.tercero_documento, Documento::new)?,
                firma_archivo_id: f.firma_archivo_id.map(ArchivoId::from_uuid),
                foto_archivo_id: f.foto_archivo_id.map(ArchivoId::from_uuid),
                entregada_por: f.entregada_por.map(CuentaId::from_uuid),
                entregada_en: f.entregada_en,
                soportes_hasta: f.soportes_hasta,
            })
        })
        .transpose()
    }

    async fn soportes_vencidos(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<SoportesVencidos>> {
        Ok(sqlx::query!(
            "SELECT e.correspondencia_id, e.firma_archivo_id, e.foto_archivo_id
             FROM entregas_correspondencia e
             WHERE e.soportes_hasta <= $1
               AND NOT EXISTS (SELECT 1 FROM purgas_soportes_entrega p
                               WHERE p.tenant_id = e.tenant_id
                                 AND p.correspondencia_id = e.correspondencia_id)
             LIMIT 500",
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| SoportesVencidos {
            correspondencia_id: CorrespondenciaId::from_uuid(f.correspondencia_id),
            archivos: f
                .firma_archivo_id
                .into_iter()
                .chain(f.foto_archivo_id)
                .map(ArchivoId::from_uuid)
                .collect(),
        })
        .collect())
    }

    async fn marcar_soportes_purgados(
        &mut self,
        id: CorrespondenciaId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO purgas_soportes_entrega (tenant_id, correspondencia_id, purgado_en)
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            self.ctx.tenant_id.as_uuid(),
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    // --- Minuta ---

    async fn insertar_novedad(&mut self, n: &NovedadMinuta) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO minuta_porteria (id, tenant_id, tipo, texto, punto_acceso_id,
                                          registrada_por, ocurrida_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            n.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            n.tipo.as_str(),
            n.texto,
            n.punto_acceso_id.map(PuntoAccesoId::as_uuid),
            n.registrada_por.map(CuentaId::as_uuid),
            n.ocurrida_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "novedad repetida", "punto de acceso"))?;
        for a in &n.adjuntos {
            sqlx::query!(
                "INSERT INTO minuta_adjuntos (tenant_id, novedad_id, archivo_id) VALUES ($1, $2, $3)",
                self.ctx.tenant_id.as_uuid(),
                n.id.as_uuid(),
                a.as_uuid(),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        }
        Ok(())
    }

    async fn novedades(
        &mut self,
        f: &FiltroMinuta,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<NovedadMinuta>> {
        let filas = sqlx::query!(
            r#"SELECT m.id, m.tipo, m.texto, m.punto_acceso_id, m.registrada_por, m.ocurrida_en,
                      coalesce(array_agg(a.archivo_id) FILTER (WHERE a.archivo_id IS NOT NULL),
                               '{}') AS "adjuntos!"
               FROM minuta_porteria m
               LEFT JOIN minuta_adjuntos a ON a.tenant_id = m.tenant_id AND a.novedad_id = m.id
               WHERE ($1::text IS NULL OR m.tipo = $1)
                 AND ($2::timestamptz IS NULL OR m.ocurrida_en >= $2)
                 AND ($3::timestamptz IS NULL OR m.ocurrida_en < $3)
                 AND ($4::uuid IS NULL OR m.id < $4)
               GROUP BY m.id
               ORDER BY m.id DESC LIMIT $5"#,
            f.tipo.map(TipoNovedadMinuta::as_str),
            f.desde,
            f.hasta,
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(|r| {
                Ok(NovedadMinuta {
                    id: NovedadMinutaId::from_uuid(r.id),
                    tipo: TipoNovedadMinuta::try_from(r.tipo.as_str())?,
                    texto: r.texto,
                    punto_acceso_id: r.punto_acceso_id.map(PuntoAccesoId::from_uuid),
                    adjuntos: ids(r.adjuntos, ArchivoId::from_uuid),
                    registrada_por: r.registrada_por.map(CuentaId::from_uuid),
                    ocurrida_en: r.ocurrida_en,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(v, p, |n| n.id.as_uuid()))
    }

    async fn tokens_push(&mut self, cuentas: &[CuentaId]) -> AppResult<Vec<(CuentaId, String)>> {
        let cuentas: Vec<Uuid> = cuentas.iter().map(|c| c.as_uuid()).collect();
        Ok(sqlx::query!(
            r#"SELECT cuenta_id AS "cuenta_id!", token AS "token!" FROM tokens_push_de_cuentas($1)"#,
            &cuentas,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| (CuentaId::from_uuid(f.cuenta_id), f.token))
        .collect())
    }

    async fn encolar_aviso_acceso(&mut self, evento: &EventoSaliente) -> AppResult<()> {
        let payload = serde_json::to_value(&evento.mensaje).map_err(AppError::infra)?;
        sqlx::query!(
            "INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload, concepto)
             VALUES ($1, $2, $3, $4, $5, $6, 'control_acceso')",
            self.ctx.tenant_id.as_uuid(),
            evento.mensaje.canal().as_str(),
            evento.prioridad.as_i16(),
            evento.mensaje.linea(),
            evento.mensaje.destino(),
            payload,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn mensajes_avisos_acceso(
        &mut self,
        desde: DateTime<Utc>,
        hasta: DateTime<Utc>,
    ) -> AppResult<ConteoAvisos> {
        let f = sqlx::query!(
            r#"SELECT push_enviados AS "push!", whatsapp_enviados AS "whatsapp!", fallidos AS "fallidos!"
               FROM avisos_acceso_enviados($1, $2)"#,
            desde,
            hasta,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let n = |v: i64| u64::try_from(v).unwrap_or(0);
        Ok(ConteoAvisos {
            push_enviados: n(f.push),
            whatsapp_enviados: n(f.whatsapp),
            fallidos: n(f.fallidos),
        })
    }

    // --- Puntos, puentes y dispositivos ---

    async fn guardar_punto_acceso(&mut self, p: &PuntoAcceso, nuevo: bool) -> AppResult<()> {
        let r = if nuevo {
            sqlx::query!(
                "INSERT INTO puntos_acceso (id, tenant_id, nombre, tipo, sentido, recurso_id, activo)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                p.id.as_uuid(),
                self.ctx.tenant_id.as_uuid(),
                p.nombre,
                p.tipo.as_str(),
                p.sentido.as_str(),
                p.recurso_id.map(RecursoId::as_uuid),
                p.activo,
            )
            .execute(&mut *self.tx)
            .await
        } else {
            sqlx::query!(
                "UPDATE puntos_acceso SET nombre = $2, tipo = $3, sentido = $4, recurso_id = $5,
                                          activo = $6
                 WHERE id = $1",
                p.id.as_uuid(),
                p.nombre,
                p.tipo.as_str(),
                p.sentido.as_str(),
                p.recurso_id.map(RecursoId::as_uuid),
                p.activo,
            )
            .execute(&mut *self.tx)
            .await
        };
        r.map_err(|e| {
            error_escritura(e, "ya existe un punto de acceso con ese nombre", "recurso")
        })?;
        Ok(())
    }

    async fn puntos_acceso(&mut self) -> AppResult<Vec<PuntoAcceso>> {
        sqlx::query!(
            "SELECT id, nombre, tipo, sentido, recurso_id, activo FROM puntos_acceso
             ORDER BY normalizar(nombre)"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(PuntoAcceso {
                id: PuntoAccesoId::from_uuid(f.id),
                nombre: f.nombre,
                tipo: TipoPuntoAcceso::try_from(f.tipo.as_str())?,
                sentido: SentidoAcceso::try_from(f.sentido.as_str())?,
                recurso_id: f.recurso_id.map(RecursoId::from_uuid),
                activo: f.activo,
            })
        })
        .collect()
    }

    async fn insertar_puente(&mut self, p: &Puente) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO puentes (id, tenant_id, nombre, estado, credencial_hash, version,
                                  ultimo_latido_en, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            p.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            p.nombre,
            p.estado.as_str(),
            p.credencial_hash,
            p.version,
            p.ultimo_latido_en,
            p.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn actualizar_puente(&mut self, p: &Puente) -> AppResult<()> {
        sqlx::query!(
            "UPDATE puentes SET nombre = $2, estado = $3, credencial_hash = $4, version = $5,
                                ultimo_latido_en = $6
             WHERE id = $1",
            p.id.as_uuid(),
            p.nombre,
            p.estado.as_str(),
            p.credencial_hash,
            p.version,
            p.ultimo_latido_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn puente(&mut self, id: PuenteId) -> AppResult<Option<Puente>> {
        sqlx::query_as!(
            FilaPuente,
            "SELECT id, nombre, estado, credencial_hash, version, ultimo_latido_en, creado_en
             FROM puentes WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Puente::try_from)
        .transpose()
    }

    async fn puentes(&mut self) -> AppResult<Vec<Puente>> {
        sqlx::query_as!(
            FilaPuente,
            "SELECT id, nombre, estado, credencial_hash, version, ultimo_latido_en, creado_en
             FROM puentes ORDER BY creado_en, id"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Puente::try_from)
        .collect()
    }

    async fn guardar_dispositivo(&mut self, d: &DispositivoAcceso, nuevo: bool) -> AppResult<()> {
        let r = if nuevo {
            sqlx::query!(
                "INSERT INTO dispositivos_acceso (id, tenant_id, puente_id, punto_acceso_id, tipo,
                                                  fabricante, modelo, config, activo)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                d.id.as_uuid(),
                self.ctx.tenant_id.as_uuid(),
                d.puente_id.as_uuid(),
                d.punto_acceso_id.as_uuid(),
                d.tipo.as_str(),
                d.fabricante,
                d.modelo,
                d.config,
                d.activo,
            )
            .execute(&mut *self.tx)
            .await
        } else {
            sqlx::query!(
                "UPDATE dispositivos_acceso SET puente_id = $2, punto_acceso_id = $3, tipo = $4,
                     fabricante = $5, modelo = $6, config = $7, activo = $8
                 WHERE id = $1",
                d.id.as_uuid(),
                d.puente_id.as_uuid(),
                d.punto_acceso_id.as_uuid(),
                d.tipo.as_str(),
                d.fabricante,
                d.modelo,
                d.config,
                d.activo,
            )
            .execute(&mut *self.tx)
            .await
        };
        r.map_err(|e| error_escritura(e, "dispositivo repetido", "puente o punto de acceso"))?;
        Ok(())
    }

    async fn dispositivos(
        &mut self,
        puente: Option<PuenteId>,
    ) -> AppResult<Vec<DispositivoAcceso>> {
        sqlx::query!(
            "SELECT id, puente_id, punto_acceso_id, tipo, fabricante, modelo, config, activo
             FROM dispositivos_acceso WHERE ($1::uuid IS NULL OR puente_id = $1) ORDER BY id",
            puente.map(PuenteId::as_uuid),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(DispositivoAcceso {
                id: DispositivoAccesoId::from_uuid(f.id),
                puente_id: PuenteId::from_uuid(f.puente_id),
                punto_acceso_id: PuntoAccesoId::from_uuid(f.punto_acceso_id),
                tipo: TipoDispositivoAcceso::try_from(f.tipo.as_str())?,
                fabricante: f.fabricante,
                modelo: f.modelo,
                config: f.config,
                activo: f.activo,
            })
        })
        .collect()
    }

    // --- Credenciales y autorizaciones ---

    async fn insertar_credencial(&mut self, c: &Credencial) -> AppResult<()> {
        let (p, v) = c.titular.columnas();
        sqlx::query!(
            "INSERT INTO credenciales (id, tenant_id, persona_id, vehiculo_id, tipo, huella, estado,
                                       vigente_desde, vigente_hasta, borrado_confirmado_en, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            p.map(PersonaId::as_uuid),
            v.map(VehiculoId::as_uuid),
            c.tipo.as_str(),
            c.huella,
            c.estado.as_str(),
            c.vigente_desde,
            c.vigente_hasta,
            c.borrado_confirmado_en,
            c.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "ya existe una credencial vigente con ese identificador", "titular"))?;
        Ok(())
    }

    async fn actualizar_credencial(&mut self, c: &Credencial) -> AppResult<()> {
        sqlx::query!(
            "UPDATE credenciales SET estado = $2, vigente_hasta = $3, borrado_confirmado_en = $4
             WHERE id = $1",
            c.id.as_uuid(),
            c.estado.as_str(),
            c.vigente_hasta,
            c.borrado_confirmado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "ya existe una credencial vigente con ese identificador",
                "titular",
            )
        })?;
        Ok(())
    }

    async fn credencial(&mut self, id: CredencialId) -> AppResult<Option<Credencial>> {
        sqlx::query_as!(
            FilaCredencial,
            "SELECT id, persona_id, vehiculo_id, tipo, huella, estado, vigente_desde, vigente_hasta,
                    borrado_confirmado_en, creada_en, version
             FROM credenciales WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .as_ref()
        .map(Credencial::try_from)
        .transpose()
    }

    async fn credencial_por_huella(&mut self, huella: &str) -> AppResult<Option<Credencial>> {
        sqlx::query_as!(
            FilaCredencial,
            "SELECT id, persona_id, vehiculo_id, tipo, huella, estado, vigente_desde, vigente_hasta,
                    borrado_confirmado_en, creada_en, version
             FROM credenciales WHERE huella = $1
             ORDER BY (estado IN ('activa', 'suspendida')) DESC, creada_en DESC LIMIT 1",
            huella,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .as_ref()
        .map(Credencial::try_from)
        .transpose()
    }

    async fn credenciales_de(&mut self, t: TitularAcceso) -> AppResult<Vec<Credencial>> {
        let (p, v) = t.columnas();
        sqlx::query_as!(
            FilaCredencial,
            "SELECT id, persona_id, vehiculo_id, tipo, huella, estado, vigente_desde, vigente_hasta,
                    borrado_confirmado_en, creada_en, version
             FROM credenciales WHERE persona_id = $1 OR vehiculo_id = $2 ORDER BY creada_en, id",
            p.map(PersonaId::as_uuid),
            v.map(VehiculoId::as_uuid),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .iter()
        .map(Credencial::try_from)
        .collect()
    }

    async fn insertar_autorizacion_acceso(&mut self, a: &AutorizacionAcceso) -> AppResult<()> {
        let (p, v) = a.titular.columnas();
        let puntos: Vec<Uuid> = a.puntos.iter().map(|x| x.as_uuid()).collect();
        sqlx::query!(
            "INSERT INTO autorizaciones_acceso (id, tenant_id, persona_id, vehiculo_id, origen,
                 puntos, horario, vigente_desde, vigente_hasta, motivo, revocada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            a.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            p.map(PersonaId::as_uuid),
            v.map(VehiculoId::as_uuid),
            a.origen.as_str(),
            &puntos,
            json_de(a.horario.as_ref())?,
            a.vigente_desde,
            a.vigente_hasta,
            a.motivo,
            a.revocada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "autorización repetida", "titular"))?;
        Ok(())
    }

    async fn revocar_autorizacion_acceso(
        &mut self,
        id: AutorizacionAccesoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        let r = sqlx::query!(
            "UPDATE autorizaciones_acceso SET revocada_en = $2 WHERE id = $1 AND revocada_en IS NULL",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(r.rows_affected() == 1)
    }

    async fn autorizaciones_de(
        &mut self,
        t: TitularAcceso,
        incluir_revocadas: bool,
    ) -> AppResult<Vec<AutorizacionAcceso>> {
        let (p, v) = t.columnas();
        sqlx::query_as!(
            FilaAutorizacion,
            "SELECT id, persona_id, vehiculo_id, origen, puntos, horario, vigente_desde,
                    vigente_hasta, motivo, revocada_en
             FROM autorizaciones_acceso
             WHERE (persona_id = $1 OR vehiculo_id = $2) AND ($3 OR revocada_en IS NULL)
             ORDER BY creada_en, id",
            p.map(PersonaId::as_uuid),
            v.map(VehiculoId::as_uuid),
            incluir_revocadas,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(AutorizacionAcceso::try_from)
        .collect()
    }

    async fn ventanas_de(
        &mut self,
        t: TitularAcceso,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<VentanaSincronizada>> {
        self.ventanas_sql(t, ahora).await
    }

    async fn cambios_desde(
        &mut self,
        cursor: i64,
        limite: u32,
        ahora: DateTime<Utc>,
    ) -> AppResult<CambiosAcceso> {
        let filas = sqlx::query_as!(
            FilaCredencial,
            "SELECT id, persona_id, vehiculo_id, tipo, huella, estado, vigente_desde, vigente_hasta,
                    borrado_confirmado_en, creada_en, version
             FROM credenciales WHERE version > $1 ORDER BY version LIMIT $2",
            cursor,
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let mut credenciales = Vec::with_capacity(filas.len());
        for f in &filas {
            let c = Credencial::try_from(f)?;
            let activa = c.estado == EstadoCredencial::Activa
                && c.vigente_desde <= ahora
                && c.vigente_hasta.is_none_or(|h| h > ahora);
            let ventanas = if activa {
                self.ventanas_sql(c.titular, ahora).await?
            } else {
                Vec::new()
            };
            credenciales.push(CredencialSincronizada {
                id: c.id,
                tipo: c.tipo,
                huella: c.huella,
                activa,
                vigente_hasta: c.vigente_hasta,
                ventanas,
                borrar_plantilla: c.tipo == TipoCredencial::BiometriaRef
                    && !activa
                    && c.borrado_confirmado_en.is_none(),
                version: f.version,
            });
        }
        let visitas = sqlx::query!(
            "SELECT id, estado, usos, version FROM visitas
             WHERE origen = 'preautorizada' AND version > $1 ORDER BY version LIMIT $2",
            cursor,
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| VisitaSincronizada {
            id: VisitaId::from_uuid(f.id),
            activa: matches!(f.estado.as_str(), "autorizada" | "ingresada"),
            usos: u16_de(f.usos),
            version: f.version,
        })
        .collect::<Vec<_>>();
        // El cursor avanza hasta lo más alto que se entregó completo de ambas listas.
        let max_c = credenciales.last().map(|c| c.version);
        let max_v = visitas.last().map(|v| v.version);
        let lleno_c = credenciales.len() as u32 >= limite;
        let lleno_v = visitas.len() as u32 >= limite;
        let nuevo = match (lleno_c, lleno_v) {
            (true, true) => max_c.into_iter().chain(max_v).min(),
            (true, false) => max_c,
            (false, true) => max_v,
            (false, false) => max_c.into_iter().chain(max_v).max(),
        }
        .unwrap_or(cursor);
        Ok(CambiosAcceso {
            credenciales,
            visitas,
            cursor: nuevo.max(cursor),
        })
    }

    // --- Eventos ---

    async fn insertar_evento_acceso(&mut self, e: &EventoAcceso) -> AppResult<bool> {
        let r = sqlx::query!(
            "INSERT INTO eventos_acceso (id, tenant_id, punto_acceso_id, dispositivo_id, puente_id,
                 id_local, credencial_id, visita_id, sentido, resultado, motivo, placa,
                 registrado_por, ocurrido_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
             ON CONFLICT (tenant_id, puente_id, id_local, ocurrido_en) DO NOTHING",
            e.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            e.punto_acceso_id.map(PuntoAccesoId::as_uuid),
            e.dispositivo_id.map(DispositivoAccesoId::as_uuid),
            e.puente_id.map(PuenteId::as_uuid),
            e.id_local,
            e.credencial_id.map(CredencialId::as_uuid),
            e.visita_id.map(VisitaId::as_uuid),
            e.sentido.as_str(),
            e.resultado.as_str(),
            e.motivo,
            e.placa.as_ref().map(Placa::as_str),
            e.registrado_por.map(CuentaId::as_uuid),
            e.ocurrido_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|err| {
            error_escritura(
                err,
                "evento repetido",
                "punto, dispositivo, credencial o visita",
            )
        })?;
        Ok(r.rows_affected() == 1)
    }

    async fn adjuntar_foto_evento(
        &mut self,
        evento: EventoAccesoId,
        archivo: ArchivoId,
        purgar_en: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO fotos_eventos_acceso (tenant_id, evento_id, archivo_id, purgar_en)
             VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
            self.ctx.tenant_id.as_uuid(),
            evento.as_uuid(),
            archivo.as_uuid(),
            purgar_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn eventos_acceso(
        &mut self,
        f: &FiltroEventosAcceso,
        p: SolicitudPagina,
    ) -> AppResult<Pagina<EventoAcceso>> {
        // Cursor por id (UUID v7: orden de creación); la búsqueda usa el índice por fecha.
        let filas = sqlx::query_as!(
            FilaEvento,
            "SELECT id, punto_acceso_id, dispositivo_id, puente_id, id_local, credencial_id,
                    visita_id, sentido, resultado, motivo, placa, registrado_por, ocurrido_en
             FROM eventos_acceso
             WHERE ($1::timestamptz IS NULL OR ocurrido_en >= $1)
               AND ($2::timestamptz IS NULL OR ocurrido_en < $2)
               AND ($3::uuid IS NULL OR punto_acceso_id = $3)
               AND ($4::text IS NULL OR resultado = $4)
               AND ($5::uuid IS NULL OR id < $5)
             ORDER BY id DESC LIMIT $6",
            f.desde,
            f.hasta,
            f.punto_acceso_id.map(PuntoAccesoId::as_uuid),
            f.resultado.map(ResultadoAcceso::as_str),
            p.despues(),
            p.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let v = filas
            .into_iter()
            .map(EventoAcceso::try_from)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(v, p, |e| e.id.as_uuid()))
    }

    async fn fotos_eventos_vencidas(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<(EventoAccesoId, ArchivoId)>> {
        Ok(sqlx::query!(
            "SELECT evento_id, archivo_id FROM fotos_eventos_acceso
             WHERE purgar_en <= $1 AND purgada_en IS NULL LIMIT 1000",
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            (
                EventoAccesoId::from_uuid(f.evento_id),
                ArchivoId::from_uuid(f.archivo_id),
            )
        })
        .collect())
    }

    async fn marcar_foto_evento_purgada(
        &mut self,
        evento: EventoAccesoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE fotos_eventos_acceso SET purgada_en = $2 WHERE evento_id = $1",
            evento.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn purgar_eventos_acceso(&mut self, dias: u32) -> AppResult<u64> {
        let n = sqlx::query_scalar!(
            r#"SELECT purgar_eventos_acceso($1) AS "n!""#,
            i32::try_from(dias).unwrap_or(i32::MAX),
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    async fn alertas_acceso(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<AlertaAcceso>> {
        let mut alertas: Vec<AlertaAcceso> = sqlx::query!(
            "SELECT id, nombre, ultimo_latido_en FROM puentes
             WHERE estado = 'activo' AND (ultimo_latido_en IS NULL OR ultimo_latido_en < $1)",
            ahora - TOLERANCIA_LATIDO,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| AlertaAcceso::PuenteSinLatido {
            puente: PuenteId::from_uuid(f.id),
            nombre: f.nombre,
            ultimo: f.ultimo_latido_en,
        })
        .collect();
        let desde = ahora - VENTANA_ALERTAS;
        alertas.extend(
            sqlx::query!(
                "SELECT punto_acceso_id, ocurrido_en FROM eventos_acceso
                 WHERE resultado = 'forzado' AND ocurrido_en >= $1 ORDER BY ocurrido_en DESC LIMIT 50",
                desde,
            )
            .fetch_all(&mut *self.tx)
            .await
            .map_err(AppError::infra)?
            .into_iter()
            .map(|f| AlertaAcceso::PuertaForzada {
                punto: f.punto_acceso_id.map(PuntoAccesoId::from_uuid),
                ocurrido_en: f.ocurrido_en,
            }),
        );
        alertas.extend(
            sqlx::query!(
                r#"SELECT punto_acceso_id, count(*) AS "n!" FROM eventos_acceso
                   WHERE resultado = 'denegado' AND ocurrido_en >= $1
                   GROUP BY punto_acceso_id HAVING count(*) >= $2"#,
                ahora - TimeDelta::minutes(10),
                DENEGADOS_PARA_ALERTA,
            )
            .fetch_all(&mut *self.tx)
            .await
            .map_err(AppError::infra)?
            .into_iter()
            .map(|f| AlertaAcceso::DenegadosRepetidos {
                punto: f.punto_acceso_id.map(PuntoAccesoId::from_uuid),
                intentos: u32::try_from(f.n).unwrap_or(u32::MAX),
            }),
        );
        Ok(alertas)
    }

    // --- Biometría ---

    async fn insertar_consentimiento(&mut self, c: &ConsentimientoBiometrico) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO consentimientos_biometricos (id, tenant_id, persona_id, version_texto,
                 soporte_archivo_id, otorgado_en, revocado_en, registrado_por)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.persona_id.as_uuid(),
            c.version_texto,
            c.soporte_archivo_id.map(ArchivoId::as_uuid),
            c.otorgado_en,
            c.revocado_en,
            c.registrado_por.map(CuentaId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "la persona ya tiene un consentimiento biométrico vigente",
                "persona",
            )
        })?;
        Ok(())
    }

    async fn consentimiento_vigente(
        &mut self,
        persona: PersonaId,
    ) -> AppResult<Option<ConsentimientoBiometrico>> {
        Ok(sqlx::query_as!(
            FilaConsentimiento,
            "SELECT id, persona_id, version_texto, soporte_archivo_id, otorgado_en, revocado_en,
                    registrado_por
             FROM consentimientos_biometricos WHERE persona_id = $1 AND revocado_en IS NULL",
            persona.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Into::into))
    }

    async fn revocar_consentimiento(
        &mut self,
        id: ConsentimientoBiometricoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE consentimientos_biometricos SET revocado_en = $2 WHERE id = $1 AND revocado_en IS NULL",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

/// Un puente sin latido por más de esto genera alerta.
const TOLERANCIA_LATIDO: TimeDelta = TimeDelta::minutes(5);
/// Las puertas forzadas se alertan durante este tiempo.
const VENTANA_ALERTAS: TimeDelta = TimeDelta::hours(24);
/// Denegados en 10 minutos en un mismo punto para alertar.
const DENEGADOS_PARA_ALERTA: i64 = 5;

// ---------------------------------------------------------------------------------------------
// Directorio (cruza copropiedades)
// ---------------------------------------------------------------------------------------------

/// Directorio del worker de acceso y de la API de puentes.
#[derive(Debug, Clone)]
pub struct DirectorioAccesoPg {
    pool: PgPool,
}

impl DirectorioAccesoPg {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl DirectorioAcceso for DirectorioAccesoPg {
    async fn copropiedades_con_acceso_pendiente(
        &self,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<TenantId>> {
        Ok(sqlx::query_scalar!(
            r#"SELECT tenant_id AS "tenant_id!" FROM copropiedades_con_acceso_pendiente($1)"#,
            ahora,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TenantId::from_uuid)
        .collect())
    }

    async fn copropiedades_con_control_acceso(&self) -> AppResult<Vec<TenantId>> {
        Ok(sqlx::query_scalar!(
            r#"SELECT tenant_id AS "tenant_id!" FROM copropiedades_con_control_acceso()"#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TenantId::from_uuid)
        .collect())
    }

    async fn puente_por_credencial(&self, huella: &str) -> AppResult<Option<(TenantId, PuenteId)>> {
        Ok(sqlx::query!(
            r#"SELECT tenant_id AS "tenant_id!", puente_id AS "puente_id!" FROM puente_por_credencial($1)"#,
            huella,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::infra)?
        .map(|f| (TenantId::from_uuid(f.tenant_id), PuenteId::from_uuid(f.puente_id))))
    }

    async fn preparar_particiones(&self, meses: u32) -> AppResult<()> {
        sqlx::query!(
            "SELECT crear_particiones_eventos_acceso($1)",
            i32::try_from(meses).unwrap_or(24),
        )
        .execute(&self.pool)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Dispositivos push (plataforma)
// ---------------------------------------------------------------------------------------------

impl RepositorioDispositivosPush for TransaccionPlataformaPg {
    async fn registrar_dispositivo_push(
        &mut self,
        d: &DispositivoPush,
    ) -> AppResult<DispositivoPush> {
        let f = sqlx::query!(
            "INSERT INTO dispositivos_push (id, cuenta_id, plataforma, token, nombre, activo, registrado_en)
             VALUES ($1, $2, $3, $4, $5, TRUE, $6)
             ON CONFLICT (token) DO UPDATE SET cuenta_id = EXCLUDED.cuenta_id,
                 plataforma = EXCLUDED.plataforma, nombre = EXCLUDED.nombre, activo = TRUE,
                 actualizado_en = now()
             RETURNING id, registrado_en",
            d.id.as_uuid(),
            d.cuenta_id.as_uuid(),
            d.plataforma.as_str(),
            d.token,
            d.nombre,
            d.registrado_en,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(DispositivoPush {
            id: DispositivoPushId::from_uuid(f.id),
            registrado_en: f.registrado_en,
            activo: true,
            ..d.clone()
        })
    }

    async fn dispositivos_push(&mut self, cuenta: CuentaId) -> AppResult<Vec<DispositivoPush>> {
        sqlx::query!(
            "SELECT id, plataforma, token, nombre, activo, registrado_en FROM dispositivos_push
             WHERE cuenta_id = $1 AND activo ORDER BY registrado_en",
            cuenta.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(DispositivoPush {
                id: DispositivoPushId::from_uuid(f.id),
                cuenta_id: cuenta,
                plataforma: PlataformaPush::try_from(f.plataforma.as_str())?,
                token: f.token,
                nombre: f.nombre,
                activo: f.activo,
                registrado_en: f.registrado_en,
            })
        })
        .collect()
    }

    async fn retirar_dispositivo_push(
        &mut self,
        cuenta: CuentaId,
        id: DispositivoPushId,
    ) -> AppResult<bool> {
        let r = sqlx::query!(
            "UPDATE dispositivos_push SET activo = FALSE, actualizado_en = now()
             WHERE id = $1 AND cuenta_id = $2 AND activo",
            id.as_uuid(),
            cuenta.as_uuid(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(r.rows_affected() == 1)
    }
}
