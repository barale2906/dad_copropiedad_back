//! Configuración contable de la entidad: versión de la configuración, plan de cuentas, centros
//! de costo, tipos de comprobante, reglas de contabilización, rubros, notas, datos fiscales de
//! los terceros y plantillas (aplicar una o exportar la propia).

use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use domain::{
    ArchivoId, CentroCostoId, CuentaContableId, DireccionCorreo, Documento, NotaEstadosId, Permiso,
    PersonaId, PlantillaContableId, ReglaContabilizacionId, RubroEstadoFinancieroId,
    TipoComprobanteId, TipoDocumento, TipoPersona,
    contabilidad::{
        Cara, CodigoCuenta, CuentaPlantilla, LineaRegla, LineaReglaPlantilla, MarcoTecnico,
        Naturaleza, OrigenCentro, OrigenTercero, PlantillaContable, ReglaContable, ReglaPlantilla,
        RubroPlantilla, TipoComprobantePlantilla, validar_codigo_tipo_comprobante,
    },
};

use super::prelude::*;
use super::{TxContable, exigir_contabilidad, texto};
use crate::{
    AppError, AppResult, Contexto, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    paginacion::Pagina,
    puertos::{
        Reloj, UnidadDeTrabajo,
        contabilidad::{
            CentroCosto, ConfiguracionContable, CuentaContable, CuentaPuc, DatosFiscales,
            NotaEstados, PlantillaResumen, ReglaVersionada, RubroEstado, TipoComprobante,
        },
        personas::Persona,
    },
};

const ACCION_CONFIGURAR: AccionAuditoria =
    AccionAuditoria::new("contabilidad.configuracion.guardar");
const ACCION_CUENTA: AccionAuditoria = AccionAuditoria::new("contabilidad.cuenta.guardar");
const ACCION_CENTRO: AccionAuditoria = AccionAuditoria::new("contabilidad.centro_costo.guardar");
const ACCION_TIPO: AccionAuditoria = AccionAuditoria::new("contabilidad.tipo_comprobante.guardar");
const ACCION_REGLA: AccionAuditoria = AccionAuditoria::new("contabilidad.regla.crear");
const ACCION_RUBROS: AccionAuditoria = AccionAuditoria::new("contabilidad.rubros.reemplazar");
const ACCION_NOTA: AccionAuditoria = AccionAuditoria::new("contabilidad.nota.guardar");
const ACCION_TERCERO: AccionAuditoria = AccionAuditoria::new("contabilidad.tercero.guardar");
const ACCION_PLANTILLA: AccionAuditoria = AccionAuditoria::new("contabilidad.plantilla.aplicar");

async fn abrir<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    permiso: Permiso,
) -> AppResult<U::Transaccion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    ctx.exigir(permiso)?;
    let mut tx = unidad.iniciar(ctx).await?;
    exigir_contabilidad(&mut tx, reloj).await?;
    Ok(tx)
}

fn evento(accion: AccionAuditoria, entidad: &'static str, id: uuid::Uuid) -> EventoAuditoria {
    EventoAuditoria::ok(accion, entidad, id)
}

// ---------------------------------------------------------------------------------------------
// Configuración
// ---------------------------------------------------------------------------------------------

/// Configuración vigente (`None` si la entidad aún no la tiene).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`; [`AppError::ModuloNoHabilitado`].
pub async fn configuracion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Option<ConfiguracionContable>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .configuracion_vigente()
        .await
}

/// Datos de una configuración nueva.
#[derive(Debug, Clone)]
pub struct NuevaConfiguracion {
    pub marco_tecnico: MarcoTecnico,
    pub cuenta_resultado: Option<CodigoCuenta>,
    pub soporte_archivo_id: Option<ArchivoId>,
    pub vigente_desde: NaiveDate,
}

/// Guarda una versión nueva de la configuración (la anterior queda en el historial).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.configurar`; [`AppError::Validacion`] si la cuenta
/// de resultado no existe, no es de patrimonio (clase 3) o no es de movimiento.
pub async fn guardar_configuracion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaConfiguracion,
) -> AppResult<ConfiguracionContable>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadConfigurar).await?;
    let anterior = tx.configuracion_vigente().await?;
    let cuenta_resultado = match &nueva.cuenta_resultado {
        None => None,
        Some(codigo) => Some(cuenta_de_resultado(&mut tx, codigo).await?),
    };
    let c = ConfiguracionContable {
        version: anterior.as_ref().map_or(1, |a| a.version + 1),
        marco_tecnico: nueva.marco_tecnico,
        cuenta_resultado,
        soporte_archivo_id: nueva.soporte_archivo_id,
        plantilla_origen: anterior.and_then(|a| a.plantilla_origen),
        vigente_desde: nueva.vigente_desde,
        creada_por: ctx.actor_id,
        creada_en: reloj.ahora(),
    };
    tx.insertar_configuracion(&c).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("version", c.version)?
        .con("marco_tecnico", c.marco_tecnico.as_str())?;
    tx.registrar(
        evento(ACCION_CONFIGURAR, "entidad", ctx.tenant_id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

async fn cuenta_de_resultado<T: TxContable>(
    tx: &mut T,
    codigo: &CodigoCuenta,
) -> AppResult<CuentaContableId> {
    let cuenta = tx
        .cuenta_por_codigo(codigo)
        .await?
        .ok_or_else(|| AppError::Validacion(format!("la cuenta {codigo} no existe")))?;
    if codigo.clase() != 3 || !cuenta.acepta_movimiento || !cuenta.activa {
        return Err(AppError::Validacion(format!(
            "la cuenta de resultado {codigo} debe ser de patrimonio (clase 3), activa y de movimiento"
        )));
    }
    Ok(cuenta.id)
}

// ---------------------------------------------------------------------------------------------
// Plan de cuentas y PUC
// ---------------------------------------------------------------------------------------------

/// Plan de cuentas de la entidad, por código.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn cuentas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<CuentaContable>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .cuentas()
        .await
}

/// Catálogo PUC de la plataforma (solo lectura).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn catalogo_puc<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    prefijo: Option<&str>,
) -> AppResult<Vec<CuentaPuc>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .catalogo_puc(prefijo)
        .await
}

/// Datos de una cuenta nueva.
#[derive(Debug, Clone)]
pub struct NuevaCuenta {
    pub codigo: CodigoCuenta,
    pub nombre: String,
    /// `None` = la del PUC si el código está en el catálogo; si no, la de la cuenta padre.
    pub naturaleza: Option<Naturaleza>,
    pub acepta_movimiento: bool,
    pub exige_tercero: bool,
    pub exige_centro_costo: bool,
}

/// Crea una cuenta. Su padre (nivel superior) debe existir en el plan de la entidad, salvo que
/// sea una clase.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.configurar`; [`AppError::Validacion`] si falta el
/// padre, una clase o grupo pide movimientos o no se puede deducir la naturaleza;
/// [`AppError::Conflicto`] si el código ya existe.
pub async fn crear_cuenta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaCuenta,
) -> AppResult<CuentaContable>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let nombre = texto("nombre", &nueva.nombre, 200)?;
    if nueva.acepta_movimiento && !nueva.codigo.admite_movimiento() {
        return Err(AppError::Validacion(format!(
            "la cuenta {} es clase o grupo: no acepta movimientos",
            nueva.codigo
        )));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadConfigurar).await?;
    let padre =
        match nueva.codigo.padre() {
            None => None,
            Some(p) => Some(tx.cuenta_por_codigo(&p).await?.ok_or_else(|| {
                AppError::Validacion(format!("primero cree la cuenta padre {p}"))
            })?),
        };
    let naturaleza = match nueva.naturaleza {
        Some(n) => n,
        None => {
            let del_puc = tx
                .catalogo_puc(Some(nueva.codigo.as_str()))
                .await?
                .into_iter()
                .find(|c| c.codigo == nueva.codigo)
                .map(|c| c.naturaleza);
            del_puc
                .or(padre.map(|p| p.naturaleza))
                .ok_or_else(|| AppError::Validacion("indique la naturaleza de la cuenta".into()))?
        }
    };
    let cuenta = CuentaContable {
        id: CuentaContableId::nuevo(),
        codigo: nueva.codigo,
        nombre,
        naturaleza,
        acepta_movimiento: nueva.acepta_movimiento,
        exige_tercero: nueva.exige_tercero,
        exige_centro_costo: nueva.exige_centro_costo,
        activa: true,
    };
    tx.insertar_cuenta(&cuenta, reloj.ahora()).await?;
    auditar_cuenta(&mut tx, &cuenta).await?;
    tx.confirmar().await?;
    Ok(cuenta)
}

async fn auditar_cuenta<T: TxContable>(tx: &mut T, c: &CuentaContable) -> AppResult<()> {
    let detalle = DetalleAuditoria::vacio()
        .con("codigo", c.codigo.as_str())?
        .con("acepta_movimiento", c.acepta_movimiento)?
        .con("activa", c.activa)?;
    tx.registrar(evento(ACCION_CUENTA, "cuenta_contable", c.id.as_uuid()).con_detalle(detalle))
        .await
}

/// Lo que se puede cambiar de una cuenta (el código y la naturaleza no cambian).
#[derive(Debug, Clone)]
pub struct CambioCuenta {
    pub nombre: String,
    pub acepta_movimiento: bool,
    pub exige_tercero: bool,
    pub exige_centro_costo: bool,
    pub activa: bool,
}

/// Edita una cuenta.
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si no existe; [`AppError::Conflicto`] si tiene movimientos y se
/// le quita aceptar movimientos.
pub async fn editar_cuenta<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: CuentaContableId,
    cambio: CambioCuenta,
) -> AppResult<CuentaContable>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let nombre = texto("nombre", &cambio.nombre, 200)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadConfigurar).await?;
    let actual = tx
        .cuentas_por_id(&[id])
        .await?
        .pop()
        .ok_or(AppError::NoEncontrado("cuenta"))?;
    if cambio.acepta_movimiento && !actual.codigo.admite_movimiento() {
        return Err(AppError::Validacion(
            "una clase o grupo no acepta movimientos".into(),
        ));
    }
    if actual.acepta_movimiento
        && !cambio.acepta_movimiento
        && tx.cuenta_con_movimientos(id).await?
    {
        return Err(AppError::Conflicto(format!(
            "la cuenta {} ya tiene movimientos: inactívela en lugar de quitarle los movimientos",
            actual.codigo
        )));
    }
    let cuenta = CuentaContable {
        nombre,
        acepta_movimiento: cambio.acepta_movimiento,
        exige_tercero: cambio.exige_tercero,
        exige_centro_costo: cambio.exige_centro_costo,
        activa: cambio.activa,
        ..actual
    };
    tx.actualizar_cuenta(&cuenta, reloj.ahora()).await?;
    auditar_cuenta(&mut tx, &cuenta).await?;
    tx.confirmar().await?;
    Ok(cuenta)
}

// ---------------------------------------------------------------------------------------------
// Centros de costo y tipos de comprobante
// ---------------------------------------------------------------------------------------------

/// Centros de costo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn centros_costo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<CentroCosto>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .centros_costo()
        .await
}

fn validar_codigo_centro(codigo: &str) -> AppResult<()> {
    let ok = (1..=20).contains(&codigo.len())
        && codigo
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(AppError::Validacion(
            "el código del centro de costo tiene de 1 a 20 letras, dígitos, `.`, `_` o `-`".into(),
        ))
    }
}

/// Crea o edita un centro de costo (`id = None` = nuevo).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.configurar`; [`AppError::Conflicto`] si el código
/// ya existe; [`AppError::NoEncontrado`] si se edita uno que no existe.
pub async fn guardar_centro_costo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: Option<CentroCostoId>,
    codigo: &str,
    nombre: &str,
    activo: bool,
) -> AppResult<CentroCosto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    validar_codigo_centro(codigo)?;
    let nombre = texto("nombre", nombre, 120)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadConfigurar).await?;
    let centro = match id {
        None => {
            let c = CentroCosto {
                id: CentroCostoId::nuevo(),
                codigo: codigo.to_owned(),
                nombre,
                activo,
            };
            tx.insertar_centro_costo(&c, reloj.ahora()).await?;
            c
        }
        Some(id) => {
            let actual = tx
                .centros_costo()
                .await?
                .into_iter()
                .find(|c| c.id == id)
                .ok_or(AppError::NoEncontrado("centro de costo"))?;
            let c = CentroCosto {
                nombre,
                activo,
                ..actual
            };
            tx.actualizar_centro_costo(&c).await?;
            c
        }
    };
    let detalle = DetalleAuditoria::vacio().con("codigo", centro.codigo.as_str())?;
    tx.registrar(evento(ACCION_CENTRO, "centro_costo", centro.id.as_uuid()).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    Ok(centro)
}

/// Tipos de comprobante.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn tipos_comprobante<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<TipoComprobante>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .tipos_comprobante()
        .await
}

/// Datos de un tipo de comprobante.
#[derive(Debug, Clone)]
pub struct DatosTipoComprobante {
    /// Se usa solo al crear: el código no cambia.
    pub codigo: String,
    pub nombre: String,
    pub manual: bool,
    pub activo: bool,
}

/// Crea o edita un tipo de comprobante (`id = None` = nuevo; el código no cambia).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.configurar`; [`AppError::Validacion`] si el código
/// es inválido; [`AppError::Conflicto`] si ya existe.
pub async fn guardar_tipo_comprobante<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: Option<TipoComprobanteId>,
    datos: DatosTipoComprobante,
) -> AppResult<TipoComprobante>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let DatosTipoComprobante {
        codigo,
        nombre,
        manual,
        activo,
    } = datos;
    let codigo = codigo.as_str();
    let nombre = texto("nombre", &nombre, 80)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadConfigurar).await?;
    let tipo = match id {
        None => {
            validar_codigo_tipo_comprobante(codigo)?;
            let t = TipoComprobante {
                id: TipoComprobanteId::nuevo(),
                codigo: codigo.to_owned(),
                nombre,
                manual,
                activo,
            };
            tx.insertar_tipo_comprobante(&t).await?;
            t
        }
        Some(id) => {
            let actual = tx
                .tipos_comprobante()
                .await?
                .into_iter()
                .find(|t| t.id == id)
                .ok_or(AppError::NoEncontrado("tipo de comprobante"))?;
            let t = TipoComprobante {
                nombre,
                manual,
                activo,
                ..actual
            };
            tx.actualizar_tipo_comprobante(&t).await?;
            t
        }
    };
    let detalle = DetalleAuditoria::vacio().con("codigo", tipo.codigo.as_str())?;
    tx.registrar(evento(ACCION_TIPO, "tipo_comprobante", tipo.id.as_uuid()).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    Ok(tipo)
}

// ---------------------------------------------------------------------------------------------
// Reglas de contabilización
// ---------------------------------------------------------------------------------------------

/// Última versión de cada regla.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn reglas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<ReglaVersionada>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .reglas()
        .await
}

/// Línea de una regla nueva (por códigos).
#[derive(Debug, Clone)]
pub struct NuevaLineaRegla {
    pub concepto: String,
    pub cara: Cara,
    pub cuenta: CodigoCuenta,
    pub tercero: OrigenTercero,
    pub centro: OrigenCentro,
    /// Código del centro de costo si `centro = fijo`.
    pub centro_costo: Option<String>,
}

/// Versión nueva de una regla.
#[derive(Debug, Clone)]
pub struct NuevaRegla {
    pub evento: String,
    pub tipo_comprobante: String,
    pub descripcion: Option<String>,
    pub vigente_desde: NaiveDate,
    pub lineas: Vec<NuevaLineaRegla>,
}

/// Crea una versión nueva de la regla del evento (las anteriores siguen para lo ya
/// contabilizado y para fechas anteriores a `vigente_desde`).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.configurar`; [`AppError::Validacion`] si una cuenta,
/// centro o tipo no existe o una cuenta no es de movimiento; [`domain::DominioError`] si la regla
/// está mal formada.
pub async fn crear_regla<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaRegla,
) -> AppResult<ReglaVersionada>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadConfigurar).await?;
    let regla = crear_regla_en(&mut tx, reloj, ctx, nueva).await?;
    tx.confirmar().await?;
    Ok(regla)
}

async fn crear_regla_en<T: TxContable>(
    tx: &mut T,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaRegla,
) -> AppResult<ReglaVersionada> {
    let tipo = tx
        .tipos_comprobante()
        .await?
        .into_iter()
        .find(|t| t.codigo == nueva.tipo_comprobante)
        .ok_or_else(|| {
            AppError::Validacion(format!(
                "el tipo de comprobante {} no existe",
                nueva.tipo_comprobante
            ))
        })?;
    let cuentas: BTreeMap<String, CuentaContable> = tx
        .cuentas()
        .await?
        .into_iter()
        .map(|c| (c.codigo.as_str().to_owned(), c))
        .collect();
    let centros: BTreeMap<String, CentroCostoId> = tx
        .centros_costo()
        .await?
        .into_iter()
        .map(|c| (c.codigo, c.id))
        .collect();
    let mut lineas = Vec::with_capacity(nueva.lineas.len());
    for l in nueva.lineas {
        let cuenta = cuentas
            .get(l.cuenta.as_str())
            .filter(|c| c.acepta_movimiento && c.activa)
            .ok_or_else(|| {
                AppError::Validacion(format!(
                    "la cuenta {} no existe o no es de movimiento",
                    l.cuenta
                ))
            })?;
        let centro_fijo = match &l.centro_costo {
            None => None,
            Some(cod) => Some(*centros.get(cod).ok_or_else(|| {
                AppError::Validacion(format!("el centro de costo {cod} no existe"))
            })?),
        };
        lineas.push(LineaRegla {
            concepto: l.concepto,
            cara: l.cara,
            cuenta: cuenta.id,
            tercero: l.tercero,
            centro: l.centro,
            centro_fijo,
        });
    }
    let regla = ReglaContable {
        evento: nueva.evento,
        lineas,
    };
    regla.validar()?;
    let version = tx.ultima_version_regla(&regla.evento).await? + 1;
    let r = ReglaVersionada {
        id: ReglaContabilizacionId::nuevo(),
        version,
        tipo_comprobante_id: tipo.id,
        descripcion: nueva.descripcion,
        vigente_desde: nueva.vigente_desde,
        regla,
        creada_en: reloj.ahora(),
    };
    tx.insertar_regla(&r, ctx.actor_id).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("evento", r.regla.evento.as_str())?
        .con("version", r.version)?;
    tx.registrar(
        evento(ACCION_REGLA, "regla_contabilizacion", r.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    Ok(r)
}

// ---------------------------------------------------------------------------------------------
// Rubros y notas
// ---------------------------------------------------------------------------------------------

/// Rubros de los estados financieros.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn rubros<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<RubroEstado>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .rubros()
        .await
}

/// Reemplaza los rubros (mismas reglas que en una plantilla).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.configurar`; [`domain::DominioError`] si un rubro
/// está mal formado o repetido.
pub async fn reemplazar_rubros<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    rubros: Vec<RubroPlantilla>,
) -> AppResult<Vec<RubroEstado>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    PlantillaContable {
        rubros: rubros.clone(),
        ..Default::default()
    }
    .validar(&BTreeSet::new())?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadConfigurar).await?;
    let nuevos = reemplazar_rubros_en(&mut tx, ctx, rubros).await?;
    tx.confirmar().await?;
    Ok(nuevos)
}

async fn reemplazar_rubros_en<T: TxContable>(
    tx: &mut T,
    ctx: &Contexto,
    rubros: Vec<RubroPlantilla>,
) -> AppResult<Vec<RubroEstado>> {
    let existentes: BTreeMap<(String, String), RubroEstadoFinancieroId> = tx
        .rubros()
        .await?
        .into_iter()
        .map(|r| ((r.estado, r.codigo), r.id))
        .collect();
    let nuevos: Vec<RubroEstado> = rubros
        .into_iter()
        .map(|r| RubroEstado {
            id: existentes
                .get(&(r.estado.clone(), r.codigo.clone()))
                .copied()
                .unwrap_or_else(RubroEstadoFinancieroId::nuevo),
            estado: r.estado,
            codigo: r.codigo,
            nombre: r.nombre,
            orden: r.orden,
            prefijos: r.prefijos,
        })
        .collect();
    tx.reemplazar_rubros(&nuevos).await?;
    let detalle = DetalleAuditoria::vacio().con("rubros", nuevos.len())?;
    tx.registrar(evento(ACCION_RUBROS, "entidad", ctx.tenant_id.as_uuid()).con_detalle(detalle))
        .await?;
    Ok(nuevos)
}

/// Notas del año (última versión de cada una).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn notas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    anio: i32,
) -> AppResult<Vec<NotaEstados>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .notas(anio)
        .await
}

/// Nota nueva (o versión nueva de una existente).
#[derive(Debug, Clone)]
pub struct NuevaNota {
    pub anio: i32,
    pub numero: u16,
    pub titulo: String,
    pub texto: String,
    /// Código del rubro al que se asocia.
    pub rubro: Option<String>,
}

/// Guarda una versión nueva de la nota `numero` del año (las anteriores se conservan).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.configurar`; [`AppError::Validacion`] si falta el
/// título o el texto, o el rubro no existe.
pub async fn guardar_nota<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaNota,
) -> AppResult<NotaEstados>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let NuevaNota {
        anio,
        numero,
        titulo,
        texto: texto_nota,
        rubro,
    } = nueva;
    let rubro = rubro.as_deref();
    let titulo = texto("titulo", &titulo, 160)?;
    let cuerpo = texto("texto", &texto_nota, 20_000)?;
    if numero == 0 || !(2000..=2999).contains(&anio) {
        return Err(AppError::Validacion("número de nota o año inválido".into()));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadConfigurar).await?;
    let rubro_id = match rubro {
        None => None,
        Some(cod) => Some(
            tx.rubros()
                .await?
                .into_iter()
                .find(|r| r.codigo == cod)
                .ok_or_else(|| AppError::Validacion(format!("el rubro {cod} no existe")))?
                .id,
        ),
    };
    let nota = NotaEstados {
        id: NotaEstadosId::nuevo(),
        anio,
        numero,
        version: tx.ultima_version_nota(anio, numero).await? + 1,
        titulo,
        texto: cuerpo,
        rubro_id,
        creada_por: ctx.actor_id,
        creada_en: reloj.ahora(),
    };
    tx.insertar_nota(&nota).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("anio", anio)?
        .con("numero", numero)?
        .con("version", nota.version)?;
    tx.registrar(evento(ACCION_NOTA, "nota_estados", nota.id.as_uuid()).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    Ok(nota)
}

// ---------------------------------------------------------------------------------------------
// Terceros (personas de la entidad con datos fiscales)
// ---------------------------------------------------------------------------------------------

/// Tercero: la persona y sus datos fiscales.
#[derive(Debug, Clone)]
pub struct Tercero {
    pub persona: Persona,
    pub fiscal: DatosFiscales,
}

/// Terceros de la entidad (las personas), filtrables por nombre o documento.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn terceros<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    texto_busqueda: Option<&str>,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Tercero>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    let personas = tx.listar_personas(texto_busqueda, pagina).await?;
    let mut datos = Vec::with_capacity(personas.datos.len());
    for persona in personas.datos {
        let fiscal = tx.datos_fiscales(persona.id).await?.unwrap_or_default();
        datos.push(Tercero { persona, fiscal });
    }
    Ok(Pagina {
        datos,
        siguiente: personas.siguiente,
    })
}

fn validar_fiscales(d: &DatosFiscales) -> AppResult<()> {
    let digitos = |v: &Option<String>, n: usize| {
        v.as_deref()
            .is_none_or(|t| t.len() == n && t.chars().all(|c| c.is_ascii_digit()))
    };
    if !digitos(&d.actividad_ciiu, 4) {
        return Err(AppError::Validacion(
            "la actividad CIIU tiene cuatro dígitos".into(),
        ));
    }
    if !digitos(&d.municipio_fiscal_dane, 5) {
        return Err(AppError::Validacion(
            "el municipio DANE tiene cinco dígitos".into(),
        ));
    }
    if d.direccion_fiscal
        .as_deref()
        .is_some_and(|t| t.chars().count() > 200)
    {
        return Err(AppError::Validacion(
            "la dirección fiscal tiene a lo sumo 200 caracteres".into(),
        ));
    }
    if let Some(c) = &d.correo_facturacion {
        DireccionCorreo::new(c)?;
    }
    Ok(())
}

/// Datos de un tercero nuevo.
#[derive(Debug, Clone)]
pub struct NuevoTercero {
    pub tipo: TipoPersona,
    pub nombre: String,
    pub tipo_documento: Option<TipoDocumento>,
    pub documento: Documento,
    pub fiscal: DatosFiscales,
}

/// Crea un tercero (una persona de la entidad) con sus datos fiscales. Si ya existe una persona
/// con ese documento, la reutiliza y solo actualiza sus datos fiscales: no se duplican terceros.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.terceros.gestionar`; [`AppError::Validacion`] si un
/// dato fiscal es inválido.
pub async fn crear_tercero<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoTercero,
) -> AppResult<Tercero>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let nombre = texto("nombre", &nuevo.nombre, 200)?;
    validar_fiscales(&nuevo.fiscal)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadTercerosGestionar).await?;
    let persona = match tx.persona_por_documento(&nuevo.documento).await? {
        Some(p) => p,
        None => {
            let mut p = Persona::basica(
                PersonaId::nuevo(),
                nuevo.tipo,
                nombre,
                Some(nuevo.documento),
            );
            p.tipo_documento = nuevo.tipo_documento;
            tx.insertar_persona(&p).await?;
            p
        }
    };
    tx.guardar_datos_fiscales(persona.id, &nuevo.fiscal).await?;
    tx.registrar(evento(ACCION_TERCERO, "persona", persona.id.as_uuid()))
        .await?;
    tx.confirmar().await?;
    Ok(Tercero {
        persona,
        fiscal: nuevo.fiscal,
    })
}

/// Reemplaza los datos fiscales de una persona de la entidad.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.terceros.gestionar`; [`AppError::NoEncontrado`] si
/// la persona no existe; [`AppError::Validacion`] si un dato es inválido.
pub async fn guardar_datos_fiscales<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    persona: PersonaId,
    datos: DatosFiscales,
) -> AppResult<DatosFiscales>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    validar_fiscales(&datos)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadTercerosGestionar).await?;
    tx.persona(persona)
        .await?
        .ok_or(AppError::NoEncontrado("persona"))?;
    tx.guardar_datos_fiscales(persona, &datos).await?;
    tx.registrar(evento(ACCION_TERCERO, "persona", persona.as_uuid()))
        .await?;
    tx.confirmar().await?;
    Ok(datos)
}

// ---------------------------------------------------------------------------------------------
// Plantillas: aplicar una y exportar la configuración propia
// ---------------------------------------------------------------------------------------------

/// Plantillas que la cuenta puede aplicar.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.configurar`.
pub async fn plantillas_disponibles<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<PlantillaResumen>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadConfigurar)
        .await?
        .plantillas_disponibles(ctx.actor_id)
        .await
}

/// Qué hizo (o haría, en vista previa) aplicar una plantilla.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumenAplicacion {
    pub cuentas_creadas: u32,
    /// Ya existían en la entidad y se dejaron como estaban.
    pub cuentas_existentes: u32,
    pub tipos_creados: u32,
    pub tipos_existentes: u32,
    pub reglas_creadas: u32,
    pub rubros: u32,
    /// ¿Se creó una versión nueva de la configuración (marco técnico o cuenta de resultado)?
    pub configuracion: bool,
}

/// Aplica una plantilla: crea lo que falta (cuentas y tipos por código; lo existente no se
/// toca), una versión nueva de cada regla, reemplaza los rubros si la plantilla trae y, si trae
/// marco técnico o cuenta de resultado, una versión nueva de la configuración. Con
/// `previsualizar` hace todo y lo deshace: devuelve el resumen sin cambiar nada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.configurar`; [`AppError::NoEncontrado`] si la
/// plantilla no existe, no está publicada o es de una firma de la que la cuenta no es miembro;
/// [`domain::DominioError::ReglaContable`] si no es coherente con el plan de la entidad.
pub async fn aplicar_plantilla<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    plantilla: PlantillaContableId,
    previsualizar: bool,
) -> AppResult<ResumenAplicacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadConfigurar).await?;
    let (nombre, version, json) = tx
        .plantilla_para_aplicar(plantilla, ctx.actor_id)
        .await?
        .ok_or(AppError::NoEncontrado("plantilla"))?;
    let p: PlantillaContable = serde_json::from_value(json)
        .map_err(|e| AppError::Validacion(format!("la plantilla no es legible: {e}")))?;
    let existentes: BTreeSet<String> = tx
        .cuentas()
        .await?
        .into_iter()
        .map(|c| c.codigo.as_str().to_owned())
        .collect();
    p.validar(&existentes)?;
    let mut r = ResumenAplicacion::default();
    // Las cuentas, de padre a hijo (por largo de código).
    let mut cuentas = p.cuentas.clone();
    cuentas.sort_by_key(|c| (c.codigo.as_str().len(), c.codigo.clone()));
    for c in cuentas {
        if existentes.contains(c.codigo.as_str()) {
            r.cuentas_existentes += 1;
            continue;
        }
        let cuenta = CuentaContable {
            id: CuentaContableId::nuevo(),
            codigo: c.codigo,
            nombre: c.nombre,
            naturaleza: c.naturaleza,
            acepta_movimiento: c.acepta_movimiento,
            exige_tercero: c.exige_tercero,
            exige_centro_costo: c.exige_centro_costo,
            activa: true,
        };
        tx.insertar_cuenta(&cuenta, reloj.ahora()).await?;
        r.cuentas_creadas += 1;
    }
    let tipos: BTreeSet<String> = tx
        .tipos_comprobante()
        .await?
        .into_iter()
        .map(|t| t.codigo)
        .collect();
    for t in &p.tipos_comprobante {
        if tipos.contains(&t.codigo) {
            r.tipos_existentes += 1;
            continue;
        }
        tx.insertar_tipo_comprobante(&TipoComprobante {
            id: TipoComprobanteId::nuevo(),
            codigo: t.codigo.clone(),
            nombre: t.nombre.clone(),
            manual: t.manual,
            activo: true,
        })
        .await?;
        r.tipos_creados += 1;
    }
    let hoy = reloj.ahora().date_naive();
    for regla in &p.reglas {
        let nueva = NuevaRegla {
            evento: regla.evento.clone(),
            tipo_comprobante: regla.tipo_comprobante.clone(),
            descripcion: regla.descripcion.clone(),
            vigente_desde: hoy,
            lineas: regla
                .lineas
                .iter()
                .map(|l| NuevaLineaRegla {
                    concepto: l.concepto.clone(),
                    cara: l.cara,
                    cuenta: l.cuenta.clone(),
                    tercero: l.tercero,
                    centro: l.centro,
                    centro_costo: None,
                })
                .collect(),
        };
        crear_regla_en(&mut tx, reloj, ctx, nueva).await?;
        r.reglas_creadas += 1;
    }
    if !p.rubros.is_empty() {
        r.rubros = u32::try_from(
            reemplazar_rubros_en(&mut tx, ctx, p.rubros.clone())
                .await?
                .len(),
        )
        .unwrap_or(u32::MAX);
    }
    if p.marco_tecnico.is_some() || p.cuenta_resultado.is_some() {
        let anterior = tx.configuracion_vigente().await?;
        let cuenta_resultado = match &p.cuenta_resultado {
            Some(c) => Some(cuenta_de_resultado(&mut tx, c).await?),
            None => anterior.as_ref().and_then(|a| a.cuenta_resultado),
        };
        let marco = p
            .marco_tecnico
            .or(anterior.as_ref().map(|a| a.marco_tecnico))
            .ok_or_else(|| {
                AppError::Validacion(
                    "la plantilla no trae marco técnico y la entidad no tiene configuración".into(),
                )
            })?;
        tx.insertar_configuracion(&ConfiguracionContable {
            version: anterior.as_ref().map_or(1, |a| a.version + 1),
            marco_tecnico: marco,
            cuenta_resultado,
            soporte_archivo_id: anterior.as_ref().and_then(|a| a.soporte_archivo_id),
            plantilla_origen: Some(format!("{nombre} v{version}")),
            vigente_desde: anterior.as_ref().map_or(hoy, |a| a.vigente_desde),
            creada_por: ctx.actor_id,
            creada_en: reloj.ahora(),
        })
        .await?;
        r.configuracion = true;
    }
    if previsualizar {
        // Sin confirmar, la transacción se revierte al soltarse.
        return Ok(r);
    }
    let detalle = DetalleAuditoria::vacio()
        .con("plantilla", plantilla.as_uuid().to_string())?
        .con("version", version)?
        .con("cuentas_creadas", r.cuentas_creadas)?
        .con("reglas_creadas", r.reglas_creadas)?;
    tx.registrar(evento(ACCION_PLANTILLA, "entidad", ctx.tenant_id.as_uuid()).con_detalle(detalle))
        .await?;
    tx.confirmar().await?;
    Ok(r)
}

/// Exporta la configuración de la entidad como contenido de plantilla (para guardarla como
/// plantilla propia o estándar con [`super::plantillas::guardar_como_plantilla`]).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.configurar`.
pub async fn exportar_plantilla<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<PlantillaContable>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxContable,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadConfigurar).await?;
    let cuentas = tx.cuentas().await?;
    let por_id: BTreeMap<CuentaContableId, CodigoCuenta> =
        cuentas.iter().map(|c| (c.id, c.codigo.clone())).collect();
    let tipos = tx.tipos_comprobante().await?;
    let tipo_de: BTreeMap<TipoComprobanteId, String> =
        tipos.iter().map(|t| (t.id, t.codigo.clone())).collect();
    let config = tx.configuracion_vigente().await?;
    let mut reglas = Vec::new();
    for r in tx.reglas().await? {
        reglas.push(ReglaPlantilla {
            evento: r.regla.evento.clone(),
            tipo_comprobante: tipo_de
                .get(&r.tipo_comprobante_id)
                .cloned()
                .unwrap_or_default(),
            descripcion: r.descripcion.clone(),
            lineas: r
                .regla
                .lineas
                .iter()
                .filter_map(|l| {
                    Some(LineaReglaPlantilla {
                        concepto: l.concepto.clone(),
                        cara: l.cara,
                        cuenta: por_id.get(&l.cuenta)?.clone(),
                        tercero: l.tercero,
                        // Los centros son de cada entidad: el fijo se exporta como del documento.
                        centro: if l.centro == OrigenCentro::Fijo {
                            OrigenCentro::Documento
                        } else {
                            l.centro
                        },
                    })
                })
                .collect(),
        });
    }
    let rubros = tx
        .rubros()
        .await?
        .into_iter()
        .map(|r| RubroPlantilla {
            estado: r.estado,
            codigo: r.codigo,
            nombre: r.nombre,
            orden: r.orden,
            prefijos: r.prefijos,
        })
        .collect();
    Ok(PlantillaContable {
        marco_tecnico: config.as_ref().map(|c| c.marco_tecnico),
        cuenta_resultado: config
            .and_then(|c| c.cuenta_resultado)
            .and_then(|id| por_id.get(&id).cloned()),
        cuentas: cuentas
            .into_iter()
            .map(|c| CuentaPlantilla {
                codigo: c.codigo,
                nombre: c.nombre,
                naturaleza: c.naturaleza,
                acepta_movimiento: c.acepta_movimiento,
                exige_tercero: c.exige_tercero,
                exige_centro_costo: c.exige_centro_costo,
            })
            .collect(),
        tipos_comprobante: tipos
            .into_iter()
            .map(|t| TipoComprobantePlantilla {
                codigo: t.codigo,
                nombre: t.nombre,
                manual: t.manual,
            })
            .collect(),
        reglas,
        rubros,
    })
}
