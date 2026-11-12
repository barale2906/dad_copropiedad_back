//! Tarifas, política de publicidad y liquidación de la pauta (`plataforma.publicidad.liquidar`,
//! plan 08, fase 4). Los precios viven en `tarifas_publicidad` y se cambian sin desplegar; cada
//! campaña fija la suya al aprobarse. La liquidación es por anunciante y mes, **sin reparto a
//! las copropiedades**; el costo de WhatsApp es informativo (lo que le costó a la plataforma).
//! La facturación electrónica y el pago en línea llegan con el módulo de contabilidad.

use chrono::{DateTime, Datelike, Months, NaiveDate, Utc};
use domain::{
    AnuncianteId, EstadoLiquidacion, LiquidacionId, ModeloCobro, Permiso, PoliticaPublicidadId,
    publicidad::valor_liquidacion,
};
use rust_decimal::Decimal;

use super::TxPublicidadPlataforma;
use crate::puertos::{RegistroAuditoria, RepositorioPublicidadPlataforma};
use crate::{
    AppError, AppResult, ContextoPlataforma,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        Reloj, Transaccion, UnidadDeTrabajoPlataforma,
        publicidad::{Liquidacion, PoliticaPublicidad, TarifaPublicidad},
    },
};

pub const ACCION_TARIFA: AccionAuditoria = AccionAuditoria::new("publicidad.tarifa.fijar");
pub const ACCION_POLITICA: AccionAuditoria = AccionAuditoria::new("publicidad.politica.publicar");
pub const ACCION_LIQUIDAR: AccionAuditoria =
    AccionAuditoria::new("publicidad.liquidacion.calcular");
pub const ACCION_ESTADO_LIQUIDACION: AccionAuditoria =
    AccionAuditoria::new("publicidad.liquidacion.cambiar_estado");

fn exigir(ctx: &ContextoPlataforma) -> AppResult<()> {
    ctx.exigir(Permiso::PlataformaPublicidadLiquidar)
}

/// Todas las tarifas (vigentes, anteriores y programadas).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.publicidad.liquidar`.
pub async fn tarifas<UP>(unidad: &UP, ctx: &ContextoPlataforma) -> AppResult<Vec<TarifaPublicidad>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    exigir(ctx)?;
    unidad.iniciar_plataforma(ctx).await?.tarifas().await
}

/// Fija la tarifa de un modelo desde una fecha (hoy o futura). No cambia las campañas ya
/// aprobadas.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::Validacion`] si el valor es negativo o la fecha ya
/// pasó; [`AppError::Conflicto`] si ya hay una tarifa del modelo desde ese día.
pub async fn fijar_tarifa<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    tarifa: TarifaPublicidad,
) -> AppResult<TarifaPublicidad>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    exigir(ctx)?;
    let ahora = reloj.ahora();
    if tarifa.valor < Decimal::ZERO {
        return Err(AppError::Validacion(
            "la tarifa no puede ser negativa".into(),
        ));
    }
    if tarifa.vigente_desde < ahora.date_naive() {
        return Err(AppError::Validacion(
            "la tarifa rige desde hoy o una fecha futura".into(),
        ));
    }
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.insertar_tarifa(&tarifa, ctx.actor_id, ahora).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("modelo", tarifa.modelo.as_str())?
        .con("valor", tarifa.valor.to_string())?
        .con("vigente_desde", tarifa.vigente_desde.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_TARIFA, "tarifa_publicidad", uuid::Uuid::nil())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(tarifa)
}

/// Política de publicidad vigente de la plataforma (pública: la lee quien va a autorizar).
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn politica_vigente<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
) -> AppResult<Option<PoliticaPublicidad>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    unidad
        .iniciar_plataforma(&ContextoPlataforma::sistema())
        .await?
        .politica_publicidad_vigente(reloj.ahora())
        .await
}

/// Publica una versión nueva de la política de publicidad (rige desde ya). Las autorizaciones
/// anteriores siguen asociadas a la versión que se aceptó.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::Validacion`] si el texto está vacío o es muy largo.
pub async fn publicar_politica<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    texto: &str,
) -> AppResult<PoliticaPublicidad>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    exigir(ctx)?;
    let texto = domain::copropiedad::texto_obligatorio("texto", texto, 50_000)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let p = PoliticaPublicidad {
        id: PoliticaPublicidadId::nuevo(),
        version: tx.ultima_version_politica().await? + 1,
        texto,
        vigente_desde: ahora,
        publicada_por: ctx.actor_id,
        creada_en: ahora,
    };
    tx.insertar_politica_publicidad(&p).await?;
    let detalle = DetalleAuditoria::vacio().con("version", p.version)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_POLITICA, "politica_publicidad", p.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Primer día del mes de `fecha`.
fn mes_de(fecha: NaiveDate) -> NaiveDate {
    fecha.with_day(1).unwrap_or(fecha)
}

/// Calcula (o recalcula, si siguen `calculada`) las liquidaciones de un mes ya cerrado: el
/// consumo variable ya valorado, los cargos fijos de las campañas por periodo, los contactos
/// autorizados por la tarifa `por_lead` y el costo de WhatsApp para la plataforma. Las ya
/// facturadas o pagadas no cambian.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::Validacion`] si el mes no ha terminado.
pub async fn liquidar_mes<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    mes: NaiveDate,
) -> AppResult<Vec<Liquidacion>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    exigir(ctx)?;
    let ahora = reloj.ahora();
    let desde = mes_de(mes);
    let hasta = desde
        .checked_add_months(Months::new(1))
        .ok_or_else(|| AppError::Validacion("mes inválido".into()))?;
    if hasta > ahora.date_naive() {
        return Err(AppError::Validacion(
            "solo se liquida un mes que ya terminó".into(),
        ));
    }
    let ultimo_dia = hasta.pred_opt().unwrap_or(desde);
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let tarifa_lead = tx
        .tarifa_vigente(ModeloCobro::PorLead, ultimo_dia)
        .await?
        .map_or(Decimal::ZERO, |t| t.valor);
    let tarifa_wa = tx
        .tarifa_whatsapp_marketing(ultimo_dia)
        .await?
        .unwrap_or(Decimal::ZERO);
    let mut resultado = Vec::new();
    for c in tx.consumo_periodo(desde, hasta).await? {
        let previa = tx.liquidacion(c.anunciante_id, desde).await?;
        if let Some(p) = &previa
            && p.estado != EstadoLiquidacion::Calculada
        {
            resultado.push(p.clone());
            continue;
        }
        let valor_leads = (tarifa_lead * Decimal::from(c.leads)).round_dp(2);
        let l = Liquidacion {
            id: previa.as_ref().map_or_else(LiquidacionId::nuevo, |p| p.id),
            anunciante_id: c.anunciante_id,
            periodo: desde,
            variable: c.variable.round_dp(2),
            fijos: c.fijos.round_dp(2),
            leads: c.leads,
            valor_leads,
            valor: valor_liquidacion(c.variable, c.fijos, c.leads, tarifa_lead),
            costo_whatsapp: (tarifa_wa * Decimal::from(c.mensajes_whatsapp)).round_dp(2),
            estado: EstadoLiquidacion::Calculada,
            calculada_en: previa.as_ref().map_or(ahora, |p| p.calculada_en),
            actualizada_en: ahora,
        };
        tx.guardar_liquidacion(&l).await?;
        resultado.push(l);
    }
    let detalle = DetalleAuditoria::vacio()
        .con("periodo", desde.to_string())?
        .con("anunciantes", resultado.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_LIQUIDAR, "liquidacion", uuid::Uuid::nil()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(resultado)
}

/// Liquidaciones de un mes.
///
/// # Errors
///
/// [`AppError::SinPermiso`].
pub async fn liquidaciones<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    mes: NaiveDate,
) -> AppResult<Vec<Liquidacion>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    exigir(ctx)?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .liquidaciones(mes_de(mes))
        .await
}

/// Marca una liquidación facturada o pagada ([`EstadoLiquidacion::transicionar`]).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si la
/// transición no está permitida.
pub async fn cambiar_estado_liquidacion<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
    mes: NaiveDate,
    hacia: EstadoLiquidacion,
) -> AppResult<Liquidacion>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    exigir(ctx)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut l = tx
        .liquidacion(anunciante, mes_de(mes))
        .await?
        .ok_or(AppError::NoEncontrado("liquidación"))?;
    let desde = l.estado;
    l.estado = desde.transicionar(hacia)?;
    l.actualizada_en = reloj.ahora();
    tx.guardar_liquidacion(&l).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("desde", desde.as_str())?
        .con("hacia", hacia.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ESTADO_LIQUIDACION, "liquidacion", l.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(l)
}

/// Finaliza las campañas cuya vigencia terminó (worker, contexto del sistema).
///
/// # Errors
///
/// [`AppError::Infraestructura`] si falla la base de datos.
pub async fn finalizar_vencidas<UP>(unidad: &UP, ahora: DateTime<Utc>) -> AppResult<u64>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let mut tx = unidad
        .iniciar_plataforma(&ContextoPlataforma::sistema())
        .await?;
    let n = tx.finalizar_vencidas(ahora).await?;
    tx.confirmar().await?;
    Ok(n)
}
