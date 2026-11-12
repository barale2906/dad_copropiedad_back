//! Búsqueda de proveedores, contacto autorizado y enlace público del directorio (plan 08,
//! fase 2; ADR 0014).
//!
//! - [`buscar_proveedores`]: el residente escribe lo que necesita (chat o app); se reconoce la
//!   categoría por sinónimos y se sugieren hasta [`MAXIMO_SUGERENCIAS`] fichas que alcanzan a
//!   su PH. Responder una búsqueda que inició el residente **no es publicidad**: no exige la
//!   finalidad `publicidad` ni usa plantillas de marketing.
//! - [`autorizar_contacto`]: solo con un «Sí» explícito se comparten su nombre y su teléfono
//!   con esos proveedores (los que reciben WhatsApp) y **para esa solicitud** (Ley 1581). Cada
//!   envío queda en `contactos_proveedor` y, a los [`DIAS_SEGUIMIENTO`] días, se le pregunta
//!   cómo le fue para alimentar las reseñas verificadas.
//! - [`generar_enlace_directorio`] / [`directorio_publico`]: el enlace que la administración
//!   fija en los grupos de WhatsApp de la PH (página pública con `LimitePublico` y un enlace
//!   `wa.me` que abre el chat con «Busco …»). La plataforma no entra a los grupos.
//!
//! Ningún dato del residente llega al proveedor sin la autorización de esa solicitud, y la
//! auditoría no guarda lo que buscó.

use chrono::{DateTime, Duration, Utc};
use domain::{
    AnuncianteId, CanalSolicitud, CodigoCategoria, Modulo, Permiso, PersonaId,
    SolicitudProveedorId, TenantId,
    directorio::busqueda::{
        Candidato, CategoriaBuscable, MAXIMO_SUGERENCIAS, TEXTO_ENLACE_GRUPOS,
        comparar_sugerencias, consulta_busqueda, enlace_whatsapp, resolver_categoria,
    },
};

use crate::{
    AppError, AppResult, Contexto, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{
        avisos::{IDIOMA_PLANTILLAS, parametro},
        directorio::{ficha_visible, sin_pauta_si},
        tickets::TxTickets,
    },
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        RegistroAuditoria, Reloj, RepositorioCopropiedad, RepositorioDirectorio,
        RepositorioModulos, RepositorioPersonas, RepositorioSolicitudesProveedor,
        RepositorioTickets, Transaccion, UnidadDeTrabajo,
        directorio::{
            Categoria, EnlaceDirectorio, FichaDirectorio, FiltroDirectorio, SolicitudProveedor,
        },
        mensajeria::{ContenidoSaliente, MensajeSaliente},
    },
    seguridad::{hash_secreto, iguales, token_aleatorio},
};

/// Plantilla de utilidad «contacto_proveedor» (aprobar en Meta), al proveedor: «Hola, {{1}}.
/// {{2}} busca {{3}} y autorizó que lo contacte al {{4}}. Este dato es solo para esta
/// solicitud.»
pub const PLANTILLA_CONTACTO_PROVEEDOR: &str = "contacto_proveedor";
/// Plantilla de utilidad «opinion_proveedor» (aprobar en Meta), al residente: «¿Cómo le fue
/// con {{1}}? Califíquelo para ayudar a sus vecinos de {{2}}.» Botón «Calificar» con
/// `op:{tenant}:{solicitud}` en el `payload`.
pub const PLANTILLA_OPINION_PROVEEDOR: &str = "opinion_proveedor";
/// Días que se conserva el texto de una búsqueda (la purga el worker de gestión).
pub use domain::directorio::busqueda::RETENCION_SOLICITUDES_DIAS;

/// Días después del contacto en que se pregunta «¿Le sirvió?».
pub const DIAS_SEGUIMIENTO: i64 = 7;
/// Fichas que se consideran antes de ordenar las sugerencias.
const CANDIDATOS: u32 = SolicitudPagina::LIMITE_MAXIMO;

pub(crate) const ACCION_BUSCAR: AccionAuditoria =
    AccionAuditoria::new("directorio.solicitud.crear");
pub(crate) const ACCION_AUTORIZAR: AccionAuditoria =
    AccionAuditoria::new("directorio.solicitud.autorizar_contacto");
pub(crate) const ACCION_GENERAR_ENLACE: AccionAuditoria =
    AccionAuditoria::new("directorio.enlace.generar");
pub(crate) const ACCION_REVOCAR_ENLACE: AccionAuditoria =
    AccionAuditoria::new("directorio.enlace.revocar");

/// Puertos de la transacción que usan estos casos de uso.
pub trait TxBusquedaProveedores:
    TxTickets + RepositorioDirectorio + RepositorioSolicitudesProveedor
{
}

impl<T> TxBusquedaProveedores for T where
    T: TxTickets + RepositorioDirectorio + RepositorioSolicitudesProveedor
{
}

/// Botón «Calificar» de la plantilla [`PLANTILLA_OPINION_PROVEEDOR`]: dice la copropiedad y la
/// solicitud (cualquier línea atiende cualquier PH).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotonOpinion {
    pub tenant: TenantId,
    pub solicitud: SolicitudProveedorId,
}

impl BotonOpinion {
    const PREFIJO: &'static str = "op:";

    /// `payload` del botón.
    pub fn id(&self) -> String {
        format!("{}{}:{}", Self::PREFIJO, self.tenant, self.solicitud)
    }

    /// Interpreta el `payload`; `None` si no es un botón de opinión.
    pub fn desde_id(id: &str) -> Option<Self> {
        let (tenant, solicitud) = id.strip_prefix(Self::PREFIJO)?.split_once(':')?;
        Some(Self {
            tenant: TenantId::from_uuid(tenant.parse().ok()?),
            solicitud: SolicitudProveedorId::from_uuid(solicitud.parse().ok()?),
        })
    }
}

fn corto(texto: &str, max: usize) -> String {
    texto.chars().take(max).collect()
}

fn buscables(categorias: &[Categoria]) -> Vec<CategoriaBuscable> {
    categorias
        .iter()
        .map(|c| CategoriaBuscable {
            codigo: c.codigo.clone(),
            nombre: c.nombre.clone(),
            sinonimos: c.sinonimos.clone(),
            es_rama: c.padre.is_none(),
        })
        .collect()
}

fn candidato(f: &FichaDirectorio) -> Candidato {
    Candidato {
        destacada: f.destacada,
        promedio: f.calificacion.promedio,
        resenas: f.calificacion.total,
        distancia_km: f.distancia_km,
    }
}

// ---------------------------------------------------------------------------------------------
// Búsqueda
// ---------------------------------------------------------------------------------------------

/// Búsqueda que hace un residente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NuevaBusqueda {
    pub persona_id: PersonaId,
    pub canal: CanalSolicitud,
    /// Lo que escribió («busco costurera»).
    pub texto: String,
    /// `wamid` en WhatsApp; en la app, una clave por petición.
    pub clave: String,
}

/// Resultado de una búsqueda.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultadoBusqueda {
    pub solicitud: SolicitudProveedor,
    /// Categoría reconocida, si la hubo.
    pub categoria: Option<Categoria>,
    /// Hasta [`MAXIMO_SUGERENCIAS`], en orden.
    pub sugerencias: Vec<FichaDirectorio>,
}

impl ResultadoBusqueda {
    /// ¿Alguna sugerencia recibe contactos por WhatsApp? (Solo entonces se ofrece el «Sí».)
    pub fn hay_contactables(&self) -> bool {
        self.sugerencias
            .iter()
            .any(|f| f.contacto.whatsapp.is_some())
    }
}

/// Busca proveedores para un residente y registra la solicitud (idempotente por `clave`).
/// Primero por la categoría que nombra el texto; si no hay ninguna o no tiene fichas, por el
/// texto (nombre, descripción, categorías y sinónimos). Las destacadas solo van primero donde
/// la plataforma habilitó el módulo `publicidad`.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `directorio.ver`.
/// - [`AppError::Dominio`] si no se entiende qué busca (muy corto o muy largo).
/// - [`AppError::NoEncontrado`] si la persona no es de la copropiedad.
pub async fn buscar_proveedores<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    b: NuevaBusqueda,
) -> AppResult<ResultadoBusqueda>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxBusquedaProveedores,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    let consulta = consulta_busqueda(&b.texto)?;
    let clave = b.clave.trim();
    if clave.is_empty() || clave.chars().count() > 200 {
        return Err(AppError::Validacion(
            "clave de idempotencia inválida".into(),
        ));
    }
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    tx.persona(b.persona_id)
        .await?
        .ok_or(AppError::NoEncontrado("persona"))?;
    let categorias = tx.categorias_directorio().await?;
    let codigo = resolver_categoria(&consulta, &buscables(&categorias));
    let con_pauta = tx.modulo_habilitado(Modulo::Publicidad, ahora).await?;
    let pagina = SolicitudPagina::new(Some(CANDIDATOS), None)?;
    let mut fichas = Vec::new();
    if let Some(c) = &codigo {
        let filtro = FiltroDirectorio {
            categoria: Some(c.clone()),
            ..FiltroDirectorio::default()
        };
        fichas = tx.fichas_directorio(&filtro, ahora, pagina).await?.datos;
    }
    if fichas.is_empty() {
        let filtro = FiltroDirectorio {
            texto: Some(consulta),
            ..FiltroDirectorio::default()
        };
        fichas = tx.fichas_directorio(&filtro, ahora, pagina).await?.datos;
    }
    let mut sugerencias: Vec<FichaDirectorio> = fichas
        .into_iter()
        .map(|f| sin_pauta_si(con_pauta, f))
        .collect();
    sugerencias.sort_by(|a, b| comparar_sugerencias(&candidato(a), &candidato(b)));
    sugerencias.truncate(MAXIMO_SUGERENCIAS);

    let nueva = SolicitudProveedor {
        id: SolicitudProveedorId::nuevo(),
        persona_id: b.persona_id,
        canal: b.canal,
        clave_idempotencia: clave.to_owned(),
        texto: Some(corto(b.texto.trim(), 500)),
        categoria: codigo,
        sugeridos: sugerencias.iter().map(|f| f.anunciante_id).collect(),
        autorizado_en: None,
        creada_en: ahora,
    };
    let solicitud = tx.insertar_solicitud_proveedor(&nueva).await?;
    if solicitud.id == nueva.id {
        let detalle = DetalleAuditoria::vacio()
            .con("canal", solicitud.canal.as_str())?
            .con(
                "categoria",
                solicitud.categoria.as_ref().map(CodigoCategoria::as_str),
            )?
            .con("sugeridos", solicitud.sugeridos.len())?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_BUSCAR, "solicitud_proveedor", solicitud.id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
    } else {
        // Reintento: se devuelven las sugerencias de la solicitud original.
        sugerencias.retain(|f| solicitud.sugeridos.contains(&f.anunciante_id));
    }
    tx.confirmar().await?;
    let categoria = solicitud
        .categoria
        .as_ref()
        .and_then(|c| categorias.iter().find(|x| &x.codigo == c))
        .cloned();
    Ok(ResultadoBusqueda {
        solicitud,
        categoria,
        sugerencias,
    })
}

/// Persona del censo vinculada a la cuenta del contexto (búsquedas desde la app).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `directorio.ver`; [`AppError::NoAutenticado`] sin cuenta;
/// [`AppError::Validacion`] si la cuenta no está vinculada a una persona de la copropiedad.
pub async fn persona_de_la_cuenta<U>(unidad: &U, ctx: &Contexto) -> AppResult<PersonaId>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxBusquedaProveedores,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    let cuenta = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    unidad
        .iniciar(ctx)
        .await?
        .persona_de_cuenta(cuenta)
        .await?
        .ok_or_else(|| {
            AppError::Validacion(
                "su cuenta no está vinculada a una persona del censo de esta copropiedad".into(),
            )
        })
}

// ---------------------------------------------------------------------------------------------
// Contacto autorizado (lead)
// ---------------------------------------------------------------------------------------------

/// Resultado de autorizar el contacto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactoAutorizado {
    /// Nombres comerciales de los proveedores que recibirán el contacto.
    pub proveedores: Vec<String>,
    /// ¿Se programó el «¿Le sirvió?»?
    pub seguimiento: bool,
}

/// Con la autorización expresa del residente, comparte su nombre y su teléfono con los
/// proveedores sugeridos que reciben WhatsApp (plantilla [`PLANTILLA_CONTACTO_PROVEEDOR`]) y
/// programa el «¿Le sirvió?» a los [`DIAS_SEGUIMIENTO`] días. Repetirlo no vuelve a enviar
/// nada. `linea`: la del chat; `None` = la predeterminada.
///
/// # Errors
///
/// - [`AppError::SinPermiso`] sin `directorio.ver`.
/// - [`AppError::NoEncontrado`] si la solicitud no existe o es de otra persona.
/// - [`AppError::Validacion`] si la persona no tiene celular, si ningún proveedor recibe
///   WhatsApp o si no hay líneas de WhatsApp.
pub async fn autorizar_contacto<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: SolicitudProveedorId,
    persona_id: PersonaId,
    linea: Option<String>,
) -> AppResult<ContactoAutorizado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxBusquedaProveedores,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    let solicitud = tx
        .solicitud_proveedor(id)
        .await?
        .filter(|s| s.persona_id == persona_id)
        .ok_or(AppError::NoEncontrado("solicitud de proveedor"))?;
    if solicitud.autorizado_en.is_some() {
        let mut proveedores = Vec::new();
        for a in tx.contactos_de_solicitud(id).await? {
            if let Ok(f) = ficha_visible(&mut tx, a, ahora).await {
                proveedores.push(f.nombre_comercial);
            }
        }
        return Ok(ContactoAutorizado {
            proveedores,
            seguimiento: false,
        });
    }
    let persona = tx
        .persona(persona_id)
        .await?
        .ok_or(AppError::NoEncontrado("persona"))?;
    let telefono = persona.telefono.clone().ok_or_else(|| {
        AppError::Validacion(
            "registre su celular en el censo para que los proveedores lo contacten".into(),
        )
    })?;
    let mut contactables = Vec::new();
    for a in &solicitud.sugeridos {
        if let Ok(f) = ficha_visible(&mut tx, *a, ahora).await
            && f.contacto.whatsapp.is_some()
        {
            contactables.push(f);
        }
    }
    if contactables.is_empty() {
        return Err(AppError::Validacion(
            "ninguno de estos proveedores recibe contactos por WhatsApp; llámelos al número de \
             su ficha"
                .into(),
        ));
    }
    let linea = match linea {
        Some(l) => l,
        None => tx
            .linea_predeterminada()
            .await?
            .ok_or_else(|| AppError::Validacion("no hay líneas de WhatsApp disponibles".into()))?,
    };
    let categorias = tx.categorias_directorio().await?;
    let nombre_categoria = |c: &CodigoCategoria| {
        categorias
            .iter()
            .find(|x| &x.codigo == c)
            .map(|x| x.nombre.to_lowercase())
    };
    let buscada = solicitud.categoria.as_ref().and_then(nombre_categoria);
    let contacto = format!("+{}", telefono.as_str());
    for f in &contactables {
        let necesidad = buscada
            .clone()
            .or_else(|| f.categorias.first().and_then(nombre_categoria))
            .unwrap_or_else(|| "sus servicios".into());
        let Some(destino) = &f.contacto.whatsapp else {
            continue;
        };
        tx.insertar_contacto_proveedor(id, f.anunciante_id, ahora)
            .await?;
        let mensaje = MensajeOutbox::WhatsApp(MensajeSaliente {
            linea: linea.clone(),
            telefono: destino.as_str().to_owned(),
            contenido: ContenidoSaliente::Plantilla {
                nombre: PLANTILLA_CONTACTO_PROVEEDOR.to_owned(),
                idioma: IDIOMA_PLANTILLAS.to_owned(),
                parametros: vec![
                    parametro(&f.nombre_comercial),
                    parametro(&persona.nombre),
                    parametro(&necesidad),
                    contacto.clone(),
                ],
                botones: Vec::new(),
            },
        });
        tx.encolar_directorio(
            &EventoSaliente {
                prioridad: PrioridadSalida::Evento,
                mensaje,
            },
            None,
        )
        .await?;
    }
    tx.autorizar_contacto(id, ahora).await?;

    let proveedores: Vec<String> = contactables
        .iter()
        .map(|f| f.nombre_comercial.clone())
        .collect();
    let ph = tx.ficha().await?.map(|f| f.nombre).unwrap_or_default();
    let boton = BotonOpinion {
        tenant: ctx.tenant_id,
        solicitud: id,
    };
    let seguimiento = MensajeOutbox::WhatsApp(MensajeSaliente {
        linea,
        telefono: telefono.as_str().to_owned(),
        contenido: ContenidoSaliente::Plantilla {
            nombre: PLANTILLA_OPINION_PROVEEDOR.to_owned(),
            idioma: IDIOMA_PLANTILLAS.to_owned(),
            parametros: vec![parametro(&proveedores.join(", ")), parametro(&ph)],
            botones: vec![boton.id()],
        },
    });
    tx.encolar_directorio(
        &EventoSaliente {
            prioridad: PrioridadSalida::Recordatorio,
            mensaje: seguimiento,
        },
        Some(ahora + Duration::days(DIAS_SEGUIMIENTO)),
    )
    .await?;
    let detalle = DetalleAuditoria::vacio().con("proveedores", proveedores.len())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_AUTORIZAR, "solicitud_proveedor", id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(ContactoAutorizado {
        proveedores,
        seguimiento: true,
    })
}

/// Proveedor con quien se compartió el contacto de una solicitud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProveedorContactado {
    pub anunciante_id: AnuncianteId,
    pub nombre_comercial: String,
}

/// Proveedores contactados por una solicitud de la persona que aún alcanzan a la PH (para el
/// «¿Le sirvió?»).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `directorio.ver`; [`AppError::NoEncontrado`] si la solicitud
/// no existe o es de otra persona.
pub async fn proveedores_contactados<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    id: SolicitudProveedorId,
    persona_id: PersonaId,
) -> AppResult<Vec<ProveedorContactado>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxBusquedaProveedores,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    tx.solicitud_proveedor(id)
        .await?
        .filter(|s| s.persona_id == persona_id)
        .ok_or(AppError::NoEncontrado("solicitud de proveedor"))?;
    let mut v = Vec::new();
    for a in tx.contactos_de_solicitud(id).await? {
        if let Ok(f) = ficha_visible(&mut tx, a, ahora).await {
            v.push(ProveedorContactado {
                anunciante_id: a,
                nombre_comercial: f.nombre_comercial,
            });
        }
    }
    Ok(v)
}

// ---------------------------------------------------------------------------------------------
// Enlace público del directorio (grupos de WhatsApp)
// ---------------------------------------------------------------------------------------------

/// Enlace recién generado. El token se entrega **una sola vez**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnlaceGenerado {
    pub token: String,
    pub creado_en: DateTime<Utc>,
    /// `https://wa.me/<línea>?text=Busco%20`, si hay líneas.
    pub whatsapp: Option<String>,
}

/// Estado del enlace público de la copropiedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstadoEnlace {
    /// Fecha del enlace vigente; `None` = no hay.
    pub creado_en: Option<DateTime<Utc>>,
    pub whatsapp: Option<String>,
}

async fn enlace_wa<T: RepositorioSolicitudesProveedor>(tx: &mut T) -> AppResult<Option<String>> {
    Ok(tx
        .numero_whatsapp_publico()
        .await?
        .and_then(|n| enlace_whatsapp(&n, TEXTO_ENLACE_GRUPOS)))
}

/// Genera el enlace público del directorio (reemplaza el anterior, que deja de funcionar).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `directorio.compartir`.
pub async fn generar_enlace_directorio<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<EnlaceGenerado>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxBusquedaProveedores,
{
    ctx.exigir(Permiso::DirectorioCompartir)?;
    let token = token_aleatorio()?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    let reemplaza = tx.enlace_directorio().await?.is_some();
    tx.guardar_enlace_directorio(&EnlaceDirectorio {
        token_hash: huella(&token)?,
        creado_por: ctx.actor_id,
        creado_en: ahora,
    })
    .await?;
    let whatsapp = enlace_wa(&mut tx).await?;
    let detalle = DetalleAuditoria::vacio().con("reemplaza", reemplaza)?;
    tx.registrar(
        EventoAuditoria::ok(
            ACCION_GENERAR_ENLACE,
            "enlace_directorio",
            ctx.tenant_id.as_uuid(),
        )
        .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(EnlaceGenerado {
        token,
        creado_en: ahora,
        whatsapp,
    })
}

/// ¿Hay un enlace vigente? (El token no se puede volver a ver: se genera otro.)
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `directorio.compartir`.
pub async fn estado_enlace_directorio<U>(unidad: &U, ctx: &Contexto) -> AppResult<EstadoEnlace>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxBusquedaProveedores,
{
    ctx.exigir(Permiso::DirectorioCompartir)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let creado_en = tx.enlace_directorio().await?.map(|e| e.creado_en);
    Ok(EstadoEnlace {
        creado_en,
        whatsapp: enlace_wa(&mut tx).await?,
    })
}

/// Revoca el enlace público. `false` si no había.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `directorio.compartir`.
pub async fn revocar_enlace_directorio<U>(unidad: &U, ctx: &Contexto) -> AppResult<bool>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxBusquedaProveedores,
{
    ctx.exigir(Permiso::DirectorioCompartir)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let habia = tx.borrar_enlace_directorio().await?;
    if habia {
        tx.registrar(EventoAuditoria::ok(
            ACCION_REVOCAR_ENLACE,
            "enlace_directorio",
            ctx.tenant_id.as_uuid(),
        ))
        .await?;
    }
    tx.confirmar().await?;
    Ok(habia)
}

/// SHA-256 (32 bytes) del token del enlace.
pub(crate) fn huella(token: &str) -> AppResult<Vec<u8>> {
    hex::decode(hash_secreto(token.trim())).map_err(AppError::infra)
}

/// Directorio público de una copropiedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectorioPublico {
    pub copropiedad: String,
    pub whatsapp: Option<String>,
    pub fichas: Pagina<FichaDirectorio>,
}

/// Directorio público de la copropiedad (sin sesión: el token del enlace es la credencial).
/// Las mismas fichas que ve un residente; sin texto de reseñas.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] si el token no es el del enlace vigente de la copropiedad.
pub async fn directorio_publico<U>(
    unidad: &U,
    reloj: &impl Reloj,
    tenant: TenantId,
    token: &str,
    filtro: FiltroDirectorio,
    pagina: SolicitudPagina,
) -> AppResult<DirectorioPublico>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxBusquedaProveedores,
{
    let ctx = Contexto::new(tenant);
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(&ctx).await?;
    let esperado = tx
        .enlace_directorio()
        .await?
        .ok_or(AppError::NoAutenticado)?;
    if !iguales(
        &hex::encode(&esperado.token_hash),
        &hash_secreto(token.trim()),
    ) {
        return Err(AppError::NoAutenticado);
    }
    let filtro = FiltroDirectorio {
        anunciante: None,
        texto: filtro.texto.filter(|t| !t.trim().is_empty()),
        solo_destacadas: false,
        ..filtro
    };
    let con_pauta = tx.modulo_habilitado(Modulo::Publicidad, ahora).await?;
    let fichas = tx
        .fichas_directorio(&filtro, ahora, pagina)
        .await?
        .mapear(|f| sin_pauta_si(con_pauta, f));
    let copropiedad = tx.ficha().await?.map(|f| f.nombre).unwrap_or_default();
    Ok(DirectorioPublico {
        copropiedad,
        whatsapp: enlace_wa(&mut tx).await?,
        fichas,
    })
}

#[cfg(test)]
mod tests;
