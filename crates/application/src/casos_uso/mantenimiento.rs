//! Inventario y mantenimiento (plan 07c, módulo vendible `gestion_administrativa`): activos con
//! hoja de vida, movimientos, planes preventivos, órdenes de trabajo y obligaciones periódicas.
//!
//! Invariantes:
//! 1. Las órdenes cambian de estado solo por [`EstadoOrden::transicionar`].
//! 2. La generación de órdenes preventivas es idempotente por (plan, fecha programada); la BD
//!    lo repite con un índice único.
//! 3. Un activo dado de baja no vuelve a operar ni recibe órdenes; la baja exige soporte.
//! 4. Verificar una orden correctiva puede resolver el ticket que la originó (y avisa al
//!    solicitante).

use chrono::{DateTime, Duration, NaiveDate, Utc};
use domain::{
    ActivoId, AgrupacionId, ArchivoId, ClaseActivo, CodigoActivo, EstadoActivo, EstadoOrden,
    EstadoTicket, Modulo, MomentoEvidencia, MovimientoInventarioId, NumeroOrden, ObligacionId,
    OrdenTrabajoId, OrganizacionId, OrigenOrden, Permiso, PlanMantenimientoId, PuntoChecklist,
    Recurrencia, RecursoId, ReservaId, TicketId, TipoMovimiento, TipoOrden,
    copropiedad::{texto_obligatorio, texto_opcional},
    mantenimiento::{OBLIGACIONES_SUGERIDAS, ObligacionSugerida, proxima_obligacion},
};
use rust_decimal::Decimal;

use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{
        avisos::avisar_persona,
        modulos::exigir_modulo_en,
        reservas::zona,
        tickets::{TxTickets, aviso_al_solicitante, registrar_transicion},
    },
    puertos::{
        RegistroAuditoria, Reloj, RepositorioMantenimiento, RepositorioTickets, Transaccion,
        UnidadDeTrabajo,
        mantenimiento::{
            Activo, ArchivoActivo, Cumplimiento, EvidenciaOrden, FiltroActivos, FiltroOrdenes,
            Movimiento, Obligacion, OrdenTrabajo, PlanMantenimiento, TableroMantenimiento,
        },
        tickets::{MensajeTicket, Responsable, SerieConsecutivo},
    },
};

pub const ACCION_REGISTRAR_ACTIVO: AccionAuditoria =
    AccionAuditoria::new("inventario.activo.registrar");
pub const ACCION_EDITAR_ACTIVO: AccionAuditoria = AccionAuditoria::new("inventario.activo.editar");
pub const ACCION_MOVIMIENTO: AccionAuditoria =
    AccionAuditoria::new("inventario.movimiento.registrar");
pub const ACCION_PLAN: AccionAuditoria = AccionAuditoria::new("mantenimiento.plan.editar");
pub const ACCION_CREAR_ORDEN: AccionAuditoria = AccionAuditoria::new("mantenimiento.orden.crear");
pub const ACCION_ESTADO_ORDEN: AccionAuditoria =
    AccionAuditoria::new("mantenimiento.orden.transicionar");
pub const ACCION_OBLIGACION: AccionAuditoria =
    AccionAuditoria::new("mantenimiento.obligacion.editar");
pub const ACCION_CUMPLIMIENTO: AccionAuditoria =
    AccionAuditoria::new("mantenimiento.obligacion.cumplir");

/// Puertos que usan los casos de uso de mantenimiento (los de tickets más el de mantenimiento).
pub trait TxMantenimiento: TxTickets + RepositorioMantenimiento {}

impl<T> TxMantenimiento for T where T: TxTickets + RepositorioMantenimiento {}

async fn abrir<U>(unidad: &U, reloj: &impl Reloj, ctx: &Contexto) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_modulo_en(&mut tx, reloj, Modulo::GestionAdministrativa).await?;
    Ok(tx)
}

fn tiene(ctx: &Contexto, p: Permiso) -> bool {
    ctx.exigir(p).is_ok()
}

fn exigir_alguno(ctx: &Contexto, permisos: &[Permiso]) -> AppResult<()> {
    if permisos.iter().any(|p| tiene(ctx, *p)) {
        Ok(())
    } else {
        Err(AppError::SinPermiso(permisos[0]))
    }
}

async fn hoy<T: TxMantenimiento>(tx: &mut T, ahora: DateTime<Utc>) -> AppResult<NaiveDate> {
    Ok(ahora.with_timezone(&zona(tx).await?).date_naive())
}

async fn validar_responsable<T: TxMantenimiento>(
    tx: &mut T,
    r: Option<Responsable>,
    ahora: DateTime<Utc>,
) -> AppResult<()> {
    let valido = match r {
        Some(Responsable::Cuenta(c)) => tx
            .permisos_de_cuenta(c, ahora)
            .await?
            .contiene(Permiso::MantenimientoEjecutar),
        Some(Responsable::Organizacion(o)) => tx.organizacion_con_contrato(o, ahora).await?,
        None => true,
    };
    if !valido {
        return Err(AppError::Validacion(
            "el responsable debe ser una cuenta con `mantenimiento.ejecutar` o una organización \
             con contrato de servicio vigente"
                .into(),
        ));
    }
    Ok(())
}

fn validar_valor(campo: &str, v: Option<Decimal>) -> AppResult<()> {
    if v.is_some_and(|x| x.is_sign_negative()) {
        return Err(AppError::Validacion(format!(
            "`{campo}` no puede ser negativo"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Inventario
// ---------------------------------------------------------------------------------------------

/// Datos de un activo nuevo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoActivo {
    pub codigo: CodigoActivo,
    pub nombre: String,
    pub clase: ClaseActivo,
    pub marca: Option<String>,
    pub modelo: Option<String>,
    pub serie: Option<String>,
    pub ubicacion: Option<String>,
    pub agrupacion_id: Option<AgrupacionId>,
    pub recurso_id: Option<RecursoId>,
    pub fecha_compra: Option<NaiveDate>,
    pub valor: Option<Decimal>,
    pub garantia_vence: Option<NaiveDate>,
    pub proveedor_organizacion_id: Option<OrganizacionId>,
}

fn validar_textos_activo(a: &mut Activo) -> AppResult<()> {
    a.nombre = texto_obligatorio("nombre", &a.nombre, 120)?;
    a.marca = texto_opcional("marca", a.marca.as_deref(), 60)?;
    a.modelo = texto_opcional("modelo", a.modelo.as_deref(), 60)?;
    a.serie = texto_opcional("serie", a.serie.as_deref(), 60)?;
    a.ubicacion = texto_opcional("ubicacion", a.ubicacion.as_deref(), 200)?;
    validar_valor("valor", a.valor)
}

/// Registra un activo (queda `operativo`) con un movimiento de entrada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `inventario.editar`; [`AppError::ModuloNoHabilitado`];
/// [`AppError::Conflicto`] si el código ya existe; errores de validación.
pub async fn registrar_activo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: NuevoActivo,
) -> AppResult<Activo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    ctx.exigir(Permiso::InventarioEditar)?;
    let ahora = reloj.ahora();
    let mut activo = Activo {
        id: ActivoId::nuevo(),
        codigo: n.codigo,
        nombre: n.nombre,
        clase: n.clase,
        marca: n.marca,
        modelo: n.modelo,
        serie: n.serie,
        ubicacion: n.ubicacion,
        agrupacion_id: n.agrupacion_id,
        recurso_id: n.recurso_id,
        estado: EstadoActivo::Operativo,
        fecha_compra: n.fecha_compra,
        valor: n.valor,
        garantia_vence: n.garantia_vence,
        proveedor_organizacion_id: n.proveedor_organizacion_id,
        creado_en: ahora,
    };
    validar_textos_activo(&mut activo)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.insertar_activo(&activo).await?;
    tx.insertar_movimiento(&Movimiento {
        id: MovimientoInventarioId::nuevo(),
        activo_id: activo.id,
        tipo: TipoMovimiento::Entrada,
        ubicacion_destino: activo.ubicacion.clone(),
        reserva_id: None,
        soporte_archivo_id: None,
        observacion: None,
        registrado_por: ctx.actor_id,
        ocurrido_en: ahora,
    })
    .await?;
    let detalle = DetalleAuditoria::vacio().con("clase", activo.clase.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_REGISTRAR_ACTIVO, "activo", activo.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(activo)
}

/// Cambios a un activo (`None` = sin cambio). La ubicación cambia con un traslado.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CambiosActivo {
    pub nombre: Option<String>,
    pub marca: Option<String>,
    pub modelo: Option<String>,
    pub serie: Option<String>,
    pub recurso_id: Option<RecursoId>,
    pub valor: Option<Decimal>,
    pub garantia_vence: Option<NaiveDate>,
    pub proveedor_organizacion_id: Option<OrganizacionId>,
    /// Cambio manual de estado (la baja va por movimiento).
    pub estado: Option<EstadoActivo>,
}

/// Edita los datos o el estado de un activo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `inventario.editar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si el activo está dado de baja o se intenta darlo de baja sin
/// movimiento.
pub async fn editar_activo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ActivoId,
    c: CambiosActivo,
) -> AppResult<Activo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    ctx.exigir(Permiso::InventarioEditar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut a = tx
        .activo(id)
        .await?
        .ok_or(AppError::NoEncontrado("activo"))?;
    if a.estado == EstadoActivo::DadoDeBaja {
        return Err(AppError::Conflicto("el activo está dado de baja".into()));
    }
    if let Some(e) = c.estado {
        a.estado = a.estado.cambiar(e)?;
    }
    if let Some(v) = c.nombre {
        a.nombre = v;
    }
    if c.marca.is_some() {
        a.marca = c.marca;
    }
    if c.modelo.is_some() {
        a.modelo = c.modelo;
    }
    if c.serie.is_some() {
        a.serie = c.serie;
    }
    if c.recurso_id.is_some() {
        a.recurso_id = c.recurso_id;
    }
    if c.valor.is_some() {
        a.valor = c.valor;
    }
    if c.garantia_vence.is_some() {
        a.garantia_vence = c.garantia_vence;
    }
    if c.proveedor_organizacion_id.is_some() {
        a.proveedor_organizacion_id = c.proveedor_organizacion_id;
    }
    validar_textos_activo(&mut a)?;
    tx.actualizar_activo(&a).await?;
    let detalle = DetalleAuditoria::vacio().con("estado", a.estado.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EDITAR_ACTIVO, "activo", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(a)
}

/// Agrega un archivo (manual, foto, factura, garantía) a la hoja de vida.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `inventario.editar`; [`AppError::NoEncontrado`].
pub async fn adjuntar_archivo_activo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ActivoId,
    archivo: ArchivoActivo,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    ctx.exigir(Permiso::InventarioEditar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.activo(id)
        .await?
        .ok_or(AppError::NoEncontrado("activo"))?;
    tx.insertar_archivo_activo(id, &archivo).await?;
    let detalle = DetalleAuditoria::vacio().con("tipo", archivo.tipo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EDITAR_ACTIVO, "activo", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await
}

/// Datos de un movimiento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoMovimiento {
    pub tipo: TipoMovimiento,
    /// Obligatoria en los traslados.
    pub ubicacion_destino: Option<String>,
    /// Reserva de la herramienta prestada (motor de reservas).
    pub reserva_id: Option<ReservaId>,
    /// Obligatorio en las bajas (acta o documento).
    pub soporte_archivo_id: Option<ArchivoId>,
    pub observacion: Option<String>,
}

/// Registra un movimiento. El traslado cambia la ubicación y la baja deja el activo
/// `dado_de_baja` (definitivo).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `inventario.editar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si falta el destino o el soporte; [`AppError::Conflicto`] si el
/// activo ya está dado de baja.
pub async fn registrar_movimiento<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ActivoId,
    n: NuevoMovimiento,
) -> AppResult<Movimiento>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    ctx.exigir(Permiso::InventarioEditar)?;
    let destino = texto_opcional("ubicacion_destino", n.ubicacion_destino.as_deref(), 200)?;
    let observacion = texto_opcional("observacion", n.observacion.as_deref(), 500)?;
    if n.tipo == TipoMovimiento::Traslado && destino.is_none() {
        return Err(AppError::Validacion(
            "un traslado indica `ubicacion_destino`".into(),
        ));
    }
    if n.tipo == TipoMovimiento::Baja && n.soporte_archivo_id.is_none() {
        return Err(AppError::Validacion(
            "la baja exige el soporte (`soporte_archivo_id`)".into(),
        ));
    }
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let mut a = tx
        .activo(id)
        .await?
        .ok_or(AppError::NoEncontrado("activo"))?;
    if a.estado == EstadoActivo::DadoDeBaja {
        return Err(AppError::Conflicto("el activo está dado de baja".into()));
    }
    let m = Movimiento {
        id: MovimientoInventarioId::nuevo(),
        activo_id: id,
        tipo: n.tipo,
        ubicacion_destino: destino.clone(),
        reserva_id: n.reserva_id,
        soporte_archivo_id: n.soporte_archivo_id,
        observacion,
        registrado_por: ctx.actor_id,
        ocurrido_en: ahora,
    };
    tx.insertar_movimiento(&m).await?;
    match n.tipo {
        TipoMovimiento::Traslado => {
            a.ubicacion = destino;
            tx.actualizar_activo(&a).await?;
        }
        TipoMovimiento::Baja => {
            a.estado = EstadoActivo::DadoDeBaja;
            tx.actualizar_activo(&a).await?;
        }
        _ => {}
    }
    let detalle = DetalleAuditoria::vacio().con("tipo", n.tipo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_MOVIMIENTO, "activo", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(m)
}

/// Activos (paginado).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `inventario.ver`; [`AppError::ModuloNoHabilitado`].
pub async fn listar_activos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroActivos,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Activo>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    ctx.exigir(Permiso::InventarioVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.activos(&filtro, pagina).await
}

/// Hoja de vida de un activo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HojaDeVida {
    pub activo: Activo,
    pub archivos: Vec<ArchivoActivo>,
    pub movimientos: Vec<Movimiento>,
    pub ordenes: Vec<OrdenTrabajo>,
    /// Costo acumulado de las órdenes (mano de obra + repuestos).
    pub costo_total: Decimal,
}

/// Máximo de órdenes que trae la hoja de vida.
pub const MAX_ORDENES_HOJA_DE_VIDA: u32 = 200;

/// Un activo con sus archivos, movimientos y órdenes de trabajo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `inventario.ver`; [`AppError::NoEncontrado`].
pub async fn hoja_de_vida<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ActivoId,
) -> AppResult<HojaDeVida>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    ctx.exigir(Permiso::InventarioVer)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let activo = tx
        .activo(id)
        .await?
        .ok_or(AppError::NoEncontrado("activo"))?;
    let ordenes = tx
        .ordenes(
            &FiltroOrdenes {
                activo_id: Some(id),
                ..FiltroOrdenes::default()
            },
            SolicitudPagina::new(Some(MAX_ORDENES_HOJA_DE_VIDA), None)?,
        )
        .await?
        .datos;
    let costo_total = ordenes
        .iter()
        .map(|o| o.costo_mano_obra + o.costo_repuestos)
        .sum();
    Ok(HojaDeVida {
        archivos: tx.archivos_activo(id).await?,
        movimientos: tx.movimientos(id).await?,
        ordenes,
        costo_total,
        activo,
    })
}

// ---------------------------------------------------------------------------------------------
// Planes preventivos
// ---------------------------------------------------------------------------------------------

/// Datos de un plan preventivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosPlan {
    pub activo_id: Option<ActivoId>,
    pub ubicacion: Option<String>,
    pub actividad: String,
    pub recurrencia: Recurrencia,
    pub anticipacion_dias: u32,
    pub responsable: Option<Responsable>,
    pub duracion_estimada_min: Option<u32>,
    pub checklist: Vec<String>,
    pub activo: bool,
}

async fn armar_plan<T: TxMantenimiento>(
    tx: &mut T,
    id: PlanMantenimientoId,
    d: DatosPlan,
    ahora: DateTime<Utc>,
) -> AppResult<PlanMantenimiento> {
    let ubicacion = texto_opcional("ubicacion", d.ubicacion.as_deref(), 200)?;
    if d.activo_id.is_none() && ubicacion.is_none() {
        return Err(AppError::Validacion(
            "indique el activo o la ubicación del plan".into(),
        ));
    }
    if d.anticipacion_dias > 90 {
        return Err(AppError::Validacion(
            "`anticipacion_dias` debe estar entre 0 y 90".into(),
        ));
    }
    if d.checklist.len() > 50 {
        return Err(AppError::Validacion(
            "a lo sumo 50 puntos de checklist".into(),
        ));
    }
    let checklist = d
        .checklist
        .iter()
        .map(|p| texto_obligatorio("checklist", p, 200))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(a) = d.activo_id {
        let activo = tx
            .activo(a)
            .await?
            .ok_or(AppError::NoEncontrado("activo"))?;
        if activo.estado == EstadoActivo::DadoDeBaja {
            return Err(AppError::Conflicto("el activo está dado de baja".into()));
        }
    }
    validar_responsable(tx, d.responsable, ahora).await?;
    Ok(PlanMantenimiento {
        id,
        activo_id: d.activo_id,
        ubicacion,
        actividad: texto_obligatorio("actividad", &d.actividad, 200)?,
        recurrencia: Recurrencia::new(
            d.recurrencia.frecuencia,
            d.recurrencia.intervalo,
            d.recurrencia.inicio,
        )?,
        anticipacion_dias: d.anticipacion_dias,
        responsable: d.responsable,
        duracion_estimada_min: d.duracion_estimada_min.filter(|m| *m > 0),
        checklist,
        activo: d.activo,
    })
}

/// Crea (`id = None`) o reemplaza un plan preventivo. Las órdenes ya generadas no cambian.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `mantenimiento.planificar`; [`AppError::NoEncontrado`];
/// errores de validación.
pub async fn guardar_plan<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: Option<PlanMantenimientoId>,
    d: DatosPlan,
) -> AppResult<PlanMantenimiento>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    ctx.exigir(Permiso::MantenimientoPlanificar)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let plan = match id {
        Some(id) => {
            tx.plan(id)
                .await?
                .ok_or(AppError::NoEncontrado("plan de mantenimiento"))?;
            let plan = armar_plan(&mut tx, id, d, ahora).await?;
            tx.actualizar_plan(&plan).await?;
            plan
        }
        None => {
            let plan = armar_plan(&mut tx, PlanMantenimientoId::nuevo(), d, ahora).await?;
            tx.insertar_plan(&plan).await?;
            plan
        }
    };
    let detalle = DetalleAuditoria::vacio().con("activo", plan.activo)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PLAN, "plan_mantenimiento", plan.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(plan)
}

/// Planes preventivos.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `mantenimiento.planificar` ni `inventario.ver`.
pub async fn listar_planes<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    solo_activos: bool,
) -> AppResult<Vec<PlanMantenimiento>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    exigir_alguno(
        ctx,
        &[Permiso::MantenimientoPlanificar, Permiso::InventarioVer],
    )?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.planes(solo_activos).await
}

/// Días hacia atrás que revisa la generación (recupera fechas si el worker estuvo detenido).
pub const DIAS_RECUPERACION: i64 = 7;

/// Genera las órdenes preventivas de los planes activos cuya fecha cae entre hoy (menos
/// [`DIAS_RECUPERACION`]) y hoy más la anticipación del plan. Idempotente: no repite una orden
/// por plan y fecha. Devuelve cuántas creó.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `mantenimiento.planificar` (el worker usa el contexto del
/// sistema); errores de persistencia.
pub async fn generar_ordenes_preventivas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<u32>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    ctx.exigir(Permiso::MantenimientoPlanificar)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let hoy = hoy(&mut tx, ahora).await?;
    let mut creadas = 0;
    for plan in tx.planes(true).await? {
        if let Some(a) = plan.activo_id
            && tx
                .activo(a)
                .await?
                .is_none_or(|x| x.estado == EstadoActivo::DadoDeBaja)
        {
            continue;
        }
        let desde = (hoy - Duration::days(DIAS_RECUPERACION)).max(plan.recurrencia.inicio);
        let hasta = hoy + Duration::days(i64::from(plan.anticipacion_dias));
        for fecha in plan.recurrencia.ocurrencias_entre(desde, hasta, 31) {
            if tx.existe_orden_de_plan(plan.id, fecha).await? {
                continue;
            }
            let orden = nueva_orden(
                &mut tx,
                OrigenOrden::PlanPreventivo,
                TipoOrden::Preventivo,
                ahora,
                OrdenBase {
                    plan_id: Some(plan.id),
                    ticket_id: None,
                    activo_id: plan.activo_id,
                    ubicacion: plan.ubicacion.clone(),
                    descripcion: plan.actividad.clone(),
                    asignado: plan.responsable,
                    programada_para: fecha,
                    checklist: plan.checklist.clone(),
                },
            )
            .await?;
            tx.insertar_orden(&orden).await?;
            creadas += 1;
        }
    }
    if creadas > 0 {
        let detalle = DetalleAuditoria::vacio().con("ordenes", creadas)?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_CREAR_ORDEN, "copropiedad", ctx.tenant_id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
        tx.confirmar().await?;
    }
    Ok(creadas)
}

// ---------------------------------------------------------------------------------------------
// Órdenes de trabajo
// ---------------------------------------------------------------------------------------------

struct OrdenBase {
    plan_id: Option<PlanMantenimientoId>,
    ticket_id: Option<TicketId>,
    activo_id: Option<ActivoId>,
    ubicacion: Option<String>,
    descripcion: String,
    asignado: Option<Responsable>,
    programada_para: NaiveDate,
    checklist: Vec<String>,
}

async fn nueva_orden<T: TxMantenimiento>(
    tx: &mut T,
    origen: OrigenOrden,
    tipo: TipoOrden,
    ahora: DateTime<Utc>,
    b: OrdenBase,
) -> AppResult<OrdenTrabajo> {
    let anio = chrono::Datelike::year(&ahora.with_timezone(&zona(tx).await?));
    let consecutivo = tx
        .siguiente_consecutivo(SerieConsecutivo::OrdenTrabajo, anio)
        .await?;
    Ok(OrdenTrabajo {
        id: OrdenTrabajoId::nuevo(),
        numero: NumeroOrden { anio, consecutivo },
        origen,
        tipo,
        plan_id: b.plan_id,
        ticket_id: b.ticket_id,
        activo_id: b.activo_id,
        ubicacion: b.ubicacion,
        descripcion: b.descripcion,
        asignado: b.asignado,
        programada_para: b.programada_para,
        estado: EstadoOrden::Programada,
        checklist: b
            .checklist
            .into_iter()
            .map(|descripcion| PuntoChecklist {
                descripcion,
                cumplido: false,
                observacion: None,
            })
            .collect(),
        costo_mano_obra: Decimal::ZERO,
        costo_repuestos: Decimal::ZERO,
        observaciones: None,
        recibido_por: None,
        firma_archivo_id: None,
        iniciada_en: None,
        ejecutada_en: None,
        verificada_en: None,
        verificada_por: None,
        creado_en: ahora,
        actualizado_en: ahora,
    })
}

/// Orden de trabajo correctiva, urgente o de inspección.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevaOrden {
    pub tipo: TipoOrden,
    /// Ticket que la origina (origen `ticket`); `None` = inspección.
    pub ticket_id: Option<TicketId>,
    pub activo_id: Option<ActivoId>,
    pub ubicacion: Option<String>,
    pub descripcion: String,
    pub asignado: Option<Responsable>,
    pub programada_para: NaiveDate,
    pub checklist: Vec<String>,
}

/// Crea una orden de trabajo. Si nace de un ticket, el ticket pasa a `en_progreso` (si estaba
/// asignado o reabierto) y el solicitante recibe un mensaje público.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `mantenimiento.planificar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si es preventiva (esas las generan los planes) o el responsable no
/// puede ejecutar; [`AppError::Conflicto`] si el activo está dado de baja.
pub async fn crear_orden<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    n: NuevaOrden,
) -> AppResult<OrdenTrabajo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    ctx.exigir(Permiso::MantenimientoPlanificar)?;
    if n.tipo == TipoOrden::Preventivo && n.ticket_id.is_some() {
        return Err(AppError::Validacion(
            "una orden que nace de un ticket es correctiva o urgente".into(),
        ));
    }
    let descripcion = texto_obligatorio("descripcion", &n.descripcion, 2000)?;
    let ubicacion = texto_opcional("ubicacion", n.ubicacion.as_deref(), 200)?;
    let checklist = n
        .checklist
        .iter()
        .map(|p| texto_obligatorio("checklist", p, 200))
        .collect::<Result<Vec<_>, _>>()?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    if let Some(a) = n.activo_id {
        let activo = tx
            .activo(a)
            .await?
            .ok_or(AppError::NoEncontrado("activo"))?;
        if activo.estado == EstadoActivo::DadoDeBaja {
            return Err(AppError::Conflicto("el activo está dado de baja".into()));
        }
    }
    validar_responsable(&mut tx, n.asignado, ahora).await?;
    let ticket = match n.ticket_id {
        Some(t) => Some(
            tx.ticket(t)
                .await?
                .ok_or(AppError::NoEncontrado("ticket"))?,
        ),
        None => None,
    };
    if ticket.as_ref().is_some_and(|t| !t.estado.pendiente()) {
        return Err(AppError::Conflicto("el ticket ya no está pendiente".into()));
    }
    let origen = if ticket.is_some() {
        OrigenOrden::Ticket
    } else {
        OrigenOrden::Inspeccion
    };
    let orden = nueva_orden(
        &mut tx,
        origen,
        n.tipo,
        ahora,
        OrdenBase {
            plan_id: None,
            ticket_id: n.ticket_id,
            activo_id: n.activo_id,
            ubicacion,
            descripcion,
            asignado: n.asignado,
            programada_para: n.programada_para,
            checklist,
        },
    )
    .await?;
    tx.insertar_orden(&orden).await?;
    if let Some(mut t) = ticket {
        let texto = format!(
            "Programamos la orden de trabajo {} para el {}.",
            orden.numero, orden.programada_para
        );
        tx.insertar_mensaje_ticket(&MensajeTicket {
            id: domain::MensajeTicketId::nuevo(),
            ticket_id: t.id,
            autor_cuenta_id: ctx.actor_id,
            autor_persona_id: None,
            canal: domain::CanalTicket::Web,
            interno: false,
            texto: texto.clone(),
            adjuntos: Vec::new(),
            creado_en: ahora,
        })
        .await?;
        if matches!(t.estado, EstadoTicket::Asignado | EstadoTicket::Reabierto) {
            registrar_transicion(&mut tx, ctx, &mut t, EstadoTicket::EnProgreso, None, ahora)
                .await?;
        }
        avisar_persona(
            &mut tx,
            t.solicitante_persona_id,
            &aviso_al_solicitante(&t, &texto),
        )
        .await?;
    }
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", orden.tipo.as_str())?
        .con("origen", orden.origen.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR_ORDEN, "orden_trabajo", orden.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(orden)
}

async fn organizaciones_actor<T: TxMantenimiento>(
    tx: &mut T,
    ctx: &Contexto,
    ahora: DateTime<Utc>,
) -> AppResult<Vec<OrganizacionId>> {
    match ctx.actor_id {
        Some(c) => tx.organizaciones_de_cuenta(c, ahora).await,
        None => Ok(Vec::new()),
    }
}

fn asignada_a(o: &OrdenTrabajo, ctx: &Contexto, orgs: &[OrganizacionId]) -> bool {
    match o.asignado {
        Some(Responsable::Cuenta(c)) => ctx.actor_id == Some(c),
        Some(Responsable::Organizacion(org)) => orgs.contains(&org),
        None => false,
    }
}

/// ¿Ve todas las órdenes? Si no, solo las que tiene asignadas.
fn ve_todas_las_ordenes(ctx: &Contexto) -> bool {
    tiene(ctx, Permiso::MantenimientoPlanificar)
        || tiene(ctx, Permiso::MantenimientoVerificar)
        || tiene(ctx, Permiso::InventarioVer)
}

async fn orden_visible<T: TxMantenimiento>(
    tx: &mut T,
    ctx: &Contexto,
    id: OrdenTrabajoId,
    ahora: DateTime<Utc>,
) -> AppResult<(OrdenTrabajo, bool)> {
    let o = tx
        .orden(id)
        .await?
        .ok_or(AppError::NoEncontrado("orden de trabajo"))?;
    let orgs = organizaciones_actor(tx, ctx, ahora).await?;
    let propia = asignada_a(&o, ctx, &orgs);
    if !(ve_todas_las_ordenes(ctx) || (propia && tiene(ctx, Permiso::MantenimientoEjecutar))) {
        return Err(AppError::NoEncontrado("orden de trabajo"));
    }
    Ok((o, propia))
}

/// Filtros del listado de órdenes que elige quien consulta.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConsultaOrdenes {
    pub estado: Option<EstadoOrden>,
    pub activo_id: Option<ActivoId>,
    pub ticket_id: Option<TicketId>,
    pub solo_vencidas: bool,
}

/// Órdenes visibles: todas para quien planifica, verifica o ve el inventario; las asignadas
/// para quien solo ejecuta.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin ninguno de esos permisos.
pub async fn listar_ordenes<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    c: ConsultaOrdenes,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<OrdenTrabajo>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    exigir_alguno(
        ctx,
        &[
            Permiso::MantenimientoPlanificar,
            Permiso::MantenimientoEjecutar,
            Permiso::MantenimientoVerificar,
            Permiso::InventarioVer,
        ],
    )?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let asignadas_a = if ve_todas_las_ordenes(ctx) {
        None
    } else {
        let orgs = organizaciones_actor(&mut tx, ctx, ahora).await?;
        Some((ctx.actor_id.ok_or(AppError::NoAutenticado)?, orgs))
    };
    let vencidas_a = if c.solo_vencidas {
        Some(hoy(&mut tx, ahora).await?)
    } else {
        None
    };
    tx.ordenes(
        &FiltroOrdenes {
            estado: c.estado,
            activo_id: c.activo_id,
            ticket_id: c.ticket_id,
            asignadas_a,
            vencidas_a,
        },
        pagina,
    )
    .await
}

/// Orden con sus evidencias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetalleOrden {
    pub orden: OrdenTrabajo,
    pub evidencias: Vec<EvidenciaOrden>,
}

/// Una orden de trabajo.
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si no existe o no es visible para la cuenta.
pub async fn ver_orden<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: OrdenTrabajoId,
) -> AppResult<DetalleOrden>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let (orden, _) = orden_visible(&mut tx, ctx, id, reloj.ahora()).await?;
    Ok(DetalleOrden {
        evidencias: tx.evidencias(id).await?,
        orden,
    })
}

/// Avance de una orden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvanceOrden {
    /// `programada → en_ejecucion`.
    Iniciar,
    /// `en_ejecucion → ejecutada`, con el checklist diligenciado, costos y recibido.
    Ejecutar(Ejecucion),
    /// `ejecutada → verificada` (o devuelta a `en_ejecucion` si `aprobada` es falso).
    Verificar {
        aprobada: bool,
        observacion: Option<String>,
        /// Resolver el ticket que originó la orden (con la observación como respuesta).
        resolver_ticket: bool,
    },
    /// `programada | en_ejecucion → cancelada`.
    Cancelar { motivo: String },
}

/// Datos de la ejecución.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ejecucion {
    pub checklist: Vec<PuntoChecklist>,
    pub costo_mano_obra: Decimal,
    pub costo_repuestos: Decimal,
    pub observaciones: Option<String>,
    pub recibido_por: Option<String>,
    pub firma_archivo_id: Option<ArchivoId>,
}

/// Avanza una orden por su diagrama:
///
/// - iniciar y ejecutar: quien la tiene asignada (`mantenimiento.ejecutar`) o quien planifica;
/// - verificar: `mantenimiento.verificar`;
/// - cancelar: `mantenimiento.planificar`.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si la
/// transición no está en el diagrama; errores de validación (costos negativos, checklist que
/// no corresponde, motivo vacío).
pub async fn avanzar_orden<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: OrdenTrabajoId,
    avance: AvanceOrden,
) -> AppResult<OrdenTrabajo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let (mut o, propia) = orden_visible(&mut tx, ctx, id, ahora).await?;
    let ejecuta = tiene(ctx, Permiso::MantenimientoPlanificar)
        || (propia && tiene(ctx, Permiso::MantenimientoEjecutar));
    let mut resolver = None;
    match avance {
        AvanceOrden::Iniciar => {
            if !ejecuta {
                return Err(AppError::SinPermiso(Permiso::MantenimientoEjecutar));
            }
            o.estado = o.estado.transicionar(EstadoOrden::EnEjecucion)?;
            o.iniciada_en.get_or_insert(ahora);
        }
        AvanceOrden::Ejecutar(e) => {
            if !ejecuta {
                return Err(AppError::SinPermiso(Permiso::MantenimientoEjecutar));
            }
            validar_valor("costo_mano_obra", Some(e.costo_mano_obra))?;
            validar_valor("costo_repuestos", Some(e.costo_repuestos))?;
            if !o.checklist.is_empty() {
                let mismos = e.checklist.len() == o.checklist.len()
                    && e.checklist
                        .iter()
                        .zip(&o.checklist)
                        .all(|(a, b)| a.descripcion == b.descripcion);
                if !mismos {
                    return Err(AppError::Validacion(
                        "diligencie todos los puntos del checklist de la orden".into(),
                    ));
                }
            }
            o.estado = o.estado.transicionar(EstadoOrden::Ejecutada)?;
            o.checklist = e
                .checklist
                .into_iter()
                .map(|mut p| {
                    p.observacion = texto_opcional("observacion", p.observacion.as_deref(), 500)?;
                    Ok(p)
                })
                .collect::<AppResult<Vec<_>>>()?;
            o.costo_mano_obra = e.costo_mano_obra;
            o.costo_repuestos = e.costo_repuestos;
            o.observaciones = texto_opcional("observaciones", e.observaciones.as_deref(), 2000)?;
            o.recibido_por = texto_opcional("recibido_por", e.recibido_por.as_deref(), 120)?;
            o.firma_archivo_id = e.firma_archivo_id;
            o.ejecutada_en = Some(ahora);
        }
        AvanceOrden::Verificar {
            aprobada,
            observacion,
            resolver_ticket,
        } => {
            ctx.exigir(Permiso::MantenimientoVerificar)?;
            let observacion = texto_opcional("observacion", observacion.as_deref(), 2000)?;
            if aprobada {
                o.estado = o.estado.transicionar(EstadoOrden::Verificada)?;
                o.verificada_en = Some(ahora);
                o.verificada_por = ctx.actor_id;
                if resolver_ticket && let Some(t) = o.ticket_id {
                    resolver = Some((t, observacion.clone()));
                }
            } else {
                o.estado = o.estado.transicionar(EstadoOrden::EnEjecucion)?;
                o.ejecutada_en = None;
            }
            if let Some(obs) = observacion {
                o.observaciones = Some(match o.observaciones.take() {
                    Some(prev) => format!("{prev}\nVerificación: {obs}"),
                    None => format!("Verificación: {obs}"),
                });
            }
        }
        AvanceOrden::Cancelar { motivo } => {
            ctx.exigir(Permiso::MantenimientoPlanificar)?;
            let motivo = texto_obligatorio("motivo", &motivo, 500)?;
            o.estado = o.estado.transicionar(EstadoOrden::Cancelada)?;
            o.observaciones = Some(format!("Cancelada: {motivo}"));
        }
    }
    o.actualizado_en = ahora;
    tx.actualizar_orden(&o).await?;
    if let Some((ticket_id, observacion)) = resolver {
        resolver_ticket_de_orden(&mut tx, ctx, &o, ticket_id, observacion, ahora).await?;
    }
    let detalle = DetalleAuditoria::vacio().con("estado", o.estado.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ESTADO_ORDEN, "orden_trabajo", id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(o)
}

/// Lleva el ticket a `resuelto` por el camino más corto del diagrama y avisa al solicitante.
async fn resolver_ticket_de_orden<T: TxMantenimiento>(
    tx: &mut T,
    ctx: &Contexto,
    o: &OrdenTrabajo,
    ticket_id: TicketId,
    observacion: Option<String>,
    ahora: DateTime<Utc>,
) -> AppResult<()> {
    let Some(mut t) = tx.ticket(ticket_id).await? else {
        return Ok(());
    };
    if !t.estado.pendiente() {
        return Ok(());
    }
    let respuesta = match observacion {
        Some(obs) => format!("Se ejecutó la orden de trabajo {}: {obs}", o.numero),
        None => format!("Se ejecutó la orden de trabajo {}.", o.numero),
    };
    let camino: &[EstadoTicket] = match t.estado {
        EstadoTicket::Abierto => &[EstadoTicket::EnRevision, EstadoTicket::Resuelto],
        EstadoTicket::EnEsperaTercero | EstadoTicket::EnEsperaSolicitante => {
            &[EstadoTicket::EnProgreso, EstadoTicket::Resuelto]
        }
        _ => &[EstadoTicket::Resuelto],
    };
    for hacia in camino {
        let motivo = (*hacia == EstadoTicket::Resuelto).then(|| respuesta.clone());
        registrar_transicion(tx, ctx, &mut t, *hacia, motivo, ahora).await?;
    }
    tx.insertar_mensaje_ticket(&MensajeTicket {
        id: domain::MensajeTicketId::nuevo(),
        ticket_id,
        autor_cuenta_id: ctx.actor_id,
        autor_persona_id: None,
        canal: domain::CanalTicket::Web,
        interno: false,
        texto: respuesta.clone(),
        adjuntos: Vec::new(),
        creado_en: ahora,
    })
    .await?;
    avisar_persona(
        tx,
        t.solicitante_persona_id,
        &aviso_al_solicitante(&t, &respuesta),
    )
    .await
}

/// Agrega una evidencia (foto antes/después o soporte) a una orden.
///
/// # Errors
///
/// [`AppError::SinPermiso`] si no la ejecuta ni planifica; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si está cancelada o verificada.
pub async fn agregar_evidencia<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: OrdenTrabajoId,
    archivo_id: ArchivoId,
    momento: MomentoEvidencia,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let (o, propia) = orden_visible(&mut tx, ctx, id, reloj.ahora()).await?;
    if !(tiene(ctx, Permiso::MantenimientoPlanificar)
        || (propia && tiene(ctx, Permiso::MantenimientoEjecutar)))
    {
        return Err(AppError::SinPermiso(Permiso::MantenimientoEjecutar));
    }
    if matches!(o.estado, EstadoOrden::Cancelada | EstadoOrden::Verificada) {
        return Err(AppError::Conflicto("la orden ya terminó".into()));
    }
    tx.insertar_evidencia(
        id,
        &EvidenciaOrden {
            archivo_id,
            momento,
        },
    )
    .await?;
    let detalle = DetalleAuditoria::vacio().con("momento", momento.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ESTADO_ORDEN, "orden_trabajo", id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await
}

// ---------------------------------------------------------------------------------------------
// Obligaciones periódicas
// ---------------------------------------------------------------------------------------------

/// Catálogo sugerido (no necesita transacción).
pub fn obligaciones_sugeridas() -> &'static [ObligacionSugerida] {
    OBLIGACIONES_SUGERIDAS
}

/// Datos de una obligación. Con `codigo_sugerido` los campos omitidos se toman del catálogo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatosObligacion {
    pub codigo_sugerido: Option<String>,
    pub nombre: Option<String>,
    pub descripcion: Option<String>,
    pub periodicidad_meses: Option<u32>,
    pub proxima: Option<NaiveDate>,
    pub dias_alerta: Option<u32>,
    pub activo_id: Option<ActivoId>,
    pub responsable_organizacion_id: Option<OrganizacionId>,
    pub activa: Option<bool>,
}

fn aplicar_datos(o: &mut Obligacion, d: DatosObligacion) -> AppResult<()> {
    if let Some(v) = d.nombre {
        o.nombre = v;
    }
    if d.descripcion.is_some() {
        o.descripcion = d.descripcion;
    }
    if let Some(v) = d.periodicidad_meses {
        o.periodicidad_meses = v;
    }
    if let Some(v) = d.proxima {
        o.proxima = v;
    }
    if let Some(v) = d.dias_alerta {
        o.dias_alerta = v;
    }
    if d.activo_id.is_some() {
        o.activo_id = d.activo_id;
    }
    if d.responsable_organizacion_id.is_some() {
        o.responsable_organizacion_id = d.responsable_organizacion_id;
    }
    if let Some(v) = d.activa {
        o.activa = v;
    }
    o.nombre = texto_obligatorio("nombre", &o.nombre, 120)?;
    o.descripcion = texto_opcional("descripcion", o.descripcion.as_deref(), 1000)?;
    if !(1..=120).contains(&o.periodicidad_meses) {
        return Err(AppError::Validacion(
            "`periodicidad_meses` debe estar entre 1 y 120".into(),
        ));
    }
    if o.dias_alerta > 365 {
        return Err(AppError::Validacion(
            "`dias_alerta` debe estar entre 0 y 365".into(),
        ));
    }
    Ok(())
}

/// Crea (`id = None`) o edita una obligación periódica.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `mantenimiento.planificar`; [`AppError::NoEncontrado`] si el
/// código sugerido o la obligación no existen; [`AppError::Validacion`] si faltan el nombre,
/// la periodicidad o la próxima fecha; [`AppError::Conflicto`] si la sugerida ya está activa.
pub async fn guardar_obligacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: Option<ObligacionId>,
    d: DatosObligacion,
) -> AppResult<Obligacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    ctx.exigir(Permiso::MantenimientoPlanificar)?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let obligacion = match id {
        Some(id) => {
            let mut o = tx
                .obligacion(id)
                .await?
                .ok_or(AppError::NoEncontrado("obligación"))?;
            aplicar_datos(
                &mut o,
                DatosObligacion {
                    codigo_sugerido: None,
                    ..d
                },
            )?;
            tx.actualizar_obligacion(&o).await?;
            o
        }
        None => {
            let sugerida = match &d.codigo_sugerido {
                Some(c) => Some(
                    OBLIGACIONES_SUGERIDAS
                        .iter()
                        .find(|s| s.codigo == c)
                        .ok_or(AppError::NoEncontrado("obligación sugerida"))?,
                ),
                None => None,
            };
            let faltan = || {
                AppError::Validacion("indique `nombre`, `periodicidad_meses` y `proxima`".into())
            };
            let mut o = Obligacion {
                id: ObligacionId::nuevo(),
                codigo_sugerido: sugerida.map(|s| s.codigo.to_owned()),
                nombre: d
                    .nombre
                    .clone()
                    .or_else(|| sugerida.map(|s| s.nombre.to_owned()))
                    .ok_or_else(faltan)?,
                descripcion: d
                    .descripcion
                    .clone()
                    .or_else(|| sugerida.map(|s| s.descripcion.to_owned())),
                periodicidad_meses: d
                    .periodicidad_meses
                    .or_else(|| sugerida.map(|s| s.periodicidad_meses))
                    .ok_or_else(faltan)?,
                proxima: d.proxima.ok_or_else(faltan)?,
                dias_alerta: 30,
                activo_id: None,
                responsable_organizacion_id: None,
                activa: true,
            };
            aplicar_datos(
                &mut o,
                DatosObligacion {
                    codigo_sugerido: None,
                    nombre: None,
                    descripcion: None,
                    periodicidad_meses: None,
                    proxima: None,
                    ..d
                },
            )?;
            tx.insertar_obligacion(&o).await?;
            o
        }
    };
    tx.registrar(EventoAuditoria::ok(
        ACCION_OBLIGACION,
        "obligacion",
        obligacion.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(obligacion)
}

/// Obligaciones periódicas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `mantenimiento.planificar` ni `inventario.ver`.
pub async fn listar_obligaciones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    solo_activas: bool,
) -> AppResult<Vec<Obligacion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    exigir_alguno(
        ctx,
        &[Permiso::MantenimientoPlanificar, Permiso::InventarioVer],
    )?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.obligaciones(solo_activas).await
}

/// Cumplimientos de una obligación.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`].
pub async fn cumplimientos_obligacion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ObligacionId,
) -> AppResult<Vec<Cumplimiento>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    exigir_alguno(
        ctx,
        &[Permiso::MantenimientoPlanificar, Permiso::InventarioVer],
    )?;
    let mut tx = abrir(unidad, reloj, ctx).await?;
    tx.obligacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("obligación"))?;
    tx.cumplimientos(id).await
}

/// Registra que se cumplió la obligación; la próxima fecha se calcula desde la realización.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `mantenimiento.planificar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si la fecha es futura.
pub async fn registrar_cumplimiento<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ObligacionId,
    realizada_el: NaiveDate,
    soporte_archivo_id: Option<ArchivoId>,
    observacion: Option<String>,
) -> AppResult<Obligacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    ctx.exigir(Permiso::MantenimientoPlanificar)?;
    let observacion = texto_opcional("observacion", observacion.as_deref(), 500)?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    if realizada_el > hoy(&mut tx, ahora).await? {
        return Err(AppError::Validacion(
            "la fecha de realización no puede ser futura".into(),
        ));
    }
    let mut o = tx
        .obligacion(id)
        .await?
        .ok_or(AppError::NoEncontrado("obligación"))?;
    tx.insertar_cumplimiento(&Cumplimiento {
        obligacion_id: id,
        realizada_el,
        soporte_archivo_id,
        observacion,
        registrado_por: ctx.actor_id,
        registrado_en: ahora,
    })
    .await?;
    o.proxima = proxima_obligacion(realizada_el, o.periodicidad_meses)
        .ok_or_else(|| AppError::Validacion("fecha fuera de rango".into()))?;
    tx.actualizar_obligacion(&o).await?;
    let detalle = DetalleAuditoria::vacio().con("proxima", o.proxima.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CUMPLIMIENTO, "obligacion", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(o)
}

// ---------------------------------------------------------------------------------------------
// Alertas y tablero
// ---------------------------------------------------------------------------------------------

/// Días antes del vencimiento de una garantía en que se alerta.
pub const DIAS_ALERTA_GARANTIA: i64 = 30;

/// Alertas de mantenimiento a la fecha local de hoy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlertasMantenimiento {
    /// Obligaciones vencidas o dentro de su ventana de alerta.
    pub obligaciones: Vec<Obligacion>,
    /// Activos con la garantía vencida en los últimos días o por vencer.
    pub garantias: Vec<Activo>,
    /// Órdenes pendientes con fecha programada ya pasada.
    pub ordenes_vencidas: Vec<OrdenTrabajo>,
}

/// Alertas: obligaciones por vencer, garantías por vencer y órdenes vencidas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `mantenimiento.planificar` ni `inventario.ver`.
pub async fn alertas_mantenimiento<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<AlertasMantenimiento>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    exigir_alguno(
        ctx,
        &[Permiso::MantenimientoPlanificar, Permiso::InventarioVer],
    )?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let hoy = hoy(&mut tx, ahora).await?;
    let obligaciones = tx
        .obligaciones(true)
        .await?
        .into_iter()
        .filter(|o| o.proxima - Duration::days(i64::from(o.dias_alerta)) <= hoy)
        .collect();
    let mut garantias = Vec::new();
    let mut despues = None;
    loop {
        let pagina = tx
            .activos(
                &FiltroActivos::default(),
                SolicitudPagina::new(Some(SolicitudPagina::LIMITE_MAXIMO), despues)?,
            )
            .await?;
        garantias.extend(pagina.datos.into_iter().filter(|a| {
            a.estado != EstadoActivo::DadoDeBaja
                && a.garantia_vence.is_some_and(|g| {
                    g >= hoy - Duration::days(DIAS_ALERTA_GARANTIA)
                        && g <= hoy + Duration::days(DIAS_ALERTA_GARANTIA)
                })
        }));
        match pagina.siguiente {
            Some(s) => despues = Some(s),
            None => break,
        }
    }
    let ordenes_vencidas = tx
        .ordenes(
            &FiltroOrdenes {
                vencidas_a: Some(hoy),
                ..FiltroOrdenes::default()
            },
            SolicitudPagina::new(Some(SolicitudPagina::LIMITE_MAXIMO), None)?,
        )
        .await?
        .datos;
    Ok(AlertasMantenimiento {
        obligaciones,
        garantias,
        ordenes_vencidas,
    })
}

/// Indicadores del tablero de mantenimiento.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `mantenimiento.planificar` ni `inventario.ver`.
pub async fn tablero_mantenimiento<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<TableroMantenimiento>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento,
{
    exigir_alguno(
        ctx,
        &[Permiso::MantenimientoPlanificar, Permiso::InventarioVer],
    )?;
    let ahora = reloj.ahora();
    let mut tx = abrir(unidad, reloj, ctx).await?;
    let hoy = hoy(&mut tx, ahora).await?;
    tx.tablero_mantenimiento(hoy).await
}

// ---------------------------------------------------------------------------------------------
// Worker de gestión
// ---------------------------------------------------------------------------------------------

/// Resultado de una pasada del worker de gestión.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumenGestion {
    pub copropiedades: u32,
    pub tickets_cerrados: u32,
    pub tickets_alertados: u32,
    pub ordenes_generadas: u32,
    /// Comunicados programados que salieron (plan 10).
    pub comunicados_enviados: u32,
}

/// Recorre las copropiedades con tareas pendientes y, con el contexto del sistema, cierra los
/// tickets resueltos vencidos, alerta los vencidos y genera las órdenes preventivas. Un error
/// en una copropiedad no detiene a las demás (queda en los logs). También envía los
/// comunicados programados cuya hora llegó ([`crate::casos_uso::comunicados::enviar_programados`]).
///
/// # Errors
///
/// Solo si falla la consulta del directorio.
pub async fn tareas_gestion<D, U>(
    directorio: &D,
    unidad: &U,
    reloj: &impl Reloj,
) -> AppResult<ResumenGestion>
where
    D: crate::puertos::DirectorioGestion,
    U: UnidadDeTrabajo,
    U::Transaccion: TxMantenimiento + crate::casos_uso::comunicados::TxComunicados,
{
    let mut r = ResumenGestion::default();
    for tenant in directorio.copropiedades_con_gestion_pendiente().await? {
        let ctx = Contexto::new(tenant);
        let tickets = crate::casos_uso::tickets::tareas_tickets(unidad, reloj, &ctx).await;
        let ordenes = generar_ordenes_preventivas(unidad, reloj, &ctx).await;
        let comunicados =
            crate::casos_uso::comunicados::enviar_programados(unidad, reloj, &ctx).await;
        match (tickets, ordenes, comunicados) {
            (Ok(t), Ok(o), Ok(c)) => {
                r.tickets_cerrados += t.cerrados;
                r.tickets_alertados += t.alertados;
                r.ordenes_generadas += o;
                r.comunicados_enviados += c;
            }
            (Err(AppError::ModuloNoHabilitado(_)), _, _)
            | (_, Err(AppError::ModuloNoHabilitado(_)), _)
            | (_, _, Err(AppError::ModuloNoHabilitado(_))) => {}
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
                tracing::error!(tenant_id = %tenant, error = %e, "tareas de gestión fallidas");
            }
        }
        r.copropiedades += 1;
    }
    Ok(r)
}

#[cfg(test)]
mod tests;
