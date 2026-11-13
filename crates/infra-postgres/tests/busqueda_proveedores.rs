//! Integración de la búsqueda de proveedores (plan 08, fase 2) sobre PostgreSQL como `dad_app`:
//! búsqueda por sinónimos contra el SQL real, solicitud idempotente con RLS, contacto
//! autorizado al outbox (`concepto = 'directorio'`, seguimiento diferido), contactos
//! append-only, purga del texto y enlace público con el número de la línea.

mod comun;

use application::{
    AppError, Contexto, ContextoPlataforma, SolicitudPagina,
    casos_uso::{
        busqueda_proveedores::{
            self as uc, BotonOpinion, NuevaBusqueda, PLANTILLA_CONTACTO_PROVEEDOR,
        },
        directorio::{self as dir, DatosAnunciante, DatosFicha},
    },
    puertos::{
        PurgaDirectorio, RelojSistema,
        directorio::{Contacto, FiltroDirectorio, OrganizacionAnunciante},
        identidad::NuevaOrganizacion,
    },
};
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};
use domain::{
    AnuncianteId, CanalSolicitud, Cobertura, CodigoCategoria, CodigoDane, Coordenadas,
    NaturalezaOrganizacion, Nit, PersonaId, Telefono, TenantId, TipoOrganizacion, Ubicacion,
};
use infra_postgres::{
    PurgaDirectorioPg, UnidadDeTrabajoPg, UnidadDeTrabajoPlataformaPg, transaccion_tenant,
};
use rust_decimal::dec;
use sqlx::PgPool;

const CELULAR: &str = "573001112233";

struct Entorno {
    owner: PgPool,
    app: PgPool,
    unidad: UnidadDeTrabajoPg,
    tenant: TenantId,
    persona: PersonaId,
}

impl Entorno {
    /// PH en Chapinero (Bogotá) con una residente que tiene celular en el censo.
    async fn new(pool: &PgPool) -> Self {
        let app = pool_como_app(pool).await;
        let tenant = ph(pool).await;
        let persona = persona(pool, tenant).await;
        Self {
            owner: pool.clone(),
            unidad: UnidadDeTrabajoPg::new(app.clone()),
            app,
            tenant,
            persona,
        }
    }

    /// Anunciante verificado con ficha publicada que cubre 5 km alrededor de la PH.
    async fn anunciante(
        &self,
        nit: &str,
        nombre: &str,
        cat: &str,
        whatsapp: Option<&str>,
    ) -> AnuncianteId {
        let plataforma = UnidadDeTrabajoPlataformaPg::new(self.app.clone());
        let ctx = ContextoPlataforma::sistema();
        let a = dir::dar_de_alta_anunciante(
            &plataforma,
            &RelojSistema,
            &ctx,
            OrganizacionAnunciante::Nueva(NuevaOrganizacion {
                nit: Nit::new(nit).expect("NIT válido"),
                razon_social: nombre.into(),
                naturaleza: NaturalezaOrganizacion::NaturalConRut,
                tipos: vec![TipoOrganizacion::Otro],
                correo_contacto: None,
                telefono_contacto: None,
            }),
            DatosAnunciante {
                nombre_comercial: nombre.into(),
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
        dir::guardar_ficha(
            &plataforma,
            &RelojSistema,
            &ctx,
            a.id,
            DatosFicha {
                descripcion: None,
                categorias: vec![CodigoCategoria::new(cat).expect("válido")],
                cobertura: Cobertura::Radio(5),
                contacto: Contacto {
                    whatsapp: whatsapp.map(|w| Telefono::new(w).expect("válido")),
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

    fn ctx(&self) -> Contexto {
        Contexto::new(self.tenant)
    }

    async fn buscar(&self, texto: &str, clave: &str) -> uc::ResultadoBusqueda {
        uc::buscar_proveedores(
            &self.unidad,
            &RelojSistema,
            &self.ctx(),
            NuevaBusqueda {
                persona_id: self.persona,
                canal: CanalSolicitud::Whatsapp,
                texto: texto.into(),
                clave: clave.into(),
            },
        )
        .await
        .expect("busca")
    }

    /// Cuenta filas de una tabla de la PH como `dad_app` dentro de `tenant`.
    async fn contar(&self, tenant: TenantId, sql: &'static str) -> i64 {
        let mut tx = transaccion_tenant(&self.app, tenant).await.expect("tx");
        sqlx::query_scalar(sql)
            .fetch_one(&mut *tx)
            .await
            .expect("conteo")
    }
}

async fn ph(pool: &PgPool) -> TenantId {
    let tenant = crear_copropiedad(pool).await;
    sqlx::query(
        "UPDATE entidades SET estado = 'activa', latitud = 4.650000, longitud = -74.060000,
                municipio_dane = '11001'
         WHERE id = $1",
    )
    .bind(tenant.as_uuid())
    .execute(pool)
    .await
    .expect("ubicación");
    tenant
}

async fn persona(pool: &PgPool, tenant: TenantId) -> PersonaId {
    let p = PersonaId::nuevo();
    sqlx::query(
        "INSERT INTO personas (id, tenant_id, nombre, telefono) VALUES ($1, $2, 'Ana Pérez', $3)",
    )
    .bind(p.as_uuid())
    .bind(tenant.as_uuid())
    .bind(CELULAR)
    .execute(pool)
    .await
    .expect("persona");
    p
}

async fn linea(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO lineas_whatsapp (id, phone_number_id, numero_visible, nombre)
         VALUES (gen_random_uuid(), 'PNID', '+57 300 000 0000', 'Línea')",
    )
    .execute(pool)
    .await
    .expect("línea");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn busco_costurera_encuentra_por_sinonimo_y_queda_en_la_ph(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let rosa = e
        .anunciante(
            "900123456-8",
            "Modistería Rosa",
            "costura",
            Some("573109998877"),
        )
        .await;
    e.anunciante("800197268-4", "Plomería Ya", "plomeria", None)
        .await;

    let r = e.buscar("¿Alguien conoce una costurera?", "wamid.1").await;

    assert_eq!(
        r.solicitud.categoria.as_ref().map(CodigoCategoria::as_str),
        Some("costura")
    );
    assert_eq!(
        r.sugerencias
            .iter()
            .map(|f| f.anunciante_id)
            .collect::<Vec<_>>(),
        [rosa]
    );
    assert_eq!(r.sugerencias[0].distancia_km, Some(dec!(0.6)));
    let otra = ph(&pool).await;
    assert_eq!(
        e.contar(e.tenant, "SELECT count(*) FROM solicitudes_proveedor")
            .await,
        1
    );
    assert_eq!(
        e.contar(otra, "SELECT count(*) FROM solicitudes_proveedor")
            .await,
        0,
        "RLS: otra PH no ve la búsqueda"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_categoria_busca_por_texto_en_el_sql(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let espiga = e
        .anunciante("900123456-8", "Panadería La Espiga", "panaderia", None)
        .await;

    let r = e.buscar("busco la espiga", "w1").await;

    assert_eq!(r.solicitud.categoria, None);
    assert_eq!(r.sugerencias[0].anunciante_id, espiga);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn reintentar_la_busqueda_no_duplica_la_fila(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    e.anunciante("900123456-8", "Modistería Rosa", "costura", None)
        .await;

    let a = e.buscar("busco costurera", "wamid.1").await;
    let b = e.buscar("busco costurera", "wamid.1").await;

    assert_eq!(a.solicitud.id, b.solicitud.id);
    assert_eq!(b.sugerencias.len(), 1);
    assert_eq!(
        e.contar(e.tenant, "SELECT count(*) FROM solicitudes_proveedor")
            .await,
        1
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_contacto_autorizado_va_al_outbox_y_el_seguimiento_queda_diferido(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    linea(&pool).await;
    let rosa = e
        .anunciante(
            "900123456-8",
            "Modistería Rosa",
            "costura",
            Some("573109998877"),
        )
        .await;
    let r = e.buscar("busco costurera", "wamid.1").await;
    assert_eq!(
        e.contar(e.tenant, "SELECT count(*) FROM outbox").await,
        0,
        "buscar no comparte nada"
    );

    let c = uc::autorizar_contacto(
        &e.unidad,
        &RelojSistema,
        &e.ctx(),
        r.solicitud.id,
        e.persona,
        None,
    )
    .await
    .expect("autoriza");

    assert_eq!(c.proveedores, ["Modistería Rosa"]);
    let filas: Vec<(String, i16, String, bool)> = sqlx::query_as(
        "SELECT destino, prioridad, payload -> 'contenido' ->> 'nombre',
                disponible_en > now() + interval '6 days'
         FROM outbox WHERE tenant_id = $1 AND concepto = 'directorio' ORDER BY prioridad",
    )
    .bind(e.tenant.as_uuid())
    .fetch_all(&pool)
    .await
    .expect("outbox");
    assert_eq!(
        filas,
        [
            (
                "573109998877".to_owned(),
                2,
                PLANTILLA_CONTACTO_PROVEEDOR.to_owned(),
                false
            ),
            (CELULAR.to_owned(), 3, "opinion_proveedor".to_owned(), true),
        ]
    );
    let contactados = uc::proveedores_contactados(
        &e.unidad,
        &RelojSistema,
        &e.ctx(),
        r.solicitud.id,
        e.persona,
    )
    .await
    .expect("contactados");
    assert_eq!(contactados[0].anunciante_id, rosa);
    let boton: String = sqlx::query_scalar(
        "SELECT payload -> 'contenido' -> 'botones' ->> 0 FROM outbox
         WHERE tenant_id = $1 AND prioridad = 3",
    )
    .bind(e.tenant.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("botón");
    assert_eq!(
        BotonOpinion::desde_id(&boton).map(|b| b.solicitud),
        Some(r.solicitud.id)
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn los_contactos_compartidos_son_append_only(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    linea(&pool).await;
    e.anunciante(
        "900123456-8",
        "Modistería Rosa",
        "costura",
        Some("573109998877"),
    )
    .await;
    let r = e.buscar("busco costurera", "wamid.1").await;
    uc::autorizar_contacto(
        &e.unidad,
        &RelojSistema,
        &e.ctx(),
        r.solicitud.id,
        e.persona,
        None,
    )
    .await
    .expect("autoriza");

    for sql in [
        "UPDATE contactos_proveedor SET creado_en = now()",
        "DELETE FROM contactos_proveedor",
        "UPDATE solicitudes_proveedor SET persona_id = persona_id",
        "DELETE FROM solicitudes_proveedor",
    ] {
        let mut tx = transaccion_tenant(&e.app, e.tenant).await.expect("tx");
        let r = sqlx::query(sql).execute(&mut *tx).await;
        assert!(r.is_err(), "dad_app no puede: {sql}");
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_purga_borra_solo_el_texto_de_las_busquedas_viejas(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    let vieja = e.buscar("busco costurera para un vestido", "w1").await;
    let nueva = e.buscar("busco plomero", "w2").await;
    sqlx::query(
        "UPDATE solicitudes_proveedor SET creada_en = now() - interval '100 days' WHERE id = $1",
    )
    .bind(vieja.solicitud.id.as_uuid())
    .execute(&e.owner)
    .await
    .expect("envejece");

    let n = PurgaDirectorioPg::new(e.app.clone())
        .purgar_solicitudes_proveedor(90)
        .await
        .expect("purga");

    assert_eq!(n, 1);
    let filas: Vec<(uuid::Uuid, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT id, texto, categoria FROM solicitudes_proveedor ORDER BY creada_en")
            .fetch_all(&e.owner)
            .await
            .expect("filas");
    assert_eq!(
        filas,
        [
            (vieja.solicitud.id.as_uuid(), None, Some("costura".into())),
            (
                nueva.solicitud.id.as_uuid(),
                Some("busco plomero".into()),
                Some("plomeria".into())
            ),
        ]
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_enlace_publico_abre_solo_el_directorio_de_su_ph(pool: PgPool) {
    let e = Entorno::new(&pool).await;
    linea(&pool).await;
    e.anunciante("900123456-8", "Modistería Rosa", "costura", None)
        .await;
    let otra = ph(&pool).await;

    let enlace = uc::generar_enlace_directorio(&e.unidad, &RelojSistema, &e.ctx())
        .await
        .expect("genera");
    let abrir = |tenant: TenantId, token: String| {
        let unidad = &e.unidad;
        async move {
            uc::directorio_publico(
                unidad,
                &RelojSistema,
                tenant,
                &token,
                FiltroDirectorio::default(),
                SolicitudPagina::primera(),
            )
            .await
        }
    };

    let d = abrir(e.tenant, enlace.token.clone())
        .await
        .expect("público");
    assert_eq!(d.copropiedad, "PH de prueba");
    assert_eq!(d.fichas.datos.len(), 1);
    assert_eq!(
        d.whatsapp.as_deref(),
        Some("https://wa.me/573000000000?text=Busco%20")
    );
    assert!(matches!(
        abrir(otra, enlace.token.clone()).await,
        Err(AppError::NoAutenticado)
    ));
    let hash_len: i32 =
        sqlx::query_scalar("SELECT octet_length(token_hash) FROM enlaces_directorio")
            .fetch_one(&e.owner)
            .await
            .expect("hash");
    assert_eq!(hash_len, 32);

    assert!(
        uc::revocar_enlace_directorio(&e.unidad, &e.ctx())
            .await
            .expect("revoca")
    );
    assert!(matches!(
        abrir(e.tenant, enlace.token).await,
        Err(AppError::NoAutenticado)
    ));
}
