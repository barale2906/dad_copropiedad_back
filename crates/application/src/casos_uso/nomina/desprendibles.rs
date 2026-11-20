//! Desprendibles de nómina (plan 16 fase 3): el PDF de cada liquidación, su envío por correo al
//! empleado (outbox) y el portal del empleado en `/yo/nomina`.
//!
//! Solo hay desprendible de un periodo **aprobado** o pagado. El desprendible circula fuera de
//! la entidad: las incapacidades y licencias salen como «Ausencia remunerada» (el motivo es dato
//! sensible) y no lleva la cuenta bancaria. El empleado ve los suyos aunque no tenga roles en la
//! entidad: la persona vinculada a su cuenta es la prueba.

use domain::{
    CuentaId, DireccionCorreo, LiquidacionNominaId, Permiso, TenantId,
    contabilidad::Monto,
    nomina::{
        liquidacion::{ClaseLinea, LineaLiquidacion},
        periodo::EstadoPeriodoNomina,
    },
};

use super::{TxNomina, abrir_nomina};
use crate::{
    AppError, AppResult, Contexto, ContextoPlataforma,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        GeneradorActas, RegistroAuditoria as _, RegistroOutbox, Reloj, Transaccion,
        UnidadDeTrabajo, UnidadDeTrabajoPlataforma,
        actas::TablaPdf,
        correo::{AdjuntoCorreo, CorreoSaliente},
        nomina::{
            DesprendibleDeCuenta, LiquidacionContrato, PeriodoNomina,
            RepositorioLiquidacionNomina as _, RepositorioNomina as _, RepositorioNominaDeCuenta,
        },
        personas::RepositorioPersonas as _,
        tesoreria::RepositorioFacturas,
    },
};

const ACCION_ENVIAR: AccionAuditoria = AccionAuditoria::new("nomina.desprendibles.enviar");

/// Desprendibles que lista el portal como máximo.
pub const LIMITE_DESPRENDIBLES: u32 = 120;

/// Lo que usa una transacción de los desprendibles.
pub trait TxDesprendibles: TxNomina + RepositorioFacturas + RegistroOutbox {}

impl<T> TxDesprendibles for T where T: TxNomina + RepositorioFacturas + RegistroOutbox {}

fn pesos(m: Monto) -> String {
    let entero = m
        .valor()
        .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
        .to_string();
    let mut con_puntos = String::new();
    for (i, c) in entero.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 && c != '-' {
            con_puntos.push('.');
        }
        con_puntos.push(c);
    }
    format!("$ {}", con_puntos.chars().rev().collect::<String>())
}

fn fila(l: &LineaLiquidacion) -> Vec<String> {
    let (devengo, deduccion) = match l.concepto.clase() {
        ClaseLinea::Devengo => (pesos(l.valor), String::new()),
        _ => (String::new(), pesos(l.valor)),
    };
    let cantidad = if l.cantidad.is_zero() {
        String::new()
    } else {
        l.cantidad.normalize().to_string()
    };
    vec![
        l.concepto.etiqueta_desprendible().to_owned(),
        cantidad,
        devengo,
        deduccion,
    ]
}

/// Liquidación y periodo de un desprendible (el periodo debe estar aprobado o pagado).
async fn liquidacion_aprobada<T: TxDesprendibles>(
    tx: &mut T,
    id: LiquidacionNominaId,
) -> AppResult<(LiquidacionContrato, PeriodoNomina)> {
    let l = tx
        .liquidacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("liquidación"))?;
    let p = tx
        .periodo(l.periodo_id)
        .await?
        .ok_or(AppError::NoEncontrado("periodo de nómina"))?;
    if !matches!(
        p.estado,
        EstadoPeriodoNomina::Aprobado | EstadoPeriodoNomina::Pagado
    ) {
        return Err(AppError::Conflicto(
            "el desprendible existe cuando el periodo está aprobado".into(),
        ));
    }
    Ok((l, p))
}

/// PDF del desprendible y correo del empleado (si tiene).
async fn armar<T: TxDesprendibles>(
    tx: &mut T,
    generador: &impl GeneradorActas,
    reloj: &impl Reloj,
    l: &LiquidacionContrato,
    p: &PeriodoNomina,
) -> AppResult<(Vec<u8>, Option<DireccionCorreo>)> {
    let entidad = tx.datos_entidad().await?;
    let empleado = tx
        .empleado(l.empleado_id)
        .await?
        .ok_or(AppError::NoEncontrado("empleado"))?;
    let persona = tx
        .persona(empleado.persona_id)
        .await?
        .ok_or(AppError::NoEncontrado("persona"))?;
    let cargo = tx
        .versiones(l.contrato_id)
        .await?
        .into_iter()
        .rfind(|v| v.vigente_desde <= p.hasta)
        .map(|v| v.cargo)
        .unwrap_or_default();
    let q = &l.liquidacion;
    let mut filas: Vec<Vec<String>> = q
        .lineas
        .iter()
        .filter(|x| x.concepto.clase() != ClaseLinea::AporteEmpleador)
        .map(fila)
        .collect();
    filas.push(vec![
        "Totales".into(),
        String::new(),
        pesos(q.devengos),
        pesos(q.deducciones),
    ]);
    filas.push(vec![
        "Neto a pagar".into(),
        String::new(),
        pesos(q.neto),
        String::new(),
    ]);
    let documento = persona
        .documento
        .as_ref()
        .map(|d| {
            format!(
                "{} {}",
                persona.tipo_documento.map_or("", |t| t.as_str()),
                d.as_str()
            )
        })
        .unwrap_or_default();
    let pdf = generador
        .tabla(&TablaPdf {
            titulo: "Desprendible de nómina".into(),
            copropiedad: match &entidad.nit {
                Some(nit) => format!("{} · NIT {nit}", entidad.nombre),
                None => entidad.nombre.clone(),
            },
            subtitulo: format!(
                "{} · {documento} · {cargo} · {} a {} · {} días · IBC {}",
                persona.nombre,
                p.desde,
                p.hasta,
                q.dias,
                pesos(q.ibc)
            ),
            columnas: vec![
                "Concepto".into(),
                "Cantidad".into(),
                "Devengos".into(),
                "Deducciones".into(),
            ],
            filas,
            generada: reloj.ahora().format("%Y-%m-%d %H:%M UTC").to_string(),
            qr: None,
            pie: None,
        })
        .await?;
    Ok((pdf, persona.correo))
}

/// PDF del desprendible de una liquidación de un periodo aprobado.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.salarios.ver`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si el periodo no está aprobado.
pub async fn desprendible<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: LiquidacionNominaId,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxDesprendibles,
    G: GeneradorActas,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaSalariosVer).await?;
    let (l, p) = liquidacion_aprobada(&mut tx, id).await?;
    Ok(armar(&mut tx, generador, reloj, &l, &p).await?.0)
}

/// Resultado del envío de los desprendibles de un periodo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvioDesprendibles {
    pub enviados: u32,
    /// Empleados sin correo en su persona (entréguelos a mano o en el portal).
    pub sin_correo: u32,
}

/// Envía por correo el desprendible de cada liquidación del periodo aprobado (por el outbox:
/// sale solo si la transacción se confirma).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.liquidar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si el periodo no está aprobado.
pub async fn enviar_desprendibles<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    periodo: domain::PeriodoNominaId,
) -> AppResult<EnvioDesprendibles>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxDesprendibles,
    G: GeneradorActas,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaLiquidar).await?;
    let mut envio = EnvioDesprendibles {
        enviados: 0,
        sin_correo: 0,
    };
    let mut entidad = None;
    for l in tx.liquidaciones(periodo).await? {
        let (l, p) = liquidacion_aprobada(&mut tx, l.id).await?;
        let (pdf, correo) = armar(&mut tx, generador, reloj, &l, &p).await?;
        let Some(para) = correo else {
            envio.sin_correo += 1;
            continue;
        };
        let nombre = match &entidad {
            Some(n) => n,
            None => entidad.insert(tx.datos_entidad().await?.nombre),
        }
        .clone();
        tx.encolar(EventoSaliente {
            prioridad: PrioridadSalida::Evento,
            mensaje: MensajeOutbox::Correo(CorreoSaliente {
                para,
                asunto: format!(
                    "Desprendible de nómina {} a {} · {nombre}",
                    p.desde, p.hasta
                ),
                texto: format!(
                    "{nombre} le envía el desprendible de nómina del periodo {} a {}. También lo \
                     encuentra en la sección Mi nómina de la aplicación.",
                    p.desde, p.hasta
                ),
                html: None,
                adjuntos: vec![AdjuntoCorreo {
                    nombre: format!("desprendible-{}.pdf", p.desde),
                    tipo: "application/pdf".into(),
                    contenido: pdf,
                }],
            }),
        })
        .await?;
        envio.enviados += 1;
    }
    let detalle = DetalleAuditoria::vacio()
        .con("enviados", envio.enviados)?
        .con("no_enviados", envio.sin_correo)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ENVIAR, "periodo_nomina", periodo.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(envio)
}

// ---------------------------------------------------------------------------------------------
// Portal del empleado
// ---------------------------------------------------------------------------------------------

/// Desprendibles de la cuenta autenticada en todas las entidades donde trabaja (o trabajó).
///
/// # Errors
///
/// [`AppError::NoAutenticado`] sin cuenta.
pub async fn mis_desprendibles<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<DesprendibleDeCuenta>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioNominaDeCuenta,
{
    let cuenta = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    tx.desprendibles_de_cuenta(cuenta, LIMITE_DESPRENDIBLES)
        .await
}

/// PDF de un desprendible propio: la liquidación debe ser de la persona vinculada a la cuenta
/// en esa entidad (no exige roles allí).
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si no existe o no es de la cuenta (no se distingue, para no
/// revelar liquidaciones ajenas); [`AppError::Conflicto`] si el periodo no está aprobado.
pub async fn mi_desprendible<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    cuenta: CuentaId,
    entidad: TenantId,
    id: LiquidacionNominaId,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxDesprendibles,
    G: GeneradorActas,
{
    let ctx = Contexto::de_cuenta(entidad, cuenta, domain::ConjuntoPermisos::vacio());
    let mut tx = unidad.iniciar(&ctx).await?;
    let (l, p) = liquidacion_aprobada(&mut tx, id).await?;
    let empleado = tx
        .empleado(l.empleado_id)
        .await?
        .ok_or(AppError::NoEncontrado("liquidación"))?;
    let es_suya = tx
        .persona(empleado.persona_id)
        .await?
        .is_some_and(|persona| persona.cuenta_id == Some(cuenta));
    if !es_suya {
        return Err(AppError::NoEncontrado("liquidación"));
    }
    Ok(armar(&mut tx, generador, reloj, &l, &p).await?.0)
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;

    #[test]
    fn los_pesos_llevan_separador_de_miles() {
        let m = |t: &str| Monto::new(t.parse::<Decimal>().expect("decimal")).expect("monto");
        assert_eq!(pesos(m("1580000")), "$ 1.580.000");
        assert_eq!(pesos(m("999")), "$ 999");
        assert_eq!(pesos(m("1000.50")), "$ 1.001");
    }
}
