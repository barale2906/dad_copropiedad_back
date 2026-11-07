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

/// Reloj detenido en un instante (pruebas deterministas).
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct RelojFijo(pub DateTime<Utc>);

#[cfg(test)]
impl Reloj for RelojFijo {
    fn ahora(&self) -> DateTime<Utc> {
        self.0
    }
}
