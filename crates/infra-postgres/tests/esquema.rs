//! Pruebas de las garantías que da el esquema (RLS, triggers, unicidad, secreto del voto).
//!
//! `#[sqlx::test]` crea una base aislada por prueba y aplica las migraciones. Requiere
//! `DATABASE_URL` (dueño/superusuario) y `docker compose up -d`.
//! Las operaciones "de la app" se ejecutan con `SET LOCAL ROLE dad_app`, igual que en producción.
//! El SQL dinámico (`AssertSqlSafe`) solo interpola UUIDs generados por la propia prueba.

use domain::TenantId;
use infra_postgres::transaccion_tenant;
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

const MIGRACIONES: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

struct Datos {
    tenant: TenantId,
    inmueble: Uuid,
    persona: Uuid,
    pregunta: Uuid,
    opcion: Uuid,
}

/// Siembra como dueño del esquema (sin RLS) una copropiedad con una pregunta secreta en borrador.
async fn sembrar(pool: &PgPool, telefono: &str) -> Datos {
    let d = Datos {
        tenant: TenantId::nuevo(),
        inmueble: Uuid::now_v7(),
        persona: Uuid::now_v7(),
        pregunta: Uuid::now_v7(),
        opcion: Uuid::now_v7(),
    };
    let t = d.tenant.as_uuid();
    let reunion = Uuid::now_v7();
    let linea = Uuid::now_v7();
    let sql = format!(
        "INSERT INTO lineas_whatsapp (id, phone_number_id, numero_visible, nombre)
             VALUES ('{linea}', 'pnid-{linea}', '+57 300', 'Línea');
         INSERT INTO copropiedades (id, nombre) VALUES ('{t}', 'Conjunto {t}');
         INSERT INTO inmuebles (id, tenant_id, codigo, coeficiente)
             VALUES ('{i}', '{t}', 'T1-101', 0.0125);
         INSERT INTO personas (id, tenant_id, nombre, telefono)
             VALUES ('{p}', '{t}', 'Ana', '{telefono}');
         INSERT INTO reuniones (id, tenant_id, titulo, tipo, estado, fecha_programada, linea_whatsapp_id)
             VALUES ('{reunion}', '{t}', 'Asamblea 2027', 'ordinaria', 'instalada', now(), '{linea}');
         INSERT INTO inscripciones (id, tenant_id, reunion_id, inmueble_id, persona_id, rol, metodo, telefono)
             VALUES ('{ins}', '{t}', '{reunion}', '{i}', '{p}', 'propietario', 'enlace', '{telefono}');
         INSERT INTO preguntas (id, tenant_id, reunion_id, orden, enunciado, tipo_voto, regla_mayoria)
             VALUES ('{q}', '{t}', '{reunion}', 1, '¿Aprueba?', 'secreto', 'simple');
         INSERT INTO opciones (id, tenant_id, pregunta_id, orden, texto)
             VALUES ('{o}', '{t}', '{q}', 1, 'Sí');",
        i = d.inmueble,
        p = d.persona,
        q = d.pregunta,
        o = d.opcion,
        ins = Uuid::now_v7(),
    );
    sqlx::raw_sql(AssertSqlSafe(sql))
        .execute(pool)
        .await
        .expect("sembrar datos de prueba");
    d
}

/// Ejecuta `sql` como `dad_app` dentro del tenant indicado y hace COMMIT.
async fn como_app(pool: &PgPool, tenant: TenantId, sql: &str) -> Result<u64, sqlx::Error> {
    let mut tx = transaccion_tenant(pool, tenant).await?;
    sqlx::query("SET LOCAL ROLE dad_app")
        .execute(&mut *tx)
        .await?;
    let filas = sqlx::raw_sql(AssertSqlSafe(sql.to_owned()))
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    Ok(filas)
}

async fn contar_como_app(pool: &PgPool, tenant: Option<TenantId>, tabla: &str) -> i64 {
    let mut tx = match tenant {
        Some(t) => transaccion_tenant(pool, t)
            .await
            .expect("transacción de tenant"),
        None => pool.begin().await.expect("transacción"),
    };
    sqlx::query("SET LOCAL ROLE dad_app")
        .execute(&mut *tx)
        .await
        .expect("SET ROLE dad_app");
    sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {tabla}")))
        .fetch_one(&mut *tx)
        .await
        .expect("contar filas")
}

fn participacion(d: &Datos, clave: &str) -> String {
    format!(
        "INSERT INTO participaciones (id, tenant_id, pregunta_id, inmueble_id, coeficiente, canal,
                                      emitido_por, idempotency_key, wamid)
         VALUES ('{}', '{}', '{}', '{}', 0.0125, 'whatsapp', '{}', '{clave}', '{clave}')
         ON CONFLICT DO NOTHING",
        Uuid::now_v7(),
        d.tenant,
        d.pregunta,
        d.inmueble,
        d.persona
    )
}

fn sqlstate(e: &sqlx::Error) -> String {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| c.into_owned())
        .unwrap_or_default()
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn rls_aisla_tenants(pool: PgPool) {
    let a = sembrar(&pool, "573001110001").await;
    let b = sembrar(&pool, "573001110002").await;

    assert_eq!(contar_como_app(&pool, Some(a.tenant), "inmuebles").await, 1);
    assert_eq!(
        contar_como_app(&pool, Some(b.tenant), "copropiedades").await,
        1
    );
    // Sin tenant fijado no se ve nada.
    assert_eq!(contar_como_app(&pool, None, "inmuebles").await, 0);

    // No se puede escribir en otro tenant.
    let err = como_app(
        &pool,
        a.tenant,
        &format!(
            "INSERT INTO inmuebles (id, tenant_id, codigo, coeficiente)
             VALUES ('{}', '{}', 'X', 0.1)",
            Uuid::now_v7(),
            b.tenant
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&err), "42501", "{err}"); // violación de política RLS
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn participacion_exige_pregunta_abierta_y_es_idempotente(pool: PgPool) {
    let d = sembrar(&pool, "573001110003").await;

    let err = como_app(&pool, d.tenant, &participacion(&d, "wamid.1"))
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&err), "V0001", "{err}");

    como_app(
        &pool,
        d.tenant,
        &format!(
            "UPDATE preguntas SET estado = 'abierta', abierta_en = now() WHERE id = '{}'",
            d.pregunta
        ),
    )
    .await
    .unwrap();

    assert_eq!(
        como_app(&pool, d.tenant, &participacion(&d, "wamid.1"))
            .await
            .unwrap(),
        1
    );
    // Reintento de Meta con el mismo wamid / otro mensaje del mismo inmueble: 0 filas.
    assert_eq!(
        como_app(&pool, d.tenant, &participacion(&d, "wamid.1"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        como_app(&pool, d.tenant, &participacion(&d, "wamid.2"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        contar_como_app(&pool, Some(d.tenant), "participaciones").await,
        1
    );

    // Append-only.
    let err = como_app(&pool, d.tenant, "DELETE FROM participaciones")
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&err), "42501", "{err}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn voto_secreto_sin_rastro(pool: PgPool) {
    let d = sembrar(&pool, "573001110004").await;
    let voto = |id: Uuid, inmueble: &str| {
        format!(
            "INSERT INTO votos (id, tenant_id, pregunta_id, opcion_id, coeficiente, inmueble_id)
             VALUES ('{id}', '{}', '{}', '{}', 0.0125, {inmueble})",
            d.tenant, d.pregunta, d.opcion
        )
    };

    // id v7 (revela la hora): rechazado.
    let err = como_app(&pool, d.tenant, &voto(Uuid::now_v7(), "NULL"))
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&err), "23514", "{err}");

    // Voto secreto que referencia el inmueble: rechazado por el trigger `validar_voto`.
    let err = como_app(
        &pool,
        d.tenant,
        &voto(Uuid::new_v4(), &format!("'{}'", d.inmueble)),
    )
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&err), "23514", "{err}");

    como_app(&pool, d.tenant, &voto(Uuid::new_v4(), "NULL"))
        .await
        .unwrap();
    let err = como_app(&pool, d.tenant, "UPDATE votos SET coeficiente = 1")
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&err), "42501", "{err}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn directorio_cruza_tenants_solo_por_funcion(pool: PgPool) {
    let telefono = "573001110005";
    let a = sembrar(&pool, telefono).await;
    let linea: String = sqlx::query_scalar(
        "SELECT l.phone_number_id FROM lineas_whatsapp l
         JOIN reuniones r ON r.linea_whatsapp_id = l.id WHERE r.tenant_id = $1",
    )
    .bind(a.tenant.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE dad_app")
        .execute(&mut *tx)
        .await
        .unwrap();
    let filas: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT tenant_id, estado FROM reuniones_activas_por_telefono($1, $2)")
            .bind(&linea)
            .bind(telefono)
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    assert_eq!(filas, vec![(a.tenant.as_uuid(), "instalada".to_owned())]);
}
