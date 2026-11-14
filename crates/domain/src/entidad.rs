//! La entidad (empresa) como tenant (ADR 0016): cualquier empresa de la plataforma. Una vertical
//! la extiende con su propia tabla; hoy la única es la propiedad horizontal (fila en
//! `copropiedades` con el mismo id). La plataforma misma también es una entidad (GISLA SAS), con
//! esa función marcada (`entidades.es_plataforma`).

use serde::{Deserialize, Serialize};

/// Qué es la entidad, para que el cliente sepa qué mostrar al entrar. Una persona entra a
/// gestionar una propiedad horizontal (sus menús: bienes privados, censo, asambleas…), una empresa
/// cliente (lo común a toda empresa: contabilidad, facturación…) o la plataforma (su consola de
/// operación, además de lo común para su propia empresa).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipoEntidad {
    /// Copropiedad sometida a la Ley 675 de 2001.
    PropiedadHorizontal,
    /// Empresa cliente sin vertical propia (tienda, gimnasio, firma contable…).
    Empresa,
    /// La entidad que opera la plataforma (una sola; nunca una copropiedad).
    Plataforma,
}

enum_texto!(TipoEntidad {
    PropiedadHorizontal => "propiedad_horizontal",
    Empresa => "empresa",
    Plataforma => "plataforma",
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texto_persistido_ida_y_vuelta() {
        for t in TipoEntidad::TODOS {
            assert_eq!(TipoEntidad::try_from(t.as_str()), Ok(*t));
        }
        assert_eq!(TipoEntidad::TODOS.len(), 3);
        assert!(TipoEntidad::try_from("gimnasio").is_err());
    }
}
