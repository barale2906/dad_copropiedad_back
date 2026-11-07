//! Pruebas de integración de identidad, roles y organizaciones: los casos de uso reales sobre
//! PostgreSQL con el rol `dad_app`.

use std::collections::BTreeSet;

use application::{
    AppError, Contexto, ContextoPlataforma,
    casos_uso::{
        autenticacion::{
            Registro, ResultadoInicio, SolicitudInicio, TokensSesion, autenticar, iniciar_sesion,
            mis_copropiedades, registrar_cuenta, renovar_sesion,
        },
        organizaciones::{
            NuevoContrato, asignar_rol_por_contrato, crear_organizacion, terminar_contrato,
            vincular_organizacion,
        },
        plataforma::{asignar_rol_plataforma, crear_copropiedad},
        roles::{NuevoRol, SolicitudAsignacion, asignar_rol, crear_rol, permisos_en_copropiedad},
    },
    puertos::{
        HashContrasenas, RelojSistema, RepositorioPlataforma, RepositorioRoles, UnidadDeTrabajo,
        UnidadDeTrabajoPlataforma, copropiedades::AltaCopropiedad, identidad::NuevaOrganizacion,
    },
};
use domain::{
    Ambito, ContrasenaNueva, CuentaId, DireccionCorreo, Identificador, NaturalezaOrganizacion, Nit,
    NombreUsuario, Permiso, TenantId, TipoOrganizacion,
    permisos::{CODIGO_SUPERUSUARIO, ROLES_PLANTILLA},
};
use infra_postgres::{UnidadDeTrabajoPg, UnidadDeTrabajoPlataformaPg};
use sqlx::PgPool;

mod comun;
use comun::{MIGRACIONES, crear_copropiedad as crear_ph, pool_como_app};

/// Hash de pruebas (rápido y determinista); el real es Argon2id en `infra-seguridad`.
struct HashPrueba;

impl HashContrasenas for HashPrueba {
    async fn hash(&self, c: &ContrasenaNueva) -> application::AppResult<String> {
        Ok(format!("prueba:{}", c.expose()))
    }

    async fn verificar(&self, c: &str, hash: &str) -> bool {
        hash == format!("prueba:{c}")
    }

    async fn igualar_tiempo(&self, _c: &str) {}
}

struct Entorno {
    pool: PgPool,
    plataforma: UnidadDeTrabajoPlataformaPg,
    unidad: UnidadDeTrabajoPg,
}

async fn entorno(pool: PgPool) -> Entorno {
    let app = pool_como_app(&pool).await;
    Entorno {
        pool,
        plataforma: UnidadDeTrabajoPlataformaPg::new(app.clone()),
        unidad: UnidadDeTrabajoPg::new(app),
    }
}

const CONTRASENA: &str = "una frase bien larga";

async fn registrar(e: &Entorno, usuario: &str) -> CuentaId {
    registrar_cuenta(
        &e.plataforma,
        &HashPrueba,
        &RelojSistema,
        &ContextoPlataforma::anonimo(None),
        Registro {
            nombre_mostrar: format!("Persona {usuario}"),
            usuario: Some(NombreUsuario::new(usuario).expect("válido")),
            correo: Some(DireccionCorreo::new(&format!("{usuario}@correo.co")).expect("válido")),
            contrasena: ContrasenaNueva::new(CONTRASENA).expect("válida"),
        },
    )
    .await
    .expect("registra")
}

async fn iniciar(e: &Entorno, usuario: &str, contrasena: &str) -> Result<TokensSesion, AppError> {
    let r = iniciar_sesion(
        &e.plataforma,
        &HashPrueba,
        &RelojSistema,
        &ContextoPlataforma::anonimo(None),
        SolicitudInicio {
            identificador: Identificador::parse(usuario).expect("válido"),
            contrasena: contrasena.into(),
            equipo_compartido: false,
            agente: None,
        },
    )
    .await?;
    match r {
        ResultadoInicio::Sesion(t) => Ok(t),
        ResultadoInicio::SegundoFactor { .. } => panic!("sin segundo factor en esta prueba"),
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn registro_inicio_de_sesion_y_autenticacion(pool: PgPool) {
    let e = entorno(pool).await;
    let cuenta = registrar(&e, "ana").await;

    let por_usuario = iniciar(&e, "ana", CONTRASENA).await.expect("inicia");
    let por_correo = iniciar(&e, "ANA@correo.co", CONTRASENA)
        .await
        .expect("inicia");
    let sesion = autenticar(&e.plataforma, &RelojSistema, &por_usuario.acceso)
        .await
        .expect("autentica");

    assert_eq!(sesion.cuenta_id, cuenta);
    assert!(!sesion.es_superusuario);
    assert_ne!(por_usuario.acceso, por_correo.acceso);
    assert!(matches!(
        iniciar(&e, "ana", "otra contraseña larga").await,
        Err(AppError::CredencialesInvalidas)
    ));
    assert!(matches!(
        autenticar(&e.plataforma, &RelojSistema, "token-inventado").await,
        Err(AppError::NoAutenticado)
    ));
    // El código de verificación salió por el outbox de plataforma (sin copropiedad).
    let (correos, guardado_en_claro): (i64, bool) = sqlx::query_as(
        "SELECT count(*), bool_or(payload::text LIKE '%' || $1 || '%')
         FROM outbox WHERE tenant_id IS NULL AND canal = 'correo'",
    )
    .bind(&por_usuario.acceso)
    .fetch_one(&e.pool)
    .await
    .expect("outbox");
    assert_eq!(correos, 1);
    assert!(!guardado_en_claro);
    // Solo hashes en la base.
    let tokens_en_claro: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sesiones WHERE hash_acceso = $1 OR hash_refresh = $2",
    )
    .bind(&por_usuario.acceso)
    .bind(&por_usuario.refresh)
    .fetch_one(&e.pool)
    .await
    .expect("conteo");
    assert_eq!(tokens_en_claro, 0);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn usuario_y_correo_son_unicos(pool: PgPool) {
    let e = entorno(pool).await;
    registrar(&e, "ana").await;

    let repetido = registrar_cuenta(
        &e.plataforma,
        &HashPrueba,
        &RelojSistema,
        &ContextoPlataforma::anonimo(None),
        Registro {
            nombre_mostrar: "Otra Ana".into(),
            usuario: Some(NombreUsuario::new("ana").expect("válido")),
            correo: None,
            contrasena: ContrasenaNueva::new(CONTRASENA).expect("válida"),
        },
    )
    .await;

    assert!(matches!(repetido, Err(AppError::Conflicto(_))));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn cinco_fallos_bloquean_la_cuenta_aunque_luego_acierte(pool: PgPool) {
    let e = entorno(pool).await;
    registrar(&e, "ana").await;

    for _ in 0..5 {
        assert!(matches!(
            iniciar(&e, "ana", "contraseña equivocada").await,
            Err(AppError::CredencialesInvalidas)
        ));
    }

    assert!(matches!(
        iniciar(&e, "ana", CONTRASENA).await,
        Err(AppError::DemasiadosIntentos)
    ));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn refresh_rota_y_su_reutilizacion_revoca_la_sesion(pool: PgPool) {
    let e = entorno(pool).await;
    registrar(&e, "ana").await;
    let primera = iniciar(&e, "ana", CONTRASENA).await.expect("inicia");
    let ctx = ContextoPlataforma::anonimo(None);

    let segunda = renovar_sesion(&e.plataforma, &RelojSistema, &ctx, &primera.refresh)
        .await
        .expect("renueva");
    assert_eq!(segunda.sesion_id, primera.sesion_id);
    assert!(
        autenticar(&e.plataforma, &RelojSistema, &segunda.acceso)
            .await
            .is_ok()
    );
    assert!(
        autenticar(&e.plataforma, &RelojSistema, &primera.acceso)
            .await
            .is_err(),
        "el acceso anterior deja de servir"
    );

    // Alguien reutiliza el refresh viejo: se revoca toda la sesión.
    assert!(matches!(
        renovar_sesion(&e.plataforma, &RelojSistema, &ctx, &primera.refresh).await,
        Err(AppError::NoAutenticado)
    ));
    assert!(
        autenticar(&e.plataforma, &RelojSistema, &segunda.acceso)
            .await
            .is_err()
    );
    let auditada: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM auditoria_plataforma WHERE accion = 'cuentas.sesion.reutilizacion_detectada'",
    )
    .fetch_one(&e.pool)
    .await
    .expect("auditoría");
    assert_eq!(auditada, 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_transaccion_de_copropiedad_no_ve_cuentas_ni_sesiones(pool: PgPool) {
    let e = entorno(pool).await;
    registrar(&e, "ana").await;
    iniciar(&e, "ana", CONTRASENA).await.expect("inicia");
    let tenant = crear_ph(&e.pool).await;

    let mut tx = infra_postgres::transaccion_tenant(&pool_como_app(&e.pool).await, tenant)
        .await
        .expect("tx");
    let (cuentas, sesiones): (i64, i64) =
        sqlx::query_as("SELECT (SELECT count(*) FROM cuentas), (SELECT count(*) FROM sesiones)")
            .fetch_one(&mut *tx)
            .await
            .expect("conteo");

    assert_eq!((cuentas, sesiones), (0, 0));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn alta_de_copropiedad_deja_administrador_y_membresia(pool: PgPool) {
    let e = entorno(pool).await;
    let admin = registrar(&e, "ana").await;

    let tenant = crear_copropiedad(
        &e.plataforma,
        &e.unidad,
        &RelojSistema,
        &ContextoPlataforma::sistema(),
        AltaCopropiedad::con_nombre("Conjunto Alameda"),
        admin,
    )
    .await
    .expect("crea");

    let ctx = ContextoPlataforma::de_cuenta(admin, Default::default(), None);
    let mias = mis_copropiedades(&e.plataforma, &ctx).await.expect("lista");
    assert_eq!(mias.len(), 1);
    assert_eq!(mias[0].tenant_id, tenant);
    assert_eq!(mias[0].roles, vec!["administrador".to_owned()]);
    let permisos = permisos_en_copropiedad(&e.unidad, &RelojSistema, tenant, admin, false)
        .await
        .expect("permisos");
    assert!(permisos.contiene(Permiso::RolesAsignar));
    let otra = permisos_en_copropiedad(
        &e.unidad,
        &RelojSistema,
        crear_ph(&e.pool).await,
        admin,
        false,
    )
    .await
    .expect("permisos");
    assert!(otra.es_vacio(), "sin roles en otra PH no tiene nada allí");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn superusuario_tiene_todo_en_plataforma(pool: PgPool) {
    let e = entorno(pool).await;
    let cuenta = registrar(&e, "root").await;
    let super_rol = e
        .plataforma
        .iniciar_plataforma(&ContextoPlataforma::sistema())
        .await
        .expect("tx")
        .rol_plataforma_por_codigo(CODIGO_SUPERUSUARIO)
        .await
        .expect("consulta")
        .expect("plantilla");

    asignar_rol_plataforma(
        &e.plataforma,
        &RelojSistema,
        &ContextoPlataforma::sistema(),
        cuenta,
        super_rol.id,
    )
    .await
    .expect("asigna");

    let tokens = iniciar(&e, "root", CONTRASENA).await.expect("inicia");
    let sesion = autenticar(&e.plataforma, &RelojSistema, &tokens.acceso)
        .await
        .expect("autentica");
    assert!(sesion.es_superusuario);
    assert_eq!(
        sesion.permisos_plataforma,
        domain::ConjuntoPermisos::todos(Ambito::Plataforma)
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn roles_propios_solo_se_ven_y_asignan_en_su_copropiedad(pool: PgPool) {
    let e = entorno(pool).await;
    let cuenta = registrar(&e, "ana").await;
    let ph1 = Contexto::new(crear_ph(&e.pool).await);
    let ph2 = Contexto::new(crear_ph(&e.pool).await);
    let rol = crear_rol(
        &e.unidad,
        &ph1,
        NuevoRol {
            codigo: "auxiliar_cartera".into(),
            nombre: "Auxiliar de cartera".into(),
            descripcion: None,
            permisos: [Permiso::ContabilidadVer].into_iter().collect(),
        },
    )
    .await
    .expect("crea");

    let en_ph2 = asignar_rol(
        &e.unidad,
        &RelojSistema,
        &ph2,
        SolicitudAsignacion::manual(cuenta, rol.id),
    )
    .await;
    assert!(matches!(en_ph2, Err(AppError::NoEncontrado("rol"))));
    let roles_ph2 = e
        .unidad
        .iniciar(&ph2)
        .await
        .expect("tx")
        .roles()
        .await
        .expect("roles");
    assert!(roles_ph2.iter().all(|r| r.codigo != "auxiliar_cartera"));

    asignar_rol(
        &e.unidad,
        &RelojSistema,
        &ph1,
        SolicitudAsignacion::manual(cuenta, rol.id),
    )
    .await
    .expect("asigna en su PH");
    // La app no puede tocar plantillas.
    let app = pool_como_app(&e.pool).await;
    let mut tx = infra_postgres::transaccion_tenant(&app, ph1.tenant_id)
        .await
        .expect("tx");
    let cambiadas = sqlx::query("UPDATE roles SET nombre = 'Jefe' WHERE es_plantilla")
        .execute(&mut *tx)
        .await
        .expect("update")
        .rows_affected();
    assert_eq!(cambiadas, 0);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn catalogo_de_permisos_y_plantillas_coinciden_con_el_codigo(pool: PgPool) {
    let en_bd: BTreeSet<(String, String)> = sqlx::query_as("SELECT codigo, ambito FROM permisos")
        .fetch_all(&pool)
        .await
        .expect("permisos")
        .into_iter()
        .collect();
    let en_codigo: BTreeSet<(String, String)> = Permiso::TODOS
        .iter()
        .map(|p| (p.as_str().to_owned(), p.ambito().as_str().to_owned()))
        .collect();
    assert_eq!(en_bd, en_codigo, "sincronice el catálogo con una migración");

    let plantillas: BTreeSet<(String, String, String)> = sqlx::query_as(
        "SELECT r.codigo, r.ambito, rp.permiso FROM roles r JOIN roles_permisos rp ON rp.rol_id = r.id
         WHERE r.es_plantilla",
    )
    .fetch_all(&pool)
    .await
    .expect("plantillas")
    .into_iter()
    .collect();
    let esperadas: BTreeSet<(String, String, String)> = ROLES_PLANTILLA
        .iter()
        .flat_map(|r| {
            r.conjunto()
                .iter()
                .map(move |p| {
                    (
                        r.codigo.to_owned(),
                        r.ambito.as_str().to_owned(),
                        p.as_str().to_owned(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        plantillas, esperadas,
        "sincronice los roles plantilla con una migración"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn organizacion_unica_por_nit_y_contrato_vence_sus_roles(pool: PgPool) {
    let e = entorno(pool).await;
    let vigilante = registrar(&e, "vigilante1").await;
    let ph1 = Contexto::new(crear_ph(&e.pool).await);
    let ph2 = Contexto::new(crear_ph(&e.pool).await);
    let nueva = NuevaOrganizacion {
        nit: Nit::new("900123456").expect("válido"),
        razon_social: "Seguridad Andina S.A.S.".into(),
        naturaleza: NaturalezaOrganizacion::Juridica,
        tipos: vec![TipoOrganizacion::Seguridad],
        correo_contacto: None,
        telefono_contacto: None,
    };

    let (org1, creada1) = crear_organizacion(&e.unidad, &ph1, nueva.clone())
        .await
        .expect("crea");
    let (org2, creada2) = crear_organizacion(&e.unidad, &ph2, nueva)
        .await
        .expect("reutiliza");
    assert_eq!(org1, org2);
    assert_eq!((creada1, creada2), (true, false));

    let contrato = vincular_organizacion(
        &e.unidad,
        &RelojSistema,
        &ph1,
        NuevoContrato {
            organizacion_id: org1,
            tipo_servicio: TipoOrganizacion::Seguridad,
            vigente_desde: None,
            vigente_hasta: None,
            valor: None,
        },
    )
    .await
    .expect("vincula");
    let rol_vigilante = e
        .unidad
        .iniciar(&ph1)
        .await
        .expect("tx")
        .rol_por_codigo("vigilante")
        .await
        .expect("consulta")
        .expect("plantilla");
    asignar_rol_por_contrato(
        &e.unidad,
        &RelojSistema,
        &ph1,
        contrato.id,
        vigilante,
        rol_vigilante.id,
    )
    .await
    .expect("asigna");
    let antes = permisos_en_copropiedad(&e.unidad, &RelojSistema, ph1.tenant_id, vigilante, false)
        .await
        .expect("permisos");
    assert!(antes.contiene(Permiso::AccesoRegistrar));

    let vencidas = terminar_contrato(&e.unidad, &RelojSistema, &ph1, contrato.id)
        .await
        .expect("termina");

    assert_eq!(vencidas, 1);
    let despues =
        permisos_en_copropiedad(&e.unidad, &RelojSistema, ph1.tenant_id, vigilante, false)
            .await
            .expect("permisos");
    assert!(
        despues.es_vacio(),
        "al terminar el contrato el vigilante pierde el acceso"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_rol_de_otra_copropiedad_se_rechaza_tambien_en_la_bd(pool: PgPool) {
    let e = entorno(pool).await;
    let cuenta = registrar(&e, "ana").await;
    let ph1 = Contexto::new(crear_ph(&e.pool).await);
    let ph2: TenantId = crear_ph(&e.pool).await;
    let rol = crear_rol(
        &e.unidad,
        &ph1,
        NuevoRol {
            codigo: "auxiliar".into(),
            nombre: "Auxiliar".into(),
            descripcion: None,
            permisos: [Permiso::TicketsCrear].into_iter().collect(),
        },
    )
    .await
    .expect("crea");

    // Saltándose el caso de uso: el trigger lo impide (como dueño del esquema, sin RLS).
    let err = sqlx::query(
        "INSERT INTO asignaciones_rol (id, tenant_id, cuenta_id, rol_id, vigente_desde, origen)
         VALUES (gen_random_uuid(), $1, $2, $3, now(), 'manual')",
    )
    .bind(ph2.as_uuid())
    .bind(cuenta.as_uuid())
    .bind(rol.id.as_uuid())
    .execute(&e.pool)
    .await
    .expect_err("rol de otra PH");
    assert!(
        err.to_string().contains("no pertenece a la copropiedad"),
        "{err}"
    );
}
