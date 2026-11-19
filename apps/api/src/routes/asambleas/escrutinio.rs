//! Forma del escrutinio y del acta en el contrato.
//!
//! El dominio ([`domain::Escrutinio`]) y el puerto de actas ([`DatosActa`]) no conocen utoipa:
//! aquí viven sus espejos con `ToSchema`. Las conversiones desestructuran cada tipo **sin**
//! `..`, de modo que un campo nuevo en el dominio no compila hasta que se refleje aquí (y en el
//! contrato). Los decimales viajan como texto.

use application::puertos::actas::{AsistenteActa, DatosActa, PreguntaActa, QuorumActa, VotoActa};
use domain::{
    BaseComputo, Escrutinio,
    eleccion::{CuposPlancha, Reparto},
    resultados::{Conteo, Decision, ResultadoOpcion},
};
use rust_decimal::Decimal;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::dto::ReglaDto;

/// Una cantidad en ambas bases.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ConteoEscrutinio)]
pub struct ConteoDto {
    /// Suma de coeficientes (fracción de 1).
    #[schema(value_type = String, example = "0.55")]
    pub coeficiente: Decimal,
    /// Número de bienes privados.
    pub unidades: u32,
}

impl From<Conteo> for ConteoDto {
    fn from(c: Conteo) -> Self {
        let Conteo {
            coeficiente,
            unidades,
        } = c;
        Self {
            coeficiente,
            unidades,
        }
    }
}

/// Totales de una opción y lo que decidió.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ResultadoOpcion)]
pub struct ResultadoOpcionDto {
    pub opcion_id: Uuid,
    pub texto: String,
    pub en_blanco: bool,
    pub votos: ConteoDto,
    /// Porcentaje sobre lo habilitado, por coeficiente (4 decimales).
    #[schema(value_type = String, example = "55.0000")]
    pub porcentaje_coeficiente: Decimal,
    /// Porcentaje sobre lo habilitado, por unidades (4 decimales).
    #[schema(value_type = String, example = "50.0000")]
    pub porcentaje_unidades: Decimal,
    /// ¿Alcanza la regla de mayoría en la base de cómputo? `null` en elecciones.
    pub aprobada: Option<bool>,
    /// ¿Y en la otra base? Informativo, para mostrar ambos resultados.
    pub aprobada_en_otra_base: Option<bool>,
    /// Cupos ganados (solo elecciones).
    pub cupos: Option<u16>,
}

impl From<ResultadoOpcion> for ResultadoOpcionDto {
    fn from(r: ResultadoOpcion) -> Self {
        let ResultadoOpcion {
            opcion_id,
            texto,
            en_blanco,
            votos,
            porcentaje_coeficiente,
            porcentaje_unidades,
            aprobada,
            aprobada_en_otra_base,
            cupos,
        } = r;
        Self {
            opcion_id: opcion_id.as_uuid(),
            texto,
            en_blanco,
            votos: votos.into(),
            porcentaje_coeficiente,
            porcentaje_unidades,
            aprobada,
            aprobada_en_otra_base,
            cupos,
        }
    }
}

/// Cupos asignados a una plancha.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CuposPlancha)]
pub struct CuposPlanchaDto {
    pub opcion_id: Uuid,
    pub por_cociente: u16,
    pub por_residuo: u16,
    /// Votos que sobraron tras el cociente.
    #[schema(value_type = String, example = "0.12")]
    pub residuo: Decimal,
}

impl From<CuposPlancha> for CuposPlanchaDto {
    fn from(c: CuposPlancha) -> Self {
        let CuposPlancha {
            opcion_id,
            por_cociente,
            por_residuo,
            residuo,
        } = c;
        Self {
            opcion_id: opcion_id.as_uuid(),
            por_cociente,
            por_residuo,
            residuo,
        }
    }
}

/// Reparto de cupos por cociente electoral (elecciones).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = Reparto)]
pub struct RepartoDto {
    #[schema(value_type = String, example = "0.2")]
    pub cociente: Decimal,
    pub cupos: Vec<CuposPlanchaDto>,
    /// Hubo empate de residuos en el último cupo asignado.
    pub empate: bool,
    /// Cupos que no se pudieron asignar (planchas sin candidatos suficientes o sin votos).
    pub sin_asignar: u16,
}

impl From<Reparto> for RepartoDto {
    fn from(r: Reparto) -> Self {
        let Reparto {
            cociente,
            cupos,
            empate,
            sin_asignar,
        } = r;
        Self {
            cociente,
            cupos: cupos.into_iter().map(Into::into).collect(),
            empate,
            sin_asignar,
        }
    }
}

/// Decisión de la pregunta, etiquetada por `decision`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "decision", rename_all = "snake_case")]
#[schema(as = Decision)]
pub enum DecisionDto {
    /// Sí/no: la opción «Sí» alcanzó la mayoría.
    Aprobada,
    /// Sí/no: la opción «Sí» no alcanzó la mayoría.
    NoAprobada,
    /// Selección única: la más votada que alcanzó la mayoría.
    Ganadora { opcion_id: Uuid },
    /// Selección única: nadie alcanzó la mayoría (`empate`: hubo empate en el primer lugar).
    SinGanadora { empate: bool },
    /// Selección múltiple: cada opción se decide por separado (ver `aprobada`).
    PorOpcion,
    /// Elección por cociente electoral: los campos de [`RepartoDto`] junto a la etiqueta.
    Reparto(RepartoDto),
}

impl From<Decision> for DecisionDto {
    fn from(d: Decision) -> Self {
        match d {
            Decision::Aprobada => Self::Aprobada,
            Decision::NoAprobada => Self::NoAprobada,
            Decision::Ganadora { opcion_id } => Self::Ganadora {
                opcion_id: opcion_id.as_uuid(),
            },
            Decision::SinGanadora { empate } => Self::SinGanadora { empate },
            Decision::PorOpcion => Self::PorOpcion,
            Decision::Reparto(r) => Self::Reparto(r.into()),
        }
    }
}

/// Escrutinio de una pregunta: totales por coeficiente **y** por unidades (decide
/// `base_computo`) y la decisión según su regla.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = Escrutinio)]
pub struct EscrutinioDto {
    #[schema(value_type = crate::esquemas::BaseComputo)]
    pub base_computo: BaseComputo,
    /// La misma forma que `regla_mayoria` de la pregunta (`calificada_70`, no `calificada70`).
    pub regla: ReglaDto,
    /// Bienes con derecho a votar la pregunta (presentes al abrirla o que ingresaron mientras
    /// estuvo abierta, sin restricción por mora).
    pub habilitados: ConteoDto,
    /// Toda la copropiedad (padrón de la reunión).
    pub total: ConteoDto,
    /// Bienes que votaron.
    pub votantes: ConteoDto,
    /// Habilitados que no votaron.
    pub sin_votar: ConteoDto,
    pub opciones: Vec<ResultadoOpcionDto>,
    pub decision: DecisionDto,
    /// La pregunta decide por unidades: la ley exige coeficientes (art. 45; riesgo de
    /// impugnación, art. 49). Muestre la advertencia.
    pub advertencia_unidades: bool,
}

impl From<Escrutinio> for EscrutinioDto {
    fn from(e: Escrutinio) -> Self {
        let Escrutinio {
            base_computo,
            regla,
            habilitados,
            total,
            votantes,
            sin_votar,
            opciones,
            decision,
            advertencia_unidades,
        } = e;
        Self {
            base_computo,
            regla: regla.into(),
            habilitados: habilitados.into(),
            total: total.into(),
            votantes: votantes.into(),
            sin_votar: sin_votar.into(),
            opciones: opciones.into_iter().map(Into::into).collect(),
            decision: decision.into(),
            advertencia_unidades,
        }
    }
}

/// Una foto del quórum en el acta.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = QuorumActa)]
pub struct QuorumActaDto {
    /// Momento de la foto (instalación, antes de una votación…).
    pub momento: String,
    /// Hora local ya formateada.
    pub hora: String,
    #[schema(value_type = String, example = "0.55")]
    pub coeficiente: Decimal,
    #[schema(value_type = String, example = "55.0000")]
    pub porcentaje_coeficiente: Decimal,
    pub unidades: u32,
    pub unidades_total: u32,
    pub asistentes: u32,
}

impl From<QuorumActa> for QuorumActaDto {
    fn from(q: QuorumActa) -> Self {
        let QuorumActa {
            momento,
            hora,
            coeficiente,
            porcentaje_coeficiente,
            unidades,
            unidades_total,
            asistentes,
        } = q;
        Self {
            momento,
            hora,
            coeficiente,
            porcentaje_coeficiente,
            unidades,
            unidades_total,
            asistentes,
        }
    }
}

/// Asistente en el acta.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = AsistenteActa)]
pub struct AsistenteActaDto {
    /// Dato personal: nombre del asistente.
    pub nombre: String,
    /// Códigos de los bienes que representa.
    pub bienes: Vec<String>,
    #[schema(value_type = String, example = "0.0125")]
    pub coeficiente: Decimal,
    /// Bienes que representa por poder.
    pub poderes: Vec<String>,
    /// Hora de entrada ya formateada.
    pub entrada: String,
    /// Salidas con hora y motivo.
    pub salidas: Vec<String>,
}

impl From<AsistenteActa> for AsistenteActaDto {
    fn from(a: AsistenteActa) -> Self {
        let AsistenteActa {
            nombre,
            bienes,
            coeficiente,
            poderes,
            entrada,
            salidas,
        } = a;
        Self {
            nombre,
            bienes,
            coeficiente,
            poderes,
            entrada,
            salidas,
        }
    }
}

/// Una fila del detalle nominal (bien → opción).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = VotoActa)]
pub struct VotoActaDto {
    /// Código del bien privado.
    pub bien: String,
    #[schema(value_type = String, example = "0.0125")]
    pub coeficiente: Decimal,
    /// Texto de la opción elegida.
    pub opcion: String,
    /// `web`, `whatsapp`, `presencial` o `rotulo`.
    pub canal: String,
    pub hora: String,
    pub apoderado: bool,
}

impl From<VotoActa> for VotoActaDto {
    fn from(v: VotoActa) -> Self {
        let VotoActa {
            bien,
            coeficiente,
            opcion,
            canal,
            hora,
            apoderado,
        } = v;
        Self {
            bien,
            coeficiente,
            opcion,
            canal,
            hora,
            apoderado,
        }
    }
}

/// Pregunta en el acta.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = PreguntaActa)]
pub struct PreguntaActaDto {
    pub orden: u16,
    pub enunciado: String,
    /// Tipo de pregunta ya redactado para el acta.
    pub tipo: String,
    /// Visibilidad del voto ya redactada para el acta.
    pub tipo_voto: String,
    /// Hora de apertura (`null` si no se abrió).
    pub abierta: Option<String>,
    /// Hora de cierre (`null` si no se cerró).
    pub cerrada: Option<String>,
    /// `null` si la pregunta no llegó a votarse.
    pub escrutinio: Option<EscrutinioDto>,
    /// Decisión ya redactada para el acta.
    pub decision: String,
    /// Solo en votaciones nominales (en reservadas, solo totales).
    pub detalle: Vec<VotoActaDto>,
    /// Elegidos por plancha (elecciones).
    pub elegidos: Vec<String>,
}

impl From<PreguntaActa> for PreguntaActaDto {
    fn from(p: PreguntaActa) -> Self {
        let PreguntaActa {
            orden,
            enunciado,
            tipo,
            tipo_voto,
            abierta,
            cerrada,
            escrutinio,
            decision,
            detalle,
            elegidos,
        } = p;
        Self {
            orden,
            enunciado,
            tipo,
            tipo_voto,
            abierta,
            cerrada,
            escrutinio: escrutinio.map(Into::into),
            decision,
            detalle: detalle.into_iter().map(Into::into).collect(),
            elegidos,
        }
    }
}

/// Datos completos del acta, ya calculados y redactados (los mismos que van al PDF).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = DatosActa)]
pub struct DatosActaRespuesta {
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
    pub quorum: Vec<QuorumActaDto>,
    pub asistentes: Vec<AsistenteActaDto>,
    /// Códigos de los bienes en mora.
    pub morosos: Vec<String>,
    pub efecto_mora: String,
    pub preguntas: Vec<PreguntaActaDto>,
    /// Advertencias para el acta (decisiones por unidades, efecto de la mora…).
    pub advertencias: Vec<String>,
    /// Momento en que se armaron los datos, ya formateado.
    pub generada: String,
}

impl From<DatosActa> for DatosActaRespuesta {
    fn from(d: DatosActa) -> Self {
        let DatosActa {
            copropiedad,
            nit,
            titulo,
            tipo,
            convocatoria,
            modalidad,
            lugar,
            fecha,
            instalada,
            cerrada,
            orden_del_dia,
            quorum,
            asistentes,
            morosos,
            efecto_mora,
            preguntas,
            advertencias,
            generada,
        } = d;
        Self {
            copropiedad,
            nit,
            titulo,
            tipo,
            convocatoria,
            modalidad,
            lugar,
            fecha,
            instalada,
            cerrada,
            orden_del_dia,
            quorum: quorum.into_iter().map(Into::into).collect(),
            asistentes: asistentes.into_iter().map(Into::into).collect(),
            morosos,
            efecto_mora,
            preguntas: preguntas.into_iter().map(Into::into).collect(),
            advertencias,
            generada,
        }
    }
}

#[cfg(test)]
mod tests {
    use domain::{OpcionId, ReglaMayoria};

    use super::*;

    fn escrutinio(decision: Decision) -> Escrutinio {
        let conteo = Conteo::new(Decimal::new(6, 1), 6);
        Escrutinio {
            base_computo: BaseComputo::Coeficiente,
            regla: ReglaMayoria::Calificada70,
            habilitados: conteo,
            total: Conteo::new(Decimal::ONE, 10),
            votantes: conteo,
            sin_votar: Conteo::default(),
            opciones: vec![],
            decision,
            advertencia_unidades: false,
        }
    }

    #[test]
    fn la_regla_calificada_del_escrutinio_usa_el_codigo_de_la_pregunta() {
        let json = serde_json::to_value(EscrutinioDto::from(escrutinio(Decision::Aprobada)))
            .expect("serializa");
        assert_eq!(json["regla"]["regla"], "calificada_70");
        assert_eq!(json["decision"]["decision"], "aprobada");
        assert_eq!(json["habilitados"]["coeficiente"], "0.6");
    }

    #[test]
    fn el_reparto_lleva_sus_campos_junto_a_la_etiqueta() {
        let opcion = OpcionId::from_uuid(Uuid::now_v7());
        let reparto = Reparto {
            cociente: Decimal::new(2, 1),
            cupos: vec![CuposPlancha {
                opcion_id: opcion,
                por_cociente: 2,
                por_residuo: 1,
                residuo: Decimal::new(1, 1),
            }],
            empate: false,
            sin_asignar: 0,
        };
        let json =
            serde_json::to_value(EscrutinioDto::from(escrutinio(Decision::Reparto(reparto))))
                .expect("serializa");
        assert_eq!(json["decision"]["decision"], "reparto");
        assert_eq!(json["decision"]["cociente"], "0.2");
        assert_eq!(
            json["decision"]["cupos"][0]["opcion_id"],
            opcion.to_string()
        );
    }

    #[test]
    fn la_decision_del_espejo_coincide_con_la_del_dominio() {
        let ganadora = Decision::Ganadora {
            opcion_id: OpcionId::from_uuid(Uuid::now_v7()),
        };
        for d in [Decision::NoAprobada, Decision::PorOpcion, ganadora] {
            let dominio = serde_json::to_value(&d).expect("serializa");
            let espejo = serde_json::to_value(DecisionDto::from(d)).expect("serializa");
            assert_eq!(dominio, espejo);
        }
    }
}
