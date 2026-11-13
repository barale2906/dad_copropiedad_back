//! Pruebas de integración de la entidad genérica y la vertical PH (ADR 0016), como `dad_app`.

use application::{
    AppError, Contexto, ContextoPlataforma,
    casos_uso::modulos::{exigir_copropiedad, habilitar_modulo},
    puertos::RelojSistema,
};
use domain::{Modulo, TenantId};
use infra_postgres::UnidadDeTrabajoPg;
use sqlx::{PgPool, Row};
use uuid::Uuid;

mod comun;
use comun::{MIGRACIONES, crear_copropiedad, insertar_entidad, pool_como_app};

/// Entidad sin vertical (cualquier empresa de la contabilidad base).
async fn crear_entidad(pool: &PgPool) -> TenantId {
    insertar_entidad(pool, "Entidad de prueba").await
}

/// Ejecuta `sql` como `dad_app` con el tenant fijado.
async fn como_app_en(
    pool: &PgPool,
    tenant: TenantId,
    sql: &'static str,
) -> Result<u64, sqlx::Error> {
    let app = pool_como_app(pool).await;
    let mut tx = app.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.as_uuid().to_string())
        .execute(&mut *tx)
        .await?;
    let filas = sqlx::query(sql)
        .bind(tenant.as_uuid())
        .bind(Uuid::now_v7())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    Ok(filas)
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_entidad_sin_vertical_no_puede_tener_datos_de_la_ph(pool: PgPool) {
    let entidad = crear_entidad(&pool).await;

    let bien = como_app_en(
        &pool,
        entidad,
        "INSERT INTO bienes_privados (id, tenant_id, codigo, coeficiente) VALUES ($2, $1, '101', 1)",
    )
    .await;
    let persona = como_app_en(
        &pool,
        entidad,
        "INSERT INTO personas (id, tenant_id, nombre) VALUES ($2, $1, 'Proveedor')",
    )
    .await;

    let err = bien.expect_err("bienes privados solo en una copropiedad");
    assert!(
        err.as_database_error()
            .is_some_and(|d| d.is_foreign_key_violation()),
        "{err}"
    );
    assert_eq!(persona.expect("las personas son transversales"), 1);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_extension_de_la_copropiedad_queda_aislada_por_tenant(pool: PgPool) {
    let a = crear_copropiedad(&pool).await;
    let b = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;
    let mut tx = app.begin().await.expect("tx");
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(a.as_uuid().to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant");

    let visibles: Vec<Uuid> = sqlx::query("SELECT id FROM copropiedades")
        .fetch_all(&mut *tx)
        .await
        .expect("consulta")
        .iter()
        .map(|f| f.get("id"))
        .collect();
    let ajena = sqlx::query("UPDATE copropiedades SET estrato = 3 WHERE id = $1")
        .bind(b.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("update")
        .rows_affected();

    assert_eq!(visibles, vec![a.as_uuid()]);
    assert_eq!(ajena, 0, "no toca la copropiedad de otro tenant");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn los_modulos_de_la_ph_solo_se_habilitan_en_una_copropiedad(pool: PgPool) {
    let entidad = crear_entidad(&pool).await;
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);
    let sistema = ContextoPlataforma::sistema();
    let habilitar = |modulo| {
        habilitar_modulo(
            &unidad,
            &RelojSistema,
            &sistema,
            entidad,
            modulo,
            None,
            None,
        )
    };

    let asambleas = habilitar(Modulo::Asambleas).await;
    let contabilidad = habilitar(Modulo::Contabilidad).await;

    assert!(
        matches!(asambleas, Err(AppError::NoEncontrado("copropiedad"))),
        "{asambleas:?}"
    );
    contabilidad.expect("la contabilidad sirve a cualquier entidad");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn las_rutas_de_copropiedad_solo_abren_para_una_copropiedad(pool: PgPool) {
    let ph = crear_copropiedad(&pool).await;
    let entidad = crear_entidad(&pool).await;
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);

    exigir_copropiedad(&unidad, &Contexto::new(ph))
        .await
        .expect("es copropiedad, sin ningún módulo vendible");
    let no_ph = exigir_copropiedad(&unidad, &Contexto::new(entidad)).await;

    assert!(matches!(no_ph, Err(AppError::NoEncontrado("copropiedad"))));
}
