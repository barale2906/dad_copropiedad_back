//! Agrupaciones, bienes privados, anexos y usos exclusivos.
//!
//! - El código de un bien privado es único por copropiedad: el que escribe la PH o el que
//!   calcula su [`PlantillaNomenclatura`] con la cadena de agrupaciones y el número.
//! - El coeficiente **no** se edita aquí: lo fija la activación de un cuadro
//!   ([`crate::casos_uso::coeficientes`]).
//! - Parqueaderos y depósitos: si tienen folio propio son bienes privados (con coeficiente) y
//!   pueden vincularse como anexos de otro; si están englobados con el apartamento no se
//!   registran; si son comunes asignados, son **usos exclusivos** (art. 22, sin coeficiente).

use chrono::{DateTime, Utc};
use domain::{
    AgrupacionId, BienPrivadoId, CodigoAgrupacion, CodigoBienPrivado, NumeroBienPrivado, Permiso,
    PlantillaNomenclatura, TipoAgrupacion, TipoBienPrivado, TipoUsoExclusivo, UsoBienPrivado,
    UsoExclusivoId,
    bien_privado::PROFUNDIDAD_MAXIMA_AGRUPACIONES,
    copropiedad::{texto_obligatorio, texto_opcional},
};
use rust_decimal::Decimal;

use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioBienesPrivados, RepositorioCopropiedad,
        RepositorioTitulares, Transaccion, UnidadDeTrabajo,
        copropiedades::{Agrupacion, BienPrivado, FiltroBienes, Titularidad, UsoExclusivo},
    },
};

pub const ACCION_CREAR_AGRUPACION: AccionAuditoria =
    AccionAuditoria::new("bienes_privados.agrupacion.crear");
pub const ACCION_CREAR: AccionAuditoria = AccionAuditoria::new("bienes_privados.bien.crear");
pub const ACCION_EDITAR: AccionAuditoria = AccionAuditoria::new("bienes_privados.bien.editar");
pub const ACCION_INACTIVAR: AccionAuditoria =
    AccionAuditoria::new("bienes_privados.bien.inactivar");
pub const ACCION_VINCULAR: AccionAuditoria = AccionAuditoria::new("bienes_privados.bien.vincular");
pub const ACCION_USO_EXCLUSIVO: AccionAuditoria =
    AccionAuditoria::new("bienes_privados.uso_exclusivo.asignar");
pub const ACCION_TERMINAR_USO: AccionAuditoria =
    AccionAuditoria::new("bienes_privados.uso_exclusivo.terminar");

/// Cadena de agrupaciones desde la raíz hasta `id` (incluida).
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si `id` (o algún ancestro) no existe.
pub fn ruta_de(agrupaciones: &[Agrupacion], id: AgrupacionId) -> AppResult<Vec<&Agrupacion>> {
    let mut ruta = Vec::new();
    let mut actual = Some(id);
    while let Some(a) = actual {
        let nodo = agrupaciones
            .iter()
            .find(|x| x.id == a)
            .ok_or(AppError::NoEncontrado("agrupación"))?;
        if ruta.len() > PROFUNDIDAD_MAXIMA_AGRUPACIONES {
            return Err(AppError::infra(std::io::Error::other(
                "ciclo en el árbol de agrupaciones",
            )));
        }
        ruta.push(nodo);
        actual = nodo.padre_id;
    }
    ruta.reverse();
    Ok(ruta)
}

/// Código de un bien privado: el propio si viene; si no, el de la plantilla.
///
/// # Errors
///
/// [`AppError::Validacion`] si no hay código ni número; [`AppError::NoEncontrado`] si la
/// agrupación no existe.
pub fn calcular_codigo(
    plantilla: &PlantillaNomenclatura,
    agrupaciones: &[Agrupacion],
    agrupacion: Option<AgrupacionId>,
    numero: Option<&NumeroBienPrivado>,
    propio: Option<CodigoBienPrivado>,
) -> AppResult<CodigoBienPrivado> {
    if let Some(c) = propio {
        return Ok(c);
    }
    let numero = numero.ok_or_else(|| {
        AppError::Validacion("indique el número del bien privado o su código".into())
    })?;
    let ruta: Vec<CodigoAgrupacion> = match agrupacion {
        Some(id) => ruta_de(agrupaciones, id)?
            .into_iter()
            .map(|a| a.codigo.clone())
            .collect(),
        None => Vec::new(),
    };
    Ok(plantilla.codigo(&ruta, numero)?)
}

async fn plantilla<T: RepositorioCopropiedad>(tx: &mut T) -> AppResult<PlantillaNomenclatura> {
    Ok(tx
        .ficha()
        .await?
        .ok_or(AppError::NoEncontrado("copropiedad"))?
        .plantilla_codigo)
}

// ---------------------------------------------------------------------------------------------
// Agrupaciones
// ---------------------------------------------------------------------------------------------

/// Datos de una agrupación nueva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevaAgrupacion {
    pub tipo: TipoAgrupacion,
    pub nombre: String,
    pub codigo: CodigoAgrupacion,
    pub padre_id: Option<AgrupacionId>,
    pub orden: i32,
}

/// Crea una agrupación (edificio, etapa…), raíz o hija de otra (máximo 3 niveles).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `agrupaciones.editar`; [`AppError::NoEncontrado`] si el padre no
/// existe; [`AppError::Validacion`] si supera la profundidad; [`AppError::Conflicto`] si el
/// código se repite bajo el mismo padre.
pub async fn crear_agrupacion<U>(
    unidad: &U,
    ctx: &Contexto,
    nueva: NuevaAgrupacion,
) -> AppResult<Agrupacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioBienesPrivados + RegistroAuditoria,
{
    ctx.exigir(Permiso::AgrupacionesEditar)?;
    let agrupacion = Agrupacion {
        id: AgrupacionId::nuevo(),
        tipo: nueva.tipo,
        nombre: texto_obligatorio("nombre", &nueva.nombre, 80)?,
        codigo: nueva.codigo,
        padre_id: nueva.padre_id,
        orden: nueva.orden,
    };
    let mut tx = unidad.iniciar(ctx).await?;
    if let Some(padre) = agrupacion.padre_id {
        let todas = tx.agrupaciones().await?;
        if ruta_de(&todas, padre)?.len() >= PROFUNDIDAD_MAXIMA_AGRUPACIONES {
            return Err(AppError::Validacion(format!(
                "las agrupaciones admiten como máximo {PROFUNDIDAD_MAXIMA_AGRUPACIONES} niveles"
            )));
        }
    }
    tx.insertar_agrupacion(&agrupacion).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_CREAR_AGRUPACION,
        "agrupacion",
        agrupacion.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(agrupacion)
}

/// Nodo del árbol de agrupaciones con sus bienes privados directos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodoAgrupacion {
    pub agrupacion: Agrupacion,
    pub bienes: Vec<BienPrivado>,
    pub hijas: Vec<NodoAgrupacion>,
}

/// Árbol de agrupaciones con los bienes privados activos de cada una, y los bienes sin
/// agrupación (casas, lotes…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArbolAgrupaciones {
    pub raices: Vec<NodoAgrupacion>,
    pub sin_agrupacion: Vec<BienPrivado>,
}

fn armar_nodo(a: &Agrupacion, todas: &[Agrupacion], bienes: &[BienPrivado]) -> NodoAgrupacion {
    NodoAgrupacion {
        agrupacion: a.clone(),
        bienes: bienes
            .iter()
            .filter(|b| b.agrupacion_id == Some(a.id))
            .cloned()
            .collect(),
        hijas: todas
            .iter()
            .filter(|h| h.padre_id == Some(a.id))
            .map(|h| armar_nodo(h, todas, bienes))
            .collect(),
    }
}

/// Árbol de agrupaciones con bienes privados.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_privados.ver`.
pub async fn arbol_agrupaciones<U>(unidad: &U, ctx: &Contexto) -> AppResult<ArbolAgrupaciones>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioBienesPrivados,
{
    ctx.exigir(Permiso::BienesPrivadosVer)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let todas = tx.agrupaciones().await?;
    let bienes: Vec<BienPrivado> = tx
        .todos_los_bienes()
        .await?
        .into_iter()
        .filter(|b| b.activo)
        .collect();
    Ok(ArbolAgrupaciones {
        raices: todas
            .iter()
            .filter(|a| a.padre_id.is_none())
            .map(|a| armar_nodo(a, &todas, &bienes))
            .collect(),
        sin_agrupacion: bienes
            .iter()
            .filter(|b| b.agrupacion_id.is_none())
            .cloned()
            .collect(),
    })
}

// ---------------------------------------------------------------------------------------------
// Bienes privados
// ---------------------------------------------------------------------------------------------

/// Datos de un bien privado nuevo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoBienPrivado {
    pub tipo: TipoBienPrivado,
    pub agrupacion_id: Option<AgrupacionId>,
    pub numero: Option<NumeroBienPrivado>,
    /// Código propio; si no viene, se calcula con la plantilla.
    pub codigo: Option<CodigoBienPrivado>,
    pub piso: Option<i16>,
    pub area_privada_m2: Option<Decimal>,
    pub matricula_inmobiliaria: Option<String>,
    pub cedula_catastral: Option<String>,
    pub uso: UsoBienPrivado,
}

fn validar_area(area: Option<Decimal>) -> AppResult<Option<Decimal>> {
    match area {
        Some(a) if a <= Decimal::ZERO || a.normalize().scale() > 2 => Err(AppError::Validacion(
            "`area_privada_m2` debe ser positiva con máximo 2 decimales".into(),
        )),
        otra => Ok(otra),
    }
}

fn validar_piso(piso: Option<i16>) -> AppResult<Option<i16>> {
    match piso {
        Some(p) if !(-10..=200).contains(&p) => Err(AppError::Validacion(
            "`piso` debe estar entre -10 y 200".into(),
        )),
        otro => Ok(otro),
    }
}

/// Crea un bien privado (sin coeficiente hasta que se active un cuadro que lo incluya).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_privados.editar`; [`AppError::Validacion`];
/// [`AppError::NoEncontrado`] si la agrupación no existe; [`AppError::Conflicto`] si el código
/// ya existe.
pub async fn crear_bien_privado<U>(
    unidad: &U,
    ctx: &Contexto,
    nuevo: NuevoBienPrivado,
) -> AppResult<BienPrivado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioBienesPrivados + RepositorioCopropiedad + RegistroAuditoria,
{
    ctx.exigir(Permiso::BienesPrivadosEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let plantilla = plantilla(&mut tx).await?;
    let agrupaciones = tx.agrupaciones().await?;
    let bien = BienPrivado {
        id: BienPrivadoId::nuevo(),
        codigo: calcular_codigo(
            &plantilla,
            &agrupaciones,
            nuevo.agrupacion_id,
            nuevo.numero.as_ref(),
            nuevo.codigo,
        )?,
        tipo: nuevo.tipo,
        agrupacion_id: nuevo.agrupacion_id,
        numero: nuevo.numero,
        piso: validar_piso(nuevo.piso)?,
        area_privada_m2: validar_area(nuevo.area_privada_m2)?,
        matricula_inmobiliaria: texto_opcional(
            "matricula_inmobiliaria",
            nuevo.matricula_inmobiliaria.as_deref(),
            40,
        )?,
        cedula_catastral: texto_opcional(
            "cedula_catastral",
            nuevo.cedula_catastral.as_deref(),
            40,
        )?,
        uso: nuevo.uso,
        activo: true,
        bien_principal_id: None,
        coeficiente: None,
    };
    tx.insertar_bien(&bien).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("codigo", bien.codigo.as_str())?
        .con("tipo", bien.tipo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR, "bien_privado", bien.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(bien)
}

/// Cambios a un bien privado. `None` = sin cambio; en los textos, `Some("")` borra.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CambiosBienPrivado {
    pub tipo: Option<TipoBienPrivado>,
    pub agrupacion_id: Option<AgrupacionId>,
    pub numero: Option<NumeroBienPrivado>,
    /// Código propio. Si no viene y cambian el número o la agrupación, se recalcula.
    pub codigo: Option<CodigoBienPrivado>,
    pub piso: Option<i16>,
    pub area_privada_m2: Option<Decimal>,
    pub matricula_inmobiliaria: Option<String>,
    pub cedula_catastral: Option<String>,
    pub uso: Option<UsoBienPrivado>,
}

/// Edita los datos de un bien privado (nunca su coeficiente).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_privados.editar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`]; [`AppError::Conflicto`] si el código nuevo ya existe.
pub async fn editar_bien_privado<U>(
    unidad: &U,
    ctx: &Contexto,
    id: BienPrivadoId,
    c: CambiosBienPrivado,
) -> AppResult<BienPrivado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioBienesPrivados + RepositorioCopropiedad + RegistroAuditoria,
{
    ctx.exigir(Permiso::BienesPrivadosEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut bien = tx
        .bien(id)
        .await?
        .ok_or(AppError::NoEncontrado("bien privado"))?;
    let recalcular = c.codigo.is_none() && (c.numero.is_some() || c.agrupacion_id.is_some());
    if let Some(v) = c.tipo {
        bien.tipo = v;
    }
    if let Some(v) = c.agrupacion_id {
        bien.agrupacion_id = Some(v);
    }
    if let Some(v) = c.numero {
        bien.numero = Some(v);
    }
    if let Some(v) = c.piso {
        bien.piso = validar_piso(Some(v))?;
    }
    if let Some(v) = c.area_privada_m2 {
        bien.area_privada_m2 = validar_area(Some(v))?;
    }
    if let Some(v) = c.matricula_inmobiliaria {
        bien.matricula_inmobiliaria = texto_opcional("matricula_inmobiliaria", Some(&v), 40)?;
    }
    if let Some(v) = c.cedula_catastral {
        bien.cedula_catastral = texto_opcional("cedula_catastral", Some(&v), 40)?;
    }
    if let Some(v) = c.uso {
        bien.uso = v;
    }
    if let Some(codigo) = c.codigo {
        bien.codigo = codigo;
    } else if recalcular {
        let plantilla = plantilla(&mut tx).await?;
        let agrupaciones = tx.agrupaciones().await?;
        bien.codigo = calcular_codigo(
            &plantilla,
            &agrupaciones,
            bien.agrupacion_id,
            bien.numero.as_ref(),
            None,
        )?;
    }
    tx.actualizar_bien(&bien).await?;
    let detalle = DetalleAuditoria::vacio().con("codigo", bien.codigo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EDITAR, "bien_privado", bien.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(bien)
}

/// Inactiva un bien privado (p. ej. englobado con otro por reforma). Conserva su historia; el
/// próximo cuadro de coeficientes ya no lo incluye. Sus anexos quedan desvinculados.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_privados.editar`; [`AppError::NoEncontrado`].
pub async fn inactivar_bien_privado<U>(
    unidad: &U,
    ctx: &Contexto,
    id: BienPrivadoId,
) -> AppResult<BienPrivado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioBienesPrivados + RegistroAuditoria,
{
    ctx.exigir(Permiso::BienesPrivadosEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut bien = tx
        .bien(id)
        .await?
        .ok_or(AppError::NoEncontrado("bien privado"))?;
    if !bien.activo {
        return Ok(bien);
    }
    for mut anexo in tx.anexos(id).await? {
        anexo.bien_principal_id = None;
        tx.actualizar_bien(&anexo).await?;
    }
    bien.activo = false;
    bien.bien_principal_id = None;
    tx.actualizar_bien(&bien).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_INACTIVAR,
        "bien_privado",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(bien)
}

/// Vincula un parqueadero, depósito o bodega con folio propio a su bien principal (o lo
/// desvincula con `None`). El anexo conserva su coeficiente propio.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `bienes_privados.editar`; [`AppError::NoEncontrado`].
/// - [`AppError::Validacion`] si el anexo no es parqueadero/depósito/bodega, si el principal es
///   él mismo, está inactivo o es a su vez un anexo.
pub async fn vincular_bienes_privados<U>(
    unidad: &U,
    ctx: &Contexto,
    anexo: BienPrivadoId,
    principal: Option<BienPrivadoId>,
) -> AppResult<BienPrivado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioBienesPrivados + RegistroAuditoria,
{
    ctx.exigir(Permiso::BienesPrivadosEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let mut bien = tx
        .bien(anexo)
        .await?
        .ok_or(AppError::NoEncontrado("bien privado"))?;
    if let Some(p) = principal {
        if !bien.tipo.puede_ser_anexo() {
            return Err(AppError::Validacion(
                "solo un parqueadero, depósito o bodega se vincula como anexo".into(),
            ));
        }
        if p == anexo {
            return Err(AppError::Validacion(
                "un bien privado no es anexo de sí mismo".into(),
            ));
        }
        let destino = tx
            .bien(p)
            .await?
            .ok_or(AppError::NoEncontrado("bien privado principal"))?;
        if !destino.activo || !bien.activo || destino.bien_principal_id.is_some() {
            return Err(AppError::Validacion(
                "ambos bienes deben estar activos y el principal no puede ser a su vez un anexo"
                    .into(),
            ));
        }
        if !tx.anexos(anexo).await?.is_empty() {
            return Err(AppError::Validacion(
                "el bien tiene sus propios anexos: no puede ser anexo de otro".into(),
            ));
        }
    }
    bien.bien_principal_id = principal;
    tx.actualizar_bien(&bien).await?;
    let mut detalle = DetalleAuditoria::vacio();
    if let Some(p) = principal {
        detalle = detalle.con("bien_principal_id", p.to_string())?;
    }
    tx.registrar(
        EventoAuditoria::ok(ACCION_VINCULAR, "bien_privado", anexo.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(bien)
}

/// Listado paginado de bienes privados.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_privados.ver`.
pub async fn listar_bienes_privados<U>(
    unidad: &U,
    ctx: &Contexto,
    filtro: FiltroBienes,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<BienPrivado>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioBienesPrivados,
{
    ctx.exigir(Permiso::BienesPrivadosVer)?;
    unidad
        .iniciar(ctx)
        .await?
        .listar_bienes(&filtro, pagina)
        .await
}

/// Ficha de un bien privado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FichaBienPrivado {
    pub bien: BienPrivado,
    pub anexos: Vec<BienPrivado>,
    pub usos_exclusivos: Vec<UsoExclusivo>,
    /// Titulares vigentes; vacío si quien consulta no tiene `titulares.ver` (ni es titular).
    pub titulares: Vec<Titularidad>,
}

/// Ficha de un bien privado: datos, coeficiente vigente, anexos, usos exclusivos y titulares.
/// Un titular vigente la ve aunque no tenga `bienes_privados.ver`.
///
/// # Errors
///
/// [`AppError::SinPermiso`] si no tiene `bienes_privados.ver` ni es titular;
/// [`AppError::NoEncontrado`].
pub async fn ficha_bien_privado<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: BienPrivadoId,
) -> AppResult<FichaBienPrivado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioBienesPrivados + RepositorioTitulares,
{
    let mut tx = unidad.iniciar(ctx).await?;
    let es_titular = match ctx.actor_id {
        Some(c) => tx.bienes_de_cuenta(c).await?.contains(&id),
        None => false,
    };
    if !es_titular {
        ctx.exigir(Permiso::BienesPrivadosVer)?;
    }
    let bien = tx
        .bien(id)
        .await?
        .ok_or(AppError::NoEncontrado("bien privado"))?;
    let ahora = reloj.ahora();
    let titulares = if es_titular || ctx.exigir(Permiso::TitularesVer).is_ok() {
        tx.titulares_vigentes(id).await?
    } else {
        Vec::new()
    };
    Ok(FichaBienPrivado {
        anexos: tx.anexos(id).await?,
        usos_exclusivos: tx
            .usos_exclusivos(id)
            .await?
            .into_iter()
            .filter(|u| u.vigente_hasta.is_none_or(|h| h > ahora))
            .collect(),
        titulares,
        bien,
    })
}

/// Bienes privados de los que la cuenta del contexto es titular vigente.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] sin cuenta.
pub async fn mis_bienes_privados<U>(unidad: &U, ctx: &Contexto) -> AppResult<Vec<BienPrivado>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioBienesPrivados + RepositorioTitulares,
{
    let cuenta = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let ids = tx.bienes_de_cuenta(cuenta).await?;
    let mut bienes = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(b) = tx.bien(id).await? {
            bienes.push(b);
        }
    }
    Ok(bienes)
}

// ---------------------------------------------------------------------------------------------
// Usos exclusivos
// ---------------------------------------------------------------------------------------------

/// Datos de una asignación de uso exclusivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevoUsoExclusivo {
    pub tipo: TipoUsoExclusivo,
    pub codigo: String,
    pub descripcion: Option<String>,
    /// `None` = desde ahora.
    pub vigente_desde: Option<DateTime<Utc>>,
}

/// Asigna un bien común en uso exclusivo a un bien privado (art. 22).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_privados.editar`; [`AppError::NoEncontrado`];
/// [`AppError::Conflicto`] si ese bien común ya está asignado a otro.
pub async fn asignar_uso_exclusivo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    nuevo: NuevoUsoExclusivo,
) -> AppResult<UsoExclusivo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioBienesPrivados + RegistroAuditoria,
{
    ctx.exigir(Permiso::BienesPrivadosEditar)?;
    let uso = UsoExclusivo {
        id: UsoExclusivoId::nuevo(),
        bien_privado_id: bien,
        tipo: nuevo.tipo,
        codigo: texto_obligatorio("codigo", &nuevo.codigo, 40)?,
        descripcion: texto_opcional("descripcion", nuevo.descripcion.as_deref(), 200)?,
        vigente_desde: nuevo.vigente_desde.unwrap_or_else(|| reloj.ahora()),
        vigente_hasta: None,
    };
    let mut tx = unidad.iniciar(ctx).await?;
    tx.bien(bien)
        .await?
        .filter(|b| b.activo)
        .ok_or(AppError::NoEncontrado("bien privado"))?;
    tx.insertar_uso_exclusivo(&uso).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("tipo", uso.tipo.as_str())?
        .con("bien_privado_id", bien.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_USO_EXCLUSIVO, "uso_exclusivo", uso.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(uso)
}

/// Termina una asignación de uso exclusivo desde ahora.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `bienes_privados.editar`; [`AppError::NoEncontrado`].
pub async fn terminar_uso_exclusivo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    bien: BienPrivadoId,
    id: UsoExclusivoId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioBienesPrivados + RegistroAuditoria,
{
    ctx.exigir(Permiso::BienesPrivadosEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    if !tx.terminar_uso_exclusivo(bien, id, reloj.ahora()).await? {
        return Err(AppError::NoEncontrado("uso exclusivo"));
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_TERMINAR_USO,
        "uso_exclusivo",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::puertos::unidad_trabajo::memoria::UnidadEnMemoria;

    fn agrupacion(codigo: &str, padre: Option<AgrupacionId>) -> NuevaAgrupacion {
        NuevaAgrupacion {
            tipo: TipoAgrupacion::Edificio,
            nombre: format!("Torre {codigo}"),
            codigo: CodigoAgrupacion::new(codigo).expect("válido"),
            padre_id: padre,
            orden: 0,
        }
    }

    pub(crate) fn nuevo_bien(numero: &str, agrupacion: Option<AgrupacionId>) -> NuevoBienPrivado {
        NuevoBienPrivado {
            tipo: TipoBienPrivado::Apartamento,
            agrupacion_id: agrupacion,
            numero: Some(NumeroBienPrivado::new(numero).expect("válido")),
            codigo: None,
            piso: None,
            area_privada_m2: None,
            matricula_inmobiliaria: None,
            cedula_catastral: None,
            uso: UsoBienPrivado::Residencial,
        }
    }

    fn ctx() -> Contexto {
        Contexto::new(domain::TenantId::nuevo())
    }

    #[tokio::test]
    async fn codigo_con_plantilla_y_codigo_personalizado() {
        let unidad = UnidadEnMemoria::default();
        let ctx = ctx();
        let torre = crear_agrupacion(&unidad, &ctx, agrupacion("T3", None))
            .await
            .expect("crea");
        let b = crear_bien_privado(&unidad, &ctx, nuevo_bien("506", Some(torre.id)))
            .await
            .expect("crea");
        assert_eq!(b.codigo.as_str(), "T3-506");
        assert_eq!(b.coeficiente, None);

        let propio = NuevoBienPrivado {
            codigo: Some(CodigoBienPrivado::new("3506").expect("válido")),
            ..nuevo_bien("506", Some(torre.id))
        };
        let b = crear_bien_privado(&unidad, &ctx, propio)
            .await
            .expect("crea");
        assert_eq!(b.codigo.as_str(), "3506");
    }

    #[tokio::test]
    async fn codigo_repetido_es_conflicto() {
        let unidad = UnidadEnMemoria::default();
        let ctx = ctx();
        crear_bien_privado(&unidad, &ctx, nuevo_bien("1", None))
            .await
            .expect("crea");
        assert!(matches!(
            crear_bien_privado(&unidad, &ctx, nuevo_bien("1", None)).await,
            Err(AppError::Conflicto(_))
        ));
    }

    #[tokio::test]
    async fn arbol_admite_tres_niveles() {
        let unidad = UnidadEnMemoria::default();
        let ctx = ctx();
        let e = crear_agrupacion(&unidad, &ctx, agrupacion("E1", None))
            .await
            .expect("1");
        let t = crear_agrupacion(&unidad, &ctx, agrupacion("T1", Some(e.id)))
            .await
            .expect("2");
        let i = crear_agrupacion(&unidad, &ctx, agrupacion("I1", Some(t.id)))
            .await
            .expect("3");
        assert!(matches!(
            crear_agrupacion(&unidad, &ctx, agrupacion("X", Some(i.id))).await,
            Err(AppError::Validacion(_))
        ));
        crear_bien_privado(&unidad, &ctx, nuevo_bien("101", Some(i.id)))
            .await
            .expect("crea");
        let arbol = arbol_agrupaciones(&unidad, &ctx).await.expect("árbol");
        assert_eq!(arbol.raices.len(), 1);
        assert_eq!(arbol.raices[0].hijas[0].hijas[0].bienes.len(), 1);
    }

    #[tokio::test]
    async fn solo_parqueaderos_y_depositos_se_vinculan_como_anexos() {
        let unidad = UnidadEnMemoria::default();
        let ctx = ctx();
        let apto = crear_bien_privado(&unidad, &ctx, nuevo_bien("101", None))
            .await
            .expect("1");
        let otro = crear_bien_privado(&unidad, &ctx, nuevo_bien("102", None))
            .await
            .expect("2");
        let parq = crear_bien_privado(
            &unidad,
            &ctx,
            NuevoBienPrivado {
                tipo: TipoBienPrivado::Parqueadero,
                ..nuevo_bien("P1", None)
            },
        )
        .await
        .expect("3");
        let v = vincular_bienes_privados(&unidad, &ctx, parq.id, Some(apto.id))
            .await
            .expect("vincula");
        assert_eq!(v.bien_principal_id, Some(apto.id));
        assert!(matches!(
            vincular_bienes_privados(&unidad, &ctx, otro.id, Some(apto.id)).await,
            Err(AppError::Validacion(_))
        ));
        // Inactivar el principal desvincula sus anexos.
        inactivar_bien_privado(&unidad, &ctx, apto.id)
            .await
            .expect("inactiva");
        let ficha = ficha_bien_privado(&unidad, &crate::puertos::RelojSistema, &ctx, parq.id)
            .await
            .expect("ficha");
        assert_eq!(ficha.bien.bien_principal_id, None);
    }

    #[tokio::test]
    async fn sin_permiso_no_crea() {
        let unidad = UnidadEnMemoria::default();
        let ctx = Contexto::de_cuenta(
            domain::TenantId::nuevo(),
            domain::CuentaId::nuevo(),
            [Permiso::BienesPrivadosVer].into_iter().collect(),
        );
        assert!(matches!(
            crear_bien_privado(&unidad, &ctx, nuevo_bien("1", None)).await,
            Err(AppError::SinPermiso(Permiso::BienesPrivadosEditar))
        ));
    }
}
