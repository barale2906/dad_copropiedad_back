//! Puertos del módulo de copropiedades y bienes privados (plan 03). Todos se usan dentro de la
//! transacción de una copropiedad ([`crate::puertos::UnidadDeTrabajo`]).
//!
//! - [`RepositorioCopropiedad`]: ficha de la PH, cargos vigentes y puntos de atención.
//! - [`RepositorioBienesPrivados`]: agrupaciones, bienes privados y usos exclusivos.
//! - [`RepositorioCoeficientes`]: cuadros de coeficientes versionados.
//! - [`RepositorioPersonas`]: personas registradas en la copropiedad.
//! - [`RepositorioTitulares`]: titularidades de los bienes privados.
//! - [`RepositorioImportaciones`]: importaciones masivas.
//! - [`LectorPlanillas`](super::planillas::LectorPlanillas): lee CSV/XLSX (adaptador `infra-planillas`).

use std::{collections::BTreeMap, future::Future};

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AgrupacionId, ArchivoId, BienPrivadoId, CodigoAgrupacion, CodigoBienPrivado, CodigoDane,
    Coeficiente, CuadroCoeficientesId, CuentaId, DireccionCorreo, Documento, EstadoCopropiedad,
    EstadoCuadro, EstadoImportacion, FormaCopropiedad, Horario, ImportacionId,
    ModuloContribucionId, Nit, NumeroBienPrivado, PersonaId, PlantillaNomenclatura, Porcentaje,
    PuntoAtencionId, Telefono, TipoAgrupacion, TipoBienPrivado, TipoCopropiedad, TipoCuadro,
    TipoDocumento, TipoImportacion, TipoPersona, TipoPuntoAtencion, TipoTitularidad,
    TipoUsoExclusivo, TitularidadId, UsoBienPrivado, UsoExclusivoId, importacion::ErrorFila,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::{AppResult, Pagina, SolicitudPagina};

// ---------------------------------------------------------------------------------------------
// Copropiedad
// ---------------------------------------------------------------------------------------------

/// Datos de la copropiedad (fila de `copropiedades`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FichaCopropiedad {
    pub nombre: String,
    pub razon_social: Option<String>,
    pub nit: Option<Nit>,
    pub tipo: TipoCopropiedad,
    pub forma: FormaCopropiedad,
    pub direccion: Option<String>,
    pub municipio_dane: Option<CodigoDane>,
    pub departamento: Option<String>,
    pub barrio: Option<String>,
    pub latitud: Option<Decimal>,
    pub longitud: Option<Decimal>,
    pub estrato: Option<u8>,
    pub telefono: Option<Telefono>,
    pub correo: Option<DireccionCorreo>,
    pub sitio_web: Option<String>,
    pub escritura_numero: Option<String>,
    pub notaria: Option<String>,
    pub escritura_fecha: Option<NaiveDate>,
    pub matricula_matriz: Option<String>,
    pub reglamento_archivo_id: Option<ArchivoId>,
    pub certificado_archivo_id: Option<ArchivoId>,
    pub zona_horaria: String,
    pub estado: EstadoCopropiedad,
    pub plantilla_codigo: PlantillaNomenclatura,
}

/// Datos con que la plataforma da de alta una copropiedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AltaCopropiedad {
    pub nombre: String,
    pub nit: Option<Nit>,
    pub tipo: TipoCopropiedad,
    pub forma: FormaCopropiedad,
}

impl AltaCopropiedad {
    /// Conjunto residencial sin NIT (los datos se completan después en la ficha).
    pub fn con_nombre(nombre: &str) -> Self {
        Self {
            nombre: nombre.to_owned(),
            nit: None,
            tipo: TipoCopropiedad::Residencial,
            forma: FormaCopropiedad::Conjunto,
        }
    }
}

/// Cargo vigente (asignación de un rol de órgano o de administración).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoVigente {
    /// Código del rol plantilla (`administrador`, `revisor_fiscal`…).
    pub rol: String,
    pub rol_nombre: String,
    pub cuenta_id: CuentaId,
    pub nombre: String,
    /// Empresa por la que ejerce el cargo (contrato de servicio), si aplica.
    pub organizacion: Option<String>,
    pub vigente_desde: DateTime<Utc>,
}

/// Punto de atención con su horario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PuntoAtencion {
    pub id: PuntoAtencionId,
    pub tipo: TipoPuntoAtencion,
    pub nombre: String,
    pub ubicacion: Option<String>,
    pub telefono: Option<Telefono>,
    pub horario: Horario,
}

/// Ficha de la copropiedad, cargos y puntos de atención.
pub trait RepositorioCopropiedad: Send {
    /// Datos de la copropiedad de la transacción.
    fn ficha(&mut self) -> impl Future<Output = AppResult<Option<FichaCopropiedad>>> + Send;

    /// Reemplaza los datos de la copropiedad.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el NIT ya lo usa otra copropiedad;
    /// [`crate::AppError::NoEncontrado`] si un archivo referido no existe en ella.
    fn guardar_ficha(
        &mut self,
        ficha: &FichaCopropiedad,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Cargos vigentes (roles de órgano o de administración asignados).
    fn cargos_vigentes(&mut self) -> impl Future<Output = AppResult<Vec<CargoVigente>>> + Send;

    /// Bienes privados activos sin contar parqueaderos ni depósitos (art. 53).
    fn bienes_para_consejo(&mut self) -> impl Future<Output = AppResult<u64>> + Send;

    /// Puntos de atención de la copropiedad.
    fn puntos_atencion(&mut self) -> impl Future<Output = AppResult<Vec<PuntoAtencion>>> + Send;

    /// Guarda un punto de atención nuevo.
    fn insertar_punto_atencion(
        &mut self,
        punto: &PuntoAtencion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Reemplaza franjas y excepciones del punto. `false` si el punto no existe.
    fn reemplazar_horario(
        &mut self,
        punto: PuntoAtencionId,
        horario: &Horario,
    ) -> impl Future<Output = AppResult<bool>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Agrupaciones, bienes privados y usos exclusivos
// ---------------------------------------------------------------------------------------------

/// Edificio, etapa, bloque… de la copropiedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agrupacion {
    pub id: AgrupacionId,
    pub tipo: TipoAgrupacion,
    pub nombre: String,
    pub codigo: CodigoAgrupacion,
    pub padre_id: Option<AgrupacionId>,
    pub orden: i32,
}

/// Bien privado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BienPrivado {
    pub id: BienPrivadoId,
    pub codigo: CodigoBienPrivado,
    pub tipo: TipoBienPrivado,
    pub agrupacion_id: Option<AgrupacionId>,
    pub numero: Option<NumeroBienPrivado>,
    pub piso: Option<i16>,
    pub area_privada_m2: Option<Decimal>,
    pub matricula_inmobiliaria: Option<String>,
    pub cedula_catastral: Option<String>,
    pub uso: UsoBienPrivado,
    pub activo: bool,
    /// Bien al que está vinculado como anexo (parqueadero o depósito con folio propio).
    pub bien_principal_id: Option<BienPrivadoId>,
    /// Coeficiente del cuadro vigente (solo lectura: lo escribe la activación de un cuadro).
    pub coeficiente: Option<Coeficiente>,
}

/// Filtros del listado de bienes privados.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroBienes {
    /// Fragmento del código, sin importar tildes ni mayúsculas.
    pub texto: Option<String>,
    pub agrupacion_id: Option<AgrupacionId>,
    pub tipo: Option<TipoBienPrivado>,
    pub incluir_inactivos: bool,
}

/// Bien común asignado en uso exclusivo a un bien privado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsoExclusivo {
    pub id: UsoExclusivoId,
    pub bien_privado_id: BienPrivadoId,
    pub tipo: TipoUsoExclusivo,
    /// Identificación del bien común ("Parqueadero 45").
    pub codigo: String,
    pub descripcion: Option<String>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
}

/// Persistencia de bienes privados, agrupaciones y usos exclusivos.
pub trait RepositorioBienesPrivados: Send {
    /// Todas las agrupaciones, ordenadas por `orden` y nombre.
    fn agrupaciones(&mut self) -> impl Future<Output = AppResult<Vec<Agrupacion>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay una con ese código bajo el mismo padre.
    fn insertar_agrupacion(
        &mut self,
        agrupacion: &Agrupacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Bien privado por su id.
    fn bien(
        &mut self,
        id: BienPrivadoId,
    ) -> impl Future<Output = AppResult<Option<BienPrivado>>> + Send;

    /// Todos los bienes privados (activos e inactivos), ordenados por código.
    fn todos_los_bienes(&mut self) -> impl Future<Output = AppResult<Vec<BienPrivado>>> + Send;

    /// Bienes privados que cumplen el filtro, paginados por cursor.
    fn listar_bienes(
        &mut self,
        filtro: &FiltroBienes,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<BienPrivado>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el código ya existe en la copropiedad.
    fn insertar_bien(&mut self, bien: &BienPrivado) -> impl Future<Output = AppResult<()>> + Send;

    /// Actualiza todo salvo el coeficiente.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el código nuevo ya existe.
    fn actualizar_bien(&mut self, bien: &BienPrivado)
    -> impl Future<Output = AppResult<()>> + Send;

    /// Bienes vinculados como anexos de `principal`.
    fn anexos(
        &mut self,
        principal: BienPrivadoId,
    ) -> impl Future<Output = AppResult<Vec<BienPrivado>>> + Send;

    /// Usos exclusivos de bienes comunes asignados al bien.
    fn usos_exclusivos(
        &mut self,
        bien: BienPrivadoId,
    ) -> impl Future<Output = AppResult<Vec<UsoExclusivo>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ese bien común ya está asignado.
    fn insertar_uso_exclusivo(
        &mut self,
        uso: &UsoExclusivo,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Termina la asignación si seguía vigente. `false` si no existe para ese bien.
    fn terminar_uso_exclusivo(
        &mut self,
        bien: BienPrivadoId,
        id: UsoExclusivoId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Cuadros de coeficientes
// ---------------------------------------------------------------------------------------------

/// Cuadro de coeficientes (sin sus filas).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuadroCoeficientes {
    pub id: CuadroCoeficientesId,
    pub version: u32,
    pub tipo: TipoCuadro,
    /// Módulo de contribución (solo en los cuadros de tipo `modulo_contribucion`).
    pub modulo_id: Option<ModuloContribucionId>,
    pub estado: EstadoCuadro,
    pub descripcion: Option<String>,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub vigente_desde: Option<DateTime<Utc>>,
    pub aprobado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
}

/// Persistencia de los cuadros de coeficientes (ADR 0006).
pub trait RepositorioCoeficientes: Send {
    /// Cuadros de la copropiedad, del más reciente al más antiguo.
    fn cuadros(&mut self) -> impl Future<Output = AppResult<Vec<CuadroCoeficientes>>> + Send;

    /// Cuadro de coeficientes por su id.
    fn cuadro(
        &mut self,
        id: CuadroCoeficientesId,
    ) -> impl Future<Output = AppResult<Option<CuadroCoeficientes>>> + Send;

    /// Cuadro vigente de la copropiedad (`None`) o del módulo de contribución.
    fn cuadro_vigente(
        &mut self,
        modulo: Option<ModuloContribucionId>,
    ) -> impl Future<Output = AppResult<Option<CuadroCoeficientes>>> + Send;

    /// Última versión usada en los cuadros de la copropiedad (`None`) o del módulo (0 si no
    /// hay cuadros).
    fn ultima_version(
        &mut self,
        modulo: Option<ModuloContribucionId>,
    ) -> impl Future<Output = AppResult<u32>> + Send;

    /// ¿Existe el módulo de contribución y está activo?
    fn modulo_contribucion_activo(
        &mut self,
        modulo: ModuloContribucionId,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Guarda un cuadro nuevo en borrador.
    fn insertar_cuadro(
        &mut self,
        cuadro: &CuadroCoeficientes,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Coeficiente de cada bien en el cuadro.
    fn coeficientes(
        &mut self,
        cuadro: CuadroCoeficientesId,
    ) -> impl Future<Output = AppResult<BTreeMap<BienPrivadoId, Coeficiente>>> + Send;

    /// Reemplaza todas las filas de un borrador.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::NoEncontrado`] si algún bien privado no existe en la copropiedad.
    fn reemplazar_coeficientes(
        &mut self,
        cuadro: CuadroCoeficientesId,
        coeficientes: &BTreeMap<BienPrivadoId, Coeficiente>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Pasa `anterior` (si hay) a histórico y `cuadro` a vigente; los bienes privados toman su
    /// coeficiente en la misma transacción.
    fn activar(
        &mut self,
        cuadro: CuadroCoeficientesId,
        anterior: Option<CuadroCoeficientesId>,
        ahora: DateTime<Utc>,
        aprobado_por: Option<CuentaId>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Reuniones en estado `instalada` (con votaciones posibles en curso).
    fn reuniones_instaladas(&mut self) -> impl Future<Output = AppResult<u64>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Personas y titularidades
// ---------------------------------------------------------------------------------------------

/// Persona (natural o jurídica) registrada en la copropiedad. Datos personales (Ley 1581).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Persona {
    pub id: PersonaId,
    pub tipo: TipoPersona,
    pub nombre: String,
    pub documento: Option<Documento>,
    pub telefono: Option<Telefono>,
    pub correo: Option<DireccionCorreo>,
    /// Cuenta de la plataforma con que inicia sesión, si la tiene.
    pub cuenta_id: Option<CuentaId>,
    pub tipo_documento: Option<TipoDocumento>,
    /// Dato sensible: identifica a los menores de edad.
    pub fecha_nacimiento: Option<NaiveDate>,
    pub contacto_emergencia_nombre: Option<String>,
    pub contacto_emergencia_telefono: Option<Telefono>,
    /// Dato sensible (salud); solo con consentimiento.
    pub movilidad_reducida: bool,
}

impl Persona {
    /// Persona con los datos básicos y el resto vacío.
    pub fn basica(
        id: PersonaId,
        tipo: TipoPersona,
        nombre: String,
        documento: Option<Documento>,
    ) -> Self {
        Self {
            id,
            tipo,
            nombre,
            documento,
            telefono: None,
            correo: None,
            cuenta_id: None,
            tipo_documento: None,
            fecha_nacimiento: None,
            contacto_emergencia_nombre: None,
            contacto_emergencia_telefono: None,
            movilidad_reducida: false,
        }
    }
}

/// Titularidad de una persona sobre un bien privado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Titularidad {
    pub id: TitularidadId,
    pub bien_privado_id: BienPrivadoId,
    pub persona_id: PersonaId,
    pub tipo: TipoTitularidad,
    pub porcentaje: Porcentaje,
    pub es_representante: bool,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
    pub soporte_archivo_id: Option<ArchivoId>,
}

/// Persistencia de las personas del censo.
pub trait RepositorioPersonas: Send {
    /// Persona por su id.
    fn persona(&mut self, id: PersonaId)
    -> impl Future<Output = AppResult<Option<Persona>>> + Send;

    /// Persona por su documento de identidad.
    fn persona_por_documento(
        &mut self,
        documento: &Documento,
    ) -> impl Future<Output = AppResult<Option<Persona>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el documento o la cuenta ya están registrados.
    fn insertar_persona(&mut self, persona: &Persona)
    -> impl Future<Output = AppResult<()>> + Send;

    /// Vincula la persona con una cuenta de la plataforma.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si la cuenta ya está vinculada a otra persona;
    /// [`crate::AppError::NoEncontrado`] si la cuenta no existe.
    fn vincular_cuenta(
        &mut self,
        persona: PersonaId,
        cuenta: CuentaId,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Reemplaza los datos de la persona (no su cuenta).
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el documento ya lo tiene otra persona.
    fn actualizar_persona(
        &mut self,
        persona: &Persona,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Personas de la copropiedad, opcionalmente filtradas por un fragmento del nombre (sin
    /// tildes ni mayúsculas) o del documento.
    fn listar_personas(
        &mut self,
        texto: Option<&str>,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Persona>>> + Send;
}

/// Persistencia de las titularidades de los bienes privados.
pub trait RepositorioTitulares: Send {
    /// Titulares vigentes del bien privado.
    fn titulares_vigentes(
        &mut self,
        bien: BienPrivadoId,
    ) -> impl Future<Output = AppResult<Vec<Titularidad>>> + Send;

    /// Historial completo, más recientes primero.
    fn historial_titulares(
        &mut self,
        bien: BienPrivadoId,
    ) -> impl Future<Output = AppResult<Vec<Titularidad>>> + Send;

    /// Guarda una titularidad (el rol `propietario` se deriva de ella).
    fn insertar_titularidad(
        &mut self,
        titularidad: &Titularidad,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Termina los titulares vigentes del bien (`vigente_hasta = ahora`) y devuelve sus ids.
    fn terminar_titulares(
        &mut self,
        bien: BienPrivadoId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<TitularidadId>>> + Send;

    /// Deja como único representante vigente a `persona`.
    fn designar_representante(
        &mut self,
        bien: BienPrivadoId,
        persona: PersonaId,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Bienes privados de los que la cuenta es titular vigente.
    fn bienes_de_cuenta(
        &mut self,
        cuenta: CuentaId,
    ) -> impl Future<Output = AppResult<Vec<BienPrivadoId>>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Importaciones
// ---------------------------------------------------------------------------------------------

/// Resumen de lo que hace (o hizo) una importación de bienes privados.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumenImportacion {
    pub bienes_privados: u32,
    pub agrupaciones_nuevas: u32,
    pub titulares: u32,
    pub personas_nuevas: u32,
    pub coeficientes: u32,
    /// Borrador creado con los coeficientes del archivo.
    pub cuadro_borrador_id: Option<CuadroCoeficientesId>,
}

/// Importación registrada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Importacion {
    pub id: ImportacionId,
    pub tipo: TipoImportacion,
    pub archivo_id: ArchivoId,
    pub estado: EstadoImportacion,
    pub total_filas: u32,
    pub filas_validas: u32,
    pub errores: Vec<ErrorFila>,
    pub resumen: ResumenImportacion,
    pub creada_por: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
    pub aplicada_en: Option<DateTime<Utc>>,
}

/// Persistencia de las importaciones masivas (ADR 0006).
pub trait RepositorioImportaciones: Send {
    /// Guarda una importación validada en vista previa.
    fn insertar_importacion(
        &mut self,
        importacion: &Importacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Importación por su id.
    fn importacion(
        &mut self,
        id: ImportacionId,
    ) -> impl Future<Output = AppResult<Option<Importacion>>> + Send;

    /// Marca la importación como aplicada con su resumen.
    fn marcar_aplicada(
        &mut self,
        id: ImportacionId,
        resumen: &ResumenImportacion,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;
}
