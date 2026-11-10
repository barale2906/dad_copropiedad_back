//! Pruebas de integración de la protección de datos como `dad_app`: los casos de uso reales
//! sobre PostgreSQL, RLS, inmutabilidad y clasificación de columnas personales.

use std::collections::BTreeSet;

use application::{
    Contexto,
    casos_uso::proteccion_datos::{
        AutorizacionOtorgada, NuevaPolitica, esta_autorizado, publicar_politica, radicar_solicitud,
        registrar_autorizacion, revocar_autorizacion,
    },
    puertos::RelojSistema,
};
use domain::{
    FinalidadTratamiento::{self, *},
    MedioAutorizacion, PersonaId, TenantId, TipoSolicitudTitular,
};
use infra_postgres::UnidadDeTrabajoPg;
use sqlx::PgPool;

mod comun;
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};

async fn crear_persona(pool: &PgPool, tenant: TenantId) -> PersonaId {
    let persona = PersonaId::nuevo();
    sqlx::query("INSERT INTO personas (id, tenant_id, nombre) VALUES ($1, $2, 'Titular')")
        .bind(persona.as_uuid())
        .bind(tenant.as_uuid())
        .execute(pool)
        .await
        .expect("persona");
    persona
}

fn otorgada(persona: PersonaId, f: &[FinalidadTratamiento]) -> AutorizacionOtorgada {
    AutorizacionOtorgada {
        persona_id: persona,
        finalidades: f.iter().copied().collect::<BTreeSet<_>>(),
        medio: MedioAutorizacion::Web,
        evidencia_archivo_id: None,
    }
}

async fn con_politica(pool: &PgPool) -> (UnidadDeTrabajoPg, Contexto, PersonaId) {
    let tenant = crear_copropiedad(pool).await;
    let persona = crear_persona(pool, tenant).await;
    let unidad = UnidadDeTrabajoPg::new(pool_como_app(pool).await);
    let ctx = Contexto::new(tenant);
    publicar_politica(
        &unidad,
        &RelojSistema,
        &ctx,
        NuevaPolitica {
            texto: Some("Política de tratamiento".into()),
            archivo_id: None,
            vigente_desde: None,
        },
    )
    .await
    .expect("publica");
    (unidad, ctx, persona)
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn autorizar_revocar_y_consultar_sobre_postgres(pool: PgPool) {
    let (unidad, ctx, persona) = con_politica(&pool).await;

    let nuevas = registrar_autorizacion(
        &unidad,
        &RelojSistema,
        &ctx,
        otorgada(persona, &[Comunicaciones, Publicidad]),
    )
    .await
    .expect("autoriza");
    let repetidas = registrar_autorizacion(
        &unidad,
        &RelojSistema,
        &ctx,
        otorgada(persona, &[Publicidad]),
    )
    .await
    .expect("autoriza");
    let revocadas = revocar_autorizacion(&unidad, &RelojSistema, &ctx, persona, &[Publicidad])
        .await
        .expect("revoca");

    assert_eq!(nuevas, vec![Comunicaciones, Publicidad]);
    assert!(repetidas.is_empty(), "no duplica una vigente");
    assert_eq!(revocadas, 1);
    assert!(
        esta_autorizado(&unidad, &ctx, persona, Comunicaciones)
            .await
            .expect("consulta")
    );
    assert!(
        !esta_autorizado(&unidad, &ctx, persona, Publicidad)
            .await
            .expect("consulta")
    );
    let (filas, revocadas): (i64, i64) =
        sqlx::query_as("SELECT count(*), count(revocada_en) FROM autorizaciones_tratamiento")
            .fetch_one(&pool)
            .await
            .expect("conteo");
    assert_eq!(
        (filas, revocadas),
        (2, 1),
        "la revocada se conserva como prueba"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn otra_copropiedad_no_ve_las_autorizaciones(pool: PgPool) {
    let (unidad, ctx, persona) = con_politica(&pool).await;
    registrar_autorizacion(
        &unidad,
        &RelojSistema,
        &ctx,
        otorgada(persona, &[Comunicaciones]),
    )
    .await
    .expect("autoriza");
    let otra = Contexto::new(crear_copropiedad(&pool).await);

    assert!(
        !esta_autorizado(&unidad, &otra, persona, Comunicaciones)
            .await
            .expect("consulta")
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn solicitud_queda_radicada_con_su_vencimiento(pool: PgPool) {
    let (unidad, ctx, persona) = con_politica(&pool).await;

    let solicitud = radicar_solicitud(
        &unidad,
        &RelojSistema,
        &ctx,
        persona,
        TipoSolicitudTitular::Supresion,
        Some("Quiero que borren mi correo".into()),
    )
    .await
    .expect("radica");

    let (estado, vence_el): (String, chrono::NaiveDate) =
        sqlx::query_as("SELECT estado, vence_el FROM solicitudes_titular WHERE id = $1")
            .bind(solicitud.id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("fila");
    assert_eq!(
        (estado.as_str(), vence_el),
        ("radicada", solicitud.vence_el)
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_app_no_reescribe_politicas_ni_autorizaciones(pool: PgPool) {
    let (unidad, ctx, persona) = con_politica(&pool).await;
    registrar_autorizacion(
        &unidad,
        &RelojSistema,
        &ctx,
        otorgada(persona, &[Comunicaciones]),
    )
    .await
    .expect("autoriza");
    let app = pool_como_app(&pool).await;

    for sql in [
        "UPDATE politicas_tratamiento SET texto = 'otra'",
        "DELETE FROM politicas_tratamiento",
        "UPDATE autorizaciones_tratamiento SET finalidad = 'publicidad'",
        "DELETE FROM autorizaciones_tratamiento",
        "DELETE FROM solicitudes_titular",
    ] {
        let err = sqlx::query(sql).execute(&app).await.expect_err(sql);
        assert!(
            err.to_string().contains("permission denied"),
            "{sql}: {err}"
        );
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn columnas_personales_estan_clasificadas(pool: PgPool) {
    let sin_clasificar: Vec<String> = sqlx::query_scalar(
        "SELECT c.table_name || '.' || c.column_name
         FROM information_schema.columns c
         WHERE c.table_schema = 'public'
           AND (c.table_name, c.column_name) IN (
               ('personas', 'nombre'), ('personas', 'documento'), ('personas', 'telefono'),
               ('personas', 'correo'), ('archivos', 'nombre_original'), ('outbox', 'destino'),
               ('outbox', 'payload'), ('outbox', 'ultimo_error'),
               ('bienes_privados', 'matricula_inmobiliaria'),
               ('bienes_privados', 'cedula_catastral'), ('personas', 'fecha_nacimiento'),
               ('personas', 'contacto_emergencia_nombre'),
               ('personas', 'contacto_emergencia_telefono'), ('personas', 'movilidad_reducida'),
               ('vehiculos', 'placa'), ('cambios_censo', 'datos'),
               ('tickets', 'descripcion'), ('tickets', 'comentario_encuesta'),
               ('ticket_mensajes', 'texto'), ('conversaciones', 'telefono'),
               ('conversacion_mensajes', 'texto'), ('sesiones_chat', 'telefono'),
               ('sesiones_chat', 'estado'), ('ordenes_trabajo', 'recibido_por'),
               ('asistentes', 'telefono'), ('visitas', 'visitante_nombre'),
               ('visitas', 'visitante_documento'), ('visitas', 'placa'),
               ('entregas_correspondencia', 'tercero_nombre'),
               ('entregas_correspondencia', 'tercero_documento'), ('minuta_porteria', 'texto'),
               ('eventos_acceso', 'placa'), ('dispositivos_push', 'token'),
               ('consentimientos_biometricos', 'persona_id'), ('solicitudes_titular', 'detalle'),
               ('solicitudes_titular', 'respuesta'), ('respuestas_censo', 'detalle'), ('peticiones_idempotentes', 'cuerpo'))
           AND coalesce(col_description(
                   format('%I.%I', c.table_schema, c.table_name)::regclass, c.ordinal_position),
                   '') !~ '^dato_(personal|sensible):'",
    )
    .fetch_all(&pool)
    .await
    .expect("consulta");

    assert!(
        sin_clasificar.is_empty(),
        "sin COMMENT dato_personal: {sin_clasificar:?}"
    );
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_solicitud_recorre_su_ciclo_en_la_base(pool: PgPool) {
    use application::{
        SolicitudPagina,
        casos_uso::proteccion_datos::{
            listar_solicitudes, mis_datos, prorrogar_solicitud, responder_solicitud,
            tramitar_solicitud,
        },
        puertos::proteccion_datos::FiltroSolicitudes,
    };
    let (unidad, ctx, persona) = con_politica(&pool).await;
    registrar_autorizacion(
        &unidad,
        &RelojSistema,
        &ctx,
        otorgada(persona, &[Comunicaciones]),
    )
    .await
    .expect("autoriza");
    let s = radicar_solicitud(
        &unidad,
        &RelojSistema,
        &ctx,
        persona,
        TipoSolicitudTitular::Consulta,
        None,
    )
    .await
    .expect("radica");

    tramitar_solicitud(&unidad, &ctx, s.id)
        .await
        .expect("trámite");
    let p = prorrogar_solicitud(&unidad, &RelojSistema, &ctx, s.id, "Verificación")
        .await
        .expect("prórroga");
    responder_solicitud(
        &unidad,
        &RelojSistema,
        &ctx,
        s.id,
        Some("Sus datos son…".into()),
        None,
    )
    .await
    .expect("responde");

    let todas = listar_solicitudes(
        &unidad,
        &ctx,
        FiltroSolicitudes::default(),
        SolicitudPagina::default(),
    )
    .await
    .expect("listar");
    assert_eq!(todas.datos.len(), 1);
    assert_eq!(todas.datos[0].prorrogada_hasta, p.prorrogada_hasta);
    let vencidas = listar_solicitudes(
        &unidad,
        &ctx,
        FiltroSolicitudes {
            vencen_hasta: Some(chrono::NaiveDate::MAX),
            ..FiltroSolicitudes::default()
        },
        SolicitudPagina::default(),
    )
    .await
    .expect("listar");
    assert!(
        vencidas.datos.is_empty(),
        "respondida: no cuenta como vencida"
    );
    let datos = mis_datos(&unidad, &ctx, Some(persona))
        .await
        .expect("datos");
    assert_eq!(
        (datos.autorizaciones.len(), datos.solicitudes.len()),
        (1, 1)
    );

    // La base rechaza una respuesta vacía aunque alguien salte el caso de uso.
    let r = sqlx::query("UPDATE solicitudes_titular SET respuesta = NULL WHERE id = $1")
        .bind(s.id.as_uuid())
        .execute(&pool)
        .await;
    assert!(r.is_err());
}
