//! Pruebas de integración de los módulos habilitados y de la búsqueda normalizada, como
//! `dad_app`.

use application::{
    AppError, Contexto, ContextoPlataforma,
    casos_uso::modulos::{exigir_modulo, habilitar_modulo, terminar_modulo},
    puertos::RelojSistema,
};
use domain::Modulo;
use infra_postgres::UnidadDeTrabajoPg;
use sqlx::PgPool;

mod comun;
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};

#[sqlx::test(migrator = "MIGRACIONES")]
async fn modulo_habilitado_solo_en_su_copropiedad_y_hasta_terminarlo(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let otra = Contexto::new(crear_copropiedad(&pool).await);
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);
    let ctx = Contexto::new(tenant);

    assert!(matches!(
        exigir_modulo(&unidad, &RelojSistema, &ctx, Modulo::Asambleas).await,
        Err(AppError::ModuloNoHabilitado(Modulo::Asambleas))
    ));
    habilitar_modulo(
        &unidad,
        &RelojSistema,
        &ContextoPlataforma::sistema(),
        tenant,
        Modulo::Asambleas,
        None,
        None,
    )
    .await
    .expect("habilita");
    exigir_modulo(&unidad, &RelojSistema, &ctx, Modulo::Asambleas)
        .await
        .expect("habilitado");
    assert!(
        exigir_modulo(&unidad, &RelojSistema, &otra, Modulo::Asambleas)
            .await
            .is_err(),
        "RLS: otra copropiedad no lo tiene"
    );

    let terminados = terminar_modulo(
        &unidad,
        &RelojSistema,
        &ContextoPlataforma::sistema(),
        tenant,
        Modulo::Asambleas,
    )
    .await
    .expect("termina");

    assert_eq!(terminados, 1);
    assert!(
        exigir_modulo(&unidad, &RelojSistema, &ctx, Modulo::Asambleas)
            .await
            .is_err()
    );
    let historial: i64 =
        sqlx::query_scalar("SELECT count(*) FROM modulos_habilitados WHERE modulo = 'asambleas'")
            .fetch_one(&pool)
            .await
            .expect("conteo");
    assert_eq!(historial, 1, "el periodo se conserva para facturación");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_app_no_borra_el_historial_de_modulos(pool: PgPool) {
    let app = pool_como_app(&pool).await;

    for sql in [
        "DELETE FROM modulos_habilitados",
        "UPDATE modulos_habilitados SET modulo = 'publicidad'",
    ] {
        let err = sqlx::query(sql).execute(&app).await.expect_err(sql);
        assert!(
            err.to_string().contains("permission denied"),
            "{sql}: {err}"
        );
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn normalizar_quita_tildes_y_mayusculas(pool: PgPool) {
    let app = pool_como_app(&pool).await;

    let normalizado: String = sqlx::query_scalar("SELECT normalizar('José ÁLVAREZ Ñúñez')")
        .fetch_one(&app)
        .await
        .expect("normaliza");

    assert_eq!(normalizado, "jose alvarez nunez");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn busqueda_por_fragmento_sin_tildes_usa_el_indice(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    for nombre in ["José Álvarez", "María Peña", "Joselyn Ruiz"] {
        sqlx::query("INSERT INTO personas (id, tenant_id, nombre) VALUES ($1, $2, $3)")
            .bind(uuid::Uuid::now_v7())
            .bind(tenant.as_uuid())
            .bind(nombre)
            .execute(&pool)
            .await
            .expect("persona");
    }
    const BUSCAR: &str = "SELECT nombre FROM personas
                          WHERE normalizar(nombre) LIKE '%' || normalizar($1) || '%' ORDER BY normalizar(nombre)";

    let encontrados: Vec<String> = sqlx::query_scalar(BUSCAR)
        .bind("JOSE")
        .fetch_all(&pool)
        .await
        .expect("busca");
    let pena: Vec<String> = sqlx::query_scalar(BUSCAR)
        .bind("pena")
        .fetch_all(&pool)
        .await
        .expect("busca");

    assert_eq!(encontrados, vec!["José Álvarez", "Joselyn Ruiz"]);
    assert_eq!(pena, vec!["María Peña"]);

    // Con el escaneo secuencial desactivado, el plan debe usar el índice GIN de trigramas.
    let mut tx = pool.begin().await.expect("tx");
    sqlx::query("SET LOCAL enable_seqscan = off")
        .execute(&mut *tx)
        .await
        .expect("set");
    let plan: Vec<String> = sqlx::query_scalar(
        "EXPLAIN SELECT nombre FROM personas
         WHERE normalizar(nombre) LIKE '%' || normalizar($1) || '%' ORDER BY normalizar(nombre)",
    )
    .bind("alvarez")
    .fetch_all(&mut *tx)
    .await
    .expect("explain");
    assert!(
        plan.iter()
            .any(|l| l.contains("ix_personas_nombre_busqueda")),
        "{plan:#?}"
    );
}
