//! Elección por planchas con **cociente electoral** (consejo de administración, comités).
//!
//! Regla usual en Colombia (aplicada por analogía con el art. 197 del Código de Comercio y
//! adoptada por muchos reglamentos de propiedad horizontal; **validar con el reglamento**):
//!
//! 1. Cociente = votos válidos (incluido el voto en blanco) ÷ cupos.
//! 2. Cada plancha recibe tantos cupos como veces quepa el cociente en sus votos.
//! 3. Los cupos que falten se asignan a los **residuos** mayores.
//!
//! Los votos se pesan con la base de cómputo de la pregunta (coeficientes o unidades). Un
//! empate en el residuo que decide el último cupo queda marcado para que la asamblea lo
//! resuelva (sorteo o nueva votación).

use rust_decimal::{Decimal, RoundingStrategy, prelude::ToPrimitive};
use serde::{Deserialize, Serialize};

use crate::OpcionId;

/// Plancha (o candidato) con sus votos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plancha {
    pub opcion_id: OpcionId,
    pub votos: Decimal,
    /// El voto en blanco cuenta para el cociente pero no gana cupos.
    pub en_blanco: bool,
    /// Integrantes de la plancha: no puede recibir más cupos que candidatos. `None` = sin tope.
    pub capacidad: Option<u16>,
}

/// Cupos asignados a una plancha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CuposPlancha {
    pub opcion_id: OpcionId,
    pub por_cociente: u16,
    pub por_residuo: u16,
    pub residuo: Decimal,
}

impl CuposPlancha {
    /// Total de cupos de la plancha.
    pub const fn total(&self) -> u16 {
        self.por_cociente + self.por_residuo
    }
}

/// Resultado del reparto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reparto {
    pub cociente: Decimal,
    pub cupos: Vec<CuposPlancha>,
    /// Hubo empate de residuos en el último cupo asignado.
    pub empate: bool,
    /// Cupos que no se pudieron asignar (planchas sin candidatos suficientes o sin votos).
    pub sin_asignar: u16,
}

/// Reparte `cupos` entre las planchas por cociente electoral.
pub fn repartir_cupos(planchas: &[Plancha], cupos: u16) -> Reparto {
    let total: Decimal = planchas.iter().map(|p| p.votos).sum();
    let mut resultado: Vec<CuposPlancha> = planchas
        .iter()
        .filter(|p| !p.en_blanco)
        .map(|p| CuposPlancha {
            opcion_id: p.opcion_id,
            por_cociente: 0,
            por_residuo: 0,
            residuo: p.votos,
        })
        .collect();
    if total <= Decimal::ZERO || cupos == 0 {
        return Reparto {
            cociente: Decimal::ZERO,
            cupos: resultado,
            empate: false,
            sin_asignar: cupos,
        };
    }
    let cociente = total / Decimal::from(cupos);
    let capacidad = |id: OpcionId| {
        planchas
            .iter()
            .find(|p| p.opcion_id == id)
            .and_then(|p| p.capacidad)
            .unwrap_or(u16::MAX)
    };
    let mut asignados: u16 = 0;
    for c in &mut resultado {
        let veces = (c.residuo / cociente).round_dp_with_strategy(0, RoundingStrategy::ToZero);
        let veces = veces.to_u16().unwrap_or(u16::MAX);
        let veces = veces.min(capacidad(c.opcion_id)).min(cupos - asignados);
        c.por_cociente = veces;
        c.residuo -= cociente * Decimal::from(veces);
        asignados += veces;
    }

    let mut orden: Vec<usize> = (0..resultado.len()).collect();
    // Residuo mayor primero; a igualdad, el orden de las opciones (estable).
    orden.sort_by(|a, b| resultado[*b].residuo.cmp(&resultado[*a].residuo));
    let mut empate = false;
    let mut ultimo_residuo = None;
    for i in orden {
        if asignados == cupos {
            empate = ultimo_residuo.is_some_and(|r| r == resultado[i].residuo && r > Decimal::ZERO);
            break;
        }
        let c = &mut resultado[i];
        if c.residuo <= Decimal::ZERO || c.total() >= capacidad(c.opcion_id) {
            continue;
        }
        c.por_residuo += 1;
        asignados += 1;
        ultimo_residuo = Some(c.residuo);
    }
    Reparto {
        cociente: cociente.round_dp(8),
        cupos: resultado,
        empate,
        sin_asignar: cupos - asignados,
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;

    fn plancha(votos: Decimal) -> Plancha {
        Plancha {
            opcion_id: OpcionId::nuevo(),
            votos,
            en_blanco: false,
            capacidad: None,
        }
    }

    fn totales(r: &Reparto) -> Vec<u16> {
        r.cupos.iter().map(CuposPlancha::total).collect()
    }

    #[test]
    fn cociente_y_residuos() {
        // 100 votos, 5 cupos → cociente 20. A=47 (2, residuo 7), B=33 (1, residuo 13),
        // C=20 (1, residuo 0). Falta 1 cupo: residuo mayor = B.
        let r = repartir_cupos(
            &[plancha(dec!(47)), plancha(dec!(33)), plancha(dec!(20))],
            5,
        );
        assert_eq!(r.cociente, dec!(20));
        assert_eq!(totales(&r), vec![2, 2, 1]);
        assert!(!r.empate);
        assert_eq!(r.sin_asignar, 0);
    }

    #[test]
    fn por_coeficientes() {
        let r = repartir_cupos(&[plancha(dec!(0.41)), plancha(dec!(0.19))], 3);
        // cociente 0.2: A=2 (residuo 0.01), B=0 (residuo 0.19) → B gana el tercero.
        assert_eq!(totales(&r), vec![2, 1]);
    }

    #[test]
    fn voto_en_blanco_cuenta_para_el_cociente_pero_no_gana() {
        let blanco = Plancha {
            en_blanco: true,
            ..plancha(dec!(60))
        };
        let r = repartir_cupos(&[plancha(dec!(40)), blanco], 2);
        assert_eq!(r.cociente, dec!(50));
        assert_eq!(r.cupos.len(), 1);
        assert_eq!(totales(&r), vec![1]);
        assert_eq!(r.sin_asignar, 1);
    }

    #[test]
    fn respeta_la_capacidad_de_la_plancha() {
        let corta = Plancha {
            capacidad: Some(1),
            ..plancha(dec!(90))
        };
        let r = repartir_cupos(&[corta, plancha(dec!(10))], 3);
        assert_eq!(totales(&r), vec![1, 1]);
        assert_eq!(r.sin_asignar, 1);
    }

    #[test]
    fn empate_en_el_ultimo_residuo() {
        let r = repartir_cupos(&[plancha(dec!(50)), plancha(dec!(50))], 1);
        assert_eq!(totales(&r).iter().sum::<u16>(), 1);
        assert!(r.empate);
    }

    #[test]
    fn sin_votos_no_asigna() {
        let r = repartir_cupos(&[plancha(Decimal::ZERO)], 3);
        assert_eq!(r.sin_asignar, 3);
    }
}
