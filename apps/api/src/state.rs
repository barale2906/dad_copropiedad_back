use std::sync::Arc;

use infra_archivos::AlmacenLocal;
use infra_postgres::PgPool;

/// Estado compartido entre handlers. Clonarlo es barato (todo va detrás de `Arc`).
///
/// Aquí se inyectan los adaptadores (repositorios Postgres, almacén de archivos, publicador de
/// resultados…) que consumen los casos de uso de `application`.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// Almacén de archivos configurado (`ARCHIVOS_ALMACEN`). Hoy solo el disco local.
    pub archivos: Arc<AlmacenLocal>,
}
