//! Certificado laboral (plan 16 fase 3): PDF con los datos del empleado y de sus contratos en la
//! entidad (cargo, tipo de contrato, fechas y, si se pide, el salario actual). Lo descarga la
//! administración desde la ficha del empleado o el propio empleado desde «Mi nómina».

use domain::{CuentaId, EmpleadoId, Permiso, TenantId};

use super::{abrir_nomina, desprendibles::TxDesprendibles};
use crate::{
    AppError, AppResult, Contexto,
    puertos::{
        GeneradorActas, Reloj, RepositorioTickets, UnidadDeTrabajo,
        actas::TablaPdf,
        nomina::{Empleado, RepositorioNomina as _},
    },
};

fn tipo_contrato(t: domain::nomina::TipoContratoLaboral) -> &'static str {
    use domain::nomina::TipoContratoLaboral as T;
    match t {
        T::TerminoFijo => "Término fijo",
        T::TerminoIndefinido => "Término indefinido",
        T::ObraLabor => "Obra o labor",
        T::Aprendizaje => "Aprendizaje",
    }
}

fn pesos(v: rust_decimal::Decimal) -> String {
    let entero = v
        .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
        .to_string();
    let mut s = String::new();
    for (i, c) in entero.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            s.push('.');
        }
        s.push(c);
    }
    format!("$ {}", s.chars().rev().collect::<String>())
}

async fn armar<T: TxDesprendibles>(
    tx: &mut T,
    generador: &impl GeneradorActas,
    reloj: &impl Reloj,
    empleado: &Empleado,
    con_salario: bool,
) -> AppResult<Vec<u8>> {
    let entidad = tx.datos_entidad().await?;
    let persona = tx
        .persona(empleado.persona_id)
        .await?
        .ok_or(AppError::NoEncontrado("persona"))?;
    let hoy = reloj.ahora().date_naive();
    let mut filas = vec![
        vec!["Nombre".to_owned(), persona.nombre.clone()],
        vec![
            "Documento".to_owned(),
            persona
                .documento
                .as_ref()
                .map(|d| {
                    format!(
                        "{} {}",
                        persona.tipo_documento.map_or("", |t| t.as_str()),
                        d.as_str()
                    )
                })
                .unwrap_or_default(),
        ],
    ];
    let contratos = tx.contratos(empleado.id).await?;
    if contratos.is_empty() {
        return Err(AppError::NoEncontrado("contrato"));
    }
    for c in &contratos {
        let versiones = tx.versiones(c.id).await?;
        let fin = c.fecha_terminacion.or(c.fecha_fin);
        let vigente = c.fecha_terminacion.is_none_or(|f| f >= hoy);
        let version = versiones
            .iter()
            .rfind(|v| v.vigente_desde <= hoy.min(fin.unwrap_or(hoy)))
            .or(versiones.first());
        let cargo = version.map(|v| v.cargo.clone()).unwrap_or_default();
        let periodo = match (c.fecha_terminacion, vigente) {
            (Some(t), false) => format!("del {} al {t}", c.fecha_inicio),
            _ => format!("desde el {} (vigente)", c.fecha_inicio),
        };
        filas.push(vec![
            "Contrato".to_owned(),
            format!("{} · {cargo} · {periodo}", tipo_contrato(c.tipo)),
        ]);
        if con_salario
            && vigente
            && let Some(s) = version.and_then(|v| v.salario)
        {
            filas.push(vec!["Salario mensual".to_owned(), pesos(s.valor())]);
        }
    }
    generador
        .tabla(&TablaPdf {
            titulo: "Certificado laboral".into(),
            copropiedad: match &entidad.nit {
                Some(nit) => format!("{} · NIT {nit}", entidad.nombre),
                None => entidad.nombre.clone(),
            },
            subtitulo: format!(
                "{} certifica que la persona trabaja o trabajó con la entidad así:",
                entidad.nombre
            ),
            columnas: vec!["Dato".into(), "Detalle".into()],
            filas,
            generada: reloj.ahora().format("%Y-%m-%d %H:%M UTC").to_string(),
            qr: None,
            pie: None,
        })
        .await
}

/// Certificado laboral de un empleado (con el salario solo si se pide y se tiene
/// `nomina.salarios.ver`).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.ver` (o sin `nomina.salarios.ver` con salario);
/// [`AppError::NoEncontrado`] si el empleado no existe o no tiene contratos.
pub async fn certificado_laboral<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    empleado: EmpleadoId,
    con_salario: bool,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxDesprendibles,
    G: GeneradorActas,
{
    if con_salario {
        ctx.exigir(Permiso::NominaSalariosVer)?;
    }
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaVer).await?;
    let e = tx
        .empleado(empleado)
        .await?
        .ok_or(AppError::NoEncontrado("empleado"))?;
    armar(&mut tx, generador, reloj, &e, con_salario).await
}

/// Certificado laboral propio desde «Mi nómina» (con su salario): la persona vinculada a la cuenta
/// debe ser empleada de la entidad (no exige roles allí).
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si la cuenta no es empleada de la entidad.
pub async fn mi_certificado_laboral<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    cuenta: CuentaId,
    entidad: TenantId,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxDesprendibles + RepositorioTickets,
    G: GeneradorActas,
{
    let ctx = Contexto::de_cuenta(entidad, cuenta, domain::ConjuntoPermisos::vacio());
    let mut tx = unidad.iniciar(&ctx).await?;
    let persona = RepositorioTickets::persona_de_cuenta(&mut tx, cuenta)
        .await?
        .ok_or(AppError::NoEncontrado("empleado"))?;
    let e = tx
        .empleado_de_persona(persona)
        .await?
        .ok_or(AppError::NoEncontrado("empleado"))?;
    armar(&mut tx, generador, reloj, &e, true).await
}
