//! Puerto del censo (plan 07a): residencias, vehículos, mascotas, propuestas de cambio y los
//! libros de propietarios y residentes. Se usa dentro de la transacción de una copropiedad.

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, BienPrivadoId, CalidadResidencia, CambioCensoId, CodigoBienPrivado, CuentaId,
    Documento, EspecieMascota, EstadoCambioCenso, EstadoMascota, MascotaId, PersonaId, Placa,
    Porcentaje, ResidenciaId, TipoCambioCenso, TipoDocumento, TipoTitularidad, TipoVehiculo,
    VehiculoId,
};
use rust_decimal::Decimal;

use crate::AppResult;

/// Residencia de una persona en un bien privado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Residencia {
    pub id: ResidenciaId,
    pub bien_privado_id: BienPrivadoId,
    pub persona_id: PersonaId,
    pub calidad: CalidadResidencia,
    /// Cabeza del bien privado ante la copropiedad.
    pub es_responsable: bool,
    /// Propietario que autoriza al tenedor.
    pub autorizado_por: Option<PersonaId>,
    pub contrato_archivo_id: Option<ArchivoId>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
}

/// Vehículo de un bien privado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vehiculo {
    pub id: VehiculoId,
    pub bien_privado_id: BienPrivadoId,
    pub persona_id: Option<PersonaId>,
    pub tipo: TipoVehiculo,
    pub placa: Option<Placa>,
    pub marca: Option<String>,
    pub color: Option<String>,
    pub tarjeta_archivo_id: Option<ArchivoId>,
    pub vigente_desde: DateTime<Utc>,
    pub vigente_hasta: Option<DateTime<Utc>>,
}

/// Mascota de un bien privado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mascota {
    pub id: MascotaId,
    pub bien_privado_id: BienPrivadoId,
    pub responsable_id: Option<PersonaId>,
    pub especie: EspecieMascota,
    pub nombre: String,
    pub raza: Option<String>,
    pub potencialmente_peligrosa: bool,
    pub poliza_numero: Option<String>,
    pub poliza_vence: Option<NaiveDate>,
    pub poliza_archivo_id: Option<ArchivoId>,
    pub vacunas_vencen: Option<NaiveDate>,
    pub carne_archivo_id: Option<ArchivoId>,
    pub esterilizada: bool,
    pub foto_archivo_id: Option<ArchivoId>,
    pub estado: EstadoMascota,
}

/// Propuesta de cambio del censo. `datos` es la solicitud serializada (JSON) del registro que
/// se crearía al aprobarla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CambioCenso {
    pub id: CambioCensoId,
    pub bien_privado_id: BienPrivadoId,
    pub tipo: TipoCambioCenso,
    pub datos: serde_json::Value,
    pub estado: EstadoCambioCenso,
    pub propuesto_por: CuentaId,
    pub propuesto_en: DateTime<Utc>,
    pub revisado_por: Option<CuentaId>,
    pub revisado_en: Option<DateTime<Utc>>,
    pub motivo: Option<String>,
    pub resultado_id: Option<uuid::Uuid>,
}

/// Fila del libro de propietarios a una fecha de corte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaLibroPropietarios {
    pub codigo: CodigoBienPrivado,
    pub coeficiente: Option<Decimal>,
    pub nombre: String,
    pub tipo_documento: Option<TipoDocumento>,
    pub documento: Option<Documento>,
    pub tipo: TipoTitularidad,
    pub porcentaje: Porcentaje,
    pub es_representante: bool,
    pub vigente_desde: DateTime<Utc>,
}

/// Fila del libro de residentes a una fecha de corte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaLibroResidentes {
    pub codigo: CodigoBienPrivado,
    pub nombre: String,
    pub tipo_documento: Option<TipoDocumento>,
    pub documento: Option<Documento>,
    pub calidad: CalidadResidencia,
    pub es_responsable: bool,
    pub vigente_desde: DateTime<Utc>,
}

pub trait RepositorioCenso: Send {
    /// Residencias (de un bien o de todos), vigentes o con su historial, más recientes primero.
    fn residencias(
        &mut self,
        bien: Option<BienPrivadoId>,
        incluir_terminadas: bool,
    ) -> impl Future<Output = AppResult<Vec<Residencia>>> + Send;

    fn residencia(
        &mut self,
        id: ResidenciaId,
    ) -> impl Future<Output = AppResult<Option<Residencia>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si la persona ya reside allí o ya hay otro responsable.
    fn insertar_residencia(&mut self, r: &Residencia)
    -> impl Future<Output = AppResult<()>> + Send;

    fn terminar_residencia(
        &mut self,
        id: ResidenciaId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn vehiculos(
        &mut self,
        bien: Option<BienPrivadoId>,
        placa: Option<&Placa>,
        incluir_retirados: bool,
    ) -> impl Future<Output = AppResult<Vec<Vehiculo>>> + Send;

    fn vehiculo(
        &mut self,
        id: VehiculoId,
    ) -> impl Future<Output = AppResult<Option<Vehiculo>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si la placa ya está vigente en la copropiedad.
    fn insertar_vehiculo(&mut self, v: &Vehiculo) -> impl Future<Output = AppResult<()>> + Send;

    fn retirar_vehiculo(
        &mut self,
        id: VehiculoId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn mascotas(
        &mut self,
        bien: Option<BienPrivadoId>,
        solo_activas: bool,
    ) -> impl Future<Output = AppResult<Vec<Mascota>>> + Send;

    fn mascota(&mut self, id: MascotaId)
    -> impl Future<Output = AppResult<Option<Mascota>>> + Send;

    fn insertar_mascota(&mut self, m: &Mascota) -> impl Future<Output = AppResult<()>> + Send;

    fn actualizar_mascota(&mut self, m: &Mascota) -> impl Future<Output = AppResult<()>> + Send;

    fn cambios(
        &mut self,
        estado: Option<EstadoCambioCenso>,
        propuesto_por: Option<CuentaId>,
    ) -> impl Future<Output = AppResult<Vec<CambioCenso>>> + Send;

    fn cambio(
        &mut self,
        id: CambioCensoId,
    ) -> impl Future<Output = AppResult<Option<CambioCenso>>> + Send;

    fn insertar_cambio(&mut self, c: &CambioCenso) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda la resolución (estado, revisor, motivo, resultado).
    fn resolver_cambio(&mut self, c: &CambioCenso) -> impl Future<Output = AppResult<()>> + Send;

    /// ¿La cuenta es titular vigente o residente vigente del bien privado?
    fn cuenta_vinculada_a_bien(
        &mut self,
        cuenta: CuentaId,
        bien: BienPrivadoId,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    fn libro_propietarios(
        &mut self,
        corte: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<FilaLibroPropietarios>>> + Send;

    fn libro_residentes(
        &mut self,
        corte: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<FilaLibroResidentes>>> + Send;

    /// Residentes vigentes sin autorización de tratamiento para `administracion`.
    fn residentes_sin_autorizacion(
        &mut self,
    ) -> impl Future<Output = AppResult<Vec<PersonaId>>> + Send;
}
