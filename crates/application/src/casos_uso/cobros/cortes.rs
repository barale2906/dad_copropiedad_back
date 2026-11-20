//! Cortes de cobro (plan 19): calcular, aprobar (o descartar), facturar y registrar el pago.
//!
//! - **Calcular** es idempotente por (suscripción, periodo): un borrador se recalcula; uno
//!   aprobado o facturado se devuelve tal cual. Las mediciones se guardan una vez por periodo, así
//!   el corte es reproducible (huella SHA-256).
//! - Un corte que cobra lo mismo que el anterior **se aprueba solo**; los demás los aprueba una
//!   persona (`plataforma.cobros.aprobar`).
//! - **Facturar** crea y emite una factura de venta en el tenant de la entidad de la plataforma
//!   (`entidades.es_plataforma`), una línea por línea del corte. Mientras la plataforma no factura
//!   en el sistema, el corte aprobado queda «por facturar» y se marca con el número de la factura
//!   emitida por fuera ([`marcar_facturado_externo`]).
//! - Pagar un corte de una entidad suspendida por mora la reactiva si no le queda otro vencido.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use domain::{
    Ambito, ConjuntoPermisos, CorteFacturacionId, Documento, EstadoCopropiedad, LiquidacionId,
    Permiso, SuscripcionId, TipoDocumento, TipoPersona,
    cobro_plataforma::{
        Ciclo, DIAS_PARA_SUSPENDER, EstadoCorte, EstadoSuscripcion, LineaACobrar, LineaCorte,
        Metrica, ModeloTarifa, Periodo, Tarifa, calcular_corte, igual_al_anterior, total,
    },
    contabilidad::Monto,
};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};

use super::{TxCobros, validacion};
use crate::puertos::{RegistroOutbox as _, RepositorioCobros as _, RepositorioPlataforma as _};
use crate::{
    AppError, AppResult, Contexto, ContextoPlataforma, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::facturacion::{
        TxVentas, abrir_ventas,
        ventas::{NuevaFacturaVenta, NuevaLineaVenta, crear_factura_en, emitir_factura_en},
    },
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioEntidad, Transaccion, UnidadDeTrabajo,
        UnidadDeTrabajoPlataforma,
        cobros::{ClienteFacturacion, Corte, FiltroCortes, OrigenCorte, Pagador, Suscripcion},
        contabilidad::Origen,
        copropiedades::Persona,
        correo::CorreoSaliente,
        tesoreria::FacturaVenta,
    },
};

pub(crate) const ACCION_CALCULAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.corte.calcular");
pub(crate) const ACCION_APROBAR: AccionAuditoria = AccionAuditoria::new("plataforma.corte.aprobar");
pub(crate) const ACCION_DESCARTAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.corte.descartar");
pub(crate) const ACCION_FACTURAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.corte.facturar");
pub(crate) const ACCION_PAGAR: AccionAuditoria = AccionAuditoria::new("plataforma.corte.pagar");
pub(crate) const ACCION_REACTIVAR: AccionAuditoria =
    AccionAuditoria::new("plataforma.entidad.reactivar_por_pago");

/// Tipo de origen de la factura de un corte (en el tenant de la plataforma).
pub const ORIGEN_CORTE: &str = "plataforma.corte";

pub(crate) fn hoy(reloj: &impl Reloj) -> NaiveDate {
    domain::proteccion_datos::fecha_en_colombia(reloj.ahora())
}

/// Calcula (o recalcula, si sigue en borrador) el corte del periodo que empieza en `desde`. En
/// un ciclo por evento se indica `hasta` (excluido).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.cobros.aprobar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si falta `hasta` en un ciclo por evento, la suscripción está
/// terminada o un producto no tiene tarifa vigente.
pub async fn calcular<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    suscripcion: SuscripcionId,
    desde: NaiveDate,
    hasta: Option<NaiveDate>,
) -> AppResult<Corte>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::PlataformaCobrosAprobar)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let s = tx
        .suscripcion(suscripcion)
        .await?
        .ok_or(AppError::NoEncontrado("suscripción"))?;
    let periodo = match (s.ciclo.periodo_desde(desde), hasta) {
        (Some(p), _) => p,
        (None, Some(h)) => Periodo::new(desde, h)?,
        (None, None) => {
            return Err(validacion(
                "una suscripción por evento necesita `hasta` (fin del periodo, excluido)",
            ));
        }
    };
    let corte = calcular_en(&mut tx, reloj, ctx, &s, periodo).await?;
    tx.confirmar().await?;
    Ok(corte)
}

/// Calcula el corte dentro de una transacción ya abierta (lo usa también el worker).
pub(crate) async fn calcular_en<T: TxCobros>(
    tx: &mut T,
    reloj: &impl Reloj,
    _ctx: &ContextoPlataforma,
    s: &Suscripcion,
    periodo: Periodo,
) -> AppResult<Corte> {
    if s.estado == EstadoSuscripcion::Terminada
        && s.terminada_en.is_some_and(|f| f <= periodo.desde)
    {
        return Err(validacion("la suscripción terminó antes de ese periodo"));
    }
    let origen = OrigenCorte::Suscripcion(s.id);
    let existente = tx.corte_vigente(origen, periodo.desde).await?;
    if let Some(c) = &existente
        && c.estado != EstadoCorte::Borrador
    {
        return Ok(c.clone());
    }
    let ultimo_dia = periodo.hasta.pred_opt().unwrap_or(periodo.hasta);
    let cobra_desde = s.cobra_desde();
    let fin = s.terminada_en;
    let mut lineas = Vec::new();
    for l in tx.lineas(s.id).await? {
        let p = tx
            .producto(l.producto_id)
            .await?
            .ok_or(AppError::NoEncontrado("producto"))?;
        let tarifa = tx
            .tarifas(l.producto_id)
            .await?
            .into_iter()
            .find(|t| t.vigente_desde <= ultimo_dia)
            .ok_or_else(|| {
                validacion(format!(
                    "el producto {} no tiene tarifa vigente al corte",
                    p.codigo
                ))
            })?;
        let hasta = match (l.hasta, fin) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        lineas.push(LineaACobrar {
            producto: p.codigo,
            nombre: p.nombre,
            plantilla: p.descripcion_factura,
            tarifa: tarifa.tarifa,
            precio_pactado: l.precio_pactado,
            desde: l.desde.max(cobra_desde),
            hasta,
        });
    }
    let ahora = reloj.ahora();
    let mut mediciones = BTreeMap::new();
    for m in lineas.iter().filter_map(|l| l.tarifa.metrica.clone()) {
        if mediciones.contains_key(&m) {
            continue;
        }
        let cantidad = match tx.medicion(s.tenant_id, &m, &periodo).await? {
            Some(c) => c,
            None => {
                let c = tx.medir(s.tenant_id, &m, &periodo).await?;
                tx.guardar_medicion(s.tenant_id, &m, &periodo, c, ahora)
                    .await?;
                c
            }
        };
        mediciones.insert(m, cantidad);
    }
    let calculadas = calcular_corte(&periodo, &lineas, &mediciones);
    let anterior = tx.corte_anterior_cobrado(s.id, periodo.desde).await?;
    let automatico = igual_al_anterior(&calculadas, anterior.as_ref().map(|c| c.lineas.as_slice()));
    let mut corte = existente.unwrap_or_else(|| Corte {
        id: CorteFacturacionId::nuevo(),
        origen,
        periodo,
        lineas: Vec::new(),
        total: Decimal::ZERO,
        huella: String::new(),
        estado: EstadoCorte::Borrador,
        motivo: None,
        aprobado_por: None,
        aprobado_en: None,
        factura_venta_id: None,
        factura_externa: None,
        facturado_en: None,
        vencimiento: None,
        pagado_en: None,
        ultimo_aviso_en: None,
        creado_en: ahora,
    });
    let nuevo = corte.huella.is_empty();
    corte.huella = huella(&mediciones, &calculadas)?;
    corte.total = total(&calculadas);
    corte.lineas = calculadas;
    if automatico {
        corte.estado = corte.estado.transicionar(EstadoCorte::Aprobado)?;
        corte.aprobado_en = Some(ahora);
    }
    if nuevo {
        tx.insertar_corte(&corte).await?;
    } else {
        tx.actualizar_corte(&corte).await?;
    }
    let detalle = DetalleAuditoria::vacio()
        .con("total", corte.total.to_string())?
        .con("aprobado_automatico", automatico)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CALCULAR, "corte_facturacion", corte.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    Ok(corte)
}

fn huella(mediciones: &BTreeMap<Metrica, u64>, lineas: &[LineaCorte]) -> AppResult<String> {
    let datos = serde_json::to_vec(&(mediciones, lineas)).map_err(AppError::infra)?;
    Ok(hex::encode(Sha256::digest(&datos)))
}

async fn corte_existente<T: TxCobros>(tx: &mut T, id: CorteFacturacionId) -> AppResult<Corte> {
    tx.corte(id, true)
        .await?
        .ok_or(AppError::NoEncontrado("corte"))
}

/// Aprueba un corte en borrador (queda «por facturar»).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.cobros.aprobar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si no está en borrador.
pub async fn aprobar<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: CorteFacturacionId,
) -> AppResult<Corte>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::PlataformaCobrosAprobar)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut c = corte_existente(&mut tx, id).await?;
    c.estado = c.estado.transicionar(EstadoCorte::Aprobado)?;
    c.aprobado_por = ctx.actor_id;
    c.aprobado_en = Some(reloj.ahora());
    tx.actualizar_corte(&c).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_APROBAR,
        "corte_facturacion",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Descarta un corte en borrador o aprobado (con motivo). El periodo se puede volver a calcular.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si ya se facturó;
/// [`AppError::Validacion`] sin motivo.
pub async fn descartar<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    id: CorteFacturacionId,
    motivo: &str,
) -> AppResult<Corte>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::PlataformaCobrosAprobar)?;
    let motivo = domain::copropiedad::texto_obligatorio("motivo", motivo, 500)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut c = corte_existente(&mut tx, id).await?;
    c.estado = c.estado.transicionar(EstadoCorte::Descartado)?;
    c.motivo = Some(motivo.clone());
    tx.actualizar_corte(&c).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_DESCARTAR, "corte_facturacion", id.as_uuid())
            .con_detalle(DetalleAuditoria::vacio().con("motivo", motivo.as_str())?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Cortes con filtro, los más recientes primero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.cobros.aprobar` ni `plataforma.cobros.facturar`.
pub async fn listar<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    filtro: &FiltroCortes,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Corte>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    exigir_cobros(ctx)?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .cortes(filtro, pagina)
        .await
}

fn exigir_cobros(ctx: &ContextoPlataforma) -> AppResult<()> {
    if ctx.permisos.contiene(Permiso::PlataformaCobrosFacturar) {
        Ok(())
    } else {
        ctx.exigir(Permiso::PlataformaCobrosAprobar)
    }
}

/// Un corte.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`].
pub async fn ver<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    id: CorteFacturacionId,
) -> AppResult<Corte>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    exigir_cobros(ctx)?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .corte(id, false)
        .await?
        .ok_or(AppError::NoEncontrado("corte"))
}

// ---------------------------------------------------------------------------------------------
// Facturación
// ---------------------------------------------------------------------------------------------

/// Quién recibe la factura del corte y a qué correo se envía.
async fn cliente_del_corte<T: TxCobros>(
    tx: &mut T,
    c: &Corte,
) -> AppResult<(ClienteFacturacion, Option<domain::DireccionCorreo>)> {
    match c.origen {
        OrigenCorte::Suscripcion(id) => {
            let s = tx
                .suscripcion(id)
                .await?
                .ok_or(AppError::NoEncontrado("suscripción"))?;
            let cliente = match s.pagador {
                Pagador::MismaEntidad => tx.cliente_entidad(s.tenant_id).await?,
                Pagador::Entidad(e) => tx.cliente_entidad(e).await?,
                Pagador::Organizacion(o) => tx.cliente_organizacion(o).await?,
            }
            .ok_or(AppError::NoEncontrado("pagador"))?;
            Ok((cliente, Some(s.correo_facturacion)))
        }
        OrigenCorte::Liquidacion(id) => {
            let l = tx
                .liquidacion_a_cobrar(id)
                .await?
                .ok_or(AppError::NoEncontrado("liquidación"))?;
            let cliente = tx
                .cliente_organizacion(l.organizacion_id)
                .await?
                .ok_or(AppError::NoEncontrado("anunciante"))?;
            let correo = cliente.correo.clone();
            Ok((cliente, correo))
        }
    }
}

/// Persona del cliente en el tenant de la plataforma (por NIT): la existente o una nueva.
async fn persona_cliente<T: TxVentas>(
    tx: &mut T,
    cliente: &ClienteFacturacion,
) -> AppResult<domain::PersonaId> {
    let nit = cliente.nit.as_deref().ok_or_else(|| {
        validacion("el pagador no tiene NIT registrado: complételo antes de facturar")
    })?;
    let documento = Documento::new(nit)?;
    if let Some(p) = tx.persona_por_documento(&documento).await? {
        return Ok(p.id);
    }
    let mut p = Persona::basica(
        domain::PersonaId::nuevo(),
        TipoPersona::Juridica,
        cliente.nombre.clone(),
        Some(documento),
    );
    p.tipo_documento = Some(TipoDocumento::Nit);
    p.correo = cliente.correo.clone();
    tx.insertar_persona(&p).await?;
    Ok(p.id)
}

/// La liquidación de publicidad sigue a su corte: facturada al facturarse y pagada al pagarse
/// (solo si está en el estado anterior, según
/// [`domain::publicidad::EstadoLiquidacion::transicionar`]).
async fn seguir_liquidacion<T: TxCobros>(
    tx: &mut T,
    reloj: &impl Reloj,
    c: &Corte,
    hacia: domain::publicidad::EstadoLiquidacion,
) -> AppResult<()> {
    let OrigenCorte::Liquidacion(id) = c.origen else {
        return Ok(());
    };
    if let Some(l) = tx.liquidacion_a_cobrar(id).await?
        && l.estado.transicionar(hacia).is_ok()
    {
        tx.avanzar_liquidacion(id, l.estado, hacia, reloj.ahora())
            .await?;
    }
    Ok(())
}

/// Factura un corte aprobado en el tenant de la entidad de la plataforma: crea y emite la factura
/// de venta (número interno; el documento electrónico se transmite desde la facturación de esa
/// entidad), fija el vencimiento con el plazo de pago y envía el aviso al correo de facturación.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.cobros.facturar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si no hay entidad de la plataforma, el pagador no tiene NIT o el
/// corte no tiene valor; [`AppError::Dominio`] si no está aprobado; los de la facturación (la
/// entidad de la plataforma necesita facturación o contabilidad).
pub async fn facturar<UP, U>(
    plataforma: &UP,
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: CorteFacturacionId,
) -> AppResult<Corte>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
    U: UnidadDeTrabajo,
    U::Transaccion: TxVentas,
{
    ctx.exigir(Permiso::PlataformaCobrosFacturar)?;
    let fecha = hoy(reloj);
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    let c = corte_existente(&mut tx, id).await?;
    c.estado.transicionar(EstadoCorte::Facturado)?;
    if c.total.is_zero() {
        return Err(validacion(
            "el corte no tiene valor que facturar: descártelo",
        ));
    }
    let gisla = tx
        .entidad_plataforma()
        .await?
        .ok_or_else(|| validacion("designe primero la entidad de la plataforma"))?;
    let (cliente, correo) = cliente_del_corte(&mut tx, &c).await?;
    let plazo = match c.origen {
        OrigenCorte::Suscripcion(s) => tx
            .suscripcion(s)
            .await?
            .map_or(domain::cobro_plataforma::PLAZO_PAGO_POR_DEFECTO, |s| {
                s.plazo_pago_dias
            }),
        OrigenCorte::Liquidacion(_) => domain::cobro_plataforma::PLAZO_PAGO_POR_DEFECTO,
    };
    let vencimiento = fecha + Duration::days(i64::from(plazo));
    let mut items = BTreeMap::new();
    for p in tx.productos().await? {
        items.insert(p.codigo.clone(), p.item_facturacion_id);
    }
    drop(tx);

    let ctx_gisla = Contexto {
        tenant_id: gisla,
        actor_id: ctx.actor_id,
        request_id: ctx.request_id.clone(),
        permisos: ConjuntoPermisos::todos(Ambito::Copropiedad),
    };
    let mut tg = abrir_ventas(
        unidad,
        reloj,
        &ctx_gisla,
        Permiso::FacturacionFacturasGestionar,
    )
    .await?;
    let persona = persona_cliente(&mut tg, &cliente).await?;
    let lineas = c
        .lineas
        .iter()
        .map(|l| {
            let codigo = l.producto.split('.').next().unwrap_or(&l.producto);
            Ok(NuevaLineaVenta {
                item: items.get(codigo).copied().flatten(),
                descripcion: Some(l.descripcion.chars().take(300).collect()),
                cuenta: None,
                centro_costo: None,
                cantidad: Decimal::ONE,
                valor_unitario: Some(Monto::new(l.valor)?),
                tarifa_iva: None,
                clasificacion: None,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    let factura = crear_factura_en(
        &mut tg,
        reloj,
        &ctx_gisla,
        NuevaFacturaVenta {
            persona,
            fecha,
            vencimiento,
            descripcion: format!("Servicios de la plataforma {}", c.periodo.texto()),
            origen: Some(Origen {
                tipo: ORIGEN_CORTE.into(),
                id: c.id.as_uuid(),
            }),
            lineas,
        },
        None,
    )
    .await?;
    let factura: FacturaVenta = emitir_factura_en(&mut tg, reloj, &ctx_gisla, factura).await?;
    tg.confirmar().await?;

    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    let mut c = corte_existente(&mut tx, id).await?;
    c.estado = c.estado.transicionar(EstadoCorte::Facturado)?;
    c.factura_venta_id = Some(factura.id);
    c.facturado_en = Some(fecha);
    c.vencimiento = Some(vencimiento);
    tx.actualizar_corte(&c).await?;
    seguir_liquidacion(
        &mut tx,
        reloj,
        &c,
        domain::publicidad::EstadoLiquidacion::Facturada,
    )
    .await?;
    if let Some(para) = correo {
        tx.encolar(aviso(
            &para,
            "Su factura de DAD Copropiedad",
            format!(
                "Hola, {}.\n\nEmitimos la factura de los servicios de la plataforma ({}) por \
                 ${} antes de impuestos. Vence el {vencimiento}.",
                cliente.nombre,
                c.periodo.texto(),
                c.total
            ),
        ))
        .await?;
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_FACTURAR,
        "corte_facturacion",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

pub(crate) fn aviso(para: &domain::DireccionCorreo, asunto: &str, texto: String) -> EventoSaliente {
    EventoSaliente {
        prioridad: PrioridadSalida::Recordatorio,
        mensaje: MensajeOutbox::Correo(CorreoSaliente {
            para: para.clone(),
            asunto: asunto.into(),
            texto,
            html: None,
            adjuntos: Vec::new(),
        }),
    }
}

/// Marca un corte aprobado como facturado por fuera (mientras la plataforma no factura en el
/// sistema): número de la factura y su fecha; el vencimiento sale del plazo de pago.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.cobros.facturar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si no está aprobado; [`AppError::Validacion`] sin número.
pub async fn marcar_facturado_externo<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: CorteFacturacionId,
    numero: &str,
    fecha: NaiveDate,
) -> AppResult<Corte>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::PlataformaCobrosFacturar)?;
    let numero = domain::copropiedad::texto_obligatorio("numero", numero, 60)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut c = corte_existente(&mut tx, id).await?;
    c.estado = c.estado.transicionar(EstadoCorte::Facturado)?;
    let plazo = match c.origen {
        OrigenCorte::Suscripcion(s) => tx
            .suscripcion(s)
            .await?
            .map_or(domain::cobro_plataforma::PLAZO_PAGO_POR_DEFECTO, |s| {
                s.plazo_pago_dias
            }),
        OrigenCorte::Liquidacion(_) => domain::cobro_plataforma::PLAZO_PAGO_POR_DEFECTO,
    };
    c.factura_externa = Some(numero.clone());
    c.facturado_en = Some(fecha);
    c.vencimiento = Some(fecha + Duration::days(i64::from(plazo)));
    tx.actualizar_corte(&c).await?;
    seguir_liquidacion(
        &mut tx,
        reloj,
        &c,
        domain::publicidad::EstadoLiquidacion::Facturada,
    )
    .await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_FACTURAR, "corte_facturacion", id.as_uuid())
            .con_detalle(DetalleAuditoria::vacio().con("factura_externa", numero.as_str())?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Registra el pago de un corte facturado. Si la entidad estaba suspendida por mora y no le queda
/// otro corte vencido hace más de [`DIAS_PARA_SUSPENDER`] días, se reactiva.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.cobros.facturar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si no está facturado.
pub async fn marcar_pagado<UP, U>(
    plataforma: &UP,
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: CorteFacturacionId,
    fecha: NaiveDate,
) -> AppResult<Corte>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioEntidad + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaCobrosFacturar)?;
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    let mut c = corte_existente(&mut tx, id).await?;
    c.estado = c.estado.transicionar(EstadoCorte::Pagado)?;
    c.pagado_en = Some(fecha);
    tx.actualizar_corte(&c).await?;
    seguir_liquidacion(
        &mut tx,
        reloj,
        &c,
        domain::publicidad::EstadoLiquidacion::Pagada,
    )
    .await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_PAGAR,
        "corte_facturacion",
        id.as_uuid(),
    ))
    .await?;
    let mut reactivar = None;
    if let OrigenCorte::Suscripcion(sid) = c.origen
        && let Some(mut s) = tx.suscripcion(sid).await?
        && s.suspendida_por_mora
    {
        let hoy = hoy(reloj);
        let sigue_en_mora = tx.cortes_por_cobrar().await?.iter().any(|x| {
            x.origen == c.origen
                && x.vencimiento
                    .is_some_and(|v| (hoy - v).num_days() >= DIAS_PARA_SUSPENDER)
        });
        if !sigue_en_mora {
            s.suspendida_por_mora = false;
            s.estado = EstadoSuscripcion::Activa;
            tx.actualizar_suscripcion(&s).await?;
            reactivar = Some(s.tenant_id);
        }
    }
    tx.confirmar().await?;
    if let Some(tenant) = reactivar {
        cambiar_estado_entidad(
            unidad,
            ctx,
            tenant,
            EstadoCopropiedad::Activa,
            ACCION_REACTIVAR,
        )
        .await?;
    }
    Ok(c)
}

/// Cambia el estado de la entidad por el cobro (suspender por mora o reactivar al pagar).
pub(crate) async fn cambiar_estado_entidad<U>(
    unidad: &U,
    ctx: &ContextoPlataforma,
    tenant: domain::TenantId,
    hacia: EstadoCopropiedad,
    accion: AccionAuditoria,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioEntidad + RegistroAuditoria,
{
    let ctx_e = Contexto {
        tenant_id: tenant,
        actor_id: ctx.actor_id,
        request_id: ctx.request_id.clone(),
        permisos: ConjuntoPermisos::todos(Ambito::Copropiedad),
    };
    let mut te = unidad.iniciar(&ctx_e).await?;
    let ficha = te
        .ficha_entidad()
        .await?
        .ok_or(AppError::NoEncontrado("entidad"))?;
    if ficha.estado == hacia {
        return Ok(());
    }
    let nuevo = ficha.estado.transicionar(hacia)?;
    te.guardar_estado_entidad(nuevo).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("desde", ficha.estado.as_str())?
        .con("hacia", nuevo.as_str())?;
    te.registrar(EventoAuditoria::ok(accion, "entidad", tenant.as_uuid()).con_detalle(detalle))
        .await?;
    te.confirmar().await
}

/// Crea el corte (en borrador) de una liquidación de publicidad de un anunciante: una línea con
/// el valor liquidado del mes.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.cobros.aprobar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si no tiene valor; [`AppError::Conflicto`] si ya tiene corte.
pub async fn cobrar_liquidacion<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    liquidacion: LiquidacionId,
) -> AppResult<Corte>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxCobros,
{
    ctx.exigir(Permiso::PlataformaCobrosAprobar)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let l = tx
        .liquidacion_a_cobrar(liquidacion)
        .await?
        .ok_or(AppError::NoEncontrado("liquidación"))?;
    let origen = OrigenCorte::Liquidacion(liquidacion);
    if let Some(c) = tx.corte_vigente(origen, l.periodo).await? {
        return Ok(c);
    }
    let valor = l.valor.round_dp(0);
    if valor <= Decimal::ZERO {
        return Err(validacion("la liquidación no tiene valor que cobrar"));
    }
    let periodo = Ciclo::Meses(1)
        .periodo_desde(l.periodo)
        .ok_or_else(|| validacion("periodo de la liquidación inválido"))?;
    let linea = LineaACobrar {
        producto: "publicidad".into(),
        nombre: format!("Publicidad de {}", l.nombre_comercial),
        plantilla: "{producto} — {periodo}".into(),
        tarifa: Tarifa {
            modelo: ModeloTarifa::Unica,
            valor,
            incluidas: None,
            valor_excedente: None,
            metrica: None,
        },
        precio_pactado: None,
        desde: periodo.desde,
        hasta: None,
    };
    let lineas = calcular_corte(&periodo, &[linea], &BTreeMap::new());
    let ahora: DateTime<Utc> = reloj.ahora();
    let c = Corte {
        id: CorteFacturacionId::nuevo(),
        origen,
        periodo,
        total: total(&lineas),
        huella: huella(&BTreeMap::new(), &lineas)?,
        lineas,
        estado: EstadoCorte::Borrador,
        motivo: None,
        aprobado_por: None,
        aprobado_en: None,
        factura_venta_id: None,
        factura_externa: None,
        facturado_en: None,
        vencimiento: None,
        pagado_en: None,
        ultimo_aviso_en: None,
        creado_en: ahora,
    };
    tx.insertar_corte(&c).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_CALCULAR,
        "corte_facturacion",
        c.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(c)
}
