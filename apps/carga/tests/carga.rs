//! Prueba funcional de la herramienta: levanta el router real de la API en un puerto local
//! (PostgreSQL con el rol `dad_app`), siembra dos copropiedades y ejecuta el escenario. Verifica
//! que la siembra deja todo listo y que la votación simultánea registra un voto por asistente
//! sin errores.

use std::time::Duration;

use api::{routes, state::AppState};
use carga::{
    cliente::Cliente,
    escenario::{OpcionesEjecucion, ejecutar},
    informe,
    siembra::{OpcionesSiembra, sembrar},
};
use infra_archivos::{AlmacenLocal, FirmanteEnlaces};
use infra_seguridad::FirmaQrEd25519;
use serde_json::json;
use sqlx::{Executor, PgPool, postgres::PgPoolOptions};

static MIGRACIONES: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
const CONTRASENA: &str = "una frase de carga larga";

/// API real escuchando en `127.0.0.1:<puerto libre>`; devuelve su URL.
async fn levantar_api(duenio: &PgPool) -> String {
    let app = PgPoolOptions::new()
        .max_connections(16)
        .after_connect(|c, _| {
            Box::pin(async move {
                c.execute("SET ROLE dad_app").await?;
                Ok(())
            })
        })
        .connect_with((*duenio.connect_options()).clone())
        .await
        .expect("pool dad_app");
    let dir = std::env::temp_dir().join(format!("dad-carga-{}", uuid::Uuid::new_v4()));
    let enlaces = FirmanteEnlaces::new(&[9; 32], "http://localhost:8080").expect("firmante");
    let almacen = AlmacenLocal::abrir(&dir, enlaces).expect("almacén");
    let config = shared::AppConfig {
        host: "127.0.0.1".into(),
        port: 0,
        database_url: String::new(),
        migrations_database_url: None,
        database_max_connections: 16,
        run_migrations: false,
        request_timeout: Duration::from_secs(30),
        log_format: shared::LogFormat::Pretty,
        docs_api_habilitada: false,
        cors_origenes: Vec::new(),
    };
    let state = AppState::new(
        app,
        almacen,
        true,
        FirmaQrEd25519::new(&[5; 32]).expect("secreto"),
    );
    let router = routes::router(state, &config);
    let escucha = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("puerto");
    let url = format!("http://{}", escucha.local_addr().expect("dirección"));
    tokio::spawn(async move { axum::serve(escucha, router).await });
    url
}

/// Superusuario como lo deja `dad_admin crear-superusuario` (sin segundo factor todavía).
async fn superusuario(c: &Cliente, duenio: &PgPool) {
    let v = c
        .pedir(
            "registro",
            reqwest::Method::POST,
            "/api/v1/auth/registro",
            None,
            Some(&json!({ "nombre": "Root", "usuario": "root", "contrasena": CONTRASENA })),
        )
        .await
        .exigir(&[201], "registrar root")
        .expect("root");
    sqlx::query(
        "INSERT INTO asignaciones_plataforma (id, cuenta_id, rol_id, vigente_desde)
         VALUES (gen_random_uuid(), $1::uuid, uuid_rol_plantilla('superusuario'), now())",
    )
    .bind(v["id"].as_str().expect("id"))
    .execute(duenio)
    .await
    .expect("superusuario");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn siembra_y_ejecucion_registran_un_voto_por_asistente_sin_errores(duenio: PgPool) {
    let url = levantar_api(&duenio).await;
    let c = Cliente::new(&url).expect("cliente");
    superusuario(&c, &duenio).await;
    let siembra = OpcionesSiembra {
        copropiedades: 2,
        bienes: 6,
        contrasena: CONTRASENA.into(),
        root_usuario: "root".into(),
        root_contrasena: CONTRASENA.into(),
        root_totp: None,
    };

    let plan = sembrar(&c, &siembra).await.expect("siembra");

    assert_eq!(plan.copropiedades.len(), 2);
    assert!(plan.copropiedades.iter().all(|p| p.asistentes.len() == 6));
    // Repetir la siembra sobre el mismo entorno se rechaza (los datos no se pueden borrar).
    assert!(sembrar(&c, &siembra).await.is_err());

    let opciones = OpcionesEjecucion {
        duracion: Duration::from_secs(3),
        usuarios_por_copropiedad: 2,
        pausa: Duration::from_millis(50),
        concurrencia_votos: 4,
        inicio_votacion: Duration::from_millis(500),
        contrasena: CONTRASENA.into(),
    };
    let resultado = ejecutar(&c, &plan, &opciones).await.expect("ejecución");

    assert_eq!(
        (resultado.votos_registrados, resultado.votos_esperados),
        (12, 12)
    );
    assert_eq!(
        informe::errores_por_mil(&resultado),
        0,
        "{}",
        informe::texto(&resultado)
    );
    let operaciones: Vec<&str> = resultado
        .resumenes
        .iter()
        .map(|r| r.operacion.as_str())
        .collect();
    for op in [
        "bienes",
        "capacidades",
        "quorum",
        "tablero",
        "votar",
        "total",
    ] {
        assert!(operaciones.contains(&op), "falta {op}: {operaciones:?}");
    }
    // La ejecución abre su propia pregunta: se puede repetir sobre la misma siembra.
    let otra = ejecutar(&c, &plan, &opciones)
        .await
        .expect("segunda ejecución");
    assert_eq!(otra.votos_registrados, 12);
}
