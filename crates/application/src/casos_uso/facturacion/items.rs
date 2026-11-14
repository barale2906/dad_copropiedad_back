//! Catálogo de productos y servicios (plan 18 fase A3): lo que la entidad factura, con su precio
//! sugerido, su unidad DIAN, su IVA y su clasificación tributaria. Una línea de factura puede
//! salir de un ítem y tomar de él lo que no traiga. La cuenta contable de ingreso solo se elige
//! con el módulo de contabilidad.

use domain::{
    ItemFacturacionId, Permiso,
    contabilidad::{CodigoCuenta, Monto},
    facturacion::{validar_codigo_item, validar_iva_item, validar_unidad},
    tributario::ClasificacionTributaria,
};
use rust_decimal::Decimal;

use super::{TxVentas, abrir_ventas, con_contabilidad};
use crate::casos_uso::contabilidad::prelude::*;
use crate::casos_uso::contabilidad::{tesoreria::cuenta_de_movimiento, texto};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        Reloj, UnidadDeTrabajo,
        facturacion::{FiltroItems, ItemFacturacion, RepositorioFacturacion as _},
    },
};

const ACCION_CREAR: AccionAuditoria = AccionAuditoria::new("facturacion.item.crear");
const ACCION_EDITAR: AccionAuditoria = AccionAuditoria::new("facturacion.item.editar");

/// Máximo de ítems en un listado.
pub const LIMITE_ITEMS: u32 = 500;

/// Datos de un producto o servicio (el código solo se da al crearlo).
#[derive(Debug, Clone)]
pub struct DatosItem {
    pub descripcion: String,
    /// Código de unidad de la DIAN; `94` (unidad) si no viene.
    pub unidad: Option<String>,
    pub precio: Option<Monto>,
    pub tarifa_iva: Decimal,
    pub clasificacion: ClasificacionTributaria,
    /// Cuenta de ingreso (solo con contabilidad).
    pub cuenta: Option<CodigoCuenta>,
    pub activo: bool,
}

/// Valida los datos y resuelve la cuenta dentro de la transacción.
async fn validar<T: TxVentas>(
    tx: &mut T,
    reloj: &impl Reloj,
    datos: &DatosItem,
) -> AppResult<(String, String, Option<domain::CuentaContableId>)> {
    let descripcion = texto("descripcion", &datos.descripcion, 300)?;
    let unidad = validar_unidad(datos.unidad.as_deref().unwrap_or("94"))?;
    validar_iva_item(datos.tarifa_iva, datos.clasificacion)?;
    let cuenta = match &datos.cuenta {
        None => None,
        Some(c) => {
            if !con_contabilidad(tx, reloj).await? {
                return Err(AppError::Validacion(
                    "sin el módulo de contabilidad el ítem no lleva cuenta contable".into(),
                ));
            }
            Some(cuenta_de_movimiento(tx, c).await?)
        }
    };
    Ok((descripcion, unidad, cuenta))
}

fn auditar(i: &ItemFacturacion) -> AppResult<DetalleAuditoria> {
    DetalleAuditoria::vacio()
        .con("codigo", i.codigo.clone())?
        .con("activo", i.activo)
}

/// Crea un producto o servicio.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.items.gestionar`;
/// [`AppError::ModuloNoHabilitado`] sin facturación ni contabilidad; [`AppError::Dominio`] o
/// [`AppError::Validacion`] si un dato no cumple; [`AppError::Conflicto`] si el código ya existe.
pub async fn crear_item<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    codigo: &str,
    datos: DatosItem,
) -> AppResult<ItemFacturacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    let codigo = validar_codigo_item(codigo)?;
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionItemsGestionar).await?;
    let (descripcion, unidad_dian, cuenta_id) = validar(&mut tx, reloj, &datos).await?;
    let ahora = reloj.ahora();
    let item = ItemFacturacion {
        id: ItemFacturacionId::nuevo(),
        codigo,
        descripcion,
        unidad: unidad_dian,
        precio: datos.precio,
        tarifa_iva: datos.tarifa_iva,
        clasificacion: datos.clasificacion,
        cuenta_id,
        activo: datos.activo,
        creado_por: ctx.actor_id,
        creado_en: ahora,
        actualizado_en: ahora,
    };
    tx.insertar_item(&item).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR, "item_facturacion", item.id.as_uuid())
            .con_detalle(auditar(&item)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(item)
}

/// Reemplaza los datos de un ítem (el código no cambia). Desactivarlo lo saca del catálogo de
/// las facturas nuevas; las ya emitidas no cambian.
///
/// # Errors
///
/// Los de [`crear_item`]; [`AppError::NoEncontrado`] si el ítem no existe.
pub async fn actualizar_item<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ItemFacturacionId,
    datos: DatosItem,
) -> AppResult<ItemFacturacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionItemsGestionar).await?;
    let mut item = tx.item(id).await?.ok_or(AppError::NoEncontrado("ítem"))?;
    let (descripcion, unidad_dian, cuenta_id) = validar(&mut tx, reloj, &datos).await?;
    item.descripcion = descripcion;
    item.unidad = unidad_dian;
    item.precio = datos.precio;
    item.tarifa_iva = datos.tarifa_iva;
    item.clasificacion = datos.clasificacion;
    item.cuenta_id = cuenta_id;
    item.activo = datos.activo;
    item.actualizado_en = reloj.ahora();
    tx.actualizar_item(&item).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EDITAR, "item_facturacion", item.id.as_uuid())
            .con_detalle(auditar(&item)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(item)
}

/// Catálogo por descripción (a lo sumo [`LIMITE_ITEMS`]).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.ver`; [`AppError::ModuloNoHabilitado`].
pub async fn items<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroItems,
) -> AppResult<Vec<ItemFacturacion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionVer).await?;
    tx.items(&filtro, LIMITE_ITEMS).await
}

/// Ítem por id.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `facturacion.ver`; [`AppError::NoEncontrado`].
pub async fn item<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ItemFacturacionId,
) -> AppResult<ItemFacturacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    let mut tx = abrir_ventas(unidad, reloj, ctx, Permiso::FacturacionVer).await?;
    tx.item(id).await?.ok_or(AppError::NoEncontrado("ítem"))
}
