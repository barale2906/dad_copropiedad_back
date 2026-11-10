//! Integración de los comunicados (plan 10, fase 4) sobre PostgreSQL como `dad_app`: audiencia
//! resuelta en SQL (agrupaciones con sus hijas, propietarios y residentes, menores excluidos,
//! finalidad `comunicaciones` solo con autorización), entregas, outbox con concepto y RLS.

mod comun;

use application::{
    Contexto,
    casos_uso::comunicados::{NuevoComunicado, crear_comunicado, publicar_comunicado},
    puertos::RelojSistema,
};
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};
use domain::{AgrupacionId, Audiencia, BienPrivadoId, FinalidadTratamiento, TenantId};
use infra_postgres::UnidadDeTrabajoPg;
use sqlx::PgPool;
use uuid::Uuid;

/// Torre A (con su interior A1) y torre B; un bien en A1 y otro en B.
struct Ph {
    tenant: TenantId,
    torre_a: AgrupacionId,
    bien_a1: BienPrivadoId,
    /// Propietaria del bien de A1, autorizó `comunicaciones`.
    dueña_a1: Uuid,
    /// Hijo menor que vive en A1.
    menor_a1: Uuid,
    /// Residente adulta (tenedora) del bien de B.
    tenedora_b: Uuid,
    /// Propietario del bien de B, no reside.
    dueño_b: Uuid,
}

async fn preparar(pool: &PgPool) -> Ph {
    let tenant = crear_copropiedad(pool).await;
    sqlx::query(
        "INSERT INTO modulos_habilitados (id, tenant_id, modulo, desde)
         VALUES (gen_random_uuid(), $1, 'gestion_administrativa', now() - interval '1 day')",
    )
    .bind(tenant.as_uuid())
    .execute(pool)
    .await
    .expect("módulo");
    let (torre_a, a1, torre_b) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let (bien_a1, bien_b) = (Uuid::now_v7(), Uuid::now_v7());
    let (dueña_a1, menor_a1, tenedora_b, dueño_b) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    let politica = Uuid::now_v7();
    let mut tx = pool.begin().await.expect("tx");
    for sql in [
        "INSERT INTO agrupaciones (id, tenant_id, tipo, nombre, codigo, padre_id) VALUES
            ($2, $1, 'edificio', 'Torre A', 'A', NULL),
            ($3, $1, 'interior', 'Interior A1', 'A1', $2),
            ($4, $1, 'edificio', 'Torre B', 'B', NULL)",
        "INSERT INTO bienes_privados (id, tenant_id, codigo, agrupacion_id) VALUES
            ($5, $1, 'A1-101', $3), ($6, $1, 'B-201', $4)",
        "INSERT INTO personas (id, tenant_id, nombre, correo, telefono, fecha_nacimiento) VALUES
            ($7, $1, 'Ana', 'ana@correo.co', NULL, '1980-01-01'),
            ($8, $1, 'Tomás', NULL, NULL, current_date - interval '10 years'),
            ($9, $1, 'Teresa', NULL, '573001112233', NULL),
            ($10, $1, 'Bruno', 'bruno@correo.co', NULL, NULL)",
        "INSERT INTO titularidades (id, tenant_id, bien_privado_id, persona_id, tipo, porcentaje,
                                    es_representante, vigente_desde) VALUES
            (gen_random_uuid(), $1, $5, $7, 'propietario', 100, true, now() - interval '1 year'),
            (gen_random_uuid(), $1, $6, $10, 'propietario', 100, true, now() - interval '1 year')",
        "INSERT INTO residencias (id, tenant_id, bien_privado_id, persona_id, calidad,
                                  es_responsable, autorizado_por, vigente_desde) VALUES
            (gen_random_uuid(), $1, $5, $7, 'propietario_residente', true, NULL, now() - interval '1 year'),
            (gen_random_uuid(), $1, $5, $8, 'familiar', false, NULL, now() - interval '1 year'),
            (gen_random_uuid(), $1, $6, $9, 'tenedor', true, $10, now() - interval '1 year')",
        "INSERT INTO politicas_tratamiento (id, tenant_id, version, texto, vigente_desde)
            VALUES ($11, $1, 1, 'Política', now() - interval '1 year')",
        "INSERT INTO autorizaciones_tratamiento (id, tenant_id, persona_id, politica_id,
                                                 finalidad, medio, otorgada_en)
            VALUES (gen_random_uuid(), $1, $7, $11, 'comunicaciones', 'app', now() - interval '1 day')",
    ] {
        sqlx::query(sql)
            .bind(tenant.as_uuid())
            .bind(torre_a)
            .bind(a1)
            .bind(torre_b)
            .bind(bien_a1)
            .bind(bien_b)
            .bind(dueña_a1)
            .bind(menor_a1)
            .bind(tenedora_b)
            .bind(dueño_b)
            .bind(politica)
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    tx.commit().await.expect("datos de la PH");
    Ph {
        tenant,
        torre_a: AgrupacionId::from_uuid(torre_a),
        bien_a1: BienPrivadoId::from_uuid(bien_a1),
        dueña_a1,
        menor_a1,
        tenedora_b,
        dueño_b,
    }
}

fn nuevo(finalidad: FinalidadTratamiento, audiencia: Audiencia) -> NuevoComunicado {
    NuevoComunicado {
        titulo: "Asamblea".into(),
        cuerpo: "Convocatoria.\nOrden del día adjunto.".into(),
        finalidad,
        audiencia,
        importante: true,
        requiere_confirmacion: false,
        adjuntos: Vec::new(),
    }
}

/// Crea y envía el comunicado; devuelve las personas a las que se dirigió.
async fn enviar(pool: &PgPool, ph: &Ph, n: NuevoComunicado) -> Vec<Uuid> {
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(pool).await);
    let ctx = Contexto::new(ph.tenant);
    let c = crear_comunicado(&unidad, &RelojSistema, &ctx, n)
        .await
        .expect("creado");
    publicar_comunicado(&unidad, &RelojSistema, &ctx, c.id, None)
        .await
        .expect("enviado");
    let mut v: Vec<Uuid> = sqlx::query_scalar(
        "SELECT persona_id FROM comunicado_destinatarios WHERE comunicado_id = $1",
    )
    .bind(c.id.as_uuid())
    .fetch_all(pool)
    .await
    .expect("destinatarios");
    v.sort();
    v
}

fn ordenados(mut v: Vec<Uuid>) -> Vec<Uuid> {
    v.sort();
    v
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn todos_llega_a_propietarios_y_residentes_adultos(pool: PgPool) {
    let ph = preparar(&pool).await;
    let v = enviar(
        &pool,
        &ph,
        nuevo(FinalidadTratamiento::Administracion, Audiencia::Todos),
    )
    .await;
    assert_eq!(v, ordenados(vec![ph.dueña_a1, ph.tenedora_b, ph.dueño_b]));
    assert!(!v.contains(&ph.menor_a1), "nunca a menores de edad");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn propietarios_y_residentes_por_separado(pool: PgPool) {
    let ph = preparar(&pool).await;
    let p = enviar(
        &pool,
        &ph,
        nuevo(
            FinalidadTratamiento::Administracion,
            Audiencia::Propietarios,
        ),
    )
    .await;
    assert_eq!(p, ordenados(vec![ph.dueña_a1, ph.dueño_b]));
    let r = enviar(
        &pool,
        &ph,
        nuevo(FinalidadTratamiento::Administracion, Audiencia::Residentes),
    )
    .await;
    assert_eq!(r, ordenados(vec![ph.dueña_a1, ph.tenedora_b]));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_torre_incluye_sus_subagrupaciones_y_los_bienes_por_id(pool: PgPool) {
    let ph = preparar(&pool).await;
    let t = enviar(
        &pool,
        &ph,
        nuevo(
            FinalidadTratamiento::Administracion,
            Audiencia::Agrupaciones(vec![ph.torre_a]),
        ),
    )
    .await;
    assert_eq!(
        t,
        vec![ph.dueña_a1],
        "el bien de A1 está dentro de la torre A"
    );
    let b = enviar(
        &pool,
        &ph,
        nuevo(
            FinalidadTratamiento::Administracion,
            Audiencia::Bienes(vec![ph.bien_a1]),
        ),
    )
    .await;
    assert_eq!(b, vec![ph.dueña_a1]);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn comunicaciones_solo_a_quien_la_autorizo(pool: PgPool) {
    let ph = preparar(&pool).await;
    let v = enviar(
        &pool,
        &ph,
        nuevo(FinalidadTratamiento::Comunicaciones, Audiencia::Todos),
    )
    .await;
    assert_eq!(v, vec![ph.dueña_a1]);
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_envio_queda_en_el_outbox_con_concepto_y_aislado_por_tenant(pool: PgPool) {
    let ph = preparar(&pool).await;
    sqlx::query(
        "INSERT INTO lineas_whatsapp (id, phone_number_id, numero_visible, nombre)
         VALUES (gen_random_uuid(), 'PNID', '+57 300', 'Línea')",
    )
    .execute(&pool)
    .await
    .expect("línea");
    enviar(
        &pool,
        &ph,
        nuevo(FinalidadTratamiento::Administracion, Audiencia::Todos),
    )
    .await;

    let canales: Vec<(String, String)> =
        sqlx::query_as("SELECT canal, concepto FROM outbox WHERE tenant_id = $1 ORDER BY canal")
            .bind(ph.tenant.as_uuid())
            .fetch_all(&pool)
            .await
            .expect("outbox");
    // Ana y Bruno por correo; Teresa, sin app ni correo, por WhatsApp (es importante).
    assert_eq!(
        canales,
        vec![
            ("correo".to_owned(), "comunicados".to_owned()),
            ("correo".to_owned(), "comunicados".to_owned()),
            ("whatsapp".to_owned(), "comunicados".to_owned()),
        ]
    );

    let otra = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;
    let mut tx = infra_postgres::transaccion_tenant(&app, otra)
        .await
        .expect("tx");
    let vistos: i64 = sqlx::query_scalar("SELECT count(*) FROM comunicados")
        .fetch_one(&mut *tx)
        .await
        .expect("conteo");
    assert_eq!(vistos, 0, "RLS: otra copropiedad no ve los comunicados");
}
