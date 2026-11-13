//! Configuración de la cartera (plan 14 fase 1 y módulos de la fase 2): módulos de
//! contribución, conceptos de cobro, políticas de cartera versionadas y datos de pago de cada
//! bien privado. Las variables las elige el contador o la asamblea (P1); aquí solo se validan
//! los topes legales.

use std::collections::BTreeSet;

use chrono::NaiveDate;
use domain::{
    BienPrivadoId, ConceptoCobroId, ModuloContribucionId, Permiso, PersonaId, PoliticaCarteraId,
    PreguntaId, TipoBienPrivado,
    contabilidad::Monto,
    contabilidad_ph::{CriterioDistribucion, ParametrosPolitica, TipoConceptoCobro},
};

use super::{TxCartera, abrir, validacion};
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::contabilidad::{prelude::*, texto, texto_opcional},
    puertos::{
        Reloj, RepositorioCartera as _, UnidadDeTrabajo,
        cartera::{BienCartera, ConceptoCobro, ModuloContribucion, PoliticaCartera},
    },
};

const ACCION_MODULO: AccionAuditoria = AccionAuditoria::new("cartera.modulo_contribucion.guardar");
const ACCION_CONCEPTO: AccionAuditoria = AccionAuditoria::new("cartera.concepto_cobro.guardar");
const ACCION_POLITICA: AccionAuditoria = AccionAuditoria::new("cartera.politica.crear");
const ACCION_DATOS_PAGO: AccionAuditoria = AccionAuditoria::new("cartera.bien.datos_pago");

// ---------------------------------------------------------------------------------------------
// Módulos de contribución
// ---------------------------------------------------------------------------------------------

/// Módulos de contribución por nombre.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn modulos_contribucion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<ModuloContribucion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    abrir(unidad, reloj, ctx, Permiso::CarteraVer)
        .await?
        .modulos_contribucion()
        .await
}

/// Crea (`id = None`) o actualiza un módulo de contribución. Sus coeficientes se cargan como un
/// cuadro de coeficientes del módulo (`casos_uso::coeficientes`).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.configurar`; [`AppError::Validacion`];
/// [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si el nombre ya existe.
pub async fn guardar_modulo_contribucion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: Option<ModuloContribucionId>,
    nombre: &str,
    descripcion: Option<&str>,
    activo: bool,
) -> AppResult<ModuloContribucion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let nombre = texto("nombre", nombre, 100)?;
    let descripcion = texto_opcional("descripcion", descripcion, 300)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraConfigurar).await?;
    let modulo = match id {
        None => ModuloContribucion {
            id: ModuloContribucionId::nuevo(),
            nombre,
            descripcion,
            activo,
            creado_en: reloj.ahora(),
        },
        Some(id) => {
            let actual = tx
                .modulos_contribucion()
                .await?
                .into_iter()
                .find(|m| m.id == id)
                .ok_or(AppError::NoEncontrado("módulo de contribución"))?;
            ModuloContribucion {
                nombre,
                descripcion,
                activo,
                ..actual
            }
        }
    };
    tx.guardar_modulo_contribucion(&modulo).await?;
    let detalle = DetalleAuditoria::vacio().con("activo", modulo.activo)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_MODULO, "modulo_contribucion", modulo.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(modulo)
}

// ---------------------------------------------------------------------------------------------
// Conceptos de cobro
// ---------------------------------------------------------------------------------------------

/// Conceptos de cobro por código.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn conceptos_cobro<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<ConceptoCobro>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    abrir(unidad, reloj, ctx, Permiso::CarteraVer)
        .await?
        .conceptos_cobro()
        .await
}

/// Concepto de cobro nuevo.
#[derive(Debug, Clone)]
pub struct NuevoConcepto {
    /// `administracion`, `fondo_imprevistos`… (minúsculas, dígitos y `_`).
    pub codigo: String,
    pub nombre: String,
    pub tipo: TipoConceptoCobro,
    pub criterio: CriterioDistribucion,
    pub modulo: Option<ModuloContribucionId>,
    pub valor_fijo: Option<Monto>,
    pub tarifas: Vec<(TipoBienPrivado, Monto)>,
    pub genera_interes: bool,
    /// Concepto de las reglas `ph.*` que elige las cuentas.
    pub concepto_contable: String,
}

/// Lo que se puede cambiar de un concepto (el código, el tipo y el criterio no cambian: otro
/// reparto es otro concepto).
#[derive(Debug, Clone)]
pub struct CambioConcepto {
    pub nombre: String,
    pub valor_fijo: Option<Monto>,
    pub tarifas: Vec<(TipoBienPrivado, Monto)>,
    pub genera_interes: bool,
    pub concepto_contable: String,
    pub activo: bool,
}

fn es_codigo(texto: &str, minimo: usize, maximo: usize) -> bool {
    let mut c = texto.chars();
    (minimo..=maximo).contains(&texto.len())
        && c.next().is_some_and(|x| x.is_ascii_lowercase())
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '_')
}

/// Coherencia del concepto: lo que exige cada criterio y cada tipo.
async fn validar_concepto<T: TxCartera>(tx: &mut T, c: &ConceptoCobro) -> AppResult<()> {
    if !es_codigo(&c.codigo, 2, 40) {
        return Err(validacion(
            "el código usa minúsculas, dígitos y `_`, empieza por letra (2 a 40)",
        ));
    }
    if !es_codigo(&c.concepto_contable, 1, 60) {
        return Err(validacion(
            "el concepto contable usa minúsculas, dígitos y `_` y empieza por letra",
        ));
    }
    match c.criterio {
        CriterioDistribucion::Modulo => {
            let m = c
                .modulo_id
                .ok_or_else(|| validacion("el criterio `modulo` exige el módulo"))?;
            let activo = tx
                .modulos_contribucion()
                .await?
                .iter()
                .any(|x| x.id == m && x.activo);
            if !activo {
                return Err(AppError::NoEncontrado("módulo de contribución"));
            }
        }
        _ if c.modulo_id.is_some() => {
            return Err(validacion("solo el criterio `modulo` lleva módulo"));
        }
        _ => {}
    }
    if (c.criterio == CriterioDistribucion::ValorFijo) != c.valor_fijo.is_some_and(|v| !v.es_cero())
    {
        return Err(validacion(
            "el criterio `valor_fijo` exige un valor mayor que cero (y solo él lo lleva)",
        ));
    }
    let tipos: BTreeSet<&str> = c.tarifas.iter().map(|(t, _)| t.as_str()).collect();
    if tipos.len() != c.tarifas.len() || c.tarifas.iter().any(|(_, v)| v.es_cero()) {
        return Err(validacion(
            "cada tipo de bien lleva una sola tarifa mayor que cero",
        ));
    }
    if (c.criterio == CriterioDistribucion::PorTipoBien) != !c.tarifas.is_empty() {
        return Err(validacion(
            "el criterio `por_tipo_bien` exige tarifas (y solo él las lleva)",
        ));
    }
    if c.tipo == TipoConceptoCobro::Extraordinaria && !c.criterio.reparte_total() {
        return Err(validacion(
            "una cuota extraordinaria se reparte por coeficiente, módulo o área",
        ));
    }
    if c.tipo == TipoConceptoCobro::InteresMora {
        // Sin anatocismo: los intereses no generan intereses.
        if c.genera_interes {
            return Err(validacion("los intereses de mora no generan intereses"));
        }
        let otro = tx
            .conceptos_cobro()
            .await?
            .into_iter()
            .any(|x| x.activo && x.id != c.id && x.tipo == TipoConceptoCobro::InteresMora);
        if c.activo && otro {
            return Err(AppError::Conflicto(
                "ya hay un concepto activo de intereses de mora".into(),
            ));
        }
    }
    if c.tipo == TipoConceptoCobro::FondoImprevistos {
        if c.criterio != CriterioDistribucion::Coeficiente {
            return Err(validacion(
                "el fondo de imprevistos se calcula sobre lo ordinario de cada bien: use el \
                 criterio `coeficiente`",
            ));
        }
        let otro = tx
            .conceptos_cobro()
            .await?
            .into_iter()
            .any(|x| x.activo && x.id != c.id && x.tipo == TipoConceptoCobro::FondoImprevistos);
        if c.activo && otro {
            return Err(AppError::Conflicto(
                "ya hay un concepto activo del fondo de imprevistos".into(),
            ));
        }
    }
    Ok(())
}

async fn guardar_concepto<T: TxCartera>(tx: &mut T, c: &ConceptoCobro) -> AppResult<()> {
    validar_concepto(tx, c).await?;
    tx.guardar_concepto_cobro(c).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("codigo", c.codigo.as_str())?
        .con("tipo", c.tipo.as_str())?
        .con("criterio", c.criterio.as_str())?
        .con("activo", c.activo)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CONCEPTO, "concepto_cobro", c.id.as_uuid()).con_detalle(detalle),
    )
    .await
}

/// Crea un concepto de cobro.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.configurar`; [`AppError::Validacion`] si no es
/// coherente con su criterio o su tipo; [`AppError::Conflicto`] si el código existe o ya hay un
/// fondo de imprevistos activo.
pub async fn crear_concepto_cobro<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nuevo: NuevoConcepto,
) -> AppResult<ConceptoCobro>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraConfigurar).await?;
    let concepto = ConceptoCobro {
        id: ConceptoCobroId::nuevo(),
        codigo: nuevo.codigo.trim().to_owned(),
        nombre: texto("nombre", &nuevo.nombre, 100)?,
        tipo: nuevo.tipo,
        criterio: nuevo.criterio,
        modulo_id: nuevo.modulo,
        valor_fijo: nuevo.valor_fijo,
        tarifas: nuevo.tarifas,
        genera_interes: nuevo.genera_interes,
        concepto_contable: nuevo.concepto_contable.trim().to_owned(),
        activo: true,
        creado_en: reloj.ahora(),
    };
    guardar_concepto(&mut tx, &concepto).await?;
    tx.confirmar().await?;
    Ok(concepto)
}

/// Actualiza un concepto de cobro.
///
/// # Errors
///
/// Los de [`crear_concepto_cobro`] y [`AppError::NoEncontrado`].
pub async fn actualizar_concepto_cobro<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ConceptoCobroId,
    cambio: CambioConcepto,
) -> AppResult<ConceptoCobro>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraConfigurar).await?;
    let actual = tx
        .conceptos_cobro()
        .await?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or(AppError::NoEncontrado("concepto de cobro"))?;
    let concepto = ConceptoCobro {
        nombre: texto("nombre", &cambio.nombre, 100)?,
        valor_fijo: cambio.valor_fijo,
        tarifas: cambio.tarifas,
        genera_interes: cambio.genera_interes,
        concepto_contable: cambio.concepto_contable.trim().to_owned(),
        activo: cambio.activo,
        ..actual
    };
    guardar_concepto(&mut tx, &concepto).await?;
    tx.confirmar().await?;
    Ok(concepto)
}

// ---------------------------------------------------------------------------------------------
// Políticas de cartera
// ---------------------------------------------------------------------------------------------

/// Versiones de la política de cartera, la más reciente primero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn politicas_cartera<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<PoliticaCartera>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    abrir(unidad, reloj, ctx, Permiso::CarteraVer)
        .await?
        .politicas_cartera()
        .await
}

/// La versión vigente en la fecha (la de mayor `vigente_desde` hasta esa fecha).
///
/// # Errors
///
/// [`AppError::Validacion`] si no hay ninguna vigente.
pub(crate) async fn politica_en<T: TxCartera>(
    tx: &mut T,
    fecha: NaiveDate,
) -> AppResult<PoliticaCartera> {
    tx.politicas_cartera()
        .await?
        .into_iter()
        .filter(|p| p.vigente_desde <= fecha)
        .max_by_key(|p| p.vigente_desde)
        .ok_or_else(|| {
            validacion(format!(
                "no hay una política de cartera vigente el {fecha}: créela primero"
            ))
        })
}

/// Crea una versión nueva de la política de cartera, vigente desde la fecha dada.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.configurar`; [`AppError::Dominio`] si incumple un tope
/// legal (mora hasta 1,5 veces el interés bancario corriente, fondo de al menos 1 % salvo
/// suspensión); [`AppError::Conflicto`] si ya hay una versión con esa fecha.
pub async fn crear_politica_cartera<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    vigente_desde: NaiveDate,
    parametros: ParametrosPolitica,
    pregunta: Option<PreguntaId>,
) -> AppResult<PoliticaCartera>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    parametros.validar()?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraConfigurar).await?;
    let version = tx
        .politicas_cartera()
        .await?
        .iter()
        .map(|p| p.version)
        .max()
        .unwrap_or(0)
        + 1;
    let politica = PoliticaCartera {
        id: PoliticaCarteraId::nuevo(),
        version,
        vigente_desde,
        parametros,
        pregunta_id: pregunta,
        creada_por: ctx.actor_id,
        creada_en: reloj.ahora(),
    };
    tx.insertar_politica(&politica).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("version", politica.version)?
        .con("vigente_desde", politica.vigente_desde.to_string())?
        .con("factor_mora", politica.parametros.factor_mora.to_string())?
        .con(
            "porcentaje_fondo",
            politica.parametros.porcentaje_fondo.to_string(),
        )?
        .con("fondo_suspendido", politica.parametros.fondo_suspendido)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_POLITICA, "politica_cartera", politica.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(politica)
}

// ---------------------------------------------------------------------------------------------
// Datos de pago de los bienes privados
// ---------------------------------------------------------------------------------------------

/// Bienes privados con su referencia de pago, pagador y deudor (propietario representante).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.ver`.
pub async fn bienes_cartera<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<BienCartera>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    abrir(unidad, reloj, ctx, Permiso::CarteraVer)
        .await?
        .bienes_cartera()
        .await
}

/// Fija la referencia de pago (numérica, única en la copropiedad: la usan el recaudo bancario y
/// la pasarela) y el pagador del bien (tenedor, art. 29; vacío = el propietario representante).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `cartera.configurar`; [`AppError::Validacion`] si la referencia
/// no es numérica; [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si otro bien ya usa la
/// referencia.
pub async fn guardar_datos_pago<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    referencia: Option<&str>,
    pagador: Option<PersonaId>,
) -> AppResult<BienCartera>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCartera,
{
    let referencia = referencia.map(str::trim).filter(|r| !r.is_empty());
    if referencia.is_some_and(|r| r.len() > 24 || !r.chars().all(|c| c.is_ascii_digit())) {
        return Err(validacion("la referencia de pago tiene de 1 a 24 dígitos"));
    }
    let mut tx = abrir(unidad, reloj, ctx, Permiso::CarteraConfigurar).await?;
    if let Some(p) = pagador {
        tx.persona(p)
            .await?
            .ok_or(AppError::NoEncontrado("persona"))?;
    }
    if !tx.guardar_datos_pago(bien, referencia, pagador).await? {
        return Err(AppError::NoEncontrado("bien privado"));
    }
    let detalle = DetalleAuditoria::vacio()
        .con("con_referencia", referencia.is_some())?
        .con("con_pagador", pagador.is_some())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_DATOS_PAGO, "bien_privado", bien.as_uuid()).con_detalle(detalle),
    )
    .await?;
    let actualizado = tx
        .bienes_cartera()
        .await?
        .into_iter()
        .find(|b| b.id == bien)
        .ok_or(AppError::NoEncontrado("bien privado"))?;
    tx.confirmar().await?;
    Ok(actualizado)
}
