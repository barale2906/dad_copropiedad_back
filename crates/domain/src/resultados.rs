//! Escrutinio de una pregunta: totales por opción **por coeficiente y por unidades** (se
//! muestran ambos; decide la base de cómputo de la pregunta) y la decisión según su regla.
//!
//! La verdad está en PostgreSQL (`votos`, `participaciones`, `pregunta_habilitados`); el
//! escrutinio se recalcula desde ahí: en vivo cada ~500 ms por reunión y al cerrar, cuando
//! queda guardado en `preguntas.resultado` para el acta.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{
    BaseComputo, OpcionId, Pregunta, ReglaMayoria, TipoPregunta,
    eleccion::{Plancha, Reparto, repartir_cupos},
};

/// Una cantidad en ambas bases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Conteo {
    pub coeficiente: Decimal,
    pub unidades: u32,
}

impl Conteo {
    /// Conteo por coeficiente y por unidades.
    pub fn new(coeficiente: Decimal, unidades: u32) -> Self {
        Self {
            coeficiente,
            unidades,
        }
    }

    /// Valor en la base indicada.
    pub fn en(&self, base: BaseComputo) -> Decimal {
        match base {
            BaseComputo::Coeficiente => self.coeficiente,
            BaseComputo::Unidades => Decimal::from(self.unidades),
        }
    }

    /// `self - otro`, sin bajar de cero.
    pub fn menos(&self, otro: &Self) -> Self {
        Self {
            coeficiente: (self.coeficiente - otro.coeficiente).max(Decimal::ZERO),
            unidades: self.unidades.saturating_sub(otro.unidades),
        }
    }
}

/// Totales de una opción y lo que decidió.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultadoOpcion {
    pub opcion_id: OpcionId,
    pub texto: String,
    pub en_blanco: bool,
    pub votos: Conteo,
    /// Porcentaje sobre lo habilitado, por coeficiente (4 decimales).
    pub porcentaje_coeficiente: Decimal,
    /// Porcentaje sobre lo habilitado, por unidades (4 decimales).
    pub porcentaje_unidades: Decimal,
    /// ¿Alcanza la regla de mayoría en la base de cómputo? (`None` en elecciones.)
    pub aprobada: Option<bool>,
    /// ¿Y en la otra base? Informativo, para mostrar ambos resultados.
    pub aprobada_en_otra_base: Option<bool>,
    /// Cupos ganados (solo elecciones).
    pub cupos: Option<u16>,
}

/// Decisión de la pregunta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    /// Sí/no: la opción «Sí» alcanzó la mayoría.
    Aprobada,
    NoAprobada,
    /// Selección única: la más votada que alcanzó la mayoría.
    Ganadora {
        opcion_id: OpcionId,
    },
    /// Selección única: nadie alcanzó la mayoría o hubo empate en el primer lugar.
    SinGanadora {
        empate: bool,
    },
    /// Selección múltiple: cada opción se decide por separado (ver `aprobada`).
    PorOpcion,
    /// Elección por cociente electoral.
    Reparto(Reparto),
}

/// Escrutinio completo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Escrutinio {
    pub base_computo: BaseComputo,
    pub regla: ReglaMayoria,
    /// Bienes con derecho a votar la pregunta (presentes al abrirla o que ingresaron mientras
    /// estuvo abierta, sin restricción por mora).
    pub habilitados: Conteo,
    /// Toda la copropiedad (padrón de la reunión).
    pub total: Conteo,
    /// Bienes que votaron.
    pub votantes: Conteo,
    /// Habilitados que no votaron.
    pub sin_votar: Conteo,
    pub opciones: Vec<ResultadoOpcion>,
    pub decision: Decision,
    /// La pregunta decide por unidades: la ley exige coeficientes (art. 45; riesgo de
    /// impugnación, art. 49). Se advierte en la interfaz y en el acta.
    pub advertencia_unidades: bool,
}

/// Votos de una opción tal como salen de la BD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VotosOpcion {
    pub opcion_id: OpcionId,
    pub votos: Conteo,
}

fn porcentaje(parte: Decimal, total: Decimal) -> Decimal {
    if total <= Decimal::ZERO {
        Decimal::ZERO
    } else {
        (parte * Decimal::ONE_HUNDRED / total).round_dp(4)
    }
}

fn otra(base: BaseComputo) -> BaseComputo {
    match base {
        BaseComputo::Coeficiente => BaseComputo::Unidades,
        BaseComputo::Unidades => BaseComputo::Coeficiente,
    }
}

impl Escrutinio {
    /// Calcula el escrutinio. `capacidad` da el número de candidatos de cada plancha
    /// (elecciones).
    pub fn calcular(
        pregunta: &Pregunta,
        habilitados: Conteo,
        total: Conteo,
        votantes: Conteo,
        votos: &[VotosOpcion],
        capacidad: impl Fn(OpcionId) -> Option<u16>,
    ) -> Self {
        let base = pregunta.base_computo;
        let regla = pregunta.regla_mayoria;
        let aprueba =
            |c: &Conteo, b: BaseComputo| regla.aprobada(c.en(b), habilitados.en(b), total.en(b));
        let votos_de = |id: OpcionId| {
            votos
                .iter()
                .find(|v| v.opcion_id == id)
                .map(|v| v.votos)
                .unwrap_or_default()
        };
        let mut opciones: Vec<ResultadoOpcion> = pregunta
            .opciones
            .iter()
            .map(|o| {
                let v = votos_de(o.id);
                let decide = match pregunta.tipo {
                    TipoPregunta::Eleccion => false,
                    TipoPregunta::SiNo => {
                        pregunta.opcion_afirmativa().is_some_and(|a| a.id == o.id)
                    }
                    _ => true,
                };
                ResultadoOpcion {
                    opcion_id: o.id,
                    texto: o.texto.clone(),
                    en_blanco: o.en_blanco,
                    votos: v,
                    porcentaje_coeficiente: porcentaje(v.coeficiente, habilitados.coeficiente),
                    porcentaje_unidades: porcentaje(
                        Decimal::from(v.unidades),
                        Decimal::from(habilitados.unidades),
                    ),
                    aprobada: decide.then(|| aprueba(&v, base)),
                    aprobada_en_otra_base: decide.then(|| aprueba(&v, otra(base))),
                    cupos: None,
                }
            })
            .collect();

        let decision = match pregunta.tipo {
            TipoPregunta::SiNo => {
                if opciones.iter().any(|o| o.aprobada == Some(true)) {
                    Decision::Aprobada
                } else {
                    Decision::NoAprobada
                }
            }
            TipoPregunta::SeleccionUnica => {
                let maximo = opciones
                    .iter()
                    .map(|o| o.votos.en(base))
                    .max()
                    .unwrap_or_default();
                let primeras: Vec<_> = opciones
                    .iter()
                    .filter(|o| o.votos.en(base) == maximo && maximo > Decimal::ZERO)
                    .collect();
                match primeras.as_slice() {
                    [unica] if unica.aprobada == Some(true) => Decision::Ganadora {
                        opcion_id: unica.opcion_id,
                    },
                    varias => Decision::SinGanadora {
                        empate: varias.len() > 1,
                    },
                }
            }
            TipoPregunta::SeleccionMultiple => Decision::PorOpcion,
            TipoPregunta::Eleccion => {
                let planchas: Vec<Plancha> = opciones
                    .iter()
                    .map(|o| Plancha {
                        opcion_id: o.opcion_id,
                        votos: o.votos.en(base),
                        en_blanco: o.en_blanco,
                        capacidad: capacidad(o.opcion_id),
                    })
                    .collect();
                let reparto = repartir_cupos(&planchas, pregunta.cupos.unwrap_or(1));
                for o in &mut opciones {
                    o.cupos = reparto
                        .cupos
                        .iter()
                        .find(|c| c.opcion_id == o.opcion_id)
                        .map(crate::eleccion::CuposPlancha::total);
                }
                Decision::Reparto(reparto)
            }
        };
        Self {
            base_computo: base,
            regla,
            habilitados,
            total,
            votantes,
            sin_votar: habilitados.menos(&votantes),
            opciones,
            decision,
            advertencia_unidades: base == BaseComputo::Unidades,
        }
    }

    /// Resultado de una opción.
    pub fn opcion(&self, id: OpcionId) -> Option<&ResultadoOpcion> {
        self.opciones.iter().find(|o| o.opcion_id == id)
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;
    use crate::{EstadoPregunta, TEXTOS_SI_NO, pregunta::tests::pregunta};

    fn votos(p: &Pregunta, conteos: &[(Decimal, u32)]) -> Vec<VotosOpcion> {
        p.opciones
            .iter()
            .zip(conteos)
            .map(|(o, (c, u))| VotosOpcion {
                opcion_id: o.id,
                votos: Conteo::new(*c, *u),
            })
            .collect()
    }

    #[test]
    fn si_no_suma_coeficientes_y_muestra_ambas_bases() {
        let p = pregunta(TipoPregunta::SiNo, &TEXTOS_SI_NO, EstadoPregunta::Cerrada);
        // Presentes 0.60 (40 bienes). Sí = 0.31 con 10 bienes; No = 0.20 con 25 bienes.
        let e = Escrutinio::calcular(
            &p,
            Conteo::new(dec!(0.60), 40),
            Conteo::new(dec!(1), 100),
            Conteo::new(dec!(0.55), 38),
            &votos(&p, &[(dec!(0.31), 10), (dec!(0.20), 25), (dec!(0.04), 3)]),
            |_| None,
        );
        assert_eq!(e.decision, Decision::Aprobada);
        assert_eq!(e.opciones[0].aprobada, Some(true));
        // Por unidades no habría aprobado (10 de 40).
        assert_eq!(e.opciones[0].aprobada_en_otra_base, Some(false));
        assert_eq!(e.opciones[1].aprobada, None);
        assert_eq!(e.sin_votar, Conteo::new(dec!(0.05), 2));
        assert_eq!(e.opciones[0].porcentaje_coeficiente, dec!(51.6667));
        assert!(!e.advertencia_unidades);
    }

    #[test]
    fn calificada_se_mide_sobre_el_total() {
        let mut p = pregunta(
            TipoPregunta::SiNo,
            &TEXTOS_SI_NO[..2],
            EstadoPregunta::Cerrada,
        );
        p.regla_mayoria = ReglaMayoria::Calificada70;
        let e = Escrutinio::calcular(
            &p,
            Conteo::new(dec!(0.75), 60),
            Conteo::new(dec!(1), 100),
            Conteo::new(dec!(0.75), 60),
            &votos(&p, &[(dec!(0.69), 55), (dec!(0.06), 5)]),
            |_| None,
        );
        assert_eq!(e.decision, Decision::NoAprobada);
    }

    #[test]
    fn por_unidades_decide_el_numero_de_bienes_y_advierte() {
        let mut p = pregunta(
            TipoPregunta::SiNo,
            &TEXTOS_SI_NO[..2],
            EstadoPregunta::Cerrada,
        );
        p.base_computo = BaseComputo::Unidades;
        let e = Escrutinio::calcular(
            &p,
            Conteo::new(dec!(0.60), 40),
            Conteo::new(dec!(1), 100),
            Conteo::new(dec!(0.60), 40),
            &votos(&p, &[(dec!(0.10), 21), (dec!(0.50), 19)]),
            |_| None,
        );
        assert_eq!(e.decision, Decision::Aprobada);
        assert!(e.advertencia_unidades);
    }

    #[test]
    fn seleccion_unica_ganadora_o_empate() {
        let p = pregunta(
            TipoPregunta::SeleccionUnica,
            &["A", "B", "C"],
            EstadoPregunta::Cerrada,
        );
        let hab = Conteo::new(dec!(0.6), 30);
        let total = Conteo::new(dec!(1), 100);
        let e = Escrutinio::calcular(
            &p,
            hab,
            total,
            hab,
            &votos(&p, &[(dec!(0.4), 10), (dec!(0.1), 10), (dec!(0.1), 10)]),
            |_| None,
        );
        assert_eq!(
            e.decision,
            Decision::Ganadora {
                opcion_id: p.opciones[0].id
            }
        );

        let e = Escrutinio::calcular(
            &p,
            hab,
            total,
            hab,
            &votos(&p, &[(dec!(0.2), 10), (dec!(0.2), 10), (dec!(0.2), 10)]),
            |_| None,
        );
        assert_eq!(e.decision, Decision::SinGanadora { empate: true });

        let e = Escrutinio::calcular(
            &p,
            hab,
            total,
            hab,
            &votos(&p, &[(dec!(0.25), 10), (dec!(0.2), 10), (dec!(0.15), 10)]),
            |_| None,
        );
        assert_eq!(e.decision, Decision::SinGanadora { empate: false });
    }

    #[test]
    fn seleccion_multiple_decide_cada_opcion() {
        let mut p = pregunta(
            TipoPregunta::SeleccionMultiple,
            &["A", "B"],
            EstadoPregunta::Cerrada,
        );
        p.max_selecciones = 2;
        let hab = Conteo::new(dec!(0.6), 30);
        let e = Escrutinio::calcular(
            &p,
            hab,
            Conteo::new(dec!(1), 100),
            hab,
            &votos(&p, &[(dec!(0.5), 25), (dec!(0.2), 9)]),
            |_| None,
        );
        assert_eq!(e.decision, Decision::PorOpcion);
        assert_eq!(e.opciones[0].aprobada, Some(true));
        assert_eq!(e.opciones[1].aprobada, Some(false));
    }

    #[test]
    fn eleccion_reparte_por_cociente() {
        let mut p = pregunta(
            TipoPregunta::Eleccion,
            &["Plancha 1", "Plancha 2"],
            EstadoPregunta::Cerrada,
        );
        p.cupos = Some(5);
        let hab = Conteo::new(dec!(1), 100);
        let e = Escrutinio::calcular(
            &p,
            hab,
            hab,
            hab,
            &votos(&p, &[(dec!(0.47), 40), (dec!(0.53), 60)]),
            |_| Some(5),
        );
        let Decision::Reparto(r) = &e.decision else {
            panic!("reparto")
        };
        assert_eq!(r.cupos.iter().map(|c| c.total()).sum::<u16>(), 5);
        assert_eq!(e.opciones[0].cupos, Some(2));
        assert_eq!(e.opciones[1].cupos, Some(3));
        assert_eq!(e.opciones[0].aprobada, None);
    }
}
