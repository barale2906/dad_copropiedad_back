//! Comunicados de la administración (plan 10, fase 4): la cartelera digital de la PH.
//!
//! - Ciclo: `borrador → programado → enviado`, o `anulado` antes de enviarse
//!   ([`EstadoComunicado::transicionar`]). Un comunicado programado para ahora sale de
//!   inmediato; uno futuro, cuando llega su hora.
//! - **Finalidad** (Ley 1581, decisión del 2026-11-10): los administrativos (asambleas, cortes de
//!   agua, cobros) son finalidad `administracion` y llegan a todos; los de bienestar o eventos son
//!   `comunicaciones` y solo a quien lo autorizó.
//! - **Audiencia** ([`Audiencia`]): toda la PH, propietarios, residentes adultos, agrupaciones
//!   (torres, con sus subagrupaciones), bienes concretos o roles (consejo, comité).
//! - **Canales** ([`canales_comunicado`]): app (push) y correo siempre; WhatsApp solo si el
//!   comunicado es importante y la persona no tiene la app (costo por destinatario).

use serde::{Deserialize, Serialize};

use crate::{AgrupacionId, BienPrivadoId, DominioError, FinalidadTratamiento};

/// Estado de un comunicado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoComunicado {
    Borrador,
    Programado,
    Enviado,
    Anulado,
}

enum_texto!(EstadoComunicado {
    Borrador => "borrador",
    Programado => "programado",
    Enviado => "enviado",
    Anulado => "anulado",
});

impl EstadoComunicado {
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está en el diagrama (uno enviado no se anula
    /// ni se edita).
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoComunicado::*;
        match (self, hacia) {
            (Borrador, Programado | Anulado) | (Programado, Enviado | Anulado | Borrador) => {
                Ok(hacia)
            }
            _ => Err(DominioError::TransicionInvalida {
                entidad: "comunicado",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }

    /// ¿Se puede editar el contenido?
    pub const fn editable(self) -> bool {
        matches!(self, Self::Borrador)
    }
}

/// A quién va un comunicado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", content = "ids", rename_all = "snake_case")]
pub enum Audiencia {
    /// Propietarios y residentes adultos de todos los bienes activos.
    Todos,
    Propietarios,
    /// Residentes mayores de edad.
    Residentes,
    /// Propietarios y residentes de los bienes de esas agrupaciones (incluidas sus hijas).
    Agrupaciones(Vec<AgrupacionId>),
    /// Propietarios y residentes de esos bienes.
    Bienes(Vec<BienPrivadoId>),
    /// Personas cuyas cuentas tienen alguno de esos roles en la PH (p. ej. `presidente_consejo`).
    Roles(Vec<String>),
}

/// Destinos máximos por lista de la audiencia.
pub const MAX_IDS_AUDIENCIA: usize = 500;

impl Audiencia {
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] si una lista está vacía o es demasiado larga, o si un código
    /// de rol no es válido.
    pub fn validar(self) -> Result<Self, DominioError> {
        let invalido = |regla| DominioError::DatoInvalido {
            campo: "audiencia",
            regla,
        };
        let n = match &self {
            Self::Agrupaciones(v) => Some(v.len()),
            Self::Bienes(v) => Some(v.len()),
            Self::Roles(v) => {
                if v.iter().any(|r| {
                    r.is_empty()
                        || r.len() > 60
                        || !r
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
                }) {
                    return Err(invalido("códigos de rol en minúsculas con guion bajo"));
                }
                Some(v.len())
            }
            _ => None,
        };
        match n {
            Some(0) => Err(invalido("indique al menos un destino")),
            Some(n) if n > MAX_IDS_AUDIENCIA => Err(invalido("demasiados destinos (máximo 500)")),
            _ => Ok(self),
        }
    }
}

/// Finalidad de un comunicado: solo `administracion` o `comunicaciones`.
///
/// # Errors
///
/// [`DominioError::DatoInvalido`] con otra finalidad (control de acceso o publicidad no se
/// envían como comunicados).
pub fn finalidad_comunicado(f: FinalidadTratamiento) -> Result<FinalidadTratamiento, DominioError> {
    match f {
        FinalidadTratamiento::Administracion | FinalidadTratamiento::Comunicaciones => Ok(f),
        _ => Err(DominioError::DatoInvalido {
            campo: "finalidad",
            regla: "un comunicado es de administración o de comunicaciones",
        }),
    }
}

/// Canales por los que sale un comunicado a una persona.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanalesComunicado {
    pub push: bool,
    pub correo: bool,
    pub whatsapp: bool,
}

impl CanalesComunicado {
    pub const fn alguno(&self) -> bool {
        self.push || self.correo || self.whatsapp
    }
}

/// App y correo siempre que existan; WhatsApp solo si es importante y no tiene la app (decisión
/// del 2026-11-10: el WhatsApp tiene costo por destinatario).
pub const fn canales_comunicado(
    importante: bool,
    tiene_push: bool,
    tiene_correo: bool,
    tiene_whatsapp: bool,
) -> CanalesComunicado {
    CanalesComunicado {
        push: tiene_push,
        correo: tiene_correo,
        whatsapp: importante && !tiene_push && tiene_whatsapp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ciclo_del_comunicado() {
        use EstadoComunicado::*;
        assert_eq!(Borrador.transicionar(Programado), Ok(Programado));
        assert_eq!(Programado.transicionar(Enviado), Ok(Enviado));
        assert_eq!(
            Programado.transicionar(Borrador),
            Ok(Borrador),
            "despublicar antes de enviarse"
        );
        assert!(
            Enviado.transicionar(Anulado).is_err(),
            "lo enviado no se anula"
        );
        assert!(Anulado.transicionar(Programado).is_err());
        assert!(Borrador.editable() && !Programado.editable());
    }

    #[test]
    fn audiencia_valida() {
        assert!(Audiencia::Todos.validar().is_ok());
        assert!(Audiencia::Bienes(vec![]).validar().is_err());
        assert!(
            Audiencia::Roles(vec!["Presidente".into()])
                .validar()
                .is_err()
        );
        assert!(
            Audiencia::Roles(vec!["presidente_consejo".into()])
                .validar()
                .is_ok()
        );
        assert!(
            Audiencia::Agrupaciones(vec![AgrupacionId::nuevo(); 501])
                .validar()
                .is_err()
        );
    }

    #[test]
    fn solo_administracion_o_comunicaciones() {
        assert!(finalidad_comunicado(FinalidadTratamiento::Administracion).is_ok());
        assert!(finalidad_comunicado(FinalidadTratamiento::Publicidad).is_err());
    }

    #[test]
    fn whatsapp_solo_si_es_importante_y_no_tiene_app() {
        assert_eq!(
            canales_comunicado(false, false, true, true),
            CanalesComunicado {
                push: false,
                correo: true,
                whatsapp: false
            }
        );
        assert_eq!(
            canales_comunicado(true, false, false, true),
            CanalesComunicado {
                push: false,
                correo: false,
                whatsapp: true
            }
        );
        assert_eq!(
            canales_comunicado(true, true, true, true),
            CanalesComunicado {
                push: true,
                correo: true,
                whatsapp: false
            }
        );
        assert!(!canales_comunicado(true, false, false, false).alguno());
    }

    #[test]
    fn audiencia_se_serializa_con_tipo_e_ids() {
        let a =
            serde_json::to_value(Audiencia::Roles(vec!["revisor_fiscal".into()])).expect("json");
        assert_eq!(
            a,
            serde_json::json!({"tipo": "roles", "ids": ["revisor_fiscal"]})
        );
        let t = serde_json::to_value(Audiencia::Todos).expect("json");
        assert_eq!(t, serde_json::json!({"tipo": "todos"}));
    }
}
