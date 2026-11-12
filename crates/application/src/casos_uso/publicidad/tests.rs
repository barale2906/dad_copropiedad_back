//! Pruebas de la entrega de publicidad en una copropiedad con dobles en memoria: solo a quien
//! autorizó, topes de presupuesto, frecuencia de los masivos, cupones y pantalla. El portal, la
//! moderación y la liquidación (plataforma) se prueban contra PostgreSQL en
//! `infra-postgres/tests/publicidad.rs`.

use chrono::{DateTime, Duration, TimeZone, Utc};
use domain::{
    CampanaId, CanalPublicidad, ConjuntoPermisos, CreativoId, CuentaId, EstadoCampana,
    FormatoCreativo, MedioAutorizacion, ModeloCobro, Modulo, Permiso, PersonaId,
    PoliticaPublicidadId, TenantId, TipoPersona,
    publicidad::{CodigoCupon, ContenidoCreativo, Cupon, PiezaCandidata, Presupuesto},
};
use rust_decimal::{Decimal, dec};

use super::entrega::*;
use crate::{
    AppError, Contexto,
    outbox::{MensajeOutbox, PrioridadSalida},
    puertos::{
        DirectorioPublicidad,
        copropiedades::Persona,
        mensajeria::ContenidoSaliente,
        publicidad::{DestinatarioMasivo, PiezaPublicidad, PoliticaPublicidad},
        reloj::RelojFijo,
        unidad_trabajo::memoria::UnidadEnMemoria,
    },
};

fn ahora() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 11, 15, 0, 0)
        .single()
        .expect("fecha")
}

fn reloj() -> RelojFijo {
    RelojFijo(ahora())
}

fn pieza(
    formato: FormatoCreativo,
    modelo: ModeloCobro,
    tarifa: Decimal,
    total: Decimal,
) -> PiezaPublicidad {
    PiezaPublicidad {
        candidata: PiezaCandidata {
            creativo: CreativoId::nuevo(),
            campana: CampanaId::nuevo(),
            anunciante_verificado: true,
            creativo_aprobado: true,
            estado_campana: EstadoCampana::Activa,
            inicio: ahora() - Duration::days(1),
            fin: ahora() + Duration::days(10),
            modelo,
            tarifa,
            presupuesto: Presupuesto::new(total, None).expect("válido"),
            consumido_total: Decimal::ZERO,
            consumido_hoy: Decimal::ZERO,
            impresiones_hoy: 0,
        },
        anunciante_id: domain::AnuncianteId::nuevo(),
        nombre_comercial: "Modistería Rosa".into(),
        contenido: ContenidoCreativo {
            formato,
            titulo: "2x1 en arreglos".into(),
            texto: "Toda la semana".into(),
            boton: None,
            enlace: None,
            condiciones: Some("Hasta el domingo".into()),
            es_promocion: true,
            cupon: (formato == FormatoCreativo::OfertaCupon).then(|| Cupon {
                codigo: CodigoCupon::new("ROSA10").expect("válido"),
                limite_usos: None,
            }),
            plantilla_whatsapp: (formato == FormatoCreativo::MensajeWhatsapp)
                .then(|| "oferta_semanal".to_owned()),
        },
    }
}

struct Entorno {
    unidad: UnidadEnMemoria,
    tenant: TenantId,
    cuenta: CuentaId,
    persona: PersonaId,
}

fn entorno(piezas: Vec<PiezaPublicidad>) -> Entorno {
    let unidad = UnidadEnMemoria::default();
    unidad.habilitar(Modulo::Publicidad);
    let (cuenta, persona) = (CuentaId::nuevo(), PersonaId::nuevo());
    unidad.con_datos(|d| {
        let mut p = Persona::basica(persona, TipoPersona::Natural, "Ana".into(), None);
        p.cuenta_id = Some(cuenta);
        d.personas.push(p);
        d.publicidad.piezas = piezas;
        d.publicidad.politica = Some(PoliticaPublicidad {
            id: PoliticaPublicidadId::nuevo(),
            version: 2,
            texto: "Política de publicidad".into(),
            vigente_desde: ahora() - Duration::days(30),
            publicada_por: None,
            creada_en: ahora() - Duration::days(30),
        });
        d.gestion.linea = Some("LINEA".into());
    });
    Entorno {
        unidad,
        tenant: TenantId::nuevo(),
        cuenta,
        persona,
    }
}

impl Entorno {
    fn ctx(&self) -> Contexto {
        Contexto::de_cuenta(
            self.tenant,
            self.cuenta,
            [Permiso::DirectorioVer].into_iter().collect(),
        )
    }

    async fn autorizar(&self) {
        assert!(
            autorizar_publicidad(
                &self.unidad,
                &reloj(),
                &self.ctx(),
                2,
                MedioAutorizacion::App
            )
            .await
            .expect("autoriza")
        );
    }

    async fn piezas(&self) -> Vec<PiezaPublicidad> {
        piezas_para_residente(&self.unidad, &reloj(), &self.ctx())
            .await
            .expect("piezas")
    }
}

#[tokio::test]
async fn sin_autorizacion_de_publicidad_no_hay_piezas() {
    let e = entorno(vec![pieza(
        FormatoCreativo::Banner,
        ModeloCobro::Cpm,
        dec!(20000),
        dec!(100000),
    )]);

    assert!(e.piezas().await.is_empty());
    e.autorizar().await;
    assert_eq!(e.piezas().await.len(), 1);
    assert!(
        revocar_publicidad(&e.unidad, &reloj(), &e.ctx(), None)
            .await
            .expect("revoca")
    );
    assert!(e.piezas().await.is_empty(), "revocar la quita de inmediato");
}

#[tokio::test]
async fn sin_el_modulo_no_hay_piezas_aunque_haya_autorizado() {
    let e = entorno(vec![pieza(
        FormatoCreativo::Banner,
        ModeloCobro::Cpm,
        dec!(20000),
        dec!(100000),
    )]);
    e.autorizar().await;
    e.unidad.con_datos(|d| d.modulos.clear());

    assert!(e.piezas().await.is_empty());
}

#[tokio::test]
async fn autorizar_exige_la_version_vigente_y_no_duplica() {
    let e = entorno(vec![]);

    let vieja =
        autorizar_publicidad(&e.unidad, &reloj(), &e.ctx(), 1, MedioAutorizacion::App).await;
    assert!(matches!(vieja, Err(AppError::Validacion(_))));
    e.autorizar().await;
    let otra = autorizar_publicidad(&e.unidad, &reloj(), &e.ctx(), 2, MedioAutorizacion::App)
        .await
        .expect("repite");
    assert!(!otra);
    let estado = autorizacion_publicidad(&e.unidad, &reloj(), &e.ctx())
        .await
        .expect("estado");
    assert!(estado.autorizada);
    assert_eq!(estado.politica.map(|p| p.version), Some(2));
}

#[tokio::test]
async fn los_eventos_respetan_el_presupuesto_y_finalizan_la_campana() {
    let p = pieza(
        FormatoCreativo::Banner,
        ModeloCobro::Cpc,
        dec!(1000),
        dec!(2500),
    );
    let creativo = p.candidata.creativo;
    let e = entorno(vec![p]);
    e.autorizar().await;

    let a = registrar_eventos(
        &e.unidad,
        &reloj(),
        &e.ctx(),
        &[EventoPieza {
            creativo,
            impresiones: 10,
            clics: 5,
        }],
    )
    .await
    .expect("eventos");

    assert_eq!((a.impresiones, a.clics), (10, 2), "solo caben dos clics");
    let datos = e.unidad.datos().publicidad;
    assert_eq!(datos.entregas[0].costo, dec!(2000));
    assert!(
        datos.entregas[0].finalizada,
        "quedan 500: no alcanza otro clic"
    );
    let b = registrar_eventos(
        &e.unidad,
        &reloj(),
        &e.ctx(),
        &[EventoPieza {
            creativo,
            impresiones: 1,
            clics: 1,
        }],
    )
    .await
    .expect("eventos");
    assert!(b.vacio(), "la campaña finalizada ya no entrega");
}

#[tokio::test]
async fn los_eventos_de_quien_no_autorizo_no_cuentan() {
    let p = pieza(
        FormatoCreativo::Banner,
        ModeloCobro::Cpc,
        dec!(1000),
        dec!(9000),
    );
    let creativo = p.candidata.creativo;
    let e = entorno(vec![p]);

    let a = registrar_eventos(
        &e.unidad,
        &reloj(),
        &e.ctx(),
        &[EventoPieza {
            creativo,
            impresiones: 3,
            clics: 1,
        }],
    )
    .await
    .expect("eventos");

    assert!(a.vacio());
    assert!(e.unidad.datos().publicidad.entregas.is_empty());
}

#[tokio::test]
async fn demasiados_eventos_se_rechazan() {
    let e = entorno(vec![]);
    let r = registrar_eventos(
        &e.unidad,
        &reloj(),
        &e.ctx(),
        &[EventoPieza {
            creativo: CreativoId::nuevo(),
            impresiones: MAXIMO_EVENTOS + 1,
            clics: 0,
        }],
    )
    .await;
    assert!(matches!(r, Err(AppError::Validacion(_))));
}

#[tokio::test]
async fn el_cupon_se_entrega_a_quien_autorizo_y_cuenta_como_clic() {
    let p = pieza(
        FormatoCreativo::OfertaCupon,
        ModeloCobro::Cpc,
        dec!(500),
        dec!(9000),
    );
    let creativo = p.candidata.creativo;
    let e = entorno(vec![p]);

    let sin = obtener_cupon(&e.unidad, &reloj(), &e.ctx(), creativo).await;
    assert!(matches!(sin, Err(AppError::NoEncontrado(_))));
    e.autorizar().await;
    let c = obtener_cupon(&e.unidad, &reloj(), &e.ctx(), creativo)
        .await
        .expect("cupón");

    assert_eq!(c.codigo, "ROSA10");
    assert_eq!(e.unidad.datos().publicidad.entregas[0].conteo.clics, 1);
}

#[tokio::test]
async fn reportar_un_anuncio_una_vez_por_cuenta() {
    let e = entorno(vec![]);
    let c = CreativoId::nuevo();
    assert!(
        reportar_anuncio(&e.unidad, &reloj(), &e.ctx(), c, "Publicidad engañosa")
            .await
            .expect("reporta")
    );
    assert!(
        !reportar_anuncio(&e.unidad, &reloj(), &e.ctx(), c, "Otra vez")
            .await
            .expect("repite")
    );
    let vacio = reportar_anuncio(&e.unidad, &reloj(), &e.ctx(), c, " ").await;
    assert!(matches!(vacio, Err(AppError::Validacion(_))));
}

struct ConPublicidad(TenantId);

impl DirectorioPublicidad for ConPublicidad {
    async fn copropiedades_con_publicidad(
        &self,
        _ahora: DateTime<Utc>,
    ) -> crate::AppResult<Vec<TenantId>> {
        Ok(vec![self.0])
    }
}

#[tokio::test]
async fn el_masivo_solo_llega_a_quien_autorizo_y_respeta_la_frecuencia() {
    let p = pieza(
        FormatoCreativo::MensajeWhatsapp,
        ModeloCobro::Cpm,
        dec!(20000),
        dec!(100000),
    );
    let campana = p.candidata.campana;
    let e = entorno(vec![p]);
    e.autorizar().await;
    let (sin_autorizar, reciente) = (PersonaId::nuevo(), PersonaId::nuevo());
    e.unidad.con_datos(|d| {
        for (id, tel) in [
            (e.persona, "573001110001"),
            (sin_autorizar, "573001110002"),
            (reciente, "573001110003"),
        ] {
            d.publicidad.personas.push(DestinatarioMasivo {
                persona_id: id,
                telefono: Some(tel.into()),
                correo: None,
            });
        }
        // `reciente` autorizó, pero recibió otra campaña hace 3 días.
        let a = d.autorizaciones[0].0.clone();
        d.autorizaciones.push((
            crate::puertos::proteccion_datos::Autorizacion {
                id: domain::AutorizacionId::nuevo(),
                persona_id: reciente,
                ..a
            },
            None,
        ));
        d.publicidad.envios.push((
            CampanaId::nuevo(),
            reciente,
            CanalPublicidad::Whatsapp,
            ahora() - Duration::days(3),
        ));
    });
    let dir = ConPublicidad(e.tenant);

    let r = enviar_masivos(&dir, &e.unidad, &reloj())
        .await
        .expect("envía");

    assert_eq!(r.mensajes, 1);
    let datos = e.unidad.datos().publicidad;
    assert_eq!(datos.encolados.len(), 1);
    let (ev, c) = &datos.encolados[0];
    assert_eq!(*c, campana);
    assert_eq!(ev.prioridad, PrioridadSalida::Publicidad);
    let MensajeOutbox::WhatsApp(m) = &ev.mensaje else {
        panic!("WhatsApp");
    };
    assert_eq!(m.telefono, "573001110001");
    let ContenidoSaliente::Plantilla { botones, .. } = &m.contenido else {
        panic!("plantilla");
    };
    assert_eq!(
        BotonBaja::desde_id(&botones[0]),
        Some(BotonBaja { tenant: e.tenant })
    );
    assert_eq!(datos.entregas[0].conteo.impresiones, 1);
    assert_eq!(datos.entregas[0].costo, dec!(20));

    let otra = enviar_masivos(&dir, &e.unidad, &reloj())
        .await
        .expect("envía");
    assert_eq!(otra.mensajes, 0, "no repite la campaña");
}

#[tokio::test]
async fn la_baja_por_whatsapp_revoca_sin_cuenta() {
    let e = entorno(vec![]);
    e.autorizar().await;

    let r = revocar_publicidad(
        &e.unidad,
        &reloj(),
        &Contexto::new(e.tenant),
        Some(e.persona),
    )
    .await
    .expect("revoca");

    assert!(r);
    assert!(
        !autorizacion_publicidad(&e.unidad, &reloj(), &e.ctx())
            .await
            .expect("estado")
            .autorizada
    );
}

#[tokio::test]
async fn la_pantalla_exige_su_token_y_cuenta_impresiones() {
    let p = pieza(
        FormatoCreativo::PantallaLobby,
        ModeloCobro::Cpm,
        dec!(20000),
        dec!(100000),
    );
    let e = entorno(vec![p]);
    let token = "t".repeat(64);
    e.unidad.con_datos(|d| {
        d.publicidad.pantalla = Some(crate::puertos::publicidad::PantallaPublicidad {
            token_hash: hex::decode(crate::seguridad::hash_secreto(&token)).expect("hex"),
            creado_por: None,
            creado_en: ahora(),
        });
    });

    let mal = piezas_pantalla(&e.unidad, &reloj(), e.tenant, "otro").await;
    assert!(matches!(mal, Err(AppError::NoAutenticado)));
    let v = piezas_pantalla(&e.unidad, &reloj(), e.tenant, &token)
        .await
        .expect("pantalla");
    assert_eq!(v.len(), 1);
    assert_eq!(
        e.unidad.datos().publicidad.entregas[0].conteo.impresiones,
        1
    );
}

#[test]
fn boton_de_baja_ida_y_vuelta() {
    let b = BotonBaja {
        tenant: TenantId::nuevo(),
    };
    assert_eq!(BotonBaja::desde_id(&b.id()), Some(b));
    assert_eq!(BotonBaja::desde_id("op:x"), None);
}

#[test]
fn sin_cuenta_ni_permiso_no_hay_contexto_de_residente() {
    let ctx = Contexto::de_cuenta(
        TenantId::nuevo(),
        CuentaId::nuevo(),
        ConjuntoPermisos::vacio(),
    );
    assert!(ctx.exigir(Permiso::DirectorioVer).is_err());
}
