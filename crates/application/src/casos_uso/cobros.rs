//! Cobro de la plataforma a sus clientes (plan 19, ADR 0024).
//!
//! - **Catálogo comercial** ([`crear_producto`], [`fijar_tarifa`]) y **suscripciones**
//!   ([`crear_suscripcion`], [`agregar_linea`]): `plataforma.comercial.gestionar`. Los precios los
//!   define la plataforma (sin descuentos automáticos ni topes); una línea puede llevar un precio
//!   pactado con su motivo.
//! - **Cortes** ([`cortes`]): se calculan al cierre de cada periodo (o a pedido) con las reglas
//!   puras de `domain::cobro_plataforma`, se aprueban solos si cobran lo mismo que el anterior y
//!   se facturan como facturas normales del tenant de la entidad de la plataforma.
//! - **Tareas del worker** ([`tareas`]): cortes de los periodos vencidos y calendario de mora
//!   (avisos y suspensión en solo lectura).
//! - **Lo que ve la entidad** ([`portal`]): su plan, su consumo y sus facturas.

use chrono::NaiveDate;
use domain::{
    DireccionCorreo, ItemFacturacionId, LineaSuscripcionId, Modulo, OrganizacionId, Permiso,
    ProductoPlataformaId, SuscripcionId, TenantId,
    cobro_plataforma::{
        Ciclo, EstadoSuscripcion, PLAZO_PAGO_POR_DEFECTO, Tarifa, validar_dias_prueba,
    },
    copropiedad::{texto_obligatorio, texto_opcional},
};
use rust_decimal::Decimal;

use crate::{
    AppError, AppResult, ContextoPlataforma,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, RegistroOutbox, Reloj, RepositorioCobros, RepositorioPlataforma,
        Transaccion, UnidadDeTrabajoPlataforma,
        cobros::{LineaSuscripcion, Pagador, Producto, Suscripcion, TarifaProducto},
    },
};

pub mod cortes;
pub mod portal;
pub mod tareas;

pub(crate) const ACCION_PRODUCTO: AccionAuditoria =
    AccionAuditoria::new("plataforma.producto.guardar");
pub(crate) const ACCION_TARIFA: AccionAuditoria = AccionAuditoria::new("plataforma.tarifa.fijar");
pub(crate) const ACCION_SUSCRIPCION: AccionAuditoria =
    AccionAuditoria::new("plataforma.suscripcion.guardar");
pub(crate) const ACCION_LINEA: AccionAuditoria =
    AccionAuditoria::new("plataforma.suscripcion.linea");

/// Lo que necesita la transacción de plataforma para cobrar.
pub trait TxCobros:
    RepositorioCobros + RepositorioPlataforma + RegistroAuditoria + RegistroOutbox
{
}

impl<T> TxCobros for T where
    T: RepositorioCobros + RepositorioPlataforma + RegistroAuditoria + RegistroOutbox
{
}

fn validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

// ---------------------------------------------------------------------------------------------
// Catálogo
// ---------------------------------------------------------------------------------------------

/// Datos de un producto del catálogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosProducto {
    pub codigo: String,
    pub nombre: String,
    pub descripcion_factura: String,
    pub modulo: Option<Modulo>,
    pub item_facturacion_id: Option<ItemFacturacionId>,
    pub activo: bool,
}

fn validar_producto(d: DatosProducto) -> AppResult<DatosProducto> {
    let codigo = d.codigo.trim().to_owned();
    let valido = (2..=40).contains(&codigo.len())
        && codigo.starts_with(|c: char| c.is_ascii_lowercase())
        && codigo
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valido {
        return Err(validacion(
            "el código del producto es de 2 a 40 caracteres a-z, 0-9 o _, empezando por letra",
        ));
    }
    Ok(DatosProducto {
        codigo,
        nombre: texto_obligatorio("nombre", &d.nombre, 120)?,
        descripcion_factura: texto_obligatorio("descripcion_factura", &d.descripcion_factura, 300)?,
        ..d
    })
}

/// Crea un producto del catálogo comercial.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.comercial.gestionar`; [`AppError::Validacion`] si
/// los datos no son válidos; [`AppError::Conflicto`] si el código ya existe.
pub async fn crear_producto<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    datos: DatosProducto,
) -> AppResult<Producto>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::PlataformaComercialGestionar)?;
    let d = validar_producto(datos)?;
    let p = Producto {
        id: ProductoPlataformaId::nuevo(),
        codigo: d.codigo,
        nombre: d.nombre,
        descripcion_factura: d.descripcion_factura,
        modulo: d.modulo,
        item_facturacion_id: d.item_facturacion_id,
        activo: d.activo,
        creado_en: reloj.ahora(),
    };
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.insertar_producto(&p).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PRODUCTO,
        "producto_plataforma",
        p.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Edita un producto (el código no cambia).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Validacion`].
pub async fn editar_producto<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    id: ProductoPlataformaId,
    datos: DatosProducto,
) -> AppResult<Producto>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::PlataformaComercialGestionar)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut p = tx
        .producto(id)
        .await?
        .ok_or(AppError::NoEncontrado("producto"))?;
    let d = validar_producto(DatosProducto {
        codigo: p.codigo.clone(),
        ..datos
    })?;
    p.nombre = d.nombre;
    p.descripcion_factura = d.descripcion_factura;
    p.modulo = d.modulo;
    p.item_facturacion_id = d.item_facturacion_id;
    p.activo = d.activo;
    tx.actualizar_producto(&p).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PRODUCTO,
        "producto_plataforma",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(p)
}

/// Producto con sus tarifas (la más reciente primero).
pub type ProductoConTarifas = (Producto, Vec<TarifaProducto>);

/// Catálogo comercial con las tarifas de cada producto.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.comercial.gestionar` ni `plataforma.cobros.aprobar`.
pub async fn catalogo<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<ProductoConTarifas>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    exigir_alguno(
        ctx,
        &[
            Permiso::PlataformaComercialGestionar,
            Permiso::PlataformaCobrosAprobar,
        ],
    )?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut v = Vec::new();
    for p in tx.productos().await? {
        let t = tx.tarifas(p.id).await?;
        v.push((p, t));
    }
    Ok(v)
}

fn exigir_alguno(ctx: &ContextoPlataforma, permisos: &[Permiso]) -> AppResult<()> {
    if permisos.iter().any(|p| ctx.permisos.contiene(*p)) {
        Ok(())
    } else {
        ctx.exigir(permisos[0])
    }
}

/// Fija la tarifa del producto desde una fecha (versionada: la anterior rige hasta ese día).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si la tarifa no
/// es coherente con su modelo; [`AppError::Conflicto`] si ya hay una desde ese día.
pub async fn fijar_tarifa<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    producto: ProductoPlataformaId,
    tarifa: Tarifa,
    vigente_desde: NaiveDate,
) -> AppResult<TarifaProducto>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::PlataformaComercialGestionar)?;
    let t = TarifaProducto {
        id: uuid::Uuid::now_v7(),
        producto_id: producto,
        tarifa: tarifa.validada()?,
        vigente_desde,
        creada_por: ctx.actor_id,
        creada_en: reloj.ahora(),
    };
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.producto(producto)
        .await?
        .ok_or(AppError::NoEncontrado("producto"))?;
    tx.insertar_tarifa(&t).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("modelo", t.tarifa.modelo.as_str())?
        .con("valor", t.tarifa.valor.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_TARIFA, "producto_plataforma", producto.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(t)
}

// ---------------------------------------------------------------------------------------------
// Suscripciones
// ---------------------------------------------------------------------------------------------

/// Datos de una suscripción nueva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevaSuscripcion {
    pub tenant_id: TenantId,
    pub pagador: Pagador,
    pub ciclo: Ciclo,
    pub inicio: NaiveDate,
    pub dias_prueba: u16,
    /// `None` = [`PLAZO_PAGO_POR_DEFECTO`].
    pub plazo_pago_dias: Option<u16>,
    pub correo_facturacion: DireccionCorreo,
}

/// Exige que la entidad exista (vista de la plataforma).
async fn exigir_entidad<T: TxCobros>(
    tx: &mut T,
    tenant: TenantId,
    ahora: chrono::DateTime<chrono::Utc>,
) -> AppResult<domain::TipoEntidad> {
    tx.entidades(
        None,
        Some(tenant),
        crate::SolicitudPagina::new(Some(1), None)?,
        ahora,
    )
    .await?
    .datos
    .into_iter()
    .next()
    .map(|e| e.tipo)
    .ok_or(AppError::NoEncontrado("entidad"))
}

async fn exigir_organizacion<T: TxCobros>(tx: &mut T, id: OrganizacionId) -> AppResult<()> {
    tx.cliente_organizacion(id)
        .await?
        .map(|_| ())
        .ok_or(AppError::NoEncontrado("organización pagadora"))
}

/// Crea la suscripción de una entidad (una vigente por entidad).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.comercial.gestionar`; [`AppError::NoEncontrado`]
/// si la entidad o el pagador no existen; [`AppError::Dominio`] si el ciclo o la prueba no son
/// válidos; [`AppError::Conflicto`] si ya tiene una vigente.
pub async fn crear_suscripcion<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    nueva: NuevaSuscripcion,
) -> AppResult<Suscripcion>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::PlataformaComercialGestionar)?;
    let ahora = reloj.ahora();
    let s = Suscripcion {
        id: SuscripcionId::nuevo(),
        tenant_id: nueva.tenant_id,
        pagador: nueva.pagador,
        ciclo: nueva.ciclo.validado()?,
        inicio: nueva.inicio,
        dias_prueba: validar_dias_prueba(nueva.dias_prueba)?,
        plazo_pago_dias: nueva.plazo_pago_dias.unwrap_or(PLAZO_PAGO_POR_DEFECTO),
        correo_facturacion: nueva.correo_facturacion,
        estado: EstadoSuscripcion::Activa,
        suspendida_por_mora: false,
        terminada_en: None,
        creada_por: ctx.actor_id,
        creada_en: ahora,
    };
    if s.plazo_pago_dias > 120 {
        return Err(validacion("el plazo de pago es de 0 a 120 días"));
    }
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    if exigir_entidad(&mut tx, s.tenant_id, ahora).await? == domain::TipoEntidad::Plataforma {
        return Err(validacion("la plataforma no se cobra a sí misma"));
    }
    match s.pagador {
        Pagador::MismaEntidad => {}
        Pagador::Entidad(e) => {
            exigir_entidad(&mut tx, e, ahora).await?;
        }
        Pagador::Organizacion(o) => exigir_organizacion(&mut tx, o).await?,
    }
    tx.insertar_suscripcion(&s).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_SUSCRIPCION,
        "suscripcion",
        s.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(s)
}

/// Suscripción con sus líneas y los módulos habilitados que no tienen línea comercial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetalleSuscripcion {
    pub suscripcion: Suscripcion,
    pub lineas: Vec<(LineaSuscripcion, Producto)>,
    /// Módulos habilitados en la entidad sin un producto que los cobre (para no olvidarlos).
    pub modulos_sin_cobro: Vec<String>,
}

/// Detalle de una suscripción.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.comercial.gestionar` ni `plataforma.cobros.aprobar`;
/// [`AppError::NoEncontrado`].
pub async fn ver_suscripcion<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: SuscripcionId,
) -> AppResult<DetalleSuscripcion>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    exigir_alguno(
        ctx,
        &[
            Permiso::PlataformaComercialGestionar,
            Permiso::PlataformaCobrosAprobar,
        ],
    )?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let s = tx
        .suscripcion(id)
        .await?
        .ok_or(AppError::NoEncontrado("suscripción"))?;
    detalle_en(&mut tx, reloj, s).await
}

pub(crate) async fn detalle_en<T: TxCobros>(
    tx: &mut T,
    reloj: &impl Reloj,
    s: Suscripcion,
) -> AppResult<DetalleSuscripcion> {
    let hoy = domain::proteccion_datos::fecha_en_colombia(reloj.ahora());
    let mut lineas = Vec::new();
    for l in tx.lineas(s.id).await? {
        let p = tx
            .producto(l.producto_id)
            .await?
            .ok_or(AppError::NoEncontrado("producto"))?;
        lineas.push((l, p));
    }
    let cubiertos: Vec<&str> = lineas
        .iter()
        .filter(|(l, _)| l.hasta.is_none_or(|h| h > hoy))
        .filter_map(|(_, p)| p.modulo.map(Modulo::as_str))
        .collect();
    let habilitados = tx
        .entidades(
            None,
            Some(s.tenant_id),
            crate::SolicitudPagina::new(Some(1), None)?,
            reloj.ahora(),
        )
        .await?
        .datos
        .into_iter()
        .next()
        .map(|e| e.modulos)
        .unwrap_or_default();
    let modulos_sin_cobro = habilitados
        .into_iter()
        .filter(|m| !cubiertos.contains(&m.as_str()))
        .collect();
    Ok(DetalleSuscripcion {
        suscripcion: s,
        lineas,
        modulos_sin_cobro,
    })
}

/// Suscripciones vigentes (no terminadas).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.comercial.gestionar` ni `plataforma.cobros.aprobar`.
pub async fn suscripciones<UP>(unidad: &UP, ctx: &ContextoPlataforma) -> AppResult<Vec<Suscripcion>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    exigir_alguno(
        ctx,
        &[
            Permiso::PlataformaComercialGestionar,
            Permiso::PlataformaCobrosAprobar,
        ],
    )?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .suscripciones_vigentes()
        .await
}

/// Agrega un producto a la suscripción desde una fecha (con precio pactado opcional y su motivo).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Validacion`] si el producto
/// está inactivo o no tiene tarifa, la suscripción está terminada, o el precio pactado no trae
/// motivo.
pub async fn agregar_linea<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    suscripcion: SuscripcionId,
    producto: ProductoPlataformaId,
    desde: NaiveDate,
    precio_pactado: Option<(Decimal, String)>,
) -> AppResult<(LineaSuscripcion, Producto)>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::PlataformaComercialGestionar)?;
    let (precio, motivo) = match precio_pactado {
        Some((p, m)) => {
            if p < Decimal::ZERO {
                return Err(validacion("el precio pactado no puede ser negativo"));
            }
            (Some(p), Some(texto_obligatorio("motivo_precio", &m, 300)?))
        }
        None => (None, None),
    };
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let s = tx
        .suscripcion(suscripcion)
        .await?
        .ok_or(AppError::NoEncontrado("suscripción"))?;
    if s.estado == EstadoSuscripcion::Terminada {
        return Err(validacion("la suscripción está terminada"));
    }
    let p = tx
        .producto(producto)
        .await?
        .ok_or(AppError::NoEncontrado("producto"))?;
    if !p.activo {
        return Err(validacion("el producto está inactivo"));
    }
    if tx.tarifas(producto).await?.is_empty() {
        return Err(validacion("fije primero la tarifa del producto"));
    }
    let l = LineaSuscripcion {
        id: LineaSuscripcionId::nuevo(),
        suscripcion_id: suscripcion,
        producto_id: producto,
        precio_pactado: precio,
        motivo_precio: motivo,
        desde,
        hasta: None,
        creada_en: reloj.ahora(),
    };
    tx.insertar_linea(&l).await?;
    let detalle = DetalleAuditoria::vacio().con("producto", p.codigo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_LINEA, "suscripcion", suscripcion.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok((l, p))
}

/// Termina una línea (deja de cobrarse desde `hasta`, excluido).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Validacion`] si `hasta` no
/// es posterior al inicio.
pub async fn terminar_linea<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    linea: LineaSuscripcionId,
    hasta: NaiveDate,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::PlataformaComercialGestionar)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let l = tx
        .linea(linea)
        .await?
        .ok_or(AppError::NoEncontrado("línea"))?;
    if hasta <= l.desde {
        return Err(validacion("la línea debe terminar después de empezar"));
    }
    tx.terminar_linea(linea, hasta).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_LINEA,
        "suscripcion",
        l.suscripcion_id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Cambios de una suscripción.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CambiosSuscripcion {
    pub correo_facturacion: Option<DireccionCorreo>,
    pub plazo_pago_dias: Option<u16>,
    /// Termina la suscripción en esa fecha (no genera más cortes).
    pub terminar_en: Option<NaiveDate>,
    pub motivo: Option<String>,
}

/// Edita el correo, el plazo de pago o termina la suscripción.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Validacion`].
pub async fn editar_suscripcion<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    id: SuscripcionId,
    c: CambiosSuscripcion,
) -> AppResult<Suscripcion>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::PlataformaComercialGestionar)?;
    let motivo = texto_opcional("motivo", c.motivo.as_deref(), 500)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut s = tx
        .suscripcion(id)
        .await?
        .ok_or(AppError::NoEncontrado("suscripción"))?;
    if s.estado == EstadoSuscripcion::Terminada {
        return Err(validacion("la suscripción está terminada"));
    }
    if let Some(correo) = c.correo_facturacion {
        s.correo_facturacion = correo;
    }
    if let Some(p) = c.plazo_pago_dias {
        if p > 120 {
            return Err(validacion("el plazo de pago es de 0 a 120 días"));
        }
        s.plazo_pago_dias = p;
    }
    if let Some(fin) = c.terminar_en {
        s.estado = EstadoSuscripcion::Terminada;
        s.terminada_en = Some(fin);
    }
    tx.actualizar_suscripcion(&s).await?;
    let mut detalle = DetalleAuditoria::vacio().con("estado", s.estado.as_str())?;
    if let Some(m) = &motivo {
        detalle = detalle.con("motivo", m.as_str())?;
    }
    tx.registrar(
        EventoAuditoria::ok(ACCION_SUSCRIPCION, "suscripcion", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(s)
}
