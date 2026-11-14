//! Cuentas por pagar (plan 13 fase 2): catálogo de retenciones de la plataforma (plan 15 fase 1,
//! adelantado), facturas de proveedores con sus retenciones calculadas con la tarifa vigente en
//! la fecha de la factura, causación por el contabilizador y certificados de retención.
//!
//! Flujo mínimo: registrar → causar → egreso (autorizar y pagar) → conciliar.

use std::collections::BTreeSet;

use chrono::{Datelike, NaiveDate};
use domain::{
    ConceptoRetencionId, DireccionCorreo, FacturaProveedorId, Permiso, PersonaId,
    contabilidad::{
        CodigoCuenta, ImporteDocumento, Monto, RegimenIva,
        tesoreria::{
            EstadoFacturaProveedor, ImpuestoRetencion, PerfilRetencionTercero, TarifaRetencion,
            calcular_retencion, situacion_pago,
        },
    },
};
use rust_decimal::Decimal;

use super::comprobantes::{self, DocumentoOrigen, revertir_en};
use super::prelude::*;
use super::tesoreria::{
    LIMITE_LISTADO, abrir, centro_por_codigo, conceptos, cuenta_de_movimiento, eventos, origen,
};
use super::{TxTesoreria, texto, texto_opcional};
use crate::{
    AppError, AppResult, Contexto, ContextoPlataforma,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        GeneradorActas, RegistroAuditoria, RegistroOutbox, Reloj, UnidadDeTrabajo,
        UnidadDeTrabajoPlataforma,
        actas::TablaPdf,
        contabilidad::Origen,
        correo::CorreoSaliente,
        tesoreria::{
            CatalogoRetencionesPlataforma, ConceptoRetencion, FacturaProveedor, FilaCertificado,
            FiltroFacturas, LineaFacturaProveedor, RetencionLinea, ValorUvt,
        },
    },
};

const ACCION_CONCEPTO: AccionAuditoria = AccionAuditoria::new("plataforma.retencion.crear");
const ACCION_UVT: AccionAuditoria = AccionAuditoria::new("plataforma.uvt.guardar");
const ACCION_REGISTRAR: AccionAuditoria =
    AccionAuditoria::new("tesoreria.factura_proveedor.registrar");
const ACCION_CAUSAR: AccionAuditoria = AccionAuditoria::new("tesoreria.factura_proveedor.causar");
const ACCION_ANULAR: AccionAuditoria = AccionAuditoria::new("tesoreria.factura_proveedor.anular");
const ACCION_CERTIFICADO: AccionAuditoria =
    AccionAuditoria::new("tesoreria.certificado_retencion.enviar");

/// Máximo de líneas de una factura.
pub const MAXIMO_LINEAS: usize = 200;

fn validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

// ---------------------------------------------------------------------------------------------
// Catálogo de retenciones
// ---------------------------------------------------------------------------------------------

/// Versión nueva de un concepto de retención.
#[derive(Debug, Clone)]
pub struct NuevoConceptoRetencion {
    pub codigo: String,
    pub impuesto: ImpuestoRetencion,
    pub nombre: String,
    pub base_minima_uvt: Decimal,
    /// Porcentaje.
    pub tarifa: Decimal,
    pub cuenta_sugerida: Option<CodigoCuenta>,
    pub municipio_dane: Option<String>,
    pub fuente: String,
    pub vigente_desde: NaiveDate,
}

/// Agrega una versión de un concepto de retención (la anterior del mismo código queda vigente
/// hasta el día antes).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`; [`AppError::Validacion`] si un
/// dato no cumple; [`AppError::Conflicto`] si ya hay una versión con esa fecha.
pub async fn crear_concepto_retencion<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
    nuevo: NuevoConceptoRetencion,
) -> AppResult<ConceptoRetencion>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoRetencionesPlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    let codigo = nuevo.codigo.trim().to_owned();
    let mut c = codigo.chars();
    let codigo_valido = (2..=60).contains(&codigo.len())
        && c.next().is_some_and(|x| x.is_ascii_lowercase())
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '_');
    if !codigo_valido {
        return Err(validacion(
            "el código usa minúsculas, dígitos y `_` y empieza por letra",
        ));
    }
    if nuevo.tarifa <= Decimal::ZERO
        || nuevo.tarifa > Decimal::ONE_HUNDRED
        || nuevo.base_minima_uvt < Decimal::ZERO
    {
        return Err(validacion(
            "la tarifa es un porcentaje positivo y la base mínima no es negativa",
        ));
    }
    let municipio = texto_opcional("municipio_dane", nuevo.municipio_dane.as_deref(), 5)?;
    if municipio
        .as_deref()
        .is_some_and(|m| m.len() != 5 || !m.chars().all(|c| c.is_ascii_digit()))
        || (municipio.is_some() && nuevo.impuesto != ImpuestoRetencion::Ica)
    {
        return Err(validacion(
            "el municipio (código DANE de cinco dígitos) es solo de ReteICA",
        ));
    }
    let concepto = ConceptoRetencion {
        id: ConceptoRetencionId::nuevo(),
        codigo,
        impuesto: nuevo.impuesto,
        nombre: texto("nombre", &nuevo.nombre, 200)?,
        base_minima_uvt: nuevo.base_minima_uvt,
        tarifa: nuevo.tarifa,
        cuenta_sugerida: nuevo.cuenta_sugerida,
        municipio_dane: municipio,
        fuente: texto("fuente", &nuevo.fuente, 300)?,
        vigente_desde: nuevo.vigente_desde,
        vigente_hasta: None,
    };
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    tx.insertar_concepto_retencion(&concepto).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("codigo", concepto.codigo.as_str())?
        .con("tarifa", concepto.tarifa.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CONCEPTO, "concepto_retencion", concepto.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(concepto)
}

/// Crea o corrige el valor de la UVT de un año.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`; [`AppError::Validacion`] si el
/// valor no es positivo o falta la fuente.
pub async fn guardar_uvt<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
    uvt: ValorUvt,
) -> AppResult<ValorUvt>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoRetencionesPlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    if uvt.valor <= Decimal::ZERO || !(2000..=2999).contains(&uvt.anio) {
        return Err(validacion("año entre 2000 y 2999 y valor positivo"));
    }
    let uvt = ValorUvt {
        fuente: texto("fuente", &uvt.fuente, 300)?,
        ..uvt
    };
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    tx.guardar_uvt(&uvt).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("anio", uvt.anio)?
        .con("valor", uvt.valor.to_string())?;
    tx.registrar(EventoAuditoria::ok(ACCION_UVT, "uvt", uuid::Uuid::nil()).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    Ok(uvt)
}

/// Catálogo completo de la plataforma: todas las versiones de los conceptos y los valores de la
/// UVT.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`.
pub async fn catalogo_retenciones_plataforma<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<(Vec<ConceptoRetencion>, Vec<ValorUvt>)>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoRetencionesPlataforma,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    Ok((tx.conceptos_retencion(None).await?, tx.valores_uvt().await?))
}

/// Conceptos de retención (vigentes en la fecha, o todas las versiones).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::ModuloNoHabilitado`].
pub async fn conceptos_retencion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    vigentes_en: Option<NaiveDate>,
) -> AppResult<Vec<ConceptoRetencion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    tx.conceptos_retencion(vigentes_en).await
}

/// Valores de la UVT.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::ModuloNoHabilitado`].
pub async fn valores_uvt<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<ValorUvt>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    tx.valores_uvt().await
}

// ---------------------------------------------------------------------------------------------
// Facturas de proveedores
// ---------------------------------------------------------------------------------------------

/// Línea de una factura nueva (por códigos).
#[derive(Debug, Clone)]
pub struct NuevaLineaFacturaProveedor {
    pub descripcion: String,
    /// Cuenta de gasto, costo o activo.
    pub cuenta: CodigoCuenta,
    /// Código del centro de costo.
    pub centro_costo: Option<String>,
    pub base: Monto,
    pub iva: Monto,
    pub iva_descontable: bool,
    /// Códigos de los conceptos de retención que aplican (a lo sumo uno por impuesto).
    pub retenciones: Vec<String>,
}

/// Factura de proveedor nueva.
#[derive(Debug, Clone)]
pub struct NuevaFacturaProveedor {
    pub persona: PersonaId,
    pub prefijo: String,
    pub numero: String,
    pub fecha: NaiveDate,
    pub vencimiento: NaiveDate,
    pub descripcion: String,
    pub origen: Option<Origen>,
    pub cufe: Option<String>,
    pub archivo_id: Option<domain::ArchivoId>,
    pub lineas: Vec<NuevaLineaFacturaProveedor>,
    /// Motivo para aceptar un número que el proveedor ya usó (exige
    /// `tesoreria.egresos.autorizar`).
    pub excepcion_duplicado: Option<String>,
}

fn validar_numeracion(prefijo: &str, numero: &str) -> AppResult<()> {
    let prefijo_ok = prefijo.len() <= 10
        && prefijo
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    let numero_ok = (1..=20).contains(&numero.len())
        && numero
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if prefijo_ok && numero_ok {
        Ok(())
    } else {
        Err(validacion(
            "prefijo de hasta 10 mayúsculas o dígitos y número de hasta 20 caracteres",
        ))
    }
}

/// Calcula las retenciones de una línea con la versión vigente de cada concepto en la fecha.
async fn retenciones_de_linea<T: TxTesoreria>(
    tx: &mut T,
    linea: &NuevaLineaFacturaProveedor,
    fecha: NaiveDate,
    uvt: Option<Decimal>,
    tercero: &PerfilRetencionTercero,
) -> AppResult<Vec<RetencionLinea>> {
    let mut impuestos = BTreeSet::new();
    let mut v = Vec::new();
    for codigo in &linea.retenciones {
        let concepto = tx
            .concepto_retencion_vigente(codigo, fecha)
            .await?
            .ok_or_else(|| {
                validacion(format!(
                    "el concepto de retención `{codigo}` no está vigente en {fecha}"
                ))
            })?;
        if !impuestos.insert(concepto.impuesto) {
            return Err(validacion(
                "una línea lleva a lo sumo una retención por impuesto",
            ));
        }
        let uvt =
            uvt.ok_or_else(|| validacion(format!("no hay valor de la UVT para {}", fecha.year())))?;
        let base = match concepto.impuesto {
            ImpuestoRetencion::Iva => linea.iva,
            ImpuestoRetencion::Renta | ImpuestoRetencion::Ica => linea.base,
        };
        let tarifa = TarifaRetencion {
            impuesto: concepto.impuesto,
            base_minima_uvt: concepto.base_minima_uvt,
            tarifa: concepto.tarifa,
        };
        if let Some(valor) = calcular_retencion(&tarifa, base, uvt, tercero)? {
            v.push(RetencionLinea {
                concepto_id: concepto.id,
                impuesto: concepto.impuesto,
                base,
                tarifa: concepto.tarifa,
                valor,
            });
        }
    }
    Ok(v)
}

/// Registra una factura de proveedor y calcula sus retenciones con la tarifa vigente en la fecha
/// de la factura (sin causarla todavía). Solo practica retenciones una entidad con perfil
/// tributario confirmado de agente de retención.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.facturas_proveedor.gestionar` (o sin
/// `tesoreria.egresos.autorizar` para la excepción de duplicado); [`AppError::Validacion`] si un
/// dato no cumple o un concepto no está vigente; [`AppError::Conflicto`] si el proveedor ya
/// tiene esa factura (QA del plan: factura duplicada rechazada salvo excepción).
pub async fn registrar_factura_proveedor<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaFacturaProveedor,
) -> AppResult<FacturaProveedor>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let excepcion = texto_opcional(
        "excepcion_duplicado",
        nueva.excepcion_duplicado.as_deref(),
        500,
    )?;
    let autoriza = match &excepcion {
        None => None,
        Some(motivo) => {
            ctx.exigir(Permiso::TesoreriaEgresosAutorizar)?;
            if motivo.chars().count() < 5 {
                return Err(validacion("explique el motivo de la excepción"));
            }
            Some(
                ctx.actor_id
                    .ok_or_else(|| validacion("la excepción la autoriza una cuenta"))?,
            )
        }
    };
    let prefijo = nueva.prefijo.trim().to_owned();
    let numero = nueva.numero.trim().to_owned();
    validar_numeracion(&prefijo, &numero)?;
    if nueva.vencimiento < nueva.fecha {
        return Err(validacion(
            "el vencimiento no puede ser anterior a la fecha",
        ));
    }
    if nueva.lineas.is_empty() || nueva.lineas.len() > MAXIMO_LINEAS {
        return Err(validacion(format!(
            "la factura lleva de 1 a {MAXIMO_LINEAS} líneas"
        )));
    }
    let cufe = texto_opcional("cufe", nueva.cufe.as_deref(), 96)?.map(|c| c.to_lowercase());
    if cufe
        .as_deref()
        .is_some_and(|c| c.len() != 96 || !c.chars().all(|x| x.is_ascii_hexdigit()))
    {
        return Err(validacion("el CUFE tiene 96 caracteres hexadecimales"));
    }
    let mut tx = abrir(
        unidad,
        reloj,
        ctx,
        Permiso::TesoreriaFacturasProveedorGestionar,
    )
    .await?;
    let fiscal = tx
        .datos_fiscales(nueva.persona)
        .await?
        .ok_or(AppError::NoEncontrado("tercero"))?;
    let perfil = PerfilRetencionTercero {
        responsabilidades: fiscal.responsabilidades,
        responsable_iva: fiscal.regimen_iva == Some(RegimenIva::Responsable),
    };
    let uvt = tx.uvt(nueva.fecha.year()).await?;
    // Sin perfil tributario confirmado no hay automatismos (plan 15): solo un agente de
    // retención confirmado practica retenciones.
    if nueva.lineas.iter().any(|l| !l.retenciones.is_empty()) {
        let agente = super::tributario::perfil_confirmado_en(&mut tx, nueva.fecha)
            .await?
            .is_some_and(|p| p.parametros.agente_retenedor);
        if !agente {
            return Err(validacion(
                "la entidad no practica retenciones: su perfil tributario confirmado no la hace \
                 agente de retención (o el perfil sigue en borrador)",
            ));
        }
    }
    let mut lineas = Vec::with_capacity(nueva.lineas.len());
    for l in &nueva.lineas {
        if l.base.es_cero() {
            return Err(validacion("la base de cada línea debe ser mayor que cero"));
        }
        lineas.push(LineaFacturaProveedor {
            descripcion: texto("descripcion", &l.descripcion, 300)?,
            cuenta_id: cuenta_de_movimiento(&mut tx, &l.cuenta).await?,
            centro_costo_id: centro_por_codigo(&mut tx, l.centro_costo.as_deref()).await?,
            base: l.base,
            iva: l.iva,
            iva_descontable: l.iva_descontable,
            retenciones: retenciones_de_linea(&mut tx, l, nueva.fecha, uvt, &perfil).await?,
        });
    }
    let subtotal: Monto = lineas.iter().map(|l| l.base).sum();
    let iva: Monto = lineas.iter().map(|l| l.iva).sum();
    let retenciones: Monto = lineas
        .iter()
        .flat_map(|l| l.retenciones.iter().map(|r| r.valor))
        .sum();
    let neto = subtotal.valor() + iva.valor() - retenciones.valor();
    if neto <= Decimal::ZERO {
        return Err(validacion(
            "las retenciones no pueden igualar o superar la factura",
        ));
    }
    let factura = FacturaProveedor {
        id: FacturaProveedorId::nuevo(),
        persona_id: nueva.persona,
        prefijo,
        numero,
        fecha: nueva.fecha,
        vencimiento: nueva.vencimiento,
        descripcion: texto("descripcion", &nueva.descripcion, 300)?,
        origen: nueva.origen,
        cufe,
        archivo_id: nueva.archivo_id,
        subtotal,
        iva,
        retenciones,
        neto_pagar: Monto::new(neto)?,
        pagado: Monto::CERO,
        estado: EstadoFacturaProveedor::Registrada,
        excepcion_duplicado: excepcion,
        excepcion_autorizada_por: autoriza,
        comprobante_id: None,
        motivo_anulacion: None,
        registrada_por: ctx.actor_id,
        registrada_en: reloj.ahora(),
        lineas,
    };
    tx.insertar_factura_proveedor(&factura).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("neto", factura.neto_pagar.valor().to_string())?
        .con("retenciones", factura.retenciones.valor().to_string())?
        .con("excepcion_duplicado", factura.excepcion_duplicado.is_some())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_REGISTRAR, "factura_proveedor", factura.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(factura)
}

async fn factura_existente<T: TxTesoreria>(
    tx: &mut T,
    id: FacturaProveedorId,
) -> AppResult<FacturaProveedor> {
    tx.factura_proveedor(id)
        .await?
        .ok_or(AppError::NoEncontrado("factura de proveedor"))
}

/// Cuenta de la entidad para una retención: la del código sugerido por el catálogo si la
/// entidad la tiene; si no, la de la regla.
async fn cuenta_retencion<T: TxTesoreria>(
    tx: &mut T,
    concepto: ConceptoRetencionId,
) -> AppResult<Option<domain::CuentaContableId>> {
    let Some(sugerida) = tx
        .concepto_retencion(concepto)
        .await?
        .and_then(|c| c.cuenta_sugerida)
    else {
        return Ok(None);
    };
    Ok(tx
        .cuenta_por_codigo(&sugerida)
        .await?
        .filter(|c| c.acepta_movimiento && c.activa)
        .map(|c| c.id))
}

/// Causa la factura: gasto (más el IVA no descontable), IVA descontable, retenciones y cuenta
/// por pagar al proveedor, en la fecha de la factura.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.facturas_proveedor.gestionar`;
/// [`AppError::Conflicto`] si ya se causó o se anuló; los del contabilizador.
pub async fn causar_factura_proveedor<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaProveedorId,
) -> AppResult<FacturaProveedor>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(
        unidad,
        reloj,
        ctx,
        Permiso::TesoreriaFacturasProveedorGestionar,
    )
    .await?;
    let mut f = factura_existente(&mut tx, id).await?;
    f.estado = f.estado.transicionar(EstadoFacturaProveedor::Causada)?;
    let proveedor = Some(f.persona_id);
    let mut importes = Vec::new();
    for l in &f.lineas {
        let gasto = if l.iva_descontable {
            l.base
        } else {
            l.base + l.iva
        };
        importes.push(
            ImporteDocumento::new(conceptos::GASTO, gasto)
                .con_persona(proveedor)
                .con_centro(l.centro_costo_id)
                .con_cuenta(Some(l.cuenta_id))
                .con_descripcion(Some(l.descripcion.clone())),
        );
        if l.iva_descontable && !l.iva.es_cero() {
            importes.push(
                ImporteDocumento::new(conceptos::IVA_DESCONTABLE, l.iva)
                    .con_persona(proveedor)
                    .con_base(Some(l.base)),
            );
        }
        for r in &l.retenciones {
            let cuenta = cuenta_retencion(&mut tx, r.concepto_id).await?;
            importes.push(
                ImporteDocumento::new(r.impuesto.concepto_contable(), r.valor)
                    .con_persona(proveedor)
                    .con_base(Some(r.base))
                    .con_cuenta(cuenta),
            );
        }
    }
    importes.push(ImporteDocumento::new(conceptos::POR_PAGAR, f.neto_pagar).con_persona(proveedor));
    let numero = if f.prefijo.is_empty() {
        f.numero.clone()
    } else {
        format!("{}-{}", f.prefijo, f.numero)
    };
    let comprobante = comprobantes::contabilizar_documento(
        &mut tx,
        reloj,
        ctx,
        DocumentoOrigen {
            evento: eventos::FACTURA_PROVEEDOR.into(),
            origen: origen(eventos::FACTURA_PROVEEDOR, id.as_uuid()),
            fecha: f.fecha,
            descripcion: format!("Factura {numero}: {}", f.descripcion),
            importes,
        },
    )
    .await?;
    f.comprobante_id = Some(comprobante);
    tx.actualizar_factura_proveedor(&f).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_CAUSAR,
        "factura_proveedor",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(f)
}

/// Anula una factura sin pagos (si estaba causada, revierte su comprobante).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.facturas_proveedor.gestionar`;
/// [`domain::DominioError::TransicionInvalida`] si tiene pagos o ya se anuló; los del reverso.
pub async fn anular_factura_proveedor<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaProveedorId,
    motivo: &str,
) -> AppResult<FacturaProveedor>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(
        unidad,
        reloj,
        ctx,
        Permiso::TesoreriaFacturasProveedorGestionar,
    )
    .await?;
    let mut f = factura_existente(&mut tx, id).await?;
    f.estado = f.estado.transicionar(EstadoFacturaProveedor::Anulada)?;
    if let Some(c) = f.comprobante_id {
        revertir_en(&mut tx, reloj, ctx, c, motivo, None).await?;
    }
    f.motivo_anulacion = Some(texto("motivo", motivo, 500)?);
    tx.actualizar_factura_proveedor(&f).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_ANULAR,
        "factura_proveedor",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(f)
}

/// Factura de proveedor por id, con sus líneas y retenciones.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::NoEncontrado`].
pub async fn factura_proveedor<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: FacturaProveedorId,
) -> AppResult<FacturaProveedor>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    factura_existente(&mut tx, id).await
}

/// Facturas de proveedor por vencimiento (a lo sumo [`LIMITE_LISTADO`]).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn facturas_proveedor<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroFacturas,
) -> AppResult<Vec<FacturaProveedor>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    tx.facturas_proveedor(&filtro, LIMITE_LISTADO).await
}

/// Verifica que un egreso pueda pagar `valor` de la factura del beneficiario.
pub(crate) async fn validar_pago<T: TxTesoreria>(
    tx: &mut T,
    id: FacturaProveedorId,
    valor: Monto,
    beneficiario: PersonaId,
) -> AppResult<FacturaProveedor> {
    let f = factura_existente(tx, id).await?;
    if f.persona_id != beneficiario {
        return Err(validacion("la factura es de otro proveedor"));
    }
    if !f.estado.admite_pago() {
        return Err(AppError::Conflicto(format!(
            "la factura está {} y no admite pagos",
            f.estado.as_str()
        )));
    }
    if valor.es_cero() || valor.valor() > f.neto_pagar.valor() - f.pagado.valor() {
        return Err(validacion(format!(
            "el saldo de la factura es {}",
            f.neto_pagar.valor() - f.pagado.valor()
        )));
    }
    Ok(f)
}

/// Abona `valor` a la factura (dentro de la transacción del egreso).
pub(crate) async fn pagar_factura<T: TxTesoreria>(
    tx: &mut T,
    id: FacturaProveedorId,
    valor: Monto,
    beneficiario: PersonaId,
) -> AppResult<()> {
    let mut f = validar_pago(tx, id, valor, beneficiario).await?;
    f.pagado = f.pagado + valor;
    let hacia = match situacion_pago(f.neto_pagar, f.pagado)? {
        Some(true) => EstadoFacturaProveedor::Pagada,
        _ => EstadoFacturaProveedor::PagadaParcial,
    };
    f.estado = f.estado.transicionar(hacia)?;
    tx.actualizar_factura_proveedor(&f).await
}

/// Devuelve `valor` al saldo de la factura (anulación del egreso).
pub(crate) async fn despagar_factura<T: TxTesoreria>(
    tx: &mut T,
    id: FacturaProveedorId,
    valor: Monto,
) -> AppResult<()> {
    let mut f = factura_existente(tx, id).await?;
    f.pagado = Monto::new(f.pagado.valor() - valor.valor())?;
    let hacia = if f.pagado.es_cero() {
        EstadoFacturaProveedor::Causada
    } else {
        EstadoFacturaProveedor::PagadaParcial
    };
    f.estado = f.estado.transicionar(hacia)?;
    tx.actualizar_factura_proveedor(&f).await
}

// ---------------------------------------------------------------------------------------------
// Certificados de retención
// ---------------------------------------------------------------------------------------------

/// Certificado de las retenciones practicadas a un tercero en un periodo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificadoRetenciones {
    /// Agente retenedor.
    pub entidad: String,
    pub entidad_nit: Option<String>,
    pub tercero: String,
    /// Documento del tercero (dato personal si es persona natural).
    pub tercero_documento: Option<String>,
    pub desde: NaiveDate,
    pub hasta: NaiveDate,
    pub filas: Vec<FilaCertificado>,
    pub total_base: Decimal,
    pub total_retenido: Decimal,
}

async fn armar_certificado<T: TxTesoreria>(
    tx: &mut T,
    persona: PersonaId,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<(CertificadoRetenciones, Option<DireccionCorreo>)> {
    if hasta < desde {
        return Err(validacion("el periodo termina antes de empezar"));
    }
    let p = tx
        .persona(persona)
        .await?
        .ok_or(AppError::NoEncontrado("tercero"))?;
    let fiscal = tx.datos_fiscales(persona).await?.unwrap_or_default();
    let entidad = tx.datos_entidad().await?;
    let filas = tx.retenciones_de_tercero(persona, desde, hasta).await?;
    let correo = match fiscal.correo_facturacion.as_deref() {
        Some(c) => DireccionCorreo::new(c).ok(),
        None => p.correo.clone(),
    };
    let certificado = CertificadoRetenciones {
        entidad: entidad.nombre,
        entidad_nit: entidad.nit,
        tercero: p.nombre,
        tercero_documento: p.documento.as_ref().map(|d| d.as_str().to_owned()),
        desde,
        hasta,
        total_base: filas.iter().map(|f| f.base).sum(),
        total_retenido: filas.iter().map(|f| f.valor).sum(),
        filas,
    };
    Ok((certificado, correo))
}

/// Certificado de retenciones del tercero entre dos fechas (reporte sobre las retenciones de las
/// facturas no anuladas).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tesoreria.facturas_proveedor.gestionar`;
/// [`AppError::NoEncontrado`] si el tercero no existe.
pub async fn certificado_retenciones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    persona: PersonaId,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<CertificadoRetenciones>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(
        unidad,
        reloj,
        ctx,
        Permiso::TesoreriaFacturasProveedorGestionar,
    )
    .await?;
    Ok(armar_certificado(&mut tx, persona, desde, hasta).await?.0)
}

fn filas_texto(c: &CertificadoRetenciones) -> Vec<Vec<String>> {
    let mut filas: Vec<Vec<String>> = c
        .filas
        .iter()
        .map(|f| {
            vec![
                f.impuesto.as_str().to_owned(),
                f.concepto.clone(),
                format!("{} %", f.tarifa.normalize()),
                f.base.to_string(),
                f.valor.to_string(),
            ]
        })
        .collect();
    filas.push(vec![
        "Total".into(),
        String::new(),
        String::new(),
        c.total_base.to_string(),
        c.total_retenido.to_string(),
    ]);
    filas
}

/// El certificado en PDF.
///
/// # Errors
///
/// Los de [`certificado_retenciones`]; [`AppError::Infraestructura`] si falla el PDF.
pub async fn certificado_retenciones_pdf<U, G>(
    unidad: &U,
    generador: &G,
    reloj: &impl Reloj,
    ctx: &Contexto,
    persona: PersonaId,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<Vec<u8>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
    G: GeneradorActas,
{
    let c = certificado_retenciones(unidad, reloj, ctx, persona, desde, hasta).await?;
    generador
        .tabla(&TablaPdf {
            titulo: "Certificado de retenciones".into(),
            copropiedad: match &c.entidad_nit {
                Some(nit) => format!("{} · NIT {nit}", c.entidad),
                None => c.entidad.clone(),
            },
            subtitulo: format!(
                "Retenido a {}{} entre {} y {}",
                c.tercero,
                c.tercero_documento
                    .as_deref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default(),
                c.desde,
                c.hasta
            ),
            columnas: ["Impuesto", "Concepto", "Tarifa", "Base", "Retenido"]
                .map(str::to_owned)
                .to_vec(),
            filas: filas_texto(&c),
            generada: reloj.ahora().format("%Y-%m-%d %H:%M UTC").to_string(),
        })
        .await
}

/// Envía el certificado por correo (outbox) al correo de facturación del tercero o, si no lo
/// tiene, a su correo. Devuelve la dirección.
///
/// # Errors
///
/// Los de [`certificado_retenciones`]; [`AppError::Validacion`] si el tercero no tiene correo.
pub async fn enviar_certificado_retenciones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    persona: PersonaId,
    desde: NaiveDate,
    hasta: NaiveDate,
) -> AppResult<DireccionCorreo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria + RegistroOutbox,
{
    let mut tx = abrir(
        unidad,
        reloj,
        ctx,
        Permiso::TesoreriaFacturasProveedorGestionar,
    )
    .await?;
    let (c, correo) = armar_certificado(&mut tx, persona, desde, hasta).await?;
    let para = correo.ok_or_else(|| validacion("el tercero no tiene correo"))?;
    let mut cuerpo = format!(
        "{} certifica las retenciones practicadas a {} entre {} y {}:\n\n",
        c.entidad, c.tercero, c.desde, c.hasta
    );
    for f in filas_texto(&c) {
        cuerpo.push_str(&f.join(" | "));
        cuerpo.push('\n');
    }
    tx.encolar(EventoSaliente {
        prioridad: PrioridadSalida::Evento,
        mensaje: MensajeOutbox::Correo(CorreoSaliente {
            para: para.clone(),
            asunto: format!("Certificado de retenciones · {}", c.entidad),
            texto: cuerpo,
            html: None,
        }),
    })
    .await?;
    let detalle = DetalleAuditoria::vacio()
        .con("desde", c.desde.to_string())?
        .con("hasta", c.hasta.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CERTIFICADO, "persona", persona.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(para)
}
