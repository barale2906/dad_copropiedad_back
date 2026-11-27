//! Gestión de la pauta por el personal de la plataforma (plan 20; antes, el portal del
//! anunciante del plan 08): los comercios **no** usan la plataforma. Quien tiene
//! `plataforma.publicidad.gestionar` carga las campañas y piezas de cualquier comercio, ve sus
//! métricas agregadas y sus contactos autorizados, administra el enlace con que el comercio
//! redime cupones y excluye entidades de la publicidad. La moderación la hace **otra cuenta**
//! (cuatro ojos, [`super::moderacion`]).
//!
//! Lo que se ve de un comercio nunca incluye datos de residentes, salvo los contactos que un
//! residente autorizó expresamente compartirle para una solicitud ([`leads`]).

use chrono::{DateTime, Utc};
use domain::{
    AnuncianteId, ArchivoId, CampanaId, ClasificacionArchivo, ClaveArchivo, CodigoCategoria,
    CodigoCupon, CreativoId, EstadoCampana, EstadoCreativo, ModeloCobro, NombreArchivo,
    ObjetivoCampana, Permiso, Segmentacion, TenantId, TipoArchivo,
    copropiedad::texto_obligatorio,
    publicidad::{
        ContenidoCreativo, Conteo, Presupuesto, ajustar_al_presupuesto, costo, presupuesto_agotado,
        validar_redencion,
    },
};
use rust_decimal::Decimal;

use super::TxPublicidadPlataforma;
use crate::casos_uso::archivos::sha256_hex;
use crate::puertos::{
    AlmacenArchivos, RegistroAuditoria, RepositorioArchivosPlataforma,
    RepositorioDirectorioPlataforma as _, RepositorioPublicidadPlataforma,
    archivos::{Archivo, DescargaArchivo},
};
use crate::{
    AppError, AppResult, ContextoPlataforma, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::busqueda_proveedores::huella,
    puertos::{
        Reloj, Transaccion, UnidadDeTrabajoPlataforma,
        cofre::Cofre,
        directorio::Anunciante,
        publicidad::{Campana, Creativo, ExclusionPublicidad, Lead, MetricaCampana},
    },
    seguridad::token_aleatorio,
};

pub(crate) const ACCION_CREAR_CAMPANA: AccionAuditoria =
    AccionAuditoria::new("publicidad.campana.crear");
pub(crate) const ACCION_EDITAR_CAMPANA: AccionAuditoria =
    AccionAuditoria::new("publicidad.campana.editar");
pub(crate) const ACCION_ESTADO_CAMPANA: AccionAuditoria =
    AccionAuditoria::new("publicidad.campana.cambiar_estado");
pub(crate) const ACCION_IMAGEN: AccionAuditoria = AccionAuditoria::new("publicidad.imagen.subir");
const ACCION_LOGO: AccionAuditoria = AccionAuditoria::new("publicidad.anunciante.logo");
const ACCION_CREATIVO: AccionAuditoria = AccionAuditoria::new("publicidad.creativo.guardar");
pub(crate) const ACCION_REDIMIR: AccionAuditoria = AccionAuditoria::new("publicidad.cupon.redimir");
pub(crate) const ACCION_ENLACE: AccionAuditoria =
    AccionAuditoria::new("publicidad.enlace_comercio.generar");
pub(crate) const ACCION_REVOCAR_ENLACE: AccionAuditoria =
    AccionAuditoria::new("publicidad.enlace_comercio.revocar");
pub(crate) const ACCION_EXCLUIR: AccionAuditoria =
    AccionAuditoria::new("publicidad.entidad.excluir");
pub(crate) const ACCION_INCLUIR: AccionAuditoria =
    AccionAuditoria::new("publicidad.entidad.incluir");

/// El comercio, si la cuenta tiene `plataforma.publicidad.gestionar`.
async fn anunciante_para<T: TxPublicidadPlataforma>(
    tx: &mut T,
    ctx: &ContextoPlataforma,
    id: AnuncianteId,
) -> AppResult<Anunciante> {
    ctx.exigir(Permiso::PlataformaPublicidadGestionar)?;
    tx.anunciante(id)
        .await?
        .ok_or(AppError::NoEncontrado("anunciante"))
}

/// La campaña del anunciante (bloqueada para cambiarla).
async fn campana_de<T: TxPublicidadPlataforma>(
    tx: &mut T,
    anunciante: AnuncianteId,
    id: CampanaId,
) -> AppResult<Campana> {
    tx.campana(id, true)
        .await?
        .filter(|c| c.anunciante_id == anunciante)
        .ok_or(AppError::NoEncontrado("campaña"))
}

// ---------------------------------------------------------------------------------------------
// Campañas
// ---------------------------------------------------------------------------------------------

/// Datos de una campaña.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatosCampana {
    pub nombre: String,
    pub objetivo: ObjetivoCampana,
    /// Una de las categorías de su ficha.
    pub categoria: CodigoCategoria,
    pub inicio: DateTime<Utc>,
    pub fin: DateTime<Utc>,
    pub modelo: ModeloCobro,
    pub presupuesto_total: Decimal,
    pub presupuesto_diario: Option<Decimal>,
    pub segmentacion: Segmentacion,
}

/// Valida los datos contra la ficha del anunciante.
async fn validar_campana<T: TxPublicidadPlataforma>(
    tx: &mut T,
    anunciante: AnuncianteId,
    d: DatosCampana,
    ahora: DateTime<Utc>,
) -> AppResult<(DatosCampana, Presupuesto)> {
    let nombre = texto_obligatorio("nombre", &d.nombre, 120)?;
    if d.fin <= d.inicio || d.fin <= ahora {
        return Err(AppError::Validacion(
            "la vigencia debe terminar después de empezar y en el futuro".into(),
        ));
    }
    let ficha = tx
        .ficha(anunciante)
        .await?
        .ok_or_else(|| AppError::Validacion("publique primero su ficha del directorio".into()))?;
    if !ficha.categorias.contains(&d.categoria) {
        return Err(AppError::Validacion(
            "la categoría de la campaña debe ser una de su ficha".into(),
        ));
    }
    let presupuesto = Presupuesto::new(d.presupuesto_total, d.presupuesto_diario)?;
    let segmentacion = d.segmentacion.validada()?;
    Ok((
        DatosCampana {
            nombre,
            segmentacion,
            ..d
        },
        presupuesto,
    ))
}

/// Crea una campaña en borrador.
///
/// # Errors
///
/// [`AppError::NoEncontrado`]; [`AppError::Validacion`] / [`AppError::Dominio`] si los datos
/// no son válidos (vigencia, presupuesto, segmentación, categoría que no es de su ficha).
pub async fn crear_campana<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
    datos: DatosCampana,
) -> AppResult<Campana>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    anunciante_para(&mut tx, ctx, anunciante).await?;
    let (d, presupuesto) = validar_campana(&mut tx, anunciante, datos, ahora).await?;
    let c = Campana {
        id: CampanaId::nuevo(),
        anunciante_id: anunciante,
        nombre: d.nombre,
        objetivo: d.objetivo,
        categoria: d.categoria,
        inicio: d.inicio,
        fin: d.fin,
        modelo: d.modelo,
        presupuesto,
        tarifa: None,
        consumido: Decimal::ZERO,
        segmentacion: d.segmentacion,
        conflicto_interes: false,
        estado: EstadoCampana::Borrador,
        motivo_estado: None,
        revisada_por: None,
        revisada_en: None,
        creada_por: ctx.actor_id,
        editada_por: None,
        creada_en: ahora,
        actualizada_en: ahora,
    };
    tx.insertar_campana(&c).await?;
    let detalle = DetalleAuditoria::vacio().con("modelo", c.modelo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREAR_CAMPANA, "campana", c.id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Edita la campaña. Una aprobada o pausada vuelve a borrador (deberá pasar otra vez por la
/// moderación: ampliar la segmentación puede cambiar el conflicto de interés); una activa hay
/// que pausarla primero.
///
/// # Errors
///
/// [`AppError::NoEncontrado`]; [`AppError::Dominio`] si el estado no lo permite; los de
/// [`crear_campana`].
pub async fn editar_campana<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
    id: CampanaId,
    datos: DatosCampana,
) -> AppResult<Campana>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    anunciante_para(&mut tx, ctx, anunciante).await?;
    let mut c = campana_de(&mut tx, anunciante, id).await?;
    if !c.estado.editable() {
        c.estado = c.estado.transicionar(EstadoCampana::Borrador)?;
    }
    let (d, presupuesto) = validar_campana(&mut tx, anunciante, datos, ahora).await?;
    if presupuesto.total < c.consumido {
        return Err(AppError::Validacion(
            "el presupuesto total no puede ser menor que lo ya consumido".into(),
        ));
    }
    c.nombre = d.nombre;
    c.objetivo = d.objetivo;
    c.categoria = d.categoria;
    c.inicio = d.inicio;
    c.fin = d.fin;
    c.modelo = d.modelo;
    c.presupuesto = presupuesto;
    c.segmentacion = d.segmentacion;
    c.tarifa = None;
    // Vuelve a moderación: quien la editó no la revisa, y la revisión anterior ya no vale.
    c.editada_por = ctx.actor_id;
    c.revisada_por = None;
    c.revisada_en = None;
    c.actualizada_en = ahora;
    tx.actualizar_campana(&c).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_EDITAR_CAMPANA,
        "campana",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Cambio de estado de una campaña que pide la plataforma.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccionCampana {
    /// Borrador → en revisión (calcula el conflicto de interés).
    EnviarARevision,
    /// Aprobada o pausada → activa (anunciante verificado y vigencia por delante).
    Activar,
    Pausar,
    Finalizar,
}

/// Envía a revisión, activa, pausa o finaliza la campaña.
///
/// # Errors
///
/// [`AppError::NoEncontrado`]; [`AppError::Dominio`] si la transición no está permitida;
/// [`AppError::Validacion`] si se envía sin creativos o con alguno rechazado, o se activa sin
/// estar verificado o con la vigencia vencida.
pub async fn cambiar_estado_campana<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
    id: CampanaId,
    accion: AccionCampana,
) -> AppResult<Campana>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let a = anunciante_para(&mut tx, ctx, anunciante).await?;
    let mut c = campana_de(&mut tx, anunciante, id).await?;
    let hacia = match accion {
        AccionCampana::EnviarARevision => EstadoCampana::EnRevision,
        AccionCampana::Activar => EstadoCampana::Activa,
        AccionCampana::Pausar => EstadoCampana::Pausada,
        AccionCampana::Finalizar => EstadoCampana::Finalizada,
    };
    let desde = c.estado;
    c.estado = desde.transicionar(hacia)?;
    match accion {
        AccionCampana::EnviarARevision => {
            let creativos = tx.creativos_de_campana(id).await?;
            if creativos.is_empty()
                || creativos
                    .iter()
                    .any(|x| x.estado == EstadoCreativo::Rechazado)
            {
                return Err(AppError::Validacion(
                    "la campaña necesita al menos un creativo y ninguno rechazado".into(),
                ));
            }
            c.conflicto_interes = tx.conflicto_interes(id).await?;
            c.motivo_estado = None;
        }
        AccionCampana::Activar => {
            if !a.estado.visible() {
                return Err(AppError::Validacion(
                    "el anunciante debe estar verificado para activar campañas".into(),
                ));
            }
            if c.fin <= ahora {
                return Err(AppError::Validacion(
                    "la vigencia de la campaña ya pasó".into(),
                ));
            }
        }
        AccionCampana::Pausar | AccionCampana::Finalizar => {}
    }
    c.actualizada_en = ahora;
    tx.actualizar_campana(&c).await?;
    let detalle = DetalleAuditoria::vacio()
        .con("desde", desde.as_str())?
        .con("hacia", hacia.as_str())?
        .con("conflicto_interes", c.conflicto_interes)?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_ESTADO_CAMPANA, "campana", id.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(c)
}

/// Campañas del anunciante.
///
/// # Errors
///
/// [`AppError::NoEncontrado`].
pub async fn campanas<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Campana>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    anunciante_para(&mut tx, ctx, anunciante).await?;
    tx.campanas_de_anunciante(anunciante, pagina).await
}

/// Campaña con sus creativos.
///
/// # Errors
///
/// [`AppError::NoEncontrado`].
pub async fn campana<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
    id: CampanaId,
) -> AppResult<(Campana, Vec<Creativo>)>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    anunciante_para(&mut tx, ctx, anunciante).await?;
    let c = tx
        .campana(id, false)
        .await?
        .filter(|c| c.anunciante_id == anunciante)
        .ok_or(AppError::NoEncontrado("campaña"))?;
    let creativos = tx.creativos_de_campana(id).await?;
    Ok((c, creativos))
}

/// Crea (`id = None`) o edita un creativo de una campaña editable. Cambiar el contenido de uno
/// aprobado o rechazado lo devuelve a moderación.
///
/// # Errors
///
/// [`AppError::NoEncontrado`]; [`AppError::Conflicto`] si la campaña no es editable;
/// [`AppError::Dominio`] si el contenido no es válido (Ley 1480 en promociones).
pub async fn guardar_creativo<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
    campana: CampanaId,
    id: Option<CreativoId>,
    contenido: ContenidoCreativo,
) -> AppResult<Creativo>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let contenido = contenido.validado()?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    anunciante_para(&mut tx, ctx, anunciante).await?;
    if let Some(imagen) = contenido.imagen {
        imagen_existente(&mut tx, imagen).await?;
    }
    let c = campana_de(&mut tx, anunciante, campana).await?;
    if !c.estado.editable() {
        return Err(AppError::Conflicto(
            "la campaña no es editable: edítela primero para volverla a borrador".into(),
        ));
    }
    let creativo = match id {
        None => {
            let n = Creativo {
                id: CreativoId::nuevo(),
                campana_id: campana,
                anunciante_id: anunciante,
                contenido,
                cupon_usos: 0,
                estado: EstadoCreativo::Pendiente,
                motivo_estado: None,
                cargado_por: ctx.actor_id,
                revisado_por: None,
                creado_en: ahora,
                actualizado_en: ahora,
            };
            tx.insertar_creativo(&n).await?;
            n
        }
        Some(id) => {
            let mut x = tx
                .creativo(id)
                .await?
                .filter(|x| x.campana_id == campana)
                .ok_or(AppError::NoEncontrado("creativo"))?;
            if x.contenido != contenido {
                if x.estado != EstadoCreativo::Pendiente {
                    x.estado = x.estado.transicionar(EstadoCreativo::Pendiente)?;
                }
                x.motivo_estado = None;
                x.contenido = contenido;
                x.cargado_por = ctx.actor_id;
                x.revisado_por = None;
            }
            x.actualizado_en = ahora;
            tx.actualizar_creativo(&x).await?;
            x
        }
    };
    let detalle = DetalleAuditoria::vacio().con("formato", creativo.contenido.formato.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_CREATIVO, "creativo", creativo.id.as_uuid())
            .con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(creativo)
}

// ---------------------------------------------------------------------------------------------
// Métricas, leads y cupones
// ---------------------------------------------------------------------------------------------

/// Métricas agregadas de una campaña: sin personas ni nombres de copropiedades.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricasCampana {
    /// Copropiedades que alcanza hoy.
    pub alcance: u64,
    pub totales: Conteo,
    pub consumido: Decimal,
    pub dias: Vec<MetricaCampana>,
}

/// Métricas de la campaña.
///
/// # Errors
///
/// [`AppError::NoEncontrado`].
pub async fn metricas<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
    id: CampanaId,
) -> AppResult<MetricasCampana>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    anunciante_para(&mut tx, ctx, anunciante).await?;
    let c = tx
        .campana(id, false)
        .await?
        .filter(|c| c.anunciante_id == anunciante)
        .ok_or(AppError::NoEncontrado("campaña"))?;
    let dias = tx.metricas_campana(id).await?;
    let totales = dias.iter().fold(Conteo::default(), |t, m| Conteo {
        impresiones: t.impresiones + m.conteo.impresiones,
        clics: t.clics + m.conteo.clics,
        redenciones: t.redenciones + m.conteo.redenciones,
        leads: t.leads + m.conteo.leads,
    });
    Ok(MetricasCampana {
        alcance: tx.alcance(id).await?,
        totales,
        consumido: c.consumido,
        dias,
    })
}

/// Contactos que los residentes autorizaron compartirle (nombre y celular, para esa
/// solicitud).
///
/// # Errors
///
/// [`AppError::NoEncontrado`].
pub async fn leads<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
    pagina: SolicitudPagina,
) -> AppResult<Pagina<Lead>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    anunciante_para(&mut tx, ctx, anunciante).await?;
    tx.leads(anunciante, pagina).await
}

/// Cupón redimido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redencion {
    pub creativo_id: CreativoId,
    pub titulo: String,
    pub usos: u32,
    /// `None` = sin límite.
    pub restantes: Option<u32>,
}

/// El comercio redime un cupón en el mostrador con **su enlace** (cabecera `X-Comercio`, sin
/// cuenta). Una oferta vigente se honra aunque el presupuesto ya no alcance para cobrarla (Ley
/// 1480): se cobra lo que cabe y la campaña se finaliza.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] si el enlace no existe o fue revocado; [`AppError::NoEncontrado`]
/// si el código no es de una oferta del comercio; [`AppError::Dominio`] si no está vigente o se
/// agotaron los usos.
pub async fn redimir_cupon<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    token_comercio: &str,
    codigo: &str,
) -> AppResult<Redencion>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let codigo = CodigoCupon::new(codigo)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let anunciante = tx
        .anunciante_por_enlace(&huella(token_comercio)?)
        .await?
        .ok_or(AppError::NoAutenticado)?;
    let creativo = tx
        .creativo_por_cupon(anunciante, &codigo)
        .await?
        .ok_or(AppError::NoEncontrado("cupón"))?;
    let c = tx
        .campana(creativo.campana_id, true)
        .await?
        .ok_or(AppError::NoEncontrado("campaña"))?;
    let cupon = creativo
        .contenido
        .cupon
        .clone()
        .ok_or(AppError::NoEncontrado("cupón"))?;
    let vigente = c.estado == EstadoCampana::Activa && c.inicio <= ahora && ahora < c.fin;
    validar_redencion(&cupon, creativo.cupon_usos, vigente)?;
    let tarifa = c.tarifa.unwrap_or(Decimal::ZERO);
    let disponible = c.presupuesto.disponible(c.consumido, Decimal::ZERO);
    let pedido = Conteo {
        redenciones: 1,
        ..Conteo::default()
    };
    let cobrado = ajustar_al_presupuesto(c.modelo, tarifa, pedido, disponible);
    let valor = costo(c.modelo, tarifa, cobrado);
    let finalizar = presupuesto_agotado(c.modelo, tarifa, &c.presupuesto, c.consumido + valor);
    tx.registrar_redencion(creativo.id, ahora, valor, finalizar)
        .await?;
    let usos = creativo.cupon_usos + 1;
    tx.registrar(EventoAuditoria::ok(
        ACCION_REDIMIR,
        "creativo",
        creativo.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(Redencion {
        creativo_id: creativo.id,
        titulo: creativo.contenido.titulo,
        usos,
        restantes: cupon.limite_usos.map(|l| l.saturating_sub(usos)),
    })
}

// ---------------------------------------------------------------------------------------------
// Enlace del comercio
// ---------------------------------------------------------------------------------------------

/// Contexto del cifrado del token de un comercio (no sirve para otro comercio ni otro uso).
fn contexto_enlace(anunciante: AnuncianteId) -> String {
    format!("enlace_comercio:{anunciante}")
}

/// Genera (o reemplaza: el anterior deja de servir) el enlace con que el comercio redime
/// cupones. Devuelve el token; el frontend arma el enlace.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.publicidad.gestionar`; [`AppError::NoEncontrado`];
/// [`AppError::Validacion`] si el servidor no tiene clave del Cofre.
pub async fn generar_enlace_comercio<UP, C>(
    unidad: &UP,
    cofre: &C,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
) -> AppResult<String>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
    C: Cofre,
{
    let token = token_aleatorio()?;
    let cifrado = cofre.cifrar(&contexto_enlace(anunciante), token.as_bytes())?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    anunciante_para(&mut tx, ctx, anunciante).await?;
    tx.guardar_enlace_comercio(
        anunciante,
        &huella(&token)?,
        &cifrado,
        ctx.actor_id,
        reloj.ahora(),
    )
    .await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_ENLACE,
        "anunciante",
        anunciante.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(token)
}

/// El token vigente del enlace del comercio, sin cambiarlo; `None` si no tiene.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`]; los del Cofre.
pub async fn enlace_comercio<UP, C>(
    unidad: &UP,
    cofre: &C,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
) -> AppResult<Option<String>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
    C: Cofre,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    anunciante_para(&mut tx, ctx, anunciante).await?;
    tx.enlace_comercio(anunciante)
        .await?
        .map(|cifrado| {
            let claro = cofre.descifrar(&contexto_enlace(anunciante), &cifrado)?;
            String::from_utf8(claro).map_err(AppError::infra)
        })
        .transpose()
}

/// Revoca el enlace del comercio (deja de redimir hasta que se genere otro).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`] si no existe o no tenía enlace.
pub async fn revocar_enlace_comercio<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    anunciante_para(&mut tx, ctx, anunciante).await?;
    if !tx.borrar_enlace_comercio(anunciante).await? {
        return Err(AppError::NoEncontrado("enlace"));
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_REVOCAR_ENLACE,
        "anunciante",
        anunciante.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

// ---------------------------------------------------------------------------------------------
// Exclusiones
// ---------------------------------------------------------------------------------------------

/// Excluye una entidad de la publicidad (no recibe piezas ni destacadas; el directorio sigue).
/// Solo la plataforma decide: la entidad no controla la publicidad.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.publicidad.gestionar`; [`AppError::Validacion`] sin
/// motivo; [`AppError::NoEncontrado`] si la entidad no existe.
pub async fn excluir_entidad<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    tenant: TenantId,
    motivo: &str,
) -> AppResult<ExclusionPublicidad>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    ctx.exigir(Permiso::PlataformaPublicidadGestionar)?;
    let exclusion = ExclusionPublicidad {
        tenant_id: tenant,
        motivo: super::motivo(motivo)?,
        creada_por: ctx.actor_id,
        creada_en: reloj.ahora(),
    };
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    tx.excluir_de_publicidad(&exclusion).await?;
    let detalle = DetalleAuditoria::vacio().con("motivo", exclusion.motivo.as_str())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_EXCLUIR, "entidad", tenant.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(exclusion)
}

/// Vuelve a incluir una entidad en la publicidad.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`] si no estaba excluida.
pub async fn incluir_entidad<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    tenant: TenantId,
) -> AppResult<()>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    ctx.exigir(Permiso::PlataformaPublicidadGestionar)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    if !tx.quitar_exclusion(tenant).await? {
        return Err(AppError::NoEncontrado("exclusión"));
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_INCLUIR,
        "entidad",
        tenant.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Entidades excluidas de la publicidad.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.publicidad.gestionar`.
pub async fn exclusiones<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<ExclusionPublicidad>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    ctx.exigir(Permiso::PlataformaPublicidadGestionar)?;
    unidad.iniciar_plataforma(ctx).await?.exclusiones().await
}

// ---------------------------------------------------------------------------------------------
// Imágenes de la publicidad (creativos y logos)
// ---------------------------------------------------------------------------------------------

/// Vigencia del enlace firmado de una imagen: alcanza para que la app o la pantalla del lobby la
/// descarguen y la guarden en su caché.
pub const VIGENCIA_ENLACE_IMAGEN: chrono::Duration = chrono::Duration::hours(1);

async fn imagen_existente<T: TxPublicidadPlataforma>(
    tx: &mut T,
    id: ArchivoId,
) -> AppResult<Archivo> {
    tx.archivo_plataforma(id)
        .await?
        .ok_or(AppError::NoEncontrado("imagen"))
}

/// Sube una imagen de la publicidad (para un creativo o el logo de un comercio): JPEG, PNG o
/// WebP de hasta 2 MB, validada por su contenido. Queda en la carpeta de la plataforma del
/// almacén; el nombre original no va a la auditoría.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.publicidad.gestionar` ni
/// `plataforma.directorio.gestionar`; [`AppError::Dominio`] si no es una imagen admitida o pasa
/// del tamaño; [`AppError::Infraestructura`] si falla el almacén.
pub async fn subir_imagen<UP, A>(
    unidad: &UP,
    almacen: &A,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    nombre: NombreArchivo,
    contenido: Vec<u8>,
) -> AppResult<Archivo>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
    A: AlmacenArchivos,
{
    if ctx.exigir(Permiso::PlataformaPublicidadGestionar).is_err() {
        ctx.exigir(Permiso::PlataformaDirectorioGestionar)?;
    }
    let tipo = TipoArchivo::validar_imagen_publicidad(&contenido)?;
    let id = ArchivoId::nuevo();
    let archivo = Archivo {
        id,
        clave: ClaveArchivo::de_plataforma(id),
        almacen: almacen.almacen(),
        nombre_original: nombre,
        tipo,
        tamano_bytes: contenido.len() as u64,
        sha256: sha256_hex(&contenido),
        clasificacion: ClasificacionArchivo::Publico,
        subido_por: ctx.actor_id,
        creado_en: reloj.ahora(),
    };
    almacen.guardar(&archivo.clave, &contenido).await?;
    let resultado = async {
        let mut tx = unidad.iniciar_plataforma(ctx).await?;
        tx.insertar_archivo_plataforma(&archivo).await?;
        let detalle = DetalleAuditoria::vacio()
            .con("tipo", tipo.as_str())?
            .con("tamano_bytes", archivo.tamano_bytes)?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_IMAGEN, "archivo_plataforma", id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
        tx.confirmar().await
    }
    .await;
    if let Err(e) = resultado {
        // Sin la fila, los bytes quedarían huérfanos: se borran (si falla, solo se registra).
        if let Err(borrado) = almacen.eliminar(&archivo.clave).await {
            tracing::warn!(error = %borrado, "no se pudo borrar una imagen huérfana");
        }
        return Err(e);
    }
    Ok(archivo)
}

/// Pone (o quita, con `None`) el logo de un comercio: una imagen ya subida con [`subir_imagen`].
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `plataforma.directorio.gestionar`; [`AppError::NoEncontrado`]
/// si el comercio o la imagen no existen.
pub async fn poner_logo<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
    logo: Option<ArchivoId>,
) -> AppResult<Anunciante>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    ctx.exigir(Permiso::PlataformaDirectorioGestionar)?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let mut a = tx
        .anunciante_para_actualizar(anunciante)
        .await?
        .ok_or(AppError::NoEncontrado("anunciante"))?;
    if let Some(id) = logo {
        imagen_existente(&mut tx, id).await?;
    }
    a.logo = logo;
    a.actualizado_en = reloj.ahora();
    tx.actualizar_anunciante(&a).await?;
    let detalle = DetalleAuditoria::vacio().con("con_logo", logo.is_some())?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_LOGO, "anunciante", anunciante.as_uuid()).con_detalle(detalle),
    )
    .await?;
    tx.confirmar().await?;
    Ok(a)
}

/// Enlace firmado de una imagen de la publicidad, para que la app, la web o la pantalla del
/// lobby la descarguen sin sesión durante [`VIGENCIA_ENLACE_IMAGEN`]. Quien llama ya decidió que
/// la pieza se puede mostrar.
///
/// # Errors
///
/// [`AppError::Infraestructura`] si no se puede firmar.
pub fn enlace_imagen(
    almacen: &impl AlmacenArchivos,
    reloj: &impl Reloj,
    id: ArchivoId,
    tipo: TipoArchivo,
) -> AppResult<String> {
    almacen.url_temporal(&DescargaArchivo {
        clave: ClaveArchivo::de_plataforma(id),
        tipo,
        nombre: NombreArchivo::new(&format!("imagen.{}", tipo.extension()))?,
        vence_en: reloj.ahora() + VIGENCIA_ENLACE_IMAGEN,
    })
}
