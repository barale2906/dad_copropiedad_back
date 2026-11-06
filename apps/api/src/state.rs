use infra_postgres::PgPool;

/// Estado compartido entre handlers. Clonarlo es barato (el pool es un `Arc` interno).
///
/// Aquí se inyectarán los adaptadores (repositorios Postgres, publicador de resultados…)
/// que consumen los casos de uso de `application`.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
}
