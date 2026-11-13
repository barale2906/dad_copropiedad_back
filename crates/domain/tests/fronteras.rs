//! Prueba de arquitectura (ADR 0017): el núcleo contable (`domain::contabilidad`) sirve a
//! cualquier entidad y **no conoce a la PH**. Solo puede usar del resto del crate lo que está
//! en la lista; cualquier otra ruta `crate::…` hace fallar la prueba.

use std::{fs, path::Path};

/// Lo que el núcleo contable puede usar fuera de su módulo.
const PERMITIDOS: &[&str] = &[
    "DominioError",
    "CentroCostoId",
    "CuentaContableId",
    "PersonaId",
    "Nit",
];

/// Rutas `crate::…` de un fuente (incluidas las de `use crate::{A, B::C}`), por su primer
/// segmento.
fn rutas_de_crate(fuente: &str) -> Vec<String> {
    let mut rutas = Vec::new();
    let mut resto = fuente;
    while let Some(i) = resto.find("crate::") {
        resto = &resto[i + "crate::".len()..];
        if let Some(grupo) = resto.strip_prefix('{') {
            let fin = grupo.find('}').unwrap_or(grupo.len());
            rutas.extend(
                grupo[..fin]
                    .split(',')
                    .map(|p| p.trim().split("::").next().unwrap_or("").to_owned())
                    .filter(|p| !p.is_empty()),
            );
        } else {
            let ident: String = resto
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            rutas.push(ident);
        }
    }
    rutas
}

#[test]
fn el_nucleo_contable_no_conoce_a_la_ph() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/contabilidad");
    let mut revisados = 0;
    for entrada in fs::read_dir(&dir).expect("src/contabilidad existe") {
        let ruta = entrada.expect("entrada").path();
        let fuente = fs::read_to_string(&ruta).expect("fuente legible");
        for r in rutas_de_crate(&fuente) {
            assert!(
                PERMITIDOS.contains(&r.as_str()),
                "{} usa `crate::{r}`: el núcleo contable solo puede usar {PERMITIDOS:?}",
                ruta.display()
            );
        }
        revisados += 1;
    }
    assert!(revisados >= 5, "se revisaron {revisados} fuentes");
}

#[test]
fn la_lectura_de_rutas_reconoce_grupos_y_rutas_sueltas() {
    let rutas = rutas_de_crate("use crate::{DominioError, ids::X};\nlet a = crate::Nit::new;");
    assert_eq!(rutas, vec!["DominioError", "ids", "Nit"]);
}
