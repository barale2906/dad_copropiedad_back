//! Pruebas de integración de la unidad de trabajo y la auditoría sobre PostgreSQL.
//!
//! La unidad de trabajo opera con un pool que fija `SET ROLE dad_app` en cada conexión, igual
//! que la aplicación en producción (sin `BYPASSRLS`, no dueño de las tablas). Las
//! verificaciones se leen con el pool del dueño, que no está sujeto a RLS.

use application::{
    Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{RegistroAuditoria, Transaccion, UnidadDeTrabajo},
};
use domain::{PersonaId, TenantId};
use infra_postgres::UnidadDeTrabajoPg;
use sqlx::{Executor, PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const MIGRACIONES: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
const ACCION: AccionAuditoria = AccionAuditoria::new("pruebas.recurso.crear");

/// Pool sobre la misma BD de la prueba, pero con el rol restringido de la aplicación.
async fn pool_como_app(pool: &PgPool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|conn, _| {
            Box::pin(async move {
                conn.execute("SET ROLE dad_app").await?;
                Ok(())
            })
        })
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("pool con rol dad_app")
}

async fn crear_copropiedad(pool: &PgPool) -> TenantId {
    let tenant = TenantId::nuevo();
    sqlx::query("INSERT INTO copropiedades (id, nombre) VALUES ($1, 'PH de prueba')")
        .bind(tenant.as_uuid())
        .execute(pool)
        .await
        .expect("copropiedad");
    tenant
}

async fn contar_auditoria(pool: &PgPool, tenant: TenantId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM auditoria WHERE tenant_id = $1")
        .bind(tenant.as_uuid())
        .fetch_one(pool)
        .await
        .expect("conteo")
}

fn contexto(tenant: TenantId) -> Contexto {
    Contexto {
        tenant_id: tenant,
        actor_id: Some(PersonaId::nuevo()),
        request_id: Some("req-123".into()),
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn auditoria_confirmada_guarda_contexto_y_detalle(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);
    let ctx = contexto(tenant);
    let entidad_id = Uuid::now_v7();
    let detalle = DetalleAuditoria::vacio()
        .con("coeficiente_nuevo", "0.01250000")
        .expect("clave permitida");

    let mut tx = unidad.iniciar(&ctx).await.expect("inicia");
    tx.registrar(EventoAuditoria::ok(ACCION, "recurso", entidad_id).con_detalle(detalle))
        .await
        .expect("registra");
    tx.confirmar().await.expect("confirma");

    let fila: (
        Option<Uuid>,
        String,
        String,
        Option<Uuid>,
        String,
        serde_json::Value,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT actor_id, accion, entidad, entidad_id, resultado, detalle, request_id
             FROM auditoria WHERE tenant_id = $1",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("una fila de auditoría");
    assert_eq!(fila.0, ctx.actor_id.map(PersonaId::as_uuid));
    assert_eq!(fila.1, "pruebas.recurso.crear");
    assert_eq!(fila.2, "recurso");
    assert_eq!(fila.3, Some(entidad_id));
    assert_eq!(fila.4, "ok");
    assert_eq!(fila.5["coeficiente_nuevo"], "0.01250000");
    assert_eq!(fila.6.as_deref(), Some("req-123"));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn transaccion_sin_confirmar_no_deja_auditoria(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);

    {
        let mut tx = unidad.iniciar(&contexto(tenant)).await.expect("inicia");
        tx.registrar(EventoAuditoria::ok(ACCION, "recurso", Uuid::now_v7()))
            .await
            .expect("registra");
    }

    assert_eq!(contar_auditoria(&pool, tenant).await, 0);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn no_se_puede_auditar_en_otro_tenant(pool: PgPool) {
    // RLS: la transacción del tenant A no puede insertar filas del tenant B. Se simula un
    // adaptador defectuoso insertando a mano con otro tenant_id en la misma transacción.
    let a = crear_copropiedad(&pool).await;
    let b = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;

    let mut tx = infra_postgres::transaccion_tenant(&app, a)
        .await
        .expect("inicia");
    let res = sqlx::query(
        "INSERT INTO auditoria (tenant_id, accion, entidad, resultado) VALUES ($1, 'x.y.z', 'x', 'ok')",
    )
    .bind(b.as_uuid())
    .execute(&mut *tx)
    .await;

    let err = res.expect_err("RLS debió rechazar la fila de otro tenant");
    assert!(
        err.to_string().contains("row-level security"),
        "se esperaba un rechazo de RLS: {err}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn auditoria_es_de_solo_insercion(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(&pool).await);
    let mut tx = unidad.iniciar(&contexto(tenant)).await.expect("inicia");
    tx.registrar(EventoAuditoria::ok(ACCION, "recurso", Uuid::now_v7()))
        .await
        .expect("registra");
    tx.confirmar().await.expect("confirma");

    let res = pool
        .execute("UPDATE auditoria SET resultado = 'error'")
        .await;

    assert!(
        res.is_err(),
        "auditoria debe rechazar UPDATE incluso al dueño (trigger)"
    );
    assert_eq!(contar_auditoria(&pool, tenant).await, 1);
}
