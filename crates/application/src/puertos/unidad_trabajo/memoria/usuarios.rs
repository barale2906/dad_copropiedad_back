//! Doble en memoria de los usuarios de la entidad (plan 20): las cuentas de la plataforma que
//! ven las funciones `SECURITY DEFINER` y los códigos para definir la contraseña.

use chrono::{DateTime, Utc};
use domain::{CuentaId, DireccionCorreo, EstadoCuenta, NombreUsuario};

use super::TransaccionEnMemoria;
use crate::{
    AppResult, Pagina, SolicitudPagina,
    puertos::{
        copropiedades::Persona,
        usuarios::{
            CuentaResuelta, CuentaSolicitada, FiltroUsuarios, RepositorioUsuarios, UsuarioEntidad,
        },
    },
};

/// Cuenta de la plataforma tal como la ven las funciones de la entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuentaMemoria {
    pub id: CuentaId,
    pub nombre_mostrar: String,
    pub usuario: Option<NombreUsuario>,
    pub correo: DireccionCorreo,
    pub estado: EstadoCuenta,
    pub pendiente: bool,
}

impl CuentaMemoria {
    /// Cuenta activa con la contraseña ya definida.
    pub fn activa(nombre: &str, correo: &str) -> Self {
        Self {
            id: CuentaId::nuevo(),
            nombre_mostrar: nombre.into(),
            usuario: None,
            correo: DireccionCorreo::new(correo).expect("correo de prueba válido"),
            estado: EstadoCuenta::Activa,
            pendiente: false,
        }
    }
}

/// Código emitido para definir la contraseña.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodigoEmitido {
    pub cuenta: CuentaId,
    pub hash: String,
    pub expira_en: DateTime<Utc>,
}

/// Cuentas y códigos en memoria.
#[derive(Default, Clone)]
pub struct DatosUsuarios {
    pub cuentas: Vec<CuentaMemoria>,
    pub codigos: Vec<CodigoEmitido>,
}

impl TransaccionEnMemoria {
    fn roles_vigentes(&self, cuenta: CuentaId, ahora: DateTime<Utc>) -> Vec<String> {
        let roles = self.roles_visibles();
        let mut codigos: Vec<String> = self
            .asignaciones_tenant()
            .filter(|a| a.cuenta_id == cuenta && a.vigente_en(ahora))
            .filter_map(|a| roles.iter().find(|r| r.id == a.rol_id))
            .map(|r| r.codigo.clone())
            .collect();
        codigos.sort();
        codigos.dedup();
        codigos
    }

    fn como_usuario(&self, c: &CuentaMemoria, ahora: DateTime<Utc>) -> Option<UsuarioEntidad> {
        let roles = self.roles_vigentes(c.id, ahora);
        (!roles.is_empty()).then(|| UsuarioEntidad {
            id: c.id,
            nombre_mostrar: c.nombre_mostrar.clone(),
            usuario: c.usuario.clone(),
            correo: Some(c.correo.clone()),
            estado: c.estado,
            contrasena_pendiente: c.pendiente,
            ultimo_acceso_en: None,
            roles,
        })
    }
}

impl RepositorioUsuarios for TransaccionEnMemoria {
    async fn resolver_cuenta(
        &mut self,
        s: &CuentaSolicitada,
        _ahora: DateTime<Utc>,
    ) -> AppResult<CuentaResuelta> {
        let cuentas = &mut self.datos.usuarios.cuentas;
        if let Some(c) = cuentas.iter().find(|c| c.correo == s.correo) {
            return Ok(if c.estado == EstadoCuenta::Activa {
                CuentaResuelta::Existente {
                    id: c.id,
                    nombre_mostrar: c.nombre_mostrar.clone(),
                    pendiente: c.pendiente,
                }
            } else {
                CuentaResuelta::Inactiva
            });
        }
        if s.usuario.is_some() && cuentas.iter().any(|c| c.usuario == s.usuario) {
            return Ok(CuentaResuelta::UsuarioOcupado);
        }
        cuentas.push(CuentaMemoria {
            id: s.id,
            nombre_mostrar: s.nombre_mostrar.clone(),
            usuario: s.usuario.clone(),
            correo: s.correo.clone(),
            estado: EstadoCuenta::Activa,
            pendiente: true,
        });
        Ok(CuentaResuelta::Creada(s.id))
    }

    async fn emitir_codigo_definir(
        &mut self,
        cuenta: CuentaId,
        hash_codigo: &str,
        expira_en: DateTime<Utc>,
        ahora: DateTime<Utc>,
    ) -> AppResult<bool> {
        let aplica = self
            .datos
            .usuarios
            .cuentas
            .iter()
            .any(|c| c.id == cuenta && c.pendiente && c.estado == EstadoCuenta::Activa)
            && self.asignaciones_tenant().any(|a| a.cuenta_id == cuenta)
            && !self.roles_vigentes(cuenta, ahora).is_empty();
        if aplica {
            self.datos.usuarios.codigos.push(CodigoEmitido {
                cuenta,
                hash: hash_codigo.into(),
                expira_en,
            });
        }
        Ok(aplica)
    }

    async fn usuarios(
        &mut self,
        filtro: &FiltroUsuarios,
        pagina: SolicitudPagina,
        ahora: DateTime<Utc>,
    ) -> AppResult<Pagina<UsuarioEntidad>> {
        let texto = filtro.texto.as_ref().map(|t| t.to_lowercase());
        let mut filas: Vec<UsuarioEntidad> = self
            .datos
            .usuarios
            .cuentas
            .iter()
            .filter_map(|c| self.como_usuario(c, ahora))
            .filter(|u| {
                texto.as_ref().is_none_or(|t| {
                    u.nombre_mostrar.to_lowercase().contains(t)
                        || u.correo.as_ref().is_some_and(|c| c.as_str().contains(t))
                })
            })
            .filter(|u| pagina.despues().is_none_or(|d| u.id.as_uuid() > d))
            .collect();
        filas.sort_by_key(|u| u.id);
        filas.truncate(usize::try_from(pagina.filas_a_consultar()).unwrap_or(usize::MAX));
        Ok(Pagina::desde_filas(filas, pagina, |u| u.id.as_uuid()))
    }

    async fn usuario(
        &mut self,
        cuenta: CuentaId,
        ahora: DateTime<Utc>,
    ) -> AppResult<Option<UsuarioEntidad>> {
        Ok(self
            .datos
            .usuarios
            .cuentas
            .iter()
            .find(|c| c.id == cuenta)
            .and_then(|c| self.como_usuario(c, ahora)))
    }

    async fn persona_por_cuenta(&mut self, cuenta: CuentaId) -> AppResult<Option<Persona>> {
        Ok(self
            .datos
            .personas
            .iter()
            .find(|p| p.cuenta_id == Some(cuenta))
            .cloned())
    }
}
