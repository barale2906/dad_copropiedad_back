//! Planilla de usuarios de una entidad (plan 20): una fila por usuario con su rol. Sin IO; la
//! existencia del rol y el no escalamiento los verifica el caso de uso.
//!
//! | Columna | Obligatoria | Ejemplo |
//! |---|---|---|
//! | `nombre` | sí | `Ana Pérez` |
//! | `correo` | sí | `ana@correo.co` |
//! | `rol` | sí (código del rol en la entidad) | `contador` |
//! | `usuario` | no | `ana.perez` |
//! | `telefono` | no | `573001234567` |
//! | `documento` | no | `52123456` |
//!
//! Un correo repetido en la planilla es un error; los mensajes nunca repiten el valor de la celda.

use std::collections::BTreeSet;

use super::{Documento, ErrorFila, normalizar_encabezado};
use crate::{DireccionCorreo, NombreUsuario, copropiedad::Telefono};

/// Filas de datos máximas de una planilla de usuarios.
pub const MAXIMO_FILAS_USUARIOS: usize = 2_000;

/// Caracteres máximos del nombre de un usuario (igual que `cuentas.nombre_mostrar`).
pub const LARGO_MAXIMO_NOMBRE: usize = 120;

/// Columnas de la planilla de usuarios.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Columna {
    Nombre,
    Correo,
    Rol,
    Usuario,
    Telefono,
    Documento,
}

impl Columna {
    const TODAS: [Self; 6] = [
        Self::Nombre,
        Self::Correo,
        Self::Rol,
        Self::Usuario,
        Self::Telefono,
        Self::Documento,
    ];

    const fn nombre(self) -> &'static str {
        match self {
            Self::Nombre => "nombre",
            Self::Correo => "correo",
            Self::Rol => "rol",
            Self::Usuario => "usuario",
            Self::Telefono => "telefono",
            Self::Documento => "documento",
        }
    }
}

/// Usuario leído de una fila válida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsuarioImportado {
    /// Número de la fila en la planilla (la 1 son los encabezados).
    pub fila: u32,
    pub nombre: String,
    pub correo: DireccionCorreo,
    /// Código del rol tal como viene (en minúsculas).
    pub rol: String,
    pub usuario: Option<NombreUsuario>,
    pub telefono: Option<Telefono>,
    pub documento: Option<Documento>,
}

/// Resultado de leer la planilla: los usuarios válidos y los errores por fila.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanillaUsuarios {
    pub usuarios: Vec<UsuarioImportado>,
    pub errores: Vec<ErrorFila>,
    /// Filas de datos no vacías.
    pub total_filas: u32,
}

/// Lee la planilla (la primera fila son los encabezados, sin importar mayúsculas ni tildes).
pub fn leer_planilla_usuarios(filas: &[Vec<String>]) -> PlanillaUsuarios {
    let mut p = PlanillaUsuarios::default();
    let Some((encabezado, datos)) = filas.split_first() else {
        p.errores
            .push(ErrorFila::new(1, None, "la planilla está vacía"));
        return p;
    };
    let columnas = match encabezados(encabezado) {
        Ok(c) => c,
        Err(errores) => {
            p.errores = errores;
            return p;
        }
    };
    let datos: Vec<(u32, &Vec<String>)> = datos
        .iter()
        .enumerate()
        .filter(|(_, f)| f.iter().any(|c| !c.trim().is_empty()))
        .map(|(i, f)| (u32::try_from(i + 2).unwrap_or(u32::MAX), f))
        .collect();
    p.total_filas = u32::try_from(datos.len()).unwrap_or(u32::MAX);
    if datos.len() > MAXIMO_FILAS_USUARIOS {
        p.errores.push(ErrorFila::new(
            1,
            None,
            format!("máximo {MAXIMO_FILAS_USUARIOS} filas por archivo"),
        ));
        return p;
    }
    let mut correos = BTreeSet::new();
    for (n, celdas) in datos {
        match leer_fila(n, celdas, &columnas) {
            Ok(u) if !correos.insert(u.correo.as_str().to_owned()) => p.errores.push(
                ErrorFila::new(n, Some("correo"), "el correo se repite en la planilla"),
            ),
            Ok(u) => p.usuarios.push(u),
            Err(errores) => p.errores.extend(errores),
        }
    }
    p
}

fn encabezados(fila: &[String]) -> Result<Vec<(Columna, usize)>, Vec<ErrorFila>> {
    let mut columnas: Vec<(Columna, usize)> = Vec::new();
    let mut errores = Vec::new();
    for (i, celda) in fila.iter().enumerate() {
        let nombre = normalizar_encabezado(celda);
        if let Some(c) = Columna::TODAS.iter().find(|c| c.nombre() == nombre) {
            if columnas.iter().any(|(x, _)| x == c) {
                errores.push(ErrorFila::new(1, Some(c.nombre()), "columna repetida"));
            } else {
                columnas.push((*c, i));
            }
        }
    }
    for obligatoria in [Columna::Nombre, Columna::Correo, Columna::Rol] {
        if !columnas.iter().any(|(c, _)| *c == obligatoria) {
            errores.push(ErrorFila::new(
                1,
                Some(obligatoria.nombre()),
                "falta la columna obligatoria",
            ));
        }
    }
    if errores.is_empty() {
        Ok(columnas)
    } else {
        Err(errores)
    }
}

fn leer_fila(
    n: u32,
    celdas: &[String],
    columnas: &[(Columna, usize)],
) -> Result<UsuarioImportado, Vec<ErrorFila>> {
    let celda = |c: Columna| {
        columnas
            .iter()
            .find(|(x, _)| *x == c)
            .and_then(|(_, i)| celdas.get(*i))
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    };
    let mut errores = Vec::new();
    let mut error = |c: Columna, mensaje: &str| {
        errores.push(ErrorFila::new(n, Some(c.nombre()), mensaje));
    };
    let nombre = match celda(Columna::Nombre) {
        Some(v) if v.chars().count() <= LARGO_MAXIMO_NOMBRE => Some(v.to_owned()),
        Some(_) => {
            error(Columna::Nombre, "el nombre supera 120 caracteres");
            None
        }
        None => {
            error(Columna::Nombre, "falta el nombre");
            None
        }
    };
    let correo = match celda(Columna::Correo).map(DireccionCorreo::new) {
        Some(Ok(c)) => Some(c),
        Some(Err(_)) => {
            error(Columna::Correo, "correo inválido");
            None
        }
        None => {
            error(Columna::Correo, "falta el correo");
            None
        }
    };
    let rol = celda(Columna::Rol).map(str::to_lowercase);
    if rol.is_none() {
        error(Columna::Rol, "falta el rol");
    }
    let usuario = celda(Columna::Usuario)
        .map(NombreUsuario::new)
        .transpose()
        .unwrap_or_else(|_| {
            error(Columna::Usuario, "nombre de usuario inválido");
            None
        });
    let telefono = celda(Columna::Telefono)
        .map(Telefono::new)
        .transpose()
        .unwrap_or_else(|_| {
            error(Columna::Telefono, "teléfono inválido");
            None
        });
    let documento = celda(Columna::Documento)
        .map(Documento::new)
        .transpose()
        .unwrap_or_else(|_| {
            error(Columna::Documento, "documento inválido");
            None
        });
    match (nombre, correo, rol) {
        (Some(nombre), Some(correo), Some(rol)) if errores.is_empty() => Ok(UsuarioImportado {
            fila: n,
            nombre,
            correo,
            rol,
            usuario,
            telefono,
            documento,
        }),
        _ => Err(errores),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planilla(filas: &[&[&str]]) -> Vec<Vec<String>> {
        filas
            .iter()
            .map(|f| f.iter().map(|c| (*c).to_owned()).collect())
            .collect()
    }

    #[test]
    fn lee_usuarios_validos_con_encabezados_en_cualquier_orden() {
        let p = leer_planilla_usuarios(&planilla(&[
            &["Correo", "ROL", "Nombre", "Teléfono"],
            &["ana@correo.co", "Contador", "Ana Pérez", "573001234567"],
            &["", "", "", ""],
            &["luis@correo.co", "portero", "Luis", ""],
        ]));
        assert!(p.errores.is_empty(), "{:?}", p.errores);
        assert_eq!(p.total_filas, 2);
        assert_eq!(p.usuarios.len(), 2);
        assert_eq!(p.usuarios[0].rol, "contador");
        assert_eq!(p.usuarios[1].fila, 4);
        assert!(p.usuarios[1].telefono.is_none());
    }

    #[test]
    fn faltan_columnas_obligatorias() {
        let p = leer_planilla_usuarios(&planilla(&[&["nombre", "usuario"]]));
        let columnas: Vec<_> = p.errores.iter().filter_map(|e| e.columna.clone()).collect();
        assert_eq!(columnas, vec!["correo", "rol"]);
    }

    #[test]
    fn errores_por_celda_sin_repetir_el_valor() {
        let p = leer_planilla_usuarios(&planilla(&[
            &["nombre", "correo", "rol", "telefono"],
            &["", "no-es-correo", "", "12"],
        ]));
        assert!(p.usuarios.is_empty());
        assert_eq!(p.errores.len(), 4);
        assert!(
            p.errores
                .iter()
                .all(|e| !e.mensaje.contains("no-es-correo"))
        );
    }

    #[test]
    fn correo_repetido_en_la_planilla_es_un_error() {
        let p = leer_planilla_usuarios(&planilla(&[
            &["nombre", "correo", "rol"],
            &["Ana", "ana@correo.co", "contador"],
            &["Ana B", "ANA@correo.co", "portero"],
        ]));
        assert_eq!(p.usuarios.len(), 1);
        assert_eq!(p.errores[0].fila, 3);
    }
}
