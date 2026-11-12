//! Siembra **por la API** (ningún acceso directo a la base de datos): N copropiedades de
//! prueba con M bienes privados cada una, sus propietarios (importación masiva), el cuadro de
//! coeficientes activo, el módulo de asambleas y una asamblea instalada con todos los
//! propietarios registrados en la mesa.
//!
//! Las copropiedades se llaman `Carga NNN` y sus administradores `carga_admin_NNN`. Las personas
//! no tienen teléfono ni correo: la siembra no envía mensajes a nadie. Se ejecuta **solo en un
//! entorno de prueba** (ver `docs/despliegue.md` §9): los datos no se pueden borrar después
//! (auditoría y votos son append-only).

use std::{sync::Arc, time::Duration};

use anyhow::{Context, bail};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{sync::Semaphore, task::JoinSet};

use crate::cliente::{Cliente, Cuerpo};

/// Prefijo de las copropiedades sembradas: si ya existe alguna, la siembra se niega a repetir.
pub const PREFIJO: &str = "Carga ";

/// Cuántas peticiones de la siembra van en paralelo dentro de una copropiedad.
const PARALELO: usize = 8;

/// Qué sembrar y con qué credenciales.
#[derive(Debug, Clone)]
pub struct OpcionesSiembra {
    /// Número de copropiedades.
    pub copropiedades: usize,
    /// Bienes privados por copropiedad.
    pub bienes: usize,
    /// Contraseña de las cuentas de administrador que se crean (mínimo 10 caracteres).
    pub contrasena: String,
    /// Superusuario existente (`dad_admin crear-superusuario`).
    pub root_usuario: String,
    /// Su contraseña.
    pub root_contrasena: String,
    /// Su secreto TOTP en Base32, si ya tiene el segundo factor activo.
    pub root_totp: Option<String>,
}

/// Lo sembrado: lo que necesita la ejecución.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Una entrada por copropiedad.
    pub copropiedades: Vec<PlanCopropiedad>,
}

/// Una copropiedad sembrada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanCopropiedad {
    /// Id de la copropiedad.
    pub id: String,
    /// Usuario de su administrador.
    pub admin: String,
    /// Asamblea instalada.
    pub reunion: String,
    /// Asistentes presentes (uno por bien).
    pub asistentes: Vec<String>,
}

/// Inicia sesión reintentando si el limitador responde 429 (respeta `Retry-After`).
///
/// # Errors
///
/// Si la API rechaza las credenciales.
pub async fn iniciar_sesion(c: &Cliente, usuario: &str, contrasena: &str) -> anyhow::Result<Value> {
    let cuerpo = json!({ "identificador": usuario, "contrasena": contrasena });
    for _ in 0..10 {
        let r = c
            .pedir(
                "iniciar_sesion",
                Method::POST,
                "/api/v1/auth/sesion",
                None,
                Some(&cuerpo),
            )
            .await;
        if r.estado == 429 {
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue;
        }
        return r.exigir(&[200], &format!("iniciar sesión como {usuario}"));
    }
    bail!("el limitador no dejó iniciar sesión como {usuario}")
}

/// Token de acceso de una cuenta sin segundo factor.
///
/// # Errors
///
/// Si no inicia sesión o la cuenta pide segundo factor.
pub async fn token(c: &Cliente, usuario: &str, contrasena: &str) -> anyhow::Result<String> {
    let v = iniciar_sesion(c, usuario, contrasena).await?;
    v["tokens"]["token_acceso"]
        .as_str()
        .map(str::to_owned)
        .with_context(|| format!("{usuario}: la sesión pide segundo factor"))
}

fn codigo_totp(secreto: &str) -> anyhow::Result<String> {
    let clave = infra_seguridad::base32::decodificar(secreto).context("secreto TOTP en Base32")?;
    infra_seguridad::codigo_totp(&clave, chrono::Utc::now().timestamp() / 30).context("código TOTP")
}

/// Token del superusuario con segundo factor. Si aún no lo tiene activo, lo activa y muestra el
/// secreto por la salida de error para guardarlo en `CARGA_ROOT_TOTP`.
async fn token_plataforma(c: &Cliente, o: &OpcionesSiembra) -> anyhow::Result<String> {
    let mut secreto = o.root_totp.clone();
    loop {
        let inicio = iniciar_sesion(c, &o.root_usuario, &o.root_contrasena).await?;
        if inicio["resultado"] == "segundo_factor" {
            let s = secreto.as_deref().context(
                "el superusuario tiene segundo factor: defina CARGA_ROOT_TOTP con su secreto",
            )?;
            let v = c
                .pedir(
                    "segundo_factor",
                    Method::POST,
                    "/api/v1/auth/sesion/segundo-factor",
                    None,
                    Some(&json!({ "desafio": inicio["desafio"], "codigo": codigo_totp(s)? })),
                )
                .await
                .exigir(&[200], "segundo factor del superusuario")?;
            return v["token_acceso"]
                .as_str()
                .map(str::to_owned)
                .context("token");
        }
        let t = inicio["tokens"]["token_acceso"]
            .as_str()
            .context("token del superusuario")?
            .to_owned();
        let prueba = c
            .pedir(
                "listar_copropiedades",
                Method::GET,
                "/api/v1/plataforma/copropiedades",
                Some(&t),
                None,
            )
            .await;
        if prueba.estado != 403 {
            return Ok(t);
        }
        // La plataforma exige segundo factor y la cuenta no lo tiene: se activa una vez.
        let v = c
            .pedir(
                "segundo_factor",
                Method::POST,
                "/api/v1/yo/segundo-factor",
                Some(&t),
                None,
            )
            .await
            .exigir(&[200], "activar el segundo factor del superusuario")?;
        let s = v["secreto"].as_str().context("secreto TOTP")?.to_owned();
        c.pedir(
            "segundo_factor",
            Method::POST,
            "/api/v1/yo/segundo-factor/confirmar",
            Some(&t),
            Some(&json!({ "codigo": codigo_totp(&s)? })),
        )
        .await
        .exigir(&[204], "confirmar el segundo factor")?;
        eprintln!(
            "segundo factor activado para {}: guarde CARGA_ROOT_TOTP={s}",
            o.root_usuario
        );
        secreto = Some(s);
    }
}

/// Coeficientes de `n` bienes que suman exactamente 1 (8 decimales): el último absorbe el
/// residuo del redondeo.
pub fn coeficientes(n: usize) -> Vec<String> {
    const ESCALA: u64 = 100_000_000;
    let n64 = n as u64;
    let base = ESCALA / n64.max(1);
    (0..n64)
        .map(|i| {
            let v = if i + 1 == n64 {
                ESCALA - base * (n64 - 1)
            } else {
                base
            };
            format!("0.{v:08}")
        })
        .collect()
}

/// Planilla CSV de la importación (formato de la plantilla de la API, separada por `;`).
pub fn planilla(copropiedad: usize, bienes: usize) -> String {
    let mut csv = String::from(
        "agrupacion;numero;codigo;tipo;area_m2;coeficiente;propietario;documento;telefono;correo;porcentaje\n",
    );
    for (j, coef) in coeficientes(bienes).iter().enumerate() {
        let torre = j / 100 + 1;
        csv.push_str(&format!(
            "T{torre};{j};B-{j:04};apartamento;60;{coef};Propietario {j};9{copropiedad:03}{j:05};;;100\n"
        ));
    }
    csv
}

/// Siembra las copropiedades una tras otra (cada una en paralelo por dentro).
///
/// # Errors
///
/// Si ya hay una siembra, o si la API rechaza un paso.
pub async fn sembrar(c: &Cliente, o: &OpcionesSiembra) -> anyhow::Result<Plan> {
    if o.copropiedades == 0 || o.bienes < 2 {
        bail!("siembre al menos una copropiedad con dos bienes");
    }
    let root = token_plataforma(c, o).await?;
    let existentes = c
        .pedir(
            "listar_copropiedades",
            Method::GET,
            "/api/v1/plataforma/copropiedades",
            Some(&root),
            None,
        )
        .await
        .exigir(&[200], "listar copropiedades")?;
    if existentes.as_array().is_some_and(|l| {
        l.iter()
            .any(|p| p["nombre"].as_str().is_some_and(|n| n.starts_with(PREFIJO)))
    }) {
        bail!(
            "ya hay copropiedades «{PREFIJO}…»: use `ejecutar` con el plan guardado o un entorno nuevo"
        );
    }
    let mut plan = Plan {
        copropiedades: Vec::new(),
    };
    for i in 0..o.copropiedades {
        plan.copropiedades.push(sembrar_una(c, o, &root, i).await?);
        eprintln!("sembrada {}/{}", i + 1, o.copropiedades);
    }
    Ok(plan)
}

async fn sembrar_una(
    c: &Cliente,
    o: &OpcionesSiembra,
    root: &str,
    i: usize,
) -> anyhow::Result<PlanCopropiedad> {
    let admin = format!("carga_admin_{i:03}");
    let cuenta = c
        .pedir(
            "registro",
            Method::POST,
            "/api/v1/auth/registro",
            None,
            Some(&json!({ "nombre": format!("Administración {i:03}"), "usuario": admin, "contrasena": o.contrasena })),
        )
        .await
        .exigir(&[201], "registrar el administrador")?;
    let ph = c
        .pedir(
            "alta_copropiedad",
            Method::POST,
            "/api/v1/plataforma/copropiedades",
            Some(root),
            Some(&json!({
                "nombre": format!("{PREFIJO}{i:03}"),
                "administrador_cuenta_id": cuenta["id"],
                "tipo": "residencial",
            })),
        )
        .await
        .exigir(&[201], "dar de alta la copropiedad")?;
    let id = ph["id"]
        .as_str()
        .context("id de la copropiedad")?
        .to_owned();
    c.pedir(
        "habilitar_modulo",
        Method::POST,
        &format!("/api/v1/plataforma/copropiedades/{id}/modulos"),
        Some(root),
        Some(&json!({ "modulo": "asambleas" })),
    )
    .await
    .exigir(&[201], "habilitar asambleas")?;

    let t = token(c, &admin, &o.contrasena).await?;
    let base = format!("/api/v1/copropiedades/{id}");
    importar_bienes(c, &t, &base, i, o.bienes).await?;
    let reunion = asamblea_en_registro(c, &t, &base, o.bienes).await?;
    let asistentes =
        registrar_propietarios(c, &t, &format!("{base}/asambleas/{reunion}"), o.bienes).await?;
    c.pedir(
        "transicion",
        Method::POST,
        &format!("{base}/asambleas/{reunion}/transiciones"),
        Some(&t),
        Some(&json!({ "accion": "instalar" })),
    )
    .await
    .exigir(&[200], "instalar la asamblea")?;
    Ok(PlanCopropiedad {
        id,
        admin,
        reunion,
        asistentes,
    })
}

async fn importar_bienes(
    c: &Cliente,
    t: &str,
    base: &str,
    i: usize,
    bienes: usize,
) -> anyhow::Result<()> {
    let csv = planilla(i, bienes);
    let vista = c
        .pedir_con(
            "importar",
            Method::POST,
            &format!("{base}/importaciones/bienes-privados?nombre=carga.csv"),
            Some(t),
            &[],
            Cuerpo::Bytes(csv.as_bytes()),
        )
        .await
        .exigir(&[201], "subir la planilla")?;
    let errores = &vista["importacion"]["errores"];
    if errores.as_array().is_some_and(|e| !e.is_empty()) {
        bail!("la planilla tiene errores: {errores}");
    }
    let importacion = vista["importacion"]["id"]
        .as_str()
        .context("id de la importación")?;
    let aplicada = c
        .pedir(
            "importar",
            Method::POST,
            &format!("{base}/importaciones/{importacion}/confirmar"),
            Some(t),
            None,
        )
        .await
        .exigir(&[200], "confirmar la importación")?;
    let cuadro = aplicada["resumen"]["cuadro_borrador_id"]
        .as_str()
        .context("la importación no dejó cuadro de coeficientes")?;
    c.pedir(
        "activar_cuadro",
        Method::POST,
        &format!("{base}/cuadros-coeficientes/{cuadro}/activar"),
        Some(t),
        None,
    )
    .await
    .exigir(&[200], "activar el cuadro de coeficientes")?;
    Ok(())
}

async fn asamblea_en_registro(
    c: &Cliente,
    t: &str,
    base: &str,
    bienes: usize,
) -> anyhow::Result<String> {
    let fecha = (chrono::Utc::now() + chrono::Duration::days(20)).to_rfc3339();
    let r = c
        .pedir(
            "asamblea",
            Method::POST,
            &format!("{base}/asambleas"),
            Some(t),
            Some(&json!({
                "titulo": "Asamblea de carga", "tipo": "ordinaria", "modalidad": "presencial",
                "fecha_programada": fecha, "lugar": "Salón comunal",
            })),
        )
        .await
        .exigir(&[201], "crear la asamblea")?;
    let reunion = r["id"].as_str().context("id de la reunión")?.to_owned();
    let ruta = format!("{base}/asambleas/{reunion}");
    c.pedir(
        "asamblea",
        Method::POST,
        &format!("{ruta}/puntos"),
        Some(t),
        Some(&json!({ "orden": 1, "titulo": "Quórum" })),
    )
    .await
    .exigir(&[201], "agregar el orden del día")?;
    let conv = c
        .pedir(
            "asamblea",
            Method::POST,
            &format!("{ruta}/convocatoria"),
            Some(t),
            Some(&json!({})),
        )
        .await
        .exigir(&[200], "convocar")?;
    if conv["bienes_padron"].as_u64() != Some(bienes as u64) {
        bail!("el padrón no tiene los {bienes} bienes: {conv}");
    }
    c.pedir(
        "transicion",
        Method::POST,
        &format!("{ruta}/transiciones"),
        Some(t),
        Some(&json!({ "accion": "abrir_registro" })),
    )
    .await
    .exigir(&[200], "abrir el registro")?;
    Ok(reunion)
}

/// Registra en la mesa al propietario de cada bien (busca por código, como la mesa real).
async fn registrar_propietarios(
    c: &Cliente,
    t: &str,
    ruta: &str,
    bienes: usize,
) -> anyhow::Result<Vec<String>> {
    let limite = Arc::new(Semaphore::new(PARALELO));
    let mut tareas = JoinSet::new();
    for j in 0..bienes {
        let (c, t, ruta, limite) = (c.clone(), t.to_owned(), ruta.to_owned(), limite.clone());
        tareas.spawn(async move {
            let _turno = limite.acquire_owned().await.context("semáforo")?;
            let m = c
                .pedir(
                    "mesa_buscar",
                    Method::GET,
                    &format!("{ruta}/mesa?texto=B-{j:04}"),
                    Some(&t),
                    None,
                )
                .await
                .exigir(&[200], "buscar en la mesa")?;
            let b = m
                .as_array()
                .and_then(|l| l.first())
                .context("bien en el padrón")?;
            let reg = c
                .pedir(
                    "mesa_registrar",
                    Method::POST,
                    &format!("{ruta}/asistentes"),
                    Some(&t),
                    Some(&json!({
                        "persona_id": b["representante_persona_id"],
                        "bienes_propios": [b["bien_privado_id"]],
                        "enviar_confirmacion": false,
                    })),
                )
                .await
                .exigir(&[201], "registrar al propietario")?;
            reg["asistente"]["id"]
                .as_str()
                .map(str::to_owned)
                .context("id del asistente")
        });
    }
    let mut asistentes = Vec::with_capacity(bienes);
    while let Some(r) = tareas.join_next().await {
        asistentes.push(r.context("tarea de registro")??);
    }
    Ok(asistentes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_coeficientes_suman_exactamente_uno() {
        for n in [2, 3, 7, 300, 1000] {
            let total: u64 = coeficientes(n)
                .iter()
                .map(|c| c.trim_start_matches("0.").parse::<u64>().expect("entero"))
                .sum();
            assert_eq!(total, 100_000_000, "{n} bienes");
        }
    }

    #[test]
    fn la_planilla_trae_una_fila_por_bien_con_documento_unico() {
        let csv = planilla(4, 3);
        let filas: Vec<&str> = csv.lines().skip(1).collect();
        assert_eq!(filas.len(), 3);
        assert!(filas[2].starts_with("T1;2;B-0002;apartamento"));
        assert!(filas[1].contains(";900400001;"));
    }
}
