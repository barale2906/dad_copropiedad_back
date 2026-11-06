use chrono::{DateTime, Utc};

/// Fuente de tiempo. Inyectable para que las pruebas sean deterministas.
pub trait Reloj: Send + Sync {
    fn ahora(&self) -> DateTime<Utc>;
}

/// Reloj real del sistema (UTC).
#[derive(Debug, Clone, Copy, Default)]
pub struct RelojSistema;

impl Reloj for RelojSistema {
    fn ahora(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
