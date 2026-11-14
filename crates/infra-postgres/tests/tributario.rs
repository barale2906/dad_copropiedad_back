//! Garantías de la base de datos del plan 15 (tributario y documentos electrónicos), operando
//! como `dad_app`: perfil confirmado inmutable, un documento electrónico por documento de
//! negocio, números y CUFE únicos, eventos append-only, IVA solo en alquiler o venta y catálogos
//! de plataforma.

use domain::TenantId;
use infra_postgres::transaccion_tenant;
use sqlx::{AssertSqlSafe, PgPool};

mod comun;
use comun::{MIGRACIONES, insertar_entidad};

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

fn perfil(tenant: TenantId, id: &str, version: i32, estado: &str) -> String {
    let confirmacion = if estado == "confirmado" {
        "gen_random_uuid(), now()"
    } else {
        "NULL, NULL"
    };
    format!(
        "INSERT INTO perfiles_tributarios (id, tenant_id, version, vigente_desde, estado,
             agente_retenedor, responsable_iva, gran_contribuyente, autorretenedor,
             contribuyente_renta, facturador_electronico, obligado_rub, creado_en,
             confirmado_por, confirmado_en)
         VALUES ('{id}', '{tenant}', {version}, '2026-01-01', '{estado}', TRUE, TRUE, FALSE,
                 FALSE, TRUE, TRUE, FALSE, now(), {confirmacion})",
        tenant = tenant.as_uuid()
    )
}

fn documento(tenant: TenantId, id: &str, origen: &str, numero: i64, cufe: &str) -> String {
    format!(
        "INSERT INTO documentos_electronicos (id, tenant_id, tipo, origen_tipo, origen_id, prefijo,
             numero, cufe, estado, proveedor, total, creado_en)
         VALUES ('{id}', '{tenant}', 'factura', 'ventas.factura', '{origen}', 'FE', {numero},
                 '{cufe}', 'enviado', 'simulado', 1000, now())",
        tenant = tenant.as_uuid()
    )
}

const A: &str = "01960000-0000-7000-8000-000000000001";
const B: &str = "01960000-0000-7000-8000-000000000002";
const C: &str = "01960000-0000-7000-8000-000000000003";

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_perfil_confirmado_no_se_modifica_ni_se_borra(pool: PgPool) {
    let t = insertar_entidad(&pool, "Empresa").await;
    como_app(&pool, t, &perfil(t, A, 1, "confirmado"))
        .await
        .expect("perfil");

    let editar = como_app(
        &pool,
        t,
        &format!("UPDATE perfiles_tributarios SET responsable_iva = FALSE WHERE id = '{A}'"),
    )
    .await
    .expect_err("inmutable");
    let borrar = como_app(
        &pool,
        t,
        &format!("DELETE FROM perfiles_tributarios WHERE id = '{A}'"),
    )
    .await
    .expect_err("inmutable");

    assert_eq!(sqlstate(&editar), "V0002");
    assert!(!sqlstate(&borrar).is_empty());
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_perfil_en_borrador_se_puede_confirmar(pool: PgPool) {
    let t = insertar_entidad(&pool, "Empresa").await;
    como_app(&pool, t, &perfil(t, A, 1, "borrador"))
        .await
        .expect("perfil");

    como_app(
        &pool,
        t,
        &format!(
            "UPDATE perfiles_tributarios SET estado = 'confirmado',
                 confirmado_por = gen_random_uuid(), confirmado_en = now() WHERE id = '{A}'"
        ),
    )
    .await
    .expect("confirma");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_documento_de_negocio_tiene_un_solo_documento_electronico_por_tipo(pool: PgPool) {
    let t = insertar_entidad(&pool, "Empresa").await;
    como_app(&pool, t, &documento(t, A, C, 1, &"a".repeat(96)))
        .await
        .expect("primero");

    let doble = como_app(&pool, t, &documento(t, B, C, 2, &"b".repeat(96)))
        .await
        .expect_err("mismo origen");

    assert_eq!(sqlstate(&doble), "23505");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_numero_y_un_cufe_no_se_repiten(pool: PgPool) {
    let t = insertar_entidad(&pool, "Empresa").await;
    como_app(&pool, t, &documento(t, A, A, 1, &"a".repeat(96)))
        .await
        .expect("primero");

    let numero = como_app(&pool, t, &documento(t, B, B, 1, &"b".repeat(96)))
        .await
        .expect_err("número repetido");
    let cufe = como_app(&pool, t, &documento(t, C, C, 3, &"a".repeat(96)))
        .await
        .expect_err("CUFE repetido");

    assert_eq!(sqlstate(&numero), "23505");
    assert_eq!(sqlstate(&cufe), "23505");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn un_documento_validado_exige_cufe(pool: PgPool) {
    let t = insertar_entidad(&pool, "Empresa").await;
    let sql = format!(
        "INSERT INTO documentos_electronicos (id, tenant_id, tipo, origen_tipo, origen_id,
             estado, proveedor, total, creado_en, validado_en)
         VALUES ('{A}', '{}', 'factura', 'ventas.factura', '{A}', 'validado', 'simulado', 10,
                 now(), now())",
        t.as_uuid()
    );

    let e = como_app(&pool, t, &sql).await.expect_err("sin CUFE");

    assert_eq!(sqlstate(&e), "23514");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn los_eventos_del_documento_son_append_only(pool: PgPool) {
    let t = insertar_entidad(&pool, "Empresa").await;
    como_app(&pool, t, &documento(t, A, A, 1, &"a".repeat(96)))
        .await
        .expect("documento");
    como_app(
        &pool,
        t,
        &format!(
            "INSERT INTO eventos_documento_electronico (id, tenant_id, documento_id, tipo,
                 ocurrido_en) VALUES ('{B}', '{}', '{A}', 'creado', now())",
            t.as_uuid()
        ),
    )
    .await
    .expect("evento");

    let editar = como_app(
        &pool,
        t,
        &format!("UPDATE eventos_documento_electronico SET tipo = 'otro' WHERE id = '{B}'"),
    )
    .await;
    let borrar = como_app(
        &pool,
        t,
        &format!("DELETE FROM eventos_documento_electronico WHERE id = '{B}'"),
    )
    .await;

    assert!(editar.is_err(), "UPDATE rechazado");
    assert!(borrar.is_err(), "DELETE rechazado");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn otro_tenant_no_ve_los_documentos(pool: PgPool) {
    let t = insertar_entidad(&pool, "Empresa").await;
    let otro = insertar_entidad(&pool, "Otra").await;
    como_app(&pool, t, &documento(t, A, A, 1, &"a".repeat(96)))
        .await
        .expect("documento");

    let mut tx = transaccion_tenant(&pool, otro).await.expect("tx");
    sqlx::query("SET LOCAL ROLE dad_app")
        .execute(&mut *tx)
        .await
        .expect("rol");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM documentos_electronicos")
        .fetch_one(&mut *tx)
        .await
        .expect("cuenta");

    assert_eq!(n, 0);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn una_tarifa_gravada_exige_tarifa_positiva_y_solo_la_escribe_la_plataforma(pool: PgPool) {
    let t = insertar_entidad(&pool, "Empresa").await;
    let tarifa = "INSERT INTO tarifas_iva (id, codigo, nombre, tratamiento, tarifa, fuente,
                      vigente_desde)
                  VALUES (gen_random_uuid(), 'general', 'Tarifa general', 'gravado', 19,
                          'Estatuto Tributario', '2026-01-01')";

    let desde_tenant = como_app(&pool, t, tarifa)
        .await
        .expect_err("no es plataforma");
    let incoherente = sqlx::query(
        "INSERT INTO tarifas_iva (id, codigo, nombre, tratamiento, tarifa, fuente, vigente_desde)
         VALUES (gen_random_uuid(), 'excluido', 'Excluido', 'excluido', 5, 'Fuente', '2026-01-01')",
    )
    .execute(&pool)
    .await
    .expect_err("excluido con tarifa");

    assert_eq!(sqlstate(&desde_tenant), "42501");
    assert_eq!(sqlstate(&incoherente), "23514");
}
