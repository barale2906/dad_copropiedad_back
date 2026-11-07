//! Comandos de operación de la plataforma.
//!
//! ```text
//! DAD_ADMIN_CONTRASENA='…' cargo run --bin dad_admin -- crear-superusuario \
//!     --usuario admin --correo admin@dominio.co --nombre "Nombre Apellido"
//! ```
//!
//! Crea la cuenta (o reutiliza la que tenga ese usuario) y le asigna el rol superusuario. La
//! contraseña se lee de `DAD_ADMIN_CONTRASENA` para que no quede en el historial del shell.

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
    if args.first().map(String::as_str) != Some("crear-superusuario") {
        bail!(
            "uso: dad_admin crear-superusuario --usuario U --correo C --nombre N \
             (contraseña en DAD_ADMIN_CONTRASENA)"
        );
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
