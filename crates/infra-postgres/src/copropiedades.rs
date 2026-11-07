//! Ficha de la copropiedad, cargos vigentes y puntos de atención sobre la transacción del
//! tenant.

use application::{
    AppError, AppResult,
    puertos::{
        RepositorioCopropiedad,
        copropiedades::{CargoVigente, FichaCopropiedad, PuntoAtencion},
    },
};
use chrono::{NaiveDate, NaiveTime, Weekday};
use domain::{
    ArchivoId, CodigoDane, CuentaId, DireccionCorreo, EstadoCopropiedad, ExcepcionHorario,
    FormaCopropiedad, FranjaSemanal, Horario, Nit, PlantillaNomenclatura, PuntoAtencionId,
    Telefono, TipoCopropiedad, TipoPuntoAtencion,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::TransaccionPg;

/// Traduce un error de escritura: violación de unicidad → `Conflicto(unico)`, de llave foránea
/// → `NoEncontrado(foranea)`; lo demás es infraestructura.
pub(crate) fn error_escritura(e: sqlx::Error, unico: &str, foranea: &'static str) -> AppError {
    match e.as_database_error() {
        Some(d) if d.is_unique_violation() => AppError::Conflicto(unico.to_owned()),
        Some(d) if d.is_foreign_key_violation() => AppError::NoEncontrado(foranea),
        Some(d) if d.code().is_some_and(|c| c.starts_with('V')) => {
            AppError::Conflicto(d.message().to_owned())
        }
        _ => AppError::infra(e),
    }
}

pub(crate) fn opcional<T, E>(
    v: Option<String>,
    f: impl FnOnce(&str) -> Result<T, E>,
) -> AppResult<Option<T>>
where
    AppError: From<E>,
{
    v.map(|t| f(&t)).transpose().map_err(AppError::from)
}

struct FilaFicha {
    nombre: String,
    razon_social: Option<String>,
    nit: Option<String>,
    tipo: String,
    forma: String,
    direccion: Option<String>,
    municipio_dane: Option<String>,
    departamento: Option<String>,
    barrio: Option<String>,
    latitud: Option<Decimal>,
    longitud: Option<Decimal>,
    estrato: Option<i16>,
    telefono: Option<String>,
    correo: Option<String>,
    sitio_web: Option<String>,
    escritura_numero: Option<String>,
    notaria: Option<String>,
    escritura_fecha: Option<NaiveDate>,
    matricula_matriz: Option<String>,
    reglamento_archivo_id: Option<Uuid>,
    certificado_archivo_id: Option<Uuid>,
    zona_horaria: String,
    estado: String,
    plantilla_codigo: String,
}

impl TryFrom<FilaFicha> for FichaCopropiedad {
    type Error = AppError;

    fn try_from(f: FilaFicha) -> AppResult<Self> {
        Ok(Self {
            nombre: f.nombre,
            razon_social: f.razon_social,
            nit: opcional(f.nit, Nit::new)?,
            tipo: TipoCopropiedad::try_from(f.tipo.as_str())?,
            forma: FormaCopropiedad::try_from(f.forma.as_str())?,
            direccion: f.direccion,
            municipio_dane: opcional(f.municipio_dane, CodigoDane::new)?,
            departamento: f.departamento,
            barrio: f.barrio,
            latitud: f.latitud,
            longitud: f.longitud,
            estrato: f.estrato.and_then(|e| u8::try_from(e).ok()),
            telefono: opcional(f.telefono, Telefono::new)?,
            correo: opcional(f.correo, DireccionCorreo::new)?,
            sitio_web: f.sitio_web,
            escritura_numero: f.escritura_numero,
            notaria: f.notaria,
            escritura_fecha: f.escritura_fecha,
            matricula_matriz: f.matricula_matriz,
            reglamento_archivo_id: f.reglamento_archivo_id.map(ArchivoId::from_uuid),
            certificado_archivo_id: f.certificado_archivo_id.map(ArchivoId::from_uuid),
            zona_horaria: f.zona_horaria,
            estado: EstadoCopropiedad::try_from(f.estado.as_str())?,
            plantilla_codigo: PlantillaNomenclatura::new(&f.plantilla_codigo)?,
        })
    }
}

fn dia(n: i16) -> AppResult<Weekday> {
    u8::try_from(n - 1)
        .ok()
        .and_then(|d| Weekday::try_from(d).ok())
        .ok_or_else(|| AppError::infra(std::io::Error::other("dia_semana fuera de rango")))
}

fn numero_dia(d: Weekday) -> i16 {
    i16::from(u8::try_from(d.number_from_monday()).unwrap_or(1))
}

impl TransaccionPg {
    async fn horarios(&mut self) -> AppResult<Vec<(Uuid, Horario)>> {
        let franjas = sqlx::query!(
            "SELECT punto_atencion_id, dia_semana, hora_inicio, hora_fin, aplica_festivos
             FROM franjas_punto_atencion ORDER BY punto_atencion_id, dia_semana, hora_inicio"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let excepciones = sqlx::query!(
            "SELECT punto_atencion_id, fecha, hora_inicio, hora_fin, motivo
             FROM excepciones_punto_atencion ORDER BY punto_atencion_id, fecha"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let mut puntos: Vec<Uuid> = franjas
            .iter()
            .map(|f| f.punto_atencion_id)
            .chain(excepciones.iter().map(|e| e.punto_atencion_id))
            .collect();
        puntos.sort();
        puntos.dedup();
        puntos
            .into_iter()
            .map(|p| {
                let fr = franjas
                    .iter()
                    .filter(|f| f.punto_atencion_id == p)
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
                    .filter(|e| e.punto_atencion_id == p)
                    .map(|e| ExcepcionHorario {
                        fecha: e.fecha,
                        franja: e.hora_inicio.zip(e.hora_fin),
                        motivo: e.motivo.clone(),
                    })
                    .collect();
                Ok((p, Horario::new(fr, ex)?))
            })
            .collect()
    }
}

impl RepositorioCopropiedad for TransaccionPg {
    async fn ficha(&mut self) -> AppResult<Option<FichaCopropiedad>> {
        sqlx::query_as!(
            FilaFicha,
            "SELECT nombre, razon_social, nit, tipo, forma, direccion, municipio_dane, departamento,
                    barrio, latitud, longitud, estrato, telefono, correo, sitio_web,
                    escritura_numero, notaria, escritura_fecha, matricula_matriz,
                    reglamento_archivo_id, certificado_archivo_id, zona_horaria, estado,
                    plantilla_codigo
             FROM copropiedades WHERE id = $1",
            self.ctx.tenant_id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(FichaCopropiedad::try_from)
        .transpose()
    }

    async fn guardar_ficha(&mut self, f: &FichaCopropiedad) -> AppResult<()> {
        sqlx::query!(
            "UPDATE copropiedades SET
                 nombre = $2, razon_social = $3, nit = $4, tipo = $5, forma = $6, direccion = $7,
                 municipio_dane = $8, departamento = $9, barrio = $10, latitud = $11,
                 longitud = $12, estrato = $13, telefono = $14, correo = $15, sitio_web = $16,
                 escritura_numero = $17, notaria = $18, escritura_fecha = $19,
                 matricula_matriz = $20, reglamento_archivo_id = $21,
                 certificado_archivo_id = $22, zona_horaria = $23, estado = $24,
                 plantilla_codigo = $25, actualizado_en = now()
             WHERE id = $1",
            self.ctx.tenant_id.as_uuid(),
            f.nombre,
            f.razon_social,
            f.nit.as_ref().map(ToString::to_string),
            f.tipo.as_str(),
            f.forma.as_str(),
            f.direccion,
            f.municipio_dane.as_ref().map(CodigoDane::as_str),
            f.departamento,
            f.barrio,
            f.latitud,
            f.longitud,
            f.estrato.map(i16::from),
            f.telefono.as_ref().map(Telefono::as_str),
            f.correo.as_ref().map(DireccionCorreo::as_str),
            f.sitio_web,
            f.escritura_numero,
            f.notaria,
            f.escritura_fecha,
            f.matricula_matriz,
            f.reglamento_archivo_id.map(ArchivoId::as_uuid),
            f.certificado_archivo_id.map(ArchivoId::as_uuid),
            f.zona_horaria,
            f.estado.as_str(),
            f.plantilla_codigo.as_str(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "otra copropiedad ya tiene ese NIT", "archivo"))?;
        Ok(())
    }

    async fn cargos_vigentes(&mut self) -> AppResult<Vec<CargoVigente>> {
        Ok(sqlx::query!(
            r#"SELECT rol AS "rol!", rol_nombre AS "rol_nombre!", cuenta_id AS "cuenta_id!",
                      nombre AS "nombre!", organizacion, vigente_desde AS "vigente_desde!"
               FROM cargos_vigentes()"#
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| CargoVigente {
            rol: f.rol,
            rol_nombre: f.rol_nombre,
            cuenta_id: CuentaId::from_uuid(f.cuenta_id),
            nombre: f.nombre,
            organizacion: f.organizacion,
            vigente_desde: f.vigente_desde,
        })
        .collect())
    }

    async fn bienes_para_consejo(&mut self) -> AppResult<u64> {
        let n = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM bienes_privados
               WHERE activo AND tipo NOT IN ('parqueadero', 'deposito')"#
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    async fn puntos_atencion(&mut self) -> AppResult<Vec<PuntoAtencion>> {
        let filas = sqlx::query!(
            "SELECT id, tipo, nombre, ubicacion, telefono FROM puntos_atencion ORDER BY nombre, id"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let mut horarios = self.horarios().await?;
        filas
            .into_iter()
            .map(|f| {
                let horario = horarios
                    .iter()
                    .position(|(p, _)| *p == f.id)
                    .map(|i| horarios.swap_remove(i).1)
                    .unwrap_or_default();
                Ok(PuntoAtencion {
                    id: PuntoAtencionId::from_uuid(f.id),
                    tipo: TipoPuntoAtencion::try_from(f.tipo.as_str())?,
                    nombre: f.nombre,
                    ubicacion: f.ubicacion,
                    telefono: opcional(f.telefono, Telefono::new)?,
                    horario,
                })
            })
            .collect()
    }

    async fn insertar_punto_atencion(&mut self, p: &PuntoAtencion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO puntos_atencion (id, tenant_id, tipo, nombre, ubicacion, telefono)
             VALUES ($1, $2, $3, $4, $5, $6)",
            p.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            p.tipo.as_str(),
            p.nombre,
            p.ubicacion,
            p.telefono.as_ref().map(Telefono::as_str),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        self.reemplazar_horario(p.id, &p.horario).await?;
        Ok(())
    }

    async fn reemplazar_horario(
        &mut self,
        punto: PuntoAtencionId,
        horario: &Horario,
    ) -> AppResult<bool> {
        let existe = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM puntos_atencion WHERE id = $1) AS "existe!""#,
            punto.as_uuid()
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        if !existe {
            return Ok(false);
        }
        let id = punto.as_uuid();
        let tenant = self.ctx.tenant_id.as_uuid();
        sqlx::query!(
            "DELETE FROM franjas_punto_atencion WHERE punto_atencion_id = $1",
            id
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query!(
            "DELETE FROM excepciones_punto_atencion WHERE punto_atencion_id = $1",
            id
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let dias: Vec<i16> = horario
            .franjas()
            .iter()
            .map(|f| numero_dia(f.dia))
            .collect();
        let inicios: Vec<NaiveTime> = horario.franjas().iter().map(|f| f.inicio).collect();
        let fines: Vec<NaiveTime> = horario.franjas().iter().map(|f| f.fin).collect();
        let festivos: Vec<bool> = horario
            .franjas()
            .iter()
            .map(|f| f.aplica_festivos)
            .collect();
        sqlx::query!(
            "INSERT INTO franjas_punto_atencion
                 (tenant_id, punto_atencion_id, dia_semana, hora_inicio, hora_fin, aplica_festivos)
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
        for e in horario.excepciones() {
            sqlx::query!(
                "INSERT INTO excepciones_punto_atencion
                     (tenant_id, punto_atencion_id, fecha, hora_inicio, hora_fin, motivo)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                tenant,
                id,
                e.fecha,
                e.franja.map(|(i, _)| i),
                e.franja.map(|(_, f)| f),
                e.motivo,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        }
        Ok(true)
    }
}
