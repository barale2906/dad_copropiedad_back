//! Novedades de nómina (plan 16 fase 2): horas extra y recargos, incapacidades, licencias,
//! ausencias, bonificaciones y deducciones de un contrato. Son de solo inserción: una novedad
//! equivocada se **anula** con su motivo. Ninguna cae en un periodo ya aprobado.
//!
//! Las incapacidades y las licencias de maternidad o paternidad son datos sensibles (nunca el
//! diagnóstico): solo las ve quien tiene `personas.datos_sensibles.ver`. Los valores de las
//! novedades en pesos son datos personales: solo con `nomina.salarios.ver`.

use chrono::NaiveDate;
use domain::{
    ArchivoId, ContratoLaboralId, NovedadNominaId, Permiso,
    nomina::liquidacion::{Medida, Novedad, TipoNovedad},
};
use rust_decimal::Decimal;

use super::{TxNomina, abrir_nomina};
use crate::casos_uso::contabilidad::{texto, texto_opcional};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria as _, Reloj, Transaccion, UnidadDeTrabajo,
        nomina::{AnulacionNovedad, NovedadNomina, RepositorioLiquidacionNomina as _},
    },
};

const ACCION_REGISTRAR: AccionAuditoria = AccionAuditoria::new("nomina.novedad.registrar");
const ACCION_ANULAR: AccionAuditoria = AccionAuditoria::new("nomina.novedad.anular");

/// Novedad nueva de un contrato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevaNovedad {
    pub tipo: TipoNovedad,
    /// Fecha (horas y valores) o primer día (días).
    pub desde: NaiveDate,
    /// Último día (solo las de días).
    pub hasta: Option<NaiveDate>,
    /// Horas o valor en pesos (no en las de días).
    pub cantidad: Option<Decimal>,
    pub descripcion: Option<String>,
    /// Incapacidad, licencia, soporte de la bonificación…
    pub soporte_archivo_id: Option<ArchivoId>,
}

/// Oculta lo que el contexto no puede ver: las novedades sensibles (sin
/// `personas.datos_sensibles.ver`) y los valores en pesos (sin `nomina.salarios.ver`).
fn reservar(ctx: &Contexto, novedades: Vec<NovedadNomina>) -> Vec<NovedadNomina> {
    let sensibles = ctx.permisos.contiene(Permiso::PersonasDatosSensiblesVer);
    let valores = ctx.permisos.contiene(Permiso::NominaSalariosVer);
    novedades
        .into_iter()
        .filter(|n| sensibles || !n.novedad.tipo.es_sensible())
        .map(|mut n| {
            if !valores && n.novedad.tipo.medida() == Medida::Valor {
                n.novedad.cantidad = Decimal::ZERO;
            }
            n
        })
        .collect()
}

/// Registra una novedad dentro de las fechas del contrato.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `nomina.novedades.registrar`;
///   [`AppError::ModuloNoHabilitado`] sin nómina.
/// - [`AppError::NoEncontrado`] si el contrato no existe.
/// - [`domain::DominioError::ReglaLaboral`] si la medida no corresponde al tipo, una dominical no
///   cae en domingo o festivo, las fechas salen del contrato o caen en un periodo aprobado.
pub async fn registrar_novedad<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    contrato: ContratoLaboralId,
    nueva: NuevaNovedad,
) -> AppResult<NovedadNomina>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaNovedadesRegistrar).await?;
    let registro = registrar_en(&mut tx, reloj, ctx, contrato, nueva).await?;
    tx.confirmar().await?;
    Ok(registro)
}

/// Registra la novedad dentro de la transacción (lo usan el registro y la importación).
pub(crate) async fn registrar_en<T: TxNomina>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    contrato: ContratoLaboralId,
    nueva: NuevaNovedad,
) -> AppResult<NovedadNomina> {
    let novedad = Novedad::new(nueva.tipo, nueva.desde, nueva.hasta, nueva.cantidad)?;
    let descripcion = texto_opcional("descripcion", nueva.descripcion.as_deref(), 300)?;
    let c = tx
        .contrato(contrato)
        .await?
        .ok_or(AppError::NoEncontrado("contrato"))?;
    let fin = c.fecha_terminacion.or(c.fecha_fin);
    if novedad.desde < c.fecha_inicio || fin.is_some_and(|f| novedad.hasta > f) {
        return Err(domain::DominioError::ReglaLaboral(
            "la novedad cae fuera de las fechas del contrato".into(),
        )
        .into());
    }
    let registro = NovedadNomina {
        id: NovedadNominaId::nuevo(),
        contrato_id: contrato,
        novedad,
        descripcion,
        soporte_archivo_id: nueva.soporte_archivo_id,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
        anulacion: None,
    };
    tx.insertar_novedad(&registro).await?;
    // Ni el tipo (puede ser sensible) ni el valor van a la auditoría.
    let detalle = DetalleAuditoria::vacio()
        .con("contrato_id", contrato.as_uuid().to_string())?
        .con("desde", registro.novedad.desde.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_REGISTRAR, "novedad_nomina", registro.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    Ok(registro)
}

/// Anula una novedad con su motivo (deja de entrar a la liquidación).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.novedades.registrar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya estaba anulada; [`domain::DominioError::ReglaLaboral`] si cae en
/// un periodo aprobado.
pub async fn anular_novedad<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: NovedadNominaId,
    motivo: &str,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let motivo = texto("motivo", motivo, 300)?;
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaNovedadesRegistrar).await?;
    let n = tx
        .novedad(id)
        .await?
        .ok_or(AppError::NoEncontrado("novedad"))?;
    if n.anulacion.is_some() {
        return Err(AppError::Conflicto("la novedad ya estaba anulada".into()));
    }
    let anulacion = AnulacionNovedad {
        motivo,
        anulada_por: ctx.actor_id,
        anulada_en: reloj.ahora(),
    };
    tx.anular_novedad(id, &anulacion).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_ANULAR,
        "novedad_nomina",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Novedades que tocan el rango, de un contrato o de todos (con las anuladas marcadas).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.ver`; [`AppError::Validacion`] si `hasta` es anterior a
/// `desde`.
pub async fn novedades<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    contrato: Option<ContratoLaboralId>,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<Vec<NovedadNomina>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    if hasta < desde {
        return Err(AppError::Validacion(
            "`hasta` no es anterior a `desde`".into(),
        ));
    }
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaVer).await?;
    Ok(reservar(ctx, tx.novedades(contrato, desde, hasta).await?))
}
