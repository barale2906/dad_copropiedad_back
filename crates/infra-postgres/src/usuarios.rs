//! Usuarios de la entidad (plan 20) sobre la transacción del tenant. Las cuentas son de la
//! plataforma: solo se tocan con las funciones `SECURITY DEFINER` de la migración `usuarios`.

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::{
        copropiedades::Persona,
        usuarios::{
            CuentaResuelta, CuentaSolicitada, FiltroUsuarios, RepositorioUsuarios, UsuarioEntidad,
        },
    },
};
use chrono::{DateTime, Utc};
use domain::{CuentaId, DireccionCorreo, EstadoCuenta, NombreUsuario};
use uuid::Uuid;

use crate::{
    TransaccionPg,
    copropiedades::{error_escritura, opcional},
};

struct FilaUsuario {
    id: Option<Uuid>,
    nombre_mostrar: Option<String>,
    usuario: Option<String>,
    correo: Option<String>,
    estado: Option<String>,
    contrasena_pendiente: Option<bool>,
    ultimo_acceso_en: Option<DateTime<Utc>>,
    roles: Option<Vec<String>>,
}

impl TryFrom<FilaUsuario> for UsuarioEntidad {
    type Error = AppError;

    fn try_from(f: FilaUsuario) -> AppResult<Self> {
        let falta =
            || AppError::Infraestructura("usuarios_entidad devolvió una fila incompleta".into());
        Ok(Self {
            id: CuentaId::from_uuid(f.id.ok_or_else(falta)?),
            nombre_mostrar: f.nombre_mostrar.ok_or_else(falta)?,
            usuario: opcional(f.usuario, NombreUsuario::new)?,
            correo: opcional(f.correo, DireccionCorreo::new)?,
            estado: EstadoCuenta::try_from(f.estado.ok_or_else(falta)?.as_str())?,
            contrasena_pendiente: f.contrasena_pendiente.unwrap_or(false),
            ultimo_acceso_en: f.ultimo_acceso_en,
            roles: f.roles.unwrap_or_default(),
        })
    }
}

/// Escapa los comodines de `LIKE` y envuelve en `%…%`.
fn patron(texto: &str) -> String {
    let e = texto
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{e}%")
}

impl TransaccionPg {
    async fn usuarios_donde(
        &mut self,
        cuenta: Option<CuentaId>,
        texto: Option<&str>,
        despues: Option<Uuid>,
        limite: i64,
        ahora: DateTime<Utc>,
    ) -> AppResult<Vec<UsuarioEntidad>> {
        sqlx::query_as!(
            FilaUsuario,
            "SELECT id, nombre_mostrar, usuario, correo, estado, contrasena_pendiente,
                    ultimo_acceso_en, roles
             FROM usuarios_entidad($1, $2, $3, $4, $5)",
            ahora,
            cuenta.map(CuentaId::as_uuid),
            texto.map(patron),
            despues,
            i32::try_from(limite).unwrap_or(i32::MAX),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(UsuarioEntidad::try_from)
        .collect()
    }
}

impl RepositorioUsuarios for TransaccionPg {
    async fn resolver_cuenta(
        &mut self,
        s: &CuentaSolicitada,
        ahora: DateTime<Utc>,
    ) -> AppResult<CuentaResuelta> {
        let f = sqlx::query!(
            r#"SELECT id, resultado AS "resultado!", nombre_mostrar, pendiente AS "pendiente!"
               FROM resolver_cuenta_para_entidad($1, $2, $3, $4, $5)"#,
            s.id.as_uuid(),
            s.nombre_mostrar,
            s.usuario.as_ref().map(NombreUsuario::as_str),
            s.correo.as_str(),
            ahora,
        )
        .fetch_one(&mut *self.tx)
        .await
        // Otra petición creó la misma cuenta a la vez: el cliente reintenta y la reconoce.
        .map_err(|e| error_escritura(e, "el usuario o el correo se acaban de registrar", "cuenta"))?;
        let id = || {
            f.id.map(CuentaId::from_uuid).ok_or_else(|| {
                AppError::Infraestructura("resolver_cuenta_para_entidad sin id".into())
            })
        };
        Ok(match f.resultado.as_str() {
            "creada" => CuentaResuelta::Creada(id()?),
            "existente" => CuentaResuelta::Existente {
                id: id()?,
                nombre_mostrar: f.nombre_mostrar.clone().unwrap_or_default(),
                pendiente: f.pendiente,
            },
            "inactiva" => CuentaResuelta::Inactiva,
            "usuario_ocupado" => CuentaResuelta::UsuarioOcupado,
            otro => {
                return Err(AppError::Infraestructura(
                    format!("resultado inesperado de resolver_cuenta_para_entidad: {otro}").into(),
                ));
            }
        })
    }

    async fn emitir_codigo_definir(
        &mut self,
        cuenta: CuentaId,
        hash_codigo: &str,
        expira_en: DateTime<Utc>,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        sqlx::query_scalar!(
            r#"SELECT emitir_codigo_definir_contrasena($1, $2, $3, $4, $5) AS "emitido!""#,
            Uuid::now_v7(),
            cuenta.as_uuid(),
            hash_codigo,
            expira_en,
            ahora,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }

    async fn usuarios(
        &mut self,
        filtro: &FiltroUsuarios,
        pagina: SolicitudPagina,
        ahora: DateTime<Utc>,
    ) -> AppResult<Pagina<UsuarioEntidad>> {
        let filas = self
            .usuarios_donde(
                None,
                filtro.texto.as_deref(),
                pagina.despues(),
                pagina.filas_a_consultar(),
                ahora,
            )
            .await?;
        Ok(Pagina::desde_filas(filas, pagina, |u| u.id.as_uuid()))
    }

    async fn usuario(
        &mut self,
        cuenta: CuentaId,
        ahora: DateTime<Utc>,
    ) -> AppResult<Option<UsuarioEntidad>> {
        Ok(self
            .usuarios_donde(Some(cuenta), None, None, 1, ahora)
            .await?
            .into_iter()
            .next())
    }

    async fn persona_por_cuenta(&mut self, cuenta: CuentaId) -> AppResult<Option<Persona>> {
        self.persona_de_cuenta_pg(cuenta).await
    }
}
