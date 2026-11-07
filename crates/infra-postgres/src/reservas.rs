//! Recursos, reservas, parqueaderos y sorteos sobre la transacción del tenant. Las franjas son
//! `TSTZRANGE` `[inicio, fin)`; el no solape de recursos exclusivos y de asignaciones de
//! parqueadero lo garantizan restricciones `EXCLUDE` (btree_gist).

use std::ops::Bound;

use application::{
    AppError, AppResult,
    puertos::{
        DirectorioRecordatorios, RepositorioReservas,
        reservas::{
            AsignacionParqueadero, CargoPendiente, DetalleBienComun, DetalleParqueadero,
            ElementoChecklist, Entrega, FiltroReservas, InscripcionSorteo, Recurso, Reserva,
            Restriccion, Sorteo,
        },
    },
};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use domain::{
    ArchivoId, AsignacionParqueaderoId, BienPrivadoId, CalidadResidencia, CanalReserva,
    CategoriaBienComun, CobroPor, CuentaId, EstadoPago, EstadoRecurso, EstadoReserva, EstadoSorteo,
    ExcepcionHorario, Franja, FranjaSemanal, Horario, MetodoEntrega, ModalidadCita, ModoAsignacion,
    NaturalezaParqueadero, OrigenAsignacionParqueadero, PersonaId, Placa, PoliticaReserva,
    QuienReserva, RecursoId, ReservaId, ResidenciaId, RestriccionId, SorteoId, TenantId,
    TipoEntrega, TipoParqueadero, TipoRecurso, VehiculoId, VinculoSolicitante,
    sorteo::{CriteriosSorteo, Semilla},
};
use rust_decimal::Decimal;
use sqlx::{PgPool, postgres::types::PgRange};
use uuid::Uuid;

use crate::{
    TransaccionPg,
    copropiedades::{dia, error_escritura, numero_dia, opcional},
};

fn rango(inicio: DateTime<Utc>, fin: Option<DateTime<Utc>>) -> PgRange<DateTime<Utc>> {
    PgRange {
        start: Bound::Included(inicio),
        end: fin.map_or(Bound::Unbounded, Bound::Excluded),
    }
}

fn limites(r: &PgRange<DateTime<Utc>>) -> AppResult<(DateTime<Utc>, Option<DateTime<Utc>>)> {
    let inicio = match r.start {
        Bound::Included(x) | Bound::Excluded(x) => x,
        Bound::Unbounded => {
            return Err(AppError::infra(std::io::Error::other("rango sin inicio")));
        }
    };
    let fin = match r.end {
        Bound::Included(x) | Bound::Excluded(x) => Some(x),
        Bound::Unbounded => None,
    };
    Ok((inicio, fin))
}

fn franja(r: &PgRange<DateTime<Utc>>) -> AppResult<Franja> {
    match limites(r)? {
        (i, Some(f)) => Ok(Franja::new(i, f)?),
        _ => Err(AppError::infra(std::io::Error::other("franja sin fin"))),
    }
}

fn u32_de(v: i32) -> u32 {
    u32::try_from(v).unwrap_or(0)
}

fn i32_de(v: u32) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

struct FilaRecurso {
    id: Uuid,
    tipo: String,
    nombre: String,
    estado: String,
    ubicacion: Option<String>,
    descripcion: Option<String>,
    modo_asignacion: String,
    duracion_minima_min: i32,
    duracion_maxima_min: i32,
    granularidad_min: i32,
    anticipacion_minima_horas: i32,
    anticipacion_maxima_dias: i32,
    max_reservas_mes: Option<i32>,
    max_simultaneas: Option<i32>,
    alistamiento_min: i32,
    quien_reserva: String,
    tenedor_requiere_autorizacion: bool,
    aforo: Option<i32>,
    tarifa: Decimal,
    cobro_por: String,
    tarifa_festivo: Option<Decimal>,
    deposito: Decimal,
    cancelacion_sin_penalidad_horas: i32,
    penalidad_porcentaje: Decimal,
    max_invitados: i32,
    metodo_entrega: String,
    categoria: Option<String>,
    area_m2: Option<Decimal>,
    es_esencial: Option<bool>,
    reglas: Option<String>,
    reglas_archivo_id: Option<Uuid>,
    codigo: Option<String>,
    tipo_vehiculo: Option<String>,
    naturaleza: Option<String>,
    parqueadero_bien_id: Option<Uuid>,
}

impl FilaRecurso {
    fn a_recurso(self, horario: Horario) -> AppResult<Recurso> {
        let politica = PoliticaReserva {
            modo: ModoAsignacion::try_from(self.modo_asignacion.as_str())?,
            duracion_minima_min: u32_de(self.duracion_minima_min),
            duracion_maxima_min: u32_de(self.duracion_maxima_min),
            granularidad_min: u32_de(self.granularidad_min),
            anticipacion_minima_horas: u32_de(self.anticipacion_minima_horas),
            anticipacion_maxima_dias: u32_de(self.anticipacion_maxima_dias),
            max_reservas_mes: self.max_reservas_mes.map(u32_de),
            max_simultaneas: self.max_simultaneas.map(u32_de),
            alistamiento_min: u32_de(self.alistamiento_min),
            quien: QuienReserva::try_from(self.quien_reserva.as_str())?,
            tenedor_requiere_autorizacion: self.tenedor_requiere_autorizacion,
            aforo: self.aforo.map(u32_de),
            tarifa: self.tarifa,
            cobro_por: CobroPor::try_from(self.cobro_por.as_str())?,
            tarifa_festivo: self.tarifa_festivo,
            deposito: self.deposito,
            cancelacion_sin_penalidad_horas: u32_de(self.cancelacion_sin_penalidad_horas),
            penalidad_porcentaje: self.penalidad_porcentaje,
            max_invitados: u32_de(self.max_invitados),
            metodo_entrega: MetodoEntrega::try_from(self.metodo_entrega.as_str())?,
        };
        let bien_comun = match self.categoria {
            Some(c) => Some(DetalleBienComun {
                categoria: CategoriaBienComun::try_from(c.as_str())?,
                area_m2: self.area_m2,
                es_esencial: self.es_esencial.unwrap_or(false),
                reglas: self.reglas,
                reglas_archivo_id: self.reglas_archivo_id.map(ArchivoId::from_uuid),
            }),
            None => None,
        };
        let parqueadero = match (self.codigo, self.tipo_vehiculo, self.naturaleza) {
            (Some(codigo), Some(t), Some(n)) => Some(DetalleParqueadero {
                codigo,
                tipo_vehiculo: TipoParqueadero::try_from(t.as_str())?,
                naturaleza: NaturalezaParqueadero::try_from(n.as_str())?,
                bien_privado_id: self.parqueadero_bien_id.map(BienPrivadoId::from_uuid),
            }),
            _ => None,
        };
        Ok(Recurso {
            id: RecursoId::from_uuid(self.id),
            tipo: TipoRecurso::try_from(self.tipo.as_str())?,
            nombre: self.nombre,
            estado: EstadoRecurso::try_from(self.estado.as_str())?,
            ubicacion: self.ubicacion,
            descripcion: self.descripcion,
            politica,
            horario,
            bien_comun,
            parqueadero,
        })
    }
}

struct FilaReserva {
    id: Uuid,
    recurso_id: Uuid,
    bien_privado_id: Uuid,
    solicitante_persona_id: Option<Uuid>,
    solicitada_por: Option<Uuid>,
    franja: PgRange<DateTime<Utc>>,
    exclusiva: bool,
    invitados: i32,
    estado: String,
    valor: Decimal,
    deposito: Decimal,
    penalidad: Decimal,
    estado_pago: String,
    idempotency_key: String,
    canal: String,
    modalidad: Option<String>,
    motivo: Option<String>,
    placa_visitante: Option<String>,
    observacion: Option<String>,
    creada_en: DateTime<Utc>,
}

impl TryFrom<FilaReserva> for Reserva {
    type Error = AppError;

    fn try_from(f: FilaReserva) -> AppResult<Self> {
        Ok(Self {
            id: ReservaId::from_uuid(f.id),
            recurso_id: RecursoId::from_uuid(f.recurso_id),
            bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
            solicitante_persona_id: f.solicitante_persona_id.map(PersonaId::from_uuid),
            solicitada_por: f.solicitada_por.map(CuentaId::from_uuid),
            franja: franja(&f.franja)?,
            exclusiva: f.exclusiva,
            invitados: u32_de(f.invitados),
            estado: EstadoReserva::try_from(f.estado.as_str())?,
            valor: f.valor,
            deposito: f.deposito,
            penalidad: f.penalidad,
            estado_pago: EstadoPago::try_from(f.estado_pago.as_str())?,
            idempotency_key: f.idempotency_key,
            canal: CanalReserva::try_from(f.canal.as_str())?,
            modalidad: opcional(f.modalidad, |m| ModalidadCita::try_from(m))?,
            motivo: f.motivo,
            placa_visitante: opcional(f.placa_visitante, Placa::new)?,
            observacion: f.observacion,
            creada_en: f.creada_en,
        })
    }
}

struct FilaAsignacion {
    id: Uuid,
    recurso_id: Uuid,
    bien_privado_id: Uuid,
    vehiculo_id: Option<Uuid>,
    origen: String,
    vigencia: PgRange<DateTime<Utc>>,
    sorteo_id: Option<Uuid>,
}

impl TryFrom<FilaAsignacion> for AsignacionParqueadero {
    type Error = AppError;

    fn try_from(f: FilaAsignacion) -> AppResult<Self> {
        let (desde, hasta) = limites(&f.vigencia)?;
        Ok(Self {
            id: AsignacionParqueaderoId::from_uuid(f.id),
            recurso_id: RecursoId::from_uuid(f.recurso_id),
            bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
            vehiculo_id: f.vehiculo_id.map(VehiculoId::from_uuid),
            origen: OrigenAsignacionParqueadero::try_from(f.origen.as_str())?,
            desde,
            hasta,
            sorteo_id: f.sorteo_id.map(SorteoId::from_uuid),
        })
    }
}

struct FilaSorteo {
    id: Uuid,
    nombre: String,
    recursos: Vec<Uuid>,
    vigencia: PgRange<DateTime<Utc>>,
    estado: String,
    compromiso: String,
    semilla: String,
    criterios: serde_json::Value,
    resultado: Option<Vec<Uuid>>,
    creado_en: DateTime<Utc>,
    ejecutado_en: Option<DateTime<Utc>>,
}

impl TryFrom<FilaSorteo> for Sorteo {
    type Error = AppError;

    fn try_from(f: FilaSorteo) -> AppResult<Self> {
        let (desde, hasta) = limites(&f.vigencia)?;
        Ok(Self {
            id: SorteoId::from_uuid(f.id),
            nombre: f.nombre,
            recursos: f.recursos.into_iter().map(RecursoId::from_uuid).collect(),
            vigencia_desde: desde,
            vigencia_hasta: hasta.unwrap_or(desde),
            estado: EstadoSorteo::try_from(f.estado.as_str())?,
            compromiso: f.compromiso,
            semilla: Semilla::new(&f.semilla)?,
            criterios: serde_json::from_value::<CriteriosSorteo>(f.criterios)
                .map_err(AppError::infra)?,
            resultado: f
                .resultado
                .map(|v| v.into_iter().map(BienPrivadoId::from_uuid).collect()),
            creado_en: f.creado_en,
            ejecutado_en: f.ejecutado_en,
        })
    }
}

impl TransaccionPg {
    async fn horarios_recursos(&mut self) -> AppResult<Vec<(Uuid, Horario)>> {
        let franjas = sqlx::query!(
            "SELECT recurso_id, dia_semana, hora_inicio, hora_fin, aplica_festivos
             FROM franjas_recurso ORDER BY recurso_id, dia_semana, hora_inicio"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let excepciones = sqlx::query!(
            "SELECT recurso_id, fecha, hora_inicio, hora_fin, motivo
             FROM excepciones_recurso ORDER BY recurso_id, fecha"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let mut ids: Vec<Uuid> = franjas
            .iter()
            .map(|f| f.recurso_id)
            .chain(excepciones.iter().map(|e| e.recurso_id))
            .collect();
        ids.sort();
        ids.dedup();
        ids.into_iter()
            .map(|id| {
                let fr = franjas
                    .iter()
                    .filter(|f| f.recurso_id == id)
                    .map(|f| {
                        Ok(FranjaSemanal {
                            dia: dia(f.dia_semana)?,
                            inicio: f.hora_inicio,
                            fin: f.hora_fin,
                            aplica_festivos: f.aplica_festivos,
                        })
                    })
                    .collect::<AppResult<Vec<_>>>()?;
                let ex = excepciones
                    .iter()
                    .filter(|e| e.recurso_id == id)
                    .map(|e| ExcepcionHorario {
                        fecha: e.fecha,
                        franja: e.hora_inicio.zip(e.hora_fin),
                        motivo: e.motivo.clone(),
                    })
                    .collect();
                Ok((id, Horario::new(fr, ex)?))
            })
            .collect()
    }

    async fn filas_recurso(
        &mut self,
        id: Option<Uuid>,
        tipo: Option<&str>,
        bloquear: bool,
    ) -> AppResult<Vec<FilaRecurso>> {
        if bloquear && let Some(id) = id {
            sqlx::query!("SELECT id FROM recursos WHERE id = $1 FOR UPDATE", id)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(AppError::infra)?;
        }
        sqlx::query_as!(
            FilaRecurso,
            r#"SELECT r.id, r.tipo, r.nombre, r.estado, r.ubicacion, r.descripcion,
                      r.modo_asignacion, r.duracion_minima_min, r.duracion_maxima_min,
                      r.granularidad_min, r.anticipacion_minima_horas, r.anticipacion_maxima_dias,
                      r.max_reservas_mes, r.max_simultaneas, r.alistamiento_min, r.quien_reserva,
                      r.tenedor_requiere_autorizacion, r.aforo, r.tarifa, r.cobro_por,
                      r.tarifa_festivo, r.deposito, r.cancelacion_sin_penalidad_horas,
                      r.penalidad_porcentaje, r.max_invitados, r.metodo_entrega,
                      b.categoria AS "categoria?", b.area_m2 AS "area_m2?",
                      b.es_esencial AS "es_esencial?", b.reglas AS "reglas?",
                      b.reglas_archivo_id AS "reglas_archivo_id?",
                      p.codigo AS "codigo?", p.tipo_vehiculo AS "tipo_vehiculo?",
                      p.naturaleza AS "naturaleza?", p.bien_privado_id AS "parqueadero_bien_id?"
               FROM recursos r
               LEFT JOIN bienes_comunes b ON b.recurso_id = r.id
               LEFT JOIN parqueaderos p   ON p.recurso_id = r.id
               WHERE ($1::uuid IS NULL OR r.id = $1) AND ($2::text IS NULL OR r.tipo = $2)
               ORDER BY normalizar(r.nombre), r.id"#,
            id,
            tipo,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn armar_recursos(&mut self, filas: Vec<FilaRecurso>) -> AppResult<Vec<Recurso>> {
        let mut horarios = self.horarios_recursos().await?;
        filas
            .into_iter()
            .map(|f| {
                let h = horarios
                    .iter()
                    .position(|(id, _)| *id == f.id)
                    .map(|i| horarios.swap_remove(i).1)
                    .unwrap_or_default();
                f.a_recurso(h)
            })
            .collect()
    }

    async fn guardar_horario_recurso(&mut self, id: Uuid, h: &Horario) -> AppResult<()> {
        let tenant = self.ctx.tenant_id.as_uuid();
        sqlx::query!("DELETE FROM franjas_recurso WHERE recurso_id = $1", id)
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        sqlx::query!("DELETE FROM excepciones_recurso WHERE recurso_id = $1", id)
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        let dias: Vec<i16> = h.franjas().iter().map(|f| numero_dia(f.dia)).collect();
        let inicios: Vec<NaiveTime> = h.franjas().iter().map(|f| f.inicio).collect();
        let fines: Vec<NaiveTime> = h.franjas().iter().map(|f| f.fin).collect();
        let festivos: Vec<bool> = h.franjas().iter().map(|f| f.aplica_festivos).collect();
        sqlx::query!(
            "INSERT INTO franjas_recurso
                 (tenant_id, recurso_id, dia_semana, hora_inicio, hora_fin, aplica_festivos)
             SELECT $1, $2, d, i, f, a
             FROM unnest($3::smallint[], $4::time[], $5::time[], $6::bool[]) AS t (d, i, f, a)",
            tenant,
            id,
            &dias,
            &inicios,
            &fines,
            &festivos,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let fechas: Vec<NaiveDate> = h.excepciones().iter().map(|e| e.fecha).collect();
        let ex_inicios: Vec<Option<NaiveTime>> = h
            .excepciones()
            .iter()
            .map(|e| e.franja.map(|(i, _)| i))
            .collect();
        let ex_fines: Vec<Option<NaiveTime>> = h
            .excepciones()
            .iter()
            .map(|e| e.franja.map(|(_, f)| f))
            .collect();
        let motivos: Vec<Option<String>> =
            h.excepciones().iter().map(|e| e.motivo.clone()).collect();
        sqlx::query!(
            "INSERT INTO excepciones_recurso (tenant_id, recurso_id, fecha, hora_inicio, hora_fin, motivo)
             SELECT $1, $2, f, i, x, m
             FROM unnest($3::date[], $4::time[], $5::time[], $6::text[]) AS t (f, i, x, m)",
            tenant,
            id,
            &fechas,
            &ex_inicios as &[Option<NaiveTime>],
            &ex_fines as &[Option<NaiveTime>],
            &motivos as &[Option<String>],
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn guardar_detalles(&mut self, r: &Recurso, insertar: bool) -> AppResult<()> {
        let tenant = self.ctx.tenant_id.as_uuid();
        if let Some(b) = &r.bien_comun {
            sqlx::query!(
                "INSERT INTO bienes_comunes (recurso_id, tenant_id, categoria, area_m2, es_esencial,
                                             reglas, reglas_archivo_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (recurso_id) DO UPDATE SET
                     categoria = EXCLUDED.categoria, area_m2 = EXCLUDED.area_m2,
                     es_esencial = EXCLUDED.es_esencial, reglas = EXCLUDED.reglas,
                     reglas_archivo_id = EXCLUDED.reglas_archivo_id",
                r.id.as_uuid(),
                tenant,
                b.categoria.as_str(),
                b.area_m2,
                b.es_esencial,
                b.reglas,
                b.reglas_archivo_id.map(ArchivoId::as_uuid),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_escritura(e, "", "archivo"))?;
        }
        if let (Some(p), true) = (&r.parqueadero, insertar) {
            sqlx::query!(
                "INSERT INTO parqueaderos (recurso_id, tenant_id, codigo, tipo_vehiculo, naturaleza,
                                           bien_privado_id)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                r.id.as_uuid(),
                tenant,
                p.codigo,
                p.tipo_vehiculo.as_str(),
                p.naturaleza.as_str(),
                p.bien_privado_id.map(BienPrivadoId::as_uuid),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| {
                error_escritura(e, "ya existe un parqueadero con ese código", "bien privado")
            })?;
        }
        Ok(())
    }

    async fn filas_reserva(
        &mut self,
        f: &FiltroReservas,
        clave: Option<&str>,
        id: Option<Uuid>,
    ) -> AppResult<Vec<Reserva>> {
        let entre = f.entre.map(|x| rango(x.inicio(), Some(x.fin())));
        sqlx::query_as!(
            FilaReserva,
            "SELECT id, recurso_id, bien_privado_id, solicitante_persona_id, solicitada_por, franja,
                    exclusiva, invitados, estado, valor, deposito, penalidad, estado_pago,
                    idempotency_key, canal, modalidad, motivo, placa_visitante, observacion,
                    creada_en
             FROM reservas
             WHERE ($1::uuid IS NULL OR recurso_id = $1)
               AND ($2::uuid IS NULL OR bien_privado_id = $2)
               AND ($3::text IS NULL OR estado = $3)
               AND ($4::uuid IS NULL OR solicitada_por = $4)
               AND ($5::tstzrange IS NULL OR franja && $5)
               AND ($6::text IS NULL OR idempotency_key = $6)
               AND ($7::uuid IS NULL OR id = $7)
             ORDER BY lower(franja), id",
            f.recurso_id.map(RecursoId::as_uuid),
            f.bien_privado_id.map(BienPrivadoId::as_uuid),
            f.estado.map(EstadoReserva::as_str),
            f.solicitada_por.map(CuentaId::as_uuid),
            entre as Option<PgRange<DateTime<Utc>>>,
            clave,
            id,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Reserva::try_from)
        .collect()
    }

    async fn filas_asignacion(
        &mut self,
        id: Option<Uuid>,
        recurso: Option<Uuid>,
        bien: Option<Uuid>,
    ) -> AppResult<Vec<AsignacionParqueadero>> {
        sqlx::query_as!(
            FilaAsignacion,
            "SELECT id, recurso_id, bien_privado_id, vehiculo_id, origen, vigencia, sorteo_id
             FROM asignaciones_parqueadero
             WHERE ($1::uuid IS NULL OR id = $1) AND ($2::uuid IS NULL OR recurso_id = $2)
               AND ($3::uuid IS NULL OR bien_privado_id = $3)
             ORDER BY lower(vigencia) DESC, id",
            id,
            recurso,
            bien,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(AsignacionParqueadero::try_from)
        .collect()
    }

    async fn filas_sorteo(&mut self, id: Option<Uuid>) -> AppResult<Vec<Sorteo>> {
        if let Some(id) = id {
            sqlx::query!("SELECT id FROM sorteos WHERE id = $1 FOR UPDATE", id)
                .fetch_optional(&mut *self.tx)
                .await
                .map_err(AppError::infra)?;
        }
        sqlx::query_as!(
            FilaSorteo,
            "SELECT id, nombre, recursos, vigencia, estado, compromiso, semilla, criterios,
                    resultado, creado_en, ejecutado_en
             FROM sorteos WHERE ($1::uuid IS NULL OR id = $1)
             ORDER BY creado_en DESC, id",
            id,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Sorteo::try_from)
        .collect()
    }
}

impl RepositorioReservas for TransaccionPg {
    async fn recursos(&mut self, tipo: Option<TipoRecurso>) -> AppResult<Vec<Recurso>> {
        let filas = self
            .filas_recurso(None, tipo.map(TipoRecurso::as_str), false)
            .await?;
        self.armar_recursos(filas).await
    }

    async fn recurso(&mut self, id: RecursoId) -> AppResult<Option<Recurso>> {
        let filas = self.filas_recurso(Some(id.as_uuid()), None, false).await?;
        Ok(self.armar_recursos(filas).await?.pop())
    }

    async fn recurso_para_reservar(&mut self, id: RecursoId) -> AppResult<Option<Recurso>> {
        let filas = self.filas_recurso(Some(id.as_uuid()), None, true).await?;
        Ok(self.armar_recursos(filas).await?.pop())
    }

    async fn insertar_recurso(&mut self, r: &Recurso) -> AppResult<()> {
        let p = &r.politica;
        sqlx::query!(
            "INSERT INTO recursos (id, tenant_id, tipo, nombre, estado, ubicacion, descripcion,
                 modo_asignacion, duracion_minima_min, duracion_maxima_min, granularidad_min,
                 anticipacion_minima_horas, anticipacion_maxima_dias, max_reservas_mes,
                 max_simultaneas, alistamiento_min, quien_reserva, tenedor_requiere_autorizacion,
                 aforo, tarifa, cobro_por, tarifa_festivo, deposito,
                 cancelacion_sin_penalidad_horas, penalidad_porcentaje, max_invitados,
                 metodo_entrega)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18, $19, $20, $21, $22, $23, $24, $25, $26, $27)",
            r.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            r.tipo.as_str(),
            r.nombre,
            r.estado.as_str(),
            r.ubicacion,
            r.descripcion,
            p.modo.as_str(),
            i32_de(p.duracion_minima_min),
            i32_de(p.duracion_maxima_min),
            i32_de(p.granularidad_min),
            i32_de(p.anticipacion_minima_horas),
            i32_de(p.anticipacion_maxima_dias),
            p.max_reservas_mes.map(i32_de),
            p.max_simultaneas.map(i32_de),
            i32_de(p.alistamiento_min),
            p.quien.as_str(),
            p.tenedor_requiere_autorizacion,
            p.aforo.map(i32_de),
            p.tarifa,
            p.cobro_por.as_str(),
            p.tarifa_festivo,
            p.deposito,
            i32_de(p.cancelacion_sin_penalidad_horas),
            p.penalidad_porcentaje,
            i32_de(p.max_invitados),
            p.metodo_entrega.as_str(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        self.guardar_detalles(r, true).await?;
        self.guardar_horario_recurso(r.id.as_uuid(), &r.horario)
            .await
    }

    async fn actualizar_recurso(&mut self, r: &Recurso) -> AppResult<()> {
        let p = &r.politica;
        sqlx::query!(
            "UPDATE recursos SET nombre = $2, estado = $3, ubicacion = $4, descripcion = $5,
                 modo_asignacion = $6, duracion_minima_min = $7, duracion_maxima_min = $8,
                 granularidad_min = $9, anticipacion_minima_horas = $10,
                 anticipacion_maxima_dias = $11, max_reservas_mes = $12, max_simultaneas = $13,
                 alistamiento_min = $14, quien_reserva = $15, tenedor_requiere_autorizacion = $16,
                 aforo = $17, tarifa = $18, cobro_por = $19, tarifa_festivo = $20, deposito = $21,
                 cancelacion_sin_penalidad_horas = $22, penalidad_porcentaje = $23,
                 max_invitados = $24, metodo_entrega = $25, actualizado_en = now()
             WHERE id = $1",
            r.id.as_uuid(),
            r.nombre,
            r.estado.as_str(),
            r.ubicacion,
            r.descripcion,
            p.modo.as_str(),
            i32_de(p.duracion_minima_min),
            i32_de(p.duracion_maxima_min),
            i32_de(p.granularidad_min),
            i32_de(p.anticipacion_minima_horas),
            i32_de(p.anticipacion_maxima_dias),
            p.max_reservas_mes.map(i32_de),
            p.max_simultaneas.map(i32_de),
            i32_de(p.alistamiento_min),
            p.quien.as_str(),
            p.tenedor_requiere_autorizacion,
            p.aforo.map(i32_de),
            p.tarifa,
            p.cobro_por.as_str(),
            p.tarifa_festivo,
            p.deposito,
            i32_de(p.cancelacion_sin_penalidad_horas),
            p.penalidad_porcentaje,
            i32_de(p.max_invitados),
            p.metodo_entrega.as_str(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        self.guardar_detalles(r, false).await?;
        self.guardar_horario_recurso(r.id.as_uuid(), &r.horario)
            .await
    }

    async fn reservas(&mut self, filtro: &FiltroReservas) -> AppResult<Vec<Reserva>> {
        self.filas_reserva(filtro, None, None).await
    }

    async fn reserva(&mut self, id: ReservaId) -> AppResult<Option<Reserva>> {
        Ok(self
            .filas_reserva(&FiltroReservas::default(), None, Some(id.as_uuid()))
            .await?
            .pop())
    }

    async fn reserva_por_clave(&mut self, clave: &str) -> AppResult<Option<Reserva>> {
        Ok(self
            .filas_reserva(&FiltroReservas::default(), Some(clave), None)
            .await?
            .pop())
    }

    async fn insertar_reserva(&mut self, r: &Reserva) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO reservas (id, tenant_id, recurso_id, bien_privado_id,
                 solicitante_persona_id, solicitada_por, franja, exclusiva, invitados, estado,
                 valor, deposito, penalidad, estado_pago, idempotency_key, canal, modalidad,
                 motivo, placa_visitante, observacion, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18, $19, $20, $21)",
            r.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            r.recurso_id.as_uuid(),
            r.bien_privado_id.as_uuid(),
            r.solicitante_persona_id.map(PersonaId::as_uuid),
            r.solicitada_por.map(CuentaId::as_uuid),
            rango(r.franja.inicio(), Some(r.franja.fin())) as PgRange<DateTime<Utc>>,
            r.exclusiva,
            i32_de(r.invitados),
            r.estado.as_str(),
            r.valor,
            r.deposito,
            r.penalidad,
            r.estado_pago.as_str(),
            r.idempotency_key,
            r.canal.as_str(),
            r.modalidad.map(ModalidadCita::as_str),
            r.motivo,
            r.placa_visitante.as_ref().map(Placa::as_str),
            r.observacion,
            r.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "la franja ya está reservada", "recurso o bien privado"))?;
        Ok(())
    }

    async fn actualizar_reserva(&mut self, r: &Reserva) -> AppResult<()> {
        sqlx::query!(
            "UPDATE reservas SET estado = $2, penalidad = $3, estado_pago = $4, observacion = $5,
                                 actualizada_en = now()
             WHERE id = $1",
            r.id.as_uuid(),
            r.estado.as_str(),
            r.penalidad,
            r.estado_pago.as_str(),
            r.observacion,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn contar_reservas_bien(
        &mut self,
        recurso: RecursoId,
        bien: BienPrivadoId,
        desde: DateTime<Utc>,
        hasta: DateTime<Utc>,
    ) -> AppResult<u32> {
        let n = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM reservas
               WHERE recurso_id = $1 AND bien_privado_id = $2
                 AND estado IN ('solicitada', 'aprobada', 'en_curso')
                 AND lower(franja) >= $3 AND lower(franja) < $4"#,
            recurso.as_uuid(),
            bien.as_uuid(),
            desde,
            hasta,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    async fn vinculo_solicitante(
        &mut self,
        cuenta: CuentaId,
        bien: BienPrivadoId,
    ) -> AppResult<Option<(PersonaId, VinculoSolicitante)>> {
        let Some(f) = sqlx::query!(
            r#"SELECT p.id AS "persona_id!",
                      EXISTS (SELECT 1 FROM titularidades t WHERE t.persona_id = p.id
                              AND t.bien_privado_id = $2 AND t.vigente_hasta IS NULL) AS "es_titular!",
                      (SELECT r.calidad FROM residencias r WHERE r.persona_id = p.id
                              AND r.bien_privado_id = $2 AND r.vigente_hasta IS NULL) AS "calidad?",
                      (SELECT r.reservas_autorizadas FROM residencias r WHERE r.persona_id = p.id
                              AND r.bien_privado_id = $2 AND r.vigente_hasta IS NULL) AS "autorizadas?"
               FROM personas p WHERE p.cuenta_id = $1"#,
            cuenta.as_uuid(),
            bien.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        else {
            return Ok(None);
        };
        let residencia = match f.calidad {
            Some(c) => Some((
                CalidadResidencia::try_from(c.as_str())?,
                f.autorizadas.unwrap_or(false),
            )),
            None => None,
        };
        if !f.es_titular && residencia.is_none() {
            return Ok(None);
        }
        Ok(Some((
            PersonaId::from_uuid(f.persona_id),
            VinculoSolicitante {
                es_titular: f.es_titular,
                residencia,
            },
        )))
    }

    async fn restricciones(&mut self, bien: Option<BienPrivadoId>) -> AppResult<Vec<Restriccion>> {
        sqlx::query!(
            "SELECT id, bien_privado_id, motivo, soporte_archivo_id, desde, hasta, creada_por
             FROM restricciones_reserva WHERE $1::uuid IS NULL OR bien_privado_id = $1
             ORDER BY desde DESC, id",
            bien.map(BienPrivadoId::as_uuid),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)
        .map(|v| {
            v.into_iter()
                .map(|f| Restriccion {
                    id: RestriccionId::from_uuid(f.id),
                    bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
                    motivo: f.motivo,
                    soporte_archivo_id: f.soporte_archivo_id.map(ArchivoId::from_uuid),
                    desde: f.desde,
                    hasta: f.hasta,
                    creada_por: f.creada_por.map(CuentaId::from_uuid),
                })
                .collect()
        })
    }

    async fn insertar_restriccion(&mut self, r: &Restriccion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO restricciones_reserva (id, tenant_id, bien_privado_id, motivo,
                                                soporte_archivo_id, desde, hasta, creada_por)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            r.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            r.bien_privado_id.as_uuid(),
            r.motivo,
            r.soporte_archivo_id.map(ArchivoId::as_uuid),
            r.desde,
            r.hasta,
            r.creada_por.map(CuentaId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "", "bien privado o archivo"))?;
        Ok(())
    }

    async fn levantar_restriccion(
        &mut self,
        id: RestriccionId,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        let n = sqlx::query!(
            "UPDATE restricciones_reserva SET hasta = greatest($2, desde + interval '1 microsecond')
             WHERE id = $1 AND (hasta IS NULL OR hasta > $2)",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        Ok(n > 0)
    }

    async fn insertar_entrega(&mut self, e: &Entrega) -> AppResult<()> {
        let fotos: Vec<Uuid> = e.fotos.iter().map(|f| f.as_uuid()).collect();
        sqlx::query!(
            "INSERT INTO entregas_reserva (id, tenant_id, reserva_id, tipo, checklist, fotos,
                                           novedad, valor_danos, registrado_por, registrado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            Uuid::now_v7(),
            self.ctx.tenant_id.as_uuid(),
            e.reserva_id.as_uuid(),
            e.tipo.as_str(),
            serde_json::to_value(&e.checklist).map_err(AppError::infra)?,
            &fotos,
            e.novedad,
            e.valor_danos,
            e.registrado_por.map(CuentaId::as_uuid),
            e.registrado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|x| error_escritura(x, "ya se registró", "reserva"))?;
        Ok(())
    }

    async fn entregas(&mut self, reserva: ReservaId) -> AppResult<Vec<Entrega>> {
        sqlx::query!(
            "SELECT reserva_id, tipo, checklist, fotos, novedad, valor_danos, registrado_por,
                    registrado_en
             FROM entregas_reserva WHERE reserva_id = $1 ORDER BY registrado_en",
            reserva.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(Entrega {
                reserva_id: ReservaId::from_uuid(f.reserva_id),
                tipo: TipoEntrega::try_from(f.tipo.as_str())?,
                checklist: serde_json::from_value::<Vec<ElementoChecklist>>(f.checklist)
                    .map_err(AppError::infra)?,
                fotos: f.fotos.into_iter().map(ArchivoId::from_uuid).collect(),
                novedad: f.novedad,
                valor_danos: f.valor_danos,
                registrado_por: f.registrado_por.map(CuentaId::from_uuid),
                registrado_en: f.registrado_en,
            })
        })
        .collect()
    }

    async fn insertar_cargo(&mut self, c: &CargoPendiente) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO cargos_pendientes (id, tenant_id, bien_privado_id, concepto, descripcion,
                                            valor, reserva_id, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.bien_privado_id.as_uuid(),
            c.concepto.as_str(),
            c.descripcion,
            c.valor,
            c.reserva_id.map(ReservaId::as_uuid),
            c.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn autorizar_reservas(
        &mut self,
        residencia: ResidenciaId,
        autorizadas: bool,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE residencias SET reservas_autorizadas = $2 WHERE id = $1",
            residencia.as_uuid(),
            autorizadas,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn asignaciones_parqueadero(
        &mut self,
        recurso: Option<RecursoId>,
        bien: Option<BienPrivadoId>,
    ) -> AppResult<Vec<AsignacionParqueadero>> {
        self.filas_asignacion(
            None,
            recurso.map(RecursoId::as_uuid),
            bien.map(BienPrivadoId::as_uuid),
        )
        .await
    }

    async fn asignacion_parqueadero(
        &mut self,
        id: AsignacionParqueaderoId,
    ) -> AppResult<Option<AsignacionParqueadero>> {
        Ok(self
            .filas_asignacion(Some(id.as_uuid()), None, None)
            .await?
            .pop())
    }

    async fn insertar_asignacion_parqueadero(
        &mut self,
        a: &AsignacionParqueadero,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO asignaciones_parqueadero (id, tenant_id, recurso_id, bien_privado_id,
                                                   vehiculo_id, origen, vigencia, sorteo_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            a.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            a.recurso_id.as_uuid(),
            a.bien_privado_id.as_uuid(),
            a.vehiculo_id.map(VehiculoId::as_uuid),
            a.origen.as_str(),
            rango(a.desde, a.hasta) as PgRange<DateTime<Utc>>,
            a.sorteo_id.map(SorteoId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "el parqueadero ya está asignado en ese periodo",
                "parqueadero, bien o vehículo",
            )
        })?;
        Ok(())
    }

    async fn terminar_asignacion_parqueadero(
        &mut self,
        id: AsignacionParqueaderoId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        // Terminar antes de empezar deja la vigencia en un instante (no se puede vaciar el rango).
        sqlx::query!(
            "UPDATE asignaciones_parqueadero
             SET vigencia = tstzrange(lower(vigencia),
                                      greatest($2, lower(vigencia) + interval '1 microsecond'))
             WHERE id = $1 AND (upper(vigencia) IS NULL OR upper(vigencia) > $2)",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn insertar_sorteo(&mut self, s: &Sorteo) -> AppResult<()> {
        let recursos: Vec<Uuid> = s.recursos.iter().map(|r| r.as_uuid()).collect();
        sqlx::query!(
            "INSERT INTO sorteos (id, tenant_id, nombre, recursos, vigencia, estado, compromiso,
                                  semilla, criterios, creado_en)
             VALUES ($1, $2, $3, $4, $5, 'abierto', $6, $7, $8, $9)",
            s.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            s.nombre,
            &recursos,
            rango(s.vigencia_desde, Some(s.vigencia_hasta)) as PgRange<DateTime<Utc>>,
            s.compromiso,
            s.semilla.as_str(),
            serde_json::to_value(s.criterios).map_err(AppError::infra)?,
            s.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn sorteos(&mut self) -> AppResult<Vec<Sorteo>> {
        self.filas_sorteo(None).await
    }

    async fn sorteo(&mut self, id: SorteoId) -> AppResult<Option<Sorteo>> {
        Ok(self.filas_sorteo(Some(id.as_uuid())).await?.pop())
    }

    async fn inscripciones(&mut self, sorteo: SorteoId) -> AppResult<Vec<InscripcionSorteo>> {
        Ok(sqlx::query!(
            "SELECT sorteo_id, bien_privado_id, discapacidad, inscrito_por, inscrito_en
             FROM inscripciones_sorteo WHERE sorteo_id = $1 ORDER BY inscrito_en, id",
            sorteo.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| InscripcionSorteo {
            sorteo_id: SorteoId::from_uuid(f.sorteo_id),
            bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
            discapacidad: f.discapacidad,
            inscrito_por: f.inscrito_por.map(CuentaId::from_uuid),
            inscrito_en: f.inscrito_en,
        })
        .collect())
    }

    async fn insertar_inscripcion(&mut self, i: &InscripcionSorteo) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO inscripciones_sorteo (id, tenant_id, sorteo_id, bien_privado_id,
                                               discapacidad, inscrito_por, inscrito_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            Uuid::now_v7(),
            self.ctx.tenant_id.as_uuid(),
            i.sorteo_id.as_uuid(),
            i.bien_privado_id.as_uuid(),
            i.discapacidad,
            i.inscrito_por.map(CuentaId::as_uuid),
            i.inscrito_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(
                e,
                "el bien privado ya está inscrito",
                "sorteo o bien privado",
            )
        })?;
        Ok(())
    }

    async fn guardar_resultado_sorteo(
        &mut self,
        id: SorteoId,
        resultado: &[BienPrivadoId],
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        let orden: Vec<Uuid> = resultado.iter().map(|b| b.as_uuid()).collect();
        sqlx::query!(
            "UPDATE sorteos SET estado = 'ejecutado', resultado = $2, ejecutado_en = $3 WHERE id = $1",
            id.as_uuid(),
            &orden,
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn reservas_por_recordar(
        &mut self,
        ahora: DateTime<Utc>,
        hasta: DateTime<Utc>,
    ) -> AppResult<Vec<Reserva>> {
        sqlx::query_as!(
            FilaReserva,
            "SELECT id, recurso_id, bien_privado_id, solicitante_persona_id, solicitada_por, franja,
                    exclusiva, invitados, estado, valor, deposito, penalidad, estado_pago,
                    idempotency_key, canal, modalidad, motivo, placa_visitante, observacion,
                    creada_en
             FROM reservas
             WHERE estado = 'aprobada' AND recordatorio_enviado_en IS NULL
               AND lower(franja) > $1 AND lower(franja) <= $2
             ORDER BY lower(franja)
             FOR UPDATE SKIP LOCKED",
            ahora,
            hasta,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Reserva::try_from)
        .collect()
    }

    async fn marcar_recordada(&mut self, id: ReservaId, ahora: DateTime<Utc>) -> AppResult<()> {
        sqlx::query!(
            "UPDATE reservas SET recordatorio_enviado_en = $2 WHERE id = $1",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

/// Directorio de copropiedades con recordatorios pendientes (función `SECURITY DEFINER`).
#[derive(Debug, Clone)]
pub struct DirectorioRecordatoriosPg {
    pool: PgPool,
}

impl DirectorioRecordatoriosPg {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl DirectorioRecordatorios for DirectorioRecordatoriosPg {
    async fn copropiedades_con_recordatorios(
        &self,
        hasta: DateTime<Utc>,
    ) -> AppResult<Vec<TenantId>> {
        Ok(sqlx::query_scalar!(
            r#"SELECT tenant_id AS "tenant_id!" FROM copropiedades_con_recordatorios($1)"#,
            hasta,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TenantId::from_uuid)
        .collect())
    }
}
