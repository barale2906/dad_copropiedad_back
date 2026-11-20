//! Puertos de la nómina (plan 16, ADR 0025): el catálogo laboral de la plataforma (parámetros
//! versionados y administradoras de la seguridad social) y los empleados con sus contratos.
//!
//! El catálogo es de plataforma: se escribe en la transacción de plataforma y se lee también
//! desde la de la entidad (para validar un contrato contra los valores vigentes).

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AdministradoraId, ContratoLaboralId, CuentaId, Documento, EmpleadoId, Nit, ParametroLaboralId,
    PersonaId, VersionContratoId,
    contabilidad::{Monto, tesoreria::TipoCuentaBancaria},
    nomina::{
        ClaseRiesgo, CodigoEmpleado, EstadoEmpleado, EtapaAprendizaje, Jornada, MedioPago,
        MotivoTerminacion, ParametroLaboral, PeriodicidadPago, TipoAdministradora,
        TipoContratoLaboral, TipoSalario,
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
