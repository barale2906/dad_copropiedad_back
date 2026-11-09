//! Emisión de votos: separa **quién** votó por cada bien (participación) de **qué** votó (voto).
//!
//! El asistente vota **una vez por todos los bienes que representa** (decisión del 2026-11-08):
//! una emisión produce una participación por bien, todas con el coeficiente copiado y la misma
//! clave de idempotencia, y un voto por cada (bien, opción elegida).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    AsistenteId, BienPrivadoId, CalidadRepresentacion, Coeficiente, CuentaId, DominioError,
    OpcionId, ParticipacionId, PersonaId, PreguntaAbierta, PreguntaId, TipoPregunta, VotoId,
};

/// Canal por el que llegó el voto (queda en el acta).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Canal {
    /// Papeleta virtual (enlace personal) o la web.
    Web,
    Whatsapp,
    /// Registrado por un operador en la mesa (asistido, mano alzada).
    Presencial,
    /// Rótulo escaneado en el puesto de la opción («urna»).
    Rotulo,
}

enum_texto!(Canal {
    Web => "web",
    Whatsapp => "whatsapp",
    Presencial => "presencial",
    Rotulo => "rotulo",
});

/// Clave de idempotencia de una emisión. En WhatsApp es el `wamid` del mensaje; en web la
/// envía el cliente (cabecera `Idempotency-Key`); en los puestos de escaneo, una por lectura.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ClaveIdempotencia(String);

impl ClaveIdempotencia {
    pub const LONGITUD_MAXIMA: usize = 200;

    /// # Errors
    ///
    /// [`DominioError::ClaveIdempotenciaInvalida`] si está vacía o es muy larga.
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

/// Bien privado por el que vota el asistente, con su coeficiente del padrón.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BienRepresentado {
    pub bien_privado_id: BienPrivadoId,
    /// Coeficiente del padrón; se **copia** en la participación y en los votos.
    pub coeficiente: Coeficiente,
    pub calidad: CalidadRepresentacion,
}

/// Lo que entrega el asistente: sus bienes habilitados que aún no votaron, las opciones y los
/// metadatos para el acta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Papeleta {
    pub asistente_id: AsistenteId,
    /// Persona del asistente (propietario o apoderado).
    pub persona_id: PersonaId,
    pub bienes: Vec<BienRepresentado>,
    pub opciones: Vec<OpcionId>,
    pub canal: Canal,
    /// Cuenta del operador o del puesto que registró el voto, si no lo emitió la persona.
    pub registrado_por: Option<CuentaId>,
    pub idempotency_key: ClaveIdempotencia,
    /// Id del mensaje de WhatsApp (solo en ese canal).
    pub wamid: Option<String>,
}

/// Fila a insertar en `participaciones`: **quién** votó por el bien, cuándo, por qué canal y
/// con qué peso.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipacionNueva {
    pub id: ParticipacionId,
    pub pregunta_id: PreguntaId,
    pub bien_privado_id: BienPrivadoId,
    pub asistente_id: AsistenteId,
    pub coeficiente: Coeficiente,
    pub canal: Canal,
    pub emitido_por: PersonaId,
    /// `Some` cuando representa el bien por poder.
    pub apoderado_id: Option<PersonaId>,
    pub registrado_por: Option<CuentaId>,
    pub idempotency_key: ClaveIdempotencia,
    pub wamid: Option<String>,
    pub registrado_en: DateTime<Utc>,
}

/// Fila a insertar en `votos`: **qué** se votó por el bien y con qué peso. Sin marca de tiempo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VotoNuevo {
    /// UUID v4 (ver [`VotoId`]).
    pub id: VotoId,
    pub pregunta_id: PreguntaId,
    pub opcion_id: OpcionId,
    pub coeficiente: Coeficiente,
    pub bien_privado_id: BienPrivadoId,
    pub participacion_id: ParticipacionId,
}

/// Resultado de validar una papeleta: todo se inserta en **una sola transacción**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VotoEmitido {
    pub participaciones: Vec<ParticipacionNueva>,
    pub votos: Vec<VotoNuevo>,
}

impl PreguntaAbierta<'_> {
    /// Valida la papeleta contra la pregunta abierta y construye las filas a persistir.
    ///
    /// # Errors
    ///
    /// [`DominioError::OpcionInvalida`] si una opción no es de la pregunta;
    /// [`DominioError::VotoInvalido`] si el número de opciones no corresponde al tipo, se
    /// repiten opciones o bienes, no hay bienes o el `wamid` no corresponde al canal.
    pub fn emitir(
        &self,
        papeleta: Papeleta,
        ahora: DateTime<Utc>,
    ) -> Result<VotoEmitido, DominioError> {
        let pregunta = self.pregunta();
        if let Some(o) = papeleta
            .opciones
            .iter()
            .find(|o| pregunta.opcion(**o).is_none())
        {
            return Err(DominioError::OpcionInvalida(*o));
        }
        let mut opciones = papeleta.opciones.clone();
        opciones.sort();
        opciones.dedup();
        if opciones.len() != papeleta.opciones.len() {
            return Err(DominioError::VotoInvalido("una opción se eligió dos veces"));
        }
        let maximo = match pregunta.tipo {
            TipoPregunta::SeleccionMultiple => usize::from(pregunta.max_selecciones),
            TipoPregunta::SiNo | TipoPregunta::SeleccionUnica | TipoPregunta::Eleccion => 1,
        };
        if opciones.is_empty() || opciones.len() > maximo {
            return Err(DominioError::VotoInvalido(
                "el número de opciones elegidas no corresponde a la pregunta",
            ));
        }
        if papeleta.opciones.len() > 1
            && papeleta
                .opciones
                .iter()
                .any(|o| pregunta.opcion(*o).is_some_and(|x| x.en_blanco))
        {
            return Err(DominioError::VotoInvalido("el voto en blanco va solo"));
        }
        if papeleta.bienes.is_empty() {
            return Err(DominioError::VotoInvalido(
                "el asistente no tiene bienes privados habilitados para votar",
            ));
        }
        let mut bienes: Vec<_> = papeleta.bienes.iter().map(|b| b.bien_privado_id).collect();
        bienes.sort();
        bienes.dedup();
        if bienes.len() != papeleta.bienes.len() {
            return Err(DominioError::VotoInvalido("un bien privado está repetido"));
        }
        if (papeleta.canal == Canal::Whatsapp) != papeleta.wamid.is_some() {
            return Err(DominioError::VotoInvalido(
                "el wamid solo aplica al canal WhatsApp",
            ));
        }

        let mut participaciones = Vec::with_capacity(papeleta.bienes.len());
        let mut votos = Vec::with_capacity(papeleta.bienes.len() * papeleta.opciones.len());
        for bien in &papeleta.bienes {
            let participacion = ParticipacionNueva {
                id: ParticipacionId::nuevo(),
                pregunta_id: pregunta.id,
                bien_privado_id: bien.bien_privado_id,
                asistente_id: papeleta.asistente_id,
                coeficiente: bien.coeficiente,
                canal: papeleta.canal,
                emitido_por: papeleta.persona_id,
                apoderado_id: (bien.calidad == CalidadRepresentacion::Apoderado)
                    .then_some(papeleta.persona_id),
                registrado_por: papeleta.registrado_por,
                idempotency_key: papeleta.idempotency_key.clone(),
                wamid: papeleta.wamid.clone(),
                registrado_en: ahora,
            };
            votos.extend(papeleta.opciones.iter().map(|opcion_id| VotoNuevo {
                id: VotoId::nuevo(),
                pregunta_id: pregunta.id,
                opcion_id: *opcion_id,
                coeficiente: bien.coeficiente,
                bien_privado_id: bien.bien_privado_id,
                participacion_id: participacion.id,
            }));
            participaciones.push(participacion);
        }
        Ok(VotoEmitido {
            participaciones,
            votos,
        })
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::dec;

    use super::*;
    use crate::{EstadoPregunta, Pregunta, TEXTOS_SI_NO, pregunta::tests::pregunta};

    fn bien(coef: rust_decimal::Decimal, calidad: CalidadRepresentacion) -> BienRepresentado {
        BienRepresentado {
            bien_privado_id: BienPrivadoId::nuevo(),
            coeficiente: Coeficiente::new(coef).unwrap(),
            calidad,
        }
    }

    fn papeleta(p: &Pregunta, opciones: Vec<OpcionId>, bienes: Vec<BienRepresentado>) -> Papeleta {
        let _ = p;
        Papeleta {
            asistente_id: AsistenteId::nuevo(),
            persona_id: PersonaId::nuevo(),
            bienes,
            opciones,
            canal: Canal::Whatsapp,
            registrado_por: None,
            idempotency_key: ClaveIdempotencia::new("wamid.HBgM").unwrap(),
            wamid: Some("wamid.HBgM".into()),
        }
    }

    #[test]
    fn un_voto_del_apoderado_cubre_todos_sus_bienes() {
        let p = pregunta(TipoPregunta::SiNo, &TEXTOS_SI_NO, EstadoPregunta::Abierta);
        let bienes = vec![
            bien(dec!(0.01), CalidadRepresentacion::Propietario),
            bien(dec!(0.02), CalidadRepresentacion::Apoderado),
            bien(dec!(0.03), CalidadRepresentacion::Apoderado),
        ];
        let pap = papeleta(&p, vec![p.opciones[0].id], bienes.clone());
        let persona = pap.persona_id;
        let e = p.para_votar().unwrap().emitir(pap, Utc::now()).unwrap();

        assert_eq!(e.participaciones.len(), 3);
        assert_eq!(e.votos.len(), 3);
        for (i, b) in bienes.iter().enumerate() {
            assert_eq!(e.participaciones[i].coeficiente, b.coeficiente);
            assert_eq!(e.votos[i].participacion_id, e.participaciones[i].id);
            assert_eq!(e.votos[i].bien_privado_id, b.bien_privado_id);
        }
        assert_eq!(e.participaciones[0].apoderado_id, None);
        assert_eq!(e.participaciones[1].apoderado_id, Some(persona));
        assert!(
            e.votos
                .iter()
                .all(|v| v.id.as_uuid().get_version_num() == 4)
        );
    }

    #[test]
    fn seleccion_multiple_un_voto_por_opcion() {
        let mut p = pregunta(
            TipoPregunta::SeleccionMultiple,
            &["A", "B", "C"],
            EstadoPregunta::Abierta,
        );
        p.max_selecciones = 2;
        let ops = vec![p.opciones[0].id, p.opciones[2].id];
        let bienes = vec![bien(dec!(0.01), CalidadRepresentacion::Propietario); 1];
        let e = p
            .para_votar()
            .unwrap()
            .emitir(papeleta(&p, ops.clone(), bienes.clone()), Utc::now())
            .unwrap();
        assert_eq!(e.votos.len(), 2);

        let tres = vec![p.opciones[0].id, p.opciones[1].id, p.opciones[2].id];
        let err = p
            .para_votar()
            .unwrap()
            .emitir(papeleta(&p, tres, bienes), Utc::now());
        assert!(matches!(err, Err(DominioError::VotoInvalido(_))));
    }

    #[test]
    fn rechaza_opcion_ajena_repetida_o_vacia() {
        let p = pregunta(TipoPregunta::SiNo, &TEXTOS_SI_NO, EstadoPregunta::Abierta);
        let b = vec![bien(dec!(0.01), CalidadRepresentacion::Propietario)];
        let abierta = p.para_votar().unwrap();
        assert!(matches!(
            abierta.emitir(papeleta(&p, vec![OpcionId::nuevo()], b.clone()), Utc::now()),
            Err(DominioError::OpcionInvalida(_))
        ));
        assert!(
            abierta
                .emitir(papeleta(&p, vec![], b.clone()), Utc::now())
                .is_err()
        );
        assert!(
            abierta
                .emitir(
                    papeleta(&p, vec![p.opciones[0].id, p.opciones[1].id], b),
                    Utc::now()
                )
                .is_err()
        );
        assert!(
            abierta
                .emitir(papeleta(&p, vec![p.opciones[0].id], vec![]), Utc::now())
                .is_err()
        );
    }

    #[test]
    fn rechaza_bienes_repetidos_y_wamid_fuera_de_whatsapp() {
        let p = pregunta(TipoPregunta::SiNo, &TEXTOS_SI_NO, EstadoPregunta::Abierta);
        let b = bien(dec!(0.01), CalidadRepresentacion::Propietario);
        let abierta = p.para_votar().unwrap();
        assert!(
            abierta
                .emitir(papeleta(&p, vec![p.opciones[0].id], vec![b, b]), Utc::now())
                .is_err()
        );
        let mut web = papeleta(&p, vec![p.opciones[0].id], vec![b]);
        web.canal = Canal::Web;
        assert!(abierta.emitir(web.clone(), Utc::now()).is_err());
        web.wamid = None;
        assert!(abierta.emitir(web, Utc::now()).is_ok());
    }

    #[test]
    fn pregunta_cerrada_no_emite() {
        let p = pregunta(TipoPregunta::SiNo, &TEXTOS_SI_NO, EstadoPregunta::Cerrada);
        assert!(p.para_votar().is_err());
    }

    #[test]
    fn clave_idempotencia_valida_longitud() {
        assert!(ClaveIdempotencia::new("  ").is_err());
        assert!(ClaveIdempotencia::new("x".repeat(201)).is_err());
        assert!(ClaveIdempotencia::new("0193c6a2-...").is_ok());
    }
}
