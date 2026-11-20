//! Nómina (plan 16): catálogo laboral de plataforma (`parametros_laborales`,
//! `administradoras_seguridad_social`), que se lee también desde la transacción de la entidad, y
//! empleados con sus contratos y versiones (`empleados`, `contratos_laborales`,
//! `versiones_contrato_laboral`). Las reglas que rechaza la BD llegan con el código `V0010`.

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::nomina::{
        Administradora, ContratoLaboral, CuentaNomina, Empleado, FiltroEmpleados,
        RepositorioCatalogoLaboral, RepositorioNomina, ResumenEmpleado, VersionContrato,
        VersionParametroLaboral,
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AdministradoraId, ContratoLaboralId, CuentaId, Documento, DominioError, EmpleadoId, Nit,
    ParametroLaboralId, PersonaId, VersionContratoId,
    contabilidad::{Monto, tesoreria::TipoCuentaBancaria},
    nomina::{
        ClaseRiesgo, CodigoEmpleado, EstadoEmpleado, EtapaAprendizaje, Jornada, MedioPago,
        MotivoTerminacion, ParametroLaboral, PeriodicidadPago, TipoAdministradora,
        TipoContratoLaboral, TipoSalario,
    },
};
use rust_decimal::Decimal;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{TransaccionPg, TransaccionPlataformaPg, personas::patron};

/// Traduce los errores de la BD de la nómina.
fn error_nomina(e: sqlx::Error, unico: &str, foranea: &'static str) -> AppError {
    match e.as_database_error() {
        Some(d) if d.is_unique_violation() => AppError::Conflicto(unico.to_owned()),
        Some(d) if d.is_foreign_key_violation() => AppError::NoEncontrado(foranea),
        Some(d) if d.code().as_deref() == Some("23P01") => AppError::Conflicto(
            "el empleado ya tiene un contrato que se cruza con esas fechas".into(),
        ),
        Some(d) if d.code().as_deref() == Some("V0010") => {
            AppError::Dominio(DominioError::ReglaLaboral(d.message().to_owned()))
        }
        Some(d)
            if d.code().as_deref() == Some("23514")
                && d.constraint().is_some_and(|c| c.starts_with("ck_")) =>
        {
            AppError::Dominio(DominioError::ReglaLaboral(format!(
                "restricción `{}` incumplida",
                d.constraint().unwrap_or_default()
            )))
        }
        _ => AppError::infra(e),
    }
}

// ---------------------------------------------------------------------------------------------
// Catálogo laboral (plataforma; se lee desde cualquier transacción)
// ---------------------------------------------------------------------------------------------

struct FilaParametro {
    id: Uuid,
    parametro: String,
    valor: Decimal,
    fuente: String,
    vigente_desde: NaiveDate,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
}

impl TryFrom<FilaParametro> for VersionParametroLaboral {
    type Error = AppError;

    fn try_from(f: FilaParametro) -> AppResult<Self> {
        Ok(Self {
            id: ParametroLaboralId::from_uuid(f.id),
            parametro: ParametroLaboral::try_from(f.parametro.as_str())?,
            valor: f.valor,
            fuente: f.fuente,
            vigente_desde: f.vigente_desde,
            creado_por: f.creado_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
        })
    }
}

struct FilaAdministradora {
    id: Uuid,
    tipo: String,
    codigo: String,
    nit: String,
    nombre: String,
    activa: bool,
    creada_en: DateTime<Utc>,
}

impl TryFrom<FilaAdministradora> for Administradora {
    type Error = AppError;

    fn try_from(f: FilaAdministradora) -> AppResult<Self> {
        Ok(Self {
            id: AdministradoraId::from_uuid(f.id),
            tipo: TipoAdministradora::try_from(f.tipo.as_str())?,
            codigo: f.codigo,
            nit: Nit::new(&f.nit)?,
            nombre: f.nombre,
            activa: f.activa,
            creada_en: f.creada_en,
        })
    }
}

async fn insertar_parametro_en(c: &mut PgConnection, v: &VersionParametroLaboral) -> AppResult<()> {
    sqlx::query!(
        "INSERT INTO parametros_laborales (id, parametro, valor, fuente, vigente_desde,
             creado_por, creado_en)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        v.id.as_uuid(),
        v.parametro.as_str(),
        v.valor,
        v.fuente,
        v.vigente_desde,
        v.creado_por.map(CuentaId::as_uuid),
        v.creado_en,
    )
    .execute(c)
    .await
    .map_err(|e| {
        error_nomina(
            e,
            "ya hay una versión del parámetro con esa fecha",
            "parámetro",
        )
    })?;
    Ok(())
}

async fn parametros_de(
    c: &mut PgConnection,
    fecha: Option<NaiveDate>,
) -> AppResult<Vec<VersionParametroLaboral>> {
    sqlx::query_as!(
        FilaParametro,
        "SELECT id, parametro, valor, fuente, vigente_desde, creado_por, creado_en
         FROM parametros_laborales p
         WHERE $1::date IS NULL
            OR vigente_desde = (SELECT max(x.vigente_desde) FROM parametros_laborales x
                                WHERE x.parametro = p.parametro AND x.vigente_desde <= $1)
         ORDER BY parametro, vigente_desde",
        fecha,
    )
    .fetch_all(c)
    .await
    .map_err(AppError::infra)?
    .into_iter()
    .map(TryInto::try_into)
    .collect()
}

async fn insertar_administradora_en(c: &mut PgConnection, a: &Administradora) -> AppResult<()> {
    sqlx::query!(
        "INSERT INTO administradoras_seguridad_social (id, tipo, codigo, nit, nombre, activa,
             creada_en)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        a.id.as_uuid(),
        a.tipo.as_str(),
        a.codigo,
        a.nit.to_string(),
        a.nombre,
        a.activa,
        a.creada_en,
    )
    .execute(c)
    .await
    .map_err(|e| {
        error_nomina(
            e,
            "ya existe una administradora de ese tipo con ese código",
            "administradora",
        )
    })?;
    Ok(())
}

async fn actualizar_administradora_en(c: &mut PgConnection, a: &Administradora) -> AppResult<()> {
    sqlx::query!(
        "UPDATE administradoras_seguridad_social SET nombre = $2, activa = $3 WHERE id = $1",
        a.id.as_uuid(),
        a.nombre,
        a.activa,
    )
    .execute(c)
    .await
    .map_err(AppError::infra)?;
    Ok(())
}

async fn administradora_de(
    c: &mut PgConnection,
    id: AdministradoraId,
) -> AppResult<Option<Administradora>> {
    sqlx::query_as!(
        FilaAdministradora,
        "SELECT id, tipo, codigo, nit, nombre, activa, creada_en
         FROM administradoras_seguridad_social WHERE id = $1",
        id.as_uuid(),
    )
    .fetch_optional(c)
    .await
    .map_err(AppError::infra)?
    .map(TryInto::try_into)
    .transpose()
}

async fn administradoras_de(
    c: &mut PgConnection,
    solo_activas: bool,
) -> AppResult<Vec<Administradora>> {
    sqlx::query_as!(
        FilaAdministradora,
        "SELECT id, tipo, codigo, nit, nombre, activa, creada_en
         FROM administradoras_seguridad_social
         WHERE activa OR NOT $1
         ORDER BY tipo, normalizar(nombre), id",
        solo_activas,
    )
    .fetch_all(c)
    .await
    .map_err(AppError::infra)?
    .into_iter()
    .map(TryInto::try_into)
    .collect()
}

/// El catálogo se escribe desde la plataforma (las políticas RLS rechazan la escritura desde
/// una entidad) y se lee desde cualquier transacción.
macro_rules! catalogo_laboral {
    ($tipo:ty) => {
        impl RepositorioCatalogoLaboral for $tipo {
            async fn insertar_parametro(&mut self, v: &VersionParametroLaboral) -> AppResult<()> {
                insertar_parametro_en(&mut self.tx, v).await
            }

            async fn parametros(
                &mut self,
                fecha: Option<NaiveDate>,
            ) -> AppResult<Vec<VersionParametroLaboral>> {
                parametros_de(&mut self.tx, fecha).await
            }

            async fn insertar_administradora(&mut self, a: &Administradora) -> AppResult<()> {
                insertar_administradora_en(&mut self.tx, a).await
            }

            async fn actualizar_administradora(&mut self, a: &Administradora) -> AppResult<()> {
                actualizar_administradora_en(&mut self.tx, a).await
            }

            async fn administradora(
                &mut self,
                id: AdministradoraId,
            ) -> AppResult<Option<Administradora>> {
                administradora_de(&mut self.tx, id).await
            }

            async fn administradoras(
                &mut self,
                solo_activas: bool,
            ) -> AppResult<Vec<Administradora>> {
                administradoras_de(&mut self.tx, solo_activas).await
            }
        }
    };
}

catalogo_laboral!(TransaccionPg);
catalogo_laboral!(TransaccionPlataformaPg);

// ---------------------------------------------------------------------------------------------
// Empleados y contratos
// ---------------------------------------------------------------------------------------------

struct FilaEmpleado {
    id: Uuid,
    codigo: String,
    persona_id: Uuid,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
}

impl TryFrom<FilaEmpleado> for Empleado {
    type Error = AppError;

    fn try_from(f: FilaEmpleado) -> AppResult<Self> {
        Ok(Self {
            id: EmpleadoId::from_uuid(f.id),
            codigo: CodigoEmpleado::new(&f.codigo)?,
            persona_id: PersonaId::from_uuid(f.persona_id),
            creado_por: f.creado_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
        })
    }
}

struct FilaContrato {
    id: Uuid,
    empleado_id: Uuid,
    tipo: String,
    fecha_inicio: NaiveDate,
    fecha_fin: Option<NaiveDate>,
    descripcion_obra: Option<String>,
    fecha_terminacion: Option<NaiveDate>,
    motivo_terminacion: Option<String>,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
    terminado_por: Option<Uuid>,
    terminado_en: Option<DateTime<Utc>>,
}

impl TryFrom<FilaContrato> for ContratoLaboral {
    type Error = AppError;

    fn try_from(f: FilaContrato) -> AppResult<Self> {
        Ok(Self {
            id: ContratoLaboralId::from_uuid(f.id),
            empleado_id: EmpleadoId::from_uuid(f.empleado_id),
            tipo: TipoContratoLaboral::try_from(f.tipo.as_str())?,
            fecha_inicio: f.fecha_inicio,
            fecha_fin: f.fecha_fin,
            descripcion_obra: f.descripcion_obra,
            fecha_terminacion: f.fecha_terminacion,
            motivo_terminacion: f
                .motivo_terminacion
                .as_deref()
                .map(MotivoTerminacion::try_from)
                .transpose()?,
            creado_por: f.creado_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
            terminado_por: f.terminado_por.map(CuentaId::from_uuid),
            terminado_en: f.terminado_en,
        })
    }
}

struct FilaVersion {
    id: Uuid,
    contrato_id: Uuid,
    vigente_desde: NaiveDate,
    cargo: String,
    salario: Decimal,
    tipo_salario: String,
    etapa_aprendizaje: Option<String>,
    jornada: String,
    horas_semanales: Option<Decimal>,
    periodicidad: String,
    eps_id: Uuid,
    afp_id: Option<Uuid>,
    cesantias_id: Option<Uuid>,
    ccf_id: Option<Uuid>,
    arl_id: Option<Uuid>,
    clase_riesgo: Option<i16>,
    pensionado: bool,
    medio_pago: String,
    banco: Option<String>,
    tipo_cuenta: Option<String>,
    numero_cuenta: Option<String>,
    creada_por: Option<Uuid>,
    creada_en: DateTime<Utc>,
}

impl TryFrom<FilaVersion> for VersionContrato {
    type Error = AppError;

    fn try_from(f: FilaVersion) -> AppResult<Self> {
        let cuenta = match (f.banco, f.tipo_cuenta, f.numero_cuenta) {
            (Some(banco), Some(tipo), Some(numero)) => Some(CuentaNomina {
                banco,
                tipo: TipoCuentaBancaria::try_from(tipo.as_str())?,
                numero,
            }),
            _ => None,
        };
        Ok(Self {
            id: VersionContratoId::from_uuid(f.id),
            contrato_id: ContratoLaboralId::from_uuid(f.contrato_id),
            vigente_desde: f.vigente_desde,
            cargo: f.cargo,
            salario: Some(Monto::new(f.salario)?),
            tipo_salario: TipoSalario::try_from(f.tipo_salario.as_str())?,
            etapa_aprendizaje: f
                .etapa_aprendizaje
                .as_deref()
                .map(EtapaAprendizaje::try_from)
                .transpose()?,
            jornada: Jornada::try_from(f.jornada.as_str())?,
            horas_semanales: f.horas_semanales,
            periodicidad: PeriodicidadPago::try_from(f.periodicidad.as_str())?,
            eps_id: AdministradoraId::from_uuid(f.eps_id),
            afp_id: f.afp_id.map(AdministradoraId::from_uuid),
            cesantias_id: f.cesantias_id.map(AdministradoraId::from_uuid),
            ccf_id: f.ccf_id.map(AdministradoraId::from_uuid),
            arl_id: f.arl_id.map(AdministradoraId::from_uuid),
            clase_riesgo: f.clase_riesgo.map(ClaseRiesgo::de_numero).transpose()?,
            pensionado: f.pensionado,
            medio_pago: MedioPago::try_from(f.medio_pago.as_str())?,
            cuenta,
            creada_por: f.creada_por.map(CuentaId::from_uuid),
            creada_en: f.creada_en,
        })
    }
}

impl RepositorioNomina for TransaccionPg {
    async fn insertar_empleado(&mut self, e: &Empleado) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO empleados (id, tenant_id, persona_id, codigo, creado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6)",
            e.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            e.persona_id.as_uuid(),
            e.codigo.as_str(),
            e.creado_por.map(CuentaId::as_uuid),
            e.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_nomina(
                e,
                "ya existe un empleado con ese código o esa persona",
                "persona",
            )
        })?;
        Ok(())
    }

    async fn empleado(&mut self, id: EmpleadoId) -> AppResult<Option<Empleado>> {
        sqlx::query_as!(
            FilaEmpleado,
            "SELECT id, codigo, persona_id, creado_por, creado_en FROM empleados WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn empleado_de_persona(&mut self, persona: PersonaId) -> AppResult<Option<Empleado>> {
        sqlx::query_as!(
            FilaEmpleado,
            "SELECT id, codigo, persona_id, creado_por, creado_en
             FROM empleados WHERE persona_id = $1",
            persona.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn empleados(
        &mut self,
        filtro: &FiltroEmpleados,
        hoy: NaiveDate,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<ResumenEmpleado>> {
        let filas = sqlx::query!(
            r#"SELECT x.id, x.codigo, x.persona_id, x.nombre, x.documento, x.activo AS "activo!",
                      x.cargo, x.fecha_ingreso
               FROM (
                   SELECT e.id, e.codigo, e.persona_id, p.nombre, p.documento,
                          EXISTS (SELECT 1 FROM contratos_laborales c
                                  WHERE c.empleado_id = e.id
                                    AND COALESCE(c.fecha_terminacion, c.fecha_fin, 'infinity')
                                        >= $4) AS activo,
                          (SELECT v.cargo FROM versiones_contrato_laboral v
                           JOIN contratos_laborales c ON c.id = v.contrato_id
                           WHERE c.empleado_id = e.id
                           ORDER BY v.vigente_desde > $4,
                                    CASE WHEN v.vigente_desde <= $4 THEN v.vigente_desde END
                                        DESC NULLS LAST,
                                    v.vigente_desde
                           LIMIT 1) AS cargo,
                          (SELECT min(c.fecha_inicio) FROM contratos_laborales c
                           WHERE c.empleado_id = e.id) AS fecha_ingreso
                   FROM empleados e
                   JOIN personas p ON p.id = e.persona_id
                   WHERE ($1::uuid IS NULL OR e.id > $1)
                     AND ($2::text IS NULL
                          OR normalizar(p.nombre) LIKE normalizar($2) ESCAPE '\'
                          OR p.documento LIKE upper($2) ESCAPE '\'
                          OR normalizar(e.codigo) LIKE normalizar($2) ESCAPE '\')
               ) x
               WHERE $3::text IS NULL OR ($3 = 'activo') = x.activo
               ORDER BY x.id
               LIMIT $5"#,
            pagina.despues(),
            filtro.texto.as_deref().map(patron),
            filtro.estado.map(EstadoEmpleado::as_str),
            hoy,
            pagina.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(ResumenEmpleado {
                id: EmpleadoId::from_uuid(f.id),
                codigo: CodigoEmpleado::new(&f.codigo)?,
                persona_id: PersonaId::from_uuid(f.persona_id),
                nombre: f.nombre,
                documento: f.documento.as_deref().map(Documento::new).transpose()?,
                estado: if f.activo {
                    EstadoEmpleado::Activo
                } else {
                    EstadoEmpleado::Retirado
                },
                cargo: f.cargo,
                fecha_ingreso: f.fecha_ingreso,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(filas, pagina, |e| e.id.as_uuid()))
    }

    async fn insertar_contrato(&mut self, c: &ContratoLaboral) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO contratos_laborales (id, tenant_id, empleado_id, tipo, fecha_inicio,
                 fecha_fin, descripcion_obra, creado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.empleado_id.as_uuid(),
            c.tipo.as_str(),
            c.fecha_inicio,
            c.fecha_fin,
            c.descripcion_obra,
            c.creado_por.map(CuentaId::as_uuid),
            c.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_nomina(e, "contrato repetido", "empleado"))?;
        Ok(())
    }

    async fn actualizar_contrato(&mut self, c: &ContratoLaboral) -> AppResult<()> {
        sqlx::query!(
            "UPDATE contratos_laborales SET fecha_fin = $2, fecha_terminacion = $3,
                 motivo_terminacion = $4, terminado_por = $5, terminado_en = $6
             WHERE id = $1",
            c.id.as_uuid(),
            c.fecha_fin,
            c.fecha_terminacion,
            c.motivo_terminacion.map(MotivoTerminacion::as_str),
            c.terminado_por.map(CuentaId::as_uuid),
            c.terminado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_nomina(e, "contrato repetido", "contrato"))?;
        Ok(())
    }

    async fn contrato(&mut self, id: ContratoLaboralId) -> AppResult<Option<ContratoLaboral>> {
        sqlx::query_as!(
            FilaContrato,
            "SELECT id, empleado_id, tipo, fecha_inicio, fecha_fin, descripcion_obra,
                    fecha_terminacion, motivo_terminacion, creado_por, creado_en, terminado_por,
                    terminado_en
             FROM contratos_laborales WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn contratos(&mut self, empleado: EmpleadoId) -> AppResult<Vec<ContratoLaboral>> {
        sqlx::query_as!(
            FilaContrato,
            "SELECT id, empleado_id, tipo, fecha_inicio, fecha_fin, descripcion_obra,
                    fecha_terminacion, motivo_terminacion, creado_por, creado_en, terminado_por,
                    terminado_en
             FROM contratos_laborales WHERE empleado_id = $1
             ORDER BY fecha_inicio, id",
            empleado.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn insertar_version(&mut self, v: &VersionContrato) -> AppResult<()> {
        let salario = v
            .salario
            .ok_or_else(|| AppError::Validacion("la versión lleva el salario".into()))?;
        let cuenta = v.cuenta.as_ref();
        sqlx::query!(
            "INSERT INTO versiones_contrato_laboral (id, tenant_id, contrato_id, vigente_desde,
                 cargo, salario, tipo_salario, etapa_aprendizaje, jornada, horas_semanales,
                 periodicidad, eps_id, afp_id, cesantias_id, ccf_id, arl_id, clase_riesgo,
                 pensionado, medio_pago, banco, tipo_cuenta, numero_cuenta, creada_por,
                 creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                     $17, $18, $19, $20, $21, $22, $23, $24)",
            v.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            v.contrato_id.as_uuid(),
            v.vigente_desde,
            v.cargo,
            salario.valor(),
            v.tipo_salario.as_str(),
            v.etapa_aprendizaje.map(EtapaAprendizaje::as_str),
            v.jornada.as_str(),
            v.horas_semanales,
            v.periodicidad.as_str(),
            v.eps_id.as_uuid(),
            v.afp_id.map(AdministradoraId::as_uuid),
            v.cesantias_id.map(AdministradoraId::as_uuid),
            v.ccf_id.map(AdministradoraId::as_uuid),
            v.arl_id.map(AdministradoraId::as_uuid),
            v.clase_riesgo.map(ClaseRiesgo::numero),
            v.pensionado,
            v.medio_pago.as_str(),
            cuenta.map(|c| c.banco.as_str()),
            cuenta.map(|c| c.tipo.as_str()),
            cuenta.map(|c| c.numero.as_str()),
            v.creada_por.map(CuentaId::as_uuid),
            v.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_nomina(
                e,
                "ya hay una versión del contrato con esa fecha",
                "contrato o administradora",
            )
        })?;
        Ok(())
    }

    async fn versiones(&mut self, contrato: ContratoLaboralId) -> AppResult<Vec<VersionContrato>> {
        sqlx::query_as!(
            FilaVersion,
            "SELECT id, contrato_id, vigente_desde, cargo, salario, tipo_salario,
                    etapa_aprendizaje, jornada, horas_semanales, periodicidad, eps_id, afp_id,
                    cesantias_id, ccf_id, arl_id, clase_riesgo, pensionado, medio_pago, banco,
                    tipo_cuenta, numero_cuenta, creada_por, creada_en
             FROM versiones_contrato_laboral WHERE contrato_id = $1
             ORDER BY vigente_desde",
            contrato.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }
}
