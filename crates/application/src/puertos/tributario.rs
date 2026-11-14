//! Puertos del plan 15 (tributario y documentos electrónicos, ADR 0020): catálogos tributarios
//! de plataforma, perfil tributario, resoluciones de numeración, documentos electrónicos con su
//! proveedor, facturas recibidas, eventos RADIAN, exógena y RUB. Genéricos: sirven a cualquier
//! entidad (la prueba `tests/fronteras.rs` impide que importen la vertical PH).

use std::future::Future;

use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, BeneficiarioFinalId, CuentaId, DocumentoElectronicoId, FacturaProveedorId,
    FacturaRecibidaId, FormatoExogenaId, GeneracionExogenaId, PerfilTributarioId, PersonaId,
    ResolucionNumeracionId, TarifaIcaId, TarifaIvaId,
    contabilidad::{
        Monto,
        tributario::{
            CriterioBeneficiario, EstadoDocumentoElectronico, EstadoFacturaRecibida,
            EstadoPerfilTributario, EventoRadian, LadoExogena, ParametrosPerfil,
            TipoDocumentoElectronico, TipoNumeracion, TratamientoIva,
        },
    },
};
use rust_decimal::Decimal;

use super::contabilidad::Origen;
use crate::AppResult;

// ---------------------------------------------------------------------------------------------
// Catálogos de plataforma
// ---------------------------------------------------------------------------------------------

/// Versión de una tarifa de IVA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarifaIva {
    pub id: TarifaIvaId,
    pub codigo: String,
    pub nombre: String,
    pub tratamiento: TratamientoIva,
    /// Porcentaje (positivo solo si es gravado).
    pub tarifa: Decimal,
    pub fuente: String,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: Option<NaiveDate>,
}

/// Versión de una tarifa de ICA por municipio y actividad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarifaIca {
    pub id: TarifaIcaId,
    pub municipio_dane: String,
    /// Código CIIU.
    pub actividad: String,
    pub nombre: String,
    pub tarifa_por_mil: Decimal,
    pub fuente: String,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: Option<NaiveDate>,
}

/// Cuentas que suman a un concepto y columna de un formato de exógena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampoExogena {
    pub concepto: String,
    pub cuenta_prefijo: String,
    pub columna: String,
    pub lado: LadoExogena,
}

/// Versión de un formato de información exógena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatoExogena {
    pub id: FormatoExogenaId,
    pub anio: i32,
    pub codigo: String,
    pub version: u32,
    pub nombre: String,
    pub resolucion: String,
    pub cuantia_menor: Decimal,
    pub campos: Vec<CampoExogena>,
    pub creado_en: DateTime<Utc>,
}

/// Lectura de los catálogos tributarios (desde cualquier transacción).
pub trait CatalogoTributario: Send {
    /// Tarifas de IVA (vigentes en la fecha, o todas las versiones) por código.
    fn tarifas_iva(
        &mut self,
        vigentes_en: Option<NaiveDate>,
    ) -> impl Future<Output = AppResult<Vec<TarifaIva>>> + Send;

    /// Tarifas de ICA (vigentes en la fecha, o todas) por municipio y actividad.
    fn tarifas_ica(
        &mut self,
        vigentes_en: Option<NaiveDate>,
    ) -> impl Future<Output = AppResult<Vec<TarifaIca>>> + Send;

    /// Formatos de exógena (del año, o todos) con sus campos, la versión más alta primero.
    fn formatos_exogena(
        &mut self,
        anio: Option<i32>,
    ) -> impl Future<Output = AppResult<Vec<FormatoExogena>>> + Send;
}

/// Escritura de los catálogos tributarios (solo en la transacción de plataforma).
pub trait CatalogoTributarioPlataforma: CatalogoTributario {
    /// Guarda una versión y cierra la vigencia de la anterior del mismo código el día antes.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay una versión con esa fecha.
    fn insertar_tarifa_iva(&mut self, t: &TarifaIva) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda una versión y cierra la anterior del mismo municipio y actividad.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya hay una versión con esa fecha.
    fn insertar_tarifa_ica(&mut self, t: &TarifaIca) -> impl Future<Output = AppResult<()>> + Send;

    /// Guarda una versión de un formato con sus campos.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya existe esa versión.
    fn insertar_formato_exogena(
        &mut self,
        f: &FormatoExogena,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Datos de la entidad
// ---------------------------------------------------------------------------------------------

/// Versión del perfil tributario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerfilTributario {
    pub id: PerfilTributarioId,
    pub version: u32,
    pub vigente_desde: NaiveDate,
    pub estado: EstadoPerfilTributario,
    pub parametros: ParametrosPerfil,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub observaciones: Option<String>,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
    pub confirmado_por: Option<CuentaId>,
    pub confirmado_en: Option<DateTime<Utc>>,
}

/// Resolución de numeración de la DIAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolucionNumeracion {
    pub id: ResolucionNumeracionId,
    pub tipo: TipoNumeracion,
    pub prefijo: String,
    pub desde: u64,
    pub hasta: u64,
    pub numero_resolucion: String,
    pub fecha_resolucion: NaiveDate,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: NaiveDate,
    /// Secreto: no sale en la API.
    pub clave_tecnica: Option<String>,
    pub activa: bool,
    pub creada_en: DateTime<Utc>,
}

/// Documento electrónico.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentoElectronico {
    pub id: DocumentoElectronicoId,
    pub tipo: TipoDocumentoElectronico,
    pub origen: Origen,
    pub resolucion_id: Option<ResolucionNumeracionId>,
    pub prefijo: String,
    pub numero: Option<u64>,
    /// Documento al que se refiere una nota.
    pub referencia_id: Option<DocumentoElectronicoId>,
    pub cufe: Option<String>,
    pub estado: EstadoDocumentoElectronico,
    /// Adaptador que lo transmitió.
    pub proveedor: String,
    pub id_externo: Option<String>,
    pub xml_archivo_id: Option<ArchivoId>,
    pub respuesta: Option<String>,
    pub intentos: u32,
    pub total: Monto,
    pub creado_en: DateTime<Utc>,
    pub enviado_en: Option<DateTime<Utc>>,
    pub validado_en: Option<DateTime<Utc>>,
}

/// Evento (append-only) de un documento electrónico.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventoDocumento {
    pub tipo: String,
    pub detalle: Option<String>,
    pub ocurrido_en: DateTime<Utc>,
}

/// Filtro de documentos electrónicos.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroDocumentos {
    pub tipo: Option<TipoDocumentoElectronico>,
    pub estado: Option<EstadoDocumentoElectronico>,
}

/// Factura electrónica recibida de un proveedor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacturaRecibida {
    pub id: FacturaRecibidaId,
    pub archivo_id: ArchivoId,
    pub sha256: String,
    pub emisor_nit: String,
    pub emisor_nombre: String,
    pub prefijo: String,
    pub numero: String,
    pub fecha: NaiveDate,
    pub vencimiento: Option<NaiveDate>,
    pub cufe: String,
    pub subtotal: Monto,
    pub iva: Monto,
    pub total: Monto,
    pub estado: EstadoFacturaRecibida,
    pub persona_id: Option<PersonaId>,
    pub factura_proveedor_id: Option<FacturaProveedorId>,
    pub motivo_rechazo: Option<String>,
    pub recibida_por: Option<CuentaId>,
    pub recibida_en: DateTime<Utc>,
}

/// Evento RADIAN registrado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistroRadian {
    pub id: uuid::Uuid,
    pub factura_recibida_id: FacturaRecibidaId,
    pub tipo: EventoRadian,
    /// `enviado` o `rechazado`.
    pub estado: String,
    pub id_externo: Option<String>,
    pub motivo: Option<String>,
    pub respuesta: Option<String>,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
}

/// Movimiento anual de una cuenta por tercero (para la exógena).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovimientoTercero {
    pub cuenta_codigo: String,
    pub persona_id: Option<PersonaId>,
    /// Documento del tercero (NIT con DV si lo tiene, o su documento de identidad).
    pub documento: Option<String>,
    pub nombre: String,
    pub debitos: Decimal,
    pub creditos: Decimal,
}

/// Generación de un formato de exógena.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneracionExogena {
    pub id: GeneracionExogenaId,
    pub formato_id: FormatoExogenaId,
    pub anio: i32,
    pub version: u32,
    pub filas: u32,
    pub total: Decimal,
    pub errores: u32,
    pub sha256: String,
    pub archivo_id: ArchivoId,
    pub generada_por: Option<CuentaId>,
    pub generada_en: DateTime<Utc>,
}

/// Beneficiario final (RUB).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeneficiarioFinal {
    pub id: BeneficiarioFinalId,
    pub persona_id: PersonaId,
    pub criterio: CriterioBeneficiario,
    pub porcentaje: Option<Decimal>,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: Option<NaiveDate>,
    pub creado_por: Option<CuentaId>,
    pub creado_en: DateTime<Utc>,
}

/// Retenciones practicadas en un lapso (impuesto, concepto y tarifa).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumenRetencion {
    /// `renta`, `iva` o `ica`.
    pub impuesto: String,
    pub concepto: String,
    pub tarifa: Decimal,
    pub base: Decimal,
    pub valor: Decimal,
}

/// IVA de un lapso por tarifa: generado en ventas o descontable en compras.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumenIva {
    pub tarifa: Decimal,
    pub base: Decimal,
    pub iva: Decimal,
}

/// Persistencia tributaria de la entidad.
pub trait RepositorioTributario: Send {
    /// Versiones del perfil, la más reciente primero.
    fn perfiles_tributarios(
        &mut self,
    ) -> impl Future<Output = AppResult<Vec<PerfilTributario>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si la versión ya existe.
    fn insertar_perfil_tributario(
        &mut self,
        p: &PerfilTributario,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Marca confirmada una versión en borrador.
    fn confirmar_perfil_tributario(
        &mut self,
        p: &PerfilTributario,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Resoluciones por tipo y fecha.
    fn resoluciones(&mut self)
    -> impl Future<Output = AppResult<Vec<ResolucionNumeracion>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si ya existe el rango.
    fn insertar_resolucion(
        &mut self,
        r: &ResolucionNumeracion,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Activa o desactiva una resolución.
    fn activar_resolucion(
        &mut self,
        id: ResolucionNumeracionId,
        activa: bool,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Último número usado de la resolución (bloquea la resolución para numerar en orden).
    fn ultimo_numero(
        &mut self,
        resolucion: ResolucionNumeracionId,
    ) -> impl Future<Output = AppResult<Option<u64>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el documento de negocio ya tiene uno de ese tipo o el
    /// número ya se usó.
    fn insertar_documento(
        &mut self,
        d: &DocumentoElectronico,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Documento por id, bloqueado para actualizar.
    fn documento(
        &mut self,
        id: DocumentoElectronicoId,
    ) -> impl Future<Output = AppResult<Option<DocumentoElectronico>>> + Send;

    /// Documento de un tipo para un documento de negocio.
    fn documento_por_origen(
        &mut self,
        tipo: TipoDocumentoElectronico,
        origen: &Origen,
    ) -> impl Future<Output = AppResult<Option<DocumentoElectronico>>> + Send;

    /// Documentos por fecha descendente, a lo sumo `limite`.
    fn documentos(
        &mut self,
        filtro: &FiltroDocumentos,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<DocumentoElectronico>>> + Send;

    /// Guarda estado, número, CUFE, respuesta, intentos y fechas.
    fn actualizar_documento(
        &mut self,
        d: &DocumentoElectronico,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Agrega un evento al documento.
    fn insertar_evento_documento(
        &mut self,
        documento: DocumentoElectronicoId,
        e: &EventoDocumento,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Eventos del documento por fecha.
    fn eventos_documento(
        &mut self,
        documento: DocumentoElectronicoId,
    ) -> impl Future<Output = AppResult<Vec<EventoDocumento>>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el CUFE o el archivo ya se cargaron.
    fn insertar_factura_recibida(
        &mut self,
        f: &FacturaRecibida,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Factura recibida por id, bloqueada para actualizar.
    fn factura_recibida(
        &mut self,
        id: FacturaRecibidaId,
    ) -> impl Future<Output = AppResult<Option<FacturaRecibida>>> + Send;

    /// Facturas recibidas (del estado, si se da), las más recientes primero.
    fn facturas_recibidas(
        &mut self,
        estado: Option<EstadoFacturaRecibida>,
        limite: u32,
    ) -> impl Future<Output = AppResult<Vec<FacturaRecibida>>> + Send;

    /// Guarda estado, factura de proveedor, persona y motivo de rechazo.
    fn actualizar_factura_recibida(
        &mut self,
        f: &FacturaRecibida,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// # Errors
    ///
    /// [`crate::AppError::Conflicto`] si el evento ya se registró.
    fn insertar_evento_radian(
        &mut self,
        e: &RegistroRadian,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Eventos RADIAN de una factura recibida por fecha.
    fn eventos_radian(
        &mut self,
        factura: FacturaRecibidaId,
    ) -> impl Future<Output = AppResult<Vec<RegistroRadian>>> + Send;

    /// Movimientos del año por cuenta y tercero de las cuentas que empiezan por los prefijos.
    fn movimientos_por_tercero(
        &mut self,
        anio: i32,
        prefijos: &[String],
    ) -> impl Future<Output = AppResult<Vec<MovimientoTercero>>> + Send;

    /// Guarda una generación de exógena (append-only).
    fn insertar_generacion_exogena(
        &mut self,
        g: &GeneracionExogena,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Generaciones (del año, si se da), las más recientes primero.
    fn generaciones_exogena(
        &mut self,
        anio: Option<i32>,
    ) -> impl Future<Output = AppResult<Vec<GeneracionExogena>>> + Send;

    /// Beneficiarios finales (los vigentes primero).
    fn beneficiarios_finales(
        &mut self,
    ) -> impl Future<Output = AppResult<Vec<BeneficiarioFinal>>> + Send;

    /// Guarda un beneficiario final.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::NoEncontrado`] si la persona o el soporte no existen.
    fn insertar_beneficiario(
        &mut self,
        b: &BeneficiarioFinal,
    ) -> impl Future<Output = AppResult<()>> + Send;

    /// Termina la vigencia de un beneficiario.
    fn terminar_beneficiario(
        &mut self,
        id: BeneficiarioFinalId,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Retenciones practicadas en facturas de proveedor causadas entre dos fechas.
    fn resumen_retenciones(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<Vec<ResumenRetencion>>> + Send;

    /// IVA generado en facturas de venta emitidas entre dos fechas, por tarifa.
    fn iva_generado(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<Vec<ResumenIva>>> + Send;

    /// IVA descontable en facturas de proveedor causadas entre dos fechas (base = la de las
    /// líneas con IVA descontable).
    fn iva_descontable(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<Vec<ResumenIva>>> + Send;

    /// Ingresos facturados (subtotal de las facturas de venta emitidas) entre dos fechas.
    fn ingresos_facturados(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> impl Future<Output = AppResult<Decimal>> + Send;
}

// ---------------------------------------------------------------------------------------------
// Proveedor de facturación y lector de facturas (adaptadores en `infra-facturacion`)
// ---------------------------------------------------------------------------------------------

/// Parte de un documento electrónico (emisor o adquirente).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParteDocumento {
    /// NIT (sin DV) o documento de identidad.
    pub documento: String,
    pub dv: Option<String>,
    pub nombre: String,
    pub correo: Option<String>,
}

/// Línea de un documento electrónico.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineaDocumento {
    pub descripcion: String,
    pub cantidad: Decimal,
    pub valor_unitario: Decimal,
    pub subtotal: Decimal,
    pub tarifa_iva: Decimal,
    pub iva: Decimal,
}

/// Documento de negocio listo para transmitir: el puerto no conoce formatos del proveedor (ni el
/// UBL), así que sirve igual para un proveedor tecnológico que para el motor propio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentoAEmitir {
    pub id: DocumentoElectronicoId,
    pub tipo: TipoDocumentoElectronico,
    pub prefijo: String,
    pub numero: u64,
    pub fecha: NaiveDate,
    pub emisor: ParteDocumento,
    pub adquirente: ParteDocumento,
    pub lineas: Vec<LineaDocumento>,
    pub subtotal: Decimal,
    pub iva: Decimal,
    pub total: Decimal,
    /// Clave técnica de la resolución (factura).
    pub clave_tecnica: Option<String>,
    /// CUFE del documento al que se refiere una nota.
    pub referencia_cufe: Option<String>,
}

/// Respuesta del proveedor a una transmisión o consulta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RespuestaProveedor {
    pub estado: EstadoDocumentoElectronico,
    pub id_externo: Option<String>,
    pub cufe: Option<String>,
    /// XML firmado del documento, si lo entrega.
    pub xml: Option<Vec<u8>>,
    pub mensaje: Option<String>,
}

/// Proveedor de facturación electrónica (proveedor tecnológico o motor propio).
pub trait ProveedorFacturacion: Send + Sync {
    /// Código del adaptador (`simulado`, el del proveedor, `propio`).
    fn nombre(&self) -> &'static str;

    /// Transmite el documento. La misma `id` dos veces debe devolver el mismo resultado
    /// (idempotente: un solo CUFE).
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Infraestructura`] si no se pudo transmitir (contingencia).
    fn emitir(
        &self,
        documento: &DocumentoAEmitir,
    ) -> impl Future<Output = AppResult<RespuestaProveedor>> + Send;

    /// Consulta el estado de un documento enviado.
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Infraestructura`] si el proveedor no responde.
    fn consultar(
        &self,
        id_externo: &str,
    ) -> impl Future<Output = AppResult<RespuestaProveedor>> + Send;

    /// Registra un evento RADIAN sobre una factura recibida (por su CUFE).
    ///
    /// # Errors
    ///
    /// [`crate::AppError::Infraestructura`] si el proveedor no responde.
    fn evento_radian(
        &self,
        cufe: &str,
        evento: EventoRadian,
        motivo: Option<&str>,
    ) -> impl Future<Output = AppResult<RespuestaProveedor>> + Send;
}

/// Datos de una factura electrónica leídos de su XML (UBL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacturaLeida {
    pub emisor_nit: String,
    pub emisor_nombre: String,
    pub prefijo: String,
    pub numero: String,
    pub fecha: NaiveDate,
    pub vencimiento: Option<NaiveDate>,
    pub cufe: String,
    pub subtotal: Decimal,
    pub iva: Decimal,
    pub total: Decimal,
}

/// Lee una factura electrónica (XML UBL o el `AttachedDocument` que la envuelve).
pub trait LectorFacturas: Send + Sync {
    /// # Errors
    ///
    /// [`crate::AppError::Validacion`] si el archivo no es una factura electrónica legible.
    fn leer(&self, contenido: &[u8]) -> AppResult<FacturaLeida>;
}
