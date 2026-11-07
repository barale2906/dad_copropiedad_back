//! Personas de la copropiedad sobre la transacción del tenant (datos personales, Ley 1581).

use application::{
    AppError, AppResult, Pagina, SolicitudPagina,
    puertos::{RepositorioPersonas, copropiedades::Persona},
};
use chrono::NaiveDate;
use domain::{
    CuentaId, DireccionCorreo, Documento, PersonaId, Telefono, TipoDocumento, TipoPersona,
};
use uuid::Uuid;

use crate::{
    TransaccionPg,
    copropiedades::{error_escritura, opcional},
};

pub(crate) struct FilaPersona {
    id: Uuid,
    tipo: String,
    nombre: String,
    documento: Option<String>,
    telefono: Option<String>,
    correo: Option<String>,
    cuenta_id: Option<Uuid>,
    tipo_documento: Option<String>,
    fecha_nacimiento: Option<NaiveDate>,
    contacto_emergencia_nombre: Option<String>,
    contacto_emergencia_telefono: Option<String>,
    movilidad_reducida: bool,
}

impl TryFrom<FilaPersona> for Persona {
    type Error = AppError;

    fn try_from(f: FilaPersona) -> AppResult<Self> {
        Ok(Self {
            id: PersonaId::from_uuid(f.id),
            tipo: TipoPersona::try_from(f.tipo.as_str())?,
            nombre: f.nombre,
            documento: opcional(f.documento, Documento::new)?,
            telefono: opcional(f.telefono, Telefono::new)?,
            correo: opcional(f.correo, DireccionCorreo::new)?,
            cuenta_id: f.cuenta_id.map(CuentaId::from_uuid),
            tipo_documento: opcional(f.tipo_documento, |t| TipoDocumento::try_from(t))?,
            fecha_nacimiento: f.fecha_nacimiento,
            contacto_emergencia_nombre: f.contacto_emergencia_nombre,
            contacto_emergencia_telefono: opcional(f.contacto_emergencia_telefono, Telefono::new)?,
            movilidad_reducida: f.movilidad_reducida,
        })
    }
}

/// Escapa los comodines de `LIKE`.
fn patron(texto: &str) -> String {
    let e = texto
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{e}%")
}

impl TransaccionPg {
    async fn personas_donde(
        &mut self,
        id: Option<Uuid>,
        documento: Option<&str>,
    ) -> AppResult<Option<Persona>> {
        sqlx::query_as!(
            FilaPersona,
            "SELECT id, tipo, nombre, documento, telefono, correo, cuenta_id, tipo_documento,
                    fecha_nacimiento, contacto_emergencia_nombre, contacto_emergencia_telefono,
                    movilidad_reducida
             FROM personas
             WHERE ($1::uuid IS NULL OR id = $1) AND ($2::text IS NULL OR documento = $2)
             LIMIT 1",
            id,
            documento,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(Persona::try_from)
        .transpose()
    }
}

impl RepositorioPersonas for TransaccionPg {
    async fn persona(&mut self, id: PersonaId) -> AppResult<Option<Persona>> {
        self.personas_donde(Some(id.as_uuid()), None).await
    }

    async fn persona_por_documento(&mut self, documento: &Documento) -> AppResult<Option<Persona>> {
        self.personas_donde(None, Some(documento.as_str())).await
    }

    async fn insertar_persona(&mut self, p: &Persona) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO personas (id, tenant_id, tipo, nombre, documento, telefono, correo,
                                   cuenta_id, tipo_documento, fecha_nacimiento,
                                   contacto_emergencia_nombre, contacto_emergencia_telefono,
                                   movilidad_reducida)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            p.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            p.tipo.as_str(),
            p.nombre,
            p.documento.as_ref().map(Documento::as_str),
            p.telefono.as_ref().map(Telefono::as_str),
            p.correo.as_ref().map(DireccionCorreo::as_str),
            p.cuenta_id.map(CuentaId::as_uuid),
            p.tipo_documento.map(TipoDocumento::as_str),
            p.fecha_nacimiento,
            p.contacto_emergencia_nombre,
            p.contacto_emergencia_telefono
                .as_ref()
                .map(Telefono::as_str),
            p.movilidad_reducida,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_escritura(e, "el documento o la cuenta ya están registrados", "cuenta")
        })?;
        Ok(())
    }

    async fn vincular_cuenta(&mut self, persona: PersonaId, cuenta: CuentaId) -> AppResult<()> {
        sqlx::query!(
            "UPDATE personas SET cuenta_id = $2, actualizado_en = now() WHERE id = $1",
            persona.as_uuid(),
            cuenta.as_uuid(),
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "la cuenta ya está vinculada a otra persona", "cuenta"))?;
        Ok(())
    }

    async fn actualizar_persona(&mut self, p: &Persona) -> AppResult<()> {
        sqlx::query!(
            "UPDATE personas SET
                 nombre = $2, documento = $3, telefono = $4, correo = $5, tipo_documento = $6,
                 fecha_nacimiento = $7, contacto_emergencia_nombre = $8,
                 contacto_emergencia_telefono = $9, movilidad_reducida = $10, actualizado_en = now()
             WHERE id = $1",
            p.id.as_uuid(),
            p.nombre,
            p.documento.as_ref().map(Documento::as_str),
            p.telefono.as_ref().map(Telefono::as_str),
            p.correo.as_ref().map(DireccionCorreo::as_str),
            p.tipo_documento.map(TipoDocumento::as_str),
            p.fecha_nacimiento,
            p.contacto_emergencia_nombre,
            p.contacto_emergencia_telefono
                .as_ref()
                .map(Telefono::as_str),
            p.movilidad_reducida,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_escritura(e, "otra persona ya tiene ese documento", "persona"))?;
        Ok(())
    }

    async fn listar_personas(
        &mut self,
        texto: Option<&str>,
        pagina: SolicitudPagina,
    ) -> AppResult<Pagina<Persona>> {
        let filas = sqlx::query_as!(
            FilaPersona,
            r#"SELECT id, tipo, nombre, documento, telefono, correo, cuenta_id, tipo_documento,
                      fecha_nacimiento, contacto_emergencia_nombre, contacto_emergencia_telefono,
                      movilidad_reducida
               FROM personas
               WHERE ($1::uuid IS NULL OR id > $1)
                 AND ($2::text IS NULL OR normalizar(nombre) LIKE normalizar($2) ESCAPE '\'
                      OR documento LIKE upper($2) ESCAPE '\')
               ORDER BY id
               LIMIT $3"#,
            pagina.despues(),
            texto.map(patron),
            pagina.filas_a_consultar(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(Persona::try_from)
        .collect::<AppResult<Vec<_>>>()?;
        Ok(Pagina::desde_filas(filas, pagina, |p| p.id.as_uuid()))
    }
}
