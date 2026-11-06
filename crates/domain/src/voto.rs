//! Emisión de votos: separa **quién** votó (participación) de **qué** votó (voto).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    BienPrivadoId, Coeficiente, DominioError, OpcionId, ParticipacionId, PersonaId,
    PreguntaAbierta, PreguntaId, TipoVoto, VotoId,
};

/// Canal por el que llegó el voto (queda en el acta).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Canal {
    Web,
    Whatsapp,
    /// Registrado por un operador en la mesa (voto en papel / mano alzada).
    Presencial,
}

enum_texto!(Canal {
    Web => "web",
    Whatsapp => "whatsapp",
    Presencial => "presencial",
});

/// Clave de idempotencia de una emisión. En WhatsApp es el `wamid` del mensaje; en web la
/// envía el cliente (cabecera `Idempotency-Key`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ClaveIdempotencia(String);

impl ClaveIdempotencia {
    pub const LONGITUD_MAXIMA: usize = 200;

    pub fn new(valor: impl Into<String>) -> Result<Self, DominioError> {
        let valor = valor.into();
        if valor.trim().is_empty() {
            return Err(DominioError::ClaveIdempotenciaInvalida("vacía"));
        }
        if valor.len() > Self::LONGITUD_MAXIMA {
            return Err(DominioError::ClaveIdempotenciaInvalida("demasiado larga"));
        }
        Ok(Self(valor))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ClaveIdempotencia {
    type Error = DominioError;

    fn try_from(valor: String) -> Result<Self, Self::Error> {
        Self::new(valor)
    }
}

impl From<ClaveIdempotencia> for String {
    fn from(c: ClaveIdempotencia) -> Self {
        c.0
    }
}

/// Lo que el votante entrega: bien privado, opción y metadatos para el acta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Papeleta {
    pub bien_privado_id: BienPrivadoId,
    /// Coeficiente vigente del bien privado; se **copia** en la fila al momento de votar.
    pub coeficiente: Coeficiente,
    pub opcion_id: OpcionId,
    pub canal: Canal,
    /// Persona que emite el voto (propietario o apoderado).
    pub emitido_por: PersonaId,
    /// `Some` cuando quien vota es un apoderado.
    pub apoderado_id: Option<PersonaId>,
    pub idempotency_key: ClaveIdempotencia,
    /// Id del mensaje de WhatsApp (Meta reintenta webhooks: `UNIQUE (wamid)`).
    pub wamid: Option<String>,
}

/// Fila a insertar en `participaciones`: **quién** votó, cuándo, por qué canal y con qué peso.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipacionNueva {
    pub id: ParticipacionId,
    pub pregunta_id: PreguntaId,
    pub bien_privado_id: BienPrivadoId,
    pub coeficiente: Coeficiente,
    pub canal: Canal,
    pub emitido_por: PersonaId,
    pub apoderado_id: Option<PersonaId>,
    pub idempotency_key: ClaveIdempotencia,
    pub wamid: Option<String>,
    pub registrado_en: DateTime<Utc>,
}

/// Vínculo voto ↔ bien privado. Solo existe en votaciones nominales.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VinculoNominal {
    pub bien_privado_id: BienPrivadoId,
    pub participacion_id: ParticipacionId,
}

/// Fila a insertar en `votos`: **qué** se votó y con qué peso. Sin marca de tiempo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VotoNuevo {
    /// UUID v4 (ver [`VotoId`]).
    pub id: VotoId,
    pub pregunta_id: PreguntaId,
    pub opcion_id: OpcionId,
    pub coeficiente: Coeficiente,
    /// `None` en votaciones secretas: nada enlaza el voto con el bien privado.
    pub vinculo: Option<VinculoNominal>,
}

/// Resultado de validar una papeleta: ambas filas se insertan en **una sola transacción**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VotoEmitido {
    pub participacion: ParticipacionNueva,
    pub voto: VotoNuevo,
}

impl PreguntaAbierta<'_> {
    /// Valida la papeleta contra la pregunta abierta y construye las filas a persistir.
    pub fn emitir(
        &self,
        papeleta: Papeleta,
        ahora: DateTime<Utc>,
    ) -> Result<VotoEmitido, DominioError> {
        let pregunta = self.pregunta();
        if !pregunta.opciones.contains(&papeleta.opcion_id) {
            return Err(DominioError::OpcionInvalida(papeleta.opcion_id));
        }

        let participacion = ParticipacionNueva {
            id: ParticipacionId::nuevo(),
            pregunta_id: pregunta.id,
            bien_privado_id: papeleta.bien_privado_id,
            coeficiente: papeleta.coeficiente,
            canal: papeleta.canal,
            emitido_por: papeleta.emitido_por,
            apoderado_id: papeleta.apoderado_id,
            idempotency_key: papeleta.idempotency_key,
            wamid: papeleta.wamid,
            registrado_en: ahora,
        };

        let vinculo = match pregunta.tipo_voto {
            TipoVoto::Nominal => Some(VinculoNominal {
                bien_privado_id: participacion.bien_privado_id,
                participacion_id: participacion.id,
            }),
            TipoVoto::Secreto => None,
        };

        let voto = VotoNuevo {
            id: VotoId::nuevo(),
            pregunta_id: pregunta.id,
            opcion_id: papeleta.opcion_id,
            coeficiente: papeleta.coeficiente,
            vinculo,
        };

        Ok(VotoEmitido {
            participacion,
            voto,
        })
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;
    use crate::{EstadoPregunta, Pregunta, ReglaMayoria, ReunionId};

    fn pregunta(tipo_voto: TipoVoto, estado: EstadoPregunta) -> Pregunta {
        Pregunta {
            id: PreguntaId::nuevo(),
            reunion_id: ReunionId::nuevo(),
            enunciado: "¿Aprueba el presupuesto 2027?".into(),
            tipo_voto,
            regla_mayoria: ReglaMayoria::Simple,
            estado,
            opciones: vec![OpcionId::nuevo(), OpcionId::nuevo()],
        }
    }

    fn papeleta(opcion_id: OpcionId) -> Papeleta {
        Papeleta {
            bien_privado_id: BienPrivadoId::nuevo(),
            coeficiente: Coeficiente::new(dec!(0.0123)).unwrap(),
            opcion_id,
            canal: Canal::Whatsapp,
            emitido_por: PersonaId::nuevo(),
            apoderado_id: None,
            idempotency_key: ClaveIdempotencia::new("wamid.HBgM").unwrap(),
            wamid: Some("wamid.HBgM".into()),
        }
    }

    #[test]
    fn nominal_vincula_voto_con_bien_privado() {
        let p = pregunta(TipoVoto::Nominal, EstadoPregunta::Abierta);
        let emitido = p
            .para_votar()
            .unwrap()
            .emitir(papeleta(p.opciones[0]), Utc::now())
            .unwrap();
        let vinculo = emitido.voto.vinculo.expect("nominal debe tener vínculo");
        assert_eq!(vinculo.participacion_id, emitido.participacion.id);
        assert_eq!(
            vinculo.bien_privado_id,
            emitido.participacion.bien_privado_id
        );
    }

    #[test]
    fn secreto_no_deja_rastro_del_bien_privado() {
        let p = pregunta(TipoVoto::Secreto, EstadoPregunta::Abierta);
        let emitido = p
            .para_votar()
            .unwrap()
            .emitir(papeleta(p.opciones[1]), Utc::now())
            .unwrap();
        assert!(emitido.voto.vinculo.is_none());
        assert_eq!(emitido.voto.id.as_uuid().get_version_num(), 4);
    }

    #[test]
    fn rechaza_opcion_ajena_y_pregunta_cerrada() {
        let p = pregunta(TipoVoto::Nominal, EstadoPregunta::Abierta);
        let err = p
            .para_votar()
            .unwrap()
            .emitir(papeleta(OpcionId::nuevo()), Utc::now())
            .unwrap_err();
        assert!(matches!(err, DominioError::OpcionInvalida(_)));

        let cerrada = pregunta(TipoVoto::Nominal, EstadoPregunta::Cerrada);
        assert!(cerrada.para_votar().is_err());
    }

    #[test]
    fn clave_idempotencia_valida_longitud() {
        assert!(ClaveIdempotencia::new("  ").is_err());
        assert!(ClaveIdempotencia::new("x".repeat(201)).is_err());
        assert!(ClaveIdempotencia::new("0193c6a2-...").is_ok());
    }
}
