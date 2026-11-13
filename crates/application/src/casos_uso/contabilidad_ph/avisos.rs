//! Avisos automáticos de la cartera (plan 14 fase 6; los ejecuta el worker del gateway):
//! recordatorio de pago unos días antes del vencimiento y aviso de mora unos días después, una
//! sola vez por cuenta de cobro y clase. Van al pagador del bien (o al propietario
//! representante) por correo y, si tiene teléfono, por WhatsApp con la plantilla
//! [`PLANTILLA_RECORDATORIO_PAGO`]. Finalidad `administracion`. El valor solo va en el correo:
//! la plantilla de WhatsApp no lo lleva (no se expone en la vista previa del celular).
//!
//! La causación de intereses y la restricción de reservas no son automáticas (crean asientos y
//! bloqueos): las dispara la administración.

use chrono::NaiveDate;
use domain::{Modulo, contabilidad_ph::TipoAvisoCartera};
use rust_decimal::Decimal;

use super::TxCartera;
use crate::{
    AppResult, Contexto,
    casos_uso::{
        avisos::{Aviso, avisar_persona},
        contabilidad::prelude::*,
    },
    outbox::PrioridadSalida,
    puertos::{
        Reloj, RepositorioModulos as _, UnidadDeTrabajo,
        cartera::{DirectorioCartera, FiltroObligaciones},
    },
};

/// Plantilla de utilidad «recordatorio_pago» (por aprobar en Meta). Parámetros: bien privado,
/// fecha de vencimiento y situación (`vence pronto` o `está vencida`).
pub const PLANTILLA_RECORDATORIO_PAGO: &str = "recordatorio_pago";
/// Días antes del vencimiento en que se recuerda el pago.
pub const DIAS_ANTES: u32 = 3;
/// Días después del vencimiento en que se avisa la mora.
pub const DIAS_DESPUES: u32 = 5;

/// Avisos de una copropiedad: envía los pendientes y deja constancia de cada uno.
async fn avisar_en<T: TxCartera>(
    tx: &mut T,
    reloj: &impl Reloj,
    hoy: NaiveDate,
) -> AppResult<usize> {
    let ahora = reloj.ahora();
    let entidad = tx.datos_entidad().await?.nombre;
    let bienes = tx.bienes_cartera().await?;
    let mut enviados = 0;
    for (cuenta, tipo) in tx.cuentas_por_avisar(hoy, DIAS_ANTES, DIAS_DESPUES).await? {
        let pendientes = tx
            .saldos_obligaciones(&FiltroObligaciones {
                bien: Some(cuenta.bien_privado_id),
                con_saldo: true,
                ..FiltroObligaciones::default()
            })
            .await?;
        let saldo: Decimal = pendientes.iter().map(|o| o.saldo).sum();
        let vencido: Decimal = pendientes
            .iter()
            .filter(|o| o.obligacion.vencimiento < hoy)
            .map(|o| o.saldo)
            .sum();
        let corresponde = match tipo {
            TipoAvisoCartera::Recordatorio => saldo > Decimal::ZERO,
            TipoAvisoCartera::Mora => vencido > Decimal::ZERO,
        };
        if corresponde {
            let bien = bienes.iter().find(|b| b.id == cuenta.bien_privado_id);
            let codigo = bien
                .map(|b| b.codigo.as_str().to_owned())
                .unwrap_or_default();
            let destinatario = bien
                .and_then(|b| b.pagador())
                .or(Some(cuenta.deudor_persona_id));
            let referencia = cuenta
                .referencia_pago
                .as_deref()
                .map(|r| format!(" Referencia de pago: {r}."))
                .unwrap_or_default();
            let (asunto, texto, situacion) = match tipo {
                TipoAvisoCartera::Recordatorio => (
                    format!("Recordatorio de pago · {entidad}"),
                    format!(
                        "El saldo del bien {codigo} es {} y vence el {}.{referencia}",
                        saldo.round_dp(2),
                        cuenta.vencimiento
                    ),
                    "vence pronto",
                ),
                TipoAvisoCartera::Mora => (
                    format!("Saldo vencido · {entidad}"),
                    format!(
                        "El bien {codigo} tiene {} vencido desde el {}. Póngase al día para \
                         evitar intereses de mora.{referencia}",
                        vencido.round_dp(2),
                        cuenta.vencimiento
                    ),
                    "está vencida",
                ),
            };
            avisar_persona(
                tx,
                destinatario,
                &Aviso {
                    prioridad: PrioridadSalida::Recordatorio,
                    asunto,
                    texto,
                    plantilla: Some((
                        PLANTILLA_RECORDATORIO_PAGO,
                        vec![codigo, cuenta.vencimiento.to_string(), situacion.into()],
                    )),
                },
            )
            .await?;
            enviados += 1;
        }
        tx.registrar_aviso(cuenta.id, tipo, ahora).await?;
    }
    Ok(enviados)
}

/// Recorre las copropiedades con avisos pendientes y los encola (cada una en su transacción;
/// si una falla, sigue con las demás). Devuelve cuántos avisos encoló.
///
/// # Errors
///
/// [`crate::AppError::Infraestructura`] si falla la consulta de copropiedades.
pub async fn avisar_cartera<D, U>(
    directorio: &D,
    unidad: &U,
    reloj: &impl Reloj,
) -> AppResult<usize>
where
    D: DirectorioCartera,
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let hoy = reloj.ahora().date_naive();
    let mut total = 0;
    for tenant in directorio
        .copropiedades_con_avisos(hoy, DIAS_ANTES, DIAS_DESPUES)
        .await?
    {
        let ctx = Contexto::new(tenant);
        let resultado: AppResult<usize> = async {
            let mut tx = unidad.iniciar(&ctx).await?;
            if !tx
                .modulo_habilitado(Modulo::ContabilidadPh, reloj.ahora())
                .await?
            {
                return Ok(0);
            }
            let n = avisar_en(&mut tx, reloj, hoy).await?;
            tx.confirmar().await?;
            Ok(n)
        }
        .await;
        match resultado {
            Ok(n) => total += n,
            Err(e) => tracing::warn!(tenant_id = ?tenant, error = %e, "avisos de cartera fallidos"),
        }
    }
    Ok(total)
}
