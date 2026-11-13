//! Puertos del núcleo contable (plan 12, ADR 0017). Traits pequeños por agregado; los
//! implementa la transacción de la entidad (`UnidadDeTrabajo`), salvo
//! [`CatalogoContablePlataforma`], que vive en la transacción de plataforma.
//!
//! El tercero contable es la persona del tenant ([`super::RepositorioPersonas`]); aquí solo se
//! agregan sus datos fiscales.

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, CentroCostoId, ComprobanteId, CuentaContableId, CuentaId, NotaEstadosId,
    OrganizacionId, PeriodoContableId, PersonaId, PlantillaContableId, ReaperturaPeriodoId,
    ReglaContabilizacionId, RubroEstadoFinancieroId, TipoComprobanteId,
    contabilidad::{
        CodigoCuenta, EstadoComprobante, EstadoPeriodo, EstadoReapertura, LineaAsiento,
        MarcoTecnico, Naturaleza, PeriodoContable, RegimenIva, ReglaContable, ReglasCuenta,
        ResponsabilidadFiscal,
    },
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::AppResult;

// ---------------------------------------------------------------------------------------------
// Plan de cuentas y centros de costo
// ---------------------------------------------------------------------------------------------

/// Cuenta del plan de cuentas de la entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuentaContable {
    pub id: CuentaContableId,
    pub codigo: CodigoCuenta,
    pub nombre: String,
    pub naturaleza: Naturaleza,
    pub acepta_movimiento: bool,
    pub exige_tercero: bool,
    pub exige_centro_costo: bool,
    pub activa: bool,
}

impl CuentaContable {
    /// Lo que el asiento necesita para validarse contra esta cuenta.
    pub fn reglas(&self) -> ReglasCuenta {
        ReglasCuenta {
            codigo: self.codigo.as_str().to_owned(),
            acepta_movimiento: self.acepta_movimiento,
            activa: self.activa,
            exige_tercero: self.exige_tercero,
            exige_centro_costo: self.exige_centro_costo,
        }
    }
}

/// Centro de costo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CentroCosto {
    pub id: CentroCostoId,
    pub codigo: String,
    pub nombre: String,
    pub activo: bool,
}

/// Plan de cuentas y centros de costo de la entidad.
pub trait RepositorioPlanCuentas: Send {
    /// Todas las cuentas, por código.
    fn cuentas(&mut self) -> impl Future<Output = AppResult<Vec<CuentaContable>>> + Send;

    /// Cuenta por código.
    fn cuenta_por_codigo(
        &mut self,
        codigo: &CodigoCuenta,
    ) -> impl Future<Output = AppResult<Option<CuentaContable>>> + Send;

    /// Cuentas por id (las que existan).
    fn cuentas_por_id(
        &mut self,
        ids: &[CuentaContableId],
    ) -> impl Future<Output = AppResult<Vec<CuentaContable>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el código ya existe.
    fn insertar_cuenta(
        &mut self,
        cuenta: &CuentaContable,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Actualiza nombre, banderas y estado (el código y la naturaleza no cambian).
    fn actualizar_cuenta(
        &mut self,
        cuenta: &CuentaContable,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// ¿La cuenta tiene líneas en algún comprobante?
    fn cuenta_con_movimientos(
        &mut self,
        id: CuentaContableId,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Centros de costo, por código.
    fn centros_costo(&mut self) -> impl Future<Output = AppResult<Vec<CentroCosto>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el código ya existe.
    fn insertar_centro_costo(
        &mut self,
        centro: &CentroCosto,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Actualiza nombre y estado.
    fn actualizar_centro_costo(
        &mut self,
        centro: &CentroCosto,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Configuración, tipos de comprobante, reglas, rubros, notas y terceros
// ---------------------------------------------------------------------------------------------

/// Versión de la configuración contable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguracionContable {
    pub version: u32,
    pub marco_tecnico: MarcoTecnico,
    /// Cuenta de excedente o déficit del ejercicio para el cierre anual.
    pub cuenta_resultado: Option<CuentaContableId>,
    pub soporte_archivo_id: Option<ArchivoId>,
    /// Plantilla (nombre y versión) de la que partió, informativo.
    pub plantilla_origen: Option<String>,
    /// Desde cuándo se contabiliza con esta configuración.
    pub vigente_desde: NaiveDate,
    pub creada_por: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
}

/// Tipo de comprobante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TipoComprobante {
    pub id: TipoComprobanteId,
    pub codigo: String,
    pub nombre: String,
    /// `false` = solo los usa el sistema (cierre, apertura, documentos de los módulos).
    pub manual: bool,
    pub activo: bool,
}

/// Versión de una regla de contabilización.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReglaVersionada {
    pub id: ReglaContabilizacionId,
    pub version: u32,
    pub tipo_comprobante_id: TipoComprobanteId,
    pub descripcion: Option<String>,
    pub vigente_desde: NaiveDate,
    pub regla: ReglaContable,
    pub creada_en: DateTime<Utc>,
}

/// Rubro de un estado financiero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RubroEstado {
    pub id: RubroEstadoFinancieroId,
    /// `situacion_financiera` o `resultados`.
    pub estado: String,
    pub codigo: String,
    pub nombre: String,
    pub orden: u16,
    pub prefijos: Vec<String>,
}

/// Versión de una nota a los estados financieros.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotaEstados {
    pub id: NotaEstadosId,
    pub anio: i32,
    pub numero: u16,
    pub version: u32,
    pub titulo: String,
    pub texto: String,
    pub rubro_id: Option<RubroEstadoFinancieroId>,
    pub creada_por: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
}

/// Datos fiscales del tercero (persona del tenant). La dirección y el correo son datos
/// personales.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DatosFiscales {
    /// Ficha global de la organización, si el tercero es una de ellas.
    pub organizacion_id: Option<OrganizacionId>,
    pub regimen_iva: Option<RegimenIva>,
    pub responsabilidades: Vec<ResponsabilidadFiscal>,
    /// Código CIIU de cuatro dígitos.
    pub actividad_ciiu: Option<String>,
    pub direccion_fiscal: Option<String>,
    pub municipio_fiscal_dane: Option<String>,
    pub correo_facturacion: Option<String>,
}

/// Configuración y catálogos propios de la entidad.
pub trait RepositorioConfiguracionContable: Send {
    /// Versión vigente (la de mayor número).
    fn configuracion_vigente(
        &mut self,
    ) -> impl Future<Output = AppResult<Option<ConfiguracionContable>>> + Send;

    /// Fecha desde la que se contabiliza: el menor `vigente_desde` de todas las versiones.
    fn inicio_contabilidad(&mut self) -> impl Future<Output = AppResult<Option<NaiveDate>>> + Send;

    /// Guarda una versión nueva (el número lo trae `c`).
    fn insertar_configuracion(
        &mut self,
        c: &ConfiguracionContable,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Tipos de comprobante, por código.
    fn tipos_comprobante(&mut self)
    -> impl Future<Output = AppResult<Vec<TipoComprobante>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el código ya existe.
    fn insertar_tipo_comprobante(
        &mut self,
        t: &TipoComprobante,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Actualiza nombre, uso manual y estado.
    fn actualizar_tipo_comprobante(
        &mut self,
        t: &TipoComprobante,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Última versión de cada regla (por evento).
    fn reglas(&mut self) -> impl Future<Output = AppResult<Vec<ReglaVersionada>>> + Send;

    /// Versión vigente del evento en la fecha (la de mayor versión con `vigente_desde <= fecha`).
    fn regla_vigente(
        &mut self,
        evento: &str,
        fecha: NaiveDate,
    ) -> impl Future<Output = AppResult<Option<ReglaVersionada>>> + Send;

    /// Mayor versión del evento (0 si no hay).
    fn ultima_version_regla(&mut self, evento: &str)
    -> impl Future<Output = AppResult<u32>> + Send;

    /// Guarda una versión nueva con sus líneas.
    fn insertar_regla(
        &mut self,
        r: &ReglaVersionada,
        creada_por: Option<CuentaId>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Rubros de los estados financieros, por estado y orden.
    fn rubros(&mut self) -> impl Future<Output = AppResult<Vec<RubroEstado>>> + Send;

    /// Reemplaza los rubros (los de notas existentes se conservan si el código sigue).
    fn reemplazar_rubros(
        &mut self,
        rubros: &[RubroEstado],
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Última versión de cada nota del año.
    fn notas(&mut self, anio: i32) -> impl Future<Output = AppResult<Vec<NotaEstados>>> + Send;

    /// Mayor versión de la nota (0 si no hay).
    fn ultima_version_nota(
        &mut self,
        anio: i32,
        numero: u16,
    ) -> impl Future<Output = AppResult<u32>> + Send;

    /// Guarda una versión nueva de la nota.
    fn insertar_nota(&mut self, n: &NotaEstados) -> impl Future<Output = AppResult<()>> + Send;

    /// Datos fiscales de la persona (`None` si la persona no existe).
    fn datos_fiscales(
        &mut self,
        persona: PersonaId,
    ) -> impl Future<Output = AppResult<Option<DatosFiscales>>> + Send;

    /// Reemplaza los datos fiscales de la persona.
    fn guardar_datos_fiscales(
        &mut self,
        persona: PersonaId,
        datos: &DatosFiscales,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Plantilla publicada que la cuenta puede aplicar (estándar, o de su firma contable):
    /// nombre, versión y contenido JSON.
    fn plantilla_para_aplicar(
        &mut self,
        plantilla: PlantillaContableId,
        cuenta: Option<CuentaId>,
    ) -> impl Future<Output = AppResult<Option<(String, u32, serde_json::Value)>>> + Send;

    /// Plantillas que la cuenta puede aplicar.
    fn plantillas_disponibles(
        &mut self,
        cuenta: Option<CuentaId>,
    ) -> impl Future<Output = AppResult<Vec<PlantillaResumen>>> + Send;

    /// Catálogo PUC de la plataforma (solo lectura), por código; `prefijo` filtra.
    fn catalogo_puc(
        &mut self,
        prefijo: Option<&str>,
    ) -> impl Future<Output = AppResult<Vec<CuentaPuc>>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Periodos, comprobantes y saldos
// ---------------------------------------------------------------------------------------------

/// Periodo contable de la entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Periodo {
    pub id: PeriodoContableId,
    pub periodo: PeriodoContable,
    pub estado: EstadoPeriodo,
    pub cerrado_por: Option<CuentaId>,
    pub cerrado_en: Option<DateTime<Utc>>,
}

/// Solicitud de reapertura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reapertura {
    pub id: ReaperturaPeriodoId,
    pub periodo_id: PeriodoContableId,
    pub motivo: String,
    pub alcance: Option<String>,
    pub solicitada_por: CuentaId,
    pub solicitada_en: DateTime<Utc>,
    pub estado: EstadoReapertura,
    pub decidida_por: Option<CuentaId>,
    pub decidida_en: Option<DateTime<Utc>>,
}

/// Documento origen de un comprobante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origen {
    /// Tipo del documento (`ventas.factura`, `contabilidad.cierre_anual`…).
    pub tipo: String,
    pub id: Uuid,
}

/// Comprobante con sus líneas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comprobante {
    pub id: ComprobanteId,
    pub tipo_comprobante_id: TipoComprobanteId,
    pub tipo_codigo: String,
    pub periodo_id: PeriodoContableId,
    pub periodo: PeriodoContable,
    /// Se asigna al contabilizar.
    pub numero: Option<u32>,
    pub fecha: NaiveDate,
    pub descripcion: String,
    pub estado: EstadoComprobante,
    pub origen: Option<Origen>,
    pub regla_id: Option<ReglaContabilizacionId>,
    pub revierte_a: Option<ComprobanteId>,
    pub motivo_reverso: Option<String>,
    /// El comprobante que revirtió a este, si lo hay.
    pub revertido_por: Option<ComprobanteId>,
    pub lineas: Vec<LineaAsiento>,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
    pub contabilizado_por: Option<CuentaId>,
    pub contabilizado_en: Option<DateTime<Utc>>,
}

/// Filtro del listado de comprobantes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroComprobantes {
    pub desde: Option<NaiveDate>,
    pub hasta: Option<NaiveDate>,
    pub tipo_comprobante_id: Option<TipoComprobanteId>,
    pub estado: Option<EstadoComprobante>,
}

/// Saldo de una cuenta (por tercero y centro de costo si se agrupa así) entre dos fechas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaSaldo {
    pub cuenta_id: CuentaContableId,
    pub codigo: CodigoCuenta,
    pub nombre: String,
    pub naturaleza: Naturaleza,
    pub persona_id: Option<PersonaId>,
    pub centro_costo_id: Option<CentroCostoId>,
    /// Débitos − créditos antes de `desde`.
    pub saldo_inicial: Decimal,
    pub debitos: Decimal,
    pub creditos: Decimal,
}

impl FilaSaldo {
    /// Saldo inicial + débitos − créditos.
    pub fn saldo_final(&self) -> Decimal {
        self.saldo_inicial + self.debitos - self.creditos
    }
}

/// Cómo se agrupan los saldos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AgrupacionSaldos {
    pub por_tercero: bool,
    pub por_centro_costo: bool,
    /// Sin los comprobantes del periodo 13 (el estado de resultados no muestra el cierre).
    pub sin_cierre_anual: bool,
}

/// Línea contabilizada de una cuenta (libro mayor y auxiliares).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Movimiento {
    pub comprobante_id: ComprobanteId,
    pub tipo_codigo: String,
    pub numero: u32,
    pub fecha: NaiveDate,
    pub descripcion: String,
    pub cuenta_id: CuentaContableId,
    pub persona_id: Option<PersonaId>,
    pub centro_costo_id: Option<CentroCostoId>,
    pub debito: Decimal,
    pub credito: Decimal,
}

/// Periodos, comprobantes y consultas de saldos.
pub trait RepositorioComprobantes: Send {
    /// Periodos del año (o todos), por año y mes.
    fn periodos(
        &mut self,
        anio: Option<i32>,
    ) -> impl Future<Output = AppResult<Vec<Periodo>>> + Send;

    /// Periodo por id.
    fn periodo(
        &mut self,
        id: PeriodoContableId,
    ) -> impl Future<Output = AppResult<Option<Periodo>>> + Send;

    /// El periodo; lo crea abierto si no existe.
    fn periodo_o_crear(
        &mut self,
        periodo: PeriodoContable,
    ) -> impl Future<Output = AppResult<Periodo>> + Send;

    /// Cambia el estado (y quién y cuándo lo cerró).
    fn cambiar_estado_periodo(&mut self, p: &Periodo)
    -> impl Future<Output = AppResult<()>> + Send;

    /// Reaperturas de un periodo, más recientes primero.
    fn reaperturas(
        &mut self,
        periodo: PeriodoContableId,
    ) -> impl Future<Output = AppResult<Vec<Reapertura>>> + Send;

    /// Reapertura por id.
    fn reapertura(
        &mut self,
        id: ReaperturaPeriodoId,
    ) -> impl Future<Output = AppResult<Option<Reapertura>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay una pendiente para el periodo.
    fn insertar_reapertura(&mut self, r: &Reapertura)
    -> impl Future<Output = AppResult<()>> + Send;

    /// Registra la decisión.
    fn decidir_reapertura(&mut self, r: &Reapertura) -> impl Future<Output = AppResult<()>> + Send;

    /// Siguiente número del tipo de comprobante en el año (atómico en la transacción).
    fn siguiente_numero(
        &mut self,
        tipo_codigo: &str,
        anio: i32,
    ) -> impl Future<Output = AppResult<u32>> + Send;

    /// Guarda un comprobante en borrador con sus líneas.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya existe uno del mismo origen.
    fn insertar_comprobante(
        &mut self,
        c: &Comprobante,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Reemplaza encabezado y líneas de un borrador.
    fn actualizar_borrador(
        &mut self,
        c: &Comprobante,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Borra un borrador.
    fn eliminar_borrador(
        &mut self,
        id: ComprobanteId,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Marca el borrador como contabilizado con su número (la base de datos verifica periodo,
    /// cuentas y partida doble).
    fn marcar_contabilizado(
        &mut self,
        id: ComprobanteId,
        numero: u32,
        por: Option<CuentaId>,
        en: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Comprobante por id, con sus líneas.
    fn comprobante(
        &mut self,
        id: ComprobanteId,
    ) -> impl Future<Output = AppResult<Option<Comprobante>>> + Send;

    /// Comprobante de un documento origen.
    fn comprobante_por_origen(
        &mut self,
        origen: &Origen,
    ) -> impl Future<Output = AppResult<Option<ComprobanteId>>> + Send;

    /// Comprobantes por fecha, sin líneas, a lo sumo `limite`.
    fn comprobantes(
        &mut self,
        filtro: &FiltroComprobantes,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<Comprobante>>> + Send;

    /// Comprobantes contabilizados entre dos fechas, con sus líneas (libro diario).
    fn diario(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<Vec<Comprobante>>> + Send;

    /// Saldos de las cuentas con movimientos contabilizados hasta `hasta`; `saldo_inicial` es
    /// lo anterior a `desde`.
    fn saldos(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
        agrupacion: AgrupacionSaldos,
    ) -> impl Future<Output = AppResult<Vec<FilaSaldo>>> + Send;

    /// Líneas contabilizadas de la cuenta (y del tercero, si se da) entre dos fechas.
    fn movimientos(
        &mut self,
        cuenta: CuentaContableId,
        persona: Option<PersonaId>,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<Vec<Movimiento>>> + Send;

    /// ¿Hay un cierre anual contabilizado del año que no esté revertido?
    fn cierre_anual_vigente(
        &mut self,
        anio: i32,
    ) -> impl Future<Output = AppResult<Option<ComprobanteId>>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Catálogo de plataforma: PUC y plantillas
// ---------------------------------------------------------------------------------------------

/// Cuenta del catálogo PUC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuentaPuc {
    pub codigo: CodigoCuenta,
    pub nombre: String,
    pub naturaleza: Naturaleza,
}

/// Ámbito de una plantilla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbitoPlantilla {
    /// Estándar de la plataforma.
    Plataforma,
    /// Propia de una firma contable.
    Organizacion(OrganizacionId),
}

/// Estado de una plantilla.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstadoPlantilla {
    Borrador,
    Publicada,
    Retirada,
}

domain::enum_texto!(EstadoPlantilla {
    Borrador => "borrador",
    Publicada => "publicada",
    Retirada => "retirada",
});

/// Plantilla contable completa.
#[derive(Debug, Clone, PartialEq)]
pub struct PlantillaGuardada {
    pub id: PlantillaContableId,
    pub ambito: AmbitoPlantilla,
    pub nombre: String,
    /// Tipo de empresa al que apunta (`propiedad_horizontal`, `gimnasio`…).
    pub tipo_empresa: String,
    pub version: u32,
    pub estado: EstadoPlantilla,
    pub contenido: serde_json::Value,
    pub autor: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
    pub publicada_en: Option<DateTime<Utc>>,
}

/// Plantilla en un listado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlantillaResumen {
    pub id: PlantillaContableId,
    /// `plataforma` u `organizacion`.
    pub ambito: String,
    /// Razón social de la firma contable, si es de una.
    pub organizacion: Option<String>,
    pub nombre: String,
    pub tipo_empresa: String,
    pub version: u32,
    pub estado: EstadoPlantilla,
    pub publicada_en: Option<DateTime<Utc>>,
}

/// PUC y plantillas, sobre la transacción de plataforma.
pub trait CatalogoContablePlataforma: Send {
    /// Inserta o actualiza cuentas del PUC. Devuelve cuántas tocó.
    fn cargar_puc(&mut self, cuentas: &[CuentaPuc]) -> impl Future<Output = AppResult<u64>> + Send;

    /// Plantillas del ámbito (todas sus versiones y estados).
    fn plantillas(
        &mut self,
        ambito: AmbitoPlantilla,
    ) -> impl Future<Output = AppResult<Vec<PlantillaResumen>>> + Send;

    /// Plantilla por id.
    fn plantilla(
        &mut self,
        id: PlantillaContableId,
    ) -> impl Future<Output = AppResult<Option<PlantillaGuardada>>> + Send;

    /// Mayor versión con ese nombre en el ámbito (0 si no hay).
    fn ultima_version_plantilla(
        &mut self,
        ambito: AmbitoPlantilla,
        nombre: &str,
    ) -> impl Future<Output = AppResult<u32>> + Send;

    /// Guarda una plantilla nueva.
    fn insertar_plantilla(
        &mut self,
        p: &PlantillaGuardada,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Cambia el contenido de un borrador.
    fn actualizar_contenido_plantilla(
        &mut self,
        id: PlantillaContableId,
        contenido: &serde_json::Value,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Cambia el estado.
    fn cambiar_estado_plantilla(
        &mut self,
        id: PlantillaContableId,
        estado: EstadoPlantilla,
        en: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// ¿La cuenta es dueña o gestora de la organización?
    fn gestiona_organizacion(
        &mut self,
        organizacion: OrganizacionId,
        cuenta: CuentaId,
    ) -> impl Future<Output = AppResult<bool>> + Send;
}
