//! Prueba de arquitectura (ADR 0017): los casos de uso y el puerto del núcleo contable sirven a
//! cualquier entidad y **no importan nada de la vertical PH** ni de sus módulos. Si un día lo
//! necesitan, el puente va en la vertical (que sí puede usar el núcleo), nunca al revés.

use std::{fs, path::Path};

/// Módulos de `application` que son de la PH o de sus módulos vendibles.
const PROHIBIDOS: &[&str] = &[
    "copropiedades",
    "bienes_privados",
    "coeficientes",
    "titulares",
    "censo",
    "campanas_censo",
    "asambleas",
    "reservas",
    "parqueaderos",
    "acceso",
    "tickets",
    "conversaciones",
    "mantenimiento",
    "comunicados",
    "directorio",
    "busqueda_proveedores",
    "publicidad",
    "chat",
];

fn fuentes() -> Vec<std::path::PathBuf> {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut v: Vec<_> = fs::read_dir(raiz.join("casos_uso/contabilidad"))
        .expect("casos_uso/contabilidad existe")
        .map(|e| e.expect("entrada").path())
        .collect();
    v.push(raiz.join("puertos/contabilidad.rs"));
    v.push(raiz.join("puertos/tesoreria.rs"));
    v.push(raiz.join("puertos/planillas.rs"));
    v
}

/// Segmentos de las rutas `crate::…` y `super::…` de un fuente.
fn segmentos(fuente: &str) -> Vec<String> {
    let mut v = Vec::new();
    for linea in fuente.lines().filter(|l| !l.trim_start().starts_with("//")) {
        for (i, _) in linea.match_indices("crate::") {
            v.extend(
                linea[i..]
                    .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    v
}

#[test]
fn el_nucleo_contable_no_importa_la_vertical_ph() {
    let fuentes = fuentes();
    assert!(
        fuentes.len() >= 12,
        "se revisaron {} fuentes",
        fuentes.len()
    );
    for ruta in fuentes {
        let texto = fs::read_to_string(&ruta).expect("fuente legible");
        for s in segmentos(&texto) {
            assert!(
                !PROHIBIDOS.contains(&s.as_str()),
                "{} usa `{s}`, que es de la vertical PH",
                ruta.display()
            );
        }
    }
}

#[test]
fn reconoce_las_rutas_de_crate() {
    let s =
        segmentos("use crate::{puertos::copropiedades::Persona, AppError};\n// crate::asambleas");
    assert!(s.contains(&"copropiedades".to_owned()));
    assert!(
        !s.contains(&"asambleas".to_owned()),
        "los comentarios no cuentan"
    );
}
