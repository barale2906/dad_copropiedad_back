//! Preguntas sometidas a votación: tipos (sí/no, selección única o múltiple, elección por
//! cociente electoral), visibilidad del voto y ciclo de vida.

use serde::{Deserialize, Serialize};

use crate::{BaseComputo, DominioError, OpcionId, PreguntaId, ReglaMayoria, ReunionId};

/// Ciclo de vida de una pregunta sometida a votación.
///
/// ```text
/// Borrador ──abrir──▶ Abierta ──cerrar──▶ Cerrada
///    │                   │
///    └──────anular───────┴──▶ Anulada
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoPregunta {
    Borrador,
    Abierta,
    Cerrada,
    Anulada,
}

enum_texto!(EstadoPregunta {
    Borrador => "borrador",
    Abierta => "abierta",
    Cerrada => "cerrada",
    Anulada => "anulada",
});

impl EstadoPregunta {
    /// Valida y aplica una transición de estado. Única fuente de verdad de las transiciones.
    ///
    /// # Errors
    ///
    /// [`DominioError::TransicionInvalida`] si no está permitida.
    pub fn transicionar(self, hacia: Self) -> Result<Self, DominioError> {
        use EstadoPregunta::{Abierta, Anulada, Borrador, Cerrada};
        match (self, hacia) {
            (Borrador, Abierta) | (Abierta, Cerrada) | (Borrador | Abierta, Anulada) => Ok(hacia),
            _ => Err(DominioError::TransicionInvalida {
                entidad: "pregunta",
                desde: self.as_str(),
                hacia: hacia.as_str(),
            }),
        }
    }

    /// Verifica que la pregunta acepte votos.
    ///
    /// # Errors
    ///
    /// [`DominioError::PreguntaNoAbierta`].
    pub fn exigir_abierta(self) -> Result<(), DominioError> {
        if self == Self::Abierta {
            Ok(())
        } else {
            Err(DominioError::PreguntaNoAbierta(self))
        }
    }
}

/// Qué se pregunta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoPregunta {
    /// «Sí» (opción 1), «No» y, si se configura, «Abstención».
    SiNo,
    SeleccionUnica,
    /// Hasta `max_selecciones` opciones; cada una se decide por separado.
    SeleccionMultiple,
    /// Planchas o candidatos para `cupos` cargos, repartidos por cociente electoral.
    Eleccion,
}

enum_texto!(TipoPregunta {
    SiNo => "si_no",
    SeleccionUnica => "seleccion_unica",
    SeleccionMultiple => "seleccion_multiple",
    Eleccion => "eleccion",
});

/// Visibilidad del voto (ADR 0010: no existe el secreto absoluto).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoVoto {
    /// El detalle bien privado → opción lo ve la asamblea y va en el acta.
    Nominal,
    /// El vínculo se guarda, pero solo lo consulta quien tiene `asambleas.votos.ver_detalle`
    /// (cada consulta se audita); los resultados públicos muestran solo totales.
    Reservado,
}

enum_texto!(TipoVoto {
    Nominal => "nominal",
    Reservado => "reservado",
});

/// Cargo o designación que se elige con una pregunta de elección.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CargoEleccion {
    PresidenteAsamblea,
    SecretarioAsamblea,
    ComisionVerificadora,
    Consejero,
    RevisorFiscal,
    ComiteConvivencia,
}

enum_texto!(CargoEleccion {
    PresidenteAsamblea => "presidente_asamblea",
    SecretarioAsamblea => "secretario_asamblea",
    ComisionVerificadora => "comision_verificadora",
    Consejero => "consejero",
    RevisorFiscal => "revisor_fiscal",
    ComiteConvivencia => "comite_convivencia",
});

impl CargoEleccion {
    /// Rol plantilla que reciben los elegidos (asignación con origen `asamblea`). Las
    /// designaciones de la propia reunión (presidente, secretario, comisión) no crean roles.
    pub const fn rol_plantilla(self) -> Option<&'static str> {
        match self {
            Self::Consejero => Some("consejero"),
            Self::RevisorFiscal => Some("revisor_fiscal"),
            Self::ComiteConvivencia => Some("comite_convivencia"),
            Self::PresidenteAsamblea | Self::SecretarioAsamblea | Self::ComisionVerificadora => {
                None
            }
        }
    }
}

/// Opción de una pregunta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpcionPregunta {
    pub id: OpcionId,
    pub orden: u16,
    pub texto: String,
    /// Voto en blanco (elecciones): cuenta para el cociente pero no gana cupos.
    pub en_blanco: bool,
}

/// Pregunta tal como la carga el repositorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pregunta {
    pub id: PreguntaId,
    pub reunion_id: ReunionId,
    pub enunciado: String,
    pub tipo: TipoPregunta,
    pub tipo_voto: TipoVoto,
    pub base_computo: BaseComputo,
    pub regla_mayoria: ReglaMayoria,
    pub max_selecciones: u16,
    pub cupos: Option<u16>,
    pub cargo: Option<CargoEleccion>,
    pub estado: EstadoPregunta,
    /// En orden.
    pub opciones: Vec<OpcionPregunta>,
}

/// Textos de las opciones de una pregunta sí/no.
pub const TEXTOS_SI_NO: [&str; 3] = ["Sí", "No", "Abstención"];

impl Pregunta {
    /// Única forma de obtener una [`PreguntaAbierta`].
    ///
    /// # Errors
    ///
    /// [`DominioError::PreguntaNoAbierta`] si la pregunta no está abierta.
    pub fn para_votar(&self) -> Result<PreguntaAbierta<'_>, DominioError> {
        self.estado.exigir_abierta()?;
        Ok(PreguntaAbierta(self))
    }

    /// Valida que la definición sea coherente con su tipo (se exige al crearla y al abrirla).
    ///
    /// # Errors
    ///
    /// [`DominioError::DatoInvalido`] con la regla incumplida.
    pub fn validar_definicion(&self) -> Result<(), DominioError> {
        let invalido = |regla| {
            Err(DominioError::DatoInvalido {
                campo: "pregunta",
                regla,
            })
        };
        let validas = self.opciones.iter().filter(|o| !o.en_blanco).count();
        if self.opciones.len() < 2 {
            return invalido("necesita al menos dos opciones");
        }
        let mut textos: Vec<String> = self
            .opciones
            .iter()
            .map(|o| o.texto.to_lowercase())
            .collect();
        textos.sort();
        textos.dedup();
        if textos.len() != self.opciones.len() {
            return invalido("las opciones no pueden repetirse");
        }
        if self.tipo != TipoPregunta::Eleccion && validas != self.opciones.len() {
            return invalido("el voto en blanco solo aplica en elecciones");
        }
        match self.tipo {
            TipoPregunta::SiNo if !(2..=3).contains(&self.opciones.len()) => {
                invalido("sí/no lleva «Sí», «No» y opcionalmente «Abstención»")
            }
            TipoPregunta::SeleccionMultiple
                if self.max_selecciones < 2 || usize::from(self.max_selecciones) > validas =>
            {
                invalido("la selección múltiple permite de 2 al número de opciones")
            }
            TipoPregunta::SiNo | TipoPregunta::SeleccionUnica | TipoPregunta::Eleccion
                if self.max_selecciones != 1 =>
            {
                invalido("solo la selección múltiple permite elegir varias opciones")
            }
            TipoPregunta::Eleccion if self.cupos.is_none_or(|c| c == 0) || validas == 0 => {
                invalido("una elección necesita cupos y al menos una plancha o candidato")
            }
            TipoPregunta::SiNo | TipoPregunta::SeleccionUnica | TipoPregunta::SeleccionMultiple
                if self.cupos.is_some() || self.cargo.is_some() =>
            {
                invalido("los cupos y el cargo solo aplican a elecciones")
            }
            _ => Ok(()),
        }
    }

    /// Opción que aprueba una pregunta sí/no («Sí», la primera).
    pub fn opcion_afirmativa(&self) -> Option<&OpcionPregunta> {
        (self.tipo == TipoPregunta::SiNo)
            .then(|| self.opciones.iter().min_by_key(|o| o.orden))
            .flatten()
    }

    pub fn opcion(&self, id: OpcionId) -> Option<&OpcionPregunta> {
        self.opciones.iter().find(|o| o.id == id)
    }
}

/// Prueba, verificada por el compilador, de que la pregunta estaba abierta al validarla.
///
/// No tiene constructor público: solo [`Pregunta::para_votar`] la crea. Como la emisión de
/// votos ([`PreguntaAbierta::emitir`](crate::voto)) solo existe sobre este tipo, «votar en una
/// pregunta cerrada» no compila. La base de datos vuelve a verificar el estado dentro de la
/// transacción (la pregunta pudo cerrarse entre la lectura y la escritura).
#[derive(Debug, Clone, Copy)]
pub struct PreguntaAbierta<'a>(&'a Pregunta);

impl PreguntaAbierta<'_> {
    pub fn pregunta(&self) -> &Pregunta {
        self.0
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::EstadoPregunta::{self, *};
    use super::*;

    /// Pregunta de prueba con opciones `textos`.
    pub(crate) fn pregunta(
        tipo: TipoPregunta,
        textos: &[&str],
        estado: EstadoPregunta,
    ) -> Pregunta {
        Pregunta {
            id: PreguntaId::nuevo(),
            reunion_id: ReunionId::nuevo(),
            enunciado: "¿Aprueba el presupuesto 2027?".into(),
            tipo,
            tipo_voto: TipoVoto::Nominal,
            base_computo: BaseComputo::Coeficiente,
            regla_mayoria: ReglaMayoria::Simple,
            max_selecciones: 1,
            cupos: (tipo == TipoPregunta::Eleccion).then_some(1),
            cargo: None,
            estado,
            opciones: textos
                .iter()
                .enumerate()
                .map(|(i, t)| OpcionPregunta {
                    id: OpcionId::nuevo(),
                    orden: u16::try_from(i + 1).unwrap(),
                    texto: (*t).to_owned(),
                    en_blanco: false,
                })
                .collect(),
        }
    }

    #[test]
    fn transiciones_validas() {
        assert_eq!(Borrador.transicionar(Abierta).unwrap(), Abierta);
        assert_eq!(Abierta.transicionar(Cerrada).unwrap(), Cerrada);
        assert_eq!(Borrador.transicionar(Anulada).unwrap(), Anulada);
        assert_eq!(Abierta.transicionar(Anulada).unwrap(), Anulada);
    }

    #[test]
    fn transiciones_invalidas() {
        for (desde, hacia) in [
            (Borrador, Cerrada),
            (Cerrada, Abierta),
            (Cerrada, Anulada),
            (Anulada, Abierta),
            (Abierta, Borrador),
        ] {
            assert!(
                desde.transicionar(hacia).is_err(),
                "{desde} -> {hacia} debería fallar"
            );
        }
    }

    #[test]
    fn solo_abierta_recibe_votos() {
        assert!(Abierta.exigir_abierta().is_ok());
        for e in [Borrador, Cerrada, Anulada] {
            assert!(e.exigir_abierta().is_err());
        }
    }

    #[test]
    fn definicion_coherente_con_el_tipo() {
        assert!(
            pregunta(TipoPregunta::SiNo, &TEXTOS_SI_NO, Borrador)
                .validar_definicion()
                .is_ok()
        );
        assert!(
            pregunta(TipoPregunta::SiNo, &["Sí"], Borrador)
                .validar_definicion()
                .is_err()
        );
        assert!(
            pregunta(TipoPregunta::SeleccionUnica, &["A", "a"], Borrador)
                .validar_definicion()
                .is_err()
        );
        let mut multiple = pregunta(TipoPregunta::SeleccionMultiple, &["A", "B", "C"], Borrador);
        assert!(
            multiple.validar_definicion().is_err(),
            "max_selecciones = 1"
        );
        multiple.max_selecciones = 3;
        assert!(multiple.validar_definicion().is_ok());
        multiple.max_selecciones = 4;
        assert!(multiple.validar_definicion().is_err());

        let mut eleccion = pregunta(
            TipoPregunta::Eleccion,
            &["Plancha 1", "En blanco"],
            Borrador,
        );
        eleccion.opciones[1].en_blanco = true;
        assert!(eleccion.validar_definicion().is_ok());
        eleccion.cupos = None;
        assert!(eleccion.validar_definicion().is_err());

        let mut blanco = pregunta(TipoPregunta::SeleccionUnica, &["A", "B"], Borrador);
        blanco.opciones[1].en_blanco = true;
        assert!(blanco.validar_definicion().is_err());
    }

    #[test]
    fn opcion_afirmativa_es_la_primera_del_si_no() {
        let p = pregunta(TipoPregunta::SiNo, &TEXTOS_SI_NO, Borrador);
        assert_eq!(p.opcion_afirmativa().map(|o| o.texto.as_str()), Some("Sí"));
        assert!(
            pregunta(TipoPregunta::SeleccionUnica, &["A", "B"], Borrador)
                .opcion_afirmativa()
                .is_none()
        );
    }

    #[test]
    fn solo_consejo_revisor_y_comite_crean_roles() {
        assert_eq!(CargoEleccion::Consejero.rol_plantilla(), Some("consejero"));
        assert_eq!(CargoEleccion::PresidenteAsamblea.rol_plantilla(), None);
    }
}
