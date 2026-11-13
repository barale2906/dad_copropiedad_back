//! Pruebas de las garantías que da el esquema (RLS, triggers, unicidad, votos append-only).
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
    bien_privado: Uuid,
    persona: Uuid,
    asistente: Uuid,
    pregunta: Uuid,
    opcion: Uuid,
}

/// Siembra como dueño del esquema (sin RLS) una copropiedad con una reunión instalada, un
/// asistente presente que representa un bien y una pregunta reservada en borrador.
async fn sembrar(pool: &PgPool, telefono: &str) -> Datos {
    let d = Datos {
        tenant: TenantId::nuevo(),
        bien_privado: Uuid::now_v7(),
        persona: Uuid::now_v7(),
        asistente: Uuid::now_v7(),
        pregunta: Uuid::now_v7(),
        opcion: Uuid::now_v7(),
    };
    let t = d.tenant.as_uuid();
    let reunion = Uuid::now_v7();
    let sql = format!(
        "INSERT INTO entidades (id, nombre, estado) VALUES ('{t}', 'Conjunto {t}', 'activa');
         INSERT INTO copropiedades (id) VALUES ('{t}');
         INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
             VALUES (gen_random_uuid(), '{t}', 'asambleas', now() - interval '1 day');
         INSERT INTO bienes_privados (id, tenant_id, codigo, coeficiente)
             VALUES ('{i}', '{t}', 'T1-101', 0.0125);
         INSERT INTO personas (id, tenant_id, nombre, telefono)
             VALUES ('{p}', '{t}', 'Ana', '{telefono}');
         INSERT INTO reuniones (id, tenant_id, titulo, tipo, estado, fecha_programada)
             VALUES ('{reunion}', '{t}', 'Asamblea 2027', 'ordinaria', 'instalada', now());
         INSERT INTO padron_reunion (tenant_id, reunion_id, bien_privado_id, coeficiente,
                                     representante_persona_id)
             VALUES ('{t}', '{reunion}', '{i}', 0.0125, '{p}');
         INSERT INTO asistentes (id, tenant_id, reunion_id, persona_id, metodo, telefono)
             VALUES ('{a}', '{t}', '{reunion}', '{p}', 'mesa', '{telefono}');
         INSERT INTO asistente_bienes (id, tenant_id, reunion_id, asistente_id, bien_privado_id,
                                       calidad, desde)
             VALUES (gen_random_uuid(), '{t}', '{reunion}', '{a}', '{i}', 'propietario', now());
         INSERT INTO asistente_presencias (id, tenant_id, reunion_id, asistente_id, entrada_en)
             VALUES (gen_random_uuid(), '{t}', '{reunion}', '{a}', now());
         INSERT INTO preguntas (id, tenant_id, reunion_id, orden, enunciado, tipo, tipo_voto,
                                base_computo, regla_mayoria)
             VALUES ('{q}', '{t}', '{reunion}', 1, '¿Aprueba?', 'si_no', 'reservado', 'coeficiente', 'simple');
         INSERT INTO opciones (id, tenant_id, pregunta_id, orden, texto)
             VALUES ('{o}', '{t}', '{q}', 1, 'Sí');",
        i = d.bien_privado,
        p = d.persona,
        a = d.asistente,
        q = d.pregunta,
        o = d.opcion,
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

fn participacion(d: &Datos, id: Uuid, clave: &str) -> String {
    format!(
        "INSERT INTO participaciones (id, tenant_id, pregunta_id, bien_privado_id, asistente_id,
                                      coeficiente, canal, emitido_por, idempotency_key, wamid,
                                      registrado_en)
         VALUES ('{id}', '{}', '{}', '{}', '{}', 0.0125, 'whatsapp', '{}', '{clave}', '{clave}', now())
         ON CONFLICT DO NOTHING",
        d.tenant, d.pregunta, d.bien_privado, d.asistente, d.persona
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

    assert_eq!(
        contar_como_app(&pool, Some(a.tenant), "bienes_privados").await,
        1
    );
    assert_eq!(
        contar_como_app(&pool, Some(b.tenant), "copropiedades").await,
        1
    );
    // Sin tenant fijado no se ve nada.
    assert_eq!(contar_como_app(&pool, None, "bienes_privados").await, 0);

    // No se puede escribir en otro tenant.
    let err = como_app(
        &pool,
        a.tenant,
        &format!(
            "INSERT INTO bienes_privados (id, tenant_id, codigo, coeficiente)
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
async fn participacion_exige_pregunta_abierta_bien_habilitado_y_un_voto_por_bien(pool: PgPool) {
    let d = sembrar(&pool, "573001110003").await;

    let err = como_app(
        &pool,
        d.tenant,
        &participacion(&d, Uuid::now_v7(), "wamid.1"),
    )
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
    // Abierta pero el bien no quedó habilitado al abrirla (p. ej. moroso sin voto).
    let err = como_app(
        &pool,
        d.tenant,
        &participacion(&d, Uuid::now_v7(), "wamid.1"),
    )
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&err), "V0005", "{err}");

    como_app(
        &pool,
        d.tenant,
        &format!(
            "INSERT INTO pregunta_habilitados (tenant_id, pregunta_id, bien_privado_id, coeficiente, desde)
             VALUES ('{}', '{}', '{}', 0.0125, now())",
            d.tenant, d.pregunta, d.bien_privado
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        como_app(
            &pool,
            d.tenant,
            &participacion(&d, Uuid::now_v7(), "wamid.1")
        )
        .await
        .unwrap(),
        1
    );
    // Reintento de Meta / otro mensaje del mismo bien privado: 0 filas.
    for clave in ["wamid.1", "wamid.2"] {
        assert_eq!(
            como_app(&pool, d.tenant, &participacion(&d, Uuid::now_v7(), clave))
                .await
                .unwrap(),
            0
        );
    }
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
async fn voto_reservado_guarda_el_vinculo_y_es_append_only(pool: PgPool) {
    let d = sembrar(&pool, "573001110004").await;
    let participacion_id = Uuid::now_v7();
    como_app(
        &pool,
        d.tenant,
        &format!(
            "UPDATE preguntas SET estado = 'abierta', abierta_en = now() WHERE id = '{q}';
             INSERT INTO pregunta_habilitados (tenant_id, pregunta_id, bien_privado_id, coeficiente, desde)
             VALUES ('{t}', '{q}', '{b}', 0.0125, now());",
            t = d.tenant,
            q = d.pregunta,
            b = d.bien_privado
        ),
    )
    .await
    .unwrap();
    como_app(&pool, d.tenant, &participacion(&d, participacion_id, "k"))
        .await
        .unwrap();
    let voto = |id: Uuid, bien: &str| {
        format!(
            "INSERT INTO votos (id, tenant_id, pregunta_id, opcion_id, coeficiente, bien_privado_id,
                                participacion_id)
             VALUES ('{id}', '{}', '{}', '{}', 0.0125, {bien}, '{participacion_id}')",
            d.tenant, d.pregunta, d.opcion
        )
    };

    // id v7 (revela la hora): rechazado.
    let err = como_app(
        &pool,
        d.tenant,
        &voto(Uuid::now_v7(), &format!("'{}'", d.bien_privado)),
    )
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&err), "23514", "{err}");
    // El vínculo con el bien es obligatorio también en `reservado` (ADR 0010).
    let err = como_app(&pool, d.tenant, &voto(Uuid::new_v4(), "NULL"))
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&err), "23502", "{err}");

    como_app(
        &pool,
        d.tenant,
        &voto(Uuid::new_v4(), &format!("'{}'", d.bien_privado)),
    )
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
    sembrar(&pool, "573001110006").await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE dad_app")
        .execute(&mut *tx)
        .await
        .unwrap();
    let filas: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT tenant_id, asistente_id FROM asistencias_por_telefono($1)")
            .bind(telefono)
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    assert_eq!(filas, vec![(a.tenant.as_uuid(), a.asistente)]);
    // Sin tenant fijado, la tabla no muestra nada.
    let directas: i64 = sqlx::query_scalar("SELECT count(*) FROM asistentes")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(directas, 0);
}

/// Glosario de la Ley 675 (ADR 0002): ninguna tabla, columna, restricción ni función conserva
/// el término `inmueble`; el bien con coeficiente es un `bien_privado`.
#[sqlx::test(migrator = "MIGRACIONES")]
async fn esquema_usa_terminos_de_la_ley_675(pool: PgPool) {
    let restos: Vec<String> = sqlx::query_scalar(
        "SELECT table_name || '.' || column_name FROM information_schema.columns
          WHERE table_schema = 'public' AND (table_name ILIKE '%inmueble%' OR column_name ILIKE '%inmueble%')
         UNION ALL
         SELECT conname::text FROM pg_constraint WHERE conname ILIKE '%inmueble%'
         UNION ALL
         SELECT proname::text FROM pg_proc
          WHERE pronamespace = 'public'::regnamespace AND prosrc ILIKE '%inmueble%'",
    )
    .fetch_all(&pool)
    .await
    .expect("consulta del catálogo");

    assert!(
        restos.is_empty(),
        "objetos con el término viejo: {restos:?}"
    );
    assert_eq!(contar_como_app(&pool, None, "bienes_privados").await, 0);
}

/// Tablas con `tenant_id` que no usan `aislamiento_tenant`, con su motivo. Agregar una aquí
/// exige que su política propia también aísle por copropiedad o la reserve a la plataforma.
const TENANT_CON_POLITICA_PROPIA: &[&str] = &[
    // Plantillas globales (`tenant_id IS NULL`) visibles para todas; las propias, solo en su PH.
    "roles",
    "roles_permisos",
    // Publicidad: datos de la plataforma por copropiedad (ADR 0015); ninguna PH los lee.
    "campana_copropiedades",
    "entregas_publicidad",
];

/// Multi-tenant (§8): toda tabla de negocio con `tenant_id` tiene RLS activa y la política
/// `aislamiento_tenant` (salvo las excepciones documentadas). Las particiones no tienen RLS
/// propia, así que `dad_app` no debe poder leerlas sin pasar por la tabla madre.
#[sqlx::test(migrator = "MIGRACIONES")]
async fn toda_tabla_con_tenant_tiene_rls(pool: PgPool) {
    let sin_aislamiento: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname::text
         FROM pg_class c
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attname = 'tenant_id' AND NOT a.attisdropped
         WHERE c.relnamespace = 'public'::regnamespace AND c.relkind IN ('r', 'p')
           AND NOT c.relispartition
           AND (NOT c.relrowsecurity
                OR NOT EXISTS (SELECT 1 FROM pg_policy p
                               WHERE p.polrelid = c.oid AND p.polname = 'aislamiento_tenant'))
           AND NOT (c.relrowsecurity AND c.relname = ANY ($1)
                    AND EXISTS (SELECT 1 FROM pg_policy p WHERE p.polrelid = c.oid))
         ORDER BY 1",
    )
    .bind(TENANT_CON_POLITICA_PROPIA)
    .fetch_all(&pool)
    .await
    .expect("consulta del catálogo");
    assert!(
        sin_aislamiento.is_empty(),
        "tablas con tenant_id sin RLS: {sin_aislamiento:?}"
    );

    let particiones_legibles: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname::text FROM pg_class c
         WHERE c.relnamespace = 'public'::regnamespace AND c.relispartition AND c.relkind = 'r'
           AND (has_table_privilege('dad_app', c.oid, 'SELECT')
                OR has_table_privilege('dad_app', c.oid, 'INSERT'))",
    )
    .fetch_all(&pool)
    .await
    .expect("consulta del catálogo");
    assert!(
        particiones_legibles.is_empty(),
        "particiones accesibles sin RLS: {particiones_legibles:?}"
    );
}

/// Las funciones `SECURITY DEFINER` cruzan tenants: fijan `search_path` (sin él, un objeto
/// homónimo en otro esquema suplantaría al real) y no se ejecutan desde `PUBLIC`.
#[sqlx::test(migrator = "MIGRACIONES")]
async fn funciones_security_definer_fijan_search_path_y_no_son_publicas(pool: PgPool) {
    let inseguras: Vec<String> = sqlx::query_scalar(
        "SELECT p.oid::regprocedure::text FROM pg_proc p
         WHERE p.pronamespace = 'public'::regnamespace AND p.prosecdef
           AND (NOT EXISTS (SELECT 1 FROM unnest(coalesce(p.proconfig, '{}')) x
                            WHERE x LIKE 'search_path=%')
                OR has_function_privilege('public', p.oid, 'EXECUTE'))
         ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .expect("consulta del catálogo");
    assert!(
        inseguras.is_empty(),
        "SECURITY DEFINER inseguras: {inseguras:?}"
    );
}

async fn versiones_aplicadas(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
        .fetch_all(pool)
        .await
        .expect("versiones")
}

/// `dad_admin revertir` deshace las migraciones posteriores a la versión pedida (y solo esas,
/// con sus `.down.sql`); volver a migrar las aplica de nuevo.
#[sqlx::test(migrator = "MIGRACIONES")]
async fn revertir_deshace_solo_las_migraciones_posteriores_y_migrar_las_reaplica(pool: PgPool) {
    let todas = versiones_aplicadas(&pool).await;
    let destino = todas[todas.len() - 3];

    infra_postgres::revertir_en(&pool, destino)
        .await
        .expect("revertir");

    let quedan = versiones_aplicadas(&pool).await;
    assert_eq!(quedan, todas[..todas.len() - 2], "solo las dos posteriores");
    MIGRACIONES.run(&pool).await.expect("migrar de nuevo");
    assert_eq!(versiones_aplicadas(&pool).await, todas);
}
