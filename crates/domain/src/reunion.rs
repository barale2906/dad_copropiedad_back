//! Reunión de la asamblea general de propietarios (Ley 675 de 2001, arts. 37–49): ciclo de
//! vida, modalidad, configuración por asamblea y plazos legales de la convocatoria.

use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::{DominioError, calendario};

/// Ciclo de vida de una reunión.
///
/// ```text
/// Borrador ─convocar─▶ Convocada ─abrir registro─▶ EnRegistro ─instalar─▶ Instalada ⇄ Suspendida
///    │                    │                           │                       │           │
///    └────────────────────┴───────cancelar────────────┘                       └──cerrar───┴─▶ Cerrada
/// ```
///
/// Una consulta por escrito (art. 43) pasa de `Convocada` a `Cerrada` al vencer el plazo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoReunion {
    /// En preparación: orden del día, documentos y configuración editables.
    Borrador,
    /// Convocatoria enviada; el padrón quedó fijado. Se aceptan prerregistros.
    Convocada,
    /// El día de la reunión: la mesa registra asistentes y el quórum corre en vivo.
    EnRegistro,
    /// Quórum verificado y sesión instalada: sus preguntas pueden abrirse.
    Instalada,
    /// Receso: no se abren preguntas; la asistencia sigue registrándose.
    Suspendida,
    Cerrada,
    Cancelada,
}

enum_texto!(EstadoReunion {
    Borrador => "borrador",
    Convocada => "convocada",
    EnRegistro => "en_registro",
    Instalada => "instalada",
    Suspendida => "suspendida",
    Cerrada => "cerrada",
    Cancelada => "cancelada",
});

impl EstadoReunion {
    /// Valida y aplica una transición. Única fuente de verdad de las transiciones.
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoReunion::{
            Borrador, Cancelada, Cerrada, Convocada, EnRegistro, Instalada, Suspendida,
        };
        match (self, hacia) {
            (Borrador, Convocada | Cancelada)
            | (Convocada, EnRegistro | Cancelada | Cerrada)
            | (EnRegistro, Instalada | Cancelada)
            | (Instalada, Suspendida | Cerrada)
            | (Suspendida, Instalada | Cerrada) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "reunión",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }

    /// Una pregunta solo puede abrirse en una reunión instalada.
    ///
    /// # Errors
    ///
    /// [`DominioError::ReunionNoInstalada`].
    pub fn exigir_instalada(self) -> Result<(), DominioError> {
        if self == Self::Instalada {
            Ok(())
        } else {
            Err(DominioError::ReunionNoInstalada(self))
        }
    }

    /// ¿Se puede registrar asistencia (entrada, salida, poderes, rótulos)?
    pub const fn admite_registro(self) -> bool {
        matches!(self, Self::EnRegistro | Self::Instalada | Self::Suspendida)
    }

    /// ¿Se puede preparar (orden del día, documentos, configuración)?
    pub const fn editable(self) -> bool {
        matches!(self, Self::Borrador)
    }

    /// Reuniones con las que un asistente puede interactuar (registrarse o votar).
    pub const fn activa(self) -> bool {
        matches!(
            self,
            Self::Convocada | Self::EnRegistro | Self::Instalada | Self::Suspendida
        )
    }
}

/// Tipo de reunión según la Ley 675 (art. 39).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoReunion {
    /// Una vez al año, dentro de los tres meses siguientes al vencimiento del periodo
    /// presupuestal.
    Ordinaria,
    Extraordinaria,
}

enum_texto!(TipoReunion {
    Ordinaria => "ordinaria",
    Extraordinaria => "extraordinaria",
});

/// Cómo se reúne la asamblea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modalidad {
    Presencial,
    /// No presencial (art. 42): video en un proveedor externo; votación y quórum aquí.
    Virtual,
    /// Presencial y virtual a la vez: ambos suman al mismo quórum.
    Mixta,
    /// Decisión por comunicación escrita (art. 43): sin reunión, con plazo para responder.
    Escrita,
}

enum_texto!(Modalidad {
    Presencial => "presencial",
    Virtual => "virtual",
    Mixta => "mixta",
    Escrita => "escrita",
});

/// Qué pasa con los bienes privados del listado de morosos que entrega la copropiedad.
///
/// **Validar con asesor**: la jurisprudencia ha limitado restringir el voto del moroso; la
/// elección queda auditada y en el acta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EfectoMora {
    /// Solo se marca en el registro y en el acta.
    #[default]
    Informativo,
    /// Asiste y cuenta para el quórum, pero no vota.
    SinVoto,
    /// No vota ni cuenta para el quórum.
    SinVotoNiQuorum,
}

enum_texto!(EfectoMora {
    Informativo => "informativo",
    SinVoto => "sin_voto",
    SinVotoNiQuorum => "sin_voto_ni_quorum",
});

impl EfectoMora {
    /// Si un bien moroso vota con este efecto de la mora.
    pub const fn vota(self) -> bool {
        matches!(self, Self::Informativo)
    }

    /// Si un bien moroso cuenta para el quórum con este efecto de la mora.
    pub const fn cuenta_para_quorum(self) -> bool {
        !matches!(self, Self::SinVotoNiQuorum)
    }
}

/// Estado del acta de la reunión.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoActa {
    Pendiente,
    /// PDF generado; puede regenerarse mientras no se apruebe.
    Generada,
    /// Aprobada por la comisión verificadora, con las firmas escaneadas.
    Aprobada,
    /// Publicada a los propietarios (libro de actas).
    Publicada,
}

enum_texto!(EstadoActa {
    Pendiente => "pendiente",
    Generada => "generada",
    Aprobada => "aprobada",
    Publicada => "publicada",
});

impl EstadoActa {
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoActa::{Aprobada, Generada, Pendiente, Publicada};
        match (self, hacia) {
            (Pendiente | Generada, Generada) | (Generada, Aprobada) | (Aprobada, Publicada) => {
                Ok(hacia)
            }
            _ => Err(DominioError::TransicionInvalida {
                entidad: "acta",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }
}

/// Estado de un punto del orden del día.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoPunto {
    Pendiente,
    EnCurso,
    Terminado,
    Aplazado,
}

enum_texto!(EstadoPunto {
    Pendiente => "pendiente",
    EnCurso => "en_curso",
    Terminado => "terminado",
    Aplazado => "aplazado",
});

impl EstadoPunto {
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoPunto::{Aplazado, EnCurso, Pendiente, Terminado};
        match (self, hacia) {
            (Pendiente | Aplazado, EnCurso)
            | (EnCurso, Terminado | Aplazado)
            | (Pendiente, Aplazado) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "punto del orden del día",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }
}

/// Tipo de documento anexo a la convocatoria.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoDocumentoReunion {
    EstadosFinancieros,
    Presupuesto,
    Informe,
    Reglamento,
    Otro,
}

enum_texto!(TipoDocumentoReunion {
    EstadosFinancieros => "estados_financieros",
    Presupuesto => "presupuesto",
    Informe => "informe",
    Reglamento => "reglamento",
    Otro => "otro",
});

/// Días calendario mínimos entre la convocatoria y una reunión ordinaria (art. 39).
pub const ANTELACION_ORDINARIA_DIAS: u64 = 15;
/// Plazo máximo para responder una consulta por escrito (art. 43: un mes).
pub const MESES_CONSULTA_ESCRITA: u32 = 1;
/// Hora de la segunda convocatoria y de la reunión por derecho propio (arts. 39 y 41).
pub const HORA_LEGAL: NaiveTime = match NaiveTime::from_hms_opt(20, 0, 0) {
    Some(h) => h,
    None => NaiveTime::MIN,
};

/// Antelación mínima legal para convocar. `None` = la fija el reglamento (extraordinarias).
pub const fn antelacion_minima_dias(tipo: TipoReunion) -> Option<u64> {
    match tipo {
        TipoReunion::Ordinaria => Some(ANTELACION_ORDINARIA_DIAS),
        TipoReunion::Extraordinaria => None,
    }
}

/// ¿Se respeta la antelación mínima entre `convocada_el` y `reunion_el` (fechas locales)?
pub fn antelacion_suficiente(
    tipo: TipoReunion,
    convocada_el: NaiveDate,
    reunion_el: NaiveDate,
) -> bool {
    match antelacion_minima_dias(tipo) {
        Some(dias) => convocada_el
            .checked_add_days(Days::new(dias))
            .is_some_and(|minimo| reunion_el >= minimo),
        None => reunion_el >= convocada_el,
    }
}

/// Fecha y hora local de la reunión de segunda convocatoria cuando la de primera no tuvo
/// quórum: el tercer día hábil siguiente, a las 8:00 p. m. (art. 41).
pub fn fecha_segunda_convocatoria(primera_el: NaiveDate) -> Option<NaiveDateTime> {
    calendario::sumar_dias_habiles(primera_el, 3).map(|d| d.and_time(HORA_LEGAL))
}

/// Fecha y hora local de la reunión ordinaria por derecho propio si no se convocó: el primer
/// día hábil del cuarto mes siguiente al vencimiento del periodo presupuestal, a las 8:00 p. m.
/// (art. 39, parágrafo). `fin_periodo` es el último día del periodo (normalmente el 31 de
/// diciembre).
pub fn fecha_derecho_propio(fin_periodo: NaiveDate) -> Option<NaiveDateTime> {
    let primero_siguiente = fin_periodo.succ_opt()?.with_day(1)?;
    let mut dia = primero_siguiente.checked_add_months(chrono::Months::new(3))?;
    while !calendario::es_dia_habil(dia) {
        dia = dia.succ_opt()?;
    }
    Some(dia.and_time(HORA_LEGAL))
}

#[cfg(test)]
mod tests {
    use super::EstadoReunion::*;
    use super::*;

    fn fecha(a: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(a, m, d).unwrap()
    }

    #[test]
    fn transiciones_validas_e_invalidas() {
        for (d, h) in [
            (Borrador, Convocada),
            (Convocada, EnRegistro),
            (EnRegistro, Instalada),
            (Instalada, Suspendida),
            (Suspendida, Instalada),
            (Instalada, Cerrada),
            (Convocada, Cerrada),
            (EnRegistro, Cancelada),
        ] {
            assert_eq!(d.transicionar(h).unwrap(), h, "{d} -> {h}");
        }
        for (d, h) in [
            (Borrador, Instalada),
            (Convocada, Instalada),
            (Cerrada, Instalada),
            (Instalada, Cancelada),
            (Cancelada, Convocada),
            (Instalada, EnRegistro),
        ] {
            assert!(d.transicionar(h).is_err(), "{d} -> {h}");
        }
    }

    #[test]
    fn solo_instalada_permite_abrir_preguntas() {
        assert!(Instalada.exigir_instalada().is_ok());
        for e in [Convocada, EnRegistro, Suspendida] {
            assert!(e.exigir_instalada().is_err());
        }
    }

    #[test]
    fn registro_y_edicion_segun_estado() {
        assert!(EnRegistro.admite_registro() && Suspendida.admite_registro());
        assert!(!Convocada.admite_registro() && !Cerrada.admite_registro());
        assert!(Borrador.editable() && !Convocada.editable());
    }

    #[test]
    fn textos_ida_y_vuelta() {
        for e in EstadoReunion::TODOS {
            assert_eq!(EstadoReunion::try_from(e.as_str()).unwrap(), *e);
        }
        for m in Modalidad::TODOS {
            assert_eq!(Modalidad::try_from(m.as_str()).unwrap(), *m);
        }
        for e in EfectoMora::TODOS {
            assert_eq!(EfectoMora::try_from(e.as_str()).unwrap(), *e);
        }
    }

    #[test]
    fn efecto_de_la_mora() {
        assert!(EfectoMora::Informativo.vota() && EfectoMora::Informativo.cuenta_para_quorum());
        assert!(!EfectoMora::SinVoto.vota() && EfectoMora::SinVoto.cuenta_para_quorum());
        assert!(!EfectoMora::SinVotoNiQuorum.cuenta_para_quorum());
    }

    #[test]
    fn antelacion_de_quince_dias_en_ordinarias() {
        let convocada = fecha(2027, 3, 1);
        assert!(antelacion_suficiente(
            TipoReunion::Ordinaria,
            convocada,
            fecha(2027, 3, 16)
        ));
        assert!(!antelacion_suficiente(
            TipoReunion::Ordinaria,
            convocada,
            fecha(2027, 3, 15)
        ));
        assert!(antelacion_suficiente(
            TipoReunion::Extraordinaria,
            convocada,
            fecha(2027, 3, 2)
        ));
        assert!(!antelacion_suficiente(
            TipoReunion::Extraordinaria,
            convocada,
            fecha(2027, 2, 28)
        ));
    }

    #[test]
    fn segunda_convocatoria_tercer_dia_habil_a_las_ocho() {
        // Jueves Santo de 2026: no cuentan Viernes Santo ni el fin de semana.
        assert_eq!(
            fecha_segunda_convocatoria(fecha(2026, 4, 2)),
            Some(fecha(2026, 4, 8).and_time(HORA_LEGAL))
        );
    }

    #[test]
    fn derecho_propio_primer_dia_habil_de_abril() {
        assert_eq!(
            fecha_derecho_propio(fecha(2026, 12, 31)),
            Some(fecha(2027, 4, 1).and_time(HORA_LEGAL))
        );
        // Periodo 2028: el 1 de abril de 2029 es domingo de Pascua; el lunes 2 es hábil.
        assert_eq!(
            fecha_derecho_propio(fecha(2028, 12, 31)),
            Some(fecha(2029, 4, 2).and_time(HORA_LEGAL))
        );
    }

    #[test]
    fn acta_y_puntos() {
        assert!(
            EstadoActa::Pendiente
                .transicionar(EstadoActa::Generada)
                .is_ok()
        );
        assert!(
            EstadoActa::Generada
                .transicionar(EstadoActa::Generada)
                .is_ok()
        );
        assert!(
            EstadoActa::Pendiente
                .transicionar(EstadoActa::Aprobada)
                .is_err()
        );
        assert!(
            EstadoActa::Publicada
                .transicionar(EstadoActa::Generada)
                .is_err()
        );
        assert!(
            EstadoPunto::Pendiente
                .transicionar(EstadoPunto::EnCurso)
                .is_ok()
        );
        assert!(
            EstadoPunto::Terminado
                .transicionar(EstadoPunto::EnCurso)
                .is_err()
        );
    }
}
