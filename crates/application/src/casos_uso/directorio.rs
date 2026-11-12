//! Directorio de comercios y servicios (plan 08, fase 1; ADR 0013).
//!
//! - **Plataforma** (`plataforma.directorio.gestionar`): catálogo de categorías, alta de
//!   anunciantes (quedan verificados: la plataforma los revisa al darlos de alta) y sus fichas.
//! - **Moderación** (`plataforma.publicidad.moderar`): suspender o reactivar anunciantes y
//!   atender los reportes de reseñas (ocultarlas con motivo o mantenerlas).
//! - **Copropiedad** (`directorio.ver`, `directorio.resenar`): el directorio que alcanza a la
//!   PH y las reseñas de sus residentes y propietarios vigentes.
//!
//! El directorio funciona en todas las copropiedades. Que una ficha destacada salga primero es
//! pauta: solo donde la plataforma habilitó el módulo `publicidad`. Las reseñas se publican de
//! inmediato, sin autor; su texto solo se ve en la misma PH y el promedio suma todas.

use domain::{
    AnuncianteId, Calificacion, Cobertura, CodigoCategoria, ConjuntoPermisos, CuentaId,
    EstadoResena, EstadoVerificacion, Modulo, Permiso, ResenaId, TenantId, Ubicacion,
    copropiedad::{texto_obligatorio, texto_opcional},
    directorio::{texto_resena, validar_categorias, validar_cobertura},
};

use crate::{
    AppError, AppResult, Contexto, ContextoPlataforma, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::organizaciones::validar_nueva,
    puertos::{
        RegistroAuditoria, Reloj, RepositorioDirectorio, RepositorioDirectorioPlataforma,
        RepositorioModulos, Transaccion, UnidadDeTrabajo, UnidadDeTrabajoPlataforma,
        directorio::{
            Anunciante, Categoria, Contacto, Ficha, FichaDirectorio, FiltroAnunciantes,
            FiltroDirectorio, OrganizacionAnunciante, ReporteResena, Resena, ResenaReportada,
        },
    },
};

pub const ACCION_GUARDAR_CATEGORIA: AccionAuditoria =
    AccionAuditoria::new("plataforma.directorio_categoria.guardar");
pub const ACCION_CREAR_ANUNCIANTE: AccionAuditoria =
    AccionAuditoria::new("plataforma.anunciante.crear");
pub const ACCION_EDITAR_ANUNCIANTE: AccionAuditoria =
    AccionAuditoria::new("plataforma.anunciante.editar");
pub const ACCION_VERIFICACION: AccionAuditoria =
    AccionAuditoria::new("plataforma.anunciante.cambiar_estado");
pub const ACCION_GUARDAR_FICHA: AccionAuditoria =
    AccionAuditoria::new("plataforma.ficha_directorio.guardar");
pub const ACCION_RESENAR: AccionAuditoria = AccionAuditoria::new("directorio.resena.guardar");
pub const ACCION_REPORTAR: AccionAuditoria = AccionAuditoria::new("directorio.resena.reportar");
pub const ACCION_MODERAR: AccionAuditoria = AccionAuditoria::new("directorio.resena.moderar");

/// Máximo de fichas destacadas que se devuelven a la vez.
pub const MAXIMO_DESTACADAS: u32 = 10;

fn exigir_gestion(ctx: &ContextoPlataforma) -> AppResult<()> {
    ctx.exigir(Permiso::PlataformaDirectorioGestionar)
}

/// Gestionar o moderar: ambos pueden consultar los anunciantes.
fn exigir_gestion_o_moderacion(ctx: &ContextoPlataforma) -> AppResult<()> {
    exigir_gestion(ctx).or_else(|_| ctx.exigir(Permiso::PlataformaPublicidadModerar))
}

fn motivo(campo: &'static str, valor: Option<&str>) -> AppResult<Option<String>> {
    Ok(texto_opcional(campo, valor, 500)?)
}

// ---------------------------------------------------------------------------------------------
// Catálogo de categorías (plataforma)
// ---------------------------------------------------------------------------------------------

/// Todo el catálogo (incluidas las restringidas e inactivas).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.directorio.gestionar`.
pub async fn categorias_plataforma<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<Categoria>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioDirectorioPlataforma,
{
    exigir_gestion(ctx)?;
    unidad.iniciar_plataforma(ctx).await?.categorias().await
}

/// Crea o edita una categoría del catálogo. El árbol tiene dos niveles: la rama no puede tener
/// rama, y una rama con subcategorías no puede pasar a ser subcategoría.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::Validacion`] / [`AppError::Dominio`] si los textos no
/// son válidos o la rama no existe o no es una rama.
pub async fn guardar_categoria<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    categoria: Categoria,
) -> AppResult<Categoria>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioDirectorioPlataforma + RegistroAuditoria,
{
    exigir_gestion(ctx)?;
    let mut sinonimos = Vec::new();
    for s in &categoria.sinonimos {
        if let Some(s) = texto_opcional("sinonimos", Some(s), 60)?
            && !sinonimos.contains(&s)
        {
            sinonimos.push(s);
        }
    }
    if sinonimos.len() > 30 {
        return Err(AppError::Validacion(
            "hasta 30 sinónimos por categoría".into(),
        ));
    }
    let categoria = Categoria {
        nombre: texto_obligatorio("nombre", &categoria.nombre, 80)?,
        sinonimos,
        ..categoria
    };
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    if let Some(padre) = &categoria.padre {
        if padre == &categoria.codigo {
            return Err(AppError::Validacion(
                "una categoría no es rama de sí misma".into(),
            ));
        }
        let rama = tx
            .categoria(padre)
            .await?
            .ok_or_else(|| AppError::Validacion("la rama indicada no existe".into()))?;
        if rama.padre.is_some() {
            return Err(AppError::Validacion(
                "el catálogo tiene dos niveles: la rama no puede ser una subcategoría".into(),
            ));
        }
        if tx.tiene_subcategorias(&categoria.codigo).await? {
            return Err(AppError::Validacion(
                "una rama con subcategorías no puede pasar a ser subcategoría".into(),
            ));
        }
    }
    tx.guardar_categoria(&categoria).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("codigo", categoria.codigo.as_str())?
        .con("restringida", categoria.restringida)?
        .con("activa", categoria.activa)?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_GUARDAR_CATEGORIA,
            "categoria_publicidad",
            uuid::Uuid::nil(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(categoria)
}

// ---------------------------------------------------------------------------------------------
// Anunciantes y fichas (plataforma)
// ---------------------------------------------------------------------------------------------

/// Datos de un anunciante que se pueden editar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosAnunciante {
    pub nombre_comercial: String,
    pub direccion: Option<String>,
    pub ubicacion: Ubicacion,
}

impl DatosAnunciante {
    pub(crate) fn validados(self) -> AppResult<Self> {
        Ok(Self {
            nombre_comercial: texto_obligatorio("nombre_comercial", &self.nombre_comercial, 120)?,
            direccion: texto_opcional("direccion", self.direccion.as_deref(), 200)?,
            ubicacion: self.ubicacion,
        })
    }
}

/// Da de alta un anunciante sobre una organización (existente o nueva, reutilizada por NIT).
/// Queda **verificado**: en esta fase la plataforma lo revisa antes de darlo de alta (el
/// portal del anunciante con revisión previa llega en la fase 3).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::Validacion`]; [`AppError::NoEncontrado`] si la
/// organización no existe; [`AppError::Conflicto`] si ya tiene un anunciante.
pub async fn dar_de_alta_anunciante<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    organizacion: OrganizacionAnunciante,
    datos: DatosAnunciante,
) -> AppResult<Anunciante>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioDirectorioPlataforma + RegistroAuditoria,
{
    exigir_gestion(ctx)?;
    if let OrganizacionAnunciante::Nueva(nueva) = &organizacion {
        validar_nueva(nueva)?;
    }
    let datos = datos.validados()?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let organizacion_id = tx
        .organizacion_para_anunciante(&organizacion, ctx.actor_id)
        .await?
        .ok_or(AppError::NoEncontrado("organización"))?;
    let ahora = reloj.ahora();
    let anunciante = Anunciante {
        id: AnuncianteId::nuevo(),
        organizacion_id,
        nombre_comercial: datos.nombre_comercial,
        direccion: datos.direccion,
        ubicacion: datos.ubicacion,
        estado: EstadoVerificacion::Verificado,
        motivo_estado: None,
        creado_por: ctx.actor_id,
        creado_en: ahora,
        actualizado_en: ahora,
    };
    tx.insertar_anunciante(&anunciante).await?;
    let detalle = DetalleAuditoria::vacio().con("organizacion_id", organizacion_id.to_string())?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_CREAR_ANUNCIANTE,
            "anunciante",
            anunciante.id.as_uuid(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(anunciante)
}

/// Edita nombre, dirección y ubicación. Si ya tiene ficha, la ubicación nueva debe seguir
/// permitiendo su cobertura.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] si la ubicación
/// no sirve para la cobertura de la ficha.
pub async fn editar_anunciante<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: AnuncianteId,
    datos: DatosAnunciante,
) -> AppResult<Anunciante>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioDirectorioPlataforma + RegistroAuditoria,
{
    exigir_gestion(ctx)?;
    let datos = datos.validados()?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut anunciante = tx
        .anunciante_para_actualizar(id)
        .await?
        .ok_or(AppError::NoEncontrado("anunciante"))?;
    if let Some(ficha) = tx.ficha(id).await? {
        validar_cobertura(ficha.cobertura, &datos.ubicacion)?;
    }
    anunciante.nombre_comercial = datos.nombre_comercial;
    anunciante.direccion = datos.direccion;
    anunciante.ubicacion = datos.ubicacion;
    anunciante.actualizado_en = reloj.ahora();
    tx.actualizar_anunciante(&anunciante).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_EDITAR_ANUNCIANTE,
        "anunciante",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(anunciante)
}

/// Anunciante con su ficha (si la tiene).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin gestionar ni moderar; [`AppError::NoEncontrado`].
pub async fn anunciante<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    id: AnuncianteId,
) -> AppResult<(Anunciante, Option<Ficha>)>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioDirectorioPlataforma,
{
    exigir_gestion_o_moderacion(ctx)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let anunciante = tx
        .anunciante(id)
        .await?
        .ok_or(AppError::NoEncontrado("anunciante"))?;
    let ficha = tx.ficha(id).await?;
    Ok((anunciante, ficha))
}

/// Anunciantes por estado o nombre, paginados por `id`.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin gestionar ni moderar.
pub async fn listar_anunciantes<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    filtro: &FiltroAnunciantes,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Anunciante>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioDirectorioPlataforma,
{
    exigir_gestion_o_moderacion(ctx)?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .anunciantes(filtro, pagina)
        .await
}

/// Contenido editable de una ficha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosFicha {
    pub descripcion: Option<String>,
    pub categorias: Vec<CodigoCategoria>,
    pub cobertura: Cobertura,
    pub contacto: Contacto,
    pub horario: Option<String>,
    pub destacada_hasta: Option<chrono::DateTime<chrono::Utc>>,
    pub publicada: bool,
}

/// Crea o reemplaza la ficha del anunciante. Las categorías siguen
/// [`validar_categorias`] (nunca una restringida) y la cobertura debe ser posible con la
/// ubicación del anunciante.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; [`AppError::Dominio`] o
/// [`AppError::Validacion`] si el contenido no es válido.
pub async fn guardar_ficha<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: AnuncianteId,
    datos: DatosFicha,
) -> AppResult<Ficha>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioDirectorioPlataforma + RegistroAuditoria,
{
    exigir_gestion(ctx)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let ficha = guardar_ficha_en(&mut tx, reloj.ahora(), id, datos).await?;
    tx.confirmar().await?;
    Ok(ficha)
}

/// Valida y guarda la ficha dentro de la transacción (también la usa el portal del
/// anunciante, `casos_uso::publicidad::portal`).
pub(crate) async fn guardar_ficha_en<T>(
    tx: &mut T,
    ahora: chrono::DateTime<chrono::Utc>,
    id: AnuncianteId,
    datos: DatosFicha,
) -> AppResult<Ficha>
where
    T: RepositorioDirectorioPlataforma + RegistroAuditoria,
{
    if datos.contacto.sitio_web.as_deref().is_some_and(|s| {
        !s.starts_with("https://") || s.chars().count() > 300 || s.contains(char::is_whitespace)
    }) {
        return Err(AppError::Validacion(
            "el sitio web debe empezar por https:// y tener hasta 300 caracteres".into(),
        ));
    }
    let descripcion = texto_resena(datos.descripcion.as_deref())
        .map_err(|_| AppError::Validacion("la descripción admite hasta 1000 caracteres".into()))?;
    let horario = texto_opcional("horario", datos.horario.as_deref(), 300)?;
    let anunciante = tx
        .anunciante_para_actualizar(id)
        .await?
        .ok_or(AppError::NoEncontrado("anunciante"))?;
    let catalogo: Vec<_> = tx
        .categorias_por_codigo(&datos.categorias)
        .await?
        .iter()
        .map(Categoria::elegible)
        .collect();
    validar_categorias(&datos.categorias, &catalogo)?;
    validar_cobertura(datos.cobertura, &anunciante.ubicacion)?;
    let ficha = Ficha {
        anunciante_id: id,
        descripcion,
        categorias: datos.categorias,
        cobertura: datos.cobertura,
        contacto: datos.contacto,
        horario,
        destacada_hasta: datos.destacada_hasta,
        publicada: datos.publicada,
        actualizada_en: ahora,
    };
    tx.guardar_ficha(&ficha).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("cobertura", ficha.cobertura.tipo().as_str())?
        .con("publicada", ficha.publicada)?
        .con("destacada", ficha.destacada_hasta.is_some())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_GUARDAR_FICHA, "anunciante", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    Ok(ficha)
}

/// Verifica, rechaza, suspende o reactiva un anunciante ([`EstadoVerificacion::transicionar`]).
/// Rechazar y suspender exigen motivo.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.publicidad.moderar`; [`AppError::NoEncontrado`];
/// [`AppError::Dominio`] si la transición no está permitida; [`AppError::Validacion`] sin
/// motivo.
pub async fn cambiar_verificacion<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: AnuncianteId,
    hacia: EstadoVerificacion,
    motivo_cambio: Option<&str>,
) -> AppResult<Anunciante>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioDirectorioPlataforma + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaPublicidadModerar)?;
    let motivo_cambio = motivo("motivo", motivo_cambio)?;
    if hacia.exige_motivo() && motivo_cambio.is_none() {
        return Err(AppError::Validacion(
            "indique el motivo del rechazo o de la suspensión".into(),
        ));
    }
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut anunciante = tx
        .anunciante_para_actualizar(id)
        .await?
        .ok_or(AppError::NoEncontrado("anunciante"))?;
    let desde = anunciante.estado;
    anunciante.estado = desde.transicionar(hacia)?;
    anunciante.motivo_estado = if hacia.exige_motivo() {
        motivo_cambio
    } else {
        None
    };
    anunciante.actualizado_en = reloj.ahora();
    tx.actualizar_anunciante(&anunciante).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("desde", desde.as_str())?
        .con("hacia", hacia.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_VERIFICACION, "anunciante", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(anunciante)
}

// ---------------------------------------------------------------------------------------------
// Moderación de reseñas (plataforma)
// ---------------------------------------------------------------------------------------------

/// Cola de moderación: reseñas publicadas con reportes sin atender, de todas las PH.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.publicidad.moderar`.
pub async fn resenas_reportadas<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<ResenaReportada>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: RepositorioDirectorioPlataforma,
{
    ctx.exigir(Permiso::PlataformaPublicidadModerar)?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .resenas_reportadas(pagina)
        .await
}

/// Decisión de la moderación sobre una reseña reportada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionModeracion {
    /// Sale del directorio; el motivo queda registrado.
    Ocultar { motivo: String },
    /// Sigue publicada; los reportes quedan atendidos.
    Mantener,
}

/// Atiende los reportes de una reseña: la oculta con motivo o la mantiene. Se ejecuta en la
/// copropiedad de la reseña, auditado a nombre del moderador.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.publicidad.moderar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si ocultar no trae motivo.
pub async fn moderar_resena<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    tenant: TenantId,
    id: ResenaId,
    decision: DecisionModeracion,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioDirectorio + RegistroAuditoria,
{
    ctx.exigir(Permiso::PlataformaPublicidadModerar)?;
    let ocultar = match &decision {
        DecisionModeracion::Ocultar { motivo: m } => Some(
            motivo("motivo", Some(m))?
                .ok_or_else(|| AppError::Validacion("indique el motivo para ocultarla".into()))?,
        ),
        DecisionModeracion::Mantener => None,
    };
    // El moderador actúa en la PH solo sobre la reseña; sin permisos de copropiedad.
    let ctx_ph = Contexto {
        tenant_id: tenant,
        actor_id: ctx.actor_id,
        request_id: ctx.request_id.clone(),
        permisos: ConjuntoPermisos::vacio(),
    };
    let mut tx = unidad.iniciar(&ctx_ph).await?;
    tx.resena(id)
        .await?
        .ok_or(AppError::NoEncontrado("reseña"))?;
    tx.moderar_resena(id, ocultar.as_deref(), ctx.actor_id, reloj.ahora())
        .await?;
    let detalle = DetalleAuditoria::vacio().con("ocultada", ocultar.is_some())?;
    tx.registrar(EventoAuditoria::ok(ACCION_MODERAR, "resena", id.as_uuid()).con_detalle(detalle))
        .await?;
    tx.confirmar().await
}

// ---------------------------------------------------------------------------------------------
// Directorio de la copropiedad
// ---------------------------------------------------------------------------------------------

/// Las fichas que alcanzan a la copropiedad, paginadas por `id`. `destacada` solo es `true`
/// donde la plataforma habilitó el módulo `publicidad`.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `directorio.ver`.
pub async fn consultar_directorio<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    filtro: FiltroDirectorio,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<FichaDirectorio>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioDirectorio + RepositorioModulos,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    let ahora = reloj.ahora();
    let filtro = FiltroDirectorio {
        texto: filtro.texto.filter(|t| !t.trim().is_empty()),
        solo_destacadas: false,
        ..filtro
    };
    let mut tx = unidad.iniciar(ctx).await?;
    let con_pauta = tx.modulo_habilitado(Modulo::Publicidad, ahora).await?;
    let pagina = tx.fichas_directorio(&filtro, ahora, pagina).await?;
    Ok(pagina.mapear(|f| sin_pauta_si(con_pauta, f)))
}

pub(crate) fn sin_pauta_si(con_pauta: bool, f: FichaDirectorio) -> FichaDirectorio {
    FichaDirectorio {
        destacada: f.destacada && con_pauta,
        ..f
    }
}

/// Fichas destacadas vigentes (hasta [`MAXIMO_DESTACADAS`]). Vacío si la copropiedad no tiene
/// el módulo `publicidad`: sin él no hay pauta.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `directorio.ver`.
pub async fn destacadas<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    categoria: Option<CodigoCategoria>,
) -> AppResult<Vec<FichaDirectorio>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioDirectorio + RepositorioModulos,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    if !tx.modulo_habilitado(Modulo::Publicidad, ahora).await? {
        return Ok(Vec::new());
    }
    let filtro = FiltroDirectorio {
        categoria,
        solo_destacadas: true,
        ..FiltroDirectorio::default()
    };
    let pagina = SolicitudPagina::new(Some(MAXIMO_DESTACADAS), None)?;
    Ok(tx.fichas_directorio(&filtro, ahora, pagina).await?.datos)
}

/// Una ficha del directorio de la copropiedad.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`] si no existe o no alcanza a la PH.
pub async fn ficha_directorio<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: AnuncianteId,
) -> AppResult<FichaDirectorio>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioDirectorio + RepositorioModulos,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    let con_pauta = tx.modulo_habilitado(Modulo::Publicidad, ahora).await?;
    let ficha = ficha_visible(&mut tx, id, ahora).await?;
    Ok(sin_pauta_si(con_pauta, ficha))
}

pub(crate) async fn ficha_visible<T: RepositorioDirectorio>(
    tx: &mut T,
    id: AnuncianteId,
    ahora: chrono::DateTime<chrono::Utc>,
) -> AppResult<FichaDirectorio> {
    let filtro = FiltroDirectorio {
        anunciante: Some(id),
        ..FiltroDirectorio::default()
    };
    tx.fichas_directorio(&filtro, ahora, SolicitudPagina::primera())
        .await?
        .datos
        .into_iter()
        .next()
        .ok_or(AppError::NoEncontrado("ficha del directorio"))
}

/// Categorías que se pueden usar para filtrar (activas y no restringidas).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `directorio.ver`.
pub async fn categorias_directorio<U>(unidad: &U, ctx: &Contexto) -> AppResult<Vec<Categoria>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioDirectorio,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    unidad.iniciar(ctx).await?.categorias_directorio().await
}

/// Reseñas publicadas del anunciante en esta copropiedad (sin autor).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`] si la ficha no alcanza a la PH.
pub async fn resenas_de_anunciante<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: AnuncianteId,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Resena>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioDirectorio,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    let mut tx = unidad.iniciar(ctx).await?;
    ficha_visible(&mut tx, id, reloj.ahora()).await?;
    tx.resenas_publicadas(id, pagina).await
}

/// Califica (o vuelve a calificar) a un anunciante. Solo propietarios y residentes vigentes de
/// una PH que el anunciante alcanza; una reseña por cuenta y anunciante en cada PH.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `directorio.resenar` o si la cuenta no es propietaria ni
///   residente vigente.
/// - [`AppError::NoEncontrado`] si la ficha no alcanza a la PH.
/// - [`AppError::Conflicto`] si la moderación ocultó su reseña anterior.
/// - [`AppError::Dominio`] si la calificación o el texto no son válidos.
pub async fn resenar<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: AnuncianteId,
    calificacion: u8,
    texto: Option<&str>,
) -> AppResult<Resena>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioDirectorio + RegistroAuditoria,
{
    ctx.exigir(Permiso::DirectorioResenar)?;
    let cuenta = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    let calificacion = Calificacion::new(calificacion)?;
    let texto = texto_resena(texto)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    if !tx.es_residente_vigente(cuenta, ahora).await? {
        return Err(AppError::SinPermiso(Permiso::DirectorioResenar));
    }
    ficha_visible(&mut tx, id, ahora).await?;
    let resena = match tx.resena_de_cuenta(id, cuenta).await? {
        Some(r) if r.estado == EstadoResena::Oculta => {
            return Err(AppError::Conflicto(
                "la moderación ocultó su reseña de este anunciante".into(),
            ));
        }
        Some(r) => Resena {
            calificacion,
            texto,
            actualizada_en: ahora,
            ..r
        },
        None => Resena {
            id: ResenaId::nuevo(),
            anunciante_id: id,
            cuenta_id: cuenta,
            calificacion,
            texto,
            estado: EstadoResena::Publicada,
            creada_en: ahora,
            actualizada_en: ahora,
        },
    };
    tx.guardar_resena(&resena).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("anunciante_id", id.to_string())?
        .con("calificacion", calificacion.valor())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_RESENAR, "resena", resena.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(resena)
}

/// Reporta una reseña publicada de la copropiedad para que la modere la plataforma. `false` si
/// la cuenta ya la había reportado.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `directorio.ver`; [`AppError::NoEncontrado`] si no existe o ya
/// está oculta; [`AppError::Validacion`] sin motivo.
pub async fn reportar_resena<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: ResenaId,
    motivo_reporte: &str,
) -> AppResult<bool>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioDirectorio + RegistroAuditoria,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    let cuenta: CuentaId = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    let motivo_reporte = motivo("motivo", Some(motivo_reporte))?
        .ok_or_else(|| AppError::Validacion("indique por qué reporta la reseña".into()))?;
    let mut tx = unidad.iniciar(ctx).await?;
    tx.resena(id)
        .await?
        .filter(|r| r.estado == EstadoResena::Publicada)
        .ok_or(AppError::NoEncontrado("reseña"))?;
    let nuevo = tx
        .insertar_reporte(&ReporteResena {
            resena_id: id,
            cuenta_id: cuenta,
            motivo: motivo_reporte,
            creado_en: reloj.ahora(),
        })
        .await?;
    if nuevo {
        tx.registrar(EventoAuditoria::ok(ACCION_REPORTAR, "resena", id.as_uuid()))
            .await?;
    }
    tx.confirmar().await?;
    Ok(nuevo)
}

#[cfg(test)]
mod tests;
