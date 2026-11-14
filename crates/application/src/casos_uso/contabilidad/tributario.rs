//! Tributario (plan 15 fase 1): catálogos de plataforma de IVA, ICA y formatos de exógena (UVT y
//! retenciones están en [`super::cuentas_por_pagar`]), perfil tributario versionado de la
//! entidad (borrador hasta que el contador lo confirma; sin confirmar no hay automatismos) y
//! resoluciones de numeración de la DIAN.

use chrono::NaiveDate;
use domain::{
    ArchivoId, FormatoExogenaId, PerfilTributarioId, Permiso, ResolucionNumeracionId, TarifaIcaId,
    TarifaIvaId,
    contabilidad::tributario::{
        EstadoPerfilTributario, LadoExogena, ParametrosPerfil, TipoNumeracion, TratamientoIva,
    },
};
use rust_decimal::Decimal;

use super::prelude::*;
use super::tesoreria::abrir;
use super::{TxTesoreria, texto, texto_opcional};
use crate::{
    AppError, AppResult, Contexto, ContextoPlataforma,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, Reloj, UnidadDeTrabajo, UnidadDeTrabajoPlataforma,
        tributario::{
            CampoExogena, CatalogoTributarioPlataforma, FormatoExogena, PerfilTributario,
            ResolucionNumeracion, TarifaIca, TarifaIva,
        },
    },
};

const ACCION_IVA: AccionAuditoria = AccionAuditoria::new("plataforma.tarifa_iva.crear");
const ACCION_ICA: AccionAuditoria = AccionAuditoria::new("plataforma.tarifa_ica.crear");
const ACCION_FORMATO: AccionAuditoria = AccionAuditoria::new("plataforma.formato_exogena.crear");
const ACCION_PERFIL: AccionAuditoria = AccionAuditoria::new("tributario.perfil.crear");
const ACCION_CONFIRMAR: AccionAuditoria = AccionAuditoria::new("tributario.perfil.confirmar");
const ACCION_RESOLUCION: AccionAuditoria = AccionAuditoria::new("tributario.resolucion.crear");
const ACCION_RESOLUCION_ESTADO: AccionAuditoria =
    AccionAuditoria::new("tributario.resolucion.estado");

fn validacion(texto: impl Into<String>) -> AppError {
    AppError::Validacion(texto.into())
}

fn es_codigo(texto: &str) -> bool {
    let mut c = texto.chars();
    (2..=60).contains(&texto.len())
        && c.next().is_some_and(|x| x.is_ascii_lowercase())
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '_')
}

fn digitos(texto: &str, n: usize) -> bool {
    texto.len() == n && texto.chars().all(|c| c.is_ascii_digit())
}

// ---------------------------------------------------------------------------------------------
// Catálogos de plataforma
// ---------------------------------------------------------------------------------------------

/// Versión nueva de una tarifa de IVA.
#[derive(Debug, Clone)]
pub struct NuevaTarifaIva {
    pub codigo: String,
    pub nombre: String,
    pub tratamiento: TratamientoIva,
    pub tarifa: Decimal,
    pub fuente: String,
    pub vigente_desde: NaiveDate,
}

/// Agrega una versión de una tarifa de IVA (la anterior del mismo código queda vigente hasta el
/// día antes).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`; [`AppError::Validacion`] si el
/// código o la tarifa no son coherentes con el tratamiento; [`AppError::Conflicto`] si ya hay una
/// versión con esa fecha.
pub async fn crear_tarifa_iva<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
    nueva: NuevaTarifaIva,
) -> AppResult<TarifaIva>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoTributarioPlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    let codigo = nueva.codigo.trim().to_owned();
    if !es_codigo(&codigo) {
        return Err(validacion(
            "el código usa minúsculas, dígitos y `_` y empieza por letra",
        ));
    }
    let gravado = nueva.tratamiento == TratamientoIva::Gravado;
    if nueva.tarifa < Decimal::ZERO
        || nueva.tarifa > Decimal::ONE_HUNDRED
        || gravado != (nueva.tarifa > Decimal::ZERO)
    {
        return Err(validacion(
            "solo una tarifa gravada lleva porcentaje (positivo y hasta 100)",
        ));
    }
    let tarifa = TarifaIva {
        id: TarifaIvaId::nuevo(),
        codigo,
        nombre: texto("nombre", &nueva.nombre, 200)?,
        tratamiento: nueva.tratamiento,
        tarifa: nueva.tarifa.round_dp(2),
        fuente: texto("fuente", &nueva.fuente, 300)?,
        vigente_desde: nueva.vigente_desde,
        vigente_hasta: None,
    };
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    tx.insertar_tarifa_iva(&tarifa).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("codigo", tarifa.codigo.as_str())?
        .con("tarifa", tarifa.tarifa.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_IVA, "tarifa_iva", tarifa.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(tarifa)
}

/// Versión nueva de una tarifa de ICA.
#[derive(Debug, Clone)]
pub struct NuevaTarifaIca {
    pub municipio_dane: String,
    pub actividad: String,
    pub nombre: String,
    pub tarifa_por_mil: Decimal,
    pub fuente: String,
    pub vigente_desde: NaiveDate,
}

/// Agrega una versión de una tarifa de ICA por municipio y actividad CIIU.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`; [`AppError::Validacion`];
/// [`AppError::Conflicto`] si ya hay una versión con esa fecha.
pub async fn crear_tarifa_ica<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
    nueva: NuevaTarifaIca,
) -> AppResult<TarifaIca>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoTributarioPlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    if !digitos(&nueva.municipio_dane, 5) || !digitos(&nueva.actividad, 4) {
        return Err(validacion(
            "el municipio es su código DANE (5 dígitos) y la actividad su CIIU (4 dígitos)",
        ));
    }
    if nueva.tarifa_por_mil <= Decimal::ZERO || nueva.tarifa_por_mil > Decimal::ONE_HUNDRED {
        return Err(validacion("la tarifa por mil es positiva y hasta 100"));
    }
    let tarifa = TarifaIca {
        id: TarifaIcaId::nuevo(),
        municipio_dane: nueva.municipio_dane,
        actividad: nueva.actividad,
        nombre: texto("nombre", &nueva.nombre, 200)?,
        tarifa_por_mil: nueva.tarifa_por_mil.round_dp(3),
        fuente: texto("fuente", &nueva.fuente, 300)?,
        vigente_desde: nueva.vigente_desde,
        vigente_hasta: None,
    };
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    tx.insertar_tarifa_ica(&tarifa).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("municipio", tarifa.municipio_dane.as_str())?
        .con("actividad", tarifa.actividad.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ICA, "tarifa_ica", tarifa.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(tarifa)
}

/// Formato nuevo de exógena.
#[derive(Debug, Clone)]
pub struct NuevoFormatoExogena {
    pub anio: i32,
    pub codigo: String,
    pub nombre: String,
    pub resolucion: String,
    pub cuantia_menor: Decimal,
    pub campos: Vec<CampoExogena>,
}

/// Agrega una versión de un formato de exógena (la siguiente del año y código) con su mapeo de
/// cuentas a conceptos y columnas.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`; [`AppError::Validacion`] si un
/// campo no es válido o no hay campos.
pub async fn crear_formato_exogena<UP>(
    plataforma: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    nuevo: NuevoFormatoExogena,
) -> AppResult<FormatoExogena>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoTributarioPlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    if !(2000..=2999).contains(&nuevo.anio) || !digitos(&nuevo.codigo, 4) {
        return Err(validacion(
            "año válido y código de formato de cuatro dígitos",
        ));
    }
    if nuevo.campos.is_empty() || nuevo.cuantia_menor < Decimal::ZERO {
        return Err(validacion(
            "el formato necesita campos y una cuantía menor no negativa",
        ));
    }
    for c in &nuevo.campos {
        let prefijo_ok = (1..=10).contains(&c.cuenta_prefijo.len())
            && c.cuenta_prefijo.chars().all(|x| x.is_ascii_digit())
            && !c.cuenta_prefijo.starts_with('0');
        if !digitos(&c.concepto, 4) || !prefijo_ok || !es_codigo(&c.columna) {
            return Err(validacion(
                "cada campo lleva un concepto de cuatro dígitos, un prefijo de cuenta y una \
                 columna en minúsculas",
            ));
        }
    }
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    let version = tx
        .formatos_exogena(Some(nuevo.anio))
        .await?
        .iter()
        .filter(|f| f.codigo == nuevo.codigo)
        .map(|f| f.version)
        .max()
        .unwrap_or(0)
        + 1;
    let formato = FormatoExogena {
        id: FormatoExogenaId::nuevo(),
        anio: nuevo.anio,
        codigo: nuevo.codigo,
        version,
        nombre: texto("nombre", &nuevo.nombre, 200)?,
        resolucion: texto("resolucion", &nuevo.resolucion, 200)?,
        cuantia_menor: nuevo.cuantia_menor,
        campos: nuevo.campos,
        creado_en: reloj.ahora(),
    };
    tx.insertar_formato_exogena(&formato).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("anio", formato.anio)?
        .con("codigo", formato.codigo.as_str())?
        .con("version", formato.version)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_FORMATO, "formato_exogena", formato.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(formato)
}

/// Catálogo tributario completo (todas las versiones) visto desde la plataforma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogoCompleto {
    pub iva: Vec<TarifaIva>,
    pub ica: Vec<TarifaIca>,
    pub formatos: Vec<FormatoExogena>,
}

/// Catálogo tributario completo de la plataforma.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.normativa.gestionar`.
pub async fn catalogo_tributario_plataforma<UP>(
    plataforma: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<CatalogoCompleto>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: CatalogoTributarioPlataforma,
{
    ctx.exigir(Permiso::PlataformaNormativaGestionar)?;
    let mut tx = plataforma.iniciar_plataforma(ctx).await?;
    Ok(CatalogoCompleto {
        iva: tx.tarifas_iva(None).await?,
        ica: tx.tarifas_ica(None).await?,
        formatos: tx.formatos_exogena(None).await?,
    })
}

/// Catálogo vigente en la fecha (lo que la entidad puede usar).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn catalogo_tributario<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    fecha: Option<NaiveDate>,
) -> AppResult<CatalogoCompleto>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let fecha = fecha.unwrap_or_else(|| reloj.ahora().date_naive());
    let mut tx = abrir(unidad, reloj, ctx, Permiso::ContabilidadVer).await?;
    Ok(CatalogoCompleto {
        iva: tx.tarifas_iva(Some(fecha)).await?,
        ica: tx.tarifas_ica(Some(fecha)).await?,
        formatos: tx.formatos_exogena(None).await?,
    })
}

// ---------------------------------------------------------------------------------------------
// Perfil tributario
// ---------------------------------------------------------------------------------------------

/// El perfil confirmado vigente en la fecha (la versión confirmada de mayor `vigente_desde`
/// hasta esa fecha). `None` = no hay automatismos tributarios.
pub(crate) async fn perfil_confirmado_en<T: TxTesoreria>(
    tx: &mut T,
    fecha: NaiveDate,
) -> AppResult<Option<PerfilTributario>> {
    Ok(tx
        .perfiles_tributarios()
        .await?
        .into_iter()
        .filter(|p| p.estado == EstadoPerfilTributario::Confirmado && p.vigente_desde <= fecha)
        .max_by_key(|p| (p.vigente_desde, p.version)))
}

/// Tarifas gravadas de IVA vigentes en la fecha (normalizadas para compararlas).
pub(crate) async fn tarifas_gravadas<T: TxTesoreria>(
    tx: &mut T,
    fecha: NaiveDate,
) -> AppResult<Vec<Decimal>> {
    Ok(tx
        .tarifas_iva(Some(fecha))
        .await?
        .into_iter()
        .filter(|t| t.tratamiento == TratamientoIva::Gravado)
        .map(|t| t.tarifa.normalize())
        .collect())
}

/// Versiones del perfil tributario, la más reciente primero.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn perfiles_tributarios<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<PerfilTributario>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .perfiles_tributarios()
        .await
}

/// Crea una versión del perfil tributario en **borrador** (no tiene efecto hasta que el
/// contador la confirme).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tributario.configurar`; [`AppError::Dominio`] si el perfil no es
/// coherente con el RUT.
pub async fn crear_perfil_tributario<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    vigente_desde: NaiveDate,
    parametros: ParametrosPerfil,
    soporte: Option<ArchivoId>,
    observaciones: Option<&str>,
) -> AppResult<PerfilTributario>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    parametros.validar()?;
    let observaciones = texto_opcional("observaciones", observaciones, 500)?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TributarioConfigurar).await?;
    let version = tx
        .perfiles_tributarios()
        .await?
        .iter()
        .map(|p| p.version)
        .max()
        .unwrap_or(0)
        + 1;
    let perfil = PerfilTributario {
        id: PerfilTributarioId::nuevo(),
        version,
        vigente_desde,
        estado: EstadoPerfilTributario::Borrador,
        parametros,
        soporte_archivo_id: soporte,
        observaciones,
        creado_por: ctx.actor_id,
        creado_en: reloj.ahora(),
        confirmado_por: None,
        confirmado_en: None,
    };
    tx.insertar_perfil_tributario(&perfil).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("version", perfil.version)?
        .con("vigente_desde", perfil.vigente_desde.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_PERFIL, "perfil_tributario", perfil.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(perfil)
}

/// El contador confirma una versión del perfil: desde su vigencia rigen los automatismos
/// tributarios (retenciones, IVA, ICA, exógena). Lo confirmado no se modifica.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tributario.perfil.confirmar` o sin cuenta;
/// [`AppError::NoEncontrado`]; [`AppError::Dominio`] si ya está confirmada.
pub async fn confirmar_perfil_tributario<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: PerfilTributarioId,
) -> AppResult<PerfilTributario>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let cuenta = ctx
        .actor_id
        .ok_or(AppError::SinPermiso(Permiso::TributarioPerfilConfirmar))?;
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TributarioPerfilConfirmar).await?;
    let mut perfil = tx
        .perfiles_tributarios()
        .await?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or(AppError::NoEncontrado("perfil tributario"))?;
    perfil.estado = perfil
        .estado
        .transicionar(EstadoPerfilTributario::Confirmado)?;
    perfil.confirmado_por = Some(cuenta);
    perfil.confirmado_en = Some(reloj.ahora());
    tx.confirmar_perfil_tributario(&perfil).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_CONFIRMAR,
        "perfil_tributario",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(perfil)
}

// ---------------------------------------------------------------------------------------------
// Resoluciones de numeración
// ---------------------------------------------------------------------------------------------

/// Resolución de numeración nueva.
#[derive(Debug, Clone)]
pub struct NuevaResolucion {
    pub tipo: TipoNumeracion,
    pub prefijo: String,
    pub desde: u64,
    pub hasta: u64,
    pub numero_resolucion: String,
    pub fecha_resolucion: NaiveDate,
    pub vigente_desde: NaiveDate,
    pub vigente_hasta: NaiveDate,
    pub clave_tecnica: Option<String>,
}

/// Resoluciones de numeración.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `contabilidad.ver`.
pub async fn resoluciones<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<ResolucionNumeracion>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    abrir(unidad, reloj, ctx, Permiso::ContabilidadVer)
        .await?
        .resoluciones()
        .await
}

/// Registra una resolución de numeración de la DIAN.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tributario.configurar`; [`AppError::Validacion`] si el prefijo,
/// el rango o las fechas no son válidos; [`AppError::Conflicto`] si el rango ya existe.
pub async fn crear_resolucion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    nueva: NuevaResolucion,
) -> AppResult<ResolucionNumeracion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let prefijo = nueva.prefijo.trim().to_uppercase();
    if prefijo.len() > 4 || !prefijo.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(validacion(
            "el prefijo tiene a lo sumo cuatro letras o dígitos",
        ));
    }
    if nueva.desde == 0 || nueva.hasta < nueva.desde || nueva.vigente_hasta < nueva.vigente_desde {
        return Err(validacion("el rango o la vigencia no son válidos"));
    }
    let resolucion = ResolucionNumeracion {
        id: ResolucionNumeracionId::nuevo(),
        tipo: nueva.tipo,
        prefijo,
        desde: nueva.desde,
        hasta: nueva.hasta,
        numero_resolucion: texto("numero_resolucion", &nueva.numero_resolucion, 40)?,
        fecha_resolucion: nueva.fecha_resolucion,
        vigente_desde: nueva.vigente_desde,
        vigente_hasta: nueva.vigente_hasta,
        clave_tecnica: texto_opcional("clave_tecnica", nueva.clave_tecnica.as_deref(), 200)?,
        activa: true,
        creada_en: reloj.ahora(),
    };
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TributarioConfigurar).await?;
    tx.insertar_resolucion(&resolucion).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", resolucion.tipo.as_str())?
        .con("prefijo", resolucion.prefijo.as_str())?
        .con("desde", resolucion.desde)?
        .con("hasta", resolucion.hasta)?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_RESOLUCION,
            "resolucion_numeracion",
            resolucion.id.as_uuid(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(resolucion)
}

/// Activa o desactiva una resolución (una desactivada no numera).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `tributario.configurar`; [`AppError::NoEncontrado`].
pub async fn activar_resolucion<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ResolucionNumeracionId,
    activa: bool,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxTesoreria,
{
    let mut tx = abrir(unidad, reloj, ctx, Permiso::TributarioConfigurar).await?;
    if !tx.activar_resolucion(id, activa).await? {
        return Err(AppError::NoEncontrado("resolución de numeración"));
    }
    let detalle = DetalleAuditoria::vacio().con("activa", activa)?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_RESOLUCION_ESTADO,
            "resolucion_numeracion",
            id.as_uuid(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await
}

/// Lado de un campo de exógena desde su texto (para la API).
///
/// # Errors
///
/// [`AppError::Dominio`] si no es `debito`, `credito` o `neto`.
pub fn lado_exogena(texto: &str) -> AppResult<LadoExogena> {
    Ok(LadoExogena::try_from(texto)?)
}
