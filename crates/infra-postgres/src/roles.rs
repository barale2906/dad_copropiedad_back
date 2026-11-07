//! Roles, asignaciones y contratos de servicio dentro de la transacción de una copropiedad.

use application::{
    AppError, AppResult,
    puertos::{
        RepositorioContratos, RepositorioRoles,
        copropiedades::AltaCopropiedad,
        identidad::{
            Asignacion, ContratoServicio, CuentaResumen, FichaOrganizacion, NuevaOrganizacion, Rol,
        },
    },
};
use chrono::{DateTime, Utc};
use domain::{
    AsignacionRolId, ConjuntoPermisos, ContratoServicioId, CuentaId, EstadoContrato,
    EstadoOrganizacion, Identificador, NaturalezaOrganizacion, Nit, OrganizacionId,
    OrigenAsignacion, RolId, TipoOrganizacion, permisos::CODIGO_ADMINISTRADOR,
};
use uuid::Uuid;

use crate::{
    TransaccionPg,
    plataforma::{FilaRol, conflicto_si_unico, conjunto, tipos},
};

struct FilaAsignacion {
    id: Uuid,
    cuenta_id: Uuid,
    rol_id: Uuid,
    vigente_desde: DateTime<Utc>,
    vigente_hasta: Option<DateTime<Utc>>,
    origen: String,
    origen_id: Option<Uuid>,
    asignado_por: Option<Uuid>,
}

impl TryFrom<FilaAsignacion> for Asignacion {
    type Error = AppError;

    fn try_from(f: FilaAsignacion) -> AppResult<Self> {
        Ok(Self {
            id: AsignacionRolId::from_uuid(f.id),
            cuenta_id: CuentaId::from_uuid(f.cuenta_id),
            rol_id: RolId::from_uuid(f.rol_id),
            vigente_desde: f.vigente_desde,
            vigente_hasta: f.vigente_hasta,
            origen: OrigenAsignacion::try_from(f.origen.as_str())?,
            origen_id: f.origen_id,
            asignado_por: f.asignado_por.map(CuentaId::from_uuid),
        })
    }
}

impl TransaccionPg {
    /// Roles de copropiedad visibles (plantillas + propios, por RLS).
    async fn roles_donde(&mut self, id: Option<Uuid>, codigo: Option<&str>) -> AppResult<Vec<Rol>> {
        sqlx::query_as!(
            FilaRol,
            r#"SELECT r.id, r.tenant_id, r.codigo, r.nombre, r.descripcion, r.ambito, r.es_plantilla,
                      coalesce(array_agg(rp.permiso) FILTER (WHERE rp.permiso IS NOT NULL), '{}')
                          AS "permisos!"
               FROM roles r LEFT JOIN roles_permisos rp ON rp.rol_id = r.id
               WHERE r.ambito = 'copropiedad'
                 AND ($1::uuid IS NULL OR r.id = $1)
                 AND ($2::text IS NULL OR r.codigo = $2)
               GROUP BY r.id
               ORDER BY r.es_plantilla DESC, r.nombre"#,
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

    async fn insertar_permisos_rol(
        &mut self,
        rol: RolId,
        permisos: &ConjuntoPermisos,
    ) -> AppResult<()> {
        let textos: Vec<String> = permisos.iter().map(|p| p.as_str().to_owned()).collect();
        sqlx::query!(
            "INSERT INTO roles_permisos (rol_id, ambito, permiso, tenant_id)
             SELECT $1, 'copropiedad', p, $3 FROM unnest($2::text[]) AS p",
            rol.as_uuid(),
            &textos,
            self.ctx.tenant_id.as_uuid(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}

impl RepositorioRoles for TransaccionPg {
    async fn roles(&mut self) -> AppResult<Vec<Rol>> {
        self.roles_donde(None, None).await
    }

    async fn rol(&mut self, id: RolId) -> AppResult<Option<Rol>> {
        Ok(self.roles_donde(Some(id.as_uuid()), None).await?.pop())
    }

    async fn rol_por_codigo(&mut self, codigo: &str) -> AppResult<Option<Rol>> {
        // Si una PH creó un rol con el código de una plantilla, gana la plantilla.
        Ok(self
            .roles_donde(None, Some(codigo))
            .await?
            .into_iter()
            .next())
    }

    async fn insertar_rol(&mut self, rol: &Rol) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO roles (id, tenant_id, codigo, nombre, descripcion, ambito)
             VALUES ($1, $2, $3, $4, $5, 'copropiedad')",
            rol.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            rol.codigo,
            rol.nombre,
            rol.descripcion,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "ya existe un rol con ese código"))?;
        self.insertar_permisos_rol(rol.id, &rol.permisos).await
    }

    async fn reemplazar_permisos(
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
        self.insertar_permisos_rol(rol, permisos).await
    }

    async fn insertar_asignacion(&mut self, a: &Asignacion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO asignaciones_rol (id, tenant_id, cuenta_id, rol_id, vigente_desde,
                                           vigente_hasta, origen, origen_id, asignado_por)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            a.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            a.cuenta_id.as_uuid(),
            a.rol_id.as_uuid(),
            a.vigente_desde,
            a.vigente_hasta,
            a.origen.as_str(),
            a.origen_id,
            a.asignado_por.map(CuentaId::as_uuid),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            if e.as_database_error()
                .is_some_and(|d| d.is_foreign_key_violation())
            {
                AppError::NoEncontrado("cuenta")
            } else {
                AppError::infra(e)
            }
        })?;
        Ok(())
    }

    async fn asignacion(&mut self, id: AsignacionRolId) -> AppResult<Option<Asignacion>> {
        sqlx::query_as!(
            FilaAsignacion,
            "SELECT id, cuenta_id, rol_id, vigente_desde, vigente_hasta, origen, origen_id, asignado_por
             FROM asignaciones_rol WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Asignacion::try_from)
        .transpose()
    }

    async fn asignaciones(&mut self, cuenta: Option<CuentaId>) -> AppResult<Vec<Asignacion>> {
        sqlx::query_as!(
            FilaAsignacion,
            "SELECT id, cuenta_id, rol_id, vigente_desde, vigente_hasta, origen, origen_id, asignado_por
             FROM asignaciones_rol
             WHERE $1::uuid IS NULL OR cuenta_id = $1
             ORDER BY vigente_desde DESC, id DESC",
            cuenta.map(CuentaId::as_uuid),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Asignacion::try_from)
        .collect()
    }

    async fn terminar_asignacion(
        &mut self,
        id: AsignacionRolId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE asignaciones_rol SET vigente_hasta = greatest($2, vigente_desde)
             WHERE id = $1 AND (vigente_hasta IS NULL OR vigente_hasta > $2)",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn terminar_por_origen(
        &mut self,
        origen: OrigenAsignacion,
        origen_id: Uuid,
        ahora: DateTime<Utc>,
    ) -> AppResult<u64> {
        Ok(sqlx::query!(
            "UPDATE asignaciones_rol SET vigente_hasta = greatest($3, vigente_desde)
             WHERE origen = $1 AND origen_id = $2 AND (vigente_hasta IS NULL OR vigente_hasta > $3)",
            origen.as_str(),
            origen_id,
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected())
    }

    async fn permisos_de_cuenta(
        &mut self,
        cuenta: CuentaId,
        ahora: DateTime<Utc>,
    ) -> AppResult<ConjuntoPermisos> {
        let permisos = sqlx::query_scalar!(
            r#"SELECT DISTINCT rp.permiso AS "permiso!"
               FROM asignaciones_rol a
               JOIN roles_permisos rp ON rp.rol_id = a.rol_id
               WHERE a.cuenta_id = $1 AND a.vigente_desde <= $2
                 AND (a.vigente_hasta IS NULL OR a.vigente_hasta > $2)"#,
            cuenta.as_uuid(),
            ahora,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(conjunto(&permisos))
    }

    async fn administradores_vigentes(&mut self, ahora: DateTime<Utc>) -> AppResult<u64> {
        let n = sqlx::query_scalar!(
            r#"SELECT count(DISTINCT a.cuenta_id) AS "n!"
               FROM asignaciones_rol a JOIN roles r ON r.id = a.rol_id
               WHERE r.es_plantilla AND r.codigo = $1 AND a.vigente_desde <= $2
                 AND (a.vigente_hasta IS NULL OR a.vigente_hasta > $2)"#,
            CODIGO_ADMINISTRADOR,
            ahora,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    async fn buscar_cuenta(
        &mut self,
        identificador: &Identificador,
    ) -> AppResult<Option<CuentaResumen>> {
        let texto = match identificador {
            Identificador::Correo(c) => c.as_str(),
            Identificador::Usuario(u) => u.as_str(),
        };
        Ok(sqlx::query!(
            r#"SELECT id AS "id!", nombre_mostrar AS "nombre_mostrar!" FROM buscar_cuenta($1)"#,
            texto
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|f| CuentaResumen {
            id: CuentaId::from_uuid(f.id),
            nombre_mostrar: f.nombre_mostrar,
        }))
    }

    async fn insertar_copropiedad(&mut self, alta: &AltaCopropiedad) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO copropiedades (id, nombre, nit, tipo, forma) VALUES ($1, $2, $3, $4, $5)",
            self.ctx.tenant_id.as_uuid(),
            alta.nombre,
            alta.nit.as_ref().map(ToString::to_string),
            alta.tipo.as_str(),
            alta.forma.as_str(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflicto_si_unico(e, "otra copropiedad ya tiene ese NIT"))?;
        Ok(())
    }
}

struct FilaContrato {
    id: Uuid,
    organizacion_id: Uuid,
    tipo_servicio: String,
    vigente_desde: DateTime<Utc>,
    vigente_hasta: Option<DateTime<Utc>>,
    estado: String,
    valor: Option<rust_decimal::Decimal>,
}

impl TryFrom<FilaContrato> for ContratoServicio {
    type Error = AppError;

    fn try_from(f: FilaContrato) -> AppResult<Self> {
        Ok(Self {
            id: ContratoServicioId::from_uuid(f.id),
            organizacion_id: OrganizacionId::from_uuid(f.organizacion_id),
            tipo_servicio: TipoOrganizacion::try_from(f.tipo_servicio.as_str())?,
            vigente_desde: f.vigente_desde,
            vigente_hasta: f.vigente_hasta,
            estado: EstadoContrato::try_from(f.estado.as_str())?,
            valor: f.valor,
        })
    }
}

impl RepositorioContratos for TransaccionPg {
    async fn buscar_organizacion_por_nit(
        &mut self,
        nit: &Nit,
    ) -> AppResult<Option<FichaOrganizacion>> {
        let Some(f) = sqlx::query!(
            r#"SELECT id AS "id!", razon_social AS "razon_social!", naturaleza AS "naturaleza!",
                      tipos AS "tipos!", estado AS "estado!"
               FROM buscar_organizacion_por_nit($1)"#,
            nit.to_string(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        else {
            return Ok(None);
        };
        Ok(Some(FichaOrganizacion {
            id: OrganizacionId::from_uuid(f.id),
            razon_social: f.razon_social,
            naturaleza: NaturalezaOrganizacion::try_from(f.naturaleza.as_str())?,
            tipos: tipos(&f.tipos)?,
            estado: EstadoOrganizacion::try_from(f.estado.as_str())?,
        }))
    }

    async fn crear_organizacion(
        &mut self,
        id: OrganizacionId,
        n: &NuevaOrganizacion,
        creada_por: Option<CuentaId>,
    ) -> AppResult<(OrganizacionId, bool)> {
        let tipos: Vec<String> = n.tipos.iter().map(|t| t.as_str().to_owned()).collect();
        let f = sqlx::query!(
            r#"SELECT id AS "id!", creada AS "creada!"
               FROM crear_organizacion($1, $2, $3, $4, $5, $6, $7, $8)"#,
            id.as_uuid(),
            n.nit.to_string(),
            n.razon_social.trim(),
            n.naturaleza.as_str(),
            &tipos,
            n.correo_contacto.as_deref(),
            n.telefono_contacto.as_deref(),
            creada_por.map(CuentaId::as_uuid),
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok((OrganizacionId::from_uuid(f.id), f.creada))
    }

    async fn insertar_contrato(&mut self, c: &ContratoServicio) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO contratos_servicio (id, tenant_id, organizacion_id, tipo_servicio,
                                             vigente_desde, vigente_hasta, estado, valor)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            c.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            c.organizacion_id.as_uuid(),
            c.tipo_servicio.as_str(),
            c.vigente_desde,
            c.vigente_hasta,
            c.estado.as_str(),
            c.valor,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            if e.as_database_error()
                .is_some_and(|d| d.is_foreign_key_violation())
            {
                AppError::NoEncontrado("organización")
            } else {
                AppError::infra(e)
            }
        })?;
        Ok(())
    }

    async fn contrato(&mut self, id: ContratoServicioId) -> AppResult<Option<ContratoServicio>> {
        sqlx::query_as!(
            FilaContrato,
            "SELECT id, organizacion_id, tipo_servicio, vigente_desde, vigente_hasta, estado, valor
             FROM contratos_servicio WHERE id = $1",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(ContratoServicio::try_from)
        .transpose()
    }

    async fn contratos(&mut self) -> AppResult<Vec<ContratoServicio>> {
        sqlx::query_as!(
            FilaContrato,
            "SELECT id, organizacion_id, tipo_servicio, vigente_desde, vigente_hasta, estado, valor
             FROM contratos_servicio ORDER BY vigente_desde DESC",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(ContratoServicio::try_from)
        .collect()
    }

    async fn terminar_contrato(
        &mut self,
        id: ContratoServicioId,
        ahora: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query!(
            "UPDATE contratos_servicio
             SET estado = 'terminado',
                 vigente_hasta = CASE WHEN vigente_hasta IS NULL OR vigente_hasta > $2
                                      THEN greatest($2, vigente_desde) ELSE vigente_hasta END
             WHERE id = $1",
            id.as_uuid(),
            ahora,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }
}
