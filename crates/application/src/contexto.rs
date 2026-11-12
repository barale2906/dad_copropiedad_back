//! Contexto de una operación: la copropiedad (tenant), quién actúa y sus permisos. Los casos
//! de uso lo reciben y exigen permisos con `exigir`; `Contexto::new` es el del sistema.

use domain::{Ambito, ConjuntoPermisos, CuentaId, Permiso, TenantId};

use crate::{AppError, AppResult};

/// Contexto de ejecución de un caso de uso **dentro de una copropiedad**.
///
/// Todo acceso a datos se hace dentro de un tenant: el adaptador de PostgreSQL fija
/// `app.tenant_id` al iniciar cada transacción y las políticas RLS filtran el resto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contexto {
    pub tenant_id: TenantId,
    /// Cuenta autenticada que ejecuta la acción (`None` = sistema).
    pub actor_id: Option<CuentaId>,
    /// Correlación con logs y auditoría (`x-request-id` o `wamid`).
    pub request_id: Option<String>,
    /// Permisos efectivos del actor en esta copropiedad (unión de sus roles vigentes).
    pub permisos: ConjuntoPermisos,
}

impl Contexto {
    /// Contexto del **sistema** (tareas internas, workers, pruebas): sin actor y con todos los
    /// permisos de copropiedad. Las peticiones de usuarios usan [`Contexto::de_cuenta`].
    pub fn new(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            actor_id: None,
            request_id: None,
            permisos: ConjuntoPermisos::todos(Ambito::Copropiedad),
        }
    }

    /// Contexto de una cuenta autenticada con sus permisos en la copropiedad.
    pub fn de_cuenta(tenant_id: TenantId, cuenta: CuentaId, permisos: ConjuntoPermisos) -> Self {
        Self {
            tenant_id,
            actor_id: Some(cuenta),
            request_id: None,
            permisos,
        }
    }

    /// Agrega el `request_id` para la auditoría.
    pub fn con_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }

    /// # Errors
    ///
    /// [`AppError::SinPermiso`] si el actor no tiene el permiso.
    pub fn exigir(&self, permiso: Permiso) -> AppResult<()> {
        exigir(&self.permisos, permiso)
    }
}

/// Contexto de un caso de uso **de plataforma** (cuentas, sesiones, roles de plataforma, altas
/// de copropiedades). No ve datos de ninguna copropiedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextoPlataforma {
    pub actor_id: Option<CuentaId>,
    pub request_id: Option<String>,
    /// Permisos de plataforma del actor.
    pub permisos: ConjuntoPermisos,
}

impl ContextoPlataforma {
    /// Sistema: todos los permisos de plataforma (arranque, comandos de operación, pruebas).
    pub fn sistema() -> Self {
        Self {
            actor_id: None,
            request_id: None,
            permisos: ConjuntoPermisos::todos(Ambito::Plataforma),
        }
    }

    /// Petición sin sesión (registro, inicio de sesión, recuperar contraseña).
    pub fn anonimo(request_id: Option<String>) -> Self {
        Self {
            actor_id: None,
            request_id,
            permisos: ConjuntoPermisos::vacio(),
        }
    }

    /// Cuenta autenticada con sus permisos de plataforma.
    pub fn de_cuenta(
        cuenta: CuentaId,
        permisos: ConjuntoPermisos,
        request_id: Option<String>,
    ) -> Self {
        Self {
            actor_id: Some(cuenta),
            request_id,
            permisos,
        }
    }

    /// # Errors
    ///
    /// [`AppError::SinPermiso`] si el actor no tiene el permiso.
    pub fn exigir(&self, permiso: Permiso) -> AppResult<()> {
        exigir(&self.permisos, permiso)
    }

    /// La cuenta del actor.
    ///
    /// # Errors
    ///
    /// [`AppError::NoAutenticado`] si no hay sesión.
    pub fn cuenta(&self) -> AppResult<CuentaId> {
        self.actor_id.ok_or(AppError::NoAutenticado)
    }
}

fn exigir(permisos: &ConjuntoPermisos, permiso: Permiso) -> AppResult<()> {
    if permisos.contiene(permiso) {
        Ok(())
    } else {
        Err(AppError::SinPermiso(permiso))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exigir_permiso() {
        let ctx = Contexto::de_cuenta(
            TenantId::nuevo(),
            CuentaId::nuevo(),
            [Permiso::RolesVer].into_iter().collect(),
        );
        assert!(ctx.exigir(Permiso::RolesVer).is_ok());
        assert!(matches!(
            ctx.exigir(Permiso::RolesAsignar),
            Err(AppError::SinPermiso(Permiso::RolesAsignar))
        ));
        assert!(
            Contexto::new(TenantId::nuevo())
                .exigir(Permiso::RolesAsignar)
                .is_ok()
        );
        assert!(
            ContextoPlataforma::anonimo(None)
                .exigir(Permiso::PlataformaCuentasVer)
                .is_err()
        );
    }
}
