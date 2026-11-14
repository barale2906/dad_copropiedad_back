//! Preparación tributaria (plan 15 fase 5): reportes para preparar la retención en la fuente, el
//! IVA y el ICA (no se presentan desde aquí: presentar y firmar lo hace el responsable
//! tributario), conciliación de las retenciones contra el mayor, información exógena por formato
//! versionado (reproducible) y beneficiarios finales (RUB).

use std::collections::BTreeSet;

use chrono::NaiveDate;
use domain::{
    BeneficiarioFinalId, ClasificacionArchivo, FormatoExogenaId, GeneracionExogenaId,
    NombreArchivo, Permiso, PersonaId, TipoArchivo,
    contabilidad::{
        Cara,
        tributario::{CriterioBeneficiario, LadoExogena, ValorExogena, consolidar_exogena},
    },
};
use rust_decimal::Decimal;

use super::facturacion::TxFacturacion;
use super::prelude::*;
use super::tesoreria::{abrir, eventos};
use super::tributario::perfil_confirmado_en;
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::archivos::{
        ArchivoSubido, borrar_huerfano, nuevo_archivo, registrar_en, sha256_hex,
    },
    puertos::{
        AlmacenArchivos, Reloj, UnidadDeTrabajo,
        contabilidad::AgrupacionSaldos,
        tributario::{BeneficiarioFinal, GeneracionExogena, ResumenIva, ResumenRetencion},
    },
};

const ACCION_EXOGENA: AccionAuditoria = AccionAuditoria::new("tributario.exogena.generar");
const ACCION_BENEFICIARIO: AccionAuditoria = AccionAuditoria::new("tributario.rub.registrar");
const ACCION_BENEFICIARIO_FIN: AccionAuditoria = AccionAuditoria::new("tributario.rub.terminar");

fn validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

fn rango(desde: NaiveDate, hasta: NaiveDate) -> AppResult<()> {
    if hasta < desde {
        return Err(validacion("el periodo termina antes de empezar"));
    }
    Ok(())
}

/// Preparación de la retención en la fuente de un periodo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparacionRetencion {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    /// Por impuesto, concepto y tarifa.
    pub detalle: Vec<ResumenRetencion>,
    pub total_practicado: Decimal,
    /// Neto de las cuentas que acreditan las retenciones en la regla de `cxp.factura`.
    pub total_mayor: Decimal,
    /// Mayor − practicado (cero si cuadra con los certificados).
    pub diferencia: Decimal,
    /// ¿Debe declarar? Solo un agente de retención con perfil confirmado y retenciones en el
    /// periodo (§7.3: el formulario 350 solo si está obligada en ese periodo).
    pub obligada: bool,
}

/// Prepara la retención en la fuente del periodo: lo practicado en las facturas de proveedor
/// causadas y su conciliación contra el mayor.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tributario.preparar`; [`AppError::Validacion`] si el periodo no
/// es válido.
pub async fn preparar_retencion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<PreparacionRetencion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    rango(desde, hasta)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TributarioPreparar).await?;
    let detalle = tx.resumen_retenciones(desde, hasta).await?;
    let total_practicado: Decimal = detalle.iter().map(|r| r.valor).sum();
    let cuentas: BTreeSet<_> = match tx.regla_vigente(eventos::FACTURA_PROVEEDOR, hasta).await? {
        Some(r) => r
            .regla
            .lineas
            .iter()
            .filter(|l| l.cara == Cara::Credito && l.concepto.starts_with("retencion_"))
            .map(|l| l.cuenta)
            .collect(),
        None => BTreeSet::new(),
    };
    let total_mayor: Decimal = tx
        .saldos(desde, hasta, AgrupacionSaldos::default())
        .await?
        .iter()
        .filter(|s| cuentas.contains(&s.cuenta_id))
        .map(|s| s.creditos - s.debitos)
        .sum();
    let agente = perfil_confirmado_en(&mut tx, hasta)
        .await?
        .is_some_and(|p| p.parametros.agente_retenedor);
    Ok(PreparacionRetencion {
        desde,
        hasta,
        obligada: agente && !total_practicado.is_zero(),
        detalle,
        total_practicado,
        total_mayor,
        diferencia: total_mayor - total_practicado,
    })
}

/// Preparación del IVA de un periodo (bimestral o cuatrimestral, según la entidad).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparacionIva {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    pub generado: Vec<ResumenIva>,
    pub descontable: Vec<ResumenIva>,
    pub total_generado: Decimal,
    pub total_descontable: Decimal,
    /// Generado − descontable (positivo = a pagar).
    pub saldo: Decimal,
    /// Responsable de IVA con perfil confirmado.
    pub responsable: bool,
}

/// Prepara el IVA del periodo: generado en ventas y descontable en compras, por tarifa.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tributario.preparar`; [`AppError::Validacion`].
pub async fn preparar_iva<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<PreparacionIva>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    rango(desde, hasta)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TributarioPreparar).await?;
    let generado = tx.iva_generado(desde, hasta).await?;
    let descontable = tx.iva_descontable(desde, hasta).await?;
    let total_generado: Decimal = generado.iter().map(|r| r.iva).sum();
    let total_descontable: Decimal = descontable.iter().map(|r| r.iva).sum();
    let responsable = perfil_confirmado_en(&mut tx, hasta)
        .await?
        .is_some_and(|p| p.parametros.responsable_iva);
    Ok(PreparacionIva {
        desde,
        hasta,
        generado,
        descontable,
        total_generado,
        total_descontable,
        saldo: total_generado - total_descontable,
        responsable,
    })
}

/// ICA estimado de un municipio y actividad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineaIca {
    pub municipio_dane: String,
    pub actividad: String,
    /// `None` si el catálogo no tiene la tarifa vigente.
    pub tarifa_por_mil: Option<Decimal>,
    pub base: Decimal,
    pub impuesto: Option<Decimal>,
}

/// Preparación del ICA y la ReteICA de un periodo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparacionIca {
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    /// Ingresos facturados en el periodo (base de cada municipio y actividad: si hay varios, el
    /// responsable distribuye la base).
    pub ingresos: Decimal,
    pub lineas: Vec<LineaIca>,
    /// ReteICA practicada en el periodo.
    pub reteica_practicada: Decimal,
}

/// Prepara el ICA de los municipios y actividades del perfil confirmado con las tarifas vigentes
/// del catálogo, y la ReteICA practicada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tributario.preparar`; [`AppError::Validacion`] sin perfil
/// confirmado.
pub async fn preparar_ica<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<PreparacionIca>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    rango(desde, hasta)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TributarioPreparar).await?;
    let perfil = perfil_confirmado_en(&mut tx, hasta)
        .await?
        .ok_or_else(|| validacion("confirme el perfil tributario para preparar el ICA"))?;
    let ingresos = tx.ingresos_facturados(desde, hasta).await?;
    let tarifas = tx.tarifas_ica(Some(hasta)).await?;
    let mut lineas = Vec::new();
    for municipio in &perfil.parametros.municipios_ica {
        for actividad in &perfil.parametros.actividades_ica {
            let tarifa = tarifas
                .iter()
                .find(|t| &t.municipio_dane == municipio && &t.actividad == actividad)
                .map(|t| t.tarifa_por_mil);
            lineas.push(LineaIca {
                municipio_dane: municipio.clone(),
                actividad: actividad.clone(),
                tarifa_por_mil: tarifa,
                base: ingresos,
                impuesto: tarifa.map(|t| (ingresos * t / Decimal::from(1000)).round_dp(0)),
            });
        }
    }
    let reteica_practicada = tx
        .resumen_retenciones(desde, hasta)
        .await?
        .iter()
        .filter(|r| r.impuesto == "ica")
        .map(|r| r.valor)
        .sum();
    Ok(PreparacionIca {
        desde,
        hasta,
        ingresos,
        lineas,
        reteica_practicada,
    })
}

// ---------------------------------------------------------------------------------------------
// Información exógena
// ---------------------------------------------------------------------------------------------

/// Generación de la exógena con su archivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExogenaGenerada {
    pub generacion: GeneracionExogena,
    /// Archivo de preparación (CSV separado por `;`).
    pub csv: String,
    pub errores: Vec<String>,
}

fn csv_exogena(filas: &[domain::contabilidad::tributario::FilaExogena]) -> String {
    let columnas: BTreeSet<&str> = filas
        .iter()
        .flat_map(|f| f.columnas.keys().map(String::as_str))
        .collect();
    let mut csv = String::from("concepto;documento;dv;nombre");
    for c in &columnas {
        csv.push(';');
        csv.push_str(c);
    }
    csv.push('\n');
    for f in filas {
        let nombre = f.nombre.replace([';', '\n', '\r'], " ");
        csv.push_str(&format!(
            "{};{};{};{}",
            f.concepto,
            f.documento,
            f.dv.as_deref().unwrap_or_default(),
            nombre
        ));
        for c in &columnas {
            csv.push(';');
            csv.push_str(
                &f.columnas
                    .get(*c)
                    .map(|v| v.round_dp(0).to_string())
                    .unwrap_or_else(|| "0".into()),
            );
        }
        csv.push('\n');
    }
    csv
}

/// Genera un formato de exógena del año con la versión de reglas del catálogo: suma por tercero
/// los movimientos de las cuentas de cada concepto, agrega las cuantías menores, valida los
/// documentos y guarda el archivo con su huella (la misma versión y los mismos datos dan el
/// mismo archivo).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tributario.preparar`; [`AppError::NoEncontrado`] si el formato
/// no existe.
pub async fn generar_exogena<U, A>(
    unidad: &U,
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &Contexto,
    formato: FormatoExogenaId,
) -> AppResult<ExogenaGenerada>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
    A: AlmacenArchivos,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TributarioPreparar).await?;
    let f = tx
        .formatos_exogena(None)
        .await?
        .into_iter()
        .find(|x| x.id == formato)
        .ok_or(AppError::NoEncontrado("formato de exógena"))?;
    let prefijos: Vec<String> = f
        .campos
        .iter()
        .map(|c| c.cuenta_prefijo.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let movimientos = tx.movimientos_por_tercero(f.anio, &prefijos).await?;
    let mut valores = Vec::new();
    for m in &movimientos {
        for c in f
            .campos
            .iter()
            .filter(|c| m.cuenta_codigo.starts_with(&c.cuenta_prefijo))
        {
            let valor = match c.lado {
                LadoExogena::Debito => m.debitos,
                LadoExogena::Credito => m.creditos,
                LadoExogena::Neto => m.debitos - m.creditos,
            };
            valores.push(ValorExogena {
                concepto: c.concepto.clone(),
                columna: c.columna.clone(),
                documento: m.documento.clone(),
                nombre: m.nombre.clone(),
                valor,
            });
        }
    }
    let exogena = consolidar_exogena(&valores, f.cuantia_menor);
    let csv = csv_exogena(&exogena.filas);
    let contenido = csv.clone().into_bytes();
    let sha256 = sha256_hex(&contenido);
    let subido = ArchivoSubido {
        nombre_original: NombreArchivo::new(&format!(
            "exogena-{}-{}-v{}.csv",
            f.codigo, f.anio, f.version
        ))?,
        clasificacion: ClasificacionArchivo::Personal,
        contenido,
    };
    let archivo = nuevo_archivo(ctx, almacen, reloj, &subido, TipoArchivo::Csv);
    almacen.guardar(&archivo.clave, &subido.contenido).await?;
    let resultado = async {
        registrar_en(&mut tx, &archivo).await?;
        let total: Decimal = exogena.filas.iter().flat_map(|x| x.columnas.values()).sum();
        let generacion = GeneracionExogena {
            id: GeneracionExogenaId::nuevo(),
            formato_id: f.id,
            anio: f.anio,
            version: f.version,
            filas: u32::try_from(exogena.filas.len()).unwrap_or(u32::MAX),
            total,
            errores: u32::try_from(exogena.errores.len()).unwrap_or(u32::MAX),
            sha256,
            archivo_id: archivo.id,
            generada_por: ctx.actor_id,
            generada_en: reloj.ahora(),
        };
        tx.insertar_generacion_exogena(&generacion).await?;
        let detalle = DetalleAuditoria::vacio()
            .con("formato", f.codigo.as_str())?
            .con("anio", f.anio)?
            .con("version", f.version)?
            .con("sha256", generacion.sha256.as_str())?;
        tx.registrar(
            EventoAuditoria::ok(
                ACCION_EXOGENA,
                "generacion_exogena",
                generacion.id.as_uuid(),
            )
            .con_detalle(detalle),
        )
        .await?;
        tx.confirmar().await?;
        Ok(generacion)
    }
    .await;
    match resultado {
        Ok(generacion) => Ok(ExogenaGenerada {
            generacion,
            csv,
            errores: exogena.errores,
        }),
        Err(e) => {
            borrar_huerfano(almacen, ctx, &archivo).await;
            Err(e)
        }
    }
}

/// Generaciones de exógena (del año, si se da).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tributario.preparar`.
pub async fn generaciones_exogena<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    anio: Option<i32>,
) -> AppResult<Vec<GeneracionExogena>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    abrir(unidad, reloj, ctx, Permiso::TributarioPreparar)
        .await?
        .generaciones_exogena(anio)
        .await
}

// ---------------------------------------------------------------------------------------------
// RUB
// ---------------------------------------------------------------------------------------------

/// Beneficiarios finales de la entidad.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tributario.preparar`.
pub async fn beneficiarios_finales<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<BeneficiarioFinal>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    abrir(unidad, reloj, ctx, Permiso::TributarioPreparar)
        .await?
        .beneficiarios_finales()
        .await
}

/// Beneficiario final nuevo.
#[derive(Debug, Clone)]
pub struct NuevoBeneficiario {
    pub persona: PersonaId,
    pub criterio: CriterioBeneficiario,
    /// Participación (%) cuando el criterio es la propiedad.
    pub porcentaje: Option<Decimal>,
    pub soporte_archivo_id: Option<domain::ArchivoId>,
    pub vigente_desde: NaiveDate,
}

/// Registra un beneficiario final (RUB). Ser propietario de un bien de una PH no lo hace
/// beneficiario final de la PH.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tributario.preparar`; [`AppError::NoEncontrado`] si la persona
/// no existe; [`AppError::Validacion`] si el porcentaje no corresponde al criterio.
pub async fn registrar_beneficiario<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoBeneficiario,
) -> AppResult<BeneficiarioFinal>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    let porcentaje_ok = match nuevo.criterio {
        CriterioBeneficiario::Propiedad => nuevo
            .porcentaje
            .is_some_and(|p| p > Decimal::ZERO && p <= Decimal::ONE_HUNDRED),
        _ => nuevo.porcentaje.is_none(),
    };
    if !porcentaje_ok {
        return Err(validacion(
            "la participación (de 0 a 100 %) va solo con el criterio de propiedad",
        ));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TributarioPreparar).await?;
    tx.persona(nuevo.persona)
        .await?
        .ok_or(AppError::NoEncontrado("persona"))?;
    let b = BeneficiarioFinal {
        id: BeneficiarioFinalId::nuevo(),
        persona_id: nuevo.persona,
        criterio: nuevo.criterio,
        porcentaje: nuevo.porcentaje,
        soporte_archivo_id: nuevo.soporte_archivo_id,
        vigente_desde: nuevo.vigente_desde,
        vigente_hasta: None,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
    };
    tx.insertar_beneficiario(&b).await?;
    let detalle = DetalleAuditoria::vacio().con("criterio", b.criterio.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_BENEFICIARIO, "beneficiario_final", b.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(b)
}

/// Termina la vigencia de un beneficiario final.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tributario.preparar`; [`AppError::NoEncontrado`] si no existe o
/// ya terminó.
pub async fn terminar_beneficiario<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: BeneficiarioFinalId,
    hasta: NaiveDate,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxFacturacion,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TributarioPreparar).await?;
    if !tx.terminar_beneficiario(id, hasta).await? {
        return Err(AppError::NoEncontrado("beneficiario final vigente"));
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_BENEFICIARIO_FIN,
        "beneficiario_final",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}
