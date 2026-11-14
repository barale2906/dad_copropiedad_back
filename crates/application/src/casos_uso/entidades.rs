//! Ficha de cualquier entidad (ADR 0016): nombre, NIT, ubicación, contacto y zona horaria. La
//! sirve `/entidades/{id}` para la empresa que no es PH; la PH la edita con su ficha completa
//! ([`crate::casos_uso::copropiedades`]), que valida estos mismos campos con [`CambiosEntidad`].

use domain::{
    CodigoDane, DireccionCorreo, Nit, Permiso, Telefono,
    copropiedad::{texto_obligatorio, texto_opcional},
};
use rust_decimal::Decimal;

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, RepositorioEntidad, Transaccion, UnidadDeTrabajo,
        entidades::FichaEntidad,
    },
};

pub(crate) const ACCION_ACTUALIZAR: AccionAuditoria =
    AccionAuditoria::new("entidad.datos.actualizar");

/// Cambios a los datos generales. `None` = sin cambio. En los textos, `Some("")` borra el valor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CambiosEntidad {
    pub nombre: Option<String>,
    pub razon_social: Option<String>,
    pub nit: Option<Nit>,
    pub direccion: Option<String>,
    pub municipio_dane: Option<CodigoDane>,
    pub departamento: Option<String>,
    pub barrio: Option<String>,
    pub latitud: Option<Decimal>,
    pub longitud: Option<Decimal>,
    pub telefono: Option<Telefono>,
    pub correo: Option<DireccionCorreo>,
    pub sitio_web: Option<String>,
    pub zona_horaria: Option<String>,
}

fn coordenada(campo: &'static str, v: Decimal, limite: Decimal) -> AppResult<Decimal> {
    if v.abs() > limite || v.normalize().scale() > 6 {
        return Err(AppError::Validacion(format!(
            "`{campo}` debe estar entre -{limite} y {limite} con máximo 6 decimales"
        )));
    }
    Ok(v)
}

pub(crate) fn texto(
    actual: &mut Option<String>,
    campo: &'static str,
    nuevo: Option<String>,
    maximo: usize,
) -> AppResult<()> {
    if let Some(v) = nuevo {
        *actual = texto_opcional(campo, Some(&v), maximo)?;
    }
    Ok(())
}

fn zona_iana(z: &str) -> AppResult<String> {
    let z = z.trim();
    let valida = z.split('/').count() >= 2
        && z.split('/')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphabetic() || c == '_'));
    if !valida {
        return Err(AppError::Validacion(
            "`zona_horaria` debe ser una zona IANA, p. ej. America/Bogota".into(),
        ));
    }
    Ok(z.to_owned())
}

impl CambiosEntidad {
    /// Aplica los cambios sobre la ficha validando cada campo.
    ///
    /// # Errors
    ///
    /// [`AppError::Validacion`] o [`AppError::Dominio`] con el primer campo inválido.
    pub fn aplicar(self, f: &mut FichaEntidad) -> AppResult<()> {
        if let Some(n) = self.nombre {
            f.nombre = texto_obligatorio("nombre", &n, 200)?;
        }
        texto(&mut f.razon_social, "razon_social", self.razon_social, 200)?;
        texto(&mut f.direccion, "direccion", self.direccion, 200)?;
        texto(&mut f.departamento, "departamento", self.departamento, 80)?;
        texto(&mut f.barrio, "barrio", self.barrio, 120)?;
        texto(&mut f.sitio_web, "sitio_web", self.sitio_web, 200)?;
        if f.sitio_web
            .as_deref()
            .is_some_and(|s| !(s.starts_with("https://") || s.starts_with("http://")))
        {
            return Err(AppError::Validacion(
                "`sitio_web` debe empezar por https://".into(),
            ));
        }
        if let Some(z) = self.zona_horaria {
            f.zona_horaria = zona_iana(&z)?;
        }
        if let Some(v) = self.latitud {
            f.latitud = Some(coordenada("latitud", v, Decimal::from(90))?);
        }
        if let Some(v) = self.longitud {
            f.longitud = Some(coordenada("longitud", v, Decimal::from(180))?);
        }
        if let Some(v) = self.nit {
            f.nit = Some(v);
        }
        if let Some(v) = self.municipio_dane {
            f.municipio_dane = Some(v);
        }
        if let Some(v) = self.telefono {
            f.telefono = Some(v);
        }
        if let Some(v) = self.correo {
            f.correo = Some(v);
        }
        Ok(())
    }
}

/// Ficha de la entidad del contexto.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `copropiedad.datos.ver`; [`AppError::NoEncontrado`].
pub async fn ver_ficha_entidad<U>(unidad: &U, ctx: &Contexto) -> AppResult<FichaEntidad>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioEntidad,
{
    ctx.exigir(Permiso::CopropiedadDatosVer)?;
    unidad
        .iniciar(ctx)
        .await?
        .ficha_entidad()
        .await?
        .ok_or(AppError::NoEncontrado("entidad"))
}

/// Actualiza los datos generales de la entidad (el estado lo cambia solo la plataforma).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `copropiedad.datos.editar`; errores de validación;
/// [`AppError::Conflicto`] si el NIT ya está registrado en otra entidad.
pub async fn actualizar_ficha_entidad<U>(
    unidad: &U,
    ctx: &Contexto,
    cambios: CambiosEntidad,
) -> AppResult<FichaEntidad>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioEntidad + RegistroAuditoria,
{
    ctx.exigir(Permiso::CopropiedadDatosEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut ficha = tx
        .ficha_entidad()
        .await?
        .ok_or(AppError::NoEncontrado("entidad"))?;
    cambios.aplicar(&mut ficha)?;
    tx.guardar_ficha_entidad(&ficha).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_ACTUALIZAR,
        "entidad",
        ctx.tenant_id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(ficha)
}
