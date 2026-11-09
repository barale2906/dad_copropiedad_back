//! Pruebas funcionales de la descarga con enlace firmado (`GET /api/v1/archivos/descarga`):
//! el cliente usa la URL que generó el servidor y recibe el archivo, o el error uniforme si el
//! enlace fue alterado, venció o el archivo ya no existe.

use std::{path::PathBuf, sync::Arc};

use api::{routes, state::AppState};
use application::puertos::{AlmacenArchivos, archivos::DescargaArchivo};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    response::Response,
};
use chrono::Utc;
use domain::{ArchivoId, ClaveArchivo, NombreArchivo, TenantId, TipoArchivo};
use infra_archivos::{AlmacenLocal, FirmanteEnlaces};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

const URL_PUBLICA: &str = "https://api.dad.co";
const PDF: &[u8] = b"%PDF-1.7\nacta de asamblea";

fn enlaces() -> FirmanteEnlaces {
    FirmanteEnlaces::new(b"secreto-de-pruebas-de-32-bytes!!", URL_PUBLICA).expect("válido")
}

/// Directorio temporal del almacén; se borra al terminar la prueba.
struct Entorno {
    dir: PathBuf,
    almacen: Arc<AlmacenLocal>,
}

impl Entorno {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("dad-api-archivos-{}", uuid::Uuid::new_v4()));
        let almacen = Arc::new(AlmacenLocal::abrir(&dir, enlaces()).expect("abre"));
        Self { dir, almacen }
    }

    async fn pedir(&self, url: &str) -> Response {
        let config = shared::AppConfig {
            host: "127.0.0.1".into(),
            port: 0,
            database_url: "postgres://x:x@localhost:1/x".into(),
            migrations_database_url: None,
            database_max_connections: 1,
            run_migrations: false,
            request_timeout: std::time::Duration::from_secs(5),
            log_format: shared::LogFormat::Pretty,
            docs_api_habilitada: false,
        };
        let db = PgPoolOptions::new()
            .connect_lazy(&config.database_url)
            .expect("URL válida");
        let mut state = AppState::new(
            db,
            AlmacenLocal::abrir(&self.dir, enlaces()).expect("abre"),
            true,
            infra_seguridad::FirmaQrEd25519::new(&[5; 32]).expect("secreto de prueba"),
        );
        state.archivos = Arc::clone(&self.almacen);
        let ruta = url.strip_prefix(URL_PUBLICA).expect("URL de la API");
        routes::router(state, &config)
            .oneshot(Request::get(ruta).body(Body::empty()).expect("petición"))
            .await
            .expect("el router no falla")
    }

    /// Guarda un PDF y devuelve el enlace firmado que vence en `segundos`.
    async fn pdf_con_enlace(&self, segundos: i64) -> String {
        let clave = ClaveArchivo::new(TenantId::nuevo(), ArchivoId::nuevo());
        self.almacen.guardar(&clave, PDF).await.expect("guarda");
        self.almacen
            .url_temporal(&DescargaArchivo {
                clave,
                tipo: TipoArchivo::Pdf,
                nombre: NombreArchivo::new("Acta asamblea año 2026.pdf").expect("válido"),
                vence_en: Utc::now() + chrono::Duration::seconds(segundos),
            })
            .expect("firma")
    }
}

impl Drop for Entorno {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn codigo_error(res: Response) -> Value {
    let bytes = to_bytes(res.into_body(), usize::MAX).await.expect("cuerpo");
    let json: Value = serde_json::from_slice(&bytes).expect("JSON");
    json["error"]["codigo"].clone()
}

#[tokio::test]
async fn enlace_vigente_descarga_el_archivo_con_cabeceras_seguras() {
    let entorno = Entorno::new();
    let url = entorno.pdf_con_enlace(300).await;

    let res = entorno.pedir(&url).await;

    assert_eq!(res.status(), StatusCode::OK);
    let h = res.headers();
    assert_eq!(h[header::CONTENT_TYPE], "application/pdf");
    assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(h[header::CACHE_CONTROL], "private, no-store");
    assert_eq!(
        h[header::CONTENT_DISPOSITION],
        "inline; filename=\"Acta asamblea a_o 2026.pdf\"; \
         filename*=UTF-8''Acta%20asamblea%20a%C3%B1o%202026.pdf"
    );
    let cuerpo = to_bytes(res.into_body(), usize::MAX).await.expect("cuerpo");
    assert_eq!(&cuerpo[..], PDF);
}

#[tokio::test]
async fn enlace_alterado_responde_enlace_invalido() {
    let entorno = Entorno::new();
    let url = entorno.pdf_con_enlace(300).await;
    let alterada = url.replace("tipo=application%2Fpdf", "tipo=image%2Fpng");
    assert_ne!(alterada, url);

    let res = entorno.pedir(&alterada).await;

    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(codigo_error(res).await, "enlace_invalido");
}

#[tokio::test]
async fn enlace_vencido_responde_enlace_invalido() {
    let entorno = Entorno::new();
    let url = entorno.pdf_con_enlace(-1).await;

    let res = entorno.pedir(&url).await;

    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert_eq!(codigo_error(res).await, "enlace_invalido");
}

#[tokio::test]
async fn sin_parametros_responde_peticion_invalida() {
    let entorno = Entorno::new();

    let res = entorno
        .pedir(&format!("{URL_PUBLICA}/api/v1/archivos/descarga?clave=x"))
        .await;

    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(codigo_error(res).await, "peticion_invalida");
}

#[tokio::test]
async fn archivo_borrado_del_almacen_responde_no_encontrado() {
    let entorno = Entorno::new();
    let url = entorno.pdf_con_enlace(300).await;
    std::fs::remove_dir_all(&entorno.dir).expect("borra el almacén");

    let res = entorno.pedir(&url).await;

    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(codigo_error(res).await, "no_encontrado");
}
