//! Portal del anunciante (plan 08, fase 3): lo usan las cuentas miembros de la organización
//! anunciante, con su papel ([`domain::publicidad::permite`]): dueño y gestor administran todo;
//! el operario solo redime cupones. A quien no es miembro se le responde «no encontrado».
//!
//! Lo que ve el anunciante nunca incluye datos de residentes, salvo los contactos que un
//! residente autorizó expresamente compartirle para una solicitud ([`leads`]).

use chrono::{DateTime, Utc};
use domain::{
    AnuncianteId, CampanaId, CodigoCategoria, CodigoCupon, CreativoId, EstadoCampana,
    EstadoCreativo, EstadoVerificacion, ModeloCobro, ObjetivoCampana, RolOrganizacion,
    Segmentacion,
    copropiedad::texto_obligatorio,
    publicidad::{
        AccionAnunciante, ContenidoCreativo, Conteo, Presupuesto, ajustar_al_presupuesto, costo,
        permite, presupuesto_agotado, validar_redencion,
    },
};
use rust_decimal::Decimal;

use super::TxPublicidadPlataforma;
use crate::puertos::{
    RegistroAuditoria, RepositorioDirectorioPlataforma, RepositorioPublicidadPlataforma,
    identidad::RepositorioOrganizaciones,
};
use crate::{
    AppError, AppResult, ContextoPlataforma, Pagina, SolicitudPagina,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{
        directorio::{DatosAnunciante, DatosFicha, guardar_ficha_en},
        organizaciones::validar_nueva,
    },
    puertos::{
        Reloj, Transaccion, UnidadDeTrabajoPlataforma,
        directorio::{Anunciante, Ficha, OrganizacionAnunciante},
        identidad::NuevaOrganizacion,
        publicidad::{Campana, Creativo, Lead, MetricaCampana},
    },
};

pub(crate) const ACCION_REGISTRAR: AccionAuditoria =
    AccionAuditoria::new("publicidad.anunciante.registrar");
pub(crate) const ACCION_FICHA: AccionAuditoria = AccionAuditoria::new("publicidad.ficha.editar");
pub(crate) const ACCION_CREAR_CAMPANA: AccionAuditoria =
    AccionAuditoria::new("publicidad.campana.crear");
pub(crate) const ACCION_EDITAR_CAMPANA: AccionAuditoria =
    AccionAuditoria::new("publicidad.campana.editar");
pub(crate) const ACCION_ESTADO_CAMPANA: AccionAuditoria =
    AccionAuditoria::new("publicidad.campana.cambiar_estado");
pub(crate) const ACCION_CREATIVO: AccionAuditoria =
    AccionAuditoria::new("publicidad.creativo.guardar");
pub(crate) const ACCION_REDIMIR: AccionAuditoria = AccionAuditoria::new("publicidad.cupon.redimir");

/// El anunciante, si la cuenta es miembro de su organización con un papel que permite la
/// acción.
async fn anunciante_para<T: TxPublicidadPlataforma>(
    tx: &mut T,
    ctx: &ContextoPlataforma,
    id: AnuncianteId,
    accion: AccionAnunciante,
) -> AppResult<Anunciante> {
    let no = AppError::NoEncontrado("anunciante");
    let a = tx.anunciante(id).await?.ok_or(no)?;
    match tx.rol_miembro(a.organizacion_id, ctx.cuenta()?).await? {
        Some(rol) if permite(rol, accion) => Ok(a),
        _ => Err(AppError::NoEncontrado("anunciante")),
    }
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
// Registro y ficha
// ---------------------------------------------------------------------------------------------

/// Registra la cuenta como anunciante: crea la organización (y la deja como dueña) o usa la
/// existente si ya es su dueña o gestora. El anunciante queda **pendiente** hasta que la
/// moderación lo verifique.
///
/// # Errors
///
/// [`AppError::NoAutenticado`]; [`AppError::Validacion`] / [`AppError::Dominio`] si los datos
/// no son válidos; [`AppError::Conflicto`] si la organización ya existe y la cuenta no la
/// administra (debe reclamarla) o ya tiene un anunciante.
pub async fn registrarse<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    organizacion: NuevaOrganizacion,
    datos: DatosAnunciante,
) -> AppResult<Anunciante>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let cuenta = ctx.cuenta()?;
    validar_nueva(&organizacion)?;
    let datos = datos.validados()?;
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    if let Some(existente) = tx.organizacion_por_nit(&organizacion.nit).await? {
        let rol = tx.rol_miembro(existente, cuenta).await?;
        if !rol.is_some_and(RolOrganizacion::edita_ficha) {
            return Err(AppError::Conflicto(
                "la organización ya está registrada: pida a quien la administra que lo agregue \
                 o reclámela"
                    .into(),
            ));
        }
    }
    let organizacion_id = tx
        .organizacion_para_anunciante(&OrganizacionAnunciante::Nueva(organizacion), Some(cuenta))
        .await?
        .ok_or(AppError::NoEncontrado("organización"))?;
    if tx.rol_miembro(organizacion_id, cuenta).await?.is_none() {
        tx.guardar_miembro(organizacion_id, cuenta, RolOrganizacion::Dueno)
            .await?;
    }
    let ahora = reloj.ahora();
    let anunciante = Anunciante {
        id: AnuncianteId::nuevo(),
        organizacion_id,
        nombre_comercial: datos.nombre_comercial,
        direccion: datos.direccion,
        ubicacion: datos.ubicacion,
        estado: EstadoVerificacion::Pendiente,
        motivo_estado: None,
        creado_por: Some(cuenta),
        creado_en: ahora,
        actualizado_en: ahora,
    };
    tx.insertar_anunciante(&anunciante).await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_REGISTRAR,
        "anunciante",
        anunciante.id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(anunciante)
}

/// Anunciantes de la cuenta con su papel.
///
/// # Errors
///
/// [`AppError::NoAutenticado`].
pub async fn mis_anunciantes<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
) -> AppResult<Vec<(Anunciante, RolOrganizacion)>>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let cuenta = ctx.cuenta()?;
    unidad
        .iniciar_plataforma(ctx)
        .await?
        .anunciantes_de_cuenta(cuenta)
        .await
}

/// El anunciante con su ficha (cualquier miembro).
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si no existe o la cuenta no es miembro.
pub async fn mi_anunciante<UP>(
    unidad: &UP,
    ctx: &ContextoPlataforma,
    id: AnuncianteId,
) -> AppResult<(Anunciante, Option<Ficha>)>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    let a = anunciante_para(&mut tx, ctx, id, AccionAnunciante::RedimirCupones).await?;
    let ficha = tx.ficha(id).await?;
    Ok((a, ficha))
}

/// El anunciante edita su ficha del directorio. La fecha de destacada no la decide él (es una
/// suscripción que gestiona la plataforma): se conserva la vigente.
///
/// # Errors
///
/// [`AppError::NoEncontrado`]; los de la validación de la ficha (categorías restringidas,
/// cobertura, contacto).
pub async fn editar_ficha<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    id: AnuncianteId,
    datos: DatosFicha,
) -> AppResult<Ficha>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    anunciante_para(&mut tx, ctx, id, AccionAnunciante::Gestionar).await?;
    let destacada_hasta = tx.ficha(id).await?.and_then(|f| f.destacada_hasta);
    let ficha = guardar_ficha_en(
        &mut tx,
        reloj.ahora(),
        id,
        DatosFicha {
            destacada_hasta,
            ..datos
        },
    )
    .await?;
    tx.registrar(EventoAuditoria::ok(
        ACCION_FICHA,
        "anunciante",
        id.as_uuid(),
    ))
    .await?;
    tx.confirmar().await?;
    Ok(ficha)
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
    anunciante_para(&mut tx, ctx, anunciante, AccionAnunciante::Gestionar).await?;
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
    anunciante_para(&mut tx, ctx, anunciante, AccionAnunciante::Gestionar).await?;
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

/// Cambio de estado que pide el anunciante.
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
    let a = anunciante_para(&mut tx, ctx, anunciante, AccionAnunciante::Gestionar).await?;
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
    anunciante_para(&mut tx, ctx, anunciante, AccionAnunciante::VerMetricas).await?;
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
    anunciante_para(&mut tx, ctx, anunciante, AccionAnunciante::VerMetricas).await?;
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
    anunciante_para(&mut tx, ctx, anunciante, AccionAnunciante::Gestionar).await?;
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
    anunciante_para(&mut tx, ctx, anunciante, AccionAnunciante::VerMetricas).await?;
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
    anunciante_para(&mut tx, ctx, anunciante, AccionAnunciante::VerLeads).await?;
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

/// Redime un cupón en el mostrador. Una oferta vigente se honra aunque el presupuesto ya no
/// alcance para cobrarla (Ley 1480): se cobra lo que cabe y la campaña se finaliza.
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si el código no es de una oferta del anunciante;
/// [`AppError::Dominio`] si no está vigente o se agotaron los usos.
pub async fn redimir_cupon<UP>(
    unidad: &UP,
    reloj: &impl Reloj,
    ctx: &ContextoPlataforma,
    anunciante: AnuncianteId,
    codigo: &str,
) -> AppResult<Redencion>
where
    UP: UnidadDeTrabajoPlataforma,
    UP::Transaccion: TxPublicidadPlataforma,
{
    let codigo = CodigoCupon::new(codigo)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar_plataforma(ctx).await?;
    anunciante_para(&mut tx, ctx, anunciante, AccionAnunciante::RedimirCupones).await?;
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
