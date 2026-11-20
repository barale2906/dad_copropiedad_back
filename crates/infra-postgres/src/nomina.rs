//! Nómina (plan 16): catálogo laboral de plataforma (`parametros_laborales`,
//! `administradoras_seguridad_social`), que se lee también desde la transacción de la entidad, y
//! empleados con sus contratos y versiones (`empleados`, `contratos_laborales`,
//! `versiones_contrato_laboral`). Las reglas que rechaza la BD llegan con el código `V0010`.

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::nomina::{
        Administradora, AnulacionNovedad, ConfiguracionNomina, ContratoLaboral, CuentaNomina,
        DeduccionRetencion, DesprendibleDeCuenta, Empleado, FiltroEmpleados, LiquidacionContrato,
        NovedadNomina, PeriodoNomina, RepositorioCatalogoLaboral, RepositorioLiquidacionNomina,
        RepositorioNomina, RepositorioNominaDeCuenta, ResumenEmpleado, VersionContrato,
        VersionParametroLaboral, VersionTablaLaboral,
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AdministradoraId, ArchivoId, ContratoLaboralId, CuentaId, Documento, DominioError, EmpleadoId,
    LiquidacionNominaId, Nit, NovedadNominaId, ParametroLaboralId, PeriodoNominaId, PersonaId,
    VersionContratoId,
    contabilidad::{Monto, tesoreria::TipoCuentaBancaria},
    nomina::{
        ClaseRiesgo, CodigoEmpleado, EstadoEmpleado, EtapaAprendizaje, Jornada, MedioPago,
        MotivoTerminacion, ParametroLaboral, PeriodicidadPago, TipoAdministradora,
        TipoContratoLaboral, TipoSalario,
        liquidacion::{
            AcumuladoMes, ConceptoNomina, LineaLiquidacion, Liquidacion, Novedad, TablaLaboral,
            TablaTramos, TipoNovedad, Tramo,
        },
        periodo::EstadoPeriodoNomina,
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

async fn insertar_tabla_en(c: &mut PgConnection, t: &VersionTablaLaboral) -> AppResult<()> {
    for tramo in t.tramos.tramos() {
        sqlx::query!(
            "INSERT INTO tramos_laborales (id, tabla, vigente_desde, desde, hasta, tarifa,
                 adicional, fuente, creado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            Uuid::now_v7(),
            t.tabla.as_str(),
            t.vigente_desde,
            tramo.desde,
            tramo.hasta,
            tramo.tarifa,
            tramo.adicional,
            t.fuente,
            t.creado_por.map(CuentaId::as_uuid),
            t.creado_en,
        )
        .execute(&mut *c)
        .await
        .map_err(|e| error_nomina(e, "ya hay una versión de la tabla con esa fecha", "tabla"))?;
    }
    Ok(())
}

async fn tablas_de(
    c: &mut PgConnection,
    fecha: Option<NaiveDate>,
) -> AppResult<Vec<VersionTablaLaboral>> {
    let filas = sqlx::query!(
        "SELECT tabla, vigente_desde, desde, hasta, tarifa, adicional, fuente, creado_por,
                creado_en
         FROM tramos_laborales t
         WHERE $1::date IS NULL
            OR vigente_desde = (SELECT max(x.vigente_desde) FROM tramos_laborales x
                                WHERE x.tabla = t.tabla AND x.vigente_desde <= $1)
         ORDER BY tabla, vigente_desde, desde",
        fecha,
    )
    .fetch_all(c)
    .await
    .map_err(AppError::infra)?;
    // Agrupa las filas por (tabla, vigente_desde): cada grupo es una versión.
    let mut grupos: Vec<(VersionTablaLaboral, Vec<Tramo>)> = Vec::new();
    for f in filas {
        let tabla = TablaLaboral::try_from(f.tabla.as_str())?;
        let tramo = Tramo {
            desde: f.desde,
            hasta: f.hasta,
            tarifa: f.tarifa,
            adicional: f.adicional,
        };
        match grupos.last_mut() {
            Some((v, tramos)) if v.tabla == tabla && v.vigente_desde == f.vigente_desde => {
                tramos.push(tramo);
            }
            _ => grupos.push((
                VersionTablaLaboral {
                    tabla,
                    vigente_desde: f.vigente_desde,
                    fuente: f.fuente,
                    // Se reemplaza al cerrar el grupo con sus tramos.
                    tramos: TablaTramos::new(vec![Tramo {
                        desde: Decimal::ZERO,
                        hasta: None,
                        tarifa: Decimal::ZERO,
                        adicional: Decimal::ZERO,
                    }])?,
                    creado_por: f.creado_por.map(CuentaId::from_uuid),
                    creado_en: f.creado_en,
                },
                vec![tramo],
            )),
        }
    }
    grupos
        .into_iter()
        .map(|(mut v, tramos)| {
            v.tramos = TablaTramos::new(tramos)?;
            Ok(v)
        })
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

            async fn insertar_tabla(&mut self, t: &VersionTablaLaboral) -> AppResult<()> {
                insertar_tabla_en(&mut self.tx, t).await
            }

            async fn tablas(
                &mut self,
                fecha: Option<NaiveDate>,
            ) -> AppResult<Vec<VersionTablaLaboral>> {
                tablas_de(&mut self.tx, fecha).await
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
    centro_costo_id: Option<Uuid>,
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
            centro_costo_id: f.centro_costo_id.map(domain::CentroCostoId::from_uuid),
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

    async fn empleado_por_codigo(
        &mut self,
        codigo: &CodigoEmpleado,
    ) -> AppResult<Option<Empleado>> {
        sqlx::query_as!(
            FilaEmpleado,
            "SELECT id, codigo, persona_id, creado_por, creado_en FROM empleados WHERE codigo = $1",
            codigo.as_str(),
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
                 creada_en, centro_costo_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                     $17, $18, $19, $20, $21, $22, $23, $24, $25)",
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
            v.centro_costo_id.map(domain::CentroCostoId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_nomina(
                e,
                "ya hay una versión del contrato con esa fecha",
                "contrato, administradora o centro de costo",
            )
        })?;
        Ok(())
    }

    async fn versiones(&mut self, contrato: ContratoLaboralId) -> AppResult<Vec<VersionContrato>> {
        sqlx::query_as!(
            FilaVersion,
            "SELECT id, contrato_id, vigente_desde, cargo, salario, tipo_salario,
                    etapa_aprendizaje, jornada, horas_semanales, periodicidad, eps_id, afp_id,
                    cesantias_id, ccf_id, arl_id, clase_riesgo, pensionado, centro_costo_id,
                    medio_pago, banco, tipo_cuenta, numero_cuenta, creada_por, creada_en
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

// ---------------------------------------------------------------------------------------------
// Liquidación
// ---------------------------------------------------------------------------------------------

struct FilaPeriodo {
    id: Uuid,
    periodicidad: String,
    desde: NaiveDate,
    hasta: NaiveDate,
    estado: String,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
    liquidado_por: Option<Uuid>,
    liquidado_en: Option<DateTime<Utc>>,
    aprobado_por: Option<Uuid>,
    aprobado_en: Option<DateTime<Utc>>,
    pagado_por: Option<Uuid>,
    pagado_en: Option<DateTime<Utc>>,
    fecha_pago: Option<NaiveDate>,
}

impl TryFrom<FilaPeriodo> for PeriodoNomina {
    type Error = AppError;

    fn try_from(f: FilaPeriodo) -> AppResult<Self> {
        let cuenta = |u: Option<Uuid>| u.map(CuentaId::from_uuid);
        Ok(Self {
            id: PeriodoNominaId::from_uuid(f.id),
            periodicidad: PeriodicidadPago::try_from(f.periodicidad.as_str())?,
            desde: f.desde,
            hasta: f.hasta,
            estado: EstadoPeriodoNomina::try_from(f.estado.as_str())?,
            creado_por: cuenta(f.creado_por),
            creado_en: f.creado_en,
            liquidado_por: cuenta(f.liquidado_por),
            liquidado_en: f.liquidado_en,
            aprobado_por: cuenta(f.aprobado_por),
            aprobado_en: f.aprobado_en,
            pagado_por: cuenta(f.pagado_por),
            pagado_en: f.pagado_en,
            fecha_pago: f.fecha_pago,
        })
    }
}

struct FilaDeduccion {
    id: Uuid,
    empleado_id: Uuid,
    tipo: String,
    valor_mensual: Decimal,
    vigente_desde: NaiveDate,
    vigente_hasta: Option<NaiveDate>,
    soporte_archivo_id: Option<Uuid>,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
}

impl TryFrom<FilaDeduccion> for DeduccionRetencion {
    type Error = AppError;

    fn try_from(f: FilaDeduccion) -> AppResult<Self> {
        Ok(Self {
            id: domain::DeduccionRetencionId::from_uuid(f.id),
            empleado_id: EmpleadoId::from_uuid(f.empleado_id),
            tipo: domain::nomina::liquidacion::TipoDeduccion::try_from(f.tipo.as_str())?,
            valor_mensual: Monto::new(f.valor_mensual)?,
            vigente_desde: f.vigente_desde,
            vigente_hasta: f.vigente_hasta,
            soporte_archivo_id: f.soporte_archivo_id.map(ArchivoId::from_uuid),
            creado_por: f.creado_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
        })
    }
}

struct FilaNovedad {
    id: Uuid,
    contrato_id: Uuid,
    tipo: String,
    desde: NaiveDate,
    hasta: NaiveDate,
    cantidad: Decimal,
    descripcion: Option<String>,
    soporte_archivo_id: Option<Uuid>,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
    motivo: Option<String>,
    anulada_por: Option<Uuid>,
    anulada_en: Option<DateTime<Utc>>,
}

impl TryFrom<FilaNovedad> for NovedadNomina {
    type Error = AppError;

    fn try_from(f: FilaNovedad) -> AppResult<Self> {
        Ok(Self {
            id: NovedadNominaId::from_uuid(f.id),
            contrato_id: ContratoLaboralId::from_uuid(f.contrato_id),
            // Ya validada al registrarla: se reconstruye tal cual.
            novedad: Novedad {
                tipo: TipoNovedad::try_from(f.tipo.as_str())?,
                desde: f.desde,
                hasta: f.hasta,
                cantidad: f.cantidad,
            },
            descripcion: f.descripcion,
            soporte_archivo_id: f.soporte_archivo_id.map(ArchivoId::from_uuid),
            creado_por: f.creado_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
            anulacion: match (f.motivo, f.anulada_en) {
                (Some(motivo), Some(anulada_en)) => Some(AnulacionNovedad {
                    motivo,
                    anulada_por: f.anulada_por.map(CuentaId::from_uuid),
                    anulada_en,
                }),
                _ => None,
            },
        })
    }
}

impl RepositorioLiquidacionNomina for TransaccionPg {
    async fn configuracion(&mut self) -> AppResult<Option<ConfiguracionNomina>> {
        Ok(sqlx::query!(
            "SELECT exonerada_aportes, actualizada_por, actualizada_en FROM configuraciones_nomina"
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| ConfiguracionNomina {
            exonerada_aportes: f.exonerada_aportes,
            actualizada_por: f.actualizada_por.map(CuentaId::from_uuid),
            actualizada_en: f.actualizada_en,
        }))
    }

    async fn guardar_configuracion(&mut self, c: &ConfiguracionNomina) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO configuraciones_nomina (tenant_id, exonerada_aportes, actualizada_por,
                 actualizada_en)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (tenant_id) DO UPDATE SET exonerada_aportes = EXCLUDED.exonerada_aportes,
                 actualizada_por = EXCLUDED.actualizada_por,
                 actualizada_en = EXCLUDED.actualizada_en",
            self.ctx.tenant_id.as_uuid(),
            c.exonerada_aportes,
            c.actualizada_por.map(CuentaId::as_uuid),
            c.actualizada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn insertar_periodo(&mut self, p: &PeriodoNomina) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO periodos_nomina (id, tenant_id, periodicidad, desde, hasta, estado,
                 creado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            p.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            p.periodicidad.as_str(),
            p.desde,
            p.hasta,
            p.estado.as_str(),
            p.creado_por.map(CuentaId::as_uuid),
            p.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_nomina(e, "el periodo ya existe", "periodo"))?;
        Ok(())
    }

    async fn actualizar_periodo(&mut self, p: &PeriodoNomina) -> AppResult<()> {
        sqlx::query!(
            "UPDATE periodos_nomina SET estado = $2, liquidado_por = $3, liquidado_en = $4,
                 aprobado_por = $5, aprobado_en = $6, pagado_por = $7, pagado_en = $8,
                 fecha_pago = $9
             WHERE id = $1",
            p.id.as_uuid(),
            p.estado.as_str(),
            p.liquidado_por.map(CuentaId::as_uuid),
            p.liquidado_en,
            p.aprobado_por.map(CuentaId::as_uuid),
            p.aprobado_en,
            p.pagado_por.map(CuentaId::as_uuid),
            p.pagado_en,
            p.fecha_pago,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_nomina(e, "periodo repetido", "periodo"))?;
        Ok(())
    }

    async fn periodo(&mut self, id: PeriodoNominaId) -> AppResult<Option<PeriodoNomina>> {
        sqlx::query_as!(
            FilaPeriodo,
            "SELECT id, periodicidad, desde, hasta, estado, creado_por, creado_en, liquidado_por,
                    liquidado_en, aprobado_por, aprobado_en, pagado_por, pagado_en, fecha_pago
             FROM periodos_nomina WHERE id = $1 FOR UPDATE",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn periodo_por_inicio(
        &mut self,
        periodicidad: PeriodicidadPago,
        desde: NaiveDate,
    ) -> AppResult<Option<PeriodoNomina>> {
        sqlx::query_as!(
            FilaPeriodo,
            "SELECT id, periodicidad, desde, hasta, estado, creado_por, creado_en, liquidado_por,
                    liquidado_en, aprobado_por, aprobado_en, pagado_por, pagado_en, fecha_pago
             FROM periodos_nomina WHERE periodicidad = $1 AND desde = $2",
            periodicidad.as_str(),
            desde,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn periodos(&mut self, limite: u32) -> AppResult<Vec<PeriodoNomina>> {
        sqlx::query_as!(
            FilaPeriodo,
            "SELECT id, periodicidad, desde, hasta, estado, creado_por, creado_en, liquidado_por,
                    liquidado_en, aprobado_por, aprobado_en, pagado_por, pagado_en, fecha_pago
             FROM periodos_nomina ORDER BY desde DESC, periodicidad LIMIT $1",
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn insertar_novedad(&mut self, n: &NovedadNomina) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO novedades_nomina (id, tenant_id, contrato_id, tipo, desde, hasta,
                 cantidad, descripcion, soporte_archivo_id, creado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            n.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            n.contrato_id.as_uuid(),
            n.novedad.tipo.as_str(),
            n.novedad.desde,
            n.novedad.hasta,
            n.novedad.cantidad,
            n.descripcion,
            n.soporte_archivo_id.map(ArchivoId::as_uuid),
            n.creado_por.map(CuentaId::as_uuid),
            n.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_nomina(e, "novedad repetida", "contrato o soporte"))?;
        Ok(())
    }

    async fn anular_novedad(&mut self, id: NovedadNominaId, a: &AnulacionNovedad) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO anulaciones_novedad_nomina (novedad_id, tenant_id, motivo, anulada_por,
                 anulada_en)
             VALUES ($1, $2, $3, $4, $5)",
            id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            a.motivo,
            a.anulada_por.map(CuentaId::as_uuid),
            a.anulada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_nomina(e, "la novedad ya estaba anulada", "novedad"))?;
        Ok(())
    }

    async fn novedad(&mut self, id: NovedadNominaId) -> AppResult<Option<NovedadNomina>> {
        sqlx::query_as!(
            FilaNovedad,
            r#"SELECT n.id, n.contrato_id, n.tipo, n.desde, n.hasta, n.cantidad, n.descripcion,
                      n.soporte_archivo_id, n.creado_por, n.creado_en,
                      a.motivo AS "motivo?", a.anulada_por AS "anulada_por?",
                      a.anulada_en AS "anulada_en?"
               FROM novedades_nomina n
               LEFT JOIN anulaciones_novedad_nomina a ON a.novedad_id = n.id
               WHERE n.id = $1"#,
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn novedades(
        &mut self,
        contrato: Option<ContratoLaboralId>,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> AppResult<Vec<NovedadNomina>> {
        sqlx::query_as!(
            FilaNovedad,
            r#"SELECT n.id, n.contrato_id, n.tipo, n.desde, n.hasta, n.cantidad, n.descripcion,
                      n.soporte_archivo_id, n.creado_por, n.creado_en,
                      a.motivo AS "motivo?", a.anulada_por AS "anulada_por?",
                      a.anulada_en AS "anulada_en?"
               FROM novedades_nomina n
               LEFT JOIN anulaciones_novedad_nomina a ON a.novedad_id = n.id
               WHERE ($1::uuid IS NULL OR n.contrato_id = $1)
                 AND n.desde <= $3 AND n.hasta >= $2
               ORDER BY n.desde, n.id"#,
            contrato.map(ContratoLaboralId::as_uuid),
            desde,
            hasta,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn ultimo_cambio_novedades(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> AppResult<Option<DateTime<Utc>>> {
        sqlx::query_scalar!(
            "SELECT max(GREATEST(n.creado_en, a.anulada_en))
             FROM novedades_nomina n
             LEFT JOIN anulaciones_novedad_nomina a ON a.novedad_id = n.id
             WHERE n.desde <= $2 AND n.hasta >= $1",
            desde,
            hasta,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn contratos_en(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> AppResult<Vec<ContratoLaboral>> {
        sqlx::query_as!(
            FilaContrato,
            "SELECT id, empleado_id, tipo, fecha_inicio, fecha_fin, descripcion_obra,
                    fecha_terminacion, motivo_terminacion, creado_por, creado_en, terminado_por,
                    terminado_en
             FROM contratos_laborales
             WHERE fecha_inicio <= $2
               AND COALESCE(fecha_terminacion, fecha_fin, 'infinity') >= $1
             ORDER BY empleado_id, fecha_inicio",
            desde,
            hasta,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn reemplazar_liquidaciones(
        &mut self,
        periodo: PeriodoNominaId,
        liquidaciones: &[LiquidacionContrato],
    ) -> AppResult<()> {
        sqlx::query!(
            "DELETE FROM liquidaciones_nomina WHERE periodo_id = $1",
            periodo.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_nomina(e, "liquidación repetida", "periodo"))?;
        let tenant = self.ctx.tenant_id.as_uuid();
        for l in liquidaciones {
            let q = &l.liquidacion;
            sqlx::query!(
                "INSERT INTO liquidaciones_nomina (id, tenant_id, periodo_id, contrato_id,
                     empleado_id, dias, ibc, devengos, deducciones, neto, aportes_empleador,
                     ingresos_mes, aportes_mes, retenido_mes, creado_en)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
                l.id.as_uuid(),
                tenant,
                periodo.as_uuid(),
                l.contrato_id.as_uuid(),
                l.empleado_id.as_uuid(),
                i16::try_from(q.dias).map_err(AppError::infra)?,
                q.ibc.valor(),
                q.devengos.valor(),
                q.deducciones.valor(),
                q.neto.valor(),
                q.aportes_empleador.valor(),
                q.acumulado.ingresos,
                q.acumulado.aportes,
                q.acumulado.retenido,
                l.creado_en,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_nomina(e, "liquidación repetida", "contrato"))?;
            for linea in &q.lineas {
                sqlx::query!(
                    "INSERT INTO lineas_liquidacion_nomina (liquidacion_id, tenant_id, concepto,
                         clase, cantidad, valor)
                     VALUES ($1, $2, $3, $4, $5, $6)",
                    l.id.as_uuid(),
                    tenant,
                    linea.concepto.as_str(),
                    linea.concepto.clase().as_str(),
                    linea.cantidad.round_dp(4),
                    linea.valor.valor(),
                )
                .execute(&mut *self.tx)
                .await
                .map_err(|e| error_nomina(e, "línea repetida", "liquidación"))?;
            }
        }
        Ok(())
    }

    async fn variables_liquidadas(
        &mut self,
        contrato: ContratoLaboralId,
        desde: NaiveDate,
        hasta: NaiveDate,
        excluir: Option<PeriodoNominaId>,
    ) -> AppResult<Decimal> {
        let conceptos: Vec<String> = [
            ConceptoNomina::HoraExtraDiurna,
            ConceptoNomina::HoraExtraNocturna,
            ConceptoNomina::HoraExtraDominicalDiurna,
            ConceptoNomina::HoraExtraDominicalNocturna,
            ConceptoNomina::RecargoNocturno,
            ConceptoNomina::RecargoDominical,
            ConceptoNomina::RecargoNocturnoDominical,
            ConceptoNomina::Comision,
            ConceptoNomina::BonificacionSalarial,
        ]
        .iter()
        .map(|c| c.as_str().to_owned())
        .collect();
        Ok(sqlx::query_scalar!(
            "SELECT COALESCE(sum(li.valor), 0)
             FROM lineas_liquidacion_nomina li
             JOIN liquidaciones_nomina l ON l.id = li.liquidacion_id
             JOIN periodos_nomina p ON p.id = l.periodo_id
             WHERE l.contrato_id = $1 AND ($4::uuid IS NULL OR p.id <> $4)
               AND p.desde >= $2 AND p.hasta <= $3 AND li.concepto = ANY($5)",
            contrato.as_uuid(),
            desde,
            hasta,
            excluir.map(PeriodoNominaId::as_uuid),
            &conceptos,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .unwrap_or_default())
    }

    async fn insertar_deduccion(&mut self, d: &DeduccionRetencion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO deducciones_retencion (id, tenant_id, empleado_id, tipo, valor_mensual,
                 vigente_desde, vigente_hasta, soporte_archivo_id, creado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            d.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            d.empleado_id.as_uuid(),
            d.tipo.as_str(),
            d.valor_mensual.valor(),
            d.vigente_desde,
            d.vigente_hasta,
            d.soporte_archivo_id.map(ArchivoId::as_uuid),
            d.creado_por.map(CuentaId::as_uuid),
            d.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_nomina(e, "deducción repetida", "empleado o soporte"))?;
        Ok(())
    }

    async fn terminar_deduccion(
        &mut self,
        id: domain::DeduccionRetencionId,
        hasta: NaiveDate,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE deducciones_retencion SET vigente_hasta = $2 WHERE id = $1",
            id.as_uuid(),
            hasta,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_nomina(e, "deducción repetida", "deducción"))?;
        Ok(())
    }

    async fn deduccion(
        &mut self,
        id: domain::DeduccionRetencionId,
    ) -> AppResult<Option<DeduccionRetencion>> {
        sqlx::query_as!(
            FilaDeduccion,
            "SELECT id, empleado_id, tipo, valor_mensual, vigente_desde, vigente_hasta,
                    soporte_archivo_id, creado_por, creado_en
             FROM deducciones_retencion WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn deducciones(&mut self, empleado: EmpleadoId) -> AppResult<Vec<DeduccionRetencion>> {
        sqlx::query_as!(
            FilaDeduccion,
            "SELECT id, empleado_id, tipo, valor_mensual, vigente_desde, vigente_hasta,
                    soporte_archivo_id, creado_por, creado_en
             FROM deducciones_retencion WHERE empleado_id = $1
             ORDER BY vigente_desde DESC, id DESC",
            empleado.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn aviso_enviado(&mut self, tipo: &str, referencia: &str) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM avisos_nomina WHERE tipo = $1 AND referencia = $2)
                   AS "existe!""#,
            tipo,
            referencia,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn registrar_aviso(
        &mut self,
        tipo: &str,
        referencia: &str,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO avisos_nomina (tenant_id, tipo, referencia, enviado_en)
             VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
            self.ctx.tenant_id.as_uuid(),
            tipo,
            referencia,
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn liquidacion(
        &mut self,
        id: LiquidacionNominaId,
    ) -> AppResult<Option<LiquidacionContrato>> {
        let periodo = sqlx::query_scalar!(
            "SELECT periodo_id FROM liquidaciones_nomina WHERE id = $1",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let Some(periodo) = periodo else {
            return Ok(None);
        };
        Ok(self
            .liquidaciones(PeriodoNominaId::from_uuid(periodo))
            .await?
            .into_iter()
            .find(|l| l.id == id))
    }

    async fn liquidaciones(
        &mut self,
        periodo: PeriodoNominaId,
    ) -> AppResult<Vec<LiquidacionContrato>> {
        let cabeceras = sqlx::query!(
            "SELECT id, contrato_id, empleado_id, dias, ibc, devengos, deducciones, neto,
                    aportes_empleador, ingresos_mes, aportes_mes, retenido_mes, creado_en
             FROM liquidaciones_nomina WHERE periodo_id = $1
             ORDER BY empleado_id, contrato_id",
            periodo.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let ids: Vec<Uuid> = cabeceras.iter().map(|c| c.id).collect();
        let lineas = sqlx::query!(
            "SELECT liquidacion_id, concepto, cantidad, valor FROM lineas_liquidacion_nomina
             WHERE liquidacion_id = ANY($1) ORDER BY concepto",
            &ids,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        cabeceras
            .into_iter()
            .map(|c| {
                let lineas = lineas
                    .iter()
                    .filter(|l| l.liquidacion_id == c.id)
                    .map(|l| {
                        Ok(LineaLiquidacion {
                            concepto: ConceptoNomina::try_from(l.concepto.as_str())?,
                            cantidad: l.cantidad,
                            valor: Monto::new(l.valor)?,
                        })
                    })
                    .collect::<AppResult<Vec<_>>>()?;
                Ok(LiquidacionContrato {
                    id: LiquidacionNominaId::from_uuid(c.id),
                    periodo_id: periodo,
                    contrato_id: ContratoLaboralId::from_uuid(c.contrato_id),
                    empleado_id: EmpleadoId::from_uuid(c.empleado_id),
                    liquidacion: Liquidacion {
                        dias: u32::try_from(c.dias).map_err(AppError::infra)?,
                        ibc: Monto::new(c.ibc)?,
                        lineas,
                        devengos: Monto::new(c.devengos)?,
                        deducciones: Monto::new(c.deducciones)?,
                        neto: Monto::new(c.neto)?,
                        aportes_empleador: Monto::new(c.aportes_empleador)?,
                        acumulado: AcumuladoMes {
                            ingresos: c.ingresos_mes,
                            aportes: c.aportes_mes,
                            retenido: c.retenido_mes,
                        },
                    },
                    creado_en: c.creado_en,
                })
            })
            .collect()
    }
}

impl RepositorioNominaDeCuenta for TransaccionPlataformaPg {
    async fn desprendibles_de_cuenta(
        &mut self,
        cuenta: CuentaId,
        limite: u32,
    ) -> AppResult<Vec<DesprendibleDeCuenta>> {
        sqlx::query!(
            r#"SELECT tenant_id AS "tenant_id!", entidad AS "entidad!",
                      liquidacion_id AS "liquidacion_id!", periodicidad AS "periodicidad!",
                      desde AS "desde!", hasta AS "hasta!", estado AS "estado!", fecha_pago,
                      devengos AS "devengos!", deducciones AS "deducciones!", neto AS "neto!"
               FROM desprendibles_de_cuenta($1, $2)"#,
            cuenta.as_uuid(),
            i32::try_from(limite).unwrap_or(i32::MAX),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(DesprendibleDeCuenta {
                tenant_id: domain::TenantId::from_uuid(f.tenant_id),
                entidad: f.entidad,
                liquidacion_id: LiquidacionNominaId::from_uuid(f.liquidacion_id),
                periodicidad: PeriodicidadPago::try_from(f.periodicidad.as_str())?,
                desde: f.desde,
                hasta: f.hasta,
                estado: EstadoPeriodoNomina::try_from(f.estado.as_str())?,
                fecha_pago: f.fecha_pago,
                devengos: Monto::new(f.devengos)?,
                deducciones: Monto::new(f.deducciones)?,
                neto: Monto::new(f.neto)?,
            })
        })
        .collect()
    }
}

/// Directorio de las entidades con nómina (consulta entre tenants del worker de avisos).
#[derive(Debug, Clone)]
pub struct DirectorioNominaPg {
    pool: sqlx::PgPool,
}

impl DirectorioNominaPg {
    /// Directorio sobre el pool de la aplicación (rol `dad_app`).
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl application::puertos::nomina::DirectorioNomina for DirectorioNominaPg {
    async fn entidades_con_nomina(&self, ahora: DateTime<Utc>) -> AppResult<Vec<domain::TenantId>> {
        Ok(sqlx::query_scalar!(
            r#"SELECT tenant_id AS "tenant_id!" FROM entidades_con_nomina($1)"#,
            ahora
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(domain::TenantId::from_uuid)
        .collect())
    }
}
