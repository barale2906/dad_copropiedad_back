//! Asambleas sobre la transacción del tenant (plan 06): reuniones, padrón, asistentes con sus
//! bienes, poderes, presencias, rótulos, preguntas, votos, fotos del quórum, constancias y
//! mensajes de la asamblea. También el directorio que cruza copropiedades (gateway) y las
//! líneas y tarifas de WhatsApp (plataforma).

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    outbox::EventoSaliente,
    puertos::{
        RepositorioAsambleas, RepositorioLineasWhatsapp,
        asambleas::{
            AcuseEntrega, AsistenciaTelefono, Asistente, AsistenteNuevo, BienDeAsistente,
            BienPadron, BienParaPadron, BienPresente, CanalConvocatoria, CoincidenciaMesa,
            ConfiguracionReunion, ConteoVotos, DetalleVoto, DirectorioAsambleas, DocumentoReunion,
            EnvioConvocatoria, EstadoLinea, FotoQuorum, LineaWhatsapp, MensajesLinea,
            ParticipacionActa, Poder, PreguntaGuardada, Presencia, PuntoOrden, Reunion,
            TarifaWhatsapp,
        },
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, AsistenteId, BaseComputo, BienPrivadoId, CalidadRepresentacion, Canal,
    CargoEleccion, CodigoRotulo, Coeficiente, Conteo, Convocatoria, CuentaId, DominioError,
    EfectoMora, Escrutinio, EstadoActa, EstadoPoder, EstadoPregunta, EstadoPunto, EstadoReunion,
    MetodoRegistro, Modalidad, MomentoQuorum, MotivoSalida, OpcionId, OpcionPregunta, PersonaId,
    PoderId, Pregunta, PreguntaId, PresenciaId, PuntoOrdenId, QuorumReunion, ReglaMayoria,
    ReunionId, TenantId, TipoDocumentoReunion, TipoPregunta, TipoReunion, TipoVoto, VotoEmitido,
    VotosOpcion,
};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{TransaccionPg, TransaccionPlataformaPg, copropiedades::error_escritura};

/// Canal de `pg_notify` de las pantallas en vivo (`{tenant}:{reunion}`).
pub const CANAL_EN_VIVO: &str = "asambleas_en_vivo";

fn coef(d: Decimal) -> AppResult<Coeficiente> {
    Ok(Coeficiente::new(d)?)
}

fn entero<T: TryFrom<i64>>(v: i64, campo: &'static str) -> AppResult<T> {
    T::try_from(v).map_err(|_| AppError::Validacion(format!("`{campo}` fuera de rango")))
}

fn opt_u16(v: Option<i16>, campo: &'static str) -> AppResult<Option<u16>> {
    v.map(|x| entero(i64::from(x), campo)).transpose()
}

fn opt_u32(v: Option<i32>, campo: &'static str) -> AppResult<Option<u32>> {
    v.map(|x| entero(i64::from(x), campo)).transpose()
}

fn escapar_like(t: &str) -> String {
    format!(
        "%{}%",
        t.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

// ---------------------------------------------------------------------------------------------
// Reuniones
// ---------------------------------------------------------------------------------------------

struct FilaReunion {
    id: Uuid,
    titulo: String,
    tipo: String,
    convocatoria: String,
    modalidad: String,
    estado: String,
    fecha_programada: DateTime<Utc>,
    lugar: Option<String>,
    enlace_video: Option<String>,
    plazo_respuesta: Option<DateTime<Utc>>,
    reunion_origen_id: Option<Uuid>,
    base_computo: String,
    max_bienes_apoderado: Option<i16>,
    apoderados_prohibidos: Vec<String>,
    exigir_soporte_poder: bool,
    efecto_mora: String,
    temporizador_segundos: Option<i32>,
    convocada_en: Option<DateTime<Utc>>,
    registro_abierto_en: Option<DateTime<Utc>>,
    instalada_en: Option<DateTime<Utc>>,
    cerrada_en: Option<DateTime<Utc>>,
    cancelada_en: Option<DateTime<Utc>>,
    motivo_cancelacion: Option<String>,
    acta_estado: String,
    acta_archivo_id: Option<Uuid>,
    acta_firmada_archivo_id: Option<Uuid>,
    acta_generada_en: Option<DateTime<Utc>>,
    acta_aprobada_en: Option<DateTime<Utc>>,
    acta_publicada_en: Option<DateTime<Utc>>,
    creada_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
}

impl TryFrom<FilaReunion> for Reunion {
    type Error = AppError;

    fn try_from(f: FilaReunion) -> AppResult<Self> {
        Ok(Self {
            id: ReunionId::from_uuid(f.id),
            titulo: f.titulo,
            tipo: TipoReunion::try_from(f.tipo.as_str())?,
            convocatoria: Convocatoria::try_from(f.convocatoria.as_str())?,
            modalidad: Modalidad::try_from(f.modalidad.as_str())?,
            estado: EstadoReunion::try_from(f.estado.as_str())?,
            fecha_programada: f.fecha_programada,
            lugar: f.lugar,
            enlace_video: f.enlace_video,
            plazo_respuesta: f.plazo_respuesta,
            reunion_origen_id: f.reunion_origen_id.map(ReunionId::from_uuid),
            configuracion: ConfiguracionReunion {
                base_computo: BaseComputo::try_from(f.base_computo.as_str())?,
                max_bienes_apoderado: opt_u16(f.max_bienes_apoderado, "max_bienes_apoderado")?,
                apoderados_prohibidos: f.apoderados_prohibidos,
                exigir_soporte_poder: f.exigir_soporte_poder,
                efecto_mora: EfectoMora::try_from(f.efecto_mora.as_str())?,
                temporizador_segundos: opt_u32(f.temporizador_segundos, "temporizador_segundos")?,
            },
            convocada_en: f.convocada_en,
            registro_abierto_en: f.registro_abierto_en,
            instalada_en: f.instalada_en,
            cerrada_en: f.cerrada_en,
            cancelada_en: f.cancelada_en,
            motivo_cancelacion: f.motivo_cancelacion,
            acta_estado: EstadoActa::try_from(f.acta_estado.as_str())?,
            acta_archivo_id: f.acta_archivo_id.map(ArchivoId::from_uuid),
            acta_firmada_archivo_id: f.acta_firmada_archivo_id.map(ArchivoId::from_uuid),
            acta_generada_en: f.acta_generada_en,
            acta_aprobada_en: f.acta_aprobada_en,
            acta_publicada_en: f.acta_publicada_en,
            creada_por: f.creada_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Preguntas
// ---------------------------------------------------------------------------------------------

struct FilaPregunta {
    id: Uuid,
    reunion_id: Uuid,
    punto_id: Option<Uuid>,
    orden: i16,
    enunciado: String,
    tipo: String,
    tipo_voto: String,
    base_computo: String,
    regla_mayoria: String,
    umbral_porcentaje: Option<Decimal>,
    umbral_sobre: Option<String>,
    max_selecciones: i16,
    cupos: Option<i16>,
    cargo: Option<String>,
    estado: String,
    temporizador_segundos: Option<i32>,
    cierra_en: Option<DateTime<Utc>>,
    abierta_en: Option<DateTime<Utc>>,
    cerrada_en: Option<DateTime<Utc>>,
    resultado: Option<serde_json::Value>,
    roles_aplicados_en: Option<DateTime<Utc>>,
}

struct FilaOpcion {
    id: Uuid,
    pregunta_id: Uuid,
    orden: i16,
    texto: String,
    en_blanco: bool,
}

struct FilaCandidato {
    opcion_id: Uuid,
    persona_id: Uuid,
}

fn armar_pregunta(
    f: FilaPregunta,
    opciones: &[FilaOpcion],
    candidatos: &[FilaCandidato],
) -> AppResult<PreguntaGuardada> {
    let ops: Vec<OpcionPregunta> = opciones
        .iter()
        .filter(|o| o.pregunta_id == f.id)
        .map(|o| {
            Ok(OpcionPregunta {
                id: OpcionId::from_uuid(o.id),
                orden: entero(i64::from(o.orden), "orden")?,
                texto: o.texto.clone(),
                en_blanco: o.en_blanco,
            })
        })
        .collect::<AppResult<_>>()?;
    let cands = ops
        .iter()
        .filter_map(|o| {
            let c: Vec<PersonaId> = candidatos
                .iter()
                .filter(|c| c.opcion_id == o.id.as_uuid())
                .map(|c| PersonaId::from_uuid(c.persona_id))
                .collect();
            (!c.is_empty()).then_some((o.id, c))
        })
        .collect();
    let resultado = f
        .resultado
        .map(serde_json::from_value::<Escrutinio>)
        .transpose()
        .map_err(AppError::infra)?;
    Ok(PreguntaGuardada {
        pregunta: Pregunta {
            id: PreguntaId::from_uuid(f.id),
            reunion_id: ReunionId::from_uuid(f.reunion_id),
            enunciado: f.enunciado,
            tipo: TipoPregunta::try_from(f.tipo.as_str())?,
            tipo_voto: TipoVoto::try_from(f.tipo_voto.as_str())?,
            base_computo: BaseComputo::try_from(f.base_computo.as_str())?,
            regla_mayoria: ReglaMayoria::desde_columnas(
                &f.regla_mayoria,
                f.umbral_porcentaje,
                f.umbral_sobre.as_deref(),
            )?,
            max_selecciones: entero(i64::from(f.max_selecciones), "max_selecciones")?,
            cupos: opt_u16(f.cupos, "cupos")?,
            cargo: f
                .cargo
                .as_deref()
                .map(CargoEleccion::try_from)
                .transpose()?,
            estado: EstadoPregunta::try_from(f.estado.as_str())?,
            opciones: ops,
        },
        orden: entero(i64::from(f.orden), "orden")?,
        punto_id: f.punto_id.map(PuntoOrdenId::from_uuid),
        temporizador_segundos: opt_u32(f.temporizador_segundos, "temporizador_segundos")?,
        cierra_en: f.cierra_en,
        abierta_en: f.abierta_en,
        cerrada_en: f.cerrada_en,
        resultado,
        candidatos: cands,
        roles_aplicados_en: f.roles_aplicados_en,
    })
}

// ---------------------------------------------------------------------------------------------
// Asistentes
// ---------------------------------------------------------------------------------------------

struct FilaAsistente {
    id: Uuid,
    reunion_id: Uuid,
    persona_id: Uuid,
    nombre: String,
    metodo: String,
    telefono: Option<String>,
    linea_whatsapp: Option<String>,
    whatsapp_confirmado_en: Option<DateTime<Utc>>,
    confirmacion_outbox_id: Option<i64>,
    registrado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
    presente: bool,
    rotulo: Option<String>,
}

/// Filtro de la consulta de asistentes (uno solo a la vez).
#[derive(Default)]
struct FiltroAsistentes<'a> {
    reunion: Option<Uuid>,
    id: Option<Uuid>,
    persona: Option<Uuid>,
    token: Option<&'a [u8]>,
    telefono: Option<&'a str>,
    rotulo: Option<&'a str>,
}

impl TransaccionPg {
    async fn cargar_asistentes(&mut self, f: FiltroAsistentes<'_>) -> AppResult<Vec<Asistente>> {
        let filas = sqlx::query_as!(
            FilaAsistente,
            r#"SELECT a.id, a.reunion_id, a.persona_id, p.nombre, a.metodo, a.telefono,
                      a.linea_whatsapp, a.whatsapp_confirmado_en, a.confirmacion_outbox_id,
                      a.registrado_por, a.creado_en,
                      EXISTS (SELECT 1 FROM asistente_presencias x
                              WHERE x.asistente_id = a.id AND x.salida_en IS NULL) AS "presente!",
                      (SELECT r.codigo FROM rotulos r
                       WHERE r.asistente_id = a.id AND r.liberado_en IS NULL) AS rotulo
               FROM asistentes a
               JOIN personas p ON p.tenant_id = a.tenant_id AND p.id = a.persona_id
               WHERE ($1::uuid IS NULL OR a.reunion_id = $1)
                 AND ($2::uuid IS NULL OR a.id = $2)
                 AND ($3::uuid IS NULL OR a.persona_id = $3)
                 AND ($4::bytea IS NULL OR a.token_hash = $4)
                 AND ($5::text IS NULL OR a.telefono = $5)
                 AND ($6::text IS NULL OR a.id = (SELECT r.asistente_id FROM rotulos r
                                                  WHERE r.reunion_id = $1 AND r.codigo = $6
                                                    AND r.liberado_en IS NULL))
               ORDER BY normalizar(p.nombre), a.id"#,
            f.reunion,
            f.id,
            f.persona,
            f.token,
            f.telefono,
            f.rotulo,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let ids: Vec<Uuid> = filas.iter().map(|f| f.id).collect();
        let bienes = sqlx::query!(
            r#"SELECT ab.asistente_id, ab.bien_privado_id, b.codigo, pr.coeficiente, ab.calidad,
                      ab.poder_id,
                      EXISTS (SELECT 1 FROM morosos_reunion m
                              WHERE m.reunion_id = ab.reunion_id
                                AND m.bien_privado_id = ab.bien_privado_id) AS "moroso!"
               FROM asistente_bienes ab
               JOIN bienes_privados b ON b.tenant_id = ab.tenant_id AND b.id = ab.bien_privado_id
               JOIN padron_reunion pr ON pr.reunion_id = ab.reunion_id
                                     AND pr.bien_privado_id = ab.bien_privado_id
               WHERE ab.asistente_id = ANY($1) AND ab.hasta IS NULL
               ORDER BY normalizar(b.codigo), ab.bien_privado_id"#,
            &ids,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        filas
            .into_iter()
            .map(|a| {
                Ok(Asistente {
                    id: AsistenteId::from_uuid(a.id),
                    reunion_id: ReunionId::from_uuid(a.reunion_id),
                    persona_id: PersonaId::from_uuid(a.persona_id),
                    nombre: a.nombre,
                    metodo: MetodoRegistro::try_from(a.metodo.as_str())?,
                    telefono: a.telefono,
                    linea_whatsapp: a.linea_whatsapp,
                    whatsapp_confirmado_en: a.whatsapp_confirmado_en,
                    confirmacion_outbox_id: a.confirmacion_outbox_id,
                    registrado_por: a.registrado_por.map(CuentaId::from_uuid),
                    creado_en: a.creado_en,
                    presente: a.presente,
                    rotulo: a.rotulo.as_deref().map(CodigoRotulo::new).transpose()?,
                    bienes: bienes
                        .iter()
                        .filter(|b| b.asistente_id == a.id)
                        .map(|b| {
                            Ok(BienDeAsistente {
                                bien_privado_id: BienPrivadoId::from_uuid(b.bien_privado_id),
                                codigo: b.codigo.clone(),
                                coeficiente: coef(b.coeficiente)?,
                                calidad: CalidadRepresentacion::try_from(b.calidad.as_str())?,
                                poder_id: b.poder_id.map(PoderId::from_uuid),
                                moroso: b.moroso,
                            })
                        })
                        .collect::<AppResult<_>>()?,
                })
            })
            .collect()
    }

    async fn una_reunion(&mut self, id: ReunionId, bloquear: bool) -> AppResult<Option<Reunion>> {
        let fila = if bloquear {
            sqlx::query_as!(
                FilaReunion,
                "SELECT id, titulo, tipo, convocatoria, modalidad, estado, fecha_programada, lugar, enlace_video,
         plazo_respuesta, reunion_origen_id, base_computo, max_bienes_apoderado,
         apoderados_prohibidos, exigir_soporte_poder, efecto_mora, temporizador_segundos,
         convocada_en, registro_abierto_en, instalada_en, cerrada_en, cancelada_en,
         motivo_cancelacion, acta_estado, acta_archivo_id, acta_firmada_archivo_id,
         acta_generada_en, acta_aprobada_en, acta_publicada_en, creada_por, creado_en FROM reuniones WHERE id = $1 FOR UPDATE",
                id.as_uuid()
            )
            .fetch_optional(&mut *self.tx)
            .await
        } else {
            sqlx::query_as!(
                FilaReunion,
                "SELECT id, titulo, tipo, convocatoria, modalidad, estado, fecha_programada, lugar, enlace_video,
         plazo_respuesta, reunion_origen_id, base_computo, max_bienes_apoderado,
         apoderados_prohibidos, exigir_soporte_poder, efecto_mora, temporizador_segundos,
         convocada_en, registro_abierto_en, instalada_en, cerrada_en, cancelada_en,
         motivo_cancelacion, acta_estado, acta_archivo_id, acta_firmada_archivo_id,
         acta_generada_en, acta_aprobada_en, acta_publicada_en, creada_por, creado_en FROM reuniones WHERE id = $1",
                id.as_uuid()
            )
            .fetch_optional(&mut *self.tx)
            .await
        }
        .map_err(AppError::infra)?;
        fila.map(Reunion::try_from).transpose()
    }

    async fn cargar_preguntas(
        &mut self,
        reunion: Option<Uuid>,
        id: Option<Uuid>,
        bloquear: bool,
    ) -> AppResult<Vec<PreguntaGuardada>> {
        let filas = if bloquear {
            sqlx::query_as!(
                FilaPregunta,
                "SELECT id, reunion_id, punto_id, orden, enunciado, tipo, tipo_voto, base_computo, regla_mayoria,
         umbral_porcentaje, umbral_sobre, max_selecciones, cupos, cargo, estado,
         temporizador_segundos, cierra_en, abierta_en, cerrada_en, resultado, roles_aplicados_en FROM preguntas WHERE id = $1 FOR UPDATE",
                id
            )
            .fetch_all(&mut *self.tx)
            .await
        } else {
            sqlx::query_as!(
                FilaPregunta,
                "SELECT id, reunion_id, punto_id, orden, enunciado, tipo, tipo_voto, base_computo, regla_mayoria,
         umbral_porcentaje, umbral_sobre, max_selecciones, cupos, cargo, estado,
         temporizador_segundos, cierra_en, abierta_en, cerrada_en, resultado, roles_aplicados_en
                 FROM preguntas
                 WHERE ($1::uuid IS NULL OR reunion_id = $1) AND ($2::uuid IS NULL OR id = $2)
                 ORDER BY orden",
                reunion,
                id
            )
            .fetch_all(&mut *self.tx)
            .await
        }
        .map_err(AppError::infra)?;
        let ids: Vec<Uuid> = filas.iter().map(|f| f.id).collect();
        let opciones = sqlx::query_as!(
            FilaOpcion,
            "SELECT id, pregunta_id, orden, texto, en_blanco FROM opciones
             WHERE pregunta_id = ANY($1) ORDER BY pregunta_id, orden",
            &ids
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let candidatos = sqlx::query_as!(
            FilaCandidato,
            "SELECT opcion_id, persona_id FROM candidatos
             WHERE pregunta_id = ANY($1) ORDER BY opcion_id, orden",
            &ids
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        filas
            .into_iter()
            .map(|f| armar_pregunta(f, &opciones, &candidatos))
            .collect()
    }

    async fn insertar_opciones(&mut self, p: &PreguntaGuardada) -> AppResult<()> {
        for o in &p.pregunta.opciones {
            sqlx::query!(
                "INSERT INTO opciones (id, tenant_id, pregunta_id, orden, texto, en_blanco)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                o.id.as_uuid(),
                self.ctx.tenant_id.as_uuid(),
                p.pregunta.id.as_uuid(),
                i16::try_from(o.orden).unwrap_or(i16::MAX),
                o.texto,
                o.en_blanco,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_escritura(e, "opción repetida", "pregunta"))?;
        }
        for (opcion, personas) in &p.candidatos {
            for (i, persona) in personas.iter().enumerate() {
                sqlx::query!(
                    "INSERT INTO candidatos (tenant_id, pregunta_id, opcion_id, orden, persona_id)
                     VALUES ($1, $2, $3, $4, $5)",
                    self.ctx.tenant_id.as_uuid(),
                    p.pregunta.id.as_uuid(),
                    opcion.as_uuid(),
                    i16::try_from(i + 1).unwrap_or(i16::MAX),
                    persona.as_uuid(),
                )
                .execute(&mut *self.tx)
                .await
                .map_err(|e| {
                    error_escritura(e, "un candidato está repetido en la plancha", "persona")
                })?;
            }
        }
        Ok(())
    }
}

fn error_voto(e: sqlx::Error) -> AppError {
    match e.as_database_error().and_then(|d| d.code()) {
        Some(c) if c == "V0001" => DominioError::PreguntaNoAbierta(EstadoPregunta::Cerrada).into(),
        Some(c) if c == "V0005" => AppError::Conflicto(
            "el bien privado no está habilitado para votar esta pregunta".into(),
        ),
        _ => AppError::infra(e),
    }
}

impl RepositorioAsambleas for TransaccionPg {
    async fn insertar_reunion(&mut self, r: &Reunion) -> AppResult<()> {
        let c = &r.configuracion;
        sqlx::query!(
            "INSERT INTO reuniones (id, tenant_id, titulo, tipo, convocatoria, modalidad, estado,
                 fecha_programada, lugar, enlace_video, plazo_respuesta, reunion_origen_id,
                 base_computo, max_bienes_apoderado, apoderados_prohibidos, exigir_soporte_poder,
                 efecto_mora, temporizador_segundos, creada_por, creado_en, actualizado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18, $19, $20, $20)",
            r.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            r.titulo,
            r.tipo.as_str(),
            r.convocatoria.as_str(),
            r.modalidad.as_str(),
            r.estado.as_str(),
            r.fecha_programada,
            r.lugar,
            r.enlace_video,
            r.plazo_respuesta,
            r.reunion_origen_id.map(ReunionId::as_uuid),
            c.base_computo.as_str(),
            c.max_bienes_apoderado
                .map(|m| i16::try_from(m).unwrap_or(i16::MAX)),
            &c.apoderados_prohibidos,
            c.exigir_soporte_poder,
            c.efecto_mora.as_str(),
            c.temporizador_segundos
                .map(|s| i32::try_from(s).unwrap_or(i32::MAX)),
            r.creada_por.map(CuentaId::as_uuid),
            r.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "reunión repetida", "reunión de origen"))?;
        Ok(())
    }

    async fn actualizar_reunion(&mut self, r: &Reunion) -> AppResult<()> {
        let c = &r.configuracion;
        sqlx::query!(
            "UPDATE reuniones SET titulo = $2, tipo = $3, convocatoria = $4, modalidad = $5,
                 estado = $6, fecha_programada = $7, lugar = $8, enlace_video = $9,
                 plazo_respuesta = $10, base_computo = $11, max_bienes_apoderado = $12,
                 apoderados_prohibidos = $13, exigir_soporte_poder = $14, efecto_mora = $15,
                 temporizador_segundos = $16, convocada_en = $17, registro_abierto_en = $18,
                 instalada_en = $19, cerrada_en = $20, cancelada_en = $21,
                 motivo_cancelacion = $22, acta_estado = $23, acta_archivo_id = $24,
                 acta_firmada_archivo_id = $25, acta_generada_en = $26, acta_aprobada_en = $27,
                 acta_publicada_en = $28, actualizado_en = now()
             WHERE id = $1",
            r.id.as_uuid(),
            r.titulo,
            r.tipo.as_str(),
            r.convocatoria.as_str(),
            r.modalidad.as_str(),
            r.estado.as_str(),
            r.fecha_programada,
            r.lugar,
            r.enlace_video,
            r.plazo_respuesta,
            c.base_computo.as_str(),
            c.max_bienes_apoderado
                .map(|m| i16::try_from(m).unwrap_or(i16::MAX)),
            &c.apoderados_prohibidos,
            c.exigir_soporte_poder,
            c.efecto_mora.as_str(),
            c.temporizador_segundos
                .map(|s| i32::try_from(s).unwrap_or(i32::MAX)),
            r.convocada_en,
            r.registro_abierto_en,
            r.instalada_en,
            r.cerrada_en,
            r.cancelada_en,
            r.motivo_cancelacion,
            r.acta_estado.as_str(),
            r.acta_archivo_id.map(ArchivoId::as_uuid),
            r.acta_firmada_archivo_id.map(ArchivoId::as_uuid),
            r.acta_generada_en,
            r.acta_aprobada_en,
            r.acta_publicada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "reunión repetida", "archivo"))?;
        Ok(())
    }

    async fn reunion(&mut self, id: ReunionId) -> AppResult<Option<Reunion>> {
        self.una_reunion(id, false).await
    }

    async fn bloquear_reunion(&mut self, id: ReunionId) -> AppResult<Option<Reunion>> {
        self.una_reunion(id, true).await
    }

    async fn reuniones(
        &mut self,
        estado: Option<EstadoReunion>,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<Reunion>> {
        let filas = sqlx::query_as!(
            FilaReunion,
            "SELECT id, titulo, tipo, convocatoria, modalidad, estado, fecha_programada, lugar, enlace_video,
         plazo_respuesta, reunion_origen_id, base_computo, max_bienes_apoderado,
         apoderados_prohibidos, exigir_soporte_poder, efecto_mora, temporizador_segundos,
         convocada_en, registro_abierto_en, instalada_en, cerrada_en, cancelada_en,
         motivo_cancelacion, acta_estado, acta_archivo_id, acta_firmada_archivo_id,
         acta_generada_en, acta_aprobada_en, acta_publicada_en, creada_por, creado_en
             FROM reuniones
             WHERE ($1::text IS NULL OR estado = $1) AND ($2::uuid IS NULL OR id < $2)
             ORDER BY id DESC LIMIT $3",
            estado.map(EstadoReunion::as_str),
            pagina.despues(),
            pagina.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Reunion::try_from)
        .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(filas, pagina, |r| r.id.as_uuid()))
    }

    async fn actas_publicadas(&mut self) -> AppResult<Vec<Reunion>> {
        sqlx::query_as!(
            FilaReunion,
            "SELECT id, titulo, tipo, convocatoria, modalidad, estado, fecha_programada, lugar, enlace_video,
         plazo_respuesta, reunion_origen_id, base_computo, max_bienes_apoderado,
         apoderados_prohibidos, exigir_soporte_poder, efecto_mora, temporizador_segundos,
         convocada_en, registro_abierto_en, instalada_en, cerrada_en, cancelada_en,
         motivo_cancelacion, acta_estado, acta_archivo_id, acta_firmada_archivo_id,
         acta_generada_en, acta_aprobada_en, acta_publicada_en, creada_por, creado_en
             FROM reuniones WHERE acta_estado = 'publicada' ORDER BY fecha_programada DESC, id"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Reunion::try_from)
        .collect()
    }

    async fn puntos(&mut self, reunion: ReunionId) -> AppResult<Vec<PuntoOrden>> {
        sqlx::query!(
            "SELECT id, orden, titulo, descripcion, estado, iniciado_en, terminado_en
             FROM puntos_orden_dia WHERE reunion_id = $1 ORDER BY orden",
            reunion.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(PuntoOrden {
                id: PuntoOrdenId::from_uuid(f.id),
                orden: entero(i64::from(f.orden), "orden")?,
                titulo: f.titulo,
                descripcion: f.descripcion,
                estado: EstadoPunto::try_from(f.estado.as_str())?,
                iniciado_en: f.iniciado_en,
                terminado_en: f.terminado_en,
            })
        })
        .collect()
    }

    async fn guardar_punto(
        &mut self,
        reunion: ReunionId,
        p: &PuntoOrden,
        nuevo: bool,
    ) -> AppResult<()> {
        let orden = i16::try_from(p.orden).unwrap_or(i16::MAX);
        let r = if nuevo {
            sqlx::query!(
                "INSERT INTO puntos_orden_dia (id, tenant_id, reunion_id, orden, titulo, descripcion,
                     estado, iniciado_en, terminado_en)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                p.id.as_uuid(),
                self.ctx.tenant_id.as_uuid(),
                reunion.as_uuid(),
                orden,
                p.titulo,
                p.descripcion,
                p.estado.as_str(),
                p.iniciado_en,
                p.terminado_en,
            )
            .execute(&mut *self.tx)
            .await
        } else {
            sqlx::query!(
                "UPDATE puntos_orden_dia SET orden = $2, titulo = $3, descripcion = $4, estado = $5,
                     iniciado_en = $6, terminado_en = $7
                 WHERE id = $1",
                p.id.as_uuid(),
                orden,
                p.titulo,
                p.descripcion,
                p.estado.as_str(),
                p.iniciado_en,
                p.terminado_en,
            )
            .execute(&mut *self.tx)
            .await
        };
        r.map_err(|e| error_escritura(e, "ya hay un punto con ese orden", "reunión"))?;
        Ok(())
    }

    async fn eliminar_punto(&mut self, id: PuntoOrdenId) -> AppResult<bool> {
        Ok(
            sqlx::query!("DELETE FROM puntos_orden_dia WHERE id = $1", id.as_uuid())
                .execute(&mut *self.tx)
                .await
                .map_err(|e| error_escritura(e, "el punto tiene preguntas", "punto"))?
                .rows_affected()
                > 0,
        )
    }

    async fn documentos(&mut self, reunion: ReunionId) -> AppResult<Vec<DocumentoReunion>> {
        sqlx::query!(
            "SELECT archivo_id, tipo, titulo FROM documentos_reunion WHERE reunion_id = $1
             ORDER BY titulo, archivo_id",
            reunion.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(DocumentoReunion {
                archivo_id: ArchivoId::from_uuid(f.archivo_id),
                tipo: TipoDocumentoReunion::try_from(f.tipo.as_str())?,
                titulo: f.titulo,
            })
        })
        .collect()
    }

    async fn agregar_documento(
        &mut self,
        reunion: ReunionId,
        d: &DocumentoReunion,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO documentos_reunion (tenant_id, reunion_id, archivo_id, tipo, titulo)
             VALUES ($1, $2, $3, $4, $5)",
            self.ctx.tenant_id.as_uuid(),
            reunion.as_uuid(),
            d.archivo_id.as_uuid(),
            d.tipo.as_str(),
            d.titulo,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "el documento ya está anexo", "archivo"))?;
        Ok(())
    }

    async fn quitar_documento(
        &mut self,
        reunion: ReunionId,
        archivo: ArchivoId,
    ) -> AppResult<bool> {
        Ok(sqlx::query!(
            "DELETE FROM documentos_reunion WHERE reunion_id = $1 AND archivo_id = $2",
            reunion.as_uuid(),
            archivo.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected()
            > 0)
    }

    async fn bienes_para_padron(&mut self) -> AppResult<Vec<BienParaPadron>> {
        sqlx::query!(
            r#"SELECT b.id, b.coeficiente AS "coeficiente!", t.persona_id AS "representante?"
               FROM bienes_privados b
               LEFT JOIN titularidades t
                      ON t.tenant_id = b.tenant_id AND t.bien_privado_id = b.id AND t.es_representante
                     AND t.vigente_desde <= now() AND (t.vigente_hasta IS NULL OR t.vigente_hasta > now())
               WHERE b.activo AND b.coeficiente IS NOT NULL
               ORDER BY b.id"#
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(BienParaPadron {
                bien_privado_id: BienPrivadoId::from_uuid(f.id),
                coeficiente: coef(f.coeficiente)?,
                representante: f.representante.map(PersonaId::from_uuid),
            })
        })
        .collect()
    }

    async fn insertar_padron(
        &mut self,
        reunion: ReunionId,
        bienes: &[BienParaPadron],
    ) -> AppResult<()> {
        let ids: Vec<Uuid> = bienes.iter().map(|b| b.bien_privado_id.as_uuid()).collect();
        let coefs: Vec<Decimal> = bienes.iter().map(|b| b.coeficiente.valor()).collect();
        let reps: Vec<Option<Uuid>> = bienes
            .iter()
            .map(|b| b.representante.map(PersonaId::as_uuid))
            .collect();
        sqlx::query!(
            "INSERT INTO padron_reunion (tenant_id, reunion_id, bien_privado_id, coeficiente,
                 representante_persona_id)
             SELECT $1, $2, b, c, r FROM UNNEST($3::uuid[], $4::numeric[], $5::uuid[]) AS x (b, c, r)",
            self.ctx.tenant_id.as_uuid(),
            reunion.as_uuid(),
            &ids,
            &coefs,
            &reps as &[Option<Uuid>],
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "el padrón ya se tomó", "bien privado"))?;
        Ok(())
    }

    async fn padron(&mut self, reunion: ReunionId) -> AppResult<Vec<BienPadron>> {
        sqlx::query!(
            r#"SELECT pr.bien_privado_id, b.codigo, pr.coeficiente, pr.representante_persona_id,
                      (m.bien_privado_id IS NOT NULL) AS "moroso!"
               FROM padron_reunion pr
               JOIN bienes_privados b ON b.tenant_id = pr.tenant_id AND b.id = pr.bien_privado_id
               LEFT JOIN morosos_reunion m ON m.reunion_id = pr.reunion_id
                                          AND m.bien_privado_id = pr.bien_privado_id
               WHERE pr.reunion_id = $1
               ORDER BY normalizar(b.codigo), pr.bien_privado_id"#,
            reunion.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(BienPadron {
                bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
                codigo: f.codigo,
                coeficiente: coef(f.coeficiente)?,
                representante: f.representante_persona_id.map(PersonaId::from_uuid),
                moroso: f.moroso,
            })
        })
        .collect()
    }

    async fn buscar_en_padron(
        &mut self,
        reunion: ReunionId,
        texto: &str,
        limite: u32,
    ) -> AppResult<Vec<CoincidenciaMesa>> {
        let filas = sqlx::query!(
            r#"SELECT pr.bien_privado_id, b.codigo, pr.coeficiente, pr.representante_persona_id,
                      p.nombre AS "nombre?", p.telefono AS "telefono?",
                      (m.bien_privado_id IS NOT NULL) AS "moroso!", ab.asistente_id AS "asistente_id?"
               FROM padron_reunion pr
               JOIN bienes_privados b ON b.tenant_id = pr.tenant_id AND b.id = pr.bien_privado_id
               LEFT JOIN personas p ON p.tenant_id = pr.tenant_id AND p.id = pr.representante_persona_id
               LEFT JOIN morosos_reunion m ON m.reunion_id = pr.reunion_id
                                          AND m.bien_privado_id = pr.bien_privado_id
               LEFT JOIN asistente_bienes ab ON ab.reunion_id = pr.reunion_id
                                            AND ab.bien_privado_id = pr.bien_privado_id
                                            AND ab.hasta IS NULL
               WHERE pr.reunion_id = $1
                 AND (normalizar(b.codigo) LIKE normalizar($2) ESCAPE '\' OR p.documento = $3)
               ORDER BY normalizar(b.codigo), pr.bien_privado_id
               LIMIT $4"#,
            reunion.as_uuid(),
            escapar_like(texto),
            texto.trim(),
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let reps: Vec<Uuid> = filas
            .iter()
            .filter_map(|f| f.representante_persona_id)
            .collect();
        let hermanos = sqlx::query!(
            r#"SELECT bien_privado_id, representante_persona_id AS "rep!"
               FROM padron_reunion WHERE reunion_id = $1 AND representante_persona_id = ANY($2)"#,
            reunion.as_uuid(),
            &reps
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        filas
            .into_iter()
            .map(|f| {
                Ok(CoincidenciaMesa {
                    otros_bienes: hermanos
                        .iter()
                        .filter(|h| {
                            Some(h.rep) == f.representante_persona_id
                                && h.bien_privado_id != f.bien_privado_id
                        })
                        .map(|h| BienPrivadoId::from_uuid(h.bien_privado_id))
                        .collect(),
                    bien: BienPadron {
                        bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
                        codigo: f.codigo,
                        coeficiente: coef(f.coeficiente)?,
                        representante: f.representante_persona_id.map(PersonaId::from_uuid),
                        moroso: f.moroso,
                    },
                    representante_nombre: f.nombre,
                    representante_telefono: f.telefono,
                    asistente_actual: f.asistente_id.map(AsistenteId::from_uuid),
                })
            })
            .collect()
    }

    async fn reemplazar_morosos(
        &mut self,
        reunion: ReunionId,
        bienes: &[BienPrivadoId],
    ) -> AppResult<()> {
        sqlx::query!(
            "DELETE FROM morosos_reunion WHERE reunion_id = $1",
            reunion.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let ids: Vec<Uuid> = bienes.iter().map(|b| b.as_uuid()).collect();
        sqlx::query!(
            "INSERT INTO morosos_reunion (tenant_id, reunion_id, bien_privado_id)
             SELECT $1, $2, b FROM UNNEST($3::uuid[]) AS b",
            self.ctx.tenant_id.as_uuid(),
            reunion.as_uuid(),
            &ids
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "bien moroso repetido", "bien privado del padrón"))?;
        Ok(())
    }

    async fn insertar_asistente(&mut self, a: &AsistenteNuevo) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO asistentes (id, tenant_id, reunion_id, persona_id, metodo, telefono,
                 registrado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            a.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            a.reunion_id.as_uuid(),
            a.persona_id.as_uuid(),
            a.metodo.as_str(),
            a.telefono,
            a.registrado_por.map(CuentaId::as_uuid),
            a.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(e, "la persona ya está registrada en la reunión", "persona")
        })?;
        Ok(())
    }

    async fn asistente(&mut self, id: AsistenteId) -> AppResult<Option<Asistente>> {
        Ok(self
            .cargar_asistentes(FiltroAsistentes {
                id: Some(id.as_uuid()),
                ..Default::default()
            })
            .await?
            .pop())
    }

    async fn asistente_de_persona(
        &mut self,
        reunion: ReunionId,
        persona: PersonaId,
    ) -> AppResult<Option<Asistente>> {
        Ok(self
            .cargar_asistentes(FiltroAsistentes {
                reunion: Some(reunion.as_uuid()),
                persona: Some(persona.as_uuid()),
                ..Default::default()
            })
            .await?
            .pop())
    }

    async fn asistentes(&mut self, reunion: ReunionId) -> AppResult<Vec<Asistente>> {
        self.cargar_asistentes(FiltroAsistentes {
            reunion: Some(reunion.as_uuid()),
            ..Default::default()
        })
        .await
    }

    async fn asistente_por_token(&mut self, token_hash: &[u8]) -> AppResult<Option<Asistente>> {
        Ok(self
            .cargar_asistentes(FiltroAsistentes {
                token: Some(token_hash),
                ..Default::default()
            })
            .await?
            .pop())
    }

    async fn asistente_por_rotulo(
        &mut self,
        reunion: ReunionId,
        codigo: &CodigoRotulo,
    ) -> AppResult<Option<Asistente>> {
        Ok(self
            .cargar_asistentes(FiltroAsistentes {
                reunion: Some(reunion.as_uuid()),
                rotulo: Some(codigo.as_str()),
                ..Default::default()
            })
            .await?
            .pop())
    }

    async fn asistentes_por_telefono(&mut self, telefono: &str) -> AppResult<Vec<Asistente>> {
        self.cargar_asistentes(FiltroAsistentes {
            telefono: Some(telefono),
            ..Default::default()
        })
        .await
    }

    async fn actualizar_medios(
        &mut self,
        id: AsistenteId,
        telefono: Option<&str>,
        linea: Option<&str>,
        confirmado_en: Option<DateTime<Utc>>,
        confirmacion_outbox_id: Option<i64>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE asistentes SET telefono = $2, linea_whatsapp = $3, whatsapp_confirmado_en = $4,
                 confirmacion_outbox_id = $5
             WHERE id = $1",
            id.as_uuid(),
            telefono,
            linea,
            confirmado_en,
            confirmacion_outbox_id,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn fijar_token(&mut self, id: AsistenteId, token_hash: &[u8]) -> AppResult<()> {
        sqlx::query!(
            "UPDATE asistentes SET token_hash = $2 WHERE id = $1",
            id.as_uuid(),
            token_hash
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "token repetido", "asistente"))?;
        Ok(())
    }

    async fn vincular_bien(
        &mut self,
        reunion: ReunionId,
        asistente: AsistenteId,
        bien: BienPrivadoId,
        calidad: CalidadRepresentacion,
        poder: Option<PoderId>,
        desde: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO asistente_bienes (id, tenant_id, reunion_id, asistente_id, bien_privado_id,
                 calidad, poder_id, desde)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            Uuid::now_v7(),
            self.ctx.tenant_id.as_uuid(),
            reunion.as_uuid(),
            asistente.as_uuid(),
            bien.as_uuid(),
            calidad.as_str(),
            poder.map(PoderId::as_uuid),
            desde,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "el bien ya está registrado con otro asistente",
                "bien privado del padrón",
            )
        })?;
        Ok(())
    }

    async fn desvincular_bien(
        &mut self,
        reunion: ReunionId,
        bien: BienPrivadoId,
        hasta: DateTime<Utc>,
    ) -> AppResult<Option<AsistenteId>> {
        Ok(sqlx::query_scalar!(
            "UPDATE asistente_bienes SET hasta = greatest($3, desde)
             WHERE reunion_id = $1 AND bien_privado_id = $2 AND hasta IS NULL
             RETURNING asistente_id",
            reunion.as_uuid(),
            bien.as_uuid(),
            hasta
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(AsistenteId::from_uuid))
    }

    async fn insertar_poder(&mut self, p: &Poder) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO poderes (id, tenant_id, reunion_id, bien_privado_id, poderdante_persona_id,
                 apoderado_persona_id, soporte_archivo_id, estado, motivo, registrado_por,
                 registrado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            p.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            p.reunion_id.as_uuid(),
            p.bien_privado_id.as_uuid(),
            p.poderdante.map(PersonaId::as_uuid),
            p.apoderado.as_uuid(),
            p.soporte_archivo_id.map(ArchivoId::as_uuid),
            p.estado.as_str(),
            p.motivo,
            p.registrado_por.map(CuentaId::as_uuid),
            p.registrado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "el bien ya tiene un poder vigente",
                "archivo o bien del padrón",
            )
        })?;
        Ok(())
    }

    async fn poder(&mut self, id: PoderId) -> AppResult<Option<Poder>> {
        Ok(self.poderes_donde(None, Some(id.as_uuid())).await?.pop())
    }

    async fn actualizar_poder(&mut self, p: &Poder) -> AppResult<()> {
        sqlx::query!(
            "UPDATE poderes SET estado = $2, motivo = $3, revisado_por = $4, revisado_en = $5
             WHERE id = $1",
            p.id.as_uuid(),
            p.estado.as_str(),
            p.motivo,
            p.revisado_por.map(CuentaId::as_uuid),
            p.revisado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn poderes(&mut self, reunion: ReunionId) -> AppResult<Vec<Poder>> {
        self.poderes_donde(Some(reunion.as_uuid()), None).await
    }

    async fn abrir_presencia(&mut self, reunion: ReunionId, p: &Presencia) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO asistente_presencias (id, tenant_id, reunion_id, asistente_id, entrada_en,
                 registrada_por)
             VALUES ($1, $2, $3, $4, $5, $6)",
            p.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            reunion.as_uuid(),
            p.asistente_id.as_uuid(),
            p.entrada_en,
            p.registrada_por.map(CuentaId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "el asistente ya está presente", "asistente"))?;
        Ok(())
    }

    async fn cerrar_presencia(
        &mut self,
        asistente: AsistenteId,
        salida: DateTime<Utc>,
        motivo: MotivoSalida,
        detalle: Option<&str>,
    ) -> AppResult<bool> {
        Ok(sqlx::query!(
            "UPDATE asistente_presencias
             SET salida_en = greatest($2, entrada_en), motivo_salida = $3, detalle_salida = $4
             WHERE asistente_id = $1 AND salida_en IS NULL",
            asistente.as_uuid(),
            salida,
            motivo.as_str(),
            detalle,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected()
            > 0)
    }

    async fn presencias(&mut self, reunion: ReunionId) -> AppResult<Vec<Presencia>> {
        sqlx::query!(
            "SELECT id, asistente_id, entrada_en, salida_en, motivo_salida, detalle_salida, registrada_por
             FROM asistente_presencias WHERE reunion_id = $1 ORDER BY entrada_en, id",
            reunion.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(Presencia {
                id: PresenciaId::from_uuid(f.id),
                asistente_id: AsistenteId::from_uuid(f.asistente_id),
                entrada_en: f.entrada_en,
                salida_en: f.salida_en,
                motivo_salida: f.motivo_salida.as_deref().map(MotivoSalida::try_from).transpose()?,
                detalle_salida: f.detalle_salida,
                registrada_por: f.registrada_por.map(CuentaId::from_uuid),
            })
        })
        .collect()
    }

    async fn bienes_presentes(&mut self, reunion: ReunionId) -> AppResult<Vec<BienPresente>> {
        sqlx::query!(
            r#"SELECT ab.bien_privado_id, pr.coeficiente, ab.asistente_id,
                      (m.bien_privado_id IS NOT NULL) AS "moroso!"
               FROM asistente_bienes ab
               JOIN asistente_presencias x ON x.asistente_id = ab.asistente_id AND x.salida_en IS NULL
               JOIN padron_reunion pr ON pr.reunion_id = ab.reunion_id
                                     AND pr.bien_privado_id = ab.bien_privado_id
               LEFT JOIN morosos_reunion m ON m.reunion_id = ab.reunion_id
                                          AND m.bien_privado_id = ab.bien_privado_id
               WHERE ab.reunion_id = $1 AND ab.hasta IS NULL"#,
            reunion.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(BienPresente {
                bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
                coeficiente: coef(f.coeficiente)?,
                asistente_id: AsistenteId::from_uuid(f.asistente_id),
                moroso: f.moroso,
            })
        })
        .collect()
    }

    async fn asignar_rotulo(
        &mut self,
        reunion: ReunionId,
        codigo: &CodigoRotulo,
        asistente: AsistenteId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO rotulos (id, tenant_id, reunion_id, codigo, asistente_id, asignado_en)
             VALUES ($1, $2, $3, $4, $5, $6)",
            Uuid::now_v7(),
            self.ctx.tenant_id.as_uuid(),
            reunion.as_uuid(),
            codigo.as_str(),
            asistente.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "el rótulo ya está en uso", "asistente"))?;
        Ok(())
    }

    async fn liberar_rotulo(
        &mut self,
        asistente: AsistenteId,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        Ok(sqlx::query!(
            "UPDATE rotulos SET liberado_en = greatest($2, asignado_en)
             WHERE asistente_id = $1 AND liberado_en IS NULL",
            asistente.as_uuid(),
            ahora
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected()
            > 0)
    }

    async fn insertar_foto(&mut self, reunion: ReunionId, f: &FotoQuorum) -> AppResult<()> {
        let q = &f.quorum;
        sqlx::query!(
            "INSERT INTO fotos_quorum (id, tenant_id, reunion_id, pregunta_id, momento,
                 coeficiente_presente, unidades_presentes, coeficiente_total, unidades_total,
                 asistentes, tomada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            Uuid::now_v7(),
            self.ctx.tenant_id.as_uuid(),
            reunion.as_uuid(),
            f.pregunta_id.map(PreguntaId::as_uuid),
            f.momento.as_str(),
            q.coeficientes.representados,
            q.unidades.representados.to_i32().unwrap_or(i32::MAX),
            q.coeficientes.total,
            q.unidades.total.to_i32().unwrap_or(i32::MAX),
            i32::try_from(q.asistentes).unwrap_or(i32::MAX),
            f.tomada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn fotos(&mut self, reunion: ReunionId) -> AppResult<Vec<FotoQuorum>> {
        sqlx::query!(
            "SELECT pregunta_id, momento, coeficiente_presente, unidades_presentes,
                    coeficiente_total, unidades_total, asistentes, tomada_en
             FROM fotos_quorum WHERE reunion_id = $1 ORDER BY tomada_en, id",
            reunion.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(FotoQuorum {
                pregunta_id: f.pregunta_id.map(PreguntaId::from_uuid),
                momento: MomentoQuorum::try_from(f.momento.as_str())?,
                quorum: QuorumReunion::new(
                    f.coeficiente_presente,
                    f.coeficiente_total,
                    entero(i64::from(f.unidades_presentes), "unidades")?,
                    entero(i64::from(f.unidades_total), "unidades")?,
                    entero(i64::from(f.asistentes), "asistentes")?,
                )?,
                tomada_en: f.tomada_en,
            })
        })
        .collect()
    }

    async fn guardar_pregunta(&mut self, p: &PreguntaGuardada, nueva: bool) -> AppResult<()> {
        let q = &p.pregunta;
        let (umbral, sobre) = q.regla_mayoria.umbral();
        let resultado = p
            .resultado
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(AppError::infra)?;
        if nueva {
            sqlx::query!(
                "INSERT INTO preguntas (id, tenant_id, reunion_id, punto_id, orden, enunciado, tipo,
                     tipo_voto, base_computo, regla_mayoria, umbral_porcentaje, umbral_sobre,
                     max_selecciones, cupos, cargo, estado, temporizador_segundos, resultado)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)",
                q.id.as_uuid(),
                self.ctx.tenant_id.as_uuid(),
                q.reunion_id.as_uuid(),
                p.punto_id.map(PuntoOrdenId::as_uuid),
                i16::try_from(p.orden).unwrap_or(i16::MAX),
                q.enunciado,
                q.tipo.as_str(),
                q.tipo_voto.as_str(),
                q.base_computo.as_str(),
                q.regla_mayoria.codigo(),
                umbral,
                sobre.map(domain::BaseMayoria::as_str),
                i16::try_from(q.max_selecciones).unwrap_or(i16::MAX),
                q.cupos.map(|c| i16::try_from(c).unwrap_or(i16::MAX)),
                q.cargo.map(CargoEleccion::as_str),
                q.estado.as_str(),
                p.temporizador_segundos.map(|s| i32::try_from(s).unwrap_or(i32::MAX)),
                resultado,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_escritura(e, "ya hay una pregunta con ese orden", "reunión o punto"))?;
        } else {
            sqlx::query!(
                "DELETE FROM candidatos WHERE pregunta_id = $1",
                q.id.as_uuid()
            )
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
            sqlx::query!(
                "DELETE FROM opciones WHERE pregunta_id = $1",
                q.id.as_uuid()
            )
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
            sqlx::query!(
                "UPDATE preguntas SET punto_id = $2, enunciado = $3, tipo = $4, tipo_voto = $5,
                     base_computo = $6, regla_mayoria = $7, umbral_porcentaje = $8,
                     umbral_sobre = $9, max_selecciones = $10, cupos = $11, cargo = $12,
                     temporizador_segundos = $13
                 WHERE id = $1",
                q.id.as_uuid(),
                p.punto_id.map(PuntoOrdenId::as_uuid),
                q.enunciado,
                q.tipo.as_str(),
                q.tipo_voto.as_str(),
                q.base_computo.as_str(),
                q.regla_mayoria.codigo(),
                umbral,
                sobre.map(domain::BaseMayoria::as_str),
                i16::try_from(q.max_selecciones).unwrap_or(i16::MAX),
                q.cupos.map(|c| i16::try_from(c).unwrap_or(i16::MAX)),
                q.cargo.map(CargoEleccion::as_str),
                p.temporizador_segundos
                    .map(|s| i32::try_from(s).unwrap_or(i32::MAX)),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_escritura(e, "pregunta inválida", "punto"))?;
        }
        self.insertar_opciones(p).await
    }

    async fn eliminar_pregunta(&mut self, id: PreguntaId) -> AppResult<bool> {
        for sql in [
            "DELETE FROM candidatos WHERE pregunta_id = $1",
            "DELETE FROM opciones WHERE pregunta_id = $1",
        ] {
            sqlx::query(sql)
                .bind(id.as_uuid())
                .execute(&mut *self.tx)
                .await
                .map_err(AppError::infra)?;
        }
        Ok(sqlx::query!(
            "DELETE FROM preguntas WHERE id = $1 AND estado = 'borrador'",
            id.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected()
            > 0)
    }

    async fn pregunta(&mut self, id: PreguntaId) -> AppResult<Option<PreguntaGuardada>> {
        Ok(self
            .cargar_preguntas(None, Some(id.as_uuid()), false)
            .await?
            .pop())
    }

    async fn bloquear_pregunta(&mut self, id: PreguntaId) -> AppResult<Option<PreguntaGuardada>> {
        Ok(self
            .cargar_preguntas(None, Some(id.as_uuid()), true)
            .await?
            .pop())
    }

    async fn preguntas(&mut self, reunion: ReunionId) -> AppResult<Vec<PreguntaGuardada>> {
        self.cargar_preguntas(Some(reunion.as_uuid()), None, false)
            .await
    }

    async fn preguntas_vencidas(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<PreguntaId>> {
        Ok(sqlx::query_scalar!(
            "SELECT id FROM preguntas WHERE estado = 'abierta' AND cierra_en <= $1 ORDER BY cierra_en",
            ahora
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(PreguntaId::from_uuid)
        .collect())
    }

    async fn consultas_vencidas(&mut self, ahora: DateTime<Utc>) -> AppResult<Vec<ReunionId>> {
        Ok(sqlx::query_scalar!(
            "SELECT id FROM reuniones
             WHERE modalidad = 'escrita' AND estado = 'instalada' AND plazo_respuesta <= $1",
            ahora
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(ReunionId::from_uuid)
        .collect())
    }

    async fn actualizar_pregunta(&mut self, p: &PreguntaGuardada) -> AppResult<()> {
        let resultado = p
            .resultado
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(AppError::infra)?;
        sqlx::query!(
            "UPDATE preguntas SET estado = $2, abierta_en = $3, cierra_en = $4, cerrada_en = $5,
                 resultado = $6, roles_aplicados_en = $7
             WHERE id = $1",
            p.pregunta.id.as_uuid(),
            p.pregunta.estado.as_str(),
            p.abierta_en,
            p.cierra_en,
            p.cerrada_en,
            resultado,
            p.roles_aplicados_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn habilitar(
        &mut self,
        pregunta: PreguntaId,
        bienes: &[(BienPrivadoId, Coeficiente)],
        desde: DateTime<Utc>,
    ) -> AppResult<()> {
        let ids: Vec<Uuid> = bienes.iter().map(|(b, _)| b.as_uuid()).collect();
        let coefs: Vec<Decimal> = bienes.iter().map(|(_, c)| c.valor()).collect();
        sqlx::query!(
            "INSERT INTO pregunta_habilitados (tenant_id, pregunta_id, bien_privado_id, coeficiente, desde)
             SELECT $1, $2, b, c, $5 FROM UNNEST($3::uuid[], $4::numeric[]) AS x (b, c)
             ON CONFLICT (pregunta_id, bien_privado_id) DO NOTHING",
            self.ctx.tenant_id.as_uuid(),
            pregunta.as_uuid(),
            &ids,
            &coefs,
            desde,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn conteo_habilitados(&mut self, pregunta: PreguntaId) -> AppResult<Conteo> {
        let f = sqlx::query!(
            r#"SELECT coalesce(sum(coeficiente), 0) AS "c!", count(*) AS "u!"
               FROM pregunta_habilitados WHERE pregunta_id = $1"#,
            pregunta.as_uuid()
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(Conteo::new(f.c, entero(f.u, "habilitados")?))
    }

    async fn pendientes_de_voto(
        &mut self,
        pregunta: PreguntaId,
        bienes: &[BienPrivadoId],
    ) -> AppResult<Vec<(BienPrivadoId, Coeficiente)>> {
        let ids: Vec<Uuid> = bienes.iter().map(|b| b.as_uuid()).collect();
        sqlx::query!(
            "SELECT h.bien_privado_id, h.coeficiente
             FROM pregunta_habilitados h
             WHERE h.pregunta_id = $1 AND h.bien_privado_id = ANY($2)
               AND NOT EXISTS (SELECT 1 FROM participaciones p
                               WHERE p.pregunta_id = h.pregunta_id
                                 AND p.bien_privado_id = h.bien_privado_id)
             ORDER BY h.bien_privado_id",
            pregunta.as_uuid(),
            &ids
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok((
                BienPrivadoId::from_uuid(f.bien_privado_id),
                coef(f.coeficiente)?,
            ))
        })
        .collect()
    }

    async fn participaciones_con_clave(
        &mut self,
        pregunta: PreguntaId,
        asistente: AsistenteId,
        clave: &str,
    ) -> AppResult<u32> {
        let n = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM participaciones
               WHERE pregunta_id = $1 AND asistente_id = $2 AND idempotency_key = $3"#,
            pregunta.as_uuid(),
            asistente.as_uuid(),
            clave
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        entero(n, "participaciones")
    }

    async fn registrar_voto(&mut self, v: &VotoEmitido) -> AppResult<u32> {
        let mut n = 0;
        for p in &v.participaciones {
            let insertada = sqlx::query_scalar!(
                "INSERT INTO participaciones (id, tenant_id, pregunta_id, bien_privado_id,
                     asistente_id, coeficiente, canal, emitido_por, apoderado_id, registrado_por,
                     idempotency_key, wamid, registrado_en)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                 ON CONFLICT (pregunta_id, bien_privado_id) DO NOTHING
                 RETURNING id",
                p.id.as_uuid(),
                self.ctx.tenant_id.as_uuid(),
                p.pregunta_id.as_uuid(),
                p.bien_privado_id.as_uuid(),
                p.asistente_id.as_uuid(),
                p.coeficiente.valor(),
                p.canal.as_str(),
                p.emitido_por.as_uuid(),
                p.apoderado_id.map(PersonaId::as_uuid),
                p.registrado_por.map(CuentaId::as_uuid),
                p.idempotency_key.as_str(),
                p.wamid,
                p.registrado_en,
            )
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(error_voto)?;
            if insertada.is_none() {
                continue;
            }
            for voto in v.votos.iter().filter(|x| x.participacion_id == p.id) {
                sqlx::query!(
                    "INSERT INTO votos (id, tenant_id, pregunta_id, opcion_id, coeficiente,
                         bien_privado_id, participacion_id)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    voto.id.as_uuid(),
                    self.ctx.tenant_id.as_uuid(),
                    voto.pregunta_id.as_uuid(),
                    voto.opcion_id.as_uuid(),
                    voto.coeficiente.valor(),
                    voto.bien_privado_id.as_uuid(),
                    voto.participacion_id.as_uuid(),
                )
                .execute(&mut *self.tx)
                .await
                .map_err(error_voto)?;
            }
            n += 1;
        }
        Ok(n)
    }

    async fn conteo_votos(&mut self, pregunta: PreguntaId) -> AppResult<ConteoVotos> {
        let v = sqlx::query!(
            r#"SELECT coalesce(sum(coeficiente), 0) AS "c!", count(*) AS "u!"
               FROM participaciones WHERE pregunta_id = $1"#,
            pregunta.as_uuid()
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let por_opcion = sqlx::query!(
            r#"SELECT opcion_id, sum(coeficiente) AS "c!", count(*) AS "u!"
               FROM votos WHERE pregunta_id = $1 GROUP BY opcion_id"#,
            pregunta.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(VotosOpcion {
                opcion_id: OpcionId::from_uuid(f.opcion_id),
                votos: Conteo::new(f.c, entero(f.u, "votos")?),
            })
        })
        .collect::<AppResult<_>>()?;
        Ok(ConteoVotos {
            votantes: Conteo::new(v.c, entero(v.u, "votantes")?),
            por_opcion,
        })
    }

    async fn detalle_votos(&mut self, pregunta: PreguntaId) -> AppResult<Vec<DetalleVoto>> {
        sqlx::query!(
            "SELECT v.bien_privado_id, b.codigo, v.coeficiente, v.opcion_id, p.canal, pe.nombre,
                    p.apoderado_id, p.registrado_en
             FROM votos v
             JOIN participaciones p ON p.id = v.participacion_id
             JOIN bienes_privados b ON b.tenant_id = v.tenant_id AND b.id = v.bien_privado_id
             JOIN personas pe ON pe.tenant_id = p.tenant_id AND pe.id = p.emitido_por
             WHERE v.pregunta_id = $1
             ORDER BY normalizar(b.codigo), v.opcion_id",
            pregunta.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(DetalleVoto {
                bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
                codigo: f.codigo,
                coeficiente: coef(f.coeficiente)?,
                opcion_id: OpcionId::from_uuid(f.opcion_id),
                canal: Canal::try_from(f.canal.as_str())?,
                asistente: f.nombre,
                apoderado: f.apoderado_id.is_some(),
                registrado_en: f.registrado_en,
            })
        })
        .collect()
    }

    async fn participaciones(&mut self, pregunta: PreguntaId) -> AppResult<Vec<ParticipacionActa>> {
        sqlx::query!(
            "SELECT p.bien_privado_id, b.codigo, p.coeficiente, p.canal, pe.nombre, p.apoderado_id,
                    p.registrado_en
             FROM participaciones p
             JOIN bienes_privados b ON b.tenant_id = p.tenant_id AND b.id = p.bien_privado_id
             JOIN personas pe ON pe.tenant_id = p.tenant_id AND pe.id = p.emitido_por
             WHERE p.pregunta_id = $1
             ORDER BY normalizar(b.codigo), p.bien_privado_id",
            pregunta.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(ParticipacionActa {
                bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
                codigo: f.codigo,
                coeficiente: coef(f.coeficiente)?,
                canal: Canal::try_from(f.canal.as_str())?,
                asistente: f.nombre,
                apoderado: f.apoderado_id.is_some(),
                registrado_en: f.registrado_en,
            })
        })
        .collect()
    }

    async fn asistente_voto(
        &mut self,
        pregunta: PreguntaId,
        asistente: AsistenteId,
    ) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM participaciones
                              WHERE pregunta_id = $1 AND asistente_id = $2) AS "existe!""#,
            pregunta.as_uuid(),
            asistente.as_uuid()
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn avisar_en_vivo(&mut self, reunion: ReunionId) -> AppResult<()> {
        // Se entrega a los que escuchan solo si la transacción se confirma.
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(CANAL_EN_VIVO)
            .bind(format!("{}:{}", self.ctx.tenant_id, reunion))
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        Ok(())
    }

    async fn encolar_de_reunion(
        &mut self,
        reunion: ReunionId,
        evento: &EventoSaliente,
    ) -> AppResult<i64> {
        let payload = serde_json::to_value(&evento.mensaje).map_err(AppError::infra)?;
        sqlx::query_scalar!(
            "INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload, reunion_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
            self.ctx.tenant_id.as_uuid(),
            evento.mensaje.canal().as_str(),
            evento.prioridad.as_i16(),
            evento.mensaje.linea(),
            evento.mensaje.destino(),
            payload,
            reunion.as_uuid(),
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn registrar_envio_convocatoria(
        &mut self,
        reunion: ReunionId,
        persona: PersonaId,
        canal: CanalConvocatoria,
        outbox_id: i64,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO envios_convocatoria (tenant_id, reunion_id, persona_id, canal, outbox_id)
             VALUES ($1, $2, $3, $4, $5)",
            self.ctx.tenant_id.as_uuid(),
            reunion.as_uuid(),
            persona.as_uuid(),
            canal.as_str(),
            outbox_id,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn envios_convocatoria(
        &mut self,
        reunion: ReunionId,
    ) -> AppResult<Vec<EnvioConvocatoria>> {
        sqlx::query!(
            r#"SELECT e.persona_id, p.nombre, e.canal, o.estado AS "estado?", o.enviado_en,
                      x.estado AS "acuse?"
               FROM envios_convocatoria e
               JOIN personas p ON p.tenant_id = e.tenant_id AND p.id = e.persona_id
               LEFT JOIN outbox o ON o.id = e.outbox_id
               LEFT JOIN LATERAL estados_entrega(ARRAY[e.outbox_id]) x ON TRUE
               WHERE e.reunion_id = $1
               ORDER BY normalizar(p.nombre), e.id"#,
            reunion.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(EnvioConvocatoria {
                persona_id: PersonaId::from_uuid(f.persona_id),
                nombre: f.nombre,
                canal: if f.canal == "correo" {
                    CanalConvocatoria::Correo
                } else {
                    CanalConvocatoria::Whatsapp
                },
                estado_envio: f.estado.unwrap_or_else(|| "pendiente".into()),
                acuse: f.acuse,
                enviado_en: f.enviado_en,
            })
        })
        .collect()
    }

    async fn estados_entrega(&mut self, outbox: &[i64]) -> AppResult<Vec<(i64, Option<String>)>> {
        Ok(sqlx::query!(
            r#"SELECT outbox_id AS "outbox_id!", estado FROM estados_entrega($1)"#,
            outbox
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| (f.outbox_id, f.estado))
        .collect())
    }

    async fn mensajes_whatsapp(&mut self, reunion: ReunionId) -> AppResult<Vec<MensajesLinea>> {
        sqlx::query!(
            r#"SELECT es_plantilla AS "es_plantilla!", linea, enviados AS "enviados!", fallidos AS "fallidos!"
               FROM mensajes_whatsapp_reunion($1)"#,
            reunion.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(MensajesLinea {
                linea: f.linea,
                es_plantilla: f.es_plantilla,
                enviados: entero(f.enviados, "enviados")?,
                fallidos: entero(f.fallidos, "fallidos")?,
            })
        })
        .collect()
    }

    async fn tarifas_whatsapp(
        &mut self,
        pais: &str,
        moneda: &str,
        fecha: NaiveDate,
    ) -> AppResult<Vec<(String, Decimal)>> {
        Ok(sqlx::query!(
            r#"SELECT categoria AS "categoria!", valor AS "valor!" FROM tarifas_whatsapp_vigentes($1, $2, $3)"#,
            pais,
            moneda,
            fecha
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| (f.categoria, f.valor))
        .collect())
    }

    async fn elegir_linea(&mut self) -> AppResult<Option<String>> {
        sqlx::query_scalar!("SELECT elegir_linea_whatsapp()")
            .fetch_one(&mut *self.tx)
            .await
            .map_err(AppError::infra)
    }
}

impl TransaccionPg {
    async fn poderes_donde(
        &mut self,
        reunion: Option<Uuid>,
        id: Option<Uuid>,
    ) -> AppResult<Vec<Poder>> {
        sqlx::query!(
            "SELECT id, reunion_id, bien_privado_id, poderdante_persona_id, apoderado_persona_id,
                    soporte_archivo_id, estado, motivo, registrado_por, registrado_en,
                    revisado_por, revisado_en
             FROM poderes
             WHERE ($1::uuid IS NULL OR reunion_id = $1) AND ($2::uuid IS NULL OR id = $2)
             ORDER BY registrado_en, id",
            reunion,
            id
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(Poder {
                id: PoderId::from_uuid(f.id),
                reunion_id: ReunionId::from_uuid(f.reunion_id),
                bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
                poderdante: f.poderdante_persona_id.map(PersonaId::from_uuid),
                apoderado: PersonaId::from_uuid(f.apoderado_persona_id),
                soporte_archivo_id: f.soporte_archivo_id.map(ArchivoId::from_uuid),
                estado: EstadoPoder::try_from(f.estado.as_str())?,
                motivo: f.motivo,
                registrado_por: f.registrado_por.map(CuentaId::from_uuid),
                registrado_en: f.registrado_en,
                revisado_por: f.revisado_por.map(CuentaId::from_uuid),
                revisado_en: f.revisado_en,
            })
        })
        .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// Directorio (gateway)
// ---------------------------------------------------------------------------------------------

/// Directorio de asambleas entre copropiedades (funciones `SECURITY DEFINER`).
#[derive(Debug, Clone)]
pub struct DirectorioAsambleasPg {
    pool: PgPool,
}

impl DirectorioAsambleasPg {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl DirectorioAsambleas for DirectorioAsambleasPg {
    async fn asistencias_por_telefono(&self, telefono: &str) -> AppResult<Vec<AsistenciaTelefono>> {
        Ok(sqlx::query!(
            r#"SELECT tenant_id AS "tenant_id!", copropiedad AS "copropiedad!",
                      reunion_id AS "reunion_id!", titulo AS "titulo!",
                      asistente_id AS "asistente_id!", linea, confirmado AS "confirmado!"
               FROM asistencias_por_telefono($1)"#,
            telefono
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| AsistenciaTelefono {
            tenant_id: TenantId::from_uuid(f.tenant_id),
            copropiedad: f.copropiedad,
            reunion_id: ReunionId::from_uuid(f.reunion_id),
            titulo: f.titulo,
            asistente_id: AsistenteId::from_uuid(f.asistente_id),
            linea: f.linea,
            confirmado: f.confirmado,
        })
        .collect())
    }

    async fn copropiedades_con_asambleas_pendientes(&self) -> AppResult<Vec<TenantId>> {
        Ok(sqlx::query_scalar!(
            r#"SELECT tenant_id AS "tenant_id!" FROM copropiedades_con_asambleas_pendientes()"#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TenantId::from_uuid)
        .collect())
    }

    async fn registrar_acuse(&self, a: &AcuseEntrega) -> AppResult<()> {
        sqlx::query!(
            "SELECT registrar_estado_whatsapp($1, $2, $3, $4, $5, $6, $7)",
            a.wamid,
            a.linea,
            a.estado,
            a.categoria,
            a.facturable,
            a.error_codigo,
            a.ocurrido_en,
        )
        .execute(&self.pool)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn actualizar_calidad_linea(
        &self,
        numero: &str,
        degradada: bool,
        calidad: Option<&str>,
    ) -> AppResult<()> {
        sqlx::query!(
            "SELECT actualizar_calidad_linea($1, $2, $3)",
            numero,
            degradada,
            calidad
        )
        .execute(&self.pool)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Líneas y tarifas (plataforma)
// ---------------------------------------------------------------------------------------------

impl RepositorioLineasWhatsapp for TransaccionPlataformaPg {
    async fn lineas(&mut self) -> AppResult<Vec<LineaWhatsapp>> {
        sqlx::query!(
            r#"SELECT l.id, l.phone_number_id, l.numero_visible, l.nombre, l.limite_mps, l.estado,
                      l.calidad, l.franquicia_servicio_mes, coalesce(u.servicio, 0) AS "servicio!",
                      coalesce(u.plantillas, 0) AS "plantillas!"
               FROM lineas_whatsapp l
               LEFT JOIN uso_lineas_whatsapp() u ON u.linea = l.phone_number_id
               ORDER BY l.creado_en, l.id"#
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(LineaWhatsapp {
                id: f.id,
                phone_number_id: f.phone_number_id,
                numero_visible: f.numero_visible,
                nombre: f.nombre,
                limite_mps: entero(i64::from(f.limite_mps), "limite_mps")?,
                estado: EstadoLinea::desde(&f.estado)?,
                calidad: f.calidad,
                franquicia_servicio_mes: entero(
                    i64::from(f.franquicia_servicio_mes),
                    "franquicia",
                )?,
                servicio_mes: entero(f.servicio, "servicio")?,
                plantillas_mes: entero(f.plantillas, "plantillas")?,
            })
        })
        .collect()
    }

    async fn insertar_linea(&mut self, l: &LineaWhatsapp) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO lineas_whatsapp (id, phone_number_id, numero_visible, nombre, limite_mps,
                 estado, franquicia_servicio_mes)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            l.id,
            l.phone_number_id,
            l.numero_visible,
            l.nombre,
            i32::try_from(l.limite_mps).unwrap_or(i32::MAX),
            l.estado.as_str(),
            i32::try_from(l.franquicia_servicio_mes).unwrap_or(i32::MAX),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "la línea ya está registrada", "línea"))?;
        Ok(())
    }

    async fn actualizar_linea(
        &mut self,
        id: Uuid,
        estado: EstadoLinea,
        limite_mps: u32,
        franquicia: u32,
    ) -> AppResult<bool> {
        Ok(sqlx::query!(
            "UPDATE lineas_whatsapp SET estado = $2, limite_mps = $3, franquicia_servicio_mes = $4,
                 actualizado_en = now()
             WHERE id = $1",
            id,
            estado.as_str(),
            i32::try_from(limite_mps).unwrap_or(i32::MAX),
            i32::try_from(franquicia).unwrap_or(i32::MAX),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected()
            > 0)
    }

    async fn tarifas(&mut self) -> AppResult<Vec<TarifaWhatsapp>> {
        Ok(sqlx::query!(
            "SELECT pais, categoria, moneda, valor, vigente_desde FROM tarifas_whatsapp
             ORDER BY pais, categoria, moneda, vigente_desde DESC"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| TarifaWhatsapp {
            pais: f.pais,
            categoria: f.categoria,
            moneda: f.moneda,
            valor: f.valor,
            vigente_desde: f.vigente_desde,
        })
        .collect())
    }

    async fn guardar_tarifa(&mut self, t: &TarifaWhatsapp) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO tarifas_whatsapp (id, pais, categoria, moneda, valor, vigente_desde)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (pais, categoria, moneda, vigente_desde) DO UPDATE SET valor = EXCLUDED.valor",
            Uuid::now_v7(),
            t.pais,
            t.categoria,
            t.moneda,
            t.valor,
            t.vigente_desde,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Avisos en vivo
// ---------------------------------------------------------------------------------------------

/// Oyente de los avisos en vivo de las asambleas (`LISTEN` sobre [`CANAL_EN_VIVO`]). Cada aviso
/// es `{tenant}:{reunion}`; solo llega si la transacción que lo emitió se confirmó.
#[derive(Debug)]
pub struct OyenteEnVivo(sqlx::postgres::PgListener);

impl OyenteEnVivo {
    /// # Errors
    ///
    /// Errores de conexión con PostgreSQL.
    pub async fn conectar(pool: &PgPool) -> Result<Self, crate::InfraPgError> {
        let mut l = sqlx::postgres::PgListener::connect_with(pool).await?;
        l.listen(CANAL_EN_VIVO).await?;
        Ok(Self(l))
    }

    /// Espera el siguiente aviso.
    ///
    /// # Errors
    ///
    /// Si la conexión se perdió (hay que volver a conectar).
    pub async fn recibir(&mut self) -> Result<String, crate::InfraPgError> {
        Ok(self.0.recv().await?.payload().to_owned())
    }
}
