//! Empleados y contratos laborales (plan 16 fase 1).
//!
//! El empleado es una **persona de la entidad** (la misma de los terceros o del censo: si ya
//! existe con ese documento se reutiliza) y nace con su primer contrato. Las condiciones del
//! contrato (cargo, salario, jornada, afiliaciones, cuenta) son **versiones**: modificarlas es
//! agregar una versión desde una fecha posterior a la última, validada contra el catálogo
//! laboral vigente en esa fecha. Un contrato a término fijo se prorroga; cualquier contrato se
//! termina con su motivo. Nada se borra.
//!
//! Los salarios y las cuentas bancarias son datos personales: solo los ve quien tiene
//! `nomina.salarios.ver` (para el resto salen vacíos).

use chrono::NaiveDate;
use domain::{
    AdministradoraId, ContratoLaboralId, DireccionCorreo, Documento, EmpleadoId, Permiso,
    PersonaId, Telefono, TipoDocumento, TipoPersona, VersionContratoId,
    contabilidad::Monto,
    nomina::{
        Afiliaciones, ClaseRiesgo, CodigoEmpleado, CondicionesContrato, EtapaAprendizaje, Jornada,
        MedioPago, MotivoTerminacion, PeriodicidadPago, TipoAdministradora, TipoContratoLaboral,
        TipoSalario, validar_condiciones, validar_fechas_contrato, validar_prorroga,
        validar_terminacion,
    },
};
use rust_decimal::Decimal;

use super::{TxNomina, abrir_nomina, catalogo::catalogo_en};
use crate::casos_uso::contabilidad::{texto, texto_opcional};
use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria as _, Reloj, Transaccion, UnidadDeTrabajo,
        nomina::{
            ContratoLaboral, CuentaNomina, Empleado, FiltroEmpleados, RepositorioCatalogoLaboral,
            RepositorioNomina as _, ResumenEmpleado, VersionContrato,
        },
        personas::{Persona, RepositorioPersonas as _},
    },
};

const ACCION_EMPLEADO: AccionAuditoria = AccionAuditoria::new("nomina.empleado.crear");
const ACCION_CONTRATO: AccionAuditoria = AccionAuditoria::new("nomina.contrato.crear");
const ACCION_MODIFICAR: AccionAuditoria = AccionAuditoria::new("nomina.contrato.modificar");
const ACCION_PRORROGAR: AccionAuditoria = AccionAuditoria::new("nomina.contrato.prorrogar");
const ACCION_TERMINAR: AccionAuditoria = AccionAuditoria::new("nomina.contrato.terminar");

/// Condiciones de una versión del contrato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CondicionesVersion {
    pub cargo: String,
    /// Salario mensual (o apoyo de sostenimiento del aprendiz).
    pub salario: Monto,
    pub tipo_salario: TipoSalario,
    /// Solo con el apoyo de sostenimiento.
    pub etapa_aprendizaje: Option<EtapaAprendizaje>,
    pub jornada: Jornada,
    /// Solo en jornada parcial.
    pub horas_semanales: Option<Decimal>,
    pub periodicidad: PeriodicidadPago,
    pub eps_id: AdministradoraId,
    pub afp_id: Option<AdministradoraId>,
    pub cesantias_id: Option<AdministradoraId>,
    pub ccf_id: Option<AdministradoraId>,
    pub arl_id: Option<AdministradoraId>,
    /// Va con la ARL.
    pub clase_riesgo: Option<ClaseRiesgo>,
    pub pensionado: bool,
    pub medio_pago: MedioPago,
    /// Solo con transferencia.
    pub cuenta: Option<CuentaNomina>,
}

/// Contrato nuevo con sus condiciones iniciales.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoContrato {
    pub tipo: TipoContratoLaboral,
    pub fecha_inicio: NaiveDate,
    /// Término fijo y aprendizaje.
    pub fecha_fin: Option<NaiveDate>,
    /// Obra o labor.
    pub descripcion_obra: Option<String>,
    pub condiciones: CondicionesVersion,
}

/// Datos de una persona que aún no está en la entidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevaPersonaEmpleado {
    pub nombre: String,
    pub tipo_documento: TipoDocumento,
    pub documento: Documento,
    pub correo: Option<DireccionCorreo>,
    pub telefono: Option<Telefono>,
}

/// Quién es el empleado: una persona que ya está en la entidad o una nueva (si el documento ya
/// existe, se usa esa persona).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonaEmpleado {
    Existente(PersonaId),
    Nueva(NuevaPersonaEmpleado),
}

/// Empleado nuevo con su primer contrato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoEmpleado {
    pub codigo: CodigoEmpleado,
    pub persona: PersonaEmpleado,
    pub contrato: NuevoContrato,
}

/// Contrato con sus versiones (de la más antigua a la más reciente).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContratoConVersiones {
    pub contrato: ContratoLaboral,
    pub versiones: Vec<VersionContrato>,
}

/// Empleado con su persona y su historia de contratos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FichaEmpleado {
    pub empleado: Empleado,
    /// Dato personal.
    pub nombre: String,
    pub tipo_documento: Option<TipoDocumento>,
    /// Dato personal.
    pub documento: Option<Documento>,
    pub contratos: Vec<ContratoConVersiones>,
}

/// Oculta salarios y cuentas a quien no tiene `nomina.salarios.ver`.
fn reservar(ctx: &Contexto, versiones: &mut [VersionContrato]) {
    if !ctx.permisos.contiene(Permiso::NominaSalariosVer) {
        for v in versiones {
            v.salario = None;
            v.cuenta = None;
        }
    }
}

fn validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

/// Persona del empleado: natural, con documento y sin otro empleo en la entidad.
async fn resolver_persona<T: TxNomina>(
    tx: &mut T,
    persona: &PersonaEmpleado,
) -> AppResult<(Persona, bool)> {
    let (p, nueva) = match persona {
        PersonaEmpleado::Existente(id) => (
            tx.persona(*id)
                .await?
                .ok_or(AppError::NoEncontrado("persona"))?,
            false,
        ),
        PersonaEmpleado::Nueva(n) => match tx.persona_por_documento(&n.documento).await? {
            Some(p) => (p, false),
            None => {
                let mut p = Persona::basica(
                    PersonaId::nuevo(),
                    TipoPersona::Natural,
                    texto("nombre", &n.nombre, 200)?,
                    Some(n.documento.clone()),
                );
                p.tipo_documento = Some(n.tipo_documento);
                p.correo = n.correo.clone();
                p.telefono = n.telefono.clone();
                tx.insertar_persona(&p).await?;
                (p, true)
            }
        },
    };
    if p.tipo != TipoPersona::Natural || p.documento.is_none() {
        return Err(validacion(
            "el empleado es una persona natural con documento de identidad",
        ));
    }
    if tx.empleado_de_persona(p.id).await?.is_some() {
        return Err(AppError::Conflicto(
            "la persona ya es empleada: agréguele un contrato".into(),
        ));
    }
    Ok((p, nueva))
}

/// Verifica que la administradora exista, esté activa y sea del tipo esperado.
async fn exigir_administradora<T: RepositorioCatalogoLaboral>(
    tx: &mut T,
    id: Option<AdministradoraId>,
    tipo: TipoAdministradora,
) -> AppResult<bool> {
    let Some(id) = id else { return Ok(false) };
    match tx.administradora(id).await? {
        Some(a) if a.tipo == tipo && a.activa => Ok(true),
        _ => Err(validacion(format!(
            "la administradora de `{tipo}` no existe, no es de ese tipo o no está activa"
        ))),
    }
}

/// Valida las condiciones de una versión del contrato contra el catálogo vigente en `fecha`.
async fn validar_version<T: TxNomina>(
    tx: &mut T,
    tipo_contrato: TipoContratoLaboral,
    fecha: NaiveDate,
    c: &CondicionesVersion,
) -> AppResult<()> {
    if c.arl_id.is_some() != c.clase_riesgo.is_some() {
        return Err(validacion("la ARL va con la clase de riesgo"));
    }
    match (&c.cuenta, c.medio_pago) {
        (Some(cuenta), MedioPago::Transferencia) => {
            texto("banco", &cuenta.banco, 120)?;
            if !(4..=20).contains(&cuenta.numero.len())
                || !cuenta.numero.chars().all(|x| x.is_ascii_digit())
            {
                return Err(validacion("el número de la cuenta tiene de 4 a 20 dígitos"));
            }
        }
        (None, MedioPago::Transferencia) => {
            return Err(validacion("la transferencia lleva la cuenta del empleado"));
        }
        (Some(_), _) => return Err(validacion("solo la transferencia lleva cuenta")),
        (None, _) => {}
    }
    let afiliaciones = Afiliaciones {
        eps: exigir_administradora(tx, Some(c.eps_id), TipoAdministradora::Eps).await?,
        afp: exigir_administradora(tx, c.afp_id, TipoAdministradora::Afp).await?,
        cesantias: exigir_administradora(tx, c.cesantias_id, TipoAdministradora::Cesantias).await?,
        ccf: exigir_administradora(tx, c.ccf_id, TipoAdministradora::Ccf).await?,
        arl: exigir_administradora(tx, c.arl_id, TipoAdministradora::Arl).await?,
    };
    let catalogo = catalogo_en(tx, fecha).await?;
    validar_condiciones(
        &CondicionesContrato {
            tipo_contrato,
            salario: c.salario,
            tipo_salario: c.tipo_salario,
            etapa_aprendizaje: c.etapa_aprendizaje,
            jornada: c.jornada,
            horas_semanales: c.horas_semanales,
            pensionado: c.pensionado,
            afiliaciones,
        },
        &catalogo,
    )?;
    Ok(())
}

fn version_de(
    ctx: &Contexto,
    reloj: &impl Reloj,
    contrato: ContratoLaboralId,
    vigente_desde: NaiveDate,
    c: CondicionesVersion,
) -> AppResult<VersionContrato> {
    Ok(VersionContrato {
        id: VersionContratoId::nuevo(),
        contrato_id: contrato,
        vigente_desde,
        cargo: texto("cargo", &c.cargo, 120)?,
        salario: Some(c.salario),
        tipo_salario: c.tipo_salario,
        etapa_aprendizaje: c.etapa_aprendizaje,
        jornada: c.jornada,
        horas_semanales: c.horas_semanales,
        periodicidad: c.periodicidad,
        eps_id: c.eps_id,
        afp_id: c.afp_id,
        cesantias_id: c.cesantias_id,
        ccf_id: c.ccf_id,
        arl_id: c.arl_id,
        clase_riesgo: c.clase_riesgo,
        pensionado: c.pensionado,
        medio_pago: c.medio_pago,
        cuenta: c.cuenta,
        creada_por: ctx.actor_id,
        creada_en: reloj.ahora(),
    })
}

/// Valida fechas y condiciones de un contrato nuevo sin guardarlo (la vista previa de la
/// importación lo usa). Devuelve la descripción de la obra normalizada.
pub(crate) async fn validar_nuevo_contrato<T: TxNomina>(
    tx: &mut T,
    nuevo: &NuevoContrato,
) -> AppResult<Option<String>> {
    let descripcion = texto_opcional("descripcion_obra", nuevo.descripcion_obra.as_deref(), 300)?;
    validar_fechas_contrato(
        nuevo.tipo,
        nuevo.fecha_inicio,
        nuevo.fecha_fin,
        descripcion.as_deref(),
    )?;
    validar_version(tx, nuevo.tipo, nuevo.fecha_inicio, &nuevo.condiciones).await?;
    Ok(descripcion)
}

/// Crea el contrato con su primera versión dentro de la transacción (lo usan el alta del
/// empleado, el contrato nuevo y la importación).
pub(crate) async fn contratar_en<T: TxNomina>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    empleado: EmpleadoId,
    nuevo: NuevoContrato,
) -> AppResult<ContratoConVersiones> {
    let descripcion = validar_nuevo_contrato(tx, &nuevo).await?;
    let contrato = ContratoLaboral {
        id: ContratoLaboralId::nuevo(),
        empleado_id: empleado,
        tipo: nuevo.tipo,
        fecha_inicio: nuevo.fecha_inicio,
        fecha_fin: nuevo.fecha_fin,
        descripcion_obra: descripcion,
        fecha_terminacion: None,
        motivo_terminacion: None,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
        terminado_por: None,
        terminado_en: None,
    };
    let version = version_de(
        ctx,
        reloj,
        contrato.id,
        contrato.fecha_inicio,
        nuevo.condiciones,
    )?;
    tx.insertar_contrato(&contrato).await?;
    tx.insertar_version(&version).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("empleado_id", empleado.as_uuid().to_string())?
        .con("tipo", contrato.tipo.as_str())?
        .con("fecha_inicio", contrato.fecha_inicio.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CONTRATO, "contrato_laboral", contrato.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    Ok(ContratoConVersiones {
        contrato,
        versiones: vec![version],
    })
}

/// Crea el empleado (y su persona, si no existe) dentro de la transacción. Devuelve si la
/// persona es nueva.
pub(crate) async fn crear_empleado_en<T: TxNomina>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoEmpleado,
) -> AppResult<(FichaEmpleado, bool)> {
    let (persona, persona_nueva) = resolver_persona(tx, &nuevo.persona).await?;
    let empleado = Empleado {
        id: EmpleadoId::nuevo(),
        codigo: nuevo.codigo,
        persona_id: persona.id,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
    };
    tx.insertar_empleado(&empleado).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("codigo", empleado.codigo.as_str())?
        .con("persona_nueva", persona_nueva)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EMPLEADO, "empleado", empleado.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    let contrato = contratar_en(tx, reloj, ctx, empleado.id, nuevo.contrato).await?;
    Ok((
        FichaEmpleado {
            empleado,
            nombre: persona.nombre,
            tipo_documento: persona.tipo_documento,
            documento: persona.documento,
            contratos: vec![contrato],
        },
        persona_nueva,
    ))
}

/// Da de alta un empleado con su primer contrato. Si la persona no está en la entidad se crea;
/// si ya existe con ese documento, se usa.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `nomina.empleados.gestionar`;
///   [`AppError::ModuloNoHabilitado`] sin nómina.
/// - [`AppError::NoEncontrado`] si la persona indicada no existe.
/// - [`AppError::Conflicto`] si el código ya existe o la persona ya es empleada.
/// - [`AppError::Validacion`] o [`domain::DominioError::ReglaLaboral`] si el contrato no cumple
///   (fechas, salario, afiliaciones, catálogo vigente).
pub async fn crear_empleado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoEmpleado,
) -> AppResult<FichaEmpleado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaEmpleadosGestionar).await?;
    let (mut ficha, _) = crear_empleado_en(&mut tx, reloj, ctx, nuevo).await?;
    tx.confirmar().await?;
    for c in &mut ficha.contratos {
        reservar(ctx, &mut c.versiones);
    }
    Ok(ficha)
}

/// Agrega un contrato a un empleado (recontratación o contrato nuevo tras el anterior).
///
/// # Errors
///
/// Los de [`crear_empleado`]; [`AppError::NoEncontrado`] si el empleado no existe;
/// [`AppError::Conflicto`] si se cruza en fechas con otro contrato del empleado.
pub async fn crear_contrato<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    empleado: EmpleadoId,
    nuevo: NuevoContrato,
) -> AppResult<ContratoConVersiones>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaEmpleadosGestionar).await?;
    tx.empleado(empleado)
        .await?
        .ok_or(AppError::NoEncontrado("empleado"))?;
    let mut c = contratar_en(&mut tx, reloj, ctx, empleado, nuevo).await?;
    tx.confirmar().await?;
    reservar(ctx, &mut c.versiones);
    Ok(c)
}

/// Contrato vigente (no terminado) por id.
async fn contrato_abierto<T: TxNomina>(
    tx: &mut T,
    id: ContratoLaboralId,
) -> AppResult<ContratoLaboral> {
    let c = tx
        .contrato(id)
        .await?
        .ok_or(AppError::NoEncontrado("contrato"))?;
    if c.fecha_terminacion.is_some() {
        return Err(AppError::Conflicto("el contrato ya terminó".into()));
    }
    Ok(c)
}

/// Modifica las condiciones del contrato desde `vigente_desde` (una versión nueva, posterior a
/// la última y dentro de las fechas del contrato).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.empleados.gestionar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si el contrato terminó; [`AppError::Validacion`] o
/// [`domain::DominioError::ReglaLaboral`] si la fecha o las condiciones no cumplen.
pub async fn modificar_contrato<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    contrato: ContratoLaboralId,
    vigente_desde: NaiveDate,
    condiciones: CondicionesVersion,
) -> AppResult<VersionContrato>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaEmpleadosGestionar).await?;
    let c = contrato_abierto(&mut tx, contrato).await?;
    let ultima = tx
        .versiones(contrato)
        .await?
        .last()
        .map(|v| v.vigente_desde);
    if ultima.is_some_and(|u| vigente_desde <= u) || c.fecha_fin.is_some_and(|f| vigente_desde > f)
    {
        return Err(validacion(
            "la modificación empieza después de la última versión y dentro del contrato",
        ));
    }
    validar_version(&mut tx, c.tipo, vigente_desde, &condiciones).await?;
    let mut v = version_de(ctx, reloj, contrato, vigente_desde, condiciones)?;
    tx.insertar_version(&v).await?;
    let detalle = DetalleAuditoria::vacio().con("vigente_desde", vigente_desde.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_MODIFICAR, "contrato_laboral", contrato.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    reservar(ctx, std::slice::from_mut(&mut v));
    Ok(v)
}

/// Prorroga un contrato a término fijo hasta `nueva_fecha_fin`.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.empleados.gestionar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si terminó o la prórroga lo cruza con otro contrato;
/// [`domain::DominioError::ReglaLaboral`] si no es a término fijo o la fecha no es posterior.
pub async fn prorrogar_contrato<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    contrato: ContratoLaboralId,
    nueva_fecha_fin: NaiveDate,
) -> AppResult<ContratoLaboral>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaEmpleadosGestionar).await?;
    let mut c = contrato_abierto(&mut tx, contrato).await?;
    validar_prorroga(c.tipo, c.fecha_fin, nueva_fecha_fin)?;
    let anterior = c.fecha_fin;
    c.fecha_fin = Some(nueva_fecha_fin);
    tx.actualizar_contrato(&c).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("fecha_fin_anterior", anterior.map(|f| f.to_string()))?
        .con("fecha_fin", nueva_fecha_fin.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PRORROGAR, "contrato_laboral", c.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Termina el contrato en `fecha` con su motivo (la liquidación llega en la fase 4). No se
/// termina antes de su última versión.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.empleados.gestionar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ya terminó; [`domain::DominioError::ReglaLaboral`] si la fecha
/// cae fuera del contrato o antes de la última versión, o el motivo no aplica al tipo.
pub async fn terminar_contrato<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    contrato: ContratoLaboralId,
    fecha: NaiveDate,
    motivo: MotivoTerminacion,
) -> AppResult<ContratoLaboral>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaEmpleadosGestionar).await?;
    let mut c = contrato_abierto(&mut tx, contrato).await?;
    validar_terminacion(c.tipo, c.fecha_inicio, c.fecha_fin, fecha, motivo)?;
    if tx
        .versiones(contrato)
        .await?
        .last()
        .is_some_and(|v| v.vigente_desde > fecha)
    {
        return Err(domain::DominioError::ReglaLaboral(
            "el contrato tiene una versión posterior a la terminación".into(),
        )
        .into());
    }
    c.fecha_terminacion = Some(fecha);
    c.motivo_terminacion = Some(motivo);
    c.terminado_por = ctx.actor_id;
    c.terminado_en = Some(reloj.ahora());
    tx.actualizar_contrato(&c).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("fecha", fecha.to_string())?
        .con("motivo", motivo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_TERMINAR, "contrato_laboral", c.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Empleados de la entidad con su estado de hoy (paginado por id).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.ver`; [`AppError::ModuloNoHabilitado`].
pub async fn empleados<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroEmpleados,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<ResumenEmpleado>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaVer).await?;
    tx.empleados(&filtro, reloj.ahora().date_naive(), pagina)
        .await
}

/// Ficha del empleado con todos sus contratos y versiones (salarios y cuentas solo con
/// `nomina.salarios.ver`).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `nomina.ver`; [`AppError::NoEncontrado`].
pub async fn ficha_empleado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: EmpleadoId,
) -> AppResult<FichaEmpleado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxNomina,
{
    let mut tx = abrir_nomina(unidad, reloj, ctx, Permiso::NominaVer).await?;
    let empleado = tx
        .empleado(id)
        .await?
        .ok_or(AppError::NoEncontrado("empleado"))?;
    let persona = tx
        .persona(empleado.persona_id)
        .await?
        .ok_or(AppError::NoEncontrado("persona"))?;
    let mut contratos = Vec::new();
    for contrato in tx.contratos(id).await? {
        let mut versiones = tx.versiones(contrato.id).await?;
        reservar(ctx, &mut versiones);
        contratos.push(ContratoConVersiones {
            contrato,
            versiones,
        });
    }
    Ok(FichaEmpleado {
        empleado,
        nombre: persona.nombre,
        tipo_documento: persona.tipo_documento,
        documento: persona.documento,
        contratos,
    })
}
