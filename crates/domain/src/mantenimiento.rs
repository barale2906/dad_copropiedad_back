//! Inventario de activos y mantenimiento (plan 07c): activos con hoja de vida, movimientos,
//! planes preventivos con recurrencia, órdenes de trabajo y obligaciones periódicas.
//!
//! Reglas de este módulo:
//! - Las órdenes de trabajo cambian de estado solo por [`EstadoOrden::transicionar`].
//! - Un plan preventivo genera a lo sumo una orden por fecha programada ([`Recurrencia`]):
//!   la generación es idempotente por (plan, fecha).
//! - Un activo dado de baja no vuelve a operar ni recibe órdenes.
//! - Las obligaciones periódicas sugeridas ([`OBLIGACIONES_SUGERIDAS`]) son una ayuda editable:
//!   la periodicidad exacta depende de la norma local, que la copropiedad confirma.

use chrono::{Duration, Months, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::DominioError;

/// Clase de activo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaseActivo {
    /// Bombas, ascensores, planta eléctrica, CCTV, cerca eléctrica…
    Equipo,
    Herramienta,
    Mobiliario,
    /// Extintores, botiquines, señalización.
    Dotacion,
}

enum_texto!(ClaseActivo {
    Equipo => "equipo",
    Herramienta => "herramienta",
    Mobiliario => "mobiliario",
    Dotacion => "dotacion",
});

/// Estado operativo de un activo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoActivo {
    Operativo,
    ConFalla,
    EnReparacion,
    FueraDeServicio,
    /// Final: solo se llega con un movimiento de baja.
    DadoDeBaja,
}

enum_texto!(EstadoActivo {
    Operativo => "operativo",
    ConFalla => "con_falla",
    EnReparacion => "en_reparacion",
    FueraDeServicio => "fuera_de_servicio",
    DadoDeBaja => "dado_de_baja",
});

impl EstadoActivo {
    /// Cambio manual de estado (la baja va por [`TipoMovimiento::Baja`]).
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] desde o hacia `dado_de_baja`.
    pub fn cambiar(self, hacia: Self) -> Result<Self, DominioError> {
        if self == Self::DadoDeBaja || hacia == Self::DadoDeBaja {
            return Err(DominioError::TransicionInvalida {
                entidad: "activo",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            });
        }
        Ok(hacia)
    }
}

/// Movimiento de inventario (historial append-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoMovimiento {
    Entrada,
    Salida,
    Traslado,
    /// Préstamo (las herramientas prestables se reservan con el motor de reservas).
    Prestamo,
    Devolucion,
    /// Exige soporte (acta o documento) y deja el activo `dado_de_baja`.
    Baja,
}

enum_texto!(TipoMovimiento {
    Entrada => "entrada",
    Salida => "salida",
    Traslado => "traslado",
    Prestamo => "prestamo",
    Devolucion => "devolucion",
    Baja => "baja",
});

/// Clase de archivo de la hoja de vida de un activo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoArchivoActivo {
    Manual,
    Foto,
    Factura,
    Garantia,
    Otro,
}

enum_texto!(TipoArchivoActivo {
    Manual => "manual",
    Foto => "foto",
    Factura => "factura",
    Garantia => "garantia",
    Otro => "otro",
});

/// Momento de una evidencia de una orden de trabajo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MomentoEvidencia {
    Antes,
    Despues,
    /// Factura, informe técnico, certificado.
    Soporte,
}

enum_texto!(MomentoEvidencia {
    Antes => "antes",
    Despues => "despues",
    Soporte => "soporte",
});

/// Número legible de una orden de trabajo (`OT-2026-00007`): consecutivo por copropiedad y año.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NumeroOrden {
    pub anio: i32,
    pub consecutivo: u32,
}

impl std::fmt::Display for NumeroOrden {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OT-{}-{:05}", self.anio, self.consecutivo)
    }
}

/// Código interno o etiqueta (QR) de un activo: `A-Z`, `0-9` y guiones, hasta 30 caracteres.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CodigoActivo(String);

impl CodigoActivo {
    /// Normaliza a mayúsculas.
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si queda vacío, es muy largo o trae otros caracteres.
    pub fn new(valor: &str) -> Result<Self, DominioError> {
        let v = valor.trim().to_uppercase();
        let valido = !v.is_empty()
            && v.len() <= 30
            && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if valido {
            Ok(Self(v))
        } else {
            Err(DominioError::DatoInvalido {
                campo: "codigo",
                regla: "de 1 a 30 letras, dígitos o guiones",
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CodigoActivo {
    type Error = DominioError;

    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::new(&v)
    }
}

impl From<CodigoActivo> for String {
    fn from(c: CodigoActivo) -> Self {
        c.0
    }
}

/// Unidad de la recurrencia de un plan preventivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frecuencia {
    Dias,
    Semanas,
    /// Mensual, trimestral (3), semestral (6), anual en fecha fija (12)…
    Meses,
}

enum_texto!(Frecuencia {
    Dias => "dias",
    Semanas => "semanas",
    Meses => "meses",
});

/// Recurrencia de un plan: "cada `intervalo` `frecuencia` desde `inicio`".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Recurrencia {
    pub frecuencia: Frecuencia,
    pub intervalo: u32,
    pub inicio: NaiveDate,
}

impl Recurrencia {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si el intervalo no está entre 1 y 366.
    pub fn new(
        frecuencia: Frecuencia,
        intervalo: u32,
        inicio: NaiveDate,
    ) -> Result<Self, DominioError> {
        if !(1..=366).contains(&intervalo) {
            return Err(DominioError::DatoInvalido {
                campo: "intervalo",
                regla: "debe estar entre 1 y 366",
            });
        }
        Ok(Self {
            frecuencia,
            intervalo,
            inicio,
        })
    }

    /// Fecha de la `k`-ésima ocurrencia (la 0 es `inicio`). Los meses se cuentan siempre desde
    /// `inicio` para no arrastrar el recorte de fin de mes (31 de enero → 28 de febrero → 31
    /// de marzo).
    pub fn ocurrencia(&self, k: u32) -> Option<NaiveDate> {
        let pasos = k.checked_mul(self.intervalo)?;
        match self.frecuencia {
            Frecuencia::Dias => self
                .inicio
                .checked_add_signed(Duration::days(i64::from(pasos))),
            Frecuencia::Semanas => self
                .inicio
                .checked_add_signed(Duration::weeks(i64::from(pasos))),
            Frecuencia::Meses => self.inicio.checked_add_months(Months::new(pasos)),
        }
    }

    /// Ocurrencias en `[desde, hasta]` (a lo sumo `maximo`).
    pub fn ocurrencias_entre(
        &self,
        desde: NaiveDate,
        hasta: NaiveDate,
        maximo: usize,
    ) -> Vec<NaiveDate> {
        let mut fechas = Vec::new();
        let mut k = 0;
        while let Some(f) = self.ocurrencia(k) {
            if f > hasta || fechas.len() >= maximo {
                break;
            }
            if f >= desde {
                fechas.push(f);
            }
            k += 1;
        }
        fechas
    }
}

/// De dónde nace una orden de trabajo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrigenOrden {
    PlanPreventivo,
    Ticket,
    Inspeccion,
}

enum_texto!(OrigenOrden {
    PlanPreventivo => "plan_preventivo",
    Ticket => "ticket",
    Inspeccion => "inspeccion",
});

/// Tipo de mantenimiento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoOrden {
    Preventivo,
    Correctivo,
    Urgente,
}

enum_texto!(TipoOrden {
    Preventivo => "preventivo",
    Correctivo => "correctivo",
    Urgente => "urgente",
});

/// Estado de una orden de trabajo.
///
/// `programada → en_ejecucion → ejecutada → verificada`; `programada | en_ejecucion →
/// cancelada`; la verificación puede devolverla (`ejecutada → en_ejecucion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoOrden {
    Programada,
    EnEjecucion,
    Ejecutada,
    Verificada,
    Cancelada,
}

enum_texto!(EstadoOrden {
    Programada => "programada",
    EnEjecucion => "en_ejecucion",
    Ejecutada => "ejecutada",
    Verificada => "verificada",
    Cancelada => "cancelada",
});

impl EstadoOrden {
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está en el diagrama.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoOrden::*;
        match (self, hacia) {
            (Programada, EnEjecucion | Cancelada)
            | (EnEjecucion, Ejecutada | Cancelada)
            | (Ejecutada, Verificada | EnEjecucion) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "orden de trabajo",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }

    /// Pendiente de ejecutar (cuenta para vencidas y cumplimiento).
    pub const fn pendiente(self) -> bool {
        matches!(self, Self::Programada | Self::EnEjecucion)
    }

    /// Cumplida para el indicador del plan preventivo.
    pub const fn cumplida(self) -> bool {
        matches!(self, Self::Ejecutada | Self::Verificada)
    }
}

/// Punto del checklist de una orden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PuntoChecklist {
    pub descripcion: String,
    pub cumplido: bool,
    pub observacion: Option<String>,
}

/// Obligación periódica sugerida (certificaciones, renovaciones, inspecciones).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObligacionSugerida {
    pub codigo: &'static str,
    pub nombre: &'static str,
    pub descripcion: &'static str,
    pub periodicidad_meses: u32,
}

/// Catálogo sugerido (decisión del 2026-11-08). No cita normas locales: la copropiedad confirma
/// la periodicidad que le exige su municipio y la ajusta al activarla.
pub const OBLIGACIONES_SUGERIDAS: &[ObligacionSugerida] = &[
    ObligacionSugerida {
        codigo: "inspeccion_ascensores",
        nombre: "Inspección y certificación de ascensores",
        descripcion: "Revisión por un organismo de inspección acreditado y certificado vigente.",
        periodicidad_meses: 12,
    },
    ObligacionSugerida {
        codigo: "lavado_tanques",
        nombre: "Lavado y desinfección de tanques de agua potable",
        descripcion: "Con empresa autorizada; conservar el certificado y el análisis del agua.",
        periodicidad_meses: 6,
    },
    ObligacionSugerida {
        codigo: "recarga_extintores",
        nombre: "Recarga y revisión de extintores",
        descripcion: "Recarga, prueba y etiqueta de vencimiento de cada extintor.",
        periodicidad_meses: 12,
    },
    ObligacionSugerida {
        codigo: "poliza_bienes_comunes",
        nombre: "Póliza de seguro de los bienes comunes",
        descripcion: "Seguro contra incendio y terremoto de los bienes comunes (Ley 675, art. 15).",
        periodicidad_meses: 12,
    },
    ObligacionSugerida {
        codigo: "plan_emergencias",
        nombre: "Actualización del plan de emergencias y contingencias",
        descripcion: "Revisar rutas de evacuación, brigada y puntos de encuentro.",
        periodicidad_meses: 12,
    },
    ObligacionSugerida {
        codigo: "simulacro_evacuacion",
        nombre: "Simulacro de evacuación",
        descripcion: "Con la brigada de emergencias y registro de asistencia.",
        periodicidad_meses: 12,
    },
    ObligacionSugerida {
        codigo: "control_plagas",
        nombre: "Control de plagas (fumigación y desratización)",
        descripcion: "Con empresa autorizada; conservar el certificado.",
        periodicidad_meses: 6,
    },
    ObligacionSugerida {
        codigo: "mantenimiento_planta_electrica",
        nombre: "Mantenimiento de la planta eléctrica de emergencia",
        descripcion: "Revisión del proveedor y prueba con carga.",
        periodicidad_meses: 6,
    },
];

/// Próxima fecha de una obligación a partir de su última realización.
pub fn proxima_obligacion(ultima: NaiveDate, periodicidad_meses: u32) -> Option<NaiveDate> {
    ultima.checked_add_months(Months::new(periodicidad_meses))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fecha(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("fecha")
    }

    #[test]
    fn recurrencia_mensual_no_arrastra_fin_de_mes() {
        let r = Recurrencia::new(Frecuencia::Meses, 1, fecha(2026, 1, 31)).expect("válida");
        assert_eq!(
            r.ocurrencias_entre(fecha(2026, 1, 1), fecha(2026, 4, 30), 10),
            vec![
                fecha(2026, 1, 31),
                fecha(2026, 2, 28),
                fecha(2026, 3, 31),
                fecha(2026, 4, 30)
            ]
        );
    }

    #[test]
    fn recurrencia_semanal_y_por_dias_dentro_del_rango() {
        let s = Recurrencia::new(Frecuencia::Semanas, 2, fecha(2026, 11, 2)).expect("válida");
        assert_eq!(
            s.ocurrencias_entre(fecha(2026, 11, 10), fecha(2026, 12, 1), 10),
            vec![fecha(2026, 11, 16), fecha(2026, 11, 30)]
        );
        let d = Recurrencia::new(Frecuencia::Dias, 10, fecha(2026, 11, 1)).expect("válida");
        assert_eq!(
            d.ocurrencias_entre(fecha(2026, 11, 1), fecha(2026, 11, 30), 2),
            vec![fecha(2026, 11, 1), fecha(2026, 11, 11)]
        );
        assert!(Recurrencia::new(Frecuencia::Dias, 0, fecha(2026, 1, 1)).is_err());
    }

    #[test]
    fn orden_sigue_su_diagrama() {
        use EstadoOrden::*;
        assert!(Programada.transicionar(EnEjecucion).is_ok());
        assert!(Ejecutada.transicionar(EnEjecucion).is_ok());
        assert!(Ejecutada.transicionar(Verificada).is_ok());
        for (d, h) in [
            (Programada, Ejecutada),
            (Verificada, Cancelada),
            (Cancelada, Programada),
            (Ejecutada, Cancelada),
        ] {
            assert!(d.transicionar(h).is_err(), "{d} -> {h}");
        }
        assert!(Verificada.cumplida() && !Programada.cumplida() && Programada.pendiente());
    }

    #[test]
    fn baja_solo_por_movimiento() {
        use EstadoActivo::*;
        assert_eq!(Operativo.cambiar(EnReparacion), Ok(EnReparacion));
        assert!(Operativo.cambiar(DadoDeBaja).is_err());
        assert!(DadoDeBaja.cambiar(Operativo).is_err());
    }

    #[test]
    fn numero_de_orden_legible() {
        let n = NumeroOrden {
            anio: 2026,
            consecutivo: 7,
        };
        assert_eq!(n.to_string(), "OT-2026-00007");
    }

    #[test]
    fn codigo_de_activo_normalizado() {
        assert_eq!(
            CodigoActivo::new(" bomba-01 ").map(|c| c.as_str().to_owned()),
            Ok("BOMBA-01".into())
        );
        assert!(CodigoActivo::new("").is_err());
        assert!(CodigoActivo::new("con espacio").is_err());
    }

    #[test]
    fn catalogo_sugerido_unico_y_con_periodicidad() {
        let mut codigos: Vec<_> = OBLIGACIONES_SUGERIDAS.iter().map(|o| o.codigo).collect();
        codigos.sort_unstable();
        codigos.dedup();
        assert_eq!(codigos.len(), OBLIGACIONES_SUGERIDAS.len());
        assert!(
            OBLIGACIONES_SUGERIDAS
                .iter()
                .all(|o| o.periodicidad_meses > 0)
        );
        assert_eq!(
            proxima_obligacion(fecha(2026, 8, 31), 6),
            Some(fecha(2027, 2, 28))
        );
    }
}
