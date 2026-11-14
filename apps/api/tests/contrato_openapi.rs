//! Prueba de contrato: el OpenAPI generado coincide con `apps/api/openapi.json`.
//!
//! El archivo versionado es el contrato que consumen el front web y las apps móviles para
//! generar sus clientes tipados. Si un cambio en las rutas o los DTOs altera el contrato, esta
//! prueba falla hasta que se regenere el archivo a propósito con `make openapi`
//! (`ACTUALIZAR_OPENAPI=1`), de modo que el cambio quede visible en el diff del PR.

use std::{env, fs, path::Path};

#[test]
fn openapi_generado_coincide_con_el_versionado() {
    let generado = format!(
        "{}\n",
        api::routes::openapi()
            .to_pretty_json()
            .expect("el documento OpenAPI se serializa")
    );
    let ruta = Path::new(env!("CARGO_MANIFEST_DIR")).join("openapi.json");

    if env::var_os("ACTUALIZAR_OPENAPI").is_some() {
        fs::write(&ruta, &generado).expect("se puede escribir openapi.json");
        return;
    }

    let versionado = fs::read_to_string(&ruta).unwrap_or_default();
    assert!(
        versionado == generado,
        "El contrato OpenAPI cambió. Revise el cambio y regenere con `make openapi`."
    );
}

/// Cada operación dice qué permiso exige su 403, para que el frontend sepa qué mostrar sin
/// adivinar: o nombra el permiso (`modulo.recurso.accion`), o explica la regla que no es un
/// permiso (titular, papeleta, enlace firmado…), o dice «No exige permisos». La descripción
/// genérica que pone `openapi::completar_respuestas_comunes` solo es un respaldo: si aparece, al
/// handler le falta declarar su `(status = FORBIDDEN, …)`.
#[test]
fn cada_prohibido_nombra_su_permiso() {
    let doc: serde_json::Value = serde_json::from_str(
        &api::routes::openapi()
            .to_json()
            .expect("el documento OpenAPI se serializa"),
    )
    .expect("JSON válido");
    let es_permiso = |texto: &str| {
        texto.split('`').skip(1).step_by(2).any(|t| {
            t.contains('.')
                && t.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.')
        })
    };
    let mut sin_nombre = Vec::new();
    for (ruta, item) in doc["paths"].as_object().expect("paths") {
        for (metodo, op) in item.as_object().expect("operaciones") {
            let Some(texto) = op["responses"]["403"]["description"].as_str() else {
                continue;
            };
            let generica = texto.to_lowercase().contains("permiso")
                && !es_permiso(texto)
                && !texto.starts_with("No exige permisos");
            if generica {
                sin_nombre.push(format!("{} {ruta}: {texto}", metodo.to_uppercase()));
            }
        }
    }
    assert!(
        sin_nombre.is_empty(),
        "Operaciones cuyo 403 no nombra el permiso (declare `(status = FORBIDDEN, …)` en el \
         handler):\n{}",
        sin_nombre.join("\n")
    );
}
