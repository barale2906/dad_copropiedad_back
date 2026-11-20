//! Avisos automáticos de la nómina (worker `worker_nomina` del gateway): a quienes gestionan la
//! nómina (`nomina.empleados.gestionar`), por correo y una sola vez por referencia:
//!
//! - **Contratos a término fijo por vencer** en los próximos [`DIAS_AVISO`] días: si no se va a
//!   renovar, el preaviso se entrega con [`DIAS_PREAVISO`] días de anticipación (CST art. 46); sin
//!   él, el contrato se prorroga por un periodo igual. Una prórroga cambia la fecha final y genera
//!   un aviso nuevo.
//! - **Fechas legales** ([`domain::nomina::reportes::recordatorios_legales`]): intereses de las
//!   cesantías, consignación de las cesantías y prima.

use chrono::{Days, NaiveDate};
use domain::{Modulo, Permiso, nomina::reportes::recordatorios_legales};

use super::TxNomina;
use crate::{
    AppResult, Contexto,
    casos_uso::avisos::{Aviso, avisar_con_permiso},
    outbox::PrioridadSalida,
    puertos::{
        RegistroOutbox, Reloj, RepositorioModulos as _, RepositorioTickets, Transaccion,
        UnidadDeTrabajo, nomina::DirectorioNomina,
    },
};

/// Con cuántos días de anticipación se avisa el vencimiento de un contrato a término fijo.
pub const DIAS_AVISO: u64 = 45;
/// Días de preaviso para no renovar un contrato a término fijo (CST art. 46).
pub const DIAS_PREAVISO: u64 = 30;

/// Lo que usa la transacción de los avisos.
pub trait TxAvisosNomina: TxNomina + RepositorioTickets + RegistroOutbox {}

impl<T> TxAvisosNomina for T where T: TxNomina + RepositorioTickets + RegistroOutbox {}

/// Encola los avisos pendientes de una entidad. Devuelve cuántos avisos (no correos) encoló.
pub(crate) async fn avisar_en<T: TxAvisosNomina>(
    tx: &mut T,
    reloj: &impl Reloj,
    hoy: NaiveDate,
) -> AppResult<usize> {
    let mut n = 0;
    let limite = hoy.checked_add_days(Days::new(DIAS_AVISO)).unwrap_or(hoy);
    for c in tx.contratos_en(hoy, limite).await? {
        let Some(fin) = c.fecha_fin else { continue };
        if c.tipo != domain::nomina::TipoContratoLaboral::TerminoFijo
            || c.fecha_terminacion.is_some()
            || fin < hoy
            || fin > limite
        {
            continue;
        }
        let referencia = format!("{}:{fin}", c.id);
        if tx
            .aviso_enviado("vencimiento_contrato", &referencia)
            .await?
        {
            continue;
        }
        let codigo = tx
            .empleado(c.empleado_id)
            .await?
            .map(|e| e.codigo.as_str().to_owned())
            .unwrap_or_default();
        let preaviso = fin
            .checked_sub_days(Days::new(DIAS_PREAVISO))
            .unwrap_or(fin);
        let texto = if preaviso >= hoy {
            format!(
                "El contrato a término fijo del empleado {codigo} vence el {fin}. Si no se va a \
                 renovar, entregue el preaviso a más tardar el {preaviso}; si no, se prorroga por un \
                 periodo igual."
            )
        } else {
            format!(
                "El contrato a término fijo del empleado {codigo} vence el {fin} y ya pasó el plazo \
                 del preaviso ({preaviso}): si no se entregó, el contrato se prorroga por un periodo \
                 igual. Registre la prórroga o la terminación en la nómina."
            )
        };
        avisar_con_permiso(
            tx,
            Permiso::NominaEmpleadosGestionar,
            &Aviso {
                prioridad: PrioridadSalida::Recordatorio,
                asunto: format!("Contrato por vencer · empleado {codigo}"),
                texto,
                plantilla: None,
            },
        )
        .await?;
        tx.registrar_aviso("vencimiento_contrato", &referencia, reloj.ahora())
            .await?;
        n += 1;
    }
    for r in recordatorios_legales(hoy) {
        if tx.aviso_enviado(r.tipo, &r.referencia).await? {
            continue;
        }
        avisar_con_permiso(
            tx,
            Permiso::NominaEmpleadosGestionar,
            &Aviso {
                prioridad: PrioridadSalida::Recordatorio,
                asunto: r.asunto,
                texto: r.texto,
                plantilla: None,
            },
        )
        .await?;
        tx.registrar_aviso(r.tipo, &r.referencia, reloj.ahora())
            .await?;
        n += 1;
    }
    Ok(n)
}

/// Recorre las entidades con nómina y encola sus avisos pendientes (cada una en su transacción;
/// si una falla, sigue con las demás). Devuelve cuántos avisos encoló.
///
/// # Errors
///
/// [`crate::AppError::Infraestructura`] si falla la consulta de entidades.
pub async fn avisar_nomina<D, U>(directorio: &D, unidad: &U, reloj: &impl Reloj) -> AppResult<usize>
where
    D: DirectorioNomina,
    U: UnidadDeTrabajo,
    U::Transaccion: TxAvisosNomina,
{
    let hoy = domain::proteccion_datos::fecha_en_colombia(reloj.ahora());
    let mut total = 0;
    for tenant in directorio.entidades_con_nomina(reloj.ahora()).await? {
        let ctx = Contexto::new(tenant);
        let resultado: AppResult<usize> = async {
            let mut tx = unidad.iniciar(&ctx).await?;
            if !tx.modulo_habilitado(Modulo::Nomina, reloj.ahora()).await? {
                return Ok(0);
            }
            let n = avisar_en(&mut tx, reloj, hoy).await?;
            tx.confirmar().await?;
            Ok(n)
        }
        .await;
        match resultado {
            Ok(n) => total += n,
            Err(e) => tracing::warn!(tenant_id = ?tenant, error = %e, "avisos de nómina fallidos"),
        }
    }
    Ok(total)
}
