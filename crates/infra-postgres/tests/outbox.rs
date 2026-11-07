//! Pruebas de integración del outbox multicanal: encolado transaccional (rol `dad_app`) y
//! consumo por el worker con las funciones `SECURITY DEFINER`.

use std::time::Duration;

use application::{
    Contexto,
    outbox::{Canal, EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        ColaSalida, LoteSalida, RegistroOutbox, Transaccion, UnidadDeTrabajo,
        correo::CorreoSaliente,
    },
};
use domain::{DireccionCorreo, TenantId};
use infra_postgres::{ColaSalidaPg, UnidadDeTrabajoPg};
use sqlx::PgPool;

mod comun;
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};

fn correo(para: &str) -> EventoSaliente {
    EventoSaliente {
        prioridad: PrioridadSalida::Evento,
        mensaje: MensajeOutbox::Correo(CorreoSaliente {
            para: DireccionCorreo::new(para).expect("válida"),
            asunto: "Reserva aprobada".into(),
            texto: "Su reserva del salón social fue aprobada.".into(),
            html: None,
        }),
    }
}

async fn encolar(app: &PgPool, tenant: TenantId, evento: EventoSaliente, confirmar: bool) {
    let unidad = UnidadDeTrabajoPg::new(app.clone());
    let mut tx = unidad
        .iniciar(&Contexto::new(tenant))
        .await
        .expect("inicia");
    tx.encolar(evento).await.expect("encola");
    if confirmar {
        tx.confirmar().await.expect("confirma");
    }
}

/// Inserta un mensaje de WhatsApp directamente (aún no hay quien los envíe).
async fn insertar_whatsapp(pool: &PgPool, tenant: TenantId) {
    sqlx::query(
        "INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload)
         VALUES ($1, 'whatsapp', 1, '123', '573001112233', '{}')",
    )
    .bind(tenant.as_uuid())
    .execute(pool)
    .await
    .expect("mensaje de WhatsApp");
}

async fn estado(pool: &PgPool) -> Vec<(String, String, i32, Option<String>, bool)> {
    sqlx::query_as(
        "SELECT canal, estado, intentos, referencia_salida, disponible_en > now()
         FROM outbox ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("estado del outbox")
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn encolar_correo_guarda_canal_destino_y_sin_linea(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;

    encolar(&app, tenant, correo("residente@correo.co"), true).await;

    let fila: (String, i16, Option<String>, String, serde_json::Value) = sqlx::query_as(
        "SELECT canal, prioridad, linea, destino, payload FROM outbox WHERE tenant_id = $1",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("una fila");
    assert_eq!(fila.0, "correo");
    assert_eq!(fila.1, PrioridadSalida::Evento.as_i16());
    assert_eq!(fila.2, None);
    assert_eq!(fila.3, "residente@correo.co");
    assert_eq!(fila.4["canal"], "correo");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn sin_confirmar_no_se_encola_nada(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;

    encolar(&app, tenant, correo("a@b.co"), false).await;

    assert!(estado(&pool).await.is_empty());
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn linea_solo_se_admite_en_whatsapp(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;

    let res = sqlx::query(
        "INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload)
         VALUES ($1, 'correo', 2, '123', 'a@b.co', '{}')",
    )
    .bind(tenant.as_uuid())
    .execute(&pool)
    .await;

    let err = res.expect_err("correo con línea debe rechazarse");
    assert!(
        err.to_string().contains("ck_outbox_linea_solo_whatsapp"),
        "{err}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn el_worker_solo_reclama_sus_canales_y_marca_enviado(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;
    insertar_whatsapp(&pool, tenant).await;
    encolar(&app, tenant, correo("a@b.co"), true).await;

    let cola = ColaSalidaPg::new(app.clone());
    let mut lote = cola.reclamar(&[Canal::Correo], 10).await.expect("reclama");
    let mensajes = lote.tomar_mensajes();
    assert_eq!(mensajes.len(), 1);
    assert!(matches!(mensajes[0].mensaje, Ok(MensajeOutbox::Correo(_))));
    assert_eq!(mensajes[0].tenant_id, tenant);
    lote.marcar_enviado(mensajes[0].id, Some("id-smtp-1".into()))
        .await
        .expect("marca");
    lote.confirmar().await.expect("confirma");

    let filas = estado(&pool).await;
    assert_eq!(filas[0].0, "whatsapp");
    assert_eq!(filas[0].1, "pendiente", "WhatsApp no se toca");
    assert_eq!(filas[1].1, "enviado");
    assert_eq!(filas[1].2, 1);
    assert_eq!(filas[1].3.as_deref(), Some("id-smtp-1"));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn fallo_reprograma_y_al_agotar_intentos_queda_fallido(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;
    encolar(&app, tenant, correo("a@b.co"), true).await;
    let cola = ColaSalidaPg::new(app.clone());
    let id: i64 = sqlx::query_scalar("SELECT id FROM outbox")
        .fetch_one(&pool)
        .await
        .expect("id");

    let mut lote = cola.reclamar(&[Canal::Correo], 10).await.expect("reclama");
    lote.marcar_fallido(id, "550 rechazado".into(), Duration::from_secs(60), 2)
        .await
        .expect("marca");
    lote.confirmar().await.expect("confirma");
    let filas = estado(&pool).await;
    assert_eq!(
        (filas[0].1.as_str(), filas[0].2, filas[0].4),
        ("pendiente", 1, true)
    );

    // Aún no está disponible (espera de 60 s): no se reclama.
    let mut lote = cola.reclamar(&[Canal::Correo], 10).await.expect("reclama");
    assert!(lote.tomar_mensajes().is_empty());
    lote.marcar_fallido(id, "550 rechazado".into(), Duration::ZERO, 2)
        .await
        .expect("marca");
    lote.confirmar().await.expect("confirma");

    assert_eq!(estado(&pool).await[0].1, "fallido");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn dos_workers_no_reclaman_el_mismo_mensaje(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;
    encolar(&app, tenant, correo("a@b.co"), true).await;
    let cola = ColaSalidaPg::new(app.clone());

    let mut primero = cola.reclamar(&[Canal::Correo], 10).await.expect("reclama");
    let mut segundo = cola.reclamar(&[Canal::Correo], 10).await.expect("reclama");

    assert_eq!(primero.tomar_mensajes().len(), 1);
    assert!(
        segundo.tomar_mensajes().is_empty(),
        "FOR UPDATE SKIP LOCKED"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn lote_sin_confirmar_deja_los_mensajes_pendientes(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;
    encolar(&app, tenant, correo("a@b.co"), true).await;
    let cola = ColaSalidaPg::new(app.clone());

    {
        let mut lote = cola.reclamar(&[Canal::Correo], 10).await.expect("reclama");
        let id = lote.tomar_mensajes()[0].id;
        lote.marcar_enviado(id, None).await.expect("marca");
    }

    let mut lote = cola.reclamar(&[Canal::Correo], 10).await.expect("reclama");
    assert_eq!(lote.tomar_mensajes().len(), 1);
}
