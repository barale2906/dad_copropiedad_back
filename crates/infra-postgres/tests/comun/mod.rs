//! Ayudantes compartidos por las pruebas de integración de `infra-postgres`.

use domain::TenantId;
use sqlx::{Executor, PgPool, postgres::PgPoolOptions};

/// Migraciones del repositorio (las aplica `#[sqlx::test]` en una BD aislada por prueba).
pub const MIGRACIONES: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Pool sobre la misma BD de la prueba, pero con el rol restringido de la aplicación
/// (`SET ROLE dad_app` en cada conexión), igual que en producción.
// Las pruebas que operan con SQL directo (garantías del plan 15) no la usan.
#[allow(dead_code)]
pub async fn pool_como_app(pool: &PgPool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|conn, _| {
            Box::pin(async move {
                conn.execute("SET ROLE dad_app").await?;
                Ok(())
            })
        })
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("pool con rol dad_app")
}

/// Crea una copropiedad (entidad + extensión de la vertical PH) con el dueño del esquema.
// Las pruebas de entidades sin vertical (tesorería) no la usan.
#[allow(dead_code)]
pub async fn crear_copropiedad(pool: &PgPool) -> TenantId {
    let tenant = insertar_entidad(pool, "PH de prueba").await;
    sqlx::query("INSERT INTO copropiedades (id) VALUES ($1)")
        .bind(tenant.as_uuid())
        .execute(pool)
        .await
        .expect("copropiedad");
    tenant
}

/// Crea una entidad sin vertical con el dueño del esquema.
pub async fn insertar_entidad(pool: &PgPool, nombre: &str) -> TenantId {
    let tenant = TenantId::nuevo();
    sqlx::query("INSERT INTO entidades (id, nombre) VALUES ($1, $2)")
        .bind(tenant.as_uuid())
        .bind(nombre)
        .execute(pool)
        .await
        .expect("entidad");
    tenant
}

/// Cofre de pruebas: "cifra" anteponiendo el contexto (descifrar con otro contexto falla).
#[allow(dead_code)]
pub struct CofrePrueba;

impl application::puertos::cofre::Cofre for CofrePrueba {
    fn cifrar(&self, contexto: &str, datos: &[u8]) -> application::AppResult<Vec<u8>> {
        let mut v = contexto.as_bytes().to_vec();
        v.push(0);
        v.extend_from_slice(datos);
        Ok(v)
    }

    fn descifrar(&self, contexto: &str, cifrado: &[u8]) -> application::AppResult<Vec<u8>> {
        let prefijo = [contexto.as_bytes(), &[0]].concat();
        cifrado
            .strip_prefix(prefijo.as_slice())
            .map(<[u8]>::to_vec)
            .ok_or_else(|| application::AppError::Validacion("otro contexto".into()))
    }
}
