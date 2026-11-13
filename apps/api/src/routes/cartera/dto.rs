//! DTO de la cartera de la PH (plan 14). Los montos viajan como texto decimal (`"150000.00"`),
//! nunca como número de punto flotante.

use application::{
    casos_uso::contabilidad_ph::{
        informes::{EstadoFondo, MesFlujo},
        liquidacion::{CobroPrevisto, EnvioCuentas, Liquidado, TotalConcepto, VistaLiquidacion},
        mora::{AcuerdoConSituacion, CalculoMora, Causado, CertificadoEmitido, EdadesBien},
        presupuesto::{EjecucionPresupuestal, EjecucionRubro},
        recaudo::{
            CarteraInicial, CuadreCuenta, EstadoCuenta, Recaudo, RecaudoExtracto, SaldoBien,
        },
    },
    puertos::cartera::{
        AcuerdoPago, AjusteObligacion, BienCartera, CausacionMora, CertificadoCartera,
        ConceptoCobro, CuentaCobro, CuotaExtraordinaria, DetalleInteres, LiquidacionExpensas,
        ModuloContribucion, PagoAplicado, PoliticaCartera, Presupuesto, RubroPresupuesto,
        SaldoObligacion, TasaReferencia,
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    TipoBienPrivado,
    contabilidad::{Monto, tesoreria::MedioRecibo},
    contabilidad_ph::{
        CriterioDistribucion, EstadoCuotaExtraordinaria, EstadoLiquidacionExpensas,
        EstadoObligacion, EstadoPresupuesto, OrdenImputacion, OrigenObligacion, TipoAjuste,
        TipoConceptoCobro,
        mora::{EstadoAcuerdo, EstadoCausacion, ModalidadTasa, SituacionAcuerdo, TipoCertificado},
    },
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::routes::contabilidad::dto_tesoreria::ReciboRespuesta;

fn verdadero() -> bool {
    true
}

/// Dinero con exactamente dos decimales (`"0.00"`, igual que [`Monto`]): las sumas de la cartera
/// pueden perder la escala.
fn pesos(d: Decimal) -> Decimal {
    let mut v = d.round_dp(2);
    v.rescale(2);
    v
}

// ---------------------------------------------------------------------------------------------
// Configuración
// ---------------------------------------------------------------------------------------------

/// Módulo de contribución (crear o actualizar).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ModuloContribucionPeticion {
    #[schema(example = "Torre con ascensor")]
    pub nombre: String,
    pub descripcion: Option<String>,
    #[serde(default = "verdadero")]
    pub activo: bool,
}

/// Módulo de contribución. Sus coeficientes son un cuadro con `modulo_id`
/// (`/copropiedades/{id}/cuadros-coeficientes`).
#[derive(Debug, Serialize, ToSchema)]
pub struct ModuloContribucionRespuesta {
    pub id: Uuid,
    pub nombre: String,
    pub descripcion: Option<String>,
    pub activo: bool,
    pub creado_en: DateTime<Utc>,
}

impl From<ModuloContribucion> for ModuloContribucionRespuesta {
    fn from(m: ModuloContribucion) -> Self {
        Self {
            id: m.id.as_uuid(),
            nombre: m.nombre,
            descripcion: m.descripcion,
            activo: m.activo,
            creado_en: m.creado_en,
        }
    }
}

/// Valor mensual de un concepto para un tipo de bien.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TarifaDto {
    #[schema(value_type = crate::esquemas::TipoBienPrivado)]
    pub tipo_bien: TipoBienPrivado,
    #[schema(value_type = String, example = "20000.00")]
    pub valor: Monto,
}

/// Concepto de cobro nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoConceptoPeticion {
    #[schema(example = "administracion")]
    pub codigo: String,
    #[schema(example = "Cuota de administración")]
    pub nombre: String,
    #[schema(value_type = crate::esquemas::TipoConceptoCobro)]
    pub tipo: TipoConceptoCobro,
    #[schema(value_type = crate::esquemas::CriterioDistribucion)]
    pub criterio: CriterioDistribucion,
    /// Solo con el criterio `modulo`.
    pub modulo_id: Option<Uuid>,
    /// Solo con el criterio `valor_fijo` (valor mensual por bien).
    #[schema(value_type = Option<String>)]
    pub valor_fijo: Option<Monto>,
    /// Solo con el criterio `por_tipo_bien`.
    #[serde(default)]
    pub tarifas: Vec<TarifaDto>,
    #[serde(default)]
    pub genera_interes: bool,
    /// Concepto de las reglas `ph.*` del contador (elige las cuentas).
    #[schema(example = "administracion")]
    pub concepto_contable: String,
}

/// Cambios de un concepto de cobro (código, tipo y criterio no cambian).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CambioConceptoPeticion {
    pub nombre: String,
    #[schema(value_type = Option<String>)]
    pub valor_fijo: Option<Monto>,
    #[serde(default)]
    pub tarifas: Vec<TarifaDto>,
    #[serde(default)]
    pub genera_interes: bool,
    pub concepto_contable: String,
    #[serde(default = "verdadero")]
    pub activo: bool,
}

/// Concepto de cobro.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConceptoCobroRespuesta {
    pub id: Uuid,
    pub codigo: String,
    pub nombre: String,
    #[schema(value_type = crate::esquemas::TipoConceptoCobro)]
    pub tipo: TipoConceptoCobro,
    #[schema(value_type = crate::esquemas::CriterioDistribucion)]
    pub criterio: CriterioDistribucion,
    pub modulo_id: Option<Uuid>,
    #[schema(value_type = Option<String>)]
    pub valor_fijo: Option<Monto>,
    pub tarifas: Vec<TarifaDto>,
    pub genera_interes: bool,
    pub concepto_contable: String,
    pub activo: bool,
    pub creado_en: DateTime<Utc>,
}

impl From<ConceptoCobro> for ConceptoCobroRespuesta {
    fn from(c: ConceptoCobro) -> Self {
        Self {
            id: c.id.as_uuid(),
            codigo: c.codigo,
            nombre: c.nombre,
            tipo: c.tipo,
            criterio: c.criterio,
            modulo_id: c.modulo_id.map(|m| m.as_uuid()),
            valor_fijo: c.valor_fijo,
            tarifas: c
                .tarifas
                .into_iter()
                .map(|(tipo_bien, valor)| TarifaDto { tipo_bien, valor })
                .collect(),
            genera_interes: c.genera_interes,
            concepto_contable: c.concepto_contable,
            activo: c.activo,
            creado_en: c.creado_en,
        }
    }
}

/// Versión nueva de la política de cartera.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaPoliticaPeticion {
    pub vigente_desde: NaiveDate,
    /// Día del mes en que vence la cuenta de cobro (1 a 28).
    #[schema(example = 10)]
    pub dia_vencimiento: u8,
    /// Múltiplo al que se redondea cada cobro (1 = al peso).
    #[schema(value_type = String, example = "100")]
    pub redondeo: Decimal,
    /// Descuento por pronto pago sobre lo ordinario del periodo (%).
    #[schema(value_type = Option<String>, example = "5")]
    pub pronto_pago_porcentaje: Option<Decimal>,
    /// Día límite del pronto pago (no posterior al vencimiento).
    pub pronto_pago_dia: Option<u8>,
    /// Veces el interés bancario corriente (a lo sumo 1,5).
    #[schema(value_type = String, example = "1.5")]
    pub factor_mora: Decimal,
    /// Tasa mensual inferior aprobada por la asamblea (%).
    #[schema(value_type = Option<String>)]
    pub tasa_mora_mensual: Option<Decimal>,
    #[schema(value_type = crate::esquemas::OrdenImputacion)]
    pub imputacion: OrdenImputacion,
    #[serde(default)]
    pub permite_elegir_obligacion: bool,
    /// Porcentaje del fondo de imprevistos (al menos 1 salvo suspensión).
    #[schema(value_type = String, example = "1")]
    pub porcentaje_fondo: Decimal,
    #[serde(default)]
    pub fondo_suspendido: bool,
    /// Pregunta de la asamblea que la aprobó.
    pub pregunta_id: Option<Uuid>,
}

/// Versión de la política de cartera.
#[derive(Debug, Serialize, ToSchema)]
pub struct PoliticaCarteraRespuesta {
    pub id: Uuid,
    pub version: u32,
    pub vigente_desde: NaiveDate,
    pub dia_vencimiento: u8,
    #[schema(value_type = String)]
    pub redondeo: Decimal,
    #[schema(value_type = Option<String>)]
    pub pronto_pago_porcentaje: Option<Decimal>,
    pub pronto_pago_dia: Option<u8>,
    #[schema(value_type = String)]
    pub factor_mora: Decimal,
    #[schema(value_type = Option<String>)]
    pub tasa_mora_mensual: Option<Decimal>,
    #[schema(value_type = crate::esquemas::OrdenImputacion)]
    pub imputacion: OrdenImputacion,
    pub permite_elegir_obligacion: bool,
    #[schema(value_type = String)]
    pub porcentaje_fondo: Decimal,
    pub fondo_suspendido: bool,
    pub pregunta_id: Option<Uuid>,
    pub creada_en: DateTime<Utc>,
}

impl From<PoliticaCartera> for PoliticaCarteraRespuesta {
    fn from(p: PoliticaCartera) -> Self {
        let x = p.parametros;
        Self {
            id: p.id.as_uuid(),
            version: p.version,
            vigente_desde: p.vigente_desde,
            dia_vencimiento: x.dia_vencimiento,
            redondeo: x.redondeo,
            pronto_pago_porcentaje: x.pronto_pago.map(|p| p.0),
            pronto_pago_dia: x.pronto_pago.map(|p| p.1),
            factor_mora: x.factor_mora,
            tasa_mora_mensual: x.tasa_mora_mensual,
            imputacion: x.imputacion,
            permite_elegir_obligacion: x.permite_elegir_obligacion,
            porcentaje_fondo: x.porcentaje_fondo,
            fondo_suspendido: x.fondo_suspendido,
            pregunta_id: p.pregunta_id.map(|q| q.as_uuid()),
            creada_en: p.creada_en,
        }
    }
}

/// Datos de pago de un bien privado.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DatosPagoPeticion {
    /// Referencia numérica del recaudo (1 a 24 dígitos, única en la copropiedad).
    #[schema(example = "101")]
    pub referencia_pago: Option<String>,
    /// Quien paga (tenedor); vacío = el propietario representante.
    pub pagador_persona_id: Option<Uuid>,
}

/// Bien privado visto por la cartera.
#[derive(Debug, Serialize, ToSchema)]
pub struct BienCarteraRespuesta {
    pub id: Uuid,
    pub codigo: String,
    #[schema(value_type = crate::esquemas::TipoBienPrivado)]
    pub tipo: TipoBienPrivado,
    #[schema(value_type = Option<String>)]
    pub area_m2: Option<Decimal>,
    pub activo: bool,
    pub referencia_pago: Option<String>,
    pub pagador_persona_id: Option<Uuid>,
    /// Propietario representante: el deudor en el mayor.
    pub deudor_persona_id: Option<Uuid>,
}

impl From<BienCartera> for BienCarteraRespuesta {
    fn from(b: BienCartera) -> Self {
        Self {
            id: b.id.as_uuid(),
            codigo: b.codigo.as_str().to_owned(),
            tipo: b.tipo,
            area_m2: b.area_m2,
            activo: b.activo,
            referencia_pago: b.referencia_pago,
            pagador_persona_id: b.pagador_persona_id.map(|p| p.as_uuid()),
            deudor_persona_id: b.deudor_persona_id.map(|p| p.as_uuid()),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Presupuesto
// ---------------------------------------------------------------------------------------------

/// Rubro de un presupuesto.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RubroPeticion {
    /// Concepto ordinario que lo financia.
    pub concepto_cobro_id: Uuid,
    /// Cuenta contra la que se mide la ejecución.
    pub cuenta_contable_id: Uuid,
    pub centro_costo_id: Option<Uuid>,
    #[schema(example = "Vigilancia")]
    pub nombre: String,
    /// Enero a diciembre (12 valores).
    #[schema(value_type = Vec<String>)]
    pub valores_mensuales: Vec<Monto>,
}

/// Presupuesto nuevo (borrador).
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoPresupuestoPeticion {
    #[schema(example = 2027)]
    pub anio: i32,
    pub descripcion: Option<String>,
    pub rubros: Vec<RubroPeticion>,
}

/// Rubros de un borrador.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RubrosPeticion {
    pub rubros: Vec<RubroPeticion>,
}

/// Aprobación de un presupuesto por la asamblea.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AprobacionPresupuestoPeticion {
    pub vigente_desde: NaiveDate,
    /// Pregunta de la asamblea que lo aprobó.
    pub pregunta_id: Option<Uuid>,
}

/// Rubro de un presupuesto.
#[derive(Debug, Serialize, ToSchema)]
pub struct RubroRespuesta {
    pub id: Uuid,
    pub concepto_cobro_id: Uuid,
    pub cuenta_contable_id: Uuid,
    pub centro_costo_id: Option<Uuid>,
    pub nombre: String,
    #[schema(value_type = Vec<String>)]
    pub valores_mensuales: Vec<Monto>,
    #[schema(value_type = String)]
    pub valor_anual: Decimal,
}

impl From<RubroPresupuesto> for RubroRespuesta {
    fn from(r: RubroPresupuesto) -> Self {
        Self {
            id: r.id.as_uuid(),
            concepto_cobro_id: r.concepto_cobro_id.as_uuid(),
            cuenta_contable_id: r.cuenta_contable_id.as_uuid(),
            centro_costo_id: r.centro_costo_id.map(|c| c.as_uuid()),
            nombre: r.nombre.clone(),
            valor_anual: pesos(r.valor_anual()),
            valores_mensuales: r.valores_mensuales.to_vec(),
        }
    }
}

/// Presupuesto con sus rubros.
#[derive(Debug, Serialize, ToSchema)]
pub struct PresupuestoRespuesta {
    pub id: Uuid,
    pub anio: i32,
    pub version: u32,
    #[schema(value_type = crate::esquemas::EstadoPresupuesto)]
    pub estado: EstadoPresupuesto,
    pub descripcion: Option<String>,
    pub pregunta_id: Option<Uuid>,
    pub vigente_desde: Option<NaiveDate>,
    pub creado_en: DateTime<Utc>,
    #[schema(value_type = String)]
    pub total_anual: Decimal,
    pub rubros: Vec<RubroRespuesta>,
}

impl From<Presupuesto> for PresupuestoRespuesta {
    fn from(p: Presupuesto) -> Self {
        Self {
            id: p.id.as_uuid(),
            anio: p.anio,
            version: p.version,
            estado: p.estado,
            descripcion: p.descripcion,
            pregunta_id: p.pregunta_id.map(|q| q.as_uuid()),
            vigente_desde: p.vigente_desde,
            creado_en: p.creado_en,
            total_anual: pesos(p.rubros.iter().map(RubroPresupuesto::valor_anual).sum()),
            rubros: p.rubros.into_iter().map(Into::into).collect(),
        }
    }
}

/// Ejecución de un rubro.
#[derive(Debug, Serialize, ToSchema)]
pub struct EjecucionRubroRespuesta {
    pub rubro_id: Uuid,
    pub nombre: String,
    pub cuenta_contable_id: Uuid,
    pub cuenta_codigo: String,
    pub centro_costo_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub presupuestado: Decimal,
    #[schema(value_type = String)]
    pub ejecutado: Decimal,
    #[schema(value_type = String)]
    pub diferencia: Decimal,
    #[schema(value_type = String)]
    pub presupuesto_anual: Decimal,
}

impl From<EjecucionRubro> for EjecucionRubroRespuesta {
    fn from(r: EjecucionRubro) -> Self {
        Self {
            rubro_id: r.rubro_id.as_uuid(),
            nombre: r.nombre,
            cuenta_contable_id: r.cuenta_id.as_uuid(),
            cuenta_codigo: r.cuenta_codigo,
            centro_costo_id: r.centro_costo_id.map(|c| c.as_uuid()),
            presupuestado: pesos(r.presupuestado),
            ejecutado: pesos(r.ejecutado),
            diferencia: pesos(r.diferencia),
            presupuesto_anual: pesos(r.presupuesto_anual),
        }
    }
}

/// Ejecución presupuestal de enero al mes del corte.
#[derive(Debug, Serialize, ToSchema)]
pub struct EjecucionPresupuestalRespuesta {
    pub presupuesto_id: Uuid,
    pub anio: i32,
    pub hasta_mes: u32,
    pub rubros: Vec<EjecucionRubroRespuesta>,
}

impl From<EjecucionPresupuestal> for EjecucionPresupuestalRespuesta {
    fn from(e: EjecucionPresupuestal) -> Self {
        Self {
            presupuesto_id: e.presupuesto.id.as_uuid(),
            anio: e.presupuesto.anio,
            hasta_mes: e.hasta_mes,
            rubros: e.rubros.into_iter().map(Into::into).collect(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Liquidación
// ---------------------------------------------------------------------------------------------

/// Cuota extraordinaria nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaCuotaPeticion {
    /// Concepto de tipo `extraordinaria`.
    pub concepto_cobro_id: Uuid,
    #[schema(example = "Impermeabilización de cubiertas")]
    pub finalidad: String,
    #[schema(value_type = String, example = "30000000")]
    pub valor_total: Monto,
    #[schema(example = 6)]
    pub numero_cuotas: u16,
    /// Mes de la primera cuota.
    pub primer_periodo: NaiveDate,
    #[serde(default = "verdadero")]
    pub destinacion_especifica: bool,
    pub pregunta_id: Option<Uuid>,
}

/// Cuota extraordinaria.
#[derive(Debug, Serialize, ToSchema)]
pub struct CuotaExtraordinariaRespuesta {
    pub id: Uuid,
    pub concepto_cobro_id: Uuid,
    pub finalidad: String,
    #[schema(value_type = String)]
    pub valor_total: Monto,
    pub numero_cuotas: u16,
    pub primer_periodo: NaiveDate,
    pub destinacion_especifica: bool,
    pub pregunta_id: Option<Uuid>,
    #[schema(value_type = crate::esquemas::EstadoCuotaExtraordinaria)]
    pub estado: EstadoCuotaExtraordinaria,
    pub creada_en: DateTime<Utc>,
}

impl From<CuotaExtraordinaria> for CuotaExtraordinariaRespuesta {
    fn from(c: CuotaExtraordinaria) -> Self {
        Self {
            id: c.id.as_uuid(),
            concepto_cobro_id: c.concepto_cobro_id.as_uuid(),
            finalidad: c.finalidad,
            valor_total: c.valor_total,
            numero_cuotas: c.numero_cuotas,
            primer_periodo: c.primer_periodo,
            destinacion_especifica: c.destinacion_especifica,
            pregunta_id: c.pregunta_id.map(|q| q.as_uuid()),
            estado: c.estado,
            creada_en: c.creada_en,
        }
    }
}

/// Periodo a liquidar (cualquier día del mes).
#[derive(Debug, Deserialize, ToSchema)]
pub struct PeriodoPeticion {
    #[schema(example = "2026-11-01")]
    pub periodo: NaiveDate,
}

/// Anulación con motivo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MotivoPeticion {
    #[schema(example = "Se liquidó con el presupuesto equivocado")]
    pub motivo: String,
}

/// Cobro previsto de un bien.
#[derive(Debug, Serialize, ToSchema)]
pub struct CobroPrevistoRespuesta {
    pub bien_privado_id: Uuid,
    pub bien_codigo: String,
    pub concepto_cobro_id: Uuid,
    pub concepto_codigo: String,
    pub cuota_extraordinaria_id: Option<Uuid>,
    #[schema(value_type = crate::esquemas::OrigenObligacion)]
    pub origen: OrigenObligacion,
    #[schema(value_type = String)]
    pub valor: Monto,
    pub deudor_persona_id: Option<Uuid>,
}

impl From<CobroPrevisto> for CobroPrevistoRespuesta {
    fn from(c: CobroPrevisto) -> Self {
        Self {
            bien_privado_id: c.bien_id.as_uuid(),
            bien_codigo: c.bien_codigo,
            concepto_cobro_id: c.concepto_id.as_uuid(),
            concepto_codigo: c.concepto_codigo,
            cuota_extraordinaria_id: c.cuota_id.map(|q| q.as_uuid()),
            origen: c.origen,
            valor: c.valor,
            deudor_persona_id: c.deudor.map(|p| p.as_uuid()),
        }
    }
}

/// Total de un concepto.
#[derive(Debug, Serialize, ToSchema)]
pub struct TotalConceptoRespuesta {
    pub concepto_cobro_id: Uuid,
    pub codigo: String,
    pub nombre: String,
    #[schema(value_type = String)]
    pub total: Decimal,
}

impl From<TotalConcepto> for TotalConceptoRespuesta {
    fn from(t: TotalConcepto) -> Self {
        Self {
            concepto_cobro_id: t.concepto_id.as_uuid(),
            codigo: t.codigo,
            nombre: t.nombre,
            total: pesos(t.total),
        }
    }
}

/// Vista previa de una liquidación (no guarda nada).
#[derive(Debug, Serialize, ToSchema)]
pub struct VistaLiquidacionRespuesta {
    pub periodo: NaiveDate,
    pub vencimiento: NaiveDate,
    pub politica_id: Uuid,
    pub presupuesto_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub total: Decimal,
    pub por_concepto: Vec<TotalConceptoRespuesta>,
    pub cobros: Vec<CobroPrevistoRespuesta>,
    /// Advertencias que no impiden liquidar.
    pub avisos: Vec<String>,
}

impl From<VistaLiquidacion> for VistaLiquidacionRespuesta {
    fn from(v: VistaLiquidacion) -> Self {
        Self {
            periodo: v.periodo,
            vencimiento: v.vencimiento,
            politica_id: v.politica_id.as_uuid(),
            presupuesto_id: v.presupuesto_id.map(|p| p.as_uuid()),
            total: pesos(v.total),
            por_concepto: v.por_concepto.into_iter().map(Into::into).collect(),
            cobros: v.cobros.into_iter().map(Into::into).collect(),
            avisos: v.avisos,
        }
    }
}

/// Liquidación de expensas.
#[derive(Debug, Serialize, ToSchema)]
pub struct LiquidacionRespuesta {
    pub id: Uuid,
    pub periodo: NaiveDate,
    #[schema(value_type = crate::esquemas::EstadoLiquidacionExpensas)]
    pub estado: EstadoLiquidacionExpensas,
    pub presupuesto_id: Option<Uuid>,
    pub politica_id: Uuid,
    #[schema(value_type = String)]
    pub total: Monto,
    pub vencimiento: NaiveDate,
    pub comprobante_id: Uuid,
    pub motivo_anulacion: Option<String>,
    pub liquidada_en: DateTime<Utc>,
    pub anulada_en: Option<DateTime<Utc>>,
}

impl From<LiquidacionExpensas> for LiquidacionRespuesta {
    fn from(l: LiquidacionExpensas) -> Self {
        Self {
            id: l.id.as_uuid(),
            periodo: l.periodo,
            estado: l.estado,
            presupuesto_id: l.presupuesto_id.map(|p| p.as_uuid()),
            politica_id: l.politica_id.as_uuid(),
            total: l.total,
            vencimiento: l.vencimiento,
            comprobante_id: l.comprobante_id.as_uuid(),
            motivo_anulacion: l.motivo_anulacion,
            liquidada_en: l.liquidada_en,
            anulada_en: l.anulada_en,
        }
    }
}

/// Cuenta de cobro (inmutable).
#[derive(Debug, Serialize, ToSchema)]
pub struct CuentaCobroRespuesta {
    pub id: Uuid,
    /// `CB-2026-00012`.
    pub numero: String,
    pub liquidacion_id: Uuid,
    pub bien_privado_id: Uuid,
    pub periodo: NaiveDate,
    pub deudor_persona_id: Uuid,
    #[schema(value_type = String)]
    pub saldo_anterior: Decimal,
    #[schema(value_type = String)]
    pub cargos: Monto,
    #[schema(value_type = String)]
    pub total: Decimal,
    pub vencimiento: NaiveDate,
    pub pronto_pago_hasta: Option<NaiveDate>,
    #[schema(value_type = Option<String>)]
    pub descuento_pronto_pago: Option<Monto>,
    pub referencia_pago: Option<String>,
    pub creada_en: DateTime<Utc>,
}

impl From<CuentaCobro> for CuentaCobroRespuesta {
    fn from(c: CuentaCobro) -> Self {
        Self {
            numero: application::casos_uso::contabilidad_ph::liquidacion::numero_cuenta_cobro(&c),
            id: c.id.as_uuid(),
            liquidacion_id: c.liquidacion_id.as_uuid(),
            bien_privado_id: c.bien_privado_id.as_uuid(),
            periodo: c.periodo,
            deudor_persona_id: c.deudor_persona_id.as_uuid(),
            saldo_anterior: pesos(c.saldo_anterior),
            cargos: c.cargos,
            total: pesos(c.total),
            vencimiento: c.vencimiento,
            pronto_pago_hasta: c.pronto_pago_hasta,
            descuento_pronto_pago: c.descuento_pronto_pago,
            referencia_pago: c.referencia_pago,
            creada_en: c.creada_en,
        }
    }
}

/// Resultado de liquidar.
#[derive(Debug, Serialize, ToSchema)]
pub struct LiquidadoRespuesta {
    pub liquidacion: LiquidacionRespuesta,
    pub cuentas_cobro: Vec<CuentaCobroRespuesta>,
    /// Anticipos que se aplicaron a lo liquidado.
    pub anticipos_aplicados: usize,
    /// Se avisó que el fondo de imprevistos llegó al 50 % del presupuesto.
    pub aviso_fondo: bool,
    pub avisos: Vec<String>,
}

impl From<Liquidado> for LiquidadoRespuesta {
    fn from(l: Liquidado) -> Self {
        Self {
            liquidacion: l.liquidacion.into(),
            cuentas_cobro: l.cuentas_cobro.into_iter().map(Into::into).collect(),
            anticipos_aplicados: l.anticipos_aplicados,
            aviso_fondo: l.aviso_fondo,
            avisos: l.avisos,
        }
    }
}

/// Obligación con su saldo.
#[derive(Debug, Serialize, ToSchema)]
pub struct ObligacionRespuesta {
    pub id: Uuid,
    pub bien_privado_id: Uuid,
    pub concepto_cobro_id: Uuid,
    pub concepto_codigo: String,
    pub concepto_nombre: String,
    #[schema(value_type = crate::esquemas::TipoConceptoCobro)]
    pub concepto_tipo: TipoConceptoCobro,
    pub periodo: NaiveDate,
    #[schema(value_type = String)]
    pub valor: Monto,
    pub vencimiento: NaiveDate,
    pub deudor_persona_id: Uuid,
    #[schema(value_type = crate::esquemas::OrigenObligacion)]
    pub origen: OrigenObligacion,
    pub liquidacion_id: Option<Uuid>,
    pub cuota_extraordinaria_id: Option<Uuid>,
    pub cuenta_cobro_id: Option<Uuid>,
    pub comprobante_id: Option<Uuid>,
    pub descripcion: Option<String>,
    #[schema(value_type = crate::esquemas::EstadoObligacion)]
    pub estado: EstadoObligacion,
    #[schema(value_type = String)]
    pub ajustes: Decimal,
    #[schema(value_type = String)]
    pub aplicado: Decimal,
    #[schema(value_type = String)]
    pub saldo: Decimal,
}

impl From<SaldoObligacion> for ObligacionRespuesta {
    fn from(s: SaldoObligacion) -> Self {
        let o = s.obligacion;
        Self {
            id: o.id.as_uuid(),
            bien_privado_id: o.bien_privado_id.as_uuid(),
            concepto_cobro_id: o.concepto_cobro_id.as_uuid(),
            concepto_codigo: s.concepto_codigo,
            concepto_nombre: s.concepto_nombre,
            concepto_tipo: s.concepto_tipo,
            periodo: o.periodo,
            valor: o.valor,
            vencimiento: o.vencimiento,
            deudor_persona_id: o.deudor_persona_id.as_uuid(),
            origen: o.origen,
            liquidacion_id: o.liquidacion_id.map(|x| x.as_uuid()),
            cuota_extraordinaria_id: o.cuota_extraordinaria_id.map(|x| x.as_uuid()),
            cuenta_cobro_id: o.cuenta_cobro_id.map(|x| x.as_uuid()),
            comprobante_id: o.comprobante_id.map(|x| x.as_uuid()),
            descripcion: o.descripcion,
            estado: o.estado,
            ajustes: pesos(s.ajustes),
            aplicado: pesos(s.aplicado),
            saldo: pesos(s.saldo),
        }
    }
}

/// Liquidación con sus obligaciones.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleLiquidacionRespuesta {
    pub liquidacion: LiquidacionRespuesta,
    pub obligaciones: Vec<ObligacionRespuesta>,
}

/// Resultado del envío de las cuentas de cobro.
#[derive(Debug, Serialize, ToSchema)]
pub struct EnvioCuentasRespuesta {
    pub enviadas: usize,
    /// Pagadores sin correo.
    pub sin_correo: usize,
}

impl From<EnvioCuentas> for EnvioCuentasRespuesta {
    fn from(e: EnvioCuentas) -> Self {
        Self {
            enviadas: e.enviadas,
            sin_correo: e.sin_correo,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Recaudo
// ---------------------------------------------------------------------------------------------

/// Pago recibido para un bien privado.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoRecaudoPeticion {
    pub bien_privado_id: Uuid,
    pub cuenta_tesoreria_id: Uuid,
    pub fecha: NaiveDate,
    #[schema(value_type = String, example = "505000")]
    pub valor: Monto,
    #[schema(value_type = crate::esquemas::MedioRecibo)]
    pub medio: MedioRecibo,
    pub referencia: Option<String>,
    pub descripcion: Option<String>,
    pub soporte_archivo_id: Option<Uuid>,
    /// Obligaciones que se cubren primero (solo si la política lo permite).
    #[serde(default)]
    pub obligaciones_elegidas: Vec<Uuid>,
}

/// Lo aplicado a una obligación.
#[derive(Debug, Serialize, ToSchema)]
pub struct AplicadoRespuesta {
    pub obligacion_id: Uuid,
    #[schema(value_type = String)]
    pub valor: Monto,
}

/// Resultado de un recaudo.
#[derive(Debug, Serialize, ToSchema)]
pub struct RecaudoRespuesta {
    pub bien_privado_id: Uuid,
    pub recibo: ReciboRespuesta,
    pub aplicado: Vec<AplicadoRespuesta>,
    /// Lo que quedó sin aplicar en el recibo.
    #[schema(value_type = String)]
    pub anticipo: Decimal,
    /// Descuento por pronto pago ganado.
    #[schema(value_type = Option<String>)]
    pub descuento: Option<Monto>,
}

pub(super) fn aplicados(v: Vec<(domain::ObligacionCobroId, Monto)>) -> Vec<AplicadoRespuesta> {
    v.into_iter()
        .map(|(o, valor)| AplicadoRespuesta {
            obligacion_id: o.as_uuid(),
            valor,
        })
        .collect()
}

impl From<Recaudo> for RecaudoRespuesta {
    fn from(r: Recaudo) -> Self {
        Self {
            bien_privado_id: r.bien_id.as_uuid(),
            recibo: r.recibo.into(),
            aplicado: aplicados(r.aplicado),
            anticipo: pesos(r.anticipo),
            descuento: r.descuento,
        }
    }
}

/// Recaudo desde el extracto por referencia de pago.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RecaudoReferenciaPeticion {
    pub cuenta_tesoreria_id: Uuid,
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
}

/// Resultado del recaudo desde el extracto.
#[derive(Debug, Serialize, ToSchema)]
pub struct RecaudoExtractoRespuesta {
    pub recaudos: Vec<RecaudoRespuesta>,
    /// Créditos cuya referencia no es de ningún bien.
    pub sin_identificar: usize,
}

impl From<RecaudoExtracto> for RecaudoExtractoRespuesta {
    fn from(r: RecaudoExtracto) -> Self {
        Self {
            recaudos: r.recaudos.into_iter().map(Into::into).collect(),
            sin_identificar: r.sin_identificar,
        }
    }
}

/// Aplicación de lo sin aplicar de un recibo a un bien.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AplicacionBienPeticion {
    pub bien_privado_id: Uuid,
    /// Fecha contable (por defecto, hoy).
    pub fecha: Option<NaiveDate>,
}

/// Cobro manual.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoCobroPeticion {
    pub bien_privado_id: Uuid,
    pub concepto_cobro_id: Uuid,
    #[schema(value_type = String, example = "150000")]
    pub valor: Monto,
    pub fecha: NaiveDate,
    pub vencimiento: NaiveDate,
    #[schema(example = "Multa por ruido (acta del comité de convivencia 12)")]
    pub descripcion: String,
}

/// Nota crédito sobre una obligación.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NotaCreditoPeticion {
    #[schema(value_type = String, example = "20000")]
    pub valor: Monto,
    #[schema(example = "Rebaja aprobada por el consejo")]
    pub motivo: String,
    /// Fecha contable (por defecto, hoy).
    pub fecha: Option<NaiveDate>,
}

/// Ajuste de una obligación.
#[derive(Debug, Serialize, ToSchema)]
pub struct AjusteRespuesta {
    pub id: Uuid,
    pub obligacion_id: Uuid,
    #[schema(value_type = crate::esquemas::TipoAjuste)]
    pub tipo: TipoAjuste,
    #[schema(value_type = String)]
    pub valor: Monto,
    pub motivo: String,
    pub comprobante_id: Uuid,
    pub recibo_id: Option<Uuid>,
    pub creado_en: DateTime<Utc>,
}

impl From<AjusteObligacion> for AjusteRespuesta {
    fn from(a: AjusteObligacion) -> Self {
        Self {
            id: a.id.as_uuid(),
            obligacion_id: a.obligacion_id.as_uuid(),
            tipo: a.tipo,
            valor: a.valor,
            motivo: a.motivo,
            comprobante_id: a.comprobante_id.as_uuid(),
            recibo_id: a.recibo_id.map(|r| r.as_uuid()),
            creado_en: a.creado_en,
        }
    }
}

/// Fila de la cartera inicial.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FilaCarteraInicialPeticion {
    /// Código del bien privado.
    #[schema(example = "T1-101")]
    pub bien: String,
    /// Código del concepto de cobro.
    #[schema(example = "administracion")]
    pub concepto: String,
    pub periodo: NaiveDate,
    #[schema(value_type = String, example = "350000")]
    pub valor: Monto,
    pub vencimiento: NaiveDate,
}

/// Cartera inicial (deuda por bien, concepto y antigüedad).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CarteraInicialPeticion {
    pub filas: Vec<FilaCarteraInicialPeticion>,
    /// Solo valida (vista previa).
    #[serde(default)]
    pub solo_validar: bool,
}

/// Error de una fila.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorFilaRespuesta {
    /// Número de fila desde 1.
    pub fila: usize,
    pub mensaje: String,
}

/// Resultado de la cartera inicial.
#[derive(Debug, Serialize, ToSchema)]
pub struct CarteraInicialRespuesta {
    pub filas: usize,
    #[schema(value_type = String)]
    pub total: Decimal,
    /// Con errores no se carga nada.
    pub errores: Vec<ErrorFilaRespuesta>,
    pub cargada: bool,
}

impl From<CarteraInicial> for CarteraInicialRespuesta {
    fn from(c: CarteraInicial) -> Self {
        Self {
            filas: c.filas,
            total: pesos(c.total),
            errores: c
                .errores
                .into_iter()
                .map(|(fila, mensaje)| ErrorFilaRespuesta { fila, mensaje })
                .collect(),
            cargada: c.cargada,
        }
    }
}

/// Pago aplicado a una obligación.
#[derive(Debug, Serialize, ToSchema)]
pub struct PagoAplicadoRespuesta {
    pub recibo_id: Uuid,
    pub fecha: NaiveDate,
    pub obligacion_id: Uuid,
    #[schema(value_type = String)]
    pub valor: Monto,
}

impl From<PagoAplicado> for PagoAplicadoRespuesta {
    fn from(p: PagoAplicado) -> Self {
        Self {
            recibo_id: p.recibo_id.as_uuid(),
            fecha: p.fecha,
            obligacion_id: p.obligacion_id.as_uuid(),
            valor: p.valor,
        }
    }
}

/// Estado de cuenta de un bien.
#[derive(Debug, Serialize, ToSchema)]
pub struct EstadoCuentaRespuesta {
    pub bien: BienCarteraRespuesta,
    pub corte: NaiveDate,
    pub obligaciones: Vec<ObligacionRespuesta>,
    pub pagos: Vec<PagoAplicadoRespuesta>,
    #[schema(value_type = String)]
    pub saldo: Decimal,
    #[schema(value_type = String)]
    pub vencido: Decimal,
    /// Dinero sin aplicar de los recibos del pagador y del deudor.
    #[schema(value_type = String)]
    pub anticipos: Decimal,
}

impl From<EstadoCuenta> for EstadoCuentaRespuesta {
    fn from(e: EstadoCuenta) -> Self {
        Self {
            bien: e.bien.into(),
            corte: e.corte,
            obligaciones: e.obligaciones.into_iter().map(Into::into).collect(),
            pagos: e.pagos.into_iter().map(Into::into).collect(),
            saldo: pesos(e.saldo),
            vencido: pesos(e.vencido),
            anticipos: pesos(e.anticipos),
        }
    }
}

/// Saldo de un bien.
#[derive(Debug, Serialize, ToSchema)]
pub struct SaldoBienRespuesta {
    pub bien_privado_id: Uuid,
    pub codigo: String,
    pub deudor_persona_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub saldo: Decimal,
    #[schema(value_type = String)]
    pub vencido: Decimal,
}

impl From<SaldoBien> for SaldoBienRespuesta {
    fn from(s: SaldoBien) -> Self {
        Self {
            bien_privado_id: s.bien_id.as_uuid(),
            codigo: s.codigo,
            deudor_persona_id: s.deudor_persona_id.map(|p| p.as_uuid()),
            saldo: pesos(s.saldo),
            vencido: pesos(s.vencido),
        }
    }
}

/// Cuadre de una cuenta de cartera.
#[derive(Debug, Serialize, ToSchema)]
pub struct CuadreCuentaRespuesta {
    pub cuenta_contable_id: Uuid,
    pub codigo: String,
    pub nombre: String,
    #[schema(value_type = String)]
    pub saldo_mayor: Decimal,
    #[schema(value_type = String)]
    pub saldo_cartera: Decimal,
    /// Mayor − cartera (cero si cuadra).
    #[schema(value_type = String)]
    pub diferencia: Decimal,
}

impl From<CuadreCuenta> for CuadreCuentaRespuesta {
    fn from(c: CuadreCuenta) -> Self {
        Self {
            cuenta_contable_id: c.cuenta_id.as_uuid(),
            codigo: c.codigo,
            nombre: c.nombre,
            saldo_mayor: pesos(c.saldo_mayor),
            saldo_cartera: pesos(c.saldo_cartera),
            diferencia: pesos(c.diferencia),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Mora, acuerdos, edades y certificados (fase 5)
// ---------------------------------------------------------------------------------------------

/// Tasa de referencia nueva (interés bancario corriente certificado).
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaTasaPeticion {
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: NaiveDate,
    /// Efectiva anual, en %.
    #[schema(value_type = String, example = "18.95")]
    pub tasa_efectiva_anual: Decimal,
    #[schema(example = "Resolución 1234 de 2026")]
    pub resolucion: String,
    #[schema(example = "Superintendencia Financiera de Colombia")]
    pub fuente: String,
}

/// Tasa de referencia.
#[derive(Debug, Serialize, ToSchema)]
pub struct TasaReferenciaRespuesta {
    pub id: Uuid,
    #[schema(value_type = crate::esquemas::ModalidadTasa)]
    pub modalidad: ModalidadTasa,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: NaiveDate,
    #[schema(value_type = String)]
    pub tasa_efectiva_anual: Decimal,
    pub resolucion: String,
    pub fuente: String,
    pub creada_en: DateTime<Utc>,
}

impl From<TasaReferencia> for TasaReferenciaRespuesta {
    fn from(t: TasaReferencia) -> Self {
        Self {
            id: t.id.as_uuid(),
            modalidad: t.modalidad,
            vigente_desde: t.vigente_desde,
            vigente_hasta: t.vigente_hasta,
            tasa_efectiva_anual: t.tasa_efectiva_anual,
            resolucion: t.resolucion,
            fuente: t.fuente,
            creada_en: t.creada_en,
        }
    }
}

/// Fecha hasta la que se causan los intereses (inclusive).
#[derive(Debug, Deserialize, ToSchema)]
pub struct HastaPeticion {
    #[schema(example = "2026-11-30")]
    pub hasta: NaiveDate,
}

/// Tramo causado (base, días y tasa).
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleInteresRespuesta {
    pub obligacion_id: Uuid,
    pub obligacion_interes_id: Uuid,
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    pub dias: u32,
    #[schema(value_type = String)]
    pub base: Decimal,
    /// Tasa de mora aplicada (efectiva anual, %).
    #[schema(value_type = String)]
    pub tasa_efectiva_anual: Decimal,
    pub tasa_referencia_id: Uuid,
    #[schema(value_type = String)]
    pub valor: Decimal,
}

impl From<DetalleInteres> for DetalleInteresRespuesta {
    fn from(d: DetalleInteres) -> Self {
        Self {
            obligacion_id: d.obligacion_id.as_uuid(),
            obligacion_interes_id: d.obligacion_interes_id.as_uuid(),
            desde: d.desde,
            hasta: d.hasta,
            dias: d.dias,
            base: pesos(d.base),
            tasa_efectiva_anual: d.tasa_efectiva_anual,
            tasa_referencia_id: d.tasa_referencia_id.as_uuid(),
            valor: pesos(d.valor),
        }
    }
}

/// Intereses de un bien.
#[derive(Debug, Serialize, ToSchema)]
pub struct InteresesBienRespuesta {
    pub bien_privado_id: Uuid,
    pub bien_codigo: String,
    pub deudor_persona_id: Uuid,
    #[schema(value_type = String)]
    pub total: Decimal,
    pub detalle: Vec<DetalleInteresRespuesta>,
}

/// Cálculo de intereses (vista previa o lo causado).
#[derive(Debug, Serialize, ToSchema)]
pub struct CalculoMoraRespuesta {
    pub periodo: NaiveDate,
    pub hasta: NaiveDate,
    #[schema(value_type = String)]
    pub total: Decimal,
    pub bienes: Vec<InteresesBienRespuesta>,
}

impl From<CalculoMora> for CalculoMoraRespuesta {
    fn from(c: CalculoMora) -> Self {
        Self {
            periodo: c.periodo,
            hasta: c.hasta,
            total: pesos(c.total),
            bienes: c
                .bienes
                .into_iter()
                .map(|b| InteresesBienRespuesta {
                    bien_privado_id: b.bien_id.as_uuid(),
                    bien_codigo: b.bien_codigo,
                    deudor_persona_id: b.deudor.as_uuid(),
                    total: pesos(b.total),
                    detalle: b.detalle.into_iter().map(Into::into).collect(),
                })
                .collect(),
        }
    }
}

/// Causación de intereses.
#[derive(Debug, Serialize, ToSchema)]
pub struct CausacionRespuesta {
    pub id: Uuid,
    pub periodo: NaiveDate,
    pub hasta: NaiveDate,
    #[schema(value_type = crate::esquemas::EstadoCausacion)]
    pub estado: EstadoCausacion,
    #[schema(value_type = String)]
    pub total: Monto,
    pub comprobante_id: Uuid,
    pub motivo_anulacion: Option<String>,
    pub causada_en: DateTime<Utc>,
    pub anulada_en: Option<DateTime<Utc>>,
}

impl From<CausacionMora> for CausacionRespuesta {
    fn from(c: CausacionMora) -> Self {
        Self {
            id: c.id.as_uuid(),
            periodo: c.periodo,
            hasta: c.hasta,
            estado: c.estado,
            total: c.total,
            comprobante_id: c.comprobante_id.as_uuid(),
            motivo_anulacion: c.motivo_anulacion,
            causada_en: c.causada_en,
            anulada_en: c.anulada_en,
        }
    }
}

/// Resultado de causar los intereses.
#[derive(Debug, Serialize, ToSchema)]
pub struct CausadoRespuesta {
    pub causacion: CausacionRespuesta,
    pub calculo: CalculoMoraRespuesta,
    /// Acuerdos que cambiaron de estado al revisarlos.
    pub acuerdos_revisados: usize,
}

impl From<Causado> for CausadoRespuesta {
    fn from(c: Causado) -> Self {
        Self {
            causacion: c.causacion.into(),
            calculo: c.calculo.into(),
            acuerdos_revisados: c.acuerdos_revisados,
        }
    }
}

/// Causación con su detalle.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetalleCausacionRespuesta {
    pub causacion: CausacionRespuesta,
    pub detalle: Vec<DetalleInteresRespuesta>,
}

/// Acuerdo de pago nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoAcuerdoPeticion {
    pub bien_privado_id: Uuid,
    #[schema(example = 6)]
    pub numero_cuotas: u16,
    pub primer_vencimiento: NaiveDate,
    /// Desde cuándo se suspende la mora (por defecto, hoy).
    pub fecha: Option<NaiveDate>,
    pub observaciones: Option<String>,
}

/// Cuota de un acuerdo.
#[derive(Debug, Serialize, ToSchema)]
pub struct CuotaAcuerdoRespuesta {
    pub numero: u16,
    pub vencimiento: NaiveDate,
    #[schema(value_type = String)]
    pub valor: Decimal,
}

/// Acuerdo de pago.
#[derive(Debug, Serialize, ToSchema)]
pub struct AcuerdoRespuesta {
    pub id: Uuid,
    pub bien_privado_id: Uuid,
    pub deudor_persona_id: Uuid,
    pub fecha: NaiveDate,
    #[schema(value_type = String)]
    pub valor: Monto,
    #[schema(value_type = crate::esquemas::EstadoAcuerdo)]
    pub estado: EstadoAcuerdo,
    pub observaciones: Option<String>,
    pub creado_en: DateTime<Utc>,
    pub aprobado_en: Option<DateTime<Utc>>,
    pub cerrado_en: Option<DateTime<Utc>>,
    pub cuotas: Vec<CuotaAcuerdoRespuesta>,
    /// Lo pagado al bien desde la fecha del acuerdo (en los listados).
    #[schema(value_type = Option<String>)]
    pub pagado: Option<Decimal>,
    /// Situación hoy de un acuerdo vigente: `al_dia`, `cumplido` o `incumplido` (aún sin
    /// guardar: se guarda al causar los intereses).
    pub situacion: Option<String>,
}

impl From<AcuerdoPago> for AcuerdoRespuesta {
    fn from(a: AcuerdoPago) -> Self {
        Self {
            id: a.id.as_uuid(),
            bien_privado_id: a.bien_privado_id.as_uuid(),
            deudor_persona_id: a.deudor_persona_id.as_uuid(),
            fecha: a.fecha,
            valor: a.valor,
            estado: a.estado,
            observaciones: a.observaciones,
            creado_en: a.creado_en,
            aprobado_en: a.aprobado_en,
            cerrado_en: a.cerrado_en,
            cuotas: a
                .cuotas
                .into_iter()
                .map(|c| CuotaAcuerdoRespuesta {
                    numero: c.numero,
                    vencimiento: c.vencimiento,
                    valor: pesos(c.valor),
                })
                .collect(),
            pagado: None,
            situacion: None,
        }
    }
}

impl From<AcuerdoConSituacion> for AcuerdoRespuesta {
    fn from(a: AcuerdoConSituacion) -> Self {
        let situacion = a.situacion.map(|s| {
            match s {
                SituacionAcuerdo::AlDia => "al_dia",
                SituacionAcuerdo::Cumplido => "cumplido",
                SituacionAcuerdo::Incumplido { .. } => "incumplido",
            }
            .to_owned()
        });
        Self {
            pagado: Some(pesos(a.pagado)),
            situacion,
            ..a.acuerdo.into()
        }
    }
}

/// Edades de la deuda de un bien (o el total, sin bien).
#[derive(Debug, Serialize, ToSchema)]
pub struct EdadesRespuesta {
    pub bien_privado_id: Option<Uuid>,
    pub codigo: String,
    pub deudor_persona_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub corriente: Decimal,
    #[schema(value_type = String)]
    pub de_1_a_30: Decimal,
    #[schema(value_type = String)]
    pub de_31_a_60: Decimal,
    #[schema(value_type = String)]
    pub de_61_a_90: Decimal,
    #[schema(value_type = String)]
    pub mas_de_90: Decimal,
    #[schema(value_type = String)]
    pub total: Decimal,
}

impl From<EdadesBien> for EdadesRespuesta {
    fn from(e: EdadesBien) -> Self {
        Self {
            bien_privado_id: e.bien_id.map(|b| b.as_uuid()),
            codigo: e.codigo,
            deudor_persona_id: e.deudor_persona_id.map(|p| p.as_uuid()),
            corriente: pesos(e.corriente),
            de_1_a_30: pesos(e.de_1_a_30),
            de_31_a_60: pesos(e.de_31_a_60),
            de_61_a_90: pesos(e.de_61_a_90),
            mas_de_90: pesos(e.mas_de_90),
            total: pesos(e.total),
        }
    }
}

/// Cartera por edades al corte.
#[derive(Debug, Serialize, ToSchema)]
pub struct CarteraEdadesRespuesta {
    pub corte: NaiveDate,
    pub bienes: Vec<EdadesRespuesta>,
    pub total: EdadesRespuesta,
}

/// Certificado por emitir.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoCertificadoPeticion {
    #[schema(value_type = crate::esquemas::TipoCertificado)]
    pub tipo: TipoCertificado,
}

/// Certificado emitido.
#[derive(Debug, Serialize, ToSchema)]
pub struct CertificadoRespuesta {
    pub id: Uuid,
    #[schema(value_type = crate::esquemas::TipoCertificado)]
    pub tipo: TipoCertificado,
    pub bien_privado_id: Uuid,
    pub corte: NaiveDate,
    #[schema(value_type = String)]
    pub saldo: Decimal,
    #[schema(value_type = String)]
    pub vencido: Decimal,
    /// SHA-256 del PDF entregado.
    pub pdf_sha256: String,
    pub emitido_en: DateTime<Utc>,
}

impl From<CertificadoCartera> for CertificadoRespuesta {
    fn from(c: CertificadoCartera) -> Self {
        Self {
            id: c.id.as_uuid(),
            tipo: c.tipo,
            bien_privado_id: c.bien_privado_id.as_uuid(),
            corte: c.corte,
            saldo: pesos(c.saldo),
            vencido: pesos(c.vencido),
            pdf_sha256: c.pdf_sha256,
            emitido_en: c.emitido_en,
        }
    }
}

/// Certificado recién emitido: el código y el PDF se entregan **una sola vez**.
#[derive(Debug, Serialize, ToSchema)]
pub struct CertificadoEmitidoRespuesta {
    pub certificado: CertificadoRespuesta,
    /// Código de verificación (impreso en el PDF; solo se guarda su huella).
    #[schema(example = "8F3A-21C0-9B7E-44D2-A1F0")]
    pub codigo: String,
    /// El PDF en Base64.
    pub pdf_base64: String,
}

impl From<CertificadoEmitido> for CertificadoEmitidoRespuesta {
    fn from(c: CertificadoEmitido) -> Self {
        use base64::{Engine, engine::general_purpose::STANDARD};
        Self {
            certificado: c.certificado.into(),
            codigo: c.codigo,
            pdf_base64: STANDARD.encode(&c.pdf),
        }
    }
}

/// Código por verificar.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VerificacionPeticion {
    #[schema(example = "8F3A-21C0-9B7E-44D2-A1F0")]
    pub codigo: String,
}

/// Resultado de la verificación: solo si es válido, su clase y su corte.
#[derive(Debug, Serialize, ToSchema)]
pub struct VerificacionRespuesta {
    pub valido: bool,
    #[schema(value_type = Option<crate::esquemas::TipoCertificado>)]
    pub tipo: Option<TipoCertificado>,
    pub corte: Option<NaiveDate>,
}

// ---------------------------------------------------------------------------------------------
// Integraciones e informes (fases 6 y 7)
// ---------------------------------------------------------------------------------------------

/// Fecha de corte de los morosos (por defecto, hoy).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CorteMorososPeticion {
    pub corte: Option<NaiveDate>,
}

/// Morosos fijados en la reunión.
#[derive(Debug, Serialize, ToSchema)]
pub struct MorososRespuesta {
    pub marcados: u32,
    /// Bienes con cartera vencida que no están en el padrón.
    pub no_encontrados: Vec<String>,
}

/// Umbral de vencido desde el que se restringen las reservas.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UmbralMoraPeticion {
    #[schema(value_type = String, example = "300000")]
    pub umbral: Monto,
}

/// Resultado de sincronizar las restricciones por mora.
#[derive(Debug, Serialize, ToSchema)]
pub struct SincronizacionMoraRespuesta {
    pub restringidos: usize,
    pub levantadas: usize,
}

/// Estado del fondo de imprevistos.
#[derive(Debug, Serialize, ToSchema)]
pub struct EstadoFondoRespuesta {
    pub anio: i32,
    #[schema(value_type = String)]
    pub liquidado: Decimal,
    #[schema(value_type = String)]
    pub recaudado: Decimal,
    #[schema(value_type = String)]
    pub presupuesto_anual: Decimal,
    /// Liquidado / presupuesto ordinario anual, en %.
    #[schema(value_type = String)]
    pub porcentaje: Decimal,
    /// Llegó al 50 %: la asamblea puede suspender los aportes (art. 35).
    pub llego_a_la_mitad: bool,
    #[schema(value_type = String)]
    pub porcentaje_politica: Decimal,
    pub suspendido: bool,
}

impl From<EstadoFondo> for EstadoFondoRespuesta {
    fn from(f: EstadoFondo) -> Self {
        Self {
            anio: f.anio,
            liquidado: pesos(f.liquidado),
            recaudado: pesos(f.recaudado),
            presupuesto_anual: pesos(f.presupuesto_anual),
            porcentaje: f.porcentaje,
            llego_a_la_mitad: f.llego_a_la_mitad,
            porcentaje_politica: f.porcentaje_politica,
            suspendido: f.suspendido,
        }
    }
}

/// Flujo de caja de un mes.
#[derive(Debug, Serialize, ToSchema)]
pub struct MesFlujoRespuesta {
    pub mes: NaiveDate,
    #[schema(value_type = String)]
    pub saldo_inicial: Decimal,
    #[schema(value_type = String)]
    pub entradas: Decimal,
    #[schema(value_type = String)]
    pub salidas: Decimal,
    #[schema(value_type = String)]
    pub saldo_final: Decimal,
}

impl From<MesFlujo> for MesFlujoRespuesta {
    fn from(m: MesFlujo) -> Self {
        Self {
            mes: m.mes,
            saldo_inicial: pesos(m.saldo_inicial),
            entradas: pesos(m.entradas),
            salidas: pesos(m.salidas),
            saldo_final: pesos(m.saldo_final),
        }
    }
}

/// Proyecto de presupuesto del año siguiente.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ProyectoPresupuestoPeticion {
    /// Año del presupuesto vigente que se copia.
    #[schema(example = 2026)]
    pub anio_base: i32,
    /// Ajuste en % (entre -50 y 100).
    #[schema(value_type = String, example = "9.5")]
    pub ajuste_porcentual: Decimal,
}
