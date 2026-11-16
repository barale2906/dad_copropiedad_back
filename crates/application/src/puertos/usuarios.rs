//! Usuarios de una entidad (plan 20): crear o reconocer la cuenta de quien trabaja allí, emitir
//! el código para que defina su contraseña y listar las cuentas con roles en la entidad.
//!
//! Lo implementa la transacción de la **entidad** ([`crate::puertos::UnidadDeTrabajo`]). Las
//! cuentas son de la plataforma: el adaptador cruza de ámbito solo con funciones `SECURITY
//! DEFINER` que hacen lo mínimo (AGENTS.md §5, dos ámbitos de transacción). Así crear el
//! usuario, asignarle el rol y vincular su persona ocurre en **una** transacción.

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::{CuentaId, DireccionCorreo, EstadoCuenta, NombreUsuario};

use crate::{AppResult, Pagina, SolicitudPagina, puertos::copropiedades::Persona};

/// Datos de la cuenta que pide crear la entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuentaSolicitada {
    /// Id para la cuenta si hay que crearla.
    pub id: CuentaId,
    pub nombre_mostrar: String,
    pub usuario: Option<NombreUsuario>,
    pub correo: DireccionCorreo,
}

/// Qué pasó con la cuenta pedida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CuentaResuelta {
    /// No había cuenta con ese correo: se creó con la contraseña pendiente.
    Creada(CuentaId),
    /// Ya había una cuenta activa con ese correo (la persona trabaja en otras entidades).
    Existente {
        id: CuentaId,
        nombre_mostrar: String,
        /// La persona aún no define su contraseña.
        pendiente: bool,
    },
    /// La cuenta con ese correo está bloqueada o eliminada.
    Inactiva,
    /// El nombre de usuario pedido ya es de otra cuenta.
    UsuarioOcupado,
}

/// Usuario de la entidad: cuenta con algún rol vigente allí.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsuarioEntidad {
    pub id: CuentaId,
    pub nombre_mostrar: String,
    pub usuario: Option<NombreUsuario>,
    pub correo: Option<DireccionCorreo>,
    pub estado: EstadoCuenta,
    /// Aún no definió su contraseña.
    pub contrasena_pendiente: bool,
    pub ultimo_acceso_en: Option<DateTime<Utc>>,
    /// Códigos de sus roles vigentes en la entidad, en orden alfabético.
    pub roles: Vec<String>,
}

/// Filtro del listado de usuarios.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroUsuarios {
    /// Fragmento del nombre (sin tildes ni mayúsculas), del correo o del usuario.
    pub texto: Option<String>,
}

/// Usuarios de la entidad de la transacción.
pub trait RepositorioUsuarios: Send {
    /// La cuenta activa con ese correo, o una nueva con la contraseña pendiente.
    fn resolver_cuenta(
        &mut self,
        solicitada: &CuentaSolicitada,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<CuentaResuelta>> + Send;

    /// Guarda el código (solo su hash) para que la cuenta defina su contraseña. Solo se emite
    /// para una cuenta pendiente con algún rol en la entidad: `false` si no aplica.
    fn emitir_codigo_definir(
        &mut self,
        cuenta: CuentaId,
        hash_codigo: &str,
        expira_en: DateTime<Utc>,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Usuarios de la entidad (por cursor de id).
    fn usuarios(
        &mut self,
        filtro: &FiltroUsuarios,
        pagina: SolicitudPagina,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Pagina<UsuarioEntidad>>> + Send;

    /// Un usuario de la entidad; `None` si la cuenta no tiene roles vigentes allí.
    fn usuario(
        &mut self,
        cuenta: CuentaId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<UsuarioEntidad>>> + Send;

    /// Persona de la entidad vinculada a la cuenta, si existe.
    fn persona_por_cuenta(
        &mut self,
        cuenta: CuentaId,
    ) -> impl Future<Output = AppResult<Option<Persona>>> + Send;
}
