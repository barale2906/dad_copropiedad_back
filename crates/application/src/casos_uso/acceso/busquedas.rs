//! Búsquedas de la portería: el bien al que va una visita o un paquete y la persona a la que se
//! emite una credencial, **sin** `bienes_privados.ver` ni `personas.ver`. Devuelven lo mínimo
//! (código del bien; nombre y documento enmascarado de la persona) para que las plantillas
//! `vigilante` y `supervisor_seguridad` trabajen sin ver fichas completas.

use domain::{BienPrivadoId, Permiso, PersonaId, acceso::documento_enmascarado};

use super::{TxAcceso, abrir, tiene};
use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    puertos::{
        Reloj, RepositorioBienesPrivados, RepositorioPersonas, UnidadDeTrabajo,
        copropiedades::FiltroBienes,
    },
};

/// Mínimo de caracteres para buscar personas: evita listar a todas desde la portería.
pub const MIN_TEXTO_PERSONA: usize = 2;

/// Bien privado tal como lo ve la portería.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BienPorteria {
    pub id: BienPrivadoId,
    /// Código del bien («T1-101»).
    pub codigo: String,
}

/// Persona tal como la ve la portería.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaPorteria {
    pub id: PersonaId,
    /// Dato personal.
    pub nombre: String,
    /// Últimos cuatro caracteres del documento (`****5678`), si lo tiene.
    pub documento: Option<String>,
}

/// Bienes privados activos cuyo código contiene `texto` (sin tildes ni mayúsculas), para
/// anunciar visitas y recibir correspondencia.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.registrar`;
/// [`AppError::ModuloNoHabilitado`] sin el control de acceso.
pub async fn buscar_bienes<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    texto: Option<String>,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<BienPorteria>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    ctx.exigir(Permiso::AccesoRegistrar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let filtro = FiltroBienes {
        texto: texto.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()),
        incluir_inactivos: false,
        ..FiltroBienes::default()
    };
    let pagina = RepositorioBienesPrivados::listar_bienes(&mut tx, &filtro, pagina).await?;
    Ok(pagina.mapear(|b| BienPorteria {
        id: b.id,
        codigo: b.codigo.as_str().to_owned(),
    }))
}

/// Personas cuyo nombre (sin tildes ni mayúsculas) o documento contiene `texto`, para emitir
/// credenciales o registrar el consentimiento biométrico.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `acceso.credenciales.gestionar` ni `acceso.biometria.enrolar`;
/// [`AppError::Validacion`] con menos de [`MIN_TEXTO_PERSONA`] caracteres;
/// [`AppError::ModuloNoHabilitado`] sin el control de acceso.
pub async fn buscar_personas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    texto: &str,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<PersonaPorteria>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxAcceso,
{
    if !tiene(ctx, Permiso::AccesoCredencialesGestionar) {
        ctx.exigir(Permiso::AccesoBiometriaEnrolar)?;
    }
    let texto = texto.trim();
    if texto.chars().count() < MIN_TEXTO_PERSONA {
        return Err(AppError::Validacion(
            "escriba al menos 2 caracteres del nombre o del documento".into(),
        ));
    }
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let pagina = RepositorioPersonas::listar_personas(&mut tx, Some(texto), pagina).await?;
    Ok(pagina.mapear(|p| PersonaPorteria {
        id: p.id,
        nombre: p.nombre,
        documento: p
            .documento
            .as_ref()
            .map(|d| documento_enmascarado(d.as_str())),
    }))
}
