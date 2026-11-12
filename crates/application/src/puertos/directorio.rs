//! Puertos del directorio de comercios y servicios (plan 08, fase 1; ADR 0013).
//!
//! - [`RepositorioDirectorioPlataforma`]: en la transacción de **plataforma** (catálogo de
//!   categorías, anunciantes, fichas y cola de moderación de reseñas).
//! - [`RepositorioDirectorio`]: en la transacción de una **copropiedad** (fichas que la
//!   alcanzan, por una función `SECURITY DEFINER`, y reseñas de sus residentes).
//! - [`RepositorioSolicitudesProveedor`]: búsquedas de proveedores, contactos autorizados y el
//!   enlace público del directorio (fase 2, ADR 0014), también por copropiedad.
//! - [`PurgaDirectorio`]: borra el texto de las búsquedas vencidas en todas las copropiedades.

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::{
    AnuncianteId, Calificacion, CanalSolicitud, CategoriaElegible, Cobertura, CodigoCategoria,
    CodigoDane, CuentaId, DireccionCorreo, EstadoResena, EstadoVerificacion, OrganizacionId,
    PersonaId, ResenaId, SolicitudProveedorId, Telefono, TenantId, Ubicacion,
};
use rust_decimal::Decimal;

use crate::{
    AppResult, Pagina, SolicitudPagina, outbox::EventoSaliente,
    puertos::identidad::NuevaOrganizacion,
};

/// Categoría del catálogo (árbol de dos niveles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Categoria {
    pub codigo: CodigoCategoria,
    /// Rama a la que pertenece; `None` = es una rama.
    pub padre: Option<CodigoCategoria>,
    pub nombre: String,
    /// Otras formas de nombrarla («modista», «arreglos de ropa» → costura).
    pub sinonimos: Vec<String>,
    pub restringida: bool,
    pub activa: bool,
}

impl Categoria {
    /// Lo que necesita la regla [`domain::directorio::validar_categorias`].
    pub fn elegible(&self) -> CategoriaElegible {
        CategoriaElegible {
            codigo: self.codigo.clone(),
            restringida: self.restringida,
            activa: self.activa,
        }
    }
}

/// Anunciante (comercio, profesional o persona natural con RUT), sobre una organización.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anunciante {
    pub id: AnuncianteId,
    pub organizacion_id: OrganizacionId,
    pub nombre_comercial: String,
    pub direccion: Option<String>,
    pub ubicacion: Ubicacion,
    pub estado: EstadoVerificacion,
    /// Motivo del rechazo o de la suspensión.
    pub motivo_estado: Option<String>,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
    pub actualizado_en: DateTime<Utc>,
}

/// Datos de contacto que el anunciante publica en su ficha.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Contacto {
    pub telefono: Option<Telefono>,
    pub whatsapp: Option<Telefono>,
    pub correo: Option<DireccionCorreo>,
    /// Solo `https://`.
    pub sitio_web: Option<String>,
}

/// Ficha del directorio de un anunciante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ficha {
    pub anunciante_id: AnuncianteId,
    pub descripcion: Option<String>,
    /// De 1 a 5; la primera es la principal.
    pub categorias: Vec<CodigoCategoria>,
    pub cobertura: Cobertura,
    pub contacto: Contacto,
    pub horario: Option<String>,
    /// Destacada (suscripción) hasta esta fecha.
    pub destacada_hasta: Option<DateTime<Utc>>,
    /// `false` la retira del directorio sin borrarla.
    pub publicada: bool,
    pub actualizada_en: DateTime<Utc>,
}

/// Filtro de anunciantes en la plataforma.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroAnunciantes {
    pub estado: Option<EstadoVerificacion>,
    /// Fragmento del nombre comercial (sin tildes ni mayúsculas).
    pub texto: Option<String>,
}

/// Organización del anunciante: una que ya existe o una nueva (se reutiliza si el NIT existe).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrganizacionAnunciante {
    Existente(OrganizacionId),
    Nueva(NuevaOrganizacion),
}

/// Reseña con reportes sin atender (cola de moderación de la plataforma).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResenaReportada {
    pub resena_id: ResenaId,
    pub tenant_id: TenantId,
    pub copropiedad: String,
    pub anunciante_id: AnuncianteId,
    pub nombre_comercial: String,
    pub calificacion: Calificacion,
    pub texto: Option<String>,
    pub reportes: u32,
    pub motivos: Vec<String>,
    pub creada_en: DateTime<Utc>,
}

/// Catálogo, anunciantes y fichas en la transacción de plataforma.
pub trait RepositorioDirectorioPlataforma: Send {
    /// Todo el catálogo, por rama y nombre.
    fn categorias(&mut self) -> impl Future<Output = AppResult<Vec<Categoria>>> + Send;

    fn categoria(
        &mut self,
        codigo: &CodigoCategoria,
    ) -> impl Future<Output = AppResult<Option<Categoria>>> + Send;

    /// Crea la categoría o reemplaza sus datos.
    fn guardar_categoria(
        &mut self,
        categoria: &Categoria,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// ¿Alguna categoría tiene a esta como rama?
    fn tiene_subcategorias(
        &mut self,
        codigo: &CodigoCategoria,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Categorías del catálogo con esos códigos (las que no existen no aparecen).
    fn categorias_por_codigo(
        &mut self,
        codigos: &[CodigoCategoria],
    ) -> impl Future<Output = AppResult<Vec<Categoria>>> + Send;

    /// Devuelve la organización indicada (y si existe) o crea la nueva (reutiliza la que
    /// tenga el mismo NIT); en ambos casos le agrega el tipo `anunciante`. `None` si la
    /// existente no se encuentra.
    fn organizacion_para_anunciante(
        &mut self,
        organizacion: &OrganizacionAnunciante,
        creada_por: Option<CuentaId>,
    ) -> impl Future<Output = AppResult<Option<OrganizacionId>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si la organización ya tiene un anunciante.
    fn insertar_anunciante(
        &mut self,
        anunciante: &Anunciante,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn anunciante(
        &mut self,
        id: AnuncianteId,
    ) -> impl Future<Output = AppResult<Option<Anunciante>>> + Send;

    /// Bloquea la fila del anunciante para cambiarlo (`FOR UPDATE`).
    fn anunciante_para_actualizar(
        &mut self,
        id: AnuncianteId,
    ) -> impl Future<Output = AppResult<Option<Anunciante>>> + Send;

    fn anunciantes(
        &mut self,
        filtro: &FiltroAnunciantes,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Anunciante>>> + Send;

    /// Guarda nombre, dirección, ubicación, estado y motivo.
    fn actualizar_anunciante(
        &mut self,
        anunciante: &Anunciante,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn ficha(&mut self, id: AnuncianteId) -> impl Future<Output = AppResult<Option<Ficha>>> + Send;

    /// Crea o reemplaza la ficha y sus categorías (en su orden).
    fn guardar_ficha(&mut self, ficha: &Ficha) -> impl Future<Output = AppResult<()>> + Send;

    /// Reseñas con reportes sin atender, de todas las copropiedades.
    fn resenas_reportadas(
        &mut self,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<ResenaReportada>>> + Send;
}

/// Calificación agregada de un anunciante (todas las copropiedades, reseñas publicadas).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResumenCalificacion {
    /// Con un decimal; `None` sin reseñas.
    pub promedio: Option<Decimal>,
    pub total: u32,
}

/// Ficha tal como la ve una copropiedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FichaDirectorio {
    pub anunciante_id: AnuncianteId,
    pub nombre_comercial: String,
    pub descripcion: Option<String>,
    pub categorias: Vec<CodigoCategoria>,
    pub contacto: Contacto,
    pub horario: Option<String>,
    pub direccion: Option<String>,
    pub municipio: Option<CodigoDane>,
    /// Destacada vigente. El caso de uso la apaga si la PH no tiene el módulo `publicidad`.
    pub destacada: bool,
    /// Kilómetros (un decimal) entre el anunciante y la copropiedad, si ambos están ubicados.
    pub distancia_km: Option<Decimal>,
    pub calificacion: ResumenCalificacion,
}

/// Filtro del directorio de una copropiedad.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroDirectorio {
    pub anunciante: Option<AnuncianteId>,
    /// La categoría o su rama.
    pub categoria: Option<CodigoCategoria>,
    /// Texto libre: nombre, descripción, categorías y sinónimos (sin tildes ni mayúsculas).
    pub texto: Option<String>,
    pub solo_destacadas: bool,
}

/// Reseña de un residente o propietario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resena {
    pub id: ResenaId,
    pub anunciante_id: AnuncianteId,
    /// Autor. Nunca sale de la API: las reseñas se muestran sin autor.
    pub cuenta_id: CuentaId,
    pub calificacion: Calificacion,
    pub texto: Option<String>,
    pub estado: EstadoResena,
    pub creada_en: DateTime<Utc>,
    pub actualizada_en: DateTime<Utc>,
}

/// Reporte de una reseña por parte de un residente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReporteResena {
    pub resena_id: ResenaId,
    pub cuenta_id: CuentaId,
    pub motivo: String,
    pub creado_en: DateTime<Utc>,
}

/// Directorio y reseñas en la transacción de una copropiedad.
pub trait RepositorioDirectorio: Send {
    /// Fichas visibles en la copropiedad de la transacción (anunciante verificado, ficha
    /// publicada, cobertura que la alcanza, sin categorías restringidas), por `id`.
    fn fichas_directorio(
        &mut self,
        filtro: &FiltroDirectorio,
        ahora: DateTime<Utc>,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<FichaDirectorio>>> + Send;

    /// Categorías activas y no restringidas.
    fn categorias_directorio(&mut self) -> impl Future<Output = AppResult<Vec<Categoria>>> + Send;

    /// ¿La cuenta es propietaria o residente vigente de la copropiedad (rol derivado de la
    /// titularidad o de la residencia)?
    fn es_residente_vigente(
        &mut self,
        cuenta: CuentaId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    fn resena_de_cuenta(
        &mut self,
        anunciante: AnuncianteId,
        cuenta: CuentaId,
    ) -> impl Future<Output = AppResult<Option<Resena>>> + Send;

    fn resena(&mut self, id: ResenaId) -> impl Future<Output = AppResult<Option<Resena>>> + Send;

    /// Crea la reseña o actualiza calificación, texto y fecha de la existente.
    fn guardar_resena(&mut self, resena: &Resena) -> impl Future<Output = AppResult<()>> + Send;

    /// Reseñas publicadas del anunciante en esta copropiedad, de la más reciente a la más
    /// antigua (cursor por `id`).
    fn resenas_publicadas(
        &mut self,
        anunciante: AnuncianteId,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Resena>>> + Send;

    /// `false` si la cuenta ya la había reportado.
    fn insertar_reporte(
        &mut self,
        reporte: &ReporteResena,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Oculta la reseña (`motivo` = `Some`) o la mantiene, y marca atendidos sus reportes.
    fn moderar_resena(
        &mut self,
        id: ResenaId,
        ocultar_con_motivo: Option<&str>,
        moderador: Option<CuentaId>,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

/// Búsqueda de un proveedor hecha por un residente (chat o app).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolicitudProveedor {
    pub id: SolicitudProveedorId,
    pub persona_id: PersonaId,
    pub canal: CanalSolicitud,
    /// `wamid` en WhatsApp; una clave por petición en la app.
    pub clave_idempotencia: String,
    /// Lo que escribió (dato personal). `None` después de la purga.
    pub texto: Option<String>,
    /// Categoría del catálogo que nombra la búsqueda, si se reconoció.
    pub categoria: Option<CodigoCategoria>,
    /// Fichas sugeridas, en el orden en que se mostraron.
    pub sugeridos: Vec<AnuncianteId>,
    /// Cuándo autorizó que esos proveedores lo contacten (Ley 1581, para esta solicitud).
    pub autorizado_en: Option<DateTime<Utc>>,
    pub creada_en: DateTime<Utc>,
}

/// Enlace público del directorio de la copropiedad (uno vigente).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnlaceDirectorio {
    /// SHA-256 del token: el token solo se entrega al generarlo.
    pub token_hash: Vec<u8>,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
}

/// Búsquedas, contactos autorizados y enlace público en la transacción de una copropiedad.
pub trait RepositorioSolicitudesProveedor: Send {
    /// Inserta la solicitud. Si ya hay una con la misma clave (reintento), no inserta nada y
    /// devuelve la existente.
    fn insertar_solicitud_proveedor(
        &mut self,
        solicitud: &SolicitudProveedor,
    ) -> impl Future<Output = AppResult<SolicitudProveedor>> + Send;

    /// La solicitud, bloqueada para autorizarla (`FOR UPDATE`).
    fn solicitud_proveedor(
        &mut self,
        id: SolicitudProveedorId,
    ) -> impl Future<Output = AppResult<Option<SolicitudProveedor>>> + Send;

    fn autorizar_contacto(
        &mut self,
        id: SolicitudProveedorId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Registra que el contacto se compartió con el anunciante. `false` si ya estaba.
    fn insertar_contacto_proveedor(
        &mut self,
        solicitud: SolicitudProveedorId,
        anunciante: AnuncianteId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Anunciantes con quienes se compartió el contacto de la solicitud.
    fn contactos_de_solicitud(
        &mut self,
        solicitud: SolicitudProveedorId,
    ) -> impl Future<Output = AppResult<Vec<AnuncianteId>>> + Send;

    /// Encola un mensaje del directorio (`concepto = 'directorio'`), opcionalmente para más
    /// tarde (`disponible_en`).
    fn encolar_directorio(
        &mut self,
        evento: &EventoSaliente,
        disponible_en: Option<DateTime<Utc>>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn enlace_directorio(
        &mut self,
    ) -> impl Future<Output = AppResult<Option<EnlaceDirectorio>>> + Send;

    /// Crea el enlace o reemplaza el vigente (el anterior deja de funcionar).
    fn guardar_enlace_directorio(
        &mut self,
        enlace: &EnlaceDirectorio,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// `false` si no había enlace.
    fn borrar_enlace_directorio(&mut self) -> impl Future<Output = AppResult<bool>> + Send;

    /// Número visible de la línea de WhatsApp de los enlaces `wa.me` (`None` sin líneas).
    fn numero_whatsapp_publico(&mut self)
    -> impl Future<Output = AppResult<Option<String>>> + Send;
}

/// Purga de las búsquedas en todas las copropiedades (función `SECURITY DEFINER`).
pub trait PurgaDirectorio: Send + Sync {
    /// Borra el texto de las búsquedas con más de `dias` días. Devuelve cuántas purgó.
    fn purgar_solicitudes_proveedor(
        &self,
        dias: u32,
    ) -> impl Future<Output = AppResult<u64>> + Send;
}
