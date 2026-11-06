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
