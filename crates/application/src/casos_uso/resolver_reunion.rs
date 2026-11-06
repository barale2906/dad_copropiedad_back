//! ¿A qué reunión se refiere un mensaje de WhatsApp?
//!
//! La sesión se busca por (línea, teléfono) y se valida contra las reuniones activas en que el
//! teléfono está inscrito. Si hay varias y no hay una elegida vigente, se le pregunta con una
//! lista. Corrige el problema de la clave única `whatsapp:conv:{telefono}`.

use domain::ReunionId;

use crate::puertos::repositorios::ReunionActiva;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolucion {
    /// El teléfono no está inscrito en ninguna reunión activa de esta línea.
    Ninguna,
    Unica(ReunionActiva),
    /// Enviar una lista para que elija.
    Elegir(Vec<ReunionActiva>),
}

/// `elegida` es la reunión guardada en la sesión de conversación, si existe.
pub fn resolver_reunion(activas: Vec<ReunionActiva>, elegida: Option<ReunionId>) -> Resolucion {
    if let Some(id) = elegida
        && let Some(r) = activas.iter().find(|r| r.reunion_id == id)
    {
        return Resolucion::Unica(r.clone());
    }
    let mut activas = activas;
    match activas.len() {
        0 => Resolucion::Ninguna,
        1 => Resolucion::Unica(activas.remove(0)),
        _ => Resolucion::Elegir(activas),
    }
}

#[cfg(test)]
mod tests {
    use domain::{EstadoReunion, TenantId};

    use super::*;

    fn reunion() -> ReunionActiva {
        ReunionActiva {
            tenant_id: TenantId::nuevo(),
            reunion_id: ReunionId::nuevo(),
            titulo: "Asamblea ordinaria 2027".into(),
            estado: EstadoReunion::Instalada,
        }
    }

    #[test]
    fn casos() {
        assert_eq!(resolver_reunion(vec![], None), Resolucion::Ninguna);

        let a = reunion();
        assert_eq!(
            resolver_reunion(vec![a.clone()], None),
            Resolucion::Unica(a.clone())
        );

        let b = reunion();
        assert!(matches!(
            resolver_reunion(vec![a.clone(), b.clone()], None),
            Resolucion::Elegir(v) if v.len() == 2
        ));
        assert_eq!(
            resolver_reunion(vec![a.clone(), b.clone()], Some(b.reunion_id)),
            Resolucion::Unica(b)
        );
        // La sesión apunta a una reunión que ya no está activa: se vuelve a preguntar.
        assert!(matches!(
            resolver_reunion(vec![a, reunion()], Some(ReunionId::nuevo())),
            Resolucion::Elegir(_)
        ));
    }
}
