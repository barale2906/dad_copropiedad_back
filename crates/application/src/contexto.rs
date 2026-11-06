use domain::{PersonaId, TenantId};

/// Contexto de ejecución de un caso de uso.
///
/// Todo acceso a datos se hace **dentro de un tenant**: el adaptador de PostgreSQL ejecuta
/// `set_config('app.tenant_id', …, true)` al iniciar cada transacción y las políticas RLS
/// filtran el resto. Un caso de uso nunca opera "sin tenant".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contexto {
    pub tenant_id: TenantId,
    /// Persona autenticada que ejecuta la acción (`None` = sistema).
    pub actor_id: Option<PersonaId>,
    /// Correlación con logs y auditoría (`x-request-id` o `wamid`).
    pub request_id: Option<String>,
}

impl Contexto {
    pub const fn new(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            actor_id: None,
            request_id: None,
        }
    }
}
