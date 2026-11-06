#[derive(Debug, thiserror::Error)]
pub enum InfraPgError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),

    #[error(transparent)]
    Migracion(#[from] sqlx::migrate::MigrateError),

    #[error(
        "el rol `{0}` es superusuario o tiene BYPASSRLS: la aplicación debe conectarse con \
         `dad_app` (APP_DATABASE_URL) para que RLS aísle los tenants"
    )]
    RolPrivilegiado(String),
}
