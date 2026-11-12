//! Puertos de la publicidad de la plataforma (plan 08, fases 3 a 6; ADR 0015).
//!
//! - [`RepositorioPublicidadPlataforma`]: en la transacción de **plataforma** (portal del
//!   anunciante, moderación, tarifas, política de publicidad, liquidaciones y redención).
//! - [`RepositorioPublicidad`]: en la transacción de una **copropiedad** (piezas que la
//!   alcanzan, entregas agregadas, autorización del titular, envíos masivos con tope de
//!   frecuencia, reportes y pantalla del lobby). Lo que toca datos de plataforma pasa por
//!   funciones `SECURITY DEFINER`.
//! - [`DirectorioPublicidad`]: copropiedades con el módulo `publicidad` (worker).

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    AnuncianteId, CampanaId, CanalPublicidad, CodigoCategoria, CodigoCupon, CreativoId, CuentaId,
    EstadoCampana, EstadoCreativo, EstadoLiquidacion, FormatoCreativo, LiquidacionId,
    MedioAutorizacion, ModeloCobro, Nit, ObjetivoCampana, OrganizacionId, PersonaId,
    PoliticaPublicidadId, ReportePublicidadId, RolOrganizacion, Segmentacion, SolicitudProveedorId,
    Telefono, TenantId,
    publicidad::{ContenidoCreativo, Conteo, PiezaCandidata, Presupuesto},
};
use rust_decimal::Decimal;

use crate::{
    AppResult, Pagina, SolicitudPagina, outbox::EventoSaliente, puertos::directorio::Anunciante,
};

/// Campaña de un anunciante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Campana {
    pub id: CampanaId,
    pub anunciante_id: AnuncianteId,
    pub nombre: String,
    pub objetivo: ObjetivoCampana,
    /// Una de las categorías de su ficha; nunca restringida.
    pub categoria: CodigoCategoria,
    pub inicio: DateTime<Utc>,
    pub fin: DateTime<Utc>,
    pub modelo: ModeloCobro,
    pub presupuesto: Presupuesto,
    /// Tarifa fijada al aprobarla.
    pub tarifa: Option<Decimal>,
    pub consumido: Decimal,
    pub segmentacion: Segmentacion,
    pub conflicto_interes: bool,
    pub estado: EstadoCampana,
    pub motivo_estado: Option<String>,
    pub revisada_por: Option<CuentaId>,
    pub revisada_en: Option<DateTime<Utc>>,
    pub creada_por: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
    pub actualizada_en: DateTime<Utc>,
}

/// Creativo (pieza) de una campaña.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creativo {
    pub id: CreativoId,
    pub campana_id: CampanaId,
    pub anunciante_id: AnuncianteId,
    pub contenido: ContenidoCreativo,
    pub cupon_usos: u32,
    pub estado: EstadoCreativo,
    pub motivo_estado: Option<String>,
    pub creado_en: DateTime<Utc>,
    pub actualizado_en: DateTime<Utc>,
}

/// Tarifa de la pauta (COP) por modelo de cobro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarifaPublicidad {
    pub modelo: ModeloCobro,
    pub valor: Decimal,
    pub vigente_desde: NaiveDate,
}

/// Métrica agregada de una campaña por día y canal (sin datos de personas ni de PH).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricaCampana {
    pub dia: NaiveDate,
    pub canal: CanalPublicidad,
    pub conteo: Conteo,
    pub costo: Decimal,
}

/// Contacto que un residente autorizó compartir con el anunciante para una solicitud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lead {
    pub solicitud_id: SolicitudProveedorId,
    pub creado_en: DateTime<Utc>,
    pub categoria: Option<CodigoCategoria>,
    pub nombre: String,
    pub telefono: Option<Telefono>,
}

/// Consumo de un anunciante en un periodo: variable (entregas valoradas) y cargos fijos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumoPeriodo {
    pub anunciante_id: AnuncianteId,
    pub variable: Decimal,
    pub fijos: Decimal,
    pub leads: u32,
    pub mensajes_whatsapp: u32,
}

/// Liquidación de un anunciante en un mes. **Sin reparto a las copropiedades.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Liquidacion {
    pub id: LiquidacionId,
    pub anunciante_id: AnuncianteId,
    /// Primer día del mes.
    pub periodo: NaiveDate,
    pub variable: Decimal,
    pub fijos: Decimal,
    pub leads: u32,
    pub valor_leads: Decimal,
    pub valor: Decimal,
    pub costo_whatsapp: Decimal,
    pub estado: EstadoLiquidacion,
    pub calculada_en: DateTime<Utc>,
    pub actualizada_en: DateTime<Utc>,
}

/// Creativo con reportes sin atender (cola de moderación).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreativoReportado {
    pub creativo_id: CreativoId,
    pub campana_id: CampanaId,
    pub anunciante_id: AnuncianteId,
    pub nombre_comercial: String,
    pub titulo: String,
    pub reportes: u32,
    pub motivos: Vec<String>,
    pub primero_en: DateTime<Utc>,
}

/// Versión de la política de tratamiento de la plataforma para la publicidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoliticaPublicidad {
    pub id: PoliticaPublicidadId,
    pub version: u32,
    pub texto: String,
    pub vigente_desde: DateTime<Utc>,
    pub publicada_por: Option<CuentaId>,
    pub creada_en: DateTime<Utc>,
}

/// Publicidad en la transacción de plataforma.
pub trait RepositorioPublicidadPlataforma: Send {
    /// La organización con ese NIT, si ya existe.
    fn organizacion_por_nit(
        &mut self,
        nit: &Nit,
    ) -> impl Future<Output = AppResult<Option<OrganizacionId>>> + Send;

    /// Anunciantes de las organizaciones de las que la cuenta es miembro, con su papel.
    fn anunciantes_de_cuenta(
        &mut self,
        cuenta: CuentaId,
    ) -> impl Future<Output = AppResult<Vec<(Anunciante, RolOrganizacion)>>> + Send;

    /// Guarda una campaña nueva.
    fn insertar_campana(&mut self, c: &Campana) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda todos los datos de la campaña (incluida la segmentación).
    fn actualizar_campana(&mut self, c: &Campana) -> impl Future<Output = AppResult<()>> + Send;

    /// La campaña; `bloquear` = `FOR UPDATE`.
    fn campana(
        &mut self,
        id: CampanaId,
        bloquear: bool,
    ) -> impl Future<Output = AppResult<Option<Campana>>> + Send;

    /// Campañas del anunciante, paginadas por cursor.
    fn campanas_de_anunciante(
        &mut self,
        anunciante: AnuncianteId,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Campana>>> + Send;

    /// Campañas en espera de moderación, paginadas por cursor.
    fn campanas_en_revision(
        &mut self,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Campana>>> + Send;

    /// Pasa a `finalizada` las aprobadas, activas o pausadas cuyo `fin` ya pasó.
    fn finalizar_vencidas(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<u64>> + Send;

    /// Guarda un creativo nuevo.
    fn insertar_creativo(&mut self, c: &Creativo) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda los cambios de un creativo.
    fn actualizar_creativo(&mut self, c: &Creativo) -> impl Future<Output = AppResult<()>> + Send;

    /// Creativo por su id.
    fn creativo(
        &mut self,
        id: CreativoId,
    ) -> impl Future<Output = AppResult<Option<Creativo>>> + Send;

    /// Creativos de la campaña.
    fn creativos_de_campana(
        &mut self,
        campana: CampanaId,
    ) -> impl Future<Output = AppResult<Vec<Creativo>>> + Send;

    /// Creativos en espera de moderación, paginados por cursor.
    fn creativos_pendientes(
        &mut self,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Creativo>>> + Send;

    /// El creativo con ese cupón del anunciante, con su campaña bloqueada.
    fn creativo_por_cupon(
        &mut self,
        anunciante: AnuncianteId,
        codigo: &CodigoCupon,
    ) -> impl Future<Output = AppResult<Option<Creativo>>> + Send;

    /// Suma un uso al cupón y registra la redención (entrega sin copropiedad) con su costo.
    fn registrar_redencion(
        &mut self,
        creativo: CreativoId,
        ahora: DateTime<Utc>,
        costo: Decimal,
        finalizar: bool,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// ¿La organización presta servicio en una copropiedad que la campaña alcanza?
    fn conflicto_interes(
        &mut self,
        campana: CampanaId,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Cuántas copropiedades con el módulo alcanza hoy (sin decir cuáles).
    fn alcance(&mut self, campana: CampanaId) -> impl Future<Output = AppResult<u64>> + Send;

    /// Tarifa del modelo de cobro vigente en el día.
    fn tarifa_vigente(
        &mut self,
        modelo: ModeloCobro,
        dia: NaiveDate,
    ) -> impl Future<Output = AppResult<Option<TarifaPublicidad>>> + Send;

    /// Todas las tarifas, por modelo y fecha.
    fn tarifas(&mut self) -> impl Future<Output = AppResult<Vec<TarifaPublicidad>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay una tarifa del modelo desde ese día.
    fn insertar_tarifa(
        &mut self,
        t: &TarifaPublicidad,
        creada_por: Option<CuentaId>,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Métricas agregadas de la campaña (sin datos de residentes).
    fn metricas_campana(
        &mut self,
        campana: CampanaId,
    ) -> impl Future<Output = AppResult<Vec<MetricaCampana>>> + Send;

    /// Contactos autorizados del anunciante, el más reciente primero.
    fn leads(
        &mut self,
        anunciante: AnuncianteId,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<Lead>>> + Send;

    /// Consumo de cada anunciante en `[desde, hasta)`.
    fn consumo_periodo(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<Vec<ConsumoPeriodo>>> + Send;

    /// Costo de un mensaje de marketing de WhatsApp en Colombia (COP) ese día.
    fn tarifa_whatsapp_marketing(
        &mut self,
        dia: NaiveDate,
    ) -> impl Future<Output = AppResult<Option<Decimal>>> + Send;

    /// Liquidación del anunciante en el periodo, si existe.
    fn liquidacion(
        &mut self,
        anunciante: AnuncianteId,
        periodo: NaiveDate,
    ) -> impl Future<Output = AppResult<Option<Liquidacion>>> + Send;

    /// Crea o reemplaza la liquidación del anunciante en el periodo.
    fn guardar_liquidacion(
        &mut self,
        l: &Liquidacion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Liquidaciones del periodo.
    fn liquidaciones(
        &mut self,
        periodo: NaiveDate,
    ) -> impl Future<Output = AppResult<Vec<Liquidacion>>> + Send;

    /// Creativos reportados por residentes pendientes de atender, paginados por cursor.
    fn reportes_pendientes(
        &mut self,
        pagina: SolicitudPagina,
    ) -> impl Future<Output = AppResult<Pagina<CreativoReportado>>> + Send;

    /// Marca atendidos los reportes del creativo en todas las copropiedades.
    fn atender_reportes(
        &mut self,
        creativo: CreativoId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<u64>> + Send;

    /// Política de publicidad vigente en el instante.
    fn politica_publicidad_vigente(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<PoliticaPublicidad>>> + Send;

    /// Última versión publicada de la política (0 si no hay ninguna).
    fn ultima_version_politica(&mut self) -> impl Future<Output = AppResult<u32>> + Send;

    /// Publica una versión nueva de la política de publicidad.
    fn insertar_politica_publicidad(
        &mut self,
        p: &PoliticaPublicidad,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

/// Pieza que puede mostrarse en una copropiedad, con lo que necesita el cliente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiezaPublicidad {
    pub candidata: PiezaCandidata,
    pub anunciante_id: AnuncianteId,
    pub nombre_comercial: String,
    pub contenido: ContenidoCreativo,
}

/// Persona que puede recibir un mensaje masivo (autorizó la publicidad).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinatarioMasivo {
    pub persona_id: PersonaId,
    pub telefono: Option<String>,
    pub correo: Option<String>,
}

/// Reporte de un anuncio hecho por un residente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportePublicidad {
    pub id: ReportePublicidadId,
    pub creativo_id: CreativoId,
    pub cuenta_id: CuentaId,
    pub motivo: String,
    pub creado_en: DateTime<Utc>,
}

/// Token (solo su SHA-256) de la pantalla del lobby de la copropiedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PantallaPublicidad {
    pub token_hash: Vec<u8>,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
}

/// Publicidad en la transacción de una copropiedad.
pub trait RepositorioPublicidad: Send {
    /// Piezas de esos formatos que pueden entregarse en la copropiedad (función
    /// `piezas_publicidad`: vacío si no tiene el módulo `publicidad`).
    fn piezas(
        &mut self,
        formatos: &[FormatoCreativo],
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<PiezaPublicidad>>> + Send;

    /// La pieza con su campaña bloqueada para registrar una entrega; `None` si no se puede
    /// entregar en la copropiedad.
    fn pieza_para_entrega(
        &mut self,
        creativo: CreativoId,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<(PiezaCandidata, FormatoCreativo)>>> + Send;

    /// Suma la entrega agregada y su costo; `finalizar` cierra la campaña.
    fn aplicar_entrega(
        &mut self,
        creativo: CreativoId,
        canal: CanalPublicidad,
        ahora: DateTime<Utc>,
        conteo: Conteo,
        costo: Decimal,
        finalizar: bool,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Política de publicidad vigente en el instante (la otorga el titular frente a la plataforma).
    fn politica_publicidad_vigente(
        &mut self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<PoliticaPublicidad>>> + Send;

    /// Registra la autorización `publicidad` bajo la política de la plataforma. `false` si ya
    /// había una vigente.
    fn autorizar_publicidad(
        &mut self,
        persona: PersonaId,
        politica: PoliticaPublicidadId,
        medio: MedioAutorizacion,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// `false` si la cuenta ya había reportado el creativo.
    fn insertar_reporte_publicidad(
        &mut self,
        r: &ReportePublicidad,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Personas con la autorización `publicidad` vigente (propietarios o residentes vigentes,
    /// nunca menores) que no han recibido esta campaña por el canal ni otro masivo en los
    /// últimos días del tope de frecuencia.
    fn destinatarios_masivos(
        &mut self,
        campana: CampanaId,
        canal: CanalPublicidad,
        ahora: DateTime<Utc>,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<DestinatarioMasivo>>> + Send;

    /// Registra el envío de una pieza masiva a una persona (tope de frecuencia).
    fn registrar_envio_publicidad(
        &mut self,
        campana: CampanaId,
        persona: PersonaId,
        canal: CanalPublicidad,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Encola con `concepto = 'publicidad'` y la campaña (costo de WhatsApp por campaña).
    fn encolar_publicidad(
        &mut self,
        evento: &EventoSaliente,
        campana: CampanaId,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Pantalla del lobby de la copropiedad, si tiene una.
    fn pantalla(&mut self) -> impl Future<Output = AppResult<Option<PantallaPublicidad>>> + Send;

    /// Crea o reemplaza la pantalla del lobby (nuevo token).
    fn guardar_pantalla(
        &mut self,
        p: &PantallaPublicidad,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

/// Copropiedades con el módulo `publicidad` habilitado (función `SECURITY DEFINER`).
pub trait DirectorioPublicidad: Send + Sync {
    /// Copropiedades activas con el módulo `publicidad` (worker de envíos masivos).
    fn copropiedades_con_publicidad(
        &self,
        ahora: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Vec<TenantId>>> + Send;
}
