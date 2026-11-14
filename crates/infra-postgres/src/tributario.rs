//! Tributario y documentos electrónicos sobre PostgreSQL (plan 15, ADR 0020): catálogos de IVA,
//! ICA y exógena (lectura en cualquier transacción; escritura en la de plataforma), perfil
//! tributario, resoluciones de numeración, documentos electrónicos con sus eventos, facturas
//! recibidas con RADIAN, generaciones de exógena, beneficiarios finales y los resúmenes que
//! preparan las declaraciones.

use application::{
    AppError, AppResult,
    puertos::{
        contabilidad::Origen,
        tributario::{
            BeneficiarioFinal, CampoExogena, CatalogoTributario, CatalogoTributarioPlataforma,
            DocumentoElectronico, EventoDocumento, FacturaRecibida, FiltroDocumentos,
            FormatoExogena, GeneracionExogena, MovimientoTercero, PerfilTributario, RegistroRadian,
            RepositorioTributario, ResolucionNumeracion, ResumenIva, ResumenRetencion, TarifaIca,
            TarifaIva,
        },
    },
};
use chrono::{DateTime, NaiveDate, Utc};
use domain::{
    ArchivoId, BeneficiarioFinalId, CuentaId, DocumentoElectronicoId, FacturaProveedorId,
    FacturaRecibidaId, FormatoExogenaId, GeneracionExogenaId, PerfilTributarioId, PersonaId,
    ResolucionNumeracionId, TarifaIcaId, TarifaIvaId,
    contabilidad::{
        Monto, ResponsabilidadFiscal,
        tributario::{
            CanalFacturacion, CriterioBeneficiario, EstadoDocumentoElectronico,
            EstadoFacturaRecibida, EstadoPerfilTributario, EventoRadian, LadoExogena,
            ParametrosPerfil, TipoDocumentoElectronico, TipoNumeracion, TratamientoIva,
        },
    },
};
use rust_decimal::Decimal;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{
    TransaccionPg,
    contabilidad::{error_contable, u32_de},
    plataforma::TransaccionPlataformaPg,
};

fn monto(v: Decimal) -> AppResult<Monto> {
    Ok(Monto::new(v)?)
}

fn u64_de(v: i64, campo: &'static str) -> AppResult<u64> {
    u64::try_from(v).map_err(|_| AppError::Validacion(format!("`{campo}` fuera de rango")))
}

fn i64_de(v: u64, campo: &'static str) -> AppResult<i64> {
    i64::try_from(v).map_err(|_| AppError::Validacion(format!("`{campo}` fuera de rango")))
}

fn i32_de(v: u32, campo: &'static str) -> AppResult<i32> {
    i32::try_from(v).map_err(|_| AppError::Validacion(format!("`{campo}` fuera de rango")))
}

fn i16_anio(v: i32) -> AppResult<i16> {
    i16::try_from(v).map_err(|_| AppError::Validacion("`anio` fuera de rango".into()))
}

// ---------------------------------------------------------------------------------------------
// Catálogos de plataforma
// ---------------------------------------------------------------------------------------------

async fn tarifas_iva_de(
    c: &mut PgConnection,
    vigentes_en: Option<NaiveDate>,
) -> AppResult<Vec<TarifaIva>> {
    sqlx::query!(
        "SELECT id, codigo, nombre, tratamiento, tarifa, fuente, vigente_desde, vigente_hasta
         FROM tarifas_iva
         WHERE $1::date IS NULL
            OR (vigente_desde <= $1 AND (vigente_hasta IS NULL OR vigente_hasta >= $1))
         ORDER BY codigo, vigente_desde DESC",
        vigentes_en,
    )
    .fetch_all(c)
    .await
    .map_err(AppError::infra)?
    .into_iter()
    .map(|t| {
        Ok(TarifaIva {
            id: TarifaIvaId::from_uuid(t.id),
            codigo: t.codigo,
            nombre: t.nombre,
            tratamiento: TratamientoIva::try_from(t.tratamiento.as_str())?,
            tarifa: t.tarifa,
            fuente: t.fuente,
            vigente_desde: t.vigente_desde,
            vigente_hasta: t.vigente_hasta,
        })
    })
    .collect()
}

async fn tarifas_ica_de(
    c: &mut PgConnection,
    vigentes_en: Option<NaiveDate>,
) -> AppResult<Vec<TarifaIca>> {
    Ok(sqlx::query!(
        "SELECT id, municipio_dane, actividad, nombre, tarifa_por_mil, fuente, vigente_desde,
                vigente_hasta
         FROM tarifas_ica
         WHERE $1::date IS NULL
            OR (vigente_desde <= $1 AND (vigente_hasta IS NULL OR vigente_hasta >= $1))
         ORDER BY municipio_dane, actividad, vigente_desde DESC",
        vigentes_en,
    )
    .fetch_all(c)
    .await
    .map_err(AppError::infra)?
    .into_iter()
    .map(|t| TarifaIca {
        id: TarifaIcaId::from_uuid(t.id),
        municipio_dane: t.municipio_dane,
        actividad: t.actividad,
        nombre: t.nombre,
        tarifa_por_mil: t.tarifa_por_mil,
        fuente: t.fuente,
        vigente_desde: t.vigente_desde,
        vigente_hasta: t.vigente_hasta,
    })
    .collect())
}

async fn formatos_de(c: &mut PgConnection, anio: Option<i32>) -> AppResult<Vec<FormatoExogena>> {
    let anio = anio.map(i16_anio).transpose()?;
    let filas = sqlx::query!(
        "SELECT id, anio, codigo, version, nombre, resolucion, cuantia_menor, creado_en
         FROM formatos_exogena
         WHERE $1::smallint IS NULL OR anio = $1
         ORDER BY anio DESC, codigo, version DESC",
        anio,
    )
    .fetch_all(&mut *c)
    .await
    .map_err(AppError::infra)?;
    let ids: Vec<Uuid> = filas.iter().map(|f| f.id).collect();
    let campos = sqlx::query!(
        "SELECT formato_id, concepto, cuenta_prefijo, columna, lado
         FROM campos_formato_exogena WHERE formato_id = ANY($1)
         ORDER BY concepto, cuenta_prefijo, columna",
        &ids,
    )
    .fetch_all(&mut *c)
    .await
    .map_err(AppError::infra)?;
    filas
        .into_iter()
        .map(|f| {
            let campos = campos
                .iter()
                .filter(|x| x.formato_id == f.id)
                .map(|x| {
                    Ok(CampoExogena {
                        concepto: x.concepto.clone(),
                        cuenta_prefijo: x.cuenta_prefijo.clone(),
                        columna: x.columna.clone(),
                        lado: LadoExogena::try_from(x.lado.as_str())?,
                    })
                })
                .collect::<AppResult<Vec<_>>>()?;
            Ok(FormatoExogena {
                id: FormatoExogenaId::from_uuid(f.id),
                anio: i32::from(f.anio),
                codigo: f.codigo,
                version: u32_de(f.version, "version")?,
                nombre: f.nombre,
                resolucion: f.resolucion,
                cuantia_menor: f.cuantia_menor,
                campos,
                creado_en: f.creado_en,
            })
        })
        .collect()
}

/// Implementa la lectura de los catálogos sobre la conexión de la transacción.
macro_rules! catalogo_tributario {
    ($tipo:ty) => {
        impl CatalogoTributario for $tipo {
            async fn tarifas_iva(
                &mut self,
                vigentes_en: Option<NaiveDate>,
            ) -> AppResult<Vec<TarifaIva>> {
                tarifas_iva_de(&mut self.tx, vigentes_en).await
            }

            async fn tarifas_ica(
                &mut self,
                vigentes_en: Option<NaiveDate>,
            ) -> AppResult<Vec<TarifaIca>> {
                tarifas_ica_de(&mut self.tx, vigentes_en).await
            }

            async fn formatos_exogena(
                &mut self,
                anio: Option<i32>,
            ) -> AppResult<Vec<FormatoExogena>> {
                formatos_de(&mut self.tx, anio).await
            }
        }
    };
}

catalogo_tributario!(TransaccionPg);
catalogo_tributario!(TransaccionPlataformaPg);

impl CatalogoTributarioPlataforma for TransaccionPlataformaPg {
    async fn insertar_tarifa_iva(&mut self, t: &TarifaIva) -> AppResult<()> {
        sqlx::query!(
            "UPDATE tarifas_iva SET vigente_hasta = $2::date - 1
             WHERE codigo = $1 AND vigente_desde < $2
               AND (vigente_hasta IS NULL OR vigente_hasta >= $2)",
            t.codigo,
            t.vigente_desde,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query!(
            "INSERT INTO tarifas_iva (id, codigo, nombre, tratamiento, tarifa, fuente,
                 vigente_desde, vigente_hasta)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            t.id.as_uuid(),
            t.codigo,
            t.nombre,
            t.tratamiento.as_str(),
            t.tarifa,
            t.fuente,
            t.vigente_desde,
            t.vigente_hasta,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(e, "ya hay una versión de la tarifa con esa fecha", "tarifa")
        })?;
        Ok(())
    }

    async fn insertar_tarifa_ica(&mut self, t: &TarifaIca) -> AppResult<()> {
        sqlx::query!(
            "UPDATE tarifas_ica SET vigente_hasta = $3::date - 1
             WHERE municipio_dane = $1 AND actividad = $2 AND vigente_desde < $3
               AND (vigente_hasta IS NULL OR vigente_hasta >= $3)",
            t.municipio_dane,
            t.actividad,
            t.vigente_desde,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        sqlx::query!(
            "INSERT INTO tarifas_ica (id, municipio_dane, actividad, nombre, tarifa_por_mil,
                 fuente, vigente_desde, vigente_hasta)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            t.id.as_uuid(),
            t.municipio_dane,
            t.actividad,
            t.nombre,
            t.tarifa_por_mil,
            t.fuente,
            t.vigente_desde,
            t.vigente_hasta,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(e, "ya hay una versión de la tarifa con esa fecha", "tarifa")
        })?;
        Ok(())
    }

    async fn insertar_formato_exogena(&mut self, f: &FormatoExogena) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO formatos_exogena (id, anio, codigo, version, nombre, resolucion,
                 cuantia_menor, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            f.id.as_uuid(),
            i16_anio(f.anio)?,
            f.codigo,
            i32_de(f.version, "version")?,
            f.nombre,
            f.resolucion,
            f.cuantia_menor,
            f.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "esa versión del formato ya existe", "formato"))?;
        for c in &f.campos {
            sqlx::query!(
                "INSERT INTO campos_formato_exogena (formato_id, concepto, cuenta_prefijo,
                     columna, lado)
                 VALUES ($1, $2, $3, $4, $5)",
                f.id.as_uuid(),
                c.concepto,
                c.cuenta_prefijo,
                c.columna,
                c.lado.as_str(),
            )
            .execute(&mut *self.tx)
            .await
            .map_err(|e| error_contable(e, "campo repetido en el formato", "formato"))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Filas
// ---------------------------------------------------------------------------------------------

struct FilaPerfil {
    id: Uuid,
    version: i32,
    vigente_desde: NaiveDate,
    estado: String,
    responsabilidades: Vec<String>,
    agente_retenedor: bool,
    responsable_iva: bool,
    gran_contribuyente: bool,
    autorretenedor: bool,
    contribuyente_renta: bool,
    facturador_electronico: bool,
    obligado_rub: bool,
    municipios_ica: Vec<String>,
    actividades_ica: Vec<String>,
    canal_facturacion: String,
    soporte_archivo_id: Option<Uuid>,
    observaciones: Option<String>,
    creado_por: Option<Uuid>,
    creado_en: DateTime<Utc>,
    confirmado_por: Option<Uuid>,
    confirmado_en: Option<DateTime<Utc>>,
}

impl FilaPerfil {
    fn convertir(self) -> AppResult<PerfilTributario> {
        Ok(PerfilTributario {
            id: PerfilTributarioId::from_uuid(self.id),
            version: u32_de(self.version, "version")?,
            vigente_desde: self.vigente_desde,
            estado: EstadoPerfilTributario::try_from(self.estado.as_str())?,
            parametros: ParametrosPerfil {
                responsabilidades: self
                    .responsabilidades
                    .iter()
                    .map(|r| ResponsabilidadFiscal::try_from(r.as_str()))
                    .collect::<Result<_, _>>()?,
                agente_retenedor: self.agente_retenedor,
                responsable_iva: self.responsable_iva,
                gran_contribuyente: self.gran_contribuyente,
                autorretenedor: self.autorretenedor,
                contribuyente_renta: self.contribuyente_renta,
                facturador_electronico: self.facturador_electronico,
                obligado_rub: self.obligado_rub,
                municipios_ica: self.municipios_ica,
                actividades_ica: self.actividades_ica,
                canal_facturacion: CanalFacturacion::try_from(self.canal_facturacion.as_str())?,
            },
            soporte_archivo_id: self.soporte_archivo_id.map(ArchivoId::from_uuid),
            observaciones: self.observaciones,
            creado_por: self.creado_por.map(CuentaId::from_uuid),
            creado_en: self.creado_en,
            confirmado_por: self.confirmado_por.map(CuentaId::from_uuid),
            confirmado_en: self.confirmado_en,
        })
    }
}

struct FilaResolucion {
    id: Uuid,
    tipo_documento: String,
    prefijo: String,
    desde: i64,
    hasta: i64,
    numero_resolucion: String,
    fecha_resolucion: NaiveDate,
    vigente_desde: NaiveDate,
    vigente_hasta: NaiveDate,
    clave_tecnica: Option<String>,
    activa: bool,
    creada_en: DateTime<Utc>,
}

impl FilaResolucion {
    fn convertir(self) -> AppResult<ResolucionNumeracion> {
        Ok(ResolucionNumeracion {
            id: ResolucionNumeracionId::from_uuid(self.id),
            tipo: TipoNumeracion::try_from(self.tipo_documento.as_str())?,
            prefijo: self.prefijo,
            desde: u64_de(self.desde, "desde")?,
            hasta: u64_de(self.hasta, "hasta")?,
            numero_resolucion: self.numero_resolucion,
            fecha_resolucion: self.fecha_resolucion,
            vigente_desde: self.vigente_desde,
            vigente_hasta: self.vigente_hasta,
            clave_tecnica: self.clave_tecnica,
            activa: self.activa,
            creada_en: self.creada_en,
        })
    }
}

struct FilaDocumento {
    id: Uuid,
    tipo: String,
    origen_tipo: String,
    origen_id: Uuid,
    resolucion_id: Option<Uuid>,
    prefijo: String,
    numero: Option<i64>,
    referencia_id: Option<Uuid>,
    cufe: Option<String>,
    estado: String,
    proveedor: String,
    id_externo: Option<String>,
    xml_archivo_id: Option<Uuid>,
    respuesta: Option<String>,
    intentos: i32,
    total: Decimal,
    creado_en: DateTime<Utc>,
    enviado_en: Option<DateTime<Utc>>,
    validado_en: Option<DateTime<Utc>>,
}

impl FilaDocumento {
    fn convertir(self) -> AppResult<DocumentoElectronico> {
        Ok(DocumentoElectronico {
            id: DocumentoElectronicoId::from_uuid(self.id),
            tipo: TipoDocumentoElectronico::try_from(self.tipo.as_str())?,
            origen: Origen {
                tipo: self.origen_tipo,
                id: self.origen_id,
            },
            resolucion_id: self.resolucion_id.map(ResolucionNumeracionId::from_uuid),
            prefijo: self.prefijo,
            numero: self.numero.map(|n| u64_de(n, "numero")).transpose()?,
            referencia_id: self.referencia_id.map(DocumentoElectronicoId::from_uuid),
            cufe: self.cufe,
            estado: EstadoDocumentoElectronico::try_from(self.estado.as_str())?,
            proveedor: self.proveedor,
            id_externo: self.id_externo,
            xml_archivo_id: self.xml_archivo_id.map(ArchivoId::from_uuid),
            respuesta: self.respuesta,
            intentos: u32_de(self.intentos, "intentos")?,
            total: monto(self.total)?,
            creado_en: self.creado_en,
            enviado_en: self.enviado_en,
            validado_en: self.validado_en,
        })
    }
}

struct FilaRecibida {
    id: Uuid,
    archivo_id: Uuid,
    sha256: String,
    emisor_nit: String,
    emisor_nombre: String,
    prefijo: String,
    numero: String,
    fecha: NaiveDate,
    vencimiento: Option<NaiveDate>,
    cufe: String,
    subtotal: Decimal,
    iva: Decimal,
    total: Decimal,
    estado: String,
    persona_id: Option<Uuid>,
    factura_proveedor_id: Option<Uuid>,
    motivo_rechazo: Option<String>,
    recibida_por: Option<Uuid>,
    recibida_en: DateTime<Utc>,
}

impl FilaRecibida {
    fn convertir(self) -> AppResult<FacturaRecibida> {
        Ok(FacturaRecibida {
            id: FacturaRecibidaId::from_uuid(self.id),
            archivo_id: ArchivoId::from_uuid(self.archivo_id),
            sha256: self.sha256,
            emisor_nit: self.emisor_nit,
            emisor_nombre: self.emisor_nombre,
            prefijo: self.prefijo,
            numero: self.numero,
            fecha: self.fecha,
            vencimiento: self.vencimiento,
            cufe: self.cufe,
            subtotal: monto(self.subtotal)?,
            iva: monto(self.iva)?,
            total: monto(self.total)?,
            estado: EstadoFacturaRecibida::try_from(self.estado.as_str())?,
            persona_id: self.persona_id.map(PersonaId::from_uuid),
            factura_proveedor_id: self.factura_proveedor_id.map(FacturaProveedorId::from_uuid),
            motivo_rechazo: self.motivo_rechazo,
            recibida_por: self.recibida_por.map(CuentaId::from_uuid),
            recibida_en: self.recibida_en,
        })
    }
}

struct FilaGeneracion {
    id: Uuid,
    formato_id: Uuid,
    anio: i16,
    version: i32,
    filas: i32,
    total: Decimal,
    errores: i32,
    sha256: String,
    archivo_id: Uuid,
    generada_por: Option<Uuid>,
    generada_en: DateTime<Utc>,
}

impl FilaGeneracion {
    fn convertir(self) -> AppResult<GeneracionExogena> {
        Ok(GeneracionExogena {
            id: GeneracionExogenaId::from_uuid(self.id),
            formato_id: FormatoExogenaId::from_uuid(self.formato_id),
            anio: i32::from(self.anio),
            version: u32_de(self.version, "version")?,
            filas: u32_de(self.filas, "filas")?,
            total: self.total,
            errores: u32_de(self.errores, "errores")?,
            sha256: self.sha256,
            archivo_id: ArchivoId::from_uuid(self.archivo_id),
            generada_por: self.generada_por.map(CuentaId::from_uuid),
            generada_en: self.generada_en,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Repositorio de la entidad
// ---------------------------------------------------------------------------------------------

impl RepositorioTributario for TransaccionPg {
    async fn perfiles_tributarios(&mut self) -> AppResult<Vec<PerfilTributario>> {
        sqlx::query_as!(
            FilaPerfil,
            "SELECT id, version, vigente_desde, estado, responsabilidades, agente_retenedor,
                    responsable_iva, gran_contribuyente, autorretenedor, contribuyente_renta,
                    facturador_electronico, obligado_rub, municipios_ica, actividades_ica,
                    canal_facturacion, soporte_archivo_id, observaciones, creado_por, creado_en,
                    confirmado_por, confirmado_en
             FROM perfiles_tributarios ORDER BY version DESC",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(FilaPerfil::convertir)
        .collect()
    }

    async fn insertar_perfil_tributario(&mut self, p: &PerfilTributario) -> AppResult<()> {
        let r: Vec<String> = p
            .parametros
            .responsabilidades
            .iter()
            .map(|x| x.as_str().to_owned())
            .collect();
        sqlx::query!(
            "INSERT INTO perfiles_tributarios (id, tenant_id, version, vigente_desde, estado,
                 responsabilidades, agente_retenedor, responsable_iva, gran_contribuyente,
                 autorretenedor, contribuyente_renta, facturador_electronico, obligado_rub,
                 municipios_ica, actividades_ica, canal_facturacion, soporte_archivo_id,
                 observaciones, creado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18, $19, $20)",
            p.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            i32_de(p.version, "version")?,
            p.vigente_desde,
            p.estado.as_str(),
            &r,
            p.parametros.agente_retenedor,
            p.parametros.responsable_iva,
            p.parametros.gran_contribuyente,
            p.parametros.autorretenedor,
            p.parametros.contribuyente_renta,
            p.parametros.facturador_electronico,
            p.parametros.obligado_rub,
            &p.parametros.municipios_ica,
            &p.parametros.actividades_ica,
            p.parametros.canal_facturacion.as_str(),
            p.soporte_archivo_id.map(ArchivoId::as_uuid),
            p.observaciones,
            p.creado_por.map(CuentaId::as_uuid),
            p.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "esa versión del perfil ya existe", "soporte"))?;
        Ok(())
    }

    async fn confirmar_perfil_tributario(&mut self, p: &PerfilTributario) -> AppResult<()> {
        let n = sqlx::query!(
            "UPDATE perfiles_tributarios SET estado = 'confirmado', confirmado_por = $2,
                 confirmado_en = $3
             WHERE id = $1 AND estado = 'borrador'",
            p.id.as_uuid(),
            p.confirmado_por.map(CuentaId::as_uuid),
            p.confirmado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected();
        if n == 0 {
            return Err(AppError::Conflicto(
                "el perfil ya está confirmado: cree una versión nueva".into(),
            ));
        }
        Ok(())
    }

    async fn resoluciones(&mut self) -> AppResult<Vec<ResolucionNumeracion>> {
        sqlx::query_as!(
            FilaResolucion,
            "SELECT id, tipo_documento, prefijo, desde, hasta, numero_resolucion,
                    fecha_resolucion, vigente_desde, vigente_hasta, clave_tecnica, activa,
                    creada_en
             FROM resoluciones_numeracion ORDER BY tipo_documento, vigente_desde DESC",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(FilaResolucion::convertir)
        .collect()
    }

    async fn insertar_resolucion(&mut self, r: &ResolucionNumeracion) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO resoluciones_numeracion (id, tenant_id, tipo_documento, prefijo, desde,
                 hasta, numero_resolucion, fecha_resolucion, vigente_desde, vigente_hasta,
                 clave_tecnica, activa, creada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            r.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            r.tipo.as_str(),
            r.prefijo,
            i64_de(r.desde, "desde")?,
            i64_de(r.hasta, "hasta")?,
            r.numero_resolucion,
            r.fecha_resolucion,
            r.vigente_desde,
            r.vigente_hasta,
            r.clave_tecnica,
            r.activa,
            r.creada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "ya existe una resolución con ese rango", "resolución"))?;
        Ok(())
    }

    async fn activar_resolucion(
        &mut self,
        id: ResolucionNumeracionId,
        activa: bool,
    ) -> AppResult<bool> {
        Ok(sqlx::query!(
            "UPDATE resoluciones_numeracion SET activa = $2 WHERE id = $1",
            id.as_uuid(),
            activa,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected()
            > 0)
    }

    async fn ultimo_numero(
        &mut self,
        resolucion: ResolucionNumeracionId,
    ) -> AppResult<Option<u64>> {
        // Bloquear la resolución numera en orden aunque haya emisiones simultáneas.
        sqlx::query!(
            "SELECT id FROM resoluciones_numeracion WHERE id = $1 FOR UPDATE",
            resolucion.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .ok_or(AppError::NoEncontrado("resolución de numeración"))?;
        sqlx::query_scalar!(
            "SELECT MAX(numero) FROM documentos_electronicos WHERE resolucion_id = $1",
            resolucion.as_uuid(),
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(|n| u64_de(n, "numero"))
        .transpose()
    }

    async fn insertar_documento(&mut self, d: &DocumentoElectronico) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO documentos_electronicos (id, tenant_id, tipo, origen_tipo, origen_id,
                 resolucion_id, prefijo, numero, referencia_id, cufe, estado, proveedor,
                 id_externo, xml_archivo_id, respuesta, intentos, total, creado_en, enviado_en,
                 validado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18, $19, $20)",
            d.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            d.tipo.as_str(),
            d.origen.tipo,
            d.origen.id,
            d.resolucion_id.map(ResolucionNumeracionId::as_uuid),
            d.prefijo,
            d.numero.map(|n| i64_de(n, "numero")).transpose()?,
            d.referencia_id.map(DocumentoElectronicoId::as_uuid),
            d.cufe,
            d.estado.as_str(),
            d.proveedor,
            d.id_externo,
            d.xml_archivo_id.map(ArchivoId::as_uuid),
            d.respuesta,
            i32_de(d.intentos, "intentos")?,
            d.total.valor(),
            d.creado_en,
            d.enviado_en,
            d.validado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            error_contable(
                e,
                "el documento ya tiene un documento electrónico de ese tipo o el número ya se usó",
                "resolución o documento de referencia",
            )
        })?;
        Ok(())
    }

    async fn documento(
        &mut self,
        id: DocumentoElectronicoId,
    ) -> AppResult<Option<DocumentoElectronico>> {
        sqlx::query_as!(
            FilaDocumento,
            "SELECT id, tipo, origen_tipo, origen_id, resolucion_id, prefijo, numero,
                    referencia_id, cufe, estado, proveedor, id_externo, xml_archivo_id, respuesta,
                    intentos, total, creado_en, enviado_en, validado_en
             FROM documentos_electronicos WHERE id = $1 FOR UPDATE",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(FilaDocumento::convertir)
        .transpose()
    }

    async fn documento_por_origen(
        &mut self,
        tipo: TipoDocumentoElectronico,
        origen: &Origen,
    ) -> AppResult<Option<DocumentoElectronico>> {
        sqlx::query_as!(
            FilaDocumento,
            "SELECT id, tipo, origen_tipo, origen_id, resolucion_id, prefijo, numero,
                    referencia_id, cufe, estado, proveedor, id_externo, xml_archivo_id, respuesta,
                    intentos, total, creado_en, enviado_en, validado_en
             FROM documentos_electronicos
             WHERE tipo = $1 AND origen_tipo = $2 AND origen_id = $3 FOR UPDATE",
            tipo.as_str(),
            origen.tipo,
            origen.id,
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(FilaDocumento::convertir)
        .transpose()
    }

    async fn documentos(
        &mut self,
        filtro: &FiltroDocumentos,
        limite: u32,
    ) -> AppResult<Vec<DocumentoElectronico>> {
        sqlx::query_as!(
            FilaDocumento,
            "SELECT id, tipo, origen_tipo, origen_id, resolucion_id, prefijo, numero,
                    referencia_id, cufe, estado, proveedor, id_externo, xml_archivo_id, respuesta,
                    intentos, total, creado_en, enviado_en, validado_en
             FROM documentos_electronicos
             WHERE ($1::text IS NULL OR tipo = $1) AND ($2::text IS NULL OR estado = $2)
             ORDER BY creado_en DESC, id DESC
             LIMIT $3",
            filtro.tipo.map(|t| t.as_str()),
            filtro.estado.map(|e| e.as_str()),
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(FilaDocumento::convertir)
        .collect()
    }

    async fn actualizar_documento(&mut self, d: &DocumentoElectronico) -> AppResult<()> {
        sqlx::query!(
            "UPDATE documentos_electronicos SET estado = $2, cufe = $3, proveedor = $4,
                 id_externo = $5, xml_archivo_id = $6, respuesta = $7, intentos = $8,
                 enviado_en = $9, validado_en = $10
             WHERE id = $1",
            d.id.as_uuid(),
            d.estado.as_str(),
            d.cufe,
            d.proveedor,
            d.id_externo,
            d.xml_archivo_id.map(ArchivoId::as_uuid),
            d.respuesta,
            i32_de(d.intentos, "intentos")?,
            d.enviado_en,
            d.validado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "el CUFE ya está registrado", "archivo"))?;
        Ok(())
    }

    async fn insertar_evento_documento(
        &mut self,
        documento: DocumentoElectronicoId,
        e: &EventoDocumento,
    ) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO eventos_documento_electronico (id, tenant_id, documento_id, tipo,
                 detalle, ocurrido_en)
             VALUES ($1, $2, $3, $4, $5, $6)",
            Uuid::now_v7(),
            self.ctx.tenant_id.as_uuid(),
            documento.as_uuid(),
            e.tipo,
            e.detalle,
            e.ocurrido_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?;
        Ok(())
    }

    async fn eventos_documento(
        &mut self,
        documento: DocumentoElectronicoId,
    ) -> AppResult<Vec<EventoDocumento>> {
        Ok(sqlx::query!(
            "SELECT tipo, detalle, ocurrido_en FROM eventos_documento_electronico
             WHERE documento_id = $1 ORDER BY ocurrido_en, id",
            documento.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|e| EventoDocumento {
            tipo: e.tipo,
            detalle: e.detalle,
            ocurrido_en: e.ocurrido_en,
        })
        .collect())
    }

    async fn insertar_factura_recibida(&mut self, f: &FacturaRecibida) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO facturas_recibidas (id, tenant_id, archivo_id, sha256, emisor_nit,
                 emisor_nombre, prefijo, numero, fecha, vencimiento, cufe, subtotal, iva, total,
                 estado, persona_id, factura_proveedor_id, motivo_rechazo, recibida_por,
                 recibida_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                     $18, $19, $20)",
            f.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            f.archivo_id.as_uuid(),
            f.sha256,
            f.emisor_nit,
            f.emisor_nombre,
            f.prefijo,
            f.numero,
            f.fecha,
            f.vencimiento,
            f.cufe,
            f.subtotal.valor(),
            f.iva.valor(),
            f.total.valor(),
            f.estado.as_str(),
            f.persona_id.map(PersonaId::as_uuid),
            f.factura_proveedor_id.map(FacturaProveedorId::as_uuid),
            f.motivo_rechazo,
            f.recibida_por.map(CuentaId::as_uuid),
            f.recibida_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "esa factura ya se cargó", "archivo o proveedor"))?;
        Ok(())
    }

    async fn factura_recibida(
        &mut self,
        id: FacturaRecibidaId,
    ) -> AppResult<Option<FacturaRecibida>> {
        sqlx::query_as!(
            FilaRecibida,
            "SELECT id, archivo_id, sha256, emisor_nit, emisor_nombre, prefijo, numero, fecha,
                    vencimiento, cufe, subtotal, iva, total, estado, persona_id,
                    factura_proveedor_id, motivo_rechazo, recibida_por, recibida_en
             FROM facturas_recibidas WHERE id = $1 FOR UPDATE",
            id.as_uuid(),
        )
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .map(FilaRecibida::convertir)
        .transpose()
    }

    async fn facturas_recibidas(
        &mut self,
        estado: Option<EstadoFacturaRecibida>,
        limite: u32,
    ) -> AppResult<Vec<FacturaRecibida>> {
        sqlx::query_as!(
            FilaRecibida,
            "SELECT id, archivo_id, sha256, emisor_nit, emisor_nombre, prefijo, numero, fecha,
                    vencimiento, cufe, subtotal, iva, total, estado, persona_id,
                    factura_proveedor_id, motivo_rechazo, recibida_por, recibida_en
             FROM facturas_recibidas
             WHERE $1::text IS NULL OR estado = $1
             ORDER BY recibida_en DESC, id DESC
             LIMIT $2",
            estado.map(|e| e.as_str()),
            i64::from(limite),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(FilaRecibida::convertir)
        .collect()
    }

    async fn actualizar_factura_recibida(&mut self, f: &FacturaRecibida) -> AppResult<()> {
        sqlx::query!(
            "UPDATE facturas_recibidas SET estado = $2, persona_id = $3,
                 factura_proveedor_id = $4, motivo_rechazo = $5
             WHERE id = $1",
            f.id.as_uuid(),
            f.estado.as_str(),
            f.persona_id.map(PersonaId::as_uuid),
            f.factura_proveedor_id.map(FacturaProveedorId::as_uuid),
            f.motivo_rechazo,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "factura repetida", "proveedor o factura"))?;
        Ok(())
    }

    async fn insertar_evento_radian(&mut self, e: &RegistroRadian) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO eventos_radian (id, tenant_id, factura_recibida_id, tipo, estado,
                 id_externo, motivo, respuesta, creado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            e.id,
            self.ctx.tenant_id.as_uuid(),
            e.factura_recibida_id.as_uuid(),
            e.tipo.as_str(),
            e.estado,
            e.id_externo,
            e.motivo,
            e.respuesta,
            e.creado_por.map(CuentaId::as_uuid),
            e.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|x| error_contable(x, "ese evento ya se registró", "factura recibida"))?;
        Ok(())
    }

    async fn eventos_radian(
        &mut self,
        factura: FacturaRecibidaId,
    ) -> AppResult<Vec<RegistroRadian>> {
        sqlx::query!(
            "SELECT id, factura_recibida_id, tipo, estado, id_externo, motivo, respuesta,
                    creado_por, creado_en
             FROM eventos_radian WHERE factura_recibida_id = $1 ORDER BY creado_en, id",
            factura.as_uuid(),
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|e| {
            Ok(RegistroRadian {
                id: e.id,
                factura_recibida_id: FacturaRecibidaId::from_uuid(e.factura_recibida_id),
                tipo: EventoRadian::try_from(e.tipo.as_str())?,
                estado: e.estado,
                id_externo: e.id_externo,
                motivo: e.motivo,
                respuesta: e.respuesta,
                creado_por: e.creado_por.map(CuentaId::from_uuid),
                creado_en: e.creado_en,
            })
        })
        .collect()
    }

    async fn movimientos_por_tercero(
        &mut self,
        anio: i32,
        prefijos: &[String],
    ) -> AppResult<Vec<MovimientoTercero>> {
        // Los comprobantes revertidos y sus reversos se excluyen: se anulan entre sí y, si no,
        // inflarían los débitos y créditos brutos de la exógena.
        Ok(sqlx::query!(
            r#"SELECT cc.codigo AS "cuenta_codigo!", l.persona_id, p.documento,
                      COALESCE(p.nombre, '') AS "nombre!",
                      SUM(l.debito) AS "debitos!", SUM(l.credito) AS "creditos!"
               FROM lineas_comprobante l
               JOIN comprobantes c ON c.tenant_id = l.tenant_id AND c.id = l.comprobante_id
               JOIN cuentas_contables cc ON cc.tenant_id = l.tenant_id AND cc.id = l.cuenta_id
               LEFT JOIN personas p ON p.tenant_id = l.tenant_id AND p.id = l.persona_id
               WHERE c.estado = 'contabilizado'
                 AND EXTRACT(YEAR FROM c.fecha)::int = $1
                 AND c.revierte_a IS NULL
                 AND NOT EXISTS (SELECT 1 FROM comprobantes r
                                 WHERE r.tenant_id = c.tenant_id AND r.revierte_a = c.id)
                 AND EXISTS (SELECT 1 FROM unnest($2::text[]) AS pre
                             WHERE cc.codigo LIKE pre || '%')
               GROUP BY cc.codigo, l.persona_id, p.documento, p.nombre
               ORDER BY cc.codigo, p.documento NULLS LAST, l.persona_id"#,
            anio,
            prefijos,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|m| MovimientoTercero {
            cuenta_codigo: m.cuenta_codigo,
            persona_id: m.persona_id.map(PersonaId::from_uuid),
            documento: m.documento,
            nombre: m.nombre,
            debitos: m.debitos,
            creditos: m.creditos,
        })
        .collect())
    }

    async fn insertar_generacion_exogena(&mut self, g: &GeneracionExogena) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO generaciones_exogena (id, tenant_id, formato_id, anio, version, filas,
                 total, errores, sha256, archivo_id, generada_por, generada_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
            g.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            g.formato_id.as_uuid(),
            i16_anio(g.anio)?,
            i32_de(g.version, "version")?,
            i32_de(g.filas, "filas")?,
            g.total,
            i32_de(g.errores, "errores")?,
            g.sha256,
            g.archivo_id.as_uuid(),
            g.generada_por.map(CuentaId::as_uuid),
            g.generada_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "generación repetida", "formato o archivo"))?;
        Ok(())
    }

    async fn generaciones_exogena(
        &mut self,
        anio: Option<i32>,
    ) -> AppResult<Vec<GeneracionExogena>> {
        let anio = anio.map(i16_anio).transpose()?;
        sqlx::query_as!(
            FilaGeneracion,
            "SELECT id, formato_id, anio, version, filas, total, errores, sha256, archivo_id,
                    generada_por, generada_en
             FROM generaciones_exogena
             WHERE $1::smallint IS NULL OR anio = $1
             ORDER BY generada_en DESC, id DESC",
            anio,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(FilaGeneracion::convertir)
        .collect()
    }

    async fn beneficiarios_finales(&mut self) -> AppResult<Vec<BeneficiarioFinal>> {
        sqlx::query!(
            "SELECT id, persona_id, criterio, porcentaje, soporte_archivo_id, vigente_desde,
                    vigente_hasta, creado_por, creado_en
             FROM beneficiarios_finales
             ORDER BY vigente_hasta IS NOT NULL, vigente_desde DESC, id",
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|b| {
            Ok(BeneficiarioFinal {
                id: BeneficiarioFinalId::from_uuid(b.id),
                persona_id: PersonaId::from_uuid(b.persona_id),
                criterio: CriterioBeneficiario::try_from(b.criterio.as_str())?,
                porcentaje: b.porcentaje,
                soporte_archivo_id: b.soporte_archivo_id.map(ArchivoId::from_uuid),
                vigente_desde: b.vigente_desde,
                vigente_hasta: b.vigente_hasta,
                creado_por: b.creado_por.map(CuentaId::from_uuid),
                creado_en: b.creado_en,
            })
        })
        .collect()
    }

    async fn insertar_beneficiario(&mut self, b: &BeneficiarioFinal) -> AppResult<()> {
        sqlx::query!(
            "INSERT INTO beneficiarios_finales (id, tenant_id, persona_id, criterio, porcentaje,
                 soporte_archivo_id, vigente_desde, vigente_hasta, creado_por, creado_en)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            b.id.as_uuid(),
            self.ctx.tenant_id.as_uuid(),
            b.persona_id.as_uuid(),
            b.criterio.as_str(),
            b.porcentaje,
            b.soporte_archivo_id.map(ArchivoId::as_uuid),
            b.vigente_desde,
            b.vigente_hasta,
            b.creado_por.map(CuentaId::as_uuid),
            b.creado_en,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(|e| error_contable(e, "beneficiario repetido", "persona o soporte"))?;
        Ok(())
    }

    async fn terminar_beneficiario(
        &mut self,
        id: BeneficiarioFinalId,
        hasta: NaiveDate,
    ) -> AppResult<bool> {
        Ok(sqlx::query!(
            "UPDATE beneficiarios_finales SET vigente_hasta = $2
             WHERE id = $1 AND vigente_hasta IS NULL AND vigente_desde <= $2",
            id.as_uuid(),
            hasta,
        )
        .execute(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .rows_affected()
            > 0)
    }

    async fn resumen_retenciones(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> AppResult<Vec<ResumenRetencion>> {
        Ok(sqlx::query!(
            r#"SELECT r.impuesto, c.codigo AS concepto, r.tarifa,
                      SUM(r.base) AS "base!", SUM(r.valor) AS "valor!"
               FROM retenciones_factura_proveedor r
               JOIN facturas_proveedor f ON f.tenant_id = r.tenant_id AND f.id = r.factura_id
               JOIN conceptos_retencion c ON c.id = r.concepto_retencion_id
               WHERE f.estado IN ('causada', 'pagada_parcial', 'pagada')
                 AND f.fecha BETWEEN $1 AND $2
               GROUP BY r.impuesto, c.codigo, r.tarifa
               ORDER BY r.impuesto, c.codigo, r.tarifa"#,
            desde,
            hasta,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| ResumenRetencion {
            impuesto: r.impuesto,
            concepto: r.concepto,
            tarifa: r.tarifa,
            base: r.base,
            valor: r.valor,
        })
        .collect())
    }

    async fn iva_generado(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> AppResult<Vec<ResumenIva>> {
        Ok(sqlx::query!(
            r#"SELECT l.tarifa_iva AS tarifa, SUM(l.subtotal) AS "base!", SUM(l.iva) AS "iva!"
               FROM lineas_factura_venta l
               JOIN facturas_venta f ON f.tenant_id = l.tenant_id AND f.id = l.factura_id
               WHERE f.estado IN ('emitida', 'pagada_parcial', 'pagada')
                 AND f.fecha BETWEEN $1 AND $2
               GROUP BY l.tarifa_iva ORDER BY l.tarifa_iva"#,
            desde,
            hasta,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| ResumenIva {
            tarifa: r.tarifa,
            base: r.base,
            iva: r.iva,
        })
        .collect())
    }

    async fn iva_descontable(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> AppResult<Vec<ResumenIva>> {
        // Las líneas de compra no guardan la tarifa: se deduce del IVA sobre la base.
        Ok(sqlx::query!(
            r#"SELECT ROUND(l.iva * 100 / l.base, 0) AS "tarifa!",
                      SUM(l.base) AS "base!", SUM(l.iva) AS "iva!"
               FROM lineas_factura_proveedor l
               JOIN facturas_proveedor f ON f.tenant_id = l.tenant_id AND f.id = l.factura_id
               WHERE f.estado IN ('causada', 'pagada_parcial', 'pagada')
                 AND f.fecha BETWEEN $1 AND $2
                 AND l.iva > 0 AND l.iva_descontable
               GROUP BY 1 ORDER BY 1"#,
            desde,
            hasta,
        )
        .fetch_all(&mut *self.tx)
        .await
        .map_err(AppError::infra)?
        .into_iter()
        .map(|r| ResumenIva {
            tarifa: r.tarifa,
            base: r.base,
            iva: r.iva,
        })
        .collect())
    }

    async fn ingresos_facturados(
        &mut self,
        desde: NaiveDate,
        hasta: NaiveDate,
    ) -> AppResult<Decimal> {
        sqlx::query_scalar!(
            r#"SELECT COALESCE(SUM(subtotal), 0) AS "total!" FROM facturas_venta
               WHERE estado IN ('emitida', 'pagada_parcial', 'pagada')
                 AND fecha BETWEEN $1 AND $2"#,
            desde,
            hasta,
        )
        .fetch_one(&mut *self.tx)
        .await
        .map_err(AppError::infra)
    }
}
