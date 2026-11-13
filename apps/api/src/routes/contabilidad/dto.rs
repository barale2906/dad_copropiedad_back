//! DTO del núcleo contable. Los montos viajan como texto decimal (`"1500.50"`), nunca como
//! número de punto flotante.

use application::{
    casos_uso::contabilidad::{
        comprobantes::LineaEntrada,
        configuracion::ResumenAplicacion,
        configuracion::Tercero,
        reportes::{EstadosFinancieros, Mayor},
    },
    puertos::contabilidad::{
        CentroCosto, Comprobante, ConfiguracionContable, CuentaContable, CuentaPuc, DatosFiscales,
        FilaSaldo, Movimiento, NotaEstados, Periodo, PlantillaGuardada, PlantillaResumen,
        Reapertura, ReglaVersionada, RubroEstado, TipoComprobante,
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    OrganizacionId, PersonaId, TipoDocumento, TipoPersona,
    contabilidad::{
        AsientoBalanceado, Cara, CodigoCuenta, EstadoComprobante, EstadoPeriodo, EstadoReapertura,
        LineaAsiento, MarcoTecnico, Monto, Naturaleza, NivelCuenta, OrigenCentro, OrigenTercero,
        RegimenIva, ResponsabilidadFiscal, RubroPlantilla, estados::ValorRubro,
    },
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

pub(super) fn codigo(texto: &str) -> ApiResult<CodigoCuenta> {
    CodigoCuenta::new(texto).map_err(|e| ApiError::App(e.into()))
}

// ---------------------------------------------------------------------------------------------
// Configuración y plan de cuentas
// ---------------------------------------------------------------------------------------------

/// Configuración contable vigente.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConfiguracionRespuesta {
    pub version: u32,
    #[schema(value_type = crate::esquemas::MarcoTecnico)]
    pub marco_tecnico: MarcoTecnico,
    /// Cuenta de excedente o déficit del ejercicio.
    pub cuenta_resultado_id: Option<Uuid>,
    pub soporte_archivo_id: Option<Uuid>,
    /// Plantilla (nombre y versión) de la que partió.
    pub plantilla_origen: Option<String>,
    pub vigente_desde: NaiveDate,
    pub creada_en: DateTime<Utc>,
}

impl From<ConfiguracionContable> for ConfiguracionRespuesta {
    fn from(c: ConfiguracionContable) -> Self {
        Self {
            version: c.version,
            marco_tecnico: c.marco_tecnico,
            cuenta_resultado_id: c.cuenta_resultado.map(|i| i.as_uuid()),
            soporte_archivo_id: c.soporte_archivo_id.map(|i| i.as_uuid()),
            plantilla_origen: c.plantilla_origen,
            vigente_desde: c.vigente_desde,
            creada_en: c.creada_en,
        }
    }
}

/// Versión nueva de la configuración.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfiguracionPeticion {
    #[schema(value_type = crate::esquemas::MarcoTecnico)]
    pub marco_tecnico: MarcoTecnico,
    /// Código de la cuenta de excedente o déficit (clase 3, de movimiento).
    #[schema(example = "360505")]
    pub cuenta_resultado: Option<String>,
    /// Soporte de la clasificación (archivo ya subido).
    pub soporte_archivo_id: Option<Uuid>,
    /// Desde cuándo se contabiliza (no se aceptan comprobantes anteriores al primer inicio).
    pub vigente_desde: NaiveDate,
}

/// Cuenta del plan de cuentas.
#[derive(Debug, Serialize, ToSchema)]
pub struct CuentaRespuesta {
    pub id: Uuid,
    #[schema(example = "110505")]
    pub codigo: String,
    pub nombre: String,
    #[schema(value_type = crate::esquemas::Naturaleza)]
    pub naturaleza: Naturaleza,
    #[schema(value_type = crate::esquemas::NivelCuenta)]
    pub nivel: NivelCuenta,
    pub acepta_movimiento: bool,
    pub exige_tercero: bool,
    pub exige_centro_costo: bool,
    pub activa: bool,
}

impl From<CuentaContable> for CuentaRespuesta {
    fn from(c: CuentaContable) -> Self {
        Self {
            id: c.id.as_uuid(),
            nivel: c.codigo.nivel(),
            codigo: c.codigo.as_str().to_owned(),
            nombre: c.nombre,
            naturaleza: c.naturaleza,
            acepta_movimiento: c.acepta_movimiento,
            exige_tercero: c.exige_tercero,
            exige_centro_costo: c.exige_centro_costo,
            activa: c.activa,
        }
    }
}

/// Cuenta nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaCuentaPeticion {
    #[schema(example = "11050501")]
    pub codigo: String,
    pub nombre: String,
    /// Sin valor: la del PUC si el código está en el catálogo; si no, la de la cuenta padre.
    #[schema(value_type = Option<crate::esquemas::Naturaleza>)]
    pub naturaleza: Option<Naturaleza>,
    pub acepta_movimiento: bool,
    #[serde(default)]
    pub exige_tercero: bool,
    #[serde(default)]
    pub exige_centro_costo: bool,
}

/// Cambios de una cuenta (el código y la naturaleza no cambian).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CambioCuentaPeticion {
    pub nombre: String,
    pub acepta_movimiento: bool,
    pub exige_tercero: bool,
    pub exige_centro_costo: bool,
    pub activa: bool,
}

/// Cuenta del catálogo PUC.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CuentaPucDto {
    #[schema(example = "1105")]
    pub codigo: String,
    pub nombre: String,
    #[schema(value_type = crate::esquemas::Naturaleza)]
    pub naturaleza: Naturaleza,
}

impl From<CuentaPuc> for CuentaPucDto {
    fn from(c: CuentaPuc) -> Self {
        Self {
            codigo: c.codigo.as_str().to_owned(),
            nombre: c.nombre,
            naturaleza: c.naturaleza,
        }
    }
}

/// Centro de costo.
#[derive(Debug, Serialize, ToSchema)]
pub struct CentroCostoRespuesta {
    pub id: Uuid,
    pub codigo: String,
    pub nombre: String,
    pub activo: bool,
}

impl From<CentroCosto> for CentroCostoRespuesta {
    fn from(c: CentroCosto) -> Self {
        Self {
            id: c.id.as_uuid(),
            codigo: c.codigo,
            nombre: c.nombre,
            activo: c.activo,
        }
    }
}

/// Centro de costo nuevo o editado (al editar, el código no cambia).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CentroCostoPeticion {
    #[schema(example = "ADM")]
    pub codigo: String,
    pub nombre: String,
    #[serde(default = "verdadero")]
    pub activo: bool,
}

fn verdadero() -> bool {
    true
}

/// Tipo de comprobante.
#[derive(Debug, Serialize, ToSchema)]
pub struct TipoComprobanteRespuesta {
    pub id: Uuid,
    #[schema(example = "RC")]
    pub codigo: String,
    pub nombre: String,
    /// `false` = solo lo usa el sistema (cierre, apertura, documentos de los módulos).
    pub manual: bool,
    pub activo: bool,
}

impl From<TipoComprobante> for TipoComprobanteRespuesta {
    fn from(t: TipoComprobante) -> Self {
        Self {
            id: t.id.as_uuid(),
            codigo: t.codigo,
            nombre: t.nombre,
            manual: t.manual,
            activo: t.activo,
        }
    }
}

/// Tipo de comprobante nuevo o editado (al editar, el código no cambia).
#[derive(Debug, Deserialize, ToSchema)]
pub struct TipoComprobantePeticion {
    #[schema(example = "RC")]
    pub codigo: String,
    pub nombre: String,
    #[serde(default = "verdadero")]
    pub manual: bool,
    #[serde(default = "verdadero")]
    pub activo: bool,
}

// ---------------------------------------------------------------------------------------------
// Reglas, rubros y notas
// ---------------------------------------------------------------------------------------------

/// Línea de una regla.
#[derive(Debug, Serialize, ToSchema)]
pub struct LineaReglaRespuesta {
    pub concepto: String,
    #[schema(value_type = crate::esquemas::Cara)]
    pub cara: Cara,
    pub cuenta_id: Uuid,
    #[schema(value_type = crate::esquemas::OrigenTercero)]
    pub tercero: OrigenTercero,
    #[schema(value_type = crate::esquemas::OrigenCentro)]
    pub centro: OrigenCentro,
    pub centro_costo_id: Option<Uuid>,
}

/// Versión de una regla de contabilización.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReglaRespuesta {
    pub id: Uuid,
    #[schema(example = "ventas.factura")]
    pub evento: String,
    pub version: u32,
    pub tipo_comprobante_id: Uuid,
    pub descripcion: Option<String>,
    pub vigente_desde: NaiveDate,
    pub lineas: Vec<LineaReglaRespuesta>,
    pub creada_en: DateTime<Utc>,
}

impl From<ReglaVersionada> for ReglaRespuesta {
    fn from(r: ReglaVersionada) -> Self {
        Self {
            id: r.id.as_uuid(),
            evento: r.regla.evento,
            version: r.version,
            tipo_comprobante_id: r.tipo_comprobante_id.as_uuid(),
            descripcion: r.descripcion,
            vigente_desde: r.vigente_desde,
            lineas: r
                .regla
                .lineas
                .into_iter()
                .map(|l| LineaReglaRespuesta {
                    concepto: l.concepto,
                    cara: l.cara,
                    cuenta_id: l.cuenta.as_uuid(),
                    tercero: l.tercero,
                    centro: l.centro,
                    centro_costo_id: l.centro_fijo.map(|c| c.as_uuid()),
                })
                .collect(),
            creada_en: r.creada_en,
        }
    }
}

/// Línea de una regla nueva (por códigos).
#[derive(Debug, Deserialize, ToSchema)]
pub struct LineaReglaPeticion {
    #[schema(example = "venta")]
    pub concepto: String,
    #[schema(value_type = crate::esquemas::Cara)]
    pub cara: Cara,
    #[schema(example = "130505")]
    pub cuenta: String,
    #[schema(value_type = crate::esquemas::OrigenTercero)]
    pub tercero: OrigenTercero,
    #[schema(value_type = crate::esquemas::OrigenCentro)]
    pub centro: OrigenCentro,
    /// Código del centro de costo si `centro = fijo`.
    pub centro_costo: Option<String>,
}

/// Versión nueva de una regla.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaReglaPeticion {
    #[schema(example = "ventas.factura")]
    pub evento: String,
    /// Código del tipo de comprobante que produce.
    #[schema(example = "FV")]
    pub tipo_comprobante: String,
    pub descripcion: Option<String>,
    pub vigente_desde: NaiveDate,
    pub lineas: Vec<LineaReglaPeticion>,
}

/// Rubro de un estado financiero.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RubroDto {
    /// `situacion_financiera` o `resultados`.
    #[schema(example = "situacion_financiera")]
    pub estado: String,
    #[schema(example = "efectivo")]
    pub codigo: String,
    pub nombre: String,
    pub orden: u16,
    /// Prefijos de código de cuenta que suma.
    #[schema(example = json!(["11"]))]
    pub prefijos: Vec<String>,
}

impl From<RubroEstado> for RubroDto {
    fn from(r: RubroEstado) -> Self {
        Self {
            estado: r.estado,
            codigo: r.codigo,
            nombre: r.nombre,
            orden: r.orden,
            prefijos: r.prefijos,
        }
    }
}

impl From<RubroDto> for RubroPlantilla {
    fn from(r: RubroDto) -> Self {
        Self {
            estado: r.estado,
            codigo: r.codigo,
            nombre: r.nombre,
            orden: r.orden,
            prefijos: r.prefijos,
        }
    }
}

/// Nota a los estados financieros (última versión).
#[derive(Debug, Serialize, ToSchema)]
pub struct NotaRespuesta {
    pub id: Uuid,
    pub anio: i32,
    pub numero: u16,
    pub version: u32,
    pub titulo: String,
    pub texto: String,
    pub rubro_id: Option<Uuid>,
    pub creada_en: DateTime<Utc>,
}

impl From<NotaEstados> for NotaRespuesta {
    fn from(n: NotaEstados) -> Self {
        Self {
            id: n.id.as_uuid(),
            anio: n.anio,
            numero: n.numero,
            version: n.version,
            titulo: n.titulo,
            texto: n.texto,
            rubro_id: n.rubro_id.map(|r| r.as_uuid()),
            creada_en: n.creada_en,
        }
    }
}

/// Versión nueva de una nota.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NotaPeticion {
    pub anio: i32,
    pub numero: u16,
    pub titulo: String,
    pub texto: String,
    /// Código del rubro al que se asocia.
    pub rubro: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Terceros
// ---------------------------------------------------------------------------------------------

/// Datos fiscales del tercero. La dirección y el correo son datos personales.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct DatosFiscalesDto {
    pub organizacion_id: Option<Uuid>,
    #[schema(value_type = Option<crate::esquemas::RegimenIva>)]
    pub regimen_iva: Option<RegimenIva>,
    #[serde(default)]
    #[schema(value_type = Vec<crate::esquemas::ResponsabilidadFiscal>)]
    pub responsabilidades: Vec<ResponsabilidadFiscal>,
    #[schema(example = "8110")]
    pub actividad_ciiu: Option<String>,
    pub direccion_fiscal: Option<String>,
    #[schema(example = "11001")]
    pub municipio_fiscal_dane: Option<String>,
    pub correo_facturacion: Option<String>,
}

impl From<DatosFiscales> for DatosFiscalesDto {
    fn from(d: DatosFiscales) -> Self {
        Self {
            organizacion_id: d.organizacion_id.map(OrganizacionId::as_uuid),
            regimen_iva: d.regimen_iva,
            responsabilidades: d.responsabilidades,
            actividad_ciiu: d.actividad_ciiu,
            direccion_fiscal: d.direccion_fiscal,
            municipio_fiscal_dane: d.municipio_fiscal_dane,
            correo_facturacion: d.correo_facturacion,
        }
    }
}

impl From<DatosFiscalesDto> for DatosFiscales {
    fn from(d: DatosFiscalesDto) -> Self {
        Self {
            organizacion_id: d.organizacion_id.map(OrganizacionId::from_uuid),
            regimen_iva: d.regimen_iva,
            responsabilidades: d.responsabilidades,
            actividad_ciiu: d.actividad_ciiu,
            direccion_fiscal: d.direccion_fiscal,
            municipio_fiscal_dane: d.municipio_fiscal_dane,
            correo_facturacion: d.correo_facturacion,
        }
    }
}

/// Tercero: persona de la entidad con sus datos fiscales.
#[derive(Debug, Serialize, ToSchema)]
pub struct TerceroRespuesta {
    pub persona_id: Uuid,
    #[schema(value_type = crate::esquemas::TipoPersona)]
    pub tipo: TipoPersona,
    pub nombre: String,
    #[schema(value_type = Option<crate::esquemas::TipoDocumento>)]
    pub tipo_documento: Option<TipoDocumento>,
    /// Dato personal.
    pub documento: Option<String>,
    pub fiscal: DatosFiscalesDto,
}

impl From<Tercero> for TerceroRespuesta {
    fn from(t: Tercero) -> Self {
        Self {
            persona_id: t.persona.id.as_uuid(),
            tipo: t.persona.tipo,
            nombre: t.persona.nombre,
            tipo_documento: t.persona.tipo_documento,
            documento: t.persona.documento.map(|d| d.as_str().to_owned()),
            fiscal: t.fiscal.into(),
        }
    }
}

/// Tercero nuevo (si ya hay una persona con ese documento, se reutiliza).
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoTerceroPeticion {
    #[schema(value_type = crate::esquemas::TipoPersona)]
    pub tipo: TipoPersona,
    pub nombre: String,
    #[schema(value_type = Option<crate::esquemas::TipoDocumento>)]
    pub tipo_documento: Option<TipoDocumento>,
    #[schema(example = "900123456")]
    pub documento: String,
    #[serde(default)]
    pub fiscal: DatosFiscalesDto,
}

// ---------------------------------------------------------------------------------------------
// Plantillas
// ---------------------------------------------------------------------------------------------

/// Plantilla en un listado.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlantillaResumenRespuesta {
    pub id: Uuid,
    /// `plataforma` u `organizacion`.
    pub ambito: String,
    pub organizacion: Option<String>,
    pub nombre: String,
    #[schema(example = "propiedad_horizontal")]
    pub tipo_empresa: String,
    pub version: u32,
    #[schema(value_type = crate::esquemas::EstadoPlantilla)]
    pub estado: application::puertos::contabilidad::EstadoPlantilla,
    pub publicada_en: Option<DateTime<Utc>>,
}

impl From<PlantillaResumen> for PlantillaResumenRespuesta {
    fn from(p: PlantillaResumen) -> Self {
        Self {
            id: p.id.as_uuid(),
            ambito: p.ambito,
            organizacion: p.organizacion,
            nombre: p.nombre,
            tipo_empresa: p.tipo_empresa,
            version: p.version,
            estado: p.estado,
            publicada_en: p.publicada_en,
        }
    }
}

/// Plantilla con su contenido.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlantillaRespuesta {
    pub id: Uuid,
    pub nombre: String,
    pub tipo_empresa: String,
    pub version: u32,
    #[schema(value_type = crate::esquemas::EstadoPlantilla)]
    pub estado: application::puertos::contabilidad::EstadoPlantilla,
    /// Contenido (`PlantillaContable`): marco técnico, cuentas, tipos de comprobante, reglas,
    /// rubros y cuenta de resultado.
    #[schema(value_type = Object)]
    pub contenido: serde_json::Value,
    pub creada_en: DateTime<Utc>,
    pub publicada_en: Option<DateTime<Utc>>,
}

impl From<PlantillaGuardada> for PlantillaRespuesta {
    fn from(p: PlantillaGuardada) -> Self {
        Self {
            id: p.id.as_uuid(),
            nombre: p.nombre,
            tipo_empresa: p.tipo_empresa,
            version: p.version,
            estado: p.estado,
            contenido: p.contenido,
            creada_en: p.creada_en,
            publicada_en: p.publicada_en,
        }
    }
}

/// Aplicar una plantilla a la entidad.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AplicarPlantillaPeticion {
    pub plantilla_id: Uuid,
    /// `true` = solo mostrar qué haría, sin cambiar nada.
    #[serde(default)]
    pub previsualizar: bool,
}

/// Lo que hizo (o haría) aplicar la plantilla.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResumenAplicacionRespuesta {
    pub cuentas_creadas: u32,
    pub cuentas_existentes: u32,
    pub tipos_creados: u32,
    pub tipos_existentes: u32,
    pub reglas_creadas: u32,
    pub rubros: u32,
    pub configuracion: bool,
    pub previsualizacion: bool,
}

impl ResumenAplicacionRespuesta {
    pub(super) fn de(r: ResumenAplicacion, previsualizacion: bool) -> Self {
        Self {
            cuentas_creadas: r.cuentas_creadas,
            cuentas_existentes: r.cuentas_existentes,
            tipos_creados: r.tipos_creados,
            tipos_existentes: r.tipos_existentes,
            reglas_creadas: r.reglas_creadas,
            rubros: r.rubros,
            configuracion: r.configuracion,
            previsualizacion,
        }
    }
}

/// Guardar la configuración de la entidad como plantilla (en borrador).
#[derive(Debug, Deserialize, ToSchema)]
pub struct GuardarPlantillaPeticion {
    /// Firma contable donde se guarda; sin valor = estándar de la plataforma (exige el permiso
    /// `plataforma.plantillas_contables.publicar`).
    pub organizacion_id: Option<Uuid>,
    pub nombre: String,
    #[schema(example = "propiedad_horizontal")]
    pub tipo_empresa: String,
}

/// Plantilla nueva de la plataforma.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaPlantillaPeticion {
    pub nombre: String,
    #[schema(example = "propiedad_horizontal")]
    pub tipo_empresa: String,
    /// Contenido (`PlantillaContable`).
    #[schema(value_type = Object)]
    pub contenido: serde_json::Value,
}

/// Contenido nuevo de una plantilla en borrador.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ContenidoPlantillaPeticion {
    #[schema(value_type = Object)]
    pub contenido: serde_json::Value,
}

/// Cambio de estado de una plantilla (`publicada` o `retirada`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct EstadoPlantillaPeticion {
    #[schema(value_type = crate::esquemas::EstadoPlantilla)]
    pub estado: application::puertos::contabilidad::EstadoPlantilla,
}

/// Cuentas del PUC para cargar.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CargaPucPeticion {
    pub cuentas: Vec<CuentaPucDto>,
}

/// Resultado de la carga del PUC.
#[derive(Debug, Serialize, ToSchema)]
pub struct CargaPucRespuesta {
    /// Cuentas insertadas o actualizadas.
    pub cuentas: u64,
}

// ---------------------------------------------------------------------------------------------
// Periodos
// ---------------------------------------------------------------------------------------------

/// Periodo contable (mes 13 = cierre anual).
#[derive(Debug, Serialize, ToSchema)]
pub struct PeriodoRespuesta {
    pub id: Uuid,
    pub anio: i32,
    pub mes: u8,
    #[schema(value_type = crate::esquemas::EstadoPeriodo)]
    pub estado: EstadoPeriodo,
    pub cerrado_en: Option<DateTime<Utc>>,
}

impl From<Periodo> for PeriodoRespuesta {
    fn from(p: Periodo) -> Self {
        Self {
            id: p.id.as_uuid(),
            anio: p.periodo.anio,
            mes: p.periodo.mes,
            estado: p.estado,
            cerrado_en: p.cerrado_en,
        }
    }
}

/// Periodo por abrir.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AbrirPeriodoPeticion {
    pub anio: i32,
    /// 1 a 12, o 13 (cierre anual).
    pub mes: u8,
}

/// Estado nuevo del periodo (`en_revision`, `cerrado` o `abierto` desde revisión).
#[derive(Debug, Deserialize, ToSchema)]
pub struct EstadoPeriodoPeticion {
    #[schema(value_type = crate::esquemas::EstadoPeriodo)]
    pub estado: EstadoPeriodo,
}

/// Reapertura de un periodo.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReaperturaRespuesta {
    pub id: Uuid,
    pub periodo_id: Uuid,
    pub motivo: String,
    pub alcance: Option<String>,
    pub solicitada_por: Uuid,
    pub solicitada_en: DateTime<Utc>,
    #[schema(value_type = crate::esquemas::EstadoReapertura)]
    pub estado: EstadoReapertura,
    pub decidida_por: Option<Uuid>,
    pub decidida_en: Option<DateTime<Utc>>,
}

impl From<Reapertura> for ReaperturaRespuesta {
    fn from(r: Reapertura) -> Self {
        Self {
            id: r.id.as_uuid(),
            periodo_id: r.periodo_id.as_uuid(),
            motivo: r.motivo,
            alcance: r.alcance,
            solicitada_por: r.solicitada_por.as_uuid(),
            solicitada_en: r.solicitada_en,
            estado: r.estado,
            decidida_por: r.decidida_por.map(|c| c.as_uuid()),
            decidida_en: r.decidida_en,
        }
    }
}

/// Solicitud de reapertura.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SolicitudReaperturaPeticion {
    pub motivo: String,
    pub alcance: Option<String>,
}

/// Decisión sobre una reapertura (la toma otra cuenta).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DecisionReaperturaPeticion {
    pub aprobar: bool,
}

// ---------------------------------------------------------------------------------------------
// Comprobantes
// ---------------------------------------------------------------------------------------------

/// Línea de un asiento.
#[derive(Debug, Serialize, ToSchema)]
pub struct LineaRespuesta {
    pub cuenta_id: Uuid,
    pub persona_id: Option<Uuid>,
    pub centro_costo_id: Option<Uuid>,
    #[schema(value_type = String, example = "150000.00")]
    pub debito: Decimal,
    #[schema(value_type = String, example = "0")]
    pub credito: Decimal,
    #[schema(value_type = Option<String>)]
    pub base: Option<Decimal>,
    pub descripcion: Option<String>,
}

impl From<&LineaAsiento> for LineaRespuesta {
    fn from(l: &LineaAsiento) -> Self {
        Self {
            cuenta_id: l.cuenta.as_uuid(),
            persona_id: l.persona.map(PersonaId::as_uuid),
            centro_costo_id: l.centro_costo.map(|c| c.as_uuid()),
            debito: l.debito.valor(),
            credito: l.credito.valor(),
            base: l.base.map(Monto::valor),
            descripcion: l.descripcion.clone(),
        }
    }
}

/// Línea de entrada (por códigos).
#[derive(Debug, Deserialize, ToSchema)]
pub struct LineaPeticion {
    #[schema(example = "110505")]
    pub cuenta: String,
    pub persona_id: Option<Uuid>,
    /// Código del centro de costo.
    pub centro_costo: Option<String>,
    #[serde(default)]
    #[schema(value_type = String, example = "150000")]
    pub debito: Monto,
    #[serde(default)]
    #[schema(value_type = String, example = "0")]
    pub credito: Monto,
    #[schema(value_type = Option<String>)]
    pub base: Option<Monto>,
    pub descripcion: Option<String>,
}

pub(super) fn lineas(entrada: Vec<LineaPeticion>) -> ApiResult<Vec<LineaEntrada>> {
    entrada
        .into_iter()
        .map(|l| {
            Ok(LineaEntrada {
                cuenta: codigo(&l.cuenta)?,
                persona: l.persona_id.map(PersonaId::from_uuid),
                centro_costo: l.centro_costo,
                debito: l.debito,
                credito: l.credito,
                base: l.base,
                descripcion: l.descripcion,
            })
        })
        .collect()
}

/// Comprobante manual (borrador, previsualización).
#[derive(Debug, Deserialize, ToSchema)]
pub struct BorradorPeticion {
    /// Código del tipo de comprobante (manual).
    #[schema(example = "CC")]
    pub tipo: String,
    pub fecha: NaiveDate,
    pub descripcion: String,
    pub lineas: Vec<LineaPeticion>,
}

/// Comprobante.
#[derive(Debug, Serialize, ToSchema)]
pub struct ComprobanteRespuesta {
    pub id: Uuid,
    #[schema(example = "RC")]
    pub tipo: String,
    /// Se asigna al contabilizar.
    pub numero: Option<u32>,
    pub fecha: NaiveDate,
    pub anio: i32,
    /// 13 = cierre anual.
    pub mes: u8,
    pub descripcion: String,
    #[schema(value_type = crate::esquemas::EstadoComprobante)]
    pub estado: EstadoComprobante,
    pub origen_tipo: Option<String>,
    pub origen_id: Option<Uuid>,
    pub revierte_a: Option<Uuid>,
    pub motivo_reverso: Option<String>,
    pub revertido_por: Option<Uuid>,
    /// Vacío en los listados.
    pub lineas: Vec<LineaRespuesta>,
    pub creado_en: DateTime<Utc>,
    pub contabilizado_en: Option<DateTime<Utc>>,
}

impl From<Comprobante> for ComprobanteRespuesta {
    fn from(c: Comprobante) -> Self {
        Self {
            id: c.id.as_uuid(),
            tipo: c.tipo_codigo,
            numero: c.numero,
            fecha: c.fecha,
            anio: c.periodo.anio,
            mes: c.periodo.mes,
            descripcion: c.descripcion,
            estado: c.estado,
            origen_tipo: c.origen.as_ref().map(|o| o.tipo.clone()),
            origen_id: c.origen.as_ref().map(|o| o.id),
            revierte_a: c.revierte_a.map(|r| r.as_uuid()),
            motivo_reverso: c.motivo_reverso,
            revertido_por: c.revertido_por.map(|r| r.as_uuid()),
            lineas: c.lineas.iter().map(Into::into).collect(),
            creado_en: c.creado_en,
            contabilizado_en: c.contabilizado_en,
        }
    }
}

/// Asiento validado (previsualización).
#[derive(Debug, Serialize, ToSchema)]
pub struct AsientoRespuesta {
    #[schema(value_type = String)]
    pub total: Decimal,
    pub lineas: Vec<LineaRespuesta>,
}

impl From<AsientoBalanceado> for AsientoRespuesta {
    fn from(a: AsientoBalanceado) -> Self {
        Self {
            total: a.total().valor(),
            lineas: a.lineas().iter().map(Into::into).collect(),
        }
    }
}

/// Reverso de un comprobante.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RevertirPeticion {
    pub motivo: String,
    /// Fecha del reverso; sin valor = hoy.
    pub fecha: Option<NaiveDate>,
}

/// Importe de un documento origen.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ImportePeticion {
    #[schema(example = "venta")]
    pub concepto: String,
    #[schema(value_type = String, example = "100000")]
    pub monto: Monto,
    pub persona_id: Option<Uuid>,
    pub centro_costo_id: Option<Uuid>,
    pub descripcion: Option<String>,
}

/// Documento origen para previsualizar su asiento con la regla vigente.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DocumentoPeticion {
    #[schema(example = "ventas.factura")]
    pub evento: String,
    pub fecha: NaiveDate,
    pub descripcion: String,
    pub importes: Vec<ImportePeticion>,
}

/// Saldos iniciales.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SaldosInicialesPeticion {
    pub fecha: NaiveDate,
    /// Código del tipo de comprobante; por defecto `AP`.
    pub tipo: Option<String>,
    pub descripcion: Option<String>,
    pub lineas: Vec<LineaPeticion>,
}

/// Cierre anual.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CierreAnualPeticion {
    pub anio: i32,
    /// Código del tipo de comprobante; por defecto `CI`.
    pub tipo: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Reportes
// ---------------------------------------------------------------------------------------------

/// Fila del balance de prueba.
#[derive(Debug, Serialize, ToSchema)]
pub struct FilaSaldoRespuesta {
    pub cuenta_id: Uuid,
    pub codigo: String,
    pub nombre: String,
    #[schema(value_type = crate::esquemas::Naturaleza)]
    pub naturaleza: Naturaleza,
    pub persona_id: Option<Uuid>,
    pub centro_costo_id: Option<Uuid>,
    /// Débitos − créditos antes de `desde`.
    #[schema(value_type = String)]
    pub saldo_inicial: Decimal,
    #[schema(value_type = String)]
    pub debitos: Decimal,
    #[schema(value_type = String)]
    pub creditos: Decimal,
    #[schema(value_type = String)]
    pub saldo_final: Decimal,
}

impl From<FilaSaldo> for FilaSaldoRespuesta {
    fn from(f: FilaSaldo) -> Self {
        Self {
            saldo_final: f.saldo_final(),
            cuenta_id: f.cuenta_id.as_uuid(),
            codigo: f.codigo.as_str().to_owned(),
            nombre: f.nombre,
            naturaleza: f.naturaleza,
            persona_id: f.persona_id.map(PersonaId::as_uuid),
            centro_costo_id: f.centro_costo_id.map(|c| c.as_uuid()),
            saldo_inicial: f.saldo_inicial,
            debitos: f.debitos,
            creditos: f.creditos,
        }
    }
}

/// Movimiento del libro mayor.
#[derive(Debug, Serialize, ToSchema)]
pub struct MovimientoRespuesta {
    pub comprobante_id: Uuid,
    pub tipo: String,
    pub numero: u32,
    pub fecha: NaiveDate,
    pub descripcion: String,
    pub persona_id: Option<Uuid>,
    pub centro_costo_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub debito: Decimal,
    #[schema(value_type = String)]
    pub credito: Decimal,
}

impl From<Movimiento> for MovimientoRespuesta {
    fn from(m: Movimiento) -> Self {
        Self {
            comprobante_id: m.comprobante_id.as_uuid(),
            tipo: m.tipo_codigo,
            numero: m.numero,
            fecha: m.fecha,
            descripcion: m.descripcion,
            persona_id: m.persona_id.map(PersonaId::as_uuid),
            centro_costo_id: m.centro_costo_id.map(|c| c.as_uuid()),
            debito: m.debito,
            credito: m.credito,
        }
    }
}

/// Libro mayor (o auxiliar de un tercero) de una cuenta.
#[derive(Debug, Serialize, ToSchema)]
pub struct MayorRespuesta {
    pub cuenta: CuentaRespuesta,
    pub persona_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub saldo_inicial: Decimal,
    pub movimientos: Vec<MovimientoRespuesta>,
    #[schema(value_type = String)]
    pub saldo_final: Decimal,
}

impl From<Mayor> for MayorRespuesta {
    fn from(m: Mayor) -> Self {
        Self {
            cuenta: m.cuenta.into(),
            persona_id: m.persona.map(PersonaId::as_uuid),
            saldo_inicial: m.saldo_inicial,
            movimientos: m.movimientos.into_iter().map(Into::into).collect(),
            saldo_final: m.saldo_final,
        }
    }
}

/// Valor de un rubro.
#[derive(Debug, Serialize, ToSchema)]
pub struct ValorRubroRespuesta {
    pub codigo: String,
    pub nombre: String,
    #[schema(value_type = String)]
    pub valor: Decimal,
}

impl From<ValorRubro> for ValorRubroRespuesta {
    fn from(v: ValorRubro) -> Self {
        Self {
            codigo: v.codigo,
            nombre: v.nombre,
            valor: v.valor,
        }
    }
}

/// Estados financieros a una fecha de corte.
#[derive(Debug, Serialize, ToSchema)]
pub struct EstadosFinancierosRespuesta {
    pub corte: NaiveDate,
    pub situacion_financiera: Vec<ValorRubroRespuesta>,
    pub resultados: Vec<ValorRubroRespuesta>,
    /// Ingresos − gastos − costos del año (sin el cierre anual).
    #[schema(value_type = String)]
    pub resultado_ejercicio: Decimal,
    /// Si ya hay cierre anual, el resultado ya está en el patrimonio.
    pub cierre_contabilizado: bool,
    pub notas: Vec<NotaRespuesta>,
}

impl From<EstadosFinancieros> for EstadosFinancierosRespuesta {
    fn from(e: EstadosFinancieros) -> Self {
        Self {
            corte: e.corte,
            situacion_financiera: e.situacion_financiera.into_iter().map(Into::into).collect(),
            resultados: e.resultados.into_iter().map(Into::into).collect(),
            resultado_ejercicio: e.resultado_ejercicio,
            cierre_contabilizado: e.cierre_contabilizado,
            notas: e.notas.into_iter().map(Into::into).collect(),
        }
    }
}
