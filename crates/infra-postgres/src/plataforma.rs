//! Transacciones de **plataforma** (`app.plataforma = 'on'`): cuentas, sesiones, roles de
//! plataforma y organizaciones. Una transacción de copropiedad no ve estas tablas.

use application::{
    AppError, AppResult, ContextoPlataforma,
    auditoria::EventoAuditoria,
    outbox::EventoSaliente,
    puertos::{
        RegistroAuditoria, RegistroOutbox, RepositorioCuentas, RepositorioOrganizaciones,
        RepositorioPlataforma, RepositorioSesiones, Transaccion, UnidadDeTrabajoPlataforma,
        identidad::{
            AsignacionPlataforma, CambiosFicha, CodigoVerificacion, CopropiedadResumen, Cuenta,
            FichaOrganizacion, HashesTokens, Membresia, Organizacion, PropositoCodigo, Reclamacion,
            Rol, Sesion,
        },
    },
};
use chrono::{DateTime, Utc};
use domain::{
    Ambito, AsignacionRolId, ConjuntoPermisos, CuentaId, DireccionCorreo, EstadoCuenta,
    EstadoOrganizacion, EstadoReclamacion, Identificador, NaturalezaOrganizacion, Nit,
    NombreUsuario, OrganizacionId, Permiso, ReclamacionId, RolId, RolOrganizacion, SesionId,
    TenantId, TipoOrganizacion, permisos::CODIGO_SUPERUSUARIO,
};
use sqlx::{PgPool, Postgres};
use uuid::Uuid;

/// Unidad de trabajo de plataforma con el pool de la aplicación (rol `dad_app`).
#[derive(Debug, Clone)]
pub struct UnidadDeTrabajoPlataformaPg {
    pool: PgPool,
}

impl UnidadDeTrabajoPlataformaPg {
    /// Unidad de trabajo de plataforma sobre el pool de la app (`dad_app`).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Transacción de plataforma. Soltarla sin confirmar hace ROLLBACK.
#[derive(Debug)]
pub struct TransaccionPlataformaPg {
    pub(crate) tx: sqlx::Transaction<'static, Postgres>,
    ctx: ContextoPlataforma,
}

impl UnidadDeTrabajoPlataforma for UnidadDeTrabajoPlataformaPg {
    type Transaccion = TransaccionPlataformaPg;

    async fn iniciar_plataforma(&self, ctx: &ContextoPlataforma) -> AppResult<Self::Transaccion> {
        let mut tx = self.pool.begin().await.map_err(AppError::infra)?;
        sqlx::query("SELECT set_config('app.plataforma', 'on', true)")
            .execute(&mut *tx)
            .await
            .map_err(AppError::infra)?;
        Ok(TransaccionPlataformaPg {
            tx,
            ctx: ctx.clone(),
        })
    }
}

impl Transaccion for TransaccionPlataformaPg {
    async fn confirmar(self) -> AppResult<()> {
        self.tx.commit().await.map_err(AppError::infra)
    }
}

/// Convierte una violación de unicidad en [`AppError::Conflicto`].
pub(crate) fn conflicto_si_unico(e: sqlx::Error, mensaje: &str) -> AppError {
    if e.as_database_error()
        .is_some_and(|d| d.is_unique_violation())
    {
        AppError::Conflicto(mensaje.to_owned())
    } else {
        AppError::infra(e)
    }
}

fn a_u32(v: i32) -> u32 {
    u32::try_from(v).unwrap_or(0)
}

impl RegistroAuditoria for TransaccionPlataformaPg {
    async fn registrar(&mut self, evento: EventoAuditoria) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO auditoria_plataforma
                 (actor_id, accion, entidad, entidad_id, resultado, detalle, request_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            self.ctx.actor_id.map(CuentaId::as_uuid),
            evento.accion.as_str(),
            evento.entidad,
            evento.entidad_id,
            evento.resultado.as_str(),
            evento.detalle.a_json(),
            self.ctx.request_id.as_deref(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

impl RegistroOutbox for TransaccionPlataformaPg {
    async fn encolar(&mut self, evento: EventoSaliente) -> AppResult<()> {
        let payload = serde_json::to_value(&evento.mensaje).map_err(AppError::infra)?;
        sqlx::query!(
            "INSERT INTO outbox (tenant_id, canal, prioridad, linea, destino, payload)
             VALUES (NULL, $1, $2, $3, $4, $5)",
            evento.mensaje.canal().as_str(),
            evento.prioridad.as_i16(),
            evento.mensaje.linea(),
            evento.mensaje.destino(),
            payload,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Cuentas
// ---------------------------------------------------------------------------------------------

struct FilaCuenta {
    id: Uuid,
    nombre_mostrar: String,
    usuario: Option<String>,
    correo: Option<String>,
    correo_verificado_en: Option<DateTime<Utc>>,
    estado: String,
    hash_contrasena: String,
    intentos_fallidos: i32,
    bloqueo_hasta: Option<DateTime<Utc>>,
    totp_secreto: Option<String>,
    totp_activado_en: Option<DateTime<Utc>>,
    creada_en: DateTime<Utc>,
}

impl TryFrom<FilaCuenta> for Cuenta {
    type Error = AppError;

    fn try_from(f: FilaCuenta) -> AppResult<Self> {
        Ok(Self {
            id: CuentaId::from_uuid(f.id),
            nombre_mostrar: f.nombre_mostrar,
            usuario: f.usuario.as_deref().map(NombreUsuario::new).transpose()?,
            correo: f.correo.as_deref().map(DireccionCorreo::new).transpose()?,
            correo_verificado: f.correo_verificado_en.is_some(),
            estado: EstadoCuenta::try_from(f.estado.as_str())?,
            hash_contrasena: f.hash_contrasena,
            intentos_fallidos: a_u32(f.intentos_fallidos),
            bloqueo_hasta: f.bloqueo_hasta,
            totp_secreto: f.totp_secreto,
            totp_activo: f.totp_activado_en.is_some(),
            creada_en: f.creada_en,
        })
    }
}

impl RepositorioCuentas for TransaccionPlataformaPg {
    async fn insertar_cuenta(&mut self, c: &Cuenta) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO cuentas (id, nombre_mostrar, usuario, correo, estado, hash_contrasena,
                                  contrasena_actualizada_en, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7)",
            c.id.as_uuid(),
            c.nombre_mostrar,
            c.usuario.as_ref().map(NombreUsuario::as_str),
            c.correo.as_ref().map(DireccionCorreo::as_str),
            c.estado.as_str(),
            c.hash_contrasena,
            c.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "el usuario o el correo ya están registrados"))?;
        Ok(())
    }

    async fn cuenta_por_identificador(&mut self, id: &Identificador) -> AppResult<Option<Cuenta>> {
        let (usuario, correo) = match id {
            Identificador::Usuario(u) => (Some(u.as_str()), None),
            Identificador::Correo(c) => (None, Some(c.as_str())),
        };
        sqlx::query_as!(
            FilaCuenta,
            "SELECT id, nombre_mostrar, usuario, correo, correo_verificado_en, estado,
                    hash_contrasena, intentos_fallidos, bloqueo_hasta, totp_secreto,
                    totp_activado_en, creada_en
             FROM cuentas WHERE usuario = $1 OR correo = $2",
            usuario,
            correo,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Cuenta::try_from)
        .transpose()
    }

    async fn cuenta_por_id(&mut self, id: CuentaId) -> AppResult<Option<Cuenta>> {
        sqlx::query_as!(
            FilaCuenta,
            "SELECT id, nombre_mostrar, usuario, correo, correo_verificado_en, estado,
                    hash_contrasena, intentos_fallidos, bloqueo_hasta, totp_secreto,
                    totp_activado_en, creada_en
             FROM cuentas WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Cuenta::try_from)
        .transpose()
    }

    async fn registrar_fallo(
        &mut self,
        id: CuentaId,
        intentos: u32,
        bloqueo_hasta: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE cuentas SET intentos_fallidos = $2, bloqueo_hasta = $3 WHERE id = $1",
            id.as_uuid(),
            i32::try_from(intentos).unwrap_or(i32::MAX),
            bloqueo_hasta,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn registrar_acceso(&mut self, id: CuentaId, ahora: DateTime<Utc>) -> AppResult<()> {
        sqlx::query!(
            "UPDATE cuentas SET intentos_fallidos = 0, bloqueo_hasta = NULL, ultimo_acceso_en = $2
             WHERE id = $1",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn cambiar_contrasena(
        &mut self,
        id: CuentaId,
        hash: &str,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE cuentas SET hash_contrasena = $2, contrasena_actualizada_en = $3,
                                intentos_fallidos = 0, bloqueo_hasta = NULL
             WHERE id = $1",
            id.as_uuid(),
            hash,
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn marcar_correo_verificado(
        &mut self,
        id: CuentaId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE cuentas SET correo_verificado_en = coalesce(correo_verificado_en, $2)
             WHERE id = $1",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn guardar_totp(
        &mut self,
        id: CuentaId,
        secreto: Option<&str>,
        activado_en: Option<DateTime<Utc>>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE cuentas SET totp_secreto = $2, totp_activado_en = $3 WHERE id = $1",
            id.as_uuid(),
            secreto,
            activado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn cambiar_estado(&mut self, id: CuentaId, estado: EstadoCuenta) -> AppResult<()> {
        sqlx::query!(
            "UPDATE cuentas SET estado = $2 WHERE id = $1",
            id.as_uuid(),
            estado.as_str(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn insertar_codigo(&mut self, c: &CodigoVerificacion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO codigos_verificacion
                 (id, cuenta_id, proposito, hash_codigo, expira_en, intentos, creado_en)
             VALUES ($1, $2, $3, $4, $5, 0, $6)",
            c.id,
            c.cuenta_id.as_uuid(),
            c.proposito.as_str(),
            c.hash_codigo,
            c.expira_en,
            c.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn codigo_vigente(
        &mut self,
        cuenta: CuentaId,
        proposito: PropositoCodigo,
        ahora: DateTime<Utc>,
    ) -> AppResult<Option<CodigoVerificacion>> {
        let fila = sqlx::query!(
            "SELECT id, hash_codigo, expira_en, intentos, creado_en
             FROM codigos_verificacion
             WHERE cuenta_id = $1 AND proposito = $2 AND usado_en IS NULL AND expira_en > $3
             ORDER BY creado_en DESC
             LIMIT 1",
            cuenta.as_uuid(),
            proposito.as_str(),
            ahora,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(fila.map(|f| CodigoVerificacion {
            id: f.id,
            cuenta_id: cuenta,
            proposito,
            hash_codigo: f.hash_codigo,
            expira_en: f.expira_en,
            intentos: a_u32(f.intentos),
            creado_en: f.creado_en,
        }))
    }

    async fn registrar_intento_codigo(&mut self, id: Uuid) -> AppResult<()> {
        sqlx::query!(
            "UPDATE codigos_verificacion SET intentos = intentos + 1 WHERE id = $1",
            id
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn marcar_codigo_usado(&mut self, id: Uuid, ahora: DateTime<Utc>) -> AppResult<()> {
        sqlx::query!(
            "UPDATE codigos_verificacion SET usado_en = $2 WHERE id = $1",
            id,
            ahora
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Sesiones
// ---------------------------------------------------------------------------------------------

struct FilaSesion {
    id: Uuid,
    cuenta_id: Uuid,
    hash_acceso: String,
    acceso_expira_en: DateTime<Utc>,
    hash_refresh: Option<String>,
    expira_maximo_en: DateTime<Utc>,
    segundo_factor_pendiente: bool,
    segundo_factor_verificado: bool,
    equipo_compartido: bool,
    agente: Option<String>,
    creada_en: DateTime<Utc>,
    revocada_en: Option<DateTime<Utc>>,
}

impl From<FilaSesion> for Sesion {
    fn from(f: FilaSesion) -> Self {
        Self {
            id: SesionId::from_uuid(f.id),
            cuenta_id: CuentaId::from_uuid(f.cuenta_id),
            hash_acceso: f.hash_acceso,
            acceso_expira_en: f.acceso_expira_en,
            hash_refresh: f.hash_refresh,
            expira_maximo_en: f.expira_maximo_en,
            segundo_factor_pendiente: f.segundo_factor_pendiente,
            segundo_factor_verificado: f.segundo_factor_verificado,
            equipo_compartido: f.equipo_compartido,
            agente: f.agente,
            creada_en: f.creada_en,
            revocada: f.revocada_en.is_some(),
        }
    }
}

impl RepositorioSesiones for TransaccionPlataformaPg {
    async fn insertar_sesion(&mut self, s: &Sesion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO sesiones (id, cuenta_id, hash_acceso, acceso_expira_en, hash_refresh,
                                   expira_maximo_en, segundo_factor_pendiente,
                                   segundo_factor_verificado, equipo_compartido, agente, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            s.id.as_uuid(),
            s.cuenta_id.as_uuid(),
            s.hash_acceso,
            s.acceso_expira_en,
            s.hash_refresh,
            s.expira_maximo_en,
            s.segundo_factor_pendiente,
            s.segundo_factor_verificado,
            s.equipo_compartido,
            s.agente,
            s.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn sesion_por_acceso(&mut self, hash: &str) -> AppResult<Option<Sesion>> {
        Ok(sqlx::query_as!(
            FilaSesion,
            "SELECT id, cuenta_id, hash_acceso, acceso_expira_en, hash_refresh, expira_maximo_en,
                    segundo_factor_pendiente, segundo_factor_verificado, equipo_compartido,
                    agente, creada_en, revocada_en
             FROM sesiones WHERE hash_acceso = $1",
            hash,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Sesion::from))
    }

    async fn sesion_por_refresh(&mut self, hash: &str) -> AppResult<Option<Sesion>> {
        Ok(sqlx::query_as!(
            FilaSesion,
            "SELECT id, cuenta_id, hash_acceso, acceso_expira_en, hash_refresh, expira_maximo_en,
                    segundo_factor_pendiente, segundo_factor_verificado, equipo_compartido,
                    agente, creada_en, revocada_en
             FROM sesiones WHERE hash_refresh = $1
             FOR UPDATE",
            hash,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Sesion::from))
    }

    async fn sesion_por_refresh_anterior(&mut self, hash: &str) -> AppResult<Option<SesionId>> {
        Ok(sqlx::query_scalar!(
            "SELECT id FROM sesiones WHERE hash_refresh_anterior = $1",
            hash
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(SesionId::from_uuid))
    }

    async fn rotar_tokens(
        &mut self,
        id: SesionId,
        nuevos: &HashesTokens,
        refresh_anterior: Option<&str>,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE sesiones SET hash_acceso = $2, acceso_expira_en = $3, hash_refresh = $4,
                                 hash_refresh_anterior = $5, renovada_en = $6
             WHERE id = $1",
            id.as_uuid(),
            nuevos.acceso,
            nuevos.acceso_expira_en,
            nuevos.refresh,
            refresh_anterior,
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn completar_segundo_factor(
        &mut self,
        id: SesionId,
        nuevos: &HashesTokens,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE sesiones SET hash_acceso = $2, acceso_expira_en = $3, hash_refresh = $4,
                                 segundo_factor_pendiente = FALSE, segundo_factor_verificado = TRUE
             WHERE id = $1",
            id.as_uuid(),
            nuevos.acceso,
            nuevos.acceso_expira_en,
            nuevos.refresh,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn revocar_sesion(
        &mut self,
        id: SesionId,
        motivo: &str,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE sesiones SET revocada_en = $2, motivo_revocacion = $3
             WHERE id = $1 AND revocada_en IS NULL",
            id.as_uuid(),
            ahora,
            motivo,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn revocar_sesiones_de(
        &mut self,
        cuenta: CuentaId,
        motivo: &str,
        ahora: DateTime<Utc>,
    ) -> AppResult<u64> {
        Ok(sqlx::query!(
            "UPDATE sesiones SET revocada_en = $2, motivo_revocacion = $3
             WHERE cuenta_id = $1 AND revocada_en IS NULL",
            cuenta.as_uuid(),
            ahora,
            motivo,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected())
    }
}

// ---------------------------------------------------------------------------------------------
// Roles de plataforma y vistas globales
// ---------------------------------------------------------------------------------------------

/// Permisos (texto) → conjunto; ignora textos desconocidos (catálogo desincronizado).
pub(crate) fn conjunto(permisos: &[String]) -> ConjuntoPermisos {
    permisos
        .iter()
        .filter_map(|p| Permiso::try_from(p.as_str()).ok())
        .collect()
}

pub(crate) struct FilaRol {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub codigo: String,
    pub nombre: String,
    pub descripcion: Option<String>,
    pub ambito: String,
    pub es_plantilla: bool,
    pub permisos: Vec<String>,
}

impl TryFrom<FilaRol> for Rol {
    type Error = AppError;

    fn try_from(f: FilaRol) -> AppResult<Self> {
        Ok(Self {
            id: RolId::from_uuid(f.id),
            tenant_id: f.tenant_id.map(TenantId::from_uuid),
            codigo: f.codigo,
            nombre: f.nombre,
            descripcion: f.descripcion,
            ambito: Ambito::try_from(f.ambito.as_str())?,
            es_plantilla: f.es_plantilla,
            permisos: conjunto(&f.permisos),
        })
    }
}

impl TransaccionPlataformaPg {
    async fn roles_plataforma_donde(
        &mut self,
        id: Option<Uuid>,
        codigo: Option<&str>,
    ) -> AppResult<Vec<Rol>> {
        sqlx::query_as!(
            FilaRol,
            r#"SELECT r.id, r.tenant_id, r.codigo, r.nombre, r.descripcion, r.ambito, r.es_plantilla,
                      coalesce(array_agg(rp.permiso) FILTER (WHERE rp.permiso IS NOT NULL), '{}')
                          AS "permisos!"
               FROM roles r LEFT JOIN roles_permisos rp ON rp.rol_id = r.id
               WHERE r.ambito = 'plataforma'
                 AND ($1::uuid IS NULL OR r.id = $1)
                 AND ($2::text IS NULL OR r.codigo = $2)
               GROUP BY r.id
               ORDER BY r.es_plantilla DESC, r.codigo"#,
            id,
            codigo,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Rol::try_from)
        .collect()
    }

    async fn insertar_permisos(
        &mut self,
        rol: RolId,
        permisos: &ConjuntoPermisos,
    ) -> AppResult<()> {
        let textos: Vec<String> = permisos.iter().map(|p| p.as_str().to_owned()).collect();
        sqlx::query!(
            "INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
             SELECT $1, 'plataforma', p, NULL FROM unnest($2::text[]) AS p",
            rol.as_uuid(),
            &textos,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

impl RepositorioPlataforma for TransaccionPlataformaPg {
    async fn permisos_plataforma(
        &mut self,
        cuenta: CuentaId,
        ahora: DateTime<Utc>,
    ) -> AppResult<(ConjuntoPermisos, bool)> {
        let fila = sqlx::query!(
            r#"SELECT coalesce(array_agg(DISTINCT rp.permiso) FILTER (WHERE rp.permiso IS NOT NULL), '{}')
                          AS "permisos!",
                      coalesce(bool_or(r.es_plantilla AND r.codigo = $3), FALSE) AS "superusuario!"
               FROM asignaciones_plataforma a
               JOIN roles r ON r.id = a.rol_id
               LEFT JOIN roles_permisos rp ON rp.rol_id = r.id
               WHERE a.cuenta_id = $1 AND a.vigente_desde <= $2
                 AND (a.vigente_hasta IS NULL OR a.vigente_hasta > $2)"#,
            cuenta.as_uuid(),
            ahora,
            CODIGO_SUPERUSUARIO,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        let permisos = if fila.superusuario {
            ConjuntoPermisos::todos(Ambito::Plataforma)
        } else {
            conjunto(&fila.permisos)
        };
        Ok((permisos, fila.superusuario))
    }

    async fn membresias(&mut self, cuenta: CuentaId) -> AppResult<Vec<Membresia>> {
        Ok(sqlx::query!(
            r#"SELECT tenant_id AS "tenant_id!", nombre AS "nombre!", roles AS "roles!"
               FROM membresias_de_cuenta($1)"#,
            cuenta.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| Membresia {
            tenant_id: TenantId::from_uuid(f.tenant_id),
            nombre: f.nombre,
            roles: f.roles,
        })
        .collect())
    }

    async fn roles_plataforma(&mut self) -> AppResult<Vec<Rol>> {
        self.roles_plataforma_donde(None, None).await
    }

    async fn rol_plataforma(&mut self, id: RolId) -> AppResult<Option<Rol>> {
        Ok(self
            .roles_plataforma_donde(Some(id.as_uuid()), None)
            .await?
            .pop())
    }

    async fn rol_plataforma_por_codigo(&mut self, codigo: &str) -> AppResult<Option<Rol>> {
        Ok(self.roles_plataforma_donde(None, Some(codigo)).await?.pop())
    }

    async fn insertar_rol_plataforma(&mut self, rol: &Rol) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO roles (id, tenant_id, codigo, nombre, descripcion, ambito)
             VALUES ($1, NULL, $2, $3, $4, 'plataforma')",
            rol.id.as_uuid(),
            rol.codigo,
            rol.nombre,
            rol.descripcion,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "ya existe un rol con ese código"))?;
        self.insertar_permisos(rol.id, &rol.permisos).await
    }

    async fn reemplazar_permisos_plataforma(
        &mut self,
        rol: RolId,
        permisos: &ConjuntoPermisos,
    ) -> AppResult<()> {
        sqlx::query!(
            "DELETE FROM roles_permisos WHERE rol_id = $1",
            rol.as_uuid()
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        self.insertar_permisos(rol, permisos).await
    }

    async fn insertar_asignacion_plataforma(&mut self, a: &AsignacionPlataforma) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO asignaciones_plataforma
                 (id, cuenta_id, rol_id, vigente_desde, vigente_hasta, asignado_por)
             VALUES ($1, $2, $3, $4, $5, $6)",
            a.id.as_uuid(),
            a.cuenta_id.as_uuid(),
            a.rol_id.as_uuid(),
            a.vigente_desde,
            a.vigente_hasta,
            a.asignado_por.map(CuentaId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn asignacion_plataforma(
        &mut self,
        id: AsignacionRolId,
    ) -> AppResult<Option<AsignacionPlataforma>> {
        Ok(sqlx::query!(
            "SELECT id, cuenta_id, rol_id, vigente_desde, vigente_hasta, asignado_por
             FROM asignaciones_plataforma WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| AsignacionPlataforma {
            id: AsignacionRolId::from_uuid(f.id),
            cuenta_id: CuentaId::from_uuid(f.cuenta_id),
            rol_id: RolId::from_uuid(f.rol_id),
            vigente_desde: f.vigente_desde,
            vigente_hasta: f.vigente_hasta,
            asignado_por: f.asignado_por.map(CuentaId::from_uuid),
        }))
    }

    async fn terminar_asignacion_plataforma(
        &mut self,
        id: AsignacionRolId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE asignaciones_plataforma SET vigente_hasta = greatest($2, vigente_desde)
             WHERE id = $1 AND (vigente_hasta IS NULL OR vigente_hasta > $2)",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn superusuarios_vigentes(&mut self, ahora: DateTime<Utc>) -> AppResult<u64> {
        let n = sqlx::query_scalar!(
            r#"SELECT count(DISTINCT a.cuenta_id) AS "n!"
               FROM asignaciones_plataforma a JOIN roles r ON r.id = a.rol_id
               WHERE r.es_plantilla AND r.codigo = $1 AND a.vigente_desde <= $2
                 AND (a.vigente_hasta IS NULL OR a.vigente_hasta > $2)"#,
            CODIGO_SUPERUSUARIO,
            ahora,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    async fn copropiedades(&mut self) -> AppResult<Vec<CopropiedadResumen>> {
        Ok(sqlx::query!(
            "SELECT e.id, e.nombre FROM entidades e
                 JOIN copropiedades c ON c.id = e.id ORDER BY e.nombre"
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| CopropiedadResumen {
            id: TenantId::from_uuid(f.id),
            nombre: f.nombre,
        })
        .collect())
    }
}

// ---------------------------------------------------------------------------------------------
// Organizaciones (plataforma)
// ---------------------------------------------------------------------------------------------

pub(crate) fn tipos(textos: &[String]) -> AppResult<Vec<TipoOrganizacion>> {
    textos
        .iter()
        .map(|t| TipoOrganizacion::try_from(t.as_str()).map_err(AppError::from))
        .collect()
}

fn textos_tipos(tipos: &[TipoOrganizacion]) -> Vec<String> {
    tipos.iter().map(|t| t.as_str().to_owned()).collect()
}

impl RepositorioOrganizaciones for TransaccionPlataformaPg {
    async fn organizacion(&mut self, id: OrganizacionId) -> AppResult<Option<Organizacion>> {
        let Some(f) = sqlx::query!(
            "SELECT id, nit, razon_social, naturaleza, tipos, estado, correo_contacto,
                    telefono_contacto, creada_por_cuenta
             FROM organizaciones WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        else {
            return Ok(None);
        };
        Ok(Some(Organizacion {
            ficha: FichaOrganizacion {
                id: OrganizacionId::from_uuid(f.id),
                razon_social: f.razon_social,
                naturaleza: NaturalezaOrganizacion::try_from(f.naturaleza.as_str())?,
                tipos: tipos(&f.tipos)?,
                estado: EstadoOrganizacion::try_from(f.estado.as_str())?,
            },
            nit: Nit::new(&f.nit)?,
            correo_contacto: f.correo_contacto,
            telefono_contacto: f.telefono_contacto,
            creada_por_cuenta: f.creada_por_cuenta.map(CuentaId::from_uuid),
        }))
    }

    async fn rol_miembro(
        &mut self,
        org: OrganizacionId,
        cuenta: CuentaId,
    ) -> AppResult<Option<RolOrganizacion>> {
        sqlx::query_scalar!(
            "SELECT rol FROM organizaciones_miembros WHERE organizacion_id = $1 AND cuenta_id = $2",
            org.as_uuid(),
            cuenta.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|r| RolOrganizacion::try_from(r.as_str()).map_err(AppError::from))
        .transpose()
    }

    async fn guardar_miembro(
        &mut self,
        org: OrganizacionId,
        cuenta: CuentaId,
        rol: RolOrganizacion,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO organizaciones_miembros (organizacion_id, cuenta_id, rol)
             VALUES ($1, $2, $3)
             ON CONFLICT (organizacion_id, cuenta_id) DO UPDATE SET rol = EXCLUDED.rol",
            org.as_uuid(),
            cuenta.as_uuid(),
            rol.as_str(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn actualizar_ficha(
        &mut self,
        id: OrganizacionId,
        c: &CambiosFicha,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        let tipos = c.tipos.as_deref().map(textos_tipos);
        sqlx::query!(
            "UPDATE organizaciones SET
                 razon_social = coalesce($2, razon_social),
                 tipos = coalesce($3, tipos),
                 correo_contacto = coalesce($4, correo_contacto),
                 telefono_contacto = coalesce($5, telefono_contacto),
                 actualizada_en = $6
             WHERE id = $1",
            id.as_uuid(),
            c.razon_social.as_deref(),
            tipos.as_deref(),
            c.correo_contacto.as_deref(),
            c.telefono_contacto.as_deref(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn cambiar_estado_organizacion(
        &mut self,
        id: OrganizacionId,
        estado: EstadoOrganizacion,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE organizaciones SET estado = $2 WHERE id = $1",
            id.as_uuid(),
            estado.as_str(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn insertar_reclamacion(&mut self, r: &Reclamacion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO reclamaciones_organizacion (id, organizacion_id, cuenta_id, mensaje, estado, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6)",
            r.id.as_uuid(),
            r.organizacion_id.as_uuid(),
            r.cuenta_id.as_uuid(),
            r.mensaje,
            r.estado.as_str(),
            r.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "ya tiene una reclamación pendiente"))?;
        Ok(())
    }

    async fn reclamacion(&mut self, id: ReclamacionId) -> AppResult<Option<Reclamacion>> {
        sqlx::query!(
            "SELECT id, organizacion_id, cuenta_id, mensaje, estado, creada_en
             FROM reclamaciones_organizacion WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| {
            Ok(Reclamacion {
                id: ReclamacionId::from_uuid(f.id),
                organizacion_id: OrganizacionId::from_uuid(f.organizacion_id),
                cuenta_id: CuentaId::from_uuid(f.cuenta_id),
                mensaje: f.mensaje,
                estado: EstadoReclamacion::try_from(f.estado.as_str())?,
                creada_en: f.creada_en,
            })
        })
        .transpose()
    }

    async fn reclamaciones_pendientes(&mut self) -> AppResult<Vec<Reclamacion>> {
        sqlx::query!(
            "SELECT id, organizacion_id, cuenta_id, mensaje, estado, creada_en
             FROM reclamaciones_organizacion WHERE estado = 'pendiente' ORDER BY creada_en",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|f| {
            Ok(Reclamacion {
                id: ReclamacionId::from_uuid(f.id),
                organizacion_id: OrganizacionId::from_uuid(f.organizacion_id),
                cuenta_id: CuentaId::from_uuid(f.cuenta_id),
                mensaje: f.mensaje,
                estado: EstadoReclamacion::try_from(f.estado.as_str())?,
                creada_en: f.creada_en,
            })
        })
        .collect()
    }

    async fn resolver_reclamacion(
        &mut self,
        id: ReclamacionId,
        estado: EstadoReclamacion,
        revisor: Option<CuentaId>,
        motivo: Option<&str>,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE reclamaciones_organizacion
             SET estado = $2, revisada_por = $3, motivo = $4, revisada_en = $5
             WHERE id = $1",
            id.as_uuid(),
            estado.as_str(),
            revisor.map(CuentaId::as_uuid),
            motivo,
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}
