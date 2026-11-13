//! Tesorería, cuentas por pagar y por cobrar (plan 13, ADR 0018): estados de los documentos,
//! cálculo de retenciones e IVA, edades de cartera y la conciliación automática.
//!
//! Todo es genérico (cualquier entidad). Los montos son [`Monto`] o `Decimal` con signo cuando
//! el sentido importa (movimientos del extracto); nunca flotantes.

use chrono::NaiveDate;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

use super::{Monto, ResponsabilidadFiscal};
use crate::DominioError;

/// Tipo de cuenta de tesorería.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoCuentaTesoreria {
    Banco,
    CajaGeneral,
    CajaMenor,
    /// Dinero con destinación específica (p. ej. el fondo de imprevistos de una PH).
    Fondo,
}

enum_texto!(TipoCuentaTesoreria {
    Banco => "banco",
    CajaGeneral => "caja_general",
    CajaMenor => "caja_menor",
    Fondo => "fondo",
});

/// Tipo de cuenta bancaria.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoCuentaBancaria {
    Ahorros,
    Corriente,
}

enum_texto!(TipoCuentaBancaria {
    Ahorros => "ahorros",
    Corriente => "corriente",
});

/// Deja solo los cuatro últimos dígitos de un número de cuenta (`****1234`): el número completo
/// no se guarda.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si no tiene al menos cuatro dígitos.
pub fn enmascarar_numero_cuenta(numero: &str) -> Result<String, DominioError> {
    let digitos: Vec<char> = numero.chars().filter(char::is_ascii_digit).collect();
    if digitos.len() < 4
        || numero
            .chars()
            .any(|c| !(c.is_ascii_digit() || c == '-' || c == ' '))
    {
        return Err(DominioError::DatoInvalido {
            campo: "numero_cuenta",
            regla: "dígitos (con guiones o espacios), al menos cuatro",
        });
    }
    let ultimos: String = digitos[digitos.len() - 4..].iter().collect();
    Ok(format!("****{ultimos}"))
}

/// Medio de un recibo de caja.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MedioRecibo {
    Efectivo,
    Transferencia,
    Consignacion,
    Pasarela,
    Cheque,
}

enum_texto!(MedioRecibo {
    Efectivo => "efectivo",
    Transferencia => "transferencia",
    Consignacion => "consignacion",
    Pasarela => "pasarela",
    Cheque => "cheque",
});

/// Medio de un egreso. El cheque es solo un medio con su número (sin chequera).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MedioEgreso {
    Efectivo,
    Transferencia,
    Consignacion,
    Pse,
    Cheque,
}

enum_texto!(MedioEgreso {
    Efectivo => "efectivo",
    Transferencia => "transferencia",
    Consignacion => "consignacion",
    Pse => "pse",
    Cheque => "cheque",
});

/// Estado de un recibo de caja: se contabiliza al registrarse; anular = reverso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoRecibo {
    Registrado,
    Anulado,
}

enum_texto!(EstadoRecibo {
    Registrado => "registrado",
    Anulado => "anulado",
});

fn transicion<E: Copy>(
    entidad: &'static str,
    desde: E,
    hacia: E,
    texto: fn(E) -> &'static str,
    permitida: bool,
) -> Result<E, DominioError> {
    if permitida {
        Ok(hacia)
    } else {
        Err(DominioError::TransicionInvalida {
            entidad,
            desde: texto(desde),
            hacia: texto(hacia),
        })
    }
}

/// Estado de un egreso: registrar → autorizar (otra cuenta) → pagar (se contabiliza).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoEgreso {
    PorAutorizar,
    Autorizado,
    Pagado,
    Anulado,
}

enum_texto!(EstadoEgreso {
    PorAutorizar => "por_autorizar",
    Autorizado => "autorizado",
    Pagado => "pagado",
    Anulado => "anulado",
});

impl EstadoEgreso {
    /// por autorizar → autorizado → pagado; cualquiera menos anulado → anulado (el pagado se
    /// anula con reverso).
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(self, hacia: EstadoEgreso) -> Result<EstadoEgreso, DominioError> {
        use EstadoEgreso::{Anulado, Autorizado, Pagado, PorAutorizar};
        let ok = matches!(
            (self, hacia),
            (PorAutorizar, Autorizado)
                | (Autorizado, Pagado)
                | (PorAutorizar | Autorizado | Pagado, Anulado)
        );
        transicion("egreso", self, hacia, Self::as_str, ok)
    }
}

/// Estado de una factura de proveedor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoFacturaProveedor {
    Registrada,
    /// Contabilizada (gasto, IVA, retenciones y cuenta por pagar).
    Causada,
    PagadaParcial,
    Pagada,
    Anulada,
}

enum_texto!(EstadoFacturaProveedor {
    Registrada => "registrada",
    Causada => "causada",
    PagadaParcial => "pagada_parcial",
    Pagada => "pagada",
    Anulada => "anulada",
});

impl EstadoFacturaProveedor {
    /// registrada → causada → pagada parcial ⇄ pagada (por pagos y sus anulaciones); se anula
    /// registrada o causada sin pagos.
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(
        self,
        hacia: EstadoFacturaProveedor,
    ) -> Result<EstadoFacturaProveedor, DominioError> {
        use EstadoFacturaProveedor::{Anulada, Causada, Pagada, PagadaParcial, Registrada};
        let ok = matches!(
            (self, hacia),
            (Registrada, Causada | Anulada)
                | (Causada, PagadaParcial | Pagada | Anulada)
                | (PagadaParcial, Pagada | Causada | PagadaParcial)
                | (Pagada, PagadaParcial | Causada)
        );
        transicion("factura de proveedor", self, hacia, Self::as_str, ok)
    }

    /// ¿Admite pagos?
    pub fn admite_pago(self) -> bool {
        matches!(self, Self::Causada | Self::PagadaParcial)
    }
}

/// Estado de una factura de venta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoFacturaVenta {
    Borrador,
    Emitida,
    PagadaParcial,
    Pagada,
    Anulada,
}

enum_texto!(EstadoFacturaVenta {
    Borrador => "borrador",
    Emitida => "emitida",
    PagadaParcial => "pagada_parcial",
    Pagada => "pagada",
    Anulada => "anulada",
});

impl EstadoFacturaVenta {
    /// borrador → emitida → pagada parcial ⇄ pagada (por recaudos y sus anulaciones); se anula
    /// emitida sin recaudos (el borrador se borra).
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(
        self,
        hacia: EstadoFacturaVenta,
    ) -> Result<EstadoFacturaVenta, DominioError> {
        use EstadoFacturaVenta::{Anulada, Borrador, Emitida, Pagada, PagadaParcial};
        let ok = matches!(
            (self, hacia),
            (Borrador, Emitida)
                | (Emitida, PagadaParcial | Pagada | Anulada)
                | (PagadaParcial, Pagada | Emitida | PagadaParcial)
                | (Pagada, PagadaParcial | Emitida)
        );
        transicion("factura de venta", self, hacia, Self::as_str, ok)
    }

    /// ¿Admite recaudos?
    pub fn admite_recaudo(self) -> bool {
        matches!(self, Self::Emitida | Self::PagadaParcial)
    }
}

/// Estado de pago de un documento según lo pagado: `None` = sin pagos, `Some(false)` = parcial,
/// `Some(true)` = total.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si lo pagado supera el total.
pub fn situacion_pago(total: Monto, pagado: Monto) -> Result<Option<bool>, DominioError> {
    if pagado > total {
        return Err(DominioError::DatoInvalido {
            campo: "valor",
            regla: "lo pagado no puede superar el saldo del documento",
        });
    }
    Ok(if pagado.es_cero() {
        None
    } else {
        Some(pagado == total)
    })
}

/// Impuesto de una retención.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpuestoRetencion {
    Renta,
    Iva,
    Ica,
}

enum_texto!(ImpuestoRetencion {
    Renta => "renta",
    Iva => "iva",
    Ica => "ica",
});

impl ImpuestoRetencion {
    /// Concepto del documento origen con que la retención llega al contabilizador.
    pub fn concepto_contable(self) -> &'static str {
        match self {
            Self::Renta => "retencion_renta",
            Self::Iva => "retencion_iva",
            Self::Ica => "retencion_ica",
        }
    }
}

/// Lo que el cálculo necesita de un concepto de retención (versión vigente en la fecha).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarifaRetencion {
    pub impuesto: ImpuestoRetencion,
    /// Base mínima en UVT (0 = desde el primer peso).
    pub base_minima_uvt: Decimal,
    /// Porcentaje.
    pub tarifa: Decimal,
}

/// Lo que el cálculo necesita del proveedor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PerfilRetencionTercero {
    pub responsabilidades: Vec<ResponsabilidadFiscal>,
    /// ¿Es responsable de IVA? (sin el dato no se le retiene IVA).
    pub responsable_iva: bool,
}

/// Valor de una retención, redondeado al peso. `base` es la base gravable (para la de IVA, el
/// IVA de la línea). Devuelve `None` cuando no aplica:
///
/// - la base no alcanza la base mínima en UVT del año de la operación;
/// - renta a un autorretenedor (O-15) o a uno del régimen simple (O-47, art. 911 ET);
/// - IVA a quien no es responsable de IVA.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si el valor de la UVT no es positivo.
pub fn calcular_retencion(
    tarifa: &TarifaRetencion,
    base: Monto,
    uvt: Decimal,
    tercero: &PerfilRetencionTercero,
) -> Result<Option<Monto>, DominioError> {
    if uvt <= Decimal::ZERO {
        return Err(DominioError::DatoInvalido {
            campo: "uvt",
            regla: "debe ser positivo",
        });
    }
    let excluido = match tarifa.impuesto {
        ImpuestoRetencion::Renta => tercero.responsabilidades.iter().any(|r| {
            matches!(
                r,
                ResponsabilidadFiscal::Autorretenedor | ResponsabilidadFiscal::RegimenSimple
            )
        }),
        ImpuestoRetencion::Iva => !tercero.responsable_iva,
        ImpuestoRetencion::Ica => false,
    };
    if excluido || base.es_cero() || base.valor() < tarifa.base_minima_uvt * uvt {
        return Ok(None);
    }
    let valor = (base.valor() * tarifa.tarifa / Decimal::ONE_HUNDRED)
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero);
    if valor.is_zero() {
        return Ok(None);
    }
    Monto::new(valor).map(Some)
}

/// Subtotal e IVA de una línea de venta: `cantidad × valor unitario` y el porcentaje que eligió
/// quien factura, ambos redondeados al centavo.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] si la cantidad no es positiva o la tarifa no está entre 0 y
/// 100.
pub fn liquidar_linea_venta(
    cantidad: Decimal,
    valor_unitario: Monto,
    tarifa_iva: Decimal,
) -> Result<(Monto, Monto), DominioError> {
    if cantidad <= Decimal::ZERO || cantidad.scale() > 4 {
        return Err(DominioError::DatoInvalido {
            campo: "cantidad",
            regla: "positiva y con a lo sumo cuatro decimales",
        });
    }
    if tarifa_iva < Decimal::ZERO || tarifa_iva > Decimal::ONE_HUNDRED {
        return Err(DominioError::DatoInvalido {
            campo: "tarifa_iva",
            regla: "porcentaje entre 0 y 100",
        });
    }
    let redondear =
        |d: Decimal| d.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    let subtotal = Monto::new(redondear(cantidad * valor_unitario.valor()))?;
    if subtotal.es_cero() {
        return Err(DominioError::DatoInvalido {
            campo: "valor_unitario",
            regla: "la línea no puede quedar en cero",
        });
    }
    let iva = Monto::new(redondear(
        subtotal.valor() * tarifa_iva / Decimal::ONE_HUNDRED,
    ))?;
    Ok((subtotal, iva))
}

/// Tramo de edad de una cuenta por cobrar o por pagar a una fecha de corte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TramoEdad {
    /// Sin vencer.
    Corriente,
    #[serde(rename = "de_1_a_30")]
    De1A30,
    #[serde(rename = "de_31_a_60")]
    De31A60,
    #[serde(rename = "de_61_a_90")]
    De61A90,
    MasDe90,
}

enum_texto!(TramoEdad {
    Corriente => "corriente",
    De1A30 => "de_1_a_30",
    De31A60 => "de_31_a_60",
    De61A90 => "de_61_a_90",
    MasDe90 => "mas_de_90",
});

impl TramoEdad {
    /// Tramo según los días vencidos a la fecha de corte.
    pub fn de(vencimiento: NaiveDate, corte: NaiveDate) -> TramoEdad {
        match (corte - vencimiento).num_days() {
            ..=0 => Self::Corriente,
            1..=30 => Self::De1A30,
            31..=60 => Self::De31A60,
            61..=90 => Self::De61A90,
            _ => Self::MasDe90,
        }
    }
}

/// Formato del extracto bancario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatoExtracto {
    /// CSV o XLSX con mapeo de columnas.
    Planilla,
    /// Archivo de recaudo Asobancaria 2001 (texto de ancho fijo).
    #[serde(rename = "asobancaria_2001")]
    Asobancaria2001,
}

enum_texto!(FormatoExtracto {
    Planilla => "planilla",
    Asobancaria2001 => "asobancaria_2001",
});

/// Estado de un movimiento del extracto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoMovimientoExtracto {
    Pendiente,
    Conciliado,
    /// Se deja como partida conciliatoria del informe (no casa con nada en libros).
    PartidaConciliatoria,
}

enum_texto!(EstadoMovimientoExtracto {
    Pendiente => "pendiente",
    Conciliado => "conciliado",
    PartidaConciliatoria => "partida_conciliatoria",
});

/// Cómo se hizo una conciliación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModoConciliacion {
    Automatica,
    Manual,
}

enum_texto!(ModoConciliacion {
    Automatica => "automatica",
    Manual => "manual",
});

/// Partida que se puede conciliar: un movimiento del extracto o una línea contable de la cuenta
/// bancaria. `valor` lleva el signo del banco: positivo entra dinero (débito en libros).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartidaConciliable<K> {
    pub clave: K,
    pub fecha: NaiveDate,
    pub valor: Decimal,
    pub referencia: Option<String>,
}

fn misma_referencia(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => {
            let (x, y) = (x.trim(), y.trim());
            !x.is_empty() && (x.eq_ignore_ascii_case(y) || x.contains(y) || y.contains(x))
        }
        _ => false,
    }
}

/// Conciliación automática uno a uno: a cada movimiento le busca una línea de libros con el
/// **mismo valor** y fecha a lo sumo a `tolerancia_dias`; entre varias candidatas prefiere la de
/// la misma referencia y luego la de fecha más cercana. Cada línea se usa una sola vez. Lo que no
/// casa queda para la conciliación manual.
pub fn conciliar_automaticamente<M: Clone, L: Clone>(
    movimientos: &[PartidaConciliable<M>],
    lineas: &[PartidaConciliable<L>],
    tolerancia_dias: u32,
) -> Vec<(M, L)> {
    let mut usadas = vec![false; lineas.len()];
    let mut pares = Vec::new();
    for m in movimientos {
        let mejor = lineas
            .iter()
            .enumerate()
            .filter(|(i, l)| {
                !usadas[*i]
                    && l.valor == m.valor
                    && (l.fecha - m.fecha).num_days().unsigned_abs() <= u64::from(tolerancia_dias)
            })
            .min_by_key(|(_, l)| {
                (
                    !misma_referencia(&m.referencia, &l.referencia),
                    (l.fecha - m.fecha).num_days().abs(),
                )
            });
        if let Some((i, l)) = mejor {
            usadas[i] = true;
            pares.push((m.clave.clone(), l.clave.clone()));
        }
    }
    pares
}

/// Una conciliación (manual) es válida si tiene al menos una partida de cada lado y las sumas
/// coinciden exactamente.
///
/// # Errors
///
/// [`DominioError::ReglaContable`] si falta un lado o las sumas difieren.
pub fn validar_conciliacion(
    movimientos: &[Decimal],
    lineas: &[Decimal],
) -> Result<(), DominioError> {
    if movimientos.is_empty() || lineas.is_empty() {
        return Err(DominioError::ReglaContable(
            "una conciliación lleva al menos un movimiento del extracto y una línea de libros"
                .into(),
        ));
    }
    let (m, l): (Decimal, Decimal) = (movimientos.iter().sum(), lineas.iter().sum());
    if m != l {
        return Err(DominioError::ReglaContable(format!(
            "el extracto suma {m} y los libros {l}: no concilian"
        )));
    }
    Ok(())
}

/// Estado de un evento de la pasarela de pagos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoEventoPasarela {
    Pendiente,
    /// La pasarela confirmó el pago: se crea el recibo.
    Aprobado,
    /// La pasarela liquidó el dinero a la cuenta (la disponibilidad se confirma al conciliar).
    Liquidado,
    /// Se reversó: se anula el recibo.
    Reversado,
    Contracargo,
}

enum_texto!(EstadoEventoPasarela {
    Pendiente => "pendiente",
    Aprobado => "aprobado",
    Liquidado => "liquidado",
    Reversado => "reversado",
    Contracargo => "contracargo",
});

impl EstadoEventoPasarela {
    /// ¿Deshace un pago aprobado?
    pub fn deshace_pago(self) -> bool {
        matches!(self, Self::Reversado | Self::Contracargo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(t: &str) -> Decimal {
        t.parse().expect("decimal")
    }

    fn m(t: &str) -> Monto {
        Monto::new(d(t)).expect("monto")
    }

    fn fecha(t: &str) -> NaiveDate {
        t.parse().expect("fecha")
    }

    #[test]
    fn los_enums_viajan_con_su_texto_persistido() {
        for (json, texto) in [
            (
                serde_json::to_value(TramoEdad::De31A60).expect("json"),
                TramoEdad::De31A60.as_str(),
            ),
            (
                serde_json::to_value(FormatoExtracto::Asobancaria2001).expect("json"),
                FormatoExtracto::Asobancaria2001.as_str(),
            ),
            (
                serde_json::to_value(EstadoEgreso::PorAutorizar).expect("json"),
                EstadoEgreso::PorAutorizar.as_str(),
            ),
        ] {
            assert_eq!(json, serde_json::json!(texto));
        }
    }

    #[test]
    fn el_numero_de_cuenta_se_enmascara() {
        assert_eq!(
            enmascarar_numero_cuenta("123-456789-01").expect("ok"),
            "****8901"
        );
        assert!(enmascarar_numero_cuenta("12a4567").is_err());
        assert!(enmascarar_numero_cuenta("123").is_err());
    }

    #[test]
    fn transiciones_del_egreso() {
        use EstadoEgreso::*;
        assert!(PorAutorizar.transicionar(Autorizado).is_ok());
        assert!(Autorizado.transicionar(Pagado).is_ok());
        assert!(Pagado.transicionar(Anulado).is_ok());
        assert!(
            PorAutorizar.transicionar(Pagado).is_err(),
            "no se paga sin autorizar"
        );
        assert!(Anulado.transicionar(PorAutorizar).is_err());
    }

    #[test]
    fn transiciones_de_las_facturas() {
        use EstadoFacturaProveedor as P;
        use EstadoFacturaVenta as V;
        assert!(P::Registrada.transicionar(P::Causada).is_ok());
        assert!(P::Registrada.transicionar(P::Pagada).is_err());
        assert!(
            P::Pagada.transicionar(P::Anulada).is_err(),
            "con pagos no se anula"
        );
        assert!(V::Borrador.transicionar(V::Emitida).is_ok());
        assert!(V::Borrador.transicionar(V::Pagada).is_err());
        assert!(V::PagadaParcial.transicionar(V::Anulada).is_err());
        assert!(V::Emitida.admite_recaudo() && !V::Pagada.admite_recaudo());
        assert!(P::Causada.admite_pago() && !P::Registrada.admite_pago());
    }

    #[test]
    fn situacion_de_pago() {
        assert_eq!(situacion_pago(m("100"), m("0")).expect("ok"), None);
        assert_eq!(situacion_pago(m("100"), m("40")).expect("ok"), Some(false));
        assert_eq!(situacion_pago(m("100"), m("100")).expect("ok"), Some(true));
        assert!(situacion_pago(m("100"), m("100.01")).is_err());
    }

    fn renta(base_uvt: &str, tarifa: &str) -> TarifaRetencion {
        TarifaRetencion {
            impuesto: ImpuestoRetencion::Renta,
            base_minima_uvt: d(base_uvt),
            tarifa: d(tarifa),
        }
    }

    #[test]
    fn la_retencion_aplica_desde_la_base_minima_en_uvt() {
        let tercero = PerfilRetencionTercero::default();
        let tarifa = renta("4", "4");
        let uvt = d("50000");

        let bajo = calcular_retencion(&tarifa, m("199999"), uvt, &tercero).expect("ok");
        let justo = calcular_retencion(&tarifa, m("200000"), uvt, &tercero).expect("ok");

        assert_eq!(bajo, None);
        assert_eq!(justo, Some(m("8000")));
    }

    #[test]
    fn la_retencion_se_redondea_al_peso() {
        let r = calcular_retencion(
            &renta("0", "2.5"),
            m("1001"),
            d("50000"),
            &PerfilRetencionTercero::default(),
        )
        .expect("ok");
        assert_eq!(r, Some(m("25")), "25,025 → 25");
    }

    #[test]
    fn autorretenedores_y_simple_no_tienen_retencion_de_renta() {
        for r in [
            ResponsabilidadFiscal::Autorretenedor,
            ResponsabilidadFiscal::RegimenSimple,
        ] {
            let tercero = PerfilRetencionTercero {
                responsabilidades: vec![r],
                responsable_iva: true,
            };
            assert_eq!(
                calcular_retencion(&renta("0", "4"), m("1000000"), d("50000"), &tercero)
                    .expect("ok"),
                None
            );
        }
    }

    #[test]
    fn la_retencion_de_iva_solo_a_responsables() {
        let tarifa = TarifaRetencion {
            impuesto: ImpuestoRetencion::Iva,
            base_minima_uvt: d("0"),
            tarifa: d("15"),
        };
        let no = PerfilRetencionTercero::default();
        let si = PerfilRetencionTercero {
            responsable_iva: true,
            ..Default::default()
        };
        assert_eq!(
            calcular_retencion(&tarifa, m("190000"), d("50000"), &no).expect("ok"),
            None
        );
        assert_eq!(
            calcular_retencion(&tarifa, m("190000"), d("50000"), &si).expect("ok"),
            Some(m("28500"))
        );
        assert!(calcular_retencion(&tarifa, m("1"), d("0"), &si).is_err());
    }

    #[test]
    fn liquida_la_linea_de_venta_sin_iva_automatico() {
        let (sub, iva) = liquidar_linea_venta(d("3"), m("33333.33"), d("0")).expect("ok");
        assert_eq!((sub, iva), (m("99999.99"), m("0")));
        let (sub, iva) = liquidar_linea_venta(d("1"), m("100000"), d("19")).expect("ok");
        assert_eq!((sub, iva), (m("100000"), m("19000")));
        assert!(liquidar_linea_venta(d("0"), m("1"), d("0")).is_err());
        assert!(liquidar_linea_venta(d("1"), m("1"), d("101")).is_err());
        assert!(liquidar_linea_venta(d("0.001"), m("1"), d("0")).is_err());
    }

    #[test]
    fn tramos_de_edad() {
        let v = fecha("2026-01-31");
        assert_eq!(TramoEdad::de(v, fecha("2026-01-31")), TramoEdad::Corriente);
        assert_eq!(TramoEdad::de(v, fecha("2026-02-01")), TramoEdad::De1A30);
        assert_eq!(TramoEdad::de(v, fecha("2026-03-31")), TramoEdad::De31A60);
        assert_eq!(TramoEdad::de(v, fecha("2026-04-30")), TramoEdad::De61A90);
        assert_eq!(
            TramoEdad::de(v, fecha("2026-05-01")),
            TramoEdad::De61A90,
            "90 días"
        );
        assert_eq!(TramoEdad::de(v, fecha("2026-05-02")), TramoEdad::MasDe90);
    }

    fn partida<K>(
        clave: K,
        f: &str,
        valor: &str,
        referencia: Option<&str>,
    ) -> PartidaConciliable<K> {
        PartidaConciliable {
            clave,
            fecha: fecha(f),
            valor: d(valor),
            referencia: referencia.map(str::to_owned),
        }
    }

    #[test]
    fn concilia_por_valor_fecha_y_referencia() {
        let movimientos = [
            partida("m1", "2026-03-05", "100", Some("REF-9")),
            partida("m2", "2026-03-10", "-50", None),
            partida("m3", "2026-03-20", "70", None),
        ];
        let lineas = [
            partida("l1", "2026-03-04", "100", None),
            partida("l2", "2026-03-06", "100", Some("ref-9")),
            partida("l3", "2026-03-08", "-50", None),
            partida("l4", "2026-03-01", "70", None),
        ];

        let pares = conciliar_automaticamente(&movimientos, &lineas, 3);

        assert_eq!(
            pares,
            vec![("m1", "l2"), ("m2", "l3")],
            "m1 prefiere la referencia; m3 queda fuera de la tolerancia"
        );
    }

    #[test]
    fn cada_linea_se_concilia_una_sola_vez() {
        let movimientos = [
            partida(1, "2026-03-05", "100", None),
            partida(2, "2026-03-05", "100", None),
        ];
        let lineas = [partida('a', "2026-03-05", "100", None)];
        assert_eq!(
            conciliar_automaticamente(&movimientos, &lineas, 0),
            vec![(1, 'a')]
        );
    }

    #[test]
    fn la_conciliacion_manual_exige_sumas_iguales() {
        assert!(validar_conciliacion(&[d("60"), d("40")], &[d("100")]).is_ok());
        assert!(validar_conciliacion(&[d("60")], &[d("100")]).is_err());
        assert!(validar_conciliacion(&[], &[d("100")]).is_err());
    }
}
