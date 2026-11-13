//! Núcleo contable sobre PostgreSQL (plan 12, ADR 0017): plan de cuentas, configuración,
//! periodos, comprobantes y saldos en la transacción de la entidad; PUC y plantillas en la de
//! plataforma. La base de datos vuelve a verificar la partida doble, el periodo abierto y las
//! cuentas (migración `contabilidad_nucleo`).

use application::{
    AppError, AppResult,
    puertos::contabilidad::{
        AgrupacionSaldos, AmbitoPlantilla, CatalogoContablePlataforma, CentroCosto, Comprobante,
        ConfiguracionContable, CuentaContable, CuentaPuc, DatosFiscales, EstadoPlantilla,
        FilaSaldo, FiltroComprobantes, Movimiento, NotaEstados, Origen, Periodo, PlantillaGuardada,
        PlantillaResumen, Reapertura, ReglaVersionada, RepositorioComprobantes,
        RepositorioConfiguracionContable, RepositorioPlanCuentas, RubroEstado, TipoComprobante,
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, CentroCostoId, ComprobanteId, CuentaContableId, CuentaId, DominioError,
    NotaEstadosId, OrganizacionId, PeriodoContableId, PersonaId, PlantillaContableId,
    ReaperturaPeriodoId, ReglaContabilizacionId, RubroEstadoFinancieroId, TipoComprobanteId,
    contabilidad::{
        Cara, CodigoCuenta, EstadoComprobante, EstadoPeriodo, EstadoReapertura, LineaAsiento,
        LineaRegla, MarcoTecnico, Monto, Naturaleza, OrigenCentro, OrigenTercero, PeriodoContable,
        RegimenIva, ReglaContable, ResponsabilidadFiscal,
    },
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{TransaccionPg, plataforma::TransaccionPlataformaPg};

/// Traduce los errores de escritura contables: únicos → conflicto; llaves foráneas → no
/// encontrado; reglas de los triggers (`ck_…`) → regla contable; protección de lo contabilizado
/// → conflicto.
fn error_contable(e: sqlx::Error, unico: &str, foranea: &'static str) -> AppError {
    match e.as_database_error() {
        Some(d) if d.is_unique_violation() => AppError::Conflicto(unico.to_owned()),
        Some(d) if d.is_foreign_key_violation() => AppError::NoEncontrado(foranea),
        Some(d)
            if d.code().as_deref() == Some("23514")
                && d.constraint().is_some_and(|c| c.starts_with("ck_")) =>
        {
            AppError::Dominio(DominioError::ReglaContable(d.message().to_owned()))
        }
        Some(d) if d.code().as_deref() == Some("42501") => {
            AppError::Conflicto(d.message().to_owned())
        }
        _ => AppError::infra(e),
    }
}

fn u32_de(v: i32, campo: &'static str) -> AppResult<u32> {
    u32::try_from(v).map_err(|_| AppError::Validacion(format!("`{campo}` fuera de rango")))
}

fn i32_de(v: u32, campo: &'static str) -> AppResult<i32> {
    i32::try_from(v).map_err(|_| AppError::Validacion(format!("`{campo}` fuera de rango")))
}

fn i16_de<T: TryInto<i16>>(v: T, campo: &'static str) -> AppResult<i16> {
    v.try_into()
        .map_err(|_| AppError::Validacion(format!("`{campo}` fuera de rango")))
}

fn periodo_de(anio: i16, mes: i16) -> AppResult<PeriodoContable> {
    Ok(PeriodoContable::new(
        i32::from(anio),
        u8::try_from(mes).unwrap_or(0),
    )?)
}

// ---------------------------------------------------------------------------------------------
// Plan de cuentas
// ---------------------------------------------------------------------------------------------

struct FilaCuenta {
    id: Uuid,
    codigo: String,
    nombre: String,
    naturaleza: String,
    acepta_movimiento: bool,
    exige_tercero: bool,
    exige_centro_costo: bool,
    activa: bool,
}

impl TryFrom<FilaCuenta> for CuentaContable {
    type Error = AppError;

    fn try_from(f: FilaCuenta) -> AppResult<Self> {
        Ok(Self {
            id: CuentaContableId::from_uuid(f.id),
            codigo: CodigoCuenta::new(&f.codigo)?,
            nombre: f.nombre,
            naturaleza: Naturaleza::try_from(f.naturaleza.as_str())?,
            acepta_movimiento: f.acepta_movimiento,
            exige_tercero: f.exige_tercero,
            exige_centro_costo: f.exige_centro_costo,
            activa: f.activa,
        })
    }
}

impl RepositorioPlanCuentas for TransaccionPg {
    async fn cuentas(&mut self) -> AppResult<Vec<CuentaContable>> {
        sqlx::query_as!(
            FilaCuenta,
            "SELECT id, codigo, nombre, naturaleza, acepta_movimiento, exige_tercero,
                    exige_centro_costo, activa
             FROM cuentas_contables ORDER BY codigo"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn cuenta_por_codigo(
        &mut self,
        codigo: &CodigoCuenta,
    ) -> AppResult<Option<CuentaContable>> {
        sqlx::query_as!(
            FilaCuenta,
            "SELECT id, codigo, nombre, naturaleza, acepta_movimiento, exige_tercero,
                    exige_centro_costo, activa
             FROM cuentas_contables WHERE codigo = $1",
            codigo.as_str()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn cuentas_por_id(&mut self, ids: &[CuentaContableId]) -> AppResult<Vec<CuentaContable>> {
        let ids: Vec<Uuid> = ids.iter().map(|i| i.as_uuid()).collect();
        sqlx::query_as!(
            FilaCuenta,
            "SELECT id, codigo, nombre, naturaleza, acepta_movimiento, exige_tercero,
                    exige_centro_costo, activa
             FROM cuentas_contables WHERE id = ANY($1)",
            &ids
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn insertar_cuenta(&mut self, c: &CuentaContable, ahora: DateTime<Utc>) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO cuentas_contables (id, tenant_id, codigo, nombre, naturaleza,
                 acepta_movimiento, exige_tercero, exige_centro_costo, activa, creada_en,
                 actualizada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.codigo.as_str(),
            c.nombre,
            c.naturaleza.as_str(),
            c.acepta_movimiento,
            c.exige_tercero,
            c.exige_centro_costo,
            c.activa,
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, &format!("la cuenta {} ya existe", c.codigo), "cuenta"))?;
        Ok(())
    }

    async fn actualizar_cuenta(
        &mut self,
        c: &CuentaContable,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE cuentas_contables SET nombre = $2, acepta_movimiento = $3, exige_tercero = $4,
                 exige_centro_costo = $5, activa = $6, actualizada_en = $7
             WHERE id = $1",
            c.id.as_uuid(),
            c.nombre,
            c.acepta_movimiento,
            c.exige_tercero,
            c.exige_centro_costo,
            c.activa,
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn cuenta_con_movimientos(&mut self, id: CuentaContableId) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM lineas_comprobante WHERE cuenta_id = $1) AS "e!""#,
            id.as_uuid()
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn centros_costo(&mut self) -> AppResult<Vec<CentroCosto>> {
        Ok(
            sqlx::query!("SELECT id, codigo, nombre, activo FROM centros_costo ORDER BY codigo")
                .fetch_all(&mut *self.tx)
                .await
                .map_err(AppError::infra)?
                .into_iter()
                .map(|f| CentroCosto {
                    id: CentroCostoId::from_uuid(f.id),
                    codigo: f.codigo,
                    nombre: f.nombre,
                    activo: f.activo,
                })
                .collect(),
        )
    }

    async fn insertar_centro_costo(
        &mut self,
        c: &CentroCosto,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO centros_costo (id, tenant_id, codigo, nombre, activo, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.codigo,
            c.nombre,
            c.activo,
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "ese código de centro de costo ya existe",
                "centro de costo",
            )
        })?;
        Ok(())
    }

    async fn actualizar_centro_costo(&mut self, c: &CentroCosto) -> AppResult<()> {
        sqlx::query!(
            "UPDATE centros_costo SET nombre = $2, activo = $3 WHERE id = $1",
            c.id.as_uuid(),
            c.nombre,
            c.activo
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Configuración, tipos, reglas, rubros, notas y terceros
// ---------------------------------------------------------------------------------------------

struct FilaRegla {
    id: Uuid,
    evento: String,
    version: i32,
    tipo_comprobante_id: Uuid,
    descripcion: Option<String>,
    vigente_desde: NaiveDate,
    creada_en: DateTime<Utc>,
}

impl TransaccionPg {
    async fn con_lineas(&mut self, filas: Vec<FilaRegla>) -> AppResult<Vec<ReglaVersionada>> {
        let ids: Vec<Uuid> = filas.iter().map(|f| f.id).collect();
        let lineas = sqlx::query!(
            "SELECT regla_id, concepto, cara, cuenta_id, tercero, centro, centro_costo_id
             FROM lineas_regla WHERE regla_id = ANY($1) ORDER BY regla_id, orden",
            &ids
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        filas
            .into_iter()
            .map(|f| {
                let lineas = lineas
                    .iter()
                    .filter(|l| l.regla_id == f.id)
                    .map(|l| {
                        Ok(LineaRegla {
                            concepto: l.concepto.clone(),
                            cara: Cara::try_from(l.cara.as_str())?,
                            cuenta: CuentaContableId::from_uuid(l.cuenta_id),
                            tercero: OrigenTercero::try_from(l.tercero.as_str())?,
                            centro: OrigenCentro::try_from(l.centro.as_str())?,
                            centro_fijo: l.centro_costo_id.map(CentroCostoId::from_uuid),
                        })
                    })
                    .collect::<AppResult<Vec<_>>>()?;
                Ok(ReglaVersionada {
                    id: ReglaContabilizacionId::from_uuid(f.id),
                    version: u32_de(f.version, "version")?,
                    tipo_comprobante_id: TipoComprobanteId::from_uuid(f.tipo_comprobante_id),
                    descripcion: f.descripcion,
                    vigente_desde: f.vigente_desde,
                    regla: ReglaContable {
                        evento: f.evento,
                        lineas,
                    },
                    creada_en: f.creada_en,
                })
            })
            .collect()
    }
}

struct FilaNota {
    id: Uuid,
    anio: i16,
    numero: i16,
    version: i32,
    titulo: String,
    texto: String,
    rubro_id: Option<Uuid>,
    creada_por: Option<Uuid>,
    creada_en: DateTime<Utc>,
}

impl TryFrom<FilaNota> for NotaEstados {
    type Error = AppError;

    fn try_from(f: FilaNota) -> AppResult<Self> {
        Ok(Self {
            id: NotaEstadosId::from_uuid(f.id),
            anio: i32::from(f.anio),
            numero: u16::try_from(f.numero)
                .map_err(|_| AppError::Validacion("número de nota".into()))?,
            version: u32_de(f.version, "version")?,
            titulo: f.titulo,
            texto: f.texto,
            rubro_id: f.rubro_id.map(RubroEstadoFinancieroId::from_uuid),
            creada_por: f.creada_por.map(CuentaId::from_uuid),
            creada_en: f.creada_en,
        })
    }
}

impl RepositorioConfiguracionContable for TransaccionPg {
    async fn configuracion_vigente(&mut self) -> AppResult<Option<ConfiguracionContable>> {
        sqlx::query!(
            "SELECT version, marco_tecnico, cuenta_resultado_id, soporte_archivo_id,
                    plantilla_origen, vigente_desde, creada_por, creada_en
             FROM configuracion_contable ORDER BY version DESC LIMIT 1"
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| {
            Ok(ConfiguracionContable {
                version: u32_de(f.version, "version")?,
                marco_tecnico: MarcoTecnico::try_from(f.marco_tecnico.as_str())?,
                cuenta_resultado: f.cuenta_resultado_id.map(CuentaContableId::from_uuid),
                soporte_archivo_id: f.soporte_archivo_id.map(ArchivoId::from_uuid),
                plantilla_origen: f.plantilla_origen,
                vigente_desde: f.vigente_desde,
                creada_por: f.creada_por.map(CuentaId::from_uuid),
                creada_en: f.creada_en,
            })
        })
        .transpose()
    }

    async fn inicio_contabilidad(&mut self) -> AppResult<Option<NaiveDate>> {
        sqlx::query_scalar!("SELECT min(vigente_desde) FROM configuracion_contable")
            .fetch_one(&mut *self.tx)
            .await
            .map_err(AppError::infra)
    }

    async fn insertar_configuracion(&mut self, c: &ConfiguracionContable) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO configuracion_contable (id, tenant_id, version, marco_tecnico,
                 cuenta_resultado_id, soporte_archivo_id, plantilla_origen, vigente_desde,
                 creada_por, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            domain::ConfiguracionContableId::nuevo().as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            i32_de(c.version, "version")?,
            c.marco_tecnico.as_str(),
            c.cuenta_resultado.map(CuentaContableId::as_uuid),
            c.soporte_archivo_id.map(ArchivoId::as_uuid),
            c.plantilla_origen,
            c.vigente_desde,
            c.creada_por.map(CuentaId::as_uuid),
            c.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "otra configuración se guardó al tiempo; reintente",
                "archivo o cuenta",
            )
        })?;
        Ok(())
    }

    async fn tipos_comprobante(&mut self) -> AppResult<Vec<TipoComprobante>> {
        Ok(sqlx::query!(
            "SELECT id, codigo, nombre, manual, activo FROM tipos_comprobante ORDER BY codigo"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| TipoComprobante {
            id: TipoComprobanteId::from_uuid(f.id),
            codigo: f.codigo,
            nombre: f.nombre,
            manual: f.manual,
            activo: f.activo,
        })
        .collect())
    }

    async fn insertar_tipo_comprobante(&mut self, t: &TipoComprobante) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO tipos_comprobante (id, tenant_id, codigo, nombre, manual, activo)
             VALUES ($1, $2, $3, $4, $5, $6)",
            t.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            t.codigo,
            t.nombre,
            t.manual,
            t.activo,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                &format!("el tipo de comprobante {} ya existe", t.codigo),
                "tipo",
            )
        })?;
        Ok(())
    }

    async fn actualizar_tipo_comprobante(&mut self, t: &TipoComprobante) -> AppResult<()> {
        sqlx::query!(
            "UPDATE tipos_comprobante SET nombre = $2, manual = $3, activo = $4 WHERE id = $1",
            t.id.as_uuid(),
            t.nombre,
            t.manual,
            t.activo,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn reglas(&mut self) -> AppResult<Vec<ReglaVersionada>> {
        let filas = sqlx::query_as!(
            FilaRegla,
            "SELECT DISTINCT ON (evento) id, evento, version, tipo_comprobante_id, descripcion,
                    vigente_desde, creada_en
             FROM reglas_contabilizacion ORDER BY evento, version DESC"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        self.con_lineas(filas).await
    }

    async fn regla_vigente(
        &mut self,
        evento: &str,
        fecha: NaiveDate,
    ) -> AppResult<Option<ReglaVersionada>> {
        let filas = sqlx::query_as!(
            FilaRegla,
            "SELECT id, evento, version, tipo_comprobante_id, descripcion, vigente_desde, creada_en
             FROM reglas_contabilizacion WHERE evento = $1 AND vigente_desde <= $2
             ORDER BY version DESC LIMIT 1",
            evento,
            fecha
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(self.con_lineas(filas).await?.pop())
    }

    async fn ultima_version_regla(&mut self, evento: &str) -> AppResult<u32> {
        let v = sqlx::query_scalar!(
            r#"SELECT coalesce(max(version), 0) AS "v!" FROM reglas_contabilizacion WHERE evento = $1"#,
            evento
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        u32_de(v, "version")
    }

    async fn insertar_regla(
        &mut self,
        r: &ReglaVersionada,
        creada_por: Option<CuentaId>,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO reglas_contabilizacion (id, tenant_id, evento, version,
                 tipo_comprobante_id, descripcion, vigente_desde, creada_por, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            r.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            r.regla.evento,
            i32_de(r.version, "version")?,
            r.tipo_comprobante_id.as_uuid(),
            r.descripcion,
            r.vigente_desde,
            creada_por.map(CuentaId::as_uuid),
            r.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "otra versión de la regla se guardó al tiempo; reintente",
                "tipo",
            )
        })?;
        for (i, l) in r.regla.lineas.iter().enumerate() {
            sqlx::query!(
                "INSERT INTO lineas_regla (tenant_id, regla_id, orden, concepto, cara, cuenta_id,
                     tercero, centro, centro_costo_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                self.ctx.tenant_id.as_uuid(),
                r.id.as_uuid(),
                i16_de(i + 1, "orden")?,
                l.concepto,
                l.cara.as_str(),
                l.cuenta.as_uuid(),
                l.tercero.as_str(),
                l.centro.as_str(),
                l.centro_fijo.map(CentroCostoId::as_uuid),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_contable(e, "línea repetida", "cuenta o centro de costo"))?;
        }
        Ok(())
    }

    async fn rubros(&mut self) -> AppResult<Vec<RubroEstado>> {
        sqlx::query!(
            "SELECT id, estado, codigo, nombre, orden, prefijos FROM rubros_estado_financiero
             ORDER BY estado, orden, codigo"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(RubroEstado {
                id: RubroEstadoFinancieroId::from_uuid(f.id),
                estado: f.estado,
                codigo: f.codigo,
                nombre: f.nombre,
                orden: u16::try_from(f.orden).map_err(|_| AppError::Validacion("orden".into()))?,
                prefijos: f.prefijos,
            })
        })
        .collect()
    }

    async fn reemplazar_rubros(&mut self, rubros: &[RubroEstado]) -> AppResult<()> {
        for r in rubros {
            sqlx::query!(
                "INSERT INTO rubros_estado_financiero (id, tenant_id, estado, codigo, nombre, orden,
                     prefijos)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (tenant_id, estado, codigo) DO UPDATE
                     SET nombre = EXCLUDED.nombre, orden = EXCLUDED.orden,
                         prefijos = EXCLUDED.prefijos",
                r.id.as_uuid(),
                self.ctx.tenant_id.as_uuid(),
                r.estado,
                r.codigo,
                r.nombre,
                i16_de(r.orden, "orden")?,
                &r.prefijos,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        }
        let codigos: Vec<String> = rubros
            .iter()
            .map(|r| format!("{}:{}", r.estado, r.codigo))
            .collect();
        sqlx::query!(
            "DELETE FROM rubros_estado_financiero WHERE NOT (estado || ':' || codigo = ANY($1))",
            &codigos
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "rubro", "rubro"))
        .map_err(|e| match e {
            AppError::NoEncontrado(_) => AppError::Conflicto(
                "un rubro que se quita tiene notas asociadas: consérvelo".into(),
            ),
            otro => otro,
        })?;
        Ok(())
    }

    async fn notas(&mut self, anio: i32) -> AppResult<Vec<NotaEstados>> {
        sqlx::query_as!(
            FilaNota,
            "SELECT DISTINCT ON (numero) id, anio, numero, version, titulo, texto, rubro_id,
                    creada_por, creada_en
             FROM notas_estados WHERE anio = $1 ORDER BY numero, version DESC",
            i16_de(anio, "anio")?
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn ultima_version_nota(&mut self, anio: i32, numero: u16) -> AppResult<u32> {
        let v = sqlx::query_scalar!(
            r#"SELECT coalesce(max(version), 0) AS "v!" FROM notas_estados
               WHERE anio = $1 AND numero = $2"#,
            i16_de(anio, "anio")?,
            i16_de(numero, "numero")?
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        u32_de(v, "version")
    }

    async fn insertar_nota(&mut self, n: &NotaEstados) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO notas_estados (id, tenant_id, anio, numero, version, titulo, texto,
                 rubro_id, creada_por, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            n.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            i16_de(n.anio, "anio")?,
            i16_de(n.numero, "numero")?,
            i32_de(n.version, "version")?,
            n.titulo,
            n.texto,
            n.rubro_id.map(RubroEstadoFinancieroId::as_uuid),
            n.creada_por.map(CuentaId::as_uuid),
            n.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "otra versión de la nota se guardó al tiempo; reintente",
                "rubro",
            )
        })?;
        Ok(())
    }

    async fn datos_fiscales(&mut self, persona: PersonaId) -> AppResult<Option<DatosFiscales>> {
        sqlx::query!(
            "SELECT organizacion_id, regimen_iva, responsabilidades_fiscales, actividad_ciiu,
                    direccion_fiscal, municipio_fiscal_dane, correo_facturacion
             FROM personas WHERE id = $1",
            persona.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| {
            Ok(DatosFiscales {
                organizacion_id: f.organizacion_id.map(OrganizacionId::from_uuid),
                regimen_iva: f
                    .regimen_iva
                    .as_deref()
                    .map(RegimenIva::try_from)
                    .transpose()?,
                responsabilidades: f
                    .responsabilidades_fiscales
                    .iter()
                    .map(|r| ResponsabilidadFiscal::try_from(r.as_str()))
                    .collect::<Result<_, _>>()?,
                actividad_ciiu: f.actividad_ciiu,
                direccion_fiscal: f.direccion_fiscal,
                municipio_fiscal_dane: f.municipio_fiscal_dane,
                correo_facturacion: f.correo_facturacion,
            })
        })
        .transpose()
    }

    async fn guardar_datos_fiscales(
        &mut self,
        persona: PersonaId,
        d: &DatosFiscales,
    ) -> AppResult<()> {
        let responsabilidades: Vec<String> = d
            .responsabilidades
            .iter()
            .map(|r| r.as_str().to_owned())
            .collect();
        sqlx::query!(
            "UPDATE personas SET organizacion_id = $2, regimen_iva = $3,
                 responsabilidades_fiscales = $4, actividad_ciiu = $5, direccion_fiscal = $6,
                 municipio_fiscal_dane = $7, correo_facturacion = $8
             WHERE id = $1",
            persona.as_uuid(),
            d.organizacion_id.map(OrganizacionId::as_uuid),
            d.regimen_iva.map(RegimenIva::as_str),
            &responsabilidades,
            d.actividad_ciiu,
            d.direccion_fiscal,
            d.municipio_fiscal_dane,
            d.correo_facturacion.as_deref().map(str::to_lowercase),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "dato fiscal repetido", "organización"))?;
        Ok(())
    }

    async fn plantilla_para_aplicar(
        &mut self,
        plantilla: PlantillaContableId,
        cuenta: Option<CuentaId>,
    ) -> AppResult<Option<(String, u32, serde_json::Value)>> {
        sqlx::query!(
            r#"SELECT nombre AS "nombre!", version AS "version!", contenido AS "contenido!"
               FROM plantilla_contable_para_aplicar($1, $2)"#,
            plantilla.as_uuid(),
            cuenta.map(CuentaId::as_uuid)
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| Ok((f.nombre, u32_de(f.version, "version")?, f.contenido)))
        .transpose()
    }

    async fn plantillas_disponibles(
        &mut self,
        cuenta: Option<CuentaId>,
    ) -> AppResult<Vec<PlantillaResumen>> {
        sqlx::query!(
            r#"SELECT id AS "id!", ambito AS "ambito!", organizacion, nombre AS "nombre!",
                      tipo_empresa AS "tipo_empresa!", version AS "version!", publicada_en
               FROM plantillas_contables_disponibles($1)"#,
            cuenta.map(CuentaId::as_uuid)
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(PlantillaResumen {
                id: PlantillaContableId::from_uuid(f.id),
                ambito: f.ambito,
                organizacion: f.organizacion,
                nombre: f.nombre,
                tipo_empresa: f.tipo_empresa,
                version: u32_de(f.version, "version")?,
                estado: EstadoPlantilla::Publicada,
                publicada_en: f.publicada_en,
            })
        })
        .collect()
    }

    async fn catalogo_puc(&mut self, prefijo: Option<&str>) -> AppResult<Vec<CuentaPuc>> {
        sqlx::query!(
            "SELECT codigo, nombre, naturaleza FROM catalogo_puc
             WHERE $1::text IS NULL OR codigo LIKE $1 || '%' ORDER BY codigo",
            prefijo
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(CuentaPuc {
                codigo: CodigoCuenta::new(&f.codigo)?,
                nombre: f.nombre,
                naturaleza: Naturaleza::try_from(f.naturaleza.as_str())?,
            })
        })
        .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// Periodos, comprobantes y saldos
// ---------------------------------------------------------------------------------------------

struct FilaPeriodo {
    id: Uuid,
    anio: i16,
    mes: i16,
    estado: String,
    cerrado_por: Option<Uuid>,
    cerrado_en: Option<DateTime<Utc>>,
}

impl TryFrom<FilaPeriodo> for Periodo {
    type Error = AppError;

    fn try_from(f: FilaPeriodo) -> AppResult<Self> {
        Ok(Self {
            id: PeriodoContableId::from_uuid(f.id),
            periodo: periodo_de(f.anio, f.mes)?,
            estado: EstadoPeriodo::try_from(f.estado.as_str())?,
            cerrado_por: f.cerrado_por.map(CuentaId::from_uuid),
            cerrado_en: f.cerrado_en,
        })
    }
}

struct FilaReapertura {
    id: Uuid,
    periodo_id: Uuid,
    motivo: String,
    alcance: Option<String>,
    solicitada_por: Uuid,
    solicitada_en: DateTime<Utc>,
    estado: String,
    decidida_por: Option<Uuid>,
    decidida_en: Option<DateTime<Utc>>,
}

impl TryFrom<FilaReapertura> for Reapertura {
    type Error = AppError;

    fn try_from(f: FilaReapertura) -> AppResult<Self> {
        Ok(Self {
            id: ReaperturaPeriodoId::from_uuid(f.id),
            periodo_id: PeriodoContableId::from_uuid(f.periodo_id),
            motivo: f.motivo,
            alcance: f.alcance,
            solicitada_por: CuentaId::from_uuid(f.solicitada_por),
            solicitada_en: f.solicitada_en,
            estado: EstadoReapertura::try_from(f.estado.as_str())?,
            decidida_por: f.decidida_por.map(CuentaId::from_uuid),
            decidida_en: f.decidida_en,
        })
    }
}

struct FilaComprobante {
    id: Uuid,
    tipo_comprobante_id: Uuid,
    tipo_codigo: String,
    periodo_id: Uuid,
    anio: i16,
    mes: i16,
    numero: Option<i32>,
    fecha: NaiveDate,
    descripcion: String,
    estado: String,
    origen_tipo: Option<String>,
    origen_id: Option<Uuid>,
    regla_id: Option<Uuid>,
    revierte_a: Option<Uuid>,
    motivo_reverso: Option<String>,
    revertido_por: Option<Uuid>,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
    contabilizado_por: Option<Uuid>,
    contabilizado_en: Option<DateTime<Utc>>,
}

impl FilaComprobante {
    fn a_comprobante(self, lineas: Vec<LineaAsiento>) -> AppResult<Comprobante> {
        Ok(Comprobante {
            id: ComprobanteId::from_uuid(self.id),
            tipo_comprobante_id: TipoComprobanteId::from_uuid(self.tipo_comprobante_id),
            tipo_codigo: self.tipo_codigo,
            periodo_id: PeriodoContableId::from_uuid(self.periodo_id),
            periodo: periodo_de(self.anio, self.mes)?,
            numero: self.numero.map(|n| u32_de(n, "numero")).transpose()?,
            fecha: self.fecha,
            descripcion: self.descripcion,
            estado: EstadoComprobante::try_from(self.estado.as_str())?,
            origen: self
                .origen_tipo
                .zip(self.origen_id)
                .map(|(tipo, id)| Origen { tipo, id }),
            regla_id: self.regla_id.map(ReglaContabilizacionId::from_uuid),
            revierte_a: self.revierte_a.map(ComprobanteId::from_uuid),
            motivo_reverso: self.motivo_reverso,
            revertido_por: self.revertido_por.map(ComprobanteId::from_uuid),
            lineas,
            creado_por: self.creado_por.map(CuentaId::from_uuid),
            creado_en: self.creado_en,
            contabilizado_por: self.contabilizado_por.map(CuentaId::from_uuid),
            contabilizado_en: self.contabilizado_en,
        })
    }
}

struct FilaLinea {
    comprobante_id: Uuid,
    cuenta_id: Uuid,
    persona_id: Option<Uuid>,
    centro_costo_id: Option<Uuid>,
    debito: Decimal,
    credito: Decimal,
    base: Option<Decimal>,
    descripcion: Option<String>,
}

impl TryFrom<&FilaLinea> for LineaAsiento {
    type Error = AppError;

    fn try_from(f: &FilaLinea) -> AppResult<Self> {
        Ok(Self {
            cuenta: CuentaContableId::from_uuid(f.cuenta_id),
            persona: f.persona_id.map(PersonaId::from_uuid),
            centro_costo: f.centro_costo_id.map(CentroCostoId::from_uuid),
            debito: Monto::new(f.debito)?,
            credito: Monto::new(f.credito)?,
            base: f.base.map(Monto::new).transpose()?,
            descripcion: f.descripcion.clone(),
        })
    }
}

impl TransaccionPg {
    async fn lineas_de(&mut self, ids: &[Uuid]) -> AppResult<Vec<FilaLinea>> {
        sqlx::query_as!(
            FilaLinea,
            "SELECT comprobante_id, cuenta_id, persona_id, centro_costo_id, debito, credito, base,
                    descripcion
             FROM lineas_comprobante WHERE comprobante_id = ANY($1)
             ORDER BY comprobante_id, orden",
            ids
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn con_sus_lineas(&mut self, filas: Vec<FilaComprobante>) -> AppResult<Vec<Comprobante>> {
        let ids: Vec<Uuid> = filas.iter().map(|f| f.id).collect();
        let lineas = self.lineas_de(&ids).await?;
        filas
            .into_iter()
            .map(|f| {
                let propias = lineas
                    .iter()
                    .filter(|l| l.comprobante_id == f.id)
                    .map(LineaAsiento::try_from)
                    .collect::<AppResult<Vec<_>>>()?;
                f.a_comprobante(propias)
            })
            .collect()
    }

    async fn insertar_lineas(&mut self, c: &Comprobante) -> AppResult<()> {
        for (i, l) in c.lineas.iter().enumerate() {
            sqlx::query!(
                "INSERT INTO lineas_comprobante (tenant_id, comprobante_id, orden, cuenta_id,
                     persona_id, centro_costo_id, debito, credito, base, descripcion)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                self.ctx.tenant_id.as_uuid(),
                c.id.as_uuid(),
                i16_de(i + 1, "orden")?,
                l.cuenta.as_uuid(),
                l.persona.map(PersonaId::as_uuid),
                l.centro_costo.map(CentroCostoId::as_uuid),
                l.debito.valor(),
                l.credito.valor(),
                l.base.map(Monto::valor),
                l.descripcion,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| {
                error_contable(e, "línea repetida", "cuenta, tercero o centro de costo")
            })?;
        }
        Ok(())
    }
}

impl RepositorioComprobantes for TransaccionPg {
    async fn periodos(&mut self, anio: Option<i32>) -> AppResult<Vec<Periodo>> {
        let anio = anio.map(|a| i16_de(a, "anio")).transpose()?;
        sqlx::query_as!(
            FilaPeriodo,
            "SELECT id, anio, mes, estado, cerrado_por, cerrado_en FROM periodos_contables
             WHERE $1::smallint IS NULL OR anio = $1 ORDER BY anio, mes",
            anio
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn periodo(&mut self, id: PeriodoContableId) -> AppResult<Option<Periodo>> {
        sqlx::query_as!(
            FilaPeriodo,
            "SELECT id, anio, mes, estado, cerrado_por, cerrado_en FROM periodos_contables
             WHERE id = $1",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn periodo_o_crear(&mut self, p: PeriodoContable) -> AppResult<Periodo> {
        let (anio, mes) = (i16_de(p.anio, "anio")?, i16::from(p.mes));
        sqlx::query!(
            "INSERT INTO periodos_contables (id, tenant_id, anio, mes) VALUES ($1, $2, $3, $4)
             ON CONFLICT (tenant_id, anio, mes) DO NOTHING",
            PeriodoContableId::nuevo().as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            anio,
            mes,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query_as!(
            FilaPeriodo,
            "SELECT id, anio, mes, estado, cerrado_por, cerrado_en FROM periodos_contables
             WHERE anio = $1 AND mes = $2",
            anio,
            mes
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .try_into()
    }

    async fn cambiar_estado_periodo(&mut self, p: &Periodo) -> AppResult<()> {
        sqlx::query!(
            "UPDATE periodos_contables SET estado = $2, cerrado_por = $3, cerrado_en = $4
             WHERE id = $1",
            p.id.as_uuid(),
            p.estado.as_str(),
            p.cerrado_por.map(CuentaId::as_uuid),
            p.cerrado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn reaperturas(&mut self, periodo: PeriodoContableId) -> AppResult<Vec<Reapertura>> {
        sqlx::query_as!(
            FilaReapertura,
            "SELECT id, periodo_id, motivo, alcance, solicitada_por, solicitada_en, estado,
                    decidida_por, decidida_en
             FROM reaperturas_periodo WHERE periodo_id = $1 ORDER BY solicitada_en DESC",
            periodo.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn reapertura(&mut self, id: ReaperturaPeriodoId) -> AppResult<Option<Reapertura>> {
        sqlx::query_as!(
            FilaReapertura,
            "SELECT id, periodo_id, motivo, alcance, solicitada_por, solicitada_en, estado,
                    decidida_por, decidida_en
             FROM reaperturas_periodo WHERE id = $1",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn insertar_reapertura(&mut self, r: &Reapertura) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO reaperturas_periodo (id, tenant_id, periodo_id, motivo, alcance,
                 solicitada_por, solicitada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            r.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            r.periodo_id.as_uuid(),
            r.motivo,
            r.alcance,
            r.solicitada_por.as_uuid(),
            r.solicitada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "ya hay una reapertura pendiente para el periodo",
                "periodo",
            )
        })?;
        Ok(())
    }

    async fn decidir_reapertura(&mut self, r: &Reapertura) -> AppResult<()> {
        sqlx::query!(
            "UPDATE reaperturas_periodo SET estado = $2, decidida_por = $3, decidida_en = $4
             WHERE id = $1 AND estado = 'pendiente'",
            r.id.as_uuid(),
            r.estado.as_str(),
            r.decidida_por.map(CuentaId::as_uuid),
            r.decidida_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "reapertura", "reapertura"))?;
        Ok(())
    }

    async fn siguiente_numero(&mut self, tipo_codigo: &str, anio: i32) -> AppResult<u32> {
        self.siguiente_en_serie(&format!("comprobante:{tipo_codigo}"), anio)
            .await
    }

    async fn insertar_comprobante(&mut self, c: &Comprobante) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO comprobantes (id, tenant_id, tipo_comprobante_id, periodo_id, fecha,
                 descripcion, origen_tipo, origen_id, regla_id, revierte_a, motivo_reverso,
                 creado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.tipo_comprobante_id.as_uuid(),
            c.periodo_id.as_uuid(),
            c.fecha,
            c.descripcion,
            c.origen.as_ref().map(|o| o.tipo.as_str()),
            c.origen.as_ref().map(|o| o.id),
            c.regla_id.map(ReglaContabilizacionId::as_uuid),
            c.revierte_a.map(ComprobanteId::as_uuid),
            c.motivo_reverso,
            c.creado_por.map(CuentaId::as_uuid),
            c.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "ese documento ya tiene comprobante",
                "tipo, periodo o regla",
            )
        })?;
        self.insertar_lineas(c).await
    }

    async fn actualizar_borrador(&mut self, c: &Comprobante) -> AppResult<()> {
        sqlx::query!(
            "UPDATE comprobantes SET tipo_comprobante_id = $2, periodo_id = $3, fecha = $4,
                 descripcion = $5
             WHERE id = $1 AND estado = 'borrador'",
            c.id.as_uuid(),
            c.tipo_comprobante_id.as_uuid(),
            c.periodo_id.as_uuid(),
            c.fecha,
            c.descripcion,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "comprobante", "tipo o periodo"))?;
        sqlx::query!(
            "DELETE FROM lineas_comprobante WHERE comprobante_id = $1",
            c.id.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "comprobante", "comprobante"))?;
        self.insertar_lineas(c).await
    }

    async fn eliminar_borrador(&mut self, id: ComprobanteId) -> AppResult<()> {
        sqlx::query!(
            "DELETE FROM comprobantes WHERE id = $1 AND estado = 'borrador'",
            id.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "comprobante", "comprobante"))?;
        Ok(())
    }

    async fn marcar_contabilizado(
        &mut self,
        id: ComprobanteId,
        numero: u32,
        por: Option<CuentaId>,
        en: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE comprobantes SET estado = 'contabilizado', numero = $2, contabilizado_por = $3,
                 contabilizado_en = $4
             WHERE id = $1 AND estado = 'borrador'",
            id.as_uuid(),
            i32_de(numero, "numero")?,
            por.map(CuentaId::as_uuid),
            en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "número de comprobante repetido; reintente",
                "comprobante",
            )
        })?;
        Ok(())
    }

    async fn comprobante(&mut self, id: ComprobanteId) -> AppResult<Option<Comprobante>> {
        let filas = sqlx::query_as!(FilaComprobante, r#"SELECT k.id, k.tipo_comprobante_id, t.codigo AS tipo_codigo, k.periodo_id,
                      p.anio, p.mes, k.numero, k.fecha, k.descripcion, k.estado, k.origen_tipo,
                      k.origen_id, k.regla_id, k.revierte_a, k.motivo_reverso,
                      (SELECT r.id FROM comprobantes r WHERE r.revierte_a = k.id) AS revertido_por,
                      k.creado_por, k.creado_en, k.contabilizado_por, k.contabilizado_en
               FROM comprobantes k
               JOIN tipos_comprobante t ON t.tenant_id = k.tenant_id AND t.id = k.tipo_comprobante_id
               JOIN periodos_contables p ON p.tenant_id = k.tenant_id AND p.id = k.periodo_id WHERE k.id = $1"#, id.as_uuid())
            .fetch_all(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        Ok(self.con_sus_lineas(filas).await?.pop())
    }

    async fn comprobante_por_origen(
        &mut self,
        origen: &Origen,
    ) -> AppResult<Option<ComprobanteId>> {
        Ok(sqlx::query_scalar!(
            "SELECT id FROM comprobantes WHERE origen_tipo = $1 AND origen_id = $2",
            origen.tipo,
            origen.id
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(ComprobanteId::from_uuid))
    }

    async fn comprobantes(
        &mut self,
        f: &FiltroComprobantes,
        limite: u32,
    ) -> AppResult<Vec<Comprobante>> {
        sqlx::query_as!(
            FilaComprobante,
            r#"SELECT k.id, k.tipo_comprobante_id, t.codigo AS tipo_codigo, k.periodo_id,
                      p.anio, p.mes, k.numero, k.fecha, k.descripcion, k.estado, k.origen_tipo,
                      k.origen_id, k.regla_id, k.revierte_a, k.motivo_reverso,
                      (SELECT r.id FROM comprobantes r WHERE r.revierte_a = k.id) AS revertido_por,
                      k.creado_por, k.creado_en, k.contabilizado_por, k.contabilizado_en
               FROM comprobantes k
               JOIN tipos_comprobante t ON t.tenant_id = k.tenant_id AND t.id = k.tipo_comprobante_id
               JOIN periodos_contables p ON p.tenant_id = k.tenant_id AND p.id = k.periodo_id WHERE ($1::date IS NULL OR k.fecha >= $1) AND ($2::date IS NULL OR k.fecha <= $2)
                   AND ($3::uuid IS NULL OR k.tipo_comprobante_id = $3)
                   AND ($4::text IS NULL OR k.estado = $4)
                 ORDER BY k.fecha DESC, k.id DESC LIMIT $5"#,
            f.desde,
            f.hasta,
            f.tipo_comprobante_id.map(TipoComprobanteId::as_uuid),
            f.estado.map(EstadoComprobante::as_str),
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| f.a_comprobante(Vec::new()))
        .collect()
    }

    async fn diario(&mut self, desde: NaiveDate, hasta: NaiveDate) -> AppResult<Vec<Comprobante>> {
        let filas = sqlx::query_as!(
            FilaComprobante,
            r#"SELECT k.id, k.tipo_comprobante_id, t.codigo AS tipo_codigo, k.periodo_id,
                      p.anio, p.mes, k.numero, k.fecha, k.descripcion, k.estado, k.origen_tipo,
                      k.origen_id, k.regla_id, k.revierte_a, k.motivo_reverso,
                      (SELECT r.id FROM comprobantes r WHERE r.revierte_a = k.id) AS revertido_por,
                      k.creado_por, k.creado_en, k.contabilizado_por, k.contabilizado_en
               FROM comprobantes k
               JOIN tipos_comprobante t ON t.tenant_id = k.tenant_id AND t.id = k.tipo_comprobante_id
               JOIN periodos_contables p ON p.tenant_id = k.tenant_id AND p.id = k.periodo_id WHERE k.estado = 'contabilizado' AND k.fecha BETWEEN $1 AND $2
                 ORDER BY k.fecha, t.codigo, k.numero"#,
            desde,
            hasta
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        self.con_sus_lineas(filas).await
    }

    async fn saldos(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
        a: AgrupacionSaldos,
    ) -> AppResult<Vec<FilaSaldo>> {
        sqlx::query!(
            r#"SELECT l.cuenta_id, c.codigo, c.nombre, c.naturaleza,
                      CASE WHEN $3 THEN l.persona_id END AS persona_id,
                      CASE WHEN $4 THEN l.centro_costo_id END AS centro_costo_id,
                      coalesce(sum(l.debito - l.credito) FILTER (WHERE k.fecha < $1), 0) AS "saldo_inicial!",
                      coalesce(sum(l.debito) FILTER (WHERE k.fecha >= $1), 0) AS "debitos!",
                      coalesce(sum(l.credito) FILTER (WHERE k.fecha >= $1), 0) AS "creditos!"
               FROM lineas_comprobante l
               JOIN comprobantes k ON k.tenant_id = l.tenant_id AND k.id = l.comprobante_id
               JOIN cuentas_contables c ON c.tenant_id = l.tenant_id AND c.id = l.cuenta_id
               JOIN periodos_contables p ON p.tenant_id = k.tenant_id AND p.id = k.periodo_id
               WHERE k.estado = 'contabilizado' AND k.fecha <= $2 AND (NOT $5 OR p.mes <> 13)
               GROUP BY l.cuenta_id, c.codigo, c.nombre, c.naturaleza, 5, 6
               ORDER BY c.codigo, 5, 6"#,
            desde,
            hasta,
            a.por_tercero,
            a.por_centro_costo,
            a.sin_cierre_anual,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(FilaSaldo {
                cuenta_id: CuentaContableId::from_uuid(f.cuenta_id),
                codigo: CodigoCuenta::new(&f.codigo)?,
                nombre: f.nombre,
                naturaleza: Naturaleza::try_from(f.naturaleza.as_str())?,
                persona_id: f.persona_id.map(PersonaId::from_uuid),
                centro_costo_id: f.centro_costo_id.map(CentroCostoId::from_uuid),
                saldo_inicial: f.saldo_inicial,
                debitos: f.debitos,
                creditos: f.creditos,
            })
        })
        .collect()
    }

    async fn movimientos(
        &mut self,
        cuenta: CuentaContableId,
        persona: Option<PersonaId>,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> AppResult<Vec<Movimiento>> {
        sqlx::query!(
            r#"SELECT k.id, t.codigo AS tipo_codigo, k.numero AS "numero!", k.fecha, k.descripcion,
                      l.cuenta_id, l.persona_id, l.centro_costo_id, l.debito, l.credito
               FROM lineas_comprobante l
               JOIN comprobantes k ON k.tenant_id = l.tenant_id AND k.id = l.comprobante_id
               JOIN tipos_comprobante t ON t.tenant_id = k.tenant_id AND t.id = k.tipo_comprobante_id
               WHERE k.estado = 'contabilizado' AND l.cuenta_id = $1
                 AND ($2::uuid IS NULL OR l.persona_id = $2)
                 AND k.fecha BETWEEN $3 AND $4
               ORDER BY k.fecha, t.codigo, k.numero, l.orden"#,
            cuenta.as_uuid(),
            persona.map(PersonaId::as_uuid),
            desde,
            hasta,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(Movimiento {
                comprobante_id: ComprobanteId::from_uuid(f.id),
                tipo_codigo: f.tipo_codigo,
                numero: u32_de(f.numero, "numero")?,
                fecha: f.fecha,
                descripcion: f.descripcion,
                cuenta_id: CuentaContableId::from_uuid(f.cuenta_id),
                persona_id: f.persona_id.map(PersonaId::from_uuid),
                centro_costo_id: f.centro_costo_id.map(CentroCostoId::from_uuid),
                debito: f.debito,
                credito: f.credito,
            })
        })
        .collect()
    }

    async fn cierre_anual_vigente(&mut self, anio: i32) -> AppResult<Option<ComprobanteId>> {
        Ok(sqlx::query_scalar!(
            "SELECT k.id FROM comprobantes k
             JOIN periodos_contables p ON p.tenant_id = k.tenant_id AND p.id = k.periodo_id
             WHERE p.anio = $1 AND p.mes = 13 AND k.estado = 'contabilizado'
               AND k.origen_tipo = 'contabilidad.cierre_anual'
               AND NOT EXISTS (SELECT 1 FROM comprobantes r WHERE r.revierte_a = k.id)
             LIMIT 1",
            i16_de(anio, "anio")?
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(ComprobanteId::from_uuid))
    }
}

// ---------------------------------------------------------------------------------------------
// Plataforma: PUC y plantillas
// ---------------------------------------------------------------------------------------------

fn ambito_de(ambito: &str, organizacion: Option<Uuid>) -> AppResult<AmbitoPlantilla> {
    match (ambito, organizacion) {
        ("plataforma", None) => Ok(AmbitoPlantilla::Plataforma),
        ("organizacion", Some(o)) => {
            Ok(AmbitoPlantilla::Organizacion(OrganizacionId::from_uuid(o)))
        }
        _ => Err(AppError::infra(std::io::Error::other(
            "ámbito de plantilla incoherente",
        ))),
    }
}

fn partes(ambito: AmbitoPlantilla) -> (&'static str, Option<Uuid>) {
    match ambito {
        AmbitoPlantilla::Plataforma => ("plataforma", None),
        AmbitoPlantilla::Organizacion(o) => ("organizacion", Some(o.as_uuid())),
    }
}

impl CatalogoContablePlataforma for TransaccionPlataformaPg {
    async fn cargar_puc(&mut self, cuentas: &[CuentaPuc]) -> AppResult<u64> {
        let mut n = 0;
        for c in cuentas {
            n += sqlx::query!(
                "INSERT INTO catalogo_puc (codigo, nombre, naturaleza) VALUES ($1, $2, $3)
                 ON CONFLICT (codigo) DO UPDATE SET nombre = EXCLUDED.nombre,
                     naturaleza = EXCLUDED.naturaleza, actualizado_en = now()",
                c.codigo.as_str(),
                c.nombre.trim(),
                c.naturaleza.as_str(),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?
            .rows_affected();
        }
        Ok(n)
    }

    async fn plantillas(&mut self, ambito: AmbitoPlantilla) -> AppResult<Vec<PlantillaResumen>> {
        let (a, org) = partes(ambito);
        sqlx::query!(
            "SELECT p.id, p.ambito, o.razon_social AS \"organizacion?\", p.nombre, p.tipo_empresa,
                    p.version, p.estado, p.publicada_en
             FROM plantillas_contables p LEFT JOIN organizaciones o ON o.id = p.organizacion_id
             WHERE p.ambito = $1 AND p.organizacion_id IS NOT DISTINCT FROM $2
             ORDER BY p.tipo_empresa, p.nombre, p.version DESC",
            a,
            org
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(PlantillaResumen {
                id: PlantillaContableId::from_uuid(f.id),
                ambito: f.ambito,
                organizacion: f.organizacion,
                nombre: f.nombre,
                tipo_empresa: f.tipo_empresa,
                version: u32_de(f.version, "version")?,
                estado: EstadoPlantilla::try_from(f.estado.as_str())?,
                publicada_en: f.publicada_en,
            })
        })
        .collect()
    }

    async fn plantilla(&mut self, id: PlantillaContableId) -> AppResult<Option<PlantillaGuardada>> {
        sqlx::query!(
            "SELECT id, ambito, organizacion_id, nombre, tipo_empresa, version, estado, contenido,
                    autor_cuenta_id, creada_en, publicada_en
             FROM plantillas_contables WHERE id = $1",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| {
            Ok(PlantillaGuardada {
                id: PlantillaContableId::from_uuid(f.id),
                ambito: ambito_de(&f.ambito, f.organizacion_id)?,
                nombre: f.nombre,
                tipo_empresa: f.tipo_empresa,
                version: u32_de(f.version, "version")?,
                estado: EstadoPlantilla::try_from(f.estado.as_str())?,
                contenido: f.contenido,
                autor: f.autor_cuenta_id.map(CuentaId::from_uuid),
                creada_en: f.creada_en,
                publicada_en: f.publicada_en,
            })
        })
        .transpose()
    }

    async fn ultima_version_plantilla(
        &mut self,
        ambito: AmbitoPlantilla,
        nombre: &str,
    ) -> AppResult<u32> {
        let (a, org) = partes(ambito);
        let v = sqlx::query_scalar!(
            r#"SELECT coalesce(max(version), 0) AS "v!" FROM plantillas_contables
               WHERE ambito = $1 AND organizacion_id IS NOT DISTINCT FROM $2 AND nombre = $3"#,
            a,
            org,
            nombre
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        u32_de(v, "version")
    }

    async fn insertar_plantilla(&mut self, p: &PlantillaGuardada) -> AppResult<()> {
        let (a, org) = partes(p.ambito);
        sqlx::query!(
            "INSERT INTO plantillas_contables (id, ambito, organizacion_id, nombre, tipo_empresa,
                 version, estado, contenido, autor_cuenta_id, creada_en, publicada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            p.id.as_uuid(),
            a,
            org,
            p.nombre,
            p.tipo_empresa,
            i32_de(p.version, "version")?,
            p.estado.as_str(),
            p.contenido,
            p.autor.map(CuentaId::as_uuid),
            p.creada_en,
            p.publicada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "otra versión de la plantilla se guardó al tiempo; reintente",
                "organización",
            )
        })?;
        Ok(())
    }

    async fn actualizar_contenido_plantilla(
        &mut self,
        id: PlantillaContableId,
        contenido: &serde_json::Value,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE plantillas_contables SET contenido = $2 WHERE id = $1 AND estado = 'borrador'",
            id.as_uuid(),
            contenido
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn cambiar_estado_plantilla(
        &mut self,
        id: PlantillaContableId,
        estado: EstadoPlantilla,
        en: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE plantillas_contables
             SET estado = $2, publicada_en = CASE WHEN $2 = 'publicada' THEN $3 ELSE publicada_en END
             WHERE id = $1",
            id.as_uuid(),
            estado.as_str(),
            en
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn gestiona_organizacion(
        &mut self,
        organizacion: OrganizacionId,
        cuenta: CuentaId,
    ) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM organizaciones_miembros
                              WHERE organizacion_id = $1 AND cuenta_id = $2
                                AND rol IN ('dueno', 'gestor')) AS "e!""#,
            organizacion.as_uuid(),
            cuenta.as_uuid()
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }
}
