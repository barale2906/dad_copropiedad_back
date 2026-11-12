//! Documentos PDF de la asamblea: el acta y la hoja de rótulos de código de barras.
//! Implementación: `crates/infra-pdf` (Typst).
//!
//! Los datos llegan **ya calculados** (quórum, escrutinios, detalle nominal): el generador solo
//! da formato y no decide nada. Se serializan a JSON para la plantilla.

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::Escrutinio;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::AppResult;

/// Una foto del quórum en el acta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuorumActa {
    pub momento: String,
    pub hora: String,
    pub coeficiente: Decimal,
    pub porcentaje_coeficiente: Decimal,
    pub unidades: u32,
    pub unidades_total: u32,
    pub asistentes: u32,
}

/// Asistente en el acta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AsistenteActa {
    pub nombre: String,
    pub bienes: Vec<String>,
    pub coeficiente: Decimal,
    /// Bienes que representa por poder.
    pub poderes: Vec<String>,
    pub entrada: String,
    /// Salidas con hora y motivo.
    pub salidas: Vec<String>,
}

/// Una fila del detalle nominal (bien → opción).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VotoActa {
    pub bien: String,
    pub coeficiente: Decimal,
    pub opcion: String,
    pub canal: String,
    pub hora: String,
    pub apoderado: bool,
}

/// Pregunta en el acta.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreguntaActa {
    pub orden: u16,
    pub enunciado: String,
    pub tipo: String,
    pub tipo_voto: String,
    pub abierta: Option<String>,
    pub cerrada: Option<String>,
    pub escrutinio: Option<Escrutinio>,
    pub decision: String,
    /// Solo en votaciones nominales (en reservadas, solo totales).
    pub detalle: Vec<VotoActa>,
    /// Elegidos por plancha (elecciones).
    pub elegidos: Vec<String>,
}

/// Datos completos del acta.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DatosActa {
    pub copropiedad: String,
    pub nit: Option<String>,
    pub titulo: String,
    pub tipo: String,
    pub convocatoria: String,
    pub modalidad: String,
    pub lugar: Option<String>,
    pub fecha: String,
    pub instalada: Option<String>,
    pub cerrada: Option<String>,
    pub orden_del_dia: Vec<String>,
    pub quorum: Vec<QuorumActa>,
    pub asistentes: Vec<AsistenteActa>,
    pub morosos: Vec<String>,
    pub efecto_mora: String,
    pub preguntas: Vec<PreguntaActa>,
    /// Advertencias para el acta (decisiones por unidades, efecto de la mora…).
    pub advertencias: Vec<String>,
    pub generada: String,
}

/// Hoja de rótulos para imprimir (paquetes genéricos numerados).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HojaRotulos {
    pub titulo: String,
    pub codigos: Vec<String>,
}

/// Documento tabular (libros del censo a una fecha de corte). Las celdas llegan como texto ya
/// formateado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TablaPdf {
    pub titulo: String,
    pub copropiedad: String,
    /// Línea bajo el título (p. ej. la fecha de corte).
    pub subtitulo: String,
    pub columnas: Vec<String>,
    pub filas: Vec<Vec<String>>,
    pub generada: String,
}

/// Generación de documentos PDF (actas, rótulos, libros del censo).
pub trait GeneradorActas: Send + Sync {
    /// PDF del acta.
    fn generar(&self, datos: &DatosActa) -> impl Future<Output = AppResult<Vec<u8>>> + Send;

    /// PDF con los rótulos en Code 128, listos para recortar.
    fn rotulos(&self, hoja: &HojaRotulos) -> impl Future<Output = AppResult<Vec<u8>>> + Send;

    /// PDF horizontal con una tabla (libros del censo).
    fn tabla(&self, tabla: &TablaPdf) -> impl Future<Output = AppResult<Vec<u8>>> + Send;
}

/// Hora local legible (`2027-03-15 19:04`) para el acta.
pub fn hora_local(t: DateTime<Utc>, tz: domain::reserva::Tz) -> String {
    t.with_timezone(&tz).format("%Y-%m-%d %H:%M").to_string()
}
