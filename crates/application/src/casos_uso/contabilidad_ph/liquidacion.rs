//! Liquidación de expensas (plan 14 fase 3): cuotas extraordinarias, vista previa y
//! contabilización de la liquidación mensual, cuentas de cobro (inmutables, consecutivo `CB`),
//! su PDF y su envío, y la anulación (reverso) de una liquidación sin pagos.
//!
//! La liquidación usa el presupuesto vigente del año, la política vigente en el periodo y el
//! cuadro de coeficientes vigente en el periodo (QA-008); lo ya liquidado no se recalcula.
//! Reliquidar el mismo periodo no duplica cobros (QA-005: índice único y una liquidación
//! vigente por periodo).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use domain::{
    BienPrivadoId, ConceptoCobroId, CuentaCobroId, CuotaExtraordinariaId, LiquidacionExpensasId,
    ObligacionCobroId, Permiso, PersonaId, PoliticaCarteraId, PreguntaId, PresupuestoId,
    contabilidad::{ImporteDocumento, Monto},
    contabilidad_ph::{
        BienLiquidable, ConceptoLiquidable, CriterioDistribucion, CuotaDelPeriodo,
        DatosLiquidacion, EstadoCuotaExtraordinaria, EstadoLiquidacionExpensas, EstadoObligacion,
        OrigenObligacion, TipoConceptoCobro, cuota_del_periodo, descuento_pronto_pago,
        dia_del_periodo, fondo_llego_a_la_mitad, liquidar, periodo_de,
    },
};
use rust_decimal::Decimal;

use super::configuracion::politica_en;
use super::recaudo::aplicar_recibo_a_bien_en;
use super::{SERIE_CUENTA_COBRO, TxCartera, abrir, eventos, monto, validacion};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{
        avisos::{Aviso, avisar_con_permiso},
        contabilidad::{
            comprobantes::{self, DocumentoOrigen, revertir_en},
            prelude::*,
            tesoreria::origen,
            texto,
        },
    },
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        GeneradorActas, RegistroOutbox as _, Reloj, RepositorioCartera as _, UnidadDeTrabajo,
        actas::TablaPdf,
        cartera::{
            BienCartera, ConceptoCobro, CuentaCobro, CuotaExtraordinaria, FiltroObligaciones,
            LiquidacionExpensas, ObligacionCobro, PoliticaCartera, SaldoObligacion,
        },
        correo::CorreoSaliente,
    },
};

const ACCION_CUOTA: AccionAuditoria = AccionAuditoria::new("cartera.cuota_extraordinaria.crear");
const ACCION_CUOTA_ANULAR: AccionAuditoria =
    AccionAuditoria::new("cartera.cuota_extraordinaria.anular");
const ACCION_LIQUIDAR: AccionAuditoria = AccionAuditoria::new("cartera.liquidacion.contabilizar");
const ACCION_ANULAR: AccionAuditoria = AccionAuditoria::new("cartera.liquidacion.anular");
const ACCION_ENVIAR: AccionAuditoria = AccionAuditoria::new("cartera.cuentas_cobro.enviar");

/// Máximo de liquidaciones en un listado.
pub const LIMITE_LIQUIDACIONES: u32 = 120;

// ---------------------------------------------------------------------------------------------
// Cuotas extraordinarias
// ---------------------------------------------------------------------------------------------

/// Cuota extraordinaria aprobada por la asamblea.
#[derive(Debug, Clone)]
pub struct NuevaCuota {
    /// Concepto de tipo `extraordinaria` (define el reparto y la regla contable).
    pub concepto: ConceptoCobroId,
    pub finalidad: String,
    pub valor_total: Monto,
    pub numero_cuotas: u16,
    /// Mes de la primera cuota (cualquier día del mes).
    pub primer_periodo: NaiveDate,
    pub destinacion_especifica: bool,
    pub pregunta: Option<PreguntaId>,
}

/// Cuotas extraordinarias, las más recientes primero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn cuotas_extraordinarias<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<CuotaExtraordinaria>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    abrir(unidad, reloj, ctx, Permiso::CarteraVer)
        .await?
        .cuotas_extraordinarias()
        .await
}

/// Registra una cuota extraordinaria: se cobra en las liquidaciones de su calendario.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.liquidar`; [`AppError::Validacion`] si el concepto no
/// es extraordinario y activo, el valor es cero o el número de cuotas no está entre 1 y 120;
/// [`AppError::NoEncontrado`].
pub async fn crear_cuota_extraordinaria<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaCuota,
) -> AppResult<CuotaExtraordinaria>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    if nueva.valor_total.es_cero() || !(1..=120).contains(&nueva.numero_cuotas) {
        return Err(validacion(
            "la cuota necesita un valor mayor que cero y de 1 a 120 cuotas",
        ));
    }
    let finalidad = texto("finalidad", &nueva.finalidad, 300)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraLiquidar).await?;
    let concepto = tx
        .conceptos_cobro()
        .await?
        .into_iter()
        .find(|c| c.id == nueva.concepto)
        .ok_or(AppError::NoEncontrado("concepto de cobro"))?;
    if concepto.tipo != TipoConceptoCobro::Extraordinaria || !concepto.activo {
        return Err(validacion(
            "la cuota necesita un concepto activo de tipo `extraordinaria`",
        ));
    }
    let cuota = CuotaExtraordinaria {
        id: CuotaExtraordinariaId::nuevo(),
        concepto_cobro_id: concepto.id,
        finalidad,
        valor_total: nueva.valor_total,
        numero_cuotas: nueva.numero_cuotas,
        primer_periodo: periodo_de(nueva.primer_periodo),
        destinacion_especifica: nueva.destinacion_especifica,
        pregunta_id: nueva.pregunta,
        estado: EstadoCuotaExtraordinaria::Vigente,
        creada_por: ctx.actor_id,
        creada_en: reloj.ahora(),
    };
    tx.insertar_cuota_extraordinaria(&cuota).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("valor_total", cuota.valor_total.valor().to_string())?
        .con("numero_cuotas", cuota.numero_cuotas)?
        .con("primer_periodo", cuota.primer_periodo.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CUOTA, "cuota_extraordinaria", cuota.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(cuota)
}

/// Anula una cuota extraordinaria: las liquidaciones siguientes ya no la cobran (las cuotas ya
/// liquidadas se corrigen con notas crédito).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.liquidar`; [`AppError::NoEncontrado`] si no existe o
/// ya estaba anulada.
pub async fn anular_cuota_extraordinaria<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CuotaExtraordinariaId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraLiquidar).await?;
    if !tx.anular_cuota_extraordinaria(id).await? {
        return Err(AppError::NoEncontrado("cuota extraordinaria vigente"));
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_CUOTA_ANULAR,
        "cuota_extraordinaria",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

// ---------------------------------------------------------------------------------------------
// Cálculo de la liquidación
// ---------------------------------------------------------------------------------------------

/// Cobro previsto de un bien en la liquidación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CobroPrevisto {
    pub bien_id: BienPrivadoId,
    pub bien_codigo: String,
    pub concepto_id: ConceptoCobroId,
    pub concepto_codigo: String,
    pub cuota_id: Option<CuotaExtraordinariaId>,
    /// `liquidacion` o `cargo` (cargo pendiente de una reserva).
    pub origen: OrigenObligacion,
    pub valor: Monto,
    pub deudor: Option<PersonaId>,
    /// Cargo pendiente que lo origina.
    pub cargo: Option<uuid::Uuid>,
}

/// Total de un concepto en la liquidación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotalConcepto {
    pub concepto_id: ConceptoCobroId,
    pub codigo: String,
    pub nombre: String,
    pub total: Decimal,
}

/// Resultado del cálculo (vista previa o lo que se contabiliza).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VistaLiquidacion {
    pub periodo: NaiveDate,
    pub vencimiento: NaiveDate,
    pub politica_id: PoliticaCarteraId,
    pub presupuesto_id: Option<PresupuestoId>,
    pub cobros: Vec<CobroPrevisto>,
    pub por_concepto: Vec<TotalConcepto>,
    pub total: Decimal,
    /// Advertencias que no impiden liquidar (cargos sin concepto de cobro…).
    pub avisos: Vec<String>,
}

fn ultimo_instante(periodo: NaiveDate) -> AppResult<DateTime<Utc>> {
    let siguiente = if periodo.month() == 12 {
        NaiveDate::from_ymd_opt(periodo.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(periodo.year(), periodo.month() + 1, 1)
    };
    siguiente
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|d| d.and_utc() - chrono::Duration::seconds(1))
        .ok_or_else(|| validacion("periodo inválido"))
}

fn liquidable(c: &ConceptoCobro) -> ConceptoLiquidable {
    ConceptoLiquidable {
        id: c.id,
        codigo: c.codigo.clone(),
        tipo: c.tipo,
        criterio: c.criterio,
        modulo: c.modulo_id,
        valor_fijo: c.valor_fijo.map(Monto::valor),
        tarifas: c.tarifas.iter().map(|(t, v)| (*t, v.valor())).collect(),
    }
}

/// Lo que el cálculo deja listo para contabilizar.
struct Calculo {
    vista: VistaLiquidacion,
    politica: PoliticaCartera,
    conceptos: Vec<ConceptoCobro>,
    bienes: Vec<BienCartera>,
    presupuesto_ordinario_anual: Decimal,
}

async fn calcular<T: TxCartera>(tx: &mut T, periodo: NaiveDate) -> AppResult<Calculo> {
    let periodo = periodo_de(periodo);
    let fin = ultimo_instante(periodo)?;
    let politica = politica_en(tx, fin.date_naive()).await?;
    let conceptos: Vec<ConceptoCobro> = tx
        .conceptos_cobro()
        .await?
        .into_iter()
        .filter(|c| c.activo)
        .collect();
    let cuotas: Vec<CuotaExtraordinaria> = tx
        .cuotas_extraordinarias()
        .await?
        .into_iter()
        .filter(|c| c.estado == EstadoCuotaExtraordinaria::Vigente)
        .collect();
    let redondeo = politica.parametros.redondeo;
    let cuotas_del_periodo: Vec<CuotaDelPeriodo> = cuotas
        .iter()
        .filter_map(|c| {
            cuota_del_periodo(
                c.valor_total.valor(),
                c.numero_cuotas,
                c.primer_periodo,
                periodo,
                redondeo,
            )
            .map(|valor| CuotaDelPeriodo {
                id: c.id,
                concepto: c.concepto_cobro_id,
                valor,
            })
        })
        .collect();
    let usa_presupuesto = conceptos
        .iter()
        .any(|c| c.tipo == TipoConceptoCobro::Ordinaria && c.criterio.reparte_total());
    let presupuesto = tx.presupuesto_vigente(periodo.year()).await?;
    if usa_presupuesto && presupuesto.is_none() {
        return Err(validacion(format!(
            "no hay un presupuesto vigente para {}: apruébelo antes de liquidar",
            periodo.year()
        )));
    }
    let mes = periodo.month0() as usize;
    let mut presupuesto_mes: BTreeMap<ConceptoCobroId, Decimal> = BTreeMap::new();
    let mut presupuesto_ordinario_anual = Decimal::ZERO;
    for r in presupuesto.iter().flat_map(|p| &p.rubros) {
        *presupuesto_mes.entry(r.concepto_cobro_id).or_default() +=
            r.valores_mensuales[mes].valor();
        presupuesto_ordinario_anual += r.valor_anual();
    }
    let bienes = tx.bienes_cartera().await?;
    let usa_coeficiente = conceptos.iter().any(|c| {
        c.criterio == CriterioDistribucion::Coeficiente
            && matches!(
                c.tipo,
                TipoConceptoCobro::Ordinaria | TipoConceptoCobro::Extraordinaria
            )
    });
    let coeficientes = match tx.coeficientes_en(None, fin).await? {
        Some(c) => c,
        None if usa_coeficiente => {
            return Err(validacion(
                "no hay un cuadro de coeficientes vigente en el periodo",
            ));
        }
        None => BTreeMap::new(),
    };
    let mut coeficientes_modulo = BTreeMap::new();
    for m in conceptos
        .iter()
        .filter_map(|c| c.modulo_id)
        .collect::<BTreeSet<_>>()
    {
        if let Some(c) = tx.coeficientes_en(Some(m), fin).await? {
            coeficientes_modulo.insert(m, c);
        }
    }
    let datos = DatosLiquidacion {
        bienes: bienes
            .iter()
            .filter(|b| b.activo)
            .map(|b| BienLiquidable {
                id: b.id,
                tipo: b.tipo,
                area_m2: b.area_m2,
            })
            .collect(),
        conceptos: conceptos.iter().map(liquidable).collect(),
        presupuesto_mes,
        coeficientes,
        coeficientes_modulo,
        cuotas: cuotas_del_periodo,
        redondeo,
        porcentaje_fondo: politica.parametros.fondo_a_liquidar(),
    };
    let por_bien: BTreeMap<BienPrivadoId, &BienCartera> =
        bienes.iter().map(|b| (b.id, b)).collect();
    let mut cobros = Vec::new();
    let mut avisos = Vec::new();
    for c in liquidar(&datos)? {
        cobros.push((
            c.bien,
            c.concepto,
            c.cuota,
            OrigenObligacion::Liquidacion,
            c.valor,
            None,
        ));
    }
    let mut sin_concepto = BTreeMap::<&str, usize>::new();
    for cargo in tx.cargos_pendientes().await? {
        let tipo = if cargo.concepto == "danos" {
            TipoConceptoCobro::Dano
        } else {
            TipoConceptoCobro::Multa
        };
        match conceptos.iter().find(|c| c.tipo == tipo) {
            Some(c) => cobros.push((
                cargo.bien_privado_id,
                c.id,
                None,
                OrigenObligacion::Cargo,
                cargo.valor.valor(),
                Some(cargo.id),
            )),
            None => *sin_concepto.entry(tipo.as_str()).or_default() += 1,
        }
    }
    for (tipo, n) in sin_concepto {
        avisos.push(format!(
            "{n} cargos de reservas quedan pendientes: cree un concepto de cobro de tipo `{tipo}`"
        ));
    }
    let mut previstos = Vec::with_capacity(cobros.len());
    for (bien, concepto, cuota, origen_obligacion, valor, cargo) in cobros {
        let b = por_bien
            .get(&bien)
            .ok_or(AppError::NoEncontrado("bien privado"))?;
        let c = conceptos
            .iter()
            .find(|x| x.id == concepto)
            .ok_or(AppError::NoEncontrado("concepto de cobro"))?;
        previstos.push(CobroPrevisto {
            bien_id: bien,
            bien_codigo: b.codigo.as_str().to_owned(),
            concepto_id: concepto,
            concepto_codigo: c.codigo.clone(),
            cuota_id: cuota,
            origen: origen_obligacion,
            valor: monto(valor)?,
            deudor: b.deudor_persona_id,
            cargo,
        });
    }
    let mut por_concepto: Vec<TotalConcepto> = Vec::new();
    for p in &previstos {
        match por_concepto
            .iter_mut()
            .find(|t| t.concepto_id == p.concepto_id)
        {
            Some(t) => t.total += p.valor.valor(),
            None => {
                let c = conceptos.iter().find(|c| c.id == p.concepto_id);
                por_concepto.push(TotalConcepto {
                    concepto_id: p.concepto_id,
                    codigo: p.concepto_codigo.clone(),
                    nombre: c.map(|c| c.nombre.clone()).unwrap_or_default(),
                    total: p.valor.valor(),
                });
            }
        }
    }
    let total = previstos.iter().map(|p| p.valor.valor()).sum();
    let vista = VistaLiquidacion {
        periodo,
        vencimiento: dia_del_periodo(periodo, politica.parametros.dia_vencimiento)?,
        politica_id: politica.id,
        presupuesto_id: presupuesto.map(|p| p.id),
        cobros: previstos,
        por_concepto,
        total,
        avisos,
    };
    Ok(Calculo {
        vista,
        politica,
        conceptos,
        bienes,
        presupuesto_ordinario_anual,
    })
}

/// Calcula la liquidación del periodo sin guardar nada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.liquidar`; [`AppError::Validacion`] sin política,
/// presupuesto o cuadro vigentes; [`AppError::Dominio`] si un concepto no se puede repartir.
pub async fn vista_previa_liquidacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    periodo: NaiveDate,
) -> AppResult<VistaLiquidacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraLiquidar).await?;
    Ok(calcular(&mut tx, periodo).await?.vista)
}

// ---------------------------------------------------------------------------------------------
// Contabilizar y anular
// ---------------------------------------------------------------------------------------------

/// Lo que dejó una liquidación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Liquidado {
    pub liquidacion: LiquidacionExpensas,
    pub cuentas_cobro: Vec<CuentaCobro>,
    /// Recibos con dinero sin aplicar (anticipos) que se aplicaron a lo liquidado.
    pub anticipos_aplicados: usize,
    /// Se avisó que el fondo de imprevistos llegó a la mitad del presupuesto (art. 35).
    pub aviso_fondo: bool,
    pub avisos: Vec<String>,
}

/// Liquida y contabiliza el periodo en una transacción: obligaciones de cada bien (cargos de
/// reservas incluidos), un comprobante (evento [`eventos::LIQUIDACION`], una línea por concepto
/// y deudor), una cuenta de cobro por bien y la aplicación de los anticipos.
///
/// # Errors
///
/// Los de [`vista_previa_liquidacion`]; [`AppError::Conflicto`] si el periodo ya está liquidado;
/// [`AppError::Validacion`] si no hay nada que liquidar o un bien con cobros no tiene
/// propietario representante; los del contabilizador (regla del evento, periodo abierto).
pub async fn liquidar_expensas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    periodo: NaiveDate,
) -> AppResult<Liquidado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraLiquidar).await?;
    let periodo = periodo_de(periodo);
    if tx.liquidacion_del_periodo(periodo).await?.is_some() {
        return Err(AppError::Conflicto(format!(
            "el periodo {periodo} ya está liquidado: anule esa liquidación para repetirla"
        )));
    }
    let Calculo {
        vista,
        politica,
        conceptos,
        bienes,
        presupuesto_ordinario_anual,
    } = calcular(&mut tx, periodo).await?;
    if vista.cobros.is_empty() {
        return Err(validacion("no hay nada que liquidar en el periodo"));
    }
    let sin_deudor: BTreeSet<&str> = vista
        .cobros
        .iter()
        .filter(|c| c.deudor.is_none())
        .map(|c| c.bien_codigo.as_str())
        .collect();
    if !sin_deudor.is_empty() {
        let lista: Vec<&str> = sin_deudor.iter().take(10).copied().collect();
        return Err(validacion(format!(
            "{} bienes no tienen propietario representante: {}",
            sin_deudor.len(),
            lista.join(", ")
        )));
    }
    let ahora = reloj.ahora();
    let id = LiquidacionExpensasId::nuevo();
    let saldos_previos = saldos_por_bien(&mut tx).await?;
    let fondo_antes = tx
        .total_liquidado(TipoConceptoCobro::FondoImprevistos)
        .await?;

    let mut por_bien: BTreeMap<BienPrivadoId, Vec<&CobroPrevisto>> = BTreeMap::new();
    for c in &vista.cobros {
        por_bien.entry(c.bien_id).or_default().push(c);
    }
    let mut cuentas = Vec::with_capacity(por_bien.len());
    let mut obligaciones = Vec::with_capacity(vista.cobros.len());
    let mut cargos = Vec::new();
    for (bien, cobros) in &por_bien {
        let b = bienes
            .iter()
            .find(|x| x.id == *bien)
            .ok_or(AppError::NoEncontrado("bien privado"))?;
        let deudor = b
            .deudor_persona_id
            .ok_or_else(|| validacion("bien sin propietario representante"))?;
        let cargos_bien: Decimal = cobros.iter().map(|c| c.valor.valor()).sum();
        let ordinario: Decimal = cobros
            .iter()
            .filter(|c| {
                conceptos
                    .iter()
                    .any(|x| x.id == c.concepto_id && x.tipo == TipoConceptoCobro::Ordinaria)
            })
            .map(|c| c.valor.valor())
            .sum();
        let pronto = match politica.parametros.pronto_pago {
            Some((porcentaje, dia)) if ordinario > Decimal::ZERO => Some((
                dia_del_periodo(periodo, dia)?,
                monto(descuento_pronto_pago(porcentaje, ordinario))?,
            )),
            _ => None,
        }
        .filter(|(_, d)| !d.es_cero());
        let saldo_anterior = saldos_previos.get(bien).copied().unwrap_or_default();
        let cuenta = CuentaCobro {
            id: CuentaCobroId::nuevo(),
            anio: periodo.year(),
            numero: tx
                .siguiente_numero(SERIE_CUENTA_COBRO, periodo.year())
                .await?,
            liquidacion_id: id,
            bien_privado_id: *bien,
            periodo,
            deudor_persona_id: deudor,
            saldo_anterior,
            cargos: monto(cargos_bien)?,
            total: saldo_anterior + cargos_bien,
            vencimiento: vista.vencimiento,
            pronto_pago_hasta: pronto.map(|p| p.0),
            descuento_pronto_pago: pronto.map(|p| p.1),
            referencia_pago: b.referencia_pago.clone(),
            creada_en: ahora,
        };
        for c in cobros {
            let o = ObligacionCobro {
                id: ObligacionCobroId::nuevo(),
                bien_privado_id: *bien,
                concepto_cobro_id: c.concepto_id,
                periodo,
                valor: c.valor,
                vencimiento: vista.vencimiento,
                deudor_persona_id: deudor,
                origen: c.origen,
                liquidacion_id: Some(id),
                cuota_extraordinaria_id: c.cuota_id,
                cuenta_cobro_id: Some(cuenta.id),
                comprobante_id: None,
                descripcion: None,
                estado: EstadoObligacion::Vigente,
                creada_en: ahora,
                anulada_en: None,
            };
            if let Some(cargo) = c.cargo {
                cargos.push((cargo, o.id));
            }
            obligaciones.push(o);
        }
        cuentas.push(cuenta);
    }

    let comprobante_id = comprobantes::contabilizar_documento(
        &mut tx,
        reloj,
        ctx,
        DocumentoOrigen {
            evento: eventos::LIQUIDACION.into(),
            origen: origen(eventos::LIQUIDACION, id.as_uuid()),
            fecha: periodo,
            descripcion: format!("Liquidación de expensas {}", periodo.format("%Y-%m")),
            importes: importes_por_deudor(&obligaciones, &conceptos)?,
        },
    )
    .await?;
    let liquidacion = LiquidacionExpensas {
        id,
        periodo,
        estado: EstadoLiquidacionExpensas::Contabilizada,
        presupuesto_id: vista.presupuesto_id,
        politica_id: politica.id,
        total: monto(vista.total)?,
        vencimiento: vista.vencimiento,
        comprobante_id,
        motivo_anulacion: None,
        liquidada_por: ctx.actor_id,
        liquidada_en: ahora,
        anulada_en: None,
    };
    tx.insertar_liquidacion(&liquidacion).await?;
    tx.insertar_cuentas_cobro(&cuentas).await?;
    tx.insertar_obligaciones(&obligaciones).await?;
    for (cargo, obligacion) in &cargos {
        tx.aplicar_cargo(*cargo, *obligacion).await?;
    }

    let anticipos_aplicados =
        aplicar_anticipos(&mut tx, reloj, ctx, &bienes, &por_bien, periodo).await?;
    let fondo_despues = tx
        .total_liquidado(TipoConceptoCobro::FondoImprevistos)
        .await?;
    let aviso_fondo = !fondo_llego_a_la_mitad(fondo_antes, presupuesto_ordinario_anual)
        && fondo_llego_a_la_mitad(fondo_despues, presupuesto_ordinario_anual);
    if aviso_fondo {
        avisar_con_permiso(
            &mut tx,
            Permiso::CarteraConfigurar,
            &Aviso {
                prioridad: PrioridadSalida::Evento,
                asunto: "El fondo de imprevistos llegó al 50 % del presupuesto".into(),
                texto: format!(
                    "Lo liquidado al fondo de imprevistos ({fondo_despues}) llegó a la mitad del \
                     presupuesto ordinario anual ({presupuesto_ordinario_anual}). La asamblea \
                     puede suspender los aportes (Ley 675, art. 35)."
                ),
                plantilla: None,
            },
        )
        .await?;
    }
    let detalle = DetalleAuditoria::vacio()
        .con("periodo", periodo.to_string())?
        .con("total", liquidacion.total.valor().to_string())?
        .con("obligaciones", obligaciones.len())?
        .con("cuentas_cobro", cuentas.len())?
        .con("anticipos_aplicados", anticipos_aplicados)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_LIQUIDAR, "liquidacion_expensas", id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(Liquidado {
        liquidacion,
        cuentas_cobro: cuentas,
        anticipos_aplicados,
        aviso_fondo,
        avisos: vista.avisos,
    })
}

/// Saldo vigente de cada bien (antes de liquidar).
async fn saldos_por_bien<T: TxCartera>(tx: &mut T) -> AppResult<BTreeMap<BienPrivadoId, Decimal>> {
    let mut saldos = BTreeMap::new();
    for s in tx
        .saldos_obligaciones(&FiltroObligaciones {
            con_saldo: true,
            ..FiltroObligaciones::default()
        })
        .await?
    {
        *saldos.entry(s.obligacion.bien_privado_id).or_default() += s.saldo;
    }
    Ok(saldos)
}

/// Un importe por concepto contable y deudor (el tercero queda en el mayor para la exógena).
fn importes_por_deudor(
    obligaciones: &[ObligacionCobro],
    conceptos: &[ConceptoCobro],
) -> AppResult<Vec<ImporteDocumento>> {
    let mut agrupado: BTreeMap<(String, PersonaId), Decimal> = BTreeMap::new();
    for o in obligaciones {
        let concepto = conceptos
            .iter()
            .find(|c| c.id == o.concepto_cobro_id)
            .ok_or(AppError::NoEncontrado("concepto de cobro"))?;
        *agrupado
            .entry((concepto.concepto_contable.clone(), o.deudor_persona_id))
            .or_default() += o.valor.valor();
    }
    agrupado
        .into_iter()
        .map(|((concepto, deudor), valor)| {
            Ok(ImporteDocumento::new(&concepto, monto(valor)?).con_persona(Some(deudor)))
        })
        .collect()
}

/// Aplica los anticipos de los pagadores (o deudores) que pagan un solo bien de la
/// liquidación. Si una persona paga varios bienes, su anticipo se aplica a mano.
async fn aplicar_anticipos<T: TxCartera>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bienes: &[BienCartera],
    liquidados: &BTreeMap<BienPrivadoId, Vec<&CobroPrevisto>>,
    periodo: NaiveDate,
) -> AppResult<usize> {
    let mut bienes_de: BTreeMap<PersonaId, BTreeSet<BienPrivadoId>> = BTreeMap::new();
    for b in bienes {
        for p in [b.pagador_persona_id, b.deudor_persona_id]
            .into_iter()
            .flatten()
        {
            bienes_de.entry(p).or_default().insert(b.id);
        }
    }
    let personas: Vec<PersonaId> = bienes
        .iter()
        .filter(|b| liquidados.contains_key(&b.id))
        .filter_map(BienCartera::pagador)
        .collect();
    let mut aplicados = 0;
    for anticipo in tx.anticipos(&personas).await? {
        let Some(propios) = bienes_de.get(&anticipo.persona_id) else {
            continue;
        };
        if propios.len() != 1 {
            continue;
        }
        let Some(bien) = propios.first().copied() else {
            continue;
        };
        let fecha = anticipo.fecha.max(periodo);
        let aplicado =
            aplicar_recibo_a_bien_en(tx, reloj, ctx, anticipo.recibo_id, bien, fecha).await?;
        if !aplicado.is_empty() {
            aplicados += 1;
        }
    }
    Ok(aplicados)
}

/// Anula una liquidación sin pagos ni ajustes: revierte su comprobante, anula sus obligaciones
/// y devuelve a pendientes los cargos de reservas que incluía. Las cuentas de cobro quedan como
/// historia. Con pagos aplicados se corrige con notas crédito.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.liquidar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya está anulada o tiene pagos o ajustes; los del reverso.
pub async fn anular_liquidacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: LiquidacionExpensasId,
    motivo: &str,
) -> AppResult<LiquidacionExpensas>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let motivo = texto("motivo", motivo, 500)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraLiquidar).await?;
    let mut l = tx
        .liquidacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("liquidación"))?;
    if l.estado == EstadoLiquidacionExpensas::Anulada {
        return Err(AppError::Conflicto("la liquidación ya está anulada".into()));
    }
    if tx.liquidacion_tiene_movimientos(id).await? {
        return Err(AppError::Conflicto(
            "la liquidación tiene pagos o ajustes aplicados: corríjala con notas crédito".into(),
        ));
    }
    revertir_en(&mut tx, reloj, ctx, l.comprobante_id, &motivo, None).await?;
    let ahora = reloj.ahora();
    tx.anular_liquidacion(id, &motivo, ahora).await?;
    l.estado = EstadoLiquidacionExpensas::Anulada;
    l.motivo_anulacion = Some(motivo);
    l.anulada_en = Some(ahora);
    tx.registrar(EventoAuditoria::ok(
        ACCION_ANULAR,
        "liquidacion_expensas",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(l)
}

/// Liquidaciones por periodo descendente.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn liquidaciones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<LiquidacionExpensas>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    abrir(unidad, reloj, ctx, Permiso::CarteraVer)
        .await?
        .liquidaciones(LIMITE_LIQUIDACIONES)
        .await
}

/// Liquidación con sus obligaciones (y su saldo actual).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`; [`AppError::NoEncontrado`].
pub async fn liquidacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: LiquidacionExpensasId,
) -> AppResult<(LiquidacionExpensas, Vec<SaldoObligacion>)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
    let l = tx
        .liquidacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("liquidación"))?;
    let obligaciones = tx
        .saldos_obligaciones(&FiltroObligaciones {
            liquidacion: Some(id),
            ..FiltroObligaciones::default()
        })
        .await?;
    Ok((l, obligaciones))
}

// ---------------------------------------------------------------------------------------------
// Cuentas de cobro
// ---------------------------------------------------------------------------------------------

/// Cuentas de cobro de una liquidación o de un bien, por número.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn cuentas_cobro<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    liquidacion: Option<LiquidacionExpensasId>,
    bien: Option<BienPrivadoId>,
) -> AppResult<Vec<CuentaCobro>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    abrir(unidad, reloj, ctx, Permiso::CarteraVer)
        .await?
        .cuentas_cobro(liquidacion, bien)
        .await
}

/// Número legible de la cuenta de cobro (`CB-2026-00012`).
pub fn numero_cuenta_cobro(c: &CuentaCobro) -> String {
    format!("{SERIE_CUENTA_COBRO}-{}-{:05}", c.anio, c.numero)
}

/// Cuenta de cobro con lo necesario para mostrarla.
struct CuentaArmada {
    cuenta: CuentaCobro,
    entidad: String,
    bien: String,
    deudor: String,
    correo: Option<domain::DireccionCorreo>,
    lineas: Vec<SaldoObligacion>,
}

async fn armar_cuenta<T: TxCartera>(tx: &mut T, cuenta: CuentaCobro) -> AppResult<CuentaArmada> {
    let entidad = tx.datos_entidad().await?;
    let bien = tx
        .bienes_cartera()
        .await?
        .into_iter()
        .find(|b| b.id == cuenta.bien_privado_id)
        .ok_or(AppError::NoEncontrado("bien privado"))?;
    let destinatario = bien.pagador().unwrap_or(cuenta.deudor_persona_id);
    let deudor = tx.persona(cuenta.deudor_persona_id).await?;
    let correo = tx.persona(destinatario).await?.and_then(|p| p.correo);
    let lineas = tx
        .saldos_obligaciones(&FiltroObligaciones {
            cuenta_cobro: Some(cuenta.id),
            ..FiltroObligaciones::default()
        })
        .await?;
    Ok(CuentaArmada {
        entidad: match entidad.nit {
            Some(nit) => format!("{} · NIT {nit}", entidad.nombre),
            None => entidad.nombre,
        },
        bien: bien.codigo.as_str().to_owned(),
        deudor: deudor.map(|p| p.nombre).unwrap_or_default(),
        correo,
        lineas,
        cuenta,
    })
}

fn filas_cuenta(a: &CuentaArmada) -> Vec<Vec<String>> {
    let mut filas: Vec<Vec<String>> = a
        .lineas
        .iter()
        .map(|l| {
            vec![
                l.concepto_nombre.clone(),
                l.obligacion.periodo.format("%Y-%m").to_string(),
                l.obligacion.vencimiento.to_string(),
                l.obligacion.valor.valor().to_string(),
            ]
        })
        .collect();
    filas.push(vec![
        "Saldo anterior".into(),
        String::new(),
        String::new(),
        a.cuenta.saldo_anterior.to_string(),
    ]);
    filas.push(vec![
        "Total a pagar".into(),
        String::new(),
        a.cuenta.vencimiento.to_string(),
        a.cuenta.total.to_string(),
    ]);
    if let (Some(hasta), Some(descuento)) =
        (a.cuenta.pronto_pago_hasta, a.cuenta.descuento_pronto_pago)
    {
        filas.push(vec![
            "Total con pronto pago".into(),
            String::new(),
            hasta.to_string(),
            (a.cuenta.total - descuento.valor()).to_string(),
        ]);
    }
    filas
}

fn subtitulo(a: &CuentaArmada) -> String {
    format!(
        "{} · Bien {} · {} · Periodo {}{}",
        numero_cuenta_cobro(&a.cuenta),
        a.bien,
        a.deudor,
        a.cuenta.periodo.format("%Y-%m"),
        a.cuenta
            .referencia_pago
            .as_deref()
            .map(|r| format!(" · Referencia de pago {r}"))
            .unwrap_or_default()
    )
}

/// La cuenta de cobro en PDF (generada al vuelo: la cuenta es inmutable).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`; [`AppError::NoEncontrado`];
/// [`AppError::Infraestructura`] si falla el PDF.
pub async fn cuenta_cobro_pdf<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CuentaCobroId,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
    G: GeneradorActas,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
    let cuenta = tx
        .cuenta_cobro(id)
        .await?
        .ok_or(AppError::NoEncontrado("cuenta de cobro"))?;
    let armada = armar_cuenta(&mut tx, cuenta).await?;
    drop(tx);
    generador
        .tabla(&TablaPdf {
            titulo: "Cuenta de cobro".into(),
            copropiedad: armada.entidad.clone(),
            subtitulo: subtitulo(&armada),
            columnas: ["Concepto", "Periodo", "Vence", "Valor"]
                .map(str::to_owned)
                .to_vec(),
            filas: filas_cuenta(&armada),
            generada: reloj.ahora().format("%Y-%m-%d %H:%M UTC").to_string(),
        })
        .await
}

/// Resultado del envío de las cuentas de cobro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvioCuentas {
    pub enviadas: usize,
    /// Sin correo del pagador: se entregan por otro medio.
    pub sin_correo: usize,
}

/// Envía por correo (outbox) cada cuenta de cobro de la liquidación a su pagador (el tenedor si
/// lo hay, si no el propietario representante). El saldo va solo en el cuerpo del correo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.liquidar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si la liquidación está anulada.
pub async fn enviar_cuentas_cobro<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    liquidacion: LiquidacionExpensasId,
) -> AppResult<EnvioCuentas>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraLiquidar).await?;
    let l = tx
        .liquidacion(liquidacion)
        .await?
        .ok_or(AppError::NoEncontrado("liquidación"))?;
    if l.estado == EstadoLiquidacionExpensas::Anulada {
        return Err(AppError::Conflicto("la liquidación está anulada".into()));
    }
    let mut envio = EnvioCuentas {
        enviadas: 0,
        sin_correo: 0,
    };
    for cuenta in tx.cuentas_cobro(Some(liquidacion), None).await? {
        let a = armar_cuenta(&mut tx, cuenta).await?;
        let Some(para) = a.correo.clone() else {
            envio.sin_correo += 1;
            continue;
        };
        let mut cuerpo = format!("{}\n{}\n\n", a.entidad, subtitulo(&a));
        for f in filas_cuenta(&a) {
            cuerpo.push_str(&f.join(" | "));
            cuerpo.push('\n');
        }
        tx.encolar(EventoSaliente {
            prioridad: PrioridadSalida::Evento,
            mensaje: MensajeOutbox::Correo(CorreoSaliente {
                para,
                asunto: format!("Cuenta de cobro {}", numero_cuenta_cobro(&a.cuenta)),
                texto: cuerpo,
                html: None,
            }),
        })
        .await?;
        envio.enviadas += 1;
    }
    let detalle = DetalleAuditoria::vacio()
        .con("enviadas", envio.enviadas)?
        .con("sin_correo", envio.sin_correo)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ENVIAR, "liquidacion_expensas", liquidacion.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(envio)
}
