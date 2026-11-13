//! Mora, acuerdos y certificados de la cartera de la PH (plan 14 fase 5).
//!
//! - **Tasas de referencia** (plataforma): interés bancario corriente certificado por periodo.
//! - **Causación de intereses**: interés diario sobre el capital vencido de los conceptos que
//!   generan interés, con tramos de tasa y de saldo (QA-010), a la tasa de la política sin
//!   superar 1,5 veces el interés bancario corriente; sin intereses sobre intereses. Queda una
//!   obligación de intereses por bien y el detalle de cada tramo (base, días, tasa) para la
//!   trazabilidad que pide el CTCP. Se contabiliza con el evento [`eventos::INTERESES`].
//! - **Acuerdos de pago**: los aprueba otra cuenta; mientras se cumplen suspenden la mora de lo
//!   que se debía; si una cuota vence sin pagarse, la mora se reanuda. La deuda original no se
//!   borra.
//! - **Cartera por edades** por bien.
//! - **Certificados** (paz y salvo y deuda, Ley 675 art. 48) con un código de verificación del
//!   que solo se guarda la huella.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use domain::{
    AcuerdoPagoId, BienPrivadoId, CausacionMoraId, CertificadoCarteraId, ObligacionCobroId,
    Permiso, PersonaId, TasaReferenciaId, TenantId,
    contabilidad::{ImporteDocumento, tesoreria::TramoEdad},
    contabilidad_ph::{
        EstadoObligacion, OrigenObligacion, TipoConceptoCobro,
        mora::{
            CalculoInteres, EstadoAcuerdo, EstadoCausacion, ModalidadTasa, Reduccion,
            SituacionAcuerdo, TipoCertificado, TramoTasa, codigo_verificacion, cuotas_acuerdo,
            intereses, normalizar_codigo, situacion_acuerdo, tasa_mora_ea,
        },
        periodo_de,
    },
};
use rust_decimal::Decimal;

use super::configuracion::politica_en;
use super::{TxCartera, abrir, eventos, monto, validacion};
use crate::{
    AppError, AppResult, Contexto, ContextoPlataforma,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{
        archivos::sha256_hex,
        contabilidad::{
            comprobantes::{self, DocumentoOrigen, revertir_en},
            prelude::*,
            tesoreria::origen,
            texto, texto_opcional,
        },
    },
    puertos::{
        GeneradorActas, RegistroAuditoria, Reloj, RepositorioCartera as _, UnidadDeTrabajo,
        UnidadDeTrabajoPlataforma,
        actas::TablaPdf,
        cartera::{
            AcuerdoPago, BienCartera, CatalogoTasas, CatalogoTasasPlataforma, CausacionMora,
            CertificadoCartera, DetalleInteres, FiltroObligaciones, ObligacionCobro,
            SaldoObligacion, TasaReferencia,
        },
    },
    seguridad::{hash_secreto, token_aleatorio},
};

const ACCION_TASA: AccionAuditoria = AccionAuditoria::new("plataforma.tasa_referencia.crear");
const ACCION_CAUSAR: AccionAuditoria = AccionAuditoria::new("cartera.intereses.causar");
const ACCION_CAUSACION_ANULAR: AccionAuditoria = AccionAuditoria::new("cartera.intereses.anular");
const ACCION_ACUERDO: AccionAuditoria = AccionAuditoria::new("cartera.acuerdo.proponer");
const ACCION_ACUERDO_APROBAR: AccionAuditoria = AccionAuditoria::new("cartera.acuerdo.aprobar");
const ACCION_ACUERDO_ANULAR: AccionAuditoria = AccionAuditoria::new("cartera.acuerdo.anular");
const ACCION_CERTIFICADO: AccionAuditoria = AccionAuditoria::new("cartera.certificado.emitir");

/// Máximo de causaciones en un listado.
pub const LIMITE_CAUSACIONES: u32 = 120;

// ---------------------------------------------------------------------------------------------
// Tasas de referencia
// ---------------------------------------------------------------------------------------------

/// Tasa de referencia nueva.
#[derive(Debug, Clone)]
pub struct NuevaTasa {
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: NaiveDate,
    /// Interés bancario corriente efectivo anual (%).
    pub tasa_efectiva_anual: Decimal,
    pub resolucion: String,
    pub fuente: String,
}

/// Registra el interés bancario corriente certificado de un periodo (modalidad consumo y
/// ordinario).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`; [`AppError::Validacion`] si la
/// tasa o las fechas no son válidas; [`AppError::Conflicto`] si se solapa con otra.
pub async fn crear_tasa_referencia<UP>(
    plataforma: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    nueva: NuevaTasa,
) -> AppResult<TasaReferencia>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoTasasPlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    if nueva.vigente_hasta < nueva.vigente_desde
        || nueva.tasa_efectiva_anual <= Decimal::ZERO
        || nueva.tasa_efectiva_anual > Decimal::from(200)
    {
        return Err(validacion(
            "la tasa es un porcentaje efectivo anual positivo y la vigencia no puede terminar \
             antes de empezar",
        ));
    }
    let tasa = TasaReferencia {
        id: TasaReferenciaId::nuevo(),
        modalidad: ModalidadTasa::ConsumoOrdinario,
        vigente_desde: nueva.vigente_desde,
        vigente_hasta: nueva.vigente_hasta,
        tasa_efectiva_anual: nueva.tasa_efectiva_anual.round_dp(4),
        resolucion: texto("resolucion", &nueva.resolucion, 120)?,
        fuente: texto("fuente", &nueva.fuente, 300)?,
        creada_en: reloj.ahora(),
    };
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    tx.insertar_tasa_referencia(&tasa).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("vigente_desde", tasa.vigente_desde.to_string())?
        .con("tasa", tasa.tasa_efectiva_anual.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_TASA, "tasa_referencia", tasa.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(tasa)
}

/// Tasas de referencia por vigencia (desde la plataforma).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`.
pub async fn tasas_referencia_plataforma<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<TasaReferencia>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoTasasPlataforma,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    plataforma
        .iniciar_plataforma(ctx)
        .await?
        .tasas_referencia(ModalidadTasa::ConsumoOrdinario)
        .await
}

/// Tasas de referencia por vigencia (desde la copropiedad).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn tasas_referencia<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<TasaReferencia>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    abrir(unidad, reloj, ctx, Permiso::CarteraVer)
        .await?
        .tasas_referencia(ModalidadTasa::ConsumoOrdinario)
        .await
}

// ---------------------------------------------------------------------------------------------
// Acuerdos de pago
// ---------------------------------------------------------------------------------------------

/// Días sin mora de un acuerdo para las obligaciones que vencieron hasta su fecha: desde la
/// fecha hasta el cierre (cumplido o incumplido) o hasta `hasta` si sigue vigente.
fn suspension(a: &AcuerdoPago, hasta: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let fin = match a.estado {
        EstadoAcuerdo::Vigente => hasta,
        EstadoAcuerdo::Cumplido => hasta,
        EstadoAcuerdo::Incumplido => a.cerrado_en?.date_naive().pred_opt()?,
        _ => return None,
    };
    (fin >= a.fecha).then_some((a.fecha, fin))
}

/// Pone al día el estado de los acuerdos vigentes según lo pagado al bien desde su fecha.
async fn revisar_acuerdos_en<T: TxCartera>(tx: &mut T, hoy: NaiveDate) -> AppResult<usize> {
    let mut cambiados = 0;
    for mut a in tx
        .acuerdos(None)
        .await?
        .into_iter()
        .filter(|a| a.estado == EstadoAcuerdo::Vigente)
    {
        let pagos: Vec<(NaiveDate, Decimal)> = tx
            .pagos_aplicados(a.bien_privado_id)
            .await?
            .into_iter()
            .filter(|p| p.fecha >= a.fecha)
            .map(|p| (p.fecha, p.valor.valor()))
            .collect();
        let (estado, cierre) = match situacion_acuerdo(&a.cuotas, &pagos, hoy) {
            SituacionAcuerdo::AlDia => continue,
            SituacionAcuerdo::Cumplido => (EstadoAcuerdo::Cumplido, hoy),
            SituacionAcuerdo::Incumplido { desde } => (EstadoAcuerdo::Incumplido, desde),
        };
        a.estado = a.estado.transicionar(estado)?;
        a.cerrado_en = cierre.and_hms_opt(0, 0, 0).map(|d| d.and_utc());
        tx.actualizar_acuerdo(&a).await?;
        cambiados += 1;
    }
    Ok(cambiados)
}

/// Acuerdo de pago con su situación al día de hoy (sin guardarla).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcuerdoConSituacion {
    pub acuerdo: AcuerdoPago,
    /// Lo pagado al bien desde la fecha del acuerdo.
    pub pagado: Decimal,
    pub situacion: Option<SituacionAcuerdo>,
}

/// Acuerdos (del bien, si se da), los más recientes primero, con su situación hoy.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn acuerdos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: Option<BienPrivadoId>,
) -> AppResult<Vec<AcuerdoConSituacion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let hoy = reloj.ahora().date_naive();
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
    let mut v = Vec::new();
    for a in tx.acuerdos(bien).await? {
        let pagos: Vec<(NaiveDate, Decimal)> = tx
            .pagos_aplicados(a.bien_privado_id)
            .await?
            .into_iter()
            .filter(|p| p.fecha >= a.fecha)
            .map(|p| (p.fecha, p.valor.valor()))
            .collect();
        let situacion =
            (a.estado == EstadoAcuerdo::Vigente).then(|| situacion_acuerdo(&a.cuotas, &pagos, hoy));
        v.push(AcuerdoConSituacion {
            pagado: pagos.iter().map(|p| p.1).sum(),
            situacion,
            acuerdo: a,
        });
    }
    Ok(v)
}

/// Acuerdo de pago propuesto.
#[derive(Debug, Clone)]
pub struct NuevoAcuerdo {
    pub bien: BienPrivadoId,
    pub numero_cuotas: u16,
    pub primer_vencimiento: NaiveDate,
    /// Desde cuándo se suspende la mora (por defecto, hoy).
    pub fecha: Option<NaiveDate>,
    pub observaciones: Option<String>,
}

/// Propone un acuerdo de pago por todo lo que debe el bien, en cuotas mensuales iguales. Queda
/// propuesto hasta que otra cuenta lo apruebe; la deuda original no cambia.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ajustar` o sin cuenta; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si el bien no debe nada, no tiene propietario o las cuotas no son
/// válidas; [`AppError::Conflicto`] si ya tiene un acuerdo activo.
pub async fn proponer_acuerdo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoAcuerdo,
) -> AppResult<AcuerdoPago>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let creado_por = ctx
        .actor_id
        .ok_or(AppError::SinPermiso(Permiso::CarteraAjustar))?;
    let observaciones = texto_opcional("observaciones", nuevo.observaciones.as_deref(), 500)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraAjustar).await?;
    let bien = bien_cartera(&mut tx, nuevo.bien).await?;
    let deudor = bien
        .deudor_persona_id
        .ok_or_else(|| validacion("el bien no tiene propietario representante"))?;
    let saldo: Decimal = tx
        .saldos_obligaciones(&FiltroObligaciones {
            bien: Some(bien.id),
            con_saldo: true,
            ..FiltroObligaciones::default()
        })
        .await?
        .iter()
        .map(|o| o.saldo)
        .sum();
    if saldo <= Decimal::ZERO {
        return Err(validacion("el bien no debe nada"));
    }
    let fecha = nuevo.fecha.unwrap_or_else(|| reloj.ahora().date_naive());
    if nuevo.primer_vencimiento < fecha {
        return Err(validacion(
            "la primera cuota no puede vencer antes de la fecha del acuerdo",
        ));
    }
    let acuerdo = AcuerdoPago {
        id: AcuerdoPagoId::nuevo(),
        bien_privado_id: bien.id,
        deudor_persona_id: deudor,
        fecha,
        valor: monto(saldo)?,
        estado: EstadoAcuerdo::Propuesto,
        observaciones,
        creado_por,
        creado_en: reloj.ahora(),
        aprobado_por: None,
        aprobado_en: None,
        cerrado_en: None,
        cuotas: cuotas_acuerdo(saldo, nuevo.numero_cuotas, nuevo.primer_vencimiento)?,
    };
    tx.insertar_acuerdo(&acuerdo).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("bien_privado_id", bien.id.to_string())?
        .con("valor", acuerdo.valor.valor().to_string())?
        .con("cuotas", acuerdo.cuotas.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ACUERDO, "acuerdo_pago", acuerdo.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(acuerdo)
}

/// Aprueba un acuerdo propuesto por **otra cuenta**: pasa a vigente y suspende la mora.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.liquidar` o sin cuenta; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si lo propuso la misma cuenta; [`AppError::Dominio`] si no está
/// propuesto.
pub async fn aprobar_acuerdo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: AcuerdoPagoId,
) -> AppResult<AcuerdoPago>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let aprobador = ctx
        .actor_id
        .ok_or(AppError::SinPermiso(Permiso::CarteraLiquidar))?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraLiquidar).await?;
    let mut a = tx
        .acuerdo(id)
        .await?
        .ok_or(AppError::NoEncontrado("acuerdo de pago"))?;
    a.estado = a.estado.transicionar(EstadoAcuerdo::Vigente)?;
    if a.creado_por == aprobador {
        return Err(AppError::Conflicto(
            "el acuerdo lo aprueba una cuenta distinta de la que lo propuso".into(),
        ));
    }
    a.aprobado_por = Some(aprobador);
    a.aprobado_en = Some(reloj.ahora());
    tx.actualizar_acuerdo(&a).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_ACUERDO_APROBAR,
        "acuerdo_pago",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(a)
}

/// Anula un acuerdo propuesto o vigente (la mora se reanuda desde ese día).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.liquidar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si ya está cerrado.
pub async fn anular_acuerdo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: AcuerdoPagoId,
) -> AppResult<AcuerdoPago>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraLiquidar).await?;
    let mut a = tx
        .acuerdo(id)
        .await?
        .ok_or(AppError::NoEncontrado("acuerdo de pago"))?;
    a.estado = a.estado.transicionar(EstadoAcuerdo::Anulado)?;
    a.cerrado_en = Some(reloj.ahora());
    tx.actualizar_acuerdo(&a).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_ACUERDO_ANULAR,
        "acuerdo_pago",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(a)
}

// ---------------------------------------------------------------------------------------------
// Causación de intereses
// ---------------------------------------------------------------------------------------------

/// Intereses causados a un bien.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteresesBien {
    pub bien_id: BienPrivadoId,
    pub bien_codigo: String,
    pub deudor: PersonaId,
    pub total: Decimal,
    pub detalle: Vec<DetalleInteres>,
}

/// Resultado del cálculo de intereses (vista previa o lo causado).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalculoMora {
    pub periodo: NaiveDate,
    pub hasta: NaiveDate,
    pub bienes: Vec<InteresesBien>,
    pub total: Decimal,
}

async fn bien_cartera<T: TxCartera>(tx: &mut T, id: BienPrivadoId) -> AppResult<BienCartera> {
    tx.bienes_cartera()
        .await?
        .into_iter()
        .find(|b| b.id == id)
        .ok_or(AppError::NoEncontrado("bien privado"))
}

/// Calcula los intereses hasta la fecha; las obligaciones de intereses y la causación quedan
/// con ids nuevos (se guardan solo al causar).
async fn calcular_mora<T: TxCartera>(
    tx: &mut T,
    hasta: NaiveDate,
    causacion: CausacionMoraId,
) -> AppResult<CalculoMora> {
    let politica = politica_en(tx, hasta).await?;
    let p = &politica.parametros;
    let tramos: Vec<TramoTasa> = tx
        .tasas_referencia(ModalidadTasa::ConsumoOrdinario)
        .await?
        .into_iter()
        .map(|t| TramoTasa {
            desde: t.vigente_desde,
            hasta: t.vigente_hasta,
            tasa_ea: tasa_mora_ea(t.tasa_efectiva_anual, p.factor_mora, p.tasa_mora_mensual),
            referencia: t.id.as_uuid(),
        })
        .collect();
    let conceptos = tx.conceptos_cobro().await?;
    let generan: Vec<_> = conceptos
        .iter()
        .filter(|c| c.genera_interes)
        .map(|c| c.id)
        .collect();
    let causado = tx.causado_hasta().await?;
    let acuerdos = tx.acuerdos(None).await?;
    let obligaciones: Vec<SaldoObligacion> = tx
        .saldos_obligaciones(&FiltroObligaciones::default())
        .await?
        .into_iter()
        .filter(|o| {
            o.obligacion.estado == EstadoObligacion::Vigente
                && o.obligacion.origen != OrigenObligacion::Interes
                && generan.contains(&o.obligacion.concepto_cobro_id)
                && o.obligacion.vencimiento < hasta
        })
        .collect();
    let ids: Vec<ObligacionCobroId> = obligaciones.iter().map(|o| o.obligacion.id).collect();
    let mut reducciones: BTreeMap<ObligacionCobroId, Vec<Reduccion>> = BTreeMap::new();
    for (o, r) in tx.reducciones(&ids).await? {
        reducciones.entry(o).or_default().push(r);
    }
    let bienes = tx.bienes_cartera().await?;
    let mut por_bien: BTreeMap<BienPrivadoId, InteresesBien> = BTreeMap::new();
    for o in &obligaciones {
        let ob = &o.obligacion;
        let inicio = ob.vencimiento.succ_opt().unwrap_or(ob.vencimiento);
        let desde = causado
            .get(&ob.id)
            .and_then(|h| h.succ_opt())
            .map_or(inicio, |d| d.max(inicio));
        if desde > hasta {
            continue;
        }
        let propias = reducciones.get(&ob.id).map(Vec::as_slice).unwrap_or(&[]);
        let antes: Decimal = propias
            .iter()
            .filter(|r| r.fecha < desde)
            .map(|r| r.valor)
            .sum();
        let saldo_inicial = ob.valor.valor() - antes;
        let dentro: Vec<Reduccion> = propias
            .iter()
            .filter(|r| r.fecha >= desde)
            .copied()
            .collect();
        if saldo_inicial <= Decimal::ZERO {
            continue;
        }
        let suspensiones: Vec<(NaiveDate, NaiveDate)> = acuerdos
            .iter()
            .filter(|a| a.bien_privado_id == ob.bien_privado_id && ob.vencimiento <= a.fecha)
            .filter_map(|a| suspension(a, hasta))
            .collect();
        let segmentos = intereses(&CalculoInteres {
            desde,
            hasta,
            saldo_inicial,
            reducciones: &dentro,
            tramos: &tramos,
            suspensiones: &suspensiones,
        })?;
        if segmentos.is_empty() {
            continue;
        }
        let entrada = por_bien
            .entry(ob.bien_privado_id)
            .or_insert_with(|| InteresesBien {
                bien_id: ob.bien_privado_id,
                bien_codigo: bienes
                    .iter()
                    .find(|b| b.id == ob.bien_privado_id)
                    .map(|b| b.codigo.as_str().to_owned())
                    .unwrap_or_default(),
                deudor: ob.deudor_persona_id,
                total: Decimal::ZERO,
                detalle: Vec::new(),
            });
        for s in segmentos {
            entrada.total += s.valor;
            entrada.detalle.push(DetalleInteres {
                causacion_id: causacion,
                obligacion_id: ob.id,
                obligacion_interes_id: ObligacionCobroId::from_uuid(uuid::Uuid::nil()),
                desde: s.desde,
                hasta: s.hasta,
                dias: s.dias,
                base: s.base,
                tasa_efectiva_anual: s.tasa_ea,
                tasa_referencia_id: TasaReferenciaId::from_uuid(s.referencia),
                valor: s.valor,
            });
        }
    }
    let bienes: Vec<InteresesBien> = por_bien
        .into_values()
        .filter(|b| b.total > Decimal::ZERO)
        .collect();
    Ok(CalculoMora {
        periodo: periodo_de(hasta),
        hasta,
        total: bienes.iter().map(|b| b.total).sum(),
        bienes,
    })
}

/// Calcula los intereses de mora hasta la fecha sin guardar nada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.liquidar`; [`AppError::Validacion`] sin política;
/// [`AppError::Dominio`] si falta la tasa de referencia de algún día.
pub async fn vista_previa_intereses<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    hasta: NaiveDate,
) -> AppResult<CalculoMora>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraLiquidar).await?;
    calcular_mora(&mut tx, hasta, CausacionMoraId::nuevo()).await
}

/// Lo que dejó una causación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Causado {
    pub causacion: CausacionMora,
    pub calculo: CalculoMora,
    /// Acuerdos que cambiaron de estado al revisarlos.
    pub acuerdos_revisados: usize,
}

/// Causa y contabiliza los intereses de mora hasta la fecha (normalmente el último día del
/// mes): revisa los acuerdos, crea una obligación de intereses por bien (concepto de tipo
/// `interes_mora`) con el detalle de cada tramo y un comprobante del evento
/// [`eventos::INTERESES`] con una línea por deudor.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.liquidar`; [`AppError::Conflicto`] si el periodo ya
/// tiene una causación vigente; [`AppError::Validacion`] sin política, sin concepto de
/// intereses o sin intereses que causar; [`AppError::Dominio`] si falta la tasa de algún día;
/// los del contabilizador.
pub async fn causar_intereses<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    hasta: NaiveDate,
) -> AppResult<Causado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraLiquidar).await?;
    let periodo = periodo_de(hasta);
    if tx.causacion_del_periodo(periodo).await?.is_some() {
        return Err(AppError::Conflicto(format!(
            "los intereses de {} ya están causados: anule esa causación para repetirla",
            periodo.format("%Y-%m")
        )));
    }
    let concepto = tx
        .conceptos_cobro()
        .await?
        .into_iter()
        .find(|c| c.activo && c.tipo == TipoConceptoCobro::InteresMora)
        .ok_or_else(|| {
            validacion("cree un concepto de cobro de tipo `interes_mora` antes de causar")
        })?;
    let acuerdos_revisados = revisar_acuerdos_en(&mut tx, hasta).await?;
    let id = CausacionMoraId::nuevo();
    let mut calculo = calcular_mora(&mut tx, hasta, id).await?;
    if calculo.bienes.is_empty() {
        return Err(validacion("no hay intereses que causar hasta esa fecha"));
    }
    let ahora = reloj.ahora();
    let mut obligaciones = Vec::with_capacity(calculo.bienes.len());
    let mut por_deudor: BTreeMap<PersonaId, Decimal> = BTreeMap::new();
    for b in &mut calculo.bienes {
        let o = ObligacionCobro {
            id: ObligacionCobroId::nuevo(),
            bien_privado_id: b.bien_id,
            concepto_cobro_id: concepto.id,
            periodo,
            valor: monto(b.total)?,
            vencimiento: hasta,
            deudor_persona_id: b.deudor,
            origen: OrigenObligacion::Interes,
            liquidacion_id: None,
            cuota_extraordinaria_id: None,
            cuenta_cobro_id: None,
            comprobante_id: None,
            causacion_id: Some(id),
            descripcion: Some(format!("Intereses de mora hasta {hasta}")),
            estado: EstadoObligacion::Vigente,
            creada_en: ahora,
            anulada_en: None,
        };
        for d in &mut b.detalle {
            d.obligacion_interes_id = o.id;
        }
        *por_deudor.entry(b.deudor).or_default() += b.total;
        obligaciones.push(o);
    }
    let importes = por_deudor
        .into_iter()
        .map(|(deudor, valor)| {
            Ok(
                ImporteDocumento::new(&concepto.concepto_contable, monto(valor)?)
                    .con_persona(Some(deudor)),
            )
        })
        .collect::<AppResult<Vec<_>>>()?;
    let comprobante_id = comprobantes::contabilizar_documento(
        &mut tx,
        reloj,
        ctx,
        DocumentoOrigen {
            evento: eventos::INTERESES.into(),
            origen: origen(eventos::INTERESES, id.as_uuid()),
            fecha: hasta,
            descripcion: format!("Intereses de mora hasta {hasta}"),
            importes,
        },
    )
    .await?;
    let causacion = CausacionMora {
        id,
        periodo,
        hasta,
        estado: EstadoCausacion::Contabilizada,
        total: monto(calculo.total)?,
        comprobante_id,
        motivo_anulacion: None,
        causada_por: ctx.actor_id,
        causada_en: ahora,
        anulada_en: None,
    };
    tx.insertar_causacion(&causacion).await?;
    tx.insertar_obligaciones(&obligaciones).await?;
    let detalle: Vec<DetalleInteres> = calculo
        .bienes
        .iter()
        .flat_map(|b| b.detalle.iter().cloned())
        .collect();
    tx.insertar_detalle_interes(&detalle).await?;
    let auditoria = DetalleAuditoria::vacio()
        .con("hasta", hasta.to_string())?
        .con("total", causacion.total.valor().to_string())?
        .con("bienes", calculo.bienes.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CAUSAR, "causacion_mora", id.as_uuid()).con_detalle(auditoria),
    )
    .await?;
    tx.confirmar().await?;
    Ok(Causado {
        causacion,
        calculo,
        acuerdos_revisados,
    })
}

/// Anula una causación sin pagos ni ajustes sobre sus intereses: revierte su comprobante y anula
/// sus obligaciones (los días vuelven a quedar por causar).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.liquidar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya está anulada o sus intereses tienen pagos o ajustes.
pub async fn anular_causacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CausacionMoraId,
    motivo: &str,
) -> AppResult<CausacionMora>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let motivo = texto("motivo", motivo, 500)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraLiquidar).await?;
    let mut c = tx
        .causacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("causación de intereses"))?;
    if c.estado == EstadoCausacion::Anulada {
        return Err(AppError::Conflicto("la causación ya está anulada".into()));
    }
    if tx.causacion_tiene_movimientos(id).await? {
        return Err(AppError::Conflicto(
            "los intereses tienen pagos o ajustes: corríjalos con notas crédito".into(),
        ));
    }
    revertir_en(&mut tx, reloj, ctx, c.comprobante_id, &motivo, None).await?;
    let ahora = reloj.ahora();
    tx.anular_causacion(id, &motivo, ahora).await?;
    c.estado = EstadoCausacion::Anulada;
    c.motivo_anulacion = Some(motivo);
    c.anulada_en = Some(ahora);
    tx.registrar(EventoAuditoria::ok(
        ACCION_CAUSACION_ANULAR,
        "causacion_mora",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Causaciones por periodo descendente.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn causaciones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<CausacionMora>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    abrir(unidad, reloj, ctx, Permiso::CarteraVer)
        .await?
        .causaciones(LIMITE_CAUSACIONES)
        .await
}

/// Causación con el detalle de cada tramo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`; [`AppError::NoEncontrado`].
pub async fn causacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CausacionMoraId,
) -> AppResult<(CausacionMora, Vec<DetalleInteres>)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
    let c = tx
        .causacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("causación de intereses"))?;
    let detalle = tx.detalle_causacion(id).await?;
    Ok((c, detalle))
}

// ---------------------------------------------------------------------------------------------
// Cartera por edades
// ---------------------------------------------------------------------------------------------

/// Edades de la deuda de un bien al corte.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EdadesBien {
    pub bien_id: Option<BienPrivadoId>,
    pub codigo: String,
    pub deudor_persona_id: Option<PersonaId>,
    /// Aún no vencido.
    pub corriente: Decimal,
    pub de_1_a_30: Decimal,
    pub de_31_a_60: Decimal,
    pub de_61_a_90: Decimal,
    pub mas_de_90: Decimal,
    pub total: Decimal,
}

impl EdadesBien {
    fn sumar(&mut self, tramo: TramoEdad, saldo: Decimal) {
        match tramo {
            TramoEdad::Corriente => self.corriente += saldo,
            TramoEdad::De1A30 => self.de_1_a_30 += saldo,
            TramoEdad::De31A60 => self.de_31_a_60 += saldo,
            TramoEdad::De61A90 => self.de_61_a_90 += saldo,
            TramoEdad::MasDe90 => self.mas_de_90 += saldo,
        }
        self.total += saldo;
    }
}

/// Cartera por edades (0–30, 31–60, 61–90, más de 90 días de vencida) por bien al corte, con la
/// fila de totales (sin bien).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn cartera_por_edades<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    corte: Option<NaiveDate>,
) -> AppResult<(Vec<EdadesBien>, EdadesBien)>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let corte = corte.unwrap_or_else(|| reloj.ahora().date_naive());
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
    let bienes = tx.bienes_cartera().await?;
    let mut por_bien: BTreeMap<BienPrivadoId, EdadesBien> = BTreeMap::new();
    let mut total = EdadesBien {
        codigo: "Total".into(),
        ..EdadesBien::default()
    };
    for o in tx
        .saldos_obligaciones(&FiltroObligaciones {
            con_saldo: true,
            hasta_periodo: Some(periodo_de(corte)),
            ..FiltroObligaciones::default()
        })
        .await?
    {
        let tramo = TramoEdad::de(o.obligacion.vencimiento, corte);
        let b = o.obligacion.bien_privado_id;
        por_bien
            .entry(b)
            .or_insert_with(|| {
                let bien = bienes.iter().find(|x| x.id == b);
                EdadesBien {
                    bien_id: Some(b),
                    codigo: bien
                        .map(|x| x.codigo.as_str().to_owned())
                        .unwrap_or_default(),
                    deudor_persona_id: bien.and_then(|x| x.deudor_persona_id),
                    ..EdadesBien::default()
                }
            })
            .sumar(tramo, o.saldo);
        total.sumar(tramo, o.saldo);
    }
    let mut filas: Vec<EdadesBien> = por_bien.into_values().collect();
    filas.sort_by(|a, b| a.codigo.cmp(&b.codigo));
    Ok((filas, total))
}

// ---------------------------------------------------------------------------------------------
// Certificados
// ---------------------------------------------------------------------------------------------

/// Certificado recién emitido: el código y el PDF se entregan una sola vez.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificadoEmitido {
    pub certificado: CertificadoCartera,
    /// Código de verificación (solo se guarda su huella).
    pub codigo: String,
    pub pdf: Vec<u8>,
}

/// Emite un paz y salvo (solo si el bien no debe nada de lo ya cobrado, incluido lo liquidado
/// por anticipado) o un certificado de deuda del bien, con un código de verificación de 80 bits
/// impreso en el PDF.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.certificados.emitir`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si se pide paz y salvo de un bien con saldo;
/// [`AppError::Infraestructura`] si falla el PDF.
pub async fn emitir_certificado<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    tipo: TipoCertificado,
) -> AppResult<CertificadoEmitido>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
    G: GeneradorActas,
{
    let ahora = reloj.ahora();
    let corte = ahora.date_naive();
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraCertificadosEmitir).await?;
    let b = bien_cartera(&mut tx, bien).await?;
    let pendientes = tx
        .saldos_obligaciones(&FiltroObligaciones {
            bien: Some(bien),
            con_saldo: true,
            // Todo lo ya cobrado cuenta, también lo liquidado por anticipado.
            ..FiltroObligaciones::default()
        })
        .await?;
    let saldo: Decimal = pendientes.iter().map(|o| o.saldo).sum();
    let vencido: Decimal = pendientes
        .iter()
        .filter(|o| o.obligacion.vencimiento < corte)
        .map(|o| o.saldo)
        .sum();
    if tipo == TipoCertificado::PazYSalvo && saldo > Decimal::ZERO {
        return Err(AppError::Conflicto(format!(
            "el bien debe {saldo}: no se puede expedir paz y salvo"
        )));
    }
    let entidad = tx.datos_entidad().await?;
    let deudor = match b.deudor_persona_id {
        Some(p) => tx.persona(p).await?.map(|x| x.nombre).unwrap_or_default(),
        None => String::new(),
    };
    let codigo = codigo_verificacion(&token_aleatorio()?)?;
    let (titulo, filas) = match tipo {
        TipoCertificado::PazYSalvo => (
            "Paz y salvo",
            vec![vec![
                format!(
                    "El bien {} está a paz y salvo por todo concepto con la copropiedad",
                    b.codigo.as_str()
                ),
                corte.to_string(),
                "0.00".into(),
            ]],
        ),
        TipoCertificado::Deuda => {
            let mut filas: Vec<Vec<String>> = pendientes
                .iter()
                .map(|o| {
                    vec![
                        format!(
                            "{} {}",
                            o.concepto_nombre,
                            o.obligacion.periodo.format("%Y-%m")
                        ),
                        o.obligacion.vencimiento.to_string(),
                        o.saldo.round_dp(2).to_string(),
                    ]
                })
                .collect();
            filas.push(vec![
                "Total".into(),
                String::new(),
                saldo.round_dp(2).to_string(),
            ]);
            ("Certificado de deuda", filas)
        }
    };
    let pdf = generador
        .tabla(&TablaPdf {
            titulo: titulo.into(),
            copropiedad: match &entidad.nit {
                Some(nit) => format!("{} · NIT {nit}", entidad.nombre),
                None => entidad.nombre.clone(),
            },
            subtitulo: format!(
                "Bien {} · {} · Corte {} · Código de verificación {}",
                b.codigo.as_str(),
                deudor,
                corte,
                codigo
            ),
            columnas: ["Concepto", "Vence", "Saldo"].map(str::to_owned).to_vec(),
            filas,
            generada: ahora.format("%Y-%m-%d %H:%M UTC").to_string(),
        })
        .await?;
    let certificado = CertificadoCartera {
        id: CertificadoCarteraId::nuevo(),
        tipo,
        bien_privado_id: bien,
        corte,
        saldo,
        vencido,
        codigo_hash: hash_secreto(&normalizar_codigo(&codigo)),
        pdf_sha256: sha256_hex(&pdf),
        emitido_por: ctx.actor_id,
        emitido_en: ahora,
    };
    tx.insertar_certificado(&certificado).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", tipo.as_str())?
        .con("bien_privado_id", bien.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_CERTIFICADO,
            "certificado_cartera",
            certificado.id.as_uuid(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(CertificadoEmitido {
        certificado,
        codigo,
        pdf,
    })
}

/// Certificados emitidos de un bien, los más recientes primero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn certificados<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
) -> AppResult<Vec<CertificadoCartera>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    abrir(unidad, reloj, ctx, Permiso::CarteraVer)
        .await?
        .certificados(bien)
        .await
}

/// Lo que se responde a quien verifica un código: solo si es válido, su clase y su corte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verificacion {
    pub tipo: TipoCertificado,
    pub corte: NaiveDate,
}

/// Verifica un código de certificado sin sesión (el código es la credencial; la ruta lleva
/// límite por IP). No revela el bien, el deudor ni el saldo.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn verificar_certificado<U>(
    unidad: &U,
    tenant: TenantId,
    codigo: &str,
) -> AppResult<Option<Verificacion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let normalizado = normalizar_codigo(codigo);
    if normalizado.len() != 20 {
        return Ok(None);
    }
    let ctx = Contexto::new(tenant);
    let mut tx = unidad.iniciar(&ctx).await?;
    Ok(tx
        .certificado_por_codigo(&hash_secreto(&normalizado))
        .await?
        .map(|c| Verificacion {
            tipo: c.tipo,
            corte: c.corte,
        }))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use domain::contabilidad::Monto;

    use super::*;

    fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, d).expect("fecha")
    }

    fn acuerdo(estado: EstadoAcuerdo, cerrado: Option<NaiveDate>) -> AcuerdoPago {
        AcuerdoPago {
            id: AcuerdoPagoId::nuevo(),
            bien_privado_id: BienPrivadoId::nuevo(),
            deudor_persona_id: PersonaId::nuevo(),
            fecha: fecha(2026, 3, 1),
            valor: Monto::new(Decimal::ONE_HUNDRED).expect("monto"),
            estado,
            observaciones: None,
            creado_por: domain::CuentaId::nuevo(),
            creado_en: Utc
                .with_ymd_and_hms(2026, 3, 1, 0, 0, 0)
                .single()
                .expect("fecha"),
            aprobado_por: None,
            aprobado_en: None,
            cerrado_en: cerrado
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|d| d.and_utc()),
            cuotas: Vec::new(),
        }
    }

    #[test]
    fn la_suspension_va_hasta_el_incumplimiento() {
        assert_eq!(
            suspension(&acuerdo(EstadoAcuerdo::Vigente, None), fecha(2026, 3, 31)),
            Some((fecha(2026, 3, 1), fecha(2026, 3, 31)))
        );
        assert_eq!(
            suspension(
                &acuerdo(EstadoAcuerdo::Incumplido, Some(fecha(2026, 3, 11))),
                fecha(2026, 3, 31)
            ),
            Some((fecha(2026, 3, 1), fecha(2026, 3, 10)))
        );
        assert_eq!(
            suspension(&acuerdo(EstadoAcuerdo::Anulado, None), fecha(2026, 3, 31)),
            None
        );
        assert_eq!(
            suspension(&acuerdo(EstadoAcuerdo::Propuesto, None), fecha(2026, 3, 31)),
            None
        );
    }
}
