//! Caso de uso: emitir un voto (web, WhatsApp o presencial).

use domain::{
    BienPrivadoId, Canal, ClaveIdempotencia, OpcionId, Papeleta, PersonaId, PreguntaId,
    RolAsistente,
};

use crate::{
    AppError, AppResult, Contexto,
    outbox::{EventoSaliente, PrioridadSalida},
    puertos::{
        Reloj, RepositorioInscripciones, RepositorioPreguntas, RepositorioVotos,
        mensajeria::{ContenidoSaliente, MensajeSaliente},
        repositorios::ResultadoRegistro,
    },
};

/// Petición de voto ya autenticada (quién es `emitido_por` lo resolvió el adaptador de entrada).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolicitudVoto {
    pub pregunta_id: PreguntaId,
    pub bien_privado_id: BienPrivadoId,
    pub opcion_id: OpcionId,
    pub emitido_por: PersonaId,
    pub canal: Canal,
    pub idempotency_key: ClaveIdempotencia,
    pub wamid: Option<String>,
    /// `(linea, telefono)` a quien confirmar por WhatsApp (vía outbox), si aplica.
    pub confirmar_a: Option<(String, String)>,
}

pub struct EmitirVoto<'a, P, I, V, R> {
    pub preguntas: &'a P,
    pub inscripciones: &'a I,
    pub votos: &'a V,
    pub reloj: &'a R,
}

impl<P, I, V, R> EmitirVoto<'_, P, I, V, R>
where
    P: RepositorioPreguntas,
    I: RepositorioInscripciones,
    V: RepositorioVotos,
    R: Reloj,
{
    pub async fn ejecutar(
        &self,
        ctx: &Contexto,
        solicitud: SolicitudVoto,
    ) -> AppResult<ResultadoRegistro> {
        let pregunta = self
            .preguntas
            .obtener(ctx, solicitud.pregunta_id)
            .await?
            .ok_or(AppError::NoEncontrado("pregunta"))?;
        let abierta = pregunta.para_votar()?;

        let inscripcion = self
            .inscripciones
            .vigente(
                ctx,
                pregunta.reunion_id,
                solicitud.bien_privado_id,
                solicitud.emitido_por,
            )
            .await?
            .ok_or(AppError::NoInscrito)?;

        let papeleta = Papeleta {
            bien_privado_id: solicitud.bien_privado_id,
            coeficiente: inscripcion.coeficiente,
            opcion_id: solicitud.opcion_id,
            canal: solicitud.canal,
            emitido_por: solicitud.emitido_por,
            apoderado_id: (inscripcion.rol == RolAsistente::Apoderado)
                .then_some(solicitud.emitido_por),
            idempotency_key: solicitud.idempotency_key,
            wamid: solicitud.wamid,
        };
        let emitido = abierta.emitir(papeleta, self.reloj.ahora())?;

        // Secreto del voto: la confirmación NO menciona la opción elegida.
        let confirmacion = solicitud
            .confirmar_a
            .map(|(linea, telefono)| EventoSaliente {
                prioridad: PrioridadSalida::ConfirmacionVoto,
                mensaje: MensajeSaliente {
                    linea,
                    telefono,
                    contenido: ContenidoSaliente::Texto {
                        cuerpo: format!("✅ Su voto fue registrado: «{}».", pregunta.enunciado),
                    },
                },
            });

        let resultado = self.votos.registrar(ctx, emitido, confirmacion).await?;
        tracing::info!(
            tenant_id = %ctx.tenant_id,
            reunion_id = %pregunta.reunion_id,
            pregunta_id = %pregunta.id,
            canal = %solicitud.canal,
            reintento = matches!(resultado, ResultadoRegistro::Reintento(_)),
            "voto procesado"
        );
        Ok(resultado)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::{DateTime, Utc};
    use domain::{
        Coeficiente, DominioError, EstadoPregunta, Pregunta, ReglaMayoria, ReunionId, TenantId,
        TipoVoto, VotoEmitido,
    };
    use rust_decimal::dec;

    use super::*;
    use crate::puertos::repositorios::InscripcionVigente;

    struct Fijo;
    impl Reloj for Fijo {
        fn ahora(&self) -> DateTime<Utc> {
            DateTime::UNIX_EPOCH
        }
    }

    struct Preguntas(Pregunta);
    impl RepositorioPreguntas for Preguntas {
        async fn obtener(&self, _: &Contexto, id: PreguntaId) -> AppResult<Option<Pregunta>> {
            Ok((self.0.id == id).then(|| self.0.clone()))
        }
    }

    struct Inscripciones(Option<InscripcionVigente>);
    impl RepositorioInscripciones for Inscripciones {
        async fn vigente(
            &self,
            _: &Contexto,
            _: ReunionId,
            bien_privado: BienPrivadoId,
            persona: PersonaId,
        ) -> AppResult<Option<InscripcionVigente>> {
            Ok(self
                .0
                .clone()
                .filter(|i| i.bien_privado_id == bien_privado && i.persona_id == persona))
        }
    }

    /// Doble en memoria que reproduce las restricciones UNIQUE de la base de datos.
    #[derive(Default)]
    struct Votos(Mutex<Vec<(VotoEmitido, Option<EventoSaliente>)>>);
    impl RepositorioVotos for Votos {
        async fn registrar(
            &self,
            _: &Contexto,
            voto: VotoEmitido,
            confirmacion: Option<EventoSaliente>,
        ) -> AppResult<ResultadoRegistro> {
            let mut filas = self.0.lock().unwrap();
            let p = &voto.participacion;
            if let Some((v, _)) = filas.iter().find(|(v, _)| {
                v.participacion.idempotency_key == p.idempotency_key
                    || (p.wamid.is_some() && v.participacion.wamid == p.wamid)
            }) {
                return Ok(ResultadoRegistro::Reintento(v.participacion.id));
            }
            if filas
                .iter()
                .any(|(v, _)| v.participacion.bien_privado_id == p.bien_privado_id)
            {
                return Err(AppError::BienPrivadoYaVoto);
            }
            let id = p.id;
            filas.push((voto, confirmacion));
            Ok(ResultadoRegistro::Registrado(id))
        }
    }

    struct Escenario {
        pregunta: Pregunta,
        inscripcion: InscripcionVigente,
    }

    fn escenario(tipo_voto: TipoVoto, estado: EstadoPregunta, rol: RolAsistente) -> Escenario {
        Escenario {
            pregunta: Pregunta {
                id: PreguntaId::nuevo(),
                reunion_id: ReunionId::nuevo(),
                enunciado: "Presupuesto 2027".into(),
                tipo_voto,
                regla_mayoria: ReglaMayoria::Simple,
                estado,
                opciones: vec![OpcionId::nuevo(), OpcionId::nuevo()],
            },
            inscripcion: InscripcionVigente {
                bien_privado_id: BienPrivadoId::nuevo(),
                persona_id: PersonaId::nuevo(),
                rol,
                coeficiente: Coeficiente::new(dec!(0.0125)).unwrap(),
            },
        }
    }

    fn solicitud(e: &Escenario, clave: &str) -> SolicitudVoto {
        SolicitudVoto {
            pregunta_id: e.pregunta.id,
            bien_privado_id: e.inscripcion.bien_privado_id,
            opcion_id: e.pregunta.opciones[0],
            emitido_por: e.inscripcion.persona_id,
            canal: Canal::Whatsapp,
            idempotency_key: ClaveIdempotencia::new(clave).unwrap(),
            wamid: Some(clave.to_owned()),
            confirmar_a: Some(("linea-1".into(), "573001112233".into())),
        }
    }

    async fn ejecutar(
        e: &Escenario,
        votos: &Votos,
        s: SolicitudVoto,
    ) -> AppResult<ResultadoRegistro> {
        let preguntas = Preguntas(e.pregunta.clone());
        let inscripciones = Inscripciones(Some(e.inscripcion.clone()));
        EmitirVoto {
            preguntas: &preguntas,
            inscripciones: &inscripciones,
            votos,
            reloj: &Fijo,
        }
        .ejecutar(&Contexto::new(TenantId::nuevo()), s)
        .await
    }

    #[tokio::test]
    async fn registra_y_es_idempotente() {
        let e = escenario(
            TipoVoto::Nominal,
            EstadoPregunta::Abierta,
            RolAsistente::Propietario,
        );
        let votos = Votos::default();

        let primero = ejecutar(&e, &votos, solicitud(&e, "wamid.A"))
            .await
            .unwrap();
        let reintento = ejecutar(&e, &votos, solicitud(&e, "wamid.A"))
            .await
            .unwrap();

        let ResultadoRegistro::Registrado(id) = primero else {
            panic!("se esperaba Registrado");
        };
        assert_eq!(reintento, ResultadoRegistro::Reintento(id));
        assert_eq!(votos.0.lock().unwrap().len(), 1);

        // Otro mensaje del mismo bien privado: rechazado.
        let err = ejecutar(&e, &votos, solicitud(&e, "wamid.B"))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BienPrivadoYaVoto));
    }

    #[tokio::test]
    async fn secreto_y_apoderado() {
        let e = escenario(
            TipoVoto::Secreto,
            EstadoPregunta::Abierta,
            RolAsistente::Apoderado,
        );
        let votos = Votos::default();
        ejecutar(&e, &votos, solicitud(&e, "wamid.C"))
            .await
            .unwrap();

        let filas = votos.0.lock().unwrap();
        let (emitido, confirmacion) = &filas[0];
        assert!(emitido.voto.vinculo.is_none());
        assert_eq!(
            emitido.participacion.apoderado_id,
            Some(e.inscripcion.persona_id)
        );
        assert_eq!(
            confirmacion.as_ref().map(|c| c.prioridad),
            Some(PrioridadSalida::ConfirmacionVoto)
        );
    }

    #[tokio::test]
    async fn rechaza_pregunta_cerrada_y_no_inscrito() {
        let e = escenario(
            TipoVoto::Nominal,
            EstadoPregunta::Cerrada,
            RolAsistente::Propietario,
        );
        let err = ejecutar(&e, &Votos::default(), solicitud(&e, "k"))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            AppError::Dominio(DominioError::PreguntaNoAbierta(EstadoPregunta::Cerrada))
        ));

        let e = escenario(
            TipoVoto::Nominal,
            EstadoPregunta::Abierta,
            RolAsistente::Propietario,
        );
        let mut s = solicitud(&e, "k");
        s.emitido_por = PersonaId::nuevo();
        let err = ejecutar(&e, &Votos::default(), s).await.unwrap_err();
        assert!(matches!(err, AppError::NoInscrito));
    }
}
