//! Integración del plan 03 sobre PostgreSQL como `dad_app`: casos de uso reales, triggers de
//! coeficientes y titulares, índice de cuadro vigente, RLS y FK compuestas.

mod comun;

use application::{
    AppError, Contexto,
    casos_uso::{
        bienes_privados::{
            NuevaAgrupacion, NuevoBienPrivado, crear_agrupacion, crear_bien_privado,
        },
        coeficientes::{
            NuevoCuadro, RefBienPrivado, activar_cuadro, cargar_coeficientes, crear_cuadro,
        },
        copropiedades::{CambiosCopropiedad, actualizar_datos, ver_ficha},
        titulares::{
            DatosPersona, RefPersona, SolicitudTitular, registrar_titulares, transferir_titularidad,
        },
    },
    puertos::{RelojSistema, RepositorioBienesPrivados, UnidadDeTrabajo},
};
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};
use domain::{
    AdvertenciaFicha, BienPrivadoId, CodigoAgrupacion, Coeficiente, CuadroCoeficientesId,
    Documento, DominioError, Nit, NumeroBienPrivado, Porcentaje, TenantId, TipoAgrupacion,
    TipoBienPrivado, TipoCopropiedad, TipoPersona, TipoTitularidad, UsoBienPrivado,
};
use infra_postgres::{UnidadDeTrabajoPg, transaccion_tenant};
use rust_decimal::Decimal;
use sqlx::{AssertSqlSafe, PgPool};

struct Entorno {
    duenio: PgPool,
    unidad: UnidadDeTrabajoPg,
    ctx: Contexto,
}

async fn entorno(pool: PgPool) -> Entorno {
    let tenant = crear_copropiedad(&pool).await;
    Entorno {
        unidad: UnidadDeTrabajoPg::new(pool_como_app(&pool).await),
        ctx: Contexto::new(tenant),
        duenio: pool,
    }
}

fn dec(texto: &str) -> Decimal {
    texto.parse().expect("decimal válido")
}

fn bien(numero: &str) -> NuevoBienPrivado {
    NuevoBienPrivado {
        tipo: TipoBienPrivado::Apartamento,
        agrupacion_id: None,
        numero: Some(NumeroBienPrivado::new(numero).expect("válido")),
        codigo: None,
        piso: None,
        area_privada_m2: None,
        matricula_inmobiliaria: None,
        cedula_catastral: None,
        uso: UsoBienPrivado::Residencial,
    }
}

async fn tres_bienes(e: &Entorno) -> Vec<BienPrivadoId> {
    let mut ids = Vec::new();
    for n in ["101", "102", "103"] {
        ids.push(
            crear_bien_privado(&e.unidad, &e.ctx, bien(n))
                .await
                .expect("crea")
                .id,
        );
    }
    ids
}

async fn cuadro_con(e: &Entorno, ids: &[BienPrivadoId], valores: &[&str]) -> CuadroCoeficientesId {
    let c = crear_cuadro(&e.unidad, &RelojSistema, &e.ctx, NuevoCuadro::default())
        .await
        .expect("crea cuadro");
    let filas = ids
        .iter()
        .zip(valores)
        .map(|(id, v)| {
            (
                RefBienPrivado::Id(*id),
                Coeficiente::new(dec(v)).expect("válido"),
            )
        })
        .collect();
    cargar_coeficientes(&e.unidad, &e.ctx, c.id, filas)
        .await
        .expect("carga");
    c.id
}

/// Ejecuta SQL como `dad_app` en el tenant y confirma.
async fn como_app(pool: &PgPool, tenant: TenantId, sql: &str) -> Result<(), sqlx::Error> {
    let mut tx = transaccion_tenant(pool, tenant).await?;
    sqlx::query("SET LOCAL ROLE dad_app")
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql(AssertSqlSafe(sql.to_owned()))
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

fn sqlstate(e: &sqlx::Error) -> String {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| c.into_owned())
        .unwrap_or_default()
}

async fn coeficiente_de(e: &Entorno, id: BienPrivadoId) -> Option<Decimal> {
    let mut tx = e.unidad.iniciar(&e.ctx).await.expect("tx");
    tx.bien(id)
        .await
        .expect("lee")
        .and_then(|b| b.coeficiente)
        .map(|c| c.valor())
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn activar_copia_el_coeficiente_y_rechaza_la_suma_inexacta(pool: PgPool) {
    let e = entorno(pool).await;
    let ids = tres_bienes(&e).await;

    let malo = cuadro_con(&e, &ids, &["0.33333333", "0.33333333", "0.33333333"]).await;
    assert!(matches!(
        activar_cuadro(&e.unidad, &RelojSistema, &e.ctx, malo).await,
        Err(AppError::Dominio(
            DominioError::CoeficientesNoSumanUno { .. }
        ))
    ));

    let bueno = cuadro_con(&e, &ids, &["0.5", "0.25", "0.25"]).await;
    activar_cuadro(&e.unidad, &RelojSistema, &e.ctx, bueno)
        .await
        .expect("activa");
    assert_eq!(coeficiente_de(&e, ids[0]).await, Some(dec("0.5")));

    // Una versión nueva reemplaza a la anterior y los bienes toman sus valores.
    let v3 = cuadro_con(&e, &ids, &["0.4", "0.3", "0.3"]).await;
    activar_cuadro(&e.unidad, &RelojSistema, &e.ctx, v3)
        .await
        .expect("activa v3");
    assert_eq!(coeficiente_de(&e, ids[0]).await, Some(dec("0.4")));
    let estados: Vec<(i32, String)> =
        sqlx::query_as("SELECT version, estado FROM cuadros_coeficientes ORDER BY version")
            .fetch_all(&e.duenio)
            .await
            .expect("cuadros");
    assert_eq!(
        estados,
        vec![
            (1, "borrador".into()),
            (2, "historico".into()),
            (3, "vigente".into())
        ]
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_bd_rechaza_activar_sin_sumar_uno_o_con_bienes_faltantes(pool: PgPool) {
    let e = entorno(pool).await;
    let ids = tres_bienes(&e).await;
    let incompleto = cuadro_con(&e, &ids[..2], &["0.5", "0.5"]).await;
    let inexacto = cuadro_con(&e, &ids, &["0.5", "0.25", "0.24999999"]).await;
    for cuadro in [incompleto, inexacto] {
        let r = como_app(
            &e.duenio,
            e.ctx.tenant_id,
            &format!(
                "UPDATE cuadros_coeficientes SET estado = 'vigente', vigente_desde = now()
                 WHERE id = '{cuadro}'"
            ),
        )
        .await;
        assert_eq!(sqlstate(&r.expect_err("rechazado")), "V0003");
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn solo_un_cuadro_vigente_y_sus_coeficientes_son_inmutables(pool: PgPool) {
    let e = entorno(pool).await;
    let ids = tres_bienes(&e).await;
    let a = cuadro_con(&e, &ids, &["0.5", "0.25", "0.25"]).await;
    let b = cuadro_con(&e, &ids, &["0.5", "0.25", "0.25"]).await;
    activar_cuadro(&e.unidad, &RelojSistema, &e.ctx, a)
        .await
        .expect("activa");

    // Activar el segundo sin pasar el primero a histórico choca con el índice único parcial.
    let r = como_app(
        &e.duenio,
        e.ctx.tenant_id,
        &format!(
            "UPDATE cuadros_coeficientes SET estado = 'vigente', vigente_desde = now() WHERE id = '{b}'"
        ),
    )
    .await;
    assert_eq!(sqlstate(&r.expect_err("único vigente")), "23505");

    let r = como_app(
        &e.duenio,
        e.ctx.tenant_id,
        &format!("UPDATE coeficientes_bien_privado SET coeficiente = 0.1 WHERE cuadro_id = '{a}'"),
    )
    .await;
    assert_eq!(sqlstate(&r.expect_err("inmutable")), "V0002");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_coeficiente_del_bien_no_se_edita_directamente(pool: PgPool) {
    let e = entorno(pool).await;
    let ids = tres_bienes(&e).await;
    let r = como_app(
        &e.duenio,
        e.ctx.tenant_id,
        &format!(
            "UPDATE bienes_privados SET coeficiente = 0.5 WHERE id = '{}'",
            ids[0]
        ),
    )
    .await;
    assert_eq!(sqlstate(&r.expect_err("solo por cuadro")), "V0002");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn no_se_activa_con_una_reunion_instalada(pool: PgPool) {
    let e = entorno(pool).await;
    let ids = tres_bienes(&e).await;
    let c = cuadro_con(&e, &ids, &["0.5", "0.25", "0.25"]).await;
    sqlx::query(
        "INSERT INTO reuniones (id, tenant_id, titulo, tipo, estado, fecha_programada)
         VALUES (gen_random_uuid(), $1, 'Asamblea', 'ordinaria', 'instalada', now())",
    )
    .bind(e.ctx.tenant_id.as_uuid())
    .execute(&e.duenio)
    .await
    .expect("reunión");
    assert!(matches!(
        activar_cuadro(&e.unidad, &RelojSistema, &e.ctx, c).await,
        Err(AppError::Conflicto(_))
    ));
    let r = como_app(
        &e.duenio,
        e.ctx.tenant_id,
        &format!("UPDATE cuadros_coeficientes SET estado = 'vigente', vigente_desde = now() WHERE id = '{c}'"),
    )
    .await;
    assert_eq!(sqlstate(&r.expect_err("bloqueado en la BD")), "V0004");
}

fn persona(doc: &str) -> RefPersona {
    RefPersona::Nueva(DatosPersona {
        tipo: TipoPersona::Natural,
        nombre: format!("Persona {doc}"),
        documento: Documento::new(doc).expect("válido"),
        telefono: None,
        correo: None,
        cuenta_id: None,
    })
}

fn titular(p: RefPersona, porcentaje: &str, repr: bool) -> SolicitudTitular {
    SolicitudTitular {
        persona: p,
        tipo: TipoTitularidad::Propietario,
        porcentaje: Porcentaje::new(dec(porcentaje)).expect("válido"),
        es_representante: repr,
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn transferencia_deja_un_solo_conjunto_vigente(pool: PgPool) {
    let e = entorno(pool).await;
    let id = crear_bien_privado(&e.unidad, &e.ctx, bien("101"))
        .await
        .expect("crea")
        .id;
    registrar_titulares(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        id,
        vec![
            titular(persona("111"), "50", true),
            titular(persona("222"), "50", false),
        ],
    )
    .await
    .expect("registra");
    transferir_titularidad(
        &e.unidad,
        &RelojSistema,
        &e.ctx,
        id,
        vec![titular(persona("333"), "100", false)],
        None,
    )
    .await
    .expect("transfiere");

    let (vigentes, total): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE vigente_hasta IS NULL), count(*) FROM titularidades
         WHERE bien_privado_id = $1",
    )
    .bind(id.as_uuid())
    .fetch_one(&e.duenio)
    .await
    .expect("conteo");
    assert_eq!((vigentes, total), (1, 3));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_bd_exige_cien_por_ciento_y_un_representante_al_confirmar(pool: PgPool) {
    let e = entorno(pool).await;
    let id = crear_bien_privado(&e.unidad, &e.ctx, bien("101"))
        .await
        .expect("crea")
        .id;
    let t = e.ctx.tenant_id;
    sqlx::query(
        "INSERT INTO personas (id, tenant_id, nombre, documento)
         VALUES ('00000000-0000-7000-8000-000000000001', $1, 'Ana', '111'),
                ('00000000-0000-7000-8000-000000000002', $1, 'Luis', '222')",
    )
    .bind(t.as_uuid())
    .execute(&e.duenio)
    .await
    .expect("personas");
    let insertar = |p: &str, porcentaje: &str, repr: bool| {
        format!(
            "INSERT INTO titularidades (id, tenant_id, bien_privado_id, persona_id, tipo, porcentaje,
                                        es_representante, vigente_desde)
             VALUES (gen_random_uuid(), '{t}', '{id}', '00000000-0000-7000-8000-00000000000{p}',
                     'propietario', {porcentaje}, {repr}, now());"
        )
    };
    let r = como_app(&e.duenio, t, &insertar("1", "60", true)).await;
    assert_eq!(sqlstate(&r.expect_err("no suma 100")), "V0005");
    let dos_sin_repr = format!(
        "{}{}",
        insertar("1", "50", false),
        insertar("2", "50", false)
    );
    let r = como_app(&e.duenio, t, &dos_sin_repr).await;
    assert_eq!(sqlstate(&r.expect_err("sin representante")), "V0005");
    let bien_hecho = format!(
        "{}{}",
        insertar("1", "50", true),
        insertar("2", "50", false)
    );
    como_app(&e.duenio, t, &bien_hecho).await.expect("válido");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn otra_copropiedad_no_ve_ni_referencia_lo_ajeno(pool: PgPool) {
    let e = entorno(pool).await;
    let torre = crear_agrupacion(
        &e.unidad,
        &e.ctx,
        NuevaAgrupacion {
            tipo: TipoAgrupacion::Edificio,
            nombre: "Torre 1".into(),
            codigo: CodigoAgrupacion::new("T1").expect("válido"),
            padre_id: None,
            orden: 0,
        },
    )
    .await
    .expect("agrupación");
    let ids = tres_bienes(&e).await;
    cuadro_con(&e, &ids, &["0.5", "0.25", "0.25"]).await;

    let otra = Contexto::new(crear_copropiedad(&e.duenio).await);
    let mut tx = e.unidad.iniciar(&otra).await.expect("tx");
    assert!(tx.agrupaciones().await.expect("lee").is_empty());
    assert!(tx.todos_los_bienes().await.expect("lee").is_empty());
    drop(tx);
    for tabla in [
        "cuadros_coeficientes",
        "coeficientes_bien_privado",
        "agrupaciones",
    ] {
        let mut tx = transaccion_tenant(&e.duenio, otra.tenant_id)
            .await
            .expect("tx");
        sqlx::query("SET LOCAL ROLE dad_app")
            .execute(&mut *tx)
            .await
            .expect("rol");
        let n: i64 = sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {tabla}")))
            .fetch_one(&mut *tx)
            .await
            .expect("cuenta");
        assert_eq!(n, 0, "{tabla}");
    }

    // FK compuesta: un bien de otra copropiedad no puede colgar de esta agrupación.
    let r = como_app(
        &e.duenio,
        otra.tenant_id,
        &format!(
            "INSERT INTO bienes_privados (id, tenant_id, codigo, agrupacion_id)
             VALUES (gen_random_uuid(), '{}', 'X-1', '{}')",
            otra.tenant_id, torre.id
        ),
    )
    .await;
    assert_eq!(sqlstate(&r.expect_err("fk compuesta")), "23503");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn ficha_con_nit_y_advertencias_legales(pool: PgPool) {
    let e = entorno(pool).await;
    let ficha = actualizar_datos(
        &e.unidad,
        &e.ctx,
        CambiosCopropiedad {
            nit: Some(Nit::new("800197268").expect("válido")),
            tipo: Some(TipoCopropiedad::Mixta),
            ..Default::default()
        },
    )
    .await
    .expect("actualiza");
    assert_eq!(ficha.nit.expect("nit").to_string(), "800197268-4");

    let completa = ver_ficha(&e.unidad, &e.ctx).await.expect("ficha");
    assert_eq!(
        completa.advertencias,
        vec![
            AdvertenciaFicha::FaltaRevisorFiscal,
            AdvertenciaFicha::SinCuadroVigente
        ]
    );

    // El NIT es único en la plataforma.
    let otra = Contexto::new(crear_copropiedad(&e.duenio).await);
    assert!(matches!(
        actualizar_datos(
            &e.unidad,
            &otra,
            CambiosCopropiedad {
                nit: Some(Nit::new("800197268-4").expect("válido")),
                ..Default::default()
            },
        )
        .await,
        Err(AppError::Conflicto(_))
    ));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn codigo_unico_por_copropiedad(pool: PgPool) {
    let e = entorno(pool).await;
    crear_bien_privado(&e.unidad, &e.ctx, bien("101"))
        .await
        .expect("crea");
    assert!(matches!(
        crear_bien_privado(&e.unidad, &e.ctx, bien("101")).await,
        Err(AppError::Conflicto(_))
    ));
    // Otra PH puede usar el mismo código.
    let otra = Contexto::new(crear_copropiedad(&e.duenio).await);
    crear_bien_privado(&e.unidad, &otra, bien("101"))
        .await
        .expect("otra PH");
}
