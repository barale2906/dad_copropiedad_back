//! Puertos de identidad, roles y organizaciones (plan 02).
//!
//! - [`UnidadDeTrabajoPlataforma`]: transacciones de **plataforma** (cuentas, sesiones, roles de
//!   plataforma, organizaciones). Su transacción implementa [`RepositorioCuentas`],
//!   [`RepositorioSesiones`], [`RepositorioPlataforma`] y [`RepositorioOrganizaciones`], además
//!   de la auditoría (`auditoria_plataforma`) y el outbox (mensajes sin copropiedad).
//! - [`RepositorioRoles`] y [`RepositorioContratos`]: dentro de la transacción de una
//!   copropiedad ([`crate::puertos::UnidadDeTrabajo`]).
//! - [`HashContrasenas`] y [`SegundoFactor`]: criptografía (adaptador `infra-seguridad`).

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::{
    Ambito, AsignacionRolId, ConjuntoPermisos, ContrasenaNueva, ContratoServicioId, CuentaId,
    DireccionCorreo, EstadoContrato, EstadoCuenta, EstadoOrganizacion, EstadoReclamacion,
    Identificador, NaturalezaOrganizacion, Nit, NombreUsuario, OrganizacionId, OrigenAsignacion,
    ReclamacionId, RolId, RolOrganizacion, SesionId, TenantId, TipoOrganizacion,
};
use uuid::Uuid;

use crate::{AppResult, ContextoPlataforma, puertos::Transaccion};

/// Abre transacciones de plataforma.
pub trait UnidadDeTrabajoPlataforma: Send + Sync {
    /// Transacción del ámbito de plataforma que entrega [`Self::iniciar_plataforma`].
    type Transaccion: Transaccion;

    /// Abre una transacción con `app.plataforma = on` (cuentas, sesiones, roles de plataforma).
    fn iniciar_plataforma(
        &self,
        ctx: &ContextoPlataforma,
    ) -> impl Future<Output = AppResult<Self::Transaccion>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Cuentas
// ---------------------------------------------------------------------------------------------

/// Cuenta tal como se guarda.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cuenta {
    pub id: CuentaId,
    pub nombre_mostrar: String,
    pub usuario: Option<NombreUsuario>,
    pub correo: Option<DireccionCorreo>,
    pub correo_verificado: bool,
    pub estado: EstadoCuenta,
    pub hash_contrasena: String,
    pub intentos_fallidos: u32,
    pub bloqueo_hasta: Option<DateTime<Utc>>,
    /// Secreto TOTP (pendiente de confirmar si `totp_activo` es `false`).
    pub totp_secreto: Option<String>,
    pub totp_activo: bool,
    pub creada_en: DateTime<Utc>,
}

/// Para qué sirve un código de un solo uso. El texto es contrato con la BD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropositoCodigo {
    VerificarCorreo,
    RecuperarContrasena,
}

domain::enum_texto!(PropositoCodigo {
    VerificarCorreo => "verificar_correo",
    RecuperarContrasena => "recuperar_contrasena",
});

/// Código de un solo uso guardado (solo el hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodigoVerificacion {
    pub id: Uuid,
    pub cuenta_id: CuentaId,
    pub proposito: PropositoCodigo,
    pub hash_codigo: String,
    pub expira_en: DateTime<Utc>,
    pub intentos: u32,
    pub creado_en: DateTime<Utc>,
}

/// Cuentas de personas y sus códigos de un solo uso.
pub trait RepositorioCuentas: Send {
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el usuario o el correo ya existen.
    fn insertar_cuenta(&mut self, cuenta: &Cuenta) -> impl Future<Output = AppResult<()>> + Send;

    /// Cuenta por correo o nombre de usuario exactos.
    fn cuenta_por_identificador(
        &mut self,
        identificador: &Identificador,
    ) -> impl Future<Output = AppResult<Option<Cuenta>>> + Send;

    /// Cuenta por su id.
    fn cuenta_por_id(
        &mut self,
        id: CuentaId,
    ) -> impl Future<Output = AppResult<Option<Cuenta>>> + Send;

    /// Anota un intento fallido (y el bloqueo, si corresponde).
    fn registrar_fallo(
        &mut self,
        id: CuentaId,
        intentos: u32,
        bloqueo_hasta: Option<DateTime<Utc>>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Inicio de sesión correcto: reinicia los intentos y guarda la hora.
    fn registrar_acceso(
        &mut self,
        id: CuentaId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Reemplaza el hash de la contraseña.
    fn cambiar_contrasena(
        &mut self,
        id: CuentaId,
        hash: &str,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Marca el correo de la cuenta como verificado.
    fn marcar_correo_verificado(
        &mut self,
        id: CuentaId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda (o borra, con `None`) el secreto TOTP y su activación.
    fn guardar_totp(
        &mut self,
        id: CuentaId,
        secreto: Option<&str>,
        activado_en: Option<DateTime<Utc>>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Cambia el estado de la cuenta.
    fn cambiar_estado(
        &mut self,
        id: CuentaId,
        estado: EstadoCuenta,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda un código de un solo uso (solo su hash).
    fn insertar_codigo(
        &mut self,
        codigo: &CodigoVerificacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// El código más reciente sin usar y sin vencer de ese propósito.
    fn codigo_vigente(
        &mut self,
        cuenta: CuentaId,
        proposito: PropositoCodigo,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<CodigoVerificacion>>> + Send;

    /// Suma un intento fallido al código.
    fn registrar_intento_codigo(&mut self, id: Uuid) -> impl Future<Output = AppResult<()>> + Send;

    /// Marca el código como usado: ya no sirve otra vez.
    fn marcar_codigo_usado(
        &mut self,
        id: Uuid,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Sesiones
// ---------------------------------------------------------------------------------------------

/// Sesión guardada. Los tokens solo existen como hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sesion {
    pub id: SesionId,
    pub cuenta_id: CuentaId,
    pub hash_acceso: String,
    pub acceso_expira_en: DateTime<Utc>,
    pub hash_refresh: Option<String>,
    /// Vencimiento absoluto: ni renovando se pasa de aquí (30 días; 12 h en equipo compartido).
    pub expira_maximo_en: DateTime<Utc>,
    /// Inició con contraseña y espera el código TOTP: todavía no sirve para nada más.
    pub segundo_factor_pendiente: bool,
    pub segundo_factor_verificado: bool,
    pub equipo_compartido: bool,
    pub agente: Option<String>,
    pub creada_en: DateTime<Utc>,
    pub revocada: bool,
}

/// Hashes de un par de tokens nuevo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashesTokens {
    pub acceso: String,
    pub acceso_expira_en: DateTime<Utc>,
    pub refresh: Option<String>,
}

/// Sesiones de las cuentas (tokens opacos de acceso y renovación).
pub trait RepositorioSesiones: Send {
    /// Guarda una sesión nueva.
    fn insertar_sesion(&mut self, sesion: &Sesion) -> impl Future<Output = AppResult<()>> + Send;

    /// Sesión por el hash del token de acceso.
    fn sesion_por_acceso(
        &mut self,
        hash: &str,
    ) -> impl Future<Output = AppResult<Option<Sesion>>> + Send;

    /// Sesión por el hash del token de renovación.
    fn sesion_por_refresh(
        &mut self,
        hash: &str,
    ) -> impl Future<Output = AppResult<Option<Sesion>>> + Send;

    /// Sesión cuyo refresh **anterior** es `hash` (un refresh ya usado que vuelve a llegar).
    fn sesion_por_refresh_anterior(
        &mut self,
        hash: &str,
    ) -> impl Future<Output = AppResult<Option<SesionId>>> + Send;

    /// Reemplaza los tokens. `refresh_anterior` queda para detectar su reutilización.
    fn rotar_tokens(
        &mut self,
        id: SesionId,
        nuevos: &HashesTokens,
        refresh_anterior: Option<&str>,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// El segundo factor quedó verificado: la sesión pasa a ser completa con tokens nuevos.
    fn completar_segundo_factor(
        &mut self,
        id: SesionId,
        nuevos: &HashesTokens,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Revoca la sesión con su motivo.
    fn revocar_sesion(
        &mut self,
        id: SesionId,
        motivo: &str,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Revoca todas las sesiones abiertas de la cuenta. Devuelve cuántas.
    fn revocar_sesiones_de(
        &mut self,
        cuenta: CuentaId,
        motivo: &str,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<u64>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------------------------

/// Rol con sus permisos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rol {
    pub id: RolId,
    /// `None` = plantilla del sistema o rol de plataforma.
    pub tenant_id: Option<TenantId>,
    pub codigo: String,
    pub nombre: String,
    pub descripcion: Option<String>,
    pub ambito: Ambito,
    pub es_plantilla: bool,
    pub permisos: ConjuntoPermisos,
}

/// Asignación de un rol a una cuenta con vigencia `[desde, hasta)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asignacion {
    pub id: AsignacionRolId,
    pub cuenta_id: CuentaId,
    pub rol_id: RolId,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub origen: OrigenAsignacion,
    /// Id del hecho que origina una asignación derivada (contrato, titularidad…).
    pub origen_id: Option<Uuid>,
    pub asignado_por: Option<CuentaId>,
}

impl Asignacion {
    /// Si la asignación está vigente en ese instante (empezó y no ha terminado).
    pub fn vigente_en(&self, ahora: DateTime<Utc>) -> bool {
        self.vigente_desde <= ahora && self.vigente_hasta.is_none_or(|h| ahora < h)
    }
}

/// Cuenta encontrada por usuario o correo exactos (solo lo necesario para asignarle un rol).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuentaResumen {
    pub id: CuentaId,
    pub nombre_mostrar: String,
}

/// Roles y asignaciones de **una copropiedad** (transacción del tenant).
pub trait RepositorioRoles: Send {
    /// Plantillas de copropiedad y roles propios de la copropiedad.
    fn roles(&mut self) -> impl Future<Output = AppResult<Vec<Rol>>> + Send;

    /// Un rol visible en la copropiedad (plantilla de copropiedad o propio).
    fn rol(&mut self, id: RolId) -> impl Future<Output = AppResult<Option<Rol>>> + Send;

    /// Rol de la copropiedad o plantilla por su código.
    fn rol_por_codigo(
        &mut self,
        codigo: &str,
    ) -> impl Future<Output = AppResult<Option<Rol>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay un rol con ese código.
    fn insertar_rol(&mut self, rol: &Rol) -> impl Future<Output = AppResult<()>> + Send;

    /// Reemplaza todos los permisos del rol.
    fn reemplazar_permisos(
        &mut self,
        rol: RolId,
        permisos: &ConjuntoPermisos,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda una asignación de rol.
    fn insertar_asignacion(
        &mut self,
        asignacion: &Asignacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Asignación de rol por su id.
    fn asignacion(
        &mut self,
        id: AsignacionRolId,
    ) -> impl Future<Output = AppResult<Option<Asignacion>>> + Send;

    /// Historial de asignaciones (todas o las de una cuenta), más recientes primero.
    fn asignaciones(
        &mut self,
        cuenta: Option<CuentaId>,
    ) -> impl Future<Output = AppResult<Vec<Asignacion>>> + Send;

    /// Fija `vigente_hasta = ahora` si seguía vigente.
    fn terminar_asignacion(
        &mut self,
        id: AsignacionRolId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Termina las asignaciones vigentes derivadas de un hecho. Devuelve cuántas.
    fn terminar_por_origen(
        &mut self,
        origen: OrigenAsignacion,
        origen_id: Uuid,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<u64>> + Send;

    /// Unión de los permisos de los roles vigentes de la cuenta en la copropiedad.
    fn permisos_de_cuenta(
        &mut self,
        cuenta: CuentaId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<ConjuntoPermisos>> + Send;

    /// Asignaciones vigentes del rol plantilla administrador.
    fn administradores_vigentes(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<u64>> + Send;

    /// Cuenta por usuario o correo exactos, con lo mínimo para asignarle un rol.
    fn buscar_cuenta(
        &mut self,
        identificador: &Identificador,
    ) -> impl Future<Output = AppResult<Option<CuentaResumen>>> + Send;

    /// Crea la fila de la copropiedad de la transacción (alta desde la plataforma).
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el NIT ya lo usa otra copropiedad.
    fn insertar_copropiedad(
        &mut self,
        alta: &crate::puertos::copropiedades::AltaCopropiedad,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

/// Copropiedad donde la cuenta tiene roles vigentes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membresia {
    pub tenant_id: TenantId,
    pub nombre: String,
    pub roles: Vec<String>,
}

/// Asignación de un rol de plataforma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsignacionPlataforma {
    pub id: AsignacionRolId,
    pub cuenta_id: CuentaId,
    pub rol_id: RolId,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub asignado_por: Option<CuentaId>,
}

/// Copropiedad vista desde la plataforma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopropiedadResumen {
    pub id: TenantId,
    pub nombre: String,
}

/// Roles de plataforma y vistas que cruzan copropiedades (transacción de plataforma).
pub trait RepositorioPlataforma: Send {
    /// Permisos de plataforma vigentes de la cuenta y si es superusuario.
    fn permisos_plataforma(
        &mut self,
        cuenta: CuentaId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<(ConjuntoPermisos, bool)>> + Send;

    /// Copropiedades donde la cuenta tiene roles vigentes.
    fn membresias(
        &mut self,
        cuenta: CuentaId,
    ) -> impl Future<Output = AppResult<Vec<Membresia>>> + Send;

    /// Roles de plataforma (plantillas y propios).
    fn roles_plataforma(&mut self) -> impl Future<Output = AppResult<Vec<Rol>>> + Send;

    /// Rol de plataforma por su id.
    fn rol_plataforma(&mut self, id: RolId) -> impl Future<Output = AppResult<Option<Rol>>> + Send;

    /// Rol de plataforma por su código.
    fn rol_plataforma_por_codigo(
        &mut self,
        codigo: &str,
    ) -> impl Future<Output = AppResult<Option<Rol>>> + Send;

    /// Crea un rol de plataforma.
    fn insertar_rol_plataforma(&mut self, rol: &Rol) -> impl Future<Output = AppResult<()>> + Send;

    /// Reemplaza todos los permisos de un rol de plataforma.
    fn reemplazar_permisos_plataforma(
        &mut self,
        rol: RolId,
        permisos: &ConjuntoPermisos,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda una asignación de rol de plataforma.
    fn insertar_asignacion_plataforma(
        &mut self,
        asignacion: &AsignacionPlataforma,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Asignación de rol de plataforma por su id.
    fn asignacion_plataforma(
        &mut self,
        id: AsignacionRolId,
    ) -> impl Future<Output = AppResult<Option<AsignacionPlataforma>>> + Send;

    /// Termina una asignación de rol de plataforma.
    fn terminar_asignacion_plataforma(
        &mut self,
        id: AsignacionRolId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Superusuarios vigentes (nunca se puede quedar en cero).
    fn superusuarios_vigentes(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<u64>> + Send;

    /// Copropiedades registradas en la plataforma.
    fn copropiedades(&mut self) -> impl Future<Output = AppResult<Vec<CopropiedadResumen>>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Organizaciones
// ---------------------------------------------------------------------------------------------

/// Ficha pública de una organización (lo que ve cualquier copropiedad que busca su NIT).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FichaOrganizacion {
    pub id: OrganizacionId,
    pub razon_social: String,
    pub naturaleza: NaturalezaOrganizacion,
    pub tipos: Vec<TipoOrganizacion>,
    pub estado: EstadoOrganizacion,
}

/// Organización completa (solo en el ámbito de plataforma).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Organizacion {
    pub ficha: FichaOrganizacion,
    pub nit: Nit,
    pub correo_contacto: Option<String>,
    pub telefono_contacto: Option<String>,
    pub creada_por_cuenta: Option<CuentaId>,
}

/// Datos para crear una organización.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevaOrganizacion {
    pub nit: Nit,
    pub razon_social: String,
    pub naturaleza: NaturalezaOrganizacion,
    pub tipos: Vec<TipoOrganizacion>,
    pub correo_contacto: Option<String>,
    pub telefono_contacto: Option<String>,
}

/// Cambios a la ficha global (campos `None` no cambian).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CambiosFicha {
    pub razon_social: Option<String>,
    pub tipos: Option<Vec<TipoOrganizacion>>,
    pub correo_contacto: Option<String>,
    pub telefono_contacto: Option<String>,
}

/// Contrato de servicio de una organización con la copropiedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContratoServicio {
    pub id: ContratoServicioId,
    pub organizacion_id: OrganizacionId,
    pub tipo_servicio: TipoOrganizacion,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub estado: EstadoContrato,
    pub valor: Option<rust_decimal::Decimal>,
}

/// Solicitud para reclamar la ficha de una organización.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reclamacion {
    pub id: ReclamacionId,
    pub organizacion_id: OrganizacionId,
    pub cuenta_id: CuentaId,
    pub mensaje: String,
    pub estado: EstadoReclamacion,
    pub creada_en: DateTime<Utc>,
}

/// Organizaciones vistas desde una copropiedad (búsqueda y alta por funciones de la BD) y sus
/// contratos (transacción del tenant).
pub trait RepositorioContratos: Send {
    /// Organización por su NIT (para no duplicarla al contratarla).
    fn buscar_organizacion_por_nit(
        &mut self,
        nit: &Nit,
    ) -> impl Future<Output = AppResult<Option<FichaOrganizacion>>> + Send;

    /// Crea la organización o devuelve la existente con el mismo NIT. `bool` = la creó.
    fn crear_organizacion(
        &mut self,
        id: OrganizacionId,
        nueva: &NuevaOrganizacion,
        creada_por: Option<CuentaId>,
    ) -> impl Future<Output = AppResult<(OrganizacionId, bool)>> + Send;

    /// Guarda un contrato de servicio con una organización.
    fn insertar_contrato(
        &mut self,
        contrato: &ContratoServicio,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Contrato de servicio por su id.
    fn contrato(
        &mut self,
        id: ContratoServicioId,
    ) -> impl Future<Output = AppResult<Option<ContratoServicio>>> + Send;

    /// Contratos de servicio de la copropiedad.
    fn contratos(&mut self) -> impl Future<Output = AppResult<Vec<ContratoServicio>>> + Send;

    /// Termina un contrato de servicio.
    fn terminar_contrato(
        &mut self,
        id: ContratoServicioId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

/// Organizaciones en el ámbito de plataforma: ficha, miembros y reclamaciones.
pub trait RepositorioOrganizaciones: Send {
    /// Organización por su id.
    fn organizacion(
        &mut self,
        id: OrganizacionId,
    ) -> impl Future<Output = AppResult<Option<Organizacion>>> + Send;

    /// Papel de la cuenta en la organización, si es miembro.
    fn rol_miembro(
        &mut self,
        organizacion: OrganizacionId,
        cuenta: CuentaId,
    ) -> impl Future<Output = AppResult<Option<RolOrganizacion>>> + Send;

    /// Agrega o actualiza el papel de la cuenta en la organización.
    fn guardar_miembro(
        &mut self,
        organizacion: OrganizacionId,
        cuenta: CuentaId,
        rol: RolOrganizacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Actualiza los datos de la ficha de la organización.
    fn actualizar_ficha(
        &mut self,
        id: OrganizacionId,
        cambios: &CambiosFicha,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Cambia el estado de la organización.
    fn cambiar_estado_organizacion(
        &mut self,
        id: OrganizacionId,
        estado: EstadoOrganizacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si la cuenta ya tiene una reclamación pendiente.
    fn insertar_reclamacion(
        &mut self,
        reclamacion: &Reclamacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Reclamación de una organización por su id.
    fn reclamacion(
        &mut self,
        id: ReclamacionId,
    ) -> impl Future<Output = AppResult<Option<Reclamacion>>> + Send;

    /// Reclamaciones pendientes de revisión.
    fn reclamaciones_pendientes(
        &mut self,
    ) -> impl Future<Output = AppResult<Vec<Reclamacion>>> + Send;

    /// Aprueba o rechaza una reclamación, con quién la revisó y el motivo.
    fn resolver_reclamacion(
        &mut self,
        id: ReclamacionId,
        estado: EstadoReclamacion,
        revisor: Option<CuentaId>,
        motivo: Option<&str>,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Criptografía
// ---------------------------------------------------------------------------------------------

/// Hash de contraseñas (Argon2id). Es costoso a propósito: el adaptador lo ejecuta fuera del
/// hilo de Tokio.
pub trait HashContrasenas: Send + Sync {
    /// Calcula el hash Argon2id de una contraseña nueva.
    fn hash(&self, contrasena: &ContrasenaNueva) -> impl Future<Output = AppResult<String>> + Send;

    /// ¿La contraseña corresponde al hash? `false` también si el hash no es válido.
    fn verificar(&self, contrasena: &str, hash: &str) -> impl Future<Output = bool> + Send;

    /// Hace el mismo trabajo que [`HashContrasenas::verificar`] sin cuenta real, para que una
    /// cuenta inexistente tarde lo mismo que una contraseña equivocada.
    fn igualar_tiempo(&self, contrasena: &str) -> impl Future<Output = ()> + Send;
}

/// Segundo factor TOTP (RFC 6238, apps autenticadoras).
pub trait SegundoFactor: Send + Sync {
    /// Secreto nuevo en Base32.
    fn generar_secreto(&self) -> String;

    /// URI `otpauth://` para el código QR.
    fn uri(&self, secreto: &str, cuenta: &str) -> String;

    /// ¿El código vale en `ahora` (con ±1 paso de tolerancia)?
    fn verificar(&self, secreto: &str, codigo: &str, ahora: DateTime<Utc>) -> bool;
}
