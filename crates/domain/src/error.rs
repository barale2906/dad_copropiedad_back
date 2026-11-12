//! Errores de reglas de negocio ([`DominioError`]).

use crate::{EstadoPregunta, EstadoReunion, OpcionId, Permiso};

/// Errores de reglas de negocio. El mapeo a HTTP vive en `apps/api/src/error.rs`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DominioError {
    #[error("transición de {entidad} no permitida: {desde} -> {hacia}")]
    TransicionInvalida {
        entidad: &'static str,
        desde: &'static str,
        hacia: &'static str,
    },

    #[error("la pregunta no está abierta para recibir votos (estado actual: {0})")]
    PreguntaNoAbierta(EstadoPregunta),

    #[error("la reunión no está instalada (estado actual: {0})")]
    ReunionNoInstalada(EstadoReunion),

    #[error("la opción {0} no pertenece a la pregunta")]
    OpcionInvalida(OpcionId),

    #[error("coeficiente inválido: {0}")]
    CoeficienteInvalido(String),

    #[error("clave de idempotencia inválida: {0}")]
    ClaveIdempotenciaInvalida(&'static str),

    /// No incluye el valor recibido: un correo es dato personal y no debe llegar a los logs.
    #[error("dirección de correo inválida")]
    CorreoInvalido,

    #[error("el archivo está vacío")]
    ArchivoVacio,

    #[error("tipo de archivo no admitido (se aceptan PDF, JPEG, PNG, WebP y HEIC)")]
    TipoArchivoNoAdmitido,

    #[error("el archivo supera el tamaño máximo de {maximo_bytes} bytes")]
    ArchivoDemasiadoGrande { maximo_bytes: u64 },

    /// No incluye el valor: un nombre de archivo puede contener datos personales.
    #[error("nombre de archivo inválido")]
    NombreArchivoInvalido,

    #[error("clave de archivo inválida")]
    ClaveArchivoInvalida,

    #[error(
        "nombre de usuario inválido: de 3 a 30 caracteres a-z, 0-9, punto, guion o guion bajo, empezando por letra"
    )]
    NombreUsuarioInvalido,

    #[error(
        "la contraseña debe tener entre 10 y 128 caracteres y no puede ser un solo carácter repetido"
    )]
    ContrasenaDebil,

    /// No incluye el valor: el NIT de una persona natural es su cédula.
    #[error("NIT inválido o con dígito de verificación incorrecto")]
    NitInvalido,

    #[error("el permiso `{0}` no corresponde al ámbito del rol")]
    PermisoDeOtroAmbito(Permiso),

    /// La suma de un cuadro de coeficientes no es exactamente 1 (sin tolerancia).
    #[error("los coeficientes suman {suma} y deben sumar exactamente 1 (diferencia: {diferencia})")]
    CoeficientesNoSumanUno {
        suma: rust_decimal::Decimal,
        /// `1 - suma`: positiva si falta, negativa si sobra.
        diferencia: rust_decimal::Decimal,
    },

    #[error("el cuadro de coeficientes no incluye {faltantes} bien(es) privado(s) activo(s)")]
    CuadroIncompleto { faltantes: usize },

    /// Conjunto de titulares de un bien privado que rompe una regla (suma, representante…).
    #[error("titulares inválidos: {0}")]
    TitularesInvalidos(&'static str),

    #[error("plantilla de nomenclatura inválida: {0}")]
    PlantillaInvalida(&'static str),

    /// Dato de entrada con formato inválido. No incluye el valor (puede ser un dato personal).
    #[error("`{campo}` inválido: {regla}")]
    DatoInvalido {
        campo: &'static str,
        regla: &'static str,
    },

    /// La reserva incumple la política del recurso o los límites del bien privado.
    #[error("reserva no permitida: {0}")]
    ReservaNoPermitida(&'static str),

    /// La papeleta no corresponde a la pregunta (número de opciones, bienes repetidos…).
    #[error("voto inválido: {0}")]
    VotoInvalido(&'static str),

    #[error("valor desconocido `{valor}` para {tipo}")]
    ValorDesconocido { tipo: &'static str, valor: String },
}
