//! Integración del directorio de comercios y servicios (plan 08, fase 1) sobre PostgreSQL como
//! `dad_app`: alta en la plataforma, cobertura por radio (haversine en SQL igual al dominio),
//! municipio y país, búsqueda por sinónimos, categorías restringidas, reseñas por PH con
//! promedio global, moderación y aislamiento entre ámbitos.

mod comun;

use application::{
    AppError, Contexto, ContextoPlataforma, SolicitudPagina,
    casos_uso::directorio::{self as uc, DatosAnunciante, DatosFicha, DecisionModeracion},
    puertos::{
        RelojSistema,
        directorio::{Contacto, FiltroDirectorio, OrganizacionAnunciante},
        identidad::NuevaOrganizacion,
    },
};
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};
use domain::{
    AnuncianteId, Cobertura, CodigoCategoria, CodigoDane, Coordenadas, CuentaId,
    EstadoVerificacion, NaturalezaOrganizacion, Nit, Permiso, Telefono, TenantId, TipoOrganizacion,
    Ubicacion,
};
use infra_postgres::{UnidadDeTrabajoPg, UnidadDeTrabajoPlataformaPg};
use rust_decimal::{Decimal, dec};
use sqlx::PgPool;

/// Tienda de Chapinero (Bogotá) con cobertura de 2 km.
const TIENDA: (Decimal, Decimal) = (dec!(4.650000), dec!(-74.060000));

fn ubicacion(c: Option<(Decimal, Decimal)>, municipio: Option<&str>) -> Ubicacion {
    Ubicacion {
        coordenadas: c.map(|(la, lo)| Coordenadas::new(la, lo).expect("válidas")),
        municipio: municipio.map(|m| CodigoDane::new(m).expect("válido")),
    }
}

fn categorias(v: &[&str]) -> Vec<CodigoCategoria> {
    v.iter()
        .map(|c| CodigoCategoria::new(c).expect("válido"))
        .collect()
}

fn ficha(cats: &[&str], cobertura: Cobertura) -> DatosFicha {
    DatosFicha {
        descripcion: Some("Arreglos de ropa y confección a la medida".into()),
        categorias: categorias(cats),
        cobertura,
        contacto: Contacto {
            whatsapp: Some(Telefono::new("573001234567").expect("válido")),
            ..Contacto::default()
        },
        horario: Some("Lunes a sábado".into()),
        destacada_hasta: None,
        publicada: true,
    }
}

fn nueva_organizacion(nit: &str, razon: &str) -> OrganizacionAnunciante {
    OrganizacionAnunciante::Nueva(NuevaOrganizacion {
        nit: Nit::new(nit).expect("NIT válido"),
        razon_social: razon.into(),
        naturaleza: NaturalezaOrganizacion::NaturalConRut,
        tipos: vec![TipoOrganizacion::Otro],
        correo_contacto: None,
        telefono_contacto: None,
    })
}

struct Entorno {
    app: PgPool,
    plataforma: UnidadDeTrabajoPlataformaPg,
    unidad: UnidadDeTrabajoPg,
}

impl Entorno {
    async fn new(pool: &PgPool) -> Self {
        let app = pool_como_app(pool).await;
        Self {
            plataforma: UnidadDeTrabajoPlataformaPg::new(app.clone()),
            unidad: UnidadDeTrabajoPg::new(app.clone()),
            app,
        }
    }

    async fn anunciante(
        &self,
        nit: &str,
        nombre: &str,
        donde: Ubicacion,
        datos: DatosFicha,
    ) -> AnuncianteId {
        let ctx = ContextoPlataforma::sistema();
        let a = uc::dar_de_alta_anunciante(
            &self.plataforma,
            &RelojSistema,
            &ctx,
            nueva_organizacion(nit, nombre),
            DatosAnunciante {
                nombre_comercial: nombre.into(),
                direccion: Some("Calle 60 # 9-10".into()),
                ubicacion: donde,
            },
        )
        .await
        .expect("alta");
        uc::guardar_ficha(&self.plataforma, &RelojSistema, &ctx, a.id, datos)
            .await
            .expect("ficha");
        a.id
    }

    async fn directorio(&self, ctx: &Contexto, filtro: FiltroDirectorio) -> Vec<AnuncianteId> {
        uc::consultar_directorio(
            &self.unidad,
            &RelojSistema,
            ctx,
            filtro,
            SolicitudPagina::primera(),
        )
        .await
        .expect("directorio")
        .datos
        .into_iter()
        .map(|f| f.anunciante_id)
        .collect()
    }
}

/// Copropiedad activa con su ubicación.
async fn ph(pool: &PgPool, c: Option<(Decimal, Decimal)>, municipio: Option<&str>) -> TenantId {
    let tenant = crear_copropiedad(pool).await;
    sqlx::query(
        "UPDATE copropiedades SET estado = 'activa', latitud = $2, longitud = $3, municipio_dane = $4
         WHERE id = $1",
    )
    .bind(tenant.as_uuid())
    .bind(c.map(|x| x.0))
    .bind(c.map(|x| x.1))
    .bind(municipio)
    .execute(pool)
    .await
    .expect("ubicación");
    tenant
}

/// Cuenta con un rol plantilla en la PH (`residencia` deriva el de residente).
async fn cuenta_con_rol(pool: &PgPool, tenant: TenantId, rol: &str, origen: &str) -> CuentaId {
    let cuenta = CuentaId::nuevo();
    sqlx::query(
        "INSERT INTO cuentas (id, nombre_mostrar, usuario, hash_contrasena, contrasena_actualizada_en, creada_en)
         VALUES ($1, 'Vecina', $2, 'x', now(), now())",
    )
    .bind(cuenta.as_uuid())
    .bind(format!("v{}", &cuenta.as_uuid().simple().to_string()[..24]))
    .execute(pool)
    .await
    .expect("cuenta");
    sqlx::query(
        "INSERT INTO asignaciones_rol (id, tenant_id, cuenta_id, rol_id, vigente_desde, origen, origen_id)
         VALUES (gen_random_uuid(), $1, $2, uuid_rol_plantilla($3), now() - interval '1 day', $4,
                 CASE WHEN $4 = 'manual' THEN NULL ELSE gen_random_uuid() END)",
    )
    .bind(tenant.as_uuid())
    .bind(cuenta.as_uuid())
    .bind(rol)
    .bind(origen)
    .execute(pool)
    .await
    .expect("asignación");
    cuenta
}

fn ctx_residente(tenant: TenantId, cuenta: CuentaId) -> Contexto {
    Contexto::de_cuenta(
        tenant,
        cuenta,
        [Permiso::DirectorioVer, Permiso::DirectorioResenar]
            .into_iter()
            .collect(),
    )
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_cobertura_por_radio_alcanza_solo_a_las_ph_cercanas(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let tienda = e
        .anunciante(
            "900123456-8",
            "Modistería Rosa",
            ubicacion(Some(TIENDA), Some("11001")),
            ficha(&["costura"], Cobertura::Radio(2)),
        )
        .await;
    // ~1,1 km al norte, ~11 km al norte y una PH de Bogotá sin coordenadas.
    let cerca = ph(
        &pool,
        Some((dec!(4.660000), dec!(-74.060000))),
        Some("11001"),
    )
    .await;
    let lejos = ph(
        &pool,
        Some((dec!(4.750000), dec!(-74.060000))),
        Some("11001"),
    )
    .await;
    let sin_ubicar = ph(&pool, None, Some("11001")).await;
    let medellin = ph(&pool, None, Some("05001")).await;

    let ver = |t| Contexto::new(t);
    assert_eq!(
        e.directorio(&ver(cerca), FiltroDirectorio::default()).await,
        vec![tienda]
    );
    assert!(
        e.directorio(&ver(lejos), FiltroDirectorio::default())
            .await
            .is_empty()
    );
    assert_eq!(
        e.directorio(&ver(sin_ubicar), FiltroDirectorio::default())
            .await,
        vec![tienda],
        "sin coordenadas se compara el municipio"
    );
    assert!(
        e.directorio(&ver(medellin), FiltroDirectorio::default())
            .await
            .is_empty()
    );

    let f = uc::ficha_directorio(&e.unidad, &RelojSistema, &ver(cerca), tienda)
        .await
        .expect("ficha");
    assert_eq!(f.distancia_km, Some(dec!(1.1)));
    assert_eq!(
        f.contacto.whatsapp.map(|t| t.as_str().to_owned()),
        Some("573001234567".into())
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_distancia_en_sql_coincide_con_la_del_dominio(pool: PgPool) {
    let puntos = [
        (
            (dec!(4.598056), dec!(-74.075833)),
            (dec!(6.250000), dec!(-75.568000)),
        ),
        (
            (dec!(4.650000), dec!(-74.060000)),
            (dec!(4.660000), dec!(-74.050000)),
        ),
        (
            (dec!(10.391049), dec!(-75.479426)),
            (dec!(3.451647), dec!(-76.532013)),
        ),
    ];
    for ((la1, lo1), (la2, lo2)) in puntos {
        let sql: f64 = sqlx::query_scalar("SELECT distancia_km($1, $2, $3, $4)")
            .bind(la1)
            .bind(lo1)
            .bind(la2)
            .bind(lo2)
            .fetch_one(&pool)
            .await
            .expect("distancia");
        let a = Coordenadas::new(la1, lo1).expect("válidas");
        let b = Coordenadas::new(la2, lo2).expect("válidas");
        assert!((sql - a.distancia_km(b)).abs() < 1e-6, "{sql} vs dominio");
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn cobertura_municipal_y_nacional(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let contador = e
        .anunciante(
            "900123456-8",
            "Contadores Asociados",
            ubicacion(None, Some("11001")),
            ficha(&["contadores"], Cobertura::Municipio),
        )
        .await;
    let abogada = e
        .anunciante(
            "800197268-4",
            "Abogada en línea",
            Ubicacion::default(),
            ficha(&["abogados"], Cobertura::Nacional),
        )
        .await;
    let bogota = ph(&pool, None, Some("11001")).await;
    let cali = ph(&pool, None, Some("76001")).await;

    let mut en_bogota = e
        .directorio(&Contexto::new(bogota), FiltroDirectorio::default())
        .await;
    en_bogota.sort();
    let mut esperado = vec![contador, abogada];
    esperado.sort();
    assert_eq!(en_bogota, esperado);
    assert_eq!(
        e.directorio(&Contexto::new(cali), FiltroDirectorio::default())
            .await,
        vec![abogada]
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn busca_por_sinonimos_y_por_rama(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let costurera = e
        .anunciante(
            "900123456-8",
            "Rosa Pérez",
            Ubicacion::default(),
            ficha(&["costura"], Cobertura::Nacional),
        )
        .await;
    e.anunciante(
        "800197268-4",
        "Plomería Express",
        Ubicacion::default(),
        ficha(&["plomeria"], Cobertura::Nacional),
    )
    .await;
    let ctx = Contexto::new(ph(&pool, None, None).await);
    let buscar = |texto: &str| FiltroDirectorio {
        texto: Some(texto.into()),
        ..FiltroDirectorio::default()
    };

    assert_eq!(e.directorio(&ctx, buscar("MODISTA")).await, vec![costurera]);
    assert_eq!(
        e.directorio(&ctx, buscar("busco costurera")).await,
        vec![costurera]
    );
    assert_eq!(
        e.directorio(&ctx, buscar("modistería")).await,
        vec![costurera]
    );
    assert!(e.directorio(&ctx, buscar("cerrajero")).await.is_empty());
    let rama = FiltroDirectorio {
        categoria: Some(CodigoCategoria::new("oficios").expect("válido")),
        ..FiltroDirectorio::default()
    };
    assert_eq!(e.directorio(&ctx, rama).await, vec![costurera]);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_categoria_restringida_nunca_entra_a_una_ficha(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let ctx = ContextoPlataforma::sistema();
    let a = uc::dar_de_alta_anunciante(
        &e.plataforma,
        &RelojSistema,
        &ctx,
        nueva_organizacion("900123456-8", "La Licorera"),
        DatosAnunciante {
            nombre_comercial: "La Licorera".into(),
            direccion: None,
            ubicacion: Ubicacion::default(),
        },
    )
    .await
    .expect("alta");

    let r = uc::guardar_ficha(
        &e.plataforma,
        &RelojSistema,
        &ctx,
        a.id,
        ficha(&["tienda", "licores"], Cobertura::Nacional),
    )
    .await;
    assert!(matches!(r, Err(AppError::Dominio(_))), "{r:?}");

    // Ni con un INSERT directo (trigger de la BD).
    uc::guardar_ficha(
        &e.plataforma,
        &RelojSistema,
        &ctx,
        a.id,
        ficha(&["tienda"], Cobertura::Nacional),
    )
    .await
    .expect("ficha válida");
    let err = sqlx::query(
        "INSERT INTO ficha_categorias (anunciante_id, categoria, orden) VALUES ($1, 'licores', 1)",
    )
    .bind(a.id.as_uuid())
    .execute(&pool)
    .await
    .expect_err("el trigger la rechaza");
    assert!(err.to_string().contains("restringida"), "{err}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn suspendido_sale_del_directorio_y_vuelve_al_reactivarlo(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let id = e
        .anunciante(
            "900123456-8",
            "Técnico Lavadoras",
            Ubicacion::default(),
            ficha(&["tecnico_lavadoras"], Cobertura::Nacional),
        )
        .await;
    let ctx_ph = Contexto::new(ph(&pool, None, None).await);
    let ctx = ContextoPlataforma::sistema();

    let sin_motivo = uc::cambiar_verificacion(
        &e.plataforma,
        &RelojSistema,
        &ctx,
        id,
        EstadoVerificacion::Suspendido,
        None,
    )
    .await;
    assert!(matches!(sin_motivo, Err(AppError::Validacion(_))));
    uc::cambiar_verificacion(
        &e.plataforma,
        &RelojSistema,
        &ctx,
        id,
        EstadoVerificacion::Suspendido,
        Some("Quejas por publicidad engañosa"),
    )
    .await
    .expect("suspendido");
    assert!(
        e.directorio(&ctx_ph, FiltroDirectorio::default())
            .await
            .is_empty()
    );

    uc::cambiar_verificacion(
        &e.plataforma,
        &RelojSistema,
        &ctx,
        id,
        EstadoVerificacion::Verificado,
        None,
    )
    .await
    .expect("reactivado");
    assert_eq!(
        e.directorio(&ctx_ph, FiltroDirectorio::default()).await,
        vec![id]
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_organizacion_existente_se_reutiliza_por_nit_y_queda_como_anunciante(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let ctx = ContextoPlataforma::sistema();
    let datos = || DatosAnunciante {
        nombre_comercial: "Aseo Total".into(),
        direccion: None,
        ubicacion: Ubicacion::default(),
    };
    let primero = uc::dar_de_alta_anunciante(
        &e.plataforma,
        &RelojSistema,
        &ctx,
        nueva_organizacion("900123456-8", "Aseo Total S.A.S."),
        datos(),
    )
    .await
    .expect("alta");

    let otra_vez = uc::dar_de_alta_anunciante(
        &e.plataforma,
        &RelojSistema,
        &ctx,
        nueva_organizacion("900.123.456-8", "Otro nombre"),
        datos(),
    )
    .await;

    assert!(
        matches!(otra_vez, Err(AppError::Conflicto(_))),
        "{otra_vez:?}"
    );
    let tipos: Vec<String> = sqlx::query_scalar("SELECT tipos FROM organizaciones WHERE id = $1")
        .bind(primero.organizacion_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("tipos");
    assert!(tipos.contains(&"anunciante".to_owned()));
    assert_eq!(primero.estado, EstadoVerificacion::Verificado);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn resenas_por_ph_con_promedio_global_y_moderacion(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let id = e
        .anunciante(
            "900123456-8",
            "Modistería Rosa",
            Ubicacion::default(),
            ficha(&["costura"], Cobertura::Nacional),
        )
        .await;
    let (ph_a, ph_b) = (ph(&pool, None, None).await, ph(&pool, None, None).await);
    let ana = cuenta_con_rol(&pool, ph_a, "residente", "residencia").await;
    let beto = cuenta_con_rol(&pool, ph_b, "propietario", "propiedad").await;
    let admin = cuenta_con_rol(&pool, ph_a, "administrador", "manual").await;

    let resena_a = uc::resenar(
        &e.unidad,
        &RelojSistema,
        &ctx_residente(ph_a, ana),
        id,
        5,
        Some("Excelente"),
    )
    .await
    .expect("reseña en A");
    uc::resenar(
        &e.unidad,
        &RelojSistema,
        &ctx_residente(ph_b, beto),
        id,
        2,
        Some("Demorada"),
    )
    .await
    .expect("reseña en B");
    let del_admin = uc::resenar(
        &e.unidad,
        &RelojSistema,
        &ctx_residente(ph_a, admin),
        id,
        1,
        None,
    )
    .await;
    assert!(
        matches!(del_admin, Err(AppError::SinPermiso(_))),
        "el administrador no es residente ni propietario"
    );

    // Promedio de las dos PH; el texto solo en la propia.
    let en_a = uc::ficha_directorio(&e.unidad, &RelojSistema, &Contexto::new(ph_a), id)
        .await
        .expect("ficha");
    assert_eq!(en_a.calificacion.promedio, Some(dec!(3.5)));
    assert_eq!(en_a.calificacion.total, 2);
    let textos_a: Vec<_> = uc::resenas_de_anunciante(
        &e.unidad,
        &RelojSistema,
        &Contexto::new(ph_a),
        id,
        SolicitudPagina::primera(),
    )
    .await
    .expect("reseñas")
    .datos
    .into_iter()
    .filter_map(|r| r.texto)
    .collect();
    assert_eq!(textos_a, vec!["Excelente".to_owned()]);

    // Reporte → cola de moderación → ocultar.
    let vecino = cuenta_con_rol(&pool, ph_a, "residente", "residencia").await;
    assert!(
        uc::reportar_resena(
            &e.unidad,
            &RelojSistema,
            &ctx_residente(ph_a, vecino),
            resena_a.id,
            "Falsa"
        )
        .await
        .expect("reporte")
    );
    let moderacion = ContextoPlataforma::sistema();
    let cola = uc::resenas_reportadas(&e.plataforma, &moderacion, SolicitudPagina::primera())
        .await
        .expect("cola");
    assert_eq!(cola.datos.len(), 1);
    assert_eq!(cola.datos[0].tenant_id, ph_a);
    assert_eq!(cola.datos[0].motivos, vec!["Falsa".to_owned()]);
    uc::moderar_resena(
        &e.unidad,
        &RelojSistema,
        &moderacion,
        ph_a,
        resena_a.id,
        DecisionModeracion::Ocultar {
            motivo: "No es verificable".into(),
        },
    )
    .await
    .expect("moderada");

    let cola = uc::resenas_reportadas(&e.plataforma, &moderacion, SolicitudPagina::primera())
        .await
        .expect("cola");
    assert!(cola.datos.is_empty());
    let en_b = uc::ficha_directorio(&e.unidad, &RelojSistema, &Contexto::new(ph_b), id)
        .await
        .expect("ficha");
    assert_eq!(
        en_b.calificacion.promedio,
        Some(dec!(2.0)),
        "la oculta ya no cuenta"
    );
}

async fn destacadas(e: &Entorno, tenant: TenantId) -> Vec<AnuncianteId> {
    uc::destacadas(&e.unidad, &RelojSistema, &Contexto::new(tenant), None)
        .await
        .expect("destacadas")
        .into_iter()
        .map(|f| f.anunciante_id)
        .collect()
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn las_destacadas_solo_salen_con_el_modulo_de_publicidad(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let mut datos = ficha(&["restaurante"], Cobertura::Nacional);
    datos.destacada_hasta = Some(chrono::Utc::now() + chrono::Duration::days(30));
    let id = e
        .anunciante(
            "900123456-8",
            "Corrientazo Doña Ana",
            Ubicacion::default(),
            datos,
        )
        .await;
    let sin_modulo = ph(&pool, None, None).await;
    let con_modulo = ph(&pool, None, None).await;
    sqlx::query(
        "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
         VALUES (gen_random_uuid(), $1, 'publicidad', now() - interval '1 day')",
    )
    .bind(con_modulo.as_uuid())
    .execute(&pool)
    .await
    .expect("módulo");

    assert!(destacadas(&e, sin_modulo).await.is_empty());
    assert_eq!(destacadas(&e, con_modulo).await, vec![id]);
    let ficha_sin = uc::ficha_directorio(&e.unidad, &RelojSistema, &Contexto::new(sin_modulo), id)
        .await
        .expect("ficha");
    assert!(!ficha_sin.destacada, "sin el módulo no hay pauta");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn las_tablas_de_plataforma_no_se_ven_desde_una_copropiedad(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    e.anunciante(
        "900123456-8",
        "Modistería Rosa",
        Ubicacion::default(),
        ficha(&["costura"], Cobertura::Nacional),
    )
    .await;
    let tenant = ph(&pool, None, None).await;

    let mut tx = e.app.begin().await.expect("tx");
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant");
    for consulta in [
        "SELECT count(*) FROM anunciantes",
        "SELECT count(*) FROM fichas_directorio",
        "SELECT count(*) FROM ficha_categorias",
    ] {
        let n: i64 = sqlx::query_scalar(consulta)
            .fetch_one(&mut *tx)
            .await
            .expect("conteo");
        assert_eq!(n, 0, "{consulta}: visible desde la copropiedad");
    }
    let cola: i64 = sqlx::query_scalar("SELECT count(*) FROM resenas_reportadas(NULL, 10)")
        .fetch_one(&mut *tx)
        .await
        .expect("cola");
    assert_eq!(cola, 0, "la cola de moderación es solo de plataforma");
    let categorias: i64 = sqlx::query_scalar("SELECT count(*) FROM categorias_publicidad")
        .fetch_one(&mut *tx)
        .await
        .expect("catálogo");
    assert!(categorias > 0, "el catálogo se lee desde cualquier lado");
    let escribir = sqlx::query(
        "INSERT INTO categorias_publicidad (codigo, nombre) VALUES ('pirata', 'Pirata')",
    )
    .execute(&mut *tx)
    .await;
    assert!(escribir.is_err(), "solo la plataforma escribe el catálogo");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn las_resenas_de_una_ph_no_se_ven_en_otra(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let id = e
        .anunciante(
            "900123456-8",
            "Modistería Rosa",
            Ubicacion::default(),
            ficha(&["costura"], Cobertura::Nacional),
        )
        .await;
    let (ph_a, ph_b) = (ph(&pool, None, None).await, ph(&pool, None, None).await);
    let ana = cuenta_con_rol(&pool, ph_a, "residente", "residencia").await;
    let r = uc::resenar(
        &e.unidad,
        &RelojSistema,
        &ctx_residente(ph_a, ana),
        id,
        4,
        Some("Bien"),
    )
    .await
    .expect("reseña");

    let reportar_desde_b = uc::reportar_resena(
        &e.unidad,
        &RelojSistema,
        &ctx_residente(ph_b, CuentaId::nuevo()),
        r.id,
        "Falsa",
    )
    .await;
    assert!(matches!(reportar_desde_b, Err(AppError::NoEncontrado(_))));
    let en_b = uc::resenas_de_anunciante(
        &e.unidad,
        &RelojSistema,
        &Contexto::new(ph_b),
        id,
        SolicitudPagina::primera(),
    )
    .await
    .expect("reseñas");
    assert!(en_b.datos.is_empty(), "el texto de A no sale en B");
}
