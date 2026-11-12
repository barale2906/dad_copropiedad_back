//! Campañas de actualización del censo por WhatsApp (plan 10, fase 5).
//!
//! - La administración (`censo.editar`) lanza una campaña: al responsable de cada bien con
//!   residentes (o al representante de los titulares si nadie reside) le llega la plantilla
//!   [`PLANTILLA_ACTUALIZAR_DATOS`] con dos botones (`cen:{tenant}:{campaña}:{bien}:ok|mod`).
//! - En el chat, «Están bien» registra la confirmación; «Hay cambios» muestra lo que el censo tiene
//!   del bien y pide describir los cambios en un mensaje.
//! - Las respuestas con cambios las revisa la administración, aplica los cambios en el censo con
//!   las herramientas de siempre y las marca atendidas. No entran como `cambios_censo`: esas
//!   propuestas exigen una cuenta y datos estructurados, y por WhatsApp muchos no tienen cuenta.

use domain::{
    BienPrivadoId, CampanaCensoId, Permiso, PersonaId, RespuestaCampanaCenso, TenantId,
    copropiedad::texto_obligatorio,
};

use crate::{
    AppError, AppResult, Contexto,
    auditoria::{AccionAuditoria, DetalleAuditoria, EventoAuditoria},
    casos_uso::{avisos::IDIOMA_PLANTILLAS, tickets::TxTickets},
    outbox::{EventoSaliente, MensajeOutbox, PrioridadSalida},
    puertos::{
        RegistroAuditoria, RegistroOutbox, Reloj, RepositorioCampanasCenso, RepositorioCenso,
        RepositorioCopropiedad, RepositorioPersonas, RepositorioTickets, Transaccion,
        UnidadDeTrabajo,
        censo::{CampanaCenso, RespuestaCenso, RespuestaCensoListada, ResumenCampanaCenso},
        mensajeria::{ContenidoSaliente, MensajeSaliente},
    },
};

pub(crate) const ACCION_LANZAR: AccionAuditoria = AccionAuditoria::new("censo.campana.lanzar");
pub(crate) const ACCION_ATENDER: AccionAuditoria = AccionAuditoria::new("censo.campana.atender");

/// Plantilla de WhatsApp (aprobar en Meta): «{{1}} está actualizando el censo. ¿Los datos de
/// {{2}} (residentes, vehículos y mascotas) siguen igual?» con los botones «Están bien» y «Hay
/// cambios».
pub const PLANTILLA_ACTUALIZAR_DATOS: &str = "actualizar_datos";
const PREFIJO: &str = "cen:";
/// Máximo de caracteres de los cambios descritos.
pub const MAX_DETALLE: usize = 1000;

/// Puertos que usan los casos de uso de las campañas.
pub trait TxCampanasCenso: TxTickets + RepositorioCampanasCenso {}

impl<T> TxCampanasCenso for T where T: TxTickets + RepositorioCampanasCenso {}

/// Botón de la plantilla, decodificado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotonCampana {
    pub tenant: TenantId,
    pub campana: CampanaCensoId,
    pub bien: BienPrivadoId,
    /// `true` = «Hay cambios».
    pub cambios: bool,
}

impl BotonCampana {
    /// `payload` del botón.
    pub fn id(&self) -> String {
        format!(
            "{PREFIJO}{}:{}:{}:{}",
            self.tenant,
            self.campana,
            self.bien,
            if self.cambios { "mod" } else { "ok" }
        )
    }

    /// Lee el `payload`; `None` si no es de una campaña.
    pub fn desde_id(id: &str) -> Option<Self> {
        let mut p = id.strip_prefix(PREFIJO)?.split(':');
        let uuid = |s: Option<&str>| s.and_then(|x| uuid::Uuid::parse_str(x).ok());
        let tenant = TenantId::from_uuid(uuid(p.next())?);
        let campana = CampanaCensoId::from_uuid(uuid(p.next())?);
        let bien = BienPrivadoId::from_uuid(uuid(p.next())?);
        let cambios = match p.next()? {
            "ok" => false,
            "mod" => true,
            _ => return None,
        };
        p.next().is_none().then_some(Self {
            tenant,
            campana,
            bien,
            cambios,
        })
    }
}

fn parametro(texto: &str) -> String {
    texto
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(200)
        .collect()
}

/// Lanza una campaña: encola la plantilla a cada responsable con teléfono.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.editar`; [`AppError::Validacion`] si no hay una línea de
/// WhatsApp.
pub async fn lanzar_campana_censo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
) -> AppResult<CampanaCenso>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCampanasCenso,
{
    ctx.exigir(Permiso::CensoEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    let linea = tx
        .linea_predeterminada()
        .await?
        .ok_or_else(|| AppError::Validacion("no hay una línea de WhatsApp configurada".into()))?;
    let ph = tx.ficha().await?.map(|f| f.nombre).unwrap_or_default();
    let ahora = reloj.ahora();
    let destinatarios = tx.destinatarios_campana_censo(ahora).await?;
    let mut campana = CampanaCenso {
        id: CampanaCensoId::nuevo(),
        creada_por: ctx.actor_id,
        creada_en: ahora,
        enviados: 0,
    };
    for d in &destinatarios {
        let boton = |cambios| {
            BotonCampana {
                tenant: ctx.tenant_id,
                campana: campana.id,
                bien: d.bien,
                cambios,
            }
            .id()
        };
        tx.encolar(EventoSaliente {
            prioridad: PrioridadSalida::Recordatorio,
            mensaje: MensajeOutbox::WhatsApp(MensajeSaliente {
                linea: linea.clone(),
                telefono: d.telefono.clone(),
                contenido: ContenidoSaliente::Plantilla {
                    nombre: PLANTILLA_ACTUALIZAR_DATOS.to_owned(),
                    idioma: IDIOMA_PLANTILLAS.to_owned(),
                    parametros: vec![parametro(&ph), parametro(&d.codigo)],
                    botones: vec![boton(false), boton(true)],
                },
            }),
        })
        .await?;
        campana.enviados += 1;
    }
    tx.insertar_campana_censo(&campana).await?;
    tx.registrar(
        EventoAuditoria::ok(ACCION_LANZAR, "campana_censo", campana.id.as_uuid())
            .con_detalle(DetalleAuditoria::vacio().con("enviados", campana.enviados)?),
    )
    .await?;
    tx.confirmar().await?;
    Ok(campana)
}

/// Campañas con sus contadores.
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.ver`.
pub async fn listar_campanas_censo<U>(
    unidad: &U,
    ctx: &Contexto,
) -> AppResult<Vec<ResumenCampanaCenso>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCampanasCenso,
{
    ctx.exigir(Permiso::CensoVer)?;
    unidad.iniciar(ctx).await?.campanas_censo().await
}

/// Respuestas de una campaña (los cambios descritos son datos personales: solo con `censo.ver`).
///
/// # Errors
///
/// [`AppError::SinPermiso`]; [`AppError::NoEncontrado`] si la campaña no existe.
pub async fn respuestas_campana_censo<U>(
    unidad: &U,
    ctx: &Contexto,
    campana: CampanaCensoId,
) -> AppResult<Vec<RespuestaCensoListada>>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCampanasCenso,
{
    ctx.exigir(Permiso::CensoVer)?;
    let mut tx = unidad.iniciar(ctx).await?;
    tx.campana_censo(campana)
        .await?
        .ok_or(AppError::NoEncontrado("campaña del censo"))?;
    tx.respuestas_censo(campana).await
}

/// Marca atendida una respuesta con cambios (ya aplicados en el censo).
///
/// # Errors
///
/// [`AppError::SinPermiso`] sin `censo.editar`; [`AppError::NoEncontrado`] si no hay una
/// respuesta con cambios de esa persona y bien.
pub async fn atender_respuesta_censo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    campana: CampanaCensoId,
    persona: PersonaId,
    bien: BienPrivadoId,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCampanasCenso,
{
    ctx.exigir(Permiso::CensoEditar)?;
    let mut tx = unidad.iniciar(ctx).await?;
    if !tx
        .atender_respuesta_censo(campana, persona, bien, ctx.actor_id, reloj.ahora())
        .await?
    {
        return Err(AppError::NoEncontrado("respuesta con cambios"));
    }
    tx.registrar(EventoAuditoria::ok(
        ACCION_ATENDER,
        "campana_censo",
        campana.as_uuid(),
    ))
    .await?;
    tx.confirmar().await
}

/// Registra la respuesta que llegó por el chat (contexto del sistema de la PH del botón).
/// Los cambios se guardan en una sola línea, recortados a [`MAX_DETALLE`].
///
/// # Errors
///
/// [`AppError::NoEncontrado`] si la campaña no existe; [`AppError::Dominio`] si el texto de
/// los cambios queda vacío.
pub async fn registrar_respuesta_censo<U>(
    unidad: &U,
    reloj: &impl Reloj,
    ctx: &Contexto,
    boton: BotonCampana,
    persona: PersonaId,
    detalle: Option<&str>,
) -> AppResult<()>
where
    U: UnidadDeTrabajo,
    U::Transaccion: TxCampanasCenso,
{
    let detalle = match (boton.cambios, detalle) {
        (false, _) => None,
        (true, Some(d)) => {
            let linea: String = d
                .split(|c: char| c.is_control() || c.is_whitespace())
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(MAX_DETALLE)
                .collect();
            Some(texto_obligatorio("detalle", &linea, MAX_DETALLE)?)
        }
        (true, None) => {
            return Err(AppError::Validacion("describa los cambios".into()));
        }
    };
    let mut tx = unidad.iniciar(ctx).await?;
    tx.campana_censo(boton.campana)
        .await?
        .ok_or(AppError::NoEncontrado("campaña del censo"))?;
    tx.guardar_respuesta_censo(&RespuestaCenso {
        campana: boton.campana,
        persona,
        bien: boton.bien,
        respuesta: if boton.cambios {
            RespuestaCampanaCenso::Cambios
        } else {
            RespuestaCampanaCenso::Confirmado
        },
        detalle,
        respondida_en: reloj.ahora(),
        atendida_en: None,
        atendida_por: None,
    })
    .await?;
    tx.confirmar().await
}

/// Lo que el censo tiene del bien, en texto corto para el chat (nombres de residentes, placas
/// y mascotas). Sin documentos ni fechas de nacimiento.
///
/// # Errors
///
/// Errores de infraestructura.
pub async fn resumen_censo_del_bien<U>(
    unidad: &U,
    ctx: &Contexto,
    bien: BienPrivadoId,
) -> AppResult<String>
where
    U: UnidadDeTrabajo,
    U::Transaccion: RepositorioCenso + RepositorioPersonas,
{
    let mut tx = unidad.iniciar(ctx).await?;
    let mut residentes = Vec::new();
    for r in tx.residencias(Some(bien), false).await? {
        if let Some(p) = tx.persona(r.persona_id).await? {
            residentes.push(p.nombre);
        }
    }
    let placas: Vec<String> = tx
        .vehiculos(Some(bien), None, false)
        .await?
        .into_iter()
        .filter_map(|v| v.placa.map(|p| p.as_str().to_owned()))
        .collect();
    let mascotas: Vec<String> = tx
        .mascotas(Some(bien), true)
        .await?
        .into_iter()
        .map(|m| m.nombre)
        .collect();
    let lista = |v: &[String]| {
        if v.is_empty() {
            "ninguno".to_owned()
        } else {
            v.join(", ")
        }
    };
    Ok(format!(
        "Esto tenemos registrado:\n• Residentes: {}\n• Vehículos: {}\n• Mascotas: {}",
        lista(&residentes),
        lista(&placas),
        lista(&mascotas)
    ))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use domain::{ConjuntoPermisos, CuentaId};

    use super::*;
    use crate::puertos::{
        censo::DestinatarioCampanaCenso, reloj::RelojFijo, unidad_trabajo::memoria::UnidadEnMemoria,
    };

    fn reloj() -> RelojFijo {
        RelojFijo(
            Utc.with_ymd_and_hms(2026, 11, 10, 15, 0, 0)
                .single()
                .expect("fecha"),
        )
    }

    fn admin(tenant: TenantId) -> Contexto {
        Contexto::de_cuenta(
            tenant,
            CuentaId::nuevo(),
            ConjuntoPermisos::todos(domain::Ambito::Copropiedad),
        )
    }

    fn con_destinatarios(n: usize) -> UnidadEnMemoria {
        let unidad = UnidadEnMemoria::default();
        unidad.con_datos(|d| {
            d.gestion.linea = Some("PNID".into());
            d.campanas.destinatarios = (0..n)
                .map(|i| DestinatarioCampanaCenso {
                    persona: PersonaId::nuevo(),
                    bien: BienPrivadoId::nuevo(),
                    codigo: format!("T1-10{i}"),
                    telefono: format!("57300111220{i}"),
                })
                .collect();
        });
        unidad
    }

    #[tokio::test]
    async fn lanzar_envia_la_plantilla_con_los_dos_botones_a_cada_responsable() {
        let unidad = con_destinatarios(2);
        let tenant = TenantId::nuevo();
        let c = lanzar_campana_censo(&unidad, &reloj(), &admin(tenant))
            .await
            .expect("lanzada");
        assert_eq!(c.enviados, 2);
        let salida = unidad.outbox();
        assert_eq!(salida.len(), 2);
        let MensajeOutbox::WhatsApp(m) = &salida[0].1.mensaje else {
            panic!("WhatsApp");
        };
        let ContenidoSaliente::Plantilla {
            nombre,
            botones,
            parametros,
            ..
        } = &m.contenido
        else {
            panic!("plantilla");
        };
        assert_eq!(nombre, PLANTILLA_ACTUALIZAR_DATOS);
        assert_eq!(parametros[1], "T1-100");
        let b = BotonCampana::desde_id(&botones[1]).expect("botón");
        assert!(b.cambios && b.campana == c.id && b.tenant == tenant);
    }

    #[tokio::test]
    async fn lanzar_exige_permiso_y_una_linea() {
        let unidad = con_destinatarios(1);
        let tenant = TenantId::nuevo();
        let sin = Contexto::de_cuenta(tenant, CuentaId::nuevo(), ConjuntoPermisos::default());
        assert!(matches!(
            lanzar_campana_censo(&unidad, &reloj(), &sin).await,
            Err(AppError::SinPermiso(_))
        ));
        unidad.con_datos(|d| d.gestion.linea = None);
        assert!(matches!(
            lanzar_campana_censo(&unidad, &reloj(), &admin(tenant)).await,
            Err(AppError::Validacion(_))
        ));
    }

    #[tokio::test]
    async fn solo_se_atienden_respuestas_con_cambios() {
        let unidad = con_destinatarios(1);
        let tenant = TenantId::nuevo();
        let ctx = admin(tenant);
        let c = lanzar_campana_censo(&unidad, &reloj(), &ctx)
            .await
            .expect("lanzada");
        let d = unidad.datos().campanas.destinatarios[0].clone();
        let boton = BotonCampana {
            tenant,
            campana: c.id,
            bien: d.bien,
            cambios: false,
        };
        let sistema = Contexto::new(tenant);
        registrar_respuesta_censo(&unidad, &reloj(), &sistema, boton, d.persona, None)
            .await
            .expect("confirmada");
        let r = atender_respuesta_censo(&unidad, &reloj(), &ctx, c.id, d.persona, d.bien).await;
        assert!(matches!(r, Err(AppError::NoEncontrado(_))));

        let cambios = BotonCampana {
            cambios: true,
            ..boton
        };
        registrar_respuesta_censo(
            &unidad,
            &reloj(),
            &sistema,
            cambios,
            d.persona,
            Some("Se fue Juan"),
        )
        .await
        .expect("cambios");
        atender_respuesta_censo(&unidad, &reloj(), &ctx, c.id, d.persona, d.bien)
            .await
            .expect("atendida");
        let resumen = listar_campanas_censo(&unidad, &ctx).await.expect("lista");
        assert_eq!(
            (
                resumen[0].con_cambios,
                resumen[0].por_atender,
                resumen[0].confirmados
            ),
            (1, 0, 0),
            "la nueva respuesta reemplazó la confirmación"
        );
    }

    #[test]
    fn el_boton_va_y_vuelve() {
        let b = BotonCampana {
            tenant: TenantId::nuevo(),
            campana: CampanaCensoId::nuevo(),
            bien: BienPrivadoId::nuevo(),
            cambios: true,
        };
        assert_eq!(BotonCampana::desde_id(&b.id()), Some(b));
        assert!(b.id().len() <= 128, "cabe en el payload de Meta");
        assert_eq!(BotonCampana::desde_id("cen:x:y:z:ok"), None);
        assert_eq!(BotonCampana::desde_id(&format!("{}:extra", b.id())), None);
        assert_eq!(BotonCampana::desde_id("asis:1"), None);
    }
}
