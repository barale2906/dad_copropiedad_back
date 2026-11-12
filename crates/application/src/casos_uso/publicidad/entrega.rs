//! Entrega de la publicidad en una copropiedad (plan 08, fases 4 a 6).
//!
//! Invariantes del plan 08 que se cumplen aquí:
//! - **Solo a quien autorizó**: sin la autorización `publicidad` vigente (frente a la
//!   plataforma) la app no recibe piezas y no se encola ningún mensaje (invariante 2).
//! - Sin el módulo `publicidad` habilitado en la PH no hay piezas ni mensajes (invariante 3:
//!   lo filtra `piezas_publicidad`).
//! - Ninguna pieza sin anunciante verificado, creativo aprobado y campaña activa y vigente
//!   (invariante 4, [`domain::publicidad::PiezaCandidata::entregable`]).
//! - Prioridad 9 del outbox (invariante 5) y topes de presupuesto y frecuencia aun con
//!   entregas concurrentes (invariante 6: la campaña se bloquea al registrar).
//!
//! Los eventos son agregados (impresiones, clics por creativo, PH, canal y día): no se guarda
//! quién vio o tocó una pieza. Por eso no van a la auditoría, que sería un registro por
//! persona.

use chrono::{DateTime, Utc};
use domain::{
    CanalPublicidad, CreativoId, FinalidadTratamiento, FormatoCreativo, MedioAutorizacion, Permiso,
    PersonaId, ReportePublicidadId, TenantId,
    copropiedad::texto_opcional,
    publicidad::{Conteo, ajustar_al_presupuesto, costo, presupuesto_agotado, seleccionar_piezas},
};

use super::TxPublicidad;
use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::avisos::{IDIOMA_PLANTILLAS, parametro},
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        DirectorioPublicidad, RegistroAuditoria, Reloj, RepositorioProteccionDatos,
        RepositorioPublicidad, RepositorioTickets, Transaccion, UnidadDeTrabajo,
        correo::CorreoSaliente,
        mensajeria::{ContenidoSaliente, MensajeSaliente},
        publicidad::{PiezaPublicidad, PoliticaPublicidad, ReportePublicidad},
    },
    seguridad::{hash_secreto, iguales},
};

pub(crate) const ACCION_AUTORIZAR: AccionAuditoria =
    AccionAuditoria::new("publicidad.autorizacion.otorgar");
pub(crate) const ACCION_REVOCAR: AccionAuditoria =
    AccionAuditoria::new("publicidad.autorizacion.revocar");
pub(crate) const ACCION_REPORTAR: AccionAuditoria =
    AccionAuditoria::new("publicidad.anuncio.reportar");

/// Piezas que se muestran a la vez en la app.
pub const MAXIMO_PIEZAS: usize = 3;
/// Eventos que acepta una sola petición (por pieza y contador).
pub const MAXIMO_EVENTOS: u32 = 100;
/// Mensajes masivos por copropiedad y campaña en cada pasada del worker.
pub const LOTE_MASIVO: u32 = 200;

/// Formatos que se muestran en la app y la web.
const FORMATOS_APP: [FormatoCreativo; 3] = [
    FormatoCreativo::Banner,
    FormatoCreativo::TarjetaDestacada,
    FormatoCreativo::OfertaCupon,
];

/// Botón «No recibir más» de los mensajes de WhatsApp: revoca la autorización de publicidad
/// de la persona en esa copropiedad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotonBaja {
    pub tenant: TenantId,
}

impl BotonBaja {
    const PREFIJO: &'static str = "baja:";

    /// Id del botón (`baja:{tenant}`).
    pub fn id(&self) -> String {
        format!("{}{}", Self::PREFIJO, self.tenant)
    }

    /// Botón a partir de su id; `None` si no es un botón de baja válido.
    pub fn desde_id(id: &str) -> Option<Self> {
        Some(Self {
            tenant: TenantId::from_uuid(id.strip_prefix(Self::PREFIJO)?.parse().ok()?),
        })
    }
}

/// La persona del censo de la cuenta y si autorizó la publicidad.
async fn persona_autorizada<T: TxPublicidad>(
    tx: &mut T,
    ctx: &Contexto,
) -> AppResult<Option<PersonaId>> {
    let Some(cuenta) = ctx.actor_id else {
        return Ok(None);
    };
    let Some(persona) = tx.persona_de_cuenta(cuenta).await? else {
        return Ok(None);
    };
    let autorizada = tx
        .finalidades_autorizadas(persona)
        .await?
        .contains(&FinalidadTratamiento::Publicidad);
    Ok(autorizada.then_some(persona))
}

// ---------------------------------------------------------------------------------------------
// Autorización del titular (frente a la plataforma)
// ---------------------------------------------------------------------------------------------

/// Estado de la autorización de publicidad de la cuenta en la copropiedad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstadoAutorizacion {
    pub autorizada: bool,
    /// Política de la plataforma que se acepta al autorizar.
    pub politica: Option<PoliticaPublicidad>,
}

async fn persona_de_la_cuenta<T: TxPublicidad>(tx: &mut T, ctx: &Contexto) -> AppResult<PersonaId> {
    let cuenta = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    tx.persona_de_cuenta(cuenta).await?.ok_or_else(|| {
        AppError::Validacion(
            "su cuenta no está vinculada a una persona del censo de esta copropiedad".into(),
        )
    })
}

/// ¿La cuenta autorizó la publicidad? Con la política vigente de la plataforma.
///
/// # Errors
///
/// [`AppError::NoAutenticado`]; [`AppError::Validacion`] si la cuenta no está en el censo.
pub async fn autorizacion_publicidad<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<EstadoAutorizacion>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
{
    let mut tx = unidad.iniciar(ctx).await?;
    let persona = persona_de_la_cuenta(&mut tx, ctx).await?;
    let autorizada = tx
        .finalidades_autorizadas(persona)
        .await?
        .contains(&FinalidadTratamiento::Publicidad);
    Ok(EstadoAutorizacion {
        autorizada,
        politica: tx.politica_publicidad_vigente(reloj.ahora()).await?,
    })
}

/// El titular autoriza recibir publicidad de la plataforma aceptando la versión vigente de su
/// política (autorización previa, expresa e informada, Ley 1581). Solo el titular, nunca la
/// administración de la PH. `false` si ya estaba autorizada.
///
/// # Errors
///
/// [`AppError::NoAutenticado`]; [`AppError::Validacion`] si no está en el censo, no hay
/// política vigente o la versión aceptada no es la vigente.
pub async fn autorizar_publicidad<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    version_aceptada: u32,
    medio: MedioAutorizacion,
) -> AppResult<bool>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
{
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    let persona = persona_de_la_cuenta(&mut tx, ctx).await?;
    let politica = tx
        .politica_publicidad_vigente(ahora)
        .await?
        .filter(|p| p.version == version_aceptada)
        .ok_or_else(|| {
            AppError::Validacion("acepte la versión vigente de la política de publicidad".into())
        })?;
    let nueva = tx
        .autorizar_publicidad(persona, politica.id, medio, ahora)
        .await?;
    if nueva {
        let detalle = DetalleAuditoria::vacio()
            .con("politica_version", politica.version)?
            .con("medio", medio.as_str())?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_AUTORIZAR, "persona", persona.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
    }
    tx.confirmar().await?;
    Ok(nueva)
}

/// Revoca la autorización de publicidad de la persona (desde la app, la cuenta; desde el botón
/// «No recibir más» de WhatsApp, el sistema con la persona del vínculo). `false` si no había.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] / [`AppError::Validacion`] si una cuenta no está en el censo.
pub async fn revocar_publicidad<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    persona: Option<PersonaId>,
) -> AppResult<bool>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
{
    let mut tx = unidad.iniciar(ctx).await?;
    let persona = match (persona, ctx.actor_id) {
        // Sin cuenta (chat de WhatsApp): la persona la identificó su teléfono.
        (Some(p), None) => p,
        _ => persona_de_la_cuenta(&mut tx, ctx).await?,
    };
    let n = tx
        .revocar_autorizaciones(persona, &[FinalidadTratamiento::Publicidad], reloj.ahora())
        .await?;
    if n > 0 {
        tx.registrar(EventoAuditoria::ok(
            ACCION_REVOCAR,
            "persona",
            persona.as_uuid(),
        ))
        .await?;
    }
    tx.confirmar().await?;
    Ok(n > 0)
}

// ---------------------------------------------------------------------------------------------
// Piezas y eventos (app, web y pantalla)
// ---------------------------------------------------------------------------------------------

/// Piezas para el residente que autorizó la publicidad (vacío si no autorizó o la PH no tiene
/// el módulo).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `directorio.ver`.
pub async fn piezas_para_residente<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<Vec<PiezaPublicidad>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    if persona_autorizada(&mut tx, ctx).await?.is_none() {
        return Ok(Vec::new());
    }
    let piezas = tx.piezas(&FORMATOS_APP, ahora).await?;
    Ok(elegir(piezas, ahora, MAXIMO_PIEZAS))
}

/// Las piezas que elige el dominio (rotación, una por campaña), en ese orden.
fn elegir(
    piezas: Vec<PiezaPublicidad>,
    ahora: DateTime<Utc>,
    maximo: usize,
) -> Vec<PiezaPublicidad> {
    let elegidas = seleccionar_piezas(
        piezas.iter().map(|p| p.candidata.clone()).collect(),
        ahora,
        maximo,
    );
    elegidas
        .iter()
        .filter_map(|c| piezas.iter().find(|p| p.candidata.creativo == c.creativo))
        .cloned()
        .collect()
}

/// Eventos de una pieza que reporta el cliente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventoPieza {
    pub creativo: CreativoId,
    pub impresiones: u32,
    pub clics: u32,
}

/// Registra una entrega: bloquea la campaña, recorta a lo que cabe en el presupuesto, la
/// valora y la finaliza si se agotó. Devuelve lo aceptado (vacío si la pieza no se puede
/// entregar en la PH o por ese canal).
async fn entregar<T: TxPublicidad>(
    tx: &mut T,
    creativo: CreativoId,
    canal: CanalPublicidad,
    pedido: Conteo,
    ahora: DateTime<Utc>,
) -> AppResult<Conteo> {
    let Some((p, formato)) = tx.pieza_para_entrega(creativo, ahora).await? else {
        return Ok(Conteo::default());
    };
    if formato.canal() != canal || !p.entregable(ahora) {
        return Ok(Conteo::default());
    }
    let disponible = p.presupuesto.disponible(p.consumido_total, p.consumido_hoy);
    let aceptado = ajustar_al_presupuesto(p.modelo, p.tarifa, pedido, disponible);
    if aceptado.vacio() {
        return Ok(aceptado);
    }
    let valor = costo(p.modelo, p.tarifa, aceptado);
    let finalizar = presupuesto_agotado(
        p.modelo,
        p.tarifa,
        &p.presupuesto,
        p.consumido_total + valor,
    );
    tx.aplicar_entrega(creativo, canal, ahora, aceptado, valor, finalizar)
        .await?;
    Ok(aceptado)
}

/// Registra impresiones y clics que reporta la app (agregados, sin identificar a nadie). Solo
/// de residentes que autorizaron la publicidad; a lo sumo [`MAXIMO_EVENTOS`] por contador.
/// Devuelve los eventos aceptados.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `directorio.ver`; [`AppError::Validacion`] si un contador
/// supera el máximo.
pub async fn registrar_eventos<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    eventos: &[EventoPieza],
) -> AppResult<Conteo>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    if eventos.len() > MAXIMO_PIEZAS * 10
        || eventos
            .iter()
            .any(|e| e.impresiones > MAXIMO_EVENTOS || e.clics > MAXIMO_EVENTOS)
    {
        return Err(AppError::Validacion(
            "demasiados eventos en una petición".into(),
        ));
    }
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    if persona_autorizada(&mut tx, ctx).await?.is_none() {
        return Ok(Conteo::default());
    }
    let mut total = Conteo::default();
    for e in eventos {
        let pedido = Conteo {
            impresiones: e.impresiones,
            clics: e.clics,
            ..Conteo::default()
        };
        let a = entregar(&mut tx, e.creativo, CanalPublicidad::App, pedido, ahora).await?;
        total.impresiones += a.impresiones;
        total.clics += a.clics;
    }
    tx.confirmar().await?;
    Ok(total)
}

/// Cupón de una oferta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuponObtenido {
    pub codigo: String,
    pub condiciones: Option<String>,
    pub vence_en: DateTime<Utc>,
    pub nombre_comercial: String,
}

/// El residente autorizado obtiene el código de una oferta (cuenta como clic). El comercio lo
/// redime en su mostrador.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`] si la oferta no se muestra en la PH o
/// la persona no autorizó la publicidad.
pub async fn obtener_cupon<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    creativo: CreativoId,
) -> AppResult<CuponObtenido>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(ctx).await?;
    let no = || AppError::NoEncontrado("oferta");
    persona_autorizada(&mut tx, ctx).await?.ok_or_else(no)?;
    let pieza = tx
        .piezas(&[FormatoCreativo::OfertaCupon], ahora)
        .await?
        .into_iter()
        .find(|p| p.candidata.creativo == creativo && p.candidata.entregable(ahora))
        .ok_or_else(no)?;
    let cupon = pieza.contenido.cupon.clone().ok_or_else(no)?;
    let clic = Conteo {
        clics: 1,
        ..Conteo::default()
    };
    entregar(&mut tx, creativo, CanalPublicidad::App, clic, ahora).await?;
    tx.confirmar().await?;
    Ok(CuponObtenido {
        codigo: cupon.codigo.as_str().to_owned(),
        condiciones: pieza.contenido.condiciones,
        vence_en: pieza.candidata.fin,
        nombre_comercial: pieza.nombre_comercial,
    })
}

/// Un residente reporta un anuncio (publicidad engañosa, ofensiva…) para la moderación.
/// `false` si ya lo había reportado.
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoAutenticado`]; [`AppError::Validacion`] sin motivo.
pub async fn reportar_anuncio<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    creativo: CreativoId,
    motivo: &str,
) -> AppResult<bool>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
{
    ctx.exigir(Permiso::DirectorioVer)?;
    let cuenta = ctx.actor_id.ok_or(AppError::NoAutenticado)?;
    let motivo = texto_opcional("motivo", Some(motivo), 500)?
        .ok_or_else(|| AppError::Validacion("indique por qué reporta el anuncio".into()))?;
    let mut tx = unidad.iniciar(ctx).await?;
    let id = ReportePublicidadId::nuevo();
    let nuevo = tx
        .insertar_reporte_publicidad(&ReportePublicidad {
            id,
            creativo_id: creativo,
            cuenta_id: cuenta,
            motivo,
            creado_en: reloj.ahora(),
        })
        .await?;
    if nuevo {
        let detalle = DetalleAuditoria::vacio().con("creativo_id", creativo.to_string())?;
        tx.registrar(
            EventoAuditoria::ok(ACCION_REPORTAR, "reporte_publicidad", id.as_uuid())
                .con_detalle(detalle),
        )
        .await?;
    }
    tx.confirmar().await?;
    Ok(nuevo)
}

/// Piezas de la pantalla del lobby (sin sesión: el token de la pantalla es la credencial). Una
/// cartelera pública no trata datos de nadie: cada pieza mostrada cuenta como una impresión.
///
/// # Errors
///
/// [`AppError::NoAutenticado`] si el token no es el de la pantalla de la copropiedad.
pub async fn piezas_pantalla<U>(
    unidad: &U,
    reloj: &impl Reloj,
    tenant: TenantId,
    token: &str,
) -> AppResult<Vec<PiezaPublicidad>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
{
    let ctx = Contexto::new(tenant);
    let ahora = reloj.ahora();
    let mut tx = unidad.iniciar(&ctx).await?;
    let esperado = tx.pantalla().await?.ok_or(AppError::NoAutenticado)?;
    if !iguales(
        &hex::encode(&esperado.token_hash),
        &hash_secreto(token.trim()),
    ) {
        return Err(AppError::NoAutenticado);
    }
    let piezas = tx.piezas(&[FormatoCreativo::PantallaLobby], ahora).await?;
    let elegidas = elegir(piezas, ahora, MAXIMO_PIEZAS);
    let mut mostradas = Vec::new();
    for p in elegidas {
        let vista = Conteo {
            impresiones: 1,
            ..Conteo::default()
        };
        let a = entregar(
            &mut tx,
            p.candidata.creativo,
            CanalPublicidad::Pantalla,
            vista,
            ahora,
        )
        .await?;
        if a.impresiones == 1 {
            mostradas.push(p);
        }
    }
    tx.confirmar().await?;
    Ok(mostradas)
}

// ---------------------------------------------------------------------------------------------
// Envíos masivos (worker)
// ---------------------------------------------------------------------------------------------

/// Resultado de una pasada del worker de envíos masivos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResultadoMasivo {
    pub copropiedades: usize,
    pub mensajes: u32,
}

/// Envía las campañas de WhatsApp y correo de cada copropiedad con el módulo, solo a quienes
/// autorizaron la publicidad, con el tope de frecuencia y dentro del presupuesto (cada mensaje
/// cuenta como impresión), con prioridad 9. Cada copropiedad en su transacción.
///
/// # Errors
///
/// Solo si falla la lista de copropiedades; una copropiedad que falla se registra y se sigue.
pub async fn enviar_masivos<D, U>(
    directorio: &D,
    unidad: &U,
    reloj: &impl Reloj,
) -> AppResult<ResultadoMasivo>
where
    D: DirectorioPublicidad,
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
{
    let ahora = reloj.ahora();
    let mut r = ResultadoMasivo::default();
    for tenant in directorio.copropiedades_con_publicidad(ahora).await? {
        match masivos_de(unidad, tenant, ahora).await {
            Ok(n) => {
                r.copropiedades += 1;
                r.mensajes += n;
            }
            Err(e) => {
                tracing::warn!(tenant_id = %tenant, error = %e, "no se pudo enviar la publicidad");
            }
        }
    }
    Ok(r)
}

async fn masivos_de<U>(unidad: &U, tenant: TenantId, ahora: DateTime<Utc>) -> AppResult<u32>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxPublicidad,
{
    let ctx = Contexto::new(tenant);
    let mut tx = unidad.iniciar(&ctx).await?;
    let piezas = tx
        .piezas(
            &[
                FormatoCreativo::MensajeWhatsapp,
                FormatoCreativo::BoletinCorreo,
            ],
            ahora,
        )
        .await?;
    let linea = tx.linea_predeterminada().await?;
    let mut enviados = 0;
    for p in elegir(piezas, ahora, usize::MAX) {
        let canal = p.contenido.formato.canal();
        if canal == CanalPublicidad::Whatsapp && linea.is_none() {
            continue;
        }
        let Some((c, _)) = tx.pieza_para_entrega(p.candidata.creativo, ahora).await? else {
            continue;
        };
        if !c.entregable(ahora) {
            continue;
        }
        let destinatarios = tx
            .destinatarios_masivos(c.campana, canal, ahora, LOTE_MASIVO)
            .await?;
        let mensajes: Vec<_> = destinatarios
            .into_iter()
            .filter_map(|d| mensaje(&p, canal, linea.as_deref(), tenant, &d).map(|m| (d, m)))
            .collect();
        let pedido = Conteo {
            impresiones: u32::try_from(mensajes.len()).unwrap_or(u32::MAX),
            ..Conteo::default()
        };
        let disponible = c.presupuesto.disponible(c.consumido_total, c.consumido_hoy);
        let caben = ajustar_al_presupuesto(c.modelo, c.tarifa, pedido, disponible).impresiones;
        if caben == 0 {
            continue;
        }
        for (d, m) in mensajes.into_iter().take(caben as usize) {
            tx.encolar_publicidad(
                &EventoSaliente {
                    prioridad: PrioridadSalida::Publicidad,
                    mensaje: m,
                },
                c.campana,
            )
            .await?;
            tx.registrar_envio_publicidad(c.campana, d.persona_id, canal, ahora)
                .await?;
        }
        let aceptado = Conteo {
            impresiones: caben,
            ..Conteo::default()
        };
        let valor = costo(c.modelo, c.tarifa, aceptado);
        let finalizar = presupuesto_agotado(
            c.modelo,
            c.tarifa,
            &c.presupuesto,
            c.consumido_total + valor,
        );
        tx.aplicar_entrega(c.creativo, canal, ahora, aceptado, valor, finalizar)
            .await?;
        enviados += caben;
    }
    tx.confirmar().await?;
    Ok(enviados)
}

/// Plantilla «… · {{1}}: {{2}}» con el botón «No recibir más», o el boletín por correo con
/// las condiciones y cómo darse de baja. `None` si a la persona le falta el dato del canal.
fn mensaje(
    p: &PiezaPublicidad,
    canal: CanalPublicidad,
    linea: Option<&str>,
    tenant: TenantId,
    d: &crate::puertos::publicidad::DestinatarioMasivo,
) -> Option<MensajeOutbox> {
    let c = &p.contenido;
    match canal {
        CanalPublicidad::Whatsapp => Some(MensajeOutbox::WhatsApp(MensajeSaliente {
            linea: linea?.to_owned(),
            telefono: d.telefono.clone()?,
            contenido: ContenidoSaliente::Plantilla {
                nombre: c.plantilla_whatsapp.clone()?,
                idioma: IDIOMA_PLANTILLAS.to_owned(),
                parametros: vec![parametro(&p.nombre_comercial), parametro(&c.titulo)],
                botones: vec![BotonBaja { tenant }.id()],
            },
        })),
        CanalPublicidad::Correo => {
            let para = domain::DireccionCorreo::new(d.correo.as_deref()?).ok()?;
            let mut texto = format!("{}\n\n{}", c.texto, p.nombre_comercial);
            if let Some(cond) = &c.condiciones {
                texto.push_str(&format!("\n\nCondiciones: {cond}"));
            }
            if let Some(e) = &c.enlace {
                texto.push_str(&format!("\n\n{e}"));
            }
            texto.push_str(
                "\n\nRecibe este mensaje porque autorizó la publicidad de la plataforma. \
                 Puede retirar la autorización en la app, en «Mis datos».",
            );
            Some(MensajeOutbox::Correo(CorreoSaliente {
                para,
                asunto: c.titulo.clone(),
                texto,
                html: None,
            }))
        }
        CanalPublicidad::App | CanalPublicidad::Pantalla => None,
    }
}
