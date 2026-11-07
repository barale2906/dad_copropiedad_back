//! Paginación por cursor (*keyset*) para los listados de los casos de uso.
//!
//! Se usa el `id` UUID v7 de la entidad como cursor: es único y crece con el tiempo de
//! creación, así que `WHERE id > $despues ORDER BY id LIMIT $limite + 1` es estable aunque se
//! inserten filas mientras alguien recorre las páginas, y aprovecha el índice de la clave
//! primaria (a diferencia de `OFFSET`, que se degrada con el número de filas).
//!
//! Flujo de un repositorio:
//! 1. Recibe una [`SolicitudPagina`] (ya validada).
//! 2. Pide [`SolicitudPagina::filas_a_consultar`] filas (una más que el límite).
//! 3. Construye la respuesta con [`Pagina::desde_filas`], que detecta si hay más páginas.

use uuid::Uuid;

use crate::{AppError, AppResult};

/// Parámetros de paginación validados: solo se construyen con un límite dentro del rango.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolicitudPagina {
    limite: u32,
    despues: Option<Uuid>,
}

impl SolicitudPagina {
    /// Límite cuando el cliente no envía uno.
    pub const LIMITE_POR_DEFECTO: u32 = 50;
    /// Máximo de elementos por página (protege a la BD de consultas enormes).
    pub const LIMITE_MAXIMO: u32 = 200;

    /// Valida los parámetros recibidos del cliente.
    ///
    /// `despues` es el cursor devuelto como `siguiente` en la página anterior; `None` pide la
    /// primera página.
    ///
    /// # Errors
    ///
    /// [`AppError::Validacion`] si `limite` es 0 o mayor que [`Self::LIMITE_MAXIMO`].
    pub fn new(limite: Option<u32>, despues: Option<Uuid>) -> AppResult<Self> {
        let limite = limite.unwrap_or(Self::LIMITE_POR_DEFECTO);
        if !(1..=Self::LIMITE_MAXIMO).contains(&limite) {
            return Err(AppError::Validacion(format!(
                "`limite` debe estar entre 1 y {}",
                Self::LIMITE_MAXIMO
            )));
        }
        Ok(Self { limite, despues })
    }

    /// Primera página con el límite por defecto.
    pub const fn primera() -> Self {
        Self {
            limite: Self::LIMITE_POR_DEFECTO,
            despues: None,
        }
    }

    /// Número máximo de elementos de la página.
    pub const fn limite(self) -> u32 {
        self.limite
    }

    /// Cursor: devolver solo elementos con `id` estrictamente mayor que este.
    pub const fn despues(self) -> Option<Uuid> {
        self.despues
    }

    /// Filas que el repositorio debe pedir (`LIMIT`): una más que el límite, para saber si
    /// existe una página siguiente sin hacer un `COUNT`.
    pub const fn filas_a_consultar(self) -> i64 {
        self.limite as i64 + 1
    }
}

impl Default for SolicitudPagina {
    fn default() -> Self {
        Self::primera()
    }
}

/// Una página de resultados y el cursor para pedir la siguiente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pagina<T> {
    /// Elementos de esta página, en orden ascendente de `id`.
    pub datos: Vec<T>,
    /// `id` del último elemento si hay más páginas; `None` si esta es la última.
    pub siguiente: Option<Uuid>,
}

impl<T> Pagina<T> {
    /// Construye la página a partir de las filas consultadas con
    /// [`SolicitudPagina::filas_a_consultar`] (ordenadas por `id` ascendente).
    ///
    /// Si llegaron más filas que el límite, descarta las sobrantes y usa el `id` del último
    /// elemento conservado como cursor `siguiente`.
    pub fn desde_filas(
        mut filas: Vec<T>,
        solicitud: SolicitudPagina,
        id: impl Fn(&T) -> Uuid,
    ) -> Self {
        let limite = solicitud.limite() as usize;
        let hay_mas = filas.len() > limite;
        filas.truncate(limite);
        let siguiente = if hay_mas { filas.last().map(id) } else { None };
        Self {
            datos: filas,
            siguiente,
        }
    }

    /// Transforma cada elemento conservando el cursor (p. ej. entidad → DTO).
    pub fn mapear<U>(self, f: impl FnMut(T) -> U) -> Pagina<U> {
        Pagina {
            datos: self.datos.into_iter().map(f).collect(),
            siguiente: self.siguiente,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<Uuid> {
        let mut v: Vec<Uuid> = (0..n).map(|_| Uuid::now_v7()).collect();
        v.sort();
        v
    }

    #[test]
    fn limite_por_defecto_y_rango_valido() {
        assert_eq!(
            SolicitudPagina::new(None, None)
                .map(SolicitudPagina::limite)
                .ok(),
            Some(SolicitudPagina::LIMITE_POR_DEFECTO)
        );
        assert!(SolicitudPagina::new(Some(1), None).is_ok());
        assert!(SolicitudPagina::new(Some(SolicitudPagina::LIMITE_MAXIMO), None).is_ok());
    }

    #[test]
    fn rechaza_limite_cero_o_excesivo() {
        for limite in [0, SolicitudPagina::LIMITE_MAXIMO + 1] {
            let err = SolicitudPagina::new(Some(limite), None);
            assert!(
                matches!(err, Err(AppError::Validacion(_))),
                "limite {limite}"
            );
        }
    }

    #[test]
    fn consulta_una_fila_mas_que_el_limite() {
        let s = SolicitudPagina::new(Some(10), None).expect("límite válido");
        assert_eq!(s.filas_a_consultar(), 11);
    }

    #[test]
    fn con_filas_sobrantes_recorta_y_devuelve_cursor() {
        let s = SolicitudPagina::new(Some(2), None).expect("límite válido");
        let filas = ids(3);

        let pagina = Pagina::desde_filas(filas.clone(), s, |id| *id);

        assert_eq!(pagina.datos, filas[..2]);
        assert_eq!(pagina.siguiente, Some(filas[1]));
    }

    #[test]
    fn ultima_pagina_no_tiene_cursor() {
        let s = SolicitudPagina::new(Some(2), None).expect("límite válido");
        let filas = ids(2);

        let pagina = Pagina::desde_filas(filas.clone(), s, |id| *id);

        assert_eq!(pagina.datos, filas);
        assert_eq!(pagina.siguiente, None);
    }

    #[test]
    fn mapear_conserva_el_cursor() {
        let s = SolicitudPagina::new(Some(1), None).expect("límite válido");
        let filas = ids(2);

        let pagina = Pagina::desde_filas(filas.clone(), s, |id| *id).mapear(|id| id.to_string());

        assert_eq!(pagina.datos, vec![filas[0].to_string()]);
        assert_eq!(pagina.siguiente, Some(filas[0]));
    }
}
