//! Reportes: balance de prueba (por cuenta, tercero y centro de costo), libro diario, libro mayor
//! y auxiliares, estados financieros con sus notas, y exportación en CSV con los metadatos de
//! generación en la auditoría (QA-017: el mismo filtro reproduce el mismo contenido, porque lo
//! contabilizado no cambia).

use std::fmt::Write as _;

use chrono::NaiveDate;
use domain::{
    Permiso, PersonaId,
    contabilidad::{
        CodigoCuenta,
        estados::{Rubro, SaldoCuenta, ValorRubro, resultado_del_ejercicio, valores_rubros},
    },
};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};

use super::prelude::*;
use super::{TxContable, exigir_contabilidad};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        Reloj, UnidadDeTrabajo,
        contabilidad::{
            AgrupacionSaldos, Comprobante, CuentaContable, FilaSaldo, Movimiento, NotaEstados,
        },
    },
};

const ACCION_EXPORTAR: AccionAuditoria = AccionAuditoria::new("contabilidad.libros.exportar");

/// Primer día que se considera en los saldos acumulados.
fn origen() -> NaiveDate {
    NaiveDate::from_ymd_opt(2000, 1, 1).expect("fecha fija válida")
}

fn validar_rango(desde: NaiveDate, hasta: NaiveDate) -> AppResult<()> {
    if desde > hasta {
        return Err(AppError::Validacion(
            "`desde` es posterior a `hasta`".into(),
        ));
    }
    Ok(())
}

async fn abrir<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    permiso: Permiso,
) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    ctx.exigir(permiso)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_contabilidad(&mut tx, reloj).await?;
    Ok(tx)
}

/// Balance de prueba entre dos fechas: saldo inicial, débitos, créditos y saldo final de cada
/// cuenta con movimientos (y por tercero o centro de costo si se agrupa así).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::Validacion`] si el rango está
/// invertido.
pub async fn balance_prueba<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    desde: NaiveDate,
    hasta: NaiveDate,
    agrupacion: AgrupacionSaldos,
) -> AppResult<Vec<FilaSaldo>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    validar_rango(desde, hasta)?;
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .saldos(desde, hasta, agrupacion)
        .await
}

/// Libro diario: comprobantes contabilizados entre dos fechas, con sus líneas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn libro_diario<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<Vec<Comprobante>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    validar_rango(desde, hasta)?;
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .diario(desde, hasta)
        .await
}

/// Libro mayor (o auxiliar de un tercero) de una cuenta.
#[derive(Debug, Clone)]
pub struct Mayor {
    pub cuenta: CuentaContable,
    pub persona: Option<PersonaId>,
    /// Débitos − créditos antes de `desde`.
    pub saldo_inicial: Decimal,
    pub movimientos: Vec<Movimiento>,
    pub saldo_final: Decimal,
}

/// Libro mayor de la cuenta entre dos fechas; con `persona`, el auxiliar de ese tercero.
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si la cuenta no existe.
pub async fn libro_mayor<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    cuenta: &CodigoCuenta,
    persona: Option<PersonaId>,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<Mayor>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    validar_rango(desde, hasta)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    let c = tx
        .cuenta_por_codigo(cuenta)
        .await?
        .ok_or(AppError::NoEncontrado("cuenta"))?;
    let agrupacion = AgrupacionSaldos {
        por_tercero: persona.is_some(),
        ..AgrupacionSaldos::default()
    };
    let saldo_inicial: Decimal = tx
        .saldos(desde, hasta, agrupacion)
        .await?
        .into_iter()
        .filter(|f| f.cuenta_id == c.id && (persona.is_none() || f.persona_id == persona))
        .map(|f| f.saldo_inicial)
        .sum();
    let movimientos = tx.movimientos(c.id, persona, desde, hasta).await?;
    let saldo_final = saldo_inicial
        + movimientos
            .iter()
            .map(|m| m.debito - m.credito)
            .sum::<Decimal>();
    Ok(Mayor {
        cuenta: c,
        persona,
        saldo_inicial,
        movimientos,
        saldo_final,
    })
}

/// Estado de situación financiera y de resultados a una fecha de corte.
#[derive(Debug, Clone)]
pub struct EstadosFinancieros {
    pub corte: NaiveDate,
    /// Saldos acumulados a la fecha de corte, por rubro.
    pub situacion_financiera: Vec<ValorRubro>,
    /// Movimiento del año hasta la fecha de corte, por rubro (sin el cierre anual).
    pub resultados: Vec<ValorRubro>,
    /// Ingresos − gastos − costos del año (sin el cierre anual).
    pub resultado_ejercicio: Decimal,
    /// ¿Ya se contabilizó el cierre del año? Si sí, el resultado ya está en el patrimonio.
    pub cierre_contabilizado: bool,
    /// Notas del año (última versión de cada una).
    pub notas: Vec<NotaEstados>,
}

fn rubros_de(rubros: &[crate::puertos::contabilidad::RubroEstado], estado: &str) -> Vec<Rubro> {
    rubros
        .iter()
        .filter(|r| r.estado == estado)
        .map(|r| Rubro {
            codigo: r.codigo.clone(),
            nombre: r.nombre.clone(),
            orden: r.orden,
            prefijos: r.prefijos.clone(),
        })
        .collect()
}

fn a_saldos(filas: &[FilaSaldo]) -> Vec<SaldoCuenta> {
    filas
        .iter()
        .map(|f| SaldoCuenta {
            codigo: f.codigo.clone(),
            debitos: f.saldo_inicial.max(Decimal::ZERO) + f.debitos,
            creditos: (-f.saldo_inicial).max(Decimal::ZERO) + f.creditos,
        })
        .collect()
}

/// Estados financieros a la fecha de corte, con los rubros que eligió el contador.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn estados_financieros<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    corte: NaiveDate,
) -> AppResult<EstadosFinancieros>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    use chrono::Datelike;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    let rubros = tx.rubros().await?;
    let acumulados = tx
        .saldos(origen(), corte, AgrupacionSaldos::default())
        .await?;
    let inicio_anio = NaiveDate::from_ymd_opt(corte.year(), 1, 1).unwrap_or(corte);
    let del_anio = tx
        .saldos(
            inicio_anio,
            corte,
            AgrupacionSaldos {
                sin_cierre_anual: true,
                ..AgrupacionSaldos::default()
            },
        )
        .await?;
    // El estado de resultados es el movimiento del año: sin el saldo inicial.
    let movimiento_anio: Vec<FilaSaldo> = del_anio
        .into_iter()
        .map(|f| FilaSaldo {
            saldo_inicial: Decimal::ZERO,
            ..f
        })
        .collect();
    let cierre = tx.cierre_anual_vigente(corte.year()).await?.is_some()
        && corte >= NaiveDate::from_ymd_opt(corte.year(), 12, 31).unwrap_or(corte);
    let saldos_resultado = a_saldos(&movimiento_anio);
    Ok(EstadosFinancieros {
        corte,
        situacion_financiera: valores_rubros(
            &rubros_de(&rubros, "situacion_financiera"),
            &a_saldos(&acumulados),
        ),
        resultados: valores_rubros(&rubros_de(&rubros, "resultados"), &saldos_resultado),
        resultado_ejercicio: resultado_del_ejercicio(&saldos_resultado),
        cierre_contabilizado: cierre,
        notas: tx.notas(corte.year()).await?,
    })
}

/// Escapa un campo CSV (comillas si trae coma, comilla o salto de línea).
fn campo(texto: &str) -> String {
    if texto.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", texto.replace('"', "\"\""))
    } else {
        texto.to_owned()
    }
}

/// Arma un CSV (UTF-8, coma, encabezado en la primera fila).
pub fn a_csv(encabezado: &[&str], filas: &[Vec<String>]) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{}",
        encabezado
            .iter()
            .map(|c| campo(c))
            .collect::<Vec<_>>()
            .join(",")
    );
    for f in filas {
        let _ = writeln!(
            s,
            "{}",
            f.iter().map(|c| campo(c)).collect::<Vec<_>>().join(",")
        );
    }
    s
}

/// Registra la exportación de un libro en la auditoría con su filtro, número de filas y huella
/// SHA-256 del contenido, y devuelve la huella.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.exportar`.
pub async fn registrar_exportacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    libro: &str,
    filtro: &str,
    filas: usize,
    contenido: &str,
) -> AppResult<String>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadExportar).await?;
    let huella = hex::encode(Sha256::digest(contenido.as_bytes()));
    let detalle = DetalleAuditoria::vacio()
        .con("libro", libro)?
        .con("filtro", filtro)?
        .con("filas", filas)?
        .con("sha256", huella.as_str())?
        .con("generado_en", reloj.ahora().to_rfc3339())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EXPORTAR, "entidad", ctx.tenant_id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(huella)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_csv_escapa_comas_comillas_y_saltos() {
        let csv = a_csv(
            &["cuenta", "nombre"],
            &[
                vec!["1105".into(), "Caja, general".into()],
                vec!["2205".into(), "Dice \"hola\"".into()],
            ],
        );
        assert_eq!(
            csv,
            "cuenta,nombre\n1105,\"Caja, general\"\n2205,\"Dice \"\"hola\"\"\"\n"
        );
    }
}
