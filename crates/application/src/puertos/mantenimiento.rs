//! Puerto de inventario y mantenimiento (plan 07c): activos, movimientos, planes preventivos,
//! órdenes de trabajo y obligaciones periódicas. Se usa dentro de la transacción de una
//! copropiedad.

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ActivoId, AgrupacionId, ArchivoId, ClaseActivo, CodigoActivo, CuentaId, EstadoActivo,
    EstadoOrden, MomentoEvidencia, MovimientoInventarioId, NumeroOrden, ObligacionId,
    OrdenTrabajoId, OrganizacionId, OrigenOrden, PlanMantenimientoId, PuntoChecklist, Recurrencia,
    RecursoId, ReservaId, TicketId, TipoArchivoActivo, TipoMovimiento, TipoOrden,
};
use rust_decimal::Decimal;

use crate::{AppResult, Pagina, SolicitudPagina, puertos::tickets::Responsable};

/// Activo del inventario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activo {
    pub id: ActivoId,
    pub codigo: CodigoActivo,
    pub nombre: String,
    pub clase: ClaseActivo,
    pub marca: Option<String>,
    pub modelo: Option<String>,
    pub serie: Option<String>,
    pub ubicacion: Option<String>,
    pub agrupacion_id: Option<AgrupacionId>,
    /// Bien común donde está, o la herramienta prestable del motor de reservas.
    pub recurso_id: Option<RecursoId>,
    pub estado: EstadoActivo,
    pub fecha_compra: Option<NaiveDate>,
    pub valor: Option<Decimal>,
    pub garantia_vence: Option<NaiveDate>,
    pub proveedor_organizacion_id: Option<OrganizacionId>,
    pub creado_en: DateTime<Utc>,
}

/// Archivo de la hoja de vida de un activo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchivoActivo {
    pub archivo_id: ArchivoId,
    pub tipo: TipoArchivoActivo,
}

/// Movimiento de inventario (append-only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Movimiento {
    pub id: MovimientoInventarioId,
    pub activo_id: ActivoId,
    pub tipo: TipoMovimiento,
    pub ubicacion_destino: Option<String>,
    pub reserva_id: Option<ReservaId>,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub observacion: Option<String>,
    pub registrado_por: Option<CuentaId>,
    pub ocurrido_en: DateTime<Utc>,
}

/// Plan de mantenimiento preventivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanMantenimiento {
    pub id: PlanMantenimientoId,
    pub activo_id: Option<ActivoId>,
    pub ubicacion: Option<String>,
    pub actividad: String,
    pub recurrencia: Recurrencia,
    /// Días antes de la fecha en que se genera la orden.
    pub anticipacion_dias: u32,
    pub responsable: Option<Responsable>,
    pub duracion_estimada_min: Option<u32>,
    pub checklist: Vec<String>,
    pub activo: bool,
}

/// Orden de trabajo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrdenTrabajo {
    pub id: OrdenTrabajoId,
    pub numero: NumeroOrden,
    pub origen: OrigenOrden,
    pub tipo: TipoOrden,
    pub plan_id: Option<PlanMantenimientoId>,
    pub ticket_id: Option<TicketId>,
    pub activo_id: Option<ActivoId>,
    pub ubicacion: Option<String>,
    pub descripcion: String,
    pub asignado: Option<Responsable>,
    pub programada_para: NaiveDate,
    pub estado: EstadoOrden,
    pub checklist: Vec<PuntoChecklist>,
    pub costo_mano_obra: Decimal,
    pub costo_repuestos: Decimal,
    pub observaciones: Option<String>,
    /// Nombre de quien recibe el trabajo (dato personal).
    pub recibido_por: Option<String>,
    pub firma_archivo_id: Option<ArchivoId>,
    pub iniciada_en: Option<DateTime<Utc>>,
    pub ejecutada_en: Option<DateTime<Utc>>,
    pub verificada_en: Option<DateTime<Utc>>,
    pub verificada_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
    pub actualizado_en: DateTime<Utc>,
}

/// Evidencia (foto antes/después o soporte) de una orden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenciaOrden {
    pub archivo_id: ArchivoId,
    pub momento: MomentoEvidencia,
}

/// Obligación periódica (certificación, renovación, inspección).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obligacion {
    pub id: ObligacionId,
    pub codigo_sugerido: Option<String>,
    pub nombre: String,
    pub descripcion: Option<String>,
    pub periodicidad_meses: u32,
    pub proxima: NaiveDate,
    /// Días antes de `proxima` en que empieza a alertar.
    pub dias_alerta: u32,
    pub activo_id: Option<ActivoId>,
    pub responsable_organizacion_id: Option<OrganizacionId>,
    pub activa: bool,
}

/// Registro de una obligación cumplida (append-only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cumplimiento {
    pub obligacion_id: ObligacionId,
    pub realizada_el: NaiveDate,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub observacion: Option<String>,
    pub registrado_por: Option<CuentaId>,
    pub registrado_en: DateTime<Utc>,
}

/// Filtro del listado de activos.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroActivos {
    pub clase: Option<ClaseActivo>,
    pub estado: Option<EstadoActivo>,
    /// Fragmento del nombre o del código (sin tildes ni mayúsculas).
    pub texto: Option<String>,
}

/// Filtro del listado de órdenes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroOrdenes {
    pub estado: Option<EstadoOrden>,
    pub activo_id: Option<ActivoId>,
    pub ticket_id: Option<TicketId>,
    /// Solo las asignadas a esta cuenta o a estas organizaciones (quien ejecuta).
    pub asignadas_a: Option<(CuentaId, Vec<OrganizacionId>)>,
    /// Pendientes con fecha programada anterior a esta.
    pub vencidas_a: Option<NaiveDate>,
}

/// Indicadores del tablero de mantenimiento.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableroMantenimiento {
    /// Preventivas programadas en los últimos 12 meses hasta hoy.
    pub preventivas_programadas: u64,
    /// De esas, ejecutadas o verificadas.
    pub preventivas_cumplidas: u64,
    pub ordenes_vencidas: u64,
    /// Costo acumulado (mano de obra + repuestos) por activo, de mayor a menor.
    pub costo_por_activo: Vec<(ActivoId, String, Decimal)>,
}

pub trait RepositorioMantenimiento: Send {
    fn activos(
        &mut self,
        filtro: &FiltroActivos,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Activo>>> + Send;

    fn activo(&mut self, id: ActivoId) -> impl Future<Output = AppResult<Option<Activo>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el código ya existe.
    fn insertar_activo(&mut self, a: &Activo) -> impl Future<Output = AppResult<()>> + Send;

    fn actualizar_activo(&mut self, a: &Activo) -> impl Future<Output = AppResult<()>> + Send;

    fn insertar_archivo_activo(
        &mut self,
        activo: ActivoId,
        a: &ArchivoActivo,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn archivos_activo(
        &mut self,
        activo: ActivoId,
    ) -> impl Future<Output = AppResult<Vec<ArchivoActivo>>> + Send;

    fn insertar_movimiento(&mut self, m: &Movimiento)
    -> impl Future<Output = AppResult<()>> + Send;

    /// Movimientos del activo en orden cronológico.
    fn movimientos(
        &mut self,
        activo: ActivoId,
    ) -> impl Future<Output = AppResult<Vec<Movimiento>>> + Send;

    fn planes(
        &mut self,
        solo_activos: bool,
    ) -> impl Future<Output = AppResult<Vec<PlanMantenimiento>>> + Send;

    fn plan(
        &mut self,
        id: PlanMantenimientoId,
    ) -> impl Future<Output = AppResult<Option<PlanMantenimiento>>> + Send;

    fn insertar_plan(
        &mut self,
        p: &PlanMantenimiento,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn actualizar_plan(
        &mut self,
        p: &PlanMantenimiento,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn ordenes(
        &mut self,
        filtro: &FiltroOrdenes,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<OrdenTrabajo>>> + Send;

    fn orden(
        &mut self,
        id: OrdenTrabajoId,
    ) -> impl Future<Output = AppResult<Option<OrdenTrabajo>>> + Send;

    /// ¿Ya existe la orden del plan para esa fecha? (generación idempotente).
    fn existe_orden_de_plan(
        &mut self,
        plan: PlanMantenimientoId,
        fecha: NaiveDate,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    fn insertar_orden(&mut self, o: &OrdenTrabajo) -> impl Future<Output = AppResult<()>> + Send;

    fn actualizar_orden(&mut self, o: &OrdenTrabajo) -> impl Future<Output = AppResult<()>> + Send;

    fn insertar_evidencia(
        &mut self,
        orden: OrdenTrabajoId,
        e: &EvidenciaOrden,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn evidencias(
        &mut self,
        orden: OrdenTrabajoId,
    ) -> impl Future<Output = AppResult<Vec<EvidenciaOrden>>> + Send;

    fn obligaciones(
        &mut self,
        solo_activas: bool,
    ) -> impl Future<Output = AppResult<Vec<Obligacion>>> + Send;

    fn obligacion(
        &mut self,
        id: ObligacionId,
    ) -> impl Future<Output = AppResult<Option<Obligacion>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay una activa con el mismo código sugerido.
    fn insertar_obligacion(&mut self, o: &Obligacion)
    -> impl Future<Output = AppResult<()>> + Send;

    fn actualizar_obligacion(
        &mut self,
        o: &Obligacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn insertar_cumplimiento(
        &mut self,
        c: &Cumplimiento,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn cumplimientos(
        &mut self,
        obligacion: ObligacionId,
    ) -> impl Future<Output = AppResult<Vec<Cumplimiento>>> + Send;

    fn tablero_mantenimiento(
        &mut self,
        hoy: NaiveDate,
    ) -> impl Future<Output = AppResult<TableroMantenimiento>> + Send;
}
