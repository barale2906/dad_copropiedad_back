//! Planilla de empleados (plan 16 fase 1): una fila por empleado con su primer contrato. Sin
//! IO; las administradoras (por su código de la PILA), el catálogo laboral vigente y que la
//! persona no sea ya empleada los verifica el caso de uso.
//!
//! Los encabezados y los valores de las listas no distinguen mayúsculas ni tildes (`Término
//! fijo` = `termino_fijo`). Fechas en `AAAA-MM-DD` o `DD/MM/AAAA`; montos con punto o coma
//! decimal y sin separador de miles.
//!
//! | Columna | Obligatoria | Ejemplo |
//! |---|---|---|
//! | `codigo` | sí | `E-001` |
//! | `nombre` | sí | `Ana Pérez` |
//! | `tipo_documento` | sí | `CC` |
//! | `documento` | sí | `52123456` |
//! | `correo`, `telefono` | no | `ana@correo.co`, `573001234567` |
//! | `tipo_contrato` | sí | `termino_fijo`, `termino_indefinido`, `obra_labor`, `aprendizaje` |
//! | `fecha_inicio` | sí | `2026-02-01` |
//! | `fecha_fin` | término fijo y aprendizaje | `2027-01-31` |
//! | `descripcion_obra` | obra o labor | `Pintura de fachadas` |
//! | `cargo` | sí | `Todero` |
//! | `salario` | sí | `1750000` |
//! | `tipo_salario` | no (`ordinario`; `apoyo_sostenimiento` en aprendizaje) | `integral` |
//! | `etapa_aprendizaje` | aprendizaje | `lectiva` |
//! | `jornada` | no (`completa`) | `parcial` |
//! | `horas_semanales` | jornada parcial | `21` |
//! | `periodicidad` | no (`mensual`) | `quincenal` |
//! | `eps` | sí (código PILA) | `EPS037` |
//! | `afp`, `cesantias`, `ccf`, `arl` | según el contrato (código PILA) | `230301` |
//! | `clase_riesgo` | con ARL | `1` |
//! | `pensionado` | no (`no`) | `si` |
//! | `medio_pago` | no (`transferencia` si trae cuenta, si no `efectivo`) | `cheque` |
//! | `banco`, `tipo_cuenta`, `numero_cuenta` | con transferencia | `Bancolombia`, `ahorros`, `0012345678` |
//!
//! Un código o un documento repetido en la planilla es un error; los mensajes nunca repiten el
//! valor de la celda.

use std::collections::BTreeSet;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::{Documento, ErrorFila, normalizar_encabezado};
use crate::{
    DireccionCorreo,
    censo::TipoDocumento,
    contabilidad::{Monto, tesoreria::TipoCuentaBancaria},
    copropiedad::Telefono,
    nomina::{
        ClaseRiesgo, CodigoEmpleado, EtapaAprendizaje, Jornada, MedioPago, PeriodicidadPago,
        TipoContratoLaboral, TipoSalario,
    },
};

/// Filas de datos máximas de una planilla de empleados.
pub const MAXIMO_FILAS_EMPLEADOS: usize = 2_000;

/// Columnas de la planilla de empleados, en el orden de la plantilla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Columna {
    Codigo,
    Nombre,
    TipoDocumento,
    Documento,
    Correo,
    Telefono,
    TipoContrato,
    FechaInicio,
    FechaFin,
    DescripcionObra,
    Cargo,
    Salario,
    TipoSalario,
    EtapaAprendizaje,
    Jornada,
    HorasSemanales,
    Periodicidad,
    Eps,
    Afp,
    Cesantias,
    Ccf,
    Arl,
    ClaseRiesgo,
    Pensionado,
    MedioPago,
    Banco,
    TipoCuenta,
    NumeroCuenta,
}

impl Columna {
    const TODAS: [Self; 28] = [
        Self::Codigo,
        Self::Nombre,
        Self::TipoDocumento,
        Self::Documento,
        Self::Correo,
        Self::Telefono,
        Self::TipoContrato,
        Self::FechaInicio,
        Self::FechaFin,
        Self::DescripcionObra,
        Self::Cargo,
        Self::Salario,
        Self::TipoSalario,
        Self::EtapaAprendizaje,
        Self::Jornada,
        Self::HorasSemanales,
        Self::Periodicidad,
        Self::Eps,
        Self::Afp,
        Self::Cesantias,
        Self::Ccf,
        Self::Arl,
        Self::ClaseRiesgo,
        Self::Pensionado,
        Self::MedioPago,
        Self::Banco,
        Self::TipoCuenta,
        Self::NumeroCuenta,
    ];

    const OBLIGATORIAS: [Self; 9] = [
        Self::Codigo,
        Self::Nombre,
        Self::TipoDocumento,
        Self::Documento,
        Self::TipoContrato,
        Self::FechaInicio,
        Self::Cargo,
        Self::Salario,
        Self::Eps,
    ];

    const fn nombre(self) -> &'static str {
        match self {
            Self::Codigo => "codigo",
            Self::Nombre => "nombre",
            Self::TipoDocumento => "tipo_documento",
            Self::Documento => "documento",
            Self::Correo => "correo",
            Self::Telefono => "telefono",
            Self::TipoContrato => "tipo_contrato",
            Self::FechaInicio => "fecha_inicio",
            Self::FechaFin => "fecha_fin",
            Self::DescripcionObra => "descripcion_obra",
            Self::Cargo => "cargo",
            Self::Salario => "salario",
            Self::TipoSalario => "tipo_salario",
            Self::EtapaAprendizaje => "etapa_aprendizaje",
            Self::Jornada => "jornada",
            Self::HorasSemanales => "horas_semanales",
            Self::Periodicidad => "periodicidad",
            Self::Eps => "eps",
            Self::Afp => "afp",
            Self::Cesantias => "cesantias",
            Self::Ccf => "ccf",
            Self::Arl => "arl",
            Self::ClaseRiesgo => "clase_riesgo",
            Self::Pensionado => "pensionado",
            Self::MedioPago => "medio_pago",
            Self::Banco => "banco",
            Self::TipoCuenta => "tipo_cuenta",
            Self::NumeroCuenta => "numero_cuenta",
        }
    }
}

/// Encabezados de la plantilla de empleados, en orden (para generar la plantilla CSV).
pub fn encabezados_empleados() -> Vec<&'static str> {
    Columna::TODAS.iter().map(|c| c.nombre()).collect()
}

/// Cuenta bancaria leída de la planilla. Dato personal: `Debug` oculta el número.
#[derive(Clone, PartialEq, Eq)]
pub struct CuentaImportada {
    pub banco: String,
    pub tipo: TipoCuentaBancaria,
    pub numero: String,
}

impl std::fmt::Debug for CuentaImportada {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CuentaImportada(<oculta>)")
    }
}

/// Empleado leído de una fila válida (las administradoras van por su código de la PILA).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmpleadoImportado {
    /// Número de la fila en la planilla (la 1 son los encabezados).
    pub fila: u32,
    pub codigo: CodigoEmpleado,
    pub nombre: String,
    pub tipo_documento: TipoDocumento,
    pub documento: Documento,
    pub correo: Option<DireccionCorreo>,
    pub telefono: Option<Telefono>,
    pub tipo_contrato: TipoContratoLaboral,
    pub fecha_inicio: NaiveDate,
    pub fecha_fin: Option<NaiveDate>,
    pub descripcion_obra: Option<String>,
    pub cargo: String,
    pub salario: Monto,
    pub tipo_salario: TipoSalario,
    pub etapa_aprendizaje: Option<EtapaAprendizaje>,
    pub jornada: Jornada,
    pub horas_semanales: Option<Decimal>,
    pub periodicidad: PeriodicidadPago,
    pub eps: String,
    pub afp: Option<String>,
    pub cesantias: Option<String>,
    pub ccf: Option<String>,
    pub arl: Option<String>,
    pub clase_riesgo: Option<ClaseRiesgo>,
    pub pensionado: bool,
    pub medio_pago: MedioPago,
    pub cuenta: Option<CuentaImportada>,
}

/// Resultado de leer la planilla: los empleados válidos y los errores por fila.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanillaEmpleados {
    pub empleados: Vec<EmpleadoImportado>,
    pub errores: Vec<ErrorFila>,
    /// Filas de datos no vacías.
    pub total_filas: u32,
}

/// Lee la planilla (la primera fila son los encabezados).
pub fn leer_planilla_empleados(filas: &[Vec<String>]) -> PlanillaEmpleados {
    let mut p = PlanillaEmpleados::default();
    let Some((encabezado, datos)) = filas.split_first() else {
        p.errores
            .push(ErrorFila::new(1, None, "la planilla está vacía"));
        return p;
    };
    let columnas = match encabezados(encabezado) {
        Ok(c) => c,
        Err(errores) => {
            p.errores = errores;
            return p;
        }
    };
    let datos: Vec<(u32, &Vec<String>)> = datos
        .iter()
        .enumerate()
        .filter(|(_, f)| f.iter().any(|c| !c.trim().is_empty()))
        .map(|(i, f)| (u32::try_from(i + 2).unwrap_or(u32::MAX), f))
        .collect();
    p.total_filas = u32::try_from(datos.len()).unwrap_or(u32::MAX);
    if datos.len() > MAXIMO_FILAS_EMPLEADOS {
        p.errores.push(ErrorFila::new(
            1,
            None,
            format!("máximo {MAXIMO_FILAS_EMPLEADOS} filas por archivo"),
        ));
        return p;
    }
    let mut codigos = BTreeSet::new();
    let mut documentos = BTreeSet::new();
    for (n, celdas) in datos {
        match leer_fila(n, celdas, &columnas) {
            Ok(e) if !codigos.insert(e.codigo.as_str().to_owned()) => p.errores.push(
                ErrorFila::new(n, Some("codigo"), "el código se repite en la planilla"),
            ),
            Ok(e) if !documentos.insert(e.documento.as_str().to_owned()) => {
                p.errores.push(ErrorFila::new(
                    n,
                    Some("documento"),
                    "el documento se repite en la planilla",
                ))
            }
            Ok(e) => p.empleados.push(e),
            Err(errores) => p.errores.extend(errores),
        }
    }
    p
}

fn encabezados(fila: &[String]) -> Result<Vec<(Columna, usize)>, Vec<ErrorFila>> {
    let mut columnas: Vec<(Columna, usize)> = Vec::new();
    let mut errores = Vec::new();
    for (i, celda) in fila.iter().enumerate() {
        let nombre = normalizar_encabezado(celda);
        if let Some(c) = Columna::TODAS.iter().find(|c| c.nombre() == nombre) {
            if columnas.iter().any(|(x, _)| x == c) {
                errores.push(ErrorFila::new(1, Some(c.nombre()), "columna repetida"));
            } else {
                columnas.push((*c, i));
            }
        }
    }
    for obligatoria in Columna::OBLIGATORIAS {
        if !columnas.iter().any(|(c, _)| *c == obligatoria) {
            errores.push(ErrorFila::new(
                1,
                Some(obligatoria.nombre()),
                "falta la columna obligatoria",
            ));
        }
    }
    if errores.is_empty() {
        Ok(columnas)
    } else {
        Err(errores)
    }
}

/// Fecha en `AAAA-MM-DD` o `DD/MM/AAAA`.
fn fecha(texto: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(texto, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(texto, "%d/%m/%Y"))
        .ok()
}

/// Número con punto o coma decimal, sin separador de miles.
fn numero(texto: &str) -> Option<Decimal> {
    texto
        .trim_start_matches('$')
        .trim()
        .replace(',', ".")
        .parse::<Decimal>()
        .ok()
}

fn tipo_contrato(texto: &str) -> Option<TipoContratoLaboral> {
    match normalizar_encabezado(texto).as_str() {
        "fijo" => Some(TipoContratoLaboral::TerminoFijo),
        "indefinido" => Some(TipoContratoLaboral::TerminoIndefinido),
        "obra_o_labor" | "obra" => Some(TipoContratoLaboral::ObraLabor),
        otro => TipoContratoLaboral::try_from(otro).ok(),
    }
}

fn si_no(texto: &str) -> Option<bool> {
    match normalizar_encabezado(texto).as_str() {
        "si" | "s" | "true" | "1" | "verdadero" => Some(true),
        "no" | "n" | "false" | "0" | "falso" => Some(false),
        _ => None,
    }
}

fn leer_fila(
    n: u32,
    celdas: &[String],
    columnas: &[(Columna, usize)],
) -> Result<EmpleadoImportado, Vec<ErrorFila>> {
    let celda = |c: Columna| {
        columnas
            .iter()
            .find(|(x, _)| *x == c)
            .and_then(|(_, i)| celdas.get(*i))
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    };
    let mut errores: Vec<ErrorFila> = Vec::new();
    // Lee una celda obligatoria u opcional con su conversión; registra el error sin el valor.
    let mut leer = |c: Columna, obligatoria: bool, mensaje: &str, f: &dyn Fn(&str) -> Option<_>| {
        match celda(c) {
            None if obligatoria => {
                errores.push(ErrorFila::new(n, Some(c.nombre()), "falta el dato"));
                None
            }
            None => None,
            Some(v) => {
                let r = f(v);
                if r.is_none() {
                    errores.push(ErrorFila::new(n, Some(c.nombre()), mensaje));
                }
                r
            }
        }
    };
    let codigo = leer(Columna::Codigo, true, "código inválido", &|v| {
        CodigoEmpleado::new(v).ok().map(Valor::Codigo)
    });
    let nombre = leer(
        Columna::Nombre,
        true,
        "el nombre supera 200 caracteres",
        &|v| (v.chars().count() <= 200).then(|| Valor::Texto(v.to_owned())),
    );
    let tipo_documento = leer(
        Columna::TipoDocumento,
        true,
        "tipo de documento inválido",
        &|v| {
            TipoDocumento::try_from(v.to_uppercase().as_str())
                .ok()
                .map(Valor::TipoDocumento)
        },
    );
    let documento = leer(Columna::Documento, true, "documento inválido", &|v| {
        Documento::new(v).ok().map(Valor::Documento)
    });
    let correo = leer(Columna::Correo, false, "correo inválido", &|v| {
        DireccionCorreo::new(v).ok().map(Valor::Correo)
    });
    let telefono = leer(Columna::Telefono, false, "teléfono inválido", &|v| {
        Telefono::new(v).ok().map(Valor::Telefono)
    });
    let tipo = leer(
        Columna::TipoContrato,
        true,
        "tipo de contrato desconocido",
        &|v| tipo_contrato(v).map(Valor::TipoContrato),
    );
    let inicio = leer(Columna::FechaInicio, true, "fecha inválida", &|v| {
        fecha(v).map(Valor::Fecha)
    });
    let fin = leer(Columna::FechaFin, false, "fecha inválida", &|v| {
        fecha(v).map(Valor::Fecha)
    });
    let obra = leer(
        Columna::DescripcionObra,
        false,
        "la descripción supera 300 caracteres",
        &|v| (v.chars().count() <= 300).then(|| Valor::Texto(v.to_owned())),
    );
    let cargo = leer(
        Columna::Cargo,
        true,
        "el cargo supera 120 caracteres",
        &|v| (v.chars().count() <= 120).then(|| Valor::Texto(v.to_owned())),
    );
    let salario = leer(
        Columna::Salario,
        true,
        "monto inválido (sin separador de miles)",
        &|v| {
            numero(v)
                .filter(|d| *d > Decimal::ZERO)
                .and_then(|d| Monto::new(d).ok())
                .map(Valor::Monto)
        },
    );
    let tipo_salario = leer(
        Columna::TipoSalario,
        false,
        "tipo de salario desconocido",
        &|v| {
            TipoSalario::try_from(normalizar_encabezado(v).as_str())
                .ok()
                .map(Valor::TipoSalario)
        },
    );
    let etapa = leer(
        Columna::EtapaAprendizaje,
        false,
        "etapa desconocida",
        &|v| {
            EtapaAprendizaje::try_from(normalizar_encabezado(v).as_str())
                .ok()
                .map(Valor::Etapa)
        },
    );
    let jornada = leer(Columna::Jornada, false, "jornada desconocida", &|v| {
        Jornada::try_from(normalizar_encabezado(v).as_str())
            .ok()
            .map(Valor::Jornada)
    });
    let horas = leer(Columna::HorasSemanales, false, "horas inválidas", &|v| {
        numero(v).filter(|d| *d > Decimal::ZERO).map(Valor::Decimal)
    });
    let periodicidad = leer(
        Columna::Periodicidad,
        false,
        "periodicidad desconocida",
        &|v| {
            PeriodicidadPago::try_from(normalizar_encabezado(v).as_str())
                .ok()
                .map(Valor::Periodicidad)
        },
    );
    let codigo_pila = |v: &str| Some(Valor::Texto(v.to_uppercase()));
    let eps = leer(Columna::Eps, true, "código inválido", &codigo_pila);
    let afp = leer(Columna::Afp, false, "código inválido", &codigo_pila);
    let cesantias = leer(Columna::Cesantias, false, "código inválido", &codigo_pila);
    let ccf = leer(Columna::Ccf, false, "código inválido", &codigo_pila);
    let arl = leer(Columna::Arl, false, "código inválido", &codigo_pila);
    let clase = leer(
        Columna::ClaseRiesgo,
        false,
        "clase de riesgo de 1 a 5",
        &|v| {
            v.parse::<i16>()
                .ok()
                .and_then(|x| ClaseRiesgo::de_numero(x).ok())
                .map(Valor::Clase)
        },
    );
    let pensionado = leer(Columna::Pensionado, false, "use sí o no", &|v| {
        si_no(v).map(Valor::Booleano)
    });
    let medio = leer(
        Columna::MedioPago,
        false,
        "medio de pago desconocido",
        &|v| {
            MedioPago::try_from(normalizar_encabezado(v).as_str())
                .ok()
                .map(Valor::Medio)
        },
    );
    let banco = leer(
        Columna::Banco,
        false,
        "el banco supera 120 caracteres",
        &|v| (v.chars().count() <= 120).then(|| Valor::Texto(v.to_owned())),
    );
    let tipo_cuenta = leer(
        Columna::TipoCuenta,
        false,
        "tipo de cuenta desconocido",
        &|v| {
            TipoCuentaBancaria::try_from(normalizar_encabezado(v).as_str())
                .ok()
                .map(Valor::TipoCuenta)
        },
    );
    let numero_cuenta = leer(Columna::NumeroCuenta, false, "de 4 a 20 dígitos", &|v| {
        ((4..=20).contains(&v.len()) && v.chars().all(|c| c.is_ascii_digit()))
            .then(|| Valor::Texto(v.to_owned()))
    });

    let cuenta = match (banco, tipo_cuenta, numero_cuenta) {
        (Some(Valor::Texto(banco)), Some(Valor::TipoCuenta(tipo)), Some(Valor::Texto(numero))) => {
            Some(CuentaImportada {
                banco,
                tipo,
                numero,
            })
        }
        (None, None, None) => None,
        _ => {
            errores.push(ErrorFila::new(
                n,
                Some("numero_cuenta"),
                "la cuenta lleva banco, tipo y número",
            ));
            None
        }
    };
    let tipo_contrato = match tipo {
        Some(Valor::TipoContrato(t)) => Some(t),
        _ => None,
    };
    let medio_pago = match medio {
        Some(Valor::Medio(m)) => m,
        _ if cuenta.is_some() => MedioPago::Transferencia,
        _ => MedioPago::Efectivo,
    };
    let tipo_salario = match tipo_salario {
        Some(Valor::TipoSalario(t)) => t,
        _ if tipo_contrato == Some(TipoContratoLaboral::Aprendizaje) => {
            TipoSalario::ApoyoSostenimiento
        }
        _ => TipoSalario::Ordinario,
    };
    if !errores.is_empty() {
        return Err(errores);
    }
    match (
        codigo,
        nombre,
        tipo_documento,
        documento,
        tipo_contrato,
        inicio,
        cargo,
        salario,
        eps,
    ) {
        (
            Some(Valor::Codigo(codigo)),
            Some(Valor::Texto(nombre)),
            Some(Valor::TipoDocumento(tipo_documento)),
            Some(Valor::Documento(documento)),
            Some(tipo_contrato),
            Some(Valor::Fecha(fecha_inicio)),
            Some(Valor::Texto(cargo)),
            Some(Valor::Monto(salario)),
            Some(Valor::Texto(eps)),
        ) => Ok(EmpleadoImportado {
            fila: n,
            codigo,
            nombre,
            tipo_documento,
            documento,
            correo: correo.and_then(Valor::correo),
            telefono: telefono.and_then(Valor::telefono),
            tipo_contrato,
            fecha_inicio,
            fecha_fin: fin.and_then(Valor::fecha),
            descripcion_obra: obra.and_then(Valor::texto),
            cargo,
            salario,
            tipo_salario,
            etapa_aprendizaje: etapa.and_then(Valor::etapa),
            jornada: jornada
                .and_then(Valor::jornada)
                .unwrap_or(Jornada::Completa),
            horas_semanales: horas.and_then(Valor::decimal),
            periodicidad: periodicidad
                .and_then(Valor::periodicidad)
                .unwrap_or(PeriodicidadPago::Mensual),
            eps,
            afp: afp.and_then(Valor::texto),
            cesantias: cesantias.and_then(Valor::texto),
            ccf: ccf.and_then(Valor::texto),
            arl: arl.and_then(Valor::texto),
            clase_riesgo: clase.and_then(Valor::clase),
            pensionado: pensionado.and_then(Valor::booleano).unwrap_or(false),
            medio_pago,
            cuenta,
        }),
        _ => Err(vec![ErrorFila::new(n, None, "fila incompleta")]),
    }
}

/// Valor leído de una celda (permite una sola función de lectura para todas las columnas).
enum Valor {
    Codigo(CodigoEmpleado),
    Texto(String),
    TipoDocumento(TipoDocumento),
    Documento(Documento),
    Correo(DireccionCorreo),
    Telefono(Telefono),
    TipoContrato(TipoContratoLaboral),
    Fecha(NaiveDate),
    Monto(Monto),
    TipoSalario(TipoSalario),
    Etapa(EtapaAprendizaje),
    Jornada(Jornada),
    Decimal(Decimal),
    Periodicidad(PeriodicidadPago),
    Clase(ClaseRiesgo),
    Booleano(bool),
    Medio(MedioPago),
    TipoCuenta(TipoCuentaBancaria),
}

impl Valor {
    fn texto(self) -> Option<String> {
        match self {
            Self::Texto(t) => Some(t),
            _ => None,
        }
    }

    fn correo(self) -> Option<DireccionCorreo> {
        match self {
            Self::Correo(c) => Some(c),
            _ => None,
        }
    }

    fn telefono(self) -> Option<Telefono> {
        match self {
            Self::Telefono(t) => Some(t),
            _ => None,
        }
    }

    fn fecha(self) -> Option<NaiveDate> {
        match self {
            Self::Fecha(f) => Some(f),
            _ => None,
        }
    }

    fn etapa(self) -> Option<EtapaAprendizaje> {
        match self {
            Self::Etapa(e) => Some(e),
            _ => None,
        }
    }

    fn jornada(self) -> Option<Jornada> {
        match self {
            Self::Jornada(j) => Some(j),
            _ => None,
        }
    }

    fn decimal(self) -> Option<Decimal> {
        match self {
            Self::Decimal(d) => Some(d),
            _ => None,
        }
    }

    fn periodicidad(self) -> Option<PeriodicidadPago> {
        match self {
            Self::Periodicidad(p) => Some(p),
            _ => None,
        }
    }

    fn clase(self) -> Option<ClaseRiesgo> {
        match self {
            Self::Clase(c) => Some(c),
            _ => None,
        }
    }

    fn booleano(self) -> Option<bool> {
        match self {
            Self::Booleano(b) => Some(b),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planilla(filas: &[&[&str]]) -> Vec<Vec<String>> {
        filas
            .iter()
            .map(|f| f.iter().map(|c| (*c).to_owned()).collect())
            .collect()
    }

    const ENCABEZADO: &[&str] = &[
        "Código",
        "Nombre",
        "Tipo documento",
        "Documento",
        "Tipo contrato",
        "Fecha inicio",
        "Fecha fin",
        "Cargo",
        "Salario",
        "EPS",
        "AFP",
        "Cesantías",
        "CCF",
        "ARL",
        "Clase riesgo",
        "Banco",
        "Tipo cuenta",
        "Número cuenta",
    ];

    fn fila<'a>(codigo: &'a str, documento: &'a str, salario: &'a str) -> Vec<&'a str> {
        vec![
            codigo,
            "Ana Pérez",
            "cc",
            documento,
            "Término fijo",
            "01/02/2026",
            "2027-01-31",
            "Todero",
            salario,
            "eps037",
            "230301",
            "230301",
            "CCF22",
            "ARL14",
            "1",
            "Banco X",
            "Ahorros",
            "0012345678",
        ]
    }

    #[test]
    fn lee_un_empleado_con_valores_por_defecto() {
        let p = leer_planilla_empleados(&planilla(&[
            ENCABEZADO,
            &fila("E-1", "52123456", "1750000,50"),
        ]));
        assert!(p.errores.is_empty(), "{:?}", p.errores);
        let e = &p.empleados[0];
        assert_eq!(e.tipo_contrato, TipoContratoLaboral::TerminoFijo);
        assert_eq!(
            e.fecha_inicio,
            NaiveDate::from_ymd_opt(2026, 2, 1).expect("fecha")
        );
        assert_eq!(e.salario.valor().to_string(), "1750000.50");
        assert_eq!(e.eps, "EPS037");
        assert_eq!(e.clase_riesgo, Some(ClaseRiesgo::I));
        assert_eq!(e.medio_pago, MedioPago::Transferencia);
        assert_eq!(e.jornada, Jornada::Completa);
        assert_eq!(e.periodicidad, PeriodicidadPago::Mensual);
        assert_eq!(e.tipo_salario, TipoSalario::Ordinario);
        assert!(!e.pensionado);
    }

    #[test]
    fn faltan_columnas_obligatorias() {
        let p = leer_planilla_empleados(&planilla(&[&["codigo", "nombre"]]));
        let columnas: Vec<_> = p.errores.iter().filter_map(|e| e.columna.clone()).collect();
        assert!(columnas.contains(&"documento".to_owned()), "{columnas:?}");
        assert!(columnas.contains(&"eps".to_owned()), "{columnas:?}");
        assert!(!columnas.contains(&"periodicidad".to_owned()));
    }

    #[test]
    fn los_errores_no_repiten_el_valor_de_la_celda() {
        let mut mala = fila("E 1", "x", "2.000.000");
        mala[5] = "31-31-2026";
        let p = leer_planilla_empleados(&planilla(&[ENCABEZADO, &mala]));
        assert!(p.empleados.is_empty());
        let columnas: Vec<_> = p.errores.iter().filter_map(|e| e.columna.clone()).collect();
        assert_eq!(
            columnas,
            vec!["codigo", "documento", "fecha_inicio", "salario"]
        );
        assert!(p.errores.iter().all(|e| !e.mensaje.contains("2.000.000")));
    }

    #[test]
    fn codigo_o_documento_repetido_es_un_error() {
        let p = leer_planilla_empleados(&planilla(&[
            ENCABEZADO,
            &fila("E-1", "52123456", "1750000"),
            &fila("E-1", "52999999", "1750000"),
            &fila("E-2", "52123456", "1750000"),
        ]));
        assert_eq!(p.empleados.len(), 1);
        let filas: Vec<_> = p.errores.iter().map(|e| e.fila).collect();
        assert_eq!(filas, vec![3, 4]);
    }

    #[test]
    fn la_cuenta_incompleta_es_un_error_y_sin_cuenta_se_paga_en_efectivo() {
        let mut incompleta = fila("E-1", "52123456", "1750000");
        incompleta[16] = "";
        let p = leer_planilla_empleados(&planilla(&[ENCABEZADO, &incompleta]));
        assert_eq!(p.errores[0].columna.as_deref(), Some("numero_cuenta"));

        let mut sin_cuenta = fila("E-1", "52123456", "1750000");
        sin_cuenta[15] = "";
        sin_cuenta[16] = "";
        sin_cuenta[17] = "";
        let p = leer_planilla_empleados(&planilla(&[ENCABEZADO, &sin_cuenta]));
        assert_eq!(p.empleados[0].medio_pago, MedioPago::Efectivo);
        assert!(p.empleados[0].cuenta.is_none());
    }

    #[test]
    fn el_aprendiz_recibe_apoyo_por_defecto() {
        let mut aprendiz = fila("A-1", "1010101010", "1125000");
        aprendiz[4] = "aprendizaje";
        let p = leer_planilla_empleados(&planilla(&[ENCABEZADO, &aprendiz]));
        assert_eq!(p.empleados[0].tipo_salario, TipoSalario::ApoyoSostenimiento);
        assert_eq!(encabezados_empleados().len(), 28);
    }
}
