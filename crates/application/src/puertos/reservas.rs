//! Puerto de bienes comunes, reservas, parqueaderos y sorteos (plan 04). Se usa dentro de la
//! transacción de una copropiedad.

use std::future::Future;

use chrono::{DateTime, Utc};
use domain::{
    ArchivoId, AsignacionParqueaderoId, BienPrivadoId, CanalReserva, CargoId, CategoriaBienComun,
    ConceptoCargo, CuentaId, EstadoPago, EstadoRecurso, EstadoReserva, EstadoSorteo, Franja,
    Horario, ModalidadCita, NaturalezaParqueadero, OrigenAsignacionParqueadero, PersonaId, Placa,
    PoliticaReserva, RecursoId, ReservaId, ResidenciaId, RestriccionId, SorteoId, TipoEntrega,
    TipoParqueadero, TipoRecurso, VehiculoId, VinculoSolicitante,
    sorteo::{CriteriosSorteo, Semilla},
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::AppResult;

/// Detalle de un bien común.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetalleBienComun {
    pub categoria: CategoriaBienComun,
    pub area_m2: Option<Decimal>,
    /// Art. 59: un bien esencial no se restringe por mora.
    pub es_esencial: bool,
    pub reglas: Option<String>,
    pub reglas_archivo_id: Option<ArchivoId>,
}

/// Detalle de un parqueadero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetalleParqueadero {
    pub codigo: String,
    pub tipo_vehiculo: TipoParqueadero,
    pub naturaleza: NaturalezaParqueadero,
    /// Para los privados: el bien privado que es (o del que es anexo).
    pub bien_privado_id: Option<BienPrivadoId>,
}

/// Recurso reservable con su política, su horario y su detalle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recurso {
    pub id: RecursoId,
    pub tipo: TipoRecurso,
    pub nombre: String,
    pub estado: EstadoRecurso,
    pub ubicacion: Option<String>,
    pub descripcion: Option<String>,
    pub politica: PoliticaReserva,
    pub horario: Horario,
    pub bien_comun: Option<DetalleBienComun>,
    pub parqueadero: Option<DetalleParqueadero>,
}

/// Reserva o cita.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reserva {
    pub id: ReservaId,
    pub recurso_id: RecursoId,
    pub bien_privado_id: BienPrivadoId,
    pub solicitante_persona_id: Option<PersonaId>,
    pub solicitada_por: Option<CuentaId>,
    pub franja: Franja,
    /// Copia de la política: exclusiva (sin aforo) o por cupo.
    pub exclusiva: bool,
    pub invitados: u32,
    pub estado: EstadoReserva,
    pub valor: Decimal,
    pub deposito: Decimal,
    pub penalidad: Decimal,
    pub estado_pago: EstadoPago,
    pub idempotency_key: String,
    pub canal: CanalReserva,
    pub modalidad: Option<ModalidadCita>,
    pub motivo: Option<String>,
    pub placa_visitante: Option<Placa>,
    /// Motivo del rechazo o de la cancelación.
    pub observacion: Option<String>,
    pub creada_en: DateTime<Utc>,
}

/// Filtros del listado de reservas.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroReservas {
    pub recurso_id: Option<RecursoId>,
    pub bien_privado_id: Option<BienPrivadoId>,
    pub estado: Option<EstadoReserva>,
    pub solicitada_por: Option<CuentaId>,
    /// Reservas que se cruzan con este intervalo.
    pub entre: Option<Franja>,
}

/// Elemento del checklist de entrega o recibo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementoChecklist {
    pub elemento: String,
    #[serde(default)]
    pub cantidad: Option<u32>,
    /// `bueno`, `regular`, `danado`… (texto libre de la PH).
    #[serde(default)]
    pub estado: Option<String>,
}

/// Registro de entrega o recibo de una reserva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entrega {
    pub reserva_id: ReservaId,
    pub tipo: TipoEntrega,
    pub checklist: Vec<ElementoChecklist>,
    pub fotos: Vec<ArchivoId>,
    pub novedad: Option<String>,
    pub valor_danos: Option<Decimal>,
    pub registrado_por: Option<CuentaId>,
    pub registrado_en: DateTime<Utc>,
}

/// Cargo pendiente para el futuro módulo de contabilidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoPendiente {
    pub id: CargoId,
    pub bien_privado_id: BienPrivadoId,
    pub concepto: ConceptoCargo,
    pub descripcion: Option<String>,
    pub valor: Decimal,
    pub reserva_id: Option<ReservaId>,
    pub creado_en: DateTime<Utc>,
}

/// Restricción de reservas de un bien privado (mora o sanción).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restriccion {
    pub id: RestriccionId,
    pub bien_privado_id: BienPrivadoId,
    pub motivo: String,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub desde: DateTime<Utc>,
    pub hasta: Option<DateTime<Utc>>,
    pub creada_por: Option<CuentaId>,
}

/// Asignación de un parqueadero a un bien privado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsignacionParqueadero {
    pub id: AsignacionParqueaderoId,
    pub recurso_id: RecursoId,
    pub bien_privado_id: BienPrivadoId,
    pub vehiculo_id: Option<VehiculoId>,
    pub origen: OrigenAsignacionParqueadero,
    pub desde: DateTime<Utc>,
    /// `None` = sin fecha de fin.
    pub hasta: Option<DateTime<Utc>>,
    pub sorteo_id: Option<SorteoId>,
}

impl AsignacionParqueadero {
    pub fn vigente_en(&self, ahora: DateTime<Utc>) -> bool {
        self.desde <= ahora && self.hasta.is_none_or(|h| ahora < h)
    }
}

/// Sorteo de recursos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sorteo {
    pub id: SorteoId,
    pub nombre: String,
    pub recursos: Vec<RecursoId>,
    pub vigencia_desde: DateTime<Utc>,
    pub vigencia_hasta: DateTime<Utc>,
    pub estado: EstadoSorteo,
    pub compromiso: String,
    /// Secreta hasta ejecutar el sorteo.
    pub semilla: Semilla,
    pub criterios: CriteriosSorteo,
    pub resultado: Option<Vec<BienPrivadoId>>,
    pub creado_en: DateTime<Utc>,
    pub ejecutado_en: Option<DateTime<Utc>>,
}

/// Inscripción de un bien privado en un sorteo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InscripcionSorteo {
    pub sorteo_id: SorteoId,
    pub bien_privado_id: BienPrivadoId,
    pub discapacidad: bool,
    pub inscrito_por: Option<CuentaId>,
    pub inscrito_en: DateTime<Utc>,
}

pub trait RepositorioReservas: Send {
    fn recursos(
        &mut self,
        tipo: Option<TipoRecurso>,
    ) -> impl Future<Output = AppResult<Vec<Recurso>>> + Send;

    fn recurso(&mut self, id: RecursoId)
    -> impl Future<Output = AppResult<Option<Recurso>>> + Send;

    /// Lee el recurso bloqueándolo hasta el fin de la transacción: serializa las reservas del
    /// mismo recurso (cupos de aforo).
    fn recurso_para_reservar(
        &mut self,
        id: RecursoId,
    ) -> impl Future<Output = AppResult<Option<Recurso>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el código del parqueadero ya existe.
    fn insertar_recurso(&mut self, r: &Recurso) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda datos, estado, política, horario y detalle.
    fn actualizar_recurso(&mut self, r: &Recurso) -> impl Future<Output = AppResult<()>> + Send;

    fn reservas(
        &mut self,
        filtro: &FiltroReservas,
    ) -> impl Future<Output = AppResult<Vec<Reserva>>> + Send;

    fn reserva(&mut self, id: ReservaId)
    -> impl Future<Output = AppResult<Option<Reserva>>> + Send;

    fn reserva_por_clave(
        &mut self,
        clave: &str,
    ) -> impl Future<Output = AppResult<Option<Reserva>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si la franja ya está ocupada (recurso exclusivo) o la
    /// clave de idempotencia ya se usó.
    fn insertar_reserva(&mut self, r: &Reserva) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda estado, penalidad, estado del pago y observación.
    fn actualizar_reserva(&mut self, r: &Reserva) -> impl Future<Output = AppResult<()>> + Send;

    /// Reservas activas del bien en un recurso que empiezan en `[desde, hasta)`.
    fn contar_reservas_bien(
        &mut self,
        recurso: RecursoId,
        bien: BienPrivadoId,
        desde: DateTime<Utc>,
        hasta: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<u32>> + Send;

    /// Persona y vínculo de la cuenta con el bien privado (titular o residente vigente).
    fn vinculo_solicitante(
        &mut self,
        cuenta: CuentaId,
        bien: BienPrivadoId,
    ) -> impl Future<Output = AppResult<Option<(PersonaId, VinculoSolicitante)>>> + Send;

    fn restricciones(
        &mut self,
        bien: Option<BienPrivadoId>,
    ) -> impl Future<Output = AppResult<Vec<Restriccion>>> + Send;

    fn insertar_restriccion(
        &mut self,
        r: &Restriccion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Fija `hasta = ahora`. `false` si no existe o ya terminó.
    fn levantar_restriccion(
        &mut self,
        id: RestriccionId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya se registró ese tipo para la reserva.
    fn insertar_entrega(&mut self, e: &Entrega) -> impl Future<Output = AppResult<()>> + Send;

    fn entregas(
        &mut self,
        reserva: ReservaId,
    ) -> impl Future<Output = AppResult<Vec<Entrega>>> + Send;

    fn insertar_cargo(&mut self, c: &CargoPendiente) -> impl Future<Output = AppResult<()>> + Send;

    /// Autoriza (o no) las reservas de un tenedor.
    fn autorizar_reservas(
        &mut self,
        residencia: ResidenciaId,
        autorizadas: bool,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn asignaciones_parqueadero(
        &mut self,
        recurso: Option<RecursoId>,
        bien: Option<BienPrivadoId>,
    ) -> impl Future<Output = AppResult<Vec<AsignacionParqueadero>>> + Send;

    fn asignacion_parqueadero(
        &mut self,
        id: AsignacionParqueaderoId,
    ) -> impl Future<Output = AppResult<Option<AsignacionParqueadero>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el parqueadero ya está asignado en ese periodo.
    fn insertar_asignacion_parqueadero(
        &mut self,
        a: &AsignacionParqueadero,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Termina la asignación en `ahora` (o antes de empezar, la deja sin efecto).
    fn terminar_asignacion_parqueadero(
        &mut self,
        id: AsignacionParqueaderoId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn insertar_sorteo(&mut self, s: &Sorteo) -> impl Future<Output = AppResult<()>> + Send;

    fn sorteos(&mut self) -> impl Future<Output = AppResult<Vec<Sorteo>>> + Send;

    /// Lee el sorteo bloqueándolo (inscripciones y ejecución no se cruzan).
    fn sorteo(&mut self, id: SorteoId) -> impl Future<Output = AppResult<Option<Sorteo>>> + Send;

    fn inscripciones(
        &mut self,
        sorteo: SorteoId,
    ) -> impl Future<Output = AppResult<Vec<InscripcionSorteo>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el bien privado ya está inscrito.
    fn insertar_inscripcion(
        &mut self,
        i: &InscripcionSorteo,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn guardar_resultado_sorteo(
        &mut self,
        id: SorteoId,
        resultado: &[BienPrivadoId],
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Reservas aprobadas que empiezan antes de `hasta` y aún no se recordaron.
    fn reservas_por_recordar(
        &mut self,
        ahora: DateTime<Utc>,
        hasta: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<Reserva>>> + Send;

    fn marcar_recordada(
        &mut self,
        id: ReservaId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

/// Copropiedades con recordatorios pendientes (cruza tenants: función `SECURITY DEFINER`).
pub trait DirectorioRecordatorios: Send + Sync {
    fn copropiedades_con_recordatorios(
        &self,
        hasta: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<domain::TenantId>>> + Send;
}
