//! Pruebas de integración del repositorio de archivos como `dad_app`: persistencia, RLS,
//! restricciones del esquema y migración entre almacenes.

use application::{
    Contexto,
    puertos::{RepositorioArchivos, Transaccion, UnidadDeTrabajo, archivos::Archivo},
};
use chrono::{TimeZone, Utc};
use domain::{
    Almacen, ArchivoId, ClasificacionArchivo, ClaveArchivo, NombreArchivo, TenantId, TipoArchivo,
};
use infra_postgres::UnidadDeTrabajoPg;
use sqlx::PgPool;

mod comun;
use comun::{MIGRACIONES, crear_copropiedad, pool_como_app};

fn archivo(tenant: TenantId) -> Archivo {
    let id = ArchivoId::nuevo();
    Archivo {
        id,
        clave: ClaveArchivo::new(tenant, id),
        almacen: Almacen::Local,
        nombre_original: NombreArchivo::new("Acta asamblea 2026.pdf").expect("válido"),
        tipo: TipoArchivo::Pdf,
        tamano_bytes: 1024,
        sha256: "a".repeat(64),
        clasificacion: ClasificacionArchivo::Interno,
        subido_por: None,
        creado_en: Utc
            .with_ymd_and_hms(2026, 11, 6, 15, 0, 0)
            .single()
            .expect("fecha válida"),
    }
}

async fn insertar(app: &PgPool, tenant: TenantId, a: &Archivo) {
    let mut tx = UnidadDeTrabajoPg::new(app.clone())
        .iniciar(&Contexto::new(tenant))
        .await
        .expect("inicia");
    tx.insertar(a).await.expect("inserta");
    tx.confirmar().await.expect("confirma");
}

async fn buscar(app: &PgPool, tenant: TenantId, id: ArchivoId) -> Option<Archivo> {
    UnidadDeTrabajoPg::new(app.clone())
        .iniciar(&Contexto::new(tenant))
        .await
        .expect("inicia")
        .buscar(id)
        .await
        .expect("busca")
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn se_guarda_y_se_recupera_igual(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;
    let a = archivo(tenant);

    insertar(&app, tenant, &a).await;

    assert_eq!(buscar(&app, tenant, a.id).await, Some(a));
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn otra_copropiedad_no_ve_el_archivo(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let otra = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;
    let a = archivo(tenant);
    insertar(&app, tenant, &a).await;

    assert_eq!(buscar(&app, otra, a.id).await, None, "RLS");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_clave_debe_estar_bajo_la_carpeta_de_su_copropiedad(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let otra = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;
    let mut a = archivo(tenant);
    a.clave = ClaveArchivo::new(otra, a.id);

    let mut tx = UnidadDeTrabajoPg::new(app.clone())
        .iniciar(&Contexto::new(tenant))
        .await
        .expect("inicia");
    let err = tx.insertar(&a).await.expect_err("clave de otro tenant");

    assert!(err.to_string().contains("check"), "{err}");
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn la_app_no_puede_borrar_ni_cambiar_el_contenido_registrado(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;
    let a = archivo(tenant);
    insertar(&app, tenant, &a).await;

    for sql in [
        "DELETE FROM archivos",
        "UPDATE archivos SET sha256 = repeat('b', 64)",
    ] {
        let err = sqlx::query(sql).execute(&app).await.expect_err(sql);
        assert!(
            err.to_string().contains("permission denied"),
            "{sql}: {err}"
        );
    }
}

#[sqlx::test(migrator = "MIGRACIONES")]
async fn migracion_toma_archivos_del_almacen_y_cambia_su_almacen(pool: PgPool) {
    let tenant = crear_copropiedad(&pool).await;
    let app = pool_como_app(&pool).await;
    let (a, b) = (archivo(tenant), archivo(tenant));
    insertar(&app, tenant, &a).await;
    insertar(&app, tenant, &b).await;
    let unidad = UnidadDeTrabajoPg::new(app.clone());
    let ctx = Contexto::new(tenant);

    let mut tx = unidad.iniciar(&ctx).await.expect("inicia");
    let pendientes = tx.en_almacen(Almacen::Local, 1).await.expect("lista");
    assert_eq!(
        pendientes,
        vec![a.clone()],
        "orden por id, respeta el límite"
    );
    // Otra migración simultánea no toma el archivo bloqueado.
    let mut otra = unidad.iniciar(&ctx).await.expect("inicia");
    assert_eq!(
        otra.en_almacen(Almacen::Local, 10).await.expect("lista"),
        vec![b.clone()]
    );
    drop(otra);
    tx.cambiar_almacen(a.id, Almacen::S3).await.expect("cambia");
    tx.confirmar().await.expect("confirma");

    assert_eq!(
        buscar(&app, tenant, a.id).await.map(|x| x.almacen),
        Some(Almacen::S3)
    );
    let mut tx = unidad.iniciar(&ctx).await.expect("inicia");
    assert_eq!(
        tx.en_almacen(Almacen::Local, 10).await.expect("lista"),
        vec![b]
    );
    assert!(matches!(
        tx.cambiar_almacen(ArchivoId::nuevo(), Almacen::S3).await,
        Err(application::AppError::NoEncontrado("archivo"))
    ));
}
