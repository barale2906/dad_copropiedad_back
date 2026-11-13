//! Integración de la cartera con los otros módulos de la PH (plan 14 fase 6):
//!
//! - **Asambleas**: los morosos de una reunión salen de la cartera vencida a la fecha de corte
//!   (antes se cargaban a mano); se fijan con el caso de uso de asambleas, que sigue sin
//!   depender de la contabilidad.
//! - **Reservas**: la restricción por mora se sincroniza con la cartera (bienes con vencido
//!   desde un umbral) usando las restricciones de reserva que ya existen.

use chrono::NaiveDate;
use domain::{
    Modulo, Permiso, RestriccionId, ReunionId, contabilidad::Monto, contabilidad_ph::periodo_de,
};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

use super::{TxCartera, abrir};
use crate::{
    AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{
        asambleas::{
            TxAsambleas,
            registro::{ResultadoMorosos, fijar_morosos},
        },
        contabilidad::prelude::*,
        modulos::exigir_modulo_en,
    },
    puertos::{
        Reloj, RepositorioReservas, UnidadDeTrabajo, cartera::FiltroObligaciones,
        reservas::Restriccion,
    },
};

/// Motivo con que la cartera crea (y reconoce) sus restricciones de reserva.
pub const MOTIVO_MORA: &str = "Mora en la cartera de la copropiedad";

const ACCION_RESTRICCIONES: AccionAuditoria =
    AccionAuditoria::new("cartera.reservas.sincronizar_mora");

/// Vencido de cada bien al corte (solo los que tienen algo vencido).
async fn vencido_por_bien<T: TxCartera>(
    tx: &mut T,
    corte: NaiveDate,
) -> AppResult<BTreeMap<domain::BienPrivadoId, (String, Decimal)>> {
    let bienes = tx.bienes_cartera().await?;
    let mut vencido: BTreeMap<domain::BienPrivadoId, (String, Decimal)> = BTreeMap::new();
    for o in tx
        .saldos_obligaciones(&FiltroObligaciones {
            con_saldo: true,
            hasta_periodo: Some(periodo_de(corte)),
            ..FiltroObligaciones::default()
        })
        .await?
        .into_iter()
        .filter(|o| o.obligacion.vencimiento < corte)
    {
        let b = o.obligacion.bien_privado_id;
        let codigo = bienes
            .iter()
            .find(|x| x.id == b)
            .map(|x| x.codigo.as_str().to_owned())
            .unwrap_or_default();
        vencido.entry(b).or_insert((codigo, Decimal::ZERO)).1 += o.saldo;
    }
    Ok(vencido)
}

/// Fija los morosos de la reunión con los bienes que tienen cartera vencida a la fecha de corte
/// (por defecto, hoy). Reemplaza el listado anterior, como la carga manual.
///
/// # Errors
///
/// [`crate::AppError::SinPermiso`] sin `cartera.ver` o sin `asambleas.convocar`; los de
/// [`fijar_morosos`] (reunión inexistente, instalada o sin padrón).
pub async fn morosos_desde_cartera<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    reunion: ReunionId,
    corte: Option<NaiveDate>,
) -> AppResult<ResultadoMorosos>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera + TxAsambleas,
{
    ctx.exigir(Permiso::AsambleasConvocar)?;
    let corte = corte.unwrap_or_else(|| reloj.ahora().date_naive());
    let codigos: Vec<String> = {
        let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
        vencido_por_bien(&mut tx, corte)
            .await?
            .into_values()
            .map(|(codigo, _)| codigo)
            .collect()
    };
    fijar_morosos(unidad, reloj, ctx, reunion, &codigos).await
}

/// Resultado de sincronizar las restricciones por mora.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SincronizacionMora {
    /// Bienes que quedaron restringidos.
    pub restringidos: usize,
    /// Restricciones por mora levantadas (el bien ya no supera el umbral).
    pub levantadas: usize,
}

/// Restringe las reservas de los bienes con cartera vencida desde el umbral (al corte de hoy) y
/// levanta las restricciones por mora de los que ya no lo superan. Solo toca las restricciones
/// que creó la cartera ([`MOTIVO_MORA`]); las manuales no cambian.
///
/// # Errors
///
/// [`crate::AppError::SinPermiso`] sin `cartera.ver` o sin `reservas.restringir`;
/// [`crate::AppError::ModuloNoHabilitado`] sin bienes comunes.
pub async fn sincronizar_restricciones_mora<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    umbral: Monto,
) -> AppResult<SincronizacionMora>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera + RepositorioReservas,
{
    ctx.exigir(Permiso::ReservasRestringir)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraVer).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::BienesComunes).await?;
    let morosos: Vec<domain::BienPrivadoId> = vencido_por_bien(&mut tx, ahora.date_naive())
        .await?
        .into_iter()
        .filter(|(_, (_, v))| *v > Decimal::ZERO && *v >= umbral.valor())
        .map(|(b, _)| b)
        .collect();
    let por_mora: Vec<Restriccion> = tx
        .restricciones(None)
        .await?
        .into_iter()
        .filter(|r| r.motivo == MOTIVO_MORA && r.hasta.is_none_or(|h| h > ahora))
        .collect();
    let mut resultado = SincronizacionMora {
        restringidos: 0,
        levantadas: 0,
    };
    for r in &por_mora {
        if !morosos.contains(&r.bien_privado_id) && tx.levantar_restriccion(r.id, ahora).await? {
            resultado.levantadas += 1;
        }
    }
    for b in morosos {
        if por_mora.iter().any(|r| r.bien_privado_id == b) {
            continue;
        }
        tx.insertar_restriccion(&Restriccion {
            id: RestriccionId::nuevo(),
            bien_privado_id: b,
            motivo: MOTIVO_MORA.into(),
            soporte_archivo_id: None,
            desde: ahora,
            hasta: None,
            creada_por: ctx.actor_id,
        })
        .await?;
        resultado.restringidos += 1;
    }
    let detalle = DetalleAuditoria::vacio()
        .con("umbral", umbral.valor().to_string())?
        .con("restringidos", resultado.restringidos)?
        .con("levantadas", resultado.levantadas)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RESTRICCIONES, "cartera", ctx.tenant_id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(resultado)
}
