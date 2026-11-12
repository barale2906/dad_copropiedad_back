//! `dad_carga`: siembra y prueba de carga contra un entorno de prueba.
//!
//! ```text
//! CARGA_CONTRASENA='…' CARGA_ROOT_USUARIO=admin CARGA_ROOT_CONTRASENA='…' \
//!   dad_carga sembrar --api http://127.0.0.1:8080 --copropiedades 20 --bienes 300 --plan carga.json
//!
//! CARGA_CONTRASENA='…' dad_carga ejecutar --api http://127.0.0.1:8080 --plan carga.json \
//!   --duracion 120 --usuarios 5 --concurrencia-votos 10 --salida informe.json
//! ```
//!
//! `ejecutar` termina con error si más de `--max-errores-por-mil` peticiones (10 por defecto)
//! no fueron 2xx. Ver `docs/despliegue.md` §9.

use std::{io::Write as _, time::Duration};

use anyhow::{Context, bail};
use carga::{
    cliente::Cliente,
    escenario::{OpcionesEjecucion, ejecutar},
    informe,
    siembra::{OpcionesSiembra, Plan, sembrar},
};

const USO: &str = "uso: dad_carga sembrar --api URL --copropiedades N --bienes M --plan ARCHIVO\n\
     \x20      dad_carga ejecutar --api URL --plan ARCHIVO [--duracion S] [--usuarios K] \
     [--pausa-ms MS] [--concurrencia-votos C] [--inicio-votacion S] [--salida ARCHIVO] \
     [--max-errores-por-mil E]\n\
     variables: CARGA_CONTRASENA; para sembrar, CARGA_ROOT_USUARIO, CARGA_ROOT_CONTRASENA y, si \
     ya tiene segundo factor, CARGA_ROOT_TOTP";

fn argumento(args: &[String], nombre: &str) -> Option<String> {
    args.iter()
        .position(|a| a == nombre)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn numero(args: &[String], nombre: &str, defecto: u64) -> anyhow::Result<u64> {
    argumento(args, nombre).map_or(Ok(defecto), |v| {
        v.parse()
            .with_context(|| format!("{nombre} debe ser un número"))
    })
}

fn entorno(nombre: &str) -> anyhow::Result<String> {
    std::env::var(nombre).with_context(|| format!("defina {nombre}"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let api = argumento(&args, "--api").context(USO)?;
    let plan_ruta = argumento(&args, "--plan").context(USO)?;
    let cliente = Cliente::new(&api)?;
    let contrasena = entorno("CARGA_CONTRASENA")?;
    match args.first().map(String::as_str) {
        Some("sembrar") => {
            let o = OpcionesSiembra {
                copropiedades: usize::try_from(numero(&args, "--copropiedades", 5)?)?,
                bienes: usize::try_from(numero(&args, "--bienes", 100)?)?,
                contrasena,
                root_usuario: entorno("CARGA_ROOT_USUARIO")?,
                root_contrasena: entorno("CARGA_ROOT_CONTRASENA")?,
                root_totp: std::env::var("CARGA_ROOT_TOTP").ok(),
            };
            let plan = sembrar(&cliente, &o).await?;
            std::fs::write(&plan_ruta, serde_json::to_vec_pretty(&plan)?)
                .with_context(|| format!("guardar {plan_ruta}"))?;
            eprintln!("plan guardado en {plan_ruta}");
            Ok(())
        }
        Some("ejecutar") => {
            let plan: Plan = serde_json::from_slice(
                &std::fs::read(&plan_ruta).with_context(|| format!("leer {plan_ruta}"))?,
            )
            .context("el plan no es válido")?;
            let o = OpcionesEjecucion {
                duracion: Duration::from_secs(numero(&args, "--duracion", 60)?),
                usuarios_por_copropiedad: usize::try_from(numero(&args, "--usuarios", 5)?)?,
                pausa: Duration::from_millis(numero(&args, "--pausa-ms", 300)?),
                concurrencia_votos: usize::try_from(numero(&args, "--concurrencia-votos", 10)?)?,
                inicio_votacion: Duration::from_secs(numero(&args, "--inicio-votacion", 5)?),
                contrasena,
            };
            let resultado = ejecutar(&cliente, &plan, &o).await?;
            let mut salida = std::io::stdout().lock();
            salida.write_all(informe::texto(&resultado).as_bytes())?;
            if let Some(ruta) = argumento(&args, "--salida") {
                std::fs::write(&ruta, serde_json::to_vec_pretty(&resultado)?)
                    .with_context(|| format!("guardar {ruta}"))?;
            }
            let tope = numero(&args, "--max-errores-por-mil", 10)?;
            let errores = informe::errores_por_mil(&resultado);
            if errores > tope || resultado.votos_registrados < resultado.votos_esperados {
                bail!(
                    "la prueba no pasó: {errores} errores por mil (tope {tope}), votos {}/{}",
                    resultado.votos_registrados,
                    resultado.votos_esperados
                );
            }
            Ok(())
        }
        _ => bail!(USO),
    }
}
