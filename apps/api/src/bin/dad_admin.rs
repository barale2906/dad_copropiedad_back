//! Comandos de operación de la plataforma.
//!
//! ```text
//! DAD_ADMIN_CONTRASENA='…' cargo run --bin dad_admin -- crear-superusuario \
//!     --usuario admin --correo admin@dominio.co --nombre "Nombre Apellido"
//! ```
//!
//! Crea la cuenta (o reutiliza la que tenga ese usuario) y le asigna el rol superusuario. La
//! contraseña se lee de `DAD_ADMIN_CONTRASENA` para que no quede en el historial del shell.
//!
//! ```text
//! DATABASE_URL=postgres://dueno… dad_admin migrar
//! ```
//!
//! Aplica las migraciones pendientes con el **dueño del esquema** (`DATABASE_URL`). Es el paso
//! explícito del despliegue: la API y el gateway nunca migran al arrancar en producción.

use anyhow::{Context, bail};
use application::{
    AppError, ContextoPlataforma,
    casos_uso::{
        autenticacion::{Registro, registrar_cuenta},
        plataforma::asignar_rol_plataforma,
    },
    puertos::{RelojSistema, RepositorioCuentas, RepositorioPlataforma, UnidadDeTrabajoPlataforma},
};
use domain::{
    ContrasenaNueva, DireccionCorreo, Identificador, NombreUsuario, permisos::CODIGO_SUPERUSUARIO,
};
use infra_postgres::UnidadDeTrabajoPlataformaPg;
use infra_seguridad::Argon2Contrasenas;
use shared::AppConfig;

fn argumento(args: &[String], nombre: &str) -> Option<String> {
    args.iter()
        .position(|a| a == nombre)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("crear-superusuario") => {}
        Some("migrar") => return migrar().await,
        _ => bail!(
            "uso: dad_admin crear-superusuario --usuario U --correo C --nombre N \
             (contraseña en DAD_ADMIN_CONTRASENA) | dad_admin migrar (dueño en DATABASE_URL)"
        ),
    }
    let usuario = NombreUsuario::new(&argumento(&args, "--usuario").context("falta --usuario")?)?;
    let correo = argumento(&args, "--correo")
        .map(|c| DireccionCorreo::new(&c))
        .transpose()?;
    let nombre = argumento(&args, "--nombre").unwrap_or_else(|| usuario.as_str().to_owned());
    let contrasena = std::env::var("DAD_ADMIN_CONTRASENA")
        .context("defina DAD_ADMIN_CONTRASENA con la contraseña del superusuario")?;
    let contrasena = ContrasenaNueva::new(&contrasena)?;

    let config = AppConfig::from_env().context("configuración inválida")?;
    let db = infra_postgres::conectar(&config.database_url, 2).await?;
    infra_postgres::verificar_rol_restringido(&db).await?;
    let plataforma = UnidadDeTrabajoPlataformaPg::new(db);
    let sistema = ContextoPlataforma::sistema();

    let cuenta = match registrar_cuenta(
        &plataforma,
        &Argon2Contrasenas,
        &RelojSistema,
        &sistema,
        Registro {
            nombre_mostrar: nombre,
            usuario: Some(usuario.clone()),
            correo,
            contrasena,
        },
    )
    .await
    {
        Ok(id) => id,
        Err(AppError::Conflicto(_)) => {
            plataforma
                .iniciar_plataforma(&sistema)
                .await?
                .cuenta_por_identificador(&Identificador::Usuario(usuario.clone()))
                .await?
                .context("el correo ya pertenece a otra cuenta")?
                .id
        }
        Err(e) => return Err(e.into()),
    };
    let rol = plataforma
        .iniciar_plataforma(&sistema)
        .await?
        .rol_plataforma_por_codigo(CODIGO_SUPERUSUARIO)
        .await?
        .context("falta el rol superusuario: ¿se aplicaron las migraciones?")?;
    asignar_rol_plataforma(&plataforma, &RelojSistema, &sistema, cuenta, rol.id).await?;
    eprintln!(
        "Superusuario listo: usuario `{}` (cuenta {cuenta}). Inicie sesión en POST /api/v1/auth/sesion.",
        usuario.as_str()
    );
    Ok(())
}

/// Aplica las migraciones con el dueño del esquema.
async fn migrar() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let url = std::env::var("DATABASE_URL")
        .context("defina DATABASE_URL con el dueño del esquema (no el rol dad_app)")?;
    infra_postgres::migrar(&url)
        .await
        .context("no se pudieron aplicar las migraciones")?;
    eprintln!("migraciones aplicadas");
    Ok(())
}
