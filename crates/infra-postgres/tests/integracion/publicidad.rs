//! Integración de la publicidad de la plataforma (plan 08, fases 3 a 6; plan 20) sobre PostgreSQL
//! como `dad_app`: la plataforma carga la pauta → otra cuenta la modera → piezas solo para quien
//! autorizó, topes de
//! presupuesto, alcance por perfil de la PH, conflicto de interés, envíos masivos con prioridad
//! 9 y tope de frecuencia, cupones, reportes, leads y liquidación sin reparto a las PH.

use crate::comun;

use application::{
    AppError, Contexto, ContextoPlataforma, SolicitudPagina,
    casos_uso::{
        directorio::{self as dir, DatosAnunciante, DatosFicha},
        publicidad::{
            entrega::{self, EventoPieza},
            gestion::{self as portal, AccionCampana, DatosCampana},
            liquidacion, moderacion,
            moderacion::Decision,
        },
    },
    puertos::{
        RelojSistema,
        directorio::{Contacto, OrganizacionAnunciante},
        identidad::NuevaOrganizacion,
        publicidad::{Campana, TarifaPublicidad},
    },
};
use chrono::{Datelike, Duration, Months, Utc};
use comun::{CofrePrueba, MIGRACIONES, crear_copropiedad, pool_como_app};
use domain::{
    AnuncianteId, Cobertura, CodigoCategoria, CodigoDane, Coordenadas, CuentaId, EstadoCampana,
    EstadoLiquidacion, EstadoVerificacion, FormatoCreativo, MedioAutorizacion, ModeloCobro,
    NaturalezaOrganizacion, Nit, ObjetivoCampana, Permiso, PersonaId, Segmentacion, Telefono,
    TenantId, TipoOrganizacion, Ubicacion,
    publicidad::{CodigoCupon, ContenidoCreativo, Cupon},
};
use infra_postgres::{DirectorioPublicidadPg, UnidadDeTrabajoPg, UnidadDeTrabajoPlataformaPg};
use rust_decimal::{Decimal, dec};
use sqlx::PgPool;
use uuid::Uuid;

const CEL_ANA: &str = "573001110001";
const CEL_BRUNO: &str = "573001110002";

async fn cuenta(pool: &PgPool) -> CuentaId {
    let c = CuentaId::nuevo();
    sqlx::query(
        "INSERT INTO cuentas (id, nombre_mostrar, usuario, hash_contrasena,
                              contrasena_actualizada_en, creada_en)
         VALUES ($1, 'Cuenta', $2, 'x', now(), now())",
    )
    .bind(c.as_uuid())
    .bind(format!("c{}", &c.as_uuid().simple().to_string()[..24]))
    .execute(pool)
    .await
    .expect("cuenta");
    c
}

struct Entorno {
    owner: PgPool,
    unidad: UnidadDeTrabajoPg,
    plataforma: UnidadDeTrabajoPlataformaPg,
    tenant: TenantId,
    ana: CuentaId,
    ana_persona: PersonaId,
    dueno: CuentaId,
}

impl Entorno {
    /// PH activa en Chapinero (estrato 4), sin habilitar nada (la publicidad no es un módulo); Ana
    /// (con cuenta) y Bruno
    /// son propietarios con celular; una línea de WhatsApp, la tarifa CPC y la política.
    async fn new(pool: &PgPool) -> Self {
        let app = pool_como_app(pool).await;
        let tenant = crear_copropiedad(pool).await;
        let (ana, dueno) = (cuenta(pool).await, cuenta(pool).await);
        let (ana_persona, bruno) = (PersonaId::nuevo(), Uuid::now_v7());
        let (b1, b2) = (Uuid::now_v7(), Uuid::now_v7());
        let mut tx = pool.begin().await.expect("tx");
        for sql in [
            "UPDATE entidades SET estado = 'activa', latitud = 4.650000, longitud = -74.060000,
                    municipio_dane = '11001' WHERE id = $1",
            "UPDATE copropiedades SET estrato = 4 WHERE id = $1",
            "INSERT INTO bienes_privados (id, tenant_id, codigo) VALUES ($2, $1, '101'), ($3, $1, '102')",
            "INSERT INTO personas (id, tenant_id, nombre, telefono, cuenta_id) VALUES
                ($4, $1, 'Ana', '573001110001', $6), ($5, $1, 'Bruno', '573001110002', NULL)",
            "INSERT INTO titularidades (id, tenant_id, bien_privado_id, persona_id, tipo, porcentaje,
                                        es_representante, vigente_desde) VALUES
                (gen_random_uuid(), $1, $2, $4, 'propietario', 100, true, now() - interval '1 year'),
                (gen_random_uuid(), $1, $3, $5, 'propietario', 100, true, now() - interval '1 year')",
            "INSERT INTO lineas_whatsapp (id, phone_number_id, numero_visible, nombre)
             VALUES (gen_random_uuid(), 'PNID', '+57 300 000 0000', 'Línea')",
        ] {
            sqlx::query(sql)
                .bind(tenant.as_uuid())
                .bind(b1)
                .bind(b2)
                .bind(ana_persona.as_uuid())
                .bind(bruno)
                .bind(ana.as_uuid())
                .execute(&mut *tx)
                .await
                .expect(sql);
        }
        tx.commit().await.expect("commit");
        let e = Self {
            owner: pool.clone(),
            unidad: UnidadDeTrabajoPg::new(app.clone()),
            plataforma: UnidadDeTrabajoPlataformaPg::new(app),
            tenant,
            ana,
            ana_persona,
            dueno,
        };
        let sis = ContextoPlataforma::sistema();
        for (modelo, valor) in [
            (ModeloCobro::Cpc, dec!(1000)),
            (ModeloCobro::Cpm, dec!(20000)),
            (ModeloCobro::PorRedencion, dec!(1500)),
            (ModeloCobro::TarifaFija, dec!(90000)),
        ] {
            liquidacion::fijar_tarifa(
                &e.plataforma,
                &RelojSistema,
                &sis,
                TarifaPublicidad {
                    modelo,
                    valor,
                    vigente_desde: Utc::now().date_naive(),
                },
            )
            .await
            .expect("tarifa");
        }
        liquidacion::publicar_politica(
            &e.plataforma,
            &RelojSistema,
            &sis,
            "Política de publicidad",
        )
        .await
        .expect("política");
        e
    }

    /// Personal de la plataforma que carga la pauta (no la modera).
    fn ctx_dueno(&self) -> ContextoPlataforma {
        ContextoPlataforma::de_cuenta(
            self.dueno,
            [
                Permiso::PlataformaPublicidadGestionar,
                Permiso::PlataformaDirectorioGestionar,
                Permiso::PlataformaPublicidadModerar,
            ]
            .into_iter()
            .collect(),
            None,
        )
    }

    fn ctx_ana(&self) -> Contexto {
        Contexto::de_cuenta(
            self.tenant,
            self.ana,
            [Permiso::DirectorioVer].into_iter().collect(),
        )
    }

    /// La plataforma da de alta el comercio, lo verifica y publica su ficha.
    async fn anunciante(&self) -> AnuncianteId {
        let a = dir::dar_de_alta_anunciante(
            &self.plataforma,
            &RelojSistema,
            &self.ctx_dueno(),
            OrganizacionAnunciante::Nueva(NuevaOrganizacion {
                nit: Nit::new("900123456-8").expect("NIT"),
                razon_social: "Rosa Pérez".into(),
                naturaleza: NaturalezaOrganizacion::NaturalConRut,
                tipos: vec![TipoOrganizacion::Otro],
                correo_contacto: None,
                telefono_contacto: None,
            }),
            DatosAnunciante {
                nombre_comercial: "Modistería Rosa".into(),
                direccion: None,
                ubicacion: Ubicacion {
                    coordenadas: Some(
                        Coordenadas::new(dec!(4.655000), dec!(-74.060000)).expect("válidas"),
                    ),
                    municipio: Some(CodigoDane::new("11001").expect("válido")),
                },
            },
        )
        .await
        .expect("alta");
        assert_eq!(a.estado, EstadoVerificacion::Verificado);
        dir::guardar_ficha(
            &self.plataforma,
            &RelojSistema,
            &self.ctx_dueno(),
            a.id,
            DatosFicha {
                descripcion: None,
                categorias: vec![CodigoCategoria::new("costura").expect("válido")],
                cobertura: Cobertura::Radio(5),
                contacto: Contacto {
                    whatsapp: Some(Telefono::new("573109998877").expect("válido")),
                    ..Contacto::default()
                },
                horario: None,
                destacada_hasta: None,
                publicada: true,
            },
        )
        .await
        .expect("ficha");
        a.id
    }

    /// Campaña con un creativo, aprobada por la moderación y activa.
    async fn campana_activa(
        &self,
        a: AnuncianteId,
        modelo: ModeloCobro,
        total: Decimal,
        segmentacion: Segmentacion,
        contenido: ContenidoCreativo,
    ) -> (Campana, domain::CreativoId) {
        let c = portal::crear_campana(
            &self.plataforma,
            &RelojSistema,
            &self.ctx_dueno(),
            a,
            DatosCampana {
                nombre: "Noviembre".into(),
                objetivo: ObjetivoCampana::Trafico,
                categoria: CodigoCategoria::new("costura").expect("válido"),
                inicio: Utc::now() - Duration::hours(1),
                fin: Utc::now() + Duration::days(30),
                modelo,
                presupuesto_total: total,
                presupuesto_diario: None,
                segmentacion,
            },
        )
        .await
        .expect("campaña");
        let cr = portal::guardar_creativo(
            &self.plataforma,
            &RelojSistema,
            &self.ctx_dueno(),
            a,
            c.id,
            None,
            contenido,
        )
        .await
        .expect("creativo");
        self.cambiar(a, c.id, AccionCampana::EnviarARevision)
            .await
            .expect("envía");
        let sis = ContextoPlataforma::sistema();
        moderacion::moderar_creativo(
            &self.plataforma,
            &RelojSistema,
            &sis,
            cr.id,
            Decision::Aprobar,
        )
        .await
        .expect("aprueba creativo");
        moderacion::moderar_campana(
            &self.plataforma,
            &RelojSistema,
            &sis,
            c.id,
            Decision::Aprobar,
        )
        .await
        .expect("aprueba campaña");
        let c = self
            .cambiar(a, c.id, AccionCampana::Activar)
            .await
            .expect("activa");
        (c, cr.id)
    }

    async fn cambiar(
        &self,
        a: AnuncianteId,
        c: domain::CampanaId,
        accion: AccionCampana,
    ) -> application::AppResult<Campana> {
        portal::cambiar_estado_campana(
            &self.plataforma,
            &RelojSistema,
            &self.ctx_dueno(),
            a,
            c,
            accion,
        )
        .await
    }

    async fn autorizar_ana(&self) {
        entrega::autorizar_publicidad(
            &self.unidad,
            &RelojSistema,
            &self.ctx_ana(),
            1,
            MedioAutorizacion::App,
        )
        .await
        .expect("autoriza");
    }

    async fn piezas_ana(&self) -> usize {
        entrega::piezas_para_residente(&self.unidad, &RelojSistema, &self.ctx_ana())
            .await
            .expect("piezas")
            .len()
    }
}

fn banner() -> ContenidoCreativo {
    ContenidoCreativo {
        formato: FormatoCreativo::Banner,
        titulo: "Arreglos en 24 horas".into(),
        texto: "Ruedos, cremalleras y ajustes".into(),
        boton: Some("Escribir".into()),
        enlace: None,
        condiciones: None,
        es_promocion: false,
        cupon: None,
        plantilla_whatsapp: None,
        imagen: None,
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn del_portal_a_la_pieza_solo_para_quien_autorizo_y_con_tope(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let a = e.anunciante().await;
    let (c, creativo) = e
        .campana_activa(
            a,
            ModeloCobro::Cpc,
            dec!(2500),
            Segmentacion::default(),
            banner(),
        )
        .await;
    assert_eq!(c.estado, EstadoCampana::Activa);
    assert_eq!(c.tarifa, Some(dec!(1000.0000)), "tarifa fijada al aprobar");

    assert_eq!(e.piezas_ana().await, 0, "sin autorización no hay piezas");
    e.autorizar_ana().await;
    assert_eq!(e.piezas_ana().await, 1);

    let aceptado = entrega::registrar_eventos(
        &e.unidad,
        &RelojSistema,
        &e.ctx_ana(),
        &[EventoPieza {
            creativo,
            impresiones: 4,
            clics: 5,
        }],
    )
    .await
    .expect("eventos");
    assert_eq!((aceptado.impresiones, aceptado.clics), (4, 2));

    let m = portal::metricas(&e.plataforma, &e.ctx_dueno(), a, c.id)
        .await
        .expect("métricas");
    assert_eq!((m.totales.impresiones, m.totales.clics), (4, 2));
    assert_eq!(m.consumido, dec!(2000.0000));
    assert_eq!(m.alcance, 1);
    let (c, _) = portal::campana(&e.plataforma, &e.ctx_dueno(), a, c.id)
        .await
        .expect("campaña");
    assert_eq!(c.estado, EstadoCampana::Finalizada, "presupuesto agotado");
    assert_eq!(e.piezas_ana().await, 0);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_autorizacion_es_del_titular_frente_a_la_plataforma(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    e.autorizar_ana().await;

    let fila: (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT politica_id, politica_publicidad_id FROM autorizaciones_tratamiento
         WHERE finalidad = 'publicidad'",
    )
    .fetch_one(&e.owner)
    .await
    .expect("autorización");
    assert!(
        fila.0.is_none() && fila.1.is_some(),
        "política de la plataforma, no de la PH"
    );
    assert!(
        entrega::revocar_publicidad(&e.unidad, &RelojSistema, &e.ctx_ana(), None)
            .await
            .expect("revoca")
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_moderacion_ni_creativos_aprobados_no_se_activa(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let a = e.anunciante().await;
    let c = portal::crear_campana(
        &e.plataforma,
        &RelojSistema,
        &e.ctx_dueno(),
        a,
        DatosCampana {
            nombre: "Sin revisar".into(),
            objetivo: ObjetivoCampana::Visibilidad,
            categoria: CodigoCategoria::new("costura").expect("válido"),
            inicio: Utc::now(),
            fin: Utc::now() + Duration::days(5),
            modelo: ModeloCobro::Cpc,
            presupuesto_total: dec!(10000),
            presupuesto_diario: None,
            segmentacion: Segmentacion::default(),
        },
    )
    .await
    .expect("campaña");

    assert!(matches!(
        e.cambiar(a, c.id, AccionCampana::Activar).await,
        Err(AppError::Dominio(_))
    ));
    assert!(
        matches!(
            e.cambiar(a, c.id, AccionCampana::EnviarARevision).await,
            Err(AppError::Validacion(_))
        ),
        "sin creativos"
    );
    portal::guardar_creativo(
        &e.plataforma,
        &RelojSistema,
        &e.ctx_dueno(),
        a,
        c.id,
        None,
        banner(),
    )
    .await
    .expect("creativo");
    e.cambiar(a, c.id, AccionCampana::EnviarARevision)
        .await
        .expect("envía");
    let r = moderacion::moderar_campana(
        &e.plataforma,
        &RelojSistema,
        &ContextoPlataforma::sistema(),
        c.id,
        Decision::Aprobar,
    )
    .await;
    assert!(
        matches!(r, Err(AppError::Validacion(_))),
        "creativo pendiente"
    );
    let otro = portal::campana(
        &e.plataforma,
        &ContextoPlataforma::de_cuenta(
            cuenta(&pool).await,
            domain::ConjuntoPermisos::vacio(),
            None,
        ),
        a,
        c.id,
    )
    .await;
    assert!(
        matches!(otro, Err(AppError::SinPermiso(_))),
        "sin plataforma.publicidad.gestionar"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_proveedor_de_la_ph_queda_con_conflicto_de_interes(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let a = e.anunciante().await;
    sqlx::query(
        "INSERT INTO contratos_servicio (id, tenant_id, organizacion_id, tipo_servicio, vigente_desde)
         SELECT gen_random_uuid(), $1, organizacion_id, 'otro', now() - interval '1 day'
         FROM anunciantes WHERE id = $2",
    )
    .bind(e.tenant.as_uuid())
    .bind(a.as_uuid())
    .execute(&e.owner)
    .await
    .expect("contrato");

    let (c, _) = e
        .campana_activa(
            a,
            ModeloCobro::Cpc,
            dec!(9000),
            Segmentacion::default(),
            banner(),
        )
        .await;

    assert!(c.conflicto_interes);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_segmentacion_usa_el_perfil_de_la_ph(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    e.autorizar_ana().await;
    let a = e.anunciante().await;
    e.campana_activa(
        a,
        ModeloCobro::Cpc,
        dec!(9000),
        Segmentacion {
            estratos: vec![2, 3],
            ..Segmentacion::default()
        },
        banner(),
    )
    .await;
    assert_eq!(e.piezas_ana().await, 0, "la PH es estrato 4");

    let (c2, _) = e
        .campana_activa(
            a,
            ModeloCobro::Cpc,
            dec!(9000),
            Segmentacion {
                radio_km: Some(1),
                estratos: vec![4],
                ..Segmentacion::default()
            },
            banner(),
        )
        .await;
    assert_eq!(e.piezas_ana().await, 1, "≈0,6 km y estrato 4");
    let otra = crear_copropiedad(&pool).await;
    let alcanza: bool = sqlx::query_scalar("SELECT campana_alcanza($1, $2)")
        .bind(c2.id.as_uuid())
        .bind(otra.as_uuid())
        .fetch_one(&e.owner)
        .await
        .expect("alcance");
    assert!(!alcanza, "una PH sin ubicación ni estrato no entra");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_masivo_sale_con_prioridad_9_solo_a_quien_autorizo(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    e.autorizar_ana().await;
    let a = e.anunciante().await;
    let (c, _) = e
        .campana_activa(
            a,
            ModeloCobro::Cpm,
            dec!(100000),
            Segmentacion::default(),
            ContenidoCreativo {
                formato: FormatoCreativo::MensajeWhatsapp,
                plantilla_whatsapp: Some("oferta_semanal".into()),
                imagen: None,
                ..banner()
            },
        )
        .await;
    let dir = DirectorioPublicidadPg::new(pool_como_app(&pool).await);

    let r = entrega::enviar_masivos(&dir, &e.unidad, &RelojSistema)
        .await
        .expect("envía");

    assert_eq!(r.mensajes, 1);
    let filas: Vec<(String, i16, Uuid)> = sqlx::query_as(
        "SELECT destino, prioridad, campana_id FROM outbox WHERE concepto = 'publicidad'",
    )
    .fetch_all(&e.owner)
    .await
    .expect("outbox");
    assert_eq!(filas, [(CEL_ANA.to_owned(), 9, c.id.as_uuid())]);
    assert!(!filas.iter().any(|f| f.0 == CEL_BRUNO), "Bruno no autorizó");
    let otra = entrega::enviar_masivos(&dir, &e.unidad, &RelojSistema)
        .await
        .expect("envía");
    assert_eq!(otra.mensajes, 0, "tope de frecuencia");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_cupon_se_redime_hasta_su_limite_y_solo_por_miembros(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    e.autorizar_ana().await;
    let a = e.anunciante().await;
    let (_, creativo) = e
        .campana_activa(
            a,
            ModeloCobro::PorRedencion,
            dec!(9000),
            Segmentacion::default(),
            ContenidoCreativo {
                formato: FormatoCreativo::OfertaCupon,
                condiciones: Some("Un uso por persona, hasta fin de mes".into()),
                cupon: Some(Cupon {
                    codigo: CodigoCupon::new("ROSA10").expect("válido"),
                    limite_usos: Some(1),
                }),
                ..banner()
            },
        )
        .await;
    let cupon = entrega::obtener_cupon(&e.unidad, &RelojSistema, &e.ctx_ana(), creativo)
        .await
        .expect("cupón");
    assert_eq!(cupon.codigo, "ROSA10");

    let anonimo = ContextoPlataforma::anonimo(None);
    assert!(matches!(
        portal::redimir_cupon(
            &e.plataforma,
            &RelojSistema,
            &anonimo,
            "sin-enlace",
            "rosa10"
        )
        .await,
        Err(AppError::NoAutenticado)
    ));
    let token = portal::generar_enlace_comercio(
        &e.plataforma,
        &CofrePrueba,
        &RelojSistema,
        &e.ctx_dueno(),
        a,
    )
    .await
    .expect("enlace");
    let vigente = portal::enlace_comercio(&e.plataforma, &CofrePrueba, &e.ctx_dueno(), a)
        .await
        .expect("consulta");
    assert_eq!(
        vigente.as_deref(),
        Some(token.as_str()),
        "el mismo token, sin cambiarlo"
    );
    let r = portal::redimir_cupon(&e.plataforma, &RelojSistema, &anonimo, &token, "rosa10")
        .await
        .expect("redime");
    assert_eq!((r.usos, r.restantes), (1, Some(0)));
    assert!(matches!(
        portal::redimir_cupon(&e.plataforma, &RelojSistema, &anonimo, &token, "ROSA10").await,
        Err(AppError::Dominio(_))
    ));
    portal::revocar_enlace_comercio(&e.plataforma, &e.ctx_dueno(), a)
        .await
        .expect("revoca");
    assert!(matches!(
        portal::redimir_cupon(&e.plataforma, &RelojSistema, &anonimo, &token, "ROSA10").await,
        Err(AppError::NoAutenticado)
    ));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn quien_carga_la_pauta_no_la_modera(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let a = e.anunciante().await;
    let c = portal::crear_campana(
        &e.plataforma,
        &RelojSistema,
        &e.ctx_dueno(),
        a,
        DatosCampana {
            nombre: "Cuatro ojos".into(),
            objetivo: ObjetivoCampana::Visibilidad,
            categoria: CodigoCategoria::new("costura").expect("válido"),
            inicio: Utc::now(),
            fin: Utc::now() + Duration::days(5),
            modelo: ModeloCobro::Cpc,
            presupuesto_total: dec!(10000),
            presupuesto_diario: None,
            segmentacion: Segmentacion::default(),
        },
    )
    .await
    .expect("campaña");
    let cr = portal::guardar_creativo(
        &e.plataforma,
        &RelojSistema,
        &e.ctx_dueno(),
        a,
        c.id,
        None,
        banner(),
    )
    .await
    .expect("creativo");
    e.cambiar(a, c.id, AccionCampana::EnviarARevision)
        .await
        .expect("envía");

    let creativo_propio = moderacion::moderar_creativo(
        &e.plataforma,
        &RelojSistema,
        &e.ctx_dueno(),
        cr.id,
        Decision::Aprobar,
    )
    .await;
    let campana_propia = moderacion::moderar_campana(
        &e.plataforma,
        &RelojSistema,
        &e.ctx_dueno(),
        c.id,
        Decision::Aprobar,
    )
    .await;
    let revisor = ContextoPlataforma::de_cuenta(
        cuenta(&pool).await,
        [Permiso::PlataformaPublicidadModerar].into_iter().collect(),
        None,
    );
    let por_otra = moderacion::moderar_creativo(
        &e.plataforma,
        &RelojSistema,
        &revisor,
        cr.id,
        Decision::Aprobar,
    )
    .await
    .expect("otra cuenta modera");

    assert!(matches!(creativo_propio, Err(AppError::Conflicto(_))));
    assert!(matches!(campana_propia, Err(AppError::Conflicto(_))));
    assert_eq!(por_otra.revisado_por, revisor.actor_id);
    // La BD también lo exige.
    let r = sqlx::query("UPDATE creativos SET revisado_por = cargado_por WHERE id = $1")
        .bind(cr.id.as_uuid())
        .execute(&e.owner)
        .await;
    assert!(r.is_err());
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_entidad_excluida_no_recibe_piezas_y_una_empresa_si(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    e.autorizar_ana().await;
    let a = e.anunciante().await;
    e.campana_activa(
        a,
        ModeloCobro::Cpc,
        dec!(9000),
        Segmentacion::default(),
        banner(),
    )
    .await;
    assert_eq!(e.piezas_ana().await, 1);

    portal::excluir_entidad(
        &e.plataforma,
        &RelojSistema,
        &e.ctx_dueno(),
        e.tenant,
        "El contrato no permite publicidad",
    )
    .await
    .expect("excluye");
    assert_eq!(e.piezas_ana().await, 0);
    portal::incluir_entidad(&e.plataforma, &e.ctx_dueno(), e.tenant)
        .await
        .expect("incluye");
    assert_eq!(e.piezas_ana().await, 1);

    // Una empresa en el mismo sector: la campaña sin criterios de PH la alcanza; una que exige
    // estrato, no.
    let empresa = comun::insertar_entidad(&e.owner, "Oficina").await;
    sqlx::query(
        "UPDATE entidades SET estado = 'activa', latitud = 4.651, longitud = -74.06,
                municipio_dane = '11001' WHERE id = $1",
    )
    .bind(empresa.as_uuid())
    .execute(&e.owner)
    .await
    .expect("empresa");
    let alcanza: bool =
        sqlx::query_scalar("SELECT campana_alcanza(c.id, $1) FROM campanas c LIMIT 1")
            .bind(empresa.as_uuid())
            .fetch_one(&e.owner)
            .await
            .expect("alcance");
    assert!(alcanza);
    sqlx::query("UPDATE campanas SET seg_estratos = '{4}'")
        .execute(&e.owner)
        .await
        .expect("estrato");
    let (ph, emp): (bool, bool) = sqlx::query_as(
        "SELECT campana_alcanza(c.id, $1), campana_alcanza(c.id, $2) FROM campanas c LIMIT 1",
    )
    .bind(e.tenant.as_uuid())
    .bind(empresa.as_uuid())
    .fetch_one(&e.owner)
    .await
    .expect("alcance");
    assert!(ph && !emp, "el estrato solo lo cumple una PH");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_reporte_llega_a_la_moderacion_y_retirar_quita_la_pieza(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    e.autorizar_ana().await;
    let a = e.anunciante().await;
    let (_, creativo) = e
        .campana_activa(
            a,
            ModeloCobro::Cpc,
            dec!(9000),
            Segmentacion::default(),
            banner(),
        )
        .await;
    assert!(
        entrega::reportar_anuncio(&e.unidad, &RelojSistema, &e.ctx_ana(), creativo, "Engañoso")
            .await
            .expect("reporta")
    );
    let sis = ContextoPlataforma::sistema();

    let cola = moderacion::reportes_pendientes(&e.plataforma, &sis, SolicitudPagina::primera())
        .await
        .expect("cola");
    assert_eq!(cola.datos[0].motivos, ["Engañoso"]);
    let n = moderacion::atender_reportes(
        &e.plataforma,
        &RelojSistema,
        &sis,
        creativo,
        Some("Publicidad engañosa"),
    )
    .await
    .expect("atiende");

    assert_eq!(n, 1);
    assert_eq!(e.piezas_ana().await, 0);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_liquidacion_del_mes_suma_lo_consumido_sin_reparto_a_las_ph(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    e.autorizar_ana().await;
    let a = e.anunciante().await;
    let (c, creativo) = e
        .campana_activa(
            a,
            ModeloCobro::Cpc,
            dec!(9000),
            Segmentacion::default(),
            banner(),
        )
        .await;
    entrega::registrar_eventos(
        &e.unidad,
        &RelojSistema,
        &e.ctx_ana(),
        &[EventoPieza {
            creativo,
            impresiones: 0,
            clics: 3,
        }],
    )
    .await
    .expect("eventos");
    let mes_pasado = Utc::now()
        .date_naive()
        .with_day(1)
        .and_then(|d| d.checked_sub_months(Months::new(1)))
        .expect("mes");
    sqlx::query("UPDATE entregas_publicidad SET dia = $1 WHERE campana_id = $2")
        .bind(mes_pasado)
        .bind(c.id.as_uuid())
        .execute(&e.owner)
        .await
        .expect("al mes pasado");
    let sis = ContextoPlataforma::sistema();

    let v = liquidacion::liquidar_mes(&e.plataforma, &RelojSistema, &sis, mes_pasado)
        .await
        .expect("liquida");

    assert_eq!(v.len(), 1);
    assert_eq!((v[0].anunciante_id, v[0].valor), (a, dec!(3000.00)));
    let f = liquidacion::cambiar_estado_liquidacion(
        &e.plataforma,
        &RelojSistema,
        &sis,
        a,
        mes_pasado,
        EstadoLiquidacion::Facturada,
    )
    .await
    .expect("facturada");
    assert_eq!(f.estado, EstadoLiquidacion::Facturada);
    let mes_actual = Utc::now().date_naive();
    assert!(
        matches!(
            liquidacion::liquidar_mes(&e.plataforma, &RelojSistema, &sis, mes_actual).await,
            Err(AppError::Validacion(_))
        ),
        "el mes no ha terminado"
    );
    let ph_cols: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns
         WHERE table_name = 'liquidaciones' AND column_name = 'tenant_id'",
    )
    .fetch_one(&e.owner)
    .await
    .expect("columnas");
    assert_eq!(ph_cols, 0, "la liquidación no reparte nada a las PH");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_anunciante_ve_sus_leads_y_ningun_otro_dato_de_residentes(pool: PgPool) {
    use application::casos_uso::busqueda_proveedores::{self as bp, NuevaBusqueda};
    let e = Entorno::new(&pool).await;
    let a = e.anunciante().await;
    let ctx = Contexto::new(e.tenant);
    let r = bp::buscar_proveedores(
        &e.unidad,
        &RelojSistema,
        &ctx,
        NuevaBusqueda {
            persona_id: e.ana_persona,
            canal: domain::CanalSolicitud::Whatsapp,
            texto: "busco costurera".into(),
            clave: "wamid.1".into(),
        },
    )
    .await
    .expect("busca");
    bp::autorizar_contacto(
        &e.unidad,
        &RelojSistema,
        &ctx,
        r.solicitud.id,
        e.ana_persona,
        None,
    )
    .await
    .expect("autoriza");

    let leads = portal::leads(&e.plataforma, &e.ctx_dueno(), a, SolicitudPagina::primera())
        .await
        .expect("leads");

    assert_eq!(leads.datos.len(), 1);
    assert_eq!(leads.datos[0].nombre, "Ana");
    assert_eq!(
        leads.datos[0].telefono.as_ref().map(Telefono::as_str),
        Some(CEL_ANA)
    );
    let (c, _) = e
        .campana_activa(
            a,
            ModeloCobro::Cpc,
            dec!(9000),
            Segmentacion::default(),
            banner(),
        )
        .await;
    let m = portal::metricas(&e.plataforma, &e.ctx_dueno(), a, c.id)
        .await
        .expect("métricas");
    let texto = format!("{m:?}");
    assert!(!texto.contains("Ana") && !texto.contains(CEL_ANA) && !texto.contains(CEL_BRUNO));
}
