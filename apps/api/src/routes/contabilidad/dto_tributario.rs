//! DTO de lo tributario y los documentos electrónicos (plan 15).

use application::{
    casos_uso::contabilidad::{
        declaraciones::{
            ExogenaGenerada, LineaIca, PreparacionIca, PreparacionIva, PreparacionRetencion,
        },
        tributario::CatalogoCompleto,
    },
    puertos::tributario::{
        BeneficiarioFinal, CampoExogena, DocumentoElectronico, EventoDocumento, FacturaRecibida,
        FormatoExogena, GeneracionExogena, PerfilTributario, RegistroRadian, ResolucionNumeracion,
        ResumenIva, ResumenRetencion, TarifaIca, TarifaIva,
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::contabilidad::{
    ResponsabilidadFiscal,
    tributario::{
        CanalFacturacion, CriterioBeneficiario, EstadoDocumentoElectronico, EstadoFacturaRecibida,
        EstadoPerfilTributario, EventoRadian, LadoExogena, ParametrosPerfil,
        TipoDocumentoElectronico, TipoNumeracion, TratamientoIva,
    },
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// ---------------------------------------------------------------------------------------------
// Catálogos
// ---------------------------------------------------------------------------------------------

/// Versión de una tarifa de IVA.
#[derive(Debug, Serialize, ToSchema)]
pub struct TarifaIvaRespuesta {
    pub id: Uuid,
    pub codigo: String,
    pub nombre: String,
    #[schema(value_type = crate::esquemas::TratamientoIva)]
    pub tratamiento: TratamientoIva,
    /// Porcentaje.
    #[schema(value_type = String, example = "19")]
    pub tarifa: Decimal,
    pub fuente: String,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: Option<NaiveDate>,
}

impl From<TarifaIva> for TarifaIvaRespuesta {
    fn from(t: TarifaIva) -> Self {
        Self {
            id: t.id.as_uuid(),
            codigo: t.codigo,
            nombre: t.nombre,
            tratamiento: t.tratamiento,
            tarifa: t.tarifa,
            fuente: t.fuente,
            vigente_desde: t.vigente_desde,
            vigente_hasta: t.vigente_hasta,
        }
    }
}

/// Versión nueva de una tarifa de IVA (cierra la anterior del mismo código el día antes).
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaTarifaIvaPeticion {
    #[schema(example = "general")]
    pub codigo: String,
    #[schema(example = "Tarifa general")]
    pub nombre: String,
    #[schema(value_type = crate::esquemas::TratamientoIva)]
    pub tratamiento: TratamientoIva,
    #[schema(value_type = String, example = "19")]
    pub tarifa: Decimal,
    #[schema(example = "Estatuto Tributario, art. 468")]
    pub fuente: String,
    pub vigente_desde: NaiveDate,
}

/// Versión de una tarifa de ICA.
#[derive(Debug, Serialize, ToSchema)]
pub struct TarifaIcaRespuesta {
    pub id: Uuid,
    pub municipio_dane: String,
    /// Código CIIU.
    pub actividad: String,
    pub nombre: String,
    #[schema(value_type = String, example = "9.66")]
    pub tarifa_por_mil: Decimal,
    pub fuente: String,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: Option<NaiveDate>,
}

impl From<TarifaIca> for TarifaIcaRespuesta {
    fn from(t: TarifaIca) -> Self {
        Self {
            id: t.id.as_uuid(),
            municipio_dane: t.municipio_dane,
            actividad: t.actividad,
            nombre: t.nombre,
            tarifa_por_mil: t.tarifa_por_mil,
            fuente: t.fuente,
            vigente_desde: t.vigente_desde,
            vigente_hasta: t.vigente_hasta,
        }
    }
}

/// Versión nueva de una tarifa de ICA.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaTarifaIcaPeticion {
    #[schema(example = "11001")]
    pub municipio_dane: String,
    #[schema(example = "6810")]
    pub actividad: String,
    pub nombre: String,
    #[schema(value_type = String, example = "9.66")]
    pub tarifa_por_mil: Decimal,
    pub fuente: String,
    pub vigente_desde: NaiveDate,
}

/// Cuentas que suman a un concepto y columna de la exógena.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CampoExogenaDto {
    #[schema(example = "5002")]
    pub concepto: String,
    #[schema(example = "5110")]
    pub cuenta_prefijo: String,
    #[schema(example = "pago_deducible")]
    pub columna: String,
    #[schema(value_type = crate::esquemas::LadoExogena)]
    pub lado: LadoExogena,
}

impl From<CampoExogena> for CampoExogenaDto {
    fn from(c: CampoExogena) -> Self {
        Self {
            concepto: c.concepto,
            cuenta_prefijo: c.cuenta_prefijo,
            columna: c.columna,
            lado: c.lado,
        }
    }
}

impl From<CampoExogenaDto> for CampoExogena {
    fn from(c: CampoExogenaDto) -> Self {
        Self {
            concepto: c.concepto,
            cuenta_prefijo: c.cuenta_prefijo,
            columna: c.columna,
            lado: c.lado,
        }
    }
}

/// Versión de un formato de exógena.
#[derive(Debug, Serialize, ToSchema)]
pub struct FormatoExogenaRespuesta {
    pub id: Uuid,
    pub anio: i32,
    #[schema(example = "1001")]
    pub codigo: String,
    pub version: u32,
    pub nombre: String,
    pub resolucion: String,
    /// Por debajo se agregan en el NIT 222222222.
    #[schema(value_type = String)]
    pub cuantia_menor: Decimal,
    pub campos: Vec<CampoExogenaDto>,
    pub creado_en: DateTime<Utc>,
}

impl From<FormatoExogena> for FormatoExogenaRespuesta {
    fn from(f: FormatoExogena) -> Self {
        Self {
            id: f.id.as_uuid(),
            anio: f.anio,
            codigo: f.codigo,
            version: f.version,
            nombre: f.nombre,
            resolucion: f.resolucion,
            cuantia_menor: f.cuantia_menor,
            campos: f.campos.into_iter().map(Into::into).collect(),
            creado_en: f.creado_en,
        }
    }
}

/// Formato de exógena nuevo (la versión la asigna el sistema: la siguiente del año y código).
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoFormatoExogenaPeticion {
    pub anio: i32,
    #[schema(example = "1001")]
    pub codigo: String,
    pub nombre: String,
    #[schema(example = "Resolución DIAN 000162 de 2023")]
    pub resolucion: String,
    #[schema(value_type = String, example = "100000")]
    pub cuantia_menor: Decimal,
    pub campos: Vec<CampoExogenaDto>,
}

/// Catálogo tributario: tarifas de IVA, de ICA y formatos de exógena.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogoTributarioRespuesta {
    pub iva: Vec<TarifaIvaRespuesta>,
    pub ica: Vec<TarifaIcaRespuesta>,
    pub formatos_exogena: Vec<FormatoExogenaRespuesta>,
}

impl From<CatalogoCompleto> for CatalogoTributarioRespuesta {
    fn from(c: CatalogoCompleto) -> Self {
        Self {
            iva: c.iva.into_iter().map(Into::into).collect(),
            ica: c.ica.into_iter().map(Into::into).collect(),
            formatos_exogena: c.formatos.into_iter().map(Into::into).collect(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Perfil tributario y resoluciones
// ---------------------------------------------------------------------------------------------

/// Parámetros del perfil tributario (lo que dice el RUT y decide el contador).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ParametrosPerfilDto {
    #[serde(default)]
    #[schema(value_type = Vec<crate::esquemas::ResponsabilidadFiscal>)]
    pub responsabilidades: Vec<ResponsabilidadFiscal>,
    pub agente_retenedor: bool,
    pub responsable_iva: bool,
    pub gran_contribuyente: bool,
    pub autorretenedor: bool,
    pub contribuyente_renta: bool,
    pub facturador_electronico: bool,
    pub obligado_rub: bool,
    /// Códigos DANE donde declara ICA.
    #[serde(default)]
    pub municipios_ica: Vec<String>,
    /// Códigos CIIU de sus actividades.
    #[serde(default)]
    pub actividades_ica: Vec<String>,
    #[schema(value_type = crate::esquemas::CanalFacturacion)]
    pub canal_facturacion: CanalFacturacion,
}

impl From<ParametrosPerfil> for ParametrosPerfilDto {
    fn from(p: ParametrosPerfil) -> Self {
        Self {
            responsabilidades: p.responsabilidades,
            agente_retenedor: p.agente_retenedor,
            responsable_iva: p.responsable_iva,
            gran_contribuyente: p.gran_contribuyente,
            autorretenedor: p.autorretenedor,
            contribuyente_renta: p.contribuyente_renta,
            facturador_electronico: p.facturador_electronico,
            obligado_rub: p.obligado_rub,
            municipios_ica: p.municipios_ica,
            actividades_ica: p.actividades_ica,
            canal_facturacion: p.canal_facturacion,
        }
    }
}

impl From<ParametrosPerfilDto> for ParametrosPerfil {
    fn from(p: ParametrosPerfilDto) -> Self {
        Self {
            responsabilidades: p.responsabilidades,
            agente_retenedor: p.agente_retenedor,
            responsable_iva: p.responsable_iva,
            gran_contribuyente: p.gran_contribuyente,
            autorretenedor: p.autorretenedor,
            contribuyente_renta: p.contribuyente_renta,
            facturador_electronico: p.facturador_electronico,
            obligado_rub: p.obligado_rub,
            municipios_ica: p.municipios_ica,
            actividades_ica: p.actividades_ica,
            canal_facturacion: p.canal_facturacion,
        }
    }
}

/// Versión del perfil tributario.
#[derive(Debug, Serialize, ToSchema)]
pub struct PerfilTributarioRespuesta {
    pub id: Uuid,
    pub version: u32,
    pub vigente_desde: NaiveDate,
    #[schema(value_type = crate::esquemas::EstadoPerfilTributario)]
    pub estado: EstadoPerfilTributario,
    pub parametros: ParametrosPerfilDto,
    pub soporte_archivo_id: Option<Uuid>,
    pub observaciones: Option<String>,
    pub creado_en: DateTime<Utc>,
    pub confirmado_por: Option<Uuid>,
    pub confirmado_en: Option<DateTime<Utc>>,
}

impl From<PerfilTributario> for PerfilTributarioRespuesta {
    fn from(p: PerfilTributario) -> Self {
        Self {
            id: p.id.as_uuid(),
            version: p.version,
            vigente_desde: p.vigente_desde,
            estado: p.estado,
            parametros: p.parametros.into(),
            soporte_archivo_id: p.soporte_archivo_id.map(|a| a.as_uuid()),
            observaciones: p.observaciones,
            creado_en: p.creado_en,
            confirmado_por: p.confirmado_por.map(|c| c.as_uuid()),
            confirmado_en: p.confirmado_en,
        }
    }
}

/// Versión nueva del perfil (queda en borrador hasta que el contador la confirme).
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoPerfilPeticion {
    pub vigente_desde: NaiveDate,
    pub parametros: ParametrosPerfilDto,
    /// RUT u otro soporte ya subido.
    pub soporte_archivo_id: Option<Uuid>,
    pub observaciones: Option<String>,
}

/// Resolución de numeración (la clave técnica no sale en la respuesta).
#[derive(Debug, Serialize, ToSchema)]
pub struct ResolucionRespuesta {
    pub id: Uuid,
    #[schema(value_type = crate::esquemas::TipoNumeracion)]
    pub tipo: TipoNumeracion,
    pub prefijo: String,
    pub desde: u64,
    pub hasta: u64,
    pub numero_resolucion: String,
    pub fecha_resolucion: NaiveDate,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: NaiveDate,
    /// Si tiene clave técnica registrada.
    pub tiene_clave_tecnica: bool,
    pub activa: bool,
    pub creada_en: DateTime<Utc>,
}

impl From<ResolucionNumeracion> for ResolucionRespuesta {
    fn from(r: ResolucionNumeracion) -> Self {
        Self {
            id: r.id.as_uuid(),
            tipo: r.tipo,
            prefijo: r.prefijo,
            desde: r.desde,
            hasta: r.hasta,
            numero_resolucion: r.numero_resolucion,
            fecha_resolucion: r.fecha_resolucion,
            vigente_desde: r.vigente_desde,
            vigente_hasta: r.vigente_hasta,
            tiene_clave_tecnica: r.clave_tecnica.is_some(),
            activa: r.activa,
            creada_en: r.creada_en,
        }
    }
}

/// Resolución de numeración nueva.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevaResolucionPeticion {
    #[schema(value_type = crate::esquemas::TipoNumeracion)]
    pub tipo: TipoNumeracion,
    #[schema(example = "FE")]
    pub prefijo: String,
    #[schema(example = 1)]
    pub desde: u64,
    #[schema(example = 5000)]
    pub hasta: u64,
    #[schema(example = "18764000000001")]
    pub numero_resolucion: String,
    pub fecha_resolucion: NaiveDate,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: NaiveDate,
    /// Clave técnica que entrega la DIAN (secreto: no se devuelve).
    pub clave_tecnica: Option<String>,
}

/// Activa o desactiva una resolución.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ActivaPeticion {
    pub activa: bool,
}

// ---------------------------------------------------------------------------------------------
// Documentos electrónicos
// ---------------------------------------------------------------------------------------------

/// Documento electrónico.
#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentoElectronicoRespuesta {
    pub id: Uuid,
    #[schema(value_type = crate::esquemas::TipoDocumentoElectronico)]
    pub tipo: TipoDocumentoElectronico,
    /// Documento de negocio que lo origina (`ventas.factura`, `cxp.factura`).
    pub origen_tipo: String,
    pub origen_id: Uuid,
    pub prefijo: String,
    pub numero: Option<u64>,
    /// Documento al que se refiere una nota.
    pub referencia_id: Option<Uuid>,
    pub cufe: Option<String>,
    #[schema(value_type = crate::esquemas::EstadoDocumentoElectronico)]
    pub estado: EstadoDocumentoElectronico,
    /// Adaptador que lo transmitió (`simulado` mientras no haya proveedor).
    pub proveedor: String,
    pub xml_archivo_id: Option<Uuid>,
    pub respuesta: Option<String>,
    pub intentos: u32,
    #[schema(value_type = String)]
    pub total: Decimal,
    pub creado_en: DateTime<Utc>,
    pub enviado_en: Option<DateTime<Utc>>,
    pub validado_en: Option<DateTime<Utc>>,
}

impl From<DocumentoElectronico> for DocumentoElectronicoRespuesta {
    fn from(d: DocumentoElectronico) -> Self {
        Self {
            id: d.id.as_uuid(),
            tipo: d.tipo,
            origen_tipo: d.origen.tipo,
            origen_id: d.origen.id,
            prefijo: d.prefijo,
            numero: d.numero,
            referencia_id: d.referencia_id.map(|r| r.as_uuid()),
            cufe: d.cufe,
            estado: d.estado,
            proveedor: d.proveedor,
            xml_archivo_id: d.xml_archivo_id.map(|a| a.as_uuid()),
            respuesta: d.respuesta,
            intentos: d.intentos,
            total: d.total.valor(),
            creado_en: d.creado_en,
            enviado_en: d.enviado_en,
            validado_en: d.validado_en,
        }
    }
}

/// Evento de un documento electrónico.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventoDocumentoRespuesta {
    pub tipo: String,
    pub detalle: Option<String>,
    pub ocurrido_en: DateTime<Utc>,
}

impl From<EventoDocumento> for EventoDocumentoRespuesta {
    fn from(e: EventoDocumento) -> Self {
        Self {
            tipo: e.tipo,
            detalle: e.detalle,
            ocurrido_en: e.ocurrido_en,
        }
    }
}

/// Documento electrónico con su historia.
#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentoDetalleRespuesta {
    pub documento: DocumentoElectronicoRespuesta,
    pub eventos: Vec<EventoDocumentoRespuesta>,
}

/// Motivo de una nota crédito.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NotaCreditoPeticion {
    #[schema(example = "Se facturó un valor equivocado")]
    pub motivo: String,
}

// ---------------------------------------------------------------------------------------------
// Facturas recibidas y RADIAN
// ---------------------------------------------------------------------------------------------

/// XML de una factura electrónica recibida (la `Invoice` o el `AttachedDocument`), en Base64.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CargarFacturaPeticion {
    #[schema(example = "fv-SETP990000123.xml")]
    pub nombre_archivo: String,
    pub archivo_base64: String,
}

/// Factura electrónica recibida.
#[derive(Debug, Serialize, ToSchema)]
pub struct FacturaRecibidaRespuesta {
    pub id: Uuid,
    pub archivo_id: Uuid,
    pub emisor_nit: String,
    pub emisor_nombre: String,
    pub prefijo: String,
    pub numero: String,
    pub fecha: NaiveDate,
    pub vencimiento: Option<NaiveDate>,
    pub cufe: String,
    #[schema(value_type = String)]
    pub subtotal: Decimal,
    #[schema(value_type = String)]
    pub iva: Decimal,
    #[schema(value_type = String)]
    pub total: Decimal,
    #[schema(value_type = crate::esquemas::EstadoFacturaRecibida)]
    pub estado: EstadoFacturaRecibida,
    /// Proveedor de la entidad con ese NIT, si existe.
    pub persona_id: Option<Uuid>,
    pub factura_proveedor_id: Option<Uuid>,
    pub motivo_rechazo: Option<String>,
    pub recibida_en: DateTime<Utc>,
}

impl From<FacturaRecibida> for FacturaRecibidaRespuesta {
    fn from(f: FacturaRecibida) -> Self {
        Self {
            id: f.id.as_uuid(),
            archivo_id: f.archivo_id.as_uuid(),
            emisor_nit: f.emisor_nit,
            emisor_nombre: f.emisor_nombre,
            prefijo: f.prefijo,
            numero: f.numero,
            fecha: f.fecha,
            vencimiento: f.vencimiento,
            cufe: f.cufe,
            subtotal: f.subtotal.valor(),
            iva: f.iva.valor(),
            total: f.total.valor(),
            estado: f.estado,
            persona_id: f.persona_id.map(|p| p.as_uuid()),
            factura_proveedor_id: f.factura_proveedor_id.map(|x| x.as_uuid()),
            motivo_rechazo: f.motivo_rechazo,
            recibida_en: f.recibida_en,
        }
    }
}

/// Evento RADIAN registrado.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventoRadianRespuesta {
    pub id: Uuid,
    #[schema(value_type = crate::esquemas::EventoRadian)]
    pub tipo: EventoRadian,
    /// `enviado` o `rechazado`.
    pub estado: String,
    pub motivo: Option<String>,
    pub respuesta: Option<String>,
    pub creado_en: DateTime<Utc>,
}

impl From<RegistroRadian> for EventoRadianRespuesta {
    fn from(e: RegistroRadian) -> Self {
        Self {
            id: e.id,
            tipo: e.tipo,
            estado: e.estado,
            motivo: e.motivo,
            respuesta: e.respuesta,
            creado_en: e.creado_en,
        }
    }
}

/// Factura recibida con sus eventos RADIAN.
#[derive(Debug, Serialize, ToSchema)]
pub struct FacturaRecibidaDetalleRespuesta {
    pub factura: FacturaRecibidaRespuesta,
    pub eventos: Vec<EventoRadianRespuesta>,
}

/// Factura de proveedor registrada que corresponde a la recibida (mismo CUFE).
#[derive(Debug, Deserialize, ToSchema)]
pub struct VincularPeticion {
    pub factura_proveedor_id: Uuid,
}

/// Motivo de un rechazo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RechazoPeticion {
    #[schema(example = "La factura no corresponde a un servicio prestado")]
    pub motivo: String,
}

/// Evento RADIAN a registrar.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EventoRadianPeticion {
    #[schema(value_type = crate::esquemas::EventoRadian)]
    pub tipo: EventoRadian,
    /// Obligatorio en un reclamo.
    pub motivo: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Preparación de declaraciones, exógena y RUB
// ---------------------------------------------------------------------------------------------

/// Retenciones de un impuesto, concepto y tarifa.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResumenRetencionRespuesta {
    /// `renta`, `iva` o `ica`.
    pub impuesto: String,
    pub concepto: String,
    #[schema(value_type = String)]
    pub tarifa: Decimal,
    #[schema(value_type = String)]
    pub base: Decimal,
    #[schema(value_type = String)]
    pub valor: Decimal,
}

impl From<ResumenRetencion> for ResumenRetencionRespuesta {
    fn from(r: ResumenRetencion) -> Self {
        Self {
            impuesto: r.impuesto,
            concepto: r.concepto,
            tarifa: r.tarifa,
            base: r.base,
            valor: r.valor,
        }
    }
}

/// Preparación de la retención en la fuente con su conciliación contra el mayor.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreparacionRetencionRespuesta {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    pub detalle: Vec<ResumenRetencionRespuesta>,
    #[schema(value_type = String)]
    pub total_practicado: Decimal,
    /// Neto de las cuentas de retenciones en el mayor.
    #[schema(value_type = String)]
    pub total_mayor: Decimal,
    /// Mayor − practicado (cero si cuadra).
    #[schema(value_type = String)]
    pub diferencia: Decimal,
    /// Agente de retención con retenciones en el periodo.
    pub obligada: bool,
}

impl From<PreparacionRetencion> for PreparacionRetencionRespuesta {
    fn from(p: PreparacionRetencion) -> Self {
        Self {
            desde: p.desde,
            hasta: p.hasta,
            detalle: p.detalle.into_iter().map(Into::into).collect(),
            total_practicado: p.total_practicado,
            total_mayor: p.total_mayor,
            diferencia: p.diferencia,
            obligada: p.obligada,
        }
    }
}

/// IVA de una tarifa.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResumenIvaRespuesta {
    #[schema(value_type = String)]
    pub tarifa: Decimal,
    #[schema(value_type = String)]
    pub base: Decimal,
    #[schema(value_type = String)]
    pub iva: Decimal,
}

impl From<ResumenIva> for ResumenIvaRespuesta {
    fn from(r: ResumenIva) -> Self {
        Self {
            tarifa: r.tarifa,
            base: r.base,
            iva: r.iva,
        }
    }
}

/// Preparación del IVA.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreparacionIvaRespuesta {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    pub generado: Vec<ResumenIvaRespuesta>,
    pub descontable: Vec<ResumenIvaRespuesta>,
    #[schema(value_type = String)]
    pub total_generado: Decimal,
    #[schema(value_type = String)]
    pub total_descontable: Decimal,
    /// Generado − descontable (positivo = a pagar).
    #[schema(value_type = String)]
    pub saldo: Decimal,
    pub responsable: bool,
}

impl From<PreparacionIva> for PreparacionIvaRespuesta {
    fn from(p: PreparacionIva) -> Self {
        Self {
            desde: p.desde,
            hasta: p.hasta,
            generado: p.generado.into_iter().map(Into::into).collect(),
            descontable: p.descontable.into_iter().map(Into::into).collect(),
            total_generado: p.total_generado,
            total_descontable: p.total_descontable,
            saldo: p.saldo,
            responsable: p.responsable,
        }
    }
}

/// ICA estimado de un municipio y actividad.
#[derive(Debug, Serialize, ToSchema)]
pub struct LineaIcaRespuesta {
    pub municipio_dane: String,
    pub actividad: String,
    /// `null` si el catálogo no tiene la tarifa vigente.
    #[schema(value_type = Option<String>)]
    pub tarifa_por_mil: Option<Decimal>,
    #[schema(value_type = String)]
    pub base: Decimal,
    #[schema(value_type = Option<String>)]
    pub impuesto: Option<Decimal>,
}

impl From<LineaIca> for LineaIcaRespuesta {
    fn from(l: LineaIca) -> Self {
        Self {
            municipio_dane: l.municipio_dane,
            actividad: l.actividad,
            tarifa_por_mil: l.tarifa_por_mil,
            base: l.base,
            impuesto: l.impuesto,
        }
    }
}

/// Preparación del ICA y la ReteICA.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreparacionIcaRespuesta {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    #[schema(value_type = String)]
    pub ingresos: Decimal,
    pub lineas: Vec<LineaIcaRespuesta>,
    #[schema(value_type = String)]
    pub reteica_practicada: Decimal,
}

impl From<PreparacionIca> for PreparacionIcaRespuesta {
    fn from(p: PreparacionIca) -> Self {
        Self {
            desde: p.desde,
            hasta: p.hasta,
            ingresos: p.ingresos,
            lineas: p.lineas.into_iter().map(Into::into).collect(),
            reteica_practicada: p.reteica_practicada,
        }
    }
}

/// Generación de un formato de exógena.
#[derive(Debug, Serialize, ToSchema)]
pub struct GeneracionExogenaRespuesta {
    pub id: Uuid,
    pub formato_id: Uuid,
    pub anio: i32,
    pub version: u32,
    pub filas: u32,
    #[schema(value_type = String)]
    pub total: Decimal,
    pub errores: u32,
    /// Huella del archivo (los mismos datos y la misma versión dan la misma huella).
    pub sha256: String,
    pub archivo_id: Uuid,
    pub generada_en: DateTime<Utc>,
}

impl From<GeneracionExogena> for GeneracionExogenaRespuesta {
    fn from(g: GeneracionExogena) -> Self {
        Self {
            id: g.id.as_uuid(),
            formato_id: g.formato_id.as_uuid(),
            anio: g.anio,
            version: g.version,
            filas: g.filas,
            total: g.total,
            errores: g.errores,
            sha256: g.sha256,
            archivo_id: g.archivo_id.as_uuid(),
            generada_en: g.generada_en,
        }
    }
}

/// Exógena generada con su archivo y los terceros con datos por corregir.
#[derive(Debug, Serialize, ToSchema)]
pub struct ExogenaGeneradaRespuesta {
    pub generacion: GeneracionExogenaRespuesta,
    /// Archivo de preparación (CSV separado por `;`).
    pub csv: String,
    pub errores: Vec<String>,
}

impl From<ExogenaGenerada> for ExogenaGeneradaRespuesta {
    fn from(e: ExogenaGenerada) -> Self {
        Self {
            generacion: e.generacion.into(),
            csv: e.csv,
            errores: e.errores,
        }
    }
}

/// Formato a generar.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GenerarExogenaPeticion {
    pub formato_id: Uuid,
}

/// Beneficiario final (RUB).
#[derive(Debug, Serialize, ToSchema)]
pub struct BeneficiarioRespuesta {
    pub id: Uuid,
    pub persona_id: Uuid,
    #[schema(value_type = crate::esquemas::CriterioBeneficiario)]
    pub criterio: CriterioBeneficiario,
    #[schema(value_type = Option<String>)]
    pub porcentaje: Option<Decimal>,
    pub soporte_archivo_id: Option<Uuid>,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: Option<NaiveDate>,
    pub creado_en: DateTime<Utc>,
}

impl From<BeneficiarioFinal> for BeneficiarioRespuesta {
    fn from(b: BeneficiarioFinal) -> Self {
        Self {
            id: b.id.as_uuid(),
            persona_id: b.persona_id.as_uuid(),
            criterio: b.criterio,
            porcentaje: b.porcentaje,
            soporte_archivo_id: b.soporte_archivo_id.map(|a| a.as_uuid()),
            vigente_desde: b.vigente_desde,
            vigente_hasta: b.vigente_hasta,
            creado_en: b.creado_en,
        }
    }
}

/// Beneficiario final nuevo.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NuevoBeneficiarioPeticion {
    pub persona_id: Uuid,
    #[schema(value_type = crate::esquemas::CriterioBeneficiario)]
    pub criterio: CriterioBeneficiario,
    /// Participación (%) solo con el criterio de propiedad.
    #[schema(value_type = Option<String>, example = "25")]
    pub porcentaje: Option<Decimal>,
    pub soporte_archivo_id: Option<Uuid>,
    pub vigente_desde: NaiveDate,
}

/// Fecha en que termina una vigencia.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TerminarPeticion {
    pub hasta: NaiveDate,
}
