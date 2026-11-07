//! Eventos de auditoría: quién hizo qué, sobre qué entidad, con qué resultado.
//!
//! Se registran con el puerto [`crate::puertos::RegistroAuditoria`] **dentro de la misma
//! transacción** que el cambio que describen (ver [`crate::puertos::UnidadDeTrabajo`]): si el
//! cambio se revierte, su auditoría también.
//!
//! Protección de datos (Ley 1581) y secreto del voto (AGENTS.md §9.4): el detalle de un evento
//! nunca lleva datos personales ni la opción de un voto. [`DetalleAuditoria`] rechaza las claves
//! que los identifican.

use std::collections::BTreeMap;

use serde_json::Value;
use uuid::Uuid;

use crate::{AppError, AppResult};

/// Acción auditada con el formato `modulo.recurso.verbo` en minúsculas
/// (p. ej. `bienes_privados.coeficiente.modificar`).
///
/// Se declara como constante para que un formato inválido sea un error de compilación:
///
/// ```
/// use application::auditoria::AccionAuditoria;
///
/// const ACTIVAR_CUADRO: AccionAuditoria =
///     AccionAuditoria::new("bienes_privados.cuadro_coeficientes.activar");
/// assert_eq!(ACTIVAR_CUADRO.as_str(), "bienes_privados.cuadro_coeficientes.activar");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccionAuditoria(&'static str);

impl AccionAuditoria {
    /// Crea la acción validando el formato.
    ///
    /// # Panics
    ///
    /// Si el texto no tiene exactamente tres segmentos de `[a-z_]` separados por puntos. Usada
    /// en una constante, el pánico ocurre al compilar.
    pub const fn new(accion: &'static str) -> Self {
        assert!(
            formato_valido(accion),
            "acción de auditoría inválida: use modulo.recurso.verbo"
        );
        Self(accion)
    }

    /// Texto persistido en `auditoria.accion`.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Tres segmentos no vacíos de `[a-z_]` separados por `.`.
const fn formato_valido(texto: &str) -> bool {
    let bytes = texto.as_bytes();
    let mut puntos = 0;
    let mut largo_segmento = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'.' => {
                if largo_segmento == 0 {
                    return false;
                }
                puntos += 1;
                largo_segmento = 0;
            }
            b'a'..=b'z' | b'_' => largo_segmento += 1,
            _ => return false,
        }
        i += 1;
    }
    puntos == 2 && largo_segmento > 0
}

/// Resultado de la acción auditada. El texto es contrato con el `CHECK` de `auditoria.resultado`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResultadoAuditoria {
    /// La acción se completó.
    Ok,
    /// Se rechazó por una regla de negocio o de permisos.
    Rechazado,
    /// Falló por un error técnico.
    Error,
}

impl ResultadoAuditoria {
    /// Texto persistido en `auditoria.resultado`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Rechazado => "rechazado",
            Self::Error => "error",
        }
    }
}

/// Datos adicionales del evento (`auditoria.detalle`, JSONB): identificadores, valores antes y
/// después de un cambio, motivo de un rechazo.
///
/// Rechaza claves que identifican datos personales o el contenido de un voto: si **algún
/// segmento** de la clave (separada por `_`) está en [`Self::CLAVES_PROHIBIDAS`], por ejemplo
/// `telefono_contacto` u `opcion_id`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DetalleAuditoria(BTreeMap<String, Value>);

impl DetalleAuditoria {
    /// Segmentos de clave que nunca se auditan.
    pub const CLAVES_PROHIBIDAS: &'static [&'static str] = &[
        "telefono",
        "celular",
        "correo",
        "email",
        "documento",
        "cedula",
        "direccion",
        "contrasena",
        "password",
        "token",
        "secreto",
        "biometria",
        "opcion",
        "voto",
    ];

    /// Detalle vacío.
    pub fn vacio() -> Self {
        Self::default()
    }

    /// Agrega un dato.
    ///
    /// # Errors
    ///
    /// [`AppError::Validacion`] si la clave identifica un dato personal o el contenido de un voto.
    pub fn con(mut self, clave: &str, valor: impl Into<Value>) -> AppResult<Self> {
        let prohibida = clave
            .to_ascii_lowercase()
            .split('_')
            .any(|segmento| Self::CLAVES_PROHIBIDAS.contains(&segmento));
        if prohibida {
            return Err(AppError::Validacion(format!(
                "la clave `{clave}` no se puede auditar (dato personal o contenido de un voto)"
            )));
        }
        self.0.insert(clave.to_owned(), valor.into());
        Ok(self)
    }

    /// Representación JSON para persistir.
    pub fn a_json(&self) -> Value {
        Value::Object(self.0.clone().into_iter().collect())
    }
}

/// Evento a registrar. El tenant, el actor y el `request_id` los aporta el
/// [`crate::Contexto`] de la transacción, no el evento.
#[derive(Debug, Clone, PartialEq)]
pub struct EventoAuditoria {
    pub accion: AccionAuditoria,
    /// Tipo de entidad afectada, en singular y `snake_case` (p. ej. `bien_privado`).
    pub entidad: &'static str,
    pub entidad_id: Option<Uuid>,
    pub resultado: ResultadoAuditoria,
    pub detalle: DetalleAuditoria,
}

impl EventoAuditoria {
    /// Evento exitoso sobre una entidad, sin detalle.
    pub fn ok(accion: AccionAuditoria, entidad: &'static str, entidad_id: Uuid) -> Self {
        Self {
            accion,
            entidad,
            entidad_id: Some(entidad_id),
            resultado: ResultadoAuditoria::Ok,
            detalle: DetalleAuditoria::vacio(),
        }
    }

    /// Reemplaza el detalle.
    pub fn con_detalle(mut self, detalle: DetalleAuditoria) -> Self {
        self.detalle = detalle;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formato_de_accion() {
        assert!(formato_valido("bienes_privados.coeficiente.modificar"));
        for invalida in [
            "",
            "dos.segmentos",
            "cuatro.seg.men.tos",
            "Mayus.recurso.verbo",
            "modulo..verbo",
            "modulo.recurso.",
            ".recurso.verbo",
            "con-guion.recurso.verbo",
            "con espacio.recurso.verbo",
        ] {
            assert!(!formato_valido(invalida), "{invalida:?}");
        }
    }

    #[test]
    #[should_panic(expected = "acción de auditoría inválida")]
    fn accion_invalida_en_tiempo_de_ejecucion_entra_en_panico() {
        let texto: &'static str = Box::leak(String::from("sin_formato").into_boxed_str());
        let _ = AccionAuditoria::new(texto);
    }

    #[test]
    fn detalle_acepta_identificadores_y_valores_de_negocio() {
        let detalle = DetalleAuditoria::vacio()
            .con("coeficiente_anterior", "0.00523000")
            .and_then(|d| d.con("cuadro_id", Uuid::nil().to_string()))
            .expect("claves permitidas");

        assert_eq!(detalle.a_json()["coeficiente_anterior"], "0.00523000");
    }

    #[test]
    fn detalle_rechaza_datos_personales_y_contenido_del_voto() {
        for clave in [
            "telefono",
            "telefono_contacto",
            "Correo",
            "numero_documento",
            "opcion_id",
            "voto",
            "token_acceso",
        ] {
            let res = DetalleAuditoria::vacio().con(clave, "x");
            assert!(matches!(res, Err(AppError::Validacion(_))), "{clave}");
        }
    }

    #[test]
    fn textos_de_resultado_coinciden_con_el_check_de_la_bd() {
        let textos: Vec<_> = [
            ResultadoAuditoria::Ok,
            ResultadoAuditoria::Rechazado,
            ResultadoAuditoria::Error,
        ]
        .map(ResultadoAuditoria::as_str)
        .into();
        assert_eq!(textos, ["ok", "rechazado", "error"]);
    }
}
