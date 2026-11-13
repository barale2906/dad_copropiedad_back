//! Cartera de la PH sobre PostgreSQL (plan 14, ADR 0019). La base de datos impide liquidar dos
//! veces el mismo periodo u obligación (`ux_liquidacion_periodo`, `ux_obligacion_liquidada`),
//! deja intactos los rubros de un presupuesto aprobado y las cuentas de cobro, y el saldo de
//! cada obligación se calcula aquí sumando ajustes y aplicaciones vigentes.

use std::collections::BTreeMap;

use application::{
    AppError, AppResult,
    puertos::cartera::{
        AcuerdoPago, AjusteObligacion, Anticipo, BienCartera, CargoPendiente, CatalogoTasas,
        CatalogoTasasPlataforma, CausacionMora, CertificadoCartera, ConceptoCobro, CuentaCobro,
        CuotaExtraordinaria, DetalleInteres, DirectorioCartera, FiltroObligaciones,
        LiquidacionExpensas, ModuloContribucion, ObligacionCobro, PagoAplicado, PoliticaCartera,
        Presupuesto, RepositorioCartera, ResumenBien, ResumenCartera, RubroPresupuesto,
        SaldoObligacion, TasaReferencia,
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AcuerdoPagoId, AjusteObligacionId, BienPrivadoId, CausacionMoraId, CentroCostoId,
    CertificadoCarteraId, CodigoBienPrivado, ComprobanteId, ConceptoCobroId, CuentaCobroId,
    CuentaContableId, CuentaId, CuotaExtraordinariaId, LiquidacionExpensasId, ModuloContribucionId,
    ObligacionCobroId, PersonaId, PoliticaCarteraId, PreguntaId, PresupuestoId, ReciboCajaId,
    RubroPresupuestoId, TasaReferenciaId, TipoBienPrivado,
    contabilidad::Monto,
    contabilidad_ph::{
        CriterioDistribucion, EstadoCuotaExtraordinaria, EstadoLiquidacionExpensas,
        EstadoObligacion, EstadoPresupuesto, OrdenImputacion, OrigenObligacion, ParametrosPolitica,
        TipoAjuste, TipoAvisoCartera, TipoConceptoCobro,
        mora::{
            CuotaAcuerdo, EstadoAcuerdo, EstadoCausacion, ModalidadTasa, Reduccion, TipoCertificado,
        },
    },
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{
    TransaccionPg, TransaccionPlataformaPg,
    contabilidad::{error_contable, i16_de, u32_de},
};

fn monto(v: Decimal) -> AppResult<Monto> {
    Ok(Monto::new(v)?)
}

fn u8_de(v: i16, campo: &'static str) -> AppResult<u8> {
    u8::try_from(v).map_err(|_| AppError::Validacion(format!("`{campo}` fuera de rango")))
}

fn i32_de(v: u32, campo: &'static str) -> AppResult<i32> {
    i32::try_from(v).map_err(|_| AppError::Validacion(format!("`{campo}` fuera de rango")))
}

// ---------------------------------------------------------------------------------------------
// Filas
// ---------------------------------------------------------------------------------------------

struct FilaConcepto {
    id: Uuid,
    codigo: String,
    nombre: String,
    tipo: String,
    criterio: String,
    modulo_id: Option<Uuid>,
    valor_fijo: Option<Decimal>,
    genera_interes: bool,
    concepto_contable: String,
    activo: bool,
    creado_en: DateTime<Utc>,
}

struct FilaPolitica {
    id: Uuid,
    version: i32,
    vigente_desde: NaiveDate,
    dia_vencimiento: i16,
    redondeo: Decimal,
    pronto_pago_porcentaje: Option<Decimal>,
    pronto_pago_dia: Option<i16>,
    factor_mora: Decimal,
    tasa_mora_mensual: Option<Decimal>,
    imputacion: String,
    permite_elegir_obligacion: bool,
    porcentaje_fondo: Decimal,
    fondo_suspendido: bool,
    pregunta_id: Option<Uuid>,
    creada_por: Option<Uuid>,
    creada_en: DateTime<Utc>,
}

impl TryFrom<FilaPolitica> for PoliticaCartera {
    type Error = AppError;

    fn try_from(f: FilaPolitica) -> AppResult<Self> {
        let pronto_pago = match (f.pronto_pago_porcentaje, f.pronto_pago_dia) {
            (Some(p), Some(d)) => Some((p, u8_de(d, "pronto_pago_dia")?)),
            _ => None,
        };
        Ok(Self {
            id: PoliticaCarteraId::from_uuid(f.id),
            version: u32_de(f.version, "version")?,
            vigente_desde: f.vigente_desde,
            parametros: ParametrosPolitica {
                dia_vencimiento: u8_de(f.dia_vencimiento, "dia_vencimiento")?,
                redondeo: f.redondeo,
                pronto_pago,
                factor_mora: f.factor_mora,
                tasa_mora_mensual: f.tasa_mora_mensual,
                imputacion: OrdenImputacion::try_from(f.imputacion.as_str())?,
                permite_elegir_obligacion: f.permite_elegir_obligacion,
                porcentaje_fondo: f.porcentaje_fondo,
                fondo_suspendido: f.fondo_suspendido,
            },
            pregunta_id: f.pregunta_id.map(PreguntaId::from_uuid),
            creada_por: f.creada_por.map(CuentaId::from_uuid),
            creada_en: f.creada_en,
        })
    }
}

struct FilaPresupuesto {
    id: Uuid,
    anio: i16,
    version: i32,
    estado: String,
    descripcion: Option<String>,
    pregunta_id: Option<Uuid>,
    vigente_desde: Option<NaiveDate>,
    aprobado_por: Option<Uuid>,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
}

struct FilaRubro {
    id: Uuid,
    presupuesto_id: Uuid,
    concepto_cobro_id: Uuid,
    cuenta_contable_id: Uuid,
    centro_costo_id: Option<Uuid>,
    nombre: String,
    valores_mensuales: Vec<Decimal>,
}

struct FilaCuota {
    id: Uuid,
    concepto_cobro_id: Uuid,
    finalidad: String,
    valor_total: Decimal,
    numero_cuotas: i16,
    primer_periodo: NaiveDate,
    destinacion_especifica: bool,
    pregunta_id: Option<Uuid>,
    estado: String,
    creada_por: Option<Uuid>,
    creada_en: DateTime<Utc>,
}

impl TryFrom<FilaCuota> for CuotaExtraordinaria {
    type Error = AppError;

    fn try_from(f: FilaCuota) -> AppResult<Self> {
        Ok(Self {
            id: CuotaExtraordinariaId::from_uuid(f.id),
            concepto_cobro_id: ConceptoCobroId::from_uuid(f.concepto_cobro_id),
            finalidad: f.finalidad,
            valor_total: monto(f.valor_total)?,
            numero_cuotas: u16::try_from(f.numero_cuotas).unwrap_or(0),
            primer_periodo: f.primer_periodo,
            destinacion_especifica: f.destinacion_especifica,
            pregunta_id: f.pregunta_id.map(PreguntaId::from_uuid),
            estado: EstadoCuotaExtraordinaria::try_from(f.estado.as_str())?,
            creada_por: f.creada_por.map(CuentaId::from_uuid),
            creada_en: f.creada_en,
        })
    }
}

struct FilaLiquidacion {
    id: Uuid,
    periodo: NaiveDate,
    estado: String,
    presupuesto_id: Option<Uuid>,
    politica_id: Uuid,
    total: Decimal,
    vencimiento: NaiveDate,
    comprobante_id: Uuid,
    motivo_anulacion: Option<String>,
    liquidada_por: Option<Uuid>,
    liquidada_en: DateTime<Utc>,
    anulada_en: Option<DateTime<Utc>>,
}

impl TryFrom<FilaLiquidacion> for LiquidacionExpensas {
    type Error = AppError;

    fn try_from(f: FilaLiquidacion) -> AppResult<Self> {
        Ok(Self {
            id: LiquidacionExpensasId::from_uuid(f.id),
            periodo: f.periodo,
            estado: EstadoLiquidacionExpensas::try_from(f.estado.as_str())?,
            presupuesto_id: f.presupuesto_id.map(PresupuestoId::from_uuid),
            politica_id: PoliticaCarteraId::from_uuid(f.politica_id),
            total: monto(f.total)?,
            vencimiento: f.vencimiento,
            comprobante_id: ComprobanteId::from_uuid(f.comprobante_id),
            motivo_anulacion: f.motivo_anulacion,
            liquidada_por: f.liquidada_por.map(CuentaId::from_uuid),
            liquidada_en: f.liquidada_en,
            anulada_en: f.anulada_en,
        })
    }
}

struct FilaSaldo {
    id: Uuid,
    bien_privado_id: Uuid,
    concepto_cobro_id: Uuid,
    periodo: NaiveDate,
    valor: Decimal,
    vencimiento: NaiveDate,
    deudor_persona_id: Uuid,
    origen: String,
    liquidacion_id: Option<Uuid>,
    cuota_extraordinaria_id: Option<Uuid>,
    cuenta_cobro_id: Option<Uuid>,
    comprobante_id: Option<Uuid>,
    causacion_id: Option<Uuid>,
    descripcion: Option<String>,
    estado: String,
    creada_en: DateTime<Utc>,
    anulada_en: Option<DateTime<Utc>>,
    concepto_codigo: String,
    concepto_nombre: String,
    concepto_tipo: String,
    concepto_contable: String,
    ajustes: Decimal,
    aplicado: Decimal,
}

impl TryFrom<FilaSaldo> for SaldoObligacion {
    type Error = AppError;

    fn try_from(f: FilaSaldo) -> AppResult<Self> {
        let estado = EstadoObligacion::try_from(f.estado.as_str())?;
        let saldo = match estado {
            EstadoObligacion::Vigente => f.valor - f.ajustes - f.aplicado,
            EstadoObligacion::Anulada => Decimal::ZERO,
        };
        Ok(Self {
            obligacion: ObligacionCobro {
                id: ObligacionCobroId::from_uuid(f.id),
                bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
                concepto_cobro_id: ConceptoCobroId::from_uuid(f.concepto_cobro_id),
                periodo: f.periodo,
                valor: monto(f.valor)?,
                vencimiento: f.vencimiento,
                deudor_persona_id: PersonaId::from_uuid(f.deudor_persona_id),
                origen: OrigenObligacion::try_from(f.origen.as_str())?,
                liquidacion_id: f.liquidacion_id.map(LiquidacionExpensasId::from_uuid),
                cuota_extraordinaria_id: f
                    .cuota_extraordinaria_id
                    .map(CuotaExtraordinariaId::from_uuid),
                cuenta_cobro_id: f.cuenta_cobro_id.map(CuentaCobroId::from_uuid),
                comprobante_id: f.comprobante_id.map(ComprobanteId::from_uuid),
                causacion_id: f.causacion_id.map(CausacionMoraId::from_uuid),
                descripcion: f.descripcion,
                estado,
                creada_en: f.creada_en,
                anulada_en: f.anulada_en,
            },
            concepto_codigo: f.concepto_codigo,
            concepto_nombre: f.concepto_nombre,
            concepto_tipo: TipoConceptoCobro::try_from(f.concepto_tipo.as_str())?,
            concepto_contable: f.concepto_contable,
            ajustes: f.ajustes,
            aplicado: f.aplicado,
            saldo,
        })
    }
}

struct FilaCuentaCobro {
    id: Uuid,
    anio: i16,
    numero: i32,
    liquidacion_id: Uuid,
    bien_privado_id: Uuid,
    periodo: NaiveDate,
    deudor_persona_id: Uuid,
    saldo_anterior: Decimal,
    cargos: Decimal,
    total: Decimal,
    vencimiento: NaiveDate,
    pronto_pago_hasta: Option<NaiveDate>,
    descuento_pronto_pago: Option<Decimal>,
    referencia_pago: Option<String>,
    creada_en: DateTime<Utc>,
}

impl TryFrom<FilaCuentaCobro> for CuentaCobro {
    type Error = AppError;

    fn try_from(f: FilaCuentaCobro) -> AppResult<Self> {
        Ok(Self {
            id: CuentaCobroId::from_uuid(f.id),
            anio: i32::from(f.anio),
            numero: u32_de(f.numero, "numero")?,
            liquidacion_id: LiquidacionExpensasId::from_uuid(f.liquidacion_id),
            bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
            periodo: f.periodo,
            deudor_persona_id: PersonaId::from_uuid(f.deudor_persona_id),
            saldo_anterior: f.saldo_anterior,
            cargos: monto(f.cargos)?,
            total: f.total,
            vencimiento: f.vencimiento,
            pronto_pago_hasta: f.pronto_pago_hasta,
            descuento_pronto_pago: f.descuento_pronto_pago.map(monto).transpose()?,
            referencia_pago: f.referencia_pago,
            creada_en: f.creada_en,
        })
    }
}

struct FilaAjuste {
    id: Uuid,
    obligacion_id: Uuid,
    tipo: String,
    valor: Decimal,
    motivo: String,
    comprobante_id: Uuid,
    recibo_id: Option<Uuid>,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
    anulado_en: Option<DateTime<Utc>>,
}

impl TryFrom<FilaAjuste> for AjusteObligacion {
    type Error = AppError;

    fn try_from(f: FilaAjuste) -> AppResult<Self> {
        Ok(Self {
            id: AjusteObligacionId::from_uuid(f.id),
            obligacion_id: ObligacionCobroId::from_uuid(f.obligacion_id),
            tipo: TipoAjuste::try_from(f.tipo.as_str())?,
            valor: monto(f.valor)?,
            motivo: f.motivo,
            comprobante_id: ComprobanteId::from_uuid(f.comprobante_id),
            recibo_id: f.recibo_id.map(ReciboCajaId::from_uuid),
            creado_por: f.creado_por.map(CuentaId::from_uuid),
            creado_en: f.creado_en,
            anulado_en: f.anulado_en,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Consultas compartidas
// ---------------------------------------------------------------------------------------------

impl TransaccionPg {
    async fn presupuestos_donde(
        &mut self,
        id: Option<Uuid>,
        anio: Option<i16>,
        solo_vigente: bool,
    ) -> AppResult<Vec<Presupuesto>> {
        let filas = sqlx::query_as!(
            FilaPresupuesto,
            "SELECT id, anio, version, estado, descripcion, pregunta_id, vigente_desde,
                    aprobado_por, creado_por, creado_en
             FROM presupuestos
             WHERE ($1::uuid IS NULL OR id = $1) AND ($2::smallint IS NULL OR anio = $2)
               AND (NOT $3 OR estado = 'vigente')
             ORDER BY anio DESC, version DESC",
            id,
            anio,
            solo_vigente,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let ids: Vec<Uuid> = filas.iter().map(|f| f.id).collect();
        let rubros = sqlx::query_as!(
            FilaRubro,
            r#"SELECT id, presupuesto_id, concepto_cobro_id, cuenta_contable_id, centro_costo_id,
                      nombre, valores_mensuales AS "valores_mensuales!: Vec<Decimal>"
               FROM rubros_presupuesto WHERE presupuesto_id = ANY($1) ORDER BY nombre, id"#,
            &ids,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let mut por_presupuesto: BTreeMap<Uuid, Vec<RubroPresupuesto>> = BTreeMap::new();
        for r in rubros {
            let valores: Vec<Monto> = r
                .valores_mensuales
                .into_iter()
                .map(monto)
                .collect::<AppResult<_>>()?;
            por_presupuesto
                .entry(r.presupuesto_id)
                .or_default()
                .push(RubroPresupuesto {
                    id: RubroPresupuestoId::from_uuid(r.id),
                    concepto_cobro_id: ConceptoCobroId::from_uuid(r.concepto_cobro_id),
                    cuenta_contable_id: CuentaContableId::from_uuid(r.cuenta_contable_id),
                    centro_costo_id: r.centro_costo_id.map(CentroCostoId::from_uuid),
                    nombre: r.nombre,
                    valores_mensuales: domain::contabilidad_ph::valores_mensuales(&valores)?,
                });
        }
        filas
            .into_iter()
            .map(|f| {
                Ok(Presupuesto {
                    id: PresupuestoId::from_uuid(f.id),
                    anio: i32::from(f.anio),
                    version: u32_de(f.version, "version")?,
                    estado: EstadoPresupuesto::try_from(f.estado.as_str())?,
                    descripcion: f.descripcion,
                    pregunta_id: f.pregunta_id.map(PreguntaId::from_uuid),
                    vigente_desde: f.vigente_desde,
                    aprobado_por: f.aprobado_por.map(CuentaId::from_uuid),
                    creado_por: f.creado_por.map(CuentaId::from_uuid),
                    creado_en: f.creado_en,
                    rubros: por_presupuesto.remove(&f.id).unwrap_or_default(),
                })
            })
            .collect()
    }

    async fn insertar_rubros(
        &mut self,
        id: PresupuestoId,
        rubros: &[RubroPresupuesto],
    ) -> AppResult<()> {
        for r in rubros {
            let valores: Vec<Decimal> = r.valores_mensuales.iter().map(|v| v.valor()).collect();
            sqlx::query!(
                "INSERT INTO rubros_presupuesto (id, tenant_id, presupuesto_id, concepto_cobro_id,
                     cuenta_contable_id, centro_costo_id, nombre, valores_mensuales)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                r.id.as_uuid(),
                self.ctx.tenant_id.as_uuid(),
                id.as_uuid(),
                r.concepto_cobro_id.as_uuid(),
                r.cuenta_contable_id.as_uuid(),
                r.centro_costo_id.map(CentroCostoId::as_uuid),
                r.nombre,
                &valores,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| {
                error_contable(e, "rubro repetido", "concepto, cuenta o centro de costo")
            })?;
        }
        Ok(())
    }

    async fn liquidaciones_donde(
        &mut self,
        id: Option<Uuid>,
        periodo: Option<NaiveDate>,
        limite: i64,
    ) -> AppResult<Vec<LiquidacionExpensas>> {
        sqlx::query_as!(
            FilaLiquidacion,
            "SELECT id, periodo, estado, presupuesto_id, politica_id, total, vencimiento,
                    comprobante_id, motivo_anulacion, liquidada_por, liquidada_en, anulada_en
             FROM liquidaciones_expensas
             WHERE ($1::uuid IS NULL OR id = $1)
               AND ($2::date IS NULL OR (periodo = $2 AND estado = 'contabilizada'))
             ORDER BY periodo DESC, liquidada_en DESC
             LIMIT $3",
            id,
            periodo,
            limite,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(LiquidacionExpensas::try_from)
        .collect()
    }

    async fn cuentas_cobro_donde(
        &mut self,
        id: Option<Uuid>,
        liquidacion: Option<Uuid>,
        bien: Option<Uuid>,
    ) -> AppResult<Vec<CuentaCobro>> {
        sqlx::query_as!(
            FilaCuentaCobro,
            "SELECT id, anio, numero, liquidacion_id, bien_privado_id, periodo, deudor_persona_id,
                    saldo_anterior, cargos, total, vencimiento, pronto_pago_hasta,
                    descuento_pronto_pago, referencia_pago, creada_en
             FROM cuentas_cobro
             WHERE ($1::uuid IS NULL OR id = $1) AND ($2::uuid IS NULL OR liquidacion_id = $2)
               AND ($3::uuid IS NULL OR bien_privado_id = $3)
             ORDER BY anio, numero",
            id,
            liquidacion,
            bien,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(CuentaCobro::try_from)
        .collect()
    }

    async fn saldos_donde(
        &mut self,
        id: Option<Uuid>,
        f: &FiltroObligaciones,
    ) -> AppResult<Vec<SaldoObligacion>> {
        sqlx::query_as!(
            FilaSaldo,
            r#"SELECT o.id, o.bien_privado_id, o.concepto_cobro_id, o.periodo, o.valor,
                      o.vencimiento, o.deudor_persona_id, o.origen, o.liquidacion_id,
                      o.cuota_extraordinaria_id, o.cuenta_cobro_id, o.comprobante_id,
                      o.causacion_id, o.descripcion, o.estado, o.creada_en, o.anulada_en,
                      c.codigo AS concepto_codigo, c.nombre AS concepto_nombre,
                      c.tipo AS concepto_tipo, c.concepto_contable,
                      coalesce(aj.total, 0) AS "ajustes!", coalesce(ap.total, 0) AS "aplicado!"
               FROM obligaciones o
               JOIN conceptos_cobro c ON c.id = o.concepto_cobro_id
               LEFT JOIN LATERAL (SELECT sum(a.valor) AS total FROM ajustes_obligacion a
                                  WHERE a.obligacion_id = o.id AND a.anulado_en IS NULL) aj ON TRUE
               LEFT JOIN LATERAL (SELECT sum(p.valor) AS total FROM aplicaciones_pago p
                                  WHERE p.obligacion_id = o.id AND p.anulada_en IS NULL) ap ON TRUE
               WHERE ($1::uuid IS NULL OR o.id = $1)
                 AND ($2::uuid IS NULL OR o.bien_privado_id = $2)
                 AND ($3::uuid IS NULL OR o.liquidacion_id = $3)
                 AND ($4::uuid IS NULL OR o.cuenta_cobro_id = $4)
                 AND ($5::date IS NULL OR o.periodo <= $5)
                 AND (NOT $6 OR (o.estado = 'vigente'
                                 AND o.valor - coalesce(aj.total, 0) - coalesce(ap.total, 0) > 0))
               ORDER BY o.bien_privado_id, o.periodo, o.creada_en, o.id"#,
            id,
            f.bien.map(BienPrivadoId::as_uuid),
            f.liquidacion.map(LiquidacionExpensasId::as_uuid),
            f.cuenta_cobro.map(CuentaCobroId::as_uuid),
            f.hasta_periodo,
            f.con_saldo,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(SaldoObligacion::try_from)
        .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// Puerto
// ---------------------------------------------------------------------------------------------

impl RepositorioCartera for TransaccionPg {
    async fn modulos_contribucion(&mut self) -> AppResult<Vec<ModuloContribucion>> {
        Ok(sqlx::query!(
            "SELECT id, nombre, descripcion, activo, creado_en FROM modulos_contribucion
             ORDER BY normalizar(nombre), id"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| ModuloContribucion {
            id: ModuloContribucionId::from_uuid(f.id),
            nombre: f.nombre,
            descripcion: f.descripcion,
            activo: f.activo,
            creado_en: f.creado_en,
        })
        .collect())
    }

    async fn guardar_modulo_contribucion(&mut self, m: &ModuloContribucion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO modulos_contribucion (id, tenant_id, nombre, descripcion, activo, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (id) DO UPDATE
             SET nombre = EXCLUDED.nombre, descripcion = EXCLUDED.descripcion,
                 activo = EXCLUDED.activo",
            m.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            m.nombre,
            m.descripcion,
            m.activo,
            m.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "ya existe un módulo con ese nombre", "copropiedad"))?;
        Ok(())
    }

    async fn conceptos_cobro(&mut self) -> AppResult<Vec<ConceptoCobro>> {
        let filas = sqlx::query_as!(
            FilaConcepto,
            "SELECT id, codigo, nombre, tipo, criterio, modulo_id, valor_fijo, genera_interes,
                    concepto_contable, activo, creado_en
             FROM conceptos_cobro ORDER BY codigo"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let tarifas = sqlx::query!(
            "SELECT concepto_id, tipo_bien, valor FROM tarifas_concepto_cobro
             ORDER BY concepto_id, tipo_bien"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let mut por_concepto: BTreeMap<Uuid, Vec<(TipoBienPrivado, Monto)>> = BTreeMap::new();
        for t in tarifas {
            por_concepto.entry(t.concepto_id).or_default().push((
                TipoBienPrivado::try_from(t.tipo_bien.as_str())?,
                monto(t.valor)?,
            ));
        }
        filas
            .into_iter()
            .map(|f| {
                Ok(ConceptoCobro {
                    id: ConceptoCobroId::from_uuid(f.id),
                    codigo: f.codigo,
                    nombre: f.nombre,
                    tipo: TipoConceptoCobro::try_from(f.tipo.as_str())?,
                    criterio: CriterioDistribucion::try_from(f.criterio.as_str())?,
                    modulo_id: f.modulo_id.map(ModuloContribucionId::from_uuid),
                    valor_fijo: f.valor_fijo.map(monto).transpose()?,
                    tarifas: por_concepto.remove(&f.id).unwrap_or_default(),
                    genera_interes: f.genera_interes,
                    concepto_contable: f.concepto_contable,
                    activo: f.activo,
                    creado_en: f.creado_en,
                })
            })
            .collect()
    }

    async fn guardar_concepto_cobro(&mut self, c: &ConceptoCobro) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO conceptos_cobro (id, tenant_id, codigo, nombre, tipo, criterio, modulo_id,
                 valor_fijo, genera_interes, concepto_contable, activo, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (id) DO UPDATE
             SET nombre = EXCLUDED.nombre, valor_fijo = EXCLUDED.valor_fijo,
                 genera_interes = EXCLUDED.genera_interes,
                 concepto_contable = EXCLUDED.concepto_contable, activo = EXCLUDED.activo",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.codigo,
            c.nombre,
            c.tipo.as_str(),
            c.criterio.as_str(),
            c.modulo_id.map(ModuloContribucionId::as_uuid),
            c.valor_fijo.map(Monto::valor),
            c.genera_interes,
            c.concepto_contable,
            c.activo,
            c.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "ya existe un concepto con ese código", "módulo"))?;
        sqlx::query!(
            "DELETE FROM tarifas_concepto_cobro WHERE concepto_id = $1",
            c.id.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let tipos: Vec<String> = c
            .tarifas
            .iter()
            .map(|(t, _)| t.as_str().to_owned())
            .collect();
        let valores: Vec<Decimal> = c.tarifas.iter().map(|(_, v)| v.valor()).collect();
        sqlx::query!(
            "INSERT INTO tarifas_concepto_cobro (tenant_id, concepto_id, tipo_bien, valor)
             SELECT $1, $2, t, v FROM unnest($3::text[], $4::numeric[]) AS x (t, v)",
            self.ctx.tenant_id.as_uuid(),
            c.id.as_uuid(),
            &tipos,
            &valores,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "tarifa repetida", "concepto de cobro"))?;
        Ok(())
    }

    async fn politicas_cartera(&mut self) -> AppResult<Vec<PoliticaCartera>> {
        sqlx::query_as!(
            FilaPolitica,
            "SELECT id, version, vigente_desde, dia_vencimiento, redondeo, pronto_pago_porcentaje,
                    pronto_pago_dia, factor_mora, tasa_mora_mensual, imputacion,
                    permite_elegir_obligacion, porcentaje_fondo, fondo_suspendido, pregunta_id,
                    creada_por, creada_en
             FROM politicas_cartera ORDER BY version DESC"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(PoliticaCartera::try_from)
        .collect()
    }

    async fn insertar_politica(&mut self, p: &PoliticaCartera) -> AppResult<()> {
        let x = &p.parametros;
        sqlx::query!(
            "INSERT INTO politicas_cartera (id, tenant_id, version, vigente_desde, dia_vencimiento,
                 redondeo, pronto_pago_porcentaje, pronto_pago_dia, factor_mora, tasa_mora_mensual,
                 imputacion, permite_elegir_obligacion, porcentaje_fondo, fondo_suspendido,
                 pregunta_id, creada_por, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
            p.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            i32_de(p.version, "version")?,
            p.vigente_desde,
            i16::from(x.dia_vencimiento),
            x.redondeo,
            x.pronto_pago.map(|p| p.0),
            x.pronto_pago.map(|p| i16::from(p.1)),
            x.factor_mora,
            x.tasa_mora_mensual,
            x.imputacion.as_str(),
            x.permite_elegir_obligacion,
            x.porcentaje_fondo,
            x.fondo_suspendido,
            p.pregunta_id.map(PreguntaId::as_uuid),
            p.creada_por.map(CuentaId::as_uuid),
            p.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "ya hay una versión de la política vigente desde esa fecha",
                "pregunta de la asamblea",
            )
        })?;
        Ok(())
    }

    async fn bienes_cartera(&mut self) -> AppResult<Vec<BienCartera>> {
        sqlx::query!(
            r#"SELECT b.id, b.codigo, b.tipo, b.area_privada_m2, b.activo, b.referencia_pago,
                      b.pagador_persona_id,
                      (SELECT t.persona_id FROM titularidades t
                       WHERE t.bien_privado_id = b.id AND t.es_representante
                         AND t.vigente_hasta IS NULL
                       LIMIT 1) AS deudor_persona_id
               FROM bienes_privados b ORDER BY normalizar(b.codigo), b.id"#
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(BienCartera {
                id: BienPrivadoId::from_uuid(f.id),
                codigo: CodigoBienPrivado::new(&f.codigo)?,
                tipo: TipoBienPrivado::try_from(f.tipo.as_str())?,
                area_m2: f.area_privada_m2,
                activo: f.activo,
                referencia_pago: f.referencia_pago,
                pagador_persona_id: f.pagador_persona_id.map(PersonaId::from_uuid),
                deudor_persona_id: f.deudor_persona_id.map(PersonaId::from_uuid),
            })
        })
        .collect()
    }

    async fn guardar_datos_pago(
        &mut self,
        bien: BienPrivadoId,
        referencia: Option<&str>,
        pagador: Option<PersonaId>,
    ) -> AppResult<bool> {
        let r = sqlx::query!(
            "UPDATE bienes_privados SET referencia_pago = $2, pagador_persona_id = $3
             WHERE id = $1",
            bien.as_uuid(),
            referencia,
            pagador.map(PersonaId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "otro bien ya usa esa referencia de pago", "persona"))?;
        Ok(r.rows_affected() > 0)
    }

    async fn coeficientes_en(
        &mut self,
        modulo: Option<ModuloContribucionId>,
        momento: DateTime<Utc>,
    ) -> AppResult<Option<BTreeMap<BienPrivadoId, Decimal>>> {
        let filas = sqlx::query!(
            "SELECT c.bien_privado_id, c.coeficiente FROM coeficientes_bien_privado c
             WHERE c.cuadro_id = (
                 SELECT q.id FROM cuadros_coeficientes q
                 WHERE q.estado IN ('vigente', 'historico')
                   AND q.modulo_id IS NOT DISTINCT FROM $1 AND q.vigente_desde <= $2
                 ORDER BY q.vigente_desde DESC LIMIT 1)",
            modulo.map(ModuloContribucionId::as_uuid),
            momento,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok((!filas.is_empty()).then(|| {
            filas
                .into_iter()
                .map(|f| (BienPrivadoId::from_uuid(f.bien_privado_id), f.coeficiente))
                .collect()
        }))
    }

    async fn presupuestos(&mut self, anio: Option<i32>) -> AppResult<Vec<Presupuesto>> {
        let anio = anio.map(|a| i16_de(a, "anio")).transpose()?;
        self.presupuestos_donde(None, anio, false).await
    }

    async fn presupuesto(&mut self, id: PresupuestoId) -> AppResult<Option<Presupuesto>> {
        sqlx::query!(
            "SELECT id FROM presupuestos WHERE id = $1 FOR UPDATE",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(self
            .presupuestos_donde(Some(id.as_uuid()), None, false)
            .await?
            .pop())
    }

    async fn presupuesto_vigente(&mut self, anio: i32) -> AppResult<Option<Presupuesto>> {
        Ok(self
            .presupuestos_donde(None, Some(i16_de(anio, "anio")?), true)
            .await?
            .pop())
    }

    async fn ultima_version_presupuesto(&mut self, anio: i32) -> AppResult<u32> {
        let v = sqlx::query_scalar!(
            r#"SELECT coalesce(max(version), 0) AS "v!" FROM presupuestos WHERE anio = $1"#,
            i16_de(anio, "anio")?,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        u32_de(v, "version")
    }

    async fn insertar_presupuesto(&mut self, p: &Presupuesto) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO presupuestos (id, tenant_id, anio, version, estado, descripcion,
                 creado_por, creado_en)
             VALUES ($1, $2, $3, $4, 'borrador', $5, $6, $7)",
            p.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            i16_de(p.anio, "anio")?,
            i32_de(p.version, "version")?,
            p.descripcion,
            p.creado_por.map(CuentaId::as_uuid),
            p.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "ya existe esa versión del presupuesto", "copropiedad"))?;
        self.insertar_rubros(p.id, &p.rubros).await
    }

    async fn reemplazar_rubros_presupuesto(
        &mut self,
        id: PresupuestoId,
        rubros: &[RubroPresupuesto],
    ) -> AppResult<()> {
        sqlx::query!(
            "DELETE FROM rubros_presupuesto WHERE presupuesto_id = $1",
            id.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        self.insertar_rubros(id, rubros).await
    }

    async fn aprobar_presupuesto(
        &mut self,
        p: &Presupuesto,
        anterior: Option<PresupuestoId>,
    ) -> AppResult<()> {
        if let Some(a) = anterior {
            sqlx::query!(
                "UPDATE presupuestos SET estado = 'reemplazado' WHERE id = $1",
                a.as_uuid()
            )
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        }
        sqlx::query!(
            "UPDATE presupuestos
             SET estado = 'vigente', vigente_desde = $2, pregunta_id = $3, aprobado_por = $4
             WHERE id = $1",
            p.id.as_uuid(),
            p.vigente_desde,
            p.pregunta_id.map(PreguntaId::as_uuid),
            p.aprobado_por.map(CuentaId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "ya hay otro presupuesto vigente",
                "pregunta de la asamblea",
            )
        })?;
        Ok(())
    }

    async fn cuotas_extraordinarias(&mut self) -> AppResult<Vec<CuotaExtraordinaria>> {
        sqlx::query_as!(
            FilaCuota,
            "SELECT id, concepto_cobro_id, finalidad, valor_total, numero_cuotas, primer_periodo,
                    destinacion_especifica, pregunta_id, estado, creada_por, creada_en
             FROM cuotas_extraordinarias ORDER BY creada_en DESC, id"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(CuotaExtraordinaria::try_from)
        .collect()
    }

    async fn insertar_cuota_extraordinaria(&mut self, c: &CuotaExtraordinaria) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO cuotas_extraordinarias (id, tenant_id, concepto_cobro_id, finalidad,
                 valor_total, numero_cuotas, primer_periodo, destinacion_especifica, pregunta_id,
                 estado, creada_por, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'vigente', $10, $11)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.concepto_cobro_id.as_uuid(),
            c.finalidad,
            c.valor_total.valor(),
            i16_de(c.numero_cuotas, "numero_cuotas")?,
            c.primer_periodo,
            c.destinacion_especifica,
            c.pregunta_id.map(PreguntaId::as_uuid),
            c.creada_por.map(CuentaId::as_uuid),
            c.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "cuota repetida", "concepto o pregunta"))?;
        Ok(())
    }

    async fn anular_cuota_extraordinaria(&mut self, id: CuotaExtraordinariaId) -> AppResult<bool> {
        let r = sqlx::query!(
            "UPDATE cuotas_extraordinarias SET estado = 'anulada'
             WHERE id = $1 AND estado = 'vigente'",
            id.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(r.rows_affected() > 0)
    }

    async fn liquidacion(
        &mut self,
        id: LiquidacionExpensasId,
    ) -> AppResult<Option<LiquidacionExpensas>> {
        sqlx::query!(
            "SELECT id FROM liquidaciones_expensas WHERE id = $1 FOR UPDATE",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(self
            .liquidaciones_donde(Some(id.as_uuid()), None, 1)
            .await?
            .pop())
    }

    async fn liquidacion_del_periodo(
        &mut self,
        periodo: NaiveDate,
    ) -> AppResult<Option<LiquidacionExpensas>> {
        Ok(self
            .liquidaciones_donde(None, Some(periodo), 1)
            .await?
            .pop())
    }

    async fn liquidaciones(&mut self, limite: u32) -> AppResult<Vec<LiquidacionExpensas>> {
        self.liquidaciones_donde(None, None, i64::from(limite))
            .await
    }

    async fn insertar_liquidacion(&mut self, l: &LiquidacionExpensas) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO liquidaciones_expensas (id, tenant_id, periodo, estado, presupuesto_id,
                 politica_id, total, vencimiento, comprobante_id, liquidada_por, liquidada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            l.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            l.periodo,
            l.estado.as_str(),
            l.presupuesto_id.map(PresupuestoId::as_uuid),
            l.politica_id.as_uuid(),
            l.total.valor(),
            l.vencimiento,
            l.comprobante_id.as_uuid(),
            l.liquidada_por.map(CuentaId::as_uuid),
            l.liquidada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "el periodo ya está liquidado", "presupuesto o política"))?;
        Ok(())
    }

    async fn anular_liquidacion(
        &mut self,
        id: LiquidacionExpensasId,
        motivo: &str,
        en: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE cargos_pendientes SET estado = 'pendiente', obligacion_id = NULL
             WHERE obligacion_id IN (SELECT id FROM obligaciones WHERE liquidacion_id = $1)",
            id.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query!(
            "UPDATE obligaciones SET estado = 'anulada', anulada_en = $2
             WHERE liquidacion_id = $1 AND estado = 'vigente'",
            id.as_uuid(),
            en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query!(
            "UPDATE liquidaciones_expensas
             SET estado = 'anulada', motivo_anulacion = $2, anulada_en = $3
             WHERE id = $1",
            id.as_uuid(),
            motivo,
            en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn insertar_obligaciones(&mut self, o: &[ObligacionCobro]) -> AppResult<()> {
        let ids: Vec<Uuid> = o.iter().map(|x| x.id.as_uuid()).collect();
        let bienes: Vec<Uuid> = o.iter().map(|x| x.bien_privado_id.as_uuid()).collect();
        let conceptos: Vec<Uuid> = o.iter().map(|x| x.concepto_cobro_id.as_uuid()).collect();
        let periodos: Vec<NaiveDate> = o.iter().map(|x| x.periodo).collect();
        let valores: Vec<Decimal> = o.iter().map(|x| x.valor.valor()).collect();
        let vencimientos: Vec<NaiveDate> = o.iter().map(|x| x.vencimiento).collect();
        let deudores: Vec<Uuid> = o.iter().map(|x| x.deudor_persona_id.as_uuid()).collect();
        let origenes: Vec<String> = o.iter().map(|x| x.origen.as_str().to_owned()).collect();
        let liquidaciones: Vec<Option<Uuid>> = o
            .iter()
            .map(|x| x.liquidacion_id.map(LiquidacionExpensasId::as_uuid))
            .collect();
        let cuotas: Vec<Option<Uuid>> = o
            .iter()
            .map(|x| {
                x.cuota_extraordinaria_id
                    .map(CuotaExtraordinariaId::as_uuid)
            })
            .collect();
        let cuentas: Vec<Option<Uuid>> = o
            .iter()
            .map(|x| x.cuenta_cobro_id.map(CuentaCobroId::as_uuid))
            .collect();
        let comprobantes: Vec<Option<Uuid>> = o
            .iter()
            .map(|x| x.comprobante_id.map(ComprobanteId::as_uuid))
            .collect();
        let causaciones: Vec<Option<Uuid>> = o
            .iter()
            .map(|x| x.causacion_id.map(CausacionMoraId::as_uuid))
            .collect();
        let descripciones: Vec<Option<String>> = o.iter().map(|x| x.descripcion.clone()).collect();
        let creadas: Vec<DateTime<Utc>> = o.iter().map(|x| x.creada_en).collect();
        sqlx::query!(
            "INSERT INTO obligaciones (id, tenant_id, bien_privado_id, concepto_cobro_id, periodo,
                 valor, vencimiento, deudor_persona_id, origen, liquidacion_id,
                 cuota_extraordinaria_id, cuenta_cobro_id, comprobante_id, descripcion, estado,
                 creada_en, causacion_id)
             SELECT x.id, $1, x.bien, x.concepto, x.periodo, x.valor, x.vencimiento, x.deudor,
                    x.origen, x.liquidacion, x.cuota, x.cuenta, x.comprobante, x.descripcion,
                    'vigente', x.creada, x.causacion
             FROM unnest($2::uuid[], $3::uuid[], $4::uuid[], $5::date[], $6::numeric[],
                         $7::date[], $8::uuid[], $9::text[], $10::uuid[], $11::uuid[],
                         $12::uuid[], $13::uuid[], $14::text[], $15::timestamptz[], $16::uuid[])
                  AS x (id, bien, concepto, periodo, valor, vencimiento, deudor, origen,
                        liquidacion, cuota, cuenta, comprobante, descripcion, creada, causacion)",
            self.ctx.tenant_id.as_uuid(),
            &ids,
            &bienes,
            &conceptos,
            &periodos,
            &valores,
            &vencimientos,
            &deudores,
            &origenes,
            &liquidaciones as &[Option<Uuid>],
            &cuotas as &[Option<Uuid>],
            &cuentas as &[Option<Uuid>],
            &comprobantes as &[Option<Uuid>],
            &descripciones as &[Option<String>],
            &creadas,
            &causaciones as &[Option<Uuid>],
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "la obligación de ese bien, concepto y periodo ya está liquidada",
                "bien, concepto, deudor o cuenta de cobro",
            )
        })?;
        Ok(())
    }

    async fn anular_obligacion(
        &mut self,
        id: ObligacionCobroId,
        en: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE obligaciones SET estado = 'anulada', anulada_en = $2
             WHERE id = $1 AND estado = 'vigente'",
            id.as_uuid(),
            en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn saldos_obligaciones(
        &mut self,
        filtro: &FiltroObligaciones,
    ) -> AppResult<Vec<SaldoObligacion>> {
        self.saldos_donde(None, filtro).await
    }

    async fn obligacion(&mut self, id: ObligacionCobroId) -> AppResult<Option<SaldoObligacion>> {
        sqlx::query!(
            "SELECT id FROM obligaciones WHERE id = $1 FOR UPDATE",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(self
            .saldos_donde(Some(id.as_uuid()), &FiltroObligaciones::default())
            .await?
            .pop())
    }

    async fn insertar_cuentas_cobro(&mut self, c: &[CuentaCobro]) -> AppResult<()> {
        for x in c {
            sqlx::query!(
                "INSERT INTO cuentas_cobro (id, tenant_id, anio, numero, liquidacion_id,
                     bien_privado_id, periodo, deudor_persona_id, saldo_anterior, cargos, total,
                     vencimiento, pronto_pago_hasta, descuento_pronto_pago, referencia_pago,
                     creada_en)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
                x.id.as_uuid(),
                self.ctx.tenant_id.as_uuid(),
                i16_de(x.anio, "anio")?,
                i32_de(x.numero, "numero")?,
                x.liquidacion_id.as_uuid(),
                x.bien_privado_id.as_uuid(),
                x.periodo,
                x.deudor_persona_id.as_uuid(),
                x.saldo_anterior,
                x.cargos.valor(),
                x.total,
                x.vencimiento,
                x.pronto_pago_hasta,
                x.descuento_pronto_pago.map(Monto::valor),
                x.referencia_pago,
                x.creada_en,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_contable(e, "cuenta de cobro repetida", "liquidación o bien"))?;
        }
        Ok(())
    }

    async fn cuentas_cobro(
        &mut self,
        liquidacion: Option<LiquidacionExpensasId>,
        bien: Option<BienPrivadoId>,
    ) -> AppResult<Vec<CuentaCobro>> {
        self.cuentas_cobro_donde(
            None,
            liquidacion.map(LiquidacionExpensasId::as_uuid),
            bien.map(BienPrivadoId::as_uuid),
        )
        .await
    }

    async fn cuenta_cobro(&mut self, id: CuentaCobroId) -> AppResult<Option<CuentaCobro>> {
        Ok(self
            .cuentas_cobro_donde(Some(id.as_uuid()), None, None)
            .await?
            .pop())
    }

    async fn insertar_ajuste(&mut self, a: &AjusteObligacion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO ajustes_obligacion (id, tenant_id, obligacion_id, tipo, valor, motivo,
                 comprobante_id, recibo_id, creado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            a.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            a.obligacion_id.as_uuid(),
            a.tipo.as_str(),
            a.valor.valor(),
            a.motivo,
            a.comprobante_id.as_uuid(),
            a.recibo_id.map(ReciboCajaId::as_uuid),
            a.creado_por.map(CuentaId::as_uuid),
            a.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "ajuste repetido", "obligación, comprobante o recibo"))?;
        Ok(())
    }

    async fn ajustes(
        &mut self,
        recibo: Option<ReciboCajaId>,
        obligacion: Option<ObligacionCobroId>,
    ) -> AppResult<Vec<AjusteObligacion>> {
        sqlx::query_as!(
            FilaAjuste,
            "SELECT id, obligacion_id, tipo, valor, motivo, comprobante_id, recibo_id, creado_por,
                    creado_en, anulado_en
             FROM ajustes_obligacion
             WHERE anulado_en IS NULL AND ($1::uuid IS NULL OR recibo_id = $1)
               AND ($2::uuid IS NULL OR obligacion_id = $2)
             ORDER BY creado_en, id",
            recibo.map(ReciboCajaId::as_uuid),
            obligacion.map(ObligacionCobroId::as_uuid),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(AjusteObligacion::try_from)
        .collect()
    }

    async fn anular_ajuste(&mut self, id: AjusteObligacionId, en: DateTime<Utc>) -> AppResult<()> {
        sqlx::query!(
            "UPDATE ajustes_obligacion SET anulado_en = $2 WHERE id = $1 AND anulado_en IS NULL",
            id.as_uuid(),
            en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn cargos_pendientes(&mut self) -> AppResult<Vec<CargoPendiente>> {
        sqlx::query!(
            "SELECT id, bien_privado_id, concepto, descripcion, valor FROM cargos_pendientes
             WHERE estado = 'pendiente' ORDER BY creado_en, id"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(CargoPendiente {
                id: f.id,
                bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
                concepto: f.concepto,
                descripcion: f.descripcion,
                valor: monto(f.valor)?,
            })
        })
        .collect()
    }

    async fn aplicar_cargo(&mut self, cargo: Uuid, obligacion: ObligacionCobroId) -> AppResult<()> {
        sqlx::query!(
            "UPDATE cargos_pendientes SET estado = 'aplicado', obligacion_id = $2 WHERE id = $1",
            cargo,
            obligacion.as_uuid(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn pagos_aplicados(&mut self, bien: BienPrivadoId) -> AppResult<Vec<PagoAplicado>> {
        sqlx::query!(
            r#"SELECT p.recibo_id, r.fecha, p.obligacion_id AS "obligacion_id!", p.valor
               FROM aplicaciones_pago p
               JOIN recibos_caja r ON r.id = p.recibo_id
               JOIN obligaciones o ON o.id = p.obligacion_id
               WHERE o.bien_privado_id = $1 AND p.anulada_en IS NULL
               ORDER BY r.fecha, p.aplicada_en, p.id"#,
            bien.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(PagoAplicado {
                recibo_id: ReciboCajaId::from_uuid(f.recibo_id),
                fecha: f.fecha,
                obligacion_id: ObligacionCobroId::from_uuid(f.obligacion_id),
                valor: monto(f.valor)?,
            })
        })
        .collect()
    }

    async fn anticipos(&mut self, personas: &[PersonaId]) -> AppResult<Vec<Anticipo>> {
        let ids: Vec<Uuid> = personas.iter().map(|p| p.as_uuid()).collect();
        Ok(sqlx::query!(
            r#"SELECT r.id, r.persona_id AS "persona_id!", r.fecha,
                      r.valor - coalesce(sum(p.valor) FILTER (WHERE p.anulada_en IS NULL), 0)
                          AS "sin_aplicar!"
               FROM recibos_caja r
               LEFT JOIN aplicaciones_pago p ON p.recibo_id = r.id
               WHERE r.estado = 'registrado' AND r.persona_id = ANY($1)
               GROUP BY r.id
               HAVING r.valor - coalesce(sum(p.valor) FILTER (WHERE p.anulada_en IS NULL), 0) > 0
               ORDER BY r.fecha, r.id"#,
            &ids,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| Anticipo {
            recibo_id: ReciboCajaId::from_uuid(f.id),
            persona_id: PersonaId::from_uuid(f.persona_id),
            fecha: f.fecha,
            sin_aplicar: f.sin_aplicar,
        })
        .collect())
    }

    async fn liquidacion_tiene_movimientos(
        &mut self,
        id: LiquidacionExpensasId,
    ) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (
                   SELECT 1 FROM aplicaciones_pago p JOIN obligaciones o ON o.id = p.obligacion_id
                   WHERE o.liquidacion_id = $1 AND p.anulada_en IS NULL
                   UNION ALL
                   SELECT 1 FROM ajustes_obligacion a JOIN obligaciones o ON o.id = a.obligacion_id
                   WHERE o.liquidacion_id = $1 AND a.anulado_en IS NULL) AS "e!""#,
            id.as_uuid()
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn total_liquidado(&mut self, tipo: TipoConceptoCobro) -> AppResult<Decimal> {
        sqlx::query_scalar!(
            r#"SELECT coalesce(sum(o.valor), 0) AS "t!"
               FROM obligaciones o JOIN conceptos_cobro c ON c.id = o.concepto_cobro_id
               WHERE o.estado = 'vigente' AND c.tipo = $1"#,
            tipo.as_str()
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }
    async fn causado_hasta(&mut self) -> AppResult<BTreeMap<ObligacionCobroId, NaiveDate>> {
        Ok(sqlx::query!(
            r#"SELECT d.obligacion_id, max(d.hasta) AS "hasta!"
               FROM causaciones_interes d
               JOIN causaciones_mora c ON c.id = d.causacion_id
               WHERE c.estado = 'contabilizada'
               GROUP BY d.obligacion_id"#
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| (ObligacionCobroId::from_uuid(f.obligacion_id), f.hasta))
        .collect())
    }

    async fn reducciones(
        &mut self,
        obligaciones: &[ObligacionCobroId],
    ) -> AppResult<Vec<(ObligacionCobroId, Reduccion)>> {
        let ids: Vec<Uuid> = obligaciones.iter().map(|o| o.as_uuid()).collect();
        Ok(sqlx::query!(
            r#"SELECT p.obligacion_id AS "obligacion_id!", r.fecha AS "fecha!", p.valor AS "valor!"
               FROM aplicaciones_pago p JOIN recibos_caja r ON r.id = p.recibo_id
               WHERE p.obligacion_id = ANY($1) AND p.anulada_en IS NULL
               UNION ALL
               SELECT a.obligacion_id, (a.creado_en AT TIME ZONE 'UTC')::date, a.valor
               FROM ajustes_obligacion a
               WHERE a.obligacion_id = ANY($1) AND a.anulado_en IS NULL"#,
            &ids,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            (
                ObligacionCobroId::from_uuid(f.obligacion_id),
                Reduccion {
                    fecha: f.fecha,
                    valor: f.valor,
                },
            )
        })
        .collect())
    }

    async fn causacion_del_periodo(
        &mut self,
        periodo: NaiveDate,
    ) -> AppResult<Option<CausacionMora>> {
        Ok(self.causaciones_donde(None, Some(periodo), 1).await?.pop())
    }

    async fn causacion(&mut self, id: CausacionMoraId) -> AppResult<Option<CausacionMora>> {
        sqlx::query!(
            "SELECT id FROM causaciones_mora WHERE id = $1 FOR UPDATE",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(self
            .causaciones_donde(Some(id.as_uuid()), None, 1)
            .await?
            .pop())
    }

    async fn causaciones(&mut self, limite: u32) -> AppResult<Vec<CausacionMora>> {
        self.causaciones_donde(None, None, i64::from(limite)).await
    }

    async fn detalle_causacion(&mut self, id: CausacionMoraId) -> AppResult<Vec<DetalleInteres>> {
        sqlx::query!(
            "SELECT causacion_id, obligacion_id, obligacion_interes_id, desde, hasta, dias, base,
                    tasa_efectiva_anual, tasa_referencia_id, valor
             FROM causaciones_interes WHERE causacion_id = $1
             ORDER BY obligacion_interes_id, obligacion_id, desde",
            id.as_uuid()
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(DetalleInteres {
                causacion_id: CausacionMoraId::from_uuid(f.causacion_id),
                obligacion_id: ObligacionCobroId::from_uuid(f.obligacion_id),
                obligacion_interes_id: ObligacionCobroId::from_uuid(f.obligacion_interes_id),
                desde: f.desde,
                hasta: f.hasta,
                dias: u32_de(f.dias, "dias")?,
                base: f.base,
                tasa_efectiva_anual: f.tasa_efectiva_anual,
                tasa_referencia_id: TasaReferenciaId::from_uuid(f.tasa_referencia_id),
                valor: f.valor,
            })
        })
        .collect()
    }

    async fn insertar_causacion(&mut self, c: &CausacionMora) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO causaciones_mora (id, tenant_id, periodo, hasta, estado, total,
                 comprobante_id, causada_por, causada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.periodo,
            c.hasta,
            c.estado.as_str(),
            c.total.valor(),
            c.comprobante_id.as_uuid(),
            c.causada_por.map(CuentaId::as_uuid),
            c.causada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "los intereses del periodo ya están causados",
                "comprobante",
            )
        })?;
        Ok(())
    }

    async fn insertar_detalle_interes(&mut self, d: &[DetalleInteres]) -> AppResult<()> {
        for x in d {
            sqlx::query!(
                "INSERT INTO causaciones_interes (id, tenant_id, causacion_id, obligacion_id,
                     obligacion_interes_id, desde, hasta, dias, base, tasa_efectiva_anual,
                     tasa_referencia_id, valor)
                 VALUES (uuidv7(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                self.ctx.tenant_id.as_uuid(),
                x.causacion_id.as_uuid(),
                x.obligacion_id.as_uuid(),
                x.obligacion_interes_id.as_uuid(),
                x.desde,
                x.hasta,
                i32_de(x.dias, "dias")?,
                x.base,
                x.tasa_efectiva_anual,
                x.tasa_referencia_id.as_uuid(),
                x.valor,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_contable(e, "tramo repetido", "obligación o tasa de referencia"))?;
        }
        Ok(())
    }

    async fn anular_causacion(
        &mut self,
        id: CausacionMoraId,
        motivo: &str,
        en: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE obligaciones SET estado = 'anulada', anulada_en = $2
             WHERE causacion_id = $1 AND estado = 'vigente'",
            id.as_uuid(),
            en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query!(
            "UPDATE causaciones_mora SET estado = 'anulada', motivo_anulacion = $2, anulada_en = $3
             WHERE id = $1",
            id.as_uuid(),
            motivo,
            en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn causacion_tiene_movimientos(&mut self, id: CausacionMoraId) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (
                   SELECT 1 FROM aplicaciones_pago p JOIN obligaciones o ON o.id = p.obligacion_id
                   WHERE o.causacion_id = $1 AND p.anulada_en IS NULL
                   UNION ALL
                   SELECT 1 FROM ajustes_obligacion a JOIN obligaciones o ON o.id = a.obligacion_id
                   WHERE o.causacion_id = $1 AND a.anulado_en IS NULL) AS "e!""#,
            id.as_uuid()
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn acuerdos(&mut self, bien: Option<BienPrivadoId>) -> AppResult<Vec<AcuerdoPago>> {
        self.acuerdos_donde(None, bien.map(BienPrivadoId::as_uuid))
            .await
    }

    async fn acuerdo(&mut self, id: AcuerdoPagoId) -> AppResult<Option<AcuerdoPago>> {
        sqlx::query!(
            "SELECT id FROM acuerdos_pago WHERE id = $1 FOR UPDATE",
            id.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(self.acuerdos_donde(Some(id.as_uuid()), None).await?.pop())
    }

    async fn insertar_acuerdo(&mut self, a: &AcuerdoPago) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO acuerdos_pago (id, tenant_id, bien_privado_id, deudor_persona_id, fecha,
                 valor, numero_cuotas, estado, observaciones, creado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            a.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            a.bien_privado_id.as_uuid(),
            a.deudor_persona_id.as_uuid(),
            a.fecha,
            a.valor.valor(),
            i16_de(a.cuotas.len(), "numero_cuotas")?,
            a.estado.as_str(),
            a.observaciones,
            a.creado_por.as_uuid(),
            a.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "el bien ya tiene un acuerdo de pago activo",
                "bien o deudor",
            )
        })?;
        for c in &a.cuotas {
            sqlx::query!(
                "INSERT INTO cuotas_acuerdo (tenant_id, acuerdo_id, numero, vencimiento, valor)
                 VALUES ($1, $2, $3, $4, $5)",
                self.ctx.tenant_id.as_uuid(),
                a.id.as_uuid(),
                i16_de(c.numero, "numero")?,
                c.vencimiento,
                c.valor,
            )
            .execute(&mut *self.tx)
            .await
            .map_err(AppError::infra)?;
        }
        Ok(())
    }

    async fn actualizar_acuerdo(&mut self, a: &AcuerdoPago) -> AppResult<()> {
        sqlx::query!(
            "UPDATE acuerdos_pago
             SET estado = $2, aprobado_por = $3, aprobado_en = $4, cerrado_en = $5
             WHERE id = $1",
            a.id.as_uuid(),
            a.estado.as_str(),
            a.aprobado_por.map(CuentaId::as_uuid),
            a.aprobado_en,
            a.cerrado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "el acuerdo lo aprueba otra cuenta", "acuerdo"))?;
        Ok(())
    }

    async fn insertar_certificado(&mut self, c: &CertificadoCartera) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO certificados_emitidos (id, tenant_id, tipo, bien_privado_id, corte, saldo,
                 vencido, codigo_hash, pdf_sha256, emitido_por, emitido_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.tipo.as_str(),
            c.bien_privado_id.as_uuid(),
            c.corte,
            c.saldo,
            c.vencido,
            c.codigo_hash,
            c.pdf_sha256,
            c.emitido_por.map(CuentaId::as_uuid),
            c.emitido_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "código de verificación repetido", "bien"))?;
        Ok(())
    }

    async fn certificado_por_codigo(
        &mut self,
        codigo_hash: &str,
    ) -> AppResult<Option<CertificadoCartera>> {
        Ok(self
            .certificados_donde(Some(codigo_hash), None)
            .await?
            .pop())
    }

    async fn certificados(&mut self, bien: BienPrivadoId) -> AppResult<Vec<CertificadoCartera>> {
        self.certificados_donde(None, Some(bien.as_uuid())).await
    }

    async fn cuentas_por_avisar(
        &mut self,
        hoy: NaiveDate,
        dias_antes: u32,
        dias_despues: u32,
    ) -> AppResult<Vec<(CuentaCobro, TipoAvisoCartera)>> {
        let antes = i32_de(dias_antes, "dias_antes")?;
        let despues = i32_de(dias_despues, "dias_despues")?;
        let filas = sqlx::query!(
            r#"SELECT c.id, t.tipo AS "tipo!"
               FROM cuentas_cobro c
               JOIN liquidaciones_expensas l ON l.id = c.liquidacion_id
                    AND l.estado = 'contabilizada'
               CROSS JOIN LATERAL (VALUES
                   (CASE WHEN c.vencimiento > $1::date AND c.vencimiento <= $1::date + $2::int
                         THEN 'recordatorio' END),
                   (CASE WHEN c.vencimiento <= $1::date - $3::int AND c.vencimiento > $1::date - 60
                         THEN 'mora' END)) AS t (tipo)
               WHERE t.tipo IS NOT NULL
                 AND NOT EXISTS (SELECT 1 FROM avisos_cartera a
                                 WHERE a.cuenta_cobro_id = c.id AND a.tipo = t.tipo)
               ORDER BY c.anio, c.numero"#,
            hoy,
            antes,
            despues,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let mut v = Vec::with_capacity(filas.len());
        for f in filas {
            if let Some(c) = self
                .cuentas_cobro_donde(Some(f.id), None, None)
                .await?
                .pop()
            {
                v.push((c, TipoAvisoCartera::try_from(f.tipo.as_str())?));
            }
        }
        Ok(v)
    }

    async fn registrar_aviso(
        &mut self,
        cuenta: CuentaCobroId,
        tipo: TipoAvisoCartera,
        en: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO avisos_cartera (tenant_id, cuenta_cobro_id, tipo, enviado_en)
             VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
            self.ctx.tenant_id.as_uuid(),
            cuenta.as_uuid(),
            tipo.as_str(),
            en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

impl ResumenCartera for TransaccionPg {
    async fn resumen_bien(
        &mut self,
        bien: BienPrivadoId,
        hoy: NaiveDate,
    ) -> AppResult<Option<ResumenBien>> {
        let Some(b) = sqlx::query!(
            "SELECT referencia_pago, pagador_persona_id FROM bienes_privados WHERE id = $1",
            bien.as_uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        else {
            return Ok(None);
        };
        let pendientes = self
            .saldos_donde(
                None,
                &FiltroObligaciones {
                    bien: Some(bien),
                    con_saldo: true,
                    ..FiltroObligaciones::default()
                },
            )
            .await?;
        Ok(Some(ResumenBien {
            saldo: pendientes.iter().map(|o| o.saldo).sum(),
            vencido: pendientes
                .iter()
                .filter(|o| o.obligacion.vencimiento < hoy)
                .map(|o| o.saldo)
                .sum(),
            proximo_vencimiento: pendientes
                .iter()
                .map(|o| o.obligacion.vencimiento)
                .filter(|v| *v >= hoy)
                .min(),
            referencia_pago: b.referencia_pago,
            pagador_persona_id: b.pagador_persona_id.map(PersonaId::from_uuid),
        }))
    }
}

/// Directorio de copropiedades con avisos de cartera pendientes (función `SECURITY DEFINER`).
#[derive(Debug, Clone)]
pub struct DirectorioCarteraPg {
    pool: sqlx::PgPool,
}

impl DirectorioCarteraPg {
    /// Directorio sobre el pool de la app (`dad_app`).
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl DirectorioCartera for DirectorioCarteraPg {
    async fn copropiedades_con_avisos(
        &self,
        hoy: NaiveDate,
        dias_antes: u32,
        dias_despues: u32,
    ) -> AppResult<Vec<domain::TenantId>> {
        Ok(sqlx::query_scalar!(
            r#"SELECT tenant_id AS "tenant_id!" FROM copropiedades_con_avisos_cartera($1, $2, $3)"#,
            hoy,
            i32_de(dias_antes, "dias_antes")?,
            i32_de(dias_despues, "dias_despues")?,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(domain::TenantId::from_uuid)
        .collect())
    }
}

impl TransaccionPg {
    async fn causaciones_donde(
        &mut self,
        id: Option<Uuid>,
        periodo: Option<NaiveDate>,
        limite: i64,
    ) -> AppResult<Vec<CausacionMora>> {
        sqlx::query!(
            "SELECT id, periodo, hasta, estado, total, comprobante_id, motivo_anulacion,
                    causada_por, causada_en, anulada_en
             FROM causaciones_mora
             WHERE ($1::uuid IS NULL OR id = $1)
               AND ($2::date IS NULL OR (periodo = $2 AND estado = 'contabilizada'))
             ORDER BY periodo DESC, causada_en DESC
             LIMIT $3",
            id,
            periodo,
            limite,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(CausacionMora {
                id: CausacionMoraId::from_uuid(f.id),
                periodo: f.periodo,
                hasta: f.hasta,
                estado: EstadoCausacion::try_from(f.estado.as_str())?,
                total: monto(f.total)?,
                comprobante_id: ComprobanteId::from_uuid(f.comprobante_id),
                motivo_anulacion: f.motivo_anulacion,
                causada_por: f.causada_por.map(CuentaId::from_uuid),
                causada_en: f.causada_en,
                anulada_en: f.anulada_en,
            })
        })
        .collect()
    }

    async fn acuerdos_donde(
        &mut self,
        id: Option<Uuid>,
        bien: Option<Uuid>,
    ) -> AppResult<Vec<AcuerdoPago>> {
        let filas = sqlx::query!(
            "SELECT id, bien_privado_id, deudor_persona_id, fecha, valor, estado, observaciones,
                    creado_por, creado_en, aprobado_por, aprobado_en, cerrado_en
             FROM acuerdos_pago
             WHERE ($1::uuid IS NULL OR id = $1) AND ($2::uuid IS NULL OR bien_privado_id = $2)
             ORDER BY creado_en DESC, id",
            id,
            bien,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let ids: Vec<Uuid> = filas.iter().map(|f| f.id).collect();
        let cuotas = sqlx::query!(
            "SELECT acuerdo_id, numero, vencimiento, valor FROM cuotas_acuerdo
             WHERE acuerdo_id = ANY($1) ORDER BY acuerdo_id, numero",
            &ids,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let mut por_acuerdo: BTreeMap<Uuid, Vec<CuotaAcuerdo>> = BTreeMap::new();
        for c in cuotas {
            por_acuerdo
                .entry(c.acuerdo_id)
                .or_default()
                .push(CuotaAcuerdo {
                    numero: u16::try_from(c.numero).unwrap_or(0),
                    vencimiento: c.vencimiento,
                    valor: c.valor,
                });
        }
        filas
            .into_iter()
            .map(|f| {
                Ok(AcuerdoPago {
                    id: AcuerdoPagoId::from_uuid(f.id),
                    bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
                    deudor_persona_id: PersonaId::from_uuid(f.deudor_persona_id),
                    fecha: f.fecha,
                    valor: monto(f.valor)?,
                    estado: EstadoAcuerdo::try_from(f.estado.as_str())?,
                    observaciones: f.observaciones,
                    creado_por: CuentaId::from_uuid(f.creado_por),
                    creado_en: f.creado_en,
                    aprobado_por: f.aprobado_por.map(CuentaId::from_uuid),
                    aprobado_en: f.aprobado_en,
                    cerrado_en: f.cerrado_en,
                    cuotas: por_acuerdo.remove(&f.id).unwrap_or_default(),
                })
            })
            .collect()
    }

    async fn certificados_donde(
        &mut self,
        codigo_hash: Option<&str>,
        bien: Option<Uuid>,
    ) -> AppResult<Vec<CertificadoCartera>> {
        sqlx::query!(
            "SELECT id, tipo, bien_privado_id, corte, saldo, vencido, codigo_hash, pdf_sha256,
                    emitido_por, emitido_en
             FROM certificados_emitidos
             WHERE ($1::text IS NULL OR codigo_hash = $1) AND ($2::uuid IS NULL OR bien_privado_id = $2)
             ORDER BY emitido_en DESC, id",
            codigo_hash,
            bien,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(CertificadoCartera {
                id: CertificadoCarteraId::from_uuid(f.id),
                tipo: TipoCertificado::try_from(f.tipo.as_str())?,
                bien_privado_id: BienPrivadoId::from_uuid(f.bien_privado_id),
                corte: f.corte,
                saldo: f.saldo,
                vencido: f.vencido,
                codigo_hash: f.codigo_hash,
                pdf_sha256: f.pdf_sha256,
                emitido_por: f.emitido_por.map(CuentaId::from_uuid),
                emitido_en: f.emitido_en,
            })
        })
        .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// Tasas de referencia (catálogo de plataforma)
// ---------------------------------------------------------------------------------------------

macro_rules! catalogo_tasas {
    ($tipo:ty) => {
        impl CatalogoTasas for $tipo {
            async fn tasas_referencia(
                &mut self,
                modalidad: ModalidadTasa,
            ) -> AppResult<Vec<TasaReferencia>> {
                sqlx::query!(
                    "SELECT id, modalidad, vigente_desde, vigente_hasta, tasa_efectiva_anual,
                            resolucion, fuente, creada_en
                     FROM tasas_referencia WHERE modalidad = $1 ORDER BY vigente_desde",
                    modalidad.as_str()
                )
                .fetch_all(&mut *self.tx)
                .await
                .map_err(AppError::infra)?
                .into_iter()
                .map(|f| {
                    Ok(TasaReferencia {
                        id: TasaReferenciaId::from_uuid(f.id),
                        modalidad: ModalidadTasa::try_from(f.modalidad.as_str())?,
                        vigente_desde: f.vigente_desde,
                        vigente_hasta: f.vigente_hasta,
                        tasa_efectiva_anual: f.tasa_efectiva_anual,
                        resolucion: f.resolucion,
                        fuente: f.fuente,
                        creada_en: f.creada_en,
                    })
                })
                .collect()
            }
        }
    };
}

catalogo_tasas!(TransaccionPg);
catalogo_tasas!(TransaccionPlataformaPg);

impl CatalogoTasasPlataforma for TransaccionPlataformaPg {
    async fn insertar_tasa_referencia(&mut self, t: &TasaReferencia) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO tasas_referencia (id, modalidad, vigente_desde, vigente_hasta,
                 tasa_efectiva_anual, resolucion, fuente, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            t.id.as_uuid(),
            t.modalidad.as_str(),
            t.vigente_desde,
            t.vigente_hasta,
            t.tasa_efectiva_anual,
            t.resolucion,
            t.fuente,
            t.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| match e.as_database_error() {
            Some(d) if d.code().as_deref() == Some("23P01") => {
                AppError::Conflicto("la vigencia se solapa con otra tasa de la modalidad".into())
            }
            _ => error_contable(e, "tasa repetida", "tasa"),
        })?;
        Ok(())
    }
}
