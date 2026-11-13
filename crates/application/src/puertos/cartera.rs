//! Puerto de la vertical contable de la PH (plan 14, ADR 0019): conceptos de cobro, políticas de
//! cartera, módulos de contribución, presupuestos, cuotas extraordinarias, liquidaciones,
//! obligaciones de cada bien privado, cuentas de cobro y ajustes.
//!
//! Es de la vertical: usa el núcleo contable y la tesorería (recibos, aplicaciones de pago), que
//! nunca lo conocen.

use std::{collections::BTreeMap, future::Future};

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
        EstadoObligacion, EstadoPresupuesto, OrigenObligacion, ParametrosPolitica, TipoAjuste,
        TipoAvisoCartera, TipoConceptoCobro,
        mora::{
            CuotaAcuerdo, EstadoAcuerdo, EstadoCausacion, ModalidadTasa, Reduccion, TipoCertificado,
        },
    },
};
use rust_decimal::Decimal;

use crate::AppResult;

/// Módulo de contribución (conjuntos mixtos, art. 29).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuloContribucion {
    pub id: ModuloContribucionId,
    pub nombre: String,
    pub descripcion: Option<String>,
    pub activo: bool,
    pub creado_en: DateTime<Utc>,
}

/// Concepto de cobro de la cartera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConceptoCobro {
    pub id: ConceptoCobroId,
    pub codigo: String,
    pub nombre: String,
    pub tipo: TipoConceptoCobro,
    pub criterio: CriterioDistribucion,
    /// Solo con el criterio `modulo`.
    pub modulo_id: Option<ModuloContribucionId>,
    /// Valor mensual por bien (criterio `valor_fijo`).
    pub valor_fijo: Option<Monto>,
    /// Valor mensual por tipo de bien (criterio `por_tipo_bien`).
    pub tarifas: Vec<(TipoBienPrivado, Monto)>,
    pub genera_interes: bool,
    /// Concepto de las reglas `ph.*` del contador.
    pub concepto_contable: String,
    pub activo: bool,
    pub creado_en: DateTime<Utc>,
}

/// Versión de la política de cartera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoliticaCartera {
    pub id: PoliticaCarteraId,
    pub version: u32,
    pub vigente_desde: NaiveDate,
    pub parametros: ParametrosPolitica,
    /// Pregunta de la asamblea que la aprobó.
    pub pregunta_id: Option<PreguntaId>,
    pub creada_por: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
}

/// Bien privado visto por la cartera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BienCartera {
    pub id: BienPrivadoId,
    pub codigo: CodigoBienPrivado,
    pub tipo: TipoBienPrivado,
    pub area_m2: Option<Decimal>,
    pub activo: bool,
    pub referencia_pago: Option<String>,
    /// Quien paga (tenedor); `None` = el deudor.
    pub pagador_persona_id: Option<PersonaId>,
    /// Propietario representante vigente: el deudor en el mayor.
    pub deudor_persona_id: Option<PersonaId>,
}

impl BienCartera {
    /// Persona que paga: el pagador o, si no hay, el deudor.
    pub fn pagador(&self) -> Option<PersonaId> {
        self.pagador_persona_id.or(self.deudor_persona_id)
    }
}

/// Rubro de un presupuesto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RubroPresupuesto {
    pub id: RubroPresupuestoId,
    pub concepto_cobro_id: ConceptoCobroId,
    pub cuenta_contable_id: CuentaContableId,
    pub centro_costo_id: Option<CentroCostoId>,
    pub nombre: String,
    /// Enero a diciembre.
    pub valores_mensuales: [Monto; 12],
}

impl RubroPresupuesto {
    /// Suma del año.
    pub fn valor_anual(&self) -> Decimal {
        self.valores_mensuales.iter().map(|v| v.valor()).sum()
    }
}

/// Versión del presupuesto anual con sus rubros.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presupuesto {
    pub id: PresupuestoId,
    pub anio: i32,
    pub version: u32,
    pub estado: EstadoPresupuesto,
    pub descripcion: Option<String>,
    pub pregunta_id: Option<PreguntaId>,
    pub vigente_desde: Option<NaiveDate>,
    pub aprobado_por: Option<CuentaId>,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
    pub rubros: Vec<RubroPresupuesto>,
}

/// Cuota extraordinaria aprobada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuotaExtraordinaria {
    pub id: CuotaExtraordinariaId,
    pub concepto_cobro_id: ConceptoCobroId,
    pub finalidad: String,
    pub valor_total: Monto,
    pub numero_cuotas: u16,
    /// Primer día del mes de la primera cuota.
    pub primer_periodo: NaiveDate,
    pub destinacion_especifica: bool,
    pub pregunta_id: Option<PreguntaId>,
    pub estado: EstadoCuotaExtraordinaria,
    pub creada_por: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
}

/// Liquidación mensual de expensas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquidacionExpensas {
    pub id: LiquidacionExpensasId,
    pub periodo: NaiveDate,
    pub estado: EstadoLiquidacionExpensas,
    pub presupuesto_id: Option<PresupuestoId>,
    pub politica_id: PoliticaCarteraId,
    pub total: Monto,
    pub vencimiento: NaiveDate,
    pub comprobante_id: ComprobanteId,
    pub motivo_anulacion: Option<String>,
    pub liquidada_por: Option<CuentaId>,
    pub liquidada_en: DateTime<Utc>,
    pub anulada_en: Option<DateTime<Utc>>,
}

/// Obligación de cobro de un bien privado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObligacionCobro {
    pub id: ObligacionCobroId,
    pub bien_privado_id: BienPrivadoId,
    pub concepto_cobro_id: ConceptoCobroId,
    pub periodo: NaiveDate,
    pub valor: Monto,
    pub vencimiento: NaiveDate,
    pub deudor_persona_id: PersonaId,
    pub origen: OrigenObligacion,
    pub liquidacion_id: Option<LiquidacionExpensasId>,
    pub cuota_extraordinaria_id: Option<CuotaExtraordinariaId>,
    pub cuenta_cobro_id: Option<CuentaCobroId>,
    /// Solo las manuales.
    pub comprobante_id: Option<ComprobanteId>,
    /// Solo las de intereses: la causación que las creó.
    pub causacion_id: Option<CausacionMoraId>,
    pub descripcion: Option<String>,
    pub estado: EstadoObligacion,
    pub creada_en: DateTime<Utc>,
    pub anulada_en: Option<DateTime<Utc>>,
}

/// Obligación con su saldo reconstruido (valor − ajustes − aplicaciones vigentes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaldoObligacion {
    pub obligacion: ObligacionCobro,
    pub concepto_codigo: String,
    pub concepto_nombre: String,
    pub concepto_tipo: TipoConceptoCobro,
    pub concepto_contable: String,
    pub ajustes: Decimal,
    pub aplicado: Decimal,
    pub saldo: Decimal,
}

/// Cuenta de cobro de un bien en una liquidación (inmutable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuentaCobro {
    pub id: CuentaCobroId,
    pub anio: i32,
    /// Consecutivo por año (serie `CB`).
    pub numero: u32,
    pub liquidacion_id: LiquidacionExpensasId,
    pub bien_privado_id: BienPrivadoId,
    pub periodo: NaiveDate,
    pub deudor_persona_id: PersonaId,
    pub saldo_anterior: Decimal,
    pub cargos: Monto,
    pub total: Decimal,
    pub vencimiento: NaiveDate,
    pub pronto_pago_hasta: Option<NaiveDate>,
    pub descuento_pronto_pago: Option<Monto>,
    pub referencia_pago: Option<String>,
    pub creada_en: DateTime<Utc>,
}

/// Ajuste que reduce una obligación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AjusteObligacion {
    pub id: AjusteObligacionId,
    pub obligacion_id: ObligacionCobroId,
    pub tipo: TipoAjuste,
    pub valor: Monto,
    pub motivo: String,
    pub comprobante_id: ComprobanteId,
    /// Recibo que ganó el pronto pago.
    pub recibo_id: Option<ReciboCajaId>,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
    pub anulado_en: Option<DateTime<Utc>>,
}

/// Cargo pendiente de una reserva (plan 04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoPendiente {
    pub id: uuid::Uuid,
    pub bien_privado_id: BienPrivadoId,
    /// `danos` o `penalidad_cancelacion`.
    pub concepto: String,
    pub descripcion: Option<String>,
    pub valor: Monto,
}

/// Aplicación de un recibo a una obligación del bien (estado de cuenta).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagoAplicado {
    pub recibo_id: ReciboCajaId,
    pub fecha: NaiveDate,
    pub obligacion_id: ObligacionCobroId,
    pub valor: Monto,
}

/// Recibo vigente con dinero sin aplicar de una persona identificada (anticipo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anticipo {
    pub recibo_id: ReciboCajaId,
    pub persona_id: PersonaId,
    pub fecha: NaiveDate,
    pub sin_aplicar: Decimal,
}

/// Filtro de obligaciones.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroObligaciones {
    pub bien: Option<BienPrivadoId>,
    pub liquidacion: Option<LiquidacionExpensasId>,
    pub cuenta_cobro: Option<CuentaCobroId>,
    /// Solo las vigentes con saldo.
    pub con_saldo: bool,
    /// Solo hasta este periodo (inclusive).
    pub hasta_periodo: Option<NaiveDate>,
}

/// Tasa de referencia certificada (interés bancario corriente) del catálogo de la plataforma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasaReferencia {
    pub id: TasaReferenciaId,
    pub modalidad: ModalidadTasa,
    pub vigente_desde: NaiveDate,
    /// Inclusive.
    pub vigente_hasta: NaiveDate,
    /// Efectiva anual, en %.
    pub tasa_efectiva_anual: Decimal,
    /// Resolución de la Superintendencia Financiera que la certifica.
    pub resolucion: String,
    pub fuente: String,
    pub creada_en: DateTime<Utc>,
}

/// Causación mensual de intereses de mora.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CausacionMora {
    pub id: CausacionMoraId,
    pub periodo: NaiveDate,
    /// Último día causado (inclusive).
    pub hasta: NaiveDate,
    pub estado: EstadoCausacion,
    pub total: Monto,
    pub comprobante_id: ComprobanteId,
    pub motivo_anulacion: Option<String>,
    pub causada_por: Option<CuentaId>,
    pub causada_en: DateTime<Utc>,
    pub anulada_en: Option<DateTime<Utc>>,
}

/// Tramo causado de una obligación (trazabilidad: base, días y tasa).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetalleInteres {
    pub causacion_id: CausacionMoraId,
    pub obligacion_id: ObligacionCobroId,
    pub obligacion_interes_id: ObligacionCobroId,
    pub desde: NaiveDate,
    /// Inclusive.
    pub hasta: NaiveDate,
    pub dias: u32,
    pub base: Decimal,
    /// Tasa de mora aplicada (efectiva anual, %).
    pub tasa_efectiva_anual: Decimal,
    pub tasa_referencia_id: TasaReferenciaId,
    pub valor: Decimal,
}

/// Acuerdo de pago de un bien con sus cuotas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcuerdoPago {
    pub id: AcuerdoPagoId,
    pub bien_privado_id: BienPrivadoId,
    pub deudor_persona_id: PersonaId,
    /// Desde esta fecha se suspende la mora de lo que se debía.
    pub fecha: NaiveDate,
    pub valor: Monto,
    pub estado: EstadoAcuerdo,
    pub observaciones: Option<String>,
    pub creado_por: CuentaId,
    pub creado_en: DateTime<Utc>,
    pub aprobado_por: Option<CuentaId>,
    pub aprobado_en: Option<DateTime<Utc>>,
    /// Cuándo se cumplió, se incumplió (desde ese día corre la mora) o se anuló.
    pub cerrado_en: Option<DateTime<Utc>>,
    pub cuotas: Vec<CuotaAcuerdo>,
}

/// Certificado de cartera emitido (del código solo queda su huella).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificadoCartera {
    pub id: CertificadoCarteraId,
    pub tipo: TipoCertificado,
    pub bien_privado_id: BienPrivadoId,
    pub corte: NaiveDate,
    pub saldo: Decimal,
    pub vencido: Decimal,
    /// SHA-256 (hex) del código de verificación normalizado.
    pub codigo_hash: String,
    pub pdf_sha256: String,
    pub emitido_por: Option<CuentaId>,
    pub emitido_en: DateTime<Utc>,
}

/// Resumen de la cartera de un bien (lo que muestra el chat).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumenBien {
    pub saldo: Decimal,
    pub vencido: Decimal,
    /// Vencimiento más próximo de lo que tiene saldo.
    pub proximo_vencimiento: Option<NaiveDate>,
    pub referencia_pago: Option<String>,
    pub pagador_persona_id: Option<PersonaId>,
}

/// Consulta mínima de la cartera para otros módulos (el chat): no exige el puerto completo.
pub trait ResumenCartera: Send {
    /// Resumen del bien al día `hoy` (`None` si el bien no existe).
    fn resumen_bien(
        &mut self,
        bien: BienPrivadoId,
        hoy: NaiveDate,
    ) -> impl Future<Output = AppResult<Option<ResumenBien>>> + Send;
}

/// Copropiedades con avisos de cartera pendientes (cruza tenants: SECURITY DEFINER).
pub trait DirectorioCartera: Send + Sync {
    /// Ids de las copropiedades con la cartera habilitada y cuentas de cobro por recordar o por
    /// avisar en mora.
    fn copropiedades_con_avisos(
        &self,
        hoy: NaiveDate,
        dias_antes: u32,
        dias_despues: u32,
    ) -> impl Future<Output = AppResult<Vec<domain::TenantId>>> + Send;
}

/// Lectura del catálogo de tasas de referencia (desde cualquier transacción).
pub trait CatalogoTasas: Send {
    /// Tasas de la modalidad por fecha de vigencia.
    fn tasas_referencia(
        &mut self,
        modalidad: ModalidadTasa,
    ) -> impl Future<Output = AppResult<Vec<TasaReferencia>>> + Send;
}

/// Escritura del catálogo de tasas (solo en la transacción de plataforma).
pub trait CatalogoTasasPlataforma: CatalogoTasas {
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si se solapa con otra tasa de la modalidad.
    fn insertar_tasa_referencia(
        &mut self,
        t: &TasaReferencia,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

/// Persistencia de la cartera de la PH.
pub trait RepositorioCartera: Send {
    /// Módulos de contribución por nombre.
    fn modulos_contribucion(
        &mut self,
    ) -> impl Future<Output = AppResult<Vec<ModuloContribucion>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el nombre ya existe.
    fn guardar_modulo_contribucion(
        &mut self,
        m: &ModuloContribucion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Conceptos de cobro por código, con sus tarifas.
    fn conceptos_cobro(&mut self) -> impl Future<Output = AppResult<Vec<ConceptoCobro>>> + Send;

    /// Inserta o actualiza el concepto (y reemplaza sus tarifas).
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el código ya existe.
    fn guardar_concepto_cobro(
        &mut self,
        c: &ConceptoCobro,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Versiones de la política, la más reciente primero.
    fn politicas_cartera(&mut self)
    -> impl Future<Output = AppResult<Vec<PoliticaCartera>>> + Send;

    /// Guarda una versión nueva.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay una versión con esa fecha.
    fn insertar_politica(
        &mut self,
        p: &PoliticaCartera,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Bienes privados con sus datos de pago y su deudor, por código.
    fn bienes_cartera(&mut self) -> impl Future<Output = AppResult<Vec<BienCartera>>> + Send;

    /// Fija la referencia de pago y el pagador del bien.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si la referencia ya la usa otro bien;
    /// [`crate::AppError::NoEncontrado`] si el bien o la persona no existen.
    fn guardar_datos_pago(
        &mut self,
        bien: BienPrivadoId,
        referencia: Option<&str>,
        pagador: Option<PersonaId>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Coeficientes del cuadro vigente en `momento` (el de mayor `vigente_desde` hasta ese
    /// momento): el de la copropiedad o el del módulo.
    fn coeficientes_en(
        &mut self,
        modulo: Option<ModuloContribucionId>,
        momento: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<BTreeMap<BienPrivadoId, Decimal>>>> + Send;

    /// Presupuestos del año (o todos), del más reciente al más antiguo, con sus rubros.
    fn presupuestos(
        &mut self,
        anio: Option<i32>,
    ) -> impl Future<Output = AppResult<Vec<Presupuesto>>> + Send;

    /// Presupuesto por id, con sus rubros.
    fn presupuesto(
        &mut self,
        id: PresupuestoId,
    ) -> impl Future<Output = AppResult<Option<Presupuesto>>> + Send;

    /// Presupuesto vigente del año, con sus rubros.
    fn presupuesto_vigente(
        &mut self,
        anio: i32,
    ) -> impl Future<Output = AppResult<Option<Presupuesto>>> + Send;

    /// Mayor versión del año (0 si no hay).
    fn ultima_version_presupuesto(
        &mut self,
        anio: i32,
    ) -> impl Future<Output = AppResult<u32>> + Send;

    /// Guarda un presupuesto en borrador con sus rubros.
    fn insertar_presupuesto(
        &mut self,
        p: &Presupuesto,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Reemplaza los rubros de un borrador.
    fn reemplazar_rubros_presupuesto(
        &mut self,
        id: PresupuestoId,
        rubros: &[RubroPresupuesto],
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Pasa el anterior vigente (si hay) a reemplazado y el borrador a vigente.
    fn aprobar_presupuesto(
        &mut self,
        p: &Presupuesto,
        anterior: Option<PresupuestoId>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Cuotas extraordinarias, las más recientes primero.
    fn cuotas_extraordinarias(
        &mut self,
    ) -> impl Future<Output = AppResult<Vec<CuotaExtraordinaria>>> + Send;

    /// Guarda una cuota extraordinaria vigente.
    fn insertar_cuota_extraordinaria(
        &mut self,
        c: &CuotaExtraordinaria,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Marca anulada la cuota (las cuotas ya liquidadas no cambian).
    fn anular_cuota_extraordinaria(
        &mut self,
        id: CuotaExtraordinariaId,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Liquidación por id, bloqueada para actualizar.
    fn liquidacion(
        &mut self,
        id: LiquidacionExpensasId,
    ) -> impl Future<Output = AppResult<Option<LiquidacionExpensas>>> + Send;

    /// Liquidación contabilizada del periodo.
    fn liquidacion_del_periodo(
        &mut self,
        periodo: NaiveDate,
    ) -> impl Future<Output = AppResult<Option<LiquidacionExpensas>>> + Send;

    /// Liquidaciones por periodo descendente, a lo sumo `limite`.
    fn liquidaciones(
        &mut self,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<LiquidacionExpensas>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el periodo ya tiene una liquidación contabilizada.
    fn insertar_liquidacion(
        &mut self,
        l: &LiquidacionExpensas,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Marca anulada la liquidación y sus obligaciones, y devuelve a pendientes sus cargos.
    fn anular_liquidacion(
        &mut self,
        id: LiquidacionExpensasId,
        motivo: &str,
        en: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda obligaciones nuevas.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya existe la obligación liquidada del mismo bien,
    /// concepto, periodo y cuota.
    fn insertar_obligaciones(
        &mut self,
        o: &[ObligacionCobro],
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Marca anulada una obligación.
    fn anular_obligacion(
        &mut self,
        id: ObligacionCobroId,
        en: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Obligaciones con su saldo, por bien, periodo y creación.
    fn saldos_obligaciones(
        &mut self,
        filtro: &FiltroObligaciones,
    ) -> impl Future<Output = AppResult<Vec<SaldoObligacion>>> + Send;

    /// Obligación con su saldo, bloqueada para actualizar.
    fn obligacion(
        &mut self,
        id: ObligacionCobroId,
    ) -> impl Future<Output = AppResult<Option<SaldoObligacion>>> + Send;

    /// Guarda las cuentas de cobro de una liquidación.
    fn insertar_cuentas_cobro(
        &mut self,
        c: &[CuentaCobro],
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Cuentas de cobro de la liquidación o del bien, por número.
    fn cuentas_cobro(
        &mut self,
        liquidacion: Option<LiquidacionExpensasId>,
        bien: Option<BienPrivadoId>,
    ) -> impl Future<Output = AppResult<Vec<CuentaCobro>>> + Send;

    /// Cuenta de cobro por id.
    fn cuenta_cobro(
        &mut self,
        id: CuentaCobroId,
    ) -> impl Future<Output = AppResult<Option<CuentaCobro>>> + Send;

    /// Guarda un ajuste.
    fn insertar_ajuste(
        &mut self,
        a: &AjusteObligacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Ajustes vigentes de un recibo (pronto pago) o de una obligación.
    fn ajustes(
        &mut self,
        recibo: Option<ReciboCajaId>,
        obligacion: Option<ObligacionCobroId>,
    ) -> impl Future<Output = AppResult<Vec<AjusteObligacion>>> + Send;

    /// Marca anulado un ajuste.
    fn anular_ajuste(
        &mut self,
        id: AjusteObligacionId,
        en: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Cargos de las reservas aún pendientes.
    fn cargos_pendientes(&mut self) -> impl Future<Output = AppResult<Vec<CargoPendiente>>> + Send;

    /// Marca el cargo aplicado con su obligación.
    fn aplicar_cargo(
        &mut self,
        cargo: uuid::Uuid,
        obligacion: ObligacionCobroId,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Pagos vigentes aplicados a las obligaciones del bien, por fecha.
    fn pagos_aplicados(
        &mut self,
        bien: BienPrivadoId,
    ) -> impl Future<Output = AppResult<Vec<PagoAplicado>>> + Send;

    /// Recibos vigentes con dinero sin aplicar de las personas dadas, por fecha.
    fn anticipos(
        &mut self,
        personas: &[PersonaId],
    ) -> impl Future<Output = AppResult<Vec<Anticipo>>> + Send;

    /// ¿Hay pagos o ajustes vigentes sobre las obligaciones de la liquidación?
    fn liquidacion_tiene_movimientos(
        &mut self,
        id: LiquidacionExpensasId,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Total vigente liquidado de los conceptos del tipo (p. ej. el fondo de imprevistos).
    fn total_liquidado(
        &mut self,
        tipo: TipoConceptoCobro,
    ) -> impl Future<Output = AppResult<Decimal>> + Send;
    /// Último día causado (inclusive) de cada obligación con causaciones vigentes.
    fn causado_hasta(
        &mut self,
    ) -> impl Future<Output = AppResult<BTreeMap<ObligacionCobroId, NaiveDate>>> + Send;

    /// Pagos aplicados (fecha del recibo) y ajustes vigentes (fecha de creación) de las
    /// obligaciones.
    fn reducciones(
        &mut self,
        obligaciones: &[ObligacionCobroId],
    ) -> impl Future<Output = AppResult<Vec<(ObligacionCobroId, Reduccion)>>> + Send;

    /// Causación vigente del periodo.
    fn causacion_del_periodo(
        &mut self,
        periodo: NaiveDate,
    ) -> impl Future<Output = AppResult<Option<CausacionMora>>> + Send;

    /// Causación por id, bloqueada para actualizar.
    fn causacion(
        &mut self,
        id: CausacionMoraId,
    ) -> impl Future<Output = AppResult<Option<CausacionMora>>> + Send;

    /// Causaciones por periodo descendente, a lo sumo `limite`.
    fn causaciones(
        &mut self,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<CausacionMora>>> + Send;

    /// Detalle (tramos) de una causación.
    fn detalle_causacion(
        &mut self,
        id: CausacionMoraId,
    ) -> impl Future<Output = AppResult<Vec<DetalleInteres>>> + Send;

    /// Guarda la causación (las obligaciones de intereses van aparte, antes del detalle).
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el periodo ya tiene una causación vigente.
    fn insertar_causacion(
        &mut self,
        c: &CausacionMora,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda el detalle de una causación.
    fn insertar_detalle_interes(
        &mut self,
        d: &[DetalleInteres],
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Marca anulada la causación y sus obligaciones de intereses.
    fn anular_causacion(
        &mut self,
        id: CausacionMoraId,
        motivo: &str,
        en: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// ¿Hay pagos o ajustes vigentes sobre los intereses de la causación?
    fn causacion_tiene_movimientos(
        &mut self,
        id: CausacionMoraId,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Acuerdos (del bien, si se da), los más recientes primero, con sus cuotas.
    fn acuerdos(
        &mut self,
        bien: Option<BienPrivadoId>,
    ) -> impl Future<Output = AppResult<Vec<AcuerdoPago>>> + Send;

    /// Acuerdo por id con sus cuotas, bloqueado para actualizar.
    fn acuerdo(
        &mut self,
        id: AcuerdoPagoId,
    ) -> impl Future<Output = AppResult<Option<AcuerdoPago>>> + Send;

    /// Guarda un acuerdo propuesto con sus cuotas.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el bien ya tiene un acuerdo activo.
    fn insertar_acuerdo(&mut self, a: &AcuerdoPago) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda el estado, la aprobación y el cierre del acuerdo.
    fn actualizar_acuerdo(&mut self, a: &AcuerdoPago)
    -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda un certificado emitido.
    fn insertar_certificado(
        &mut self,
        c: &CertificadoCartera,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Certificado por la huella de su código.
    fn certificado_por_codigo(
        &mut self,
        codigo_hash: &str,
    ) -> impl Future<Output = AppResult<Option<CertificadoCartera>>> + Send;

    /// Certificados del bien, los más recientes primero.
    fn certificados(
        &mut self,
        bien: BienPrivadoId,
    ) -> impl Future<Output = AppResult<Vec<CertificadoCartera>>> + Send;

    /// Cuentas de cobro de liquidaciones vigentes por recordar (vencen en los próximos
    /// `dias_antes` días) o por avisar en mora (vencieron hace `dias_despues` días o más, dentro
    /// de los últimos 60), sin aviso de esa clase.
    fn cuentas_por_avisar(
        &mut self,
        hoy: NaiveDate,
        dias_antes: u32,
        dias_despues: u32,
    ) -> impl Future<Output = AppResult<Vec<(CuentaCobro, TipoAvisoCartera)>>> + Send;

    /// Deja constancia del aviso (no se repite).
    fn registrar_aviso(
        &mut self,
        cuenta: CuentaCobroId,
        tipo: TipoAvisoCartera,
        en: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;
}
