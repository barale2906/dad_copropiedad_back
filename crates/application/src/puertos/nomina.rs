//! Puertos de la nómina (plan 16, ADR 0025): el catálogo laboral de la plataforma (parámetros
//! versionados y administradoras de la seguridad social) y los empleados con sus contratos.
//!
//! El catálogo es de plataforma: se escribe en la transacción de plataforma y se lee también
//! desde la de la entidad (para validar un contrato contra los valores vigentes).

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AdministradoraId, ArchivoId, ContratoLaboralId, CuentaId, Documento, EmpleadoId,
    LiquidacionNominaId, Nit, NovedadNominaId, ParametroLaboralId, PeriodoNominaId, PersonaId,
    VersionContratoId,
    contabilidad::{Monto, tesoreria::TipoCuentaBancaria},
    nomina::{
        ClaseRiesgo, CodigoEmpleado, EstadoEmpleado, EtapaAprendizaje, Jornada, MedioPago,
        MotivoTerminacion, ParametroLaboral, PeriodicidadPago, TipoAdministradora,
        TipoContratoLaboral, TipoSalario,
        liquidacion::{Liquidacion, Novedad, TablaLaboral, TablaTramos},
        periodo::EstadoPeriodoNomina,
    },
};
use rust_decimal::Decimal;

use crate::{AppResult, Pagina, SolicitudPagina};

/// Versión de un parámetro del catálogo laboral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionParametroLaboral {
    pub id: ParametroLaboralId,
    pub parametro: ParametroLaboral,
    /// En la unidad del parámetro ([`ParametroLaboral::unidad`]).
    pub valor: Decimal,
    /// Norma o documento de donde sale el valor.
    pub fuente: String,
    pub vigente_desde: NaiveDate,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
}

/// Administradora de la seguridad social con su código de la PILA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Administradora {
    pub id: AdministradoraId,
    pub tipo: TipoAdministradora,
    /// Código en la PILA (único por tipo).
    pub codigo: String,
    pub nit: Nit,
    pub nombre: String,
    /// Una inactiva no se usa en contratos nuevos ni en versiones nuevas.
    pub activa: bool,
    pub creada_en: DateTime<Utc>,
}

/// Persistencia del catálogo laboral de la plataforma.
pub trait RepositorioCatalogoLaboral: Send {
    /// Guarda una versión de un parámetro (solo inserción).
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay una versión del parámetro con esa fecha.
    fn insertar_parametro(
        &mut self,
        version: &VersionParametroLaboral,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Versiones de los parámetros: con `fecha`, solo la vigente de cada uno en esa fecha; sin
    /// ella, todas. Ordenadas por parámetro y fecha.
    fn parametros(
        &mut self,
        fecha: Option<NaiveDate>,
    ) -> impl Future<Output = AppResult<Vec<VersionParametroLaboral>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya existe una del mismo tipo con ese código.
    fn insertar_administradora(
        &mut self,
        administradora: &Administradora,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Cambia el nombre y el estado (el tipo, el código y el NIT no cambian).
    fn actualizar_administradora(
        &mut self,
        administradora: &Administradora,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Administradora por id (activa o no).
    fn administradora(
        &mut self,
        id: AdministradoraId,
    ) -> impl Future<Output = AppResult<Option<Administradora>>> + Send;

    /// Administradoras por tipo y nombre (todas, o solo las activas).
    fn administradoras(
        &mut self,
        solo_activas: bool,
    ) -> impl Future<Output = AppResult<Vec<Administradora>>> + Send;

    /// Guarda una versión de una tabla por tramos (solo inserción).
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay una versión de la tabla con esa fecha.
    fn insertar_tabla(
        &mut self,
        tabla: &VersionTablaLaboral,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Versiones de las tablas por tramos: con `fecha`, la vigente de cada tabla en esa fecha;
    /// sin ella, todas.
    fn tablas(
        &mut self,
        fecha: Option<NaiveDate>,
    ) -> impl Future<Output = AppResult<Vec<VersionTablaLaboral>>> + Send;
}

/// Versión de una tabla laboral por tramos (retención en la fuente o fondo de solidaridad).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionTablaLaboral {
    pub tabla: TablaLaboral,
    pub vigente_desde: NaiveDate,
    pub fuente: String,
    pub tramos: TablaTramos,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
}

/// Empleado de la entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Empleado {
    pub id: EmpleadoId,
    pub codigo: CodigoEmpleado,
    /// La persona de la entidad que es el empleado.
    pub persona_id: PersonaId,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
}

/// Empleado en un listado, con lo que lo identifica y su situación de hoy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumenEmpleado {
    pub id: EmpleadoId,
    pub codigo: CodigoEmpleado,
    pub persona_id: PersonaId,
    /// Dato personal.
    pub nombre: String,
    /// Dato personal.
    pub documento: Option<Documento>,
    pub estado: EstadoEmpleado,
    /// Cargo de la versión vigente (o de la última, si ya se retiró).
    pub cargo: Option<String>,
    /// Inicio del primer contrato.
    pub fecha_ingreso: Option<NaiveDate>,
}

/// Filtro del listado de empleados.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroEmpleados {
    /// Fragmento del nombre, del documento o del código (sin tildes ni mayúsculas).
    pub texto: Option<String>,
    pub estado: Option<EstadoEmpleado>,
}

/// Contrato laboral (sin sus condiciones, que van en [`VersionContrato`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContratoLaboral {
    pub id: ContratoLaboralId,
    pub empleado_id: EmpleadoId,
    pub tipo: TipoContratoLaboral,
    pub fecha_inicio: NaiveDate,
    /// Fecha final pactada (término fijo y aprendizaje).
    pub fecha_fin: Option<NaiveDate>,
    /// Obra o labor contratada.
    pub descripcion_obra: Option<String>,
    pub fecha_terminacion: Option<NaiveDate>,
    pub motivo_terminacion: Option<MotivoTerminacion>,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
    pub terminado_por: Option<CuentaId>,
    pub terminado_en: Option<DateTime<Utc>>,
}

/// Cuenta bancaria donde se paga la nómina. Dato personal: `Debug` oculta el número.
#[derive(Clone, PartialEq, Eq)]
pub struct CuentaNomina {
    pub banco: String,
    pub tipo: TipoCuentaBancaria,
    /// Solo dígitos (4 a 20).
    pub numero: String,
}

impl std::fmt::Debug for CuentaNomina {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CuentaNomina")
            .field("banco", &self.banco)
            .field("tipo", &self.tipo)
            .field("numero", &"<oculto>")
            .finish()
    }
}

/// Condiciones de un contrato desde una fecha. Una modificación es una versión nueva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionContrato {
    pub id: VersionContratoId,
    pub contrato_id: ContratoLaboralId,
    pub vigente_desde: NaiveDate,
    pub cargo: String,
    /// Salario mensual (o apoyo de sostenimiento). `None` solo cuando quien consulta no tiene
    /// `nomina.salarios.ver`; al guardar siempre lleva valor.
    pub salario: Option<Monto>,
    pub tipo_salario: TipoSalario,
    pub etapa_aprendizaje: Option<EtapaAprendizaje>,
    pub jornada: Jornada,
    /// Solo en jornada parcial.
    pub horas_semanales: Option<Decimal>,
    pub periodicidad: PeriodicidadPago,
    pub eps_id: AdministradoraId,
    pub afp_id: Option<AdministradoraId>,
    pub cesantias_id: Option<AdministradoraId>,
    pub ccf_id: Option<AdministradoraId>,
    pub arl_id: Option<AdministradoraId>,
    /// Va con la ARL.
    pub clase_riesgo: Option<ClaseRiesgo>,
    pub pensionado: bool,
    pub medio_pago: MedioPago,
    /// Con transferencia. `None` también cuando quien consulta no tiene `nomina.salarios.ver`.
    pub cuenta: Option<CuentaNomina>,
    pub creada_por: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
}

/// Persistencia de los empleados y sus contratos (en la transacción de la entidad).
pub trait RepositorioNomina: Send {
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el código ya existe o la persona ya es empleada.
    fn insertar_empleado(
        &mut self,
        empleado: &Empleado,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Empleado por id.
    fn empleado(
        &mut self,
        id: EmpleadoId,
    ) -> impl Future<Output = AppResult<Option<Empleado>>> + Send;

    /// Empleado de la persona, si lo es.
    fn empleado_de_persona(
        &mut self,
        persona: PersonaId,
    ) -> impl Future<Output = AppResult<Option<Empleado>>> + Send;

    /// Empleados por id (cursor), con su estado en la fecha `hoy`.
    fn empleados(
        &mut self,
        filtro: &FiltroEmpleados,
        hoy: NaiveDate,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<ResumenEmpleado>>> + Send;

    /// Guarda un contrato; su primera versión va en la misma transacción.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si se cruza con otro contrato del empleado.
    fn insertar_contrato(
        &mut self,
        contrato: &ContratoLaboral,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda la prórroga o la terminación del contrato.
    ///
    /// # Errors
    ///
    /// [`domain::DominioError::ReglaLaboral`] si la BD lo rechaza (contrato terminado, versión
    /// posterior a la terminación…); [`crate::AppError::Conflicto`] si la prórroga lo cruza
    /// con otro contrato.
    fn actualizar_contrato(
        &mut self,
        contrato: &ContratoLaboral,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Contrato por id.
    fn contrato(
        &mut self,
        id: ContratoLaboralId,
    ) -> impl Future<Output = AppResult<Option<ContratoLaboral>>> + Send;

    /// Contratos del empleado, del más antiguo al más reciente.
    fn contratos(
        &mut self,
        empleado: EmpleadoId,
    ) -> impl Future<Output = AppResult<Vec<ContratoLaboral>>> + Send;

    /// Guarda una versión (solo inserción; `salario` debe venir).
    ///
    /// # Errors
    ///
    /// [`domain::DominioError::ReglaLaboral`] si la BD la rechaza (fuera del contrato, antes de
    /// la última…).
    fn insertar_version(
        &mut self,
        version: &VersionContrato,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Versiones del contrato, de la más antigua a la más reciente.
    fn versiones(
        &mut self,
        contrato: ContratoLaboralId,
    ) -> impl Future<Output = AppResult<Vec<VersionContrato>>> + Send;
}

/// Configuración de la nómina de la entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguracionNomina {
    /// Aplica la exoneración del art. 114-1 del ET (salud del empleador, ICBF y SENA por debajo
    /// del tope).
    pub exonerada_aportes: bool,
    pub actualizada_por: Option<CuentaId>,
    pub actualizada_en: DateTime<Utc>,
}

/// Periodo de nómina (mes o quincena).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodoNomina {
    pub id: PeriodoNominaId,
    pub periodicidad: PeriodicidadPago,
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    pub estado: EstadoPeriodoNomina,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
    pub liquidado_por: Option<CuentaId>,
    pub liquidado_en: Option<DateTime<Utc>>,
    pub aprobado_por: Option<CuentaId>,
    pub aprobado_en: Option<DateTime<Utc>>,
    pub pagado_por: Option<CuentaId>,
    pub pagado_en: Option<DateTime<Utc>>,
    pub fecha_pago: Option<NaiveDate>,
}

/// Anulación de una novedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnulacionNovedad {
    pub motivo: String,
    pub anulada_por: Option<CuentaId>,
    pub anulada_en: DateTime<Utc>,
}

/// Novedad registrada de un contrato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NovedadNomina {
    pub id: NovedadNominaId,
    pub contrato_id: ContratoLaboralId,
    pub novedad: Novedad,
    /// Dato personal.
    pub descripcion: Option<String>,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
    /// `Some` si se anuló (ya no entra a la liquidación).
    pub anulacion: Option<AnulacionNovedad>,
}

/// Liquidación de un contrato en un periodo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquidacionContrato {
    pub id: LiquidacionNominaId,
    pub periodo_id: PeriodoNominaId,
    pub contrato_id: ContratoLaboralId,
    pub empleado_id: EmpleadoId,
    pub liquidacion: Liquidacion,
    pub creado_en: DateTime<Utc>,
}

/// Persistencia de la liquidación de la nómina (en la transacción de la entidad).
pub trait RepositorioLiquidacionNomina: Send {
    /// Configuración de la entidad (`None` = la predeterminada, sin exoneración).
    fn configuracion(
        &mut self,
    ) -> impl Future<Output = AppResult<Option<ConfiguracionNomina>>> + Send;

    /// Crea o reemplaza la configuración.
    fn guardar_configuracion(
        &mut self,
        configuracion: &ConfiguracionNomina,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya existe el periodo con esa periodicidad y fecha.
    fn insertar_periodo(
        &mut self,
        periodo: &PeriodoNomina,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda el estado y las marcas del periodo.
    ///
    /// # Errors
    ///
    /// [`domain::DominioError::ReglaLaboral`] si la BD lo rechaza (periodo aprobado, misma
    /// cuenta que liquidó…).
    fn actualizar_periodo(
        &mut self,
        periodo: &PeriodoNomina,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Periodo por id, bloqueado para actualizarlo (`FOR UPDATE`).
    fn periodo(
        &mut self,
        id: PeriodoNominaId,
    ) -> impl Future<Output = AppResult<Option<PeriodoNomina>>> + Send;

    /// Periodo por su periodicidad e inicio.
    fn periodo_por_inicio(
        &mut self,
        periodicidad: PeriodicidadPago,
        desde: NaiveDate,
    ) -> impl Future<Output = AppResult<Option<PeriodoNomina>>> + Send;

    /// Periodos, el más reciente primero (a lo sumo `limite`).
    fn periodos(
        &mut self,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<PeriodoNomina>>> + Send;

    /// Guarda una novedad (solo inserción).
    ///
    /// # Errors
    ///
    /// [`domain::DominioError::ReglaLaboral`] si cae en un periodo aprobado.
    fn insertar_novedad(
        &mut self,
        novedad: &NovedadNomina,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya estaba anulada;
    /// [`domain::DominioError::ReglaLaboral`] si cae en un periodo aprobado.
    fn anular_novedad(
        &mut self,
        id: NovedadNominaId,
        anulacion: &AnulacionNovedad,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Novedad por id.
    fn novedad(
        &mut self,
        id: NovedadNominaId,
    ) -> impl Future<Output = AppResult<Option<NovedadNomina>>> + Send;

    /// Novedades que tocan el rango (de un contrato o de todos), con su anulación.
    fn novedades(
        &mut self,
        contrato: Option<ContratoLaboralId>,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<Vec<NovedadNomina>>> + Send;

    /// Última vez que se registró o anuló una novedad que toca el rango.
    fn ultimo_cambio_novedades(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<Option<DateTime<Utc>>>> + Send;

    /// Contratos que tocan el rango (de toda la entidad).
    fn contratos_en(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<Vec<ContratoLaboral>>> + Send;

    /// Borra las liquidaciones del periodo y guarda estas (solo con el periodo sin aprobar).
    fn reemplazar_liquidaciones(
        &mut self,
        periodo: PeriodoNominaId,
        liquidaciones: &[LiquidacionContrato],
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Suma de los devengos salariales variables (horas extra, recargos, comisiones y
    /// bonificaciones salariales) liquidados al contrato en periodos dentro del rango, sin el
    /// periodo `excluir` (el que se está liquidando), si se da.
    fn variables_liquidadas(
        &mut self,
        contrato: ContratoLaboralId,
        desde: NaiveDate,
        hasta: NaiveDate,
        excluir: Option<PeriodoNominaId>,
    ) -> impl Future<Output = AppResult<Decimal>> + Send;

    /// Liquidación por id, con sus líneas.
    fn liquidacion(
        &mut self,
        id: LiquidacionNominaId,
    ) -> impl Future<Output = AppResult<Option<LiquidacionContrato>>> + Send;

    /// Liquidaciones del periodo con sus líneas.
    fn liquidaciones(
        &mut self,
        periodo: PeriodoNominaId,
    ) -> impl Future<Output = AppResult<Vec<LiquidacionContrato>>> + Send;
}

/// Desprendible de la cuenta en una de sus entidades (lo mínimo para el portal del empleado).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesprendibleDeCuenta {
    pub tenant_id: domain::TenantId,
    /// Nombre de la entidad empleadora.
    pub entidad: String,
    pub liquidacion_id: LiquidacionNominaId,
    pub periodicidad: PeriodicidadPago,
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    pub estado: EstadoPeriodoNomina,
    pub fecha_pago: Option<NaiveDate>,
    pub devengos: Monto,
    pub deducciones: Monto,
    pub neto: Monto,
}

/// Desprendibles de una cuenta en todas sus entidades (transacción de plataforma; función
/// `SECURITY DEFINER` `desprendibles_de_cuenta`).
pub trait RepositorioNominaDeCuenta: Send {
    /// Liquidaciones aprobadas o pagadas de las personas vinculadas a la cuenta, las más
    /// recientes primero.
    fn desprendibles_de_cuenta(
        &mut self,
        cuenta: CuentaId,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<DesprendibleDeCuenta>>> + Send;
}
