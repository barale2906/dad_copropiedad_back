//! Pruebas de arquitectura:
//! - ADR 0017: los casos de uso y los puertos del núcleo contable, de lo tributario y de la
//!   facturación sirven a cualquier entidad y **no importan nada de la vertical PH** ni de sus
//!   módulos. Si un día lo necesitan, el puente va en la vertical, nunca al revés.
//! - ADR 0022: la facturación electrónica se vende sola. Puede usar el núcleo contable (solo si la
//!   entidad lo tiene), pero **la contabilidad no importa la facturación**.
//! - ADR 0025: la nómina también se vende sola y no conoce a la PH; **la contabilidad no importa
//!   la nómina**.

use std::{
    fs,
    path::{Path, PathBuf},
};

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

fn raiz() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Fuentes `.rs` de un directorio (sin subdirectorios: los módulos de casos de uso son planos).
fn de_directorio(dir: &str) -> Vec<PathBuf> {
    fs::read_dir(raiz().join(dir))
        .unwrap_or_else(|_| panic!("{dir} existe"))
        .map(|e| e.expect("entrada").path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect()
}

/// Fuentes de la contabilidad (casos de uso y puertos propios).
fn fuentes_contables() -> Vec<PathBuf> {
    let mut v = de_directorio("casos_uso/contabilidad");
    v.push(raiz().join("puertos/contabilidad.rs"));
    v.push(raiz().join("puertos/tesoreria.rs"));
    v.push(raiz().join("puertos/planillas.rs"));
    v
}

fn fuentes() -> Vec<PathBuf> {
    let raiz = raiz();
    let mut v = fuentes_contables();
    v.extend(de_directorio("casos_uso/facturacion"));
    v.push(raiz.join("casos_uso/tributario.rs"));
    v.push(raiz.join("puertos/facturacion.rs"));
    v.push(raiz.join("puertos/tributario.rs"));
    v.extend(de_directorio("casos_uso/nomina"));
    v.push(raiz.join("puertos/nomina.rs"));
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
fn la_contabilidad_no_importa_la_facturacion() {
    let fuentes = fuentes_contables();
    assert!(
        fuentes.len() >= 12,
        "se revisaron {} fuentes",
        fuentes.len()
    );
    for ruta in fuentes {
        let texto = fs::read_to_string(&ruta).expect("fuente legible");
        assert!(
            !segmentos(&texto).iter().any(|s| s == "facturacion"),
            "{} usa la facturación: la contabilidad no puede depender de ella (ADR 0022)",
            ruta.display()
        );
    }
}

#[test]
fn la_contabilidad_no_importa_la_nomina() {
    for ruta in fuentes_contables() {
        let texto = fs::read_to_string(&ruta).expect("fuente legible");
        assert!(
            !segmentos(&texto).iter().any(|s| s == "nomina"),
            "{} usa la nómina: la contabilidad no puede depender de ella (ADR 0025)",
            ruta.display()
        );
    }
}

#[test]
fn el_nucleo_contable_no_importa_la_vertical_ph() {
    let fuentes = fuentes();
    assert!(
        fuentes.len() >= 20,
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
