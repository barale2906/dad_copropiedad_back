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

/// Cada código de error que nombra una respuesta de error entre paréntesis (`` (`conflicto`) ``
/// o `` (`validacion`, `conflicto`) ``) existe en `CodigoError`: el cliente del frontend solo conoce los valores de ese enum, y un código
/// inventado en la documentación (como lo fue `dominio`) nunca llega en una respuesta.
#[test]
fn cada_error_documentado_usa_un_codigo_del_catalogo() {
    let doc: serde_json::Value = serde_json::from_str(
        &api::routes::openapi()
            .to_json()
            .expect("el documento OpenAPI se serializa"),
    )
    .expect("JSON válido");
    let codigos: Vec<&str> = doc["components"]["schemas"]["CodigoError"]["enum"]
        .as_array()
        .expect("CodigoError es un enum")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    let parece_codigo =
        |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_lowercase() || c == '_');
    let mut desconocidos = Vec::new();
    for (ruta, item) in doc["paths"].as_object().expect("paths") {
        for (metodo, op) in item.as_object().expect("operaciones") {
            let Some(respuestas) = op["responses"].as_object() else {
                continue;
            };
            for (estado, r) in respuestas {
                if !estado.starts_with('4') && !estado.starts_with('5') {
                    continue;
                }
                let texto = r["description"].as_str().unwrap_or_default();
                let entre_parentesis = texto
                    .split("(`")
                    .skip(1)
                    .map(|resto| resto.split(')').next().unwrap_or_default());
                for grupo in entre_parentesis {
                    let primero = grupo.split('`').next().unwrap_or_default();
                    let siguientes = grupo.split('`').skip(2).step_by(2);
                    for t in std::iter::once(primero).chain(siguientes) {
                        if parece_codigo(t) && !codigos.contains(&t) {
                            desconocidos
                                .push(format!("{} {ruta} {estado}: `{t}`", metodo.to_uppercase()));
                        }
                    }
                }
            }
        }
    }
    assert!(
        desconocidos.is_empty(),
        "Respuestas que nombran un código que no está en `CodigoError`:\n{}",
        desconocidos.join("\n")
    );
}

/// Cada esquema del contrato tiene un nombre propio. utoipa registra los esquemas por nombre y,
/// si dos DTO se llaman igual (`ObligacionRespuesta` de cartera y de mantenimiento), publica
/// **uno solo** sin avisar: las rutas del otro quedan con tipos equivocados en los clientes
/// generados. Revisa el código (no el documento, donde la colisión ya no se ve): todo
/// `derive(ToSchema)` se nombra por su `#[schema(as = …)]` o por su identificador.
#[test]
fn cada_esquema_tiene_un_nombre_unico() {
    use std::collections::BTreeMap;

    fn archivos(dir: &Path, salida: &mut Vec<std::path::PathBuf>) {
        for entrada in fs::read_dir(dir).expect("se puede leer src") {
            let ruta = entrada.expect("entrada").path();
            if ruta.is_dir() {
                archivos(&ruta, salida);
            } else if ruta.extension().is_some_and(|e| e == "rs") {
                salida.push(ruta);
            }
        }
    }

    fn nombre_de_tipo(linea: &str) -> Option<&str> {
        let resto = [
            "pub struct ",
            "pub enum ",
            "pub(crate) struct ",
            "pub(crate) enum ",
            "struct ",
            "enum ",
        ]
        .iter()
        .find_map(|p| linea.strip_prefix(p))?;
        resto
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .filter(|n| !n.is_empty())
    }

    let mut rutas = Vec::new();
    archivos(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut rutas,
    );
    let mut nombres: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ruta in rutas {
        let texto = fs::read_to_string(&ruta).expect("archivo legible");
        let mut esquema = false;
        let mut alias: Option<String> = None;
        for linea in texto.lines().map(str::trim) {
            if linea.starts_with("#[derive(") {
                esquema = linea.contains("ToSchema");
                alias = None;
            } else if esquema && let Some(resto) = linea.strip_prefix("#[schema(as = ") {
                alias = resto.split(')').next().map(str::to_owned);
            } else if esquema && let Some(n) = nombre_de_tipo(linea) {
                let nombre = alias.take().unwrap_or_else(|| n.to_owned());
                nombres
                    .entry(nombre)
                    .or_default()
                    .push(ruta.display().to_string());
                esquema = false;
            }
        }
    }
    let repetidos: Vec<String> = nombres
        .into_iter()
        .filter(|(_, rutas)| rutas.len() > 1)
        .map(|(n, rutas)| format!("{n}: {}", rutas.join(", ")))
        .collect();
    assert!(
        repetidos.is_empty(),
        "Esquemas con el mismo nombre (agregue `#[schema(as = NombrePropio)]` a uno):\n{}",
        repetidos.join("\n")
    );
}

/// Cada operación tiene un `operationId` propio: los generadores de clientes agrupan los tipos
/// por ese nombre y, si se repite, una ruta toma los tipos de otra. `openapi::nombrar_operaciones`
/// antepone la etiqueta del módulo; si aun así dos se llaman igual, uno lleva
/// `operation_id = "…"` en su `#[utoipa::path]`.
#[test]
fn cada_operacion_tiene_un_id_unico() {
    use std::collections::BTreeMap;

    let doc: serde_json::Value = serde_json::from_str(
        &api::routes::openapi()
            .to_json()
            .expect("el documento OpenAPI se serializa"),
    )
    .expect("JSON válido");
    let mut ids: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (ruta, item) in doc["paths"].as_object().expect("paths") {
        for (metodo, op) in item.as_object().expect("operaciones") {
            let id = op["operationId"].as_str().unwrap_or_default().to_owned();
            ids.entry(id)
                .or_default()
                .push(format!("{} {ruta}", metodo.to_uppercase()));
        }
    }
    let repetidos: Vec<String> = ids
        .into_iter()
        .filter(|(id, rutas)| id.is_empty() || rutas.len() > 1)
        .map(|(id, rutas)| format!("{id:?}: {}", rutas.join(", ")))
        .collect();
    assert!(
        repetidos.is_empty(),
        "Operaciones sin `operationId` propio:\n{}",
        repetidos.join("\n")
    );
}
