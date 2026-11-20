//! Deducciones de la retención en la fuente que declara el empleado (plan 16): dependientes,
//! intereses de vivienda, medicina prepagada y aportes voluntarios, con su soporte. La
//! liquidación toma las vigentes al cierre del periodo y les aplica los topes del catálogo
//! ([`domain::nomina::liquidacion`]). Una deducción no se edita: se termina y se registra otra.

use chrono::NaiveDate;
use domain::{
    ArchivoId, DeduccionRetencionId, EmpleadoId, Permiso,
    contabilidad::Monto,
    nomina::liquidacion::{DeduccionesRetencion, TipoDeduccion},
};

use super::{TxNomina, abrir_nomina};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria as _, Reloj, Transaccion, UnidadDeTrabajo,
        nomina::{DeduccionRetencion, RepositorioLiquidacionNomina as _, RepositorioNomina as _},
    },
};

const ACCION_REGISTRAR: AccionAuditoria = AccionAuditoria::new("nomina.deduccion.registrar");
const ACCION_TERMINAR: AccionAuditoria = AccionAuditoria::new("nomina.deduccion.terminar");

/// Deducción nueva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevaDeduccion {
    pub tipo: TipoDeduccion,
    /// Valor mensual (no va en dependientes).
    pub valor_mensual: Option<Monto>,
    pub vigente_desde: NaiveDate,
    /// Certificado del banco, de la entidad de salud, declaración de dependientes…
    pub soporte_archivo_id: Option<ArchivoId>,
}

/// Registra una deducción declarada por el empleado.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.empleados.gestionar`; [`AppError::NoEncontrado`] si el
/// empleado no existe; [`AppError::Validacion`] si dependientes trae valor o los demás no.
pub async fn registrar_deduccion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    empleado: EmpleadoId,
    nueva: NuevaDeduccion,
) -> AppResult<DeduccionRetencion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let valor = match (nueva.tipo, nueva.valor_mensual) {
        (TipoDeduccion::Dependientes, None) => Monto::CERO,
        (TipoDeduccion::Dependientes, Some(_)) => {
            return Err(AppError::Validacion(
                "la deducción por dependientes no lleva valor".into(),
            ));
        }
        (_, Some(v)) if !v.es_cero() => v,
        _ => {
            return Err(AppError::Validacion(
                "la deducción lleva su valor mensual".into(),
            ));
        }
    };
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaEmpleadosGestionar).await?;
    tx.empleado(empleado)
        .await?
        .ok_or(AppError::NoEncontrado("empleado"))?;
    let d = DeduccionRetencion {
        id: DeduccionRetencionId::nuevo(),
        empleado_id: empleado,
        tipo: nueva.tipo,
        valor_mensual: valor,
        vigente_desde: nueva.vigente_desde,
        vigente_hasta: None,
        soporte_archivo_id: nueva.soporte_archivo_id,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
    };
    tx.insertar_deduccion(&d).await?;
    let detalle = DetalleAuditoria::vacio().con("vigente_desde", d.vigente_desde.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_REGISTRAR, "deduccion_retencion", d.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(d)
}

/// Termina una deducción el día `hasta` (desde el siguiente ya no se aplica).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.empleados.gestionar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si `hasta` es anterior a su inicio;
/// [`domain::DominioError::ReglaLaboral`] si ya estaba terminada.
pub async fn terminar_deduccion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: DeduccionRetencionId,
    hasta: NaiveDate,
) -> AppResult<DeduccionRetencion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaEmpleadosGestionar).await?;
    let mut d = tx
        .deduccion(id)
        .await?
        .ok_or(AppError::NoEncontrado("deducción"))?;
    if hasta < d.vigente_desde {
        return Err(AppError::Validacion(
            "la deducción termina después de su inicio".into(),
        ));
    }
    tx.terminar_deduccion(id, hasta).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_TERMINAR,
        "deduccion_retencion",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    d.vigente_hasta = Some(hasta);
    Ok(d)
}

/// Deducciones del empleado (datos personales).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.salarios.ver`.
pub async fn deducciones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    empleado: EmpleadoId,
) -> AppResult<Vec<DeduccionRetencion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaSalariosVer).await?;
    tx.deducciones(empleado).await
}

/// Deducciones vigentes del empleado en la fecha, sumadas por tipo.
pub(crate) async fn vigentes_en<T: TxNomina>(
    tx: &mut T,
    empleado: EmpleadoId,
    fecha: NaiveDate,
) -> AppResult<DeduccionesRetencion> {
    let mut d = DeduccionesRetencion::default();
    for x in tx.deducciones(empleado).await? {
        if x.vigente_desde > fecha || x.vigente_hasta.is_some_and(|h| h < fecha) {
            continue;
        }
        let v = x.valor_mensual.valor();
        match x.tipo {
            TipoDeduccion::Dependientes => d.dependientes = true,
            TipoDeduccion::InteresesVivienda => d.intereses_vivienda += v,
            TipoDeduccion::MedicinaPrepagada => d.medicina_prepagada += v,
            TipoDeduccion::AportesVoluntarios => d.aportes_voluntarios += v,
        }
    }
    Ok(d)
}
